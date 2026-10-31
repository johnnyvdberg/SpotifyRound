// The 3D dive.
//
// WESL links the shared functions below at build time.
import package::common::smooth01;
//
// The camera sits at the salon window looking out sideways (+z), level, with
// the Nautilus travelling along +x; the sea's surface is at y = 0 and the boat
// moves under it, turning the view with its pitch, roll and heading (see
// `helm.rs`). Everything here is ordinary rasterised
// geometry with a depth buffer: a background pass for the open water, the
// seabed lifted from a grid in the vertex shader, instanced models placed and
// animated by Rust, translucent jellyfish, and camera-facing sprites for snow,
// bubbles and glows.
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
    // x: surface height; y: the cave vault's; z: the ice's; w: how much of
    // the drowned city's glow is in the water.
    levels: vec4<f32>,
    // x: world seed; y: lava glow; z: share of the lava cave; w: share of ice.
    world: vec4<f32>,
    // x: point lights in use.
    counts: vec4<f32>,
    // rgb: what the water above the window leaves of daylight's colours,
    // having taken its red first, then its green.
    tint: vec4<f32>,
    // xy: the ice bake's world origin; z: 1 / its size.
    ice: vec4<f32>,
    lights: array<vec4<f32>, 8>,
    light_colours: array<vec4<f32>, 8>,
    // This visit's fourteen-leg biome circuit, four legs a vector.
    order: array<vec4<f32>, 4>,
    // The biome mix at the window and how much of this sea has a sky over
    // it: xy is the pair, z the blend, w the sky's strength there.
    sky: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: Globals;
// The seabed's relief: height and its slopes, from `heightmap.rs`.
@group(0) @binding(1) var relief_map: texture_2d<f32>;
// The surfaces, one layer each, from `textures.rs`.
@group(0) @binding(2) var surfaces: texture_2d_array<f32>;
@group(0) @binding(3) var surface_sampler: sampler;
// Caustics precomputed on the CPU once, instead of a sin/pow every vertex.
@group(0) @binding(4) var caustic_map: texture_2d<f32>;
@group(0) @binding(5) var caustic_sampler: sampler;
// The ice sheet's baked floes/leads/bumps/coverage, one sample per pixel.
@group(0) @binding(12) var ice_bake: texture_2d<f32>;
// The baked per-biome skyboxes: a cubemap array, one cube per biome, in id
// order. One fetch replaces the old per-pixel sky arithmetic.
@group(0) @binding(13) var sky_cubes: texture_cube_array<f32>;
const RELIEF_SIZE: i32 = 512;
// The lava cave's tunnel and the massif it runs through, as in `world.rs`.
const TUNNEL: f32 = 1.05;
const MASSIF: f32 = 4.5;
const RELIEF_PERIOD: f32 = 64.0;
// Textures store 0..1.6 as 0..1, so they can brighten as well as darken.
const TEXTURE_RANGE: f32 = 1.6;
// log2 of the surfaces' size, `textures::SIZE`.
const SURFACE_LOD: f32 = 8.0;
// The coral and brain coral layers of the surfaces, `textures::Surface`.
const CORAL_LAYER: i32 = 8;
const BRAIN_LAYER: i32 = 15;
// The living reef carpeting the coral's seabed, `textures::Surface::Reef`.
const REEF_LAYER: i32 = 17;
// The far staghorn's baked silhouettes, `textures::Surface::Staghorn`, and
// how many colonies a side of the layer holds, `textures::STAGHORN_CELLS`.
const STAGHORN_LAYER: i32 = 18;
const STAGHORN_CELLS: f32 = 2.0;
// The surfaces' size in texels, `textures::SIZE`.
const SIZE_STAGHORN: i32 = 256;
// How much of the terrain layer's fraction carries the reef, as
// `terrain::REEF_SHARE`.
const REEF_SHARE: f32 = 0.45;

// Whether this models pipeline draws things that are fading in or out. Only
// that one may discard fragments: the Pi's tile-based GPU skips hidden
// fragments before shading them only when a shader cannot discard, so every
// model that is fully there goes through a pipeline compiled without it.
override FADING: bool = true;
// Fading props drawn opaque: the coverage fades their colour into the water
// instead of blending them over what is behind. Same depth writes as any
// solid model, so the deferred/water pass sees them as a surface.
override FADE_OPAQUE: bool = false;
// The flat/static variant: per-object light has been baked into the instance
// colour on the CPU, so the vertex stage skips the cofactor normal and the
// whole light path. For the small, distant instances only.
// Whether this models pipeline draws things that stand on the seabed. Only
// those need the ground under their foot, so the swimmers' pipelines leave it
// out.
override GROUNDED: bool = false;

// The material every model this pipeline draws is made of, or -1 to take it
// from each instance. A pipeline per material compiles only that material's
// shading: with all of them in, the shader is too large for the Pi's GPU to
// keep more than two of its threads at once, and every pixel pays for it.
override MATERIAL: i32 = -1;

// What every colour is multiplied by as it is stored: the sea is kept in ten
// bits a channel, which hold no more than 1, so it is stored at half its
// brightness, to keep the highlights up to 2 that the old grade and
// its bending of the glass blend from.
override SEA_SCALE: f32 = 1.0;

// What the open water's pipeline opens on to overhead: the plain sea, the
// underside of the water surface, the polar sky through the ice's leads, or
// the underside with the sky through Snell's window over it, for a view
// steep enough to reach the window (`sky_mode` on the CPU picks one a frame,
// by the same tests the shader used to make per pixel). Each compiles only
// its own: together they kept the water at two threads on the Pi, and it
// shades most of a reef view. The sky's one fetch alone halves the
// ceiling's threads, and the salon's window never looks that far up.
override SKY: i32 = 1;
const SKY_OPEN: i32 = 0;
const SKY_CEILING: i32 = 1;
const SKY_POLAR: i32 = 2;
const SKY_SNELL: i32 = 3;

// The impostor form this pipeline draws (`fs_impostor`): 0 staghorn,
// 1 table, 2 fingers, 3 dome or bush, 5 fish; -1 any. A form a pipeline
// keeps the card shader small enough for four threads, where all of them in
// one needed sixty registers and two.
override FORM: i32 = -1;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const LEG: f32 = 16.5;
const BLEND: f32 = 0.10;
const BIOMES: i32 = 14;
const SHALLOWS: i32 = 0;
const FOREST: i32 = 1;
const WRECKS: i32 = 2;
const ATLANTIS: i32 = 3;
const CAVE: i32 = 4;
const ABYSS: i32 = 5;
const ARCTIC: i32 = 6;
const TRENCH: i32 = 7;
const CORAL: i32 = 8;
const SARGASSO: i32 = 9;
const VENTS: i32 = 10;
const BRINE: i32 = 11;
const BLOOM: i32 = 12;
const CITY: i32 = 13;

// One component of a four-leg vector. A dynamic index into a uniform vector
// is not worth the risk on every shader the Pi compiles, so this is unrolled.
fn leg_component(v: vec4<f32>, i: u32) -> f32 {
    var out = v.x;
    if (i == 1u) {
        out = v.y;
    } else if (i == 2u) {
        out = v.z;
    } else if (i == 3u) {
        out = v.w;
    }
    return out;
}

// The biome of the `n`th leg of this visit's circuit, from the shuffled order
// the CPU sends in `g.order` (`voyage::order_for`).
fn order(n: i32) -> i32 {
    let k = u32(((n % BIOMES) + BIOMES) % BIOMES);
    if (k < 4u) {
        return i32(leg_component(g.order[0], k));
    }
    if (k < 8u) {
        return i32(leg_component(g.order[1], k - 4u));
    }
    if (k < 12u) {
        return i32(leg_component(g.order[2], k - 8u));
    }
    return i32(leg_component(g.order[3], k - 12u));
}

// ---------------------------------------------------------------------------
// Shared with world.rs
// ---------------------------------------------------------------------------

fn hash(ix: i32, iz: i32, salt: u32) -> f32 {
    var h = (bitcast<u32>(ix) * 0x8da6b343u) ^ (bitcast<u32>(iz) * 0xd8163841u) ^ (salt * 0xcb1ab31fu);
    h = (h ^ (h >> 13u)) * 0x5bd1e995u;
    h = h ^ (h >> 15u);
    return f32(h & 0x00ffffffu) / 16777216.0;
}

// The shader switches every pipeline is compiled with: the kiosk's look
// flags and the benchmarks' cheaper paths (`dive3d::shader_switch`). A
// pipeline constant, never read per pixel: tested from the uniforms, the
// switches cost every shader their alternatives' registers, a third of the
// seabed's instructions.
override SWITCHES: u32 = 0u;
const NO_NOISE: u32 = 2u;

// Every surface a flat grey, for the benchmarks. A pipeline constant of its
// own, like `SWITCHES`: tested per pixel, it would cost the Pi's GPU about
// thirty instructions a texture lookup.
override PLAIN_TEXTURES: bool = false;

fn debug(flag: u32) -> bool {
    return (SWITCHES & flag) != 0u;
}

// Always false, but read from the uniforms, so the compiler cannot know it:
// a branch on it is a fence the Pi's compiler schedules each side of apart.
// Arithmetic that does not wait on a texture, a noise or a sparkle, is
// otherwise hoisted up beside the lookup to hide its latency, and the
// registers both hold at once cost the shader half its threads.
fn fenced() -> bool {
    return g.counts.y > 0.5;
}

fn vnoise(x: f32, z: f32, salt: u32) -> f32 {
    if (debug(NO_NOISE) || fenced()) {
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
    let a = order(i32(k));
    let b = order(i32(k) + 1);
    return Leg(a, b, smooth01(1.0 - BLEND, 1.0, u - k));
}

fn presence(l: Leg, id: i32) -> f32 {
    return select(0.0, 1.0 - l.w, l.a == id) + select(0.0, l.w, l.b == id);
}

fn by_biome(l: Leg, values: array<f32, 14>) -> f32 {
    var v = values;
    return mix(v[l.a], v[l.b], l.w);
}

// The heightmap at (x, z), bilinear, exactly as `heightmap::sample`: height,
// slopes, and the fine grain each biome shapes its floor with.
fn relief(x: f32, z: f32) -> vec4<f32> {
    let u = x / RELIEF_PERIOD * f32(RELIEF_SIZE);
    let v = z / RELIEF_PERIOD * f32(RELIEF_SIZE);
    let fu = floor(u);
    let fv = floor(v);
    let i = i32(fu);
    let j = i32(fv);
    let n = RELIEF_SIZE;
    let a = textureLoad(relief_map, vec2<i32>(((i % n) + n) % n, ((j % n) + n) % n), 0).xyzw;
    let b = textureLoad(relief_map, vec2<i32>((((i + 1) % n) + n) % n, ((j % n) + n) % n), 0).xyzw;
    let c = textureLoad(relief_map, vec2<i32>(((i % n) + n) % n, (((j + 1) % n) + n) % n), 0).xyzw;
    let d = textureLoad(relief_map, vec2<i32>((((i + 1) % n) + n) % n, (((j + 1) % n) + n) % n), 0).xyzw;
    let tu = u - fu;
    let tv = v - fv;
    return mix(mix(a, b, tu), mix(c, d, tu), tv);
}

// The seabed's height, exactly as `Seabed::ground`: the biome's depth below
// the surface, the heightmap's relief, fine roughness, and mountains far off.
fn ground(x: f32, z: f32) -> f32 {
    let parts = ground_parts(x, z);
    return parts.x + parts.y;
}

// The same in two parts: the massif under the lava cave, and the rest.
fn ground_parts(x: f32, z: f32) -> vec2<f32> {
    let l = leg(voyage_x(x, z));
    let r = relief(x, z);
    let base = -by_biome(l, array<f32, 14>(1.7, 3.6, 4.9, 5.8, 6.6, 9.3, 3.5, 4.2, 2.2, 3.4, 8.2, 6.3, 10.2, 7.2));
    let lift = by_biome(l, array<f32, 14>(0.25, 0.35, 0.35, 0.12, 0.20, 1.20, 0.40, 0.30, 0.45, 0.30, 0.90, 0.06, 0.35, 0.08)) * r.x;
    let rough = by_biome(l, array<f32, 14>(0.05, 0.07, 0.05, 0.02, 0.03, 0.08, 0.05, 0.05, 0.06, 0.05, 0.08, 0.02, 0.04, 0.02));
    let ridges = by_biome(l, array<f32, 14>(1.6, 2.2, 2.0, 2.6, 3.5, 5.0, 1.4, 2.4, 2.6, 2.0, 5.5, 0.5, 3.0, 1.2));
    let n = 0.65 * vnoise(x * 1.3, z * 1.3, 1u) + 0.35 * vnoise(x * 3.1 + 5.0, z * 3.1, 2u);
    let across = z - corridor(x);
    let off = abs(across);
    let far = smooth01(9.0, 26.0, off)
        * (0.15
            + 0.65
                * ridges
                * (0.65 * vnoise(x * 0.16, z * 0.16 + 3.0, 4u)
                    + 0.35 * vnoise(x * 0.47, z * 0.43, 6u)))
        * by_biome(
            l,
            array<f32, 14>(
                1.0, 1.0, 1.0, 0.8, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.15, 1.0, 0.3,
            ),
        );
    // The trench's terraces. Only vertex shaders find the ground, so no
    // pixel pays for them.
    var trench = presence(l, TRENCH);
    if (trench > 0.0) {
        trench *= trench_profile(x, across);
    }
    let shape = biome_shape(l, x, z, across, r.x, r.w);
    return vec2<f32>(base + lift + shape + rough * (n - 0.45) + far + trench, massif(x, z, l, off));
}

// The shape each biome gives its own floor, over the shared heightmap: the
// same function as `world::biome_shape`. Every term but the cave's repeats
// over the heightmap's 64 units, so the added shape repeats with it; the
// cave's ledges stand back from its halls, as its massif does.
fn biome_shape(l: Leg, x: f32, z: f32, across: f32, h: f32, detail: f32) -> f32 {
    var shape = 0.0;
    let shallows = presence(l, SHALLOWS);
    if (shallows > 0.0) {
        // Low banks with sand waves breaking over them.
        let banks = smooth01(0.45, 0.72, 0.5 + 0.5 * detail);
        let waves = sin(TAU * (x * 0.25 + z * 0.125) + 1.4 * detail)
            + 0.5 * sin(TAU * (x * 0.125 - z * 0.25) + 0.8 * detail);
        shape += shallows * (0.10 * h + 0.10 * banks + 0.05 * waves);
    }
    let forest = presence(l, FOREST);
    if (forest > 0.0) {
        // A low silt flat, hummocked and pocked with the mounds roots raise.
        let hummocks = sin(TAU * (x * 0.5 + z * 0.25) + 1.2 * detail)
            + 0.6 * sin(TAU * (x * 0.25 - z * 0.5) + 0.7 * detail);
        let mounds = smooth01(0.45, 0.75, 0.5 + 0.5 * detail);
        shape += forest * (-0.05 * h + 0.12 * hummocks + 0.25 * detail + 0.30 * mounds);
    }
    let wrecks = presence(l, WRECKS);
    if (wrecks > 0.0) {
        // Sediment drifts running along the wreck field, with rock showing
        // through where the drift is thin.
        let drifts = sin(TAU * z * 0.125 + 0.8 * detail)
            + 0.4 * sin(TAU * (z * 0.375 + x * 0.0625));
        let rocks = smooth01(0.72, 0.90, 0.5 + 0.5 * detail);
        shape += wrecks * (0.25 * h + 0.20 * drifts + 0.40 * rocks);
    }
    let atlantis = presence(l, ATLANTIS);
    if (atlantis > 0.0) {
        // Level city terraces, with soft risers between them.
        let terrace_x = 2.0 * smooth01(-0.22, 0.22, sin(TAU * x * 0.25 + 0.25 * detail)) - 1.0;
        let terrace_z = 2.0 * smooth01(-0.22, 0.22, sin(TAU * z * 0.5 + 0.25 * detail)) - 1.0;
        shape += atlantis * (-0.11 * h + 0.22 * terrace_x + 0.18 * terrace_z + 0.02 * detail);
    }
    let cave = presence(l, CAVE);
    if (cave > 0.0) {
        // Ledged basalt round the tunnel's mouths, clear of the tunnel itself.
        let ledges = 2.0 * smooth01(-0.35, 0.35, sin(TAU * abs(across) / 1.2 + 0.4 * detail)) - 1.0;
        let k = tunnel_wide(x);
        let near = smooth01(0.45 * k, 0.9 * k, abs(across));
        shape += cave * near * (0.25 * ledges + 0.12 * detail);
    }
    let abyss = presence(l, ABYSS);
    if (abyss > 0.0) {
        // Folded rock: sharp ridges from the grain, cut by contour canyons.
        let fold = 1.0 - 2.0 * abs(detail);
        let canyon = 1.0 - smooth01(0.0, 0.09, abs(h - 0.25));
        shape += abyss * (0.32 * h + 0.65 * (fold * fold - 0.35) - 0.45 * canyon);
    }
    let arctic = presence(l, ARCTIC);
    if (arctic > 0.0) {
        // Ice-scoured and flat: long shallow grooves, not banks.
        let scour = 1.0 - smooth01(0.0, 0.22, abs(sin(TAU * z * 0.5 + 0.7 * detail)));
        shape += arctic * (-0.10 * h + 0.06 * detail - 0.12 * scour);
    }
    let trench = presence(l, TRENCH);
    if (trench > 0.0) {
        shape += trench * (0.05 * h + 0.08 * detail);
    }
    let coral = presence(l, CORAL);
    if (coral > 0.0) {
        // A reef built of its own coral: flat-topped bommies and the ridges
        // joining them, lumpy with colonies, and sand gutters scoured deeper
        // between them (`world::reef_field`).
        let field = 0.5
            + 0.22 * sin(TAU * (x * 0.25 + z * 0.125) + 1.3 * detail)
            + 0.20 * sin(TAU * (x * 0.125 - z * 0.25) + 0.9 * detail)
            + 0.12 * sin(TAU * (x * 0.375 + z * 0.3125) + 2.1 * detail);
        let mound = smooth01(0.50, 0.82, field);
        let gutter = 1.0 - smooth01(0.12, 0.32, field);
        shape += coral * (0.15 * h + 0.42 * mound - 0.10 * gutter + 0.06 * detail * mound + 0.03 * detail);
    }
    let sargasso = presence(l, SARGASSO);
    if (sargasso > 0.0) {
        // A low, rolling plain under the floating weed, drifted into long
        // swells rather than risen into banks.
        let drift = sin(TAU * (x * 0.125 + z * 0.0625) + 0.9 * detail);
        let swell = smooth01(0.42, 0.74, 0.5 + 0.5 * detail);
        shape += sargasso * (0.04 * h + 0.10 * drift + 0.16 * swell);
    }
    let vents = presence(l, VENTS);
    if (vents > 0.0) {
        // Broken pillow basalt: hard, lumpy mounds round the chimneys, with
        // the grain's folds standing up as cooled lobes of lava.
        let fold = 1.0 - 2.0 * abs(detail);
        let mound = smooth01(0.58, 0.86, 0.5 + 0.5 * detail);
        shape += vents * (0.28 * h + 0.42 * (fold * fold - 0.35) + 0.30 * mound);
    }
    let brine = presence(l, BRINE);
    if (brine > 0.0) {
        // Salt terraces, broad and shallow, with the brine pans between them
        // sunk just below the crust.
        let terrace = 2.0 * smooth01(-0.55, 0.55, sin(TAU * x * 0.125 + 0.2 * detail)) - 1.0;
        let pan = smooth01(0.62, 0.88, 0.5 + 0.5 * detail);
        shape += brine * (0.04 * h + 0.055 * terrace - 0.06 * pan + 0.02 * detail);
    }
    let bloom = presence(l, BLOOM);
    if (bloom > 0.0) {
        // A soft ooze floor: low, rounded hummocks where the jellies gather.
        let ooze = sin(TAU * (x * 0.5 + z * 0.25) + 1.0 * detail)
            + 0.6 * sin(TAU * (x * 0.25 - z * 0.5) + 0.6 * detail);
        let mounds = smooth01(0.40, 0.72, 0.5 + 0.5 * detail);
        shape += bloom * (-0.05 * h + 0.10 * ooze + 0.22 * mounds);
    }
    let city = presence(l, CITY);
    if (city > 0.0) {
        // Level city blocks, stepped down in long, low terraces.
        let step = 2.0 * smooth01(-0.3, 0.3, sin(TAU * (x * 0.0625 + z * 0.125) + 0.3 * detail)) - 1.0;
        shape += city * (-0.06 * h + 0.10 * step + 0.015 * detail);
    }
    return shape;
}

// How far the tunnel has opened into a hall at `x`, and how many times the
// old bore's radius it is across there: as `world::hall` and
// `world::tunnel_stretch`, three halls to a leg joined by narrows.
fn hall(x: f32) -> f32 {
    let u = fract(x / LEG * 3.0 - 0.3 + 0.5) - 0.5;
    return smooth01(0.30, 0.10, abs(u));
}

fn tunnel_wide(x: f32) -> f32 {
    return 1.55 + 1.25 * hall(x);
}

// The lava cave's massif, rising either side of its tunnel: the part of the
// ground that climbs steeply across it.
fn massif(x: f32, z: f32, l: Leg, off: f32) -> f32 {
    let wide = TUNNEL * tunnel_wide(x);
    return presence(l, CAVE) * MASSIF * smooth01(wide * 1.05, wide * 1.8, off)
        * (0.85 + 0.3 * vnoise(x * 0.4, z * 0.4, 5u));
}

// The trench's terraces and gullies, exactly as `world::trench_profile`.
fn trench_profile(x: f32, across: f32) -> f32 {
    let u = across - 0.25 * sin(x * 0.29 + 1.0) - 0.12 * sin(x * 0.83);
    let down = 1.2 * smooth01(1.3, 1.6, u)
        + 1.5 * smooth01(2.1, 2.4, u)
        + 1.6 * smooth01(2.8, 3.2, u)
        + 0.8 * smooth01(3.3, 3.8, u);
    let up = 1.8 * smooth01(4.0, 4.35, u)
        + 1.6 * smooth01(4.9, 5.2, u)
        + 1.6 * smooth01(5.7, 6.0, u)
        + 1.3 * smooth01(6.6, 7.0, u);
    let back = 1.2 * smooth01(-1.9, -2.4, u) + 1.4 * smooth01(-3.6, -4.2, u);
    let cut = smooth01(0.55, 0.95, sin(x * 1.9 + 1.4 * sin(x * 0.61)));
    let gully = cut * (0.2 + 0.35 * smooth01(-1.2, 1.6, u)) * smooth01(-2.0, -1.2, u) * smooth01(2.8, 2.2, u);
    let strata = 2.0 * smooth01(-0.3, 0.3, sin(u * 9.0 + 0.8 * sin(x * 0.7))) - 1.0;
    let ledge = 0.12 * strata * smooth01(1.2, 1.6, u) * (1.0 - smooth01(6.8, 7.4, u));
    return up + back - down - gully + ledge;
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
    // A CPU-generated tile, scrolled slowly: one texture fetch in place of
    // the two sines and a pow of the old procedural version.
    let uv = p * 0.030 + vec2<f32>(t * 0.006, t * -0.004);
    return textureSampleLevel(caustic_map, caustic_sampler, uv, 0.0).r;
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
    // The drowned city's windows, too many and too far to tell apart,
    // scattered through the water as a warm glow low along the horizon.
    c += vec3<f32>(0.16, 0.085, 0.025) * g.levels.w * (1.0 - smooth01(0.0, 0.5, abs(dir.y - 0.04)));
    return c;
}

// The underside of the water surface, seen from below: the bright caustic
// web the waves focus, a slow swell and the sun's sheen. `dir` is the view
// ray; the surface lies at y = 0, so the pixel is that intersection. This is
// the ceiling of a shallow sea and it must read as water, not sky; the sky
// is only what the surface opens on to inside Snell's window.
fn water_ceiling(dir: vec3<f32>, t: f32) -> vec3<f32> {
    let up = max(dir.y, 0.002);
    let reach = clamp(-g.camera.y / up, 0.0, 180.0);
    let at = g.camera.xz + dir.xz * reach;
    let caustic = textureSampleLevel(
        caustic_map,
        caustic_sampler,
        fract(at * 0.30 + vec2<f32>(t * 0.008, t * -0.006)),
        0.0,
    ).r;
    let web = smooth01(0.30, 0.96, caustic);
    let swell = sin(at.x * 0.9 + t * 0.30)
        + 0.7 * sin(at.y * 0.8 - t * 0.26)
        + 0.3 * sin((at.x + at.y) * 1.7 + t * 0.50);
    let chop = sin(at.x * 2.2 - at.y * 1.8 - t * 0.6);
    let base = mix(g.deep.rgb, g.water.rgb, 0.50) * 1.05;
    let sun = 0.5 + 0.5 * sin(swell * 0.5);
    var colour = base * (0.975 + 0.05 * web) * (0.82 + 0.20 * sun)
        + vec3<f32>(0.18, 0.32, 0.31) * web * (0.06 + 0.10 * g.sun.w);
    // A soft glint where a wave face turns toward the sun; close to the
    // window, not a net across the whole dome.
    let sheen = pow(max(0.0, 0.5 + 0.5 * chop), 18.0);
    colour += vec3<f32>(0.86, 0.92, 0.82) * sheen * 0.05
        * (0.25 + g.sun.w) * smooth01(10.0, 2.0, reach);
    // Fine wave cells close to the window: the surface's own ripples, which
    // the caustic web alone is too smooth to show.
    let fine = 0.5 + 0.5 * sin(at.x * 3.7 + t * 1.3) * sin(at.y * 3.1 - t * 1.1);
    colour *= 0.96 + 0.04 * fine * smooth01(18.0, 3.0, reach);
    // Keep the ceiling a backdrop: wash it toward the open water so the
    // caustics never compete with the scene in front of them, and dissolve
    // the horizon end into the water's own haze.
    colour = mix(colour, water_at(dir), 0.42);
    colour = mix(colour, water_at(dir), smooth01(30.0, 120.0, reach));
    return colour;
}


// ---------------------------------------------------------------------------
// Sky
// ---------------------------------------------------------------------------
//
// The sky the water opens on to: the gradient overhead, the sun, the cloud
// deck and, under the ice, the aurora. A ray can only leave the water inside
// Snell's window; under the polar sheet the leads are the other way out. The
// ceiling itself is water (`water_ceiling`), never this. The deep seas have
// no sky: `g.sky.w` fades it to nothing over the cave, the vents, the bloom
// and the abyss.

fn sky_leg() -> Leg {
    return Leg(i32(g.sky.x + 0.5), i32(g.sky.y + 0.5), g.sky.z);
}

// The sky toward `dir`, from the baked cubemap of the biome or the mix of
// the two biomes at a border. Sampling a cube is one fetch, where the
// procedural version cost the Pi most of a frame in the arctic.
fn sky_at(dir_in: vec3<f32>, l: Leg) -> vec3<f32> {
    let dir = normalize(dir_in);
    let a = textureSampleLevel(sky_cubes, surface_sampler, dir, l.a, 0.0).rgb;
    if (l.w <= 0.01 || l.a == l.b) {
        return a;
    }
    let b = textureSampleLevel(sky_cubes, surface_sampler, dir, l.b, 0.0).rgb;
    return mix(a, b, l.w);
}

fn switched(bit: u32) -> bool {
    return debug(bit);
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
    light *= g.tint.rgb;
    if (sun > 0.03 && n.y > 0.2) {
        light *= 1.0 + caustic(p.xz * vec2<f32>(7.0, 9.0) + vec2<f32>(g.camera.w * 0.3, 0.0), g.camera.w * 0.9) * 0.9 * n.y * sun;
    }
    let to = p - lamp_origin();
    let d = length(to);
    let cone = smooth01(0.82, 0.95, dot(to / d, g.lamp.xyz));
    light += vec3<f32>(1.0, 0.92, 0.76) * g.lamp.w * cone * (max(dot(n, -to / d), 0.0) * 0.8 + 0.2) / (1.0 + d * d * 0.05) * 2.2;
    return light + point_lights(p, n);
}

// The point lights alone on a surface at `p` facing `n`: lava pools, lamps
// and the glows the scene places. `light_at` adds them to the sun and the
// searchlight; the cave's tunnel takes them by themselves (`vs_model`).
fn point_lights(p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    var light = vec3<f32>(0.0);
    // Four nearest-enough lights are enough for the window's scale: the
    // fifth and beyond were mostly behind the camera or beyond their range.
    let count = min(i32(g.counts.x), 4);
    for (var i = 0; i < 4; i = i + 1) {
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
        // A non-zero phase is a flickering light; the shader owns the time so
        // a cached placement keeps breathing.
        var flicker = 1.0;
        if (g.light_colours[i].w > 0.0001) {
            flicker = 0.8 + 0.2 * sin(g.camera.w * 1.9 + g.light_colours[i].w);
        }
        light += g.light_colours[i].rgb * fall * flicker * (max(dot(n, lv * inverseSqrt(max(d2, 1e-6))), 0.0) * 0.8 + 0.2);
    }
    return light;
}

// The water seen towards `p`, and how much of whatever is at `p` it hides.
// Worked out per vertex for the seabed and the models: it changes slowly
// across a triangle, and the Pi's GPU is short of arithmetic per pixel.
fn fog_at(p: vec3<f32>) -> vec4<f32> {
    let v = p - g.camera.xyz;
    let d = length(v);
    let dir = v / d;
    // Distant rock dissolves into the light scattered by the water above it,
    // not into the darker water looking level: that is what softens a far
    // silhouette against the surface instead of drawing a hard cut-out.
    let skyward = smooth01(-0.20, 0.20, dir.y);
    let haze = mix(
        water_at(dir),
        vec3<f32>(0.075, 0.20, 0.22),
        skyward * 0.55,
    );
    return vec4<f32>(haze, fog_of(d));
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
// window: the mask samples no further out than the glass's edge, bent a
// little outward (the static overlay), just past the square's sides. A ring from
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

@if(!full_hd)
@fragment
fn fs_background(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    // The boat's attitude turns the view; the water stays level. The sun's
    // shafts are drawn in the water, as `fs_shaft`, rather than on it.
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    return vec4<f32>(water_at(dir) * SEA_SCALE, 1.0);
}

@if(!full_hd)
@fragment
fn fs_background_cheap(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    let col = mix(g.deep.rgb, g.water.rgb, smooth01(-0.6, 0.5, dir.y)) * g.tint.rgb;
    return vec4<f32>(col * SEA_SCALE, 1.0);
}

@if(!full_hd)
@fragment
fn fs_background_shimmer(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    var col = water_at(dir);
    let up = smooth01(0.05, 0.65, dir.y);
    if (up > 0.0) {
        let t = g.camera.w;
        let shimmer = textureSampleLevel(
            caustic_map,
            caustic_sampler,
            dir.xz * 1.2 + vec2<f32>(t * 0.010, t * -0.013),
            0.0,
        ).r;
        let surface = mix(g.water.rgb, vec3<f32>(0.10, 0.32, 0.35), 0.55)
            * (0.85 + 0.50 * shimmer)
            + vec3<f32>(0.95, 0.88, 0.68) * (shimmer * shimmer) * (0.35 + g.sun.w * 0.8) * 0.6;
        col = mix(col, surface, up * 0.88);
    }
    return vec4<f32>(col * SEA_SCALE, 1.0);
}


// The mobile-style sea: one depth/height mix and the water's tint, no wave,
// glint or panel-sized arithmetic. Its own pipeline, so the expensive path is
// not merely branched around but absent from the shader's register budget.
@if(full_hd)
@fragment
fn fs_background_cheap(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    let col = mix(g.deep.rgb, g.water.rgb, smooth01(-0.6, 0.5, dir.y)) * g.tint.rgb;
    return vec4<f32>(col * SEA_SCALE, 1.0);
}

// The surface band: one caustic fetch that drifts across the underside,
// with the sun's flecks, instead of the two sines and the pow(14) glint.
// This is the cheap-water recommendation from the survey (WaltGD's
// screen-space caustic water, quantised to the top of the view): the fetch
// replaces three quarter-rate SFU ops on a fill-bound GPU.
@if(full_hd)
@fragment
fn fs_background_shimmer(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    var col = water_at(dir);
    let up = smooth01(0.05, 0.65, dir.y);
    if (up > 0.0) {
        let t = g.camera.w;
        let shimmer = textureSampleLevel(
            caustic_map,
            caustic_sampler,
            dir.xz * 1.2 + vec2<f32>(t * 0.010, t * -0.013),
            0.0,
        ).r;
        let surface = mix(g.water.rgb, vec3<f32>(0.10, 0.32, 0.35), 0.55)
            * (0.85 + 0.50 * shimmer)
            + vec3<f32>(0.95, 0.88, 0.68) * (shimmer * shimmer) * (0.35 + g.sun.w * 0.8) * 0.6;
        col = mix(col, surface, up * 0.88);
    }
    return vec4<f32>(col * SEA_SCALE, 1.0);
}

@if(full_hd)
@fragment
fn fs_background(in: Screen) -> @location(0) vec4<f32> {
    let s = in.ndc * g.view.x;
    let local = normalize(vec3<f32>(s, 1.0));
    let dir = local.x * g.right.xyz + local.y * g.up.xyz + local.z * g.out.xyz;
    if (switched(16u)) {
        let col = mix(g.deep.rgb, g.water.rgb, smooth01(-0.6, 0.5, dir.y)) * g.tint.rgb;
        return vec4<f32>(col * SEA_SCALE, 1.0);
    }
    var col = water_at(dir);
    let up = smooth01(0.05, 0.65, dir.y);
    // Under the water's ceiling the surface's glint is never seen: the
    // ceiling covers everything over a tenth of the way up its band.
    if (SKY != SKY_CEILING && SKY != SKY_SNELL && up > 0.0) {
        let t = g.camera.w;
        let wave = sin(dir.x * 13.0 + t * 0.8 + sin(dir.z * 9.0 - t * 0.5))
            * sin(dir.z * 10.0 - t * 0.65 + sin(dir.x * 7.0 + t * 0.4));
        let glint = pow(max(0.0, 0.5 + 0.5 * wave), 14.0);
        let surface = vec3<f32>(0.10, 0.32, 0.35) * (0.72 + 0.28 * wave)
            + vec3<f32>(0.95, 0.88, 0.68) * glint * (0.35 + g.sun.w * 0.8);
        col = mix(col, surface, up * 0.88);
    }
    // Above the horizon is either the polar ice's sky, seen through a lead,
    // or the underside of the water surface: caustics, swell and the sun's
    // sheen, never a painted cloud deck. The sky itself only appears where a
    // ray can actually leave the water, in Snell's window overhead.
    if (dir.y > 0.0) {
        if (SKY == SKY_POLAR) {
            let arctic = presence(sky_leg(), ARCTIC);
            let sky = sky_at(dir, sky_leg());
            let sky_up = smooth01(0.0, 0.12, dir.y) * g.sky.w * arctic;
            col = mix(col, sky, sky_up);
        } else if (SKY == SKY_CEILING || SKY == SKY_SNELL) {
            var ceiling = water_ceiling(dir, g.camera.w);
            if (SKY == SKY_SNELL && g.sky.w > 0.002 && dir.y > 0.64) {
                let sky = sky_at(dir, sky_leg());
                ceiling = mix(ceiling, sky, smooth01(0.64, 0.85, dir.y) * g.sky.w);
            }
            col = mix(col, ceiling, smooth01(0.0, 0.06, dir.y));
        }
    }
    return vec4<f32>(col * SEA_SCALE, 1.0);

}

// ---------------------------------------------------------------------------
// Seabed
// ---------------------------------------------------------------------------

@if(!full_hd)
struct Ground {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // `fog_at` the vertex.
    @location(2) fog: vec4<f32>,
    // `voyage_x` at the vertex. It bends gently, so it is worked out per
    // vertex, and only which biome it falls in is found per pixel.
    @location(3) voyage: f32,
    // Sun, lamp, point lights and caustics at the vertex, and the albedo
    // modifier from the fine noise, sand ripples and paving.
    @location(4) light_scale: vec4<f32>,
    // The lava cave's glow at the vertex.
    @location(5) emissive: vec3<f32>,
};

@if(!full_hd)
@vertex
fn vs_terrain(@location(0) world: vec3<f32>, @location(1) normal: vec3<f32>) -> Ground {
    // Built on the CPU a tile at a time, already in the world (`terrain.rs`).
    var out: Ground;
    out.world = world;
    out.normal = normal;
    out.position = project(world);
    out.fog = fog_at(world);
    let voyage = voyage_x(world.x, world.z);
    out.voyage = voyage;
    let l = leg(voyage);
    let n = normalize(normal);
    let detail = exp(-length(world - g.camera.xyz) * 0.25);
    let steep = smooth01(0.12, 0.40, 1.0 - n.y);
    var scale = 1.0;
    var emissive = vec3<f32>(0.0);
    if (!switched(8u)) {
        var scale = 1.0;
        if (detail > 0.02) {
            scale *= mix(1.0, 0.65 + 0.5 * vnoise(world.x * 14.0, world.z * 14.0, 9u), detail * (1.0 - steep));
            let sand = presence(l, SHALLOWS) + 0.5 * presence(l, FOREST) + 0.5 * presence(l, ARCTIC);
            if (sand > 0.0) {
                let ripple = sin((world.x * 0.8 + world.z) * 26.0 + vnoise(world.x * 3.0, world.z * 3.0, 5u) * 4.0);
                scale *= 1.0 - 0.16 * sand * detail * smooth01(0.3, 1.0, ripple);
            }
        }
        let paving = presence(l, ATLANTIS);
        if (paving > 0.0) {
            let row = floor(world.z * 4.0);
            let joint = min(abs(fract(world.x * 3.0 + row * 0.5) - 0.5), abs(fract(world.z * 4.0) - 0.5));
            scale *= 1.0 - paving * 0.45 * smooth01(0.45, 0.5, joint) * detail;
        }
        var emissive = vec3<f32>(0.0);
        let cave = presence(l, CAVE);
        if (cave > 0.0) {
            let flicker = 0.80 + 0.20 * sin(g.camera.w * 1.7 + world.x * 3.0) * sin(g.camera.w * 2.9 + world.z * 2.0);
            let veins = 1.0 - smooth01(0.0, 0.04, abs(vnoise(world.x * 1.4, world.z * 1.9, 6u) - 0.5));
            var fine = 0.0;
            if (detail > 0.01) {
                fine = (1.0 - smooth01(0.0, 0.03, abs(vnoise(world.x * 4.0, world.z * 5.0, 7u) - 0.5))) * detail;
            }
            let pools = smooth01(0.66, 0.74, vnoise(world.x * 0.55 + 7.0, world.z * 0.55, 8u));
            emissive = vec3<f32>(1.0, 0.36, 0.07) * (veins * 0.9 + fine * 0.5 + pools * 1.4) * flicker * cave * (1.0 - steep);
        }
    }
    if (switched(8u)) {
        let sun = g.sun.w * exp(-max(g.levels.x - world.y, 0.0) * 0.22);
        let cheap = g.tint.rgb * (
            vec3<f32>(0.55, 0.92, 0.86) * sun * max(dot(n, g.sun.xyz) * 0.75 + 0.25, 0.0) * 1.5
            + vec3<f32>(0.05, 0.10, 0.12) * (0.35 + 0.65 * (n.y * 0.5 + 0.5)) * (sun * 2.0 + 0.25)
        );
        out.light_scale = vec4<f32>(cheap, 1.0);
    } else {
        out.light_scale = vec4<f32>(light_at(world, n), scale);
    }
    out.emissive = emissive;
    return out;
}

fn floor_albedo(l: Leg) -> vec3<f32> {
    var albedos = array<vec3<f32>, 14>(
        vec3<f32>(0.62, 0.56, 0.42),
        vec3<f32>(0.40, 0.36, 0.26),
        vec3<f32>(0.32, 0.30, 0.26),
        vec3<f32>(0.46, 0.44, 0.39),
        vec3<f32>(0.10, 0.09, 0.09),
        vec3<f32>(0.17, 0.16, 0.17),
        vec3<f32>(0.36, 0.37, 0.38),
        vec3<f32>(0.58, 0.55, 0.47),
        vec3<f32>(0.80, 0.72, 0.58),
        vec3<f32>(0.39, 0.36, 0.22),
        vec3<f32>(0.08, 0.07, 0.07),
        vec3<f32>(0.74, 0.73, 0.68),
        vec3<f32>(0.20, 0.20, 0.24),
        vec3<f32>(0.25, 0.27, 0.23),
    );
    return mix(albedos[l.a], albedos[l.b], l.w);
}

fn surface_of(l: Leg) -> vec2<i32> {
    // Sand, sand, silt, sand, basalt, silt, sand, silt, then reef, silt,
    // basalt, salt (ice's crystalline layer), ooze (silt) and the drowned
    // city's silted paving: by biome.
    var layers = array<i32, 14>(12, 12, 13, 12, 10, 13, 12, 13, 17, 13, 10, 11, 13, 13);
    return vec2<i32>(layers[l.a], layers[l.b]);
}

@if(!full_hd)
@fragment
fn fs_terrain(in: Ground) -> @location(0) vec4<f32> {
    let p = in.world;
    let l = leg(in.voyage);
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
    albedo *= in.light_scale.w;
    let colour = albedo * in.light_scale.xyz + in.emissive;
    let far = smooth01(10.0, 35.0, length(p - g.camera.xyz));
    return vec4<f32>(
        mix(colour, in.fog.rgb, max(in.fog.a, far * 0.55)) * SEA_SCALE,
        1.0,
    );
}


// The full-HD profile shades at 1080 × 1080, so the seabed gets a single
// texture lookup and a light worked out per vertex: no triplanar projections,
// no point-light loop, no lava noise. WESL leaves this out of the 448 build.
@if(full_hd)
struct Ground {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) light: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    @location(3) fog: vec4<f32>,
    @location(4) @interpolate(flat) layer: i32,
    @location(5) lod: f32,
    @location(6) world: vec3<f32>,
    // How much of the lava cave is here, from the tile's baked glow.
    @location(7) cave: f32,
    // How much living reef covers the coral's seabed here, from the
    // fraction of the tile's layer (`terrain::REEF_SHARE`).
    @location(8) reef: f32,
};

@if(full_hd)
@vertex
fn vs_terrain(
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) baked: vec4<f32>,
    @location(3) shade: vec4<f32>,
    @location(4) glow: vec4<f32>,
) -> Ground {
    var out: Ground;
    out.position = project(world);
    out.fog = fog_at(world);
    out.uv = world.xz * 1.3;
    out.world = world;
    out.cave = max(glow.a, 0.0);
    out.layer = i32(baked.w + 0.5);
    out.reef = clamp((baked.w - floor(baked.w + 0.02)) / REEF_SHARE, 0.0, 1.0);
    out.lod = clamp(log2(length(world - g.camera.xyz) * 2.0), 0.0, 8.0);
    let n = normalize(normal);
    // The tile carries its floor's colour, detail and veins at full strength
    // (`terrain::build`); the distance only says how much of the fine detail
    // shows, and the cave's glow still breathes with the clock.
    let detail = exp(-length(world - g.camera.xyz) * 0.25);
    var scale = mix(1.0, shade.x, detail * shade.y);
    scale *= 1.0 - shade.z * detail;
    scale *= 1.0 - shade.w * detail;
    let flicker = 0.80 + 0.20 * sin(g.camera.w * 1.7 + world.x * 3.0) * sin(g.camera.w * 2.9 + world.z * 2.0);
    // A negative fourth marks the drowned city's lamplight pooled about its
    // buildings (`world::Spill`): light on the floor, not a glow of its own.
    let city = select(0.0, 1.0, glow.a < 0.0);
    out.albedo = baked.rgb * scale + glow.rgb * (flicker * 0.35 * (1.0 - city));
    if (switched(8u)) {
        let sun = g.sun.w * exp(-max(g.levels.x - world.y, 0.0) * 0.22);
        out.light = g.tint.rgb * (
            vec3<f32>(0.55, 0.92, 0.86) * sun * max(dot(n, g.sun.xyz) * 0.75 + 0.25, 0.0) * 1.5
            + vec3<f32>(0.05, 0.10, 0.12) * (0.35 + 0.65 * (n.y * 0.5 + 0.5)) * (sun * 2.0 + 0.25)
        );
    } else {
        out.light = light_at(world, n);
    }
    out.light += glow.rgb * (city * CITY_SPILL);
    return out;
}

// How bright the city's lamplight is on the floor it pools on.
const CITY_SPILL: f32 = 2.2;

@if(full_hd)
@fragment
fn fs_terrain(in: Ground) -> @location(0) vec4<f32> {
    // The cave's massif is a cliff of rock, not a floor: projecting it from
    // above would smear its grain straight up the face. Project world height
    // and the horizontal along the wall instead. It is the same single sample,
    // only a different pair of coordinates and the tunnel's own layer; the
    // derivative normal is worked out once, outside the branch, and the level
    // of detail is unchanged.
    let face = cross(dpdx(in.world), dpdy(in.world));
    let cliff = in.cave > 0.02 && face.y * face.y < 0.30 * dot(face, face);
    var uv = in.uv;
    var layer = in.layer;
    if (cliff) {
        // At the tunnel's own scale (its 1.5 repeats, `vs_model`'s 1.35),
        // so where the massif shows inside the arch its rock is the same.
        uv = vec2<f32>(in.world.x + in.world.z, in.world.y) * 2.0;
        layer = VAULT_LAYER;
    }
    var sample = vec4<f32>(0.6 / TEXTURE_RANGE, 0.5, 0.5, 0.0);
    if (!PLAIN_TEXTURES) {
        sample = textureSampleLevel(
            surfaces,
            surface_sampler,
            uv,
            layer,
            in.lod,
        );
    }
    var texture = sample.rgb * TEXTURE_RANGE;
    // The reef's carpet of colonies shows only where they live: down the
    // sand gutters it fades to a faint rubble.
    if (layer == REEF_LAYER) {
        texture = mix(vec3<f32>(0.92) + (texture - vec3<f32>(0.92)) * 0.25, texture, in.reef);
    }
    var glow = vec3<f32>(0.0);
    if (cliff) {
        // The tunnel's rock (`vault_hd`) without its lighting: its tone,
        // brighter on the ledges that face up to the water's light and
        // darker under their lips, and the odd crack still running molten,
        // so the massif reads as ledged lava and not a smooth brown hill.
        let relief = 0.75 + 1.6 * (sample.b - 0.5) + 0.5 * (sample.g - 0.5);
        texture = vec3<f32>(sample.r * TEXTURE_RANGE * relief);
        glow = vec3<f32>(1.0, 0.32, 0.05) * (sample.a * in.cave * 0.45);
    }
    var albedo = in.albedo * mix(vec3<f32>(1.0), texture, 0.85);
    let colour = albedo * in.light + glow;
    var shaded = mix(
        colour,
        in.fog.rgb,
        max(in.fog.a, smooth01(10.0, 35.0, length(in.world - g.camera.xyz)) * 0.55),
    );
    // Underwater grade: the water takes the colour out of the distance and
    // lifts the blue, folded into the fog mix that was already happening.
    if (switched(32u)) {
        let t = in.fog.a;
        let luma = dot(shaded, vec3<f32>(0.299, 0.587, 0.114));
        shaded = mix(shaded, vec3<f32>(luma), 0.15 * t)
            + vec3<f32>(0.010, 0.030, 0.040) * (t * 0.3);
    }
    // Low-poly facets: the triangle's own normal, banded into three sun
    // steps, over the smooth per-vertex light. The mesh is coarser and the
    // facets are the look, not a defect to hide.
    if (switched(64u)) {
        var face = normalize(cross(dpdx(in.world), dpdy(in.world)));
        if (face.y < 0.0) {
            face = -face;
        }
        let base = clamp(dot(face, g.sun.xyz) * 0.75 + 0.25, 0.0, 1.0);
        let band = floor(base * 3.0 + 0.5) / 3.0;
        shaded *= 0.65 + 0.55 * band;
    }
    return vec4<f32>(shaded * SEA_SCALE, 1.0);
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
    // rgb: the seabed's colour under the model, from the CPU.
    @location(11) floor: vec4<f32>,
};

struct Lit {
    @builtin(position) position: vec4<f32>,
    @location(0) world: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) albedo: vec3<f32>,
    @location(3) @interpolate(flat) glow: f32,
    @location(4) @interpolate(flat) material: u32,
    @location(5) local: vec3<f32>,
    @location(6) local_normal: vec3<f32>,
    @location(7) @interpolate(flat) surface: vec4<f32>,
    @location(8) @interpolate(flat) fade: f32,
    // The seabed's height under a grounded model, and the colour of the sand
    // there. `world::build` finds both once per instance; the height is kept
    // per pixel, because interpolating the burial across a tall triangle would
    // spread the foot's blend all the way up it.
    @location(9) floor_y: f32,
    @location(11) @interpolate(flat) floor: vec3<f32>,
    // `fog_at` the vertex.
    @location(10) fog: vec4<f32>,
    // The drowned city's: whether the building stands on the seabed, which
    // darkens its foot (`city_light`).
    @location(12) @interpolate(flat) grounded: f32,
};

// Bend a model where it lives: plants sway, fish sweep their tails sideways
// and whales beat their flukes up and down, and tentacles writhe. The jellies
// swim in `swim`.
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
    } else if (kind == 6u) {
        // A shell hinged at its base. The placement used to bake this from
        // w.t; the shader does it from the same clock, so the instance is
        // static and cacheable.
        let a = 0.35 + anim.z * sin(t);
        let y = p.y * cos(a) - p.z * sin(a);
        p.z = p.y * sin(a) + p.z * cos(a);
        p.y = y;
    } else if (kind == 7u) {
        // An arm waving about the vertical: the trench crinoids, which used
        // to bake the wave into the placement's bearing.
        let a = anim.z * sin(t);
        let c = cos(a);
        let s = sin(a);
        let x = p.x * c + p.z * s;
        p.z = -p.x * s + p.z * c;
        p.x = x;
    } else if (kind == 8u) {
        // A flipper beating about the forward axis: the polar penguins.
        let a = anim.z * sin(t);
        let y = p.y * cos(a) - p.z * sin(a);
        p.z = p.y * sin(a) + p.z * cos(a);
        p.y = y;
    } else if (kind == 9u) {
        // A narwhal's whole body: a slow bob and yaw around the pod's
        // formation, with the fluke's beat. The placement used to bake the
        // first two from w.t.
        let clock = g.camera.w;
        p.y += 0.03 * sin(clock * 0.3 + anim.y);
        let yaw = 0.05 * sin(clock * 0.2 + anim.y);
        let x = p.x * cos(yaw) + p.z * sin(yaw);
        p.z = -p.x * sin(yaw) + p.z * cos(yaw);
        p.x = x;
        let back = (1.0 - aux.x) * 0.5;
        p.y += anim.z * back * back * sin(t - back * 4.4);
    }
    return p;
}

// A medusa swimming, and a comb jelly gliding: the vertex in the model's
// own frame, how far the whole animal has risen in the water's, in bell
// radii, and how hard the bell is contracted where this vertex is.
struct Swim {
    pos: vec3<f32>,
    rise: vec3<f32>,
    beat: f32,
};

// How far through its stroke a medusa is, from 0 to 1, `lag` strokes late.
fn stroke_at(anim: vec4<f32>, lag: f32) -> f32 {
    return fract((g.camera.w * anim.w + anim.y) / TAU - lag);
}

// The bell's contraction through a stroke: a quick squeeze over the first
// quarter, then a slow relaxation, as the coronal muscle snaps and the
// jelly springs back.
fn squeeze_of(u: f32) -> f32 {
    return smoothstep(0.0, 0.26, u) * (1.0 - smoothstep(0.26, 1.0, u));
}

// The animal's height through a stroke, in bell radii: it shoots up on the
// contraction and coasts, then sinks slowly while the bell refills. Over a
// whole stroke the two cancel, so the drift stays where it was placed.
fn rise_of(u: f32) -> f32 {
    return 0.55 * (smoothstep(0.04, 0.50, u) - u);
}

// The kinds `world::anim` calls PULSE (5), TRAIL (10) and GLIDE (11). The
// three parts of one medusa share its phase and rate, so the bell, its oral
// arms and its fringe of tentacles keep the one stroke: the arms and threads
// take it late, the further down them the later, so the stroke travels down
// them and their tips trail the bell as it rises.
// `stretch` is how many bell radii long the part is drawn: a lion's mane's
// tentacles wave wider than a moon jelly's short fringe.
fn swim(p_in: vec3<f32>, aux: vec2<f32>, anim: vec4<f32>, kind: u32, stretch: f32) -> Swim {
    var out: Swim;
    var p = p_in;
    let clock = g.camera.w;
    // The water's slow set and swell, which the stroke rides on.
    out.rise = vec3<f32>(0.30 * sin(clock * 0.09 + anim.y), 0.35 * sin(clock * 0.13 + anim.y * 1.7), 0.0);
    if (kind == 11u) {
        // A comb jelly turns slowly about its long axis and bobs, nodding a
        // little as it goes; its cilia's rainbow is the fragment's.
        let a = clock * anim.w + anim.y;
        let c = cos(a);
        let s = sin(a);
        p = vec3<f32>(p.x * c + p.z * s, p.y, -p.x * s + p.z * c);
        let nod = anim.z * 0.4 * sin(clock * 0.7 + anim.y);
        p.x += nod * (p.y - 0.5);
        out.rise.y += anim.z * sin(clock * 0.8 + anim.y * 2.0);
        out.pos = p;
        out.beat = 0.5 + 0.5 * sin(clock * 1.9 + anim.y);
        return out;
    }
    // How far down an arm or thread this vertex hangs; the bell is all root.
    let along = select(0.0, aux.x, kind == 10u);
    let u = stroke_at(anim, along * 0.38);
    let c = squeeze_of(u);
    out.beat = c;
    out.rise.y += rise_of(u);
    if (kind == 5u) {
        // The bell: the margin narrows most, the crown hardly at all, and
        // the bell grows taller as it narrows; the margin's lip flicks in
        // and down at the height of the stroke.
        let h = aux.y;
        let narrow = 1.0 - anim.z * c * (1.0 - 0.75 * h);
        p.x *= narrow;
        p.z *= narrow;
        p.y *= 1.0 + 1.3 * anim.z * c;
        p.y -= 0.35 * anim.z * c * (1.0 - smoothstep(0.0, 0.18, h));
    } else {
        // An arm or a thread: the margin pulls its root in with the stroke,
        // the pull reaching further down it the later; it streams in the
        // water's set, and the stroke's wave runs down it, sideways and
        // out, growing toward the tip. Each thread's own bearing round the
        // bell staggers its wave, so the fringe never moves as one comb.
        let pull = 1.0 - anim.z * c * (1.0 - 0.3 * along);
        p.x *= pull;
        p.z *= pull;
        let bearing = atan2(p_in.z, p_in.x);
        let t = clock * anim.w + anim.y;
        let k = t - along * 7.0 + bearing * 2.0;
        let reach = along * (0.10 + 0.16 * along) * (0.7 + 0.6 * c) * (0.6 + 0.4 * stretch);
        p.x += reach * sin(k) + 0.20 * along * along * sin(clock * 0.31 + anim.y);
        p.z += reach * cos(k * 0.9 + 1.1) + 0.14 * along * along * cos(clock * 0.23 + anim.y * 1.3);
    }
    out.pos = p;
    return out;
}

@vertex
fn vs_model(v: Model, i: Instance) -> Lit {
    let kind = u32(i.anim.x + 0.5);
    var p = v.pos;
    var rise = vec3<f32>(0.0);
    var beat = 0.0;
    // Nothing that swims stands on the seabed, so the grounded pipelines,
    // which draw the reef, compile none of it.
    let swimming = !GROUNDED && (kind == 5u || kind == 10u || kind == 11u);
    if (swimming) {
        let across = length(vec3<f32>(i.m0.x, i.m1.x, i.m2.x));
        let stretch = length(vec3<f32>(i.m0.y, i.m1.y, i.m2.y)) / max(across, 1e-6);
        let s = swim(v.pos, vec2<f32>(v.aux), i.anim, kind, stretch);
        p = s.pos;
        rise = s.rise;
        beat = s.beat;
    } else {
        p = animate(v.pos, vec2<f32>(v.aux), i.anim);
    }
    var world = vec3<f32>(dot(i.m0, vec4<f32>(p, 1.0)), dot(i.m1, vec4<f32>(p, 1.0)), dot(i.m2, vec4<f32>(p, 1.0)));
    // The rise is the water's, not the model's: the bell and everything
    // hanging from it climb together, whatever each part's own stretch. Its
    // unit is the bell's radius, which every part's first axis carries.
    world += rise * length(vec3<f32>(i.m0.x, i.m1.x, i.m2.x));
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
    out.local = v.pos;
    out.local_normal = v.normal;
    out.surface = i.surface;
    out.fade = i.extra.z;
    out.fog = fog_at(world);
    // The GPU path: sun, lamp, point lights and caustics at the vertex.
    var light = vec3<f32>(0.0);
    if (switched(4u)) {
        let light_n = normalize(n);
        let sun = g.sun.w * exp(-max(g.levels.x - world.y, 0.0) * 0.22);
        light = g.tint.rgb * (
            vec3<f32>(0.55, 0.92, 0.86) * sun * max(dot(light_n, g.sun.xyz) * 0.75 + 0.25, 0.0) * 1.5
            + vec3<f32>(0.05, 0.10, 0.12) * (0.35 + 0.65 * (light_n.y * 0.5 + 0.5)) * (sun * 2.0 + 0.25)
        );
    } else {
        light = light_at(world, normalize(n));
    }
    out.albedo *= light;
    // `world::build` finds the ground and the sand's colour under the model,
    // once per instance.
    out.floor_y = i.extra.w;
    out.floor = i.floor.rgb;
    // Nothing that swims is grounded, so the glass pass reads these two
    // slots as how hard the bell is contracted here (the light of the stroke
    // runs down the arms and threads with it) and as the animal's own tint
    // before the water's light.
    if (swimming) {
        out.floor_y = beat;
        out.floor = i.colour.rgb;
    }
    // The tunnel's vertices carry their height over its floor and how dark
    // their rock is in `aux`, and the light its passages throw on them as
    // their colour (`world::tunnel`).
    if (MATERIAL == VAULT) {
        out.albedo = i.colour.rgb * f32(v.aux.y);
        // No sun reaches in here, and the searchlight is the fragment's: the
        // walls take only the lava pools' light, summed here once a vertex
        // and carried with the passages' baked spill, both to be laid over
        // the rock's tone per pixel. The arch's normals face into the rock,
        // so they are turned toward the window first, as the fragment does.
        var facing = normalize(n);
        if (dot(facing, g.camera.xyz - world) < 0.0) {
            facing = -facing;
        }
        out.local_normal = v.colour + point_lights(world, facing) * out.albedo * 1.2;
        // Where the rock still runs hot: broad patches whose cracks glow,
        // most of them low down, where the lava flowed last. A noise a
        // vertex, so no pixel pays for it.
        let above = f32(v.aux.x);
        let heat = smooth01(0.45, 0.70, vnoise(world.x * 0.9 + world.z * 0.5, world.y * 2.2, 31u))
            * (1.0 - 0.75 * smooth01(0.15, 1.1, above));
        out.local = vec3<f32>(above, heat, 0.0);
        // Hot water hangs in the tunnel: the far end of it fades into a dull
        // red glow instead of the cold sea, so the passage ahead reads as
        // deep and lit, and the near rock stands out dark against it.
        out.fog = vec4<f32>(mix(out.fog.rgb, vec3<f32>(0.20, 0.07, 0.03), 0.65 * g.world.z), out.fog.a);
    }
    // The city's parts mark what they are in the length of the normal they
    // carry (`city_light`), and the windows take a seed from where the
    // instance stands; its texture is the world's, so neither is missed.
    out.grounded = 0.0;
    if (MATERIAL == CITY_LIGHTS) {
        out.local_normal = v.normal * (1.0 + f32(v.aux.x));
        out.surface.w = floor(fract(i.m0.w * 0.1731 + i.m2.w * 0.0913) * 4096.0);
        out.grounded = i.floor.w;
        // How much a part shines like polished brass, from its colour
        // (`mesh::city_part`), per vertex in the slot a grounded model keeps
        // its seabed's height in: a city model is never grounded.
        out.floor_y = f32(v.aux.y);
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
    if (PLAIN_TEXTURES) {
        return vec3<f32>(0.6 * weight);
    }
    // The level of detail a sample with these gradients picks, worked out
    // here: the Pi's GPU has no sampling by gradients, and Mesa works the same
    // level out for it in a longer way, at twice the cost of the lookup itself.
    let lod = 0.5 * log2(max(dot(ddx, ddx), dot(ddy, ddy))) + SURFACE_LOD;
    return textureSampleLevel(surfaces, surface_sampler, uv, layer, lod).rgb * weight;
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

// The lava cave's tunnel and its passages (`world::tunnel`), a material of
// its own: its pipeline compiles only this, and no other pays for it.
const VAULT: i32 = 5;

// The drowned city's towers, domes and signs (`world::material::CITY`), a
// material of its own for the same reason. Each part of a model says what it
// is in `aux.x`, which the vertex shader carries to the pixel as the length
// of the local normal: 1 plain brass and iron, 2 a wall of windows, 3 glass
// panes lit from within, 4 neon. The neon's colour is the instance's own, in
// its `floor` (a city model never stands on the seabed's blend), and the
// windows' pattern is seeded by where the instance stands.
const CITY_LIGHTS: i32 = 11;

// What a city surface adds to its own lit colour, and how much of that it
// keeps: `lamps` is the light it gives out, laid on after the fog so a far
// window still glows; `sheen` is what it mirrors of the water and the haze
// round its edges, fogged with the surface; `keep` scales its own colour,
// for the dark glass of an unlit window and the silted, shadowed foot of a
// building. Everything here is arithmetic on what the vertex already
// carries: no texture, no extra pass, nothing blended, which is what the
// Pi's tiler is good at, and the rooms behind the windows are the only part
// with any length to it, worked out only where a lit pane is near enough to
// be seen as more than a dot.
struct City {
    lamps: vec3<f32>,
    sheen: vec3<f32>,
    keep: f32,
};

// The drowned city's surfaces. `n` is the surface's world normal and `v`
// the way to the camera, both unit length; `distance` how far off it is.
fn city_light(in: Lit, n: vec3<f32>, v: vec3<f32>, distance: f32, t: f32) -> City {
    var out: City;
    out.lamps = vec3<f32>(0.0);
    out.keep = 1.0;
    let seed = in.surface.w;
    let local = in.local;
    let kind = length(in.local_normal) - 1.0;
    let facing = clamp(dot(n, v), 0.0, 1.0);
    let edge = 1.0 - facing;
    let fresnel = edge * edge * edge;
    let r = reflect(-v, n);
    // The haze the water scatters round every silhouette: a rim of the
    // water's own colour where a surface turns away, so a dark tower stands
    // off the dark water behind it instead of merging with it.
    out.sheen = in.fog.rgb * fresnel * 0.7;
    if (kind > 2.3) {
        // Neon: a steady buzz and, a few times a minute, a stutter. Its tube
        // is white-hot where it faces the window and its own colour round
        // its edges, the way a lit glass tube reads in a photograph.
        let slot = floor(t * 6.0 + seed);
        let stutter = select(1.0, 0.2, fract(sin(slot * 12.9898 + seed) * 43758.5) < 0.05);
        let core = facing * facing;
        out.lamps = (in.floor * (1.9 + 0.2 * sin(t * 37.0 + seed)) + vec3<f32>(0.30) * core * core) * stutter;
        return out;
    }
    // The water every polished or glazed part mirrors: one tone, between
    // the deep below and the lit water above. A gradient keyed to the
    // reflected ray reads no better from the street, and on the Pi its
    // registers slowed every city pixel by a millisecond and more.
    let env = mix(g.deep.rgb, g.water.rgb * 1.6, 0.55);
    if (kind > 1.4) {
        // Glass panes leaded in an iron lattice, lit warm from within. The
        // lattice is in the world, so a walkway stretched between two towers
        // keeps square panes; far off the leading is too fine to see. The
        // glass mirrors the water, most where it is seen edge on.
        let q = abs(fract(in.world * 5.5) - vec3<f32>(0.5));
        let lead = min(min(q.x, q.y), q.z);
        let pane = mix(smooth01(0.03, 0.08, lead), 0.8, smooth01(10.0, 22.0, distance));
        out.lamps = vec3<f32>(0.95, 0.64, 0.30) * (0.06 + 0.32 * pane);
        out.sheen += env * (0.12 + 0.9 * fresnel) * pane;
        out.keep = 0.55;
        return out;
    }
    // A building's foot, silted and in its own shadow: darker towards the
    // seabed it stands on, over its plinth and its first floor. Baked
    // ambient occlusion in all but name, from the model's own height, so it
    // costs a smoothstep and no vertices.
    let foot = (1.0 - smooth01(-0.35, 0.95, local.y)) * in.grounded;
    out.keep = 1.0 - 0.55 * foot;
    // Polished brass mirrors the water above and the glow along the
    // horizon, with a bright band where the surface turns it to the light
    // over the city: a matcap worked out from the reflected ray.
    let brass = in.floor_y;
    if (brass > 0.01) {
        let band = exp(-60.0 * (r.y - 0.30) * (r.y - 0.30));
        out.sheen += vec3<f32>(1.0, 0.74, 0.40) * brass * (1.0 - foot)
            * (env * (0.22 + 0.9 * fresnel) + vec3<f32>(0.06, 0.05, 0.04) * band);
    }
    if (kind < 0.5) {
        return out;
    }
    // A wall of windows on a grid of floors, in the model's own units, on
    // whichever of its vertical faces this is; the roofs and ledges have none.
    let nl = in.local_normal / (kind + 1.0);
    let wall = 1.0 - smooth01(0.35, 0.6, abs(nl.y));
    if (wall <= 0.0) {
        return out;
    }
    let side = abs(nl.x) > abs(nl.z);
    let u = select(local.x, local.z, side);
    let face = select(select(0u, 1u, nl.z > 0.0), select(2u, 3u, nl.x > 0.0), side);
    let cell = vec2<f32>(u / 0.11, local.y / 0.16);
    let c = floor(cell);
    let f = cell - c;
    let h = hash(i32(c.x), i32(c.y), u32(seed) * 4u + face);
    var lit = step(0.55, h);
    let pane = smooth01(0.10, 0.18, f.x) * smooth01(0.90, 0.82, f.x)
        * smooth01(0.14, 0.24, f.y) * smooth01(0.86, 0.78, f.y);
    // Mostly lamplight; now and then a cold, greenish strip-lit room.
    var warm = mix(vec3<f32>(1.0, 0.58, 0.22), vec3<f32>(0.70, 0.90, 0.66), step(0.93, h));
    var bright = 0.45 + 0.75 * fract(h * 7.31);
    // A few windows flicker, each at its own rate, as failing lamps do.
    let fault = fract(h * 13.7);
    if (fault < 0.07) {
        bright *= step(0.35, fract(sin(floor(t * (3.0 + fault * 90.0)) + h * 91.0) * 43758.5));
    }
    // A lift in its shaft up some faces: one column of dark windows with a
    // lit car riding up and down it, slow, pausing at the ends of its run.
    let shaft = hash(i32(face), -7, u32(seed));
    if (shaft < 0.55 && c.x == floor(fract(shaft * 9.7) * 5.0) - 2.0) {
        let ride = 1.5 + 9.0 * smooth01(-0.8, 0.8, sin(t * 0.21 + shaft * 40.0));
        lit = 1.0 - smooth01(0.30, 0.55, abs(cell.y - 0.5 - ride));
        bright = 1.25;
        warm = vec3<f32>(0.85, 0.92, 1.0);
    }
    // Far off the grid is finer than the pixels: its average, steady.
    let far = smooth01(22.0, 45.0, distance);
    // The rooms only near: past eighteen units a pane is a few pixels, and
    // the room fades to its average tint, so nothing pops.
    let plain = smooth01(10.0, 18.0, distance);
    var room = vec3<f32>(1.0);
    if (lit * pane > 0.001 && plain < 0.999) {
        room = mix(city_room(f, h, t), vec3<f32>(1.0), plain);
    }
    let glow = mix(lit * pane * bright * room, vec3<f32>(0.22), far) * wall;
    out.lamps = warm * glow;
    // Dark glass where a room is unlit, mirroring the water; a lit pane
    // mirrors it faintly over its own light.
    let glass = pane * wall * (1.0 - far);
    out.keep *= 1.0 - 0.45 * glass * (1.0 - lit);
    out.sheen += env * (0.10 + 0.8 * fresnel) * glass * (1.0 - 0.7 * lit);
    return out;
}

// The room behind a lit window, painted flat on the pane: from the
// street a room's depth does not read, so rather than follow the view ray
// into a box (as interior mapping would, for a few dozen operations a
// pixel) the pane is lit as a room seen straight on. `f` is where in its
// cell the pane is struck, `h` the window's own hash. Each room is
// furnished from that hash: the lamp hangs to one side or the other and
// lights the ceiling over it, the floor and the corners are darker, the
// wallpaper is its own colour, some have their curtains half drawn or a
// blind half down, and in a few someone paces the back wall. The result is
// a tint about 1 on average, so the city's far, averaged glow is unchanged.
fn city_room(f: vec2<f32>, h: f32, t: f32) -> vec3<f32> {
    let lamp = 0.30 + 0.40 * fract(h * 3.13);
    let lx = f.x - lamp;
    let ly = f.y - 0.80;
    let spot = exp(-9.0 * (lx * lx + ly * ly));
    // The back wall in the middle of the pane, the side walls, ceiling and
    // floor towards its edges: darker low down, lit over the lamp.
    let edge = max(abs(f.x - 0.5), abs(f.y - 0.5));
    let rim = smooth01(0.28, 0.42, edge);
    var shade = (0.95 + spot) * mix(1.0, mix(0.70, 1.20, step(0.5, f.y)), rim);
    shade *= 1.12 - 0.12 * rim;
    // Someone at home, pacing slowly across the back wall.
    if (fract(h * 23.1) < 0.08) {
        let x = 0.3 + 0.4 * (0.5 + 0.5 * sin(t * 0.23 + h * 50.0));
        let body = (1.0 - smooth01(0.11, 0.15, abs(f.x - x) + 0.25 * max(f.y - 0.45, 0.0)))
            * (1.0 - smooth01(0.56, 0.60, f.y));
        let head = 1.0 - smooth01(0.08, 0.10, length(vec2<f32>(f.x - x, (f.y - 0.69) * 0.8)));
        shade *= 1.0 - 0.85 * max(body, head) * (1.0 - rim);
    }
    // The wallpaper: rose, sage or cream.
    let paper = fract(h * 5.71);
    var colour = mix(vec3<f32>(1.0, 0.82, 0.78), vec3<f32>(0.82, 1.0, 0.80), step(0.33, paper));
    colour = mix(colour, vec3<f32>(1.05, 0.98, 0.86), step(0.66, paper)) * shade;
    // Curtains half drawn across some windows, lit through and coloured; a
    // slatted blind half down others. Both hang in the pane itself.
    let style = fract(h * 9.13);
    if (style < 0.30) {
        let gap = 0.10 + 0.22 * fract(h * 17.9);
        let drawn = smooth01(gap, gap + 0.05, abs(f.x - 0.5));
        let cloth = mix(vec3<f32>(1.1, 0.42, 0.30), vec3<f32>(0.55, 0.85, 0.45), step(0.15, style))
            * (0.75 + 0.25 * sin(f.x * 40.0));
        colour = mix(colour, cloth, drawn);
    } else if (style < 0.45) {
        let down = 0.35 + 0.4 * fract(h * 31.7);
        let blind = 0.55 + 0.35 * step(0.5, fract(f.y * 9.0));
        colour = mix(colour, vec3<f32>(blind * 1.1), step(down, f.y));
    }
    return colour;
}

// The tunnel's rock: basalt laid down flow on flow, wet where the
// searchlight catches it, glowing red low down where the lava lights it and
// veined with molten rock, and lit by what glows in its passages. How high
// each point stands over the floor and the light the passages throw on it
// are baked into the tunnel's vertices (`vs_model`), so none of it is
// worked out per pixel.
fn vault(in: Lit) -> vec3<f32> {
    let p = in.world;
    let above = in.local.x;
    let t = g.camera.w;
    var n = normalize(in.normal);
    let to_camera = normalize(g.camera.xyz - p);
    if (dot(n, to_camera) < 0.0) {
        n = -n;
    }
    // The tunnel's layer keeps its tone in red; the rest is for `vault_hd`.
    let texture = vec3<f32>(triplanar(p * in.surface.y, n, i32(in.surface.x + 0.5)).x);
    let rock = vnoise(p.x * 5.0, p.z * 5.0, 12u) * 0.6 + vnoise(p.y * 7.0 + p.x, p.z * 13.0, 13u) * 0.4;
    // Strata: paler and darker bands, one flow of lava on the last.
    let band = 0.78 + 0.22 * sin(p.y * 21.0 + rock * 5.0);
    let albedo = in.albedo * texture * band;
    let glow = (1.0 - smooth01(0.0, 1.2, above)) * (0.85 + 0.15 * sin(t * 1.9 + p.x));
    var colour = albedo * (0.55 + 0.7 * rock) * (light_at(p, n) + vec3<f32>(0.10, 0.06, 0.05));
    colour += vec3<f32>(0.45, 0.10, 0.02) * glow * (0.5 + 0.5 * smooth01(0.55, 0.80, rock));
    // Wet rock, glistening where the searchlight strikes it.
    let h = normalize(to_camera - g.lamp.xyz);
    colour += vec3<f32>(0.85, 0.82, 0.75) * pow(max(dot(n, h), 0.0), 20.0) * g.lamp.w * 0.3 * rock;
    // Veins of molten rock run along the walls, so the arch reads in
    // perspective all the way up the tunnel.
    let course = vnoise(p.x * 1.1, p.y * 3.2 + p.z * 1.7, 17u);
    let vein = 1.0 - smooth01(0.010, 0.035, abs(course - 0.5));
    let pulse = 0.75 + 0.25 * sin(t * 1.3 + p.x * 2.0);
    colour += vec3<f32>(0.75, 0.24, 0.04) * vein * pulse * (0.35 + 0.65 * glow);
    // The passages' light, wavering a little.
    colour += in.local_normal * (0.9 + 0.1 * sin(t * 2.3 + p.x * 3.0 + p.y * 2.0));
    return colour;
}

@if(!full_hd)
@fragment
fn fs_model(in: Lit, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    if (MATERIAL == VAULT) {
        return vec4<f32>(mix(vault(in), in.fog.rgb, in.fog.a) * SEA_SCALE, in.fade);
    }
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
    // Only the foot: the rest of the model, most of its pixels, skips finding
    // the biome, which costs five sines a pixel.
    if (GROUNDED) {
        let buried = 1.0 - smooth01(in.floor_y + 0.005, in.floor_y + 0.05, p.y);
        if (buried > 0.0) {
            albedo = mix(albedo, in.floor, buried * 0.85);
        }
    }
    var colour = albedo * light_at(p, n);
    colour += in.albedo * in.glow;
    let t = g.camera.w;
    // Fading things out as their biome passes: the pipeline blends the
    // coverage over the water when there is one sample, or alpha-to-coverage
    // carries it when there are more.
    var lamps = vec3<f32>(0.0);
    if (MATERIAL == CITY_LIGHTS) {
        let city = city_light(in, n, to_camera, length(p - g.camera.xyz), t);
        colour = colour * city.keep + city.sheen;
        lamps = city.lamps;
    }
    var coverage = in.fade;
    // Per-vertex fog would be the far corners' fog on these four-vertex
    // planes; the ice overrides this with its own pixel distance.
    var haze = in.fog.a;
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
            if (fenced()) {
                return vec4<f32>(0.0);
            }
            let sparkle = pow(fract(sin(dot(floor(p * 140.0), vec3<f32>(12.9, 78.2, 37.7)) + floor(t * 3.0)) * 43758.5), 20.0);
            colour += vec3<f32>(1.0, 0.85, 0.45) * sparkle * 2.0;
        }
        // The pack ice from below: floes glowing with daylight, open leads.
        // Its three noise fields and its arctic coverage come baked into one
        // texture instead of three `vnoise` stacks per pixel.
        case 6u: {
            let baked = textureSampleLevel(
                ice_bake,
                surface_sampler,
                (p.xz - g.ice.xy) * g.ice.z,
                0.0,
            );
            // Soften the sheet at its own edges, so the 40-unit plane and
            // the cut that follows the arctic's border do not end in a hard
            // rectangle across the window.
            let edge = 1.0 - smooth01(0.80, 1.0, max(abs(in.local.x), abs(in.local.z)));
            coverage = baked.a * edge;
            haze = (1.0 - exp(-length(p - g.camera.xyz) * g.water.w)) * 0.55;
            let floes = baked.r;
            let lead = baked.g;
            let bumps = baked.b;
            colour = vec3<f32>(0.44, 0.70, 0.78) * (0.70 + 0.30 * bumps + 0.40 * floes);
            colour = mix(colour * texture, vec3<f32>(0.82, 0.98, 1.00), lead)
                * (0.95 + 0.75 * g.sun.w);
        }
        default: {}
    }
    if (FADING) {
        if (coverage <= 0.01) {
            discard;
        }
    }
    var shaded = mix(colour, in.fog.rgb, haze);
    // Lights show further through the water than what they light: the
    // windows of a tower the haze has all but hidden still glow.
    if (MATERIAL == CITY_LIGHTS) {
        shaded += lamps * (1.0 - 0.8 * haze);
    }
    if (switched(32u)) {
        let t = in.fog.a;
        let luma = dot(shaded, vec3<f32>(0.299, 0.587, 0.114));
        shaded = mix(shaded, vec3<f32>(luma), 0.15 * t)
            + vec3<f32>(0.010, 0.030, 0.040) * (t * 0.3);
    }
    if (FADE_OPAQUE) {
        shaded = mix(shaded, in.fog.rgb, 1.0 - coverage);
    }
    // The sea texture is opaquely composited by Slint; the fade lives in the
    // colour, never in the alpha.
    return vec4<f32>(shaded * SEA_SCALE, select(coverage, 1.0, FADE_OPAQUE));
}

// The jellies' tissues, for both profiles' glass pass. `in.floor_y` is how
// hard the bell is contracted at this vertex (`vs_model`), late down the
// arms and threads; `in.floor` is the animal's own tint, unlit, so its glow
// keeps its colour in water whose light is all one blue; `in.surface`
// carries a bell's pattern (`world::Medusa`): how many radial canals or
// stripes, how strongly its gonads show and how bright the lights round its
// margin are.
fn jelly_glass(in: Lit, n: vec3<f32>, v: vec3<f32>, rim: f32) -> vec3<f32> {
    let beat = in.floor_y;
    let own = in.floor * in.glow * 0.5;
    let clock = g.camera.w;
    let q = in.local;
    let ang = atan2(q.z, q.x);
    if (in.material == 7u) {
        // A medusa's bell: clear jelly whose edges glow (Fresnel) and whose
        // margin is a bright ring, veined with radial canals that fade
        // toward the margin; the gonads show through the crown, and the
        // whole bell brightens as it contracts.
        let h = clamp(q.y / 0.80, 0.0, 1.0);
        let r = length(q.xz);
        let canal = pow(max(cos(ang * in.surface.x), 0.0), 10.0) * smooth01(0.06, 0.40, h);
        let margin = 1.0 - smooth01(0.0, 0.09, h);
        // Four horseshoe gonads round the stomach, folded into one quadrant.
        let fold = abs(length(abs(q.xz) - vec2<f32>(0.22)) - 0.11);
        let gonad = (1.0 - smooth01(0.02, 0.055, fold)) * smooth01(0.35, 0.65, h) * in.surface.y;
        let stomach = (1.0 - smooth01(0.06, 0.22, r)) * smooth01(0.45, 0.80, h);
        let breath = 0.80 + 0.45 * beat;
        let edge = rim * rim;
        // Bioluminescent spots round the margin, and two waves of light
        // running round it, faster while the bell beats.
        let spots = pow(max(cos(ang * 24.0), 0.0), 4.0) * margin;
        let seed = in.world.x * 3.1 + in.world.z * 1.7;
        let run = pow(0.5 + 0.5 * cos(ang * 2.0 - clock * 1.6 - seed), 6.0);
        let lights = spots * (0.25 + 0.75 * run) * (0.6 + 0.6 * beat) * in.surface.z;
        return in.albedo * (0.06 + 0.62 * edge + 0.10 * canal + 0.45 * margin) * breath
            + own * (0.12 + 0.55 * edge + 0.80 * margin) * breath
            + vec3<f32>(0.55, 0.85, 1.0) * canal * 0.05 * breath
            + mix(vec3<f32>(0.80, 0.42, 0.95), in.floor, 0.3) * gonad * (0.10 + 0.12 * beat) * (0.5 + in.glow)
            + vec3<f32>(1.0, 0.86, 0.70) * stomach * (0.04 + 0.20 * in.glow)
            + vec3<f32>(0.35, 1.0, 0.78) * lights * 0.55;
    }
    if (in.material == 10u) {
        // A comb jelly: almost invisible jelly, its eight comb rows lit by
        // the beating cilia, whose rainbow runs from pole to pole.
        let y = clamp(q.y, 0.0, 1.0);
        let row = pow(max(cos(ang * 8.0), 0.0), 5.0) * sin(y * PI);
        let hue = fract(y * 1.6 - clock * 0.7 + ang * 0.16);
        let rainbow = 0.55 + 0.45 * cos(TAU * (hue + vec3<f32>(0.0, 0.33, 0.67)));
        let plates = 0.45 + 0.55 * pow(0.5 + 0.5 * sin(y * 46.0 - clock * 8.0), 2.0);
        let core = pow(abs(dot(n, v)), 3.0);
        return in.albedo * (0.03 + 0.40 * rim * rim) * (0.85 + 0.3 * beat)
            + own * (0.06 + 0.20 * core)
            + rainbow * rainbow * row * plates * (0.75 + 0.5 * rim);
    }
    // The oral arms and tentacles: the same tissue, finer and dimmer, veined
    // with light that runs down them, and brightening as the stroke reaches
    // each part of them.
    let along = clamp(-q.y, 0.0, 1.0);
    let vein = 0.72 + 0.28 * sin(q.y * 16.0 + q.x * 9.0 + clock * 1.3);
    return in.albedo * (0.10 + 0.45 * rim * rim) * vein * (0.80 + 0.50 * beat)
        + own * (0.18 + 0.35 * along) * (0.85 + 0.40 * beat);
}

// Translucent models — jellyfish bells and tentacles — add light, brightest at
// their rims, and fade with distance.
@if(!full_hd)
@fragment
fn fs_glass(in: Lit) -> @location(0) vec4<f32> {
    let p = in.world;
    let n = normalize(in.normal);
    let v = normalize(g.camera.xyz - p);
    let rim = 1.0 - abs(dot(n, v));
    let lit = light_at(p, v) * 0.25 + vec3<f32>(0.3);
    var glow = in.albedo * (0.15 + 0.85 * rim * rim) * lit + in.albedo * in.glow;
    if (in.material == 3u) {
        // A crystal: its facets are read by a warm key from the searchlight,
        // or from the passage's mouth where the beam does not reach, and a
        // violet heart where the point faces the window.
        let to_lamp = normalize(lamp_origin() - p);
        let cone = smooth01(0.78, 0.96, dot(-to_lamp, g.lamp.xyz));
        let key = normalize(mix(vec3<f32>(0.10, 0.55, -0.95), to_lamp, cone));
        let half = normalize(v + key);
        let spec = pow(max(dot(n, half), 0.0), 26.0);
        let core = pow(max(dot(n, v), 0.0), 3.0);
        let tip = smooth01(0.22, 0.92, in.local.y);
        glow += vec3<f32>(0.10, 0.05, 0.26) * in.glow * (1.0 - 0.6 * tip)
            + vec3<f32>(1.0, 0.72, 0.45) * tip * (0.08 + 0.7 * in.glow)
            + vec3<f32>(1.0, 0.90, 0.76) * spec * (0.10 + 0.4 * cone)
            + vec3<f32>(0.45, 0.30, 0.85) * core * in.glow * 0.10;
    }
    if (in.material == 7u || in.material == 9u || in.material == 10u) {
        glow = jelly_glass(in, n, v, rim);
    }
    return vec4<f32>(glow * in.fade * (1.0 - fog_of(length(p - g.camera.xyz))) * SEA_SCALE, 0.0);
}


// The tunnel's slopes as stored, read back as heights a world unit
// (`textures::SLOPE`, either way of the middle of the channel), and drawn
// a little shallower than the tile's own: enough for a ledge's lip or a joint
// to catch the light across the window, not so much that the rock boils.
// The tunnel's layer of `surfaces` (`textures::Surface::Vault`), which the
// cave's cliffs borrow.
const VAULT_LAYER: i32 = 16;
const VAULT_RELIEF: f32 = 2.0 * 10.0 * 0.22;

// The full-HD tunnel: lava rock lit as rock, from its one texture tap. The
// layer's slopes tilt the arch's smooth normal into ledges, joints and
// vesicles, and the light that falls on them is what reads as relief: the
// searchlight from its mount above the window, strongest on what faces it
// and falling off with distance, and the lava's red from the floor, caught
// by whatever faces down. Rock right at the glass lies in the boat's own
// shadow, so a near wall is darker than the passage beyond it and never one
// flat sheet of colour. Cracks in the hotter patches (`vs_model`) glow, and
// the wet rock glints where the beam strikes it. The pools' light and the
// passages' come summed per vertex; nothing here loops over lights or runs a
// noise.
@if(full_hd)
fn vault_hd(in: Lit, rock: vec4<f32>, distance: f32) -> vec3<f32> {
    let p = in.world;
    let t = g.camera.w;
    let above = in.local.x;
    let heat = in.local.y;
    let view = (g.camera.xyz - p) / distance;
    var n = normalize(in.normal);
    if (dot(n, view) < 0.0) {
        n = -n;
    }
    // The tile runs across the section by x + z and up it by height: a slope
    // in each is a rise along those directions, and only its part along the
    // surface tilts the normal.
    let slope = (rock.gb - 0.5) * VAULT_RELIEF;
    let rise = vec3<f32>(slope.x, slope.y, slope.x);
    let bumped = normalize(n - (rise - n * dot(n, rise)));
    let tone = rock.r * TEXTURE_RANGE;
    // The searchlight, a little spill outside its cone.
    let to_lamp = g.lamp_origin.xyz - p;
    let reach = length(to_lamp);
    let l = to_lamp / reach;
    let cone = 0.15 + 0.85 * smooth01(0.72, 0.95, -dot(l, g.lamp.xyz));
    let lamp = g.lamp.w * cone * max(dot(bumped, l), 0.0) / (1.0 + reach * reach * 0.12);
    // The lava's glow from the floor, breathing, on what faces down to it.
    let low = 1.0 - smooth01(0.0, 1.3, above);
    let bounce = low * (0.6 - 0.4 * bumped.y) * (0.85 + 0.15 * sin(t * 1.9 + p.x));
    let near = 0.45 + 0.55 * smooth01(0.25, 1.6, distance);
    var colour = in.albedo * tone * (
        (vec3<f32>(0.30, 0.24, 0.22) + vec3<f32>(1.1, 0.33, 0.09) * bounce) * near
            + vec3<f32>(1.0, 0.92, 0.78) * (lamp * (0.5 + 1.0 * near))
    );
    colour += in.local_normal * tone;
    // Molten cracks, and the rock about them warmed a dull red.
    let pulse = 0.7 + 0.3 * sin(t * 1.3 + p.x * 2.0 + p.y * 3.0);
    colour += vec3<f32>(1.0, 0.32, 0.05) * (rock.a * heat * pulse * 1.6)
        + vec3<f32>(0.30, 0.06, 0.01) * (heat * 0.25);
    // Wet rock, glistening where the beam strikes it.
    let shine = pow(max(dot(bumped, normalize(view + l)), 0.0), 16.0);
    colour += vec3<f32>(0.85, 0.82, 0.75) * (shine * lamp * 0.15);
    return colour;
}

// The full-HD profile's models: one texture projection, a sun/ambient light
// worked out per pixel, no point-light loop, caustics, triplanar blend or
// per-material switch. Enough for the silhouette and colour; the 448 build
// keeps the full shading.
@if(full_hd)
@fragment
fn fs_model(in: Lit, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let p = in.world;
    let n = normalize(in.normal);
    let carried = in.surface.z > 0.5;
    var uv = select(p, in.local, carried) * in.surface.y;
    if (MATERIAL == VAULT) {
        // The tunnel is a tube, not a floor: projecting straight down its y
        // axis smears every wall into vertical stripes, and a diagonal
        // projection still turns its flow bands on edge round the section.
        // Take the horizontal round the section for u and world height for
        // v, the same pair the cave's terrain wall uses, so the flow bands
        // lie level along the tunnel and through its passages.
        uv = vec3<f32>(p.x + p.z, 0.0, p.y) * (in.surface.y * 1.35);
    }
    let layer = i32(in.surface.x + 0.5);
    let distance = length(p - g.camera.xyz);
    // The tunnel is seen close up: its rock is one sample of a 0.5-unit tile,
    // and the generic distance bias would blur the grain and the cracks away.
    // Only the vault pipeline takes the sharper level; every model keeps the
    // cheap one.
    let lod = clamp(log2(distance * select(3.0, 1.0, MATERIAL == VAULT)), 0.0, 8.0);
    // The tunnel's layer is not a colour (`textures::paint_vault`): its
    // channels are read raw, and only the others' are scaled to a colour.
    var sample = vec4<f32>(0.6 / TEXTURE_RANGE, 0.5, 0.5, 0.0);
    if (!PLAIN_TEXTURES) {
        sample = textureSampleLevel(
            surfaces,
            surface_sampler,
            uv.xz,
            layer,
            lod,
        );
    }
    let texture = sample.rgb * TEXTURE_RANGE;
    var albedo = in.albedo * texture;
    // Bedded in the sand: the foot of anything on the seabed takes its colour.
    if (GROUNDED) {
        let buried = 1.0 - smooth01(in.floor_y + 0.005, in.floor_y + 0.05, p.y);
        if (buried > 0.0) {
            albedo = mix(albedo, in.floor, buried * 0.85);
        }
    }
    var colour = albedo + in.albedo * in.glow;
    // Coral and the reef's other living tissue let light through where it
    // is thin: a soft glow round each branch's silhouette, in its own
    // colour, so the reef reads as flesh over stone rather than painted
    // plaster. It costs a dot product, and only the reef's surfaces pay it.
    if (layer == CORAL_LAYER || layer == BRAIN_LAYER) {
        let rim = 1.0 - abs(dot(n, normalize(g.camera.xyz - p)));
        colour += in.albedo * (rim * rim * 0.5);
    }
    let t = g.camera.w;
    if (MATERIAL == VAULT) {
        colour = vault_hd(in, sample, distance);
    }
    var lamps = vec3<f32>(0.0);
    if (MATERIAL == CITY_LIGHTS) {
        let city = city_light(in, n, normalize(g.camera.xyz - p), distance, t);
        colour = colour * city.keep + city.sheen;
        lamps = city.lamps;
    }
    var coverage = in.fade;
    // Per-vertex fog would be the far corners' fog on these four-vertex
    // planes; the ice overrides this with its own pixel distance.
    var haze = in.fog.a;
    switch (select(in.material, u32(MATERIAL), MATERIAL >= 0)) {
        // Pillow lava, molten through its cracks.
        case 1u: {
            let crack = 1.0 - smooth01(0.0, 0.05, abs(vnoise(in.local.x * 6.0 + in.local.y * 3.0, in.local.z * 6.0, 11u) - 0.5));
            colour += vec3<f32>(1.0, 0.38, 0.08) * crack * smooth01(0.3, -0.5, in.local.y) * (0.8 + 0.2 * sin(t * 2.0));
        }
        // Copper and brass: a hot specular highlight from the searchlight.
        case 2u: {
            let h = normalize(normalize(g.camera.xyz - p) - g.lamp.xyz);
            colour += vec3<f32>(1.0, 0.75, 0.45) * pow(max(dot(n, h), 0.0), 24.0) * (0.3 + g.lamp.w);
        }
        // Gold, glittering.
        case 8u: {
            if (fenced()) {
                return vec4<f32>(0.0);
            }
            let sparkle = pow(fract(sin(dot(floor(p * 140.0), vec3<f32>(12.9, 78.2, 37.7)) + floor(t * 3.0)) * 43758.5), 20.0);
            colour += vec3<f32>(1.0, 0.85, 0.45) * sparkle * 2.0;
        }
        // The pack ice from below: floes glowing with daylight, open leads.
        // Its three noise fields and its arctic coverage come baked into one
        // texture instead of three `vnoise` stacks per pixel.
        case 6u: {
            let baked = textureSampleLevel(
                ice_bake,
                surface_sampler,
                (p.xz - g.ice.xy) * g.ice.z,
                0.0,
            );
            // Soften the sheet at its own edges, so the 40-unit plane and
            // the cut that follows the arctic's border do not end in a hard
            // rectangle across the window.
            let edge = 1.0 - smooth01(0.80, 1.0, max(abs(in.local.x), abs(in.local.z)));
            coverage = baked.a * edge;
            haze = (1.0 - exp(-length(p - g.camera.xyz) * g.water.w)) * 0.55;
            let floes = baked.r;
            let lead = baked.g;
            let bumps = baked.b;
            colour = vec3<f32>(0.44, 0.70, 0.78) * (0.70 + 0.30 * bumps + 0.40 * floes);
            colour = mix(colour * texture, vec3<f32>(0.82, 0.98, 1.00), lead)
                * (0.95 + 0.75 * g.sun.w);
        }
        default: {}
    }
    if (FADING) {
        if (coverage <= 0.01) {
            discard;
        }
    }
    var shaded = mix(colour, in.fog.rgb, haze);
    // Lights show further through the water than what they light: the
    // windows of a tower the haze has all but hidden still glow.
    if (MATERIAL == CITY_LIGHTS) {
        shaded += lamps * (1.0 - 0.8 * haze);
    }
    if (switched(32u)) {
        let t = in.fog.a;
        let luma = dot(shaded, vec3<f32>(0.299, 0.587, 0.114));
        shaded = mix(shaded, vec3<f32>(luma), 0.15 * t)
            + vec3<f32>(0.010, 0.030, 0.040) * (t * 0.3);
    }
    if (FADE_OPAQUE) {
        shaded = mix(shaded, in.fog.rgb, 1.0 - coverage);
    }
    // The sea texture is opaquely composited by Slint; the fade lives in the
    // colour, never in the alpha.
    return vec4<f32>(shaded * SEA_SCALE, select(coverage, 1.0, FADE_OPAQUE));
}

@if(full_hd)
@fragment
fn fs_glass(in: Lit) -> @location(0) vec4<f32> {
    let p = in.world;
    let n = normalize(in.normal);
    let v = normalize(g.camera.xyz - p);
    let rim = 1.0 - abs(dot(n, v));
    var colour = in.albedo * (0.2 + 0.8 * rim * rim) * 1.25
        + in.albedo * in.glow;
    if (in.material == 3u) {
        // A crystal: the facets' own normals and baked violet-to-warm
        // colours, a glance of the searchlight where its cone reaches the
        // seam, and a violet heart where the point faces the window. No
        // texture tap and one `pow`: the glass pass stays additive.
        let to_lamp = normalize(lamp_origin() - p);
        let cone = smooth01(0.78, 0.96, dot(-to_lamp, g.lamp.xyz));
        let key = normalize(mix(vec3<f32>(0.10, 0.55, -0.95), to_lamp, cone));
        let half = normalize(v + key);
        let spec = pow(max(dot(n, half), 0.0), 26.0);
        let core = pow(max(dot(n, v), 0.0), 3.0);
        // How far up its point this fragment is: the violet foot gives way
        // to the warm tip, whether or not a light reaches the seam.
        let tip = smooth01(0.22, 0.92, in.local.y);
        colour = in.albedo * (0.16 + 0.66 * rim * rim)
            + in.albedo * in.glow * 0.9
            + vec3<f32>(0.10, 0.05, 0.26) * in.glow * (1.0 - 0.6 * tip)
            + vec3<f32>(1.0, 0.72, 0.45) * tip * (0.10 + 0.8 * in.glow)
            + vec3<f32>(1.0, 0.90, 0.76) * spec * (0.14 + 0.45 * cone) * (0.18 + 0.8 * in.glow)
            + vec3<f32>(0.45, 0.30, 0.85) * core * in.glow * 0.15;
    }
    if (in.material == 7u || in.material == 9u || in.material == 10u) {
        // A jelly glows the fog's colour far off, which is what makes a
        // shoal of them shine down the bloom; near, its own colours show.
        // It fades in with its placement, as the other profile's glass does.
        let own = jelly_glass(in, n, v, rim);
        return vec4<f32>(mix(own, in.fog.rgb, in.fog.a) * in.fade * SEA_SCALE, 0.0);
    }
    return vec4<f32>(
        mix(colour, in.fog.rgb, in.fog.a) * SEA_SCALE,
        0.0,
    );
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
    let kind = u32(tint.w + 0.5);
    // Bubbles age from the camera clock: the anchor is the shed point, and
    // the shader reproduces the trail's offset/fade so the placer can cache.
    var p = place.xyz;
    var bubble_fade = 1.0;
    var ember = vec3<f32>(1.0);
    if (stretch.z > 0.0) {
        if (kind == 2u) {
            let t = g.camera.w;
            if (stretch.w > 0.5) {
                // A crater's two-sine flicker.
                bubble_fade = 0.85 + 0.15 * sin(t * 2.3 + stretch.y) * sin(t * 1.3);
            } else {
                // A sea pen's blink: a half-wave gated by the biome's share.
                bubble_fade = max(sin(t * stretch.z + stretch.y), 0.0);
            }
        } else if (kind == 0u && stretch.z > 0.0) {
            // MOTE: sediment falling from a lip, fading as it goes.
            let age = fract(g.camera.w * 0.07 + stretch.y / 18.0 + stretch.z * 7.0);
            let spread = 0.03 + 0.25 * age;
            p.x += spread * sin(stretch.y * 2.4 + g.camera.w * 0.3);
            p.y -= age * stretch.w;
            p.z += 0.35 * age + spread * 0.5 * cos(stretch.y * 1.7);
            bubble_fade = smooth01(0.0, 0.1, age) * (1.0 - age);
        } else if (kind == 3u) {
            // EMBER: rise from the floor, fade out as it rises, wavering on
            // the shimmering water as it goes.
            let rise = fract(g.camera.w * stretch.z + stretch.y);
            let waver = g.camera.w * 2.3 + stretch.y * TAU;
            p.y += rise * stretch.w;
            p.x += 0.05 * rise * sin(waver);
            p.z += 0.04 * rise * cos(waver * 0.7);
            bubble_fade = (1.0 - rise) * smooth01(0.0, 0.1, rise);
            // A warm ember leaves the pool white-hot and cools to red as it
            // climbs, flickering; the arctic's and the abyss's cold flecks
            // keep their colour.
            let warm = smooth01(0.0, 0.3, tint.r - tint.b);
            ember = mix(vec3<f32>(1.0), vec3<f32>(1.0, 0.75 - 0.45 * rise, 1.6 - 1.5 * rise), warm)
                * (1.0 + warm * (0.6 * (1.0 - rise) + 0.25 * sin(waver * 3.1)));
        } else {
            let cycle = g.camera.w / stretch.z + stretch.y;
            let age = (cycle - floor(cycle)) * stretch.z;
            bubble_fade = smooth01(0.0, 0.08 * stretch.z, age)
                * smooth01(stretch.z, 0.86 * stretch.z, age);
            let was0 = p.x;
            let was = was0 - 0.24 * age;
            p.x = was;
            p.y += 0.015 * age + 0.06 * (sin(was * 3.5 + stretch.w) - sin(was0 * 3.5 + stretch.w));
        }
    }
    var clip = project(p);
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
    if (kind == 0u) {
        let to = place.xyz - lamp_origin();
        let cone = smooth01(0.80, 0.95, dot(normalize(to), g.lamp.xyz));
        lit = 0.25 + 2.5 * cone * g.lamp.w / (1.0 + dot(to, to) * 0.08) + g.sun.w * 0.6;
    }
    out.colour = tint.rgb * ember * lit * (1.0 - fog_of(d)) * bubble_fade;
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

// ---------------------------------------------------------------------------
// Shafts of sunlight
// ---------------------------------------------------------------------------

// A pipeline of their own, so the snow and bubbles, far more of them, pay
// nothing for them.

struct Shaft {
    @builtin(position) position: vec4<f32>,
    @location(0) corner: vec2<f32>,
    @location(1) colour: vec3<f32>,
};

// A ribbon from `place` down the run in `run`, `place.w` wide either side,
// turned about its length to face the window. Its corners are placed in the
// world, so it clips like any other triangle.
@vertex
fn vs_shaft(
    @builtin(vertex_index) index: u32,
    @location(0) place: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) run: vec4<f32>,
) -> Shaft {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
    let top = place.xyz;
    let side = normalize(cross(run.xyz, g.camera.xyz - top)) * place.w;
    let at = top + run.xyz * (0.5 - 0.5 * corner.y) + side * corner.x;
    var out: Shaft;
    out.position = project(at);
    out.corner = corner;
    // Melting away as the window passes through it, rather than filling the
    // glass, and hazed by the water like everything else.
    let d = length(at - g.camera.xyz);
    // `tint.w` marks a shaft lit by its own light, as from the lava cave's
    // chimneys, rather than by the sun's.
    let strength = select(g.sun.w, 1.0, tint.w > 0.5);
    out.colour = tint.rgb * strength * smooth01(0.35, 1.1, d) * (1.0 - fog_of(d)) * SEA_SCALE;
    return out;
}

// Soft across, brightest just under the surface, fading on its way down.
@fragment
fn fs_shaft(in: Shaft) -> @location(0) vec4<f32> {
    let across = 1.0 - in.corner.x * in.corner.x;
    let y = in.corner.y;
    let along = (1.0 - smooth01(0.75, 1.0, y)) * smooth01(-1.0, 0.1, y);
    return vec4<f32>(in.colour * (across * across * along), 0.0);
}

// ---------------------------------------------------------------------------
// Searchlight beams
// ---------------------------------------------------------------------------

// Another submarine's searchlight in dark water: a ribbon from the lamp at
// `place` along `run`, `place.w` wide either side at the lamp and `tint.w`
// at the far end, turned about its length to face the window. A pipeline of
// its own, like the shafts, so nothing else pays for its falloff, and lit
// whatever the daylight.
@vertex
fn vs_beam(
    @builtin(vertex_index) index: u32,
    @location(0) place: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) run: vec4<f32>,
) -> Shaft {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
    let along = 0.5 - 0.5 * corner.y;
    let axis = place.xyz + run.xyz * along;
    let side = normalize(cross(run.xyz, g.camera.xyz - axis) + vec3<f32>(1e-5, 0.0, 0.0));
    let at = axis + side * mix(place.w, tint.w, along) * corner.x;
    var out: Shaft;
    out.position = project(at);
    out.corner = corner;
    let d = length(at - g.camera.xyz);
    out.colour = tint.rgb * smooth01(0.2, 0.8, d) * (1.0 - fog_of(d)) * SEA_SCALE;
    return out;
}

// A cone of lit water seen side on: as deep as the cone is through its
// middle and thinning to nothing at its edges, brighter still along its axis.
// Brightest at the lamp, spreading thin as the cone widens, and fading out at
// its end, which is where it meets the seabed or runs out of reach.
@fragment
fn fs_beam(in: Shaft) -> @location(0) vec4<f32> {
    let x2 = in.corner.x * in.corner.x;
    let across = sqrt(max(1.0 - x2, 0.0)) * (0.35 + 0.65 * exp(-4.0 * x2));
    let s = 0.5 - 0.5 * in.corner.y;
    let along = smooth01(0.0, 0.02, s) * (1.0 - smooth01(0.7, 1.0, s)) / (1.0 + 6.0 * s);
    return vec4<f32>(in.colour * (across * along), 0.0);
}

// ---------------------------------------------------------------------------
// Far coral
// ---------------------------------------------------------------------------

// Coral too far off for its branches to be told apart, drawn as one cut-out
// card standing on the seabed and turned about the vertical to face the
// window: `place` is its foot and its height, `tint` its colour and a seed
// for its shape, `stretch.x` its half width, `stretch.y` its form (0
// staghorn, 1 a table, 2 fingers, 3 a brain coral's dome, 4 a bushy head)
// and `stretch.z`, if not zero, how near the window the card begins to
// dissolve into the model that takes over from it.
//
// Form 5 is not coral but one of the small reef fish hovering over it
// (`world::reef_fish`): its card is centred on `place` rather than standing
// on it, and swims round that point by the clock, `stretch.w` out, darting
// and bobbing, its nose turned the way it is going. It is opaque and writes
// depth, so it hides what is behind it like the models it stands in for.
struct Impostor {
    @builtin(position) position: vec4<f32>,
    // Across from -1 to 1 in units of its height, and up from 0 to 1.
    @location(0) at: vec2<f32>,
    @location(1) colour: vec3<f32>,
    @location(2) fog: vec4<f32>,
    // Its seed, form, half width over height, and how much of it is there.
    @location(3) @interpolate(flat) shape: vec4<f32>,
};

@vertex
fn vs_impostor(
    @builtin(vertex_index) index: u32,
    @location(0) place: vec4<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) stretch: vec4<f32>,
) -> Impostor {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let corner = corners[index];
    var foot = place.xyz;
    let fish = stretch.y > 4.5;
    var heading = vec3<f32>(0.0);
    var lift = -0.015;
    if (fish) {
        // Round the swarm's centre on a loop of its own, quick or slow, one
        // way or the other, with a dart along it now and then and a bob.
        let seed = tint.w;
        let rate = (0.35 + 0.45 * seeded(seed, 1.0)) * select(-1.0, 1.0, seeded(seed, 2.0) > 0.5);
        let dart = 0.35 * sin(g.camera.w * 1.3 + seed * 5.0);
        let angle = g.camera.w * rate + dart + seed * TAU;
        let r = stretch.w;
        foot += vec3<f32>(
            cos(angle) * r,
            0.3 * r * sin(angle * 1.7 + seed) + 0.01 * sin(g.camera.w * 2.1 + seed * 3.0),
            sin(angle) * r * 0.8,
        );
        heading = vec3<f32>(-sin(angle), 0.0, cos(angle) * 0.8) * rate;
        lift = -0.5 * place.w;
    }
    let to = g.camera.xyz - foot;
    let side = normalize(vec3<f32>(-to.z, 0.0, to.x) + vec3<f32>(1e-5, 0.0, 0.0));
    // Sunk a little, so the foot meets the sand however the ground slopes.
    let p = foot + side * corner.x * stretch.x + vec3<f32>(0.0, corner.y * place.w + lift, 0.0);
    var out: Impostor;
    out.position = project(p);
    // A fish's card is drawn nose to +x: mirrored when it swims the other
    // way across the window.
    let nose = select(1.0, sign(dot(heading, side) + 1e-6), fish);
    out.at = vec2<f32>(corner.x * nose * stretch.x / place.w, corner.y);
    let middle = foot + vec3<f32>(0.0, place.w * 0.5 + lift + 0.015, 0.0);
    let facing = normalize(vec3<f32>(to.x, length(to.xz) * 0.8, to.z));
    // Lit as the models are: their sun and ambient light, and the glow the
    // reef's corals are given.
    out.colour = tint.rgb * (light_at(middle, facing) * 0.85 + vec3<f32>(0.24));
    out.fog = fog_at(middle);
    // Dissolving over the unit after `stretch.z`, where the model stands too.
    let there = select(1.0, smooth01(stretch.z, stretch.z + 1.0, length(to.xz)), stretch.z > 0.0);
    out.shape = vec4<f32>(tint.w, stretch.y, stretch.x / place.w, there);
    return out;
}

// How far `p` is outside a limb from `a` to `b`, `ra` thick at `a` and `rb`
// at `b`, its ends rounded: negative inside.
fn limb_out(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>, ra: f32, rb: f32) -> f32 {
    let ab = b - a;
    let t = clamp(dot(p - a, ab) / dot(ab, ab), 0.0, 1.0);
    return length(p - a - ab * t) - mix(ra, rb, t);
}

fn seeded(seed: f32, k: f32) -> f32 {
    return fract(sin(seed * 12.9898 + k * 78.233) * 43758.547);
}

@fragment
fn fs_impostor(in: Impostor) -> @location(0) vec4<f32> {
    let p = in.at;
    let seed = in.shape.x;
    let form = in.shape.y;
    let wide = in.shape.z;
    var d = 1e3;
    // How pale it is with new growth, how far round toward the window its
    // surface faces, and how deep in a groove, for a little relief on a flat
    // card.
    var tip = 0.0;
    var round = 1.0;
    var groove = 0.0;
    // Which form, fixed by the pipeline's `FORM` where it has one, so the
    // others are not compiled in.
    let any = FORM < 0;
    let staghorn = select(FORM == 0, (form < 0.5), any);
    let table = select(FORM == 1, (form >= 0.5) && (form < 1.5), any);
    let fingers = select(FORM == 2, (form >= 1.5) && (form < 2.5), any);
    let fish = select(FORM == 5, (form > 4.5), any);
    if (staghorn) {
        // Staghorn: trunks leaning out of one foot, each forking into two
        // upturned antlers with a pale tip, as the model grows them; baked
        // (`textures::paint_staghorn`), one of four colonies by the seed,
        // the right way round or mirrored. Traced here, the fifteen limbs
        // kept the card at half the Pi's threads.
        let colony = floor(fract(seed * 0.1) * 4.0);
        let mirror = select(1.0, -1.0, fract(seed * 3.7) < 0.5);
        let half = 0.5 / (f32(SIZE_STAGHORN) / STAGHORN_CELLS);
        let across = clamp(p.x / wide * mirror * 0.5 + 0.5, half, 1.0 - half);
        let up = clamp(p.y, half, 1.0 - half);
        let square = vec2<f32>(colony % STAGHORN_CELLS, floor(colony / STAGHORN_CELLS));
        let baked = textureSampleLevel(
            surfaces,
            surface_sampler,
            (square + vec2<f32>(across, up)) / STAGHORN_CELLS,
            STAGHORN_LAYER,
            0.0,
        );
        d = (baked.r - 0.5) * 0.5;
        tip = baked.g;
        round = smooth01(0.0, 0.05, -d);
    } else if (table) {
        // A table: a broad cupped plate on a short stem, its rim lifted and
        // lobed, its underside in shade.
        let x = p.x / wide;
        let lift = 0.14 * x * x;
        let rim = 0.04 * sin(x * 7.0 + seed * 7.0) + 0.02 * sin(x * 17.0 + seed);
        let thick = 0.16 * (1.0 - 0.55 * x * x);
        let plate = length(vec2<f32>(x * (1.0 + rim), (p.y - 0.66 - lift) / thick)) - 1.0;
        let stem = limb_out(p, vec2<f32>(0.0, 0.0), vec2<f32>(0.0, 0.62), 0.11, 0.07);
        d = min(plate * thick, stem);
        tip = smooth01(0.7, 0.98, abs(x)) * step(0.6, p.y);
        round = select(0.55, 1.0, p.y > 0.66 + lift - thick * 0.2);
    } else if (fingers) {
        // Fingers: blunt, upright and close together, of uneven height.
        for (var k = 0; k < 7; k = k + 1) {
            let f = f32(k);
            let h = seeded(seed, f + 3.0);
            let x = (f / 6.0 - 0.5) * 1.5 * wide;
            let top = vec2<f32>(x * 1.25, 0.55 + 0.42 * h);
            d = min(d, limb_out(p, vec2<f32>(x * 0.6, 0.0), top, 0.105, 0.09));
            tip = max(tip, 1.0 - smooth01(0.0, 0.14, length(p - top)));
        }
        round = smooth01(0.0, 0.07, -d);
    } else if (fish) {
        // A small reef fish side on: a deep oval body, nose to +x, and a
        // forked tail; darker along its back, paler at its belly, with a
        // dark eye.
        let x = p.x / wide;
        let q = vec2<f32>((x - 0.12) / 0.70, (p.y - 0.5) / 0.38);
        let body = (length(q) - 1.0) * min(0.70 * wide, 0.38);
        let along = (-x - 0.50) / 0.48;
        let fork = abs(p.y - 0.5) - (0.06 + 0.40 * along) + 0.12 * smooth01(0.6, 1.0, along);
        let tail = max(fork, max(-along, along - 1.0) * 0.48 * wide);
        d = min(body, tail);
        round = 1.15 - 0.55 * smooth01(0.2, 0.9, p.y);
        tip = 0.0;
        groove = 0.75 * (1.0 - smooth01(0.03, 0.07, length(vec2<f32>((x - 0.58) * wide, p.y - 0.58))));
    } else {
        // A dome: a brain coral's low boulder, or a bushy head of short
        // branches, its outline lumpy with them.
        let bush = form > 3.5;
        let q = vec2<f32>(p.x / wide, p.y / 0.97);
        let angle = atan2(q.y, q.x);
        let lumps = select(
            0.025 * sin(angle * 7.0 + seed),
            0.07 * sin(angle * 13.0 + seed) + 0.04 * sin(angle * 23.0 + seed * 3.0),
            bush,
        );
        let r = length(q);
        d = (r - 1.0 - lumps) * min(wide, 0.97);
        // The dome's surface facing: up and out from its centre.
        let z = sqrt(max(1.0 - r * r, 0.0));
        round = 0.35 + 0.65 * clamp(q.y * 0.6 + z * 0.6, 0.0, 1.0);
        if (bush) {
            // Paler where its branch ends crowd the rim of the head.
            tip = smooth01(0.6, 1.0, r) * 0.6;
        } else {
            // A hint of its meandering grooves: broad, as anything finer
            // would only crawl a few pixels off.
            let meander = sin(q.x * 9.0 + 2.0 * sin(q.y * 7.0 + seed) + seed);
            groove = smooth01(0.3, 1.0, meander) * 0.22;
            tip = 0.15;
        }
    }
    // Coverage across the edge, about a pixel wide, and how much of the card
    // is there, are both dithered: there is no blending in this pass, and a
    // hard cut leaves jagged, crawling edges. Interleaved gradient noise
    // spreads the dots evenly, so a half-covered pixel reads as a soft edge.
    let edge = clamp(0.5 - d / max(fwidth(d), 1e-4), 0.0, 1.0);
    let dither = fract(52.982918 * fract(dot(in.position.xy, vec2<f32>(0.06711056, 0.00583715))));
    if (edge * in.shape.w <= dither) {
        discard;
    }
    // In shade at the foot, darker round each branch's sides and in the
    // grooves, the paler new growth at the tips.
    var shade = (0.50 + 0.50 * smooth01(0.0, 0.8, p.y)) * (0.62 + 0.38 * round) * (1.0 - groove);
    if (fish) {
        shade = round * (1.0 - groove);
    }
    let colour = mix(in.colour * shade, in.colour * 1.2 + vec3<f32>(0.05), tip * 0.55);
    return vec4<f32>(mix(colour, in.fog.rgb, in.fog.a) * SEA_SCALE, 0.0);
}
