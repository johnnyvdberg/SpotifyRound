// The Nautilus' salon window.
//
// Three entry points share this file:
//
// - `fs_overlay` draws the brass bezel, the riveted casing and the gauge faces.
//   None of it moves, so it is drawn once when the scene is built.
// - `fs_scene` draws the sea behind the glass into a smaller square texture.
//   Water is soft by nature, so this is where the frame budget goes.
// - `fs_composite` runs at the panel's full resolution: it scales the sea up,
//   gives it an engraved finish, lays the bezel over it and draws the gauge
//   needles, which are the only moving part of the frame.
//
// The sea is a real perspective view. The window looks out sideways, level,
// with the horizon through its centre; a point at world height `y` and distance
// `z` from the glass appears at `(x - camera) / z, y / z` on screen. Everything
// — the seabed, every rock and column, every fish — has a distance, and its
// size, its height above the horizon, how fast the voyage sweeps it past, how
// much water hides it and how much of the searchlight reaches it all follow from
// that one number. One world unit is about eleven metres.

struct Uniforms {
    // x, y: output frame size in pixels; z: seconds of run time; w: world seed in 0..1.
    frame: vec4<f32>,
    // x: edge of the square the sea is rendered into, in pixels; y: how far the
    // Nautilus has come, in world units; z: its height above the seabed; w: its
    // depth below the surface. The voyage itself is steered from Rust.
    scene: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var sea: texture_2d<f32>;
@group(0) @binding(2) var bezel: texture_2d<f32>;
@group(0) @binding(3) var sea_sampler: sampler;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const GLASS: f32 = 0.404;
const PORT: f32 = 0.449;
// Half the side of the square the sea texture covers, a little wider than the
// glass so the upscale never samples past the edge.
const VIEW: f32 = 0.410;

// The voyage. The Nautilus travels to the right at SPEED world units a second;
// every LEG units the sea changes character, cross-fading over the last BLEND of
// the leg. Seven legs make one circuit of about seven minutes. These mirror the
// constants in `dive.rs`, which steers the voyage.
const SPEED: f32 = 0.055;
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

// Height of the lava cave's vault above its floor, and the thickness of the
// polar ice.
const CAVE_ROOF: f32 = 0.95;
const ICE: f32 = 0.15;

// How far from the glass the divers walk.
const DIVER_Z: f32 = 1.2;
const DIVER_HEIGHT: f32 = 0.17;
const DIVER_WALK: f32 = 0.020;

// Gauges set into the bezel: the manometer on the left reads depth, the log on
// the right turns once per leg of the voyage.
const GAUGE_R: f32 = 0.050;
const GAUGE_RING: f32 = 0.420;
const DEPTH_GAUGE: f32 = -2.235;
const LOG_GAUGE: f32 = -0.907;

// The biomes this pipeline can draw, a bit per biome. Rust builds one sea
// pipeline per leg of the voyage, naming that leg's biome and its two
// neighbours, which is all that can ever be in view; the compiler then drops
// every other biome's features and creatures, and the smaller shader keeps more
// pixels in flight on the GPU.
override BIOMES_SHOWN: u32 = 0x7fu;

// How much the thick glass bends the view at its edge, applied by the
// composite. The procedural sea bends its own view while drawing it and leaves
// this at zero; the 3D dive (`dive3d`) renders a plain perspective view and
// lets the composite bend it here.
override GLASS_BEND: f32 = 0.0;

fn shown(mask: u32) -> bool {
    return (BIOMES_SHOWN & mask) != 0u;
}

fn bit(id: i32) -> u32 {
    return 1u << u32(id);
}

// Per-pixel constants of the sea pass, set once at the top of `fs_scene`.
var<private> T: f32;
var<private> SEED: f32;
var<private> CAM: f32;
// The Nautilus' height above the seabed and depth below the surface.
var<private> ALT: f32;
var<private> DEPTH: f32;
// World units per sea-texture pixel at distance 1.
var<private> PX0: f32;

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn hash11(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// Like smoothstep, but defined when the edges are given high to low. Several
// masks below read more naturally that way, and the WGSL builtin leaves a
// reversed range undefined.
fn soft(edge0: f32, edge1: f32, x: f32) -> f32 {
    let low = min(edge0, edge1);
    let high = max(edge0, edge1);
    let t = clamp((x - low) / max(high - low, 0.00001), 0.0, 1.0);
    let v = t * t * (3.0 - 2.0 * t);
    return select(v, 1.0 - v, edge0 > edge1);
}

// Coverage of a signed distance, anti-aliased over `aa` either side of the edge.
fn fill(d: f32, aa: f32) -> f32 {
    return soft(aa, -aa, d);
}

fn box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
}

fn ell(p: vec2<f32>, radius: vec2<f32>) -> f32 {
    return length(p / radius) - 1.0;
}

// An ellipse distance in the units of `p`, close enough to metric for
// anti-aliasing and smooth unions.
fn ellm(p: vec2<f32>, radius: vec2<f32>) -> f32 {
    return (length(p / radius) - 1.0) * min(radius.x, radius.y);
}

fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.00001), 0.0, 1.0);
    return length(pa - ba * h);
}

// A capsule whose radius runs from `ra` at `a` to `rb` at `b`.
fn tcap(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, ra: f32, rb: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.00001), 0.0, 1.0);
    return length(pa - ba * h) - mix(ra, rb, h);
}

fn smin(a: f32, b: f32, k: f32) -> f32 {
    let h = clamp(0.5 + 0.5 * (b - a) / k, 0.0, 1.0);
    return mix(b, a, h) - k * h * (1.0 - h);
}

fn triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 0.00001), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 0.00001), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 0.00001), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d0 = vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x));
    let d1 = vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x));
    let d2 = vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x));
    let d = min(min(d0, d1), d2);
    return -sqrt(d.x) * sign(d.y);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}

fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var i = 0; i < 3; i = i + 1) {
        value += amplitude * vnoise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.2);
        amplitude *= 0.5;
    }
    return value / 0.875;
}

// ---------------------------------------------------------------------------
// The voyage
// ---------------------------------------------------------------------------

// The biome of leg `k`. The order is a dive and a climb: sunlit shallows, the
// submarine forest of Crespo, the galleons of Vigo Bay, Atlantis, a lava cave
// and the abyss, then up under the polar ice and back to the shallows. The seed
// only picks where a visit joins the circuit.
fn biome(k: f32, seed: f32) -> i32 {
    let i = i32(k) + i32(seed * f32(BIOMES));
    return ((i % BIOMES) + BIOMES) % BIOMES;
}

struct Leg {
    a: i32,
    b: i32,
    // How far the sea has turned from `a` into `b`.
    w: f32,
};

fn leg(wx: f32, seed: f32) -> Leg {
    let u = wx / LEG;
    let k = floor(u);
    return Leg(biome(k, seed), biome(k + 1.0, seed), soft(1.0 - BLEND, 1.0, u - k));
}

fn presence(l: Leg, id: i32) -> f32 {
    return select(0.0, 1.0 - l.w, l.a == id) + select(0.0, l.w, l.b == id);
}

fn by_biome(l: Leg, values: array<f32, 7>) -> f32 {
    var v = values;
    return mix(v[l.a], v[l.b], l.w);
}

// The biome of something at world position `x`, distance `z`. Biomes change
// along the voyage, so their borders are planes across the line of travel; seen
// sideways those would run as straight lines to the vanishing point. Beyond
// BIOME_Z, things take the biome of their screen column at that distance, so a
// change of sea sweeps through the distance like a curtain instead.
const BIOME_Z: f32 = 2.2;

fn here(x: f32, z: f32) -> Leg {
    return leg(CAM + (x - CAM) * min(1.0, BIOME_Z / max(z, 0.001)), SEED);
}

// Which biomes are anywhere within half a unit of `x`, a bit per biome: a cheap
// test before a row of features does any real work.
fn biomes_near(x: f32, z: f32) -> u32 {
    var mask = 0u;
    for (var i = -1; i <= 1; i = i + 1) {
        let l = here(x + f32(i) * 0.5, z);
        mask |= select(0u, 1u << u32(l.a), l.w < 1.0) | select(0u, 1u << u32(l.b), l.w > 0.0);
    }
    return mask;
}

fn has(mask: u32, id: i32) -> bool {
    return (mask & (BIOMES_SHOWN & (1u << u32(id)))) != 0u;
}

// How far the seabed lies below the window at world position `x`, distance `z`.
fn floor_height(x: f32, z: f32) -> f32 {
    // Level under the window, wherever the Nautilus is: the depth of the sea
    // shows in the water and the gauge, and the seabed rises and sinks with the
    // boat's height above it.
    let l = here(x, z);
    let rough = by_biome(l, array<f32, 7>(0.05, 0.06, 0.05, 0.02, 0.07, 0.12, 0.05));
    let n = 0.65 * vnoise(vec2<f32>(x * 1.3, z * 1.3 + SEED * 11.7)) + 0.35 * vnoise(vec2<f32>(x * 3.1 + 5.0, z * 3.1));
    return ALT - rough * (n - 0.45);
}

// World height of the seabed, which is below the window and so negative.
fn ground(x: f32, z: f32) -> f32 {
    return -floor_height(x, z);
}

// One slot in a procession of things moving through the water at `speed` world
// units a second, one slot every `spacing`. `i` picks the nearer (0) or the
// farther (1) of the two slots around `x`, or -1 the slot containing it. x: the slot number; y: the slot's
// centre in world x right now.
fn slot(x: f32, speed: f32, spacing: f32, salt: f32, i: i32) -> vec2<f32> {
    let u = x - speed * T;
    // -1 asks for the slot containing x, for things much smaller than a slot.
    let k = select(floor(u / spacing - 0.5) + f32(i), floor(u / spacing), i < 0);
    let centre = (k + 0.5 + (hash11(k * 3.7 + salt) - 0.5) * 0.4) * spacing + speed * T;
    return vec2<f32>(k, centre);
}

// ---------------------------------------------------------------------------
// Light
// ---------------------------------------------------------------------------

// What is the same for every distance at this pixel.
struct Sea {
    depth: f32,
    sun: f32,
    lamp: f32,
    lava: f32,
    // The searchlight cone through this pixel.
    beam: f32,
    // The divers' lamp, in front of the lead diver.
    diver: f32,
    // Water attenuation per world unit.
    fogk: f32,
    // The window's height above a level seabed.
    floor: f32,
    // Distance to the seabed along this pixel's ray; a large number above it.
    floor_z: f32,
    // Distance to the cave's vault or the ice along this pixel's ray, where
    // there is one; a large number elsewhere.
    ceil_z: f32,
    // Heights of the cave's vault and of the ice above the window.
    roof: f32,
    ice: f32,
    water: vec3<f32>,
};

// The light on, and the water in front of, something at distance `z`.
struct Env {
    light: vec3<f32>,
    water: vec3<f32>,
    fog: f32,
    // World units per pixel at this distance.
    px: f32,
};

fn water_tone(d: f32) -> vec3<f32> {
    let shallow = vec3<f32>(0.11, 0.47, 0.46);
    let middle = vec3<f32>(0.022, 0.175, 0.200);
    let deep = vec3<f32>(0.006, 0.052, 0.078);
    let abyss = vec3<f32>(0.002, 0.009, 0.020);
    var colour = mix(shallow, middle, soft(0.0, 0.36, d));
    colour = mix(colour, deep, soft(0.30, 0.72, d));
    return mix(colour, abyss, soft(0.66, 1.08, d));
}

// The Nautilus' electric searchlight, mounted above and ahead of the salon and
// thrown forward and down, plus the glow of the salon itself spilling out.
fn searchlight(q: vec2<f32>) -> f32 {
    let origin = vec2<f32>(-0.58, 0.46);
    let dir = normalize(vec2<f32>(1.0, -0.72));
    let v = q - origin;
    let along = dot(v, dir);
    let across = abs(v.x * dir.y - v.y * dir.x);
    let cone = soft(along * 0.50, along * 0.12, across) / (1.0 + along * along * 1.2);
    let pool = exp(-dot(q, q) * 3.5);
    return cone * 1.6 + pool * 0.45;
}

// The nearer of the seabed and the roof along this pixel's ray: nothing farther
// than this can be seen.
fn wall(s: Sea) -> f32 {
    return min(s.floor_z, s.ceil_z);
}

fn env_at(s: Sea, q: vec2<f32>, z: f32) -> Env {
    // Height above the seabed: the sun is brighter higher up, the volcano lights
    // the ground, and the searchlight dims with distance from the window.
    let h = q.y * z + s.floor;
    var e: Env;
    e.light = vec3<f32>(0.40, 0.82, 0.78) * s.sun * (0.45 + 0.75 * soft(0.0, 0.9, h)) * 1.6
        + vec3<f32>(1.0, 0.92, 0.76) * s.beam * s.lamp * exp(-max(z - 0.7, 0.0) * 0.28)
        + vec3<f32>(1.0, 0.38, 0.12) * s.lava * (0.10 + 0.30 * soft(0.6, 0.0, h))
        + vec3<f32>(1.0, 0.94, 0.78) * s.diver * exp(-abs(z - DIVER_Z) * 1.6)
        + vec3<f32>(0.035, 0.055, 0.070);
    e.water = s.water;
    e.fog = 1.0 - exp(-z * s.fogk);
    e.px = PX0 * z;
    return e;
}

// Shade a solid with albedo `albedo` and let the water in front swallow it.
fn lit(albedo: vec3<f32>, env: Env) -> vec3<f32> {
    return mix(albedo * env.light, env.water, env.fog);
}

// Something that makes its own light, seen through the water in front of it.
fn shine(emitted: vec3<f32>, env: Env) -> vec3<f32> {
    return emitted * (1.0 - env.fog * 0.85);
}

fn caustic(p: vec2<f32>, t: f32) -> f32 {
    let a = sin(p.x + sin(p.y * 1.3 + t));
    let b = sin(p.y * 1.1 + sin(p.x * 0.9 - t * 0.7));
    let web = abs(a + b) * 0.5;
    return pow(max(1.0 - web, 0.0), 7.0);
}

fn motes(q: vec2<f32>, scale: f32, drift: vec2<f32>, aspect: vec2<f32>) -> f32 {
    let p = q * scale + drift;
    let cell = floor(p);
    let f = fract(p);
    let centre = 0.18 + 0.64 * hash22(cell);
    let d = length((f - centre) * aspect);
    let twinkle = 0.35 + 0.65 * hash11(cell.x * 7.3 + cell.y * 13.1);
    return soft(0.045, 0.0, d) * twinkle;
}

// ---------------------------------------------------------------------------
// Creatures
// ---------------------------------------------------------------------------
//
// Each creature is drawn in its own body units, facing +x, and takes `px`, the
// size of a pixel in those units. Swimming follows the anatomy: fish and sharks
// sweep their tails from side to side, across the line of sight, so in profile
// the tail foreshortens and widens again; whales and dolphins beat their flukes
// up and down, so a wave runs down the body and the flukes pitch with it. The
// beat itself is set by the animal's speed through the water.

// A fish, snout at +1, tail fork near -1.35. x: body; y: fins; z: belly; w: eye.
fn fish(l: vec2<f32>, px: f32, beat: f32, variant: f32) -> vec4<f32> {
    let h = 0.25 + 0.12 * variant;
    let taper = 0.50 + 0.50 * soft(-0.95, 0.40, l.x);
    var body = ellm(l - vec2<f32>(0.02, 0.0), vec2<f32>(0.93, h * taper));
    body = smin(body, tcap(l, vec2<f32>(-0.55, 0.0), vec2<f32>(-0.90, 0.0), h * 0.30, 0.04), 0.06);

    // The caudal fin swings toward and away from the viewer, so its apparent
    // length follows the cosine of the sweep; it lags the body's wave.
    let sweep = 1.05 * sin(beat - 1.2);
    let squash = max(abs(cos(sweep)), 0.14);
    var f = l - vec2<f32>(-0.88, 0.0);
    f.x = f.x / squash;
    let spread = 0.30 + 0.06 * variant;
    let web = triangle(f, vec2<f32>(0.04, 0.0), vec2<f32>(-0.44, spread), vec2<f32>(-0.44, -spread));
    let fork = ellm(f - vec2<f32>(-0.52, 0.0), vec2<f32>(0.20, spread * 0.55));
    let caudal = max(web, -fork) * squash;

    // Dorsal and anal fins ride the body; the pectoral rows on its own rhythm.
    let dorsal = ellm(rot(0.35) * (l - vec2<f32>(-0.10, h * 0.82)), vec2<f32>(0.24, 0.10 + 0.01 * sin(beat * 0.5)));
    let anal = ellm(rot(0.45) * (l - vec2<f32>(-0.45, -h * 0.55)), vec2<f32>(0.14, 0.06));
    let row = sin(beat * 0.45 + variant * 3.0);
    let reach = 0.20 * (0.75 + 0.25 * row);
    let pp = rot(0.45 + 0.25 * row) * (l - vec2<f32>(0.40, -h * 0.30));
    let pectoral = ellm(pp + vec2<f32>(reach, 0.0), vec2<f32>(reach, 0.040));
    let fins = min(min(caudal, dorsal), min(anal, pectoral));

    let eye = length(l - vec2<f32>(0.70, h * 0.22)) - 0.06;
    let gill = abs(length((l - vec2<f32>(0.75, 0.0)) / vec2<f32>(1.0, 1.6)) - 0.30) - 0.008;
    let body_m = fill(body, px);
    let fins_m = fill(fins, px) * (1.0 - body_m);
    let belly = body_m * soft(0.02, -h * 0.55, l.y);
    let marks = fill(eye, px) + fill(gill, px) * 0.35 * body_m;
    return vec4<f32>(body_m, fins_m, belly, marks);
}

// A school of one species, swimming at `speed` (signed, world units a second)
// at distance `z`. Schools are patchy: they gather in some stretches of water and
// leave others empty, most are a handful of fish and now and then one is a great
// cloud. Each is densest in its core and at its leading edge, thins out to a few
// stragglers, keeps to its species' band of water above the seabed, bends as it
// turns, and a wave of turning sweeps through it. Fish sit in a loose grid of
// about a body length, so each pixel only looks at the six fish around it
// however large the school is.
fn shoal(
    col_in: vec3<f32>,
    q: vec2<f32>,
    s: Sea,
    z: f32,
    salt: f32,
    size: f32,
    speed: f32,
    band: vec2<f32>,
    back: vec3<f32>,
    gate: array<f32, 7>,
) -> vec3<f32> {
    var col = col_in;
    if (wall(s) < z) {
        return col;
    }
    let x = CAM + q.x * z;
    let y = q.y * z;
    let dir = select(-1.0, 1.0, speed >= 0.0);
    for (var i = 0; i < 2; i = i + 1) {
        let sl = slot(x, speed, 2.6, salt, i);
        let k = sl.x;
        let big = pow(hash11(k * 7.3 + salt), 2.0);
        let rx = 0.06 + 0.70 * big;
        let ry = rx * (0.20 + 0.22 * hash11(k * 2.9 + salt));
        // `band` is the species' range of height above the seabed.
        let rise = mix(band.x, band.y, hash11(k * 8.1 + salt)) + 0.04 * sin(T * 0.09 + k * 2.0);
        let cy = -s.floor + max(rise, ry + 0.05);
        var p = vec2<f32>(x - sl.y, y - cy);
        p.y -= rx * 0.22 * sin(p.x / rx * 2.0 + T * 0.25 + k);
        if (abs(p.x) > rx * 1.6 + size * 3.0 || abs(p.y) > ry * 1.6 + size * 2.0) {
            continue;
        }
        let gathering = vnoise(vec2<f32>(sl.y * 0.25, salt));
        let fade = soft(0.25, 0.6, by_biome(leg(sl.y, SEED), gate))
            * soft(0.42, 0.58, gathering * 0.7 + hash11(k * 5.1 + salt) * 0.5);
        if (fade <= 0.0) {
            continue;
        }
        let env = env_at(s, q, z);
        let g = vec2<f32>(3.2, 1.8) * size;
        let base = vec2<f32>(floor(p.x / g.x) - 1.0, floor(p.y / g.y - 0.5));
        // The outline is ragged on a scale much larger than one fish, so one
        // lookup at the pixel serves all six fish around it.
        let ragged = (vnoise(p * (3.0 / rx) + vec2<f32>(T * 0.10, k)) - 0.5) * 0.5;
        var body = 0.0;
        var fins = 0.0;
        var belly = 0.0;
        var marks = 0.0;
        var flash = 0.0;
        for (var j = 0; j < 6; j = j + 1) {
            let cell = base + vec2<f32>(f32(j % 3), f32(j / 3));
            let r = hash22(cell + vec2<f32>(k * 13.0 + salt, salt));
            let c = (cell + 0.5 + (r - 0.5) * vec2<f32>(0.40, 0.25)) * g;
            let front = soft(-rx, rx, c.x * dir);
            let e = length(c / vec2<f32>(rx, ry * (0.75 + 0.35 * front))) + ragged;
            let density = mix(0.05, 0.95, soft(1.05, 0.40, e)) * step(e, 1.6);
            if (hash21(cell * 1.7 + vec2<f32>(k, salt)) > density) {
                continue;
            }
            let fsize = size * (0.85 + 0.30 * r.y);
            let wob = vec2<f32>(sin(T * 0.7 + r.x * 20.0) * 0.08, cos(T * 0.9 + r.y * 20.0) * 0.05) * g;
            var l = (p - c - wob) / fsize;
            l.x *= dir;
            let turn = 0.10 * sin(T * 0.6 - c.x / rx * 4.0 + k) + (r.x - 0.5) * 0.12;
            l = rot(turn) * l;
            if (l.x < -1.5 || l.x > 1.1 || abs(l.y) > 0.75) {
                continue;
            }
            // One tail beat per 0.7 body lengths travelled.
            let beat = T * TAU * abs(speed) / (fsize * 1.4) * (0.85 + 0.30 * r.x) + r.y * TAU;
            let f = fish(l, env.px / fsize, beat, r.x);
            body = max(body, f.x);
            fins = max(fins, f.y);
            belly = max(belly, f.z);
            marks = max(marks, f.w * f.x);
            // A school flickers silver as individual fish turn into the light.
            flash = max(flash, f.x * pow(max(sin(T * 0.9 + r.x * 40.0 - c.x / rx * 3.0), 0.0), 14.0));
        }
        let albedo = mix(back, vec3<f32>(0.72, 0.74, 0.70), belly);
        col = mix(col, lit(albedo, env), body * fade);
        col = mix(col, lit(back * 0.9 + vec3<f32>(0.08), env), fins * 0.55 * fade);
        col = mix(col, lit(vec3<f32>(0.02), env), marks * 0.8 * fade);
        col += env.light * vec3<f32>(0.55, 0.62, 0.62) * flash * (1.0 - env.fog) * fade;
    }
    return col;
}

// A shark, snout at +1: its tail sweeps sideways like a fish's, with the long
// upper lobe of the sharks; the fins are rigid.
fn shark(l: vec2<f32>, px: f32, beat: f32) -> vec4<f32> {
    let taper = 0.30 + 0.70 * soft(-0.80, 0.15, l.x);
    let nose = 1.0 - 0.45 * soft(0.45, 0.95, l.x);
    var body = ellm(l - vec2<f32>(0.08, 0.0), vec2<f32>(0.92, 0.17 * taper * nose));
    body = smin(body, tcap(l, vec2<f32>(-0.55, 0.005), vec2<f32>(-0.84, 0.03), 0.05, 0.022), 0.05);

    let sweep = 0.95 * sin(beat - 1.0);
    let squash = max(abs(cos(sweep)), 0.18);
    var f = l - vec2<f32>(-0.82, 0.03);
    f.x = f.x / squash;
    let upper = tcap(f, vec2<f32>(0.0, 0.0), vec2<f32>(-0.36, 0.30), 0.05, 0.012);
    let lower = tcap(f, vec2<f32>(0.0, -0.01), vec2<f32>(-0.20, -0.16), 0.045, 0.012);
    let tail = min(upper, lower) * squash;

    let dorsal = triangle(l, vec2<f32>(-0.06, 0.12), vec2<f32>(0.22, 0.13), vec2<f32>(-0.02, 0.38));
    let second = triangle(l, vec2<f32>(-0.52, 0.06), vec2<f32>(-0.42, 0.06), vec2<f32>(-0.50, 0.12));
    let pectoral = triangle(l, vec2<f32>(0.22, -0.07), vec2<f32>(0.42, -0.08), vec2<f32>(0.02, -0.34 + 0.02 * sin(beat * 0.3)));
    let pelvic = triangle(l, vec2<f32>(-0.30, -0.07), vec2<f32>(-0.18, -0.08), vec2<f32>(-0.32, -0.15));
    let shape = min(min(body, tail), min(min(dorsal, second), min(pectoral, pelvic)));
    let mask = fill(shape, px);
    let belly = mask * soft(-0.01, -0.10, l.y) * soft(0.85, 0.6, l.x);
    var gills = 1.0;
    for (var g = 0; g < 5; g = g + 1) {
        let gx = 0.50 + f32(g) * 0.035;
        gills = min(gills, max(abs(l.x - gx) - 0.004, abs(l.y + 0.005) - 0.05));
    }
    let eye = length(l - vec2<f32>(0.80, 0.035)) - 0.018;
    return vec4<f32>(mask, belly, fill(gills, px) * mask, fill(eye, px));
}

// A bottlenose dolphin, beak at +0.52, or with `narwhal` a narwhal: blunt head,
// no dorsal fin and a spiral tusk. The flukes beat up and down. w: the tusk.
fn dolphin(p_in: vec2<f32>, px: f32, beat: f32, narwhal: bool) -> vec4<f32> {
    var p = p_in;
    let reach = soft(0.15, -0.50, p.x);
    let amp = 0.045 * reach * reach;
    let wave = amp * sin(beat + p.x * 7.0);
    p.y -= wave;
    let slope = amp * 7.0 * cos(beat - 0.48 * 7.0);

    var d = ellm(p - vec2<f32>(0.02, 0.0), vec2<f32>(0.34, 0.085));
    d = smin(d, tcap(p, vec2<f32>(-0.15, 0.008), vec2<f32>(-0.46, 0.0), 0.06, 0.012), 0.05);
    var tusk = 1.0;
    if (narwhal) {
        d = smin(d, ellm(p - vec2<f32>(0.33, 0.004), vec2<f32>(0.13, 0.072)), 0.03);
        // A low ridge along the back instead of a fin.
        d = smin(d, ellm(p - vec2<f32>(-0.10, 0.07), vec2<f32>(0.16, 0.022)), 0.02);
        let along = clamp((p.x - 0.44) / 0.48, 0.0, 1.0);
        tusk = segment(p, vec2<f32>(0.44, 0.0), vec2<f32>(0.92, 0.012)) - mix(0.011, 0.004, along);
    } else {
        d = smin(d, ellm(p - vec2<f32>(0.32, 0.012), vec2<f32>(0.11, 0.062)), 0.03);
        d = smin(d, tcap(p, vec2<f32>(0.38, -0.012), vec2<f32>(0.52, -0.020), 0.026, 0.014), 0.02);
        let fin = triangle(p, vec2<f32>(-0.08, 0.06), vec2<f32>(0.07, 0.07), vec2<f32>(-0.12, 0.175));
        d = smin(d, fin, 0.012);
    }
    let fl = rot(0.65 + 0.10 * sin(beat * 0.5)) * (p - vec2<f32>(0.20, -0.06));
    let flipper = ellm(fl + vec2<f32>(0.06, 0.0), vec2<f32>(0.065, 0.017));
    let fluke = ellm(rot(atan(slope) * 1.3) * (p - vec2<f32>(-0.46, 0.0)) + vec2<f32>(0.06, 0.0), vec2<f32>(0.07, 0.012));
    let shape = min(min(d, flipper), fluke);
    let mask = fill(shape, px);
    let belly = mask * soft(-0.01, -0.06, p.y + 0.02 * p.x);
    let eye = fill(length(p - vec2<f32>(0.36, 0.005)) - 0.010, px);
    let mouth = fill(segment(p, vec2<f32>(0.40, -0.022), vec2<f32>(0.52, -0.022)) - 0.002, px) * mask * select(1.0, 0.0, narwhal);
    // The tusk's left-handed spiral shows as fine diagonal grooves.
    let grooves = 0.75 + 0.25 * sin(p.x * 160.0 + p.y * 400.0);
    return vec4<f32>(mask, belly, eye + mouth, fill(tusk, px) * grooves);
}

// A North Atlantic right whale, in whale lengths with the snout at +0.5. A third
// of the animal is head: a narrow arched rostrum over the huge bowed lower lip,
// rough pale callosities, no dorsal fin, broad paddle flippers.
fn whale(col_in: vec3<f32>, p_in: vec2<f32>, beat: f32, env: Env, px: f32) -> vec3<f32> {
    var col = col_in;
    var p = p_in;
    if (p.x < -0.62 || p.x > 0.56 || abs(p.y) > 0.22) {
        return col;
    }
    let reach = soft(0.10, -0.50, p.x);
    let amp = 0.030 * reach * reach;
    p.y -= amp * sin(beat + p.x * 5.0);
    let slope = 0.030 * 5.0 * cos(beat - 0.48 * 5.0);

    // The broad back rises behind a dip at the blowholes; the head in front of
    // it is lower, its narrow rostrum arching down to the tip of the snout.
    var d = ellm(p - vec2<f32>(-0.03, 0.004), vec2<f32>(0.36, 0.104));
    d = smin(d, ellm(p - vec2<f32>(0.28, -0.028), vec2<f32>(0.20, 0.080)), 0.025);
    d = smin(d, ellm(p - vec2<f32>(0.27, -0.064), vec2<f32>(0.18, 0.060)), 0.03);
    let rp = rot(-0.30) * (p - vec2<f32>(0.375, 0.022));
    d = smin(d, ellm(rp, vec2<f32>(0.135, 0.042)), 0.02);
    d = smin(d, tcap(p, vec2<f32>(-0.22, 0.004), vec2<f32>(-0.48, 0.0), 0.075, 0.014), 0.05);
    let fp = rot(atan(slope) * 1.4) * (p - vec2<f32>(-0.48, 0.0));
    let fluke = ellm(fp + vec2<f32>(0.065, 0.0), vec2<f32>(0.075, 0.013));
    let fl = rot(0.95 + 0.08 * sin(beat * 0.5)) * (p - vec2<f32>(0.17, -0.080));
    let flipper = box(fl + vec2<f32>(0.055, 0.0), vec2<f32>(0.055, 0.022), 0.016);
    let shape = min(min(d, fluke), flipper);
    let mask = fill(shape, px * 1.2);
    if (mask <= 0.0) {
        return col;
    }

    // Black, lit from above, with a rim of light along the back.
    let shade = 0.35 + 1.10 * soft(-0.09, 0.10, p.y);
    var skin = vec3<f32>(0.075, 0.080, 0.088) * shade * (0.85 + 0.15 * vnoise(p * 60.0));
    let belly = fill(ellm(p - vec2<f32>(0.02, -0.088), vec2<f32>(0.09, 0.022)) + (vnoise(p * 50.0) - 0.5) * 0.02, px * 3.0);
    skin = mix(skin, vec3<f32>(0.55, 0.55, 0.52), belly * 0.7);

    // The mouth line bows up from the tip of the rostrum and down to the eye.
    let m = min(
        min(segment(p, vec2<f32>(0.49, 0.000), vec2<f32>(0.44, 0.030)), segment(p, vec2<f32>(0.44, 0.030), vec2<f32>(0.37, 0.040))),
        min(segment(p, vec2<f32>(0.37, 0.040), vec2<f32>(0.30, 0.020)), segment(p, vec2<f32>(0.30, 0.020), vec2<f32>(0.245, -0.020))),
    );
    // A dark crease, with light catching the edge of the great lower lip below it.
    skin *= 1.0 - 0.75 * fill(m - 0.002, px);
    let lip = fill(abs(m - 0.007) - 0.0025, px) * step(p.y, 0.045 - (p.x - 0.37) * (p.x - 0.37) * 1.6);
    skin += vec3<f32>(0.16, 0.17, 0.17) * lip * soft(0.22, 0.30, p.x);
    let eye = fill(length(p - vec2<f32>(0.232, -0.034)) - 0.006, px);
    skin = mix(skin, vec3<f32>(0.01), eye);

    // Callosities: crusted pale patches on the rostrum, over the eye and along
    // the lip, the marks by which right whales are told apart.
    let crust = (vnoise(p * 140.0) - 0.5) * 0.010;
    var callus = ellm(rot(-0.32) * (p - vec2<f32>(0.445, 0.032)), vec2<f32>(0.035, 0.014));
    callus = min(callus, ellm(p - vec2<f32>(0.37, 0.060), vec2<f32>(0.030, 0.012)));
    callus = min(callus, ellm(p - vec2<f32>(0.25, -0.002), vec2<f32>(0.020, 0.010)));
    callus = min(callus, ellm(p - vec2<f32>(0.39, -0.068), vec2<f32>(0.030, 0.013)));
    callus = min(callus, ellm(p - vec2<f32>(0.32, -0.104), vec2<f32>(0.026, 0.011)));
    callus = min(callus, ellm(p - vec2<f32>(0.46, -0.030), vec2<f32>(0.018, 0.012)));
    let pale = fill(callus + crust, px) * mask;
    skin = mix(skin, vec3<f32>(0.62, 0.60, 0.52) * (0.7 + 0.3 * vnoise(p * 400.0)), pale);

    var whale_col = lit(skin, env);
    whale_col += env.light * vec3<f32>(0.16, 0.22, 0.21) * fill(abs(shape + px * 3.0) - px * 2.0, px * 2.0) * soft(-0.02, 0.06, p.y) * (1.0 - env.fog);
    return mix(col, whale_col, mask);
}

// A jellyfish: a pulsing translucent dome, scalloped rim, radial canals and a
// fan of fine tentacles that trail the pulse rather than hanging straight.
fn jellyfish(q: vec2<f32>, centre: vec2<f32>, scale: f32, phase: f32, t: f32) -> vec3<f32> {
    let pulse = 0.5 + 0.5 * sin(t * 1.35 + phase);
    let contract = pulse * pulse;
    let rx = 0.062 * scale * (1.0 - 0.18 * contract);
    let ry = 0.043 * scale * (1.0 + 0.16 * contract);
    let l = q - centre;
    let dome = ell(l - vec2<f32>(0.0, 0.012 * scale), vec2<f32>(rx, ry));
    let upper = soft(-0.010 * scale, 0.004 * scale, l.y);
    let bell = soft(0.14, -0.03, dome) * upper;

    let rim_angle = atan2(l.y, l.x);
    let shell = soft(1.05, 0.92, length(l / vec2<f32>(rx, ry)));
    let rim_y = -0.010 * scale + 0.003 * scale * sin(rim_angle * 9.0 + t * 0.8);
    let rim = exp(-abs(l.y - rim_y) * 260.0 / scale) * shell;
    let halo = exp(-dot(l, l) / (0.0045 * scale * scale));
    let canals = pow(abs(sin(rim_angle * 7.0 + phase)), 16.0)
        * soft(0.9, 0.15, length(l / vec2<f32>(rx, ry)))
        * bell;

    let down = centre.y - q.y;
    var tentacles = 0.0;
    if (down > 0.0 && down < 0.20 * scale && abs(l.x) < 0.09 * scale) {
        for (var j = 0; j < 7; j = j + 1) {
            let fj = f32(j);
            let reach = (0.11 + 0.07 * hash11(fj * 2.3 + phase)) * scale;
            let anchor = centre.x + (fj / 6.0 - 0.5) * 0.10 * scale;
            let x = anchor
                + sin(down * 26.0 / scale + t * 2.4 + fj * 1.1 + phase) * 0.008 * scale * (0.2 + down * 3.0);
            let width = 0.0012 * scale * (1.0 + down * 2.0);
            tentacles += soft(width, 0.0, abs(q.x - x))
                * soft(0.0, 0.012, down)
                * soft(reach, reach * 0.4, down);
        }
    }

    let bell_col = vec3<f32>(0.40, 0.34, 0.82);
    let glow = vec3<f32>(0.14, 0.66, 0.76);
    return bell_col * (bell * 0.16 + rim * 0.50 + canals * 0.12)
        + glow * (halo * 0.14 + tentacles * 0.22);
}

// The poulpe of chapter XVIII: a giant squid swimming mantle first, its eight
// arms and two long tentacles trailing behind. `l` is in squid lengths, the
// mantle tip at +0.40 and the tentacle clubs near -0.80.
fn squid(col_in: vec3<f32>, l_in: vec2<f32>, px: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let t = T;
    let l = rot(0.05 * sin(t * 0.21)) * l_in;
    if (l.x < -0.86 || l.x > 0.46 || abs(l.y) > 0.26) {
        return col;
    }
    // Squid swim by jet: the mantle squeezes while the arms close up behind it.
    let jet = 0.5 + 0.5 * sin(t * 0.9);

    let taper = 1.0 - 0.62 * soft(0.0, 0.40, l.x);
    let mantle = ellm(l - vec2<f32>(0.17, 0.0), vec2<f32>(0.23, (0.066 - 0.008 * jet) * taper));
    let fin = ellm(l - vec2<f32>(0.33, 0.0), vec2<f32>(0.075, 0.075 * (1.0 - soft(0.30, 0.42, l.x) * 0.7)));
    let head = ellm(l - vec2<f32>(-0.07, 0.0), vec2<f32>(0.075, 0.050));
    var shape = smin(min(mantle, fin), head, 0.02);

    var arms = 1.0;
    let a = -(l.x + 0.11);
    if (a > -0.02) {
        for (var j = 0; j < 10; j = j + 1) {
            let fj = f32(j);
            let tentacle = j >= 8;
            let reach = select(0.24 + 0.16 * hash11(fj * 3.1 + 0.7), 0.70, tentacle);
            let lane = select((fj / 7.0 - 0.5) * 0.090, (fj - 8.5) * 0.025, tentacle);
            let spread = lane * (1.0 + a * (5.0 - 2.5 * jet));
            let along = clamp(a / reach, 0.0, 1.0);
            // Each arm writhes on its own, and curls at the tip.
            let wave = 0.035 * a * sin(a * 9.0 - t * (1.6 + 0.3 * hash11(fj)) + fj * 1.9)
                + sign(lane + 0.001) * 0.04 * along * along * along;
            let y = spread + wave;
            var width = 0.0085 * (1.0 - along) + 0.0015;
            if (tentacle) {
                width = 0.0035 + 0.009 * soft(0.80, 0.92, along) * soft(1.0, 0.95, along);
            }
            let d = max(abs(l.y - y) - width, a - reach);
            arms = min(arms, d);
        }
    }
    shape = min(shape, arms);
    let mask = fill(shape, px);
    if (mask <= 0.0) {
        return col;
    }

    let mottle = vnoise(l * 38.0 + vec2<f32>(t * 0.3, 0.0));
    let albedo = mix(vec3<f32>(0.26, 0.07, 0.05), vec3<f32>(0.48, 0.17, 0.11), mottle);
    // Lit from the searchlight above: a bright back, a dark underside.
    let round = soft(-0.05, 0.05, l.y);
    var body = albedo * (0.35 + 0.75 * round);

    // The eye, large and lit by the searchlight.
    let ep = l - vec2<f32>(-0.06, 0.010);
    let eye = fill(length(ep) - 0.026, px);
    let pupil = fill(length(ep - vec2<f32>(0.004, 0.0)) - 0.012, px);
    let glint = fill(length(ep - vec2<f32>(0.010, 0.010)) - 0.005, px);
    body = mix(body, vec3<f32>(0.70, 0.62, 0.40), eye);
    body = mix(body, vec3<f32>(0.01, 0.01, 0.015), pupil);
    var c = lit(body, env) + shine(vec3<f32>(0.9, 0.9, 0.8) * glint * 0.6, env);
    return mix(col, c, mask);
}

// An anglerfish: almost invisible, except where its own lure lights it.
fn anglerfish(col_in: vec3<f32>, l_in: vec2<f32>, px: f32, phase: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    var l = l_in;
    if (dot(l, l) > 4.0) {
        return col;
    }
    let t = T;
    l.y += 0.04 * sin(t * 1.3 + phase);
    let lure = vec2<f32>(0.78, 0.48);
    let flicker = 0.75 + 0.25 * sin(t * 3.1 + phase) * sin(t * 1.7 + phase * 2.0);
    let body = ellm(l, vec2<f32>(0.55, 0.42));
    let jaw = ellm(l - vec2<f32>(0.32, -0.12), vec2<f32>(0.30, 0.20));
    let tail = triangle(l, vec2<f32>(-0.45, 0.0), vec2<f32>(-0.85, 0.28 * cos(t * 2.0 + phase)), vec2<f32>(-0.85, -0.28 * cos(t * 2.0 + phase)));
    let shape = min(min(body, jaw), tail);
    let mouth = soft(0.03, 0.0, abs(l.y + 0.12 + 0.12 * (l.x - 0.35))) * soft(0.0, 0.10, l.x - 0.25) * soft(0.66, 0.55, l.x);
    let teeth = pow(abs(sin(l.x * 60.0)), 6.0) * mouth;
    let stalk = soft(0.035, 0.0, segment(l, vec2<f32>(0.20, 0.38), lure));
    let glow_light = exp(-length(l - lure) * 2.6) * flicker;
    let mask = fill(shape, max(px, 0.03));
    let skin = vec3<f32>(0.20, 0.17, 0.16) * (glow_light + env.light * 0.4) * (0.5 + 0.5 * vnoise(l * 9.0));
    col = mix(col, mix(skin, env.water, env.fog), mask * 0.95);
    col += shine(vec3<f32>(0.85, 0.80, 0.65) * teeth * glow_light * 0.6, env);
    col = mix(col, vec3<f32>(0.10, 0.10, 0.10) * glow_light, stalk * (1.0 - env.fog));
    col += shine(vec3<f32>(0.45, 1.0, 0.85) * (exp(-length(l - lure) * 14.0) * 0.9 + exp(-length(l - lure) * 3.5) * 0.10) * flicker, env);
    return col;
}

// A diver in a Rouquayrol-Denayrouze suit: copper helmet, air reservoir on the
// back, weighted boots. `l` is in diver heights with the feet at the origin,
// walking to the right. Variant 0 carries a Ruhmkorff lamp, the others the
// compressed-air guns of chapter XVI.
fn diver(col_in: vec3<f32>, l_in: vec2<f32>, px: f32, stride: f32, variant: i32, env: Env) -> vec3<f32> {
    var col = col_in;
    let bob = 0.012 * abs(cos(stride));
    let l = l_in - vec2<f32>(0.0, bob);
    if (l.x < -0.30 || l.x > 0.40 || l.y < -0.04 || l.y > 1.02) {
        return col;
    }

    // Legs: thigh and shin per side, the far leg half a stride behind.
    let hip = vec2<f32>(0.0, 0.44);
    var legs = 1.0;
    var boots = 1.0;
    for (var s = 0; s < 2; s = s + 1) {
        let ph = stride + f32(s) * PI;
        let thigh = 0.34 * sin(ph);
        let knee = hip + vec2<f32>(sin(thigh), -cos(thigh)) * 0.22;
        let bend = 0.45 * max(0.0, sin(ph + 1.3));
        let foot = knee + vec2<f32>(sin(thigh - bend), -cos(thigh - bend)) * 0.20;
        legs = min(legs, min(segment(l, hip, knee) - 0.050, segment(l, knee, foot) - 0.044));
        boots = min(boots, box(l - foot - vec2<f32>(0.035, -0.012), vec2<f32>(0.065, 0.026), 0.012));
    }

    let torso = box(l - vec2<f32>(0.0, 0.59), vec2<f32>(0.085, 0.15), 0.05);
    let corselet = ellm(l - vec2<f32>(0.01, 0.74), vec2<f32>(0.13, 0.05));
    let tank = box(l - vec2<f32>(-0.135, 0.62), vec2<f32>(0.045, 0.13), 0.035);
    let helmet_c = vec2<f32>(0.03, 0.865);
    let helmet = length(l - helmet_c) - 0.105;

    // Arms: the near arm reaches forward with the lamp or the gun.
    let shoulder = vec2<f32>(0.03, 0.71);
    var elbow = vec2<f32>(0.10, 0.57);
    var hand = vec2<f32>(0.19, 0.58);
    if (variant != 0) {
        elbow = vec2<f32>(0.06, 0.55);
        hand = vec2<f32>(0.15, 0.55);
    }
    let swing = 0.07 * sin(stride + PI);
    let back_hand = vec2<f32>(-0.04 + swing, 0.46);
    let arms = min(
        min(segment(l, shoulder, elbow) - 0.036, segment(l, elbow, hand) - 0.032),
        segment(l, shoulder - vec2<f32>(0.04, 0.0), back_hand) - 0.034,
    );
    var gear = 1.0;
    var lamp = 0.0;
    if (variant == 0) {
        gear = box(l - hand - vec2<f32>(0.035, -0.03), vec2<f32>(0.03, 0.04), 0.008);
        lamp = fill(box(l - hand - vec2<f32>(0.058, -0.03), vec2<f32>(0.010, 0.028), 0.004), px);
    } else {
        gear = segment(l, vec2<f32>(-0.02, 0.50), vec2<f32>(0.36, 0.63)) - 0.014;
    }

    let suit_d = min(min(min(legs, torso), min(arms, corselet)), tank);
    let suit = fill(suit_d, px);
    let boot = fill(boots, px);
    let hel = fill(helmet, px);
    let kit = fill(gear, px);
    let cover = max(max(suit, boot), max(hel, kit));
    if (cover <= 0.0) {
        return col;
    }

    let canvas = vec3<f32>(0.46, 0.40, 0.31) * (0.80 + 0.20 * vnoise(l * 30.0));
    let shade = 0.70 + 0.30 * soft(-0.10, 0.12, l.x);
    var c = col;
    c = mix(c, lit(canvas * shade, env), suit);
    c = mix(c, lit(vec3<f32>(0.30, 0.31, 0.33), env), fill(tank, px) * 0.8);
    c = mix(c, lit(vec3<f32>(0.10, 0.10, 0.11), env), boot);
    c = mix(c, lit(vec3<f32>(0.16, 0.15, 0.14), env), kit);
    c = mix(c, lit(vec3<f32>(0.62, 0.40, 0.22), env), fill(abs(corselet) - 0.012, px) * suit * 0.6);

    // The copper helmet: a lit sphere with its round faceplate and a hot spot.
    let hp = (l - helmet_c) / 0.105;
    let lambert = clamp(0.55 + 0.45 * dot(normalize(vec3<f32>(hp, sqrt(max(1.0 - dot(hp, hp), 0.0)))), normalize(vec3<f32>(-0.3, 0.6, 0.7))), 0.0, 1.0);
    var copper = vec3<f32>(0.78, 0.42, 0.20) * (0.35 + 0.95 * lambert);
    copper += vec3<f32>(0.90, 0.70, 0.45) * exp(-dot(hp - vec2<f32>(-0.35, 0.45), hp - vec2<f32>(-0.35, 0.45)) * 18.0) * 0.45;
    let port = length(l - vec2<f32>(0.10, 0.86)) - 0.045;
    copper = mix(copper, vec3<f32>(1.08, 0.74, 0.38), fill(abs(port) - 0.010, px));
    copper = mix(copper, vec3<f32>(0.20, 0.30, 0.32), fill(port + 0.008, px));
    c = mix(c, lit(copper, env), hel);

    // The Ruhmkorff lamp burns white.
    c += shine(vec3<f32>(1.0, 0.94, 0.78) * lamp * 1.4, env);
    return c;
}

// The Ruhmkorff lamp's beam around the lead diver, `v` in world units from it.
fn diver_beam(v: vec2<f32>) -> f32 {
    let dir = normalize(vec2<f32>(1.0, -0.38));
    let along = dot(v, dir);
    let across = abs(v.x * dir.y - v.y * dir.x);
    let cone = soft(along * 0.36 + 0.004, along * 0.08, across) * soft(0.0, 0.02, along) / (1.0 + along * along * 12.0);
    return cone + exp(-dot(v, v) * 1500.0) * 1.5;
}

// ---------------------------------------------------------------------------
// The sea
// ---------------------------------------------------------------------------

fn water(q: vec2<f32>, depth: f32, sun: f32, l: Leg) -> vec3<f32> {
    // Looking up the water lightens toward the surface, looking down it darkens.
    var colour = water_tone(depth - q.y * 0.45 + 0.04);
    // Polar water is clear and blue; the lava cave's is black and smoky red.
    let arctic = presence(l, ARCTIC);
    colour = mix(colour, vec3<f32>(0.05, 0.22, 0.32) * (0.70 + 0.60 * soft(-0.30, 0.40, q.y)), arctic * 0.75);
    let cave = presence(l, CAVE);
    colour = mix(colour, vec3<f32>(0.030, 0.012, 0.010) * (1.0 + 1.8 * soft(0.10, -0.40, q.y)), cave * 0.85);
    let murk = fbm(vec2<f32>(q.x + CAM * 0.25, q.y) * vec2<f32>(2.4, 3.2) + vec2<f32>(SEED * 23.0, -T * 0.018));
    colour *= 0.85 + 0.30 * murk;

    if (sun > 0.02) {
        // Sunbeams hang in the middle distance, so they drift at a third of the
        // seabed's pace.
        let down = max(0.40 - q.y, 0.0);
        let slant = (q.x + CAM / 3.0 + down * 0.58) * 12.0 + 1.4 * sin(q.x * 3.1 + T * 0.08);
        let ray_a = pow(max(sin(slant + 1.1 * sin(down * 2.3 + T * 0.13)), 0.0), 6.0);
        let ray_b = pow(max(sin(slant * 1.73 + 2.1), 0.0), 12.0);
        let shafts = (ray_a * 0.62 + ray_b * 0.38) * exp(-down * 1.6);
        colour += vec3<f32>(0.13, 0.39, 0.37) * shafts * 0.85 * sun;
    }
    return colour;
}

// The surface seen from below: past the angle of total reflection it is a
// rippling silver ceiling that recedes to the horizon like the seabed does.
fn surface_layer(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let above = DEPTH;
    if (q.y <= 0.0 || above > 6.0) {
        return col;
    }
    let z = above / q.y;
    if (z > 14.0) {
        return col;
    }
    let p = vec2<f32>(CAM + q.x * z, z);
    let ripple = sin(p.x * 9.0 + T * 0.7 + 1.5 * sin(p.y * 5.0 - T * 0.4)) * sin(p.y * 7.0 + T * 0.5 + 1.2 * sin(p.x * 4.0));
    let bright = caustic(p * vec2<f32>(5.0, 6.0) + vec2<f32>(T * 0.2, -T * 0.1), T * 0.6);
    let silver = vec3<f32>(0.45, 0.78, 0.74) * (0.78 + 0.22 * ripple) + vec3<f32>(0.35, 0.45, 0.40) * bright * exp(-z * 0.15);
    let fog = 1.0 - exp(-z * s.fogk * 0.8);
    let edge = soft(0.0, 0.01, q.y);
    return mix(col, mix(silver * s.sun * 1.3, col, fog), edge);
}

// The far distance: two ranges of mountains, and in Atlantis the volcano whose
// fire lit the ruined city for Aronnax and Nemo.
fn far_layer(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    for (var r = 0; r < 2; r = r + 1) {
        let z = select(14.0, 8.0, r == 1);
        let x = CAM + q.x * z;
        let abyss = presence(leg(CAM + q.x * 4.0, SEED), ABYSS);
        let n = 0.62 * vnoise(vec2<f32>(x * 0.35, f32(r) * 7.0 + SEED * 5.1)) + 0.38 * vnoise(vec2<f32>(x * 0.9, f32(r) * 3.0 + SEED * 2.3));
        var height = 0.3 + 1.1 * n + abyss * 0.8 * (vnoise(vec2<f32>(x * 2.2, 2.0)) - 0.3);

        var glow = 0.0;
        var halo = 0.0;
        let atlantis = presence(leg(CAM + q.x * 4.0, SEED), ATLANTIS);
        if (shown(bit(ATLANTIS)) && r == 0 && atlantis > 0.01) {
            let spacing = 16.0;
            let cell = floor(x / spacing);
            let vx = (cell + 0.5) * spacing + (hash11(cell * 3.7 + SEED) - 0.5) * 4.0;
            let dx = x - vx;
            let cone = 2.6 - abs(dx) * 0.62 + 0.15 * vnoise(vec2<f32>(x * 3.0, 1.0));
            let crater = 2.45 - 0.3 * soft(0.5, 0.0, abs(dx));
            height = max(height, min(cone, crater) * atlantis);
            let peak = 2.45 - s.floor;
            let vent = vec2<f32>(dx, q.y * z - peak) / z;
            let flicker = 0.80 + 0.20 * sin(T * 2.3 + cell) * sin(T * 1.3);
            halo = (exp(-length(vent * vec2<f32>(1.0, 1.6)) * 5.5) * 0.55 + exp(-dot(vent, vent) * 400.0) * 0.6) * atlantis * flicker;
            glow = exp(-dot(vent * vec2<f32>(1.0, 3.0), vent * vec2<f32>(1.0, 3.0)) * 2500.0) * atlantis * flicker;
            // A thread of lava down one flank, broken where it has crusted over.
            let wy = q.y * z + s.floor;
            let lava_x = dx * sign(hash11(cell * 1.9) - 0.5) - (2.45 - wy) / 0.62 * 0.6 - 0.12 * sin(wy * 5.0 + cell);
            let lava = soft(0.05, 0.0, abs(lava_x)) * soft(2.45, 2.2, wy) * soft(0.0, 0.6, wy)
                * (0.3 + 0.7 * vnoise(vec2<f32>(wy * 4.0 - T * 0.2, cell)));
            glow += lava * atlantis * 0.9 * flicker;
        }

        // The volcano's glow lights the water behind the mountains, so they stand
        // out against it; the crater and the lava burn in front.
        col += vec3<f32>(0.80, 0.26, 0.06) * halo;
        let top = (height - s.floor) / z;
        // Inside the cave its walls close off the distance.
        let walled = 1.0 - presence(leg(CAM + q.x * 4.0, SEED), CAVE);
        let mask = (1.0 - soft(top - PX0 * 2.0, top + PX0 * 2.0, q.y)) * walled;
        let fog = 1.0 - exp(-z * s.fogk * mix(0.55, 0.95, s.sun));
        col = mix(col, mix(vec3<f32>(0.015, 0.05, 0.060) * (s.sun * 0.8 + 0.2), s.water, fog), mask * 0.9);
        col += vec3<f32>(1.0, 0.40, 0.10) * glow;
    }
    return col;
}

// Where the Nautilus has a roof over it: the vault of the lava cave, lit red
// from below, or the underside of the polar ice, glowing where daylight comes
// through and bright along the open leads. Like the seabed, it recedes to the
// horizon.
fn roof_layer(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = s.ceil_z;
    if (z > 60.0) {
        return col;
    }
    let env = env_at(s, q, z);
    let p = vec2<f32>(CAM + q.x * z, z);
    let l = here(p.x, z);
    let detail = exp(-z * 0.35);
    let ice = presence(l, ARCTIC);
    let cave = presence(l, CAVE);
    if (ice > cave) {
        // Floes of differing thickness, so differing brightness; open leads
        // between them run long and nearly straight, and let the daylight in.
        let floes = vnoise(p * vec2<f32>(0.9, 1.3));
        let along = vnoise(vec2<f32>(p.x * 0.35, p.y * 0.08) + 3.0);
        let lead = soft(0.06, 0.02, abs(along - 0.5));
        let bumps = fbm(p * 6.0);
        var c = vec3<f32>(0.30, 0.56, 0.64) * (0.65 + 0.35 * bumps * detail + 0.45 * floes);
        c = mix(c, vec3<f32>(0.70, 0.95, 1.00), lead);
        c *= 0.70 + 0.9 * s.sun;
        col = mix(col, mix(c, env.water, env.fog), soft(0.35, 0.65, ice));
    } else {
        let rock = fbm(p * 5.0);
        let flicker = 0.85 + 0.15 * sin(T * 1.9 + p.x) * sin(T * 1.1);
        var c = vec3<f32>(0.08, 0.07, 0.07) * (0.55 + 0.7 * rock) * (env.light + 0.15);
        c += vec3<f32>(0.35, 0.08, 0.02) * soft(0.55, 0.80, rock) * flicker;
        col = mix(col, mix(c, env.water, env.fog), soft(0.35, 0.65, cave));
    }
    return col;
}

// Distance along this pixel's ray to the seabed: exact for a level floor, then
// refined once for the floor's undulation.
fn floor_distance(q: vec2<f32>, s: Sea) -> f32 {
    if (q.y > -0.002) {
        return 1000.0;
    }
    var z = s.floor / -q.y;
    z = floor_height(CAM + q.x * z, z) / -q.y;
    return min(z, 1000.0);
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

fn floor_layer(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = s.floor_z;
    if (z > 60.0) {
        return col;
    }
    let env = env_at(s, q, z);
    let p = vec2<f32>(CAM + q.x * z, z);
    let l = here(p.x, z);
    var albedo = floor_albedo(l);
    // Detail fades out with distance before it can shimmer.
    let detail = exp(-z * 0.35);
    let grit = fbm(p * vec2<f32>(9.0, 9.0) + vec2<f32>(SEED * 17.0, 1.3));
    albedo *= mix(1.0, 0.60 + 0.55 * grit, detail);
    // Sand ripples in the shallows, crossing the view at an angle.
    let sand = presence(l, SHALLOWS) + presence(l, FOREST) * 0.5;
    if (sand > 0.0) {
        let ripple = sin((p.x * 0.8 + p.y) * 26.0 + vnoise(p * 3.0) * 4.0);
        albedo *= 1.0 - 0.18 * sand * detail * soft(0.3, 1.0, ripple);
    }
    // The pavements of Atlantis.
    let paving = presence(l, ATLANTIS);
    if (paving > 0.0) {
        let tiles = p * vec2<f32>(3.0, 4.0);
        let row = floor(tiles.y);
        let joint = min(abs(fract(tiles.x + row * 0.5) - 0.5), abs(fract(tiles.y) - 0.5));
        albedo *= 1.0 - paving * 0.45 * soft(0.05, 0.0, 0.5 - joint) * soft(0.0, 6.0, 8.0 - z);
    }
    // Shading from the slope of the floor.
    // The height here is known from the ray, so one more sample gives the slope.
    let slope = floor_height(p.x + 0.08, z) - z * -q.y;
    albedo *= 1.0 + clamp(slope * 9.0, -0.4, 0.4);
    var shade = albedo * env.light;
    if (s.sun > 0.02) {
        let ca = caustic(p * vec2<f32>(7.0, 9.0) + vec2<f32>(T * 0.3, 0.0), T * 0.9);
        shade += vec3<f32>(0.20, 0.42, 0.38) * ca * s.sun * 0.55 * detail;
    }
    var floor_col = mix(shade, env.water, env.fog);
    // The cave floor is a crust of basalt over molten rock: veins of lava glow
    // through its cracks and it pools in the hollows.
    let cave = presence(l, CAVE);
    if (shown(bit(CAVE)) && cave > 0.0) {
        let flicker = 0.80 + 0.20 * sin(T * 1.7 + p.x * 3.0) * sin(T * 2.9 + p.y * 2.0);
        let veins = soft(0.040, 0.0, abs(vnoise(p * vec2<f32>(1.4, 1.9)) - 0.5)) * (0.4 + 0.6 * detail);
        let fine = soft(0.030, 0.0, abs(vnoise(p * vec2<f32>(4.0, 5.0) + 9.0) - 0.5)) * detail;
        let pools = soft(0.66, 0.74, vnoise(p * 0.55 + 7.0));
        let heat = (veins * 0.9 + fine * 0.5 + pools * 1.4) * flicker * cave;
        floor_col += shine(vec3<f32>(1.0, 0.36, 0.07) * heat, env);
    }
    return floor_col;
}

// ---------------------------------------------------------------------------
// Features on the seabed
// ---------------------------------------------------------------------------
//
// Each takes `w`, the pixel's position in world units at the distance `z` of its
// row, and `salt`, which keeps the rows from repeating one another.

// Bed something into the seabed so it does not stand on it like a cut-out: its
// foot darkens where little light reaches, then a drift of the floor's own sand,
// `hw` wide and `h` high, heaps over it with a ragged, feathered edge. `cover`
// is the thing's own coverage at this pixel.
fn settle(col_in: vec3<f32>, cover: f32, w: vec2<f32>, cx: f32, base: f32, hw: f32, h: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let dx = (w.x - cx) / hw;
    let lift = w.y - base;
    if (abs(dx) > 1.0 || lift > h + 0.08 || lift < -h) {
        return col;
    }
    let ao = cover * soft(h + 0.07, h * 0.4, lift);
    col *= 1.0 - 0.45 * ao * (1.0 - env.fog);
    let drift = h * (1.0 - dx * dx) * (0.70 + 0.60 * vnoise(vec2<f32>(w.x * 45.0, cx * 7.0)));
    let edge = env.px * 1.5 + h * 0.35;
    let sand = (1.0 - soft(drift - edge, drift + env.px, lift)) * soft(-h, 0.0, lift) * soft(1.0, 0.6, abs(dx));
    // The same grit as the floor around it, so the drift reads as part of it.
    let grain = 0.72 + 0.30 * vnoise(w * 90.0);
    let albedo = floor_albedo(here(w.x, env.px / PX0)) * grain;
    return mix(col, lit(albedo, env), sand);
}

fn rocks(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let s = 0.45;
    let c0 = floor(w.x / s - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        if (hash11(cell * 6.3 + salt) < 0.35) {
            continue;
        }
        let cx = (cell + 0.5 + (hash11(cell * 1.7 + salt) - 0.5) * 0.5) * s;
        let stone = vec2<f32>(0.06 + hash11(cell * 2.7 + salt) * 0.13, 0.04 + hash11(cell * 8.9 + salt) * 0.08);
        let p = vec2<f32>(w.x - cx, w.y - ground(cx, z) + stone.y * 0.25);
        if (abs(p.x) < stone.x * 1.3 && abs(p.y) < stone.y * 1.3) {
            let rd = ellm(p, stone) + (vnoise(p * 30.0 + cell) - 0.5) * 0.02;
            let mask = fill(rd, env.px);
            let top = soft(-0.4, 0.8, p.y / stone.y);
            let albedo = vec3<f32>(0.20, 0.21, 0.20) * (0.45 + 0.85 * top) * (0.8 + 0.2 * vnoise(p * 80.0));
            col = mix(col, lit(albedo, env), mask);
            // In the lava cave these are pillows of lava, molten through their cracks.
            let molten = presence(here(cx, z), CAVE);
            if (molten > 0.0) {
                let crack = soft(0.05, 0.0, abs(vnoise(p * 28.0 + cell) - 0.5)) * soft(0.4, -0.6, p.y / stone.y);
                col += shine(vec3<f32>(1.0, 0.38, 0.08) * crack * mask * molten * (0.8 + 0.2 * sin(T * 2.0 + cell)), env);
            }
            col = settle(col, mask, w, cx, ground(cx, z), stone.x * 1.5, stone.y * 0.35, env);
        }
    }
    return col;
}

fn kelp(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env, sun: f32) -> vec3<f32> {
    var col = col_in;
    let s = 0.16;
    let c0 = floor(w.x / s) - 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5 + (hash11(cell * 2.3 + salt) - 0.5) * 0.8) * s;
        let l = here(cx, z);
        let here = (presence(l, SHALLOWS) + 0.4 * presence(l, FOREST)) * 0.75;
        if (here > hash11(cell * 5.1 + salt) * 0.85 + 0.1) {
            let base = ground(cx, z);
            let height = 0.45 + hash11(cell * 9.3 + salt) * 1.0;
            let y = w.y - base;
            if (y > -0.02 && y < height + 0.05) {
                let up = clamp(y / height, 0.0, 1.0);
                let sway = (sin(y * 6.0 - T * 0.9 + cell) * 0.05 + sin(y * 2.3 + T * 0.5 + cell * 1.7) * 0.03) * up;
                // Blades on alternate sides of the stipe.
                let blade = sin(y * 26.0 + cell);
                let width = 0.004 + 0.006 * pow(max(blade, 0.0), 2.0) * (0.4 + up);
                let offset = sign(blade) * 0.005 * (0.4 + up);
                let strand = fill(abs(w.x - cx - sway - offset) - width, env.px)
                    * soft(-0.02, 0.01, y)
                    * soft(height + 0.03, height - 0.03, y);
                let albedo = mix(vec3<f32>(0.26, 0.30, 0.10), vec3<f32>(0.52, 0.46, 0.16), up * max(blade, 0.0));
                col = mix(col, lit(albedo, env) + vec3<f32>(0.10, 0.14, 0.03) * sun * up * (1.0 - env.fog), strand * 0.95);
                col = settle(col, strand, w, cx, base, 0.035, 0.012, env);
            }
        }
    }
    return col;
}

// The submarine forest of Crespo Island: tall algae that grow straight up like
// trees, branching in pairs and crowned with fronds. Drawn in the units of the
// old screen-sized tree, scaled up to world size.
fn trees(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let k = 2.3;
    let s = 0.46;
    let c0 = floor(w.x / s) - 1.0;
    let px = env.px / k;
    for (var i = 0; i < 3; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5 + (hash11(cell * 4.9 + salt) - 0.5) * 0.6) * s;
        let here = presence(here(cx, z), FOREST);
        if (here > hash11(cell * 8.1 + salt) * 0.7 + 0.15) {
            let base = ground(cx, z);
            let h = 0.36 + hash11(cell * 3.3 + salt) * 0.34;
            let p = vec2<f32>(w.x - cx, w.y - base) / k;
            if (abs(p.x) < 0.14 && p.y > -0.01 && p.y < h + 0.07) {
                let lean = (hash11(cell * 6.6 + salt) - 0.5) * 0.08;
                let sway = sin(T * 0.35 + cell) * 0.008;
                let tx = (lean + sway) * p.y / h;
                var d = abs(p.x - tx) - (0.010 * (1.0 - 0.6 * clamp(p.y / h, 0.0, 1.0)) + 0.002);
                d = max(d, p.y - h);
                var leaves = 1.0;
                for (var b = 0; b < 4; b = b + 1) {
                    let fb = f32(b);
                    let by = h * (0.34 + 0.17 * fb);
                    let len = 0.10 - fb * 0.014;
                    // Only the branches at this pixel's height can reach it.
                    if (p.y < by - 0.03 || p.y > by + len * 0.65 + 0.04) {
                        continue;
                    }
                    let side = select(-1.0, 1.0, (b % 2) == 0) * select(1.0, -1.0, hash11(cell) > 0.5);
                    let root = vec2<f32>((lean + sway) * by / h, by);
                    let tip = root + vec2<f32>(side * len, len * 0.65 + sin(T * 0.5 + fb + cell) * 0.006);
                    d = min(d, segment(p, root, tip) - 0.0035);
                    leaves = min(leaves, ellm(p - tip, vec2<f32>(0.038, 0.022)));
                }
                if (p.y > h - 0.03) {
                    let crown = vec2<f32>(lean + sway, h + 0.02);
                    leaves = min(leaves, ellm(p - crown, vec2<f32>(0.055, 0.040)) * 1.2);
                }
                // One lookup roughens the fronds, and only near one.
                var ragged = 0.0;
                if (leaves < 0.03) {
                    ragged = vnoise(p * 90.0) - 0.5;
                    leaves += ragged * 0.018;
                }
                let trunk = fill(d, px);
                let leaf = fill(leaves, px * 1.5);
                let red = vec3<f32>(0.46, 0.17, 0.12);
                let olive = vec3<f32>(0.36, 0.34, 0.14);
                let tint = mix(red, olive, hash11(cell * 2.9 + salt));
                col = mix(col, lit(tint * 0.6, env), trunk);
                col = mix(col, lit(tint * (0.8 + 0.8 * ragged), env), leaf * 0.92);
                col = settle(col, trunk, w, cx, base, 0.11, 0.025, env);
            }
        }
    }
    return col;
}

// Sea fans and a giant clam holding its pearl.
fn garden(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let k = 1.7;
    let s = 0.65;
    let px = env.px / k;
    let c0 = floor(w.x / s - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5 + (hash11(cell * 7.3 + salt) - 0.5) * 0.5) * s;
        let l = here(cx, z);
        let here = presence(l, SHALLOWS) + presence(l, FOREST) + presence(l, ARCTIC);
        if (here < hash11(cell * 3.9 + salt) * 0.8 + 0.1) {
            continue;
        }
        let p = vec2<f32>(w.x - cx, w.y - ground(cx, z)) / k - vec2<f32>(0.0, 0.002);
        if (abs(p.x) > 0.08 || p.y < -0.02 || p.y > 0.09) {
            continue;
        }
        if (presence(l, ARCTIC) > 0.5) {
            if (hash11(cell * 2.2 + salt) > 0.5) {
                // A clump of glass sponges, pale tubes open at the top.
                var tubes = 1.0;
                var mouths = 1.0;
                for (var j = 0; j < 3; j = j + 1) {
                    let fj = f32(j);
                    let tx = (fj - 1.0) * 0.018 + (hash11(fj + cell) - 0.5) * 0.006;
                    let th = 0.030 + 0.035 * hash11(fj * 3.1 + cell);
                    tubes = min(tubes, box(p - vec2<f32>(tx, th * 0.5), vec2<f32>(0.007 + 0.002 * fj, th * 0.5), 0.004));
                    mouths = min(mouths, ellm(p - vec2<f32>(tx, th), vec2<f32>(0.006, 0.0025)));
                }
                let sponge = fill(tubes, px);
                col = mix(col, lit(vec3<f32>(0.80, 0.74, 0.52) * (0.8 + 0.3 * vnoise(p * 300.0)), env), sponge);
                col = mix(col, lit(vec3<f32>(0.20, 0.16, 0.10), env), fill(mouths, px) * sponge);
                col = settle(col, sponge, w, cx, ground(cx, z), 0.08, 0.008, env);
            } else {
                // A starfish on the sand, seen at a low angle.
                let sp = vec2<f32>(p.x, (p.y - 0.003) / 0.35);
                let a = atan2(sp.y, sp.x) + hash11(cell) * TAU;
                let arms = 0.024 * (0.35 + 0.65 * pow(abs(cos(a * 2.5)), 3.0));
                let star = fill((length(sp) - arms) * 0.35, px);
                col = mix(col, lit(vec3<f32>(0.85, 0.30, 0.16) * (0.8 + 0.3 * vnoise(p * 400.0)), env), star);
            }
            continue;
        }
        if (hash11(cell * 1.3 + salt * 4.0) > 0.35 || presence(l, FOREST) < 0.5) {
            // A sea fan.
            let fr = length(p);
            let fa = atan2(p.y, p.x);
            let fan = soft(0.064, 0.044, fr) * soft(-0.01, 0.02, p.y);
            let ribs = pow(abs(sin(fa * 9.0 + 0.6)), 10.0) * soft(0.072, 0.010, fr) * soft(0.0, 0.016, p.y);
            let hue = mix(vec3<f32>(0.85, 0.28, 0.30), vec3<f32>(0.90, 0.58, 0.20), hash11(cell * 5.5 + salt));
            col = mix(col, lit(hue * 0.55, env), fan * 0.55);
            col = mix(col, lit(hue, env), ribs * 0.8);
            col = settle(col, max(fan, ribs), w, cx, ground(cx, z), 0.10, 0.012, env);
        } else {
            // A tridacna, gaping slowly, the pearl in its throat.
            let open = 0.30 + 0.18 * sin(T * 0.25 + cell);
            let shell_r = 0.050;
            let lower = max(ellm(p, vec2<f32>(shell_r, shell_r * 0.55)), p.y);
            let up = rot(-open) * (p - vec2<f32>(-shell_r, 0.0)) + vec2<f32>(shell_r, 0.0);
            let upper = max(ellm(up, vec2<f32>(shell_r, shell_r * 0.55)), -up.y);
            let ridges = pow(abs(sin(atan2(p.y + 0.02, p.x) * 11.0)), 2.0);
            let shell = fill(min(lower, upper) - 0.0015 * ridges, px);
            let mantle = fill(ellm(p - vec2<f32>(0.0, 0.004), vec2<f32>(shell_r * 0.85, 0.010 + open * 0.03)), px * 2.0);
            let pearl_p = p - vec2<f32>(0.006, 0.008);
            let pearl = fill(length(pearl_p) - 0.010, px);
            col = mix(col, lit(vec3<f32>(0.55, 0.53, 0.47) * (0.75 + 0.25 * ridges), env), shell);
            col = mix(col, lit(vec3<f32>(0.20, 0.32, 0.45), env) + shine(vec3<f32>(0.02, 0.05, 0.08), env), mantle * 0.9);
            let sheen = 0.6 + 0.4 * exp(-dot(pearl_p - vec2<f32>(-0.003, 0.003), pearl_p - vec2<f32>(-0.003, 0.003)) * 80000.0);
            col = mix(col, lit(vec3<f32>(0.85, 0.82, 0.76) * sheen, env) + shine(vec3<f32>(0.20), env), pearl);
            col += shine(vec3<f32>(0.55, 0.60, 0.55) * exp(-dot(pearl_p, pearl_p) * 3500.0) * 0.18, env);
            col = settle(col, shell, w, cx, ground(cx, z), 0.11, 0.018, env);
        }
    }
    return col;
}

// The Spanish galleons sunk in Vigo Bay in 1702, and the gold Nemo took from
// them: thirty metres of ship, so only the far rows hold one.
fn galleon(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let s = 3.6;
    let c0 = floor(w.x / s - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5 + (hash11(cell * 5.3 + salt) - 0.5) * 0.3) * s;
        if (presence(here(cx, z), WRECKS) < 0.45 || hash11(cell * 2.1 + salt) < 0.2) {
            continue;
        }
        let scale = 2.6 + hash11(cell * 3.1 + salt) * 0.8;
        let base_y = ground(cx, z) + 0.070 * scale;
        let tilt = -0.10 + (hash11(cell * 8.7 + salt) - 0.5) * 0.18;
        let wl = rot(tilt) * vec2<f32>(w.x - cx, w.y - base_y) / scale;
        if (abs(wl.x) > 0.38 || wl.y < -0.12 || wl.y > 0.29) {
            continue;
        }
        let px = env.px / scale;

        let hull_main = ell(wl - vec2<f32>(-0.010, -0.038), vec2<f32>(0.215, 0.040)) * 0.04;
        let hull_deck = box(wl - vec2<f32>(-0.010, 0.000), vec2<f32>(0.205, 0.018), 0.008);
        let hull_bow = triangle(wl, vec2<f32>(0.170, 0.020), vec2<f32>(0.295, -0.005), vec2<f32>(0.170, -0.065));
        let hull_stern = box(wl - vec2<f32>(-0.195, 0.005), vec2<f32>(0.055, 0.045), 0.015);
        var hull_shape = min(hull_main, min(hull_deck, min(hull_bow, hull_stern)));
        hull_shape = max(hull_shape, wl.y - 0.018);
        let hole = ell(wl - vec2<f32>(-0.035, -0.015), vec2<f32>(0.040, 0.025)) + (fbm(wl * 14.0 + salt * 3.0) - 0.5) * 0.25;
        let hull = fill(hull_shape, px) * (1.0 - soft(0.015, -0.008, hole) * 0.85);
        let castles = min(
            min(box(wl - vec2<f32>(-0.020, 0.040), vec2<f32>(0.055, 0.022), 0.006), box(wl - vec2<f32>(-0.165, 0.045), vec2<f32>(0.050, 0.028), 0.006)),
            box(wl - vec2<f32>(0.135, 0.036), vec2<f32>(0.045, 0.018), 0.006),
        );
        let spars = min(
            min(segment(wl, vec2<f32>(-0.015, 0.020), vec2<f32>(-0.035, 0.245)), segment(wl, vec2<f32>(0.125, 0.020), vec2<f32>(0.155, 0.200))),
            min(segment(wl, vec2<f32>(-0.140, 0.205), vec2<f32>(0.100, 0.215)), segment(wl, vec2<f32>(0.220, 0.020), vec2<f32>(0.330, 0.075))),
        );
        let rigging = min(segment(wl, vec2<f32>(-0.035, 0.245), vec2<f32>(0.230, 0.020)), segment(wl, vec2<f32>(-0.035, 0.245), vec2<f32>(-0.250, 0.020)));
        let sp = wl - vec2<f32>(-0.020, 0.135);
        let sail_shape = max(max(sp.x - 0.110, -sp.x - 0.020), max(sp.y - 0.050, -sp.y - 0.075));
        let sail = fill(sail_shape, px * 3.0) * (0.35 + 0.65 * fbm(wl * 18.0 + salt * 7.0)) * soft(0.2, 0.8, vnoise(wl * 30.0 + cell));
        let planks = pow(abs(sin(wl.y * 95.0 + wl.x * 2.0)), 12.0);

        let wood = vec3<f32>(0.30, 0.22, 0.15) * (0.75 + 0.25 * planks) * (0.7 + 0.3 * vnoise(wl * 50.0));
        col = mix(col, lit(wood, env), hull);
        col = mix(col, lit(wood * 0.9, env), fill(castles, px));
        col = mix(col, lit(vec3<f32>(0.24, 0.20, 0.15), env), fill(spars - 0.0045, px));
        col = mix(col, lit(vec3<f32>(0.30, 0.30, 0.26), env), fill(rigging - 0.0016, px) * 0.8);
        col = mix(col, lit(vec3<f32>(0.42, 0.40, 0.32), env), sail * 0.45);

        // A split chest by the bow, spilling gold over the sand.
        let chest_p = wl - vec2<f32>(0.20, -0.064);
        let chest = box(chest_p, vec2<f32>(0.020, 0.011), 0.002);
        let lid = box(rot(0.6) * (chest_p - vec2<f32>(-0.020, 0.012)) - vec2<f32>(0.020, 0.003), vec2<f32>(0.020, 0.003), 0.001);
        col = mix(col, lit(vec3<f32>(0.26, 0.18, 0.10), env), fill(min(chest, lid), px));
        let coins = fill(ellm(chest_p - vec2<f32>(0.040, -0.010), vec2<f32>(0.030, 0.0035)) + (vnoise(chest_p * 400.0) - 0.5) * 0.002, px);
        let hoard = fill(ellm(chest_p - vec2<f32>(0.0, 0.011), vec2<f32>(0.017, 0.004)), px);
        let gold = max(hoard, coins);
        let twinkle = pow(hash21(floor(chest_p * 900.0) + floor(T * 3.0)), 18.0);
        col = mix(col, lit(vec3<f32>(0.80, 0.58, 0.20), env), gold);
        col += shine(vec3<f32>(1.0, 0.85, 0.45) * twinkle * gold * 1.4, env);
        // Two centuries of silt have half buried the hull.
        col = settle(col, hull, w, cx, ground(cx, z), 0.34 * scale, 0.045 * scale, env);
    }
    return col;
}

// A temple front of Atlantis, half its columns down. Fifteen metres high.
fn temple(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let s = 3.2;
    let cell = floor(w.x / s);
    let cx = (cell + 0.5 + (hash11(cell * 9.1 + salt) - 0.5) * 0.3) * s;
    if (presence(here(cx, z), ATLANTIS) < 0.4 || hash11(cell * 5.7 + salt) < 0.3) {
        return col;
    }
    let scale = 1.5 + hash11(cell * 2.1 + salt) * 0.4;
    let l = vec2<f32>(w.x - cx, w.y - ground(cx, z)) / scale;
    if (abs(l.x) > 0.62 || l.y < -0.1 || l.y > 0.90) {
        return col;
    }
    let px = env.px / scale;
    let steps = min(box(l - vec2<f32>(0.0, 0.02), vec2<f32>(0.56, 0.03), 0.0), box(l - vec2<f32>(0.0, 0.07), vec2<f32>(0.50, 0.025), 0.0));
    var d = steps;
    for (var i = 0; i < 6; i = i + 1) {
        let fi = f32(i);
        let broken = hash11(fi * 7.3 + cell * 3.1 + salt);
        let h = select(0.50, 0.12 + broken * 0.30, broken < 0.35);
        let x = -0.40 + fi * 0.16;
        d = min(d, box(l - vec2<f32>(x, 0.10 + h * 0.5), vec2<f32>(0.028 - 0.004 * (l.y - 0.1), h * 0.5), 0.0));
    }
    let gap = hash11(cell * 6.1 + salt) * 0.6 - 0.3;
    let beam = max(box(l - vec2<f32>(0.0, 0.645), vec2<f32>(0.47, 0.045), 0.0), -box(l - vec2<f32>(gap, 0.64), vec2<f32>(0.07, 0.06), 0.0));
    let pediment = max(
        triangle(l, vec2<f32>(-0.50, 0.69), vec2<f32>(0.50, 0.69), vec2<f32>(0.0, 0.86)),
        -box(l - vec2<f32>(gap + 0.12, 0.80), vec2<f32>(0.14, 0.12), 0.0),
    );
    d = min(d, min(beam, pediment));
    let mask = fill(d, px);
    let stone = vec3<f32>(0.46, 0.45, 0.41) * (0.8 + 0.2 * vnoise(l * 20.0)) * (0.75 + 0.25 * soft(-0.3, 0.3, l.x));
    col = mix(col, lit(stone, env), mask);
    col = settle(col, mask, w, cx, ground(cx, z), 0.62 * scale, 0.035 * scale, env);
    return col;
}

// The colonnades of Atlantis: fluted Doric columns, many of them broken off,
// with fallen drums at their feet. Drawn in the units of the old screen-sized
// column, scaled up to world size.
fn columns(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let k = 2.6;
    let s = 0.30;
    let px = env.px / k;
    let c0 = floor(w.x / s - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        let here = presence(here(cell * s, z), ATLANTIS);
        if (here < hash11(cell * 2.7 + salt) * 0.6 + 0.2 || hash11(cell * 9.9 + salt) < 0.25) {
            continue;
        }
        let cx = (cell + 0.5) * s;
        let base = ground(cx, z);
        let broken = hash11(cell * 4.4 + salt * 2.0);
        var h = 0.20 + hash11(cell * 1.9 + salt) * 0.09;
        if (broken < 0.55) {
            h *= 0.35 + broken;
        }
        let p = vec2<f32>(w.x - cx, w.y - base) / k;
        if (abs(p.x) > 0.07 || p.y < -0.01 || p.y > h + 0.03) {
            continue;
        }
        let half = 0.017 - 0.003 * clamp(p.y / 0.3, 0.0, 1.0);
        var top = h;
        if (broken < 0.55) {
            top = h + (vnoise(vec2<f32>(p.x * 160.0, cell)) - 0.5) * 0.03;
        }
        let shaft = max(abs(p.x) - half, max(p.y - top, -p.y));
        let plinth = box(p - vec2<f32>(0.0, 0.008), vec2<f32>(0.026, 0.008), 0.002);
        var d = min(shaft, plinth);
        if (broken >= 0.55) {
            let echinus = max(ellm(p - vec2<f32>(0.0, h), vec2<f32>(0.026, 0.010)), -(p.y - h));
            let abacus = box(p - vec2<f32>(0.0, h + 0.014), vec2<f32>(0.030, 0.006), 0.001);
            d = min(d, min(echinus, abacus));
        } else {
            let drum = box(rot(0.08) * (p - vec2<f32>(0.045, 0.012)), vec2<f32>(0.024, 0.014), 0.003);
            d = min(d, drum);
        }
        let mask = fill(d, px);
        // Pixels beside the column still need its drift of sand.
        if (mask <= 0.0 && p.y > 0.05) {
            continue;
        }
        let n = clamp(p.x / half, -1.0, 1.0);
        let round = sqrt(max(1.0 - n * n, 0.0));
        let flutes = 0.82 + 0.18 * abs(cos(n * PI * 3.0));
        let light_side = 0.55 + 0.45 * soft(0.6, -0.6, n) * round;
        let stain = soft(0.45, 0.85, vnoise(p * vec2<f32>(260.0, 110.0) + cell)) * soft(0.25, 0.0, p.y / max(h, 0.01) - 0.2);
        let albedo = mix(vec3<f32>(0.66, 0.63, 0.56), vec3<f32>(0.30, 0.36, 0.24), stain * 0.55) * flutes * light_side;
        col = mix(col, lit(albedo, env), mask);
        col = settle(col, mask, w, cx, base, 0.13, 0.022, env);
    }
    return col;
}

// Black smokers, in the abyss only.
fn vents(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let k = 2.5;
    let s = 2.2;
    let cell = floor(w.x / s);
    let cx = (cell + 0.5 + (hash11(cell * 4.1 + salt) - 0.5) * 0.4) * s;
    let l = here(cx, z);
    if (presence(l, ABYSS) + presence(l, CAVE) < 0.5 || hash11(cell * 3.3 + salt) < 0.4 || abs(w.x - cx) > 0.30) {
        return col;
    }
    let p = vec2<f32>(w.x - cx, w.y - ground(cx, z)) / k;
    let chimney_d = length(vec2<f32>(p.x * 5.2, p.y)) - 0.050;
    let chimney = soft(0.10, -0.04, chimney_d) * soft(0.11, 0.03, p.y);
    let plume_y = max(p.y - 0.035, 0.0);
    let plume_x = sin(plume_y * 35.0 - T * 1.6 + cell) * 0.007 * (0.4 + plume_y * 6.0);
    let plume = exp(-abs(p.x - plume_x) * 46.0) * exp(-plume_y * 7.5) * soft(0.0, 0.03, plume_y);
    col = mix(col, lit(vec3<f32>(0.10, 0.10, 0.10), env), chimney);
    col = settle(col, chimney, w, cx, ground(cx, z), 0.16, 0.02, env);
    col += shine(vec3<f32>(1.0, 0.44, 0.13) * plume * 0.55, env);
    col += shine(vec3<f32>(0.30, 0.13, 0.04) * exp(-length(p) * 18.0) * 0.32, env);
    return col;
}

// The basalt of the lava cave: clustered hexagonal columns, cracked where
// they cooled, red where the lava lights them.
fn basalt(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env) -> vec3<f32> {
    var col = col_in;
    let s = 0.11;
    let c0 = floor(w.x / s - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5) * s;
        let cluster = vnoise(vec2<f32>(cell * 0.35, salt));
        if (presence(here(cx, z), CAVE) < 0.5 || cluster < 0.60) {
            continue;
        }
        let base = ground(cx, z);
        let h = 0.04 + 0.32 * soft(0.60, 0.95, cluster) * (0.6 + 0.4 * hash11(cell * 3.7 + salt));
        let hw = s * 0.48;
        let slant = (hash11(cell * 9.1 + salt) - 0.5) * 0.4;
        let p = vec2<f32>(w.x - cx, w.y - base);
        let top = h + p.x * slant;
        let d = max(abs(p.x) - hw, p.y - top);
        let mask = fill(d, env.px);
        if (mask > 0.0) {
            // Three faces of the hexagonal prism show, lit, half lit and dark;
            // cooling cracks run across them at random.
            let u = p.x / hw;
            let face = select(select(0.50, 0.75, u < 0.35), 1.0, u < -0.35);
            let crack = soft(0.05, 0.0, abs(vnoise(vec2<f32>(p.x * 8.0 + cell * 5.0, p.y * 22.0)) - 0.5));
            let albedo = vec3<f32>(0.11, 0.10, 0.10) * face * (1.0 - 0.4 * crack) * (0.85 + 0.3 * vnoise(p * 60.0));
            col = mix(col, lit(albedo, env), mask);
            col *= 1.0 - 0.6 * fill(abs(abs(p.x) - hw) - env.px * 0.5, env.px) * mask;
        }
        col = settle(col, mask, w, cx, base, s * 0.7, 0.012, env);
    }
    return col;
}

// Stalactites hanging from the cave's vault, some dripping molten rock.
fn stalactites(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env, top: f32) -> vec3<f32> {
    var col = col_in;
    let s = 0.13;
    let c0 = floor(w.x / s) - 1.0;
    for (var i = 0; i < 3; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5 + (hash11(cell * 2.9 + salt) - 0.5) * 0.6) * s;
        // Only where the vault above is fully drawn, never hanging from nothing.
        if (top < 0.05 || presence(here(cx, z), CAVE) < 0.65 || hash11(cell * 6.7 + salt) < 0.3) {
            continue;
        }
        let len = 0.05 + 0.30 * pow(hash11(cell * 4.3 + salt), 1.5);
        let hw = 0.012 + len * 0.12;
        let tip = vec2<f32>(cx + (hash11(cell * 8.3 + salt) - 0.5) * 0.02, top - len);
        let d = triangle(w, vec2<f32>(cx - hw, top + 0.03), vec2<f32>(cx + hw, top + 0.03), tip);
        let mask = fill(d + (vnoise(w * 80.0) - 0.5) * 0.004, env.px);
        let under = soft(top, top - len, w.y);
        let albedo = vec3<f32>(0.09, 0.08, 0.08) * (0.7 + 0.6 * soft(cx + hw, cx - hw, w.x));
        col = mix(col, lit(albedo, env) + shine(vec3<f32>(0.22, 0.05, 0.01) * under, env), mask);
        if (hash11(cell * 1.9 + salt) > 0.7) {
            let drip = exp(-dot(w - tip, w - tip) / (0.00004 + 0.00002 * sin(T * 3.0 + cell)));
            col += shine(vec3<f32>(1.0, 0.45, 0.10) * drip * 0.9, env);
        }
    }
    return col;
}

// Pressure ridges: keels of broken ice hanging down from the pack.
fn keels(col_in: vec3<f32>, w: vec2<f32>, z: f32, salt: f32, env: Env, top: f32) -> vec3<f32> {
    var col = col_in;
    let s = 1.3;
    let c0 = floor(w.x / s - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        let cx = (cell + 0.5 + (hash11(cell * 3.3 + salt) - 0.5) * 0.4) * s;
        if (top < 0.05 || presence(here(cx, z), ARCTIC) < 0.65 || hash11(cell * 5.9 + salt) < 0.3) {
            continue;
        }
        let deep = 0.12 + 0.45 * hash11(cell * 7.1 + salt);
        let hw = deep * 1.5;
        let dx = (w.x - cx) / hw;
        if (abs(dx) > 1.0 || w.y > top + 0.02 || w.y < top - deep - 0.05) {
            continue;
        }
        // Broken blocks: a jagged underside stepped by the size of the slabs.
        let blocks = floor(w.x / 0.05);
        let jag = (hash11(blocks + cell * 31.0) - 0.5) * 0.05 + (vnoise(w * 30.0) - 0.5) * 0.02;
        let bottom = top - deep * (1.0 - dx * dx) + jag;
        let mask = fill(bottom - w.y, env.px);
        let facet = 0.75 + 0.35 * hash11(blocks * 1.3 + cell);
        let depth_in = soft(bottom, top, w.y);
        let ice = mix(vec3<f32>(0.42, 0.72, 0.80), vec3<f32>(0.20, 0.42, 0.52), depth_in) * facet;
        col = mix(col, lit(ice, env) + shine(vec3<f32>(0.10, 0.20, 0.24) * (1.0 - depth_in), env), mask);
    }
    return col;
}

// One row of seabed features at distance `z`.
fn row(col_in: vec3<f32>, q: vec2<f32>, s: Sea, z: f32, salt: f32) -> vec3<f32> {
    var col = col_in;
    // The seabed in front hides the foot of the row, and everything below it.
    if (wall(s) < z * 0.97) {
        return col;
    }
    let env = env_at(s, q, z);
    let w = vec2<f32>(CAM + q.x * z, q.y * z);
    let near = biomes_near(w.x, z);
    // Height above a level seabed: low features need not be tried higher up.
    let above = w.y + s.floor;
    // Nor anything at all above the tallest thing these biomes grow; the vault's
    // stalactites and the ice's keels hang from above instead.
    var tallest = 0.45;
    tallest = max(tallest, select(0.0, 1.55, has(near, SHALLOWS)));
    tallest = max(tallest, select(0.0, 1.85, has(near, FOREST)));
    tallest = max(tallest, select(0.0, 1.35, has(near, WRECKS)));
    tallest = max(tallest, select(0.0, select(0.85, 1.90, z > 2.5), has(near, ATLANTIS)));
    tallest = max(tallest, select(0.0, 0.85, has(near, ABYSS)));
    if (above > tallest) {
        if (has(near, CAVE)) {
            col = stalactites(col, w, z, salt, env, s.roof);
        }
        if (has(near, ARCTIC)) {
            col = keels(col, w, z, salt, env, s.ice);
        }
        return col;
    }
    if (above < 0.40) {
        col = rocks(col, w, z, salt, env);
    }
    if (z > 2.5) {
        if (has(near, WRECKS)) {
            col = galleon(col, w, z, salt, env);
        }
        if (has(near, ATLANTIS)) {
            col = temple(col, w, z, salt, env);
        }
    }
    // The farthest row is mostly lost in the water; it keeps only the great
    // shapes, wrecks and temples, and leaves the forest to the nearer rows.
    let far = z > 4.0;
    if (has(near, FOREST) && !far) {
        col = trees(col, w, z, salt, env);
    }
    if (has(near, ATLANTIS) && above < 0.95) {
        col = columns(col, w, z, salt, env);
    }
    if ((has(near, ABYSS) || has(near, CAVE)) && above < 0.80) {
        col = vents(col, w, z, salt, env);
    }
    if (has(near, CAVE)) {
        if (z < 3.0) {
            col = basalt(col, w, z, salt, env);
        }
        col = stalactites(col, w, z, salt, env, s.roof);
    }
    if (has(near, ARCTIC)) {
        col = keels(col, w, z, salt, env, s.ice);
        if (z < 3.0 && above < 0.35) {
            col = garden(col, w, z, salt, env);
        }
    }
    let green = (has(near, SHALLOWS) || has(near, FOREST)) && !far;
    if (green) {
        if (z < 3.0 && above < 0.35) {
            col = garden(col, w, z, salt, env);
        }
        col = kelp(col, w, z, salt, env, s.sun);
    }
    return col;
}

// ---------------------------------------------------------------------------
// Swimmers
// ---------------------------------------------------------------------------

// The right whale, swimming against the Nautilus' course in the shallows.
fn whales(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = 2.6;
    let speed = -0.035;
    let body_len = 1.35;
    let x = CAM + q.x * z;
    let y = q.y * z;
    for (var i = 0; i < 2; i = i + 1) {
        let sl = slot(x, speed, 5.0, 41.0, i);
        let fade = soft(0.3, 0.6, presence(leg(sl.y, SEED), SHALLOWS)) * step(0.3, hash11(sl.x * 2.3 + 41.0));
        if (fade <= 0.0) {
            continue;
        }
        // Right whales cruise near the surface, outlined against its light.
        let cy = 0.10 + 0.10 * hash11(sl.x * 4.1) + 0.03 * sin(T * 0.11 + sl.x);
        var p = vec2<f32>(x - sl.y, y - cy) / body_len;
        p.x = -p.x;
        let env = env_at(s, q, z);
        // A right whale's flukes beat once every few seconds at this pace.
        let beat = T * TAU * abs(speed) / (0.25 * body_len) + sl.x;
        let before = col;
        col = whale(col, p, beat, env, env.px / body_len);
        col = mix(before, col, fade);
    }
    return col;
}

// A pod of dolphins overtaking the Nautilus in the shallows.
fn dolphins(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = 2.0;
    let speed = 0.13;
    let body_len = 0.24;
    let x = CAM + q.x * z;
    let y = q.y * z;
    let sl = slot(x, speed, 6.0, 53.0, -1);
    let fade = soft(0.3, 0.6, presence(leg(sl.y, SEED), SHALLOWS)) * step(0.3, hash11(sl.x * 2.9 + 53.0));
    if (fade <= 0.0 || abs(x - sl.y) > 0.9) {
        return col;
    }
    let env = env_at(s, q, z);
    for (var i = 0; i < 5; i = i + 1) {
        let fi = f32(i);
        let phase = hash11(fi * 7.1 + sl.x) * TAU;
        let c = vec2<f32>(sl.y + 0.5 - fi * 0.22 + 0.05 * sin(T * 0.3 + phase), -0.02 + hash11(fi * 3.7 + sl.x) * 0.25 + 0.03 * sin(T * 0.6 + phase));
        let size = body_len * (0.85 + 0.3 * hash11(fi * 5.3 + sl.x));
        let p = vec2<f32>(x, y) - c;
        if (abs(p.x) > size * 0.65 || abs(p.y) > size * 0.25) {
            continue;
        }
        let beat = T * TAU * speed / (0.9 * size) + phase;
        let d = dolphin(rot(0.05 * sin(T * 0.5 + phase)) * p / size, env.px / size, beat, false);
        let skin = mix(vec3<f32>(0.30, 0.35, 0.38), vec3<f32>(0.62, 0.64, 0.62), d.y);
        col = mix(col, lit(skin, env), d.x * fade);
        col = mix(col, lit(vec3<f32>(0.03), env), d.z * d.x * fade);
    }
    return col;
}

// A penguin flying underwater, beak at +0.55: the flippers beat up and down,
// the feet trail. x: body; y: flipper; z: white front; w: eye.
fn penguin(p: vec2<f32>, px: f32, beat: f32) -> vec4<f32> {
    var d = ellm(p - vec2<f32>(-0.02, 0.0), vec2<f32>(0.40, 0.12));
    d = smin(d, ellm(p - vec2<f32>(0.33, 0.025), vec2<f32>(0.11, 0.075)), 0.04);
    d = smin(d, tcap(p, vec2<f32>(0.42, 0.020), vec2<f32>(0.55, 0.004), 0.022, 0.005), 0.01);
    d = min(d, triangle(p, vec2<f32>(-0.36, 0.03), vec2<f32>(-0.54, 0.0), vec2<f32>(-0.36, -0.04)));
    d = min(d, tcap(p, vec2<f32>(-0.34, -0.06), vec2<f32>(-0.50, -0.075), 0.026, 0.012));
    let wp = rot(0.9 * sin(beat)) * (p - vec2<f32>(0.12, 0.02));
    let wing = tcap(wp, vec2<f32>(0.0, 0.0), vec2<f32>(-0.30, 0.0), 0.036, 0.010);
    let mask = fill(d, px);
    let front = mask * soft(0.0, -0.035, p.y - 0.02 * sin(p.x * 4.0)) * soft(0.42, 0.30, p.x);
    let eye = fill(length(p - vec2<f32>(0.37, 0.045)) - 0.012, px);
    return vec4<f32>(mask, fill(wing, px), front, eye);
}

// Penguins darting past under the ice, each leaving a trail of bubbles.
fn penguins(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = 1.6;
    if (wall(s) < z) {
        return col;
    }
    let speed = 0.24;
    let body_len = 0.075;
    let x = CAM + q.x * z;
    let y = q.y * z;
    let sl = slot(x, speed, 3.2, 71.0, -1);
    let fade = soft(0.3, 0.6, presence(leg(sl.y, SEED), ARCTIC)) * step(0.25, hash11(sl.x * 4.3 + 71.0));
    if (fade <= 0.0 || abs(x - sl.y) > 0.8) {
        return col;
    }
    let env = env_at(s, q, z);
    for (var i = 0; i < 4; i = i + 1) {
        let fi = f32(i);
        let phase = hash11(fi * 3.3 + sl.x) * TAU;
        let cx = sl.y + 0.40 - fi * 0.24 + 0.05 * sin(T * 0.6 + phase);
        // They porpoise: a long wave up and down as they fly.
        let wave = 0.06 * sin(cx * 3.5 + phase);
        let slope = 0.06 * 3.5 * cos(cx * 3.5 + phase);
        let cy = s.ice - 0.25 - hash11(fi * 5.1 + sl.x) * 0.45 + wave;
        let pv = vec2<f32>(x, y) - vec2<f32>(cx, cy);
        if (abs(pv.x) < 0.35 && abs(pv.y) < 0.08) {
            // A trail of bubbles hangs in the water behind each bird.
            for (var b = 0; b < 5; b = b + 1) {
                let fb = f32(b) + fract(T * speed / 0.05);
                let back = fb * 0.05 + 0.03;
                let bc = vec2<f32>(cx - back, cy - slope * back + 0.010 * fb * fb * 0.2);
                let r = 0.004 + 0.0015 * fb;
                let ring = fill(abs(length(vec2<f32>(x, y) - bc) - r) - 0.0010, env.px);
                col += shine(vec3<f32>(0.55, 0.80, 0.85) * ring * (1.0 - fb / 5.5) * (env.light * 0.6 + 0.25), env) * fade;
            }
        }
        if (abs(pv.x) > body_len * 0.65 || abs(pv.y) > body_len * 0.30) {
            continue;
        }
        let l = rot(-atan(slope)) * pv / body_len;
        let beat = T * TAU * 2.2 + phase;
        let pg = penguin(l, env.px / body_len, beat);
        let plumage = mix(vec3<f32>(0.05, 0.06, 0.07), vec3<f32>(0.85, 0.86, 0.84), pg.z);
        col = mix(col, lit(plumage, env), pg.x * fade);
        col = mix(col, lit(vec3<f32>(0.06, 0.07, 0.08), env), pg.y * fade);
        col = mix(col, lit(vec3<f32>(0.9, 0.6, 0.2), env), pg.w * 0.6 * fade);
    }
    return col;
}

// Narwhals cruising in a small pod under the ice.
fn narwhals(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = 2.6;
    if (wall(s) < z) {
        return col;
    }
    let speed = 0.07;
    let body_len = 0.42;
    let x = CAM + q.x * z;
    let y = q.y * z;
    let sl = slot(x, speed, 5.0, 83.0, -1);
    let fade = soft(0.3, 0.6, presence(leg(sl.y, SEED), ARCTIC)) * step(0.3, hash11(sl.x * 2.7 + 83.0));
    if (fade <= 0.0 || abs(x - sl.y) > 1.2) {
        return col;
    }
    let env = env_at(s, q, z);
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        let phase = hash11(fi * 7.9 + sl.x) * TAU;
        let c = vec2<f32>(sl.y + 0.6 - fi * 0.55, s.ice - 0.35 - hash11(fi * 3.1 + sl.x) * 0.5 + 0.03 * sin(T * 0.3 + phase));
        let size = body_len * (0.85 + 0.3 * hash11(fi * 5.3 + sl.x));
        let pv = vec2<f32>(x, y) - c;
        if (abs(pv.x + size * 0.2) > size * 0.8 || abs(pv.y) > size * 0.25) {
            continue;
        }
        let beat = T * TAU * speed / (0.5 * size) + phase;
        let d = dolphin(pv / size, env.px / size, beat, true);
        // Mottled grey, darker along the back.
        let spots = soft(0.55, 0.70, vnoise(pv / size * 60.0));
        let skin = mix(vec3<f32>(0.38, 0.40, 0.40), vec3<f32>(0.14, 0.15, 0.16), spots * 0.8) * mix(1.0, 1.6, d.y);
        col = mix(col, lit(skin, env), d.x * fade);
        col = mix(col, lit(vec3<f32>(0.03), env), d.z * d.x * fade);
        col = mix(col, lit(vec3<f32>(0.80, 0.76, 0.66), env), d.w * fade);
    }
    return col;
}

// Sharks prowl the wrecks against the Nautilus' course.
fn sharks(col_in: vec3<f32>, q: vec2<f32>, s: Sea, z: f32, salt: f32) -> vec3<f32> {
    var col = col_in;
    if (wall(s) < z) {
        return col;
    }
    let speed = -0.05;
    let body_len = 0.30;
    let x = CAM + q.x * z;
    let y = q.y * z;
    let sl = slot(x, speed, 2.2, salt, -1);
    let fade = soft(0.3, 0.6, presence(leg(sl.y, SEED), WRECKS)) * step(0.4, hash11(sl.x * 2.1 + salt));
    if (fade <= 0.0) {
        return col;
    }
    let cy = -s.floor + 0.15 + hash11(sl.x * 6.3 + salt) * 0.35 + 0.03 * sin(T * 0.2 + sl.x);
    var l = vec2<f32>(x - sl.y, y - cy) / (body_len * 0.5);
    l.x = -l.x;
    if (l.x < -1.4 || l.x > 1.1 || abs(l.y) > 0.5) {
        return col;
    }
    let env = env_at(s, q, z);
    let beat = T * TAU * abs(speed) / (0.35 * body_len) + sl.x;
    let sh = shark(l, env.px / (body_len * 0.5), beat);
    let skin = mix(vec3<f32>(0.30, 0.33, 0.35), vec3<f32>(0.62, 0.62, 0.58), sh.y);
    col = mix(col, lit(skin, env), sh.x * fade);
    col = mix(col, lit(vec3<f32>(0.12), env), (sh.z * 0.5 + sh.w) * fade);
    return col;
}

// The giant squid, met in the abyss and lit only by the searchlight.
fn squids(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = 2.3;
    let speed = -0.03;
    let body_len = 1.1;
    let x = CAM + q.x * z;
    let y = q.y * z;
    for (var i = 0; i < 2; i = i + 1) {
        let sl = slot(x, speed, 4.0, 67.0, i);
        let fade = soft(0.3, 0.6, presence(leg(sl.y, SEED), ABYSS));
        if (fade <= 0.0) {
            continue;
        }
        let cy = -0.12 + 0.10 * sin(T * 0.13 + sl.x);
        var l = vec2<f32>(x - sl.y, y - cy) / body_len;
        l.x = -l.x;
        let env = env_at(s, q, z);
        let before = col;
        col = squid(col, l, env.px / body_len, env);
        col = mix(before, col, fade);
    }
    return col;
}

// Jellyfish and anglerfish hang almost still in the water, so the Nautilus
// sweeps past them at the speed its distance gives.
fn drifters(col_in: vec3<f32>, q: vec2<f32>, s: Sea, z: f32, salt: f32) -> vec3<f32> {
    var col = col_in;
    if (wall(s) < z) {
        return col;
    }
    let x = CAM + q.x * z;
    let y = q.y * z;
    let env = env_at(s, q, z);
    let spacing = 0.40;
    let c0 = floor(x / spacing - 0.5);
    for (var i = 0; i < 2; i = i + 1) {
        let cell = c0 + f32(i);
        let l = leg(cell * spacing, SEED);
        let here = presence(l, ABYSS) + 0.6 * presence(l, WRECKS) + 0.5 * presence(l, ARCTIC);
        if (here < hash11(cell * 4.7 + salt) * 0.8 + 0.15) {
            continue;
        }
        let phase = hash11(cell * 13.1 + salt) * TAU;
        let scale = 0.7 + hash11(cell * 7.7 + salt) * 0.8;
        let centre = vec2<f32>(
            (cell + 0.5) * spacing + sin(T * 0.09 + phase) * 0.03,
            -s.floor + 0.18 + hash11(cell * 5.9 + salt) * (0.25 * z + 0.2) + sin(T * 0.13 + phase * 2.0) * 0.03,
        );
        let p = vec2<f32>(x, y);
        if (abs(p.x - centre.x) < 0.09 * scale && p.y < centre.y + 0.06 * scale && p.y > centre.y - 0.22 * scale) {
            let tint = mix(vec3<f32>(0.78, 0.88, 1.0), vec3<f32>(1.0, 0.70, 0.90), hash11(cell * 17.3 + salt));
            col += shine(jellyfish(p, centre, scale, phase, T) * (0.75 + 0.25 * sin(T * 0.8 + phase)) * tint * (0.6 + env.light * 0.8), env);
        }
    }

    let aspacing = 1.2;
    let ac = floor(x / aspacing);
    if (presence(leg(ac * aspacing, SEED), ABYSS) > 0.5 && hash11(ac * 3.3 + salt) > 0.4) {
        let size = 0.045;
        let centre = vec2<f32>((ac + 0.5) * aspacing, -s.floor + 0.12 + hash11(ac * 6.1 + salt) * 0.25);
        col = anglerfish(col, (vec2<f32>(x, y) - centre) / size, env.px / size, ac, env);
    }
    return col;
}

// The divers walk the seabed of the forest and of Atlantis.
fn divers(col_in: vec3<f32>, q: vec2<f32>, s: Sea, lead_x: f32) -> vec3<f32> {
    var col = col_in;
    let z = DIVER_Z;
    if (wall(s) < z * 0.97) {
        return col;
    }
    let x = CAM + q.x * z;
    let y = q.y * z;
    if (abs(x - lead_x + 0.25) > 0.45) {
        return col;
    }
    let env = env_at(s, q, z);
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        let fx = lead_x - fi * 0.22;
        let size = DIVER_HEIGHT * (1.0 - fi * 0.05);
        let feet = vec2<f32>(fx, ground(fx, z));
        // Feet stay planted: a full stride covers twice a step of the legs.
        let stride = fx / (size * 0.56) * TAU + fi * 1.9;
        col = diver(col, (vec2<f32>(x, y) - feet) / size, env.px / size, stride, i, env);
        // A trail of bubbles from each helmet's exhaust.
        for (var b = 0; b < 3; b = b + 1) {
            let fb = f32(b);
            let rise = fract(T * 0.35 + fb / 3.0 + fi * 0.3);
            let bc = feet + vec2<f32>(-0.01 - rise * 0.06 + sin(rise * 9.0 + fb) * 0.008, size * 0.98 + rise * 0.30);
            let r = 0.005 + rise * 0.004;
            let ring = fill(abs(length(vec2<f32>(x, y) - bc) - r) - 0.0012, env.px);
            col += shine(vec3<f32>(0.40, 0.65, 0.65) * ring * (1.0 - rise) * (env.light * 0.6 + 0.2), env);
        }
    }
    return col;
}

// Rocks and fronds sliding past just outside the glass, out of focus.
fn near_layer(col_in: vec3<f32>, q: vec2<f32>, s: Sea) -> vec3<f32> {
    var col = col_in;
    let z = 0.55;
    var env = env_at(s, q, z);
    let x = CAM + q.x * z;
    let y = q.y * z;
    let l = leg(x, SEED);
    let outcrop = pow(vnoise(vec2<f32>(x * 3.0, SEED * 3.0)), 3.0);
    let top = ground(x, z) + 0.05 + 0.16 * outcrop + 0.01 * vnoise(vec2<f32>(x * 16.0, 1.0));
    // Out of focus: the edge is spread over many pixels.
    let blur = 0.016;
    let rock = 1.0 - soft(top - blur, top + blur, y);
    let albedo = vec3<f32>(0.10, 0.10, 0.09) * by_biome(l, array<f32, 7>(1.6, 1.0, 0.9, 1.1, 0.7, 0.6, 1.3));
    col = mix(col, lit(albedo, env), rock * 0.96);
    col += env.light * vec3<f32>(0.08, 0.10, 0.09) * exp(-abs(y - top) * 70.0) * rock * 0.5;

    // A frond of kelp or a branch of the forest sweeping past the glass.
    let spacing = 0.30;
    let cell = floor(x / spacing);
    let veg = presence(l, SHALLOWS) + presence(l, FOREST);
    if (veg > 0.3 && hash11(cell * 3.1 + SEED) > 0.45) {
        let cx = (cell + 0.5) * spacing;
        let fy = y - top;
        let sway = sin(fy * 10.0 - T * 0.8 + cell) * 0.02 * (fy + 0.05);
        let frond = (1.0 - soft(0.006, 0.016, abs(x - cx - sway))) * soft(-0.01, 0.01, fy);
        col = mix(col, lit(vec3<f32>(0.10, 0.12, 0.05), env), frond * 0.9 * min(veg, 1.0));
    }
    return col;
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(corners[index], 0.0, 1.0);
}

@fragment
fn fs_scene(@builtin(position) in: vec4<f32>) -> @location(0) vec4<f32> {
    T = params.frame.z;
    SEED = params.frame.w;
    CAM = params.scene.y;
    ALT = params.scene.z;
    DEPTH = params.scene.w;
    let side = params.scene.x;
    PX0 = 2.0 * VIEW / side;
    let p = vec2<f32>(in.x / side * 2.0 - 1.0, 1.0 - in.y / side * 2.0) * VIEW;
    let rp = length(p);
    if (rp > GLASS + 0.004) {
        return vec4<f32>(0.0, 0.0, 0.0, 1.0);
    }

    // The thick glass bends the view a little at its edge, and the Nautilus
    // rolls gently on its course.
    let edge = soft(0.62, 1.0, rp / GLASS);
    var q = rot(sin(T * 0.071) * 0.012) * p + normalize(p + vec2<f32>(0.0001, 0.0001)) * edge * 0.008;
    q += vec2<f32>(sin(T * 0.13) * 0.004, sin(T * 0.11 + 1.3) * 0.006);

    let cam_leg = leg(CAM, SEED);
    var s: Sea;
    // How dark the water is: 0 at the surface, 1 far down the abyss.
    s.depth = 1.0 - exp(-DEPTH / 7.0);
    let cave = presence(cam_leg, CAVE);
    s.sun = exp(-s.depth * 4.3) * (1.0 - 0.95 * cave);
    s.lamp = 0.22 + 0.95 * soft(0.12, 0.70, s.depth);
    s.lava = (presence(cam_leg, ATLANTIS) + 1.8 * cave) * (0.85 + 0.15 * sin(T * 2.1) * sin(T * 1.3));
    s.beam = searchlight(q);
    s.fogk = by_biome(cam_leg, array<f32, 7>(0.18, 0.22, 0.15, 0.18, 0.20, 0.22, 0.12));
    s.floor = ALT;
    s.roof = CAVE_ROOF - ALT;
    s.ice = DEPTH - ICE;
    s.water = water(q, s.depth, s.sun, cam_leg);
    s.water += vec3<f32>(0.55, 0.52, 0.42) * s.beam * s.lamp * 0.085;
    s.water += vec3<f32>(0.35, 0.10, 0.03) * s.lava * exp(-max(q.y + 0.1, 0.0) * 3.0) * 0.20;

    // The divers walk at their own pace through the forest and the ruins; the
    // lead diver's lamp lights everything near them.
    let dslot = slot(CAM + q.x * DIVER_Z, DIVER_WALK, 7.0, 29.0, -1);
    let dl = leg(dslot.y, SEED);
    let divers_here = shown(bit(FOREST) | bit(ATLANTIS)) && presence(dl, FOREST) + presence(dl, ATLANTIS) > 0.5 && hash11(dslot.x * 3.9) > 0.2;
    s.diver = 0.0;
    if (divers_here) {
        let lamp = vec2<f32>(dslot.y, ground(dslot.y, DIVER_Z)) + vec2<f32>(0.25, 0.55) * DIVER_HEIGHT;
        s.diver = diver_beam(vec2<f32>(CAM + q.x * DIVER_Z, q.y * DIVER_Z) - lamp);
        s.water += vec3<f32>(0.55, 0.52, 0.42) * s.diver * 0.07;
    }
    s.floor_z = floor_distance(q, s);
    s.ceil_z = 1000.0;
    // Only a roof above the window counts: out of the cave the Nautilus may
    // cruise higher than its vault would be.
    if (q.y > 0.002) {
        let zc = s.roof / q.y;
        if (s.roof > 0.02 && presence(here(CAM + q.x * zc, zc), CAVE) > 0.5) {
            s.ceil_z = zc;
        }
        let zi = s.ice / q.y;
        if (s.ice > 0.02 && zi < s.ceil_z && presence(here(CAM + q.x * zi, zi), ARCTIC) > 0.5) {
            s.ceil_z = zi;
        }
    }

    // Far to near, so each thing is drawn over whatever is behind it.
    var col = surface_layer(s.water, q, s);
    col = far_layer(col, q, s);
    if (shown(bit(CAVE) | bit(ARCTIC))) {
        col = roof_layer(col, q, s);
    }
    col = floor_layer(col, q, s);
    col = row(col, q, s, 4.4, 11.0);
    if (shown(bit(SHALLOWS))) {
        col = whales(col, q, s);
        col = dolphins(col, q, s);
    }
    // Four species, each in its own band of water: big fish high in open water,
    // silversides mid-water, yellow grunts low among the ruins and reefs, and
    // small fry right down by the seabed.
    if (shown(0x4fu)) {
        col = shoal(col, q, s, 3.6, 3.0, 0.040, 0.035, vec2<f32>(0.45, 1.10), vec3<f32>(0.22, 0.26, 0.28), array<f32, 7>(1.0, 0.8, 1.0, 1.0, 0.0, 0.0, 0.5));
    }
    if (shown(bit(ARCTIC))) {
        col = narwhals(col, q, s);
    }
    col = row(col, q, s, 2.8, 23.0);
    let drift = shown(bit(ABYSS) | bit(WRECKS) | bit(ARCTIC));
    if (drift) {
        col = drifters(col, q, s, 2.4, 31.0);
    }
    if (shown(bit(ABYSS))) {
        col = squids(col, q, s);
    }
    if (shown(bit(WRECKS))) {
        col = sharks(col, q, s, 2.0, 37.0);
    }
    if (shown(0x5fu)) {
        col = shoal(col, q, s, 2.2, 7.0, 0.026, -0.045, vec2<f32>(0.25, 0.70), vec3<f32>(0.30, 0.36, 0.38), array<f32, 7>(1.0, 1.0, 0.8, 0.6, 0.15, 0.0, 0.8));
    }
    col = row(col, q, s, 1.8, 47.0);
    if (shown(0x0fu)) {
        col = shoal(col, q, s, 1.5, 13.0, 0.019, 0.075, vec2<f32>(0.08, 0.30), vec3<f32>(0.48, 0.40, 0.20), array<f32, 7>(1.0, 0.7, 0.3, 1.0, 0.0, 0.0, 0.0));
    }
    if (shown(bit(ARCTIC))) {
        col = penguins(col, q, s);
    }
    if (drift) {
        col = drifters(col, q, s, 1.4, 59.0);
    }
    col = row(col, q, s, 1.25, 61.0);
    if (divers_here) {
        col = divers(col, q, s, dslot.y);
    }
    if (shown(0x4bu)) {
        col = shoal(col, q, s, 0.95, 17.0, 0.013, 0.030, vec2<f32>(0.05, 0.22), vec3<f32>(0.32, 0.42, 0.46), array<f32, 7>(1.0, 1.0, 0.0, 0.5, 0.0, 0.0, 0.6));
    }
    col = near_layer(col, q, s);

    // Marine snow at two distances, each streaming past at its own speed; over
    // Atlantis it is ash and embers.
    var snow = 0.0;
    for (var i = 0; i < 2; i = i + 1) {
        let z = select(0.8, 1.7, i == 1);
        // Anchored in the water, so the snow streams down past the glass as the
        // Nautilus climbs and up as it dives.
        let w = vec2<f32>(CAM + q.x * z, q.y * z - DEPTH - T * 0.006);
        snow += motes(w / z, 40.0 * z / 0.8, vec2<f32>(SEED * 9.0 + f32(i) * 3.0, 0.0), vec2<f32>(0.30, 1.0)) * select(1.0, 0.6, i == 1);
    }
    let flake = mix(vec3<f32>(0.34, 0.61, 0.63), vec3<f32>(1.0, 0.45, 0.15), s.lava * 0.7);
    col += flake * snow * (0.07 + 0.40 * s.beam * s.lamp + 0.10 * s.sun);

    let arctic = presence(cam_leg, ARCTIC);
    if (shown(bit(ARCTIC)) && arctic > 0.01) {
        // Swarms of krill, pink in the light from the ice.
        let w = vec2<f32>(CAM + q.x * 1.3, q.y * 1.3 - DEPTH) / 1.3;
        let swarm = soft(0.45, 0.75, vnoise(w * 2.5 + vec2<f32>(T * 0.03, 0.0)));
        let krill = motes(w + vec2<f32>(sin(T * 0.7 + w.y * 9.0) * 0.004, 0.0), 90.0, vec2<f32>(SEED * 7.0, 0.0), vec2<f32>(0.6, 1.0));
        col += vec3<f32>(0.75, 0.45, 0.40) * krill * swarm * arctic * (0.15 + 0.5 * s.sun);
    }
    let abyss = presence(cam_leg, ABYSS);
    if (shown(bit(ABYSS)) && abyss > 0.01) {
        let w = vec2<f32>(CAM + q.x * 1.2, q.y * 1.2) / 1.2;
        let bio = motes(w * 1.4 + vec2<f32>(SEED * 13.0, 0.0), 34.0, vec2<f32>(T * 0.004, -T * 0.003), vec2<f32>(1.0, 1.0));
        let flourish = 0.45 + 0.55 * sin(T * 2.1 + hash11(floor(w.x * 20.0)) * TAU);
        col += vec3<f32>(0.10, 0.82, 0.66) * bio * flourish * 0.45 * abyss;
    }

    // Bubbles shed by the hull just outside the glass: they hang where they are
    // released, so the voyage sweeps them aft as fast as anything in view.
    for (var i = 0; i < 8; i = i + 1) {
        let fi = f32(i);
        let z = 0.45 + hash11(fi * 1.3 + SEED) * 0.3;
        let rise = 0.030 + hash11(fi + SEED * 17.0) * 0.040;
        let y = -0.48 + fract(hash11(fi * 2.3 + SEED * 5.0) + T * rise / z / 0.96) * 0.96;
        let x = 0.55 - fract(hash11(fi * 3.7 + SEED * 23.0) + T * SPEED / z / 1.10) * 1.10 + sin(T * 1.5 + fi * 2.1) * 0.010;
        let radius = (0.004 + hash11(fi * 4.1 + SEED * 29.0) * 0.006) / z;
        let d = length(q - vec2<f32>(x, y));
        if (d < radius + 0.003) {
            let shell = soft(radius, radius - 0.0020, d) * (1.0 - soft(radius - 0.0055, radius - 0.0030, d));
            let glint = soft(radius * 0.38, 0.0, length(q - vec2<f32>(x - radius * 0.36, y + radius * 0.36)));
            let light = env_at(s, q, z).light;
            col += vec3<f32>(0.38, 0.78, 0.82) * (shell * 0.40 + glint * 0.65) * (0.35 + light * 0.6);
        }
    }

    // The glass: a faint green tint, a vignette and a sheen.
    col *= vec3<f32>(0.93, 1.00, 0.96);
    col *= 1.0 - 0.55 * soft(GLASS * 0.55, GLASS, rp);
    let shear = dot(p, normalize(vec2<f32>(0.55, -0.83)));
    col += vec3<f32>(0.12, 0.20, 0.21) * exp(-pow((shear + 0.16) * 13.0, 2.0)) * 0.10;
    return vec4<f32>(col, 1.0);
}

// ---------------------------------------------------------------------------
// The bezel
// ---------------------------------------------------------------------------

fn gauge_centre(angle: f32) -> vec2<f32> {
    return vec2<f32>(cos(angle), sin(angle)) * GAUGE_RING;
}

// A gauge face: brass bezel, enamel dial, ticks over `sweep` radians and a red
// sector near the end of the scale.
fn gauge_face(p: vec2<f32>, centre: vec2<f32>, sweep: f32, ticks: f32, red: bool) -> vec4<f32> {
    let l = p - centre;
    let r = length(l);
    if (r > GAUGE_R + 0.008) {
        return vec4<f32>(0.0);
    }
    let a = atan2(l.x, l.y);
    let u = r / GAUGE_R;
    let aa = 0.0012;
    var col = vec3<f32>(0.86, 0.80, 0.66) * (0.92 + 0.08 * vnoise(l * 300.0));
    col *= 1.0 - 0.25 * soft(0.55, 1.0, u);

    let inside = abs(a) <= sweep * 0.5 + 0.02;
    let step_a = sweep / ticks;
    let k = floor((a + sweep * 0.5) / step_a + 0.5);
    let near_a = -sweep * 0.5 + k * step_a;
    let tick_d = abs(a - near_a) * r;
    let major = (i32(k) % 5) == 0;
    let reach = select(0.80, 0.68, major);
    let tick = fill(tick_d - select(0.0004, 0.0009, major), aa * 0.7) * soft(reach - 0.02, reach, u) * soft(0.92, 0.90, u);
    if (inside) {
        col = mix(col, vec3<f32>(0.10, 0.08, 0.06), tick);
    }
    if (red && a > sweep * 0.30 && a < sweep * 0.5) {
        col = mix(col, vec3<f32>(0.62, 0.10, 0.06), soft(0.70, 0.72, u) * soft(0.80, 0.78, u));
    }
    col = mix(col, vec3<f32>(0.10, 0.08, 0.06), fill(abs(u - 0.93) - 0.006, 0.01));

    // The brass bezel ring with its knurled edge.
    let bez = soft(0.96, 0.99, u);
    let light = 0.55 + 0.45 * dot(normalize(l + vec2<f32>(0.0001)), normalize(vec2<f32>(-0.5, 0.8))) * select(1.0, -1.0, u > 1.08);
    let knurl = 0.85 + 0.15 * sin(atan2(l.y, l.x) * 120.0);
    let brass = vec3<f32>(0.62, 0.42, 0.16) * (0.45 + 0.75 * light) * knurl;
    col = mix(col, brass, bez);
    let alpha = fill(r - GAUGE_R - 0.006, aa);
    return vec4<f32>(col * alpha, alpha);
}

fn brass_rim(p: vec2<f32>, rp: f32, angle: f32) -> vec3<f32> {
    let u = clamp((rp - GLASS) / (PORT - GLASS), 0.0, 1.0);
    let radial = vec2<f32>(cos(angle), sin(angle));
    // Two rounded beads with a flat engraved band between them.
    let profile = select(
        select(sin((u - 0.70) / 0.30 * PI), 0.0, u < 0.70),
        sin(u / 0.30 * PI),
        u < 0.30,
    );
    let slope = select(
        select(cos((u - 0.70) / 0.30 * PI), 0.0, u < 0.70),
        cos(u / 0.30 * PI),
        u < 0.30,
    );
    let normal = normalize(vec3<f32>(-radial * slope * 0.9, 0.5 + profile * 0.5));
    let lamplight = normalize(vec3<f32>(-0.45, 0.60, 0.70));
    let diffuse = clamp(dot(normal, lamplight), 0.0, 1.0);
    let brushed = fbm(vec2<f32>(angle * 40.0, rp * 300.0));
    var brass = vec3<f32>(0.42, 0.26, 0.09) * (0.22 + 0.85 * diffuse);
    brass += vec3<f32>(0.55, 0.40, 0.18) * pow(diffuse, 9.0) * 0.9;
    brass *= 0.86 + 0.24 * brushed;
    // Tarnish in the crevices.
    brass = mix(brass, vec3<f32>(0.10, 0.17, 0.13), soft(0.55, 0.85, fbm(p * 40.0)) * (1.0 - profile) * 0.25);

    // The engraved scale on the flat band: a tick every 5 degrees, longer every 30.
    let deg = (angle / TAU) * 360.0;
    let k = floor(deg / 5.0 + 0.5);
    let tick_d = abs(deg - k * 5.0) * TAU / 360.0 * rp;
    let major = (i32(abs(k)) % 6) == 0;
    let band = soft(0.33, 0.36, u) * soft(0.67, 0.64, u);
    let length_ok = select(soft(0.48, 0.46, u) * soft(0.36, 0.38, u), band, major);
    let engraved = fill(tick_d - select(0.00045, 0.0008, major), 0.0006) * length_ok;
    brass *= 1.0 - 0.55 * engraved;
    brass = mix(brass, brass * 0.75, band * 0.25);

    // Twelve hex-headed bolts on the outer bead.
    let sector = TAU / 12.0;
    let bolt_a = floor(angle / sector + 0.5) * sector;
    let bolt_c = vec2<f32>(cos(bolt_a), sin(bolt_a)) * (GLASS + (PORT - GLASS) * 0.85);
    let bl = rot(bolt_a * 3.0 + 0.3) * (p - bolt_c);
    let hex_q = abs(bl);
    let hex = max(hex_q.x * 0.866 + hex_q.y * 0.5, hex_q.y) - 0.0075;
    let bolt = fill(hex, 0.0008);
    let bolt_light = 0.55 + 0.45 * dot(normalize(bl + vec2<f32>(0.0001)), normalize(vec2<f32>(-0.5, 0.7)));
    var head = vec3<f32>(0.40, 0.27, 0.11) * (0.45 + 0.65 * bolt_light);
    head += vec3<f32>(0.50, 0.38, 0.18) * soft(0.006, 0.0, length(bl - vec2<f32>(-0.002, 0.003))) * 0.5;
    brass = mix(brass * (1.0 - 0.5 * soft(0.012, 0.007, length(p - bolt_c - vec2<f32>(0.0015, -0.0015)))), head, bolt);

    // Dark seams where the rim meets the glass and the casing.
    brass *= 1.0 - 0.70 * exp(-u * 30.0) - 0.60 * exp(-(1.0 - u) * 40.0);
    return brass;
}

// The Nautilus' monogram on a brass cartouche at the foot of the window.
fn cartouche(p: vec2<f32>) -> vec4<f32> {
    let c = vec2<f32>(0.0, -GAUGE_RING - 0.004);
    let l = p - c;
    let plate = ell(l, vec2<f32>(0.046, 0.030));
    if (plate > 0.4) {
        return vec4<f32>(0.0);
    }
    let shade = 0.55 + 0.45 * soft(-0.03, 0.03, l.y - l.x * 0.4);
    var col = vec3<f32>(0.55, 0.37, 0.13) * shade;
    col *= 1.0 - 0.45 * fill(abs(plate + 0.12) - 0.012, 0.01);
    let n = min(
        min(segment(l, vec2<f32>(-0.012, -0.014), vec2<f32>(-0.012, 0.014)), segment(l, vec2<f32>(0.012, -0.014), vec2<f32>(0.012, 0.014))),
        segment(l, vec2<f32>(-0.012, 0.014), vec2<f32>(0.012, -0.014)),
    );
    let serifs = min(
        min(segment(l, vec2<f32>(-0.017, -0.014), vec2<f32>(-0.007, -0.014)), segment(l, vec2<f32>(0.007, 0.014), vec2<f32>(0.017, 0.014))),
        segment(l, vec2<f32>(-0.017, 0.014), vec2<f32>(-0.012, 0.014)),
    );
    let glyph = fill(min(n - 0.0024, serifs - 0.0012), 0.0008);
    col = mix(col, vec3<f32>(0.14, 0.08, 0.03), glyph * 0.85);
    let alpha = fill(plate, 0.03);
    return vec4<f32>(col * alpha, alpha);
}

@fragment
fn fs_overlay(@builtin(position) in: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<f32>(
        (in.x - params.frame.x * 0.5) / params.frame.y,
        (params.frame.y * 0.5 - in.y) / params.frame.y,
    );
    let rp = length(p);
    let angle = atan2(p.y, p.x);

    var out = vec4<f32>(0.0);
    if (rp > PORT) {
        // Riveted iron plates of the salon wall.
        let plate_a = TAU / 8.0;
        let seam_a = abs(fract(angle / plate_a + 0.5) - 0.5) * plate_a * rp;
        let iron = vec3<f32>(0.050, 0.046, 0.042) * (0.75 + 0.35 * fbm(p * 22.0)) * (0.8 + 0.3 * soft(-0.4, 0.4, p.y - p.x));
        var casing = iron;
        casing *= 1.0 - 0.6 * fill(seam_a - 0.0012, 0.0008);
        let ring_d = abs(rp - 0.478);
        casing *= 1.0 - 0.5 * fill(ring_d - 0.0010, 0.0008);
        let rivet_a = TAU / 48.0;
        let ra = floor(angle / rivet_a + 0.5) * rivet_a;
        let rc = vec2<f32>(cos(ra), sin(ra)) * 0.465;
        let rd = length(p - rc);
        casing = mix(casing, vec3<f32>(0.11, 0.09, 0.07) * (0.6 + 0.6 * soft(0.004, -0.004, dot(p - rc, vec2<f32>(0.7, -0.7)))), fill(rd - 0.0038, 0.0007));
        casing += vec3<f32>(0.090, 0.060, 0.028) * exp(-length(p - vec2<f32>(-0.34, 0.41)) * 7.0);
        casing *= 1.0 - 0.6 * exp(-(rp - PORT) * 110.0);
        out = vec4<f32>(casing, 1.0);
    } else if (rp > GLASS) {
        out = vec4<f32>(brass_rim(p, rp, angle), 1.0);
    } else {
        // The bezel's shadow on the glass, deepest under the top edge.
        let lip = soft(GLASS - 0.030, GLASS, rp) * (0.40 + 0.30 * soft(-0.3, 0.3, p.y));
        out = vec4<f32>(0.0, 0.0, 0.0, lip * 0.75);
    }

    let depth_face = gauge_face(p, gauge_centre(DEPTH_GAUGE), 4.7, 30.0, true);
    out = vec4<f32>(depth_face.rgb + out.rgb * (1.0 - depth_face.a), depth_face.a + out.a * (1.0 - depth_face.a));
    let log_face = gauge_face(p, gauge_centre(LOG_GAUGE), TAU - 0.0001, 40.0, false);
    out = vec4<f32>(log_face.rgb + out.rgb * (1.0 - log_face.a), log_face.a + out.a * (1.0 - log_face.a));
    let plate = cartouche(p);
    out = vec4<f32>(plate.rgb + out.rgb * (1.0 - plate.a), plate.a + out.a * (1.0 - plate.a));
    return out;
}

// ---------------------------------------------------------------------------
// Composite
// ---------------------------------------------------------------------------

// A blued-steel needle with a counterweight, pointing at `angle` (0 = up,
// clockwise), plus its shadow on the dial.
fn needle(col_in: vec3<f32>, p: vec2<f32>, centre: vec2<f32>, angle: f32, tip: vec3<f32>) -> vec3<f32> {
    var col = col_in;
    let l = p - centre;
    if (dot(l, l) > GAUGE_R * GAUGE_R) {
        return col;
    }
    let dir = vec2<f32>(sin(angle), cos(angle));
    let shadow_l = l - vec2<f32>(0.0025, -0.0025);
    let reach = GAUGE_R * 0.80;
    let ds = segment(shadow_l, -dir * reach * 0.25, dir * reach) - 0.0016;
    col *= 1.0 - 0.35 * fill(ds, 0.002);
    let along = clamp(dot(l, dir) / reach, -0.25, 1.0);
    let width = mix(0.0022, 0.0006, max(along, 0.0));
    let d = segment(l, -dir * reach * 0.25, dir * reach) - width;
    let steel = mix(vec3<f32>(0.06, 0.07, 0.11), tip, soft(0.70, 0.78, along));
    col = mix(col, steel, fill(d, 0.0007));
    let cap = length(l) - 0.0055;
    col = mix(col, vec3<f32>(0.60, 0.42, 0.18) * (0.7 + 0.5 * soft(0.004, -0.004, l.x - l.y)), fill(cap, 0.0007));
    return col;
}

@fragment
fn fs_composite(@builtin(position) in: vec4<f32>) -> @location(0) vec4<f32> {
    let t = params.frame.z;
    let seed = params.frame.w;
    let p = vec2<f32>(
        (in.x - params.frame.x * 0.5) / params.frame.y,
        (params.frame.y * 0.5 - in.y) / params.frame.y,
    );
    let rp = length(p);

    let over = textureLoad(bezel, vec2<i32>(in.xy), 0);
    var col = vec3<f32>(0.0);
    if (rp < GLASS + 0.002 && over.a < 0.999) {
        var q = p;
        if (GLASS_BEND > 0.0) {
            q += normalize(p + vec2<f32>(0.0001)) * soft(0.62, 1.0, rp / GLASS) * GLASS_BEND;
        }
        let uv = vec2<f32>(q.x, -q.y) / (2.0 * VIEW) + 0.5;
        col = textureSampleLevel(sea, sea_sampler, uv, 0.0).rgb;

        // An engraver's finish, after the plates of the 1871 edition: fine
        // hatching that only shows in the shadows, and a warm, inky grade.
        let lum = dot(col, vec3<f32>(0.30, 0.55, 0.15));
        let hatch_a = 0.5 + 0.5 * sin((in.x + in.y) * 1.25);
        let hatch_b = 0.5 + 0.5 * sin((in.x - in.y) * 1.25);
        let dark = soft(0.16, 0.02, lum);
        let lines = hatch_a * soft(0.0, 1.0, dark) + hatch_b * soft(0.5, 1.0, dark);
        col *= 1.0 - 0.22 * lines * dark;
        col = mix(col, col * vec3<f32>(1.08, 1.0, 0.88), soft(0.10, 0.50, lum));
    }
    col = col * (1.0 - over.a) + over.rgb;

    // The manometer reads on a logarithmic scale, so it moves as much for the
    // Nautilus rising a few fathoms in the shallows as for a long climb out of
    // the abyss.
    let reading = log(1.0 + params.scene.w) / log(46.0);
    col = needle(col, p, gauge_centre(DEPTH_GAUGE), (reading - 0.5) * 4.7 * 0.92, vec3<f32>(0.60, 0.10, 0.06));
    col = needle(col, p, gauge_centre(LOG_GAUGE), params.scene.y / LEG * TAU, vec3<f32>(0.06, 0.07, 0.11));

    // Dither away the banding in the dark water.
    col += (hash21(in.xy + fract(t) * 91.0) - 0.5) / 255.0;
    return vec4<f32>(col, 1.0);
}
