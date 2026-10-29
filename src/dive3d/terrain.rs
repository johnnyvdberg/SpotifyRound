//! The seabed, built on the CPU a tile at a time as it comes into view, and
//! forgotten once the boat has left it behind.
//!
//! Three levels of detail, each a square of tiles around the boat: fine
//! nearby, coarse out to the horizon. A level's square is made of whole tiles
//! of the next coarser level, which leaves exactly those out, so no two
//! levels overlap. A skirt hangs from every tile's edges and hides the cracks
//! where two levels meet. Only the tiles in view are drawn, nearest first.

use std::collections::HashMap;

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
    /// noise shows most on level ground; z: the sand ripples; w: the paving's
    /// joints.
    pub shade: [f32; 4],
    /// The lava cave's glow, at full strength, rgb.
    pub glow: [f32; 4],
}

/// The surface texture each biome's floor wears, as `surface_of` in
/// `scene.wesl`: sand, sand, silt, sand, basalt, silt, sand, silt.
const LAYERS: [usize; 8] = [12, 12, 13, 12, 10, 13, 12, 13];
/// Biome ids, as the shader's constants.
const FOREST: usize = 1;
const ATLANTIS: usize = 3;
const CAVE: usize = 4;
const ARCTIC: usize = 6;

/// `smoothstep` as the shader writes it.
fn smooth01(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// What the vertex shader used to work out per vertex, every frame: the
/// floor's colour and texture layer, how strongly its fine detail shows, and
/// the lava cave's glow. All of it depends only on where the vertex is.
fn shade(w: &World, x: f32, z: f32, normal: [f32; 3]) -> ([f32; 4], [f32; 4], [f32; 4]) {
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
    let base = world::floor_albedo(&b);
    // The shader's `(1.0 - steep)`: how level this piece of floor is.
    let flat = 1.0 - smooth01(0.12, 0.40, 1.0 - normal[1]);
    let noise = 0.65 + 0.5 * world::vnoise(x * 14.0, z * 14.0, 9);
    let sand = presence(0) + 0.5 * presence(FOREST) + 0.5 * presence(ARCTIC);
    let ripple = if sand > 0.0 {
        let r = ((x * 0.8 + z) * 26.0 + world::vnoise(x * 3.0, z * 3.0, 5) * 4.0).sin();
        0.16 * sand * smooth01(0.3, 1.0, r)
    } else {
        0.0
    };
    let paving = presence(ATLANTIS);
    let pave = if paving > 0.0 {
        let frac = |v: f32| v - v.floor();
        let row = (z * 4.0).floor();
        let joint = (frac(x * 3.0 + row * 0.5) - 0.5)
            .abs()
            .min((frac(z * 4.0) - 0.5).abs());
        0.45 * paving * smooth01(0.45, 0.5, joint)
    } else {
        0.0
    };
    let cave = presence(CAVE);
    let mut glow = [0.0; 4];
    if cave > 0.0 {
        let veins =
            1.0 - smooth01(0.0, 0.04, (world::vnoise(x * 1.4, z * 1.9, 6) - 0.5).abs());
        let fine = 1.0 - smooth01(0.0, 0.03, (world::vnoise(x * 4.0, z * 5.0, 7) - 0.5).abs());
        let pools = smooth01(0.66, 0.74, world::vnoise(x * 0.55 + 7.0, z * 0.55, 8));
        let k = (veins * 0.9 + fine * 0.5 + pools * 1.4) * cave * flat;
        glow = [k, 0.36 * k, 0.07 * k, 0.0];
    }
    (
        [base[0], base[1], base[2], LAYERS[b.a] as f32],
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
}

impl Default for Terrain {
    fn default() -> Self {
        Self {
            tiles: HashMap::new(),
            free: (0..SLOTS as u32).rev().collect(),
        }
    }
}

impl Terrain {
    /// Forget every tile: the seabed has changed.
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// The slots of the tiles to draw, nearest first. Tiles coming into view
    /// for the first time are built and handed to `upload` with their slot;
    /// those the boat has left behind are dropped.
    pub fn update(&mut self, w: &World, mut upload: impl FnMut(u32, &[Point])) -> Vec<u32> {
        let c = w.pose.pos;
        let tile = |level: usize| SPACING[level] * QUADS as f32;
        // The boat's tile at each level.
        let at = |level: usize| {
            [
                (c[0] / tile(level)).floor() as i32,
                (c[2] / tile(level)).floor() as i32,
            ]
        };
        let top = SPACING.len() - 1;
        // Each level's square, in its own tiles, as [first, last] per axis.
        let square = |level: usize| -> [[i32; 2]; 2] {
            if level == top {
                at(top).map(|b| [b - REACH, b + REACH])
            } else {
                at(level + 1).map(|b| [(b - NEST) * RATIO, (b + NEST + 1) * RATIO - 1])
            }
        };
        let squares: Vec<_> = (0..SPACING.len()).map(square).collect();
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
                        let rise = (HIGHEST - LOWEST) * 0.5 + SKIRT * SPACING[level];
                        if !w.visible(middle, (size * size * 0.5 + rise * rise).sqrt()) {
                            continue;
                        }
                        let Some(slot) = self.free.pop() else {
                            continue;
                        };
                        let (points, centre, radius) = build(w, level, i, j);
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
fn build(w: &World, level: usize, i: i32, j: i32) -> (Vec<Point>, [f32; 3], f32) {
    let s = SPACING[level];
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
            let (albedo, shade, glow) = shade(w, x, z, normal);
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

#[cfg(all(test, feature = "full-hd"))]
#[cfg(all(test, feature = "full-hd"))]
#[cfg(all(test, feature = "full-hd"))]
/// One tile's vertices in its own block, for a geometry clipmap: `x` and `z`
/// in tiles of the level's spacing, and a flag for the skirt vertices (which
/// the vertex shader lowers by `SKIRT`). The grid is the same lattice and in
/// the same order as [`build`] writes, so [`indices`] fits.
pub fn block_mesh() -> Vec<[f32; 3]> {
    let mut out = Vec::with_capacity(VERTICES);
    for b in 0..SIDE {
        for a in 0..SIDE {
            out.push([a as f32, b as f32, 0.0]);
        }
    }
    for edge in edges() {
        for k in 0..SIDE {
            let i = edge[k];
            out.push([(i % SIDE) as f32, (i / SIDE) as f32, 1.0]);
        }
    }
    out
}

/// A block's world corner and vertex spacing, for the clipmap's instances.
#[cfg(all(test, feature = "full-hd"))]
pub fn block_origin(level: usize, i: i32, j: i32) -> ([f32; 2], f32) {
    let s = SPACING[level];
    let size = s * QUADS as f32;
    ([i as f32 * size, j as f32 * size], s)
}

/// The tiles a clipmap would draw this frame: level and tile index, nearest
/// first. The same squares, holes and culling as [`Terrain::update`], without
/// building any geometry.
#[cfg(all(test, feature = "full-hd"))]
pub fn visible_blocks(w: &World) -> Vec<(usize, i32, i32)> {
    let c = w.pose.pos;
    let tile = |level: usize| SPACING[level] * QUADS as f32;
    let at = |level: usize| {
        [
            (c[0] / tile(level)).floor() as i32,
            (c[2] / tile(level)).floor() as i32,
        ]
    };
    let top = SPACING.len() - 1;
    let square = |level: usize| -> [[i32; 2]; 2] {
        if level == top {
            at(top).map(|b| [b - REACH, b + REACH])
        } else {
            at(level + 1).map(|b| [(b - NEST) * RATIO, (b + NEST + 1) * RATIO - 1])
        }
    };
    let squares: Vec<_> = (0..SPACING.len()).map(square).collect();
    let wanted = |level: usize, i: i32, j: i32| {
        let [x, z] = squares[level];
        let inside = x[0] <= i && i <= x[1] && z[0] <= j && j <= z[1];
        let finer = level > 0 && {
            let b = at(level);
            (i - b[0]).abs() <= NEST && (j - b[1]).abs() <= NEST
        };
        inside && !finer
    };
    let mut out = Vec::new();
    for level in 0..SPACING.len() {
        let size = tile(level);
        let [x, z] = squares[level];
        for j in z[0]..=z[1] {
            for i in x[0]..=x[1] {
                if !wanted(level, i, j) {
                    continue;
                }
                // The block's own height range, not the seabed's: the same
                // cull the tile builder gets from its vertices, from nine
                // samples. The four corners include the skirt, which hangs
                // `SKIRT` spacings below the ground.
                let mut low = f32::MAX;
                let mut high = f32::MIN;
                for b in 0..3 {
                    for a in 0..3 {
                        let x = (i as f32 + a as f32 * 0.5) * size;
                        let z = (j as f32 + b as f32 * 0.5) * size;
                        let h = w.seabed.ground(x, z);
                        low = low.min(h);
                        high = high.max(h);
                    }
                }
                for (a, b) in [(0, 0), (2, 0), (0, 2), (2, 2)] {
                    let x = (i as f32 + a as f32 * 0.5) * size;
                    let z = (j as f32 + b as f32 * 0.5) * size;
                    low = low.min(w.seabed.ground(x, z) - SKIRT * SPACING[level]);
                }
                let middle = [
                    (i as f32 + 0.5) * size,
                    (low + high) * 0.5,
                    (j as f32 + 0.5) * size,
                ];
                let rise = (high - low) * 0.5;
                let radius = (size * size * 0.5 + rise * rise).sqrt();
                if !w.visible(middle, radius) {
                    continue;
                }
                let d = [middle[0] - c[0], middle[1] - c[1], middle[2] - c[2]];
                let reach = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - radius;
                out.push((reach, level, i, j));
            }
        }
    }
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out.into_iter().map(|(_, l, i, j)| (l, i, j)).collect()
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
