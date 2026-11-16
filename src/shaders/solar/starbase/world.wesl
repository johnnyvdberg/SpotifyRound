// A starbase's maps, painted once (`bake::bake_city`).
//
// - The world inside the base: a hollow cylinder kilometres across, the
//   tunnel and the port along its axis, land and city curving up round it
//   and overhead, three strip suns along it. Its ground, all the way round
//   and tiling along it, with its cities (`ground.wesl`, from `city.wesl`:
//   towns by rank, arterial roads between them, rivers along the axis,
//   districts, streets and lots, the same as `starbase::city`); its relief
//   (how high the ground stands, levelled where it is built over, where the
//   roads run and a number that changes along them, for the traffic); and
//   the clouds over it, at their own level.
// - A tile of blocks, laid over the ground many times for its fine grain.
// - The face of the megastructure's dish, about the mouth: rings of
//   plating in panels, rows of lit windows along the rings, a lit bay here
//   and there, and bands of light.
//
// `pane.wesl` and `surface.wesl` read them.

import package::noise3d::noise;
import package::solar::noise::TAU;
import package::solar::frame::hash_cell;
import package::solar::encounter::CraterBake;
import package::solar::starbase::ground::{ground_colour, ground_relief};

// Texels round the cylinder and along it, a tile.
const CITY_WIDTH: f32 = 2048.0;
const CITY_HEIGHT: f32 = 1024.0;
// The layers: the ground inside, the tile of blocks, the clouds inside,
// the dish's face, the ground's relief.
const BLOCKS_LAYER: i32 = 1;
const CLOUDS_LAYER: i32 = 2;
const SURFACE_LAYER: i32 = 3;
const RELIEF_LAYER: i32 = 4;
// And the metals the base is built of, photographs written in from
// outside (`bake::write_metals`).
const METAL_LAYER: i32 = 5;
// The cylinder: its axis' place across the base's frame, the ground's
// radius and the clouds', and the metres its maps tile along it (a texel
// as long as it is wide on the ground).
const WORLD_AXIS: f32 = 200.0;
const WORLD_GROUND: f32 = 4200.0;
const WORLD_CLOUDS: f32 = 3500.0;
const WORLD_TILE: f32 = 13194.0;
// The side of the dish's map, metres, about the mouth.
const SURFACE_SIDE: f32 = 64000.0;

// Noise round the cylinder and along it at `cells` lattice cells round,
// the same again where the map wraps either way: round a circle one way,
// two of it blended the other.
fn wrapped(x: vec2<f32>, cells: f32, seed: f32) -> f32 {
    let a = x.x / CITY_WIDTH * TAU;
    let w = x.y / CITY_HEIGHT;
    let span = cells * CITY_HEIGHT / CITY_WIDTH;
    let p = vec3<f32>(cos(a) * cells / TAU + seed * 17.3, sin(a) * cells / TAU + seed * 9.1, w * span);
    return mix(noise(p), noise(p - vec3<f32>(0.0, 0.0, span)), w);
}

// Fractal noise of it, five octaves from `cells` round, about -0.7..0.7.
fn wrapped_fbm(x: vec2<f32>, cells: f32, seed: f32) -> f32 {
    var sum = 0.0;
    var amplitude = 0.5;
    var c = cells;
    for (var i = 0; i < 5; i++) {
        sum += amplitude * wrapped(x, c, seed + f32(i) * 3.7);
        c *= 2.0;
        amplitude *= 0.5;
    }
    return sum;
}

// The cells round points scattered one to a square of `size` texels,
// wrapping every `period` squares: how far `x` is from its cell's edge (0
// on it, in squares) and a value of the cell's own.
fn cells_at(x: vec2<f32>, size: f32, period: vec2<f32>, seed: f32) -> vec2<f32> {
    let g = x / size;
    let cell = floor(g);
    let f = g - cell;
    var first = 9.0;
    var second = 9.0;
    var value = 0.0;
    for (var k = 0; k < 9; k++) {
        let o = vec2<f32>(f32(k % 3) - 1.0, f32(k / 3) - 1.0);
        let c = cell + o;
        let id = c - floor(c / period) * period;
        let r = o + vec2<f32>(hash_cell(vec3<f32>(id, seed)), hash_cell(vec3<f32>(id, seed + 1.0))) * 0.8 + 0.1 - f;
        let d = length(r);
        if (d < first) {
            second = first;
            first = d;
            value = hash_cell(vec3<f32>(id, seed + 2.0));
        } else if (d < second) {
            second = d;
        }
    }
    return vec2<f32>(second - first, value);
}

// How near texel `x` round the cylinder is to one of the three strip
// suns, in texels, and which side of it (-1 or 1).
fn sun_off(x: f32) -> vec2<f32> {
    let u = fract(x / CITY_WIDTH * 3.0) - 0.5;
    return vec2<f32>(abs(u) * CITY_WIDTH / 3.0, sign(u));
}

// The ground inside at texel `x` (round, along), its cities among it
// (`ground.wesl`). Alpha: how built up.
fn ground_texel(x: vec2<f32>) -> vec4<f32> {
    return ground_colour(x, vec2<f32>(CITY_WIDTH, CITY_HEIGHT));
}

// The clouds inside at texel `x`.
fn clouds_texel(x: vec2<f32>) -> vec4<f32> {
    let puff = wrapped_fbm(x + vec2<f32>(300.0, 170.0), 9.0, 21.0);
    let sun = sun_off(x.x);
    let cover = smoothstep(0.2, 0.42, puff) * 0.6;
    let shade = 0.5 + 0.5 * exp(-sun.x / 200.0) + 0.6 * (puff - 0.3);
    return vec4<f32>(vec3<f32>(0.62, 0.64, 0.68) * shade * cover, cover);
}

// The tile of blocks at texel `x`: roofs, each block its own shape and
// shade, lanes between them, and lights scattered over them, warm and
// cool.
fn blocks_texel(x: vec2<f32>) -> vec3<f32> {
    let block = cells_at(x, 16.0, vec2<f32>(CITY_WIDTH, CITY_HEIGHT) / 16.0, 51.0);
    let roof = smoothstep(0.05, 0.12, block.x);
    let h = block.y;
    let light = step(0.965 - 0.07 * h, hash_cell(vec3<f32>(floor(x * 0.5), 53.0)));
    let white = mix(vec3<f32>(1.0, 0.76, 0.46), vec3<f32>(0.6, 0.84, 1.0), step(0.6, fract(h * 9.0)));
    return vec3<f32>(0.02, 0.021, 0.024) * roof * (0.3 + 1.3 * h) + white * light * 0.6;
}

// The ground's relief at texel `x`: how high it stands (0 the lowlands
// and the water, 1 the peaks), how near a highway, and a number that
// changes along the highways, for the lights that run along them.
fn ground_relief_texel(x: vec2<f32>) -> vec4<f32> {
    return ground_relief(x, vec2<f32>(CITY_WIDTH, CITY_HEIGHT));
}

// The face of the megastructure's dish at texel `x`: rings of plating,
// each cut into panels, a shade each, seams between; rows of windows
// along the rings, some lit; a panel here and there a lit bay; and bands
// of light round the entrance and inside the rim. Alpha: its own light.
fn surface_texel(x: vec2<f32>) -> vec4<f32> {
    let m = (x / vec2<f32>(CITY_WIDTH, CITY_HEIGHT) - 0.5) * SURFACE_SIDE;
    let r = length(m);
    let a = atan2(m.y, m.x) / TAU + 0.5;
    let ring = r / 1400.0;
    let ri = floor(ring);
    let rf = ring - ri;
    // More panels to a ring the further out, so they stay much of a size.
    let panels = 6.0 + 3.0 * ri;
    let sector = a * panels + hash_cell(vec3<f32>(ri, 3.0, 81.0));
    let si = floor(sector);
    let sf = sector - si;
    let h = hash_cell(vec3<f32>(ri, si - floor(si / panels) * panels, 83.0));
    let seam = max(1.0 - smoothstep(0.0, 0.035, min(rf, 1.0 - rf)), 1.0 - smoothstep(0.0, 0.012, min(sf, 1.0 - sf)));
    var colour = vec3<f32>(0.075, 0.085, 0.11) * (0.7 + 0.6 * h) * (1.0 - 0.6 * seam);
    // Rows of windows: five to a ring, a dash every ninety metres round.
    let row = rf * 5.0;
    let dash = floor(a * TAU * r / 90.0);
    let lit = step(hash_cell(vec3<f32>(dash, floor(row) + ri * 5.0, 85.0)), 0.08 + 0.34 * step(0.45, h))
        * step(abs(fract(row) - 0.5), 0.14) * step(0.3, h);
    // A bay: the middle of a panel, lit.
    let bay = step(0.93, h) * step(abs(rf - 0.5), 0.28) * step(abs(sf - 0.5), 0.3);
    let glow = max(1.0 - smoothstep(0.0, 70.0, abs(r - 5200.0)), 1.0 - smoothstep(0.0, 90.0, abs(r - 30800.0)));
    colour = mix(colour, vec3<f32>(0.9, 0.7, 0.4), bay * 0.5);
    return vec4<f32>(colour, clamp(max(max(lit * 0.8, bay), glow), 0.0, 1.0) * step(r, 32000.0));
}

// The layer a pipeline paints: one each, so each compiles small.
override CITY_LAYER: i32 = 0;

// A layer's mip level (the instance): each of a coarser level's texels is
// the mean of the four points in it.
@fragment
fn fs_bake_city(in: CraterBake) -> @location(0) vec4<f32> {
    let level = in.layer;
    let scale = f32(1u << level);
    let corner = floor(in.position.xy) * scale;
    let n = select(1, 2, level > 0u);
    var sum = vec4<f32>(0.0);
    for (var i = 0; i < n; i++) {
        for (var j = 0; j < n; j++) {
            let x = corner + (vec2<f32>(f32(i), f32(j)) + 0.5) / f32(n) * scale;
            if (CITY_LAYER == BLOCKS_LAYER) {
                sum += vec4<f32>(blocks_texel(x), 1.0);
            } else if (CITY_LAYER == CLOUDS_LAYER) {
                sum += clouds_texel(x);
            } else if (CITY_LAYER == SURFACE_LAYER) {
                sum += surface_texel(x);
            } else if (CITY_LAYER == METAL_LAYER) {
                // Written over afterwards.
                sum += vec4<f32>(0.0);
            } else if (CITY_LAYER == RELIEF_LAYER) {
                sum += ground_relief_texel(x);
            } else {
                sum += ground_texel(x);
            }
        }
    }
    // Colour stored halved: the suns are brighter than white.
    return vec4<f32>(sum.rgb * 0.5, sum.a) / f32(n * n);
}
