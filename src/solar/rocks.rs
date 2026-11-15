//! The asteroid field's rocks: their shapes, meshes and surface maps,
//! built once on the CPU (and cached with the other models).
//!
//! A rock is a radial height field over the sphere: its radius along every
//! direction from its middle. That holds for nearly every small body that
//! has been mapped, and it makes the rest cheap:
//!
//! - The shape is a Gaussian random sphere (Muinonen 1998, "Introducing
//!   the Gaussian shape hypothesis for asteroids and comets"), the
//!   statistical model asteroid shapes are fitted with: the log of the
//!   radius a sum of spherical harmonics, each degree's weights drawn
//!   from a normal distribution whose variance falls as a power of the
//!   degree. A few shapes are pinched at the waist into contact binaries,
//!   as Itokawa and Arrokoth are.
//! - Craters are cut into it at four scales, and boulders set on it at two.
//!   Each scale has fewer, larger features than the next, as cratered
//!   surfaces have (their counts per area falling roughly as the square of
//!   their size). Fresh crater rims are paler, being less weathered.
//! - Everything is kept in octahedral maps over the directions, a layer
//!   per shape (`Maps`). The surface map holds the normal (in the frame of
//!   the radial direction), the ground's shade and its occlusion. The
//!   horizon map holds how high the ground rises toward four directions
//!   round each point (Max 1988, "Horizon mapping: shadows for bump-mapped
//!   surfaces"), so the rocks' shader casts the shadows of craters, boulders
//!   and lumps with a single fetch.
//! - The meshes are only the silhouette: an octahedral grid over the
//!   directions, each vertex moved out to the shape's radius there. That
//!   takes a few hundred to eight thousand triangles, where the surface
//!   nets took tens of thousands.

use glam::{Vec2, Vec3};

use super::models::{Mesh, ROCKS, ShipVertex, hash, noise3};
use super::system::Rng;

/// Texels along each side of a shape's maps (`ROCK_MAP` in
/// `encounter.wgsl`), and their mip levels.
pub const MAP: usize = 512;
pub const MIPS: u32 = 10;
/// Grid cells along each side of the octahedral square at each level of
/// detail: 2n² triangles.
const GRID: [usize; 3] = [64, 28, 12];
/// The highest degree of the spherical harmonics in the shape.
const DEGREE: usize = 14;

/// Where unit `d` lies on an octahedral map, 0..1 each way (`octahedral`
/// in `encounter.wgsl`).
pub fn octahedral(d: Vec3) -> Vec2 {
    let p = d / (d.x.abs() + d.y.abs() + d.z.abs());
    let mut xy = Vec2::new(p.x, p.y);
    if p.z < 0.0 {
        let s = Vec2::new(
            if p.x >= 0.0 { 1.0 } else { -1.0 },
            if p.y >= 0.0 { 1.0 } else { -1.0 },
        );
        xy = (Vec2::ONE - Vec2::new(p.y.abs(), p.x.abs())) * s;
    }
    xy * 0.5 + Vec2::splat(0.5)
}

/// The direction at `uv` on an octahedral map (`octahedral_dir`).
pub fn octahedral_dir(uv: Vec2) -> Vec3 {
    let f = uv * 2.0 - Vec2::ONE;
    let mut n = Vec3::new(f.x, f.y, 1.0 - f.x.abs() - f.y.abs());
    let t = (-n.z).max(0.0);
    n.x += if n.x >= 0.0 { -t } else { t };
    n.y += if n.y >= 0.0 { -t } else { t };
    n.normalize()
}

/// The frame round the radial direction `d` that the maps' normals and
/// horizons are kept in (`rock_frame` in `encounter.wgsl`): across the
/// z axis and along it. Only at the poles of z is it undefined, a texel or
/// two across.
pub fn frame(d: Vec3) -> (Vec3, Vec3) {
    let t1 = Vec3::new(-d.y, d.x, 0.0);
    let t1 = if t1.length_squared() < 1e-12 {
        Vec3::X
    } else {
        t1.normalize()
    };
    (t1, d.cross(t1))
}

/// A square map over the directions, bilinearly read.
struct Field {
    size: usize,
    values: Vec<f32>,
}

impl Field {
    fn build(size: usize, f: impl Fn(Vec3) -> f32 + Sync) -> Self {
        let mut values = vec![0.0; size * size];
        let threads = std::thread::available_parallelism()
            .map_or(2, |n| n.get())
            .clamp(1, 8);
        let rows = size.div_ceil(threads);
        std::thread::scope(|scope| {
            for (t, chunk) in values.chunks_mut(rows * size).enumerate() {
                let f = &f;
                scope.spawn(move || {
                    for (i, v) in chunk.iter_mut().enumerate() {
                        let (x, y) = (i % size, t * rows + i / size);
                        let uv = (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / size as f32;
                        *v = f(octahedral_dir(uv));
                    }
                });
            }
        });
        Field { size, values }
    }

    fn at(&self, d: Vec3) -> f32 {
        let n = self.size as f32;
        let p = octahedral(d) * n - Vec2::splat(0.5);
        let p = p.clamp(Vec2::ZERO, Vec2::splat(n - 1.0));
        let i = p.floor();
        let f = p - i;
        let (x0, y0) = (i.x as usize, i.y as usize);
        let (x1, y1) = ((x0 + 1).min(self.size - 1), (y0 + 1).min(self.size - 1));
        let v = |x: usize, y: usize| self.values[y * self.size + x];
        let a = v(x0, y0) + (v(x1, y0) - v(x0, y0)) * f.x;
        let b = v(x0, y1) + (v(x1, y1) - v(x0, y1)) * f.x;
        a + (b - a) * f.y
    }
}

/// The real spherical harmonics up to `DEGREE` along unit `d`, degree by
/// degree, orthonormal over the sphere.
// The Legendre recurrences read as written with their indices.
#[allow(clippy::needless_range_loop)]
fn harmonics(d: Vec3) -> Vec<f64> {
    let n = DEGREE + 1;
    let x = (d.z as f64).clamp(-1.0, 1.0);
    let s = (1.0 - x * x).max(0.0).sqrt();
    let phi = (d.y as f64).atan2(d.x as f64);
    // Associated Legendre functions P[l][m], by the usual recurrences.
    let mut p = vec![vec![0.0f64; n]; n];
    p[0][0] = 1.0;
    for m in 1..n {
        p[m][m] = -(2.0 * m as f64 - 1.0) * s * p[m - 1][m - 1];
    }
    for m in 0..n - 1 {
        p[m + 1][m] = x * (2.0 * m as f64 + 1.0) * p[m][m];
    }
    for m in 0..n {
        for l in m + 2..n {
            p[l][m] = ((2.0 * l as f64 - 1.0) * x * p[l - 1][m] - (l + m - 1) as f64 * p[l - 2][m])
                / (l - m) as f64;
        }
    }
    let factorial = |k: usize| (1..=k).map(|i| i as f64).product::<f64>();
    let mut out = Vec::with_capacity(n * n);
    for l in 0..n {
        for m in -(l as i64)..=l as i64 {
            let a = m.unsigned_abs() as usize;
            let k = ((2 * l + 1) as f64 / (4.0 * std::f64::consts::PI) * factorial(l - a)
                / factorial(l + a))
            .sqrt();
            out.push(match m.cmp(&0) {
                std::cmp::Ordering::Equal => k * p[l][0],
                std::cmp::Ordering::Greater => {
                    std::f64::consts::SQRT_2 * k * (m as f64 * phi).cos() * p[l][a]
                }
                std::cmp::Ordering::Less => {
                    std::f64::consts::SQRT_2 * k * (a as f64 * phi).sin() * p[l][a]
                }
            });
        }
    }
    out
}

/// One rock shape's parameters, drawn from its seed.
struct Shape {
    /// The log radius's harmonic weights.
    weights: Vec<f64>,
    /// Its stretch along x, y and z, and how far it is pinched at the
    /// waist across x (contact binaries).
    stretch: Vec3,
    waist: f32,
    /// Planes it was broken along (a fragment's facets): each a normal and
    /// how far out, relative to its radius there.
    facets: Vec<(Vec3, f32)>,
    /// How thickly cratered and how strewn with boulders, 0..1.
    craters: f32,
    boulders: f32,
    seed: f32,
}

impl Shape {
    fn new(variant: u32) -> Self {
        let mut rng = Rng::new(0x9a57_0000 ^ (variant as u64 * 0x9e37));
        let mut unit = || rng.unit() as f32;
        // Muinonen & Lagerros's fits to asteroids: a log radius's standard
        // deviation of some 0.2 to 0.35, its covariance falling as the
        // degree to the power 3 or 4. Degrees 0 and 1 (size and offset) are
        // left out.
        let beta = 0.2 + 0.15 * unit();
        let nu = 2.6 + unit();
        let total: f32 = (2..=DEGREE).map(|l| (l as f32).powf(-nu)).sum();
        let mut weights = Vec::new();
        for l in 0..=DEGREE {
            let c = if l < 2 {
                0.0
            } else {
                (l as f32).powf(-nu) / total
            };
            let variance = beta * beta * c * 4.0 * std::f32::consts::PI / (2 * l + 1) as f32;
            for _ in 0..2 * l + 1 {
                // Normal, by Box-Muller.
                let g =
                    (-2.0 * (1.0 - unit()).ln()).sqrt() * (std::f32::consts::TAU * unit()).cos();
                weights.push((g * variance.sqrt()) as f64);
            }
        }
        let binary = variant % 4 == 2;
        // Every other shape a fragment, broken along a few planes.
        let facets = if variant % 2 == 1 {
            (0..3 + (unit() * 4.0) as usize)
                .map(|_| {
                    let z = unit() * 2.0 - 1.0;
                    let a = unit() * std::f32::consts::TAU;
                    let r = (1.0 - z * z).sqrt();
                    (Vec3::new(r * a.cos(), r * a.sin(), z), 0.72 + 0.2 * unit())
                })
                .collect()
        } else {
            Vec::new()
        };
        let long = if binary { 1.0 } else { 0.85 + 0.15 * unit() };
        Shape {
            weights,
            stretch: Vec3::new(long, 0.6 + 0.25 * unit(), 0.5 + 0.25 * unit()),
            waist: if binary { 0.28 + 0.12 * unit() } else { 0.0 },
            facets,
            craters: 0.35 + 0.65 * unit(),
            boulders: unit(),
            seed: unit() * 40.0,
        }
    }

    /// Its radius along unit `d` before it is scaled to fit: the random
    /// sphere, stretched, pinched, its big craters cut.
    fn body(&self, d: Vec3) -> f32 {
        let y = harmonics(d);
        let s: f64 = y.iter().zip(&self.weights).map(|(a, b)| a * b).sum();
        let ellipsoid = 1.0 / (d / self.stretch).length();
        let pinch = 1.0 - self.waist * (-(d.x / 0.3).powi(2)).exp();
        let (crater, _) = craters(d * 2.2, self.seed, 0.6 * self.craters);
        let mut r = ellipsoid * pinch * (s as f32).exp();
        // Each facet cuts it where the plane passes inside, the cut's edge
        // rounded a little.
        for &(m, out) in &self.facets {
            let along = d.dot(m);
            if along > 0.05 {
                let plane = out * ellipsoid / along;
                let k = 0.04;
                let h = (0.5 + 0.5 * (plane - r) / k).clamp(0.0, 1.0);
                r = plane + (r - plane) * h - k * h * (1.0 - h);
            }
        }
        r * (1.0 + crater / 2.2)
    }

    /// What its surface adds to `body` along `d`, relative: the smaller
    /// craters, the boulders and the grain; and how fresh the ground is
    /// there (paler crater rims).
    fn surface(&self, d: Vec3) -> (f32, f32) {
        let mut height = 0.0;
        let mut fresh: f32 = 0.0;
        // Heights are in cells: over the scale, relative to the radius.
        for (scale, cover) in [(5.5, 0.65), (13.0, 0.7), (30.0, 0.72)] {
            let (h, f) = craters(
                d * scale + Vec3::splat(scale),
                self.seed + scale,
                cover * self.craters,
            );
            height += h / scale;
            fresh = fresh.max(f);
        }
        for scale in [16.0, 38.0] {
            height += boulders(
                d * scale + Vec3::splat(7.0),
                self.seed + scale,
                self.boulders,
            ) / scale;
        }
        let grain = noise3(d * 60.0 + Vec3::splat(self.seed)) * 0.004
            + noise3(d * 130.0 + Vec3::splat(self.seed + 3.0)) * 0.002;
        (height + grain, fresh)
    }
}

/// A crater's profile across `x`, its distance from the middle over its
/// radius: a bowl, a raised rim, the ground beyond.
fn crater_profile(x: f32) -> f32 {
    let s = ((x - 0.85) / 0.2).clamp(0.0, 1.0);
    let s = s * s * (3.0 - 2.0 * s);
    let fade = ((x - 1.3) / 0.5).clamp(0.0, 1.0);
    let fade = fade * fade * (3.0 - 2.0 * fade);
    (x * x - 1.0) * 0.35 * (1.0 - s) + 0.14 * (-((x - 1.0) / 0.32).powi(2)).exp() * (1.0 - fade)
}

/// Craters round `q` (a direction scaled up, a crater at most a cell) from
/// the eight cells about it, as many as `cover` (0..1) has: the ground's
/// height from them, in cells, and how fresh a rim it is on (0..1). Each
/// is softened by its age: the old ones shallower, their rims worn down.
fn craters(q: Vec3, seed: f32, cover: f32) -> (f32, f32) {
    let base = (q - Vec3::splat(0.5)).floor();
    let mut height = 0.0;
    let mut fresh: f32 = 0.0;
    for i in 0..8 {
        let cell = base + Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32);
        let k = hash(cell, seed);
        if k > cover {
            continue;
        }
        let jitter = Vec3::new(
            hash(cell, seed + 1.0),
            hash(cell, seed + 2.0),
            hash(cell, seed + 3.0),
        );
        let centre = cell + Vec3::splat(0.5) + (jitter - Vec3::splat(0.5)) * 0.5;
        // Sizes skewed small: many little ones for every big one.
        let radius = 0.12 + 0.32 * (k / cover.max(1e-3)).powi(2);
        let x = (q - centre).length() / radius;
        if x > 1.8 {
            continue;
        }
        let age = jitter.x;
        let worn = 0.45 + 0.55 * (1.0 - age);
        height += crater_profile(x) * radius * worn;
        if age < 0.3 {
            let rim = (1.0 - ((x - 1.0) / 0.3).abs()).max(0.0);
            let floor = if x < 0.9 { 0.4 } else { 0.0 };
            fresh = fresh.max(rim.max(floor) * (1.0 - age / 0.3));
        }
    }
    (height, fresh)
}

/// Boulders round `q` from the eight cells about it, as many as `cover`
/// (0..1) has: the ground's height from them, in cells. Each is a rounded
/// block, squarer than a dome.
fn boulders(q: Vec3, seed: f32, cover: f32) -> f32 {
    let base = (q - Vec3::splat(0.5)).floor();
    let mut height: f32 = 0.0;
    for i in 0..8 {
        let cell = base + Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32);
        let k = hash(cell, seed + 11.0);
        if k > 0.25 * cover {
            continue;
        }
        let jitter = Vec3::new(
            hash(cell, seed + 12.0),
            hash(cell, seed + 13.0),
            hash(cell, seed + 14.0),
        );
        let centre = cell + Vec3::splat(0.5) + (jitter - Vec3::splat(0.5)) * 0.6;
        let radius = 0.15 + 0.3 * jitter.y;
        let v = (q - centre) / radius;
        // Between round and square.
        let x = v.length() * 0.6 + v.abs().max_element() * 0.4;
        if x < 1.0 {
            height = height.max((1.0 - x.powi(4)).sqrt() * radius * (0.5 + 0.5 * jitter.z));
        }
    }
    height
}

/// Every rock shape's surface and horizon maps, mip by mip, a shape after
/// another (`Maps::layer`): rgba8 each, as `rock_maps` in the shader holds
/// them, surfaces in its first `ROCKS` layers and horizons in the rest.
pub struct Maps {
    /// Per mip level, from the finest: every layer's texels, layer by layer.
    pub levels: Vec<Vec<u8>>,
}

/// A shape's radius along every direction, scaled so its farthest point is
/// at 1 (an instance's size is its bounding radius): its body for the mesh,
/// and with its surface for the maps; and how fresh its ground is.
struct Built {
    body: Field,
    full: Field,
    fresh: Field,
}

fn build_shape(variant: u32) -> Built {
    let shape = Shape::new(variant);
    let body = Field::build(MAP / 2, |d| shape.body(d));
    let surface = Field::build(MAP, |d| shape.surface(d).0);
    let fresh = Field::build(MAP, |d| shape.surface(d).1);
    // Its mean radius 0.78 and its farthest reach at most 1.2, as the
    // surface nets' rocks had: the long ones smaller.
    let mean = body.values.iter().sum::<f32>() / body.values.len() as f32;
    let far = body.values.iter().cloned().fold(0.0, f32::max);
    let top = (mean / 0.78).max(far / 1.2);
    let body = Field {
        size: body.size,
        values: body.values.iter().map(|r| r / top).collect(),
    };
    let full = Field::build(MAP, |d| body.at(d) * (1.0 + surface.at(d)));
    Built { body, full, fresh }
}

/// A point on the surface of `r` along `d`.
fn point(r: &Field, d: Vec3) -> Vec3 {
    d * r.at(d)
}

/// The surface's normal along `d`, `eps` radians across.
fn normal(r: &Field, d: Vec3, eps: f32) -> Vec3 {
    let (t1, t2) = frame(d);
    let a = point(r, (d + t1 * eps).normalize()) - point(r, (d - t1 * eps).normalize());
    let b = point(r, (d + t2 * eps).normalize()) - point(r, (d - t2 * eps).normalize());
    let n = a.cross(b).normalize_or(d);
    if n.dot(d) < 0.0 { -n } else { n }
}

/// The sine of how high the ground rises above the plane across `d`,
/// looking from its surface along `d` toward each of the frame's four ways
/// (+t1, +t2, -t1, -t2), out to a quarter of the way round.
fn horizons(r: &Field, d: Vec3) -> [f32; 4] {
    let (t1, t2) = frame(d);
    let p = point(r, d);
    let mut out = [0.0; 4];
    for (k, way) in [t1, t2, -t1, -t2].into_iter().enumerate() {
        let mut best: f32 = -1.0;
        let mut theta: f32 = 0.004;
        while theta < 1.6 {
            let q = point(r, d * theta.cos() + way * theta.sin());
            let w = q - p;
            best = best.max(w.dot(d) / w.length().max(1e-6));
            theta *= 1.28;
        }
        out[k] = best;
    }
    out
}

fn unorm(x: f32) -> u8 {
    (x.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Every shape's maps, and its meshes at each level of detail.
pub fn build_all() -> (Maps, Vec<[Mesh; 3]>) {
    let shapes: Vec<Built> = (0..ROCKS).map(build_shape).collect();
    let texels = MAP * MAP;
    let mut finest = vec![0u8; texels * 4 * 2 * ROCKS as usize];
    let eps = 1.5 / MAP as f32;
    let threads = std::thread::available_parallelism()
        .map_or(2, |n| n.get())
        .clamp(1, 8);
    for (s, built) in shapes.iter().enumerate() {
        let (surface, rest) = finest.split_at_mut((ROCKS as usize + s) * texels * 4);
        let surface = &mut surface[s * texels * 4..(s + 1) * texels * 4];
        let horizon = &mut rest[..texels * 4];
        let rows = MAP.div_ceil(threads);
        std::thread::scope(|scope| {
            for (t, (sc, hc)) in surface
                .chunks_mut(rows * MAP * 4)
                .zip(horizon.chunks_mut(rows * MAP * 4))
                .enumerate()
            {
                let built = &built;
                scope.spawn(move || {
                    for i in 0..sc.len() / 4 {
                        let (x, y) = (i % MAP, t * rows + i / MAP);
                        let uv = (Vec2::new(x as f32, y as f32) + Vec2::splat(0.5)) / MAP as f32;
                        let d = octahedral_dir(uv);
                        let n = normal(&built.full, d, eps);
                        let h = horizons(&built.full, d);
                        // The horizon round the point as a Fourier series in
                        // the way looked: its mean, and its first harmonic as
                        // a vector across the radial direction (it leans
                        // toward where the ground rises). Free of any frame,
                        // so it filters cleanly everywhere, poles and all.
                        let (t1, t2) = frame(d);
                        let mean = h.iter().sum::<f32>() * 0.25;
                        let lean = t1 * (h[0] - h[2]) * 0.5 + t2 * (h[1] - h[3]) * 0.5;
                        let fresh = built.fresh.at(d);
                        sc[i * 4..i * 4 + 4].copy_from_slice(&[
                            unorm(n.x * 0.5 + 0.5),
                            unorm(n.y * 0.5 + 0.5),
                            unorm(n.z * 0.5 + 0.5),
                            unorm(fresh),
                        ]);
                        hc[i * 4..i * 4 + 4].copy_from_slice(&[
                            unorm(mean * 0.5 + 0.5),
                            unorm(lean.x * 0.5 + 0.5),
                            unorm(lean.y * 0.5 + 0.5),
                            unorm(lean.z * 0.5 + 0.5),
                        ]);
                    }
                });
            }
        });
    }
    let mut levels = vec![finest];
    let mut size = MAP;
    while levels.len() < MIPS as usize && size > 1 {
        let prev = levels.last().unwrap();
        let half = size / 2;
        let mut next = vec![0u8; half * half * 4 * 2 * ROCKS as usize];
        for layer in 0..2 * ROCKS as usize {
            let src = &prev[layer * size * size * 4..];
            let dst = &mut next[layer * half * half * 4..];
            for y in 0..half {
                for x in 0..half {
                    for c in 0..4 {
                        let at = |dx: usize, dy: usize| {
                            src[((2 * y + dy) * size + 2 * x + dx) * 4 + c] as u32
                        };
                        dst[(y * half + x) * 4 + c] =
                            ((at(0, 0) + at(1, 0) + at(0, 1) + at(1, 1) + 2) / 4) as u8;
                    }
                }
            }
        }
        levels.push(next);
        size = half;
    }
    let meshes = shapes
        .iter()
        .map(|built| GRID.map(|n| grid_mesh(&built.body, n)))
        .collect();
    (Maps { levels }, meshes)
}

/// The octahedral grid of `n` cells a side, each vertex out at the radius
/// along its direction; the diagonals following the octahedron's edges so
/// no cell folds.
fn grid_mesh(r: &Field, n: usize) -> Mesh {
    let side = n + 1;
    let mut vertices = Vec::with_capacity(side * side);
    for y in 0..side {
        for x in 0..side {
            let uv = Vec2::new(x as f32, y as f32) / n as f32;
            // On the square's rim, every direction twice: the same one
            // (`octahedral` gives each one place), so the seams close.
            let d = octahedral_dir(uv);
            let d = octahedral_dir(octahedral(d));
            let p = point(r, d);
            let nrm = normal(r, d, 2.0 / n as f32);
            vertices.push(ShipVertex {
                position: p.to_array(),
                normal: nrm.to_array(),
                extra: [255, 0, 0, 255],
            });
        }
    }
    let mut indices = Vec::with_capacity(n * n * 6);
    for y in 0..n {
        for x in 0..n {
            let a = (y * side + x) as u32;
            let b = a + 1;
            let c = a + side as u32;
            let d = c + 1;
            // In the quadrants where u and v lie on the same side of the
            // middle one diagonal, in the others the other.
            let same = (2 * x + 1 < n) == (2 * y + 1 < n);
            if same {
                indices.extend_from_slice(&[a, b, d, a, d, c]);
            } else {
                indices.extend_from_slice(&[a, b, c, b, d, c]);
            }
        }
    }
    // Wound outward.
    let p = |i: u32| Vec3::from_array(vertices[i as usize].position);
    let t = &indices[0..3];
    let face = (p(t[1]) - p(t[0])).cross(p(t[2]) - p(t[0]));
    if face.dot(p(t[0]) + p(t[1]) + p(t[2])) < 0.0 {
        for tri in indices.chunks_mut(3) {
            tri.swap(1, 2);
        }
    }
    let mut mesh = Mesh { vertices, indices };
    meshopt::optimize_vertex_cache_in_place(&mut mesh.indices, mesh.vertices.len());
    let kept = meshopt::optimize_vertex_fetch_in_place(&mut mesh.indices, &mut mesh.vertices);
    mesh.vertices.truncate(kept);
    mesh
}

const MAGIC: &[u8; 4] = b"RCK1";

fn cache_path() -> Option<std::path::PathBuf> {
    super::models::cache_dir().map(|d| d.join(format!("g{}-rocks.mesh", super::models::GENERATOR)))
}

/// Every shape's maps and meshes: from the cache, or built (and cached).
pub fn load_or_build() -> (Maps, Vec<[Mesh; 3]>) {
    if let Some(found) = cache_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| decode(&b))
    {
        return found;
    }
    let built = build_all();
    if let Some(path) = cache_path() {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let partial = path.with_extension("part");
        if std::fs::write(&partial, encode(&built)).is_ok() {
            let _ = std::fs::rename(&partial, &path);
        }
    }
    built
}

fn encode((maps, meshes): &(Maps, Vec<[Mesh; 3]>)) -> Vec<u8> {
    let mut out = MAGIC.to_vec();
    let chunk = |out: &mut Vec<u8>, bytes: &[u8]| {
        out.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
        out.extend_from_slice(bytes);
    };
    for level in &maps.levels {
        chunk(&mut out, level);
    }
    for mesh in meshes.iter().flatten() {
        chunk(&mut out, bytemuck::cast_slice(&mesh.vertices));
        chunk(&mut out, bytemuck::cast_slice(&mesh.indices));
    }
    out
}

fn decode(bytes: &[u8]) -> Option<(Maps, Vec<[Mesh; 3]>)> {
    let rest = bytes.strip_prefix(MAGIC)?;
    let mut at = 0;
    let mut chunk = || -> Option<&[u8]> {
        let len = u64::from_le_bytes(rest.get(at..at + 8)?.try_into().ok()?) as usize;
        let out = rest.get(at + 8..at + 8 + len)?;
        at += 8 + len;
        Some(out)
    };
    let mut levels = Vec::new();
    let mut size = MAP;
    for _ in 0..MIPS {
        let level = chunk()?;
        if level.len() != size * size * 4 * 2 * ROCKS as usize {
            return None;
        }
        levels.push(level.to_vec());
        size /= 2;
    }
    let mut meshes = Vec::new();
    for _ in 0..ROCKS {
        let mut lods = Vec::new();
        for _ in 0..3 {
            let v = chunk()?;
            let i = chunk()?;
            if v.len() % std::mem::size_of::<ShipVertex>() != 0 || i.len() % 4 != 0 {
                return None;
            }
            lods.push(Mesh {
                vertices: bytemuck::pod_collect_to_vec(v),
                indices: bytemuck::pod_collect_to_vec(i),
            });
        }
        meshes.push(lods.try_into().ok()?);
    }
    (at == rest.len()).then_some((Maps { levels }, meshes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn octahedral_round_trips() {
        for i in 0..500 {
            let mut rng = Rng::new(i);
            let d = Vec3::new(
                rng.unit() as f32 - 0.5,
                rng.unit() as f32 - 0.5,
                rng.unit() as f32 - 0.5,
            )
            .normalize();
            assert!((octahedral_dir(octahedral(d)) - d).length() < 1e-4);
        }
    }

    #[test]
    fn harmonics_are_orthonormal() {
        // Monte Carlo over the sphere: the mean of Y_i Y_j times 4π is the
        // identity.
        let n = 20000;
        let k = (DEGREE + 1) * (DEGREE + 1);
        let mut sums = vec![0.0f64; k * k];
        let mut rng = Rng::new(3);
        for _ in 0..n {
            let z = rng.unit() * 2.0 - 1.0;
            let a = rng.unit() * std::f64::consts::TAU;
            let s = (1.0 - z * z).sqrt();
            let y = harmonics(Vec3::new(
                (s * a.cos()) as f32,
                (s * a.sin()) as f32,
                z as f32,
            ));
            for i in 0..k {
                for j in 0..k {
                    sums[i * k + j] += y[i] * y[j];
                }
            }
        }
        for i in 0..k {
            for j in 0..k {
                let v = sums[i * k + j] / n as f64 * 4.0 * std::f64::consts::PI;
                let want = if i == j { 1.0 } else { 0.0 };
                assert!((v - want).abs() < 0.1, "{i} {j} {v}");
            }
        }
    }

    #[test]
    fn meshes_close_and_fit_their_bounds() {
        for v in 0..ROCKS {
            let b = build_shape(v);
            let far = b.body.values.iter().cloned().fold(0.0, f32::max);
            println!("shape {v}: farthest {far:.2}");
        }
        let built = build_shape(2);
        for n in GRID {
            let mesh = grid_mesh(&built.body, n);
            assert_eq!(mesh.indices.len(), n * n * 6);
            // Closed: every edge used twice, once each way, by position.
            let key = |i: u32| {
                let p = mesh.vertices[i as usize].position;
                [
                    (p[0] * 1e4) as i32,
                    (p[1] * 1e4) as i32,
                    (p[2] * 1e4) as i32,
                ]
            };
            let mut edges = std::collections::HashMap::new();
            for t in mesh.indices.chunks(3) {
                for e in 0..3 {
                    let (a, b) = (key(t[e]), key(t[(e + 1) % 3]));
                    *edges.entry((a, b)).or_insert(0i32) += 1;
                    *edges.entry((b, a)).or_insert(0i32) -= 1;
                }
            }
            assert!(edges.values().all(|&c| c == 0), "open at {n}");
            let far = mesh
                .vertices
                .iter()
                .map(|v| Vec3::from_array(v.position).length())
                .fold(0.0, f32::max);
            assert!(far <= 1.21 && far > 0.85, "{far}");
        }
    }

    /// Writes shape 2's surface and horizon maps as PNGs into
    /// `ROCK_MAPS_DIR`, to look at.
    #[test]
    #[ignore]
    fn dump_maps() {
        let dir = std::env::var("ROCK_MAPS_DIR").expect("ROCK_MAPS_DIR");
        let (maps, _) = build_all();
        let texels = MAP * MAP * 4;
        for (name, layer) in [("surface", 2), ("horizon", ROCKS as usize + 2)] {
            let data = maps.levels[0][layer * texels..(layer + 1) * texels].to_vec();
            image::RgbaImage::from_raw(MAP as u32, MAP as u32, data)
                .unwrap()
                .save(format!("{dir}/{name}.png"))
                .unwrap();
        }
    }
}
