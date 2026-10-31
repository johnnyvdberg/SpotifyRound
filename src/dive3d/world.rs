//! What is in the water around the Nautilus, frame by frame.
//!
//! Everything is placed procedurally and deterministically: seabed features
//! come from cells of the world, so the same rock is always in the same place;
//! swimmers travel in processions at their own speed through the water, so the
//! Nautilus overtakes some and is passed by others. Each frame, the cells in
//! view are visited and turned into model instances — a fish is one instance,
//! a diver a shared suit and helmet with the same limbs stretched between its
//! joints — plus sprites for snow, bubbles and glows and the
//! point lights that matter most.
//!
//! The noise, biome mix and terrain height here are the same functions as in
//! `scene.wgsl`, so a rock placed on the seabed sits on the seabed the GPU draws.

use std::f32::consts::{PI, TAU};

use super::helm::Pose;
use super::mesh::{self, Model};
use super::textures::Surface;
use crate::voyage::{BIOMES, Biome};

pub const SHALLOWS: usize = 0;
pub const FOREST: usize = 1;
pub const WRECKS: usize = 2;
pub const ATLANTIS: usize = 3;
pub const CAVE: usize = 4;
pub const ABYSS: usize = 5;
pub const ARCTIC: usize = 6;
/// Last in the tables, though not on the voyage: see `voyage::ORDER`.
pub const TRENCH: usize = 7;
/// The five seas added after the original eight. They keep their ids at the
/// end of every table, so no existing biome number moved.
pub const CORAL: usize = 8;
pub const SARGASSO: usize = 9;
pub const VENTS: usize = 10;
pub const BRINE: usize = 11;
pub const BLOOM: usize = 12;
/// The drowned city, added after the five: brass-and-iron towers lit from
/// within, standing in rows either side of the avenue the Nautilus takes.
pub const CITY: usize = 13;

/// Height of the cave's vault above its floor, and the ice's thickness.
pub const CAVE_ROOF: f32 = 1.6;
pub const ICE: f32 = 0.15;

/// Half the view's width at unit distance.
pub const VIEW: f32 = 0.41;

/// Toward the sun, high and a little ahead and to the side, not normalised.
pub const SUN: [f32; 3] = [0.30, 1.0, 0.25];

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Instance {
    pub rows: [[f32; 4]; 3],
    pub colour: [f32; 4],
    pub anim: [f32; 4],
    /// x: material; y: 1 if it stands on the seabed; z: how far it has faded
    /// in, as its biome comes and goes; w: the seabed's height under it, for
    /// the models that stand on it.
    pub extra: [f32; 4],
    /// x: texture layer; y: repeats per unit; z: 1 to texture in the model's
    /// own space, so the pattern moves with a swimming body.
    pub surface: [f32; 4],
    /// rgb: the seabed's colour under it, for the foot of a grounded model;
    /// the same table as `floor_albedo` in `scene.wesl`, worked out once here
    /// rather than looked up through a dynamic table in every pixel.
    pub floor: [f32; 4],
}

/// The floor's colour in each biome, as `floor_albedo` in `scene.wesl`.
pub const FLOOR_ALBEDO: [[f32; 3]; BIOMES] = [
    [0.62, 0.56, 0.42],
    [0.40, 0.36, 0.26],
    [0.32, 0.30, 0.26],
    [0.46, 0.44, 0.39],
    [0.10, 0.09, 0.09],
    [0.17, 0.16, 0.17],
    [0.36, 0.37, 0.38],
    [0.58, 0.55, 0.47],
    // Coral: the pale sand of the gutters, ground from the reef itself; the
    // living reef over it is `reef_tone`.
    [0.80, 0.72, 0.58],
    // Sargasso: olive silt under the floating weed.
    [0.39, 0.36, 0.22],
    // Vents: fresh black basalt, darker than the cave's.
    [0.08, 0.07, 0.07],
    // Brine: salt crust, pale and almost neutral.
    [0.74, 0.73, 0.68],
    // Bloom: soft blue-grey ooze.
    [0.20, 0.20, 0.24],
    // The drowned city: grey-green silt over its paving.
    [0.25, 0.27, 0.23],
];

/// The living reef's colours, as a reef flat seen from above is a mosaic of
/// them: ochre and brown acropora, purple and pink encrusting colonies, olive
/// green and cream boulder corals, and the blue-grey of staghorn tips; the
/// browns and ochres commonest. Deeper and duller than the corals' own
/// tints, as a carpet of colonies is shaded by itself.
const REEF_FLOOR: [[f32; 3]; 9] = [
    [0.52, 0.40, 0.22],
    [0.38, 0.28, 0.18],
    [0.42, 0.29, 0.40],
    [0.56, 0.46, 0.30],
    [0.38, 0.42, 0.24],
    [0.58, 0.38, 0.38],
    [0.44, 0.33, 0.20],
    [0.36, 0.40, 0.44],
    [0.60, 0.52, 0.36],
];

/// The living reef's colour at (x, z): one colony's hue to a patch of half a
/// unit or so, blending into the next only across a narrow seam, so the
/// ground between the modelled corals reads as a carpet of colonies.
pub fn reef_tone(x: f32, z: f32) -> [f32; 3] {
    // Warped a little by a second noise, so the patches are not round blobs.
    let warp = vnoise(x * 2.1 + 9.0, z * 2.1, 72) - 0.5;
    let at = vnoise(x * 1.2 + warp, z * 1.2 - warp, 70) * 3.0
        + vnoise(x * 0.31, z * 0.31 + 4.0, 71) * 4.0;
    let n = REEF_FLOOR.len();
    let i = at.floor() as usize;
    let seam = smooth(0.05, 0.95, at - at.floor());
    let (a, b) = (REEF_FLOOR[i % n], REEF_FLOOR[(i + 1) % n]);
    let c = [0, 1, 2].map(|k| a[k] + (b[k] - a[k]) * seam);
    // A carpet of many small colonies, seen from a few metres, greys toward
    // the rubble and sand between them.
    let grey = 0.30 * c[0] + 0.50 * c[1] + 0.20 * c[2];
    c.map(|v| v + (grey - v) * 0.35)
}

/// The seabed's colour at (x, z), its biome's floor with the living reef
/// laid over the coral's bommies: what the foot of a model standing there is
/// bedded into, and what the terrain is painted with.
pub fn floor_colour(seabed: &Seabed, b: &Biome, x: f32, z: f32) -> [f32; 3] {
    let base = floor_albedo(b);
    let coral = b.weight(CORAL) as f32;
    if coral <= 0.0 {
        return base;
    }
    let live = coral * reef_cover(seabed.reef(x, z));
    let tone = reef_tone(x, z);
    [0, 1, 2].map(|k| base[k] + (tone[k] - base[k]) * live)
}

/// The colour of the seabed where `b` is, blended across a biome border.
pub fn floor_albedo(b: &Biome) -> [f32; 3] {
    let t = b.w as f32;
    let (a, c) = (FLOOR_ALBEDO[b.a], FLOOR_ALBEDO[b.b]);
    [
        a[0] + (c[0] - a[0]) * t,
        a[1] + (c[1] - a[1]) * t,
        a[2] + (c[2] - a[2]) * t,
    ]
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Sprite {
    pub place: [f32; 4],
    pub tint: [f32; 4],
    /// x: how far the sprite is drawn out along the view's horizontal, for
    /// bubbles streaking past in a fast passage.
    pub stretch: [f32; 4],
}

#[derive(Clone, Copy, Debug)]
pub struct Light {
    pub pos: [f32; 3],
    pub range: f32,
    pub colour: [f32; 3],
    /// The shader's flicker phase; zero means steady.
    pub phase: f32,
}

/// Animations, as the vertex shader knows them.
pub mod anim {
    pub const SWAY: f32 = 1.0;
    pub const SWIM: f32 = 2.0;
    pub const FLUKE: f32 = 3.0;
    pub const WRITHE: f32 = 4.0;
    /// A medusa's bell swimming: a quick contraction that narrows the margin
    /// and lengthens the bell, a slow relaxation, and the animal rising on
    /// each stroke and sinking between. Amplitude is how far the margin
    /// narrows; frequency is the stroke's, in radians a second.
    pub const PULSE: f32 = 5.0;
    /// A tridacna shell hinged at its base, opening and closing.
    pub const SHELL: f32 = 6.0;
    /// An arm waving about the vertical: the trench crinoids.
    pub const WAVE: f32 = 7.0;
    /// A flipper beating about the forward axis: the polar penguins.
    pub const FLAP: f32 = 8.0;
    /// A narwhal's body: a slow bob and yaw, with the fluke's beat.
    pub const NARWHAL: f32 = 9.0;
    /// A medusa's oral arms or tentacles, placed like its bell and given the
    /// same phase, amplitude and frequency: they rise and sink with it, the
    /// margin pulls their roots in, and the stroke travels down them as a
    /// wave that lags toward the tips.
    pub const TRAIL: f32 = 10.0;
    /// A comb jelly gliding on its cilia: a slow bob, sway and turn.
    pub const GLIDE: f32 = 11.0;
}

/// Materials, as the fragment shader knows them.
pub mod material {
    pub const LAVA: f32 = 1.0;
    pub const COPPER: f32 = 2.0;
    /// Faceted glass in a passage's wall: `fs_glass` gives it a crystal's
    /// key highlight and warm core.
    pub const CRYSTAL: f32 = 3.0;
    pub const VAULT: f32 = 5.0;
    pub const ICE: f32 = 6.0;
    /// The jellyfish's translucent tissues: bell, oral arms and tentacles.
    pub const JELLY: f32 = 7.0;
    /// The thinner jelly tissues, which take a softer, veined glow.
    pub const JELLY_SOFT: f32 = 9.0;
    pub const GOLD: f32 = 8.0;
    /// A comb jelly: clear, with rainbow light running down its comb rows.
    pub const COMB: f32 = 10.0;
    /// The drowned city's brass and iron, its windows, lit glass and neon:
    /// `city_light` in `scene.wesl`, marked part by part (`mesh::city`).
    pub const CITY: f32 = 11.0;
}

// Sprite kinds.
const MOTE: f32 = 0.0;
const BUBBLE: f32 = 1.0;
const GLOW: f32 = 2.0;
const EMBER: f32 = 3.0;

pub fn hash(ix: i32, iz: i32, salt: u32) -> f32 {
    let mut h = (ix as u32).wrapping_mul(0x8da6_b343)
        ^ (iz as u32).wrapping_mul(0xd816_3841)
        ^ salt.wrapping_mul(0xcb1a_b31f);
    h = (h ^ (h >> 13)).wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0x00ff_ffff) as f32 / 16_777_216.0
}

pub fn vnoise(x: f32, z: f32, salt: u32) -> f32 {
    let (fx, fz) = (x.floor(), z.floor());
    let (ix, iz) = (fx as i32, fz as i32);
    let (tx, tz) = (x - fx, z - fz);
    let ux = tx * tx * (3.0 - 2.0 * tx);
    let uz = tz * tz * (3.0 - 2.0 * tz);
    let a = hash(ix, iz, salt);
    let b = hash(ix + 1, iz, salt);
    let c = hash(ix, iz + 1, salt);
    let d = hash(ix + 1, iz + 1, salt);
    let top = a + (b - a) * ux;
    let bottom = c + (d - c) * ux;
    top + (bottom - top) * uz
}

/// The ice sheet's three noise fields and its arctic coverage, baked into one
/// RGBA8 tile over a world region around the camera: the fragment shader
/// sampled three `vnoise` stacks per pixel before, which was the ice's whole
/// 1.1 ms. The values are exactly the ones `fs_model`'s ICE case uses.
pub fn ice_bake(w: &World, origin: [f32; 2], size: f32, res: u32) -> Vec<u8> {
    let mut out = vec![0u8; (res * res * 4) as usize];
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8;
    for j in 0..res {
        for i in 0..res {
            let u = (i as f32 + 0.5) / res as f32;
            let v = (j as f32 + 0.5) / res as f32;
            let x = origin[0] + u * size;
            let z = origin[1] + v * size;
            let coverage = smooth(0.35, 0.65, w.share(x, z, ARCTIC));
            let floes = vnoise(x * 0.9, z * 1.3, 14);
            let lead = 1.0
                - smooth(
                    0.02,
                    0.06,
                    (vnoise(x * 0.35 + 3.0, z * 0.08, 15) - 0.5).abs(),
                );
            let bumps = vnoise(x * 6.0, z * 6.0, 16);
            let k = ((j * res + i) * 4) as usize;
            out[k] = q(floes);
            out[k + 1] = q(lead);
            out[k + 2] = q(bumps);
            out[k + 3] = q(coverage);
        }
    }
    out
}

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The slope of [`smooth`] at `v`: nothing at either end, steepest halfway.
fn smooth_rate(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    6.0 * t * (1.0 - t) / (b - a)
}

/// One bubble of a stream that sheds one every `life / stream` seconds: how
/// far through its life it is, which cohort it came from (for its size and
/// spread), and how bright it is. The brightness reaches nothing at both ends
/// of its life, so a bubble is never seen to pop when the stream recycles it.
struct Bubble {
    age: f32,
    cohort: i32,
    fade: f32,
}

/// The `index`th bubble of `stream`, at time `t`, each living `life`
/// seconds: a fixed age for every (t, index), a continuous life, and no
/// per-frame reconsideration of where it was born. It rises and drifts from
/// where it was shed, so its own stream's spacing follows its emitter's way.
fn bubble(index: i32, stream: i32, t: f32, life: f32) -> Bubble {
    let cycle = t / life + index as f32 / stream as f32;
    let cohort = cycle.floor() as i32;
    let age = (cycle - cohort as f32) * life;
    Bubble {
        age,
        cohort,
        fade: smooth(0.0, 0.08 * life, age) * smooth(life, 0.86 * life, age),
    }
}

/// The line the voyage follows across the seabed: a gentle meander in z.
/// Everything — the biome borders, the far mountains, the cave's tunnel and
/// the lanes the swimmers keep to — is laid out about it.
pub fn corridor(x: f32) -> f32 {
    2.2 * (x * 0.11 + 0.7).sin() + 0.8 * (x * 0.031).sin()
}

/// The corridor's slope, dz/dx.
pub fn corridor_slope(x: f32) -> f32 {
    2.2 * 0.11 * (x * 0.11 + 0.7).cos() + 0.8 * 0.031 * (x * 0.031).cos()
}

pub fn voyage_x(x: f32, z: f32) -> f32 {
    let off = z - corridor(x);
    x + 1.2 * (off * 0.31).sin() + 0.8 * ((off * 0.73 + 1.3).sin() - 1.3f32.sin())
}

/// The trench beside the corridor, as a change in the seabed's height at
/// `across` units to port of it (where the window looks) and `x` along it: a
/// level rim the boat sails along, cut by gullies running down into the
/// trench; below it terraces stepping down past the window to a floor some
/// sixty metres down, and the far wall rising again in terraces across the
/// gulf. Behind the boat the ground steps up, so a turn about looks up a wall.
/// Sines rather than noise, as `ground` in the shader pays for it per vertex.
pub fn trench_profile(x: f32, across: f32) -> f32 {
    let u = across - trench_wander(x);
    let down = 1.2 * smooth(1.3, 1.6, u)
        + 1.5 * smooth(2.1, 2.4, u)
        + 1.6 * smooth(2.8, 3.2, u)
        + 0.8 * smooth(3.3, 3.8, u);
    let up = 1.8 * smooth(4.0, 4.35, u)
        + 1.6 * smooth(4.9, 5.2, u)
        + 1.6 * smooth(5.7, 6.0, u)
        + 1.3 * smooth(6.6, 7.0, u);
    let back = 1.2 * smooth(-1.9, -2.4, u) + 1.4 * smooth(-3.6, -4.2, u);
    // Gullies cross the rim and the first terraces, deepening as they go.
    let cut = smooth(0.55, 0.95, (x * 1.9 + 1.4 * (x * 0.61).sin()).sin());
    let gully =
        cut * (0.2 + 0.35 * smooth(-1.2, 1.6, u)) * smooth(-2.0, -1.2, u) * smooth(2.8, 2.2, u);
    // Ledged strata down the walls: small shelves that follow the terraces
    // rather than straight lines. Nothing on the rim the boat sails along.
    let strata = 2.0 * smooth(-0.3, 0.3, (u * 9.0 + 0.8 * (x * 0.7).sin()).sin()) - 1.0;
    let ledge = 0.12 * strata * smooth(1.2, 1.6, u) * (1.0 - smooth(6.8, 7.4, u));
    up + back - down - gully + ledge
}

/// How far the trench's terraces wander to port of their line at `x`, so
/// their edges never run straight.
pub fn trench_wander(x: f32) -> f32 {
    0.25 * (x * 0.29 + 1.0).sin() + 0.12 * (x * 0.83).sin()
}

/// The coral reef's plan, over the heightmap's grain: three crossing swells
/// whose sum is high on the bommies and the ridges that join them and low in
/// the sand gutters that wind between. Every wave fits the heightmap's 64
/// units, so the reef repeats with it. `biome_shape` raises the bommies out of
/// it, the seabed's colour ([`reef_cover`]) says where living coral covers
/// the ground, and the placers grow the big colonies on its tops.
pub fn reef_field(x: f32, z: f32, detail: f32) -> f32 {
    0.5 + 0.22 * (TAU * (x * 0.25 + z * 0.125) + 1.3 * detail).sin()
        + 0.20 * (TAU * (x * 0.125 - z * 0.25) + 0.9 * detail).sin()
        + 0.12 * (TAU * (x * 0.375 + z * 0.3125) + 2.1 * detail).sin()
}

/// How much of the ground at a point of [`reef_field`] is living coral rather
/// than sand: all of the bommies and their skirts, about two thirds of the
/// reef, thinning to clean sand down in the gutters.
pub fn reef_cover(field: f32) -> f32 {
    smooth(0.28, 0.46, field)
}

/// How high a bommie stands at a point of [`reef_field`], 0 to 1: its
/// flat-topped body, where the reef's largest colonies grow.
pub fn reef_mound(field: f32) -> f32 {
    smooth(0.50, 0.82, field)
}

/// The shape each biome gives its own floor, over the shared heightmap: the
/// same function as `biome_shape` in `scene.wesl`. `across` is signed distance
/// from the corridor (positive to port), `h` the heightmap's height and
/// `detail` its fine grain. Every term but the cave's is periodic over the
/// heightmap's 64 units, so the added shape repeats with it; the cave's
/// ledges stand back from its halls, as its massif does.
fn biome_shape(b: &Biome, x: f32, z: f32, across: f32, h: f32, detail: f32) -> f32 {
    let weight = |id: usize| b.weight(id) as f32;
    let mut shape = 0.0;
    let shallows = weight(SHALLOWS);
    if shallows > 0.0 {
        // Low banks with sand waves breaking over them.
        let banks = smooth(0.45, 0.72, 0.5 + 0.5 * detail);
        let waves = (TAU * (x * 0.25 + z * 0.125) + 1.4 * detail).sin()
            + 0.5 * (TAU * (x * 0.125 - z * 0.25) + 0.8 * detail).sin();
        shape += shallows * (0.10 * h + 0.10 * banks + 0.05 * waves);
    }
    let forest = weight(FOREST);
    if forest > 0.0 {
        // A low silt flat, hummocked and pocked with the mounds roots raise.
        let hummocks = (TAU * (x * 0.5 + z * 0.25) + 1.2 * detail).sin()
            + 0.6 * (TAU * (x * 0.25 - z * 0.5) + 0.7 * detail).sin();
        let mounds = smooth(0.45, 0.75, 0.5 + 0.5 * detail);
        shape += forest * (-0.05 * h + 0.12 * hummocks + 0.25 * detail + 0.30 * mounds);
    }
    let wrecks = weight(WRECKS);
    if wrecks > 0.0 {
        // Sediment drifts running along the wreck field, with rock showing
        // through where the drift is thin.
        let drifts =
            (TAU * z * 0.125 + 0.8 * detail).sin() + 0.4 * (TAU * (z * 0.375 + x * 0.0625)).sin();
        let rocks = smooth(0.60, 0.85, 0.5 + 0.5 * detail);
        shape += wrecks * (0.25 * h + 0.20 * drifts + 0.40 * rocks);
    }
    let atlantis = weight(ATLANTIS);
    if atlantis > 0.0 {
        // Level city terraces: broad plateaus in x and z, with soft risers
        // between them, so the paving keeps a clear, flat floor.
        let terrace =
            |v: f32| 2.0 * smooth(-0.22, 0.22, (TAU * v * 0.25 + 0.25 * detail).sin()) - 1.0;
        shape +=
            atlantis * (-0.11 * h + 0.22 * terrace(x) + 0.18 * terrace(z * 2.0) + 0.02 * detail);
    }
    let cave = weight(CAVE);
    if cave > 0.0 {
        // Ledged basalt round the tunnel's mouths. Kept clear of the tunnel
        // itself, so the floor the helm follows and the vault are unchanged.
        let ledges =
            2.0 * smooth(-0.35, 0.35, (TAU * across.abs() / 1.2 + 0.4 * detail).sin()) - 1.0;
        let k = tunnel_stretch(x).0;
        let near = smooth(0.45 * k, 0.9 * k, across.abs());
        shape += cave * near * (0.25 * ledges + 0.12 * detail);
    }
    let abyss = weight(ABYSS);
    if abyss > 0.0 {
        // Folded rock: sharp ridges from the grain, cut through by canyons
        // that follow the heightmap's own contours.
        let fold = 1.0 - 2.0 * detail.abs();
        let canyon = 1.0 - smooth(0.0, 0.09, (h - 0.25).abs());
        shape += abyss * (0.32 * h + 0.65 * (fold * fold - 0.35) - 0.45 * canyon);
    }
    let arctic = weight(ARCTIC);
    if arctic > 0.0 {
        // Ice-scoured and flat: long shallow grooves, not banks.
        let scour = 1.0 - smooth(0.0, 0.22, (TAU * z * 0.5 + 0.7 * detail).sin().abs());
        shape += arctic * (-0.10 * h + 0.06 * detail - 0.12 * scour);
    }
    let trench = weight(TRENCH);
    if trench > 0.0 {
        shape += trench * (0.05 * h + 0.08 * detail);
    }
    let coral = weight(CORAL);
    if coral > 0.0 {
        // A reef built of its own coral: flat-topped bommies and the ridges
        // that join them standing a couple of metres proud, their tops
        // lumpy with colonies, and the sand gutters that wind between them
        // scoured deeper ([`reef_field`]).
        let field = reef_field(x, z, detail);
        let mound = reef_mound(field);
        let gutter = 1.0 - smooth(0.12, 0.32, field);
        shape += coral
            * (0.15 * h + 0.42 * mound - 0.10 * gutter + 0.06 * detail * mound + 0.03 * detail);
    }
    let sargasso = weight(SARGASSO);
    if sargasso > 0.0 {
        // A low, rolling plain under the floating weed, drifted into long
        // swells rather than risen into banks.
        let drift = (TAU * (x * 0.125 + z * 0.0625) + 0.9 * detail).sin();
        let swell = smooth(0.42, 0.74, 0.5 + 0.5 * detail);
        shape += sargasso * (0.04 * h + 0.10 * drift + 0.16 * swell);
    }
    let vents = weight(VENTS);
    if vents > 0.0 {
        // Broken pillow basalt: hard, lumpy mounds round the chimneys, with
        // the grain's folds standing up as cooled lobes of lava.
        let fold = 1.0 - 2.0 * detail.abs();
        let mound = smooth(0.58, 0.86, 0.5 + 0.5 * detail);
        shape += vents * (0.28 * h + 0.42 * (fold * fold - 0.35) + 0.30 * mound);
    }
    let brine = weight(BRINE);
    if brine > 0.0 {
        // Salt terraces, broad and shallow, with the brine pans between them
        // sunk just below the crust.
        let terrace = 2.0 * smooth(-0.55, 0.55, (TAU * x * 0.125 + 0.2 * detail).sin()) - 1.0;
        let pan = smooth(0.62, 0.88, 0.5 + 0.5 * detail);
        shape += brine * (0.04 * h + 0.055 * terrace - 0.06 * pan + 0.02 * detail);
    }
    let bloom = weight(BLOOM);
    if bloom > 0.0 {
        // A soft ooze floor: low, rounded hummocks where the jellies gather.
        let ooze = (TAU * (x * 0.5 + z * 0.25) + 1.0 * detail).sin()
            + 0.6 * (TAU * (x * 0.25 - z * 0.5) + 0.6 * detail).sin();
        let mounds = smooth(0.40, 0.72, 0.5 + 0.5 * detail);
        shape += bloom * (-0.05 * h + 0.10 * ooze + 0.22 * mounds);
    }
    let city = weight(CITY);
    if city > 0.0 {
        // Level city blocks: broad plazas stepped down in long, low terraces
        // and scarcely touched by the heightmap, so the towers stand on
        // level ground and their plinths need not be buried deep.
        let step =
            2.0 * smooth(
                -0.3,
                0.3,
                (TAU * (x * 0.0625 + z * 0.125) + 0.3 * detail).sin(),
            ) - 1.0;
        shape += city * (-0.06 * h + 0.10 * step + 0.015 * detail);
    }
    shape
}

/// Where the trench's terrace edges lie across the corridor, before they
/// wander: the three steps down from the rim on the near side, then the far
/// wall's three lips, which face the window.
const NEAR_LIPS: [f32; 3] = [1.3, 2.1, 2.8];
const FAR_LIPS: [f32; 4] = [4.35, 5.2, 6.0, 7.0];

/// The lava cave is a tunnel through a massif: its radius, and how high the
/// rock rises either side of it.
pub const TUNNEL: f32 = 1.05;
pub const MASSIF: f32 = 4.5;
/// The cave's share of the biome mix from which its tunnel is built.
pub const TUNNEL_SHARE: f32 = 0.85;

/// The tunnel is not one bore but a string of halls joined by narrows: three
/// halls to a leg, the first a tenth of the way in, so every cave has them in
/// the same places along its length and the shader's massif, which knows only
/// `x`, agrees with the arch. In a narrows the arch is half as wide again as
/// the old bore and a little higher; in a hall nearly three times as wide
/// and twice as high, so the window, which can only look sideways, looks
/// across a floor to a far wall several hull lengths off instead of into the
/// rock beside the glass.
const HALLS: f32 = 3.0;
const HALL_AT: f32 = 0.10;
const NARROWS_WIDE: f32 = 1.55;
const NARROWS_HIGH: f32 = 1.15;
const HALL_WIDE: f32 = 2.8;
const HALL_HIGH: f32 = 1.9;

/// How far the tunnel has opened into a hall at `x`: one through a hall's
/// middle stretch, easing to nothing in the narrows between. As `hall` in
/// `scene.wesl`.
pub fn hall(x: f32) -> f32 {
    let u = (x / crate::voyage::LEG as f32 * HALLS - HALL_AT * HALLS + 0.5).rem_euclid(1.0) - 0.5;
    smooth(0.30, 0.10, u.abs())
}

/// What fills each of a cave's three halls, in order through the tunnel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HallKind {
    /// A fall of molten rock down the far wall into a glowing river along
    /// its foot, and the ember pool in its nook.
    Forge,
    /// Colonies of luminous things on the walls, a still pool lit from
    /// within, and the crystal pocket.
    Grotto,
    /// The organ of basalt columns, a shaft of pale light from a chimney in
    /// the vault, and the abandoned camp.
    Colonnade,
}

/// One hall of a cave: what is in it and the middle of it along the voyage.
#[derive(Clone, Copy, Debug)]
pub struct Hall {
    pub kind: HallKind,
    pub x: f32,
}

/// The tunnel's stretch at `x`: how many times the old bore's radius it is
/// across, and how many times its height.
pub fn tunnel_stretch(x: f32) -> (f32, f32) {
    let g = hall(x);
    (
        NARROWS_WIDE + (HALL_WIDE - NARROWS_WIDE) * g,
        NARROWS_HIGH + (HALL_HIGH - NARROWS_HIGH) * g,
    )
}

/// The tunnel's half-width at `x`, before its ragged edge.
pub fn tunnel_wide(x: f32) -> f32 {
    TUNNEL * tunnel_stretch(x).0
}

/// What lies off the lava cave's tunnel, glimpsed as the Nautilus passes:
/// passages to places it never goes, each lit by what is in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Secret {
    /// A passage down to a chamber of molten rock, embers drifting out.
    Ember,
    /// A grotto lit by colonies of luminous creatures.
    Grotto,
    /// A gallery of crystals, glinting violet.
    Crystal,
    /// A doorway cut by hands long gone, with steps up into a warm light.
    Door,
    /// A chimney in the vault, a shaft of pale light falling from it.
    Chimney,
}

impl Secret {
    const ALL: [Secret; 5] = [
        Secret::Ember,
        Secret::Grotto,
        Secret::Crystal,
        Secret::Door,
        Secret::Chimney,
    ];

    /// The colour of the light within, and how strongly it glows.
    fn light(self) -> ([f32; 3], f32) {
        match self {
            Secret::Ember => ([1.0, 0.33, 0.06], 1.2),
            Secret::Grotto => ([0.12, 0.85, 0.62], 0.75),
            Secret::Crystal => ([0.44, 0.38, 1.0], 0.50),
            Secret::Door => ([1.0, 0.72, 0.30], 0.9),
            Secret::Chimney => ([0.42, 0.68, 1.0], 0.85),
        }
    }
}

/// One of the tunnel's openings, laid out in the corridor's own frame: along
/// the voyage in x, across from the corridor in `o` (z less `corridor(x)`),
/// and up from the tunnel's floor.
#[derive(Clone, Copy, Debug)]
pub struct Opening {
    pub kind: Secret,
    /// The middle of its mouth along the voyage, and half its width.
    pub x: f32,
    pub half: f32,
    /// How far it runs out of the tunnel (up, for a chimney), how far it
    /// bends along the voyage by its end, and how far its floor climbs.
    pub depth: f32,
    pub bend: f32,
    pub rise: f32,
    /// Where it leaves the tunnel, across from the corridor: the wall for a
    /// side passage, the vault (across and up) for a chimney. The tunnel's
    /// width changes along it, so each opening takes the wall where it is.
    pub wall: f32,
    pub crown: [f32; 2],
}

/// What is tucked into one of the tunnel's shallow side nooks: a pocket of
/// crystals, a still pool of lava, or a landing party's abandoned camp.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NookKind {
    Crystal,
    Ember,
    Camp,
}

/// A shallow recess in the tunnel wall, cut where a softer pocket of lava
/// weathered out. It is a hollow, not a passage: the wall stands back a
/// little over a hand's breadth of arc, and what is in it is close enough to
/// see from the window.
#[derive(Clone, Copy, Debug)]
pub struct Nook {
    pub kind: NookKind,
    /// Its middle along the voyage, and round the arch (radians from the
    /// window-side floor).
    pub x: f32,
    pub a: f32,
    /// How far it stands back, and how wide it is along and round the arch.
    pub depth: f32,
    pub width: f32,
    pub spread: f32,
}

/// The nooks in each cave, one to a hall.
const NOOKS: usize = 3;
/// How much larger a nook's things are than they were by the old bore's
/// wall, now that the window sees them from across a hall.
const NOOK_GROW: f32 = 1.7;

/// The tunnel's rings: how far apart, and how many points round each.
const RING_STEP: f32 = 0.1;
const AROUND: usize = 36;
/// Where a side passage leaves the wall, across from the corridor, and the
/// last point round the ring its mouth takes in, from the window's side's foot.
const WALL: f32 = 1.0;
const SIDE_TOP: usize = 11;
/// Openings off each tunnel.
const OPENINGS: usize = 5;
/// The points round the ring a chimney's mouth spans: in the crown, a little
/// toward the window, where it can be seen.
const CHIMNEY_AROUND: (usize, usize) = (13, 17);
/// Where a chimney's mouth lies across from the corridor and above the floor.
const CHIMNEY_O: f32 = 0.37;
const CHIMNEY_H: f32 = 1.0;

impl Opening {
    fn side(&self) -> bool {
        self.kind != Secret::Chimney
    }

    /// The tunnel's rings the mouth spans, first and last.
    fn rings(&self) -> (i32, i32) {
        let k0 = ((self.x - self.half) / RING_STEP).round() as i32;
        let k1 = ((self.x + self.half) / RING_STEP).round() as i32;
        (k0, k1.max(k0 + 2))
    }

    /// The middle of its section a fraction `s` of the way in, as (x, o,
    /// height above the floor), and the section's sideways direction in (x, o).
    fn path(&self, s: f32) -> ([f32; 3], [f32; 2]) {
        let x = self.x + self.bend * s * s;
        if self.side() {
            let t = [2.0 * self.bend * s, self.depth];
            let l = (t[0] * t[0] + t[1] * t[1]).sqrt();
            (
                [x, self.wall + self.depth * s, 0.2 + self.rise * s * s],
                [t[1] / l, -t[0] / l],
            )
        } else {
            (
                [x, self.crown[0] + 0.35 * s, self.crown[1] + self.depth * s],
                [1.0, 0.0],
            )
        }
    }

    /// A point of its wall a fraction `s` in, `angle` round its section: an
    /// arch open to the floor for a passage (a squared one for a doorway), a
    /// ring for a chimney. In (x, o, height above the floor).
    fn wall(&self, s: f32, angle: f32) -> [f32; 3] {
        let (c, across) = self.path(s);
        if self.side() {
            let w = self.half * (1.0 + 0.1 * s);
            let h = 0.62 * (1.0 - 0.15 * s);
            let (mut a, mut b) = (angle.cos(), angle.sin());
            if self.kind == Secret::Door {
                a = (1.4 * a).clamp(-1.0, 1.0);
                b = (1.4 * b).clamp(-1.0, 1.0);
            }
            [
                c[0] + across[0] * w * a,
                c[1] + across[1] * w * a,
                c[2] + h * b,
            ]
        } else {
            let r = 1.0 - 0.3 * s;
            [
                c[0] + self.half * r * angle.cos(),
                c[1] + 0.22 * r * angle.sin(),
                c[2],
            ]
        }
    }
}

/// From the corridor's frame into the world, over a floor at `floor`.
fn corridor_point(p: [f32; 3], floor: f32) -> [f32; 3] {
    [p[0], floor + p[2], corridor(p[0]) + p[1]]
}

/// Visual depth of each biome's seabed below the surface, which lies at y = 0
/// whatever the boat does. The voyage's own depths run from one to forty
/// units; here they are compressed, so the drop into the abyss is a steep
/// shelf rather than a gulf. The gauge undoes this (`helm::depth_reading`).
pub const SEABED: [f32; BIOMES] = [
    1.7, 3.6, 4.9, 5.8, 6.6, 9.3, 3.5, 4.2, 2.2, 3.4, 8.2, 6.3, 10.2, 7.2,
];
/// How strongly the heightmap's relief shows in each biome: rugged in the
/// abyss, nearly flat under the paved city of Atlantis, the salt of the
/// brine flats and the drowned city's streets.
pub const RELIEF: [f32; BIOMES] = [
    0.25, 0.35, 0.35, 0.12, 0.20, 1.20, 0.40, 0.30, 0.45, 0.30, 0.90, 0.06, 0.35, 0.08,
];

/// The seabed of one visit: its heightmap and the biome sequence's seed.
pub struct Seabed {
    pub map: Vec<[f32; 4]>,
    pub seed: f32,
    /// This visit's fourteen-leg circuit, as [`crate::voyage::order_for`].
    pub order: [usize; BIOMES],
}

impl Seabed {
    pub fn new(visit: u64, seed: f32) -> Self {
        Self {
            map: super::heightmap::generate(visit),
            seed,
            order: crate::voyage::order_for(seed),
        }
    }

    pub fn biome(&self, x: f32, z: f32) -> Biome {
        Biome::in_order(f64::from(voyage_x(x, z)), &self.order)
    }

    pub fn share(&self, x: f32, z: f32, id: usize) -> f32 {
        self.biome(x, z).weight(id) as f32
    }

    fn by_biome(b: &Biome, values: [f32; BIOMES]) -> f32 {
        values[b.a] + (values[b.b] - values[b.a]) * b.w as f32
    }

    /// The coral reef's plan at (x, z), [`reef_field`] on this seabed's grain:
    /// high on the bommies, low in the sand gutters.
    pub fn reef(&self, x: f32, z: f32) -> f32 {
        reef_field(x, z, crate::dive3d::heightmap::sample(&self.map, x, z)[3])
    }

    /// The seabed's height at (x, z), exactly as `ground` in the shader: the
    /// biome's depth, the heightmap's relief and grain, the shape each biome
    /// gives its floor, a little fine roughness and the mountains that rise in
    /// the far distance.
    pub fn ground(&self, x: f32, z: f32) -> f32 {
        let b = self.biome(x, z);
        let sample = crate::dive3d::heightmap::sample(&self.map, x, z);
        let base = -Self::by_biome(&b, SEABED);
        let relief = Self::by_biome(&b, RELIEF) * sample[0];
        let rough = Self::by_biome(
            &b,
            [
                0.05, 0.07, 0.05, 0.02, 0.03, 0.08, 0.05, 0.05, 0.06, 0.05, 0.08, 0.02, 0.04, 0.02,
            ],
        );
        let ridges = Self::by_biome(
            &b,
            [
                1.6, 2.2, 2.0, 2.6, 3.5, 5.0, 1.4, 2.4, 2.6, 2.0, 5.5, 0.5, 3.0, 1.2,
            ],
        );
        let n = 0.65 * vnoise(x * 1.3, z * 1.3, 1) + 0.35 * vnoise(x * 3.1 + 5.0, z * 3.1, 2);
        let across = z - corridor(x);
        let off = across.abs();
        // Far mountains, kept low and broken up by a second noise octave; the
        // brine flats almost leave them out, so the salt plain runs to the
        // water instead of ending at a sharp pyramid, and the drowned city
        // keeps them low, so its far towers stand where the hills would.
        let far = smooth(9.0, 26.0, off)
            * (0.15
                + 0.65
                    * ridges
                    * (0.65 * vnoise(x * 0.16, z * 0.16 + 3.0, 4)
                        + 0.35 * vnoise(x * 0.47, z * 0.43, 6)))
            * Self::by_biome(
                &b,
                [
                    1.0, 1.0, 1.0, 0.8, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 1.0, 0.15, 1.0, 0.3,
                ],
            );
        // The lava cave's massif, rising either side of its tunnel.
        let mut massif = b.weight(CAVE) as f32
            * MASSIF
            * smooth(tunnel_wide(x) * 1.05, tunnel_wide(x) * 1.8, off)
            * (0.85 + 0.3 * vnoise(x * 0.4, z * 0.4, 5));
        // Cut back to the floor where a side passage runs into it. Only here:
        // the shader's `ground` beds models in the sand, and nothing it beds
        // stands in a passage.
        if massif > 0.0 {
            massif *= 1.0 - self.bay(x, z - corridor(x));
        }
        let trench = b.weight(TRENCH) as f32;
        let trench = if trench > 0.0 {
            trench * trench_profile(x, across)
        } else {
            0.0
        };
        let shape = biome_shape(&b, x, z, across, sample[0], sample[3]);
        base + relief + shape + rough * (n - 0.45) + far + massif + trench
    }

    /// The openings off the lava cave's tunnel nearest `x`, if a cave is
    /// near: the same five secrets in every cave, in an order and at places
    /// of the seabed's own.
    pub fn openings(&self, x: f32) -> Option<[Opening; OPENINGS]> {
        let leg = crate::voyage::LEG as f32;
        let u = (x / leg).floor();
        let k = [u, u + 1.0]
            .into_iter()
            .find(|k| Biome::at(f64::from((k + 0.5) * leg), self.seed).a == CAVE)?;
        let ki = k as i32;
        let salt = self.seed.to_bits() & 0xffff;
        // The four side passages in an order of the seabed's own; the
        // chimney last, in the vault between the first two.
        let mut kinds = Secret::ALL;
        for i in (1..OPENINGS - 1).rev() {
            let j = (hash(ki, i as i32, 700 + salt) * (i + 1) as f32) as usize;
            kinds.swap(i, j.min(i));
        }
        // The five secrets lie in the two narrows between the halls, two
        // passages to each, at the narrows' ends where the window, looking
        // up the tunnel, sees each mouth from across the hall before it; the
        // chimney rises from the vault of the first narrows, between its two.
        Some(std::array::from_fn(|i| {
            let h = |n: u32| hash(ki, i as i32, 710 + n + salt);
            let kind = kinds[i];
            let along = if kind == Secret::Chimney {
                0.252
            } else {
                // Inside the narrows (`hall` is nothing from 0.2 to 0.333
                // of the leg, and again a third on), bend and all: a
                // passage running on into a hall would hang in its water.
                [0.212, 0.292, 0.545, 0.625][i.min(3)]
            };
            let x = (k + along) * leg + (h(0) - 0.5) * 0.06;
            // Bent on ahead, all alike so none runs into the next, so the
            // window looks along each as it passes and its end is round
            // the corner; the chimney back over the tunnel.
            let toward = if kind == Secret::Chimney { -0.4 } else { 1.0 };
            let (half, depth, bend, rise) = match kind {
                Secret::Ember => (0.25, 1.5, 0.3, -0.08),
                Secret::Grotto => (0.24, 1.4, 0.3, 0.06),
                Secret::Crystal => (0.23, 1.3, 0.3, 0.10),
                Secret::Door => (0.17, 1.2, 0.3, 0.0),
                Secret::Chimney => (0.15, 1.6, 0.30, 0.0),
            };
            let (k, v) = tunnel_stretch(x);
            Opening {
                kind,
                x,
                half,
                depth: depth * (0.9 + 0.2 * h(1)),
                bend: bend * toward,
                rise,
                wall: WALL * k,
                crown: [
                    CHIMNEY_O * k,
                    0.25 * TUNNEL + (CHIMNEY_H - 0.25 * TUNNEL) * v,
                ],
            }
        }))
    }

    /// The leg of the cave nearest `x`, if a cave is near.
    fn cave_leg(&self, x: f32) -> Option<f32> {
        let leg = crate::voyage::LEG as f32;
        let u = (x / leg).floor();
        [u, u + 1.0]
            .into_iter()
            .find(|k| Biome::at(f64::from((k + 0.5) * leg), self.seed).a == CAVE)
    }

    /// The cave's three halls, where its tunnel opens out (`hall`).
    pub fn halls(&self, x: f32) -> Option<[Hall; 3]> {
        let k = self.cave_leg(x)?;
        let leg = crate::voyage::LEG as f32;
        let kinds = [HallKind::Forge, HallKind::Grotto, HallKind::Colonnade];
        Some(std::array::from_fn(|i| Hall {
            kind: kinds[i],
            x: (k + HALL_AT + i as f32 / HALLS) * leg,
        }))
    }

    /// The cave's three nooks, in the same frame as the openings: one in the
    /// far wall of each hall, at about the window's height, where the window
    /// sees it from across the hall's floor rather than from an arm's length.
    /// The lava pool is in the first hall, by the fall; the crystals in the
    /// second, the grotto; the camp in the third, among the columns.
    pub fn nooks(&self, x: f32) -> Option<[Nook; NOOKS]> {
        let leg = crate::voyage::LEG as f32;
        let u = (x / leg).floor();
        let k = [u, u + 1.0]
            .into_iter()
            .find(|k| Biome::at(f64::from((k + 0.5) * leg), self.seed).a == CAVE)?;
        let ki = k as i32;
        let salt = self.seed.to_bits() & 0xffff;
        Some(std::array::from_fn(|i| {
            let h = |n: u32| hash(ki, i as i32, 730 + n + salt);
            // A little past each hall's middle: the window looks ahead, up
            // the hall, so it frames the nook while the boat is still in it.
            let (kind, angle, depth) = match i {
                0 => (NookKind::Ember, 0.02, 0.16),
                1 => (NookKind::Crystal, 0.10, 0.18),
                _ => (NookKind::Camp, 0.0, 0.15),
            };
            let along = HALL_AT + i as f32 / HALLS + 0.035;
            Nook {
                kind,
                x: (k + along) * leg + (h(0) - 0.5) * 0.10,
                a: angle * PI + (h(1) - 0.5) * 0.06,
                depth: depth * (0.85 + 0.3 * h(2)),
                width: 0.36 + 0.08 * h(3),
                spread: 0.20 + 0.05 * h(4),
            }
        }))
    }

    /// The tunnel's extra radius at (x, a) from the nooks: a smooth hollow,
    /// only ever outward, so a nook is more room for the hull, never less.
    fn nook_bump(&self, x: f32, a: f32) -> f32 {
        let Some(nooks) = self.nooks(x) else {
            return 0.0;
        };
        nooks.iter().fold(0.0f32, |most, n| {
            let dx = (x - n.x) / n.width;
            let da = (a - n.a) / n.spread;
            let bump = n.depth * (-(dx * dx + da * da) * 1.5).exp();
            most.max(bump)
        })
    }

    /// How far the side passages clear the massif at (x, o): 1 within one,
    /// where the rock is cut back to the tunnel's floor.
    fn bay(&self, x: f32, o: f32) -> f32 {
        let k = tunnel_stretch(x).0;
        if !(0.85 * k..WALL * k + 2.3).contains(&o) {
            return 0.0;
        }
        let Some(openings) = self.openings(x) else {
            return 0.0;
        };
        openings
            .iter()
            .filter(|p| p.side())
            .map(|p| {
                let s = ((o - p.wall) / p.depth).clamp(0.0, 1.0);
                let (c, _) = p.path(s);
                let width = p.half * (1.0 + 0.1 * s) + 0.12;
                smooth(width + 0.12, width, (x - c[0]).abs())
                    * smooth(p.wall + p.depth + 0.4, p.wall + p.depth + 0.2, o)
            })
            .fold(0.0, f32::max)
    }

    /// Whether (x, o) lies in the mouth of an opening, `side` or overhead, to
    /// keep columns and stalactites out of the way.
    pub fn in_mouth(&self, x: f32, o: f32, side: bool) -> bool {
        self.openings(x).is_some_and(|list| {
            list.iter().any(|p| {
                p.side() == side
                    && (x - p.x).abs() < p.half + 0.1
                    && if side {
                        o > 0.3
                    } else {
                        (p.crown[0] - 0.42..p.crown[0] + 0.43).contains(&o)
                    }
            })
        })
    }

    /// The tunnel's cross-section at `x`, where there is one: its centre, its
    /// sideways direction and how far the cave has taken over.
    pub fn tunnel_at(&self, x: f32) -> Option<([f32; 3], [f32; 3], f32)> {
        let zc = corridor(x);
        let share = self.share(x, zc, CAVE);
        if share < TUNNEL_SHARE {
            return None;
        }
        let floor = self.tunnel_floor(x);
        let s = corridor_slope(x);
        let n = (1.0 + s * s).sqrt();
        Some((
            [x, floor + 0.25 * TUNNEL, zc],
            [-s / n, 0.0, 1.0 / n],
            share,
        ))
    }

    /// The tunnel's floor at `x`: the cave's own floor, level through the
    /// massif, rising over the ground's real bumps but never following it down
    /// the shelf beyond, so the arch runs level to its mouths.
    pub fn tunnel_floor(&self, x: f32) -> f32 {
        let local = (0..5)
            .map(|i| {
                let ax = x - 0.5 + i as f32 * 0.25;
                self.ground(ax, corridor(ax))
            })
            .fold(f32::MIN, f32::max);
        local.max(-SEABED[CAVE] - 0.2)
    }

    /// The tunnel's ragged inner radius at `x` and angle `a` around it,
    /// with the shallow nooks standing the wall back a little here and there.
    pub fn tunnel_radius(x: f32, a: f32) -> f32 {
        TUNNEL * (0.91 + 0.18 * vnoise(x * 2.2, a * 3.0, 6))
    }

    /// The same with the cave's nooks, for the world builder and the props.
    fn tunnel_radius_with_nooks(&self, x: f32, a: f32) -> f32 {
        Self::tunnel_radius(x, a) + self.nook_bump(x, a)
    }

    /// Height of the lava cave's vault.
    pub fn vault(&self) -> f32 {
        -SEABED[CAVE] + CAVE_ROOF
    }

    /// The highest the boat may rise at `x`: a little under the surface or
    /// the ice, or well inside the tunnel's arch, which it settles under before
    /// it reaches the mouth.
    pub fn ceiling(&self, x: f32) -> f32 {
        let zc = corridor(x);
        let b = self.biome(x, zc);
        let ice = smooth(0.0, 0.3, b.weight(ARCTIC) as f32);
        let open = -0.30;
        let mut ceiling = open + (-ICE - 0.35 - open) * ice;
        let cave = smooth(TUNNEL_SHARE - 0.35, TUNNEL_SHARE, b.weight(CAVE) as f32);
        if cave > 0.0 {
            let floor = self.tunnel_floor(x);
            let arch = floor + 0.25 * TUNNEL + 0.85 * TUNNEL * 0.91 - 0.25;
            ceiling += (arch.min(ceiling) - ceiling) * cave;
        }
        ceiling
    }
}

/// The world around one frame of the voyage.
pub struct World<'a> {
    pub seabed: &'a Seabed,
    pub pose: Pose,
    pub t: f32,
    /// How much wider than the window, as a share of the distance out, a
    /// placement counts as [`World::visible`]: none for what is placed
    /// afresh every frame; for the static cache, the turn and the way the
    /// window may make before the cache is built again, so nothing that
    /// comes into view in between appears at the rim.
    pub slack: f32,
}

impl World<'_> {
    pub fn biome(&self, x: f32, z: f32) -> Biome {
        self.seabed.biome(x, z)
    }

    pub fn share(&self, x: f32, z: f32, id: usize) -> f32 {
        self.seabed.share(x, z, id)
    }

    fn by_biome(b: &Biome, values: [f32; BIOMES]) -> f32 {
        Seabed::by_biome(b, values)
    }

    pub fn ground(&self, x: f32, z: f32) -> f32 {
        self.seabed.ground(x, z)
    }

    pub fn reef(&self, x: f32, z: f32) -> f32 {
        self.seabed.reef(x, z)
    }

    pub fn camera(&self) -> f32 {
        self.pose.pos[0]
    }

    /// The window's height.
    pub fn eye(&self) -> f32 {
        self.pose.pos[1]
    }

    /// The sea's surface, at the same height wherever the boat goes.
    pub fn surface(&self) -> f32 {
        0.0
    }

    pub fn vault(&self) -> f32 {
        self.seabed.vault()
    }

    /// The underside of the polar ice, floating at the surface.
    pub fn ice(&self) -> f32 {
        -ICE
    }

    /// Whether a sphere of `radius` at `p` can be seen through the window,
    /// which is round: the corners of the square the sea is drawn into are
    /// behind the brass.
    pub fn visible(&self, p: [f32; 3], radius: f32) -> bool {
        if self.slack > 0.0 {
            return self.within(p, radius);
        }
        self.reveal(p, radius) > 0.001
    }

    /// How much farther than `distance` out a placer's cut-off reaches: none
    /// for what is placed afresh every frame; for the static cache, as far
    /// as the window may move and turn before it is built again, so nothing
    /// crossing a cut-off meanwhile is missing from it.
    pub fn margin(&self, distance: f32) -> f32 {
        if self.slack > 0.0 {
            CACHE_CELL + self.slack * distance
        } else {
            0.0
        }
    }

    /// Whether a sphere of `radius` at `p` is inside the window's cone
    /// widened by [`World::slack`], and by the [`CACHE_CELL`] the window may
    /// move sideways, for the static cache.
    fn within(&self, p: [f32; 3], radius: f32) -> bool {
        let d = [
            p[0] - self.pose.pos[0],
            p[1] - self.pose.pos[1],
            p[2] - self.pose.pos[2],
        ];
        let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        let [right, up, out] = self.pose.basis;
        let z = dot(out);
        let wide = radius * 1.1 + CACHE_CELL;
        if z + wide < 0.08 {
            return false;
        }
        let reach = (VIEW * 1.08 + self.slack) * z.max(0.0) + wide;
        dot(right).hypot(dot(up)) < reach
    }

    /// How far a creature or a party at `p` has swum into the window: one well
    /// inside it, easing to nothing at its rim and at the near plane. A school,
    /// a diver or a whale is placed only when this is positive, so the ones
    /// that wander in fade over the brass instead of appearing in the middle
    /// of the glass. The band is wider for a large subject, which covers more
    /// of the rim as it crosses.
    pub fn reveal(&self, p: [f32; 3], radius: f32) -> f32 {
        let d = [
            p[0] - self.pose.pos[0],
            p[1] - self.pose.pos[1],
            p[2] - self.pose.pos[2],
        ];
        let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
        let [right, up, out] = self.pose.basis;
        let z = dot(out);
        if z + radius < 0.08 {
            return 0.0;
        }
        // A sphere touches the cone of view up to a little past its radius,
        // as the cone's side slants.
        let reach = VIEW * z.max(0.0) * 1.08 + radius * 1.1;
        let lateral = dot(right).hypot(dot(up));
        let band = 0.10 + 0.45 * radius;
        smooth(reach, (reach - band).max(0.0), lateral) * smooth(0.08, 0.16, z + radius)
    }
}

/// The least water between a bubble and the salon window: any nearer and a
/// bubble a few centimetres across is a ring across the glass. Bubbles fade
/// out over [`BUBBLE_FADE`] units before they reach it, so one that drifts in
/// is gone before it is huge.
const BUBBLE_CLEAR: f32 = 1.2;
const BUBBLE_FADE: f32 = 0.1;

/// How clear of the window's near water a bubble anchored at `p` is: nothing
/// within [`BUBBLE_CLEAR`], easing in to one by [`BUBBLE_CLEAR`] +
/// [`BUBBLE_FADE`].
fn window_near(w: &World, p: [f32; 3]) -> f32 {
    let d = [
        p[0] - w.pose.pos[0],
        p[1] - w.pose.pos[1],
        p[2] - w.pose.pos[2],
    ];
    let distance = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
    smooth(BUBBLE_CLEAR, BUBBLE_CLEAR + BUBBLE_FADE, distance)
}

/// How much of a bubble at `p` is inside the window: one well within it, easing
/// to nothing just before the cone's side or the near water, so a bubble that
/// drifts across the brass fades rather than blinking out of the sprite list.
fn window_fade(w: &World, p: [f32; 3], radius: f32) -> f32 {
    let d = [
        p[0] - w.pose.pos[0],
        p[1] - w.pose.pos[1],
        p[2] - w.pose.pos[2],
    ];
    let dot = |a: [f32; 3]| a[0] * d[0] + a[1] * d[1] + a[2] * d[2];
    let [right, up, out] = w.pose.basis;
    let z = dot(out);
    if z + radius < 0.08 {
        return 0.0;
    }
    let reach = VIEW * z.max(0.0) * 1.08 + radius * 1.1;
    let lateral = dot(right).hypot(dot(up));
    smooth(reach, 0.9 * reach, lateral) * window_near(w, p)
}

/// The lengths of a frame's lists at one moment.
struct Mark {
    solid: Vec<usize>,
    glass: Vec<usize>,
    sprites: usize,
    lights: usize,
}

/// Everything to draw in one frame.
pub struct Frame {
    /// Opaque instances per model.
    pub solid: Vec<Vec<Instance>>,
    /// Translucent instances per model.
    pub glass: Vec<Vec<Instance>>,
    pub sprites: Vec<Sprite>,
    /// Shafts of sunlight: `place` is the top, `tint` the colour, `stretch`
    /// the run down to the foot.
    pub shafts: Vec<Sprite>,
    /// Searchlight beams: `place` is the lamp and the beam's radius there,
    /// `tint` the colour and its radius at the far end, `stretch` the run.
    pub beams: Vec<Sprite>,
    /// Far coral as cut-out cards: `place` is the foot and the height,
    /// `tint` the colour and a seed for the shape, `stretch` the half width
    /// and the form.
    pub impostors: Vec<Sprite>,
    pub lights: Vec<Light>,
    /// The tunnel through the lava cave's massif, in world space for the
    /// stretch in view. It depends only on the seabed and that stretch, so it
    /// is kept from frame to frame and rebuilt only when the stretch moves on
    /// by a ring, every few seconds rather than every frame.
    pub tunnel: Vec<super::mesh::Vertex>,
    pub tunnel_indices: Vec<u32>,
    /// Counts the tunnel's rebuilds, so whoever keeps a copy of it on the GPU
    /// can tell when theirs is out of date.
    pub tunnel_version: u64,
    /// The stretch of rings the tunnel was built for, and the seabed's seed.
    tunnel_span: Option<(i32, i32, u32)>,
    /// The passages off the tunnel, built with it.
    passages: Vec<Passage>,
    /// How far what is being placed has faded in, as its biome comes and goes.
    fade: f32,
    /// The texture for models that have none of their own, while placing one
    /// kind of thing: marble for the ruins, planking for the wrecks.
    skin: Option<[f32; 4]>,
    /// The submarine encounter under way, decided when it is first seen.
    encounter: Option<Encounter>,
}

impl Frame {
    pub fn new() -> Self {
        Self {
            solid: (0..Model::ALL.len()).map(|_| Vec::new()).collect(),
            glass: (0..Model::ALL.len()).map(|_| Vec::new()).collect(),
            sprites: Vec::new(),
            shafts: Vec::new(),
            beams: Vec::new(),
            impostors: Vec::new(),
            lights: Vec::new(),
            tunnel: Vec::new(),
            tunnel_indices: Vec::new(),
            tunnel_version: 0,
            tunnel_span: None,
            passages: Vec::new(),
            fade: 1.0,
            skin: None,
            encounter: None,
        }
    }

    pub fn clear(&mut self) {
        for list in self.solid.iter_mut().chain(self.glass.iter_mut()) {
            list.clear();
        }
        self.sprites.clear();
        self.shafts.clear();
        self.beams.clear();
        self.impostors.clear();
        self.lights.clear();
    }

    /// Build the tunnel afresh next time: the seabed has changed.
    pub fn forget_tunnel(&mut self) {
        self.tunnel_span = None;
    }

    fn instance(&self, model: Model, rows: [[f32; 4]; 3], colour: [f32; 3]) -> Instance {
        // The fourth surface component is unused by the shader; it carries
        // the instance's longest axis so the per-frame LOD pick need not
        // recompute three column norms and a square root for every instance.
        let mut surface = own_skin(model)
            .or(self.skin)
            .unwrap_or([0.0, 1.0, 0.0, 0.0]);
        surface[3] = axes(rows);
        Instance {
            rows,
            colour: [colour[0], colour[1], colour[2], 0.0],
            extra: [0.0, 0.0, self.fade, 0.0],
            surface,
            ..Default::default()
        }
    }

    fn put(&mut self, model: Model, rows: [[f32; 4]; 3], colour: [f32; 3]) -> &mut Instance {
        let instance = self.instance(model, rows, colour);
        let list = &mut self.solid[model.index()];
        list.push(instance);
        list.last_mut().unwrap()
    }

    fn glass(&mut self, model: Model, rows: [[f32; 4]; 3], colour: [f32; 3]) -> &mut Instance {
        let instance = self.instance(model, rows, colour);
        let list = &mut self.glass[model.index()];
        list.push(instance);
        list.last_mut().unwrap()
    }

    /// Where the lists stand now, to turn what is added after.
    fn mark(&self) -> Mark {
        Mark {
            solid: self.solid.iter().map(Vec::len).collect(),
            glass: self.glass.iter().map(Vec::len).collect(),
            sprites: self.sprites.len(),
            lights: self.lights.len(),
        }
    }

    /// Grow everything placed since `mark` by `scale` about `from`, and set
    /// it down with `from` at `to`: models, sprites and lights, their reach
    /// with them.
    fn grow_since(&mut self, mark: &Mark, from: [f32; 3], to: [f32; 3], scale: f32) {
        let moved = |p: [f32; 3]| [0, 1, 2].map(|i| to[i] + (p[i] - from[i]) * scale);
        for (lists, marks) in [
            (&mut self.solid, &mark.solid),
            (&mut self.glass, &mark.glass),
        ] {
            for (list, &first) in lists.iter_mut().zip(marks) {
                for instance in &mut list[first..] {
                    let m = &mut instance.rows;
                    let t = moved([m[0][3], m[1][3], m[2][3]]);
                    for (i, row) in m.iter_mut().enumerate() {
                        for v in &mut row[..3] {
                            *v *= scale;
                        }
                        row[3] = t[i];
                    }
                }
            }
        }
        for sprite in &mut self.sprites[mark.sprites..] {
            let p = moved([sprite.place[0], sprite.place[1], sprite.place[2]]);
            sprite.place[..3].copy_from_slice(&p);
            sprite.place[3] *= scale;
        }
        for light in &mut self.lights[mark.lights..] {
            light.pos = moved(light.pos);
            light.range *= scale;
        }
    }

    /// Turn everything placed since `mark` by `yaw` about `pivot`.
    fn turn_since(&mut self, mark: &Mark, pivot: [f32; 3], yaw: f32) {
        let (sy, cy) = yaw.sin_cos();
        let r = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
        let spin = |p: [f32; 3]| {
            let d = [p[0] - pivot[0], p[1] - pivot[1], p[2] - pivot[2]];
            [0, 1, 2].map(|i| pivot[i] + r[i][0] * d[0] + r[i][1] * d[1] + r[i][2] * d[2])
        };
        for (list, &from) in self.solid.iter_mut().zip(&mark.solid) {
            for instance in &mut list[from..] {
                let m = instance.rows;
                let t = spin([m[0][3], m[1][3], m[2][3]]);
                for (i, row) in instance.rows.iter_mut().enumerate() {
                    *row = [0, 1, 2, 3].map(|j| {
                        if j == 3 {
                            t[i]
                        } else {
                            r[i][0] * m[0][j] + r[i][1] * m[1][j] + r[i][2] * m[2][j]
                        }
                    });
                }
            }
        }
        for sprite in &mut self.sprites[mark.sprites..] {
            let p = spin([sprite.place[0], sprite.place[1], sprite.place[2]]);
            sprite.place[..3].copy_from_slice(&p);
        }
        for light in &mut self.lights[mark.lights..] {
            light.pos = spin(light.pos);
        }
    }

    /// Start placing one kind of thing, with `skin` for its plain parts.
    fn style(&mut self, skin: Option<[f32; 4]>) {
        self.fade = 1.0;
        self.skin = skin;
    }

    fn sprite(&mut self, p: [f32; 3], size: f32, colour: [f32; 3], kind: f32) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], kind],
            stretch: [1.0, 0.0, 0.0, 0.0],
        });
    }

    /// Sediment spilling from a lip: the shader owns the fall and the fade,
    /// so the anchor is the lip and the placer stays cacheable.
    fn mote_sprite(&mut self, p: [f32; 3], size: f32, colour: [f32; 3], n: f32, h: f32, fall: f32) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], MOTE],
            stretch: [1.0, n, h, fall],
        });
    }

    /// A glow whose pulse is the shader's: `mode` 0 is a sea-pen blink
    /// (`max(sin(rate*t + phase), 0)`), 1 the crater flicker
    /// (`0.85 + 0.15 sin(2.3t + phase) sin(1.3t)`).
    fn glow_sprite(
        &mut self,
        p: [f32; 3],
        size: f32,
        colour: [f32; 3],
        phase: f32,
        rate: f32,
        mode: f32,
    ) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], GLOW],
            stretch: [1.0, phase, rate, mode],
        });
    }

    /// An ember anchored at its source: the shader owns the rise and the fade
    /// from the camera clock, so the placer emits a camera-static anchor.
    fn ember_sprite(
        &mut self,
        p: [f32; 3],
        size: f32,
        colour: [f32; 3],
        phase: f32,
        rate: f32,
        rise: f32,
    ) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], EMBER],
            stretch: [1.0, phase, rate, rise],
        });
    }

    /// A bubble anchored where it was shed: the shader owns its age from the
    /// camera clock (`phase`/`life`), so the placer emits a camera-static
    /// anchor and the placer can enter the cache.
    fn bubble_sprite(
        &mut self,
        p: [f32; 3],
        size: f32,
        colour: [f32; 3],
        phase: f32,
        life: f32,
        wave: f32,
    ) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], BUBBLE],
            stretch: [1.0, phase, life, wave],
        });
    }

    fn streak(&mut self, p: [f32; 3], size: f32, colour: [f32; 3], stretch: f32) {
        self.sprites.push(Sprite {
            place: [p[0], p[1], p[2], size],
            tint: [colour[0], colour[1], colour[2], GLOW],
            stretch: [stretch, 0.0, 0.0, 0.0],
        });
    }

    fn light(&mut self, pos: [f32; 3], range: f32, colour: [f32; 3]) {
        self.lights.push(Light {
            pos,
            range,
            colour,
            phase: 0.0,
        });
    }

    /// A light whose brightness flickers: the shader does the flicker from the
    /// camera clock and this phase, so a cached placement keeps breathing.
    fn light_flicker(&mut self, pos: [f32; 3], range: f32, colour: [f32; 3], phase: f32) {
        self.lights.push(Light {
            pos,
            range,
            colour,
            phase,
        });
    }
}

trait Tweak {
    fn grounded(&mut self) -> &mut Self;
    fn moving(&mut self, kind: f32, phase: f32, amp: f32, freq: f32) -> &mut Self;
    fn material(&mut self, m: f32) -> &mut Self;
    fn glowing(&mut self, glow: f32) -> &mut Self;
    /// Textured in world space: for things that stay put.
    fn textured(&mut self, surface: Surface, repeats: f32) -> &mut Self;
    /// A glass model's pattern, which `fs_glass` reads where a solid model
    /// keeps its texture: glass is never textured. The fourth component,
    /// the instance's longest axis, is kept.
    fn patterned(&mut self, pattern: [f32; 3]) -> &mut Self;
}

impl Tweak for Instance {
    fn grounded(&mut self) -> &mut Self {
        self.extra[1] = 1.0;
        self
    }
    fn moving(&mut self, kind: f32, phase: f32, amp: f32, freq: f32) -> &mut Self {
        self.anim = [kind, phase, amp, freq];
        self
    }
    fn material(&mut self, m: f32) -> &mut Self {
        self.extra[0] = m;
        self
    }
    fn glowing(&mut self, glow: f32) -> &mut Self {
        self.colour[3] = glow;
        self
    }
    fn textured(&mut self, surface: Surface, repeats: f32) -> &mut Self {
        self.surface = [surface.layer(), repeats, 0.0, 0.0];
        self
    }
    fn patterned(&mut self, pattern: [f32; 3]) -> &mut Self {
        self.surface = [pattern[0], pattern[1], pattern[2], self.surface[3]];
        self
    }
}

/// Texture in world space: for things that stay put, so the pattern has the
/// same scale everywhere.
fn fixed(surface: Surface, repeats: f32) -> Option<[f32; 4]> {
    Some([surface.layer(), repeats, 0.0, 0.0])
}

/// Texture in a model's own space: for things that move and bend.
fn carried(surface: Surface, repeats: f32) -> Option<[f32; 4]> {
    Some([surface.layer(), repeats, 1.0, 0.0])
}

/// The texture a model always wears; plain shapes take their context's.
fn own_skin(model: Model) -> Option<[f32; 4]> {
    match model {
        Model::Rock0 | Model::Rock1 | Model::Rock2 => fixed(Surface::Rock, 3.0),
        Model::Kelp => carried(Surface::Kelp, 3.0),
        Model::Fish => carried(Surface::Scales, 1.0),
        Model::Shark => carried(Surface::Skin, 2.0),
        Model::Whale => carried(Surface::Skin, 4.0),
        Model::Dolphin | Model::Narwhal => carried(Surface::Skin, 5.0),
        Model::Mantle | Model::Tube => carried(Surface::Skin, 4.0),
        Model::Shaft | Model::Capital | Model::Break | Model::Prism => fixed(Surface::Marble, 3.0),
        Model::Hull => carried(Surface::Wood, 3.0),
        Model::Sail => carried(Surface::Canvas, 1.5),
        Model::Fan | Model::Star | Model::Shell | Model::Sponge => carried(Surface::Coral, 2.5),
        Model::Keel => fixed(Surface::Ice, 1.5),
        Model::Hex => fixed(Surface::Basalt, 2.5),
        Model::Capsule => carried(Surface::Canvas, 1.0),
        Model::Diver | Model::DiverLimb => carried(Surface::Canvas, 2.0),
        Model::DiverHelmet => carried(Surface::Copper, 2.0),
        Model::DiverBoot => carried(Surface::Plates, 6.0),
        Model::Manta => carried(Surface::Skin, 3.0),
        Model::Turtle => carried(Surface::Skin, 2.0),
        Model::Coral | Model::CoralFinger | Model::CoralBush => carried(Surface::Coral, 2.0),
        Model::CoralBrain => carried(Surface::Brain, 1.0),
        Model::Anemone | Model::SeaWhip => carried(Surface::Skin, 3.0),
        Model::CoralTable => carried(Surface::Coral, 3.0),
        Model::Sargassum => carried(Surface::Kelp, 2.0),
        Model::Vent => fixed(Surface::Basalt, 2.5),
        Model::Tree => carried(Surface::Kelp, 2.0),
        Model::Submarine => carried(Surface::Plates, 5.0),
        Model::Screw => carried(Surface::Copper, 1.0),
        Model::Tower
        | Model::Rotunda
        | Model::Dome
        | Model::Pipework
        | Model::Ziggurat
        | Model::Factory => fixed(Surface::Plates, 3.0),
        Model::Walkway | Model::Sign | Model::Arcade | Model::Theatre => {
            fixed(Surface::Copper, 3.0)
        }
        _ => None,
    }
}

/// How far something has faded in, given its biome's share and the share it
/// needs: things grow out of the seabed and melt away rather than pop.
fn fade(share: f32, needs: f32) -> f32 {
    smooth(needs, needs + 0.2, share)
}

/// The longest of an instance's axes, as the model is stretched.
fn axes(rows: [[f32; 4]; 3]) -> f32 {
    (0..3)
        .map(|c| (0..3).map(|r| rows[r][c] * rows[r][c]).sum::<f32>())
        .fold(0.0, f32::max)
        .sqrt()
}

/// A transform: rotate by yaw (about y), pitch (about z) and roll (about x),
/// in that order outermost first, scale, and move to `pos`.
pub fn place(pos: [f32; 3], yaw: f32, pitch: f32, roll: f32, scale: [f32; 3]) -> [[f32; 4]; 3] {
    let (sy, cy) = yaw.sin_cos();
    let (sp, cp) = pitch.sin_cos();
    let (sr, cr) = roll.sin_cos();
    let ry = [[cy, 0.0, sy], [0.0, 1.0, 0.0], [-sy, 0.0, cy]];
    let rz = [[cp, -sp, 0.0], [sp, cp, 0.0], [0.0, 0.0, 1.0]];
    let rx = [[1.0, 0.0, 0.0], [0.0, cr, -sr], [0.0, sr, cr]];
    let m = mul(mul(ry, rz), rx);
    let mut rows = [[0.0; 4]; 3];
    for r in 0..3 {
        for c in 0..3 {
            rows[r][c] = m[r][c] * scale[c];
        }
        rows[r][3] = pos[r];
    }
    rows
}

fn mul(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut m = [[0.0; 3]; 3];
    for (r, row) in m.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| a[r][k] * b[k][c]).sum();
        }
    }
    m
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn norm(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-6);
    [v[0] / l, v[1] / l, v[2] / l]
}

fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}

/// A model whose y axis runs from `a` to `b`, `radius` thick: limbs, masts,
/// trunks, tentacles.
pub fn between(a: [f32; 3], b: [f32; 3], radius: f32) -> [[f32; 4]; 3] {
    let d = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let length = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt().max(1e-5);
    let y = [d[0] / length, d[1] / length, d[2] / length];
    let reference = if y[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let x = norm(cross(y, reference));
    let z = cross(x, y);
    let mut rows = [[0.0; 4]; 3];
    for r in 0..3 {
        rows[r] = [x[r] * radius, y[r] * length, z[r] * radius, a[r]];
    }
    rows
}

/// The cells of a grid of `size` that can be in view between `near` and
/// `far`, as (column, row, x, z) with the point jittered within its cell.
/// The view's footprint on the seabed between `near` and `far`, boxed in
/// world x and z: whichever way the boat is turned.
fn footprint(w: &World, near: f32, far: f32) -> ([f32; 4], [f32; 2], [f32; 2]) {
    let [right, _, out] = w.pose.basis;
    let flat = |v: [f32; 3]| {
        let l = (v[0] * v[0] + v[2] * v[2]).sqrt().max(1e-4);
        [v[0] / l, v[2] / l]
    };
    let (o, r) = (flat(out), flat(right));
    let c = w.pose.pos;
    let spread = VIEW * 1.3;
    let mut bounds = [f32::MAX, f32::MIN, f32::MAX, f32::MIN];
    for d in [near, far] {
        for side in [-1.0, 1.0] {
            let x = c[0] + o[0] * d + r[0] * side * d * spread;
            let z = c[2] + o[1] * d + r[1] * side * d * spread;
            bounds = [
                bounds[0].min(x),
                bounds[1].max(x),
                bounds[2].min(z),
                bounds[3].max(z),
            ];
        }
    }
    (bounds, o, r)
}

/// Where in its cell of `size` the thing of cell (ix, iz) stands.
fn cell_point(ix: i32, iz: i32, size: f32, salt: u32) -> (f32, f32) {
    (
        (ix as f32 + 0.5 + (hash(ix, iz, salt) - 0.5) * 0.8) * size,
        (iz as f32 + 0.5 + (hash(ix, iz, salt + 1) - 0.5) * 0.8) * size,
    )
}

/// One visible cell: its column, row and world position.
type Cell = (i32, i32, f32, f32);

thread_local! {
    /// The visible-cell lists, one vector per caller in flight; the build calls
    /// `cells` nineteen times a frame and the lists are short.
    static CELLS: std::cell::RefCell<Vec<Vec<Cell>>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// A cell list borrowed from the pool. Dropping it returns the vector, so the
/// next frame reuses the capacity instead of allocating nineteen times.
struct Cells(Vec<Cell>);

impl std::ops::Deref for Cells {
    type Target = Vec<(i32, i32, f32, f32)>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl Drop for Cells {
    fn drop(&mut self) {
        let list = std::mem::take(&mut self.0);
        CELLS.with(|pool| pool.borrow_mut().push(list));
    }
}

impl<'a> IntoIterator for &'a Cells {
    type Item = &'a (i32, i32, f32, f32);
    type IntoIter = std::slice::Iter<'a, (i32, i32, f32, f32)>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.iter()
    }
}

/// The cells of a grid of `size` that can be in view between `near` and
/// `far` from the window, as (column, row, x, z) with the point jittered
/// within its cell. The grid is fixed in the world, so the same rock is
/// always in the same place however the boat turns.
///
/// `near` is reckoned abeam, square to the boat's way rather than along the
/// window's look: the helmsman's swing turns the window towards things
/// ahead or astern, and they come at it on the diagonal, but their distance
/// abeam holds, so nothing kept off the glass appears or vanishes as the
/// boat swings. The footprint is taken from half as near, to hold them.
fn cells(w: &World, size: f32, near: f32, far: f32, salt: u32) -> Cells {
    let far = far + w.margin(far);
    let ([x0, x1, z0, z1], o, r) = footprint(w, near * 0.5, far);
    let c = w.pose.pos;
    let abeam = w.pose.abeam;
    let mut out = CELLS
        .with(|pool| pool.borrow_mut().pop())
        .unwrap_or_default();
    out.clear();
    for iz in ((z0 - size) / size).floor() as i32..=((z1 + size) / size).ceil() as i32 {
        for ix in ((x0 - size) / size).floor() as i32..=((x1 + size) / size).ceil() as i32 {
            let (x, z) = cell_point(ix, iz, size, salt);
            let (dx, dz) = (x - c[0], z - c[2]);
            let depth = dx * o[0] + dz * o[1];
            if depth < 0.0 || dx * abeam[0] + dz * abeam[1] < near || depth > far {
                continue;
            }
            if (dx * r[0] + dz * r[1]).abs() > depth * VIEW * 1.3 + size + 0.6 {
                continue;
            }
            out.push((ix, iz, x, z));
        }
    }
    Cells(out)
}

/// The static decor's placements, rebuilt only when the camera crosses a
/// 0.5-unit cell: the smallest cell the static placers use. The frame copies
/// the cached visible set, which is what the placers culled for the cell.
pub struct StaticCache {
    frame: Frame,
    /// Where the window was and which way it looked when the cache was
    /// built: a turn brings new things into view before the boat has moved
    /// far, and the cached set must follow.
    built: Option<([f32; 3], [f32; 3])>,
    seed: f32,
    /// Placed for the window exactly, with no margin: a test's reference
    /// for what should be in view.
    #[cfg(test)]
    exact: bool,
}

impl Default for StaticCache {
    fn default() -> Self {
        Self {
            frame: Frame::new(),
            built: None,
            seed: f32::NAN,
            #[cfg(test)]
            exact: false,
        }
    }
}

/// How far the window moves, and how far round it turns in radians (its
/// heading and pitch together; roll does not move the round window's
/// cone), before the static cache is built again.
const CACHE_CELL: f32 = 0.2;
const CACHE_TURN: f32 = 0.04;

/// Build the frame.
/// One kind of thing to place in the frame.
type Placer = fn(&World, &mut Frame);

pub fn build(world: &World, frame: &mut Frame, cache: &mut StaticCache) {
    #[cfg(test)]
    let begun = std::time::Instant::now();
    frame.clear();
    let wood = carried(Surface::Wood, 1.5);
    let marble = fixed(Surface::Marble, 3.0);
    let rock = fixed(Surface::Rock, 3.0);
    let basalt = fixed(Surface::Basalt, 3.0);
    let algae = carried(Surface::Kelp, 2.0);
    let copper = carried(Surface::Copper, 1.0);
    let skin = carried(Surface::Skin, 3.0);
    // The placers whose every placement-time and sprite-time motion is now
    // the shader's: the shell hinge, the tube worms' sway, the crinoid wave,
    // the penguin flap, the narwhal body, the jellyfish's swimming and the
    // four sprite kinds.
    let static_steps: [(Placer, Option<[f32; 4]>); 12] = [
        (rocks, None),
        (gardens, None),
        (reef, None),
        (wrecks, wood),
        (trench, rock),
        (ruins, marble),
        (cave, rock),
        (polar, None),
        (kelp, None),
        (forest, algae),
        (drifters, None),
        (city, None),
    ];
    let steps: [(Placer, Option<[f32; 4]>); 13] = [
        (planes, None),
        (city_beams, None),
        (tunnel, None),
        (vents, basalt),
        (ice, None),
        (divers, copper),
        (shoals, None),
        (great_swimmers, None),
        (gliders, None),
        (squid, skin),
        (submarines, None),
        (shafts, None),
        (particles, None),
    ];
    #[cfg(test)]
    const STATIC_NAMES: [&str; 12] = [
        "rocks", "gardens", "reef", "wrecks", "trench", "ruins", "cave", "polar", "kelp", "forest",
        "drifters", "city",
    ];
    #[cfg(test)]
    const STEP_NAMES: [&str; 13] = [
        "planes",
        "city_beams",
        "tunnel",
        "vents",
        "ice",
        "divers",
        "shoals",
        "great_swimmers",
        "gliders",
        "squid",
        "submarines",
        "shafts",
        "particles",
    ];
    let seed = world.seabed.seed;
    let (at, out) = (world.pose.pos, world.pose.basis[2]);
    let stale = !cache.seed.eq(&seed)
        || cache.built.is_none_or(|(was, looked)| {
            let moved = (at[0] - was[0]).hypot(at[2] - was[2]);
            let turned = out[0] * looked[0] + out[1] * looked[1] + out[2] * looked[2];
            moved > CACHE_CELL || turned < CACHE_TURN.cos()
        });
    #[cfg(test)]
    let show_steps = std::env::var_os("DIVE3D_BUILD_STEPS").is_some();
    if stale {
        cache.frame.clear();
        // The cache stands until the window has moved a cell or turned
        // [`CACHE_TURN`]: placed for a cone that much wider, what comes into
        // view in between is already there rather than appearing at the rim.
        #[cfg(test)]
        let slack = if cache.exact { 0.0 } else { CACHE_TURN };
        #[cfg(not(test))]
        let slack = CACHE_TURN;
        let wide = World { slack, ..*world };
        let world = &wide;
        #[cfg(test)]
        for (i, (step, skin)) in static_steps.into_iter().enumerate() {
            cache.frame.style(skin);
            let started = std::time::Instant::now();
            step(world, &mut cache.frame);
            if show_steps {
                eprintln!(
                    "dive3d build step: static {} {:.3}",
                    STATIC_NAMES[i],
                    started.elapsed().as_secs_f64() * 1e3
                );
            }
        }
        #[cfg(not(test))]
        for (step, skin) in static_steps {
            cache.frame.style(skin);
            step(world, &mut cache.frame);
        }
        for list in cache
            .frame
            .solid
            .iter_mut()
            .chain(cache.frame.glass.iter_mut())
        {
            for instance in list.iter_mut() {
                let r = instance.rows;
                if instance.extra[1] > 0.5 {
                    let (x, z) = (r[0][3], r[2][3]);
                    instance.extra[3] = world.ground(x, z);
                    let c = floor_colour(world.seabed, &world.seabed.biome(x, z), x, z);
                    instance.floor = [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9, 0.0];
                }
            }
        }
        cache.built = Some((at, out));
        cache.seed = seed;
    }
    for (m, src) in cache.frame.solid.iter().enumerate() {
        frame.solid[m].extend_from_slice(src);
    }
    for (m, src) in cache.frame.glass.iter().enumerate() {
        frame.glass[m].extend_from_slice(src);
    }
    frame.sprites.extend_from_slice(&cache.frame.sprites);
    frame.shafts.extend_from_slice(&cache.frame.shafts);
    frame.beams.extend_from_slice(&cache.frame.beams);
    frame.impostors.extend_from_slice(&cache.frame.impostors);
    frame.lights.extend_from_slice(&cache.frame.lights);
    #[cfg(test)]
    for (i, (step, skin)) in steps.into_iter().enumerate() {
        frame.style(skin);
        let started = std::time::Instant::now();
        step(world, frame);
        if show_steps {
            eprintln!(
                "dive3d build step: {} {:.3}",
                STEP_NAMES[i],
                started.elapsed().as_secs_f64() * 1e3
            );
        }
    }
    #[cfg(not(test))]
    for (step, skin) in steps {
        frame.style(skin);
        step(world, frame);
    }
    // The vertex shader beds a model into the sand by comparing each vertex
    // with the ground under it. Once per instance is enough: a rock, a tree or
    // a diver stands on the height at its foot, and this keeps the texture
    // lookups and noise out of every vertex of every model. The sand's colour
    // comes from the same place, so no pixel has to look up which biome it is
    // in to blend the foot in.
    #[cfg(test)]
    let placed = begun.elapsed();
    for list in frame.solid.iter_mut().chain(frame.glass.iter_mut()) {
        for instance in list.iter_mut() {
            let r = instance.rows;
            if instance.extra[1] > 0.5 {
                let (x, z) = (r[0][3], r[2][3]);
                instance.extra[3] = world.ground(x, z);
                let c = floor_colour(world.seabed, &world.seabed.biome(x, z), x, z);
                instance.floor = [c[0] * 0.9, c[1] * 0.9, c[2] * 0.9, 0.0];
            }
        }
    }
    #[cfg(test)]
    if std::env::var_os("DIVE3D_STATS").is_some() {
        let total = begun.elapsed();
        eprintln!(
            "dive3d build: placers {:.3} ms, grounding {:.3} ms",
            placed.as_secs_f64() * 1e3,
            (total - placed).as_secs_f64() * 1e3
        );
    }
}

/// The surface, the cave's vault and the ice: great planes that follow the
/// camera; the shader fades them out where they do not belong. The surface is
/// the sea's, at the same height wherever the boat is.
fn planes(w: &World, f: &mut Frame) {
    // The water's underside is part of the background pass: it is a ceiling
    // of water with caustics and swell, drawn from the view ray itself, so it
    // has no edge to seam across the window. The one plane left here is the
    // polar ice sheet, whose leads must open on to the real sky.
    let c = w.pose.pos;
    // The ice bake only covers 64 units around the camera, so its plane keeps
    // its old reach; stretching it would show the bake's edge.
    let ice_big = [40.0, 1.0, 40.0];
    // The ice sheet is only the arctic's; the fragment shader works its
    // coverage out per pixel and discards the rest, which over a 40 x 40
    // plane is mostly discarded. Cut the plane to the stretch where the
    // arctic is present and skip it entirely otherwise; the shading inside
    // is world-space, so the picture does not change.
    let near = (0..=4).any(|i| w.share(c[0] + i as f32 * 3.0 - 6.0, c[2], ARCTIC) > 0.0);
    if near && w.ice() > w.eye() {
        let (mut x0, mut x1) = (f32::MAX, f32::MIN);
        let mut x = c[0] - ice_big[0] * 0.5;
        while x <= c[0] + ice_big[0] * 0.5 {
            if w.share(x, corridor(x), ARCTIC) > 0.30 {
                x0 = x0.min(x);
                x1 = x1.max(x);
            }
            x += 0.5;
        }
        if x1 > x0 {
            // The pixels' z shifts the voyage coordinate by up to about two
            // units, so the cut keeps a margin past where the ice fades out.
            let (x0, x1) = (x0 - 3.0, x1 + 3.0);
            let width = (x1 - x0).min(ice_big[0]);
            f.put(
                Model::Plane,
                place(
                    [(x0 + x1) * 0.5, w.ice(), c[2]],
                    0.0,
                    0.0,
                    0.0,
                    [width, 1.0, ice_big[2]],
                ),
                [1.0; 3],
            )
            .material(material::ICE)
            .textured(Surface::Ice, 0.8);
        }
    }
}

fn rocks(w: &World, f: &mut Frame) {
    for &(ix, iz, x, z) in &cells(w, 0.55, 0.6, 14.0, 10) {
        // Fewer on the cave's floor, which the window sees whole across its
        // halls: a scatter of pillow lava, not a field of it.
        let sparse = 0.3 * w.share(x, z, CAVE);
        if hash(ix, iz, 12) < 0.55 + sparse {
            continue;
        }
        // Big boulders lie farther off; right by the glass only stones.
        let size = 0.05 + 0.20 * hash(ix, iz, 13).powf(2.0);
        // Boulders lie off the boat's course; only stones in its way, and
        // only stones in the drowned city's swept streets.
        if size > 0.1 && ((z - corridor(x)).abs() < 0.7 || w.share(x, z, CITY) > 0.3) {
            continue;
        }
        let y = w.ground(x, z);
        let p = [x, y + size * 0.25, z];
        if !w.visible(p, size * 1.5) {
            continue;
        }
        let b = w.biome(x, z);
        // The reef is built of coral, not strewn with boulders: most of its
        // stones are left out, and those left are dead coral rock, pale and
        // crusted, rather than grey basalt.
        let reef = b.weight(CORAL) as f32;
        if hash(ix, iz, 17) < reef * 0.9 {
            continue;
        }
        let molten = b.weight(CAVE) as f32;
        let tone = World::by_biome(
            &b,
            [
                0.32, 0.24, 0.22, 0.30, 0.12, 0.14, 0.26, 0.27, 0.34, 0.30, 0.16, 0.55, 0.23, 0.24,
            ],
        );
        let model =
            [Model::Rock0, Model::Rock1, Model::Rock2][(hash(ix, iz, 14) * 3.0) as usize % 3];
        let flat = 0.55 + 0.35 * hash(ix, iz, 15);
        let i = f.put(
            model,
            place(
                p,
                hash(ix, iz, 16) * TAU,
                0.0,
                0.0,
                [size * 1.2, size * flat, size],
            ),
            [
                tone + 0.30 * reef,
                tone * 1.02 + 0.20 * reef,
                tone * 0.98 + 0.12 * reef,
            ],
        );
        i.grounded();
        if reef > 0.5 {
            i.textured(Surface::Coral, 4.0);
        }
        if molten > 0.5 {
            i.textured(Surface::Basalt, 3.0);
        }
        if molten > 0.5 {
            i.material(material::LAVA);
        }
    }
}

/// The grid kelp grows in, and the tallest it grows. It is kept low enough
/// that a sister rising over a bed of it in the shallows still keeps under the
/// surface, and so clears her keel.
const KELP_CELL: f32 = 0.30;
const KELP_TALL: f32 = 1.2;
const KELP_SALT: u32 = 20;

/// Whether kelp grows in cell (ix, iz): its height, and how far it has faded
/// in. The frame that draws it and the sisters that keep clear of it read the
/// same plant here, so what they rise over is what is drawn.
fn kelp_here(w: &World, ix: i32, iz: i32) -> Option<(f32, f32)> {
    let (x, z) = cell_point(ix, iz, KELP_CELL, KELP_SALT);
    let b = w.biome(x, z);
    let here = (b.weight(SHALLOWS) + 0.35 * b.weight(FOREST)) as f32 * 0.7;
    let needs = hash(ix, iz, 21) * 0.9 + 0.1;
    if here < needs || (z - corridor(x)).abs() < 0.35 {
        return None;
    }
    let height = 0.5 + (KELP_TALL - 0.5) * hash(ix, iz, 22);
    // It must not reach the ceiling a sister keeps under: a raised bank in the
    // shallows would otherwise put a blade through her keel.
    let room = (-0.55 - w.ground(x, z)).max(0.0);
    let height = height.min(room);
    if height < 0.2 {
        return None;
    }
    Some((height, fade(here, needs)))
}

fn kelp(w: &World, f: &mut Frame) {
    for &(ix, iz, x, z) in &cells(w, KELP_CELL, 0.8, 15.0, KELP_SALT) {
        let Some((height, fade_in)) = kelp_here(w, ix, iz) else {
            continue;
        };
        f.fade = fade_in;
        let y = w.ground(x, z);
        if !w.visible([x, y + height * 0.5, z], height * 0.6) {
            continue;
        }
        let gold = 0.8 + 0.4 * hash(ix, iz, 23);
        f.put(
            Model::Kelp,
            place(
                [x, y, z],
                hash(ix, iz, 24) * TAU,
                0.0,
                0.0,
                [0.9, height, 0.9],
            ),
            [0.46 * gold, 0.46 * gold, 0.14 * gold],
        )
        .moving(anim::SWAY, hash(ix, iz, 25) * TAU, 0.08, 0.8)
        .grounded();
    }
    // Sargassum hangs in rafts in the upper water, in its own drifting grid.
    // They are the sea's ceiling in the sargasso: the light comes through
    // them green-gold and the tendrils trail over the window. Some float at
    // the surface, some a little deeper, so the raft layer has thickness.
    for &(ix, iz, x, z) in &cells(w, 0.65, 1.0, 13.0, 64) {
        let here = w.share(x, z, SARGASSO);
        let needs = hash(ix, iz, 65) * 0.55 + 0.15;
        if here < needs {
            continue;
        }
        let y = w.surface() - 0.12 - 0.85 * hash(ix, iz, 66).powf(1.5);
        let s = 0.65 + 0.75 * hash(ix, iz, 67);
        if !w.visible([x, y, z], s * 0.8) {
            continue;
        }
        f.fade = fade(here, needs);
        let gold = 0.80 + 0.45 * hash(ix, iz, 68);
        f.put(
            Model::Sargassum,
            place(
                [x, y, z],
                hash(ix, iz, 69) * TAU,
                (hash(ix, iz, 70) - 0.5) * 0.35,
                (hash(ix, iz, 71) - 0.5) * 0.3,
                [s, s * 0.9, s],
            ),
            [1.0 * gold, 0.88 * gold, 0.38 * gold],
        )
        .moving(anim::SWAY, hash(ix, iz, 72) * TAU, 0.06, 0.5)
        .glowing(0.45);
    }
}

/// The submarine forest of Crespo: tall algae branching like trees. Each
/// tree is one model — trunk, forks and clustered canopy — so the improved
/// silhouette costs one instance instead of a dozen.
fn forest(w: &World, f: &mut Frame) {
    // Past fourteen units the forest's haze has hidden all but a twentieth of
    // a tree, so the far trees melt into it from twelve rather than being
    // drawn out to where nothing of them shows.
    let c = w.pose.pos;
    let out = w.pose.basis[2];
    for &(ix, iz, x, z) in &cells(w, 0.75, 1.0, 14.0, 30) {
        if w.share(x, z, FOREST) < hash(ix, iz, 31) * 0.7 + 0.2 {
            continue;
        }
        let depth = (x - c[0]) * out[0] + (z - c[2]) * out[2];
        f.fade = fade(w.share(x, z, FOREST), hash(ix, iz, 31) * 0.7 + 0.2)
            * (1.0 - smooth(12.0, 14.0, depth));
        if (z - corridor(x)).abs() < 1.4 {
            continue;
        }
        let h = 0.9 + 0.8 * hash(ix, iz, 32);
        let y = w.ground(x, z);
        if !w.visible([x, y + h * 0.5, z], h * 0.7) {
            continue;
        }
        let red = [0.62, 0.25, 0.17];
        let olive = [0.46, 0.44, 0.18];
        let t = hash(ix, iz, 33);
        let tint = [
            red[0] + (olive[0] - red[0]) * t,
            red[1] + (olive[1] - red[1]) * t,
            red[2] + (olive[2] - red[2]) * t,
        ];
        f.put(
            Model::Tree,
            place(
                [x, y - 0.02, z],
                hash(ix, iz, 34) * TAU,
                (hash(ix, iz, 35) - 0.5) * 0.10,
                0.0,
                [h * 0.65, h, h * 0.65],
            ),
            tint,
        )
        .moving(anim::SWAY, ix as f32, 0.03, 0.35)
        .grounded();
    }
}

/// The seabed's gardens: sea fans and clams in the shallows and the forest;
/// the coral heads, stars and shells of the reef; the salt spires and brine
/// pools of the flats; glass sponges and glowing sea pens in the bloom; and
/// sponges and starfish on the polar floor.
fn gardens(w: &World, f: &mut Frame) {
    for &(ix, iz, x, z) in &cells(w, 0.6, 0.8, 7.0, 40) {
        let b = w.biome(x, z);
        let green = (b.weight(SHALLOWS) + b.weight(FOREST)) as f32;
        let polar = b.weight(ARCTIC) as f32;
        let coral = b.weight(CORAL) as f32;
        let brine = b.weight(BRINE) as f32;
        let bloom = b.weight(BLOOM) as f32;
        let roll = hash(ix, iz, 41);
        let needs = roll * 0.8 + 0.2;
        let life = green
            .max(polar)
            .max(coral)
            .max(brine * 0.8)
            .max(bloom * 0.8);
        // Most of the grid is turned away; the seabed under it need not be
        // looked up for those.
        if life < needs {
            continue;
        }
        let y = w.ground(x, z);
        if !w.visible([x, y + 0.1, z], 0.3) {
            continue;
        }
        f.fade = fade(life, needs);
        let yaw = hash(ix, iz, 42) * TAU;
        if coral >= needs && coral >= green && coral >= polar && coral >= brine && coral >= bloom {
            coral_garden(
                f,
                w,
                GardenSite {
                    ix,
                    iz,
                    x,
                    y,
                    z,
                    yaw,
                },
            );
        } else if brine >= needs && brine >= green && brine >= polar && brine >= bloom {
            brine_garden(
                f,
                w,
                GardenSite {
                    ix,
                    iz,
                    x,
                    y,
                    z,
                    yaw,
                },
            );
        } else if bloom >= needs && bloom >= green && bloom >= polar {
            bloom_garden(
                f,
                w,
                GardenSite {
                    ix,
                    iz,
                    x,
                    y,
                    z,
                    yaw,
                },
            );
        } else if green > needs {
            if b.weight(FOREST) > 0.5 && hash(ix, iz, 43) < 0.25 {
                // A tridacna gaping slowly, the pearl in its throat. The
                // opening is the shader's hinge (anim::SHELL), not the
                // placement, so the instance can be cached.
                let s = 0.10;
                f.put(
                    Model::Shell,
                    place([x, y + 0.005, z], yaw, PI, 0.0, [s, s, s * 0.8]),
                    [0.55, 0.53, 0.47],
                )
                .grounded();
                f.put(
                    Model::Shell,
                    place([x, y + 0.01, z], yaw, 0.0, 0.0, [s, s, s * 0.8]),
                    [0.55, 0.53, 0.47],
                )
                .moving(anim::SHELL, ix as f32, 0.2, 0.25);
                f.put(
                    Model::Sphere,
                    place([x, y + 0.03, z], 0.0, 0.0, 0.0, [0.018; 3]),
                    [0.9, 0.88, 0.82],
                )
                .glowing(0.35)
                .material(material::COPPER);
            } else {
                let hue = if hash(ix, iz, 44) < 0.5 {
                    [0.85, 0.28, 0.30]
                } else {
                    [0.90, 0.58, 0.20]
                };
                let s = 0.07 + 0.08 * hash(ix, iz, 45);
                f.put(Model::Fan, place([x, y, z], yaw, 0.0, 0.0, [s, s, s]), hue)
                    .moving(anim::SWAY, yaw, 0.15, 0.7)
                    .grounded();
            }
        } else if polar > needs {
            if hash(ix, iz, 46) < 0.5 {
                for k in 0..3 {
                    let a = k as f32 * 2.1 + yaw;
                    let h = 0.06 + 0.07 * hash(ix, iz, 47 + k);
                    f.put(
                        Model::Sponge,
                        place(
                            [x + a.cos() * 0.035, y - 0.005, z + a.sin() * 0.035],
                            a,
                            0.0,
                            0.0,
                            [0.018, h, 0.018],
                        ),
                        [0.80, 0.74, 0.52],
                    )
                    .grounded();
                }
            } else {
                let s = 0.035 + 0.02 * hash(ix, iz, 50);
                f.put(
                    Model::Star,
                    place([x, y + 0.005, z], yaw, 0.0, 0.0, [s, s, s]),
                    [0.85, 0.30, 0.16],
                )
                .grounded();
            }
        }
    }
}

/// The reef's colours: ochre and brown acropora, purple and pink, the olive
/// green and cream of the boulder corals, the blue of staghorn tips and a
/// golden accent, all muted as living tissue is under a few metres of water,
/// rather than the saturated colours of a lit photograph.
const REEF_HUES: [[f32; 3]; 8] = [
    [0.66, 0.50, 0.32],
    [0.50, 0.39, 0.28],
    [0.50, 0.40, 0.56],
    [0.70, 0.50, 0.50],
    [0.46, 0.54, 0.36],
    [0.74, 0.68, 0.54],
    [0.50, 0.56, 0.62],
    [0.72, 0.56, 0.30],
];

/// One colony's colour: a hue of the reef's, a little lighter or darker and
/// warmer or cooler than its neighbours', so a thicket of one species is not
/// a row of identical copies.
fn reef_hue(ix: i32, iz: i32, salt: u32) -> [f32; 3] {
    let hue = REEF_HUES[(hash(ix, iz, salt) * REEF_HUES.len() as f32) as usize % REEF_HUES.len()];
    let light = 0.86 + 0.20 * hash(ix, iz, salt + 1);
    let warm = (hash(ix, iz, salt + 2) - 0.5) * 0.12;
    [
        (hue[0] * light + warm).clamp(0.0, 1.0),
        (hue[1] * light).clamp(0.0, 1.0),
        (hue[2] * light - warm).clamp(0.0, 1.0),
    ]
}

/// A brain coral's colour: the tans, greens and muted mauves of the
/// boulder corals, a little lighter or darker from one to the next.
fn brain_hue(ix: i32, iz: i32, salt: u32) -> [f32; 3] {
    const HUES: [[f32; 3]; 4] = [
        [0.64, 0.56, 0.38],
        [0.48, 0.56, 0.38],
        [0.68, 0.52, 0.38],
        [0.58, 0.48, 0.54],
    ];
    let hue = HUES[(hash(ix, iz, salt) * 4.0) as usize % 4];
    let light = 0.88 + 0.18 * hash(ix, iz, salt + 1);
    hue.map(|c| (c * light).min(1.0))
}

/// The kinds of colony the reef is built of, as [`reef_species`] picks them.
const STAGHORN: usize = 0;
const TABLE: usize = 1;
const BRAIN: usize = 2;
const BUSH: usize = 3;
const FINGER: usize = 4;
const SOFT: usize = 5;

/// The patch of reef (x, z) lies in, a unit or so across: a reef is laid out
/// in stands of one species each, a staghorn thicket here, a terrace of
/// tables there, a field of boulder corals beyond, not a random mix.
fn reef_patch(x: f32, z: f32) -> (i32, i32, f32) {
    let warp = vnoise(x * 0.9 + 11.0, z * 0.9, 75) - 0.5;
    let (u, v) = (x * 0.75 + warp, z * 0.75 - warp);
    let (pi, pj) = (u.floor() as i32, v.floor() as i32);
    (pi, pj, hash(pi, pj, 69))
}

/// Which of the reef's six kinds of colony grows at (x, z): its patch's, but
/// for the odd stray of another kind, rolled by `stray`.
fn reef_species(x: f32, z: f32, stray: f32) -> usize {
    if stray < 0.15 {
        return (stray / 0.15 * 6.0) as usize % 6;
    }
    let (_, _, roll) = reef_patch(x, z);
    // Staghorn and tables are the commonest stands, soft corals the rarest.
    [
        (0.24, STAGHORN),
        (0.42, TABLE),
        (0.58, BRAIN),
        (0.74, BUSH),
        (0.88, FINGER),
        (1.01, SOFT),
    ]
    .iter()
    .find(|(top, _)| roll < *top)
    .map_or(SOFT, |&(_, kind)| kind)
}

/// The colour of a colony at (x, z): one hue to the patch, so a stand of one
/// species reads as one colony grown wide, jittered a little colony by
/// colony, with the odd colony of another hue.
fn colony_hue(x: f32, z: f32, ix: i32, iz: i32, salt: u32) -> [f32; 3] {
    let (pi, pj, _) = reef_patch(x, z);
    let base = if hash(ix, iz, salt + 7) < 0.15 {
        reef_hue(ix, iz, salt)
    } else {
        reef_hue(pi, pj, salt)
    };
    let light = 0.92 + 0.16 * hash(ix, iz, salt + 3);
    base.map(|c| (c * light).min(1.0))
}

/// The reef between the gardens' bommies: one coral to a cell, close enough
/// that the reef's living carpet (the seabed's own colour and texture) is
/// crowded with colonies, but only where coral covers the ground, so the
/// sand gutters stay clean. Each patch is one species ([`reef_species`]):
/// staghorn and bushy branching corals, finger, table and brain corals, and
/// soft anemones and sea whips; on the bommies' tops they grow larger. Near
/// the window each is a model; further off, where its branches can no longer
/// be told apart, it is a cut-out card in [`Frame::impostors`], which costs a
/// quad. The two overlap for a unit either side of the switch, the card
/// dissolving as the window nears it (`fs_impostor`), so a coral grows into
/// its model rather than popping. The anemones and whips are too small to
/// need a card.
fn reef(w: &World, f: &mut Frame) {
    const MODELS: f32 = 4.0;
    /// How far inside [`MODELS`] the cards still stand, dissolving.
    const BAND: f32 = 1.1;
    let c = w.pose.pos;
    let out = w.pose.basis[2];
    let ahead = [c[0] + out[0] * 8.0, c[2] + out[2] * 8.0];
    if w.biome(c[0], c[2]).weight(CORAL) <= 0.0 && w.biome(ahead[0], ahead[1]).weight(CORAL) <= 0.0
    {
        return;
    }
    for &(ix, iz, x, z) in &cells(w, 0.32, 0.6, 10.0, 60) {
        let coral = w.biome(x, z).weight(CORAL) as f32;
        if coral <= 0.0 {
            continue;
        }
        let field = w.reef(x, z);
        let live = coral * reef_cover(field);
        let needs = hash(ix, iz, 61) * 0.7 + 0.3;
        if live < needs {
            continue;
        }
        let mound = reef_mound(field);
        let y = w.ground(x, z);
        let grown = fade(live, needs);
        let depth = (x - c[0]) * out[0] + (z - c[2]) * out[2];
        let species = reef_species(x, z, hash(ix, iz, 63));
        let hue = if species == BRAIN {
            brain_hue(ix, iz, 62)
        } else {
            colony_hue(x, z, ix, iz, 62)
        };
        let yaw = hash(ix, iz, 64) * TAU;
        let s = (0.12 + 0.12 * hash(ix, iz, 65)) * (1.0 + 0.6 * mound);
        let tall_by = 0.85 + 0.3 * hash(ix, iz, 68);
        // Its model, its impostor's form (none for the small ones), its
        // scale, and its height and half width in units of that scale.
        let (model, kind, s, tall, half) = match species {
            STAGHORN => (Model::Coral, Some(0.0), s * 1.1, 0.45, 0.30),
            TABLE => (Model::CoralTable, Some(1.0), s * 0.85, 0.72, 1.05),
            BRAIN => (Model::CoralBrain, Some(3.0), s * 0.6, 0.76, 0.95),
            BUSH => (Model::CoralBush, Some(4.0), s * 1.15, 0.31, 0.20),
            FINGER => (Model::CoralFinger, Some(2.0), s, 0.54, 0.18),
            _ if hash(ix, iz, 66) < 0.5 => (Model::Anemone, None, s * 0.6, 0.49, 0.33),
            _ => (Model::SeaWhip, None, s * 0.9, 1.0, 0.3),
        };
        if depth < MODELS + w.margin(MODELS) {
            if !w.visible([x, y + 0.1, z], 0.25) {
                continue;
            }
            f.fade = grown;
            let tilt = match model {
                Model::CoralTable => (hash(ix, iz, 67) - 0.5) * 0.3,
                Model::CoralBrain | Model::Anemone => 0.0,
                _ => (hash(ix, iz, 67) - 0.5) * 0.2,
            };
            let rows = place([x, y - 0.01, z], yaw, tilt, tilt * 0.5, [s, s * tall_by, s]);
            let instance = f.put(model, rows, hue);
            match model {
                Model::CoralTable | Model::CoralBrain => {}
                Model::Anemone => {
                    instance.moving(anim::SWAY, yaw, 0.05, 0.8);
                }
                Model::SeaWhip => {
                    instance.moving(anim::SWAY, yaw, 0.08, 0.5);
                }
                _ => {
                    instance.moving(anim::SWAY, yaw, 0.015, 0.4);
                }
            }
            instance.glowing(0.26).grounded();
        }
        let Some(kind) = kind else {
            continue;
        };
        if depth > MODELS - BAND - w.margin(MODELS) {
            let height = tall * s * tall_by * grown;
            if height < 0.01 || !w.visible([x, y + height * 0.5, z], height) {
                continue;
            }
            f.impostors.push(Sprite {
                place: [x, y, z, height],
                tint: [hue[0], hue[1], hue[2], hash(ix, iz, 66) * 10.0],
                stretch: [half * s * grown, kind, MODELS - BAND, 0.0],
            });
        }
    }
}

/// One cell of the seabed's garden: where it is, which cell it is, and the
/// bearing its life faces.
#[derive(Clone, Copy)]
struct GardenSite {
    ix: i32,
    iz: i32,
    x: f32,
    y: f32,
    z: f32,
    yaw: f32,
}

/// A bommie's great colony, at reef scale: a staghorn thicket of many heads,
/// a terrace of table corals stepped one above another, massive boulder
/// corals a metre or two across, a stand of bushy or finger coral, or soft
/// corals and sea fans set broadside to the surge. Each is of its patch's
/// species, as the smaller corals about it are, and grows largest on the
/// bommies' tops; over many of them a swarm of small reef fish hovers. Down
/// in the sand gutters there is only the odd star or shell.
fn coral_garden(f: &mut Frame, w: &World, site: GardenSite) {
    let GardenSite {
        ix,
        iz,
        x,
        y,
        z,
        yaw,
    } = site;
    let field = w.reef(x, z);
    if reef_cover(field) < 0.35 {
        if hash(ix, iz, 90) < 0.20 {
            let s = 0.04 + 0.03 * hash(ix, iz, 91);
            f.put(
                Model::Star,
                place([x, y + 0.005, z], yaw, 0.0, 0.0, [s, s, s]),
                [0.92, 0.42, 0.26],
            )
            .grounded();
        } else if hash(ix, iz, 90) < 0.32 {
            let s = 0.09 + 0.05 * hash(ix, iz, 92);
            f.put(
                Model::Shell,
                place([x, y + 0.005, z], yaw, PI, 0.0, [s, s, s * 0.8]),
                [0.90, 0.78, 0.62],
            )
            .grounded();
        }
        return;
    }
    let mound = reef_mound(field);
    let grow = 1.0 + 0.7 * mound;
    let tint = colony_hue(x, z, ix, iz, 84);
    let shade = |k: i32| {
        let light = 0.88 + 0.22 * hash(ix + k, iz, 85);
        tint.map(|c| (c * light).min(1.0))
    };
    match reef_species(x, z, hash(ix, iz, 80)) {
        STAGHORN => {
            // A staghorn thicket: many heads of one colony crowded together.
            let heads = 4 + (hash(ix, iz, 81) * 3.0) as i32;
            for k in 0..heads {
                let a = yaw + k as f32 * 2.4;
                let off = (0.04 + 0.30 * hash(ix + k, iz, 82)) * grow;
                let p = [x + a.cos() * off, y - 0.01, z + a.sin() * off];
                if !w.visible(p, 0.3) {
                    continue;
                }
                let s = (0.24 + 0.14 * hash(ix, iz + k, 83)) * grow;
                f.put(
                    Model::Coral,
                    place(
                        p,
                        yaw + k as f32 * 1.7,
                        (hash(ix, iz + k, 86) - 0.5) * 0.25,
                        (hash(ix, iz + k, 87) - 0.5) * 0.25,
                        [s, s * (0.8 + 0.4 * hash(ix + k, iz, 88)), s],
                    ),
                    shade(k),
                )
                .moving(anim::SWAY, yaw + k as f32, 0.02, 0.4)
                .glowing(0.30)
                .grounded();
            }
        }
        TABLE => {
            // A terrace of table corals, each plate stepped up and out over
            // the one before, as they shoulder each other for the light.
            let plates = 2 + (hash(ix, iz, 120) < 0.5) as i32;
            for k in 0..plates {
                let s = (0.15 + 0.10 * hash(ix, iz + k, 121)) * grow * (1.0 - 0.22 * k as f32);
                let a = yaw + k as f32 * 2.3;
                let reach = 0.16 * grow * k as f32;
                let p = [x + a.cos() * reach, y - 0.01, z + a.sin() * reach];
                if !w.visible(p, s * 1.2) {
                    continue;
                }
                let tilt = (hash(ix, iz + k, 127) - 0.5) * 0.3;
                f.put(
                    Model::CoralTable,
                    place(p, a, tilt, tilt * 0.5, [s, s * (0.8 + 0.6 * k as f32), s]),
                    shade(k),
                )
                .glowing(0.24)
                .grounded();
            }
        }
        BRAIN => {
            // Massive boulder corals, a metre or two across, with a smaller
            // dome beside the first and an anemone in their lee.
            let s = (0.13 + 0.09 * hash(ix, iz, 93)) * grow;
            if w.visible([x, y + 0.05, z], s * 1.5) {
                let domes = 1 + (hash(ix, iz, 96) < 0.5) as i32;
                for k in 0..domes {
                    let a = yaw + k as f32 * 2.4;
                    let r = s * (1.0 - 0.4 * k as f32);
                    let p = [
                        x + a.cos() * s * 1.6 * k as f32,
                        y - 0.02,
                        z + a.sin() * s * 1.6 * k as f32,
                    ];
                    f.put(
                        Model::CoralBrain,
                        place(
                            p,
                            a,
                            0.0,
                            0.0,
                            [r, r * (0.75 + 0.3 * hash(ix + k, iz, 97)), r],
                        ),
                        brain_hue(ix, iz, 91 + k as u32),
                    )
                    .glowing(0.16)
                    .grounded();
                }
                if hash(ix, iz, 98) < 0.6 {
                    let a = yaw - 1.7;
                    let p = [x + a.cos() * s * 1.3, y - 0.01, z + a.sin() * s * 1.3];
                    let r = 0.08 + 0.05 * hash(ix, iz, 99);
                    f.put(
                        Model::Anemone,
                        place(p, a, 0.0, 0.0, [r, r, r]),
                        reef_hue(ix, iz, 100),
                    )
                    .moving(anim::SWAY, a, 0.05, 0.8)
                    .glowing(0.24)
                    .grounded();
                }
            }
        }
        kind @ (BUSH | FINGER) => {
            // A stand of bushy heads or of finger coral, one colony grown
            // into a low hummock.
            let (model, base) = if kind == BUSH {
                (Model::CoralBush, 0.26)
            } else {
                (Model::CoralFinger, 0.22)
            };
            let clumps = 3 + (hash(ix, iz, 123) * 2.0) as i32;
            for k in 0..clumps {
                let a = yaw + k as f32 * 2.4;
                let off = (0.04 + 0.18 * hash(ix + k, iz, 125)) * grow;
                let p = [x + a.cos() * off, y - 0.01, z + a.sin() * off];
                if !w.visible(p, 0.25) {
                    continue;
                }
                let s = (base + 0.12 * hash(ix, iz + k, 126)) * grow;
                f.put(model, place(p, a, 0.0, 0.0, [s, s, s]), shade(k))
                    .moving(anim::SWAY, a, 0.015, 0.4)
                    .glowing(0.28)
                    .grounded();
            }
        }
        _ => {
            // Soft corals: sea fans set broadside to the surge that runs in
            // across the reef toward the window, a clump of sea whips, and
            // an anemone at their foot.
            let fans = 1 + (hash(ix, iz, 130) * 2.0) as i32;
            for k in 0..fans {
                let p = [
                    x + (hash(ix + k, iz, 131) - 0.5) * 0.4,
                    y - 0.01,
                    z + (hash(ix, iz + k, 132) - 0.5) * 0.2,
                ];
                let s = 0.10 + 0.09 * hash(ix + k, iz, 133);
                if !w.visible(p, s) {
                    continue;
                }
                let hue = [[0.88, 0.32, 0.50], [0.94, 0.58, 0.22], [0.62, 0.36, 0.80]]
                    [(hash(ix + k, iz, 134) * 3.0) as usize % 3];
                f.put(
                    Model::Fan,
                    place(p, (hash(ix + k, iz, 135) - 0.5) * 0.5, 0.0, 0.0, [s, s, s]),
                    hue,
                )
                .moving(anim::SWAY, yaw + k as f32, 0.12, 0.7)
                .grounded();
            }
            let s = (0.22 + 0.14 * hash(ix, iz, 136)) * grow;
            f.put(
                Model::SeaWhip,
                place([x, y - 0.01, z], yaw, 0.0, 0.0, [s, s, s]),
                shade(0),
            )
            .moving(anim::SWAY, yaw, 0.08, 0.5)
            .glowing(0.2)
            .grounded();
            let a = yaw + 2.6;
            let r = 0.06 + 0.05 * hash(ix, iz, 138);
            f.put(
                Model::Anemone,
                place(
                    [x + a.cos() * 0.12, y - 0.01, z + a.sin() * 0.12],
                    a,
                    0.0,
                    0.0,
                    [r, r, r],
                ),
                reef_hue(ix, iz, 139),
            )
            .moving(anim::SWAY, a, 0.05, 0.8)
            .glowing(0.24)
            .grounded();
        }
    }
    if mound > 0.1 && hash(ix, iz, 140) < 0.6 {
        reef_fish(f, w, ix, iz, [x, y + 0.12 + 0.10 * grow, z], 0.22 * grow);
    }
}

/// A swarm of small reef fish hovering over a coral head: orange anthias,
/// blue-green chromis, yellow damsels or purple anthias, one kind to a swarm.
/// Each fish is a cut-out card ([`Frame::impostors`], form 5), a quad, and
/// `vs_impostor` swims it round and about the swarm's centre by the clock,
/// darting and turning, so the swarm stays in the static cache.
fn reef_fish(f: &mut Frame, w: &World, ix: i32, iz: i32, centre: [f32; 3], spread: f32) {
    const KINDS: [[f32; 3]; 4] = [
        [1.00, 0.48, 0.30],
        [0.30, 0.78, 0.86],
        [1.00, 0.84, 0.20],
        [0.72, 0.42, 0.95],
    ];
    if !w.visible(centre, spread * 1.5) {
        return;
    }
    let kind = KINDS[(hash(ix, iz, 141) * 4.0) as usize % 4];
    let fish = 8 + (hash(ix, iz, 142) * 14.0) as i32;
    for k in 0..fish {
        let r = |salt: u32| hash(ix * 37 + k, iz, salt);
        let length = 0.020 + 0.010 * r(143);
        let p = [
            centre[0] + (r(144) - 0.5) * spread,
            centre[1] + r(145) * spread * 0.6,
            centre[2] + (r(146) - 0.5) * spread,
        ];
        let light = 0.85 + 0.3 * r(147);
        f.impostors.push(Sprite {
            place: [p[0], p[1], p[2], length * 0.55],
            tint: [
                kind[0] * light,
                kind[1] * light,
                kind[2] * light,
                r(148) * 10.0,
            ],
            stretch: [
                length * 0.5,
                FISH_CARD,
                0.0,
                spread * (0.25 + 0.35 * r(149)),
            ],
        });
    }
}

/// The impostor form `vs_impostor` swims as a small reef fish.
const FISH_CARD: f32 = 5.0;

/// The brine flats: pale salt spires and crusted lumps, with the dark pans
/// between them and the odd brine shrimp glinting over them.
fn brine_garden(f: &mut Frame, w: &World, site: GardenSite) {
    let GardenSite {
        ix,
        iz,
        x,
        y,
        z,
        yaw,
    } = site;
    let roll = hash(ix, iz, 95);
    if roll < 0.55 {
        let towers = 2 + (hash(ix, iz, 96) * 3.0) as i32;
        for k in 0..towers {
            let a = yaw + k as f32 * 2.4;
            let off = 0.05 + 0.12 * hash(ix + k, iz, 97);
            let p = [x + a.cos() * off, y - 0.02, z + a.sin() * off];
            if !w.visible(p, 0.2) {
                continue;
            }
            let s = 0.025 + 0.040 * hash(ix, iz + k, 98);
            let tall = s * (1.6 + 1.4 * hash(ix, iz, 101));
            let lean = (
                (hash(ix, iz + k, 99) - 0.5) * 0.5,
                (hash(ix, iz + k, 100) - 0.5) * 0.5,
            );
            // Salt crust stained here and there with iron, from white to a
            // pale ochre.
            let stain = hash(ix, iz + k, 107) * 0.18;
            let crust = [0.93, 0.90 - stain * 0.3, 0.84 - stain];
            let roll = hash(ix, iz + k, 106);
            if roll < 0.25 {
                // Some of the salt is translucent: a prism catching the light
                // through its faces, a glint rather than a white slab.
                let rows = place(p, a, lean.0, lean.1, [s * 0.45, tall, s * 0.45]);
                f.glass(Model::Hex, rows, [0.42, 0.52, 0.55]).glowing(0.10);
            } else if roll < 0.6 {
                // A crusted column.
                let rows = place(p, a, lean.0, lean.1, [s * 0.45, tall, s * 0.45]);
                f.put(Model::Hex, rows, crust)
                    .textured(Surface::Marble, 2.0)
                    .glowing(0.16)
                    .grounded();
            } else {
                // A spire, built up to a point by the brine seeping out.
                let rows = place(p, a, lean.0, lean.1, [s * 0.7, tall * 1.3, s * 0.7]);
                f.put(Model::Cone, rows, crust)
                    .textured(Surface::Marble, 2.0)
                    .glowing(0.16)
                    .grounded();
            }
        }
    } else if roll < 0.8 {
        // A shallow pan of brine: a dark, still sheet sunk into the crust.
        let r = 0.07 + 0.10 * hash(ix, iz, 102);
        let p = [x, y + 0.004, z];
        if w.visible(p, r) {
            f.put(
                Model::Sphere,
                place(
                    p,
                    0.0,
                    0.0,
                    0.0,
                    [r, 0.004, r * (0.7 + 0.3 * hash(ix, iz, 103))],
                ),
                [0.035, 0.10, 0.11],
            )
            .grounded();
        }
    } else {
        let s = 0.03 + 0.02 * hash(ix, iz, 104);
        f.put(
            Model::Star,
            place([x, y + 0.005, z], yaw, 0.0, 0.0, [s, s, s]),
            [0.86, 0.80, 0.70],
        )
        .grounded();
    }
    // A brine shrimp: a fleck of rose light just over the crust.
    if hash(ix, iz, 105) < 0.22 {
        f.sprite(
            [x + 0.08 * (yaw.sin()), y + 0.03, z + 0.08 * yaw.cos()],
            0.006,
            [0.35, 0.10, 0.07],
            GLOW,
        );
    }
}

/// The bloom's floor: glass sponges, violet fans and glowing sea pens.
fn bloom_garden(f: &mut Frame, _w: &World, site: GardenSite) {
    let GardenSite {
        ix,
        iz,
        x,
        y,
        z,
        yaw,
    } = site;
    let roll = hash(ix, iz, 110);
    if roll < 0.45 {
        for k in 0..3 {
            let a = k as f32 * 2.1 + yaw;
            let h = 0.05 + 0.09 * hash(ix, iz, 111 + k);
            f.put(
                Model::Sponge,
                place(
                    [x + a.cos() * 0.04, y - 0.005, z + a.sin() * 0.04],
                    a,
                    0.0,
                    0.0,
                    [0.016, h, 0.016],
                ),
                [0.62, 0.72, 0.78],
            )
            .glowing(0.12)
            .grounded();
        }
    } else if roll < 0.72 {
        let s = 0.09 + 0.10 * hash(ix, iz, 115);
        f.put(
            Model::Fan,
            place([x, y, z], yaw, 0.0, 0.0, [s, s, s]),
            [0.48, 0.56, 0.82],
        )
        .glowing(0.15)
        .moving(anim::SWAY, yaw, 0.15, 0.6)
        .grounded();
    } else {
        let root = [x, y, z];
        let top = [
            x + 0.03 * (yaw * 2.0).sin(),
            y + 0.14 + 0.12 * hash(ix, iz, 116),
            z + 0.03 * (yaw * 2.0).cos(),
        ];
        f.glass(Model::Tube, between(root, top, 0.006), [0.16, 0.52, 0.55])
            .moving(anim::WRITHE, yaw, 0.01, 0.9);
        f.sprite(top, 0.02, [0.18, 0.46, 0.38], GLOW);
    }
}

/// The trench: life clinging to its terraces — sea pens glowing faintly on
/// the treads, crinoids by their edges, sponges, fans and tube worms on the
/// far wall facing the window — slabs of rock jutting over each drop,
/// sediment spilling from the lips and drifting down, and lights blinking
/// deep in the dark at the bottom.
fn trench(w: &World, f: &mut Frame) {
    let c = w.pose.pos;
    if [-4.0, 0.0, 4.0]
        .iter()
        .all(|dx| w.share(c[0] + dx, c[2], TRENCH) <= 0.0)
    {
        return;
    }
    let rim = -SEABED[TRENCH];
    // Life on the terraces and the far wall.
    for &(ix, iz, x, z) in &cells(w, 0.3, 0.6, 9.0, 500) {
        let share = w.share(x, z, TRENCH);
        let needs = hash(ix, iz, 501) * 0.5 + 0.35;
        if share < needs {
            continue;
        }
        let u = z - corridor(x) - trench_wander(x);
        if !(1.0..7.6).contains(&u) {
            continue;
        }
        let y = w.ground(x, z);
        if !w.visible([x, y + 0.1, z], 0.3) {
            continue;
        }
        f.fade = fade(share, needs);
        let pick = hash(ix, iz, 502);
        let slope = (w.ground(x, z + 0.04) - w.ground(x, z - 0.04)) / 0.08;
        let yaw = hash(ix, iz, 503) * TAU;
        if slope > 1.2 {
            // The far wall, facing the window: stand a little out from it.
            let p = [x, y, z - 0.03];
            if pick < 0.4 {
                let hue = [[0.85, 0.62, 0.22], [0.55, 0.30, 0.62], [0.80, 0.38, 0.22]]
                    [(hash(ix, iz, 504) * 3.0) as usize % 3];
                for k in 0..3 {
                    let a = k as f32 * 2.1 + yaw;
                    let h = 0.08 + 0.12 * hash(ix, iz, 505 + k);
                    f.put(
                        Model::Sponge,
                        place(
                            [p[0] + a.cos() * 0.04, p[1] - 0.01, p[2] + a.sin() * 0.02],
                            a,
                            0.0,
                            -0.35,
                            [0.03, h, 0.03],
                        ),
                        hue,
                    );
                }
            } else if pick < 0.7 {
                let s = 0.12 + 0.14 * hash(ix, iz, 508);
                let hue = if hash(ix, iz, 509) < 0.5 {
                    [0.90, 0.42, 0.30]
                } else {
                    [0.92, 0.80, 0.55]
                };
                f.put(
                    Model::Fan,
                    place(p, (hash(ix, iz, 510) - 0.5) * 0.6, 0.0, -0.3, [s, s, s]),
                    hue,
                )
                .moving(anim::SWAY, yaw, 0.1, 0.6);
            } else {
                // Tube worms: white tubes, red plumes.
                for k in 0..4 {
                    let a = k as f32 * 1.6 + yaw;
                    let h = 0.05 + 0.06 * hash(ix, iz, 511 + k);
                    let foot = [p[0] + a.cos() * 0.03, p[1] - 0.02, p[2] + a.sin() * 0.015];
                    let top = [foot[0], foot[1] + h, foot[2] - 0.02];
                    f.put(
                        Model::Cylinder,
                        between(foot, top, 0.006),
                        [0.80, 0.78, 0.70],
                    );
                    f.put(
                        Model::Sphere,
                        place(top, a, 0.0, 0.0, [0.012, 0.018, 0.012]),
                        [0.85, 0.12, 0.10],
                    )
                    .glowing(0.15);
                }
            }
        } else if slope.abs() < 0.6 {
            if pick < 0.55 {
                // A sea pen, glowing faintly as sea pens do.
                let h = 0.12 + 0.12 * hash(ix, iz, 515);
                f.put(
                    Model::Kelp,
                    place([x, y, z], yaw, 0.0, 0.0, [0.35, h, 0.35]),
                    [0.90, 0.62, 0.40],
                )
                .moving(anim::SWAY, yaw, 0.04, 0.7)
                .glowing(0.12)
                .grounded();
            } else if pick < 0.8 {
                // A crinoid: a stalk and a crown of feathery arms.
                let h = 0.12 + 0.1 * hash(ix, iz, 516);
                let top = [x, y + h, z];
                let hue = if hash(ix, iz, 517) < 0.5 {
                    [0.92, 0.75, 0.25]
                } else {
                    [0.80, 0.25, 0.30]
                };
                f.put(
                    Model::Cylinder,
                    between([x, y - 0.01, z], top, 0.005),
                    [0.45, 0.40, 0.32],
                )
                .grounded();
                for k in 0..5 {
                    // The wave is the shader's (anim::WAVE); the placement
                    // keeps only the arm's fixed bearing.
                    let a = k as f32 / 5.0 * TAU + yaw;
                    let tip = [
                        top[0] + a.cos() * 0.07,
                        top[1] + 0.05,
                        top[2] + a.sin() * 0.07,
                    ];
                    f.put(Model::Cone, between(top, tip, 0.008), hue).moving(
                        anim::WAVE,
                        k as f32,
                        0.15,
                        0.6,
                    );
                }
            }
        }
    }
    // Slabs of rock jutting over each drop, and sediment spilling from them.
    for ix in ((c[0] - 7.0) / 0.55).floor() as i32..=((c[0] + 7.0) / 0.55).ceil() as i32 {
        for (k, &lip) in NEAR_LIPS.iter().chain(FAR_LIPS.iter()).enumerate() {
            let k = k as i32;
            let h = hash(ix, k, 520);
            if h < 0.45 {
                continue;
            }
            let x = (ix as f32 + 0.5 + (hash(ix, k, 521) - 0.5) * 0.6) * 0.55;
            // Over the drop: toward the window on the far wall.
            let out = if k >= NEAR_LIPS.len() as i32 {
                -1.0
            } else {
                1.0
            };
            let edge = corridor(x) + trench_wander(x) + lip;
            let share = w.share(x, edge, TRENCH);
            if share < 0.6 {
                continue;
            }
            let top = w.ground(x, edge - out * 0.2);
            let size = 0.25 + 0.3 * hash(ix, k, 522);
            let p = [x, top - 0.05, edge + out * 0.05];
            if !w.visible(p, size * 1.2) {
                continue;
            }
            f.fade = fade(share, 0.6);
            let model =
                [Model::Rock0, Model::Rock1, Model::Rock2][(hash(ix, k, 523) * 3.0) as usize % 3];
            let tone = 0.26 + 0.08 * hash(ix, k, 524);
            f.put(
                model,
                place(
                    p,
                    (hash(ix, k, 525) - 0.5) * 0.8,
                    0.0,
                    0.12 * out,
                    [size * 1.4, size * 0.22, size * 0.8],
                ),
                [tone, tone * 0.96, tone * 0.9],
            );
            // Now and then sediment spills from the lip and drifts down.
            if hash(ix, k, 526) < 0.3 {
                let fall = 1.4 + 0.6 * hash(ix, k, 527);
                // The fall and fade are the shader's (mote_sprite); the anchor
                // is the lip at the particle's n.
                let a = 0.45 * f.fade;
                for n in 0..18 {
                    let nf = n as f32;
                    let q = [x, p[1] - 0.05, p[2] + out * 0.15];
                    f.mote_sprite(q, 0.03, [0.55 * a, 0.50 * a, 0.40 * a], nf, h, fall);
                }
            }
        }
    }
    // Lights blinking deep in the trench, where the daylight never reaches.
    for &(ix, iz, x, z) in &cells(w, 0.4, 1.5, 10.0, 530) {
        let share = w.share(x, z, TRENCH);
        if share < 0.5 || hash(ix, iz, 531) < 0.5 {
            continue;
        }
        let y = w.ground(x, z);
        if y > rim - 2.6 {
            continue;
        }
        let p = [x, y + 0.1 + 1.6 * hash(ix, iz, 532), z];
        if !w.visible(p, 0.1) {
            continue;
        }
        // The blink is the shader's (GLOW mode 0); the anchor keeps the
        // biome's share as a colour scale.
        let a = smooth(0.5, 0.8, share);
        let hue = if hash(ix, iz, 535) < 0.7 {
            [0.10, 0.70, 0.95]
        } else {
            [0.40, 0.95, 0.55]
        };
        let rate = 0.5 + hash(ix, iz, 533);
        let phase = hash(ix, iz, 534) * TAU;
        f.glow_sprite(
            p,
            0.025,
            [hue[0] * a, hue[1] * a, hue[2] * a],
            phase,
            rate,
            0.0,
        );
        let halo = a * 0.12;
        f.glow_sprite(
            p,
            0.14,
            [hue[0] * halo, hue[1] * halo, hue[2] * halo],
            phase,
            rate,
            0.0,
        );
    }
}

/// The galleons of Vigo Bay, half buried, their treasure spilling out.
fn wrecks(w: &World, f: &mut Frame) {
    // A graveyard of ships: several in view at once, near and far.
    for &(ix, iz, x, z) in &cells(w, 2.4, 1.6, 10.0, 60) {
        if w.share(x, z, WRECKS) < 0.5 || hash(ix, iz, 61) < 0.35 {
            continue;
        }
        f.fade = fade(w.share(x, z, WRECKS), 0.5);
        if (z - corridor(x)).abs() < 1.2 {
            continue;
        }
        let length = 2.2 + 1.0 * hash(ix, iz, 62);
        let y = w.ground(x, z);
        if !w.visible([x, y + length * 0.25, z], length * 0.7) {
            continue;
        }
        let yaw = (hash(ix, iz, 63) - 0.5) * 1.2;
        let list = 0.25 * (hash(ix, iz, 64) - 0.3);
        let wood = [0.42, 0.31, 0.21];
        let p = [x, y + 0.04 * length, z];
        f.put(
            Model::Hull,
            place(p, yaw, 0.04, list, [length, length, length]),
            wood,
        )
        .grounded();
        // Castles fore and aft, masts and yards, a torn sail.
        let (s, c) = yaw.sin_cos();
        let along = |d: f32, up: f32| {
            [
                p[0] + c * d * length,
                p[1] + up * length,
                p[2] - s * d * length,
            ]
        };
        f.put(
            Model::Box,
            place(
                along(-0.38, 0.08),
                yaw,
                0.0,
                list,
                [0.10 * length, 0.05 * length, 0.10 * length],
            ),
            [0.27, 0.20, 0.14],
        );
        f.put(
            Model::Box,
            place(
                along(0.36, 0.07),
                yaw,
                0.0,
                list,
                [0.08 * length, 0.03 * length, 0.08 * length],
            ),
            [0.27, 0.20, 0.14],
        );
        let broken = hash(ix, iz, 65);
        for (k, d) in [-0.15f32, 0.15].iter().enumerate() {
            let h = if k == 1 && broken < 0.5 { 0.25 } else { 0.55 };
            let foot = along(*d, 0.04);
            let top = [foot[0] + list * 0.3, foot[1] + h * length, foot[2]];
            f.put(
                Model::Cylinder,
                between(foot, top, 0.008 * length),
                [0.24, 0.20, 0.15],
            );
            if h > 0.3 {
                let yard_y = top[1] - 0.05 * length;
                let half = [s * 0.16 * length, 0.0, c * 0.16 * length];
                let a = [top[0] - half[0], yard_y, top[2] - half[2]];
                let b = [top[0] + half[0], yard_y, top[2] + half[2]];
                f.put(
                    Model::Cylinder,
                    between(a, b, 0.005 * length),
                    [0.24, 0.20, 0.15],
                );
                f.put(
                    Model::Sail,
                    place(
                        [top[0], yard_y, top[2]],
                        yaw + PI * 0.5,
                        0.0,
                        0.0,
                        [0.15 * length, 0.22 * length, 0.15 * length],
                    ),
                    [0.42, 0.40, 0.32],
                )
                .moving(anim::SWAY, ix as f32, 0.02, 0.4);
            }
        }
        // A split chest by the bow, gold spilling over the sand.
        let chest = along(0.55, -0.02);
        let ground = w.ground(chest[0], chest[2]);
        let cp = [chest[0], ground + 0.025, chest[2]];
        f.put(
            Model::Box,
            place(cp, yaw + 0.4, 0.0, 0.0, [0.05, 0.03, 0.035]),
            [0.26, 0.18, 0.10],
        )
        .grounded();
        f.put(
            Model::Sphere,
            place(
                [cp[0] + 0.07, ground, cp[2] - 0.03],
                0.0,
                0.0,
                0.0,
                [0.08, 0.012, 0.05],
            ),
            [0.80, 0.58, 0.20],
        )
        .material(material::GOLD)
        .glowing(0.15);
        f.sprite(
            [cp[0] + 0.05, ground + 0.03, cp[2]],
            0.10,
            [0.20, 0.14, 0.04],
            GLOW,
        );
    }
}

/// The ruins of Atlantis: colonnades along avenues, many columns broken with
/// their drums fallen beside them, temple fronts farther off, and the volcano
/// whose fire lights the drowned city.
fn ruins(w: &World, f: &mut Frame) {
    let marble = [0.48, 0.46, 0.42];
    for &(ix, iz, x, z) in &cells(w, 0.55, 1.0, 16.0, 70) {
        // Colonnades stand in rows: whole rows of cells, and gaps between.
        if hash(0, iz, 71) < 0.45 || w.share(x, z, ATLANTIS) < hash(ix, iz, 72) * 0.5 + 0.4 {
            continue;
        }
        f.fade = fade(w.share(x, z, ATLANTIS), hash(ix, iz, 72) * 0.5 + 0.4);
        if hash(ix, iz, 73) < 0.15 {
            continue;
        }
        let x = (ix as f32 + 0.5) * 0.55;
        let z = (iz as f32 + 0.5) * 0.55;
        if (z - corridor(x)).abs() < 0.5 {
            continue;
        }
        let r = 0.055;
        // Stand the column on the highest ground under its foot, and leave
        // the steep banks alone: a shaft dropped into a terrace reads as
        // clipped, however well its foot is blended.
        let at = |dx: f32, dz: f32| w.ground(x + dx, z + dz);
        let y = at(0.0, 0.0)
            .max(at(r, 0.0))
            .max(at(-r, 0.0))
            .max(at(0.0, r))
            .max(at(0.0, -r));
        let low = at(0.0, 0.0)
            .min(at(r, 0.0))
            .min(at(-r, 0.0))
            .min(at(0.0, r))
            .min(at(0.0, -r));
        if y - low > 0.14 {
            continue;
        }
        let full = 0.65 + 0.15 * hash(ix, iz, 74);
        if !w.visible([x, y + full * 0.5, z], full * 0.6) {
            continue;
        }
        // A buried drum of a plinth, wide enough to meet the ground on the
        // downhill side.
        f.put(
            Model::Cylinder,
            between([x, y - 0.10, z], [x, y + 0.015, z], r * 1.3),
            [0.40, 0.38, 0.34],
        )
        .grounded();
        f.put(
            Model::Box,
            place([x, y + 0.02, z], 0.0, 0.0, 0.0, [r * 1.5, 0.025, r * 1.5]),
            marble,
        )
        .grounded();
        let broken = hash(ix, iz, 75);
        if broken < 0.45 {
            let h = full * (0.25 + broken);
            f.put(
                Model::Shaft,
                place([x, y + 0.04, z], 0.0, 0.0, 0.0, [r, h, r]),
                marble,
            )
            .grounded();
            f.put(
                Model::Break,
                place(
                    [x, y + 0.04 + h - 0.005, z],
                    broken * 9.0,
                    0.0,
                    0.0,
                    [r, r * 0.8, r],
                ),
                marble,
            );
            // A fallen drum at its foot.
            let a = hash(ix, iz, 76) * TAU;
            let foot = [
                x + a.cos() * 0.12,
                w.ground(x + a.cos() * 0.12, z + a.sin() * 0.12) + r * 0.9,
                z + a.sin() * 0.12,
            ];
            let end = [foot[0] + a.sin() * 0.12, foot[1], foot[2] - a.cos() * 0.12];
            f.put(Model::Cylinder, between(foot, end, r * 0.95), marble)
                .grounded();
        } else {
            f.put(
                Model::Shaft,
                place([x, y + 0.04, z], 0.0, 0.0, 0.0, [r, full, r]),
                marble,
            )
            .grounded();
            f.put(
                Model::Capital,
                place([x, y + 0.04 + full, z], 0.0, 0.0, 0.0, [r, 0.04, r]),
                marble,
            );
            f.put(
                Model::Box,
                place(
                    [x, y + 0.04 + full + 0.05, z],
                    0.0,
                    0.0,
                    0.0,
                    [r * 1.35, 0.012, r * 1.35],
                ),
                marble,
            );
        }
    }
    for &(ix, iz, x, z) in &cells(w, 5.0, 6.0, 22.0, 80) {
        if w.share(x, z, ATLANTIS) < 0.5 || hash(ix, iz, 81) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, z, ATLANTIS), 0.5);
        if (z - corridor(x)).abs() < 2.0 {
            continue;
        }
        let s = 1.4 + 0.4 * hash(ix, iz, 82);
        let y = w.ground(x, z);
        if !w.visible([x, y + s * 0.4, z], s) {
            continue;
        }
        let yaw = (hash(ix, iz, 83) - 0.5) * 0.6;
        let (sn, cs) = yaw.sin_cos();
        let at = |d: f32, up: f32, depth: f32| {
            [
                x + cs * d * s + sn * depth * s,
                y + up * s,
                z - sn * d * s + cs * depth * s,
            ]
        };
        let stone = [0.46, 0.45, 0.41];
        f.put(
            Model::Box,
            place(
                at(0.0, 0.03, 0.0),
                yaw,
                0.0,
                0.0,
                [0.56 * s, 0.03 * s, 0.3 * s],
            ),
            stone,
        )
        .grounded();
        for k in 0..6 {
            let d = -0.40 + k as f32 * 0.16;
            let gone = hash(ix * 7 + k, iz, 84);
            let h = if gone < 0.3 { 0.12 + gone * 0.6 } else { 0.55 };
            f.put(
                Model::Shaft,
                place(
                    at(d, 0.06, -0.15),
                    yaw,
                    0.0,
                    0.0,
                    [0.03 * s, h * s, 0.03 * s],
                ),
                stone,
            );
        }
        f.put(
            Model::Box,
            place(
                at(0.0, 0.645, -0.15),
                yaw,
                0.0,
                0.0,
                [0.48 * s, 0.045 * s, 0.06 * s],
            ),
            stone,
        );
        f.put(
            Model::Prism,
            place(
                at(0.0, 0.69, -0.15),
                yaw,
                0.0,
                0.0,
                [0.50 * s, 0.17 * s, 0.06 * s],
            ),
            stone,
        );
    }
    // The volcano, far off across the city: a broad, craggy cone with a
    // crater and a drifting ash plume. It is faded into the water so its
    // silhouette does not read as a clean pyramid against the surface.
    for &(ix, iz, x, z) in &cells(w, 14.0, 26.0, 44.0, 90) {
        let share = w.share(x, z, ATLANTIS);
        if share < 0.4 || hash(ix, iz, 91) < 0.3 {
            continue;
        }
        let presence = fade(share, 0.4);
        f.fade = presence * 0.55;
        let y = w.ground(x, z) - 0.3;
        let h = 3.4;
        if !w.visible([x, y + h * 0.5, z], 6.0) {
            continue;
        }
        let show = presence;
        f.put(
            Model::Vent,
            place([x, y, z], hash(ix, iz, 92) * TAU, 0.0, 0.0, [4.6, h, 4.6]),
            [0.14, 0.13, 0.12],
        )
        .grounded();
        let crater = [x, y + h * 0.97, z];
        f.glow_sprite(
            crater,
            0.55,
            [0.9 * show, 0.32 * show, 0.07 * show],
            ix as f32,
            2.3,
            1.0,
        );
        f.light_flicker(
            crater,
            7.0,
            [0.55 * show, 0.20 * show, 0.06 * show],
            ix as f32 + 0.5,
        );
        // The ash plume, drifting away down-current.
        for k in 0..10 {
            let rise = (w.t * 0.035 + k as f32 / 10.0 + hash(ix, iz, 93)).fract();
            let drift = rise * 1.6 + 0.18 * (rise * 7.0 + k as f32).sin();
            let p = [
                crater[0] + drift,
                crater[1] + rise * 2.4,
                crater[2] + 0.22 * (rise * 5.0 + ix as f32).cos(),
            ];
            let fade = (1.0 - rise) * show * 0.55;
            f.sprite(
                p,
                0.25 + rise * 0.95,
                [0.09 * fade, 0.12 * fade, 0.13 * fade],
                GLOW,
            );
        }
        // A thread of lava down one flank.
        for k in 0..6 {
            let t = k as f32 / 6.0;
            let p = [
                x + t * 2.2 + (t * 9.0).sin() * 0.12,
                y + h * (0.92 - t * 0.8),
                z - 0.8,
            ];
            f.glow_sprite(
                p,
                0.14,
                [0.55 * show, 0.16 * show, 0.03 * show],
                ix as f32,
                2.3,
                1.0,
            );
        }
    }
}

/// The drowned city's grids: a near one of blocks either side of the avenue,
/// out to [`CITY_SPLIT`] units across from it, and a far one of wider blocks
/// beyond, whose towers stand as silhouettes and lit windows in the haze.
/// The split is across the corridor, fixed in the world, so the two grids
/// never put a tower in the same place whichever way the window turns.
const CITY_NEAR: f32 = 3.2;
const CITY_FAR: f32 = 6.0;
const CITY_SPLIT: f32 = 12.0;
const CITY_SALT: u32 = 800;
const CITY_FAR_SALT: u32 = 840;
/// How far either side of the corridor the city leaves its avenue clear. The
/// helm keeps the hull off the seabed but knows nothing of buildings, and
/// its track wanders a little over half a unit off the corridor at most, so
/// nothing the city builds comes within this of the corridor's line.
const AVENUE: f32 = 2.3;
/// The highest a tower's spire may reach: under the water a sister keeps
/// below as she rises over one (`tall_things`).
const CITY_CEILING: f32 = -0.9;
/// A tower's height to the tip of its spire, and its crown's, at unit
/// scale, as `mesh::tower` builds it; the round tower's are a little lower.
const TOWER_TALL: f32 = 4.78;
const TOWER_CROWN: f32 = 3.9;
const ROTUNDA_CROWN: f32 = 3.5;
/// The new blocks' heights to their tips and crowns, at unit scale, as
/// their builders in `mesh` make them.
const ZIGGURAT_TALL: f32 = 3.34;
const ZIGGURAT_CROWN: f32 = 2.80;
const ARCADE_TALL: f32 = 1.56;
const ARCADE_CROWN: f32 = 1.44;
const FACTORY_TALL: f32 = 2.63;
const FACTORY_CROWN: f32 = 1.02;
const THEATRE_TALL: f32 = 2.24;
const THEATRE_CROWN: f32 = 1.96;
/// The colours its signs and spires are lit in: sea green, rose, amber and
/// a cold violet blue.
const NEON: [[f32; 3]; 4] = [
    [0.20, 0.95, 0.80],
    [1.00, 0.32, 0.52],
    [1.00, 0.60, 0.18],
    [0.52, 0.60, 1.00],
];

/// One building of the drowned city.
struct Building {
    x: f32,
    z: f32,
    /// The ground under it: the lowest under its footing, so no side of the
    /// plinth hangs in the water.
    y: f32,
    model: Model,
    /// Its scale across, and how far it is stretched upward beyond that.
    s: f32,
    tall: f32,
    yaw: f32,
    /// Signed distance from the corridor, positive to port.
    across: f32,
    share: f32,
    neon: [f32; 3],
    /// The building's own salt for whatever else it has.
    seed: (i32, i32, u32),
}

impl Building {
    /// Its height to the top of its crown, and to the tip of its spire.
    fn crown(&self) -> f32 {
        let at = match self.model {
            Model::Rotunda => ROTUNDA_CROWN,
            Model::Dome => 1.2,
            Model::Ziggurat => ZIGGURAT_CROWN,
            Model::Arcade => ARCADE_CROWN,
            Model::Factory => FACTORY_CROWN,
            Model::Theatre => THEATRE_CROWN,
            _ => TOWER_CROWN,
        };
        self.y + at * self.s * self.tall
    }

    fn top(&self) -> f32 {
        let at = match self.model {
            Model::Dome => 1.6,
            Model::Ziggurat => ZIGGURAT_TALL,
            Model::Arcade => ARCADE_TALL,
            Model::Factory => FACTORY_TALL,
            Model::Theatre => THEATRE_TALL,
            _ => TOWER_TALL,
        };
        self.y + at * self.s * self.tall
    }

    /// The radius of its footing, however it is turned.
    fn reach(&self) -> f32 {
        match self.model {
            Model::Dome => 1.05 * self.s,
            Model::Arcade => 1.02 * self.s,
            Model::Factory => 0.98 * self.s,
            Model::Theatre => 0.96 * self.s,
            Model::Ziggurat => 0.78 * self.s,
            _ => 0.92 * self.s,
        }
    }

    fn hash(&self, n: u32) -> f32 {
        hash(self.seed.0, self.seed.1, self.seed.2 + n)
    }
}

/// The building in block (ix, iz) of the near grid or, if `far`, the far
/// one: none in a square left open, in the avenue or the other grid's
/// half, or outside the city. Each block holds one building a little off its
/// middle, so neighbours never stand in each other.
fn building(w: &World, ix: i32, iz: i32, far: bool) -> Option<Building> {
    let (size, salt) = if far {
        (CITY_FAR, CITY_FAR_SALT)
    } else {
        (CITY_NEAR, CITY_SALT)
    };
    let h = |n: u32| hash(ix, iz, salt + n);
    let x = (ix as f32 + 0.5 + (h(0) - 0.5) * 0.3) * size;
    let z = (iz as f32 + 0.5 + (h(1) - 0.5) * 0.3) * size;
    let across = z - corridor(x);
    if far != (across.abs() >= CITY_SPLIT) || h(2) < if far { 0.18 } else { 0.08 } {
        return None;
    }
    let share = w.share(x, z, CITY);
    if share < 0.3 {
        return None;
    }
    // The frontage is low and busy; the towers stand behind it and in the
    // haze. Low blocks are the arcade, the works and the picture palace.
    let front = !far && across.abs() < AVENUE + 4.2;
    let roll = h(4);
    let model = if far {
        if roll < 0.22 {
            Model::Ziggurat
        } else if roll < 0.46 {
            Model::Rotunda
        } else {
            Model::Tower
        }
    } else if front && roll < 0.42 {
        if h(5) < 0.45 {
            Model::Arcade
        } else if h(5) < 0.78 {
            Model::Factory
        } else {
            Model::Theatre
        }
    } else if roll < 0.12 {
        Model::Dome
    } else if roll < 0.30 {
        Model::Rotunda
    } else if roll < 0.46 {
        Model::Ziggurat
    } else if roll < 0.60 {
        Model::Arcade
    } else if roll < 0.72 {
        Model::Factory
    } else if roll < 0.84 {
        Model::Theatre
    } else {
        Model::Tower
    };
    let s = match model {
        Model::Dome => 0.55 + 0.5 * h(3),
        Model::Arcade => 0.80 + 0.35 * h(3),
        Model::Factory => 0.76 + 0.34 * h(3),
        Model::Theatre => 0.72 + 0.30 * h(3),
        Model::Ziggurat => 0.72 + 0.35 * h(3),
        _ if far => 1.0 + 0.3 * h(3),
        _ => 0.75 + 0.45 * h(3),
    };
    let reach = match model {
        Model::Dome => 1.05,
        Model::Arcade => 1.02,
        Model::Factory => 0.98,
        Model::Theatre => 0.96,
        Model::Ziggurat => 0.78,
        _ => 0.92,
    } * s;
    if across.abs() < AVENUE + reach {
        return None;
    }
    let foot = match model {
        Model::Arcade => 0.9 * s,
        Model::Factory => 0.82 * s,
        Model::Theatre => 0.78 * s,
        Model::Ziggurat => 0.6 * s,
        _ => 0.64 * s,
    };
    let y = [
        (0.0, 0.0),
        (foot, 0.0),
        (-foot, 0.0),
        (0.0, foot),
        (0.0, -foot),
    ]
    .iter()
    .map(|&(dx, dz)| w.ground(x + dx, z + dz))
    .fold(f32::MAX, f32::min);
    let tallest = match model {
        Model::Dome => 1.6,
        Model::Ziggurat => ZIGGURAT_TALL,
        Model::Arcade => ARCADE_TALL,
        Model::Factory => FACTORY_TALL,
        Model::Theatre => THEATRE_TALL,
        _ => TOWER_TALL,
    };
    let tall = if model == Model::Dome {
        1.0
    } else {
        let room = (CITY_CEILING - y) / (tallest * s);
        let natural = match model {
            Model::Arcade | Model::Factory | Model::Theatre => 0.95 + 0.20 * h(5),
            _ => 0.85 + 0.4 * h(5),
        };
        natural.min(room)
    };
    if tall < 0.5 {
        return None;
    }
    Some(Building {
        x,
        z,
        y,
        model,
        s,
        tall,
        // Squared to the avenue, give or take.
        yaw: -corridor_slope(x).atan() + (h(6) - 0.5) * 0.35,
        across,
        share,
        neon: NEON[((h(7) * 4.0) as usize).min(3)],
        seed: (ix, iz, salt + 20),
    })
}

/// Put one of the city's models, in its own material and with its neon's
/// colour, which the shader finds in the instance's floor colour.
fn city_put(
    f: &mut Frame,
    model: Model,
    rows: [[f32; 4]; 3],
    colour: [f32; 3],
    neon: [f32; 3],
) -> &mut Instance {
    let i = f.put(model, rows, colour);
    i.material(material::CITY);
    i.floor = [neon[0], neon[1], neon[2], 0.0];
    i
}

/// A shop or arcade on the avenue's frontage, between the tower blocks: a
/// small low building that fills the street and gives the near view a
/// pavement of lit canopies and signs. It keeps clear of the avenue and of
/// the nearest block buildings.
fn street_shop(w: &World, ix: i32, iz: i32, x: f32, z: f32) -> Option<Building> {
    let across = z - corridor(x);
    let side = across.signum();
    let s = 0.55 + 0.18 * hash(ix, iz, CITY_SALT + 131);
    let reach = 1.02 * s;
    if across.abs() < AVENUE + 0.55 + reach || across.abs() > AVENUE + 1.5 {
        return None;
    }
    // The block it would stand in front of, and the neighbours either way.
    let bx = (x / CITY_NEAR).floor() as i32;
    let bz = (z / CITY_NEAR).floor() as i32;
    for dx in -1..=1 {
        for dz in -1..=1 {
            let Some(b) = building(w, bx + dx, bz + dz, false) else {
                continue;
            };
            if (x - b.x).hypot(z - b.z) < b.reach() + reach + 0.12 {
                return None;
            }
        }
    }
    let share = w.share(x, z, CITY);
    if share < 0.4 {
        return None;
    }
    let y = w
        .ground(x - reach * 0.6, z)
        .min(w.ground(x + reach * 0.6, z));
    let model = if hash(ix, iz, CITY_SALT + 132) < 0.72 {
        Model::Arcade
    } else {
        Model::Theatre
    };
    let tall = if model == Model::Theatre {
        0.82 + 0.12 * hash(ix, iz, CITY_SALT + 133)
    } else {
        1.0
    };
    let neon = NEON[((hash(ix, iz, CITY_SALT + 134) * 4.0) as usize).min(3)].map(|c| c * 0.75);
    Some(Building {
        x,
        z,
        y,
        model,
        s,
        tall,
        yaw: -corridor_slope(x).atan() + side * (hash(ix, iz, CITY_SALT + 135) - 0.5) * 0.10,
        across,
        share,
        neon,
        seed: (ix, iz, CITY_SALT + 136),
    })
}

/// The drowned city: towers of brass and iron stepped back in cornices and
/// lit from within, round towers under glass cupolas and glass domes over
/// halls, standing in blocks either side of a clear avenue and on into the
/// haze, where only their windows show; glass walkways strung between them,
/// neon on their facades, great pipes along the avenue, kelp and bubbles at
/// their feet. All of it stays put, so it is placed once per camera cell,
/// and everything that moves — the windows' flicker, the neon's buzz, the
/// bubbles, the kelp — is the shader's.
fn city(w: &World, f: &mut Frame) {
    let c = w.pose.pos;
    for &(ix, iz, _, _) in &cells(w, CITY_NEAR, 0.5, 20.0, CITY_SALT) {
        let Some(b) = building(w, ix, iz, false) else {
            continue;
        };
        let height = b.top() - b.y;
        let centre = [b.x, b.y + height * 0.5, b.z];
        f.fade = fade(b.share, 0.3);
        if w.visible(centre, height * 0.55 + b.reach()) {
            place_building(w, f, &b);
            crown_glow(f, &b);
        }
        // Walkways to the next block along the avenue and the next out from
        // it, between towers on the same side and low enough for both.
        for (dx, dz, n) in [(1, 0, 11), (0, 1, 12)] {
            if matches!(
                b.model,
                Model::Dome | Model::Arcade | Model::Factory | Model::Theatre
            ) || b.hash(n) > 0.55
            {
                continue;
            }
            let Some(o) = building(w, ix + dx, iz + dz, false) else {
                continue;
            };
            if matches!(
                o.model,
                Model::Dome | Model::Arcade | Model::Factory | Model::Theatre
            ) || o.across.signum() != b.across.signum()
            {
                continue;
            }
            let lift =
                b.y.min(o.y) + (b.s * b.tall).min(o.s * o.tall) * (1.0 + 0.9 * b.hash(n + 2));
            let (from, to) = ([b.x, lift, b.z], [o.x, lift, o.z]);
            let mid = [(b.x + o.x) * 0.5, lift, (b.z + o.z) * 0.5];
            if (mid[2] - corridor(mid[0])).abs() < AVENUE + 0.3 {
                continue;
            }
            let span = distance(from, to);
            if !w.visible(mid, span * 0.5 + 0.2) {
                continue;
            }
            f.fade = fade(b.share.min(o.share), 0.3);
            city_put(
                f,
                Model::Walkway,
                between(from, to, 0.07 + 0.03 * b.hash(n + 4)),
                [0.95, 0.92, 0.85],
                b.neon,
            );
        }
        // Kelp, bubbles, a sign and its light only near the window.
        let near = (b.x - c[0]).hypot(b.z - c[2]);
        if near > 11.0 || !w.visible(centre, height * 0.55 + b.reach() + 0.6) {
            continue;
        }
        f.fade = fade(b.share, 0.3);
        building_life(w, f, &b);
    }
    // A pavement of low shops along both sides of the avenue, filling the
    // gaps between the blocks. They stand clear of the avenue and of the
    // towers behind them, and are close enough to the window to carry signs.
    for &(ix, iz, x, z) in &cells(w, 2.4, 0.5, 16.0, CITY_SALT + 120) {
        let Some(b) = street_shop(w, ix, iz, x, z) else {
            continue;
        };
        let height = b.top() - b.y;
        if !w.visible([b.x, b.y + height * 0.5, b.z], height * 0.55 + b.reach()) {
            continue;
        }
        f.fade = fade(b.share, 0.35);
        place_building(w, f, &b);
        if (b.x - c[0]).hypot(b.z - c[2]) < 12.0 {
            building_life(w, f, &b);
        }
    }
    for &(ix, iz, _, _) in &cells(w, CITY_FAR, 8.0, 60.0, CITY_FAR_SALT) {
        let Some(b) = building(w, ix, iz, true) else {
            continue;
        };
        let height = b.top() - b.y;
        if !w.visible([b.x, b.y + height * 0.5, b.z], height * 0.55 + b.reach()) {
            continue;
        }
        f.fade = fade(b.share, 0.3);
        place_building(w, f, &b);
        crown_glow(f, &b);
    }
    // The great pipes, along the avenue on the window's side, a run to a
    // block where they surface.
    for &(ix, iz, x, z) in &cells(w, 2.0, 0.5, 14.0, CITY_SALT + 60) {
        let share = w.share(x, z, CITY);
        let across = z - corridor(x);
        if share < 0.4 || hash(ix, iz, CITY_SALT + 61) < 0.45 || !(1.5..2.3).contains(&across) {
            continue;
        }
        let s = 0.8 + 0.4 * hash(ix, iz, CITY_SALT + 62);
        let y = w.ground(x - s * 0.8, z).min(w.ground(x + s * 0.8, z)) - 0.02;
        if !w.visible([x, y + 0.3, z], s + 0.4) {
            continue;
        }
        f.fade = fade(share, 0.4);
        let yaw = -corridor_slope(x).atan()
            + if hash(ix, iz, CITY_SALT + 63) < 0.5 {
                0.0
            } else {
                PI
            };
        city_put(
            f,
            Model::Pipework,
            place([x, y, z], yaw, 0.0, 0.0, [s; 3]),
            [0.9, 0.95, 0.9],
            [0.0; 3],
        );
    }
}

/// A building of the city, as its model, tinted brass to bronze to dark iron.
fn place_building(_w: &World, f: &mut Frame, b: &Building) {
    let tone = 0.80 + 0.35 * b.hash(8);
    let tint = [
        tone,
        tone * (0.94 + 0.08 * b.hash(9)),
        tone * (0.86 + 0.10 * b.hash(9)),
    ];
    // The fourth of the floor colour says it stands on the seabed, so the
    // shader silts and shades its foot (`city_light`).
    city_put(
        f,
        b.model,
        place([b.x, b.y, b.z], b.yaw, 0.0, 0.0, [b.s, b.s * b.tall, b.s]),
        tint,
        b.neon,
    )
    .floor[3] = 1.0;
}

/// The bloom round a building's neon, faked: soft additive glows over the
/// beacon on its spire and, on a tower or a round tower, the neon band round
/// its crown. A full-screen bloom would need a second render target and a
/// blur pass over every pixel, which the Pi's tiler pays for in memory
/// bandwidth; a few small sprites cover only the pixels that glow.
fn crown_glow(f: &mut Frame, b: &Building) {
    let show = f.fade;
    let neon = b.neon.map(|c| c * show);
    let up = b.s * b.tall;
    let beacon = [b.x, b.top() - 0.03 * up, b.z];
    f.glow_sprite(
        beacon,
        0.20 * b.s,
        neon.map(|c| 0.40 * c + 0.10 * show),
        b.hash(26) * TAU,
        1.0,
        1.0,
    );
    let band = match b.model {
        Model::Tower => 3.62,
        Model::Rotunda => 3.285,
        _ => return,
    };
    f.glow_sprite(
        [b.x, b.y + band * up, b.z],
        0.62 * b.s,
        neon.map(|c| 0.16 * c),
        b.hash(27) * TAU,
        1.0,
        1.0,
    );
}

/// What lives about a building near the window: a sign on the facade it
/// turns to the avenue and the light it throws, kelp at its foot and a
/// stream of bubbles from a vent beside it.
fn building_life(w: &World, f: &mut Frame, b: &Building) {
    let show = f.fade;
    let toward = -b.across.signum();
    // The face toward the avenue: the model's -z face for a tower to port.
    let (sy, cy) = b.yaw.sin_cos();
    let normal = [toward * sy, toward * cy];
    if b.model != Model::Dome && b.hash(13) < 0.6 {
        // The face the sign hangs on: how far out it stands, and how high up
        // the facade, as fractions of the block's height.
        let (wall, rise) = match b.model {
            Model::Rotunda => (0.47, 0.95 + 0.15 * b.hash(14)),
            Model::Ziggurat => (0.68, 0.70 + 0.18 * b.hash(14)),
            Model::Arcade => (0.66, 0.88),
            Model::Factory => (0.62, 0.56),
            Model::Theatre => (0.80, 0.76),
            _ => (0.53, 0.95 + 0.25 * b.hash(14)),
        };
        let up = b.y + b.s * b.tall * rise;
        let out = wall * b.s + 0.08 * b.s;
        let at = [b.x + normal[0] * out, up, b.z + normal[1] * out];
        city_put(
            f,
            Model::Sign,
            place(at, normal[0].atan2(normal[1]), 0.0, 0.0, [b.s * 0.9; 3]),
            [0.9; 3],
            b.neon,
        );
        let glow = [b.neon[0] * show, b.neon[1] * show, b.neon[2] * show];
        let front = [at[0] + normal[0] * 0.05, at[1], at[2] + normal[1] * 0.05];
        f.glow_sprite(
            front,
            0.45 * b.s,
            [glow[0] * 0.30, glow[1] * 0.30, glow[2] * 0.30],
            b.hash(15) * TAU,
            2.3,
            1.0,
        );
        f.light_flicker(
            front,
            1.3,
            [glow[0] * 0.55, glow[1] * 0.55, glow[2] * 0.55],
            b.hash(16) * TAU + 0.5,
        );
    }
    // Kelp rooted in the silt drifted against its foot.
    if b.hash(17) < 0.7 {
        let fronds = 2 + (b.hash(18) * 4.0) as i32;
        for k in 0..fronds {
            let a = b.hash(30 + k as u32) * TAU;
            let r = b.reach() * 0.85 + 0.1 * b.hash(40 + k as u32);
            let (x, z) = (b.x + a.cos() * r, b.z + a.sin() * r);
            if (z - corridor(x)).abs() < AVENUE {
                continue;
            }
            let height = 0.5 + 0.7 * b.hash(50 + k as u32);
            f.put(
                Model::Kelp,
                place([x, w.ground(x, z), z], a, 0.0, 0.0, [0.9, height, 0.9]),
                [0.30, 0.42, 0.16],
            )
            .moving(anim::SWAY, a * 3.0, 0.08, 0.8)
            .grounded();
        }
    }
    // A stream of bubbles from a vent at its foot, rising past its windows.
    if b.hash(19) < 0.5 {
        let a = b.hash(20) * TAU;
        let r = b.reach() + 0.12;
        let foot = [b.x + a.cos() * r, b.y + 0.05, b.z + a.sin() * r];
        let rise = (b.top() - foot[1]).min(3.5);
        for k in 0..10 {
            let size = 0.010 + 0.008 * b.hash(60 + k);
            f.ember_sprite(
                foot,
                size,
                [0.30 * show, 0.42 * show, 0.42 * show],
                k as f32 / 10.0 + 0.05 * b.hash(70 + k),
                0.09 + 0.03 * b.hash(80 + k),
                rise,
            );
        }
    }
}

/// The searchlights the city's towers still sweep across the water from
/// their crowns, each round at its own pace. They move, so they are placed
/// every frame, from the same blocks as the towers.
fn city_beams(w: &World, f: &mut Frame) {
    if w.share(w.camera(), w.pose.pos[2], CITY) < 0.05 {
        return;
    }
    for &(ix, iz, _, _) in &cells(w, CITY_NEAR, 1.0, 22.0, CITY_SALT) {
        let Some(b) = building(w, ix, iz, false) else {
            continue;
        };
        if !matches!(b.model, Model::Tower | Model::Rotunda | Model::Ziggurat) || b.hash(21) > 0.3 {
            continue;
        }
        let show = fade(b.share, 0.3);
        let o = [b.x, b.crown() + 0.05, b.z];
        let way = if b.hash(22) < 0.5 { -1.0 } else { 1.0 };
        let yaw = b.hash(23) * TAU + w.t * (0.18 + 0.14 * b.hash(24)) * way;
        let pitch = -0.30 - 0.25 * b.hash(25);
        let dir = [
            pitch.cos() * yaw.cos(),
            pitch.sin(),
            pitch.cos() * yaw.sin(),
        ];
        let reach = 5.5;
        let run = [dir[0] * reach, dir[1] * reach, dir[2] * reach];
        let middle = add(o, [run[0] * 0.5, run[1] * 0.5, run[2] * 0.5]);
        if !w.visible(middle, reach * 0.5 + 0.8) {
            continue;
        }
        // A pale core, and a wide faint halo scattered off it.
        let k = 0.45 * show;
        let spread = 0.22 * reach;
        f.beams.push(Sprite {
            place: [o[0], o[1], o[2], 0.03],
            tint: [k, 0.95 * k, 0.80 * k, spread],
            stretch: [run[0], run[1], run[2], 0.0],
        });
        let h = 0.35 * k;
        f.beams.push(Sprite {
            place: [o[0], o[1], o[2], 0.08],
            tint: [h, 0.95 * h, 0.85 * h, spread * 2.4],
            stretch: [run[0], run[1], run[2], 0.0],
        });
        f.sprite(o, 0.12, [0.9 * show, 0.85 * show, 0.65 * show], GLOW);
    }
}

/// The light a building of the drowned city throws on the seabed about its
/// foot, and the shadow it stands in, for `terrain` to bake into the floor's
/// vertices when a tile is built. The seabed's grid is fine near the window
/// (0.08 units), so a pool of lamplight spreading from each block and a dark
/// line where wall meets silt come out smooth, and cost the GPU nothing: a
/// tiler pays for every decal or light volume it blends over the floor, but
/// a colour already in the vertex is free.
pub struct Spill {
    x: f32,
    z: f32,
    /// The footing's half width, and whether it is round or square.
    half: f32,
    round: bool,
    yaw: f32,
    /// The lamplight's colour at the foot, already as strong as the block
    /// is tall and as far as it has faded in.
    colour: [f32; 3],
    /// How far from the wall the light still reaches.
    reach: f32,
}

/// How far a pool of light reaches past its building's wall, at most.
const SPILL_REACH: f32 = 1.8;

impl Spill {
    fn of(b: &Building) -> Self {
        let (half, round) = match b.model {
            Model::Rotunda => (0.64, true),
            Model::Dome => (1.0, true),
            Model::Theatre => (0.96, true),
            Model::Arcade => (0.9, false),
            Model::Factory => (0.82, false),
            Model::Ziggurat => (0.6, false),
            _ => (0.64, false),
        };
        // Taller blocks have more windows to light the street; the neon at
        // its foot tints it a little.
        let tall = ((b.top() - b.y) / 3.0).clamp(0.6, 1.0);
        let k = tall * fade(b.share, 0.3);
        let warm = [1.0, 0.58, 0.22];
        Spill {
            x: b.x,
            z: b.z,
            half: half * b.s,
            round,
            yaw: b.yaw,
            colour: [0, 1, 2].map(|j| k * (1.2 * warm[j] + 0.35 * b.neon[j])),
            reach: SPILL_REACH * (0.7 + 0.3 * tall),
        }
    }

    /// How far (x, z) is outside the building's footing: negative inside.
    fn outside(&self, x: f32, z: f32) -> f32 {
        let (dx, dz) = (x - self.x, z - self.z);
        if self.round {
            return dx.hypot(dz) - self.half;
        }
        // Into the building's own frame, as `place` turns it.
        let (s, c) = self.yaw.sin_cos();
        let (u, v) = ((c * dx - s * dz).abs(), (s * dx + c * dz).abs());
        let (qu, qv) = (u - self.half, v - self.half);
        qu.max(0.0).hypot(qv.max(0.0)) + qu.max(qv).min(0.0)
    }

    /// The lamplight this building throws on the seabed at (x, z), and how
    /// much of the floor's own light its bulk takes away there.
    pub fn at(&self, x: f32, z: f32) -> ([f32; 3], f32) {
        let d = self.outside(x, z);
        if d > self.reach {
            return ([0.0; 3], 0.0);
        }
        let shadow = 0.45 * (1.0 - smooth(0.0, 0.2, d.max(0.0)));
        // The windows begin a floor up, so the light lands a little out from
        // the wall and falls away from there.
        let pool = smooth(0.0, 0.12, d)
            * (-d.max(0.0) / (0.3 * self.reach)).exp()
            * (1.0 - smooth(self.reach * 0.6, self.reach, d));
        (self.colour.map(|c| c * pool), shadow)
    }
}

/// Every building of the city whose pool of light can reach the square from
/// (x0, z0) to (x1, z1): the same blocks, shops and far towers `city`
/// places, found the same way, so the light on the floor is always under a
/// building that is there.
pub fn city_spills(w: &World, x0: f32, z0: f32, x1: f32, z1: f32) -> Vec<Spill> {
    let mut out = Vec::new();
    let corners = [
        (x0, z0),
        (x1, z0),
        (x0, z1),
        (x1, z1),
        ((x0 + x1) * 0.5, (z0 + z1) * 0.5),
    ];
    if corners.iter().all(|&(x, z)| w.share(x, z, CITY) < 0.25) {
        return out;
    }
    let m = SPILL_REACH + 1.6;
    let grid = |size: f32| {
        let i = ((x0 - m) / size).floor() as i32..=((x1 + m) / size).floor() as i32;
        let j = ((z0 - m) / size).floor() as i32..=((z1 + m) / size).floor() as i32;
        (i, j)
    };
    for (size, far) in [(CITY_NEAR, false), (CITY_FAR, true)] {
        let (columns, rows) = grid(size);
        for iz in rows {
            for ix in columns.clone() {
                if let Some(b) = building(w, ix, iz, far) {
                    out.push(Spill::of(&b));
                }
            }
        }
    }
    let (columns, rows) = grid(2.4);
    for iz in rows {
        for ix in columns.clone() {
            let (x, z) = cell_point(ix, iz, 2.4, CITY_SALT + 120);
            // The shops keep to a strip either side of the avenue: most cells
            // are out of it, and are dropped before any building is sought.
            let across = (z - corridor(x)).abs();
            if !(AVENUE..AVENUE + 1.6).contains(&across) {
                continue;
            }
            if let Some(b) = street_shop(w, ix, iz, x, z) {
                out.push(Spill::of(&b));
            }
        }
    }
    out
}

/// The lava cave: basalt columns, stalactites dripping molten rock, and the
/// glow of the pools.
fn cave(w: &World, f: &mut Frame) {
    // Basalt columns stand in clumps against the tunnel's walls, out of the
    // hull's sweep as it is set across the tunnel: in the halls a ragged
    // organ of tall ones at the foot of the far wall, where the window sees
    // them in a row against the rock; only a few, so the floor stays open.
    for &(ix, iz, x, z) in &cells(w, 0.16, 0.5, 9.0, 100) {
        let off = (z - corridor(x)).abs();
        let wall = tunnel_wide(x);
        if !(0.95f32.max(wall * 0.62)..wall * 0.9).contains(&off) || hash(ix, iz, 105) < 0.8 {
            continue;
        }
        let cluster = vnoise(x * 0.5, z * 0.5, 101);
        let share = w.share(x, z, CAVE);
        if cluster < 0.48 || share < 0.6 {
            continue;
        }
        // Not in front of a passage's mouth.
        if w.seabed.in_mouth(x, z - corridor(x), true) {
            continue;
        }
        f.fade = fade(share, 0.6);
        let y = w.ground(x, z);
        let tall = 1.0 + 0.9 * hall(x) * smooth(0.6, 0.85, off / wall);
        let h =
            (0.06 + 0.30 * smooth(0.48, 0.90, cluster) * (0.5 + 0.5 * hash(ix, iz, 102))) * tall;
        if !w.visible([x, y + h * 0.5, z], h) {
            continue;
        }
        let v = 0.30 + 0.08 * hash(ix, iz, 103);
        f.put(
            Model::Hex,
            place(
                [x, y - 0.02, z],
                hash(ix, iz, 104),
                0.0,
                0.0,
                [0.07, h + 0.02, 0.07],
            ),
            [v, v * 0.93, v * 0.9],
        )
        .textured(Surface::Basalt, 4.0)
        .grounded();
    }
    // Stalactites hang from the tunnel's arch, some dripping molten rock.
    // As thick on the vault as they were in the old bore, which was a third
    // as wide: the cells are larger, so a hall's roof is not a forest.
    for &(ix, iz, x, z) in &cells(w, 0.36, 0.5, 12.0, 110) {
        let off = z - corridor(x);
        let (wide, high) = tunnel_stretch(x);
        if off.abs() > TUNNEL * wide * 0.75 || hash(ix, iz, 111) < 0.3 {
            continue;
        }
        // None hangs over a chimney's open mouth.
        if w.seabed.in_mouth(x, off, false) {
            continue;
        }
        let Some((c, _, share)) = w.seabed.tunnel_at(x) else {
            continue;
        };
        if share < 0.65 {
            continue;
        }
        f.fade = fade(share, 0.65);
        let a = (off / (TUNNEL * wide)).clamp(-1.0, 1.0).acos();
        let top = c[1] + 0.85 * high * w.seabed.tunnel_radius_with_nooks(x, a) * a.sin() + 0.08;
        let len = 0.05 + 0.32 * hash(ix, iz, 112).powf(1.5);
        if !w.visible([x, top - len * 0.5, z], len) {
            continue;
        }
        let r = 0.015 + len * 0.12;
        f.put(
            Model::Cone,
            between([x, top, z], [x, top - len - 0.08, z], r),
            [0.13, 0.10, 0.09],
        );
        if hash(ix, iz, 113) > 0.75 {
            let pulse = 0.7 + 0.3 * (w.t * 3.0 + ix as f32).sin();
            f.sprite(
                [x, top - len - 0.08, z],
                0.035,
                [1.0 * pulse, 0.45 * pulse, 0.10],
                GLOW,
            );
        }
    }
    // The pools that light the tunnel: a light over each of the nearest.
    for &(ix, iz, x, z) in &cells(w, 1.0, 0.5, 9.0, 120) {
        // Along the middle of the floor only, as in the old bore: the
        // halls' far sides have lights of their own (`halls`).
        if (z - corridor(x)).abs() > TUNNEL * 1.3
            || w.share(x, z, CAVE) < 0.5
            || hash(ix, iz, 121) < 0.25
        {
            continue;
        }
        let y = w.ground(x, z);
        let show = fade(w.share(x, z, CAVE), 0.5);
        f.light_flicker(
            [x, y + 0.15, z],
            1.6,
            [1.4 * show, 0.42 * show, 0.09 * show],
            ix as f32 * 1.7,
        );
        for k in 0..4 {
            // The ember's rise/fade is the shader's (ember_sprite); the
            // anchor is the pool floor.
            let p = [
                x + (hash(ix, iz * 4 + k, 123) - 0.5) * 0.6,
                y,
                z + (hash(ix + k, iz, 124) - 0.5) * 0.6,
            ];
            f.ember_sprite(
                p,
                0.012,
                [1.0 * show, 0.45 * show, 0.1 * show],
                hash(ix * 4 + k, iz, 122),
                0.12,
                1.0,
            );
        }
    }
    halls(w, f);
    cave_props(w, f);
}

/// A point on the tunnel's wall at `x` and round the arch `a`, `inset` in
/// front of the rock (negative: into it, past the wall's ragged ledges): the
/// nooks' recesses included. None outside the tunnel.
fn tunnel_spot(seabed: &Seabed, x: f32, a: f32, inset: f32) -> Option<[f32; 3]> {
    let (c, lat, share) = seabed.tunnel_at(x)?;
    if share < TUNNEL_SHARE {
        return None;
    }
    let r = (seabed.tunnel_radius_with_nooks(x, a) - inset).max(0.18);
    let (k, v) = tunnel_stretch(x);
    Some([
        c[0] + lat[0] * k * r * a.cos(),
        c[1] + 0.85 * v * r * a.sin(),
        c[2] + lat[2] * k * r * a.cos(),
    ])
}

/// A point on the window side's wall of the tunnel at `x`, `height` above
/// its floor, `inset` in front of the rock (negative: into it). The arch is
/// an ellipse, so the angle round it is found from the height.
fn wall_at(seabed: &Seabed, x: f32, height: f32, inset: f32) -> Option<[f32; 3]> {
    let (_, high) = tunnel_stretch(x);
    let s = ((height - 0.25 * TUNNEL) / (0.85 * high * TUNNEL)).clamp(-0.55, 0.98);
    tunnel_spot(seabed, x, s.asin(), inset)
}

/// The light each hall throws on its own walls, as (where, colour, reach):
/// baked into the tunnel's vertices with the passages' spill, so the rock by
/// the lava river glows and the grotto's walls are green without a light
/// paid for per pixel.
fn hall_glows(seabed: &Seabed, x: f32) -> Vec<([f32; 3], [f32; 3], f32)> {
    let mut glows = Vec::new();
    let Some(halls) = seabed.halls(x) else {
        return glows;
    };
    let mut glow = |p: Option<[f32; 3]>, colour: [f32; 3], reach: f32| {
        if let Some(p) = p {
            glows.push((p, colour, reach));
        }
    };
    for hall in halls {
        match hall.kind {
            HallKind::Forge => {
                for k in 0..6 {
                    let x = hall.x - 1.5 + 0.5 * k as f32;
                    glow(wall_at(seabed, x, 0.1, 0.3), [0.55, 0.17, 0.03], 0.8);
                }
                let fall = hall.x - 0.45;
                glow(wall_at(seabed, fall, 0.7, 0.1), [0.75, 0.28, 0.06], 1.0);
                glow(wall_at(seabed, fall, 1.3, 0.1), [0.45, 0.16, 0.04], 0.7);
            }
            HallKind::Grotto => {
                for k in 0..5 {
                    let x = hall.x - 1.4 + 0.65 * k as f32;
                    glow(wall_at(seabed, x, 0.6, 0.1), [0.04, 0.26, 0.21], 0.7);
                }
            }
            HallKind::Colonnade => {
                glow(
                    wall_at(seabed, hall.x - 0.1, 0.3, 0.4),
                    [0.10, 0.15, 0.22],
                    1.0,
                );
            }
        }
    }
    glows
}

/// What fills the cave's halls, placed each frame: a lava fall and the river
/// along the foot of the forge's far wall, the grotto's luminous colonies
/// and still pool, the colonnade's shaft of light from the vault. All of it
/// on the window's side, across the hall's floor from the glass, and almost
/// all of it light: glowing sprites, a stretched shaft and a few flat
/// discs of molten rock, rather than geometry.
fn halls(w: &World, f: &mut Frame) {
    let cam_x = w.camera();
    let Some(halls) = w.seabed.halls(cam_x) else {
        return;
    };
    let t = w.t;
    for hall in halls {
        if (hall.x - cam_x).abs() > 9.0 {
            continue;
        }
        let Some((c, _, share)) = w.seabed.tunnel_at(hall.x) else {
            continue;
        };
        if share < TUNNEL_SHARE {
            continue;
        }
        let floor = c[1] - 0.25 * TUNNEL;
        let wide = tunnel_wide(hall.x);
        // On the floor of the hall, `across` from the corridor toward the
        // window's side, standing on the ground there.
        let ground = |x: f32, across: f32| {
            let z = corridor(x) + across;
            [x, w.ground(x, z).max(floor - 0.3), z]
        };
        if !w.visible(ground(hall.x, wide * 0.6), 2.6) {
            continue;
        }
        let id = (hall.x * 10.0).round() as i32;
        let h = |k: i32, salt: u32| hash(id, k, salt);
        match hall.kind {
            HallKind::Forge => {
                // The river: molten rock pooled along the far wall's foot,
                // in flat lengths that step down the ledges like a cascade
                // of pools, each lit from within.
                for k in 0..6 {
                    let x = hall.x - 1.5 + 0.5 * k as f32;
                    let across = wide * (0.74 + 0.04 * (k as f32 * 1.7).sin());
                    let ends = [ground(x - 0.3, across), ground(x + 0.3, across)];
                    let y = ends[0][1].max(ends[1][1]).max(ground(x, across)[1]) + 0.012;
                    let p = [x, y, corridor(x) + across];
                    if !w.visible(p, 0.4) {
                        continue;
                    }
                    f.put(
                        Model::Cylinder,
                        place(p, -corridor_slope(x).atan(), 0.0, 0.0, [0.34, 0.012, 0.16]),
                        [0.9, 0.22, 0.04],
                    )
                    .material(material::LAVA)
                    .glowing(0.9);
                    // The water shimmering over it, and embers lifting off.
                    f.glow_sprite(
                        [x, y + 0.06, p[2]],
                        0.32,
                        [0.40, 0.11, 0.02],
                        h(k, 901) * TAU,
                        1.1,
                        1.0,
                    );
                    for e in 0..3 {
                        f.ember_sprite(
                            [
                                x + (h(k * 3 + e, 902) - 0.5) * 0.5,
                                y,
                                p[2] + (h(k * 3 + e, 903) - 0.5) * 0.25,
                            ],
                            0.012,
                            [1.0, 0.45, 0.10],
                            h(k * 3 + e, 904),
                            0.10 + 0.05 * h(k * 3 + e, 905),
                            0.9,
                        );
                    }
                }
                // The fall: a ribbon of molten rock from high on the far
                // wall down into the river, its glow pulsing as it pours.
                let x = hall.x - 0.45;
                if let (Some(top), Some(foot)) = (
                    wall_at(w.seabed, x, 1.35, 0.06),
                    wall_at(w.seabed, x, 0.05, 0.16),
                ) {
                    let breathe = 0.85 + 0.15 * (t * 1.7 + hall.x).sin();
                    let k = 0.9 * breathe;
                    // A bright core and, about it, a wide dim sheath: the hot
                    // water shimmering round the pour.
                    let run = [foot[0] - top[0], foot[1] - top[1], foot[2] - top[2], 0.0];
                    for (width, glow) in [(0.12, 1.0), (0.34, 0.35)] {
                        let k = k * glow;
                        f.shafts.push(Sprite {
                            place: [top[0], top[1], top[2], width],
                            // w: lit by its own light, not the sun's.
                            tint: [1.0 * k, 0.36 * k, 0.06 * k, 1.0],
                            stretch: run,
                        });
                    }
                    // Bright beads running down it, and a splash of glow and
                    // embers where it meets the river.
                    for b in 0..7 {
                        let fall = (t * 0.35 + b as f32 / 7.0).fract();
                        let q = lerp3(top, foot, fall * fall);
                        f.sprite(q, 0.08, [0.9, 0.34, 0.06], GLOW);
                    }
                    f.glow_sprite(foot, 0.45, [0.65, 0.20, 0.04], h(0, 906) * TAU, 2.3, 1.0);
                    f.light_flicker(
                        [foot[0], foot[1] + 0.25, foot[2] - 0.2],
                        1.4,
                        [1.5, 0.45, 0.08],
                        h(1, 906) * TAU,
                    );
                    for e in 0..8 {
                        f.ember_sprite(
                            [
                                foot[0] + (h(e, 907) - 0.5) * 0.4,
                                foot[1],
                                foot[2] - 0.1 * h(e, 908),
                            ],
                            0.014,
                            [1.0, 0.5, 0.12],
                            h(e, 909),
                            0.14 + 0.06 * h(e, 910),
                            1.4,
                        );
                    }
                }
            }
            HallKind::Grotto => {
                // Colonies of luminous things on the far wall, clustered as
                // fungi and worm tubes gather on a seep, each breathing its
                // light in its own time; green and blue, a few violet.
                for k in 0..30 {
                    let cluster = k / 6;
                    let x = hall.x - 1.4 + 0.65 * cluster as f32 + (h(k, 911) - 0.5) * 0.35;
                    let height = 0.15 + 0.9 * h(cluster, 912) + (h(k, 913) - 0.5) * 0.3;
                    let Some(q) = wall_at(w.seabed, x, height, 0.03) else {
                        continue;
                    };
                    let tint = match k % 5 {
                        0 => [0.35, 0.25, 0.8],
                        1 | 2 => [0.10, 0.75, 0.55],
                        _ => [0.15, 0.55, 0.85],
                    };
                    f.glow_sprite(
                        q,
                        0.025 + 0.035 * h(k, 914),
                        tint.map(|v| v * 0.8),
                        h(k, 915) * TAU,
                        0.4 + 0.6 * h(k, 916),
                        0.0,
                    );
                }
                // Tube worms in a clump on the floor before the wall: pale
                // stalks with glowing crowns.
                let bed = ground(hall.x - 0.7, wide * 0.72);
                for k in 0..6 {
                    let p = [
                        bed[0] + (h(k, 917) - 0.5) * 0.3,
                        bed[1] - 0.01,
                        bed[2] + (h(k, 918) - 0.5) * 0.2,
                    ];
                    let len = 0.10 + 0.12 * h(k, 919);
                    let tip = [p[0] + (h(k, 920) - 0.5) * 0.04, p[1] + len, p[2]];
                    f.put(Model::Cylinder, between(p, tip, 0.012), [0.62, 0.60, 0.52]);
                    f.glow_sprite(tip, 0.03, [0.9, 0.25, 0.35], h(k, 921) * TAU, 0.8, 0.0);
                }
                // A still pool in the floor, lit from below by what lives in
                // it: it holds the colonies' light as a sheet of green.
                let pool = ground(hall.x + 0.2, wide * 0.55);
                f.put(
                    Model::Cylinder,
                    place(
                        [pool[0], pool[1] + 0.01, pool[2]],
                        0.3,
                        0.0,
                        0.0,
                        [0.42, 0.008, 0.26],
                    ),
                    [0.03, 0.22, 0.20],
                )
                .glowing(0.55);
                f.glow_sprite(
                    [pool[0], pool[1] + 0.05, pool[2]],
                    0.5,
                    [0.04, 0.22, 0.18],
                    h(2, 922) * TAU,
                    0.5,
                    1.0,
                );
                f.light([pool[0], pool[1] + 0.3, pool[2]], 1.1, [0.10, 0.55, 0.45]);
            }
            HallKind::Colonnade => {
                // A chimney high in the vault lets down a shaft of pale
                // light across the columns, with motes sinking through it.
                let x = hall.x - 0.3;
                let Some(top) = wall_at(w.seabed, x, 1.75, 0.05) else {
                    continue;
                };
                let foot = ground(x + 0.25, wide * 0.55);
                let breathe = 0.8 + 0.2 * (t * 0.4 + hall.x).sin();
                let k = 0.35 * breathe;
                f.shafts.push(Sprite {
                    place: [top[0], top[1], top[2], 0.16],
                    tint: [0.42 * k, 0.68 * k, 1.0 * k, 1.0],
                    stretch: [foot[0] - top[0], foot[1] - top[1], foot[2] - top[2], 0.0],
                });
                f.glow_sprite(foot, 0.35, [0.10, 0.16, 0.24], h(0, 931) * TAU, 0.4, 1.0);
                f.light([foot[0], foot[1] + 0.4, foot[2]], 1.0, [0.25, 0.38, 0.55]);
                for m in 0..12 {
                    let fall = (t * 0.03 + h(m, 932)).fract();
                    let q = lerp3(top, foot, fall);
                    let q = [
                        q[0] + (h(m, 933) - 0.5) * 0.2,
                        q[1],
                        q[2] + (h(m, 934) - 0.5) * 0.2,
                    ];
                    let glow = smooth(0.0, 0.15, fall) * smooth(1.0, 0.8, fall);
                    f.sprite(q, 0.012, [0.5 * glow, 0.7 * glow, 0.9 * glow], GLOW);
                }
            }
        }
    }
}

/// The props in the tunnel: what has settled in the nooks (a landing party's
/// camp, a crystal pocket, a still lava pool), and the stalagmites, rubble
/// and flowstone that gather along the walls between them. The geometry of a
/// nook is the tunnel's own (`tunnel_radius_with_nooks`); these are only what
/// stands in it, so the hollow and its contents always agree.
fn cave_props(w: &World, f: &mut Frame) {
    let cam_x = w.camera();
    let cam = w.pose.pos;
    let Some(nooks) = w.seabed.nooks(cam_x) else {
        return;
    };
    let basalt = [0.16, 0.14, 0.13];
    for n in nooks {
        if (n.x - cam_x).abs() > 14.0 {
            continue;
        }
        let Some((c, lat, share)) = w.seabed.tunnel_at(n.x) else {
            continue;
        };
        if share < 0.6 {
            continue;
        }
        let floor = c[1] - 0.25 * TUNNEL;
        let side = if n.a.cos() < 0.0 { -1.0 } else { 1.0 };
        // Each nook's things are laid out as for a bore of the old size,
        // about a spot on its floor, and then grown about that spot and set
        // down at the foot of the hall's far wall: everything in it a size
        // the window can read from across the hall, nothing in a jumble.
        let at = |along: f32, across: f32| {
            [
                c[0] + along + lat[0] * side * across,
                floor,
                c[2] + lat[2] * side * across,
            ]
        };
        let (k, _) = tunnel_stretch(n.x);
        let pivot = at(0.0, 0.80);
        let foot = {
            let q = at(0.0, 0.80 * k);
            [q[0], w.ground(q[0], q[2]).max(floor - 0.1), q[2]]
        };
        if distance(foot, cam) < 0.8 || !w.visible([foot[0], foot[1] + 0.5, foot[2]], 1.6) {
            continue;
        }
        f.fade = fade(share, 0.6);
        let mark = f.mark();
        let h = |k: u32| hash((n.x * 10.0) as i32, n.kind as i32, k);
        // A broken rim of rock about the hollow's foot.
        for k in 0..4 {
            let q = at((h(20 + k) - 0.5) * 0.24, 0.78 + 0.12 * h(30 + k));
            let s = 0.020 + 0.030 * h(40 + k);
            f.put(
                [Model::Rock0, Model::Rock1, Model::Rock2][(k as usize) % 3],
                place(
                    [q[0], floor + s * 0.55, q[2]],
                    h(50 + k) * TAU,
                    0.0,
                    h(60 + k) * TAU,
                    [s, s * 0.65, s * 1.2],
                ),
                basalt,
            )
            .textured(Surface::Basalt, 5.0);
        }
        match n.kind {
            NookKind::Crystal => {
                // The points grow out of the wall itself, below; here only
                // the light they fill the hollow with.
                let (_, strength) = Secret::Crystal.light();
                let light = at(0.0, 0.62);
                f.light([light[0], floor + 0.55, light[2]], 0.45, [0.12, 0.10, 0.24]);
                let glow = Secret::Crystal.light().0.map(|v| v * strength * 0.20);
                f.glow_sprite(
                    [light[0], floor + 0.55, light[2]],
                    0.30,
                    glow,
                    h(150) * TAU,
                    1.4,
                    1.0,
                );
            }
            NookKind::Ember => {
                // A still pool in the hollow, its rim of cooled rock, and
                // embers drifting up from it.
                f.put(
                    Model::Cylinder,
                    place(
                        [at(0.0, 0.84)[0], floor + 0.012, at(0.0, 0.84)[2]],
                        0.0,
                        0.0,
                        0.0,
                        [0.16, 0.012, 0.11],
                    ),
                    [0.85, 0.18, 0.035],
                )
                .material(material::LAVA)
                .glowing(0.75);
                let a = at(0.0, 0.82);
                let flicker = 0.85 + 0.15 * (w.t * 0.7 + n.x).sin();
                f.light_flicker(
                    [a[0], floor + 0.22, a[2]],
                    0.75,
                    [1.2 * flicker, 0.36 * flicker, 0.07 * flicker],
                    h(160) * TAU,
                );
                for k in 0..10 {
                    let q = at((h(170 + k) - 0.5) * 0.26, 0.66 + 0.18 * h(180 + k));
                    f.ember_sprite(
                        [q[0], floor + 0.02, q[2]],
                        0.010 + 0.006 * h(190 + k),
                        [1.0, 0.42, 0.08],
                        h(200 + k),
                        0.10 + 0.04 * h(210 + k),
                        0.55,
                    );
                }
                f.put(
                    Model::Hex,
                    place(at(-0.26, 0.86), 0.6, 0.0, 0.0, [0.05, 0.12, 0.05]),
                    [0.10, 0.09, 0.09],
                );
            }
            NookKind::Camp => {
                // A landing party's cache, left when the Nautilus went on:
                // crates, a bedroll, a helmet and a lantern still burning.
                let warm = [1.0, 0.62, 0.24];
                let crate_at = at(-0.15, 0.82);
                f.put(
                    Model::Box,
                    place(
                        [crate_at[0], floor + 0.13, crate_at[2]],
                        n.a + 0.5,
                        0.0,
                        0.0,
                        [0.045, 0.040, 0.045],
                    ),
                    [0.32, 0.23, 0.14],
                )
                .textured(Surface::Wood, 4.0);
                f.put(
                    Model::Box,
                    place(
                        [crate_at[0] + 0.06, floor + 0.30, crate_at[2] - 0.03],
                        n.a - 0.3,
                        0.0,
                        0.0,
                        [0.042, 0.038, 0.042],
                    ),
                    [0.36, 0.26, 0.16],
                )
                .textured(Surface::Wood, 4.0);
                for (dx, dz) in [(-0.20, 0.0), (0.0, -0.12)] {
                    f.put(
                        Model::Box,
                        place(
                            [crate_at[0] + dx, floor + 0.11, crate_at[2] + dz],
                            0.0,
                            0.0,
                            0.0,
                            [0.005, 0.045, 0.050],
                        ),
                        [0.16, 0.13, 0.08],
                    )
                    .textured(Surface::Plates, 8.0);
                }
                let roll = at(0.22, 0.80);
                f.put(
                    Model::Capsule,
                    place(
                        [roll[0], floor + 0.06, roll[2]],
                        0.7,
                        0.0,
                        0.0,
                        [0.085, 0.026, 0.038],
                    ),
                    [0.42, 0.36, 0.24],
                )
                .textured(Surface::Canvas, 1.5);
                let helm = at(0.30, 0.87);
                f.put(
                    Model::DiverHelmet,
                    place([helm[0], floor + 0.08, helm[2]], 1.3, 0.0, 0.0, [0.18; 3]),
                    [0.56, 0.36, 0.22],
                );
                let lamp = at(-0.02, 0.90);
                f.put(
                    Model::Cylinder,
                    place(
                        [lamp[0], floor + 0.14, lamp[2]],
                        0.0,
                        0.0,
                        0.0,
                        [0.016, 0.026, 0.016],
                    ),
                    [0.95, 0.70, 0.32],
                )
                .material(material::GOLD)
                .glowing(1.4);
                f.put(
                    Model::Box,
                    place(
                        [lamp[0], floor + 0.21, lamp[2]],
                        0.0,
                        0.0,
                        0.0,
                        [0.021, 0.005, 0.021],
                    ),
                    [0.25, 0.20, 0.12],
                )
                .textured(Surface::Plates, 8.0);
                f.light_flicker([lamp[0], floor + 0.18, lamp[2]], 0.8, warm, 0.8);
                f.glow_sprite(
                    [lamp[0], floor + 0.18, lamp[2]],
                    0.16,
                    [0.55, 0.26, 0.08],
                    h(220) * TAU,
                    2.0,
                    1.0,
                );
                // A pick stood against the wall, its haft and iron head.
                let pick = at(0.06, 0.82);
                f.put(
                    Model::Cylinder,
                    place(
                        [pick[0], floor + 0.16, pick[2]],
                        0.0,
                        0.55,
                        0.0,
                        [0.005, 0.070, 0.005],
                    ),
                    [0.34, 0.24, 0.15],
                )
                .textured(Surface::Wood, 5.0);
                f.put(
                    Model::Box,
                    place(
                        [pick[0] + 0.07, floor + 0.28, pick[2]],
                        0.0,
                        0.55,
                        0.0,
                        [0.032, 0.006, 0.006],
                    ),
                    [0.30, 0.30, 0.32],
                );
            }
        }
        f.grow_since(&mark, pivot, foot, NOOK_GROW);
        if n.kind == NookKind::Crystal {
            // Points growing out of the hollow's wall, aimed across the hall
            // toward the boat's way, so the window sees their long faces.
            let violet = [0.42, 0.34, 0.95];
            for k in 0..6 {
                let a = n.a + (h(70 + k) - 0.5) * n.spread * 1.6;
                let x = n.x + (h(80 + k) - 0.5) * n.width * 1.4;
                let Some(base) = tunnel_spot(w.seabed, x, a, -0.07) else {
                    continue;
                };
                let inward = norm([c[0] - base[0], 0.0, c[2] - base[2]]);
                let dir = norm([
                    inward[0] * 0.55 + (h(90 + k) - 0.5) * 0.6,
                    0.35 + 0.7 * h(100 + k),
                    inward[2] * 0.55 + (h(110 + k) - 0.5) * 0.6,
                ]);
                let len = NOOK_GROW * (0.18 + 0.20 * h(120 + k));
                let tip = add(base, dir.map(|v| v * len));
                let width = NOOK_GROW * (0.025 + 0.025 * h(130 + k));
                f.glass(
                    if k < 3 {
                        Model::Crystal
                    } else {
                        Model::CrystalCluster
                    },
                    between(base, tip, width),
                    violet,
                )
                .material(material::CRYSTAL)
                .glowing(0.22 + 0.18 * h(140 + k));
                if k % 3 == 0 {
                    let twinkle = (w.t * 1.3 + k as f32 * 2.1).sin().max(0.0).powi(8);
                    let warm = 0.7 + 0.9 * twinkle;
                    f.sprite(tip, 0.03, [0.6 * warm, 0.5 * warm, 1.1 * warm], GLOW);
                }
            }
        }
    }
    // Stalagmites and rubble along the walls between the nooks, and the odd
    // flowstone column where a drip has run for a while.
    for &(ix, iz, x, z) in &cells(w, 0.42, 0.5, 10.0, 150) {
        let off = z - corridor(x);
        if !(0.95..tunnel_wide(x) * 0.88).contains(&off.abs()) || w.seabed.in_mouth(x, off, true) {
            continue;
        }
        let share = w.share(x, z, CAVE);
        if share < 0.65 {
            continue;
        }
        let Some((c, _, _)) = w.seabed.tunnel_at(x) else {
            continue;
        };
        let floor = c[1] - 0.25 * TUNNEL;
        if distance([x, floor, z], cam) < 1.10 {
            continue;
        }
        let h = hash(ix, iz, 151);
        if h < 0.16 {
            continue;
        }
        f.fade = fade(share, 0.65);
        if h > 0.62 {
            let len = 0.035 + 0.11 * hash(ix, iz, 152).powf(1.4);
            let r = 0.008 + 0.013 * hash(ix, iz, 153);
            if w.visible([x, floor + len * 0.5, z], len) {
                f.put(
                    Model::Cone,
                    place([x, floor + 0.01, z], 0.0, 0.0, 0.0, [r, len, r]),
                    basalt,
                )
                .textured(Surface::Basalt, 7.0);
            }
        } else {
            let s = 0.018 + 0.030 * hash(ix, iz, 154);
            f.put(
                [Model::Rock0, Model::Rock1, Model::Rock2][(ix + iz).rem_euclid(3) as usize],
                place(
                    [x, floor + s * 0.45, z],
                    hash(ix, iz, 155) * TAU,
                    0.0,
                    hash(ix, iz, 156) * TAU,
                    [s, s * 0.55, s * 1.2],
                ),
                basalt,
            )
            .textured(Surface::Basalt, 5.0);
        }
    }
}

/// A passage off the tunnel, as built: its opening, and the floor of the
/// tunnel where it leaves it.
#[derive(Clone, Copy, Debug)]
struct Passage {
    opening: Opening,
    floor: f32,
}

impl Passage {
    fn at(&self, p: [f32; 3]) -> [f32; 3] {
        corridor_point(p, self.floor)
    }

    /// The middle of its section a fraction `s` of the way in.
    fn centre(&self, s: f32) -> [f32; 3] {
        self.at(self.opening.path(s).0)
    }

    fn wall(&self, s: f32, angle: f32) -> [f32; 3] {
        self.at(self.opening.wall(s, angle))
    }

    /// Where its light spills out into the tunnel.
    fn mouth(&self) -> [f32; 3] {
        if self.opening.side() {
            self.at([self.opening.x, self.opening.wall - 0.05, 0.35])
        } else {
            self.centre(0.0)
        }
    }
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let d = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
}

/// The tunnel through the cave's massif: a ragged arch of rock inside, the
/// massif's dome over it outside, and at each mouth the cliff face with the
/// opening in it; and off it the passages to the cave's secrets, cut
/// through its wall and vault. Built from rings fixed in the world, so it
/// never shifts as the boat moves. A tunnel is short, so it is built whole
/// once the boat comes near it, and not again until it is left behind.
fn tunnel(w: &World, f: &mut Frame) {
    build_tunnel(w, f);
    passages(w, f);
}

fn build_tunnel(w: &World, f: &mut Frame) {
    use super::mesh::Vertex;
    let seabed = w.seabed;
    let cam = w.camera();
    let first = ((cam - 16.0) / RING_STEP).floor() as i32;
    let last = ((cam + 16.0) / RING_STEP).ceil() as i32;
    let inside = |k: i32| {
        let x = k as f32 * RING_STEP;
        seabed.share(x, corridor(x), CAVE) >= TUNNEL_SHARE
    };
    let (from, to) = match (
        (first..=last).find(|&k| inside(k)),
        (first..=last).rev().find(|&k| inside(k)),
    ) {
        (Some(a), Some(b)) => (a, b),
        _ => (0, -1),
    };
    let span = Some((from, to, seabed.seed.to_bits()));
    if f.tunnel_span == span {
        return;
    }
    f.tunnel_span = span;
    f.tunnel_version += 1;
    f.tunnel.clear();
    f.tunnel_indices.clear();
    f.passages.clear();
    if from > to {
        return;
    }
    // The passages whose mouths lie wholly within the tunnel.
    for k in [from, to] {
        for opening in seabed.openings(k as f32 * RING_STEP).into_iter().flatten() {
            let (k0, k1) = opening.rings();
            let whole = (k0 - 1..=k1 + 1).all(|k| (from..=to).contains(&k) && inside(k));
            if whole && !f.passages.iter().any(|p| p.opening.x == opening.x) {
                f.passages.push(Passage {
                    opening,
                    floor: seabed.tunnel_floor(opening.x),
                });
            }
        }
    }
    let passages = &f.passages;
    let verts = &mut f.tunnel;
    let indices = &mut f.tunnel_indices;
    // The light each passage throws on the rock about its mouth, baked into
    // the tunnel's vertices: no lights to pay for per pixel.
    let glows: Vec<([f32; 3], [f32; 3])> = passages
        .iter()
        .map(|p| {
            let (c, k) = p.opening.kind.light();
            (p.mouth(), c.map(|v| v * k * 0.45))
        })
        .collect();
    let lit_halls = hall_glows(seabed, (from + to) as f32 * 0.5 * RING_STEP);
    let spill = |p: [f32; 3]| {
        let mut e = [0.0f32; 3];
        for (m, c) in &glows {
            let d = distance(p, *m);
            let fall = (-d * d / 0.1).exp();
            for j in 0..3 {
                e[j] += c[j] * fall;
            }
        }
        for (m, c, reach) in &lit_halls {
            let d = distance(p, *m) / reach;
            let fall = (-d * d).exp();
            for j in 0..3 {
                e[j] += c[j] * fall;
            }
        }
        e
    };
    // The dome swells out over the side passages, so none runs out of it.
    let swell = |x: f32| {
        passages
            .iter()
            .filter(|p| p.opening.side())
            .map(|p| {
                let o = p.opening;
                let lo = o.x - o.half - 0.15 + o.bend.min(0.0);
                let hi = o.x + o.half + 0.15 + o.bend.max(0.0);
                smooth(lo - 0.3, lo, x) * smooth(hi + 0.3, hi, x)
            })
            .fold(0.0, f32::max)
    };
    let n = AROUND + 1;
    let n32 = n as u32;
    // Each ring: AROUND + 1 inner points then as many outer ones.
    let mut rings: Vec<Option<u32>> = Vec::new();
    for k in from..=to {
        let x = k as f32 * RING_STEP;
        let Some((c, lat, share)) = seabed.tunnel_at(x) else {
            rings.push(None);
            continue;
        };
        let floor = c[1] - 0.25 * TUNNEL;
        let swell = swell(x);
        let (wide, high) = tunnel_stretch(x);
        rings.push(Some(verts.len() as u32));
        for outer in [false, true] {
            for i in 0..n {
                let a = -0.2 * PI + i as f32 / AROUND as f32 * 1.4 * PI;
                let (across, up, shade) = if outer {
                    let across = TUNNEL * wide * 1.9 + 1.7 * swell * a.cos().max(0.0);
                    let up = MASSIF * share * (0.85 + 0.3 * vnoise(x * 0.4, a, 5));
                    // The massif's face is rock too: a broad, slowly changing
                    // tone, so the cliff about each mouth is not one smooth
                    // slab between the openings.
                    let shade = 0.86 + 0.30 * vnoise(x * 1.1, a * 2.0, 29);
                    (across, up, shade)
                } else {
                    // Rougher than the radius the boat keeps clear of, but
                    // only ever outward from it; with ledges where one flow
                    // of lava lies on the last, and darker in the hollows.
                    let r = seabed.tunnel_radius_with_nooks(x, a);
                    let rough =
                        0.06 * vnoise(x * 7.0, a * 6.0, 21) + 0.03 * vnoise(x * 17.0, a * 13.0, 24);
                    let ledge01 = smooth(0.55, 0.7, vnoise(x * 1.3, a * 4.0, 25));
                    // A lava tube's benches: where the flow that cut it
                    // stood for a while, its crust lined the walls, and the
                    // wall above that line stands back from the shelf. Two
                    // of them, wandering in height along the tunnel, on the
                    // walls only, so the arch the stalactites hang from is
                    // where it was.
                    let height = 0.85 * high * r * a.sin();
                    let walls = smooth(0.62, 0.45, a.sin());
                    let bench = |at: f32| smooth(at - 0.04, at + 0.04, height);
                    let benches = 0.05 * bench(-0.05 + 0.10 * vnoise(x * 0.7, 3.0, 30))
                        + 0.04 * bench(0.25 + 0.12 * vnoise(x * 0.5, 7.0, 30));
                    let ledge = 0.05 * ledge01 + benches * walls;
                    // The rock's own shade, baked per vertex: lava flow on
                    // flow, pale ledges over pocked hollows and soot
                    // collecting under the vault. The fragment shader only
                    // multiplies it with the sampled texture; the fine grain
                    // is the texture's, so the tunnel stays cheap to rebuild.
                    let coarse = vnoise(x * 1.6 + 3.1, a * 2.2, 23);
                    let pits = smooth(0.045, 0.085, rough);
                    let soot = smooth(0.0, 0.9, a.sin());
                    let shade = (1.0 - 3.0 * rough)
                        * (0.80 + 0.80 * coarse)
                        * (1.0 - 0.18 * pits)
                        * (0.86 + 0.28 * ledge01)
                        * (1.0 - 0.16 * soot);
                    let r = r + rough + ledge;
                    (wide * r, 0.85 * high * r, shade)
                };
                let p = [
                    c[0] + lat[0] * across * a.cos(),
                    c[1] + up * a.sin(),
                    c[2] + lat[2] * across * a.cos(),
                ];
                verts.push(Vertex {
                    pos: p,
                    normal: [0.0; 3],
                    colour: if outer { [0.0; 3] } else { spill(p) },
                    aux: [p[1] - floor, shade.max(0.3)],
                });
            }
        }
    }
    // Whether the inner quad from ring k to the next, from point i round to
    // the next, is cut away for a passage's mouth.
    let holed = |k: i32, i: usize| {
        passages.iter().any(|p| {
            let (k0, k1) = p.opening.rings();
            (k0..k1).contains(&k)
                && if p.opening.side() {
                    i < SIDE_TOP
                } else {
                    (CHIMNEY_AROUND.0..CHIMNEY_AROUND.1).contains(&i)
                }
        })
    };
    // A mouth: the cliff face between the arch and the dome, with vertices
    // of its own, so its edge stays sharp.
    let cap = |verts: &mut Vec<Vertex>, indices: &mut Vec<u32>, ring: u32| {
        let base = verts.len() as u32;
        for i in 0..2 * n {
            let v = verts[ring as usize + i];
            verts.push(Vertex {
                normal: [0.0; 3],
                ..v
            });
        }
        for i in 0..n32 - 1 {
            indices.extend_from_slice(&[
                base + i,
                base + i + 1,
                base + n32 + i + 1,
                base + i,
                base + n32 + i + 1,
                base + n32 + i,
            ]);
        }
    };
    for j in 0..rings.len() {
        let k = from + j as i32;
        let previous = if j > 0 { rings[j - 1] } else { None };
        match (previous, rings[j]) {
            (Some(p), Some(b)) => {
                for half in [0, n32] {
                    for i in 0..n32 - 1 {
                        if half == 0 && holed(k - 1, i as usize) {
                            continue;
                        }
                        let (a, bb) = (p + half + i, p + half + i + 1);
                        let (c, d) = (b + half + i, b + half + i + 1);
                        indices.extend_from_slice(&[a, c, bb, bb, c, d]);
                    }
                }
            }
            (None, Some(b)) => cap(verts, indices, b),
            (Some(p), None) => cap(verts, indices, p),
            (None, None) => {}
        }
    }
    if let Some(&Some(b)) = rings.last() {
        cap(verts, indices, b);
    }
    // The passages, lofted from the edge of each mouth into the rock.
    for p in passages {
        let o = p.opening;
        let (k0, k1) = o.rings();
        let Some(ring) = (k0..=k1)
            .map(|k| rings[(k - from) as usize])
            .collect::<Option<Vec<u32>>>()
        else {
            continue;
        };
        let ring = |k: i32| ring[(k - k0) as usize];
        let mut edge: Vec<u32> = Vec::new();
        if o.side() {
            edge.extend((0..=SIDE_TOP as u32).map(|i| ring(k0) + i));
            edge.extend((k0 + 1..k1).map(|k| ring(k) + SIDE_TOP as u32));
            edge.extend((0..=SIDE_TOP as u32).rev().map(|i| ring(k1) + i));
        } else {
            let (i0, i1) = (CHIMNEY_AROUND.0 as u32, CHIMNEY_AROUND.1 as u32);
            edge.extend((i0..=i1).map(|i| ring(k0) + i));
            edge.extend((k0 + 1..k1).map(|k| ring(k) + i1));
            edge.extend((i0..=i1).rev().map(|i| ring(k1) + i));
            edge.extend((k0 + 1..k1).rev().map(|k| ring(k) + i0));
        }
        let closed = !o.side();
        let m = edge.len();
        let points: Vec<[f32; 3]> = edge.iter().map(|&e| verts[e as usize].pos).collect();
        // Where round the passage's section each point of the edge goes: by
        // its length along the edge for an arch, by its bearing for a ring.
        let angles: Vec<f32> = if closed {
            points
                .iter()
                .map(|q| {
                    let across = (q[2] - corridor(q[0]) - o.crown[0]) / 0.22;
                    across.atan2((q[0] - o.x) / o.half)
                })
                .collect()
        } else {
            let mut along = vec![0.0f32];
            for j in 1..m {
                along.push(along[j - 1] + distance(points[j], points[j - 1]));
            }
            let total = along[m - 1].max(1e-4);
            along
                .iter()
                .map(|d| 1.2 * PI - d / total * 1.4 * PI)
                .collect()
        };
        let start: Vec<[f32; 3]> = angles.iter().map(|&a| p.wall(0.0, a)).collect();
        let (light, strength) = o.kind.light();
        let segments = if closed { m } else { m - 1 };
        let mut previous = edge;
        const STEPS: usize = 12;
        for step in 1..=STEPS {
            let s = step as f32 / STEPS as f32;
            // The mouth's own ragged edge, eased into the passage's section.
            let keep = 1.0 - smooth(0.0, 0.45, s);
            let centre = p.centre(s);
            let base = verts.len() as u32;
            for j in 0..m {
                let a = angles[j];
                let mut q = p.wall(s, a);
                if o.kind != Secret::Door {
                    let r = 0.88 + 0.24 * vnoise(s * 5.0 + o.x * 3.0, a * 2.5, 22);
                    q = lerp3(centre, q, r);
                }
                let q = [0, 1, 2].map(|i| q[i] + (points[j][i] - start[j][i]) * keep);
                // Its own light, the brighter the deeper in: in spots where
                // luminous things have settled, in glints in a crystal seam.
                let mut e = smooth(0.0, 0.75, s).powf(0.8) * strength;
                let spots = match o.kind {
                    Secret::Grotto => {
                        0.25 + 1.8
                            * smooth(0.6, 0.72, vnoise(q[0] * 9.0, q[1] * 9.0 + q[2] * 5.0, 26))
                    }
                    Secret::Crystal => {
                        0.4 + 1.4
                            * smooth(0.6, 0.75, vnoise(q[0] * 12.0 + q[2] * 7.0, q[1] * 12.0, 27))
                    }
                    _ => 1.0,
                };
                // Evenly at the far end, which shades smoothly into it.
                e *= spots + (1.0 - spots) * smooth(0.75, 1.0, s);
                verts.push(Vertex {
                    pos: q,
                    normal: [0.0; 3],
                    colour: light.map(|c| c * e),
                    aux: [q[1] - p.floor, 0.45 + 0.1 * s],
                });
            }
            for j in 0..segments {
                let j2 = (j + 1) % m;
                let (a, b) = (previous[j], previous[j2]);
                let (c, d) = (base + j as u32, base + j2 as u32);
                indices.extend_from_slice(&[a, c, b, b, c, d]);
            }
            previous = (0..m as u32).map(|j| base + j).collect();
        }
        // The far end, round the bend, where the light comes from.
        let on = norm({
            let (a, b) = (p.centre(0.9), p.centre(1.0));
            [b[0] - a[0], b[1] - a[1], b[2] - a[2]]
        });
        let middle = previous.iter().fold([0.0f32; 3], |acc, &v| {
            add(acc, verts[v as usize].pos.map(|c| c / m as f32))
        });
        let tip = add(middle, on.map(|c| c * 0.25));
        let tip_index = verts.len() as u32;
        verts.push(Vertex {
            pos: tip,
            normal: [0.0; 3],
            colour: light.map(|c| c * strength * 1.2),
            aux: [tip[1] - p.floor, 0.5],
        });
        // All the way round, across the floor too, so no gap shows under it.
        for j in 0..m {
            let j2 = (j + 1) % m;
            indices.extend_from_slice(&[previous[j], previous[j2], tip_index]);
        }
    }
    // Smooth normals, which the shader turns toward the viewer.
    let mut normals = vec![[0.0f32; 3]; verts.len()];
    for t in indices.as_chunks::<3>().0 {
        let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
        let (pa, pb, pc) = (verts[a].pos, verts[b].pos, verts[c].pos);
        let e1 = [pb[0] - pa[0], pb[1] - pa[1], pb[2] - pa[2]];
        let e2 = [pc[0] - pa[0], pc[1] - pa[1], pc[2] - pa[2]];
        let nn = cross(e1, e2);
        for i in [a, b, c] {
            for j in 0..3 {
                normals[i][j] += nn[j];
            }
        }
    }
    for (v, nn) in verts.iter_mut().zip(normals) {
        v.normal = norm(nn);
    }
}

/// What lives in the passages, placed each frame: the glow deep in each,
/// and embers, luminous colonies, crystals, a carved doorway or a shaft of
/// pale light falling from the vault.
fn passages(w: &World, f: &mut Frame) {
    let t = w.t;
    for n in 0..f.passages.len() {
        let p = f.passages[n];
        let o = p.opening;
        if !w.visible(p.centre(0.5), 1.5) {
            continue;
        }
        let (light, _) = o.kind.light();
        let tint = |k: f32| light.map(|c| c * k);
        let id = (o.x * 10.0).round() as i32;
        // The water in a side passage's mouth, hazed with its light, so it
        // shows from along the tunnel as well as from in front. The crystal
        // gallery is luminous enough on its own and its mouth is kept clear
        // of haze, so the points stay the focus.
        if o.side() {
            let (size, strength) = if o.kind == Secret::Crystal {
                (0.20, 0.07)
            } else {
                (0.32, 0.16)
            };
            f.sprite(p.centre(0.12), size, tint(strength), GLOW);
        }
        let h = |k: i32, salt: u32| hash(id, k, salt);
        match o.kind {
            Secret::Ember => {
                let flicker = 0.8 + 0.2 * (t * 2.3 + o.x).sin() * (t * 3.7).sin();
                f.sprite(p.centre(0.85), 0.6, tint(0.5 * flicker), GLOW);
                f.light_flicker(p.centre(0.45), 0.55, tint(1.0), 0.0);
                // Embers drifting out on the warm water, rising as they go.
                for k in 0..12 {
                    let along = (t * 0.06 + h(k, 801)).fract();
                    let s = 1.0 - along;
                    let mut q = lerp3(p.centre(s), p.wall(s, PI * (0.15 + 0.7 * h(k, 802))), 0.6);
                    q[1] += along * 0.35;
                    let glow = smooth(0.0, 0.2, along) * (1.0 - along);
                    f.sprite(q, 0.014, [1.0 * glow, 0.42 * glow, 0.08 * glow], EMBER);
                }
            }
            Secret::Grotto => {
                f.sprite(p.centre(0.8), 0.5, tint(0.22), GLOW);
                f.light(p.centre(0.45), 0.5, tint(0.5));
                // Colonies of luminous creatures on the walls, each breathing
                // its light in its own time.
                for k in 0..22 {
                    let s = 0.15 + 0.85 * h(k, 811);
                    let a = PI * (-0.1 + 1.2 * h(k, 812));
                    let q = lerp3(p.centre(s), p.wall(s, a), 0.92);
                    let pulse = 0.5 + 0.5 * (t * (0.5 + 0.6 * h(k, 813)) + k as f32).sin();
                    f.sprite(q, 0.02 + 0.03 * h(k, 814), tint(0.9 * pulse * pulse), GLOW);
                }
            }
            Secret::Crystal => {
                // A violet fill in the seam and a dim warm key from the
                // mouth: the points get facets from both, the basalt round
                // them keeps its own colour, and neither light floods the
                // passage.
                f.light(p.centre(0.45), 0.26, [0.10, 0.08, 0.22]);
                f.light(p.centre(0.28), 0.26, [0.10, 0.08, 0.055]);
                let basalt = [0.30, 0.29, 0.31];
                let rocks = [Model::Rock0, Model::Rock2, Model::Rock1];
                // A point growing out of a pocket of the far wall, aimed up
                // and across the mouth so the window sees a long face rather
                // than an end; a slab of basalt hides its foot.
                let sprout = |f: &mut Frame,
                              k: i32,
                              model: Model,
                              s: f32,
                              a: f32,
                              length: f32,
                              width: f32,
                              up: f32,
                              lateral: f32,
                              glow: f32| {
                    // Only past where the mouth's own wider edge has eased
                    // into the passage's section (`build_tunnel`), so no point
                    // stands in the open water of a wide mouth.
                    let s = 0.45 + 0.55 * s;
                    let wall = p.wall(s, a);
                    let c = p.centre(s);
                    let inward = norm([c[0] - wall[0], 0.0, c[2] - wall[2]]);
                    let base = [
                        wall[0] - inward[0] * 0.02,
                        wall[1] - 0.015,
                        wall[2] - inward[2] * 0.02,
                    ];
                    let side = (h(k, 823) - 0.5) * lateral;
                    let dir = norm([
                        inward[0] * 0.45 + side,
                        inward[1] * 0.45 + up,
                        inward[2] * 0.45,
                    ]);
                    let tip = add(base, dir.map(|v| v * length));
                    // Never below the passage's lip, where the point would
                    // grow through the rock.
                    let tip = [tip[0], tip[1].max(p.floor + 0.10), tip[2]];
                    f.glass(model, between(base, tip, width), [0.42, 0.34, 0.95])
                        .material(material::CRYSTAL)
                        .glowing(glow);
                    // The basalt the point grew out of: small, flat, half-
                    // buried lumps, so the seam reads as rock splitting
                    // rather than hovering glass.
                    let tangent = norm(cross([0.0, 1.0, 0.0], inward));
                    let count = if model == Model::Crystal { 2 } else { 1 };
                    for n in 0..count {
                        let r = (0.009 + 0.007 * h(k, 840 + n))
                            * if model == Model::Crystal { 1.25 } else { 1.0 };
                        let spread = if count == 1 { 0.0 } else { n as f32 - 0.5 };
                        let q = [
                            base[0] - inward[0] * r * 1.0 + tangent[0] * spread * r * 3.0,
                            base[1] - r * 0.20,
                            base[2] - inward[2] * r * 1.0 + tangent[2] * spread * r * 3.0,
                        ];
                        f.put(
                            rocks[(k + n as i32).rem_euclid(3) as usize],
                            place(
                                q,
                                h(k, 842 + n) * TAU,
                                0.0,
                                h(k, 844 + n) * TAU,
                                [r * 1.9, r * (0.70 + 0.25 * h(k, 846 + n)), r * 1.3],
                            ),
                            basalt,
                        )
                        .textured(Surface::Basalt, 6.0);
                    }
                    // The point catches the passage's light and the
                    // searchlight now and then. Only the heroes and every
                    // third cluster glint, so the wall is not a sky of sparks.
                    if model == Model::Crystal || k % 3 == 0 {
                        let twinkle = (t * 1.3 + k as f32 * 2.1).sin().max(0.0).powi(8);
                        let warm = 0.8 + 1.1 * twinkle;
                        f.sprite(
                            tip,
                            0.016 + 0.010 * h(k, 831),
                            [warm, 0.74 * warm, 0.64 * warm],
                            GLOW,
                        );
                    }
                };
                // Three heroes: a tall point at the mouth, one on the far
                // wall and one standing out of the seam a little way in.
                for (k, s, a, length, width, up, lateral, glow) in [
                    (0, 0.22, 0.32, 0.19, 0.034, 1.05, 0.70, 0.32),
                    (1, 0.40, 0.70, 0.18, 0.030, 0.95, 0.80, 0.30),
                    (2, 0.60, 0.20, 0.16, 0.028, 1.20, 0.70, 0.28),
                ] {
                    sprout(f, k, Model::Crystal, s, a, length, width, up, lateral, glow);
                }
                // Smaller points packed around the heroes and along the seam,
                // their sizes and bearings varied by hash. They are spread
                // down the passage and up the far wall so the gallery reads
                // as separate pockets, not one bright column.
                for (k, s, a) in [
                    (10, 0.14, 0.20),
                    (11, 0.20, 0.55),
                    (12, 0.29, 0.32),
                    (13, 0.36, 0.72),
                    (14, 0.45, 0.42),
                    (15, 0.54, 0.18),
                    (16, 0.64, 0.60),
                    (17, 0.78, 0.30),
                ] {
                    let length = 0.10 + 0.05 * h(k, 825);
                    let width = 0.060 + 0.030 * h(k, 826);
                    let up = 0.55 + 0.55 * h(k, 832);
                    let lateral = 0.55 + 0.55 * h(k, 833);
                    let glow = 0.18 + 0.12 * h(k, 834);
                    sprout(
                        f,
                        k,
                        Model::CrystalCluster,
                        s,
                        a,
                        length,
                        width,
                        up,
                        lateral,
                        glow,
                    );
                }
            }
            Secret::Door => {
                f.sprite(p.centre(0.9), 0.45, tint(0.32), GLOW);
                f.light(p.centre(0.5), 0.6, tint(0.7));
                // A doorway of dressed stone, set a little way in, and steps
                // up into the light.
                let into = o.wall + o.depth * 0.3;
                // Along the passage's bend, `at` out from the corridor.
                let along = |at: f32| o.path((at - o.wall) / o.depth).0[0];
                let door = along(into);
                let stone = [0.24, 0.23, 0.21];
                let block = |f: &mut Frame, at: [f32; 3], size: [f32; 3]| {
                    f.put(Model::Box, place(p.at(at), 0.0, 0.0, 0.0, size), stone)
                        .textured(Surface::Marble, 3.0);
                };
                for side in [-1.0, 1.0] {
                    block(f, [door + side * o.half, into, 0.34], [0.05, 0.37, 0.07]);
                }
                block(f, [door, into, 0.72], [o.half + 0.1, 0.06, 0.08]);
                for k in 0..3 {
                    let top = 0.05 * (k + 1) as f32;
                    let at = into + 0.12 + 0.13 * k as f32;
                    block(
                        f,
                        [along(at), at, top * 0.5 - 0.02],
                        [o.half * 0.95, top * 0.5 + 0.02, 0.07],
                    );
                }
                // Signs cut in the lintel, catching the light.
                for k in 0..5 {
                    let x = door + (k as f32 - 2.0) * o.half * 0.4;
                    let shine = 0.5 + 0.5 * (t * 0.8 + k as f32 * 1.7).sin();
                    f.sprite(
                        p.at([x, into - 0.085, 0.72]),
                        0.016,
                        tint(0.5 + 0.8 * shine),
                        GLOW,
                    );
                }
            }
            Secret::Chimney => {
                let top = p.centre(0.05);
                let foot = [top[0], p.floor + 0.05, top[2]];
                let breathe = 0.75 + 0.25 * (t * 0.4 + o.x).sin();
                let k = 0.3 * breathe;
                f.shafts.push(Sprite {
                    place: [top[0], top[1], top[2], 0.08],
                    // w: lit by its own light, not the sun's.
                    tint: [light[0] * k, light[1] * k, light[2] * k, 1.0],
                    stretch: [foot[0] - top[0], foot[1] - top[1], foot[2] - top[2], 0.0],
                });
                f.sprite(p.centre(0.7), 0.4, tint(0.35), GLOW);
                f.light([top[0], p.floor + 0.3, top[2]], 0.6, tint(0.45));
                // Motes sinking through the shaft.
                for k in 0..14 {
                    let fall = (t * 0.03 + h(k, 831)).fract();
                    let q = [
                        top[0] + (h(k, 832) - 0.5) * 0.18,
                        top[1] + (foot[1] - top[1]) * fall,
                        top[2] + (h(k, 833) - 0.5) * 0.18,
                    ];
                    let glow = smooth(0.0, 0.15, fall) * smooth(1.0, 0.8, fall);
                    f.sprite(q, 0.01, tint(0.6 * glow), GLOW);
                }
            }
        }
    }
}

/// Black smokers in the abyss, steaming vents in the lava cave.
fn vents(w: &World, f: &mut Frame) {
    // Black smokers in the abyss and the lava cave: a craggy hollow chimney
    // with a mineral lip, a small hot mouth and a pale mineral plume.
    for &(ix, iz, x, z) in &cells(w, 2.2, 1.0, 14.0, 130) {
        let b = w.biome(x, z);
        let hot = (b.weight(ABYSS) + b.weight(CAVE)) as f32;
        if hot < 0.5 || hash(ix, iz, 131) < 0.4 {
            continue;
        }
        f.fade = fade(hot, 0.5);
        let y = w.ground(x, z);
        let h = 0.28 + 0.32 * hash(ix, iz, 132);
        let d = [x - w.camera(), z - w.pose.pos[2]];
        if d[0] * d[0] + d[1] * d[1] < 0.55 * 0.55 {
            continue;
        }
        if !w.visible([x, y + h * 0.5, z], h + 0.5) {
            continue;
        }
        let show = f.fade;
        f.put(
            Model::Vent,
            place(
                [x, y - 0.03, z],
                hash(ix, iz, 133) * TAU,
                0.0,
                0.0,
                [0.17, h, 0.17],
            ),
            [0.50, 0.45, 0.36],
        )
        .grounded();
        let top = [x, y + h * 0.97, z];
        f.sprite(top, 0.035, [0.7 * show, 0.28 * show, 0.07 * show], GLOW);
        f.light(top, 0.5, [0.7 * show, 0.28 * show, 0.08 * show]);
        // The plume: a translucent, widening column of mineral-rich water
        // rather than a string of sparks.
        for k in 0..8 {
            let rise = (w.t * 0.22 + k as f32 / 8.0).fract();
            let drift = 0.08 * rise * (rise * 6.0 + hash(ix, iz, 134) * 5.0).sin();
            let p = add(
                top,
                [
                    drift,
                    rise * 0.65,
                    0.05 * rise * (rise * 4.0 + ix as f32).cos(),
                ],
            );
            let fade = (1.0 - rise) * show;
            f.sprite(
                p,
                0.05 + rise * 0.16,
                [0.30 * fade, 0.38 * fade, 0.40 * fade],
                GLOW,
            );
        }
        for k in 0..3 {
            let a = hash(ix, iz, 610 + k) * TAU;
            let r = 0.10 + 0.08 * hash(ix, iz, 614 + k);
            let root = [x + a.cos() * r, y - 0.01, z + a.sin() * r];
            let len = 0.14 + 0.20 * hash(ix, iz, 618 + k);
            let tip = [root[0], root[1] + len, root[2]];
            f.put(
                Model::Cylinder,
                between(root, tip, 0.0045),
                [0.42, 0.37, 0.29],
            )
            .moving(anim::SWAY, a, 0.01, 0.7)
            .grounded();
            f.sprite(tip, 0.013, [0.55 * show, 0.12 * show, 0.10 * show], GLOW);
        }
    }
    // The vents field: taller chimneys, mineral crusts and drifting plumes.
    for &(ix, iz, x, z) in &cells(w, 1.3, 1.0, 15.0, 600) {
        let here = w.share(x, z, VENTS);
        let needs = hash(ix, iz, 601) * 0.55 + 0.25;
        if here < needs {
            continue;
        }
        let y = w.ground(x, z);
        let h = 0.40 + 0.65 * hash(ix, iz, 602);
        let d = [x - w.camera(), z - w.pose.pos[2]];
        if d[0] * d[0] + d[1] * d[1] < 1.0 * 1.0 {
            continue;
        }
        if !w.visible([x, y + h * 0.5, z], h + 0.5) {
            continue;
        }
        f.fade = fade(here, needs);
        let show = f.fade;
        let yaw = hash(ix, iz, 603) * TAU;
        f.put(
            Model::Vent,
            place(
                [x, y - 0.04, z],
                yaw,
                0.0,
                0.0,
                [
                    0.17 + 0.08 * hash(ix, iz, 604),
                    h,
                    0.17 + 0.08 * hash(ix, iz, 605),
                ],
            ),
            [0.48, 0.42, 0.34],
        )
        .grounded();
        // The pale mineral crust the hot water has precipitated round its foot.
        f.put(
            Model::Sphere,
            place([x, y + 0.003, z], 0.0, 0.0, 0.0, [0.20, 0.004, 0.18]),
            [0.26, 0.24, 0.13],
        )
        .grounded();
        let top = [x, y + h * 0.97, z];
        f.sprite(top, 0.045, [0.65 * show, 0.30 * show, 0.08 * show], GLOW);
        if hash(ix, iz, 606) < 0.5 {
            f.light(top, 0.7, [0.8 * show, 0.34 * show, 0.10 * show]);
        }
        for k in 0..12 {
            let rise = (w.t * 0.18 + k as f32 / 12.0 + hash(ix, iz, 607)).fract();
            let p = [
                top[0] + rise * 0.45 + 0.07 * (rise * 11.0 + ix as f32).sin(),
                top[1] + rise * 0.85,
                top[2] + rise * 0.20,
            ];
            let fade = (1.0 - rise) * show;
            f.sprite(
                p,
                0.06 + rise * 0.22,
                [0.32 * fade, 0.40 * fade, 0.42 * fade],
                GLOW,
            );
        }
        // Tube worms round the foot: pale stalks with red plumes.
        for k in 0..4 {
            let a = hash(ix, iz, 610 + k) * TAU;
            let r = 0.11 + 0.09 * hash(ix, iz, 614 + k);
            let root = [x + a.cos() * r, y - 0.01, z + a.sin() * r];
            let len = 0.16 + 0.24 * hash(ix, iz, 618 + k);
            let tip = [root[0], root[1] + len, root[2]];
            f.put(
                Model::Cylinder,
                between(root, tip, 0.0045),
                [0.42, 0.37, 0.29],
            )
            .moving(anim::SWAY, a, 0.01, 0.7)
            .grounded();
            f.sprite(tip, 0.013, [0.55 * show, 0.12 * show, 0.10 * show], GLOW);
        }
    }
}

/// Keels of broken ice hanging from the pack.
fn ice(w: &World, f: &mut Frame) {
    let top = w.ice();
    if top <= w.eye() {
        return;
    }
    for &(ix, iz, x, z) in &cells(w, 1.8, 1.5, 16.0, 140) {
        if w.share(x, z, ARCTIC) < 0.65 || hash(ix, iz, 141) < 0.3 {
            continue;
        }
        f.fade = fade(w.share(x, z, ARCTIC), 0.65);
        let deep = 0.15 + 0.5 * hash(ix, iz, 142);
        let wide = 0.5 + 0.6 * hash(ix, iz, 143);
        if !w.visible([x, top - deep * 0.5, z], wide) {
            continue;
        }
        let v = 0.42 + 0.1 * hash(ix, iz, 144);
        // Daylight comes through the ice, so the keels glow rather than shade.
        f.put(
            Model::Keel,
            place(
                [x, top + 0.02, z],
                hash(ix, iz, 145) * TAU,
                0.0,
                0.0,
                [wide, deep, wide * 0.5],
            ),
            [v * 0.6, v * 1.1, v * 1.25],
        )
        .glowing(0.9);
    }
}

/// A party of divers in Rouquayrol-Denayrouze suits walking the seabed of the
/// forest and of Atlantis, the leader carrying a Ruhmkorff lamp.
fn divers(w: &World, f: &mut Frame) {
    const WALK: f32 = 0.02;
    let cam = w.camera();
    let u = cam - WALK * w.t;
    let k = (u / 7.0).floor();
    let lead = (k + 0.5) * 7.0 + WALK * w.t;
    // The party appears out of the gloom and fades in as it comes abeam; it
    // was cut off at five units, which put a whole group of divers into the
    // glass at once.
    let near = (lead - cam).abs();
    if near > 8.0 {
        return;
    }
    let entering = smooth(8.0, 4.5, near);
    // Keep the party on the side the window looks out on, or it walks behind
    // the hull where nobody can see it.
    let side = if w.pose.basis[2][2] >= 0.0 { 1.0 } else { -1.0 };
    let b = w.biome(lead, lane(lead, 1.5, side));
    let share = (b.weight(FOREST) + b.weight(ATLANTIS)) as f32;
    if share < 0.5 || hash(k as i32, 0, 150) < 0.2 {
        return;
    }
    let fade_in = fade(share, 0.5) * entering;
    f.fade = fade_in;
    // The party walks a lane beside the corridor and steps round what stands
    // in it: each diver treads the same path as the one in front.
    let obstacles = obstacles(w, lead - 1.6, lead + 0.8, lane(lead, 1.5, side));
    let path = |x: f32| {
        let base = lane(x, 1.5, side);
        base + obstacles.iter().map(|&o| detour(o, x, base)).sum::<f32>()
    };
    for d in 0..3 {
        let fd = d as f32;
        let x = lead - fd * 0.30;
        let z = path(x);
        let heading = -((path(x + 0.04) - path(x - 0.04)) / 0.08).atan();
        let feet = [x, w.ground(x, z), z];
        let size = 0.17 * (1.0 - fd * 0.04);
        let show = w.reveal([x, feet[1] + 0.45 * size, z], 0.5 * size);
        if show <= 0.001 {
            continue;
        }
        // Feet stay planted: a full stride covers twice a step of the legs.
        let stride = x / (size * 0.56) * TAU + fd * 1.9;
        f.fade = fade_in * show;
        let mark = f.mark();
        diver(w, f, feet, size, stride, d == 0);
        f.turn_since(&mark, feet, heading);
    }
    f.fade = fade_in;
}

/// What a diver must walk round near `lane` between `x0` and `x1`: boulders,
/// tree trunks and columns, as (x, z, radius). These are the same cells that
/// `rocks`, `forest` and `ruins` place.
fn obstacles(w: &World, x0: f32, x1: f32, lane: f32) -> Vec<(f32, f32, f32)> {
    let mut found = Vec::new();
    let mut scan =
        |size: f32, salt: u32, keep: &dyn Fn(i32, i32, f32, f32) -> Option<(f32, f32, f32)>| {
            for iz in ((lane - 1.2) / size).floor() as i32..=((lane + 1.2) / size).ceil() as i32 {
                for ix in ((x0 - 0.6) / size).floor() as i32..=((x1 + 0.6) / size).ceil() as i32 {
                    let (x, z) = cell_point(ix, iz, size, salt);
                    found.extend(keep(ix, iz, x, z));
                }
            }
        };
    scan(0.55, 10, &|ix, iz, x, z| {
        if hash(ix, iz, 12) < 0.55 {
            return None;
        }
        let size = 0.05 + 0.20 * hash(ix, iz, 13).powf(2.0);
        (size > 0.06 && !(size > 0.1 && (z - corridor(x)).abs() < 0.7)).then_some((
            x,
            z,
            size * 1.2,
        ))
    });
    scan(0.75, 30, &|ix, iz, x, z| {
        let grows = w.share(x, z, FOREST) >= hash(ix, iz, 31) * 0.7 + 0.2;
        (grows && (z - corridor(x)).abs() >= 0.6).then_some((x, z, 0.12))
    });
    // Columns stand at their cell's centre, in rows.
    scan(0.55, 70, &|ix, iz, _, _| {
        let (x, z) = ((ix as f32 + 0.5) * 0.55, (iz as f32 + 0.5) * 0.55);
        let stands = hash(0, iz, 71) >= 0.45
            && w.share(x, z, ATLANTIS) >= hash(ix, iz, 72) * 0.5 + 0.4
            && hash(ix, iz, 73) >= 0.15
            && (z - corridor(x)).abs() >= 0.5;
        stands.then_some((x, z, 0.13))
    });
    found
}

/// How far a path along `base` swings aside at `x` to pass obstacle `o`:
/// smoothly, starting a little before it and back after.
fn detour(o: (f32, f32, f32), x: f32, base: f32) -> f32 {
    let (ox, oz, r) = o;
    let reach = r + 0.4;
    let u = (x - ox) / reach;
    if u.abs() >= 1.0 {
        return 0.0;
    }
    let clearance = r + 0.18 - (base - oz).abs();
    if clearance <= 0.0 {
        return 0.0;
    }
    let away = if base >= oz { 1.0 } else { -1.0 };
    away * clearance * (0.5 + 0.5 * (PI * u).cos())
}

fn diver(w: &World, f: &mut Frame, feet: [f32; 3], s: f32, stride: f32, lamp: bool) {
    let show = f.fade;
    let at = |dx: f32, dy: f32, dz: f32| [feet[0] + dx * s, feet[1] + dy * s, feet[2] + dz * s];
    let bob = 0.012 * stride.cos().abs();
    let hip = at(0.0, 0.44 + bob, 0.0);
    // The suit and helmet are shared meshes, placed once for each diver; the
    // limbs are the same fleet of meshes stretched between the joints the old
    // stride stepped, so the boots still plant where the walk puts them.
    f.put(
        Model::Diver,
        place(
            [feet[0], feet[1] + bob * s, feet[2]],
            0.0,
            0.0,
            0.0,
            [s, s, s],
        ),
        [1.0; 3],
    );
    f.put(
        Model::DiverHelmet,
        place(
            [feet[0], feet[1] + bob * s, feet[2]],
            0.0,
            0.0,
            0.0,
            [s, s, s],
        ),
        [1.0; 3],
    )
    .material(material::COPPER);
    for side in [-1.0f32, 1.0] {
        let ph = stride + if side > 0.0 { PI } else { 0.0 };
        let thigh = 0.34 * ph.sin();
        let knee = [
            hip[0] + thigh.sin() * 0.22 * s,
            hip[1] - thigh.cos() * 0.22 * s,
            hip[2] + side * 0.05 * s,
        ];
        let bend = 0.45 * (ph + 1.3).sin().max(0.0);
        let foot = [
            knee[0] + (thigh - bend).sin() * 0.2 * s,
            knee[1] - (thigh - bend).cos() * 0.2 * s,
            knee[2],
        ];
        f.put(
            Model::DiverLimb,
            between([hip[0], hip[1], knee[2]], knee, 0.05 * s),
            [1.0; 3],
        );
        f.put(Model::DiverLimb, between(knee, foot, 0.045 * s), [1.0; 3]);
        f.put(
            Model::DiverBoot,
            place(
                [foot[0] + 0.02 * s, foot[1], foot[2]],
                0.0,
                0.0,
                0.0,
                [s, s, s],
            ),
            [1.0; 3],
        );
    }
    let shoulder = at(0.03, 0.71 + bob, 0.08);
    let swing = 0.07 * (stride + PI).sin();
    f.put(
        Model::DiverLimb,
        between(
            at(-0.01, 0.71 + bob, -0.08),
            at(-0.04 + swing, 0.46 + bob, -0.08),
            0.034 * s,
        ),
        [1.0; 3],
    );
    if lamp {
        let elbow = at(0.10, 0.57 + bob, 0.08);
        let hand = at(0.19, 0.58 + bob, 0.08);
        f.put(
            Model::DiverLimb,
            between(shoulder, elbow, 0.036 * s),
            [1.0; 3],
        );
        f.put(Model::DiverLimb, between(elbow, hand, 0.032 * s), [1.0; 3]);
        let light = at(0.24, 0.55 + bob, 0.08);
        f.put(
            Model::Box,
            place(light, 0.0, 0.0, 0.0, [0.03 * s, 0.04 * s, 0.03 * s]),
            [0.16, 0.15, 0.14],
        );
        f.sprite(light, 0.04, [show, 0.94 * show, 0.78 * show], GLOW);
        f.sprite(
            add(light, [0.06, -0.02, 0.0]),
            0.25,
            [0.12 * show, 0.11 * show, 0.09 * show],
            GLOW,
        );
        f.light(
            add(light, [0.05, -0.02, 0.0]),
            0.5,
            [1.4 * show, 1.3 * show, 1.05 * show],
        );
    } else {
        let elbow = at(0.06, 0.55 + bob, 0.08);
        let hand = at(0.15, 0.55 + bob, 0.08);
        f.put(
            Model::DiverLimb,
            between(shoulder, elbow, 0.036 * s),
            [1.0; 3],
        );
        f.put(Model::DiverLimb, between(elbow, hand, 0.032 * s), [1.0; 3]);
        f.put(
            Model::Cylinder,
            between(
                at(-0.02, 0.50 + bob, 0.08),
                at(0.36, 0.63 + bob, 0.08),
                0.014 * s,
            ),
            [0.16, 0.15, 0.14],
        );
    }
    // A trail of bubbles from the helmet's exhaust: each is let go where it
    // was, rises and fades in and out through its own life, so the trail
    // never snaps back to the helmet.
    let exhaust = at(-0.056, 0.918 + bob, 0.044);
    let life = 2.2;
    for b in 0..5 {
        let bubble = bubble(b, 5, w.t + feet[0] * 0.37, life);
        let rise = bubble.age / life;
        let q = add(exhaust, [-0.05 * rise, 0.1 * s + rise * 0.35, 0.01 * rise]);
        let fade = bubble.fade * window_fade(w, q, 0.012) * show;
        if fade > 0.001 {
            f.sprite(
                q,
                0.008 + rise * 0.006,
                [0.40 * fade, 0.62 * fade, 0.62 * fade],
                BUBBLE,
            );
        }
    }
}

/// A species of schooling fish: where it lives, how big it is and how fast
/// it swims through the water (signed: negative swims against the voyage).
struct Species {
    z: f32,
    depth: f32,
    size: f32,
    speed: f32,
    band: (f32, f32),
    back: [f32; 3],
    gate: [f32; BIOMES],
    salt: u32,
}

const SPECIES: [Species; 5] = [
    // Big fish high in open water, silversides mid-water, yellow grunts low
    // among the reefs and ruins, fry right down by the seabed, and golden
    // jacks under the sargasso's rafts.
    Species {
        z: 5.0,
        depth: 2.5,
        size: 0.06,
        speed: 0.035,
        band: (0.6, 1.4),
        back: [0.22, 0.26, 0.28],
        gate: [
            1.0, 0.8, 1.0, 1.0, 0.0, 0.0, 0.5, 0.8, 0.8, 0.5, 0.1, 0.3, 0.4, 0.6,
        ],
        salt: 200,
    },
    Species {
        z: 3.0,
        depth: 1.6,
        size: 0.032,
        speed: -0.045,
        band: (0.3, 0.9),
        back: [0.30, 0.36, 0.38],
        gate: [
            1.0, 1.0, 0.8, 0.6, 0.15, 0.0, 0.8, 0.7, 1.0, 0.7, 0.2, 0.4, 0.9, 1.0,
        ],
        salt: 210,
    },
    Species {
        z: 2.0,
        depth: 1.0,
        size: 0.024,
        speed: 0.075,
        band: (0.10, 0.35),
        back: [0.48, 0.40, 0.20],
        gate: [
            1.0, 0.7, 0.3, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.4, 0.0, 0.1, 0.0, 0.7,
        ],
        salt: 220,
    },
    Species {
        z: 1.2,
        depth: 0.5,
        size: 0.016,
        speed: 0.030,
        band: (0.06, 0.25),
        back: [0.32, 0.42, 0.46],
        gate: [
            1.0, 1.0, 0.0, 0.5, 0.0, 0.0, 0.6, 0.4, 1.0, 0.6, 0.2, 0.5, 0.3, 0.4,
        ],
        salt: 230,
    },
    Species {
        z: 4.0,
        depth: 1.8,
        size: 0.040,
        speed: -0.020,
        band: (0.4, 1.0),
        back: [0.52, 0.42, 0.16],
        gate: [
            0.6, 0.2, 0.2, 0.2, 0.0, 0.0, 0.0, 0.1, 0.8, 1.0, 0.0, 0.2, 0.3, 0.2,
        ],
        salt: 240,
    },
];

/// Schools: patchy, mostly small, now and then a great cloud; densest in the
/// core, thinning to stragglers; each fish beating its tail in step with its
/// speed, the school's heading rippling through it.
fn shoals(w: &World, f: &mut Frame) {
    let cam = w.camera();
    for sp in &SPECIES {
        let spacing = 3.0;
        let u = cam - sp.speed * w.t;
        // Far enough either way that no school comes or goes in view.
        let first = ((u - 10.0) / spacing).floor() as i32;
        let last = ((u + 10.0) / spacing).ceil() as i32;
        for k in first..=last {
            let centre_x =
                (k as f32 + 0.5 + (hash(k, 0, sp.salt) - 0.5) * 0.5) * spacing + sp.speed * w.t;
            let gathering = vnoise(centre_x * 0.25, 0.0, sp.salt + 1);
            let b = w.biome(centre_x, sp.z);
            let gate = World::by_biome(&b, sp.gate);
            // The school's own chance comes and goes smoothly with the
            // gathering noise. Let it fade through the last of that chance
            // rather than switching a whole cloud of fish on.
            let chance = gathering * 0.7 + hash(k, 1, sp.salt) * 0.5;
            if gate < 0.2 || chance < 0.42 {
                continue;
            }
            let school_fade = smooth(0.30, 0.55, gate) * smooth(0.42, 0.50, chance);
            let big = hash(k, 2, sp.salt).powf(2.0);
            let rx = 0.08 + 0.75 * big;
            let ry = rx * (0.22 + 0.2 * hash(k, 3, sp.salt));
            let rz = rx * (0.4 + 0.3 * hash(k, 4, sp.salt));
            let cz = lane(
                centre_x,
                sp.z + (hash(k, 5, sp.salt) - 0.5) * sp.depth,
                side(k, sp.salt),
            );
            let floor = w.ground(centre_x, cz);
            let rise = sp.band.0 + (sp.band.1 - sp.band.0) * hash(k, 6, sp.salt);
            // The school's footprint can straddle a rise in the seabed: take
            // the highest ground under it, so the fish at its lowest are over
            // that rise too.
            let mut roof = floor;
            for (dx, dz) in [
                (-0.7, 0.0),
                (0.7, 0.0),
                (0.0, -0.7),
                (0.0, 0.7),
                (-0.55, -0.55),
                (0.55, -0.55),
                (-0.55, 0.55),
                (0.55, 0.55),
            ] {
                roof = roof.max(w.ground(centre_x + dx * rx, cz + dz * rz));
            }
            // About one body length between neighbours, coarser in a big school
            // so it never runs to more than a few hundred fish.
            let length = sp.size * 2.0;
            let volume = rx * ry * rz * 4.2;
            let g = (length * 1.6).max((volume / 260.0).cbrt());
            // How far below its centre the lowest fish of the school can swim:
            // most of the vertical radius, the school's bend, its own bob and
            // the fish's body. Set the centre that far above the roof.
            let below = 1.7 * ry + 0.2 * rx + 0.05 * g + 0.5 * sp.size + 0.04;
            let cy = roof + rise.max(below) + 0.04 * (w.t * 0.09 + k as f32 * 2.0).sin();
            let centre = [centre_x, cy, cz];
            if !w.visible(centre, rx + 0.1) {
                continue;
            }
            let (nx, ny, nz) = (
                (rx / g).ceil() as i32,
                (ry / g).ceil() as i32,
                (rz / g).ceil() as i32,
            );
            let dir = if sp.speed >= 0.0 { 1.0 } else { -1.0 };
            let fade = school_fade;
            for i in -nx..=nx {
                for j in -ny..=ny {
                    for l in -nz..=nz {
                        let r = hash(i * 31 + l, j * 17 + k, sp.salt + 7);
                        let jitter = [
                            (hash(i, j * 7 + l, sp.salt + 8) - 0.5) * 0.6,
                            (hash(i + l, j, sp.salt + 9) - 0.5) * 0.6,
                            (hash(i, j + l * 5, sp.salt + 10) - 0.5) * 0.6,
                        ];
                        let local = [
                            (i as f32 + jitter[0]) * g,
                            (j as f32 + jitter[1]) * g,
                            (l as f32 + jitter[2]) * g,
                        ];
                        let front = smooth(-rx, rx, local[0] * dir);
                        let e = ((local[0] / rx).powi(2)
                            + (local[1] / (ry * (0.75 + 0.35 * front))).powi(2)
                            + (local[2] / rz).powi(2))
                        .sqrt()
                            + (vnoise(
                                local[0] * 3.0 / rx,
                                local[2] * 3.0 / rx + k as f32,
                                sp.salt + 11,
                            ) - 0.5)
                                * 0.5;
                        let density = (0.05 + 0.90 * smooth(1.05, 0.40, e))
                            * if e < 1.6 { 1.0 } else { 0.0 }
                            * fade;
                        // Fish thin out at the school's edge by fading, not popping.
                        if r > density + 0.06 {
                            continue;
                        }
                        let body = sp.size * (0.85 + 0.3 * hash(i, j, sp.salt + 12));
                        let show = w.reveal(
                            [
                                centre[0] + local[0],
                                centre[1] + local[1],
                                centre[2] + local[2],
                            ],
                            body * 1.8,
                        );
                        if show <= 0.001 {
                            continue;
                        }
                        f.fade = smooth(0.06, -0.06, r - density) * show * fade;
                        let wob = [
                            (w.t * 0.7 + r * 20.0).sin() * 0.08 * g,
                            (w.t * 0.9 + r * 13.0).cos() * 0.05 * g,
                            (w.t * 0.6 + r * 7.0).sin() * 0.08 * g,
                        ];
                        // A bend in the school, and a wave of turning through it.
                        let bend = rx * 0.2 * (local[0] / rx * 2.0 + w.t * 0.25 + k as f32).sin();
                        let fsize = sp.size * (0.85 + 0.3 * hash(i, j, sp.salt + 12));
                        let mut p = [
                            centre[0] + local[0] + wob[0],
                            centre[1] + local[1] + bend + wob[1],
                            centre[2] + local[2] + wob[2],
                        ];
                        // Where the ground rises sharply between the samples
                        // above, lift the odd fish clear rather than let it
                        // swim into the seabed.
                        let seabed = w.ground(p[0], p[2]) + 0.5 * fsize + 0.004;
                        p[1] = p[1].max(seabed);
                        let turn = 0.12 * (w.t * 0.6 - local[0] / rx * 4.0 + k as f32).sin()
                            + (r - 0.5) * 0.2;
                        let yaw = if dir > 0.0 { turn } else { PI + turn };
                        let beat = sp.speed.abs() / (fsize * 1.4) * TAU * (0.85 + 0.3 * r);
                        let pitch = 0.08 * (w.t * 0.5 + r * 9.0).sin();
                        // A fish flashes silver as it turns into the light.
                        let flash = (w.t * 0.9 + r * 40.0 - local[0] / rx * 3.0)
                            .sin()
                            .max(0.0)
                            .powi(14);
                        f.put(Model::Fish, place(p, yaw, pitch, 0.0, [fsize; 3]), sp.back)
                            .moving(anim::SWIM, r * TAU, 0.22, beat)
                            .glowing(flash * 0.6);
                    }
                }
            }
        }
    }
}

/// A procession of creatures at `speed` through the water, one slot every
/// `spacing`: the slot centres around the camera.
fn procession(w: &World, speed: f32, spacing: f32, salt: u32) -> Vec<(i32, f32)> {
    // Far enough either way that nothing comes or goes in view.
    let reach = 10.0;
    let t = w.t;
    let u = w.camera() - speed * t;
    let first = ((u - reach) / spacing).floor() as i32;
    let last = ((u + reach) / spacing).ceil() as i32;
    (first..=last)
        .map(|k| {
            (
                k,
                (k as f32 + 0.5 + (hash(k, 0, salt) - 0.5) * 0.4) * spacing + speed * t,
            )
        })
        .collect()
}

/// Which side of the corridor procession slot `k` keeps to.
fn side(k: i32, salt: u32) -> f32 {
    if hash(k, 99, salt) < 0.5 { 1.0 } else { -1.0 }
}

/// A lane `offset` from the corridor on `side`.
fn lane(x: f32, offset: f32, side: f32) -> f32 {
    corridor(x) + side * offset
}

/// The right whale, the dolphins and the sharks.
fn great_swimmers(w: &World, f: &mut Frame) {
    // A right whale near the surface of the shallows, against the voyage.
    for (k, x) in procession(w, -0.035, 6.0, 300) {
        let z = lane(x, 4.5 + hash(k, 1, 300), side(k, 300));
        if w.share(x, z, SHALLOWS) < 0.4 || hash(k, 2, 300) < 0.3 {
            continue;
        }
        let length = 1.35;
        let y = w.surface() - 0.35 - hash(k, 3, 300) * 0.25;
        let show = w.reveal([x, y, z], length);
        if show <= 0.001 {
            continue;
        }
        f.fade = fade(w.share(x, z, SHALLOWS), 0.4) * show;
        let beat = 0.035 / (0.25 * length) * TAU;
        f.put(
            Model::Whale,
            place(
                [x, y, z],
                PI + 0.1 * (w.t * 0.1).sin(),
                0.02 * (w.t * 0.2).sin(),
                0.0,
                [length; 3],
            ),
            [0.075, 0.080, 0.088],
        )
        .moving(anim::FLUKE, k as f32, 0.035, beat);
    }
    // A pod of dolphins overtaking the boat.
    for (k, x) in procession(w, 0.13, 7.0, 310) {
        let pod = lane(x, 2.2, side(k, 310));
        if w.share(x, pod, SHALLOWS) < 0.4 || hash(k, 1, 310) < 0.3 {
            continue;
        }
        let fade_in = fade(w.share(x, pod, SHALLOWS), 0.4);
        for d in 0..5 {
            let fd = d as f32;
            let phase = hash(k, d + 2, 310) * TAU;
            let size = 0.24 * (0.85 + 0.3 * hash(k, d + 10, 310));
            let p = [
                x + 0.5 - fd * 0.25 + 0.05 * (w.t * 0.3 + phase).sin(),
                w.eye() - 0.05 + hash(k, d + 20, 310) * 0.35 + 0.04 * (w.t * 0.6 + phase).sin(),
                pod + side(k, 310) * fd * 0.15,
            ];
            let show = w.reveal(p, size);
            if show <= 0.001 {
                continue;
            }
            f.fade = fade_in * show;
            let beat = 0.13 / (0.9 * size) * TAU;
            f.put(
                Model::Dolphin,
                place(
                    p,
                    0.05 * (w.t * 0.4 + phase).sin(),
                    0.06 * (w.t * 0.6 + phase).cos(),
                    0.0,
                    [size; 3],
                ),
                [0.30, 0.35, 0.38],
            )
            .moving(anim::FLUKE, phase, 0.05, beat);
        }
    }
    // Sharks prowling the wrecks, against the voyage.
    for (k, x) in procession(w, -0.05, 2.4, 320) {
        let z = lane(x, 2.2 + hash(k, 1, 320) * 1.5, side(k, 320));
        if w.share(x, z, WRECKS) < 0.4 || hash(k, 2, 320) < 0.4 {
            continue;
        }
        let length = 0.32;
        let y = w.ground(x, z) + 0.2 + hash(k, 3, 320) * 0.4;
        let p = [x, y + 0.03 * (w.t * 0.2 + k as f32).sin(), z];
        let show = w.reveal(p, length);
        if show <= 0.001 {
            continue;
        }
        f.fade = fade(w.share(x, z, WRECKS), 0.4) * show;
        let beat = 0.05 / (0.35 * length) * TAU;
        f.put(
            Model::Shark,
            place(
                p,
                PI + 0.1 * (w.t * 0.3 + k as f32).sin(),
                0.0,
                0.0,
                [length * 0.5; 3],
            ),
            [0.30, 0.33, 0.35],
        )
        .moving(anim::SWIM, k as f32, 0.18, beat);
    }
}

/// Manta rays flying slowly through the shallows, over the wrecks, among the
/// ruins of Atlantis and over the reef, banking as they go; and green turtles
/// rowing low over the shallows, through the forest and along the reef.
fn gliders(w: &World, f: &mut Frame) {
    for (k, x) in procession(w, 0.06, 5.5, 340) {
        let z = lane(x, 1.8 + 2.2 * hash(k, 1, 340), side(k, 340));
        let share = w
            .share(x, z, SHALLOWS)
            .max(w.share(x, z, WRECKS))
            .max(w.share(x, z, ATLANTIS))
            .max(w.share(x, z, CORAL));
        if share < 0.4 || hash(k, 2, 340) < 0.35 {
            continue;
        }
        let span = 0.30 + 0.12 * hash(k, 3, 340);
        let phase = hash(k, 4, 340) * TAU;
        let y = (w.ground(x, z) + 0.6 + 0.9 * hash(k, 5, 340)).min(w.surface() - 0.35)
            + 0.08 * (w.t * 0.21 + phase).sin();
        let p = [x, y, z];
        let show = w.reveal(p, span);
        if show <= 0.001 {
            continue;
        }
        f.fade = fade(share, 0.4) * show;
        f.put(
            Model::Manta,
            place(
                p,
                0.12 * (w.t * 0.07 + phase).sin(),
                0.08 * (w.t * 0.21 + phase).cos(),
                0.22 * (w.t * 0.13 + phase).sin(),
                [span; 3],
            ),
            [0.11, 0.115, 0.125],
        )
        .moving(anim::FLUKE, phase, 0.16, 0.85);
    }
    for (k, x) in procession(w, 0.035, 4.0, 350) {
        let z = lane(x, 1.2 + 2.0 * hash(k, 1, 350), side(k, 350));
        let share = w
            .share(x, z, SHALLOWS)
            .max(w.share(x, z, FOREST))
            .max(w.share(x, z, CORAL));
        if share < 0.4 || hash(k, 2, 350) < 0.4 {
            continue;
        }
        let length = 0.20 + 0.06 * hash(k, 3, 350);
        let phase = hash(k, 4, 350) * TAU;
        let y = w.ground(x, z) + 0.25 + 0.5 * hash(k, 5, 350) + 0.04 * (w.t * 0.4 + phase).sin();
        let p = [x, y, z];
        let show = w.reveal(p, length);
        if show <= 0.001 {
            continue;
        }
        f.fade = fade(share, 0.4) * show;
        f.put(
            Model::Turtle,
            place(
                p,
                0.25 * (w.t * 0.09 + phase).sin(),
                0.05 * (w.t * 0.4 + phase).cos(),
                0.06 * (w.t * 0.3 + phase).sin(),
                [length; 3],
            ),
            [0.20, 0.17, 0.10],
        )
        .moving(anim::FLUKE, phase, 0.22, 1.5);
    }
}

/// The poulpe of chapter XVIII, met in the abyss.
fn squid(w: &World, f: &mut Frame) {
    for (k, x) in procession(w, -0.03, 4.5, 330) {
        let z = lane(x, 2.6 + hash(k, 1, 330), side(k, 330));
        if w.share(x, z, ABYSS) < 0.4 {
            continue;
        }
        let s = 1.1;
        let y = w.eye() - 0.1 + 0.1 * (w.t * 0.13 + k as f32).sin();
        let head = [x, y, z];
        let show = w.reveal(head, s);
        if show <= 0.001 {
            continue;
        }
        f.fade = fade(w.share(x, z, ABYSS), 0.4) * show;
        let jet = 0.5 + 0.5 * (w.t * 0.9).sin();
        let mottled = [0.40, 0.12, 0.08];
        // It swims mantle first, toward -x: the mantle runs from the head.
        f.put(
            Model::Mantle,
            place(head, PI, 0.0, 0.0, [s, s * (1.0 - 0.1 * jet), s]),
            mottled,
        );
        f.put(
            Model::Sphere,
            place(
                add(head, [0.07 * s, 0.0, 0.0]),
                0.0,
                0.0,
                0.0,
                [0.075 * s, 0.05 * s, 0.05 * s],
            ),
            mottled,
        );
        for side in [-1.0f32, 1.0] {
            let eye = add(head, [0.06 * s, 0.012 * s, side * 0.045 * s]);
            f.put(
                Model::Sphere,
                place(eye, 0.0, 0.0, 0.0, [0.026 * s; 3]),
                [0.70, 0.62, 0.40],
            )
            .glowing(0.15);
            f.put(
                Model::Sphere,
                place(
                    add(eye, [0.004 * s, 0.0, side * 0.018 * s]),
                    0.0,
                    0.0,
                    0.0,
                    [0.013 * s; 3],
                ),
                [0.01, 0.01, 0.015],
            );
        }
        for a in 0..10 {
            let fa = a as f32;
            let tentacle = a >= 8;
            let reach = if tentacle {
                0.70
            } else {
                0.24 + 0.16 * hash(a, k, 331)
            };
            let ang = fa / 10.0 * TAU;
            let spread = if tentacle {
                0.03
            } else {
                0.06 + 0.10 * (1.0 - jet)
            };
            let root = add(head, [0.11 * s, 0.0, 0.0]);
            let tip = add(
                root,
                [
                    reach * s,
                    ang.sin() * spread * s * 2.0,
                    ang.cos() * spread * s * 2.0,
                ],
            );
            let r = if tentacle { 0.006 * s } else { 0.014 * s };
            f.put(Model::Tube, between(root, tip, r), mottled).moving(
                anim::WRITHE,
                fa * 1.9,
                0.05 * s,
                1.6 + 0.3 * hash(a, 0, 332),
            );
        }
    }
    // Anglerfish, lit only by their lures.
    for &(ix, iz, x, z) in &cells(w, 1.4, 1.0, 6.0, 340) {
        if w.share(x, z, ABYSS) < 0.5 || hash(ix, iz, 341) < 0.5 {
            continue;
        }
        let y =
            w.ground(x, z) + 0.15 + hash(ix, iz, 342) * 0.3 + 0.03 * (w.t * 1.3 + ix as f32).sin();
        let p = [x, y, z];
        let show = w.reveal(p, 0.16);
        if show <= 0.001 {
            continue;
        }
        f.fade = fade(w.share(x, z, ABYSS), 0.5) * show;
        f.put(
            Model::Sphere,
            place(p, 0.0, 0.0, 0.0, [0.045, 0.035, 0.03]),
            [0.12, 0.10, 0.10],
        );
        let lure = add(p, [0.04, 0.03, 0.0]);
        let flicker = 0.75 + 0.25 * (w.t * 3.1 + ix as f32).sin() * (w.t * 1.7).sin();
        f.sprite(
            lure,
            0.012,
            [
                0.45 * flicker * show,
                1.0 * flicker * show,
                0.85 * flicker * show,
            ],
            GLOW,
        );
        f.sprite(
            lure,
            0.06,
            [
                0.05 * flicker * show,
                0.12 * flicker * show,
                0.10 * flicker * show,
            ],
            GLOW,
        );
        f.light(
            lure,
            0.15,
            [
                0.3 * flicker * show,
                0.8 * flicker * show,
                0.7 * flicker * show,
            ],
        );
    }
}

/// Jellyfish drifting where the water carries them. Every motion of theirs —
/// the swimming stroke, the rise and sink, the wander and the tentacles'
/// trailing wave — is the vertex shader's, from the camera clock and each
/// animal's phase, so the placer is camera-static and sits in the cache.
fn drifters(w: &World, f: &mut Frame) {
    // The wide-sea drifters: wrecks, ice and abyss, with a few straying into
    // the coral and the sargasso.
    for &(ix, iz, x, z) in &cells(w, 0.9, 1.0, 8.0, 350) {
        let here = drifter_here(w, x, z);
        let needs = hash(ix, iz, 351) * 0.8 + 0.2;
        if here < needs {
            continue;
        }
        let s = 0.040 + 0.055 * hash(ix, iz, 352);
        jelly(
            w,
            f,
            JellySite {
                ix,
                iz,
                x,
                z,
                s,
                here,
                needs,
            },
        );
    }
    // The bloom: a dense shoal of jellies over the ooze, the reason the
    // Nautilus comes this way.
    for &(ix, iz, x, z) in &cells(w, 1.0, 1.2, 9.0, 700) {
        let here = w.share(x, z, BLOOM);
        let needs = hash(ix, iz, 701) * 0.50 + 0.36;
        if here < needs {
            continue;
        }
        let s = 0.048 + 0.060 * hash(ix, iz, 702);
        jelly(
            w,
            f,
            JellySite {
                ix,
                iz,
                x,
                z,
                s,
                here,
                needs,
            },
        );
    }
}

/// How much the drifting jellies like the water at (x, z).
fn drifter_here(w: &World, x: f32, z: f32) -> f32 {
    let b = w.biome(x, z);
    (b.weight(ABYSS)
        + 0.60 * b.weight(WRECKS)
        + 0.50 * b.weight(ARCTIC)
        + 1.20 * b.weight(BLOOM)
        + 0.10 * b.weight(CORAL)
        + 0.25 * b.weight(SARGASSO)) as f32
}

/// Where one jellyfish might be, and how welcome it is there.
struct JellySite {
    ix: i32,
    iz: i32,
    x: f32,
    z: f32,
    s: f32,
    here: f32,
    needs: f32,
}

/// One kind of medusa: what its bell, arms and tentacles look like and how
/// it swims.
struct Medusa {
    /// The bell's tint, and its tentacles' and oral arms'.
    bell: [f32; 3],
    fringe: [f32; 3],
    arms: [f32; 3],
    /// The bell's size against the site's, and its height against its width.
    size: f32,
    height: f32,
    /// The tentacles' and the oral arms' length, in bell radii.
    tentacles: f32,
    arms_long: f32,
    /// How far a stroke narrows the margin, and strokes a second.
    stroke: f32,
    rate: f32,
    glow: f32,
    /// What `fs_glass` paints on the bell: how many radial canals or stripes,
    /// how strongly the gonads show through the crown, and how bright the
    /// bioluminescent spots that run round the margin are.
    pattern: [f32; 3],
}

/// The medusae of the dive. The moon jelly is the bloom's own: a flat, clear
/// saucer with four violet gonad rings and a short fringe. The lion's mane
/// is the largest, amber-red and slow, trailing long tentacles and curtains
/// of arms. The crystal jelly is clear, finely canalled and ringed with
/// green-blue light; the sea nettle is rose, striped, quick and long-armed;
/// the helmet jelly is the abyss's, a tall violet cone ringed with light.
const MEDUSAE: [Medusa; 5] = [
    Medusa {
        bell: [0.60, 0.76, 0.95],
        fringe: [0.62, 0.80, 0.92],
        arms: [0.70, 0.70, 0.95],
        size: 1.0,
        height: 0.72,
        tentacles: 0.55,
        arms_long: 0.95,
        stroke: 0.20,
        rate: 0.55,
        glow: 0.16,
        pattern: [8.0, 1.0, 0.15],
    },
    Medusa {
        bell: [0.98, 0.56, 0.24],
        fringe: [1.00, 0.42, 0.18],
        arms: [1.00, 0.64, 0.34],
        size: 1.35,
        height: 0.80,
        tentacles: 5.5,
        arms_long: 2.2,
        stroke: 0.16,
        rate: 0.32,
        glow: 0.22,
        pattern: [16.0, 0.35, 0.0],
    },
    Medusa {
        bell: [0.50, 0.86, 0.86],
        fringe: [0.40, 0.85, 0.80],
        arms: [0.52, 0.82, 0.86],
        size: 0.85,
        height: 0.70,
        tentacles: 1.7,
        arms_long: 0.6,
        stroke: 0.18,
        rate: 0.45,
        glow: 0.50,
        pattern: [32.0, 0.0, 1.0],
    },
    Medusa {
        bell: [0.94, 0.50, 0.70],
        fringe: [0.96, 0.55, 0.62],
        arms: [1.00, 0.70, 0.80],
        size: 0.95,
        height: 0.92,
        tentacles: 3.4,
        arms_long: 2.6,
        stroke: 0.24,
        rate: 0.62,
        glow: 0.28,
        pattern: [16.0, 0.25, 0.25],
    },
    Medusa {
        bell: [0.62, 0.30, 0.80],
        fringe: [0.70, 0.34, 0.62],
        arms: [0.74, 0.36, 0.70],
        size: 1.0,
        height: 1.45,
        tentacles: 1.0,
        arms_long: 0.5,
        stroke: 0.09,
        rate: 0.26,
        glow: 0.50,
        pattern: [12.0, 0.7, 0.8],
    },
];

/// One jellyfish, or now and then a comb jelly in its place. A medusa is
/// three instances placed alike — the bell, its four oral arms and its
/// fringe of tentacles — sharing one swimming phase and rate, so the
/// shader can pulse the bell and send the stroke down the arms and threads
/// in step. The bell's material is its own, so the glass pipeline can shade
/// its canals, gonads and rim separately from the crystals and the plain
/// translucent props.
fn jelly(w: &World, f: &mut Frame, site: JellySite) {
    let JellySite {
        ix,
        iz,
        x,
        z,
        s,
        here,
        needs,
    } = site;
    let phase = hash(ix, iz, 353) * TAU;
    let y = w.ground(x, z) + 0.3 + hash(ix, iz, 354) * (0.3 * z + 0.3);
    let p = [x, y, z];
    let show = w.reveal(p, s * 4.0);
    if show <= 0.001 {
        return;
    }
    f.fade = fade(here, needs) * show;
    // Which kind: the abyss keeps to helmet and crystal jellies and the comb
    // jellies' rainbows; elsewhere the moon jelly is commonest.
    let deep = w.share(x, z, ABYSS);
    let pick = hash(ix, iz, 355);
    let comb = pick < 0.12 + 0.18 * deep;
    let odds: [f32; 5] = [
        0.36 * (1.0 - deep),
        0.16 * (1.0 - deep),
        0.18 + 0.20 * deep,
        0.20 * (1.0 - deep),
        0.10 + 0.50 * deep,
    ];
    let total: f32 = odds.iter().sum();
    let mut ticket = hash(ix, iz, 362) * total;
    let kind = odds
        .iter()
        .position(|&o| {
            ticket -= o;
            ticket < 0.0
        })
        .unwrap_or(0);
    let tilt = (hash(ix, iz, 356) - 0.5) * 0.30;
    let roll = (hash(ix, iz, 357) - 0.5) * 0.30;
    if comb {
        // A comb jelly: small, clear and glowing, gliding on its cilia. Its
        // rainbow is the shader's (`material::COMB`), its glide too.
        let s = s * 0.45;
        f.glass(
            Model::Comb,
            place(p, phase, tilt, roll, [s * 0.75, s * 1.15, s * 0.75]),
            [0.72, 0.86, 1.0],
        )
        .moving(anim::GLIDE, phase, 0.25, 0.30 + 0.20 * hash(ix, iz, 358))
        .material(material::COMB)
        .glowing(0.35);
        return;
    }
    let m = &MEDUSAE[kind];
    let s = s * m.size;
    // Each animal keeps its own stroke, a little off its kind's.
    let rate = TAU * m.rate * (0.85 + 0.30 * hash(ix, iz, 358));
    let fold = |c: [f32; 3], k: f32| [c[0] * k, c[1] * k, c[2] * k];
    f.glass(
        Model::Bell,
        place(p, phase, tilt, roll, [s, s * m.height, s]),
        m.bell,
    )
    .moving(anim::PULSE, phase, m.stroke, rate)
    .material(material::JELLY)
    .glowing(m.glow)
    .patterned(m.pattern);
    f.glass(
        Model::JellyArm,
        place(p, phase, tilt, roll, [s, s * m.arms_long, s]),
        fold(m.arms, 0.9),
    )
    .moving(anim::TRAIL, phase, m.stroke, rate)
    .material(material::JELLY_SOFT)
    .glowing(m.glow * 0.45);
    let long = m.tentacles * (0.8 + 0.4 * hash(ix, iz, 360));
    f.glass(
        Model::JellyFringe,
        place(p, phase, tilt, roll, [s, s * long, s]),
        fold(m.fringe, 0.85),
    )
    .moving(anim::TRAIL, phase, m.stroke, rate)
    .material(material::JELLY_SOFT)
    .glowing(m.glow * 0.30);
    // A large jelly throws its own faint light on the water around it.
    if s > 0.13 && hash(ix, iz, 361) < 0.45 {
        f.light(p, s * 5.5, fold(m.bell, m.glow * 0.4));
    }
}

/// Under the ice: penguins flying past, and narwhals.
fn polar(w: &World, f: &mut Frame) {
    let top = w.ice();
    for (k, x) in procession(w, 0.24, 3.2, 360) {
        let flight = lane(x, 1.7, side(k, 360));
        if w.share(x, flight, ARCTIC) < 0.4 || hash(k, 1, 360) < 0.25 {
            continue;
        }
        let fade_in = fade(w.share(x, flight, ARCTIC), 0.4);
        for d in 0..4 {
            let fd = d as f32;
            let phase = hash(k, d + 2, 360) * TAU;
            let px = x + 0.4 - fd * 0.26;
            let wave = 0.06 * (px * 3.5 + phase).sin();
            let slope = 0.21 * (px * 3.5 + phase).cos();
            let p = [
                px,
                top - 0.25 - hash(k, d + 6, 360) * 0.45 + wave,
                flight + side(k, 360) * fd * 0.2,
            ];
            let show = w.reveal(p, 0.18);
            if show <= 0.001 {
                continue;
            }
            f.fade = fade_in * show;
            let s = 0.075;
            let pitch = slope.atan();
            f.put(
                Model::Penguin,
                place(p, 0.0, pitch, 0.0, [s * 2.0; 3]),
                [0.12, 0.12, 0.13],
            );
            // The flippers' beat is the shader's (anim::FLAP); the placement
            // keeps only the fixed side and the body's pitch.
            for side in [-1.0f32, 1.0] {
                let root = add(p, [0.01, 0.0, side * 0.012]);
                f.put(
                    Model::Box,
                    place(
                        add(root, [-0.02, 0.0, side * 0.02]),
                        0.0,
                        pitch,
                        0.0,
                        [0.03, 0.004, 0.022],
                    ),
                    [0.06, 0.07, 0.08],
                )
                .moving(anim::FLAP, phase, side * 0.9, TAU * 2.2);
            }
            // Bubbles shed off her flippers: the anchor is where it was let
            // go, and the shader ages the bubble from the camera clock
            // (bubble_sprite), so the trail is camera-static and cacheable.
            let was0 = px - 0.04;
            let y0 = top - 0.25 - hash(k, d + 6, 360) * 0.45 + 0.06 * (was0 * 3.5 + phase).sin();
            let anchor = [was0, y0, p[2]];
            // The same fade the other trails get: the bubbles come in with
            // the window's edge as the boat turns, rather than all at once
            // the frame the penguin's body first falls inside the cone; and
            // with the penguin itself, when its pod is fading in.
            let fade = window_fade(w, anchor, 0.012) * show;
            if fade <= 0.001 {
                continue;
            }
            for b in 0..5 {
                f.bubble_sprite(
                    anchor,
                    0.005 + 0.0015 * b as f32,
                    [0.45 * fade, 0.65 * fade, 0.70 * fade],
                    b as f32 / 5.0,
                    1.05,
                    phase,
                );
            }
        }
    }
    for (k, x) in procession(w, 0.07, 6.0, 370) {
        let pod = lane(x, 2.9, side(k, 370));
        if w.share(x, pod, ARCTIC) < 0.4 || hash(k, 1, 370) < 0.3 {
            continue;
        }
        let fade_in = fade(w.share(x, pod, ARCTIC), 0.4);
        for d in 0..3 {
            let fd = d as f32;
            let phase = hash(k, d + 2, 370) * TAU;
            let size = 0.42 * (0.85 + 0.3 * hash(k, d + 8, 370));
            let p = [
                x + 0.6 - fd * 0.55,
                top - 0.35 - hash(k, d + 12, 370) * 0.5,
                pod + side(k, 370) * fd * 0.3,
            ];
            let show = w.reveal(p, size);
            if show <= 0.001 {
                continue;
            }
            f.fade = fade_in * show;
            let beat = 0.07 / (0.5 * size) * TAU;
            // The bob and yaw are the shader's (anim::NARWHAL), with the
            // fluke's beat; the placement keeps only the pod's formation.
            f.put(
                Model::Narwhal,
                place(p, 0.0, 0.0, 0.0, [size; 3]),
                [0.30, 0.32, 0.33],
            )
            .moving(anim::NARWHAL, phase, 0.04, beat);
        }
    }
}

/// Marine snow, krill, plankton, embers and the bubbles shed by the hull.
/// Seconds between chances of meeting another submarine, and her length:
/// twenty metres.
const ENCOUNTER_EVERY: f32 = 140.0;
const SUB_LENGTH: f32 = 1.8;
/// How fast the Nautilus goes on its voyage.
const VOYAGE_SPEED: f32 = crate::voyage::SPEED as f32;

/// How fast her hull may swing, nose up or down and heel, in radians a
/// second, and how fast that swinging may gather or ease, in radians a second
/// squared: a twenty-metre boat answers her helm and planes heavily.
const YAWING: (f32, f32) = (0.10, 0.045);
const NODDING: (f32, f32) = (0.035, 0.02);
const HEELING: (f32, f32) = (0.05, 0.03);
/// The most she noses up or down and banks into a turn, in radians.
const PITCH: f32 = 0.11;
const BANK: f32 = 0.10;
/// Seconds of turn she heels for: her bank follows her rate of turning, as a
/// boat leans into a bend, rather than the raw path she has just come by.
const BANKING: f32 = 2.5;
/// The slowest way through the water at which her track still says which way
/// her nose points; slower than this she holds the heading she had, for a
/// boat with no way on has no heading to read.
const WAY: f32 = 0.02;
/// How finely her attitude is stepped, in seconds, so that neither a quick
/// nor a slow run of frames changes how she moves.
const ATTITUDE_STEP: f32 = 0.05;
/// The fastest she climbs or dives, and the hardest she answers her planes
/// and blows her tanks, in world units a second and a second squared.
const CLIMB_RATE: f32 = 0.25;
const DIVE_RATE: f32 = -0.12;
const CLIMB_ACCEL: f32 = 0.2;
/// How finely her depth is stepped, in seconds, for the same reason.
const DEPTH_STEP: f32 = 0.1;

/// How another submarine passes the Nautilus.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Meeting {
    /// Close by, the other way.
    Pass,
    /// Overtaking, a little farther out.
    Overtake,
    /// Drawing up alongside, keeping pace a while, then pulling ahead.
    Alongside,
    /// Far off across the water: in the dark, only her lights.
    Far,
    /// Nosing out of the gloom toward the window, then turning away.
    Emerge,
}

/// The encounter of one slot of [`ENCOUNTER_EVERY`] seconds in one visit,
/// decided when it begins: what kind, if any, from the sea it begins in.
#[derive(Clone, Copy, Debug)]
struct Encounter {
    slot: i32,
    visit: i32,
    meeting: Option<Meeting>,
    /// A swipe hurried the Nautilus away: she is left behind.
    spoilt: bool,
    /// Her depth on a spring, as she answers her planes: when last placed,
    /// the height, and how fast it was changing.
    depth: Option<(f32, f32, f32)>,
    /// How her hull is turned through the water, once she is under way.
    attitude: Option<Attitude>,
}

/// One axis of a hull's attitude and how fast it is turning about it. Like
/// the helm's springs, but it knows an angle comes round again: she takes the
/// shortest way, never turns faster than her helm allows, and never gathers
/// that turn faster than a heavy boat can.
#[derive(Clone, Copy, Debug, Default)]
struct Turn {
    angle: f32,
    rate: f32,
}

impl Turn {
    fn at(angle: f32) -> Self {
        Self { angle, rate: 0.0 }
    }

    /// Turn toward `target` for `dt`: a critically damped approach whose
    /// speed and the gathering of that speed are both bounded.
    fn follow(&mut self, target: f32, omega: f32, (most, gathering): (f32, f32), dt: f32) {
        let gap = wrap(target - self.angle);
        let pull = omega * omega * gap - 2.0 * omega * self.rate;
        let change = (pull * dt).clamp(-gathering * dt, gathering * dt);
        self.rate = (self.rate + change).clamp(-most, most);
        self.angle += self.rate * dt;
    }
}

/// An angle folded the short way into [-π, π).
fn wrap(angle: f32) -> f32 {
    (angle + PI).rem_euclid(TAU) - PI
}

/// How one of the sisters is turned through the water: her heading, how far
/// up her nose is and how she heels. The angles follow where her way takes
/// her as a heavy hull answers helm and planes, in fine steps so that a
/// frame's irregular arrival never shows in her motion.
#[derive(Clone, Copy, Debug)]
struct Attitude {
    /// When she was last turned, so each step is measured rather than guessed.
    last: f32,
    /// The heading and nose-up angle she was asked for last, to ease from.
    asked: [f32; 2],
    yaw: Turn,
    pitch: Turn,
    roll: Turn,
}

impl Attitude {
    /// A hull settled at `asked` when first seen, banked `bank` into the turn
    /// she is making.
    fn at(last: f32, asked: [f32; 2], bank: f32) -> Self {
        Self {
            last,
            asked,
            yaw: Turn::at(asked[0]),
            pitch: Turn::at(asked[1]),
            roll: Turn::at(bank),
        }
    }

    /// Ease her heading and nose to `asked` by `now`, heeling into each turn
    /// as it gathers, in steps too short to see.
    fn turn_to(&mut self, asked: [f32; 2], now: f32) {
        let dt = now - self.last;
        if dt <= 0.0 {
            return;
        }
        let steps = (dt / ATTITUDE_STEP).ceil().max(1.0) as i32;
        let step = dt / steps as f32;
        let (from, to) = (self.asked, asked);
        let turning = wrap(to[0] - from[0]);
        for i in 1..=steps {
            let part = i as f32 / steps as f32;
            self.yaw.follow(from[0] + turning * part, 0.7, YAWING, step);
            self.pitch
                .follow(from[1] + (to[1] - from[1]) * part, 0.5, NODDING, step);
            // Her bank follows the turn she is making, not the path she has
            // come by: she leans into a bend as it gathers and rights herself
            // as it eases.
            let bank = (-BANKING * self.yaw.rate).clamp(-BANK, BANK);
            self.roll.follow(bank, 0.8, HEELING, step);
        }
        self.asked = asked;
        self.last = now;
    }
}

impl Meeting {
    fn seconds(self) -> f32 {
        match self {
            Meeting::Pass => 55.0,
            Meeting::Overtake => 90.0,
            Meeting::Alongside => 100.0,
            Meeting::Far => 80.0,
            Meeting::Emerge => 75.0,
        }
    }

    /// Turns of her screw a second.
    fn revolutions(self) -> f32 {
        match self {
            Meeting::Alongside => 1.1,
            Meeting::Overtake => 1.8,
            _ => 1.4,
        }
    }

    /// Where she is `u` (0 to 1) of the way through, from the window: along
    /// the voyage (ahead positive), out from the corridor, and above the
    /// window. `flip` is ±1, for the far crossing either way.
    fn path(self, u: f32, flip: f32) -> [f32; 3] {
        match self {
            Meeting::Pass => [3.2 - 6.4 * u, 1.05, 0.05],
            Meeting::Overtake => [-3.4 + 6.8 * u, 1.6, -0.1],
            Meeting::Alongside => [
                -3.5 * (1.0 - smooth(0.0, 0.4, u))
                    + 3.5 * smooth(0.65, 1.0, u)
                    + 0.12 * (u * 9.0).sin(),
                2.3,
                0.03 * (u * 7.0).sin(),
            ],
            Meeting::Far => [flip * (4.5 - 9.0 * u), 6.5, -0.5],
            Meeting::Emerge => {
                let (p0, p1, p2) = ([3.0, 7.5, -0.4], [1.5, 1.2, -0.1], [-3.5, 1.6, 0.05]);
                let v = 1.0 - u;
                [0, 1, 2].map(|i| v * v * p0[i] + 2.0 * v * u * p1[i] + u * u * p2[i])
            }
        }
    }

    /// How fast she moves along [`Meeting::path`] at `u` of the way through:
    /// its derivative, in her lengths of path per unit of `u`.
    fn pace(self, u: f32, flip: f32) -> [f32; 3] {
        match self {
            Meeting::Pass => [-6.4, 0.0, 0.0],
            Meeting::Overtake => [6.8, 0.0, 0.0],
            Meeting::Alongside => [
                3.5 * smooth_rate(0.0, 0.4, u)
                    + 3.5 * smooth_rate(0.65, 1.0, u)
                    + 0.12 * 9.0 * (u * 9.0).cos(),
                0.0,
                0.03 * 7.0 * (u * 7.0).cos(),
            ],
            Meeting::Far => [flip * -9.0, 0.0, 0.0],
            Meeting::Emerge => {
                let (p0, p1, p2) = ([3.0, 7.5, -0.4], [1.5, 1.2, -0.1], [-3.5, 1.6, 0.05]);
                [0, 1, 2].map(|i| 2.0 * (1.0 - u) * (p1[i] - p0[i]) + 2.0 * u * (p2[i] - p1[i]))
            }
        }
    }
}

/// The great things standing near the box from (x0, z0) to (x1, z1) that a
/// submarine must rise over, as (x, z, radius, top): Atlantis' temples, the
/// galleons' hulls and masts and the drowned city's towers, from the cells
/// `ruins`, `wrecks` and `city` place them in, and the kelp of the shallows. The forest's trees and the city's
/// columns stand too thick to weave between, so she keeps above them
/// throughout (`cruise`).
fn tall_things(w: &World, x0: f32, x1: f32, z0: f32, z1: f32) -> Vec<(f32, f32, f32, f32)> {
    let mut found = Vec::new();
    let scan = |size: f32, salt: u32, keep: &mut dyn FnMut(i32, i32, f32, f32)| {
        for iz in ((z0 - 1.0) / size).floor() as i32..=((z1 + 1.0) / size).ceil() as i32 {
            for ix in ((x0 - 1.0) / size).floor() as i32..=((x1 + 1.0) / size).ceil() as i32 {
                let (x, z) = cell_point(ix, iz, size, salt);
                keep(ix, iz, x, z);
            }
        }
    };
    // Kelp is a tall, thin thing rooted in the seabed: a sister climbs over
    // each clump she would otherwise sweep through.
    scan(KELP_CELL, KELP_SALT, &mut |ix, iz, x, z| {
        if let Some((height, fade_in)) = kelp_here(w, ix, iz)
            && fade_in > 0.05
        {
            // A little over the measured height: the hull answers its planes on a
            // spring, and a raised bank can put a blade a hand's width higher
            // by the time she is over it.
            found.push((x, z, 0.06, w.ground(x, z) + height + 0.25));
        }
    });
    scan(5.0, 80, &mut |ix, iz, x, z| {
        if w.share(x, z, ATLANTIS) >= 0.5
            && hash(ix, iz, 81) >= 0.3
            && (z - corridor(x)).abs() >= 2.0
        {
            let s = 1.4 + 0.4 * hash(ix, iz, 82);
            found.push((x, z, 0.6 * s, w.ground(x, z) + 0.9 * s));
        }
    });
    for far in [false, true] {
        let size = if far { CITY_FAR } else { CITY_NEAR };
        scan(size, 0, &mut |ix, iz, _, _| {
            if let Some(b) = building(w, ix, iz, far) {
                found.push((b.x, b.z, b.reach(), b.top()));
            }
        });
    }
    scan(2.4, 60, &mut |ix, iz, x, z| {
        if w.share(x, z, WRECKS) < 0.5 || hash(ix, iz, 61) < 0.35 || (z - corridor(x)).abs() < 1.2 {
            return;
        }
        let length = 2.2 + 1.0 * hash(ix, iz, 62);
        let yaw = (hash(ix, iz, 63) - 0.5) * 1.2;
        let (s, c) = yaw.sin_cos();
        let y = w.ground(x, z);
        let broken = hash(ix, iz, 65) < 0.5;
        for (d, top, r) in [
            (-0.35, 0.25, 0.2),
            (0.0, 0.2, 0.2),
            (0.35, 0.2, 0.2),
            (-0.15, 0.62, 0.17),
            (0.15, if broken { 0.3 } else { 0.62 }, 0.17),
        ] {
            found.push((
                x + c * d * length,
                z - s * d * length,
                r * length,
                y + top * length,
            ));
        }
    });
    found
}

/// How high the forest's canopy or the drowned city's columns stand about
/// (x, z): taken over several units either way, so a submarine climbs over
/// them, and stands off from the window, in a long, gentle slope rather than
/// as fast as one sea gives way to the next.
fn canopy(w: &World, x: f32, z: f32) -> f32 {
    let (mut forest, mut city) = (0.0, 0.0);
    for k in -4..=4 {
        let b = w.biome(x + k as f32, z);
        forest += b.weight(FOREST) as f32 / 9.0;
        city += b.weight(ATLANTIS) as f32 / 9.0;
    }
    2.0 * smooth(0.15, 0.75, forest) + 1.1 * smooth(0.15, 0.75, city)
}

/// The height a submarine at (x, z), heading along `dir` across the
/// seabed, keeps, held at `want` where nothing is in the way: as she cruises,
/// clear of the seabed, over the forest's canopy and the city's columns and
/// under the ice's keels; and then as she rises over a temple or a galleon's
/// masts and settles again after.
fn cruise(w: &World, x: f32, z: f32, dir: [f32; 2], want: f32) -> f32 {
    let half = SUB_LENGTH * 0.5 + 0.2;
    let mut ground = f32::MIN;
    for k in -2..=2 {
        let a = k as f32 * half * 0.5;
        ground = ground.max(w.ground(x + dir[0] * a, z + dir[1] * a));
    }
    let canopy = canopy(w, x, z);
    let ceiling = w.surface() - 0.3 - 0.85 * w.share(x, z, ARCTIC);
    let floor = want.max(ground + 0.45).max(ground + 0.15 + canopy);
    let mut y = floor;
    for (ox, oz, r, top) in tall_things(w, x - half, x + half, z - half, z + half) {
        // How far the obstacle is from her keel line, seen from above.
        let (px, pz) = (ox - x, oz - z);
        let along = (px * dir[0] + pz * dir[1]).clamp(-half, half);
        let gap = (px - dir[0] * along).hypot(pz - dir[1] * along);
        // She rises over it in good time, and settles again after.
        let k = smooth(r + 1.8, r + 0.2, gap);
        y = y.max(floor + k * (top + 0.3 - floor));
    }
    y.min(ceiling)
}

/// Now and then another submarine, a sister of the Nautilus, crosses the
/// window: passing close the other way, overtaking, keeping pace alongside a
/// while, far off across the water, or nosing out of the gloom. Iron plates
/// and a ram, a turning screw shedding bubbles, ports glowing warm; in the
/// dark seas her searchlight sweeps the water ahead of her, and in the abyss
/// she is little more than her lights.
fn submarines(w: &World, f: &mut Frame) {
    let t = w.t;
    let slot = (t / ENCOUNTER_EVERY).floor() as i32;
    let visit = (w.seabed.seed * 1_000_003.0) as i32;
    let start = slot as f32 * ENCOUNTER_EVERY + 8.0 + hash(slot, visit, 500) * 16.0;
    if t < start {
        return;
    }
    let speed = {
        let v = w.pose.vel;
        (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt()
    };
    let known = f.encounter.filter(|e| e.slot == slot && e.visit == visit);
    let mut encounter = known.unwrap_or_else(|| {
        // Decided from the sea she is met in: where the Nautilus will be
        // half a minute after it began, when she is most likely in view.
        let x0 = w.camera() + VOYAGE_SPEED * (start + 30.0 - t);
        let b = w.biome(x0, corridor(x0));
        let meeting = if b.weight(CAVE) > 0.5 || hash(slot, visit, 501) > 0.85 {
            None
        } else {
            let dark = 0.8 * b.weight(WRECKS) + b.weight(ATLANTIS) + b.weight(ABYSS);
            let odds = if dark > 0.5 {
                [
                    (Meeting::Far, 3.0),
                    (Meeting::Emerge, 3.0),
                    (Meeting::Pass, 2.0),
                    (Meeting::Alongside, 2.0),
                    (Meeting::Overtake, 1.0),
                ]
            } else {
                [
                    (Meeting::Pass, 3.0),
                    (Meeting::Alongside, 3.0),
                    (Meeting::Overtake, 2.0),
                    (Meeting::Emerge, 2.0),
                    (Meeting::Far, 1.0),
                ]
            };
            let mut roll = hash(slot, visit, 502) * odds.iter().map(|o| o.1).sum::<f32>();
            odds.iter()
                .find(|o| {
                    roll -= o.1;
                    roll < 0.0
                })
                .map(|o| o.0)
        };
        Encounter {
            slot,
            visit,
            meeting,
            spoilt: false,
            depth: None,
            attitude: None,
        }
    });
    // A swipe carries the Nautilus off and leaves her behind.
    encounter.spoilt |= speed > 0.25;
    f.encounter = Some(encounter);
    let Some(meeting) = encounter.meeting else {
        return;
    };
    let length = meeting.seconds();
    let u = (t - start) / length;
    if encounter.spoilt || u >= 1.0 {
        return;
    }
    let flip = if hash(slot, visit, 503) < 0.5 {
        1.0
    } else {
        -1.0
    };

    // Where she is at `tau`, in the world: along the voyage from where the
    // Nautilus is then, and over whatever is in her way.
    let at = |tau: f32, cruising: bool| -> [f32; 3] {
        let u = (tau - start) / length;
        let rel = meeting.path(u, flip);
        let pace = meeting.pace(u, flip);
        let run = [pace[0] + VOYAGE_SPEED * length, pace[1]];
        let l = run[0].hypot(run[1]).max(1e-4);
        let dir = [run[0] / l, run[1] / l];
        let x = w.camera() + VOYAGE_SPEED * (tau - t) + rel[0];
        let want = w.eye() + rel[2];
        // Held up over a forest or a city, she keeps farther out, so the
        // window, down among the trunks and columns, still sees her.
        let (a, b) = (rel[1], 2.9 * canopy(w, x, corridor(x) + rel[1]));
        // The larger, eased where one gives way to the other.
        let out = 0.5 * (a + b + ((a - b) * (a - b) + 0.8).sqrt());
        let y = if cruising {
            cruise(w, x, corridor(x) + out, dir, want)
        } else {
            0.0
        };
        [x, y, corridor(x) + out]
    };
    let mut p = at(t, true);
    // Where her track has her now, before her depth spring eases her to it:
    // the wake's positions are all relative to this, so they are pure in `t`
    // and a preview or a seek to this second sees the same trail.
    let head = p;
    // She keeps clear of the ground under her and over the next few
    // seconds' way, climbing for a shelf before she reaches it rather than
    // being lifted when she does.
    let clear = [2.0, 4.0, 6.0]
        .iter()
        .fold(w.ground(p[0], p[2]), |high, &ahead| {
            let q = at(t + ahead, false);
            high.max(w.ground(q[0], q[2]))
        });
    p[1] = p[1].max(clear + 0.36);
    // She answers her planes slowly: her depth follows the height she
    // wants on a critically damped spring, so a ridge or a shelf never
    // jerks her up or down. Both the speed and the gathering of it are
    // bounded, and it is stepped in fine stages, so a frame arriving early
    // or late changes nothing of how she moves.
    let (y, rate) = match encounter.depth {
        Some((last, mut y, mut rate)) if t > last && t - last < 0.5 => {
            let dt = t - last;
            let steps = (dt / DEPTH_STEP).ceil().max(1.0) as i32;
            let step = dt / steps as f32;
            // Faster up when a shelf rises under her, her tanks blown.
            let blown = smooth(0.2, 0.6, clear + 0.36 - y);
            let (omega, gathering) = (0.6, CLIMB_ACCEL * (1.0 + blown));
            for _ in 0..steps {
                let accel = (omega * omega * (p[1] - y) - 2.0 * omega * rate)
                    .clamp(-CLIMB_ACCEL, gathering);
                rate = (rate + accel * step).clamp(DIVE_RATE, CLIMB_RATE);
                y += rate * step;
            }
            (y, rate)
        }
        _ => (p[1], 0.0),
    };
    // Never into the seabed, though.
    p[1] = y.max(w.ground(p[0], p[2]) + 0.3);
    encounter.depth = Some((t, p[1], rate));

    // The way she is going through the water, from the path's own slope
    // rather than a step in time, so an irregular frame can never jolt her.
    // Her outward offset is the eased greater of her own and the canopy's,
    // and the canopy's slope is read in the world.
    let way = |tau: f32| -> [f32; 2] {
        let u = (tau - start) / length;
        let rel = meeting.path(u, flip);
        let pace = meeting.pace(u, flip);
        let x = w.camera() + VOYAGE_SPEED * (tau - t) + rel[0];
        let along = VOYAGE_SPEED + pace[0] / length;
        let (across, crossing) = (rel[1], pace[1] / length);
        let z = corridor(x) + across;
        let (c, h) = (canopy(w, x, z), 0.6);
        let cx = (canopy(w, x + h, z) - c) / h;
        let cz = (canopy(w, x, z + h) - c) / h;
        let b = 2.9 * c;
        let d = ((across - b) * (across - b) + 0.8).sqrt();
        let db = 2.9 * (cx * along + cz * (corridor_slope(x) * along + crossing));
        let out = 0.5 * (1.0 + (across - b) / d) * crossing + 0.5 * (1.0 - (across - b) / d) * db;
        [along, corridor_slope(x) * along + out]
    };
    let [along, across] = way(t);
    let flat = along.hypot(across).max(1e-4);
    // Where she is barely moving her way says nothing of her head, and she
    // holds the heading she had.
    let heading = match encounter.attitude {
        Some(attitude) if flat < WAY => attitude.asked[0],
        _ => (-across).atan2(along),
    };
    let nose = (rate / flat.max(0.02)).atan().clamp(-PITCH, PITCH);
    let asked = [heading, nose];
    let mut attitude = match encounter.attitude {
        Some(a) if t >= a.last && t - a.last <= 0.5 => a,
        _ => {
            // First sight of her, she is already under way: banked as the
            // turn she is making would have her.
            let ahead = way(t + 0.5);
            let turn = wrap((-ahead[1]).atan2(ahead[0]) - heading) / 0.5;
            Attitude::at(t, asked, (-BANKING * turn).clamp(-BANK, BANK))
        }
    };
    attitude.turn_to(asked, t);
    encounter.attitude = Some(attitude);
    f.encounter = Some(encounter);
    let (yaw, pitch, roll) = (
        attitude.yaw.angle,
        attitude.pitch.angle,
        attitude.roll.angle,
    );
    let water_speed = (flat * flat + rate * rate).sqrt();

    let cave = w
        .share(p[0], p[2], CAVE)
        .max(w.share(w.camera(), w.pose.pos[2], CAVE));
    let fading = fade(1.0 - cave, 0.7)
        * smooth(0.0, 0.06, u)
        * smooth(1.0, 0.94, u)
        * (1.0 - smooth(0.12, 0.3, speed))
        * w.reveal(p, SUB_LENGTH * 0.55);
    if fading <= 0.01 {
        return;
    }
    f.fade = fading;
    // How dark the water is, as the scene's daylight reckons it at the
    // window.
    let dark = {
        let depth = 1.0 - (-(w.pose.depth as f32) / 7.0).exp();
        let sun = (-depth * 4.3).exp();
        (1.0 - smooth(0.04, 0.35, sun)) * (1.0 - cave)
    };
    let hull = place(p, yaw, pitch, roll, [SUB_LENGTH; 3]);
    let to_world = |q: [f32; 3]| {
        [0, 1, 2].map(|r| hull[r][0] * q[0] + hull[r][1] * q[1] + hull[r][2] * q[2] + hull[r][3])
    };
    let turned = |v: [f32; 3]| {
        norm([0, 1, 2].map(|r| hull[r][0] * v[0] + hull[r][1] * v[1] + hull[r][2] * v[2]))
    };
    let cam = w.pose.pos;
    let distance = {
        let d = [p[0] - cam[0], p[1] - cam[1], p[2] - cam[2]];
        (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt()
    };
    let warm = [1.0, 0.82, 0.50];
    // Lamplit amber, never blown out to white however dark the water.
    let glow = 0.5 + 1.1 * dark;
    if w.visible(p, SUB_LENGTH * 0.55) {
        f.put(Model::Submarine, hull, [0.33, 0.34, 0.36])
            .material(material::COPPER);
        let spin = (t - start) * meeting.revolutions() * TAU;
        f.put(
            Model::Screw,
            compose(&hull, &place(mesh::SUB_SCREW, 0.0, 0.0, spin, [0.058; 3])),
            [0.62, 0.45, 0.25],
        )
        .material(material::COPPER);
        // Her ports, a row along each side lit from within, and the glass of
        // the pilot's cage.
        let r = mesh::SUB_RADIUS * 1.01;
        let up = 0.3f32;
        let halo = 0.03 * (1.0 + distance * 0.25);
        for side in [1.0f32, -1.0] {
            let normal = [0.0, up.sin(), side * up.cos()];
            let roll = if side > 0.0 { -up } else { PI + up };
            for x in [-0.17, -0.10, -0.03, 0.04, 0.11] {
                let local = [x, r * up.sin(), side * r * up.cos()];
                let at = to_world(local);
                let n = turned(normal);
                let view = [cam[0] - at[0], cam[1] - at[1], cam[2] - at[2]];
                if n[0] * view[0] + n[1] * view[1] + n[2] * view[2] < 0.0 {
                    continue;
                }
                f.put(
                    Model::Sphere,
                    compose(&hull, &place(local, 0.0, 0.0, roll, [0.011, 0.011, 0.004])),
                    warm,
                )
                .glowing(glow);
                if dark > 0.05 {
                    let k = 0.30 * dark * fading;
                    f.sprite(
                        add(at, [n[0] * 0.01, n[1] * 0.01, n[2] * 0.01]),
                        halo,
                        [warm[0] * k, warm[1] * k, warm[2] * k],
                        GLOW,
                    );
                }
            }
        }
        let cage = mesh::SUB_RADIUS + 0.012;
        for (local, scale) in [
            ([0.248, cage, 0.0], [0.003, 0.008, 0.012]),
            ([0.20, cage + 0.002, 0.0185], [0.02, 0.008, 0.003]),
            ([0.20, cage + 0.002, -0.0185], [0.02, 0.008, 0.003]),
        ] {
            f.put(
                Model::Sphere,
                compose(&hull, &place(local, 0.0, 0.0, 0.0, scale)),
                warm,
            )
            .glowing(glow);
        }
        // The bubbles her screw throws off, left behind in the water as she
        // goes on, rising as they drift. Where a bubble was let go is read
        // back along her own track, sampled a few seconds apart: spacing is
        // her speed through the water, the trail bends as she turns, and if
        // she loses her way the bubbles gather and rise instead of trailing.
        let screw = to_world(mesh::SUB_SCREW);
        let track = [
            head,
            at(t - 2.0, true),
            at(t - 4.0, true),
            at(t - 6.0, true),
        ];
        let was_at = |age: f32| -> [f32; 3] {
            let s = (age * 0.5).clamp(0.0, 3.0);
            let step = (s.floor() as usize).min(2);
            let part = s - step as f32;
            let (a, b) = (track[step], track[step + 1]);
            [0, 1, 2].map(|r| a[r] + (b[r] - a[r]) * part)
        };
        let churn = smooth(0.02, 0.25, water_speed);
        for k in 0..28 {
            let life = 6.0;
            let shed = bubble(k, 28, t, life);
            let age = shed.age;
            let n = shed.cohort;
            let was = was_at(age);
            let spread = (0.015 + 0.02 * age) * (0.7 + 0.6 * churn);
            let q = [
                screw[0] + was[0] - head[0] + (hash(n, k, 510) - 0.5) * spread,
                screw[1] + was[1] - head[1]
                    + (0.045 + 0.02 * churn) * age
                    + (hash(n, k, 511) - 0.5) * spread,
                screw[2] + was[2] - head[2] + (hash(n, k, 512) - 0.5) * spread,
            ];
            let alpha = shed.fade * fading * window_fade(w, q, 0.02);
            if alpha > 0.001 {
                f.sprite(
                    q,
                    0.005 + 0.007 * hash(k, n, 513) + 0.002 * churn,
                    [0.32 * alpha, 0.62 * alpha, 0.65 * alpha],
                    BUBBLE,
                );
            }
        }
    }

    // In dark water her searchlight sweeps the sea ahead of her, and the
    // ground where it falls.
    let lamp = smooth(0.15, 0.6, dark) * fading;
    if lamp > 0.01 {
        let o = to_world([
            mesh::SUB_LANTERN[0] + 0.004,
            mesh::SUB_LANTERN[1],
            mesh::SUB_LANTERN[2],
        ]);
        let phase = hash(slot, visit, 504) * TAU;
        let sweep = 0.6 * ((t - start) * 0.21 + phase).sin();
        let dip = -0.16 + 0.08 * ((t - start) * 0.13 + phase).sin();
        let dir = turned([dip.cos() * sweep.cos(), dip.sin(), -dip.cos() * sweep.sin()]);
        // Where the beam meets the seabed, if it does within its reach: it
        // stops there in a pool of light rather than running on through the
        // sand, or glows on in the water if it meets nothing.
        let mut reach = 3.6;
        let mut hit = None;
        for step in 1..=14 {
            let s = step as f32 * 0.25;
            let q = add(o, [dir[0] * s, dir[1] * s, dir[2] * s]);
            if q[1] < w.ground(q[0], q[2]) {
                reach = s - 0.1;
                hit = Some(add(q, [-dir[0] * 0.2, 0.15 - dir[1] * 0.2, -dir[2] * 0.2]));
                break;
            }
        }
        let run = [dir[0] * reach, dir[1] * reach, dir[2] * reach];
        // Twice what it shows: the sea is stored at half its brightness.
        let k = 1.6 * lamp;
        let middle = add(o, [run[0] * 0.5, run[1] * 0.5, run[2] * 0.5]);
        if w.visible(middle, reach * 0.5 + 0.6) {
            // A bright core, and a wide faint halo of light scattered off it.
            let spread = 0.24 * reach;
            f.beams.push(Sprite {
                place: [o[0], o[1], o[2], 0.02],
                tint: [k, 0.9 * k, 0.7 * k, spread],
                stretch: [run[0], run[1], run[2], 0.0],
            });
            let h = 0.45 * k;
            f.beams.push(Sprite {
                place: [o[0], o[1], o[2], 0.06],
                tint: [h, 0.92 * h, 0.78 * h, spread * 2.6],
                stretch: [run[0], run[1], run[2], 0.0],
            });
            // Motes caught in the cone, glinting as they drift through it.
            for m in 0..14 {
                let drift = (w.t * 0.05 + hash(slot, m, 505)) % 1.0;
                let along = 0.12 + 0.85 * drift;
                let a = hash(slot, m, 506) * TAU + w.t * 0.1;
                let r = spread * along * 0.8 * hash(slot, m, 507).sqrt();
                let side = norm([-dir[2], 0.0, dir[0]]);
                let lift = [
                    side[1] * dir[2] - side[2] * dir[1],
                    side[2] * dir[0] - side[0] * dir[2],
                    side[0] * dir[1] - side[1] * dir[0],
                ];
                let q = [0, 1, 2]
                    .map(|i| o[i] + run[i] * along + (side[i] * a.cos() + lift[i] * a.sin()) * r);
                let glint = lamp * (1.0 - along) * (drift * 8.0).min(1.0) * 0.5;
                f.sprite(q, 0.008, [glint, 0.9 * glint, 0.75 * glint], GLOW);
            }
        }
        if w.visible(o, 0.3) {
            // The lantern's glass, blazing when it looks toward the window.
            let view = norm([cam[0] - o[0], cam[1] - o[1], cam[2] - o[2]]);
            let facing = (dir[0] * view[0] + dir[1] * view[1] + dir[2] * view[2])
                .max(0.0)
                .powi(6);
            f.put(
                Model::Sphere,
                compose(
                    &hull,
                    &place(mesh::SUB_LANTERN, 0.0, 0.0, 0.0, [0.004, 0.010, 0.010]),
                ),
                [1.0, 0.88, 0.66],
            )
            .glowing(0.5 + 2.5 * lamp);
            let g = lamp * (0.25 + 1.5 * facing);
            f.sprite(o, 0.04 + 0.12 * facing, [g, 0.9 * g, 0.7 * g], GLOW);
        }
        let (spot, strength) = match hit {
            Some(q) => (q, 1.0),
            None => (add(o, [dir[0] * 1.5, dir[1] * 1.5, dir[2] * 1.5]), 0.5),
        };
        let c = 1.1 * lamp * strength;
        f.light(spot, 0.8, [c, 0.88 * c, 0.65 * c]);
    }
}

/// `child`'s transform, then `parent`'s: a part placed on a model.
fn compose(parent: &[[f32; 4]; 3], child: &[[f32; 4]; 3]) -> [[f32; 4]; 3] {
    let mut rows = [[0.0; 4]; 3];
    for (r, row) in rows.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| parent[r][k] * child[k][c]).sum::<f32>()
                + if c == 3 { parent[r][3] } else { 0.0 };
        }
    }
    rows
}

/// Shafts of sunlight slanting down from the surface where it is shallow and
/// bright enough: through the shallows, between the forest's trees, over the
/// wrecks and the shallow reef, down into the trench and through the leads in
/// the ice. Each is rooted in a cell of a
/// grid fixed in the world, so the boat sails among them; they breathe as the
/// waves above gather and scatter the light, and sway with them.
fn shafts(w: &World, f: &mut Frame) {
    let sun = norm(SUN);
    let far = 7.0;
    let top = w.surface();
    for &(ix, iz, x, z) in &cells(w, 0.75, 0.3, far, 450) {
        let h = hash(ix, iz, 451);
        let b = w.biome(x, z);
        let bright = (b.weight(SHALLOWS)
            + 0.6 * b.weight(FOREST)
            + 0.35 * b.weight(WRECKS)
            + 0.5 * b.weight(ARCTIC)
            + 0.45 * b.weight(TRENCH)
            + 0.45 * b.weight(CORAL)
            + 0.85 * b.weight(SARGASSO)
            + 0.35 * b.weight(BRINE)) as f32;
        if h > bright.min(1.0) * 0.55 {
            continue;
        }
        // Under the ice only the open leads let the sun through, so fewer.
        let roof = top - ICE * b.weight(ARCTIC) as f32;
        let foot = w.ground(x, z) + 0.15 + 0.4 * hash(ix, iz, 452);
        if foot > roof - 0.3 {
            continue;
        }
        let length = (roof - foot) / sun[1];
        let sway = 0.06 * (w.t * 0.13 + h * 40.0).sin();
        let start = [
            x + sun[0] * length + sway,
            roof,
            z + sun[2] * length + 0.5 * sway,
        ];
        let run = [x - start[0], foot - start[1], z - start[2]];
        let width = 0.04 + 0.12 * hash(ix, iz, 453);
        let middle = add(start, [run[0] * 0.5, run[1] * 0.5, run[2] * 0.5]);
        if !w.visible(middle, length * 0.5 + width) {
            continue;
        }
        let depth = {
            let out = w.pose.basis[2];
            let c = w.pose.pos;
            (middle[0] - c[0]) * out[0] + (middle[2] - c[2]) * out[2]
        };
        let breathe = 0.45 + 0.55 * (w.t * (0.25 + 0.2 * h) + h * 70.0).sin().max(0.0);
        let k =
            bright.min(1.0) * breathe * smooth(far, far - 2.0, depth) * 1.3 / (0.5 + width * 6.0);
        // Through the sargasso's golden weed the light comes down gold.
        let gold = b.weight(SARGASSO) as f32;
        f.shafts.push(Sprite {
            place: [start[0], start[1], start[2], width],
            tint: [
                (0.30 + 0.45 * gold) * k,
                (0.75 - 0.05 * gold) * k,
                (0.70 - 0.40 * gold) * k,
                0.0,
            ],
            stretch: [run[0], run[1], run[2], 0.0],
        });
    }
}

fn particles(w: &World, f: &mut Frame) {
    let c = w.pose.pos;
    let b = w.biome(c[0], c[2]);
    let vents = b.weight(VENTS) as f32;
    let lava = (b.weight(ATLANTIS) + b.weight(CAVE) + 0.5 * b.weight(VENTS)) as f32;
    let arctic = b.weight(ARCTIC) as f32;
    let abyss = b.weight(ABYSS) as f32;
    let bloom = b.weight(BLOOM) as f32;
    let sargasso = b.weight(SARGASSO) as f32;
    let brine = b.weight(BRINE) as f32;
    let coral = b.weight(CORAL) as f32;
    let eye = w.eye();
    // Snow hangs in a lattice fixed in the water, which sinks slowly as a
    // whole: no flake ever jumps, and the voyage sweeps the boat through it.
    let cell = 0.32;
    let sink = w.t * 0.006;
    let ([x0, x1, z0, z1], _, _) = footprint(w, 0.2, 5.0);
    for iz in (z0 / cell).floor() as i32..=(z1 / cell).ceil() as i32 {
        for ix in (x0 / cell).floor() as i32..=(x1 / cell).ceil() as i32 {
            for iy in ((eye - 2.4 + sink) / cell).floor() as i32
                ..=((eye + 2.4 + sink) / cell).ceil() as i32
            {
                let h = hash(ix, iy * 61 + iz, 400);
                if h < 0.55 {
                    continue;
                }
                let p = [
                    (ix as f32 + hash(iy, ix, 402)) * cell,
                    (iy as f32 + hash(ix + iz, iy, 403)) * cell - sink,
                    (iz as f32 + hash(ix, iz + iy, 404)) * cell,
                ];
                if !w.visible(p, 0.01) {
                    continue;
                }
                let warm = lava * 0.7;
                let mut tint = [0.34 + 0.66 * warm, 0.61 - 0.16 * warm, 0.63 - 0.48 * warm];
                // The new seas colour the snow: gold under the sargasso,
                // cyan in the bloom, a bright reef haze over the coral.
                let cast = |c: [f32; 3], to: [f32; 3], t: f32| {
                    [
                        c[0] + (to[0] - c[0]) * t,
                        c[1] + (to[1] - c[1]) * t,
                        c[2] + (to[2] - c[2]) * t,
                    ]
                };
                tint = cast(tint, [0.62, 0.56, 0.22], sargasso * 0.55);
                tint = cast(tint, [0.30, 0.72, 0.70], bloom * 0.45);
                tint = cast(tint, [0.78, 0.84, 0.72], coral * 0.25);
                tint = cast(tint, [0.66, 0.74, 0.74], brine * 0.35);
                let glow = 0.35 + 0.10 * coral + 0.06 * brine;
                f.sprite(
                    p,
                    0.004 + 0.003 * h,
                    [tint[0] * glow, tint[1] * glow, tint[2] * glow],
                    MOTE,
                );
                if arctic > 0.2 && h > 0.85 {
                    f.sprite(
                        add(p, [0.05, 0.03, 0.02]),
                        0.003,
                        [0.30 * arctic, 0.15 * arctic, 0.13 * arctic],
                        EMBER,
                    );
                }
                if abyss > 0.2 && h > 0.9 {
                    let blink = (w.t * 2.1 + h * 40.0).sin().max(0.0);
                    f.sprite(
                        add(p, [0.02, -0.04, 0.03]),
                        0.006,
                        [
                            0.05 * abyss * blink,
                            0.45 * abyss * blink,
                            0.35 * abyss * blink,
                        ],
                        GLOW,
                    );
                }
                if bloom > 0.2 && h > 0.82 {
                    let blink = (w.t * 1.7 + h * 55.0).sin().max(0.0);
                    f.sprite(
                        add(p, [0.02, 0.03, -0.02]),
                        0.008,
                        [
                            0.08 * bloom * blink,
                            0.40 * bloom * blink,
                            0.52 * bloom * blink,
                        ],
                        GLOW,
                    );
                }
                if vents > 0.2 && h > 0.88 {
                    let rise = (w.t * 0.4 + h * 17.0).fract();
                    f.sprite(
                        add(p, [0.03, rise * 0.22, 0.0]),
                        0.005,
                        [0.42 * vents * (1.0 - rise), 0.13 * vents, 0.04 * vents],
                        EMBER,
                    );
                }
                if brine > 0.2 && h > 0.92 {
                    let blink = (w.t * 2.7 + h * 30.0).sin().max(0.0);
                    f.sprite(
                        p,
                        0.004,
                        [0.30 * brine * blink, 0.08 * brine, 0.10 * brine],
                        GLOW,
                    );
                }
            }
        }
    }
    // The Nautilus sheds no bubbles of her own: a hull stream just below the
    // glass put rings across it. The water's way past the window is read from
    // the sea itself, and the speed of a passage from the rush below.
    let [right, up, out] = w.pose.basis;
    let v = w.pose.vel;
    let in_boat = |a: f32, b: f32, d: f32| {
        [
            c[0] + right[0] * a + up[0] * b + out[0] * d,
            c[1] + right[1] * a + up[1] * b + out[1] * d,
            c[2] + right[2] * a + up[2] * b + out[2] * d,
        ]
    };
    // In a fast passage the water streams past the glass: a rush of bubbles
    // beyond the near water, streaking across the window instead of blooming
    // on it. The stream keeps a fixed shape as its strength grows and ebbs: no
    // bubble is re-phased by the count changing, and the cut-off end only ever
    // fades in or out.
    let speed = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let rush = smooth(0.08, 0.5, speed);
    if rush > 0.0 {
        let life = 0.9;
        let stream = 220;
        let count = (rush * stream as f32) as i32;
        for k in 0..count {
            let shed = bubble(k, stream, w.t * 1.7, life);
            let age = shed.age;
            let n = shed.cohort;
            let d = BUBBLE_CLEAR + 0.25 + hash(n, k, 420) * 1.1;
            // The spread grows with the distance, so the rush keeps the angles
            // it had when it was close and still crosses the whole window.
            let start = in_boat(
                (hash(n, k, 421) - 0.3) * 1.6 * d,
                (hash(n, k, 422) - 0.5) * 1.6 * VIEW * d,
                d,
            );
            let p = [
                start[0] - v[0] * age,
                start[1] + age * 0.1,
                start[2] - v[2] * age,
            ];
            let edge = smooth(count as f32, (count - 24).max(1) as f32, k as f32);
            let alpha = rush * edge * shed.fade * window_fade(w, p, 0.01);
            if alpha > 0.001 {
                f.streak(
                    p,
                    0.008 + 0.014 * hash(k, n, 423),
                    [0.35 * alpha, 0.55 * alpha, 0.58 * alpha],
                    1.0 + speed * 20.0,
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::voyage::Voyage;

    /// A creature that wanders into the glass fades over its rim instead of
    /// appearing at full opacity, and the near plane does the same.
    #[test]
    fn encounters_fade_at_the_window() {
        let seabed = Seabed::new(0, 0.0);
        let pose = super::super::helm::Pose::level(0.0, -2.0);
        let w = World {
            seabed: &seabed,
            pose,
            t: 0.0,
            slack: 0.0,
        };
        let c = pose.pos;
        let out = pose.basis[2];
        let right = pose.basis[0];
        let at = |ahead: f32, across: f32| {
            [
                c[0] + out[0] * ahead + right[0] * across,
                c[1] + out[1] * ahead + right[1] * across,
                c[2] + out[2] * ahead + right[2] * across,
            ]
        };
        let mid = at(5.0, 0.0);
        let edge = VIEW * 5.0 * 1.08 + 0.02 * 1.1;
        let inside = w.reveal(at(5.0, edge - 0.04), 0.02);
        let at_edge = w.reveal(at(5.0, edge), 0.02);
        let outside = w.reveal(at(5.0, edge + 0.6), 0.02);
        assert!(w.reveal(mid, 0.02) > 0.99, "a centred subject is hidden");
        assert!(
            inside > 0.05 && inside < 0.95,
            "the rim is a hard edge: {inside}"
        );
        assert!(
            at_edge < 0.01,
            "a subject on the rim is still full: {at_edge}"
        );
        assert_eq!(outside, 0.0);
        assert_eq!(w.reveal(at(0.02, 0.0), 0.02), 0.0, "a subject in the lens");
    }

    /// Along the trench the rim the boat sails is level, and beside it, where
    /// the window looks, the seabed falls away and rises again across the gulf.
    #[test]
    fn the_trench_falls_away_beside_the_rim() {
        let seabed = Seabed::new(3, 0.0);
        let start = crate::voyage::LEG * Biome::leg_of(TRENCH) as f64;
        for k in 0..20 {
            let x = (start + crate::voyage::LEG * (0.1 + 0.025 * k as f64)) as f32;
            assert_eq!(seabed.share(x, corridor(x), TRENCH), 1.0);
            let at = |across: f32| seabed.ground(x, corridor(x) + trench_wander(x) + across);
            let rim = at(0.0);
            assert!((at(0.9) - rim).abs() < 0.8, "the rim is level at {x}");
            assert!(rim - at(3.6) > 4.0, "the trench is deep at {x}");
            assert!(at(7.5) > rim, "the far wall rises above the rim at {x}");
        }
    }

    /// Every cave's tunnel has its five secrets, each wholly inside it, and
    /// the massif is cut back for the side passages and nowhere near the
    /// boat's way.
    #[test]
    fn the_cave_has_its_secrets() {
        for visit in [0, 7, 123] {
            let seed = (visit % 1_000_003) as f32 / 1_000_003.0;
            let seabed = Seabed::new(visit, seed);
            let leg = crate::voyage::LEG as f32;
            let cave = (0..crate::voyage::BIOMES)
                .map(|k| k as f32)
                .find(|k| Biome::at(f64::from((k + 0.5) * leg), seed).a == CAVE)
                .unwrap();
            let openings = seabed.openings(cave * leg).expect("a cave's openings");
            for kind in Secret::ALL {
                assert!(openings.iter().any(|o| o.kind == kind), "no {kind:?}");
            }
            for o in openings {
                for x in [o.x - o.half - 0.1, o.x + o.half + 0.1] {
                    assert!(
                        seabed.tunnel_at(x).is_some(),
                        "{:?} outside the tunnel",
                        o.kind
                    );
                }
                if o.side() {
                    let (c, _) = o.path(0.6);
                    assert!(seabed.bay(c[0], c[1]) > 0.99, "{:?} not cut", o.kind);
                }
                assert_eq!(seabed.bay(o.x, 0.3), 0.0, "cut into the boat's way");
            }
        }
    }

    /// Over half an hour's voyage another submarine comes into view now and
    /// then, never for long, never in the lava cave, and never lurching.
    #[test]
    fn submarines_are_met_now_and_then() {
        for visit in [0u64, 500_000] {
            let seed = (visit % 1_000_003) as f32 / 1_000_003.0;
            let seabed = Seabed::new(visit, seed);
            let mut voyage = Voyage::default();
            let mut helm = super::super::helm::Helm::default();
            let mut frame = Frame::new();
            let (mut seen, mut met, mut last) = (0.0f32, 0, None::<f64>);
            // Where she was seen the last two steps.
            let mut track: Vec<(f64, [f32; 3])> = Vec::new();
            let step = 0.25;
            for i in 0..(1800.0 / step) as i32 {
                let seconds = 5.0 + i as f64 * step;
                let at = voyage.advance(seconds, seed);
                let pose = helm.steer(&seabed, at, seconds);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                build(&world, &mut frame, &mut StaticCache::default());
                let Some(hull) = frame.solid[Model::Submarine.index()].first() else {
                    track.clear();
                    continue;
                };
                seen += step as f32;
                let here = seabed.biome(pose.pos[0], pose.pos[2]);
                assert!(
                    here.weight(CAVE) < 0.5,
                    "a submarine in the cave at {seconds}"
                );
                if last.is_none_or(|l| seconds - l > 20.0) {
                    met += 1;
                }
                last = Some(seconds);
                // However she turns, rises or stands off, the way she moves
                // changes gently from one moment to the next.
                let p = [hull.rows[0][3], hull.rows[1][3], hull.rows[2][3]];
                track.retain(|&(t, _)| seconds - t < 2.0 * step + 0.01);
                if let [(_, a), (_, b)] = track[..] {
                    let jerk = (0..3)
                        .map(|i| (p[i] - 2.0 * b[i] + a[i]).powi(2))
                        .sum::<f32>()
                        .sqrt();
                    assert!(jerk < 0.02, "she lurched {jerk:.3} at {seconds} s");
                }
                track.push((seconds, p));
            }
            println!("visit {visit}: {met} submarines, in view {seen:.0} s of 1800");
            assert!(met >= 6, "only {met} encounters");
            assert!(seen < 700.0, "submarines in view {seen} s of half an hour");
        }
    }

    /// Mid-way through each leg of the voyage, its signature things are in view.
    #[test]
    fn every_biome_has_its_landmarks() {
        let landmarks = [
            (SHALLOWS, Model::Kelp),
            (FOREST, Model::Tree),
            (WRECKS, Model::Hull),
            (ATLANTIS, Model::Shaft),
            // The stalactites: the basalt columns stand low by the walls,
            // outside the round window from up the tunnel.
            (CAVE, Model::Cone),
            (ARCTIC, Model::Keel),
            (TRENCH, Model::Sponge),
            (CORAL, Model::Coral),
            (SARGASSO, Model::Sargassum),
            (VENTS, Model::Vent),
            (BRINE, Model::Hex),
            (BLOOM, Model::Sponge),
            (CITY, Model::Tower),
        ];
        for (leg, model) in landmarks {
            let mut found = 0;
            // A few moments through the leg, as any one could fall in a gap.
            for k in 0..6 {
                let seconds = crate::voyage::LEG / crate::voyage::SPEED
                    * (Biome::leg_of(leg) as f64 + 0.2 + 0.1 * k as f64);
                let mut voyage = Voyage::default();
                let position = voyage.advance(seconds, 0.0);
                let seabed = Seabed::new(3, 0.0);
                let x = position.x as f32;
                let mut pose = Pose::level(x, seabed.ground(x, 0.6) + 0.5);
                if leg == CAVE {
                    // In the cave the helm turns the window up the tunnel.
                    pose.pos[1] = seabed.tunnel_floor(x) + 0.4;
                    let (sin, cos) = super::super::helm::LOOK.sin_cos();
                    pose.basis = [[cos, 0.0, -sin], [0.0, 1.0, 0.0], [sin, 0.0, cos]];
                }
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                let mut frame = Frame::new();
                build(&world, &mut frame, &mut StaticCache::default());
                found += frame.solid[model.index()].len();
            }
            assert!(found > 0, "no {model:?} anywhere in leg {leg}");
        }
    }

    /// The party of three walks the forest and Atlantis, one shared suit and
    /// helmet apiece, seven limbs about its stride and two weighted boots: a
    /// few thousand triangles for the whole party and never a per-diver mesh.
    #[test]
    fn divers_walk_the_forest_and_atlantis() {
        const PARTY: usize = 3;
        let library = mesh::Library::build();
        let tris = |model: Model| library.lods[model.index()][0].count as usize / 3;
        let verts = |model: Model| {
            let levels = &library.lods[model.index()];
            if levels.len() > 1 {
                return (levels[1].base - levels[0].base) as usize;
            }
            let at = Model::ALL.iter().position(|&m| m == model).unwrap();
            let next = Model::ALL[(at + 1) % Model::ALL.len()];
            (library.lods[next.index()][0].base - levels[0].base) as usize
        };
        let party = PARTY * (tris(Model::Diver) + tris(Model::DiverHelmet))
            + 7 * PARTY * tris(Model::DiverLimb)
            + 2 * PARTY * tris(Model::DiverBoot)
            + tris(Model::Box)
            + 2 * tris(Model::Cylinder);
        let party_verts = PARTY * (verts(Model::Diver) + verts(Model::DiverHelmet))
            + 7 * PARTY * verts(Model::DiverLimb)
            + 2 * PARTY * verts(Model::DiverBoot);
        println!(
            "divers: suit {}/{} verts/tris, helmet {}/{}, limb {}/{}, boot {}/{}; \
             party {party_verts} verts, {party} tris",
            verts(Model::Diver),
            tris(Model::Diver),
            verts(Model::DiverHelmet),
            tris(Model::DiverHelmet),
            verts(Model::DiverLimb),
            tris(Model::DiverLimb),
            verts(Model::DiverBoot),
            tris(Model::DiverBoot),
        );
        assert!(
            party <= 5000,
            "the party costs {party} triangles, too many for the full-HD budget"
        );
        let start = Voyage::default().advance(0.0, 0.0).x;
        for (leg, name) in [(FOREST, "forest"), (ATLANTIS, "atlantis")] {
            let seabed = Seabed::new(3, 0.0);
            let mut party = None;
            let mut best = 0;
            for k in 0..200 {
                // The moment this party's leader walks level with the boat.
                // The window looks out on the +z side, where a party keeps
                // when its slot falls on that side.
                let seconds = ((f64::from(k) + 0.5) * 7.0 - start) / (crate::voyage::SPEED - 0.02);
                let x = (start + crate::voyage::SPEED * seconds) as f32;
                let at_leg =
                    (f64::from(x) / crate::voyage::LEG).rem_euclid(crate::voyage::BIOMES as f64);
                let leg_at = Biome::leg_of(leg) as f64;
                if !(leg_at + 0.2..=leg_at + 0.7).contains(&at_leg) {
                    continue;
                }
                println!(
                    "k={k} t={seconds:.1} leg={at_leg:.3} side={} hash={}",
                    side(k, 150),
                    hash(k, 0, 150)
                );
                if side(k, 150) < 0.0 || hash(k, 0, 150) < 0.2 {
                    continue;
                }
                let pose = Pose::level(x, seabed.ground(x, 0.6) + 0.5);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                let mut frame = Frame::new();
                build(&world, &mut frame, &mut StaticCache::default());
                let bodies = &frame.solid[Model::Diver.index()];
                if bodies.len() != PARTY {
                    continue;
                }
                assert_eq!(
                    frame.solid[Model::DiverHelmet.index()].len(),
                    PARTY,
                    "every {name} diver wears a helmet"
                );
                assert_eq!(
                    frame.solid[Model::DiverLimb.index()].len(),
                    7 * PARTY,
                    "every {name} diver has two arms and two legs"
                );
                assert_eq!(
                    frame.solid[Model::DiverBoot.index()].len(),
                    2 * PARTY,
                    "every {name} diver wears two boots"
                );
                let visible = bodies
                    .iter()
                    .filter(|b| world.visible([b.rows[0][3], b.rows[1][3], b.rows[2][3]], 0.09))
                    .count();
                println!(
                    "cam {:?} feet {:?} visible {visible}",
                    world.pose.pos,
                    bodies
                        .iter()
                        .map(|b| [b.rows[0][3], b.rows[1][3], b.rows[2][3]])
                        .collect::<Vec<_>>()
                );
                // The trailing diver can be at the very rim of the round
                // window, where `reveal` fades it out: two of the three are
                // enough to prove the party is drawn as one formation.
                best = best.max(visible);
                if visible < PARTY - 1 {
                    continue;
                }
                party = Some(());
                break;
            }
            assert!(
                party.is_some(),
                "only {best} of the party ever show in the {name}"
            );
        }
    }

    #[test]
    fn debug_divers_show_through_the_window() {
        let seabed = Seabed::new(0, 0.0);
        let mut voyage = Voyage::default();
        let mut helm = super::super::helm::Helm::default();
        for (leg, name) in [(FOREST, "forest"), (ATLANTIS, "atlantis")] {
            let start =
                crate::voyage::LEG / crate::voyage::SPEED * (Biome::leg_of(leg) as f64 + 0.1);
            let span = crate::voyage::LEG / crate::voyage::SPEED * 0.8;
            let (mut parties, mut visible) = (0, 0);
            for i in 0..60 {
                let seconds = start + span * f64::from(i) / 60.0;
                let at = voyage.advance(seconds, 0.0);
                let pose = helm.steer(&seabed, at, seconds);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                let mut frame = Frame::new();
                build(&world, &mut frame, &mut StaticCache::default());
                let bodies = &frame.solid[Model::Diver.index()];
                if bodies.is_empty() {
                    continue;
                }
                parties += 1;
                let here = bodies
                    .iter()
                    .filter(|b| {
                        world.visible([b.rows[0][3], b.rows[1][3] + 0.08, b.rows[2][3]], 0.10)
                    })
                    .count();
                if here > 0 {
                    visible += 1;
                }
                if i == 0 {
                    println!("pose {:?} basis {:?}", world.pose.pos, world.pose.basis);
                    for b in bodies {
                        println!(
                            "body {:?} d {:?}",
                            [b.rows[0][3], b.rows[1][3], b.rows[2][3]],
                            [0, 1, 2].map(|k| { b.rows[k][3] - world.pose.pos[k] })
                        );
                    }
                }
            }
            println!("{name}: {parties} parties, {visible} frames with a diver in view");
            // A diagnostic, not a contract: it samples by time, and one
            // party's seven-unit slot can miss a leg's window entirely.
            // `divers_walk_the_forest_and_atlantis` is what holds the
            // behaviour, and it syncs the boat to the party instead.
            if parties == 0 {
                println!("  (no party's slot fell inside this leg's window)");
            }
        }
    }

    /// No fish is ever drawn with its body in the seabed, however the school
    /// straddles a rise in it or bends as it turns.
    #[test]
    fn fish_keep_off_the_seabed() {
        let mut seen = 0;
        for visit in [0u64, 500_000] {
            let seed = (visit % 1_000_003) as f32 / 1_000_003.0;
            let seabed = Seabed::new(visit, seed);
            let mut voyage = Voyage::default();
            let mut helm = super::super::helm::Helm::default();
            let mut frame = Frame::new();
            for i in 0..600 {
                let seconds = 5.0 + i as f64 * 3.0;
                let at = voyage.advance(seconds, seed);
                let pose = helm.steer(&seabed, at, seconds);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                frame.clear();
                shoals(&world, &mut frame);
                for fish in &frame.solid[Model::Fish.index()] {
                    let rows = fish.rows;
                    let p = [rows[0][3], rows[1][3], rows[2][3]];
                    let scale =
                        (rows[1][0].powi(2) + rows[1][1].powi(2) + rows[1][2].powi(2)).sqrt();
                    // The fish's body reaches about 0.40 of its length below
                    // its centre, a little more as it noses up or down.
                    let bottom = p[1] - 0.45 * scale;
                    let ground = seabed.ground(p[0], p[2]);
                    assert!(
                        bottom >= ground - 1e-3,
                        "a fish {:.3} into the seabed at {seconds:.1} s",
                        ground - bottom
                    );
                    seen += 1;
                }
            }
        }
        assert!(seen > 1000, "only {seen} fish in view");
    }

    /// A sister never passes through a kelp stalk: where a bed stands in her
    /// way she rises over it, and the drawn hull keeps clear of every blade.
    #[test]
    fn sisters_keep_clear_of_the_kelp() {
        let mut seen = 0;
        let mut blades = 0;
        for visit in [0u64, 500_000] {
            let seed = (visit % 1_000_003) as f32 / 1_000_003.0;
            let seabed = Seabed::new(visit, seed);
            let mut voyage = Voyage::default();
            let mut helm = super::super::helm::Helm::default();
            let mut frame = Frame::new();
            for i in 0..1800 {
                let seconds = 5.0 + i as f64;
                let at = voyage.advance(seconds, seed);
                let pose = helm.steer(&seabed, at, seconds);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                build(&world, &mut frame, &mut StaticCache::default());
                let Some(hull) = frame.solid[Model::Submarine.index()].first() else {
                    continue;
                };
                seen += 1;
                let rows = hull.rows;
                let turn = rows.map(|row| [row[0], row[1], row[2]].map(|v| v / SUB_LENGTH));
                let p = [rows[0][3], rows[1][3], rows[2][3]];
                let to_local = |q: [f32; 3]| {
                    let d = [q[0] - p[0], q[1] - p[1], q[2] - p[2]];
                    [0, 1, 2].map(|c| turn[0][c] * d[0] + turn[1][c] * d[1] + turn[2][c] * d[2])
                };
                for plant in &frame.solid[Model::Kelp.index()] {
                    if plant.extra[2] < 0.05 {
                        continue;
                    }
                    blades += 1;
                    let base = [plant.rows[0][3], plant.rows[1][3], plant.rows[2][3]];
                    let tall = (plant.rows[1][0].powi(2)
                        + plant.rows[1][1].powi(2)
                        + plant.rows[1][2].powi(2))
                    .sqrt();
                    for k in 0..=10 {
                        let q = [base[0], base[1] + tall * k as f32 / 10.0, base[2]];
                        let local = to_local(q);
                        let across = local[1].hypot(local[2]);
                        assert!(
                            local[0].abs() > 0.5 || across > 0.1,
                            "a sister passes through kelp at {seconds:.1} s: \
                                 {across:.3} across at {:.2} along",
                            local[0]
                        );
                    }
                }
            }
        }
        assert!(seen > 100, "only {seen} frames with a sister in view");
        assert!(blades > 100, "only {blades} kelp blades in view with one");
    }

    /// Bubbles never pop in or out of view. Over a few seconds of the divers'
    /// helmet trails, a flight of penguins' flipper trails and a submarine's
    /// screw wake, every bright bubble near the window has a bright bubble of
    /// the frame before within its own drift: a lifecycle that recycled a
    /// visible bubble, a trail snapping back to its emitter, or a sprite cut
    /// off at the brass would leave a bright bubble with no neighbour.
    #[test]
    fn bubbles_never_pop() {
        // Moments of visit 0 with a trail near the glass: the forest's
        // Moments of visit 0 with a trail near the glass: the forest's
        // walking party, and a sister submarine coming up on the shallows.
        // The exact seconds are the ones the encounters run on in the
        // canonical circuit; the helmsman's swing turns the window a little
        // off the track, so a meeting that used to sit on the glass can move
        // out of view, and these are the ones that still cross it. Both are
        // early legs, so the drowned city later in the circuit does not move
        // them.
        let scenes = [("diver helmets", 630.0), ("shallows encounter", 840.0)];
        for (name, start) in scenes {
            let seed = 0.0f32;
            let seabed = Seabed::new(0, seed);
            let mut voyage = Voyage::default();
            let mut helm = super::super::helm::Helm::default();
            let mut frame = Frame::new();
            // Settle the helm as the previews do.
            for i in 0..600 {
                let at = start - 20.0 + f64::from(i) / 30.0;
                let position = voyage.advance(at, seed);
                helm.steer(&seabed, position, at);
            }
            let mut last: Vec<[f32; 3]> = Vec::new();
            let (mut pops, mut worst, mut seen) = (0, 0.0f32, 0);
            for i in 0..360 {
                let at = start + f64::from(i) / 60.0;
                let position = voyage.advance(at, seed);
                let pose = helm.steer(&seabed, position, at);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: at as f32,
                    slack: 0.0,
                };
                build(&world, &mut frame, &mut StaticCache::default());
                let now: Vec<([f32; 3], f32)> = frame
                    .sprites
                    .iter()
                    .filter(|s| (s.tint[3] - BUBBLE).abs() < 0.5)
                    .map(|s| {
                        (
                            [s.place[0], s.place[1], s.place[2]],
                            s.tint[0].max(s.tint[1]).max(s.tint[2]),
                        )
                    })
                    .collect();
                seen += now.len();
                if i > 0 {
                    for &(p, _) in now.iter().filter(|(_, bright)| *bright > 0.15) {
                        // Far from the window nothing is on the glass: a
                        // sister's distant screw or a party walking in at the
                        // edge of its range may enter whole, which is the
                        // placer, not a bubble bursting. The near ones are
                        // what this holds.
                        let cam = ((p[0] - pose.pos[0]).powi(2)
                            + (p[1] - pose.pos[1]).powi(2)
                            + (p[2] - pose.pos[2]).powi(2))
                        .sqrt();
                        // Only the water on the glass matters: past 1.6 the
                        // bubble is off the window's edge and may enter or
                        // leave whole.
                        if cam > 1.6 {
                            continue;
                        }
                        let d = last
                            .iter()
                            .map(|q| {
                                ((p[0] - q[0]).powi(2)
                                    + (p[1] - q[1]).powi(2)
                                    + (p[2] - q[2]).powi(2))
                                .sqrt()
                            })
                            .fold(f32::MAX, f32::min);
                        if d > 0.03 {
                            pops += 1;
                            worst = worst.max(d);
                            println!(
                                "{name} at {at:.2} s: a bright bubble jumped {d:.3} \
                                 ({cam:.2} from the window)"
                            );
                        }
                    }
                }
                last = now.into_iter().map(|(p, _)| p).collect();
            }
            assert!(
                seen > 100,
                "{name}: only {seen} bubbles in the whole sample"
            );
            assert_eq!(pops, 0, "{name}: {pops} bubbles popped, worst {worst:.3}");
        }
    }

    /// Nothing the static cache holds pops in at the window's rim. Sailing
    /// through the reef, the cave and the forest as the helm swings the
    /// boat, every static instance a fresh build would place well inside the
    /// window is already in the frame built from the standing cache.
    #[test]
    fn the_static_cache_never_lags_the_window() {
        let seed = 0.0f32;
        let seabed = Seabed::new(0, seed);
        let span = crate::voyage::LEG / crate::voyage::SPEED;
        let step = 0.1;
        let key = |m: usize, i: &Instance| {
            let r = i.rows;
            (
                m,
                [r[0][3], r[1][3], r[2][3]].map(|v| (v * 1e4).round() as i64),
            )
        };
        let mut missed = 0usize;
        for biome in [CORAL, CAVE, FOREST] {
            let leg = seabed
                .order
                .iter()
                .position(|&b| b == biome)
                .expect("a leg");
            let mut voyage = Voyage::default();
            let mut helm = super::super::helm::Helm::default();
            let mut cache = StaticCache::default();
            let (mut cached, mut fresh) = (Frame::new(), Frame::new());
            let start = (leg as f64 + 0.2) * span;
            for i in 0..900 {
                let seconds = start + f64::from(i) * step;
                let at = voyage.advance(seconds, seed);
                let pose = helm.steer(&seabed, at, seconds);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                    slack: 0.0,
                };
                build(&world, &mut cached, &mut cache);
                let mut exact = StaticCache {
                    exact: true,
                    ..StaticCache::default()
                };
                build(&world, &mut fresh, &mut exact);
                let have: std::collections::HashSet<_> = (cached.solid.iter().enumerate())
                    .chain(cached.glass.iter().enumerate())
                    .flat_map(|(m, list)| list.iter().map(move |i| key(m, i)))
                    .collect();
                for (m, list) in fresh
                    .solid
                    .iter()
                    .enumerate()
                    .chain(fresh.glass.iter().enumerate())
                {
                    for instance in list {
                        let r = instance.rows;
                        let p = [r[0][3], r[1][3], r[2][3]];
                        // Well inside the window, not just grazing its rim.
                        if world.reveal(p, 0.0) < 0.5 || have.contains(&key(m, instance)) {
                            continue;
                        }
                        missed += 1;
                        if missed <= 10 {
                            let out = world.pose.basis[2];
                            let depth = (0..3).map(|k| (p[k] - pose.pos[k]) * out[k]).sum::<f32>();
                            println!(
                                "{biome} at {seconds:.1} s: model {m} at {p:?}, {depth:.2} out, \
                                 missing from the cache"
                            );
                        }
                    }
                }
            }
        }
        assert_eq!(missed, 0, "{missed} instances would have popped in");
    }

    /// No bubble is ever drawn on the glass. Over every leg of the circuit and
    /// a swipe's passage, every BUBBLE sprite stands at least [`BUBBLE_CLEAR`]
    /// from the window, and every bright one has one of the frame before
    /// within its own drift — the `bubbles_never_pop` pattern — so dropping the
    /// hull's stream left the other trails whole.
    #[test]
    fn bubbles_keep_off_the_glass() {
        let seed = 0.0f32;
        let seabed = Seabed::new(0, seed);
        let mut voyage = Voyage::default();
        let mut helm = super::super::helm::Helm::default();
        let mut frame = Frame::new();
        let span = crate::voyage::LEG / crate::voyage::SPEED;
        // Every sprite's own drift is under a hundredth of a unit between
        // these frames, so the 0.03 pop threshold still holds.
        let step = 1.0 / 30.0;
        // All fourteen legs, then the swipe's passage and the sea beyond it.
        let sail = span * crate::voyage::BIOMES as f64;
        let swipe = ((sail - 5.0) / step) as i32;
        let frames = ((sail + 8.0 - 5.0) / step) as i32;
        let mut last: Vec<([f32; 3], f32)> = Vec::new();
        let (mut seen, mut nearest, mut pops) = (0usize, f32::MAX, 0usize);
        let mut worst = 0.0f32;
        for i in 0..frames {
            let seconds = 5.0 + f64::from(i) * step;
            if i == swipe {
                voyage.travel(2, seconds);
            }
            let at = voyage.advance(seconds, seed);
            let pose = helm.steer(&seabed, at, seconds);
            let speed = (pose.vel[0].powi(2) + pose.vel[1].powi(2) + pose.vel[2].powi(2)).sqrt();
            let world = World {
                seabed: &seabed,
                pose,
                t: seconds as f32,
                slack: 0.0,
            };
            build(&world, &mut frame, &mut StaticCache::default());
            let now: Vec<([f32; 3], f32)> = frame
                .sprites
                .iter()
                .filter(|s| (s.tint[3] - BUBBLE).abs() < 0.5)
                .map(|s| {
                    (
                        [s.place[0], s.place[1], s.place[2]],
                        s.tint[0].max(s.tint[1]).max(s.tint[2]),
                    )
                })
                .collect();
            seen += now.len();
            for &(p, _) in &now {
                let d = ((p[0] - pose.pos[0]).powi(2)
                    + (p[1] - pose.pos[1]).powi(2)
                    + (p[2] - pose.pos[2]).powi(2))
                .sqrt();
                nearest = nearest.min(d);
                assert!(
                    d >= BUBBLE_CLEAR - 0.03,
                    "a bubble {d:.3} from the window at {seconds:.1} s"
                );
            }
            // A swipe runs the voyage's clock fast, so the whole sea moves
            // under the window; only the cruise is held to a bubble's drift.
            if i > 0 && speed < 0.2 {
                for &(p, _) in now.iter().filter(|(_, bright)| *bright > 0.15) {
                    let cam = ((p[0] - pose.pos[0]).powi(2)
                        + (p[1] - pose.pos[1]).powi(2)
                        + (p[2] - pose.pos[2]).powi(2))
                    .sqrt();
                    // A party walking in at the edge of its seven-unit range
                    // appears whole, which is the placer, not a bubble bursting.
                    // The glass is at about one unit; 1.25 is still just
                    // outside it, where a sub's screw trail may enter whole.
                    if cam > 1.20 {
                        continue;
                    }
                    let d = last
                        .iter()
                        .map(|q| {
                            ((p[0] - q.0[0]).powi(2)
                                + (p[1] - q.0[1]).powi(2)
                                + (p[2] - q.0[2]).powi(2))
                            .sqrt()
                        })
                        .fold(f32::MAX, f32::min);
                    if d > 0.03 {
                        pops += 1;
                        worst = worst.max(d);
                        println!(
                            "a bright bubble jumped {d:.3} at {seconds:.2} s ({cam:.2} from the window)"
                        );
                    }
                }
            }
            last = now;
        }
        assert!(seen > 500, "only {seen} bubbles in the whole circuit");
        println!("nearest bubble {nearest:.3} from the window, {seen} seen");
        assert_eq!(pops, 0, "{pops} bubbles jumped, worst {worst:.3}");
    }

    /// The biome profiles are the same function in Rust and in `scene.wesl`.
    /// This is the shader's `biome_shape`, term for term; the two are held
    /// together over a grid of every biome pair, and the shape is periodic
    /// over the heightmap's 64 units so the added relief wraps with it.
    #[test]
    fn the_biome_shapes_match_the_shader_and_wrap() {
        use std::f32::consts::TAU;
        fn shader_shape(b: &Biome, x: f32, z: f32, across: f32, h: f32, detail: f32) -> f32 {
            let presence = |id: usize| b.weight(id) as f32;
            let mut shape = 0.0;
            let shallows = presence(SHALLOWS);
            if shallows > 0.0 {
                let banks = smooth(0.45, 0.72, 0.5 + 0.5 * detail);
                let waves = (TAU * (x * 0.25 + z * 0.125) + 1.4 * detail).sin()
                    + 0.5 * (TAU * (x * 0.125 - z * 0.25) + 0.8 * detail).sin();
                shape += shallows * (0.10 * h + 0.10 * banks + 0.05 * waves);
            }
            let forest = presence(FOREST);
            if forest > 0.0 {
                let hummocks = (TAU * (x * 0.5 + z * 0.25) + 1.2 * detail).sin()
                    + 0.6 * (TAU * (x * 0.25 - z * 0.5) + 0.7 * detail).sin();
                let mounds = smooth(0.45, 0.75, 0.5 + 0.5 * detail);
                shape += forest * (-0.05 * h + 0.12 * hummocks + 0.25 * detail + 0.30 * mounds);
            }
            let wrecks = presence(WRECKS);
            if wrecks > 0.0 {
                let drifts = (TAU * z * 0.125 + 0.8 * detail).sin()
                    + 0.4 * (TAU * (z * 0.375 + x * 0.0625)).sin();
                let rocks = smooth(0.60, 0.85, 0.5 + 0.5 * detail);
                shape += wrecks * (0.25 * h + 0.20 * drifts + 0.40 * rocks);
            }
            let atlantis = presence(ATLANTIS);
            if atlantis > 0.0 {
                let terrace = |v: f32| {
                    2.0 * smooth(-0.22, 0.22, (TAU * v * 0.25 + 0.25 * detail).sin()) - 1.0
                };
                shape += atlantis
                    * (-0.11 * h + 0.22 * terrace(x) + 0.18 * terrace(z * 2.0) + 0.02 * detail);
            }
            let cave = presence(CAVE);
            if cave > 0.0 {
                let ledges = 2.0
                    * smooth(-0.35, 0.35, (TAU * across.abs() / 1.2 + 0.4 * detail).sin())
                    - 1.0;
                let k = 1.55 + 1.25 * hall(x);
                let near = smooth(0.45 * k, 0.9 * k, across.abs());
                shape += cave * near * (0.25 * ledges + 0.12 * detail);
            }
            let abyss = presence(ABYSS);
            if abyss > 0.0 {
                let fold = 1.0 - 2.0 * detail.abs();
                let canyon = 1.0 - smooth(0.0, 0.09, (h - 0.25).abs());
                shape += abyss * (0.32 * h + 0.65 * (fold * fold - 0.35) - 0.45 * canyon);
            }
            let arctic = presence(ARCTIC);
            if arctic > 0.0 {
                let scour = 1.0 - smooth(0.0, 0.22, (TAU * z * 0.5 + 0.7 * detail).sin().abs());
                shape += arctic * (-0.10 * h + 0.06 * detail - 0.12 * scour);
            }
            let trench = presence(TRENCH);
            if trench > 0.0 {
                shape += trench * (0.05 * h + 0.08 * detail);
            }
            let coral = presence(CORAL);
            if coral > 0.0 {
                let field = 0.5
                    + 0.22 * (TAU * (x * 0.25 + z * 0.125) + 1.3 * detail).sin()
                    + 0.20 * (TAU * (x * 0.125 - z * 0.25) + 0.9 * detail).sin()
                    + 0.12 * (TAU * (x * 0.375 + z * 0.3125) + 2.1 * detail).sin();
                let mound = smooth(0.50, 0.82, field);
                let gutter = 1.0 - smooth(0.12, 0.32, field);
                shape += coral
                    * (0.15 * h + 0.42 * mound - 0.10 * gutter
                        + 0.06 * detail * mound
                        + 0.03 * detail);
            }
            let sargasso = presence(SARGASSO);
            if sargasso > 0.0 {
                let drift = (TAU * (x * 0.125 + z * 0.0625) + 0.9 * detail).sin();
                let swell = smooth(0.42, 0.74, 0.5 + 0.5 * detail);
                shape += sargasso * (0.04 * h + 0.10 * drift + 0.16 * swell);
            }
            let vents = presence(VENTS);
            if vents > 0.0 {
                let fold = 1.0 - 2.0 * detail.abs();
                let mound = smooth(0.58, 0.86, 0.5 + 0.5 * detail);
                shape += vents * (0.28 * h + 0.42 * (fold * fold - 0.35) + 0.30 * mound);
            }
            let brine = presence(BRINE);
            if brine > 0.0 {
                let terrace =
                    2.0 * smooth(-0.55, 0.55, (TAU * x * 0.125 + 0.2 * detail).sin()) - 1.0;
                let pan = smooth(0.62, 0.88, 0.5 + 0.5 * detail);
                shape += brine * (0.04 * h + 0.055 * terrace - 0.06 * pan + 0.02 * detail);
            }
            let bloom = presence(BLOOM);
            if bloom > 0.0 {
                let ooze = (TAU * (x * 0.5 + z * 0.25) + 1.0 * detail).sin()
                    + 0.6 * (TAU * (x * 0.25 - z * 0.5) + 0.6 * detail).sin();
                let mounds = smooth(0.40, 0.72, 0.5 + 0.5 * detail);
                shape += bloom * (-0.05 * h + 0.10 * ooze + 0.22 * mounds);
            }
            let city = presence(CITY);
            if city > 0.0 {
                let step =
                    2.0 * smooth(
                        -0.3,
                        0.3,
                        (TAU * (x * 0.0625 + z * 0.125) + 0.3 * detail).sin(),
                    ) - 1.0;
                shape += city * (-0.06 * h + 0.10 * step + 0.015 * detail);
            }
            shape
        }

        let ids = [
            SHALLOWS, FOREST, WRECKS, ATLANTIS, CAVE, ABYSS, ARCTIC, TRENCH, CORAL, SARGASSO,
            VENTS, BRINE, BLOOM, CITY,
        ];
        let weights = [0.0, 0.2, 0.5, 0.8, 1.0];
        for &a in &ids {
            for &c in &ids {
                for w in weights {
                    let b = Biome { a, b: c, w };
                    for i in 0..7 {
                        for j in 0..7 {
                            let x = -31.7 + i as f32 * 11.3;
                            let z = -27.1 + j as f32 * 9.7;
                            let across = -6.0 + i as f32 * 2.0;
                            let h = -1.4 + j as f32 * 0.45;
                            let detail = -0.9 + (i + j) as f32 * 0.15;
                            let want = shader_shape(&b, x, z, across, h, detail);
                            let got = biome_shape(&b, x, z, across, h, detail);
                            assert!(
                                (want - got).abs() < 2e-6,
                                "{a}/{c} w {w} at ({x}, {z}): {got} != {want}"
                            );
                        }
                    }
                }
            }
        }
        // Every term of the shape repeats with the heightmap, but the cave's,
        // which widens with its halls along the leg.
        for &id in ids.iter().filter(|&&id| id != CAVE) {
            let b = Biome {
                a: id,
                b: id,
                w: 0.0,
            };
            for i in 0..20 {
                let x = i as f32 * 2.7 - 20.0;
                let z = i as f32 * -3.1 + 7.0;
                let h = (x * 0.37).sin();
                let detail = (z * 0.29).cos();
                let here = biome_shape(&b, x, z, 0.8, h, detail);
                let next = biome_shape(&b, x + 64.0, z + 64.0, 0.8, h, detail);
                assert!((here - next).abs() < 2e-5, "biome {id} seam at {x}, {z}");
            }
        }
    }
}
