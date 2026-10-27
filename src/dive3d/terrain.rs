//! The seabed, built on the CPU a tile at a time as it comes into view, and
//! forgotten once the boat has left it behind.
//!
//! Three levels of detail, each a square of tiles around the boat: fine
//! nearby, coarse out to the horizon. A level's square is made of whole tiles
//! of the next coarser level, which leaves exactly those out, so no two
//! levels overlap. A skirt hangs from every tile's edges and hides the cracks
//! where two levels meet. Only the tiles in view are drawn, nearest first.

use std::collections::HashMap;

use super::world::World;

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

/// A vertex of the seabed, in the layout `vs_terrain` reads.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Point {
    pub position: [f32; 3],
    pub normal: [f32; 3],
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
            points.push(Point {
                position: [
                    (x0 + a as i32) as f32 * s,
                    h(a, b),
                    (z0 + b as i32) as f32 * s,
                ],
                normal: [n[0] / l, n[1] / l, n[2] / l],
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
