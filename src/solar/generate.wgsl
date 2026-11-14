// The solar tour's map generator: every kind of planet and moon, painted
// into equirectangular maps from 3D noise on the sphere. Prepended with
// `noise.wgsl` and the noise volume's declarations (`noise3d.rs`).
//
// It runs as many small pipelines, each within the V3D's four-thread
// register budget, a band of rows per frame (`stream.rs`):
//
// - `fs_height`: the large shapes and mountains; `fs_erosion` (and its
//   river valleys on living worlds) and `fs_craters` added on top. Craters
//   also leave their rays' brightness in the terrain's green channel and
//   calderas or cantaloupe cells in its blue, for the day colour to read
//   without crater loops of its own.
// - `fs_climate`: temperature, moisture, the coast's nearness and warmth, into
//   a scratch map; for lava and ice worlds and deserts, what their day and
//   night colours share or need worked out ahead, instead or besides.
// - `fs_relief`: the normal map's slopes and the water mask (for moons, the
//   third channel is left for `fs_glow`).
// - `fs_day`: the colour, from all of the above.
// - `fs_lights` (red), `fs_towns` (added) and `fs_clouds` (green): the night
//   glow and the cloud sheet, for planets.
// - `fs_sky`: the system's sky (the galactic band).

struct Gen {
    // x, y: target size in texels; z: sea level; w: the kind.
    size: vec4<f32>,
    offsets: array<vec4<f32>, 4>,
    plates: array<vec4<f32>, 12>,
    storms: array<vec4<f32>, 12>,
    palette: array<vec4<f32>, 4>,
    // x: how many mip levels finer than a 2048-wide map's the terrain is
    // (log2 of width / 2048; 0 for moons), y: cities, z: cratered (1 or 0), w: warmth (+ hotter,
    // - colder).
    flags: vec4<f32>,
};

// Which family of kinds a `fs_day`, `fs_lights` or `fs_clouds` pipeline is
// built for, and which scale a `fs_craters` one paints, so each holds only
// its own code.
override GROUP: i32 = 0;

@group(0) @binding(0) var<uniform> gen: Gen;
@group(0) @binding(1) var terrain: texture_2d<f32>;
@group(0) @binding(2) var terrain_sampler: sampler;
// The scratch climate map and the finished relief, for the passes after them.
@group(0) @binding(3) var climate_map: texture_2d<f32>;
@group(0) @binding(4) var relief_map: texture_2d<f32>;

const TERRAN: i32 = 0;
const DESERT: i32 = 1;
const OCEAN: i32 = 2;
const ICE: i32 = 3;
const LAVA: i32 = 4;
const BARREN: i32 = 5;
const GAS: i32 = 6;
const ICE_GIANT: i32 = 7;
const VOLCANIC: i32 = 8;
const ICY_MOON: i32 = 9;
const HAZY: i32 = 10;
const CAPTURED: i32 = 11;
const CLOUDY: i32 = 12;
const EYEBALL: i32 = 13;
const HOT: i32 = 14;
const JUNGLE: i32 = 15;
const GEYSER: i32 = 16;
const TWO_FACED: i32 = 17;
const NITROGEN: i32 = 18;
const RUSTY: i32 = 19;

fn kind() -> i32 {
    return i32(gen.size.w + 0.5);
}

struct Varying {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

fn sphere_at(uv: vec2<f32>) -> vec3<f32> {
    let lon = (uv.x - 0.5) * TAU;
    let lat = (0.5 - uv.y) * PI;
    return vec3<f32>(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
}

fn ridged(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    var weight = 1.0;
    for (var i = 0; i < octaves; i++) {
        var r = 1.0 - abs(noise(q));
        r = r * r * weight;
        weight = clamp(r * 1.6, 0.0, 1.0);
        sum += amplitude * r;
        q = OCTAVE * q * 2.07;
        amplitude *= 0.5;
    }
    return sum;
}

fn eroded(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    var slope = vec3<f32>(0.0);
    var frequency = 1.0;
    for (var i = 0; i < octaves; i++) {
        let n = noised(q);
        // Kept in this octave's frame: the turn is orthonormal, so the
        // slope's length is the same as in the sphere's own.
        slope += n.yzw * frequency * amplitude;
        sum += amplitude * n.x / (1.0 + dot(slope, slope) * 0.35);
        q = OCTAVE * q * 2.01;
        slope = OCTAVE * slope;
        frequency *= 2.01;
        amplitude *= 0.5;
    }
    return sum;
}

// Craters at `scale` cells per radius: bowls with raised rims, kept with
// probability `density`. Returns the height change and the bright fresh
// ejecta around the younger ones.
fn craters(p: vec3<f32>, scale: f32, density: f32, seed: i32) -> vec2<f32> {
    let x = p * scale;
    let cell = vec3<i32>(floor(x));
    var h = 0.0;
    var rays = 0.0;
    for (var k = 0; k < 27; k++) {
        let id = cell + vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        if (hash3(id + vec3<i32>(seed, 5, 9)) > density) {
            continue;
        }
        let centre = vec3<f32>(id) + vec3<f32>(
            hash3(id + vec3<i32>(seed, 1, 7)),
            hash3(id + vec3<i32>(seed, 2, 7)),
            hash3(id + vec3<i32>(seed, 3, 7)),
        );
        let size = hash3(id + vec3<i32>(seed, 4, 7));
        let radius = 0.15 + 0.45 * size * size;
        let d = distance(x, centre) / radius;
        if (d < 1.0) {
            h += (d * d - 1.0) * 0.6 * radius;
            // The bigger ones have a central peak.
            h += exp(-d * d / 0.02) * 0.2 * radius * step(0.45, size);
        }
        let rim = (d - 1.0) / 0.22;
        h += exp(-rim * rim) * 0.25 * radius;
        if (hash3(id + vec3<i32>(seed, 6, 7)) > 0.8) {
            // The young ones' bright ejecta, thrown out in streaks.
            let dir = normalize(x - centre);
            let streak = smoothstep(0.1, 0.9, sin(dot(dir, vec3<f32>(11.0, 17.0, 13.0)) + size * 40.0) * sin(dot(dir, vec3<f32>(-19.0, 7.0, 15.0))) + 0.5);
            rays += exp(-max(d - 1.0, 0.0) * mix(2.4, 0.9, streak)) * step(0.9, d) * 0.6 * mix(0.4, 1.0, streak);
        }
    }
    return vec2<f32>(h / scale, rays);
}

// A hash of a point, 0..1, in floats: the V3D has no 32-bit integer
// multiply, so `hash3`'s cost it four times over in registers, enough in
// a loop to halve its threads.
fn sq(x: f32) -> f32 {
    return x * x;
}

fn hashf(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// The edges of cells round points scattered one to a lattice cell (a
// Voronoi diagram): how far `x` is from its cell's nearest edge (the true
// distance, to the bisector of its two nearest points, so edges are straight
// and even, as crusts and ice crack) and a value of its own for each cell.
fn cell_edges(x: vec3<f32>, seed: i32) -> vec2<f32> {
    let cell = vec3<i32>(floor(x));
    let f = x - floor(x);
    var nearest = vec3<f32>(0.0);
    var best = 9.0;
    var id = cell;
    for (var k = 0; k < 27; k++) {
        let o = vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        let c = cell + o;
        let r = vec3<f32>(o) + vec3<f32>(hash3(c + vec3<i32>(seed, 1, 0)), hash3(c + vec3<i32>(seed, 2, 0)), hash3(c + vec3<i32>(seed, 3, 0))) * 0.8 + 0.1 - f;
        let d = dot(r, r);
        if (d < best) {
            best = d;
            nearest = r;
            id = c;
        }
    }
    var edge = 9.0;
    for (var k = 0; k < 27; k++) {
        let o = vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        let c = cell + o;
        let r = vec3<f32>(o) + vec3<f32>(hash3(c + vec3<i32>(seed, 1, 0)), hash3(c + vec3<i32>(seed, 2, 0)), hash3(c + vec3<i32>(seed, 3, 0))) * 0.8 + 0.1 - f;
        let between = r - nearest;
        if (dot(between, between) > 1e-6) {
            edge = min(edge, dot(0.5 * (nearest + r), normalize(between)));
        }
    }
    return vec2<f32>(edge, hash3(id + vec3<i32>(seed, 9, 9)));
}

// Lava flows from volcanoes: a few vents in cells `scale` to a radius,
// streams running out from each, branching and fading as they cool. The
// glow 0..1.
fn volcano_flows(p: vec3<f32>, scale: f32, seed: i32) -> f32 {
    let x = p * scale;
    let cell = vec3<i32>(floor(x));
    var glow = 0.0;
    for (var k = 0; k < 27; k++) {
        let id = cell + vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        if (hash3(id + vec3<i32>(seed, 7, 1)) > 0.45) {
            continue;
        }
        let centre = vec3<f32>(id) + vec3<f32>(hash3(id + vec3<i32>(seed, 1, 5)), hash3(id + vec3<i32>(seed, 2, 5)), hash3(id + vec3<i32>(seed, 3, 5)));
        let v = x - centre;
        let d = length(v);
        let reach = 0.35 + 0.4 * hash3(id + vec3<i32>(seed, 4, 5));
        if (d > reach) {
            continue;
        }
        let way = v / max(d, 1e-4);
        // Streams: thin where the noise round the vent crosses zero, wiggling
        // as they run out, fewer the farther out.
        let stream = 1.0 - smoothstep(0.0, 0.09 + 0.05 * d, abs(noise(way * 7.0 + vec3<f32>(d * 2.5) + vec3<f32>(f32(seed)))));
        let fade = 1.0 - smoothstep(reach * 0.3, reach, d);
        let vent = exp(-d * d / 0.004);
        glow = max(glow, stream * fade * (0.5 + 0.5 * fade) + vent);
    }
    return clamp(glow, 0.0, 1.0);
}

// A lava world's crust: plates of cooled basalt (cells of a warped
// lattice, so they are irregular, some large, some broken small), worked
// out a lattice at a time (`fine` false: the large plates, true: the small
// ones broken out of some), each in a pipeline of its own as two would cost
// the V3D half its threads; joined by `stored_crust`. x: distance to the
// plate's edge (0 on a seam), y: the plate's own value.
fn lava_plates(p: vec3<f32>, fine: bool) -> vec2<f32> {
    let o = gen.offsets;
    let warp = vec3<f32>(fbm(p * 3.0 + o[2].xyz, 3), fbm(p * 3.0 + o[3].xyz, 3), fbm(p * 3.0 + o[1].zyx, 3)) * 0.9;
    if (fine) {
        return cell_edges(p * 34.0 + warp * 4.0 + o[1].xyz, 11);
    }
    return cell_edges(p * 14.0 + warp * 2.0 + o[0].xyz, 7);
}

// Long fractures crossing an ice shell, as Europa's lineae: arcs of great
// circles about scattered poles, each wavering, some doubled ridges.
// x: the nearest fracture's band (1 on it), y: its ridges.
fn lineae(p: vec3<f32>) -> vec2<f32> {
    let o = gen.offsets;
    var band = 0.0;
    var ridge = 0.0;
    // The waver and reach of every fracture from one fetch, shifted by each
    // its own amount: a fetch inside the loop, or more values carried
    // through it, cost the V3D half its threads.
    let n = noised(p * 3.0 + o[1].xyz);
    // Each world's own fractures.
    let seed = floor(o[0].x * 13.0) + 77.0;
    for (var i = 0; i < 26; i++) {
        let fi = f32(i);
        let h = vec4<f32>(hashf(vec3<f32>(fi, 1.0, seed)), hashf(vec3<f32>(fi, 2.0, seed)), hashf(vec3<f32>(fi, 3.0, seed)), hashf(vec3<f32>(fi, 4.0, seed)));
        let pole = normalize(h.xyz - 0.5);
        let wave = dot(n.yzw, h.yzx - 0.5) * 0.012;
        let d = abs(dot(p, pole) + wave + (h.w - 0.5) * 0.6);
        // Each runs only part way round.
        let along = smoothstep(-0.3, 0.2, n.x * (h.x * 2.0 - 1.0) + dot(n.yzw, h.zxy - 0.5) * 0.3);
        let width = 0.0018 + 0.004 * fract(h.w * 13.7);
        band = max(band, (1.0 - smoothstep(0.0, width, d)) * along);
        ridge = max(ridge, exp(-sq((d - width * 1.6) / (width * 0.5))) * along * step(0.5, fract(h.y * 7.3)));
    }
    return vec2<f32>(band, ridge);
}

// Kinds whose ground is shaped: everything but the giants and the worlds
// hidden under haze or cloud.
fn has_ground(k: i32) -> bool {
    return !(k == GAS || k == ICE_GIANT || k == HAZY || k == HOT || k == CLOUDY);
}

// Airless and icy worlds: little tectonics, a cratered face.
fn cratered(k: i32) -> bool {
    return k == BARREN || k == CAPTURED || k == ICY_MOON || k == DESERT || k == VOLCANIC
        || k == GEYSER || k == TWO_FACED || k == NITROGEN || k == RUSTY;
}

// Planets that live: oceans, clouds, climates, maybe cities.
fn living(k: i32) -> bool {
    return k == TERRAN || k == OCEAN || k == JUNGLE || k == EYEBALL;
}

struct Shape {
    q: vec3<f32>,
    boundary: f32,
    base: f32,
};

fn shape(p: vec3<f32>) -> Shape {
    let o = gen.offsets;
    let warp = vec3<f32>(
        fbm(p * 1.4 + o[0].xyz, 4),
        fbm(p * 1.4 + o[1].xyz, 4),
        fbm(p * 1.4 + o[2].xyz, 4),
    );
    var out: Shape;
    out.q = normalize(p + warp * 0.32);
    var weights = 0.0;
    var bias = 0.0;
    var d1 = 9.0;
    var d2 = 9.0;
    for (var i = 0; i < 12; i++) {
        let plate = gen.plates[i];
        let d = distance(out.q, plate.xyz);
        let w = exp(-d * d * 14.0);
        weights += w;
        bias += w * plate.w;
        if (d < d1) {
            d2 = d1;
            d1 = d;
        } else if (d < d2) {
            d2 = d;
        }
    }
    out.boundary = d2 - d1;
    // Cratered worlds have little tectonics: half the large shapes.
    out.base = (0.42 * bias / max(weights, 1e-5) + 0.55 * fbm(out.q * 2.2 + o[3].xyz, 6)) * (1.0 - 0.5 * gen.flags.z);
    return out;
}

// Raw elevation without erosion or craters (sea level not yet taken off).
@fragment
fn fs_height(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    if (!has_ground(k)) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }
    let p = sphere_at(in.position.xy / gen.size.xy);
    let o = gen.offsets;
    let s = shape(p);
    let landish = smoothstep(-0.15, 0.25, s.base);
    let belt = exp(-s.boundary * s.boundary / (0.07 * 0.07));
    let ridges = ridged(s.q * 5.5 + o[0].zxy, 6);
    let scattered = smoothstep(0.15, 0.55, fbm(s.q * 3.0 + o[1].zxy, 3));
    let mountains = ridges * (belt * (0.35 + 0.65 * landish) * 0.42 + scattered * 0.12 * landish) * (1.0 - 0.65 * gen.flags.z);
    return vec4<f32>(s.base + mountains, 0.0, 0.0, 1.0);
}

// The eroded detail, added.
@fragment
fn fs_erosion(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    if (!has_ground(k)) {
        return vec4<f32>(0.0);
    }
    let s = shape(sphere_at(in.position.xy / gen.size.xy));
    let landish = smoothstep(-0.15, 0.25, s.base);
    if (GROUP == 1) {
        // River valleys (`GROUP` 1, a pass of their own, planned only for
        // living worlds, to keep the V3D's four threads): narrow, branching
        // channels cut into the land, a broad set and finer ones feeding
        // them, where the warped noise crosses zero; shallower on the plains.
        let warp = noised(s.q * 5.0 + gen.offsets[1].xyz).yzw * 0.06;
        let trunk = 1.0 - smoothstep(0.0, 0.06, abs(noise(s.q * 11.0 + warp + gen.offsets[3].xyz)));
        let branch = 1.0 - smoothstep(0.0, 0.08, abs(noise(s.q * 29.0 + warp * 2.0 + gen.offsets[0].xyz)));
        let cut = (trunk * 0.009 + branch * 0.004) * landish * smoothstep(0.0, 0.12, s.base + 0.1);
        return vec4<f32>(-cut, 0.0, 0.0, 0.0);
    }
    let detail = eroded(s.q * 9.0 + gen.offsets[2].zxy, 7) * (0.05 + 0.10 * landish) * (1.0 - 0.4 * gen.flags.z);
    return vec4<f32>(detail, 0.0, 0.0, 0.0);
}

// Craters, added, one scale per pipeline (`GROUP` 0, 1, 2): pits into the
// elevation, the bright rays of young ones into green, and calderas
// (volcanic moons) or a close mesh of dimples (nitrogen ice) into blue.
@fragment
fn fs_craters(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    if (!cratered(k)) {
        return vec4<f32>(0.0);
    }
    let p = sphere_at(in.position.xy / gen.size.xy);
    var amount = 1.0;
    if (k == ICY_MOON || k == GEYSER || k == NITROGEN) { amount = 0.25; }
    if (k == DESERT) { amount = 0.4; }
    var out = vec4<f32>(0.0);
    if (k == VOLCANIC) {
        if (GROUP == 0) {
            let caldera = craters(p, 9.0, 0.5, 23);
            out.g = caldera.y;
            out.b = max(-caldera.x * 30.0, 0.0);
        }
    } else if (GROUP == 0) {
        let big = craters(p, 7.0, 0.55 * amount, 3);
        out.r = big.x * 1.6;
        out.g = big.y;
    } else if (GROUP == 1) {
        let mid = craters(p, 22.0, 0.6 * amount, 17);
        out.r = mid.x * 1.4;
        out.g = mid.y;
    } else if (k == NITROGEN) {
        out.b = craters(p, 28.0, 1.0, 51).x * 28.0;
    } else {
        out.r = craters(p, 70.0, 0.6 * amount, 41).x * 1.2;
    }
    return out;
}

fn srgb(r: f32, g: f32, b: f32) -> vec3<f32> {
    return pow(vec3<f32>(r, g, b) / 255.0, vec3<f32>(2.2));
}

// The terrain blurred over the same angle at any map size: `level` is
// a 2048-wide map's.
fn wide(uv: vec2<f32>, level: f32) -> f32 {
    return textureSampleLevel(terrain, terrain_sampler, uv, level + gen.flags.x).r - gen.size.z;
}

fn elevation_at(texel: vec2<i32>) -> f32 {
    // Wrapped by compare and select: the V3D has no integer divide.
    let w = i32(gen.size.x);
    var x = select(texel.x, texel.x + w, texel.x < 0);
    x = select(x, x - w, x >= w);
    let t = vec2<i32>(x, clamp(texel.y, 0, i32(gen.size.y) - 1));
    return textureLoad(terrain, t, 0).r - gen.size.z;
}

fn settlements(p: vec3<f32>, scale: f32, density: f32, seed: i32) -> f32 {
    let x = p * scale;
    let cell = vec3<i32>(floor(x));
    var light = 0.0;
    for (var k = 0; k < 27; k++) {
        let id = cell + vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        if (hash3(id + vec3<i32>(seed, 17, 3)) > density) {
            continue;
        }
        let centre = vec3<f32>(id) + vec3<f32>(
            hash3(id + vec3<i32>(seed, 1, 0)),
            hash3(id + vec3<i32>(seed, 2, 0)),
            hash3(id + vec3<i32>(seed, 3, 0)),
        );
        let size = hash3(id + vec3<i32>(seed, 4, 0));
        let population = size * size * size;
        let radius = 0.08 + 0.42 * population;
        let d = distance(x, centre) / radius;
        light += (0.25 + 0.75 * population) * exp(-d * d * 2.5);
    }
    return light;
}

fn turn_about(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

// Thin lines where a noise field crosses zero: cracks in ice and crust.
// One fetch: the noise and, for the wiggle a finer octave would add, its
// slope (two fetches unrolled cost the day pipelines two threads).
// A line thinner than a texel is widened to one and dimmed to match, so it
// stays unbroken rather than a trail of dots (called only in uniform
// control flow, for the derivative).
fn cracks(p: vec3<f32>, width: f32) -> f32 {
    let n = noised(p);
    let a = abs(n.x + 0.12 * (n.y + n.z));
    let texel = fwidth(a);
    return (1.0 - smoothstep(0.0, max(width, texel), a)) * min(1.0, width / max(texel, 1e-6) + 0.25);
}

fn pal(i: i32) -> vec3<f32> {
    return gen.palette[i].rgb;
}

struct Surface {
    day: vec3<f32>,
    bump: vec2<f32>,
    // Water for the glint; 0 elsewhere.
    water: f32,
    // City lights, or molten rock's glow.
    night: f32,
    cloud: f32,
};

// Bands of a giant: latitude warped by turbulence and wound by storms, the
// bands' edges curled into waves where the winds shear past each other, fine
// streaks along the winds, and small eddies with pale ovals at their hearts.
fn giant(p: vec3<f32>, k: i32) -> Surface {
    let o = gen.offsets;
    var c = p;
    for (var i = 0; i < 12; i++) {
        // The first six wind broad storms, the rest small, tight eddies.
        let storm = gen.storms[i];
        let r = select(0.06, 0.12, i < 6);
        let d = distance(c, storm.xyz);
        c = turn_about(c, storm.xyz, storm.w * select(2.4, 1.4, i < 6) * exp(-d * d / (r * r)));
    }
    // Curls: the flow wound a little at a finer scale before it is drawn out.
    // (One fetch: the noise's slope gives all three directions.)
    c = normalize(c + noised(c * 13.0 + o[1].zxy).yzw * 0.02);
    let streaks = vec3<f32>(c.x * 3.0, c.y * 34.0, c.z * 3.0);
    let turbulence = fbm(streaks + o[0].xyz, 6) * 0.9 + fbm(c * 9.0 + o[1].xyz, 4) * 0.35;
    let phase = c.y * (18.0 + o[2].w) + o[3].w + turbulence * 0.9;
    // Waves along the edges of the bands, where the winds shear.
    let lon = atan2(c.x, c.z);
    let edge = abs(cos(phase));
    let waves = sin(lon * 26.0 + c.y * 30.0 + turbulence * 2.0) * edge * edge * (1.0 - smoothstep(0.6, 0.85, abs(c.y)));
    let y = c.y + turbulence * 0.05 + waves * 0.008;
    let bands = sin(y * (18.0 + o[2].w) + o[3].w) * 0.5 + 0.5;
    let fine = sin(y * 61.0 + turbulence * 3.0) * 0.5 + 0.5;
    var colour = mix(pal(0), pal(1), smoothstep(0.25, 0.75, bands));
    colour = mix(colour, pal(2), smoothstep(0.55, 0.95, fine) * 0.45);
    colour = mix(colour, pal(3), smoothstep(0.85, 0.98, abs(p.y)) * 0.6);
    // Pale ovals in the small eddies, each with a darker ring.
    for (var i = 6; i < 12; i++) {
        let rel = p - gen.storms[i].xyz;
        let d = length(vec3<f32>(rel.x, rel.y * 1.7, rel.z)) / 0.036;
        colour = mix(colour, mix(pal(0), vec3<f32>(1.0), 0.3), (1.0 - smoothstep(0.45, 1.0, d)) * 0.85);
        colour = mix(colour, pal(1) * 0.75, exp(-(d - 1.1) * (d - 1.1) / 0.03) * 0.5);
    }
    // One great storm: an oval of the darker colour with a pale rim.
    let spot = gen.storms[0].xyz;
    let rel = p - spot;
    let oval = length(vec2<f32>(dot(rel, normalize(cross(vec3<f32>(0.0, 1.0, 0.0), spot))) * 0.55, rel.y)) / 0.07;
    colour = mix(colour, pal(1) * 0.85, (1.0 - smoothstep(0.6, 1.0, oval)) * 0.85);
    colour = mix(colour, pal(3), exp(-pow((oval - 1.1) / 0.15, 2.0)) * 0.35);
    if (k == ICE_GIANT) {
        colour = mix(pal(0), colour, 0.4);
    }
    var s: Surface;
    // Mottled, in fine streaks drawn out along the winds.
    s.day = colour * (0.93 + 0.14 * (fbm(vec3<f32>(c.x * 30.0, c.y * 120.0, c.z * 30.0), 3) + 0.5));
    s.bump = vec2<f32>(0.0);
    s.water = 0.0;
    s.night = 0.0;
    if (k == HOT) {
        // A hot Jupiter glows dull red from its own heat, brightest in the
        // hotter bands.
        s.night = 0.35 + 0.35 * bands + 0.2 * turbulence;
    }
    s.cloud = 0.0;
    return s;
}

// A deck of sulphuric cloud: soft streaks swept along the latitudes into
// broad chevrons, as Venus's are in ultraviolet.
fn cloudy(p: vec3<f32>) -> Surface {
    let o = gen.offsets;
    let lon = atan2(p.x, p.z);
    let chevron = p.y * 3.0 + abs(sin(lon * 0.5)) * 0.9 * sign(p.y);
    let swirl = fbm(vec3<f32>(p.x * 2.0, p.y * 9.0 + chevron, p.z * 2.0) + o[0].xyz, 6);
    let tone = smoothstep(-0.35, 0.45, swirl + 0.25 * sin(chevron * 3.0));
    var s: Surface;
    s.day = mix(pal(1), pal(2), tone) * (0.95 + 0.1 * fbm(p * 30.0 + o[1].xyz, 3));
    s.day = mix(s.day, pal(3), smoothstep(0.8, 0.98, abs(p.y)) * 0.3);
    s.bump = vec2<f32>(0.0);
    s.water = 0.0;
    s.night = 0.0;
    s.cloud = 0.0;
    return s;
}

// What the surface passes need to know about a texel.
struct Climate {
    p: vec3<f32>,
    uv: vec2<f32>,
    lat: f32,
    e: f32,
    land: f32,
    warmth: f32,
    temperature: f32,
    moisture: f32,
    near_coast: f32,
    // The climate map's texel as it is stored (deserts keep their sands in
    // its spare channels, `desert_sands`).
    stored: vec4<f32>,
};

// Land or not, its edge eased over about a texel so coasts and the edges of
// plains are not stepped when seen close (called only in uniform control
// flow, for the derivative).
fn land_of(e: f32) -> f32 {
    let w = max(0.0015, 0.7 * fwidth(e));
    return smoothstep(-w, w, e);
}

fn work_out_climate(position: vec2<f32>) -> Climate {
    let k = kind();
    var c: Climate;
    c.uv = position / gen.size.xy;
    c.p = sphere_at(c.uv);
    let o = gen.offsets;
    c.e = textureLoad(terrain, vec2<i32>(position), 0).r - gen.size.z;
    c.land = land_of(c.e);
    c.lat = asin(clamp(c.p.y, -1.0, 1.0));
    let cos_lat = max(cos(c.lat), 0.02);
    let wobble = fbm(c.p * 4.0 + o[3].zyx, 4);
    c.warmth = pow(cos_lat, 0.85) + wobble * 0.10 + gen.flags.w;
    if (k == EYEBALL) {
        // Locked to its star, which always stands over longitude 0 (+z):
        // warm under it, frozen past the terminator.
        c.warmth = mix(-0.35, 1.2, smoothstep(-0.25, 0.95, c.p.z)) + wobble * 0.10;
    } else if (k == JUNGLE) {
        c.warmth += 0.08;
    }
    c.temperature = c.warmth - max(c.e, 0.0) * 1.5;
    c.moisture = 0.0;
    c.near_coast = 0.0;
    if (living(k)) {
        // The circulation cells, then the air's path from the sea along the
        // prevailing wind, which mountains upwind wring dry.
        let degrees = abs(c.lat) * 57.2958;
        let from_east = select(-1.0, 1.0, degrees < 30.0 || degrees > 62.0);
        var ocean_upwind = 0.0;
        var barrier = 0.0;
        for (var i = 1; i <= 5; i++) {
            let up = wide(c.uv + vec2<f32>(from_east * f32(i) * 0.0105 / cos_lat, 0.0), 4.5);
            ocean_upwind += (1.0 - smoothstep(-0.02, 0.03, up)) / f32(i);
            barrier = max(barrier, up - max(c.e, 0.0));
        }
        ocean_upwind *= 60.0 / 137.0;
        c.near_coast = 1.0 - smoothstep(0.0, 0.12, max(wide(c.uv, 5.5), 0.0));
        let interior = smoothstep(0.03, 0.25, wide(c.uv, 7.5));
        c.moisture = clamp(
            select(0.0, 0.28, k == JUNGLE) + 0.40 + 0.26 * cos(6.0 * c.lat) + 0.30 * ocean_upwind
                + 0.12 * c.near_coast - 0.18 * interior - 0.35 * smoothstep(0.05, 0.3, barrier)
                + 0.22 * fbm(c.p * 6.0 + o[2].yzx, 5),
            0.0,
            1.0,
        );
    }
    return c;
}

// Temperature, moisture, the coast's nearness and warmth, encoded for an
// 8-bit map (temperatures from -0.5 to 1.5) (`GROUP` 0). Lava and ice worlds,
// which need no climate, keep there instead what their day and night maps
// share, worked out once, a pipeline for each part so that each keeps the
// V3D's four threads: the lava crust's large plates (1, red and green), the
// small ones (5, blue) and the flows from the vents (2, alpha); the ice
// shell's lineae (3, red and green) and its finer cracks (4, blue and
// alpha), read back by `stored_crust`. Deserts keep their sands in the
// channels besides warmth (6, `desert_sands`).
@fragment
fn fs_climate(in: Varying) -> @location(0) vec4<f32> {
    if (GROUP >= 1) {
        let p = sphere_at(in.position.xy / gen.size.xy);
        if (GROUP == 1) {
            let big = lava_plates(p, false);
            return vec4<f32>(edge_code(big.x), big.y, 0.0, 0.0);
        } else if (GROUP == 5) {
            return vec4<f32>(0.0, 0.0, edge_code(lava_plates(p, true).x), 0.0);
        } else if (GROUP == 2) {
            return vec4<f32>(0.0, 0.0, 0.0, volcano_flows(p, 9.0, 3));
        } else if (GROUP == 6) {
            return vec4<f32>(desert_sands(p, in.position.xy), 0.0);
        } else if (GROUP == 3) {
            return vec4<f32>(lineae(p), 0.0, 0.0);
        }
        let small = cell_edges(p * 28.0 + gen.offsets[1].xyz, 9);
        return vec4<f32>(0.0, 0.0, edge_code(small.x), small.y);
    }
    let c = work_out_climate(in.position.xy);
    return vec4<f32>((c.temperature + 0.5) * 0.5, c.moisture, c.near_coast, (c.warmth + 0.5) * 0.5);
}

// What a desert's day colour needs worked out ahead, kept in the climate
// map's channels a desert does not use (`GROUP` 6): how much its dune
// crests (along the wind, east to west, wavering, in the lowlands' dune
// seas) pale the sand, how much its dark wind streaks darken it, and how
// much its patches of another colour show.
fn desert_sands(p: vec3<f32>, position: vec2<f32>) -> vec3<f32> {
    let o = gen.offsets;
    let low = 1.0 - land_of(textureLoad(terrain, vec2<i32>(position), 0).r - gen.size.z);
    let wind = vec3<f32>(p.x, p.y * 6.0, p.z) * 40.0;
    let crest = 0.5 + 0.5 * sin(dot(p, vec3<f32>(0.0, 90.0, 0.0)) + noise(wind * 0.25 + o[1].zxy) * 6.0);
    let dunes = smoothstep(0.1, 0.4, low + smoothstep(-0.2, 0.2, fbm(p * 3.0 + o[2].zyx, 3)) * 0.5);
    let streak = smoothstep(0.3, 0.65, fbm(vec3<f32>(p.x * 22.0, p.y * 5.0, p.z * 22.0) + o[3].zxy, 4));
    let patches = smoothstep(0.2, 0.6, fbm(p * 18.0 + o[2].xyz, 4)) * 0.25;
    return vec3<f32>(crest * 0.35 * dunes, streak * 0.25 * (1.0 - dunes), patches);
}

// A distance to a cell's edge in 8 bits, finest near the edge, up to 0.25.
fn edge_code(x: f32) -> f32 {
    return sqrt(clamp(x * 4.0, 0.0, 1.0));
}

// What `fs_climate` kept for a lava or ice world, its edge distances
// decoded. Lava: x the distance to the plate's edge (0 on a seam), y the
// plate's own value, z the seam's heat (each seam its own, many nearly
// crusted over, a few running hot), w the flows. Ice: the lineae (as
// `lineae`), the finer cracks' edge and their cells' values.
fn stored_crust(position: vec2<f32>) -> vec4<f32> {
    let s = textureLoad(climate_map, vec2<i32>(position), 0);
    if (kind() == LAVA) {
        let big = s.r * s.r * 0.25;
        let broken = s.b * s.b * 0.25;
        // Some plates shattered into smaller ones.
        let shatter = s.g > 0.62;
        let edge = select(big, min(big, broken * 0.45), shatter);
        let plate = select(s.g, fract(s.g * 7.13 + broken * 9.0), shatter);
        let p = sphere_at(position / gen.size.xy);
        let heat = smoothstep(0.35, 1.0, noise(p * 7.0 + gen.offsets[2].zxy) * 0.5 + 0.5 + s.g * 0.2);
        return vec4<f32>(edge, plate, heat, s.a);
    }
    return vec4<f32>(s.r, s.g, s.b * s.b * 0.25, s.a);
}

fn climate(position: vec2<f32>) -> Climate {
    var c: Climate;
    c.uv = position / gen.size.xy;
    c.p = sphere_at(c.uv);
    c.e = textureLoad(terrain, vec2<i32>(position), 0).r - gen.size.z;
    c.land = land_of(c.e);
    c.lat = asin(clamp(c.p.y, -1.0, 1.0));
    let stored = textureLoad(climate_map, vec2<i32>(position), 0);
    c.stored = stored;
    c.temperature = stored.r * 2.0 - 0.5;
    c.moisture = stored.g;
    c.near_coast = stored.b;
    c.warmth = stored.a * 2.0 - 0.5;
    return c;
}

// The relief's slopes, as the normal map stores them.
fn slopes(position: vec2<f32>, lat: f32, floor_at: f32) -> vec2<f32> {
    let texel = vec2<i32>(position);
    let east_step = max(cos(lat), 0.02) * TAU / gen.size.x;
    let north_step = PI / gen.size.y;
    let de = (max(elevation_at(texel + vec2<i32>(1, 0)), floor_at) - max(elevation_at(texel - vec2<i32>(1, 0)), floor_at)) / (2.0 * east_step);
    let dn = (max(elevation_at(texel - vec2<i32>(0, 1)), floor_at) - max(elevation_at(texel + vec2<i32>(0, 1)), floor_at)) / (2.0 * north_step);
    return clamp(-vec2<f32>(de, dn) * 0.0025 * 22.0, vec2<f32>(-0.45), vec2<f32>(0.45));
}

// The sea ice on living worlds, where it is cold enough.
fn sea_ice(c: Climate) -> f32 {
    return smoothstep(0.17, 0.10, c.warmth + 0.04 + 0.04 * fbm(c.p * 20.0, 3));
}

// The normal map's slopes and the water mask.
@fragment
fn fs_relief(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    if (!has_ground(k)) {
        return vec4<f32>(0.5, 0.5, 0.0, 1.0);
    }
    let c = climate(in.position.xy);
    let wet = k == TERRAN || k == OCEAN || k == LAVA || k == ICE || k == JUNGLE || k == EYEBALL;
    var bump = slopes(in.position.xy, c.lat, select(-1.0, 0.0, wet));
    var water = 0.0;
    if (living(k)) {
        bump *= c.land;
        water = (1.0 - c.land) * (1.0 - sea_ice(c));
    } else if (k == LAVA) {
        bump *= c.land;
    } else if (k == ICE || k == NITROGEN) {
        bump *= 0.5;
    } else if (k == VOLCANIC || k == GEYSER) {
        bump *= 0.4;
    } else if (k == ICY_MOON) {
        bump *= 0.6;
    }
    return vec4<f32>(vec2<f32>(0.5) + bump, water, 1.0);
}

fn steepness(position: vec2<f32>) -> f32 {
    return length(textureLoad(relief_map, vec2<i32>(position), 0).rg - vec2<f32>(0.5));
}

// The terrain's crater channels: rays' brightness and calderas or cells.
fn features(position: vec2<f32>) -> vec2<f32> {
    return textureLoad(terrain, vec2<i32>(position), 0).gb;
}

// The day colour of a living world, as Earth's reads from orbit: dark
// forest where it is warm and wet, olive grassland and scrub drier, ochre
// and pale sand in the deserts, grey-brown tundra in the cold; bare rock on
// steep and high ground, snow by latitude and by height; the sea turquoise
// over the shelves, navy over the deeps, ice where it is cold enough.
fn living_day(c: Climate, steep: f32) -> vec3<f32> {
    let o = gen.offsets;
    let p = c.p;
    let moisture = clamp(c.moisture * 1.1 + 0.1, 0.0, 1.0);
    let temperature = c.temperature;
    let variety = fbm(p * 14.0 + o[0].yxz, 4);
    let sand = mix(srgb(214.0, 184.0, 140.0), srgb(176.0, 120.0, 76.0), 0.5 * smoothstep(-0.2, 0.3, variety));
    let scrub = srgb(146.0, 128.0, 88.0);
    let grass = srgb(104.0, 108.0, 58.0);
    let temperate = srgb(40.0, 64.0, 32.0);
    let tropical = srgb(24.0, 50.0, 20.0);
    let tundra = srgb(104.0, 98.0, 84.0);
    var ground = mix(sand, scrub, smoothstep(0.12, 0.3, moisture));
    ground = mix(ground, grass, smoothstep(0.28, 0.48, moisture));
    let forest = mix(temperate, tropical, smoothstep(0.62, 0.85, temperature));
    ground = mix(ground, forest, smoothstep(0.45, 0.68, moisture) * smoothstep(0.38, 0.5, temperature));
    ground = mix(ground, tundra, smoothstep(0.44, 0.32, temperature));
    // Bare rock on steep slopes and high ground, darker for the shadowed
    // folds the map's slope cannot hold.
    let rock = mix(srgb(96.0, 86.0, 76.0), srgb(70.0, 64.0, 60.0), smoothstep(-0.2, 0.3, variety));
    ground = mix(ground, rock, clamp(smoothstep(0.1, 0.32, steep) * 0.85 + smoothstep(0.1, 0.3, c.e) * 0.6, 0.0, 0.9));
    // Snow: in the cold, and on the peaks, its line lower toward the poles.
    let snow_line = 0.46 - 0.2 * smoothstep(0.3, 1.0, abs(c.lat) / 1.5708);
    let snow = max(smoothstep(0.24, 0.14, temperature), smoothstep(snow_line, snow_line + 0.05, c.e) * smoothstep(0.85, 0.5, temperature) * smoothstep(0.06, 0.2, steep));
    ground = mix(ground, srgb(236.0, 240.0, 245.0), snow * (1.0 - 0.4 * smoothstep(0.2, 0.4, steep)));
    ground *= 0.86 + 0.28 * (fbm(p * 60.0 + o[1].yxz, 4) + 0.5);
    let depth = max(-c.e, 0.0);
    var sea = mix(srgb(36.0, 116.0, 136.0), srgb(14.0, 46.0, 84.0), smoothstep(0.0, 0.022, depth));
    sea = mix(sea, srgb(5.0, 14.0, 42.0), smoothstep(0.03, 0.2, depth));
    sea = mix(sea, srgb(218.0, 226.0, 236.0), sea_ice(c));
    return mix(sea, ground, c.land);
}

// The day colour of every other kind with ground.
fn rocky_day(k: i32, c: Climate, steep: f32, feature: vec2<f32>) -> vec3<f32> {
    let o = gen.offsets;
    let p = c.p;
    let low = 1.0 - c.land;
    let grain = fbm(p * 60.0 + o[1].yxz, 4);
    if (k == DESERT) {
        // Rust and ochre, darker basalt lowlands, polar caps; patches of
        // another colour, paler sand on the crests of the lowlands' dune
        // seas, dark streaks blown out from the heights (these three worked
        // out ahead by `desert_sands`, to keep the V3D's four threads here);
        // layered rock where the ground is high and steep.
        let tone = smoothstep(-0.3, 0.4, fbm(p * 5.0 + o[0].xyz, 5));
        var ground = mix(pal(0), pal(2), tone * 0.6);
        ground = mix(ground, pal(1), low * 0.7 + smoothstep(0.1, 0.35, steep) * 0.3);
        ground = mix(ground, pal(3), c.stored.b);
        ground = mix(ground, pal(0) * 1.25, c.stored.r) * (1.0 - c.stored.g);
        let strata = 0.5 + 0.5 * sin(c.e * 160.0 + tone * 4.0);
        ground *= mix(1.0, 0.8 + 0.35 * strata, smoothstep(0.08, 0.3, steep) * smoothstep(0.0, 0.15, c.e));
        let cap = smoothstep(0.2, 0.12, c.warmth + 0.05 * fbm(p * 12.0, 3));
        ground = mix(ground, srgb(236.0, 232.0, 226.0), cap);
        return ground * (0.88 + 0.24 * (grain + 0.5));
    } else if (k == ICE) {
        // An ice shell cracked into plates, as Europa's: long straight-ish
        // fractures (the edges of cells), the big ones doubled ridges with
        // a reddish-brown band of salts between, finer ones crossing them,
        // and patches of jumbled chaos ice stained the same.
        let kept = stored_crust(c.uv * gen.size.xy);
        let lines = kept.xy;
        let small = kept.zw;
        let stain = srgb(150.0, 104.0, 78.0);
        let fine = (1.0 - smoothstep(0.0, 0.012, small.x)) * 0.5;
        let chaos = smoothstep(0.4, 0.65, fbm(p * 7.0 + o[3].xyz, 4)) * smoothstep(0.5, 0.9, small.y);
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 6.0 + o[2].xyz, 4)));
        ground = mix(ground, pal(1), low * 0.5);
        // Subtle: stains, not drawn lines.
        ground = mix(ground, stain, clamp(lines.x * 0.45 + fine * 0.15 + chaos * 0.3, 0.0, 0.6));
        ground = mix(ground, ground * 1.1 + vec3<f32>(0.02), lines.y * 0.3);
        return ground * (0.94 + 0.12 * (grain + 0.5));
    } else if (k == LAVA) {
        // Basalt, black and charcoal, each plate cooled its own while
        // (browner the older), pale ash and sulphur on the heights; the
        // seams between dull red by day (their glow is the night map's). The
        // magma seas are crusted in floes, molten rock showing between them.
        let kept = stored_crust(c.uv * gen.size.xy);
        let crust_at = kept.xyz;
        let tone = smoothstep(-0.3, 0.4, fbm(p * 14.0 + o[2].xyz, 4));
        var crust = mix(srgb(18.0, 16.0, 15.0), srgb(50.0, 45.0, 42.0), tone);
        crust = mix(crust, srgb(74.0, 60.0, 50.0), crust_at.y * 0.4);
        let ash = smoothstep(0.12, 0.35, c.e) * smoothstep(0.1, 0.6, fbm(p * 6.0 + o[3].xyz, 3));
        crust = mix(crust, srgb(124.0, 112.0, 84.0), ash * 0.4);
        // Seams thin and mostly dull: dark red-brown, only the hot ones
        // stand out.
        let seam = (1.0 - smoothstep(0.0, 0.02, crust_at.x)) * (0.15 + 0.85 * crust_at.z * crust_at.z);
        let flows = kept.w;
        let ground = mix(crust, srgb(80.0, 22.0, 9.0), clamp(seam * 0.55 + flows * 0.5, 0.0, 1.0));
        // The seas: crusted floes, dull molten rock in the narrow gaps.
        let floes = (1.0 - smoothstep(0.005, 0.03, crust_at.x * (0.5 + crust_at.y))) * (0.3 + 0.7 * crust_at.z);
        let sea = mix(crust * 0.75, srgb(110.0, 28.0, 9.0), floes * 0.7);
        return mix(ground * (0.85 + 0.3 * (grain + 0.5)), sea, low);
    } else if (k == VOLCANIC) {
        // Sulphur yellow and orange, white frost, black calderas with red rims.
        var ground = mix(pal(0), pal(1), smoothstep(-0.1, 0.35, fbm(p * 5.0 + o[0].xyz, 5)));
        ground = mix(ground, pal(2), smoothstep(0.25, 0.5, fbm(p * 9.0 + o[1].xyz, 4)) * 0.6);
        let spot = smoothstep(0.1, 0.5, feature.x + feature.y);
        ground = mix(ground, pal(3), spot * 0.85);
        ground = mix(ground, srgb(170.0, 60.0, 30.0), smoothstep(0.0, 0.3, feature.x) * (1.0 - spot) * 0.5);
        return ground * (0.9 + 0.2 * (grain + 0.5));
    } else if (k == ICY_MOON) {
        let lines = max(cracks(p * 6.0 + o[0].xyz, 0.03), max(cracks(p * 14.0 + o[1].xyz, 0.025) * 0.8, cracks(p * 31.0 + o[2].xyz, 0.02) * 0.5));
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 4.0 + o[3].xyz, 4)));
        ground = mix(ground, pal(1), lines * 0.75);
        return mix(ground, pal(3), smoothstep(0.2, 0.5, fbm(p * 3.0 + o[0].zyx, 3)) * 0.3);
    } else if (k == GEYSER) {
        // Bright fresh ice, and blue fissures across the south pole.
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 5.0 + o[0].xyz, 4)));
        let lon = atan2(p.x, p.z);
        let stripes = 1.0 - smoothstep(0.0, 0.05, abs(sin(lon * 2.0 + p.y * 3.0 + 0.4 * noise(p * 6.0 + o[1].xyz))));
        let south = smoothstep(-0.55, -0.8, p.y);
        ground = mix(ground, pal(1), stripes * south * 0.85);
        return mix(ground, pal(3), cracks(p * 8.0 + o[2].xyz, 0.02) * 0.4 * (1.0 - south));
    } else if (k == TWO_FACED) {
        // A dark leading hemisphere with a ragged edge, a bright trailing one.
        let dark = 1.0 - smoothstep(-0.12, 0.12, p.z + 0.3 * fbm(p * 3.0 + o[0].xyz, 5));
        var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 8.0 + o[1].xyz, 4)) * 0.5);
        ground = mix(ground, mix(pal(1), pal(3), smoothstep(-0.3, 0.3, fbm(p * 6.0 + o[2].xyz, 4))), dark);
        return ground * (0.9 + 0.2 * (grain + 0.5));
    } else if (k == NITROGEN) {
        // Cantaloupe terrain: the craters' dimples; dark wind streaks; a bright
        // pinkish polar cap.
        var ground = mix(pal(0), pal(1), smoothstep(-0.2, 0.2, feature.y + 0.3 * fbm(p * 7.0 + o[0].xyz, 4)) * 0.5);
        let streak = smoothstep(0.35, 0.6, fbm(vec3<f32>(p.x * 14.0, p.y * 3.0, p.z * 14.0) + o[1].xyz, 4));
        ground = mix(ground, pal(3), streak * 0.5 * smoothstep(-0.2, 0.3, p.y));
        return mix(ground, pal(2), smoothstep(-0.25, -0.55, p.y + 0.1 * fbm(p * 5.0, 3)));
    }
    // Barren, captured and rusty rock: dark lowland plains, bright young
    // craters' rays.
    var ground = mix(pal(0), pal(2), smoothstep(-0.2, 0.4, fbm(p * 8.0 + o[0].xyz, 4)) * 0.5);
    ground = mix(ground, pal(1), low * 0.75);
    ground = mix(ground, pal(2), clamp(feature.x, 0.0, 1.0) * 0.5);
    if (kind() == CAPTURED) {
        ground *= 0.55;
    }
    return ground * (0.85 + 0.3 * (grain + 0.5));
}

// The rocky kind a day pipeline is built for: one each from `GROUP` 4 to 11,
// so the compiler drops every other kind's colours; 12 is barren, captured and
// rusty rock, which share theirs (their palettes differ, and captured rock is
// darkened).
fn rocky_kind() -> i32 {
    switch (GROUP) {
        case 4: { return DESERT; }
        case 5: { return ICE; }
        case 6: { return LAVA; }
        case 7: { return VOLCANIC; }
        case 8: { return ICY_MOON; }
        case 9: { return GEYSER; }
        case 10: { return TWO_FACED; }
        case 11: { return NITROGEN; }
        default: { return BARREN; }
    }
}

// The day map, one pipeline per family (`GROUP`): 0 giants, 1 cloud-wrapped
// worlds, 2 hazy moons, 3 living worlds, 4 and up the rocky kinds.
@fragment
fn fs_day(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    let p = sphere_at(in.position.xy / gen.size.xy);
    if (GROUP == 0) {
        return vec4<f32>(giant(p, k).day, 1.0);
    }
    if (GROUP == 1) {
        return vec4<f32>(cloudy(p).day, 1.0);
    }
    if (GROUP == 2) {
        let o = gen.offsets;
        let tone = smoothstep(-0.3, 0.3, fbm(vec3<f32>(p.x, p.y * 2.5, p.z) * 3.0 + o[0].xyz, 5));
        let day = mix(pal(0), pal(1), tone * 0.35);
        return vec4<f32>(mix(day, pal(3), smoothstep(0.75, 0.95, abs(p.y)) * 0.4), 1.0);
    }
    let c = climate(in.position.xy);
    let steep = steepness(in.position.xy);
    if (GROUP == 3) {
        return vec4<f32>(living_day(c, steep), 1.0);
    }
    return vec4<f32>(rocky_day(rocky_kind(), c, steep, features(in.position.xy)), 1.0);
}

// How likely people are to live here.
fn liveable(position: vec2<f32>) -> f32 {
    let c = climate(position);
    return c.land
        * smoothstep(0.30, 0.52, c.temperature) * (1.0 - smoothstep(0.30, 0.50, c.e))
        * smoothstep(0.10, 0.35, c.moisture) * (0.45 + 0.9 * c.near_coast)
        * smoothstep(-0.25, 0.20, fbm(c.p * 3.0 + gen.offsets[3].xzy, 3))
        * (1.0 - smoothstep(0.08, 0.25, steepness(position)));
}

// The night map, into the red channel, one pipeline per family (`GROUP`):
// 0 cities on living worlds, 1 molten rock's glow on lava worlds, 2 a hot
// Jupiter's own heat, 3 nothing (it only clears the channel).
@fragment
fn fs_lights(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    let p = sphere_at(in.position.xy / gen.size.xy);
    var light = 0.0;
    if (GROUP == 0 && gen.flags.y > 0.5) {
        let live = liveable(in.position.xy);
        if (live > 0.01) {
            light = live * settlements(p, 120.0, 0.7, 11) * 1.8;
        }
    } else if (GROUP == 1 || GROUP == 4) {
        // Molten seas and cracks: the broad cracks and the seas here, the fine
        // cracks in a second pipeline (`GROUP` 4) blended in by maximum, as
        // the one pass took the larger of the two.
        let o = gen.offsets;
        let c = climate(in.position.xy);
        var molten = 0.0;
        let kept = stored_crust(in.position.xy);
        let crust_at = kept.xyz;
        if (GROUP == 1) {
            // The seas between their floes, hottest where the gaps are
            // widest; and the flows from the vents.
            let gap = 1.0 - smoothstep(0.0, 0.035, crust_at.x * (0.5 + crust_at.y));
            let sea = (1.0 - c.land) * (0.06 + 0.7 * gap * crust_at.z * crust_at.z);
            molten = max(sea, kept.w * c.land * 0.85);
        } else {
            // The seams between the plates on land, each as hot as it is.
            // Most seams have nearly crusted over: a faint red; a few run
            // hot. Hairlines finer still, fainter.
            let seam = (1.0 - smoothstep(0.0, 0.016, crust_at.x)) * (0.04 + 0.85 * crust_at.z * crust_at.z * crust_at.z);
            let hair = (1.0 - smoothstep(0.0, 0.02, cell_edges(p * 70.0 + o[3].xyz, 17).x)) * 0.08 * crust_at.z;
            molten = max(seam, hair) * c.land;
        }
        light = molten * (0.75 + 0.25 * fbm(p * 30.0, 3));
    } else if (GROUP == 2) {
        light = giant(p, k).night;
    }
    return vec4<f32>(clamp(light, 0.0, 1.0), 0.0, 0.0, 0.0);
}

// The towns between the cities, added to the red channel.
@fragment
fn fs_towns(in: Varying) -> @location(0) vec4<f32> {
    var light = 0.0;
    if (living(kind()) && gen.flags.y > 0.5) {
        let live = liveable(in.position.xy);
        if (live > 0.01) {
            light = live * settlements(sphere_at(in.position.xy / gen.size.xy), 420.0, 0.6, 29) * 0.7;
        }
    }
    return vec4<f32>(clamp(light, 0.0, 1.0), 0.0, 0.0, 0.0);
}

// The cloud sheet, into the green channel, one pipeline per family
// (`GROUP`): 0 weather on living worlds, 1 dust on deserts and thin haze on
// ice worlds (and nothing for the rest).
@fragment
fn fs_clouds(in: Varying) -> @location(0) vec4<f32> {
    let k = kind();
    let o = gen.offsets;
    let p = sphere_at(in.position.xy / gen.size.xy);
    var cloud = 0.0;
    if (GROUP == 0) {
        let cl = climate(in.position.xy);
        var c = p;
        for (var i = 0; i < 12; i++) {
            let storm = gen.storms[i];
            let d = distance(c, storm.xyz);
            c = turn_about(c, storm.xyz, storm.w * exp(-d * d / (0.22 * 0.22)));
        }
        let stretched = vec3<f32>(c.x, c.y * 1.9, c.z);
        let cwarp = vec3<f32>(
            fbm(stretched * 2.0 + o[1].xyz, 3),
            fbm(stretched * 2.0 + o[2].xyz, 3),
            fbm(stretched * 2.0 + o[3].xyz, 3),
        );
        let sheet = fbm(stretched * 3.5 + cwarp * 0.9 + o[0].xyz, 8);
        let itcz = exp(-pow((cl.lat * 57.2958 - 4.0) / 7.0, 2.0));
        let more = select(0.0, 0.08, k == OCEAN) + select(0.0, 0.06, k == JUNGLE);
        var bands = 0.10 * cos(6.0 * cl.lat) + 0.12 * itcz;
        if (k == EYEBALL) {
            // One great storm system boils up under the star; the far side is
            // too cold for cloud.
            bands = 0.35 * smoothstep(0.55, 1.0, p.z) - 0.3 * (1.0 - smoothstep(-0.2, 0.3, p.z));
        }
        let cover = sheet + bands + 0.16 * (cl.moisture - 0.5) * cl.land - 0.10 + more;
        cloud = smoothstep(0.0, 0.42, cover);
        cloud *= smoothstep(-0.45, 0.25, fbm(c * 26.0 + o[3].yzx, 4) + cloud * 0.6);
        // Billows within the deck: brighter tops, thinner gaps between.
        let billow = 1.0 - 2.0 * abs(fbm(c * 30.0 + o[0].zxy, 4));
        cloud *= mix(1.0, 0.78 + 0.22 * billow, smoothstep(0.3, 0.9, cloud));
    } else if (k == DESERT) {
        cloud = smoothstep(0.35, 0.6, fbm(vec3<f32>(p.x, p.y * 2.0, p.z) * 4.0 + o[3].xyz, 5)) * 0.25;
    } else if (k == ICE) {
        cloud = smoothstep(0.25, 0.6, fbm(vec3<f32>(p.x, p.y * 1.6, p.z) * 3.5 + o[3].xyz, 6)) * 0.5;
    }
    return vec4<f32>(0.0, clamp(cloud, 0.0, 1.0), 0.0, 0.0);
}

// A volcanic moon's glowing calderas, into its relief's third channel.
@fragment
fn fs_glow(in: Varying) -> @location(0) vec4<f32> {
    var glow = 0.0;
    if (kind() == VOLCANIC) {
        let p = sphere_at(in.position.xy / gen.size.xy);
        let f = features(in.position.xy);
        let spot = smoothstep(0.1, 0.5, f.x + f.y);
        glow = spot * smoothstep(0.6, 0.9, hash3(vec3<i32>(floor(p * 9.0)))) * 0.8;
    }
    return vec4<f32>(0.0, 0.0, glow, 0.0);
}

// The sky: the galaxy's band along a great circle (its pole in palette[0]),
// dust lanes, and faint glowing clouds.
@fragment
fn fs_sky(in: Varying) -> @location(0) vec4<f32> {
    let p = sphere_at(in.position.xy / gen.size.xy);
    let pole = gen.palette[0].xyz;
    let o = gen.offsets;
    let off = dot(p, pole);
    let warp = fbm(p * 3.0 + o[0].xyz, 4) * 0.12;
    let band = exp(-pow((off + warp) / 0.16, 2.0));
    let core = exp(-pow((off + warp) / 0.05, 2.0));
    let dust = smoothstep(-0.1, 0.35, fbm(p * 9.0 + o[1].xyz, 6));
    let glow = fbm(p * 5.0 + o[2].xyz, 5) * 0.5 + 0.5;
    var sky = vec3<f32>(0.55, 0.52, 0.62) * band * (0.25 + 0.75 * glow) * 0.05;
    sky += vec3<f32>(0.85, 0.72, 0.55) * core * 0.04 * glow;
    sky *= 1.0 - dust * 0.75 * band;
    // A few faint nebulae, tinted.
    let cloud = smoothstep(0.25, 0.65, fbm(p * 2.2 + o[3].xyz, 5));
    let tint = mix(vec3<f32>(0.6, 0.2, 0.35), vec3<f32>(0.2, 0.35, 0.6), fbm(p * 1.3 + o[0].zyx, 3) + 0.5);
    sky += tint * cloud * 0.012;
    return vec4<f32>(sky, 1.0);
}
