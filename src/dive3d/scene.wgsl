// The 3D dive.
//
// The camera sits at the salon window looking out sideways (+z), level, with
// the Nautilus travelling along +x; the sea's surface is at y = 0 and the boat
// moves under it, turning the view with its pitch, roll and heading (see
// `helm.rs`). Everything here is ordinary rasterised
// geometry with a depth buffer: a background pass for the open water, the
// seabed lifted from a grid in the vertex shader, instanced models placed and
// animated by Rust, translucent jellyfish, and camera-facing sprites for snow,
// bubbles and glows. The porthole, its gauges and the bending of the glass are
// drawn by `dive.wgsl`'s bezel and composite passes, as for the procedural dive.
//
// Biomes, terrain height and the noise behind them mirror `dive3d/world.rs`
// exactly, so things Rust places on the seabed stand on it.

struct Globals {
    // xyz: camera position; w: seconds.
    camera: vec4<f32>,
    // x: half the view's extent at unit distance; y: near plane.
    view: vec4<f32>,
    // The window's right, up and outward directions in the world.
    right: vec4<f32>,
    up: vec4<f32>,
    out: vec4<f32>,
    // xyz: where the searchlight is mounted.
    lamp_origin: vec4<f32>,
    // xyz: toward the sun; w: its strength at the window.
    sun: vec4<f32>,
    // rgb: water looking level; w: attenuation per unit of distance.
    water: vec4<f32>,
    // rgb: water looking down; w: depth of the window below the surface.
    deep: vec4<f32>,
    // xyz: the searchlight's direction; w: its strength.
    lamp: vec4<f32>,
    // x: surface height; y: the cave vault's; z: the ice's.
    levels: vec4<f32>,
    // x: world seed; y: lava glow; z: share of the lava cave; w: share of ice.
    world: vec4<f32>,
    // x: point lights in use.
    counts: vec4<f32>,
    lights: array<vec4<f32>, 8>,
    light_colours: array<vec4<f32>, 8>,
};

@group(0) @binding(0) var<uniform> g: Globals;
// The seabed's relief: height and its slopes, from `heightmap.rs`.
@group(0) @binding(1) var relief_map: texture_2d<f32>;
// The surfaces, one layer each, from `textures.rs`.
@group(0) @binding(2) var surfaces: texture_2d_array<f32>;
@group(0) @binding(3) var surface_sampler: sampler;

const RELIEF_SIZE: i32 = 512;
// The lava cave's tunnel and the massif it runs through, as in `world.rs`.
const TUNNEL: f32 = 1.05;
const MASSIF: f32 = 4.5;
const RELIEF_PERIOD: f32 = 64.0;
// Textures store 0..1.6 as 0..1, so they can brighten as well as darken.
const TEXTURE_RANGE: f32 = 1.6;

// Whether this models pipeline draws things that are fading in or out. Only
// that one may discard fragments: the Pi's tile-based GPU skips hidden
// fragments before shading them only when a shader cannot discard, so every
// model that is fully there goes through a pipeline compiled without it.
override FADING: bool = true;

// The material every model this pipeline draws is made of, or -1 to take it
// from each instance. A pipeline per material compiles only that material's
// shading: with all of them in, the shader is too large for the Pi's GPU to
// keep more than two of its threads at once, and every pixel pays for it.
override MATERIAL: i32 = -1;

// What every colour is multiplied by as it is stored: the sea is kept in ten
// bits a channel, which hold no more than 1, so it is stored at half its
// brightness, to keep the highlights up to 2 that the porthole's grade and
// its bending of the glass blend from.
override SEA_SCALE: f32 = 1.0;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const LEG: f32 = 3.4;
const BLEND: f32 = 0.32;
const BIOMES: i32 = 7;
const SHALLOWS: i32 = 0;
const FOREST: i32 = 1;
const WRECKS: i32 = 2;
const ATLANTIS: i32 = 3;
const CAVE: i32 = 4;
const ABYSS: i32 = 5;
const ARCTIC: i32 = 6;

// ---------------------------------------------------------------------------
// Shared with world.rs
// ---------------------------------------------------------------------------

fn hash(ix: i32, iz: i32, salt: u32) -> f32 {
    var h = (bitcast<u32>(ix) * 0x8da6b343u) ^ (bitcast<u32>(iz) * 0xd8163841u) ^ (salt * 0xcb1ab31fu);
    h = (h ^ (h >> 13u)) * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0x00ffffffu) / 16777216.0;
}

// Switches the `frame_cost` benchmark sets in `counts.y`, to find what each
// kind of work costs. Always zero on screen.
const PLAIN_TEXTURES: u32 = 1u;
const NO_NOISE: u32 = 2u;

fn debug(flag: u32) -> bool {
    return (u32(g.counts.y) & flag) != 0u;
}

fn vnoise(x: f32, z: f32, salt: u32) -> f32 {
    if (debug(NO_NOISE)) {
        return 0.5;
    }
    let fx = floor(x);
    let fz = floor(z);
    let ix = i32(fx);
    let iz = i32(fz);
    let tx = x - fx;
    let tz = z - fz;
    let ux = tx * tx * (3.0 - 2.0 * tx);
    let uz = tz * tz * (3.0 - 2.0 * tz);
    let a = hash(ix, iz, salt);
    let b = hash(ix + 1, iz, salt);
    let c = hash(ix, iz + 1, salt);
    let d = hash(ix + 1, iz + 1, salt);
    return mix(mix(a, b, ux), mix(c, d, ux), uz);
}

fn smooth01(a: f32, b: f32, v: f32) -> f32 {
    let t = clamp((v - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Where along the voyage a point at (x, z) belongs. Biomes change along the
// voyage, but their borders wander with distance from its corridor, so they
// never run as straight lines to the vanishing point.
// The line the voyage follows across the seabed, as `world::corridor`.
fn corridor(x: f32) -> f32 {
    return 2.2 * sin(x * 0.11 + 0.7) + 0.8 * sin(x * 0.031);
}

fn voyage_x(x: f32, z: f32) -> f32 {
    let off = z - corridor(x);
    return x + 1.2 * sin(off * 0.31) + 0.8 * (sin(off * 0.73 + 1.3) - sin(1.3));
}

struct Leg {
    a: i32,
    b: i32,
    w: f32,
};

fn leg(x: f32) -> Leg {
    let u = x / LEG;
    let k = floor(u);
    let start = i32(g.world.x * f32(BIOMES));
    let a = ((i32(k) + start) % BIOMES + BIOMES) % BIOMES;
    let b = ((i32(k) + 1 + start) % BIOMES + BIOMES) % BIOMES;
    return Leg(a, b, smooth01(1.0 - BLEND, 1.0, u - k));
}

fn presence(l: Leg, id: i32) -> f32 {
    return select(0.0, 1.0 - l.w, l.a == id) + select(0.0, l.w, l.b == id);
}

fn by_biome(l: Leg, values: array<f32, 7>) -> f32 {
    var v = values;
    return mix(v[l.a], v[l.b], l.w);
}

// The heightmap at (x, z), bilinear, exactly as `heightmap::sample`.
fn relief(x: f32, z: f32) -> vec3<f32> {
    let u = x / RELIEF_PERIOD * f32(RELIEF_SIZE);
    let v = z / RELIEF_PERIOD * f32(RELIEF_SIZE);
    let fu = floor(u);
    let fv = floor(v);
    let i = i32(fu);
    let j = i32(fv);
    let n = RELIEF_SIZE;
    let a = textureLoad(relief_map, vec2<i32>(((i % n) + n) % n, ((j % n) + n) % n), 0).xyz;
    let b = textureLoad(relief_map, vec2<i32>((((i + 1) % n) + n) % n, ((j % n) + n) % n), 0).xyz;
    let c = textureLoad(relief_map, vec2<i32>(((i % n) + n) % n, (((j + 1) % n) + n) % n), 0).xyz;
    let d = textureLoad(relief_map, vec2<i32>((((i + 1) % n) + n) % n, (((j + 1) % n) + n) % n), 0).xyz;
    let tu = u - fu;
    let tv = v - fv;
    return mix(mix(a, b, tu), mix(c, d, tu), tv);
}

// The seabed's height, exactly as `Seabed::ground`: the biome's depth below
// the surface, the heightmap's relief, fine roughness, and mountains far off.
fn ground(x: f32, z: f32) -> f32 {
    let l = leg(voyage_x(x, z));
    let base = -by_biome(l, array<f32, 7>(1.7, 3.6, 4.9, 5.8, 6.6, 9.3, 3.5));
    let lift = by_biome(l, array<f32, 7>(0.25, 0.35, 0.35, 0.12, 0.20, 1.20, 0.40)) * relief(x, z).x;
    let rough = by_biome(l, array<f32, 7>(0.05, 0.07, 0.05, 0.02, 0.03, 0.08, 0.05));
    let ridges = by_biome(l, array<f32, 7>(1.6, 2.2, 2.0, 2.6, 3.5, 5.0, 1.4));
    let n = 0.65 * vnoise(x * 1.3, z * 1.3, 1u) + 0.35 * vnoise(x * 3.1 + 5.0, z * 3.1, 2u);
    let off = abs(z - corridor(x));
    let far = smooth01(7.0, 22.0, off) * (0.3 + ridges * vnoise(x * 0.16, z * 0.16 + 3.0, 4u));
    // The lava cave's massif, rising either side of its tunnel.
    let massif = presence(l, CAVE) * MASSIF * smooth01(TUNNEL * 1.05, TUNNEL * 1.8, off)
        * (0.85 + 0.3 * vnoise(x * 0.4, z * 0.4, 5u));
    return base + lift + rough * (n - 0.45) + far + massif;
}

// ---------------------------------------------------------------------------
// Light and water
// ---------------------------------------------------------------------------

fn project(world: vec3<f32>) -> vec4<f32> {
    let v = world - g.camera.xyz;
    let x = dot(g.right.xyz, v);
    let y = dot(g.up.xyz, v);
    let z = dot(g.out.xyz, v);
    // Reversed depth: the near plane at 1, the far distance toward 0.
    return vec4<f32>(x / g.view.x, y / g.view.x, g.view.y, z);
}

fn caustic(p: vec2<f32>, t: f32) -> f32 {
    let a = sin(p.x + sin(p.y * 1.3 + t));
    let b = sin(p.y * 1.1 + sin(p.x * 0.9 - t * 0.7));
    return pow(max(1.0 - abs(a + b) * 0.5, 0.0), 7.0);
}

// The searchlight, mounted above the window and thrown forward and down.
fn lamp_origin() -> vec3<f32> {
    return g.lamp_origin.xyz;
}

// Open water seen along `dir`: lighter looking up toward the surface, darker
// looking down, with the searchlight's beam glowing in it.
fn water_at(dir: vec3<f32>) -> vec3<f32> {
    var c = mix(g.deep.rgb, g.water.rgb, smooth01(-0.6, 0.5, dir.y));
    c *= 1.0 + 0.6 * g.sun.w * smooth01(0.0, 0.6, dir.y);
    let beam = smooth01(0.80, 0.98, dot(dir, g.lamp.xyz));
    c += vec3<f32>(0.55, 0.52, 0.42) * beam * g.lamp.w * 0.10;
    return c;
}

fn fog_of(distance: f32) -> f32 {
    return 1.0 - exp(-distance * g.water.w);
}

// Sun, sky, searchlight and point lights on a surface at `p` facing `n`.
fn light_at(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let below = max(g.levels.x - p.y, 0.0);
    let sun = g.sun.w * exp(-below * 0.22);
    var light = vec3<f32>(0.55, 0.92, 0.86) * sun * max(dot(n, g.sun.xyz) * 0.75 + 0.25, 0.0) * 1.5;
    // Light from the water all round, more from above.
    light += vec3<f32>(0.05, 0.10, 0.12) * (0.35 + 0.65 * (n.y * 0.5 + 0.5)) * (sun * 2.0 + 0.25);
    if (sun > 0.03 && n.y > 0.2) {
        light *= 1.0 + caustic(p.xz * vec2<f32>(7.0, 9.0) + vec2<f32>(g.camera.w * 0.3, 0.0), g.camera.w * 0.9) * 0.9 * n.y * sun;
    }
    let to = p - lamp_origin();
    let d = length(to);
    let cone = smooth01(0.82, 0.95, dot(to / d, g.lamp.xyz));
    light += vec3<f32>(1.0, 0.92, 0.76) * g.lamp.w * cone * (max(dot(n, -to / d), 0.0) * 0.8 + 0.2) / (1.0 + d * d * 0.05) * 2.2;
    let count = i32(g.counts.x);
    for (var i = 0; i < 8; i = i + 1) {
        if (i >= count) {
            break;
        }
        let lv = g.lights[i].xyz - p;
        let r = g.lights[i].w;
        let d2 = dot(lv, lv);
        // Beyond four ranges a light's falloff is exactly zero.
        if (d2 >= 16.0 * r * r) {
            continue;
        }
        let fall = 1.0 / (1.0 + d2 / (r * r)) * (1.0 - smooth01(r * 2.0, r * 4.0, sqrt(d2)));
        light += g.light_colours[i].rgb * fall * (max(dot(n, lv * inverseSqrt(max(d2, 1e-6))), 0.0) * 0.8 + 0.2);
    }
    return light;
}

// The water seen towards `p`, and how much of whatever is at `p` it hides.
// Worked out per vertex for the seabed and the models: it changes slowly
// across a triangle, and the Pi's GPU is short of arithmetic per pixel.
fn fog_at(p: vec3<f32>) -> vec4<f32> {
    let v = p - g.camera.xyz;
    let d = length(v);
    return vec4<f32>(water_at(v / d), fog_of(d));
}

// ---------------------------------------------------------------------------
// Background
// ---------------------------------------------------------------------------

struct Screen {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex
fn vs_background(@builtin(vertex_index) index: u32) -> Screen {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Screen;
    // At the far plane of the reversed depth buffer: drawn after everything
    // solid, it only shades the pixels nothing else covers.
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    out.ndc = corners[index];
    return out;
}

// The corners of the square the sea is drawn into lie outside the round
// window: the porthole samples no further out than the glass's edge, bent a
// little outward (`porthole.wgsl`), just past the square's sides. A ring from
// there out past the corners, at the nearest depth, is drawn before anything
// else, so nothing behind it is shaded.
const MASK_SEGMENTS: u32 = 64u;
const MASK_INNER: f32 = 1.03;

@vertex
fn vs_mask(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index % 6u];
    let a = (f32(index / 6u) + c.x) * TAU / f32(MASK_SEGMENTS);
    // The inner edge's chords stay outside the circle they cut across.
    let r = select(MASK_INNER / cos(PI / f32(MASK_SEGMENTS)), 2.0, c.y > 0.5);
    return vec4<f32>(cos(a) * r, sin(a) * r, 1.0, 1.0);
}

@fragment
fn fs_mask() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

@fragment
fn fs_background(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    // The boat's attitude turns the view; the water stays level.
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    let x = s.x;
    let y = dir.y / max(dir.z, 0.2);
    var c = water_at(dir);
    // Sunbeams slanting down through the middle distance.
    if (g.sun.w > 0.02) {
        let down = max(0.40 - y, 0.0);
        let slant = (x + g.camera.x / 3.0 + down * 0.58) * 12.0 + 1.4 * sin(x * 3.1 + g.camera.w * 0.08);
        let ray = pow(max(sin(slant + 1.1 * sin(down * 2.3 + g.camera.w * 0.13)), 0.0), 6.0) * 0.6
            + pow(max(sin(slant * 1.73 + 2.1), 0.0), 12.0) * 0.4;
        c += vec3<f32>(0.13, 0.39, 0.37) * ray * exp(-down * 1.6) * 0.85 * g.sun.w;
    }
    return vec4<f32>(c * SEA_SCALE, 1.0);
}

// ---------------------------------------------------------------------------
// Seabed
// ---------------------------------------------------------------------------

struct Ground {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // `fog_at` the vertex.
    @location(2) fog: vec4<f32>,
};

@vertex
fn vs_terrain(@location(0) world: vec3<f32>, @location(1) normal: vec3<f32>) -> Ground {
    // Built on the CPU a tile at a time, already in the world (`terrain.rs`).
    var out: Ground;
    out.world = world;
    out.normal = normal;
    out.position = project(world);
    out.fog = fog_at(world);
    return out;
}

fn floor_albedo(l: Leg) -> vec3<f32> {
    var albedos = array<vec3<f32>, 7>(
        vec3<f32>(0.62, 0.56, 0.42),
        vec3<f32>(0.40, 0.36, 0.26),
        vec3<f32>(0.32, 0.30, 0.26),
        vec3<f32>(0.46, 0.44, 0.39),
        vec3<f32>(0.10, 0.09, 0.09),
        vec3<f32>(0.17, 0.16, 0.17),
        vec3<f32>(0.36, 0.37, 0.38),
    );
    return mix(albedos[l.a], albedos[l.b], l.w);
}

fn surface_of(l: Leg) -> vec2<i32> {
    // Sand, sand, silt, sand, basalt, silt, sand: by biome.
    var layers = array<i32, 7>(12, 12, 13, 12, 10, 13, 12);
    return vec2<i32>(layers[l.a], layers[l.b]);
}

@fragment
fn fs_terrain(in: Ground) -> @location(0) vec4<f32> {
    let p = in.world;
    let l = leg(voyage_x(p.x, p.z));
    let n = normalize(in.normal);
    // Sampled first, while every pixel is still doing the same thing.
    let layers = surface_of(l);
    // Projected along all three axes: the floor's own material on the level,
    // rock on whichever vertical faces a slope turns toward, so nothing smears.
    let w = facing_weights(n);
    let at = p * 1.3;
    let dx = dpdx(at);
    let dy = dpdy(at);
    // The two biomes' floors are blended only while one sea gives way to the
    // next; the rest of the time one of them has no weight and is skipped.
    let flat_a = tap(at.xz, dx.xz, dy.xz, layers.x, w.y * (1.0 - l.w));
    let flat_b = tap(at.xz, dx.xz, dy.xz, layers.y, w.y * l.w);
    let side_x = tap(at.zy, dx.zy, dy.zy, 1, w.x);
    let side_z = tap(at.xy, dx.xy, dy.xy, 1, w.z);
    let steep = smooth01(0.12, 0.40, 1.0 - n.y);
    let texture = (flat_a + flat_b + side_x + side_z) * TEXTURE_RANGE;
    var albedo = floor_albedo(l) * mix(vec3<f32>(1.0), texture, 0.85);
    albedo = mix(albedo, vec3<f32>(0.22, 0.21, 0.20) * texture, steep * 0.6);
    let distance = length(p - g.camera.xyz);
    let detail = exp(-distance * 0.25);
    albedo *= mix(1.0, 0.65 + 0.5 * vnoise(p.x * 14.0, p.z * 14.0, 9u), detail * (1.0 - steep));
    let sand = presence(l, SHALLOWS) + 0.5 * presence(l, FOREST) + 0.5 * presence(l, ARCTIC);
    if (sand > 0.0) {
        let ripple = sin((p.x * 0.8 + p.z) * 26.0 + vnoise(p.x * 3.0, p.z * 3.0, 5u) * 4.0);
        albedo *= 1.0 - 0.16 * sand * detail * smooth01(0.3, 1.0, ripple);
    }
    let paving = presence(l, ATLANTIS);
    if (paving > 0.0) {
        let row = floor(p.z * 4.0);
        let joint = min(abs(fract(p.x * 3.0 + row * 0.5) - 0.5), abs(fract(p.z * 4.0) - 0.5));
        albedo *= 1.0 - paving * 0.45 * smooth01(0.45, 0.5, joint) * detail;
    }
    var colour = albedo * light_at(p, n);
    // The lava cave's floor: a crust of basalt over molten rock.
    let cave = presence(l, CAVE);
    if (cave > 0.0) {
        let flicker = 0.80 + 0.20 * sin(g.camera.w * 1.7 + p.x * 3.0) * sin(g.camera.w * 2.9 + p.z * 2.0);
        let veins = 1.0 - smooth01(0.0, 0.04, abs(vnoise(p.x * 1.4, p.z * 1.9, 6u) - 0.5));
        let fine = (1.0 - smooth01(0.0, 0.03, abs(vnoise(p.x * 4.0, p.z * 5.0, 7u) - 0.5))) * detail;
        let pools = smooth01(0.66, 0.74, vnoise(p.x * 0.55 + 7.0, p.z * 0.55, 8u));
        // Lava lies on the level; on a slope it would only smear.
        colour += vec3<f32>(1.0, 0.36, 0.07) * (veins * 0.9 + fine * 0.5 + pools * 1.4) * flicker * cave * (1.0 - steep);
    }
    return vec4<f32>(mix(colour, in.fog.rgb, in.fog.a) * SEA_SCALE, 1.0);
}

// ---------------------------------------------------------------------------
// Models
// ---------------------------------------------------------------------------

struct Model {
    @location(0) pos: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec3<f32>,
    @location(3) aux: vec2<f32>,
};

struct Instance {
    @location(4) m0: vec4<f32>,
    @location(5) m1: vec4<f32>,
    @location(6) m2: vec4<f32>,
    // rgb: albedo; a: how much it glows by itself.
    @location(7) colour: vec4<f32>,
    // x: animation; y: phase; z: amplitude; w: frequency.
    @location(8) anim: vec4<f32>,
    // x: material; y: 1 if it stands on the seabed; z: how far it has faded in.
    @location(9) extra: vec4<f32>,
    // x: texture layer; y: repeats per unit; z: 1 for the model's own space.
    @location(10) surface: vec4<f32>,
};

struct Lit {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    @location(3) @interpolate(flat) glow: f32,
    @location(4) @interpolate(flat) material: u32,
    @location(5) @interpolate(flat) grounded: f32,
    @location(6) local: vec3<f32>,
    @location(7) local_normal: vec3<f32>,
    @location(8) @interpolate(flat) surface: vec4<f32>,
    @location(9) @interpolate(flat) fade: f32,
    // The seabed's height under the vertex, for bedding grounded models in
    // its sand: once per vertex rather than once per pixel.
    @location(10) floor_y: f32,
    // `fog_at` the vertex.
    @location(11) fog: vec4<f32>,
};

// Bend a model where it lives: plants sway, fish sweep their tails sideways
// and whales beat their flukes up and down, tentacles writhe and bells pulse.
fn animate(p_in: vec3<f32>, aux: vec2<f32>, anim: vec4<f32>) -> vec3<f32> {
    var p = p_in;
    let kind = u32(anim.x + 0.5);
    let t = g.camera.w * anim.w + anim.y;
    if (kind == 1u) {
        let up = aux.y * aux.y;
        p.x += anim.z * up * sin(t + aux.y * 2.5);
        p.z += anim.z * 0.6 * up * cos(t * 0.8 + aux.y * 2.0);
    } else if (kind == 2u || kind == 3u) {
        // A wave running down the body from head to tail, growing toward it.
        let back = (1.0 - aux.x) * 0.5;
        let bend = anim.z * back * back * sin(t - back * 4.4) + anim.z * 0.08 * sin(t);
        if (kind == 2u) {
            p.z += bend;
        } else {
            p.y += bend;
        }
    } else if (kind == 4u) {
        let along = aux.x;
        p.x += anim.z * along * sin(t - along * 5.0);
        p.z += anim.z * along * cos(t * 0.8 - along * 4.0 + 1.3);
    } else if (kind == 5u) {
        let beat = 0.5 + 0.5 * sin(t);
        let squeeze = 1.0 - anim.z * beat * (1.0 - aux.y * 0.6);
        p.x *= squeeze;
        p.z *= squeeze;
        p.y *= 1.0 + anim.z * 0.6 * beat;
    }
    return p;
}

@vertex
fn vs_model(v: Model, i: Instance) -> Lit {
    let p = animate(v.pos, v.aux, i.anim);
    let world = vec3<f32>(dot(i.m0, vec4<f32>(p, 1.0)), dot(i.m1, vec4<f32>(p, 1.0)), dot(i.m2, vec4<f32>(p, 1.0)));
    // Normals by the cofactor matrix, which is right for any scale.
    let c0 = vec3<f32>(i.m0.x, i.m1.x, i.m2.x);
    let c1 = vec3<f32>(i.m0.y, i.m1.y, i.m2.y);
    let c2 = vec3<f32>(i.m0.z, i.m1.z, i.m2.z);
    let n = cross(c1, c2) * v.normal.x + cross(c2, c0) * v.normal.y + cross(c0, c1) * v.normal.z;
    var out: Lit;
    out.position = project(world);
    out.world = world;
    out.normal = n;
    out.albedo = i.colour.rgb * v.colour;
    out.glow = i.colour.a;
    out.material = u32(i.extra.x + 0.5);
    out.grounded = i.extra.y;
    out.local = v.pos;
    out.local_normal = v.normal;
    out.surface = i.surface;
    out.fade = i.extra.z;
    out.fog = fog_at(world);
    out.floor_y = 0.0;
    if (i.extra.y > 0.5) {
        out.floor_y = ground(world.x, world.z);
    }
    return out;
}

// A texture projected along three axes and blended by the surface's facing,
// so no model needs texture coordinates.
// One projection of a texture, skipped when it carries no weight. The
// gradients are taken outside any branch, so filtering is exactly as a plain
// sample's would be.
fn tap(uv: vec2<f32>, ddx: vec2<f32>, ddy: vec2<f32>, layer: i32, weight: f32) -> vec3<f32> {
    if (weight <= 0.0) {
        return vec3<f32>(0.0);
    }
    if (debug(PLAIN_TEXTURES)) {
        return vec3<f32>(0.6 * weight);
    }
    return textureSampleGrad(surfaces, surface_sampler, uv, layer, ddx, ddy).rgb * weight;
}

// Weights for projecting along x, y and z by a surface's facing. A projection
// under 1% of the blend is dropped and the rest renormalised: it cannot be
// seen, and most pixels then need only one texture lookup instead of three.
fn facing_weights(facing: vec3<f32>) -> vec3<f32> {
    var w = pow(abs(facing), vec3<f32>(4.0));
    w /= max(w.x + w.y + w.z, 1e-4);
    w = select(w, vec3<f32>(0.0), w < vec3<f32>(0.01));
    return w / max(w.x + w.y + w.z, 1e-4);
}

fn triplanar(at: vec3<f32>, facing: vec3<f32>, layer: i32) -> vec3<f32> {
    let w = facing_weights(facing);
    let dx = dpdx(at);
    let dy = dpdy(at);
    let x = tap(at.zy, dx.zy, dy.zy, layer, w.x);
    let y = tap(at.xz, dx.xz, dy.xz, layer, w.y);
    let z = tap(at.xy, dx.xy, dy.xy, layer, w.z);
    return (x + y + z) * TEXTURE_RANGE;
}

@fragment
fn fs_model(in: Lit, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let p = in.world;
    // Things that move carry their texture; things that stay put wear it in
    // world space, at the same scale everywhere.
    let carried = in.surface.z > 0.5;
    let at = select(p, in.local, carried) * in.surface.y;
    let facing = select(in.normal, in.local_normal, carried);
    let texture = triplanar(at, normalize(facing + vec3<f32>(1e-5)), i32(in.surface.x + 0.5));
    var n = normalize(in.normal);
    let to_camera = normalize(g.camera.xyz - p);
    // Fins, sails and fronds are single sheets, lit from whichever side is seen.
    if (dot(n, to_camera) < 0.0) {
        n = -n;
    }
    var albedo = in.albedo * texture;
    // Bedded in the sand: the foot of anything on the seabed takes its colour.
    if (in.grounded > 0.5) {
        let floor_y = in.floor_y;
        let buried = 1.0 - smooth01(floor_y + 0.005, floor_y + 0.05, p.y);
        let l = leg(voyage_x(p.x, p.z));
        albedo = mix(albedo, floor_albedo(l) * 0.9, buried * 0.85);
    }
    var colour = albedo * light_at(p, n);
    colour += in.albedo * in.glow;
    let t = g.camera.w;
    // Fading things out by coverage, through the multisampling: they melt
    // away as their biome passes, rather than pop.
    var coverage = in.fade;
    switch (select(in.material, u32(MATERIAL), MATERIAL >= 0)) {
        // Pillow lava, molten through its cracks.
        case 1u: {
            let crack = 1.0 - smooth01(0.0, 0.05, abs(vnoise(in.local.x * 6.0 + in.local.y * 3.0, in.local.z * 6.0, 11u) - 0.5));
            colour += vec3<f32>(1.0, 0.38, 0.08) * crack * smooth01(0.3, -0.5, in.local.y) * (0.8 + 0.2 * sin(t * 2.0));
        }
        // Copper and brass: a hot specular highlight from the searchlight.
        case 2u: {
            let h = normalize(to_camera - g.lamp.xyz);
            colour += vec3<f32>(1.0, 0.75, 0.45) * pow(max(dot(n, h), 0.0), 24.0) * (0.3 + g.lamp.w);
        }
        // Gold, glittering.
        case 8u: {
            let sparkle = pow(fract(sin(dot(floor(p * 140.0), vec3<f32>(12.9, 78.2, 37.7)) + floor(t * 3.0)) * 43758.5), 20.0);
            colour += vec3<f32>(1.0, 0.85, 0.45) * sparkle * 2.0;
        }
        // The surface seen from below: a rippling silver ceiling.
        case 4u: {
            let ripple = sin(p.x * 9.0 + t * 0.7 + 1.5 * sin(p.z * 5.0 - t * 0.4)) * sin(p.z * 7.0 + t * 0.5 + 1.2 * sin(p.x * 4.0));
            let bright = caustic(p.xz * vec2<f32>(5.0, 6.0) + vec2<f32>(t * 0.2, -t * 0.1), t * 0.6);
            colour = (vec3<f32>(0.45, 0.78, 0.74) * (0.78 + 0.22 * ripple) + vec3<f32>(0.35, 0.45, 0.40) * bright) * g.sun.w * 1.3;
        }
        // The tunnel's rock, glowing red low down where the lava lights it.
        case 5u: {
            let rock = vnoise(p.x * 5.0, p.z * 5.0, 12u) * 0.6 + vnoise(p.y * 7.0 + p.x, p.z * 13.0, 13u) * 0.4;
            let above = p.y - ground(p.x, p.z);
            let glow = (1.0 - smooth01(0.0, 1.8, above)) * (0.85 + 0.15 * sin(t * 1.9 + p.x));
            colour = albedo * (0.55 + 0.7 * rock) * (light_at(p, n) + vec3<f32>(0.10, 0.06, 0.05));
            colour += vec3<f32>(0.45, 0.10, 0.02) * glow * (0.5 + 0.5 * smooth01(0.55, 0.80, rock));
            // Veins of molten rock run along the walls, so the arch reads in
            // perspective all the way up the tunnel.
            let course = vnoise(p.x * 1.1, p.y * 3.2 + p.z * 1.7, 17u);
            let vein = 1.0 - smooth01(0.010, 0.035, abs(course - 0.5));
            let pulse = 0.75 + 0.25 * sin(t * 1.3 + p.x * 2.0);
            colour += vec3<f32>(0.75, 0.24, 0.04) * vein * pulse * (0.35 + 0.65 * glow);
        }
        // The pack ice from below: floes glowing with daylight, open leads.
        case 6u: {
            let l = leg(voyage_x(p.x, p.z));
            coverage = smooth01(0.35, 0.65, presence(l, ARCTIC));
            let floes = vnoise(p.x * 0.9, p.z * 1.3, 14u);
            let lead = 1.0 - smooth01(0.02, 0.06, abs(vnoise(p.x * 0.35 + 3.0, p.z * 0.08, 15u) - 0.5));
            let bumps = vnoise(p.x * 6.0, p.z * 6.0, 16u);
            colour = vec3<f32>(0.30, 0.56, 0.64) * (0.65 + 0.35 * bumps + 0.45 * floes);
            colour = mix(colour * texture, vec3<f32>(0.70, 0.95, 1.00), lead) * (0.70 + 0.9 * g.sun.w);
        }
        default: {}
    }
    if (FADING) {
        if (coverage <= 0.01) {
            discard;
        }
    }
    return vec4<f32>(mix(colour, in.fog.rgb, in.fog.a) * SEA_SCALE, coverage);
}

// Translucent models — jellyfish bells and tentacles — add light, brightest at
// their rims, and fade with distance.
@fragment
fn fs_glass(in: Lit) -> @location(0) vec4<f32> {
    let p = in.world;
    let n = normalize(in.normal);
    let v = normalize(g.camera.xyz - p);
    let rim = 1.0 - abs(dot(n, v));
    let lit = light_at(p, v) * 0.25 + vec3<f32>(0.3);
    let glow = in.albedo * (0.15 + 0.85 * rim * rim) * lit + in.albedo * in.glow;
    return vec4<f32>(glow * in.fade * (1.0 - fog_of(length(p - g.camera.xyz))) * SEA_SCALE, 0.0);
}

// ---------------------------------------------------------------------------
// Sprites
// ---------------------------------------------------------------------------

struct Sprite {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) @interpolate(flat) kind: u32,
};

@vertex
fn vs_sprite(
    @builtin(vertex_index) index: u32,
    @location(0) place: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) stretch: vec4<f32>,
) -> Sprite {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
    var clip = project(place.xyz);
    // Grown in view space, so the sprite always faces the window.
    // Drawn out along the view's horizontal for bubbles streaking past.
    clip.x += corner.x * place.w * stretch.x / g.view.x;
    clip.y += corner.y * place.w / g.view.x;
    var out: Sprite;
    out.position = clip;
    out.corner = corner;
    let d = length(place.xyz - g.camera.xyz);
    // Snow and the like show where the searchlight catches them.
    var lit = 1.0;
    let kind = u32(tint.w + 0.5);
    if (kind == 0u) {
        let to = place.xyz - lamp_origin();
        let cone = smooth01(0.80, 0.95, dot(normalize(to), g.lamp.xyz));
        lit = 0.25 + 2.5 * cone * g.lamp.w / (1.0 + dot(to, to) * 0.08) + g.sun.w * 0.6;
    }
    out.colour = tint.rgb * lit * (1.0 - fog_of(d));
    out.kind = kind;
    return out;
}

@fragment
fn fs_sprite(in: Sprite) -> @location(0) vec4<f32> {
    let r = length(in.corner);
    if (r > 1.0) {
        discard;
    }
    var a = 0.0;
    switch (in.kind) {
        // A soft mote: marine snow, krill, plankton, embers.
        case 0u, 3u: {
            a = (1.0 - smooth01(0.0, 1.0, r));
        }
        // A bubble: a bright ring with a glint.
        case 1u: {
            a = smooth01(0.6, 0.9, r) * (1.0 - smooth01(0.9, 1.0, r)) * 0.8
                + (1.0 - smooth01(0.0, 0.3, length(in.corner - vec2<f32>(-0.35, 0.35)))) * 0.9;
        }
        // A wide glow round a light.
        default: {
            a = exp(-r * r * 4.0);
        }
    }
    return vec4<f32>(in.colour * a * SEA_SCALE, 0.0);
}
