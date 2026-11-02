//! The seabed's relief: a random heightmap, made afresh for every visit.
//!
//! One tile, [`SIZE`] texels square and [`PERIOD`] world units across, wraps
//! in both directions. It mixes domain-warped rolling ground, sharp ridges and
//! the occasional canyon cut through them. `world.rs` builds the seabed and
//! places things on it by the same bilinear lookup of the same numbers, so
//! everything stands on the seabed.
//!
//! Each texel holds the height, in -1..1, its slope along x and z, so the
//! seabed's normals interpolate smoothly instead of showing the texel grid, and
//! one more field of fine grain that each biome folds into its own floor: bank
//! tops in the shallows, root mounds in the forest, the abyss's cut rock.
//!
//! A second tile of the same size holds the forms some seas shape their floor
//! from ([`Form`]): dunes, ridged rock and a Voronoi mosaic of cells.
//!
//! The noise is FastNoiseLite's (`fastnoise-lite`, MIT). It does not repeat,
//! so the tile is made to: x goes round a cylinder in its 3D noise, which
//! closes on itself exactly every [`PERIOD`] units, and z, which the voyage
//! only ever sees a few units either side of its corridor, runs from
//! -`PERIOD`/2 to `PERIOD`/2 and is cross-faded into its own far side over
//! the last [`BAND`] units, out where nothing is in sight.

use std::f32::consts::TAU;

use fastnoise_lite::{
    CellularDistanceFunction, CellularReturnType, DomainWarpType, FastNoiseLite, FractalType,
    NoiseType,
};

pub const SIZE: usize = 512;
/// World units the tile spans before it repeats: twenty minutes of voyage.
pub const PERIOD: f32 = 64.0;
const TEXEL: f32 = PERIOD / SIZE as f32;
/// The forms tile's side: half the heightmap's, a texel every eighth of a
/// unit being finer than any form needs, and a quarter of the time to make.
pub const FORMS: usize = SIZE / 2;
/// How far from its seam, at z = ±`PERIOD`/2, the tile fades into its own far
/// side: the corridor never strays more than three units from z = 0 and the
/// seas' sight is at most sixteen, so the fade lies beyond anything drawn.
const BAND: f32 = 4.0;
/// The radius of the cylinder x goes round, so a unit of x is a unit of noise.
const RADIUS: f32 = PERIOD / TAU;

/// The channels of the forms tile ([`generate`]'s second).
pub struct Form;

impl Form {
    /// Sand dunes, 0 in the troughs to 1 on a crest: crests every 3.6 units
    /// across x + z/2, wandering where the domain is warped, each with a
    /// long stoss side and a short steep lee, in fields with flat sand
    /// between.
    pub const DUNES: usize = 0;
    /// Ridged noise, -1..1, 1 along the ridges: folded rock.
    pub const RIDGED: usize = 1;
    /// How far inside its cell of the Voronoi mosaic, about three units
    /// across: 0 on the cell's edge, 0.2 to 0.7 at its heart.
    pub const EDGE: usize = 2;
    /// The cell's own value, -1..1, the same all over a cell but for a
    /// narrow riser at its edge.
    pub const CELL: usize = 3;
}

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn noise(seed: i32, kind: NoiseType, frequency: f32) -> FastNoiseLite {
    let mut n = FastNoiseLite::with_seed(seed);
    n.set_noise_type(Some(kind));
    n.set_frequency(Some(frequency));
    n
}

fn fractal(seed: i32, kind: FractalType, frequency: f32, octaves: i32) -> FastNoiseLite {
    let mut n = noise(seed, NoiseType::OpenSimplex2, frequency);
    n.set_fractal_type(Some(kind));
    n.set_fractal_octaves(Some(octaves));
    n
}

fn warp(seed: i32, frequency: f32, octaves: i32, amp: f32) -> FastNoiseLite {
    let mut n = fractal(seed, FractalType::DomainWarpProgressive, frequency, octaves);
    n.set_domain_warp_type(Some(DomainWarpType::OpenSimplex2));
    n.set_domain_warp_amp(Some(amp));
    n
}

fn cellular(seed: i32, frequency: f32, kind: CellularReturnType) -> FastNoiseLite {
    let mut n = noise(seed, NoiseType::Cellular, frequency);
    n.set_cellular_distance_function(Some(CellularDistanceFunction::Euclidean));
    n.set_cellular_return_type(Some(kind));
    n.set_cellular_jitter(Some(0.9));
    n
}

/// World (x, z) as a point in the noise's 3D space, x round the cylinder.
fn cylinder(x: f32, z: f32) -> [f32; 3] {
    let a = x / RADIUS;
    [RADIUS * a.cos(), RADIUS * a.sin(), z]
}

/// The domain warp at (x, z), as a shift along x and along z: the warp moves
/// the point in 3D, and only its part along the cylinder's surface counts.
fn shift(warp: &FastNoiseLite, x: f32, z: f32) -> (f32, f32, [f32; 3]) {
    let p = cylinder(x, z);
    let (wx, wy, wz) = warp.domain_warp_3d(p[0], p[1], p[2]);
    let a = x / RADIUS;
    let along = -(wx - p[0]) * a.sin() + (wy - p[1]) * a.cos();
    (along, wz - z, [wx, wy, wz])
}

/// Every texel of a tile of `field`, `size` texels square, made seamless
/// (see the module's notes), worked out across the CPU's cores.
fn bake<const N: usize>(size: usize, field: impl Fn(f32, f32) -> [f32; N] + Sync) -> Vec<[f32; N]> {
    let mut out = vec![[0.0; N]; size * size];
    let texel = PERIOD / size as f32;
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get());
    let rows = size.div_ceil(threads);
    std::thread::scope(|scope| {
        for (k, chunk) in out.chunks_mut(rows * size).enumerate() {
            let field = &field;
            scope.spawn(move || {
                for (n, texel_out) in chunk.iter_mut().enumerate() {
                    let (i, j) = (n % size, k * rows + n / size);
                    let x = i as f32 * texel;
                    let mut z = j as f32 * texel;
                    if z >= PERIOD / 2.0 {
                        z -= PERIOD;
                    }
                    let here = field(x, z);
                    let edge = PERIOD / 2.0 - z.abs();
                    *texel_out = if edge < BAND {
                        // Half and half at the seam, from both sides of it.
                        let s = 0.5 * smooth(BAND, 0.0, edge);
                        let there = field(x, z - PERIOD * z.signum());
                        std::array::from_fn(|c| here[c] + (there[c] - here[c]) * s)
                    } else {
                        here
                    };
                }
            });
        }
    });
    out
}

/// A tile's value at texel (i, j), wrapping.
fn at<const N: usize>(map: &[[f32; N]], i: usize, j: usize, di: i32, dj: i32) -> [f32; N] {
    let x = (i as i32 + di).rem_euclid(SIZE as i32) as usize;
    let y = (j as i32 + dj).rem_euclid(SIZE as i32) as usize;
    map[y * SIZE + x]
}

/// The heightmap and forms for `seed`, as RGBA texels. The heightmap holds
/// height, slope along x, slope along z (both per world unit), and the
/// floor's fine grain in -1..1; the forms, the channels of [`Form`].
pub fn generate(seed: u64) -> (Vec<[f32; 4]>, Vec<[f32; 4]>) {
    let salt = (seed % 1_000_003) as i32 * 16;
    // Warp the domain, so the ground wanders instead of lying in a grid.
    let wander = warp(salt + 1, 1.0 / 20.0, 3, 2.5);
    let mut rolling = fractal(salt + 2, FractalType::FBm, 4.0 / PERIOD, 4);
    // Less of the finer octaves than FastNoiseLite's default half: silt
    // has smoothed the small hummocks away.
    rolling.set_fractal_gain(Some(0.4));
    // Ridges: OpenSimplex2 folded on itself, octave on octave.
    let mut ridges = fractal(salt + 3, FractalType::Ridged, 4.0 / PERIOD, 4);
    ridges.set_fractal_gain(Some(0.4));
    // A canyon now and then, where a slow noise crosses its middle.
    let course = fractal(salt + 4, FractalType::FBm, 1.3 / PERIOD, 3);
    // Fine grain, at one to four units a feature: fine enough to read as silt
    // and pitted rock underfoot, coarse enough that every terrain level
    // still samples it rather than aliasing it.
    let grain = fractal(salt + 5, FractalType::FBm, 16.0 / PERIOD, 3);
    let raw = bake(SIZE, |x, z| {
        let (_, _, w) = shift(&wander, x, z);
        let roll = rolling.get_noise_3d(w[0], w[1], w[2]);
        let ridge = 0.5 + 0.5 * ridges.get_noise_3d(w[0], w[1], w[2]);
        let course = course.get_noise_3d(w[0], w[1], w[2]);
        let canyon = 1.0 - (course.abs() / CANYON).min(1.0);
        let h = (roll * ROLL + ridge * ridge * ridge * 0.8 - canyon * canyon * 0.9) * SCALE + LIFT;
        let p = cylinder(x, z);
        [h, grain.get_noise_3d(p[0], p[1], p[2]) * GRAIN]
    });
    // Soften the finest grain a touch, as silt would.
    let mut smoothed = vec![0.0f32; SIZE * SIZE];
    for j in 0..SIZE {
        for i in 0..SIZE {
            let h = |di, dj| at(&raw, i, j, di, dj)[0];
            smoothed[j * SIZE + i] =
                (h(0, 0) * 4.0 + h(1, 0) + h(-1, 0) + h(0, 1) + h(0, -1)) / 8.0;
        }
    }
    let mut map = vec![[0.0f32; 4]; SIZE * SIZE];
    for j in 0..SIZE {
        for i in 0..SIZE {
            let h = |di: i32, dj: i32| {
                let x = (i as i32 + di).rem_euclid(SIZE as i32) as usize;
                let y = (j as i32 + dj).rem_euclid(SIZE as i32) as usize;
                smoothed[y * SIZE + x]
            };
            let gx = (h(1, 0) - h(-1, 0)) / (2.0 * TEXEL);
            let gz = (h(0, 1) - h(0, -1)) / (2.0 * TEXEL);
            let grain = raw[j * SIZE + i][1].clamp(-0.999, 0.999);
            map[j * SIZE + i] = [h(0, 0).clamp(-1.0, 1.0), gx, gz, grain];
        }
    }
    (map, forms(salt))
}

/// Weights that bring FastNoiseLite's fields to the spread of the value
/// noise the seabed was first tuned on, so every sea's `RELIEF` and shapes
/// keep their measure.
const ROLL: f32 = 0.7;
const LIFT: f32 = 0.11;
const SCALE: f32 = 1.18;
const CANYON: f32 = 0.09;
const GRAIN: f32 = 1.0;

/// The forms tile ([`Form`]).
fn forms(salt: i32) -> Vec<[f32; 4]> {
    // Dunes: crests along a warped line, so they bend, fork and die out
    // as wind-blown sand does, in fields that come and go.
    let blown = warp(salt + 8, 1.0 / 12.0, 2, 1.6);
    let fields = noise(salt + 9, NoiseType::OpenSimplex2, 1.0 / 10.0);
    let rock = fractal(salt + 10, FractalType::Ridged, 0.16, 4);
    let edges = cellular(salt + 11, 0.3, CellularReturnType::Distance2Sub);
    let cells = cellular(salt + 11, 0.3, CellularReturnType::CellValue);
    let mut tile = bake(FORMS, |x, z| {
        let (along, across, _) = shift(&blown, x, z);
        // Sixteen crests round the tile along x, so they meet at its seam.
        let phase = (x + along) * 0.25 + (z + across) * 0.125;
        let t = phase - phase.floor();
        let dune = if t < 0.72 {
            smooth(0.0, 0.72, t)
        } else {
            smooth(1.0, 0.72, t)
        };
        let p = cylinder(x, z);
        let field = smooth(-0.35, 0.45, fields.get_noise_3d(p[0], p[1], p[2]));
        [
            dune * field,
            rock.get_noise_3d(p[0], p[1], p[2]),
            1.0 + edges.get_noise_3d(p[0], p[1], p[2]),
            cells.get_noise_3d(p[0], p[1], p[2]),
        ]
    });
    // Round the cells' risers off over about half a unit, so a terrace's
    // step is a short slope and not a texel's cliff.
    let n = FORMS as i32;
    {
        let before: Vec<f32> = tile.iter().map(|t| t[Form::CELL]).collect();
        for j in 0..FORMS {
            for i in 0..FORMS {
                let c = |di: i32, dj: i32| {
                    let x = (i as i32 + di).rem_euclid(n) as usize;
                    let y = (j as i32 + dj).rem_euclid(n) as usize;
                    before[y * FORMS + x]
                };
                tile[j * FORMS + i][Form::CELL] =
                    (c(0, 0) * 4.0 + c(1, 0) + c(-1, 0) + c(0, 1) + c(0, -1)) / 8.0;
            }
        }
    }
    tile
}

/// Height, slopes and grain at world (x, z), bilinear; or the forms, from
/// the forms tile (either tile, whatever its size).
pub fn sample(map: &[[f32; 4]], x: f32, z: f32) -> [f32; 4] {
    let size = map.len().isqrt();
    let u = x / PERIOD * size as f32;
    let v = z / PERIOD * size as f32;
    let (fu, fv) = (u.floor(), v.floor());
    let (tu, tv) = (u - fu, v - fv);
    let n = size as i32;
    let at = |i: i32, j: i32| map[(j.rem_euclid(n) * n + i.rem_euclid(n)) as usize];
    let (i, j) = (fu as i32, fv as i32);
    let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
    let mut out = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * tu;
        let bottom = c[k] + (d[k] - c[k]) * tu;
        out[k] = top + (bottom - top) * tv;
    }
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_seabed_has_relief_and_wraps() {
        let (map, forms) = generate(42);
        let heights: Vec<f32> = map.iter().map(|t| t[0]).collect();
        let lowest = heights.iter().cloned().fold(f32::MAX, f32::min);
        let highest = heights.iter().cloned().fold(f32::MIN, f32::max);
        assert!(highest - lowest > 0.8, "{lowest}..{highest} is too flat");
        // The tile repeats seamlessly, grain and all.
        let a = sample(&map, 3.3, 7.7);
        let b = sample(&map, 3.3 + PERIOD, 7.7 - PERIOD);
        for k in 0..4 {
            assert!((a[k] - b[k]).abs() < 1e-5);
        }
        // The grain varies over every texel, in -1..1.
        let grains: Vec<f32> = map.iter().map(|t| t[3]).collect();
        let low = grains.iter().cloned().fold(f32::MAX, f32::min);
        let high = grains.iter().cloned().fold(f32::MIN, f32::max);
        assert!(low > -1.0 && high < 1.0, "grain {low}..{high} out of range");
        assert!(high - low > 0.6, "grain {low}..{high} is too flat");
        // The forms repeat too, and every channel of them varies.
        let a = sample(&forms, 3.3, 7.7);
        let b = sample(&forms, 3.3 - PERIOD, 7.7 + PERIOD);
        for k in 0..4 {
            assert!((a[k] - b[k]).abs() < 1e-5);
            let (low, high) = forms
                .iter()
                .fold((f32::MAX, f32::MIN), |(l, h), t| (l.min(t[k]), h.max(t[k])));
            assert!(high - low > 0.5, "form {k} {low}..{high} is too flat");
        }
        // No step at the tile's seams: across them, neighbouring texels differ
        // no more than across as many lines of texels inside the tile.
        let step = |i: usize, j: usize, across: bool| {
            let next = if across {
                j * SIZE + (i + 1) % SIZE
            } else {
                (j + 1) % SIZE * SIZE + i
            };
            (map[j * SIZE + i][0] - map[next][0]).abs()
        };
        let most = |lines: [usize; 3]| {
            let mut most = 0.0f32;
            for line in lines {
                for k in 0..SIZE {
                    most = most.max(step(line, k, true)).max(step(k, line, false));
                }
            }
            most
        };
        let seams = most([SIZE - 1, SIZE / 2 - 1, SIZE / 2]);
        let inside = most([100, 200, 400]);
        assert!(seams < 1.5 * inside, "seam step {seams}, inside {inside}");
        // The same visit, the same seabed; another visit, another.
        let (again, _) = generate(42);
        assert_eq!(again[1000], map[1000]);
        let (other, _) = generate(43);
        assert_ne!(other[1000][0], map[1000][0]);
        assert_ne!(other[1000][3], map[1000][3]);
    }
}
