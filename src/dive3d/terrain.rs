//! The seabed, built on the CPU a tile at a time as it comes into view, and
//! forgotten once the boat has left it behind.
//!
//! Three levels of detail, each a square of tiles around the boat: fine
//! nearby, coarse out to the horizon. A level's square is made of whole tiles
//! of the next coarser level, which leaves exactly those out, so no two
//! levels overlap. A skirt hangs from every tile's edges and hides the cracks
//! where two levels meet. Only the tiles in view are drawn, nearest first.

use std::collections::HashMap;
use std::f32::consts::TAU;

use super::world::{self, World};

/// Quads along a tile's side.
const QUADS: usize = 16;
const SIDE: usize = QUADS + 1;
/// Vertices in one tile: its grid, then a skirt along each of its edges.
pub const VERTICES: usize = SIDE * SIDE + 4 * SIDE;
/// The spacing of each level's vertices, finest first. Each level's tiles are
/// `RATIO` of the next finer level's across.
const SPACING: [f32; 3] = [0.08, 0.32, 1.28];
const RATIO: i32 = 4;
/// How many tiles the coarsest level reaches either side of the boat's.
const REACH: i32 = 2;
/// How many tiles of the next coarser level a finer level's square reaches
/// either side of the boat's: the finest reaches at least five units, the
/// middle one at least twenty.
const NEST: i32 = 1;
/// How far a skirt hangs below its edge, in its level's spacings.
const SKIRT: f32 = 4.0;
/// Beyond what the seabed's height can ever be, either way.
const LOWEST: f32 = -16.0;
const HIGHEST: f32 = 8.0;
/// Room on the GPU for every tile the squares can hold at once.
pub const SLOTS: usize = {
    let nested = ((2 * NEST + 1) * RATIO) as usize;
    let top = (2 * REACH + 1) as usize;
    (SPACING.len() - 1) * nested * nested + top * top
};

/// A vertex of the seabed, in the layout `vs_terrain` reads. Everything about
/// how the floor looks that does not depend on the camera is worked out once,
/// here, when the tile is built: the shader only applies how much of the fine
/// detail the distance lets through.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Point {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    /// rgb: the floor's colour at full detail; a: its surface layer.
    pub albedo: [f32; 4],
    /// x: the fine noise at full strength; y: how flat the face is, so the
    /// noise shows most on level ground; z: the sand ripples, or the lava
    /// cave's cooling cracks; w: the paving's joints, or its finer cracks.
    pub shade: [f32; 4],
    /// The lava cave's glow, at full strength, rgb; a: how much cave there is
    /// here, so the shader can project a cave cliff off its face.
    pub glow: [f32; 4],
}

/// The surface texture each biome's floor wears, as `surface_of` in
/// `scene.wesl`: sand, sand, silt, sand, basalt, silt, sand, silt, then sand,
/// silt, basalt, salt (the ice layer's crystalline tone) and ooze (silt).
const LAYERS: [usize; 13] = [12, 12, 13, 12, 10, 13, 12, 13, 12, 13, 10, 11, 13];
/// Biome ids, as the shader's constants.
const SHALLOWS: usize = 0;
const FOREST: usize = 1;
const WRECKS: usize = 2;
const ATLANTIS: usize = 3;
const CAVE: usize = 4;
const ABYSS: usize = 5;
const ARCTIC: usize = 6;
const TRENCH: usize = 7;
const CORAL: usize = 8;
const SARGASSO: usize = 9;
const VENTS: usize = 10;
const BRINE: usize = 11;
const BLOOM: usize = 12;

/// `smoothstep` as the shader writes it.
fn smooth01(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the vertex shader used to work out per vertex, every frame: the
/// floor's colour and texture layer, how strongly its fine detail shows, and
/// the lava cave's glow. All of it depends only on where the vertex is.
///
/// The two noise fields are shared by every biome: `broad` patches the floor
/// on a scale of a couple of units, `grain` is the fine speckle. Each biome
/// weights them its own way — rust bleeding through the wreck field's silt,
/// algae in the forest, veins in the marble, soot on the cave's basalt — and
/// the ripples are strongest on the shallows' sand, faint on silt.
fn shade(
    w: &World,
    x: f32,
    z: f32,
    normal: [f32; 3],
    cavity: f32,
) -> ([f32; 4], [f32; 4], [f32; 4]) {
    let b = w.seabed.biome(x, z);
    let t = b.w as f32;
    let presence = |id: usize| {
        if b.a == id {
            1.0 - t
        } else if b.b == id {
            t
        } else {
            0.0
        }
    };
    let mut base = world::floor_albedo(&b);
    // The shader's `(1.0 - steep)`: how level this piece of floor is.
    let flat = 1.0 - smooth01(0.12, 0.40, 1.0 - normal[1]);
    let across = z - world::corridor(x);
    let broad = world::vnoise(x * 0.5, z * 0.5, 10);
    let grain = world::vnoise(x * 14.0, z * 14.0, 9);
    let mut noise = 0.65 + 0.5 * grain + 0.08 * cavity;
    let mut tint = [0.0f32; 3];
    let mut ripple = 0.0;

    let shallows = presence(SHALLOWS);
    if shallows > 0.0 {
        // Sand: broad banks paler at their tops, finer speckle between.
        noise += shallows * (0.10 * (broad - 0.5) + 0.04 * (grain - 0.5));
        tint[0] += shallows * 0.06 * (broad - 0.5);
        tint[1] += shallows * 0.045 * (broad - 0.5);
        tint[2] += shallows * 0.02 * (broad - 0.5);
    }
    let forest = presence(FOREST);
    if forest > 0.0 {
        // Silt: darker where it lies deep, a green cast in the hollows.
        noise += forest * (-0.08 + 0.06 * (broad - 0.5));
        tint[0] -= forest * 0.06 * (broad - 0.5);
        tint[1] += forest * 0.015 * (broad - 0.5);
        tint[2] -= forest * 0.02 * (broad - 0.5);
    }
    let wrecks = presence(WRECKS);
    if wrecks > 0.0 {
        // Sediment drifts with rust bleeding through where they thin.
        let rust = smooth01(0.62, 0.86, broad);
        noise += wrecks * (0.10 * (broad - 0.5) + 0.05 * (grain - 0.5));
        tint[0] += wrecks * 0.14 * rust;
        tint[1] += wrecks * 0.05 * rust;
        tint[2] += wrecks * 0.005 * rust;
    }
    let atlantis = presence(ATLANTIS);
    if atlantis > 0.0 {
        // Marble: pale stone crossed by darker veins.
        let vein = smooth01(0.70, 0.88, grain);
        noise += atlantis * 0.05 * (grain - 0.5);
        tint[0] -= atlantis * 0.06 * vein;
        tint[1] -= atlantis * 0.055 * vein;
        tint[2] -= atlantis * 0.04 * vein;
    }
    let cave = presence(CAVE);
    if cave > 0.0 {
        // Basalt: pitted and sooted, its hollows darker than its faces.
        let soot = smooth01(0.55, 0.80, broad);
        noise += cave * (0.14 * (grain - 0.5) - 0.06 * soot);
        tint[0] += cave * 0.05 * (grain - 0.5);
        tint[1] += cave * 0.015 * (grain - 0.5);
    }
    let abyss = presence(ABYSS);
    if abyss > 0.0 {
        // Cold, hard rock, its grain showing strongly.
        noise += abyss * (0.20 * (grain - 0.5) + 0.10 * (broad - 0.5));
        tint[0] -= abyss * 0.02 * (broad - 0.5);
        tint[1] += abyss * 0.015 * (broad - 0.5);
        tint[2] += abyss * 0.04 * (broad - 0.5);
    }
    let arctic = presence(ARCTIC);
    if arctic > 0.0 {
        // Ice-scoured floor: pale streaks along the grooves.
        let scour = 1.0 - smooth01(0.0, 0.22, (TAU * z * 0.5 + 0.7 * broad).sin().abs());
        noise -= arctic * 0.06 * scour;
        tint[0] += arctic * 0.04 * scour;
        tint[1] += arctic * 0.05 * scour;
        tint[2] += arctic * 0.055 * scour;
    }
    let trench = presence(TRENCH);
    if trench > 0.0 {
        // The trench's walls read as ledged rock, not a smooth slope.
        let strata = 0.5 + 0.5 * ((across * 9.0 + 1.2 * (broad - 0.5)).sin());
        noise += trench * 0.12 * (strata - 0.5);
        tint[0] += trench * 0.05 * (strata - 0.5);
        tint[1] += trench * 0.04 * (strata - 0.5);
    }
    let coral = presence(CORAL);
    if coral > 0.0 {
        // Reef sand: pale between the heads, pinker where the coral has
        // crumbled into it, and a little coarser over the bommies.
        let bloom = smooth01(0.55, 0.82, broad);
        noise += coral * (0.09 * (broad - 0.5) + 0.05 * (grain - 0.5));
        tint[0] += coral * (0.10 * (broad - 0.5) + 0.05 * bloom);
        tint[1] += coral * (0.02 * (broad - 0.5) + 0.015 * bloom);
        tint[2] -= coral * 0.02 * bloom;
    }
    let sargasso = presence(SARGASSO);
    if sargasso > 0.0 {
        // Weed drift on the silt: olive where it has rotted in, paler sand
        // showing through the thinner patches.
        let drift = smooth01(0.58, 0.84, broad);
        noise += sargasso * (-0.07 + 0.08 * (broad - 0.5));
        tint[0] -= sargasso * 0.05 * drift;
        tint[1] += sargasso * 0.04 * (broad - 0.5);
        tint[2] -= sargasso * 0.04 * drift;
    }
    let vents = presence(VENTS);
    if vents > 0.0 {
        // Fresh basalt and sulphur staining: the floor is darkest between
        // the chimneys and bleached pale round the hot mouths.
        let sulphur = smooth01(0.66, 0.90, broad);
        noise += vents * (0.18 * (grain - 0.5) - 0.05 * sulphur);
        tint[0] += vents * (0.16 * sulphur + 0.03 * (grain - 0.5));
        tint[1] += vents * (0.10 * sulphur + 0.01 * (grain - 0.5));
        tint[2] -= vents * 0.03 * sulphur;
    }
    let brine = presence(BRINE);
    if brine > 0.0 {
        // Salt crust: a pale polygonal mosaic, its cracks reading darker and
        // the pans a shade greener.
        let frac = |v: f32| v - v.floor();
        let crack = (frac(x * 2.0 + z * 0.3) - 0.5)
            .abs()
            .min((frac(z * 2.0 - x * 0.2) - 0.5).abs());
        let rim = smooth01(0.44, 0.5, crack);
        let pan = smooth01(0.64, 0.88, broad);
        noise += brine * (0.10 * (grain - 0.5) - 0.10 * rim);
        tint[0] += brine * (0.03 * rim - 0.03 * pan);
        tint[1] += brine * (0.03 * rim + 0.01 * pan);
        tint[2] += brine * (0.02 * rim + 0.03 * pan);
    }
    let bloom = presence(BLOOM);
    if bloom > 0.0 {
        // Soft ooze, faintly blue-green where the bloom's light falls on it.
        let ooze = smooth01(0.56, 0.82, broad);
        noise += bloom * (-0.06 + 0.06 * (grain - 0.5));
        tint[0] += bloom * 0.01 * (broad - 0.5);
        tint[1] += bloom * (0.02 * ooze + 0.01 * (broad - 0.5));
        tint[2] += bloom * (0.04 * ooze + 0.02 * (broad - 0.5));
    }
    let sand = shallows + 0.55 * forest + 0.35 * wrecks + 0.6 * arctic + 0.75 * coral;
    if sand > 0.0 {
        // Ripples in patches, so the sand does not corrugate as one sheet.
        let r = ((x * 0.8 + z) * 26.0 + world::vnoise(x * 3.0, z * 3.0, 5) * 4.0).sin();
        ripple = 0.16 * sand * (0.5 + 0.5 * broad) * smooth01(0.3, 1.0, r);
    }
    let paving = presence(ATLANTIS) + 0.55 * presence(BRINE);
    let mut pave = if paving > 0.0 {
        let frac = |v: f32| v - v.floor();
        let row = (z * 4.0).floor();
        let joint = (frac(x * 3.0 + row * 0.5) - 0.5)
            .abs()
            .min((frac(z * 4.0) - 0.5).abs());
        0.45 * paving * smooth01(0.45, 0.5, joint)
    } else {
        0.0
    };
    let mut glow = [0.0; 4];
    if cave > 0.0 {
        // Ledged basalt, not one dark sheet: flows of crust with ash drifted
        // between them, mineral staining where water has run, and cooling
        // cracks across the level ground. Baked per vertex, so the shader
        // only applies the distance fade.
        let mix3 = |a: [f32; 3], b: [f32; 3], t: f32| [0, 1, 2].map(|j| a[j] + (b[j] - a[j]) * t);
        let crust = world::vnoise(x * 0.9 + 2.0, z * 1.3, 60);
        let ash = smooth01(0.52, 0.78, world::vnoise(x * 3.1, z * 2.7 + 4.0, 61));
        let rust = smooth01(0.60, 0.84, world::vnoise(x * 1.5, z * 1.9, 62));
        let crust = [0.19 + 0.07 * crust, 0.115 + 0.045 * crust, 0.075];
        let target = mix3(crust, [0.075, 0.078, 0.082], ash * 0.7);
        let target = mix3(target, [0.22, 0.10, 0.055], rust * 0.35);
        // The massif rises wherever there is any cave at all, while the floor
        // about it is still mostly the last sea's: its faces are basalt
        // however little of the cave the blend has reached, so the cliff over
        // the tunnel's mouth is not a hill of pale sand or marble.
        let cliff = (1.0 - flat) * smooth01(0.0, 0.12, cave);
        base = mix3(base, target, (cave * 0.7).max(cliff * 0.9));
        let ledge = smooth01(0.54, 0.72, world::vnoise(x * 1.7, z * 2.1, 65));
        let pitted = smooth01(0.58, 0.80, world::vnoise(x * 6.5, z * 6.0, 66));
        noise *= (0.78 + 0.36 * ledge) * (1.0 - 0.24 * pitted);
        let cooling = 1.0
            - smooth01(
                0.0,
                0.030,
                (world::vnoise(x * 2.4, z * 3.2, 63) - 0.5).abs(),
            );
        let cooling2 = 1.0
            - smooth01(
                0.0,
                0.020,
                (world::vnoise(x * 5.0, z * 4.0, 64) - 0.5).abs(),
            );
        ripple += 0.55 * cave * flat * cooling;
        pave += 0.30 * cave * flat * cooling2;
        let veins = 1.0 - smooth01(0.0, 0.04, (world::vnoise(x * 1.4, z * 1.9, 6) - 0.5).abs());
        let fine = 1.0 - smooth01(0.0, 0.03, (world::vnoise(x * 4.0, z * 5.0, 7) - 0.5).abs());
        let pools = smooth01(0.66, 0.74, world::vnoise(x * 0.55 + 7.0, z * 0.55, 8));
        let heat = 0.8 + 0.4 * world::vnoise(x * 0.8 + 7.0, z * 0.8, 11);
        let k = (veins * 0.9 + fine * 0.5 + pools * 1.4) * cave * flat * heat;
        glow = [k, 0.36 * k, 0.07 * k, cave];
    }
    // Hollows lie in their own shade and crests catch the light: a little
    // baked ambient occlusion, so the relief reads even where the water is
    // too deep for the sun to shape it. Rock takes more than sand.
    let hard = presence(WRECKS)
        + presence(ATLANTIS)
        + presence(CAVE)
        + presence(ABYSS)
        + presence(TRENCH)
        + presence(CORAL)
        + presence(VENTS)
        + 0.7 * presence(BRINE);
    let cavity = cavity * (0.10 + 0.10 * hard);
    let colour = [
        base[0] * (1.0 + tint[0]) * (1.0 + cavity),
        base[1] * (1.0 + tint[1]) * (1.0 + cavity),
        base[2] * (1.0 + tint[2]) * (1.0 + cavity),
    ];
    (
        [colour[0], colour[1], colour[2], LAYERS[b.a] as f32],
        [noise, flat, ripple, pave],
        glow,
    )
}

/// One tile, by level and column and row of that level's tiles.
type Key = (usize, i32, i32);

struct Tile {
    slot: u32,
    centre: [f32; 3],
    radius: f32,
}

pub struct Terrain {
    tiles: HashMap<Key, Tile>,
    free: Vec<u32>,
    /// How many of `SPACING`'s levels to draw. Three is the full near/mid/
    /// far set; two drops the 0.08 m level, one keeps only the 1.28 m one.
    levels: usize,
}

impl Default for Terrain {
    fn default() -> Self {
        Self::with_levels(SPACING.len())
    }
}

impl Terrain {
    pub fn with_levels(levels: usize) -> Self {
        let levels = levels.clamp(1, SPACING.len());
        Self {
            tiles: HashMap::new(),
            free: (0..SLOTS as u32).rev().collect(),
            levels,
        }
    }

    /// The spacing of the `level`th drawn level: levels are dropped from the
    /// fine end, so the finest spacing grows and the far extent stays.
    fn spacing(&self, level: usize) -> f32 {
        SPACING[SPACING.len() - self.levels + level]
    }
}

impl Terrain {
    /// Forget every tile: the seabed has changed.
    pub fn clear(&mut self) {
        self.tiles.clear();
        self.free = (0..SLOTS as u32).rev().collect();
    }

    /// The slots of the tiles to draw, nearest first. Tiles coming into view
    /// for the first time are built and handed to `upload` with their slot;
    /// those the boat has left behind are dropped.
    pub fn update(&mut self, w: &World, mut upload: impl FnMut(u32, &[Point])) -> Vec<u32> {
        let c = w.pose.pos;
        let levels = self.levels;
        let spacings: Vec<f32> = (0..levels).map(|l| self.spacing(l)).collect();
        let tile = |level: usize| spacings[level] * QUADS as f32;
        // The boat's tile at each level.
        let at = |level: usize| {
            [
                (c[0] / tile(level)).floor() as i32,
                (c[2] / tile(level)).floor() as i32,
            ]
        };
        let top = levels - 1;
        // Each level's square, in its own tiles, as [first, last] per axis.
        let square = |level: usize| -> [[i32; 2]; 2] {
            if level == top {
                at(top).map(|b| [b - REACH, b + REACH])
            } else {
                at(level + 1).map(|b| [(b - NEST) * RATIO, (b + NEST + 1) * RATIO - 1])
            }
        };
        let squares: Vec<_> = (0..levels).map(square).collect();
        // Whether a tile is drawn: inside its level's square, and not where
        // the finer level's square lies instead.
        let wanted = |&(level, i, j): &Key| {
            let [x, z] = squares[level];
            let inside = x[0] <= i && i <= x[1] && z[0] <= j && j <= z[1];
            let finer = level > 0 && {
                let b = at(level);
                (i - b[0]).abs() <= NEST && (j - b[1]).abs() <= NEST
            };
            inside && !finer
        };
        let free = &mut self.free;
        self.tiles.retain(|key, t| {
            let keep = wanted(key);
            if !keep {
                free.push(t.slot);
            }
            keep
        });

        let mut drawn = Vec::new();
        for (level, [x, z]) in squares.iter().enumerate() {
            let size = tile(level);
            let spacing = spacings[level];
            for j in z[0]..=z[1] {
                for i in x[0]..=x[1] {
                    let key = (level, i, j);
                    if !wanted(&key) {
                        continue;
                    }
                    // A bound around every height the seabed can have first, so
                    // tiles out of view are not built.
                    if !self.tiles.contains_key(&key) {
                        let middle = [
                            (i as f32 + 0.5) * size,
                            (LOWEST + HIGHEST) * 0.5,
                            (j as f32 + 0.5) * size,
                        ];
                        let rise = (HIGHEST - LOWEST) * 0.5 + SKIRT * spacing;
                        if !w.visible(middle, (size * size * 0.5 + rise * rise).sqrt()) {
                            continue;
                        }
                        let Some(slot) = self.free.pop() else {
                            continue;
                        };
                        let (points, centre, radius) = build(w, spacing, i, j);
                        upload(slot, &points);
                        self.tiles.insert(
                            key,
                            Tile {
                                slot,
                                centre,
                                radius,
                            },
                        );
                    }
                    let t = &self.tiles[&key];
                    if w.visible(t.centre, t.radius) {
                        let d = [t.centre[0] - c[0], t.centre[1] - c[1], t.centre[2] - c[2]];
                        let reach = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - t.radius;
                        drawn.push((reach, t.slot));
                    }
                }
            }
        }
        drawn.sort_by(|a, b| a.0.total_cmp(&b.0));
        drawn.into_iter().map(|(_, slot)| slot).collect()
    }
}

/// Tile (i, j) of `level`: its vertices, and the centre and radius of a
/// sphere around them.
fn build(w: &World, s: f32, i: i32, j: i32) -> (Vec<Point>, [f32; 3], f32) {
    let (x0, z0) = (i * QUADS as i32, j * QUADS as i32);
    // Heights with a border one vertex wide, for the normals at the edges:
    // the same central differences, a spacing either way, as the lattice's.
    let border = SIDE + 2;
    let mut heights = vec![0.0f32; border * border];
    for b in 0..border {
        for a in 0..border {
            let x = (x0 + a as i32 - 1) as f32 * s;
            let z = (z0 + b as i32 - 1) as f32 * s;
            heights[b * border + a] = w.seabed.ground(x, z);
        }
    }
    let h = |a: usize, b: usize| heights[(b + 1) * border + a + 1];
    let mut points = Vec::with_capacity(VERTICES);
    for b in 0..SIDE {
        for a in 0..SIDE {
            let dx = heights[(b + 1) * border + a + 2] - heights[(b + 1) * border + a];
            let dz = heights[(b + 2) * border + a + 1] - heights[b * border + a + 1];
            let n = [-dx, 2.0 * s, -dz];
            let l = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
            let normal = [n[0] / l, n[1] / l, n[2] / l];
            let x = (x0 + a as i32) as f32 * s;
            let z = (z0 + b as i32) as f32 * s;
            // How much this vertex stands above or below its four
            // neighbours: positive on a crest, negative in a hollow. The
            // spacing cancels, so every level bakes the same strength.
            let c = heights[(b + 1) * border + a + 1];
            let bump = (4.0 * c
                - heights[(b + 1) * border + a]
                - heights[(b + 1) * border + a + 2]
                - heights[b * border + a + 1]
                - heights[(b + 2) * border + a + 1])
                / (4.0 * s * s);
            let cavity = (bump * 0.6).clamp(-1.0, 1.0);
            let (albedo, shade, glow) = shade(w, x, z, normal, cavity);
            points.push(Point {
                position: [x, h(a, b), z],
                normal,
                albedo,
                shade,
                glow,
            });
        }
    }
    // The skirts, under the four edges in the order `indices` stitches them.
    for edge in edges() {
        for k in 0..SIDE {
            let mut p = points[edge[k]];
            p.position[1] -= SKIRT * s;
            points.push(p);
        }
    }
    let (mut low, mut high) = (f32::MAX, f32::MIN);
    for p in &points {
        low = low.min(p.position[1]);
        high = high.max(p.position[1]);
    }
    let half = s * QUADS as f32 * 0.5;
    let centre = [
        x0 as f32 * s + half,
        (low + high) * 0.5,
        z0 as f32 * s + half,
    ];
    let rise = (high - low) * 0.5;
    let radius = (2.0 * half * half + rise * rise).sqrt();
    (points, centre, radius)
}

/// The grid vertices along each of a tile's four edges.
fn edges() -> [[usize; SIDE]; 4] {
    [
        std::array::from_fn(|k| k),
        std::array::from_fn(|k| QUADS * SIDE + k),
        std::array::from_fn(|k| k * SIDE),
        std::array::from_fn(|k| k * SIDE + QUADS),
    ]
}

/// The triangles of one tile, the same for every tile: the grid, split
/// corner to corner as the old rings were, then the skirts.
pub fn indices() -> Vec<u32> {
    let mut out = Vec::new();
    for b in 0..QUADS {
        for a in 0..QUADS {
            let p = (b * SIDE + a) as u32;
            let q = p + 1;
            let r = p + SIDE as u32;
            let t = r + 1;
            out.extend_from_slice(&[p, r, q, q, r, t]);
        }
    }
    for (e, edge) in edges().iter().enumerate() {
        let below = (SIDE * SIDE + e * SIDE) as u32;
        for k in 0..QUADS {
            let (p, q) = (edge[k] as u32, edge[k + 1] as u32);
            let (r, t) = (below + k as u32, below + k as u32 + 1);
            out.extend_from_slice(&[p, r, q, q, r, t]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dive3d::helm::Pose;
    use crate::dive3d::world::Seabed;

    #[test]
    fn tiles_cover_the_view_without_overlapping() {
        let seabed = Seabed::new(7, 0.3);
        let w = World {
            seabed: &seabed,
            pose: Pose::level(3.3, -2.0),
            t: 0.0,
        };
        let mut terrain = Terrain::default();
        let mut built = 0;
        let drawn = terrain.update(&w, |_, points| {
            assert_eq!(points.len(), VERTICES);
            built += 1;
        });
        assert!(!drawn.is_empty() && drawn.len() <= built);
        let mut unique = drawn.clone();
        unique.sort_unstable();
        unique.dedup();
        assert_eq!(unique.len(), drawn.len(), "a tile is drawn twice");
        // No two levels' tiles cover the same ground.
        for &(la, ia, ja) in terrain.tiles.keys() {
            for &(lb, ib, jb) in terrain.tiles.keys() {
                if la < lb {
                    let up = RATIO.pow((lb - la) as u32);
                    assert!(
                        ia.div_euclid(up) != ib || ja.div_euclid(up) != jb,
                        "level {la} tile ({ia}, {ja}) lies under level {lb} ({ib}, {jb})"
                    );
                }
            }
        }
        // A second look builds nothing new, and the same tiles are drawn.
        let again = terrain.update(&w, |_, _| panic!("rebuilt a tile"));
        assert_eq!(again, drawn);
        // The tiles stay within their room on the GPU, wherever the boat goes.
        for step in 0..200 {
            let w = World {
                seabed: &seabed,
                pose: Pose::level(3.3 + step as f32 * 0.7, -2.0),
                t: 0.0,
            };
            terrain.update(&w, |slot, _| assert!((slot as usize) < SLOTS));
            assert!(terrain.tiles.len() <= SLOTS);
        }
    }

    #[test]
    fn the_seabed_stays_within_its_bounds() {
        for visit in 0..4 {
            let seabed = Seabed::new(visit, visit as f32 * 0.29);
            for i in 0..20_000 {
                let (x, z) = (
                    (i % 200) as f32 * 1.7 - 170.0,
                    (i / 200) as f32 * 1.3 - 65.0,
                );
                let y = seabed.ground(x, z);
                assert!(LOWEST < y && y < HIGHEST, "{y} at ({x}, {z})");
            }
        }
    }

    #[test]
    fn a_tiles_indices_stay_within_it() {
        let indices = indices();
        assert_eq!(indices.len(), (QUADS * QUADS + 4 * QUADS) * 6);
        assert!(indices.iter().all(|&i| (i as usize) < VERTICES));
    }
}
