//! The models of the 3D dive, generated when the scene is built.
//!
//! Nothing is loaded from disk: every model comes from a handful of generators —
//! surfaces of revolution, lofted bodies, extrusions and ribbons — so a fish, a
//! fluted column or a galleon's hull is a few dozen lines of numbers. Each
//! vertex carries a colour multiplier, for the countershading of a fish or the
//! planks of a hull, and two animation coordinates the vertex shader bends by:
//! `aux.x` runs along a body from tail (-1) to head (+1), or along an arm from
//! root (0) to tip (1); `aux.y` runs up a plant from root (0) to tip (1).
//!
//! Each model is also built coarser, at up to [`LODS`] levels of detail, from
//! the same recipe with half the segments at each step: [`Lod`] records how
//! far each strays from the full model, so the renderer only draws it where
//! the difference is too small to see.

use std::cell::Cell;
use std::f32::consts::{PI, TAU};

/// Levels of detail per model, the full one first.
pub const LODS: usize = 3;

thread_local! {
    /// How many times coarser than its full detail a model is being built.
    static COARSER: Cell<u32> = const { Cell::new(0) };
}

/// `n` segments, halved for each step coarser, but never fewer than `least`.
fn fewer(n: usize, least: usize) -> usize {
    (n >> COARSER.get()).max(least.min(n))
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub colour: [f32; 3],
    pub aux: [f32; 2],
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

/// The nearest point of triangle `t` to `p`, as barycentric weights of its
/// corners.
fn closest_on_triangle(p: [f32; 3], t: [[f32; 3]; 3]) -> [f32; 3] {
    let [a, b, c] = t;
    let (ab, ac, ap) = (sub(b, a), sub(c, a), sub(p, a));
    let (d1, d2) = (dot(ab, ap), dot(ac, ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = sub(p, b);
    let (d3, d4) = (dot(ab, bp), dot(ac, bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = sub(p, c);
    let (d5, d6) = (dot(ab, cp), dot(ac, cp));
    if d6 >= 0.0 && d5 <= d6 {
        return [0.0, 0.0, 1.0];
    }
    let vb = d5 * d2 - d1 * d6;
    if vb <= 0.0 && d2 >= 0.0 && d6 <= 0.0 {
        let w = d2 / (d2 - d6);
        return [1.0 - w, 0.0, w];
    }
    let va = d3 * d6 - d5 * d4;
    if va <= 0.0 && d4 - d3 >= 0.0 && d5 - d6 >= 0.0 {
        let w = (d4 - d3) / ((d4 - d3) + (d5 - d6));
        return [0.0, 1.0 - w, w];
    }
    let denom = 1.0 / (va + vb + vc);
    let (v, w) = (vb * denom, vc * denom);
    [1.0 - v - w, v, w]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
    [v[0] / l, v[1] / l, v[2] / l]
}

/// A small, stable hash for the shapes that want a little irregularity.
fn jitter(i: u32, salt: u32) -> f32 {
    let mut h = i.wrapping_mul(0x9e37_79b9) ^ salt.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

impl Mesh {
    fn vertex(&mut self, pos: [f32; 3], colour: [f32; 3], aux: [f32; 2]) -> u32 {
        self.vertices.push(Vertex {
            pos,
            normal: [0.0; 3],
            colour,
            aux,
        });
        self.vertices.len() as u32 - 1
    }

    fn tri(&mut self, a: u32, b: u32, c: u32) {
        self.indices.extend_from_slice(&[a, b, c]);
    }

    fn quad(&mut self, a: u32, b: u32, c: u32, d: u32) {
        self.tri(a, b, c);
        self.tri(a, c, d);
    }

    /// Smooth normals from the faces around each vertex.
    fn smooth(mut self) -> Self {
        let mut normals = vec![[0.0f32; 3]; self.vertices.len()];
        for t in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
            let n = cross(
                sub(self.vertices[b].pos, self.vertices[a].pos),
                sub(self.vertices[c].pos, self.vertices[a].pos),
            );
            for i in [a, b, c] {
                for k in 0..3 {
                    normals[i][k] += n[k];
                }
            }
        }
        for (v, n) in self.vertices.iter_mut().zip(normals) {
            v.normal = normalize(n);
        }
        self
    }

    /// A triangle with its own vertices and a flat normal, for hard edges.
    fn flat(&mut self, p: [[f32; 3]; 3], colour: [f32; 3], aux: impl Fn([f32; 3]) -> [f32; 2]) {
        let n = normalize(cross(sub(p[1], p[0]), sub(p[2], p[0])));
        let base = self.vertices.len() as u32;
        for q in p {
            self.vertices.push(Vertex {
                pos: q,
                normal: n,
                colour,
                aux: aux(q),
            });
        }
        self.tri(base, base + 1, base + 2);
    }

    pub fn append(&mut self, other: Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend(other.vertices);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }

    /// Move every vertex with `f`, rotating normals with `g`.
    fn map(mut self, f: impl Fn([f32; 3]) -> [f32; 3], g: impl Fn([f32; 3]) -> [f32; 3]) -> Self {
        for v in &mut self.vertices {
            v.pos = f(v.pos);
            v.normal = normalize(g(v.normal));
        }
        self
    }

    fn scaled(self, s: [f32; 3]) -> Self {
        self.map(
            move |p| [p[0] * s[0], p[1] * s[1], p[2] * s[2]],
            move |n| [n[0] / s[0], n[1] / s[1], n[2] / s[2]],
        )
    }

    /// Turn a shape built in the xy plane over onto its back, into xz.
    fn laid_flat(self) -> Self {
        self.map(|p| [p[0], p[2], -p[1]], |n| [n[0], n[2], -n[1]])
    }

    fn with_aux(mut self, f: impl Fn([f32; 3]) -> [f32; 2]) -> Self {
        for v in &mut self.vertices {
            v.aux = f(v.pos);
        }
        self
    }
}

/// A surface of revolution about y. `profile` is (radius, height) from bottom
/// to top; a zero radius closes the shape at that end. `radius` lets the
/// radius vary around the axis, for flutes.
fn lathe(
    profile: &[(f32, f32)],
    segments: usize,
    radius: impl Fn(f32, f32) -> f32,
    colour: impl Fn(f32, f32) -> [f32; 3],
    aux: impl Fn(f32) -> [f32; 2],
) -> Mesh {
    let segments = fewer(segments, 5);
    let mut m = Mesh::default();
    let mut rings = Vec::with_capacity(profile.len());
    for &(r, y) in profile {
        let ring: Vec<u32> = (0..segments)
            .map(|s| {
                let a = s as f32 / segments as f32 * TAU;
                let rr = r * radius(y, a);
                m.vertex([rr * a.cos(), y, rr * a.sin()], colour(y, a), aux(y))
            })
            .collect();
        rings.push(ring);
    }
    for w in rings.windows(2) {
        for s in 0..segments {
            let t = (s + 1) % segments;
            m.quad(w[0][s], w[1][s], w[1][t], w[0][t]);
        }
    }
    m.smooth()
}

/// A body lofted along x from `x0` (tail) to `x1` (head). At each station the
/// cross-section is an ellipse with separate upper and lower half-heights:
/// `section(x)` gives (centre height, top, bottom, half width).
fn loft(
    x0: f32,
    x1: f32,
    stations: usize,
    ring: usize,
    section: impl Fn(f32) -> (f32, f32, f32, f32),
    colour: impl Fn(f32, f32) -> [f32; 3],
) -> Mesh {
    let (stations, ring) = (fewer(stations, 4), fewer(ring, 4));
    let mut m = Mesh::default();
    let span = x1 - x0;
    let along = |x: f32| [((x - x0) / span) * 2.0 - 1.0, 0.0];
    let tail = m.vertex([x0, section(x0).0, 0.0], colour(x0, 0.0), along(x0));
    let mut rings = Vec::new();
    for i in 1..stations {
        let x = x0 + span * i as f32 / stations as f32;
        let (cy, top, bottom, hw) = section(x);
        let r: Vec<u32> = (0..ring)
            .map(|s| {
                let a = s as f32 / ring as f32 * TAU;
                let up = a.sin();
                let y = cy + up * if up > 0.0 { top } else { bottom };
                m.vertex([x, y, hw * a.cos()], colour(x, up), along(x))
            })
            .collect();
        rings.push(r);
    }
    let head = m.vertex([x1, section(x1).0, 0.0], colour(x1, 0.0), along(x1));
    for s in 0..ring {
        let t = (s + 1) % ring;
        m.tri(tail, rings[0][t], rings[0][s]);
        m.tri(head, rings[rings.len() - 1][s], rings[rings.len() - 1][t]);
    }
    for w in rings.windows(2) {
        for s in 0..ring {
            let t = (s + 1) % ring;
            m.quad(w[0][s], w[0][t], w[1][t], w[1][s]);
        }
    }
    m.smooth()
}

/// A flat polygon in the xy plane, `thickness` deep in z, with hard edges. The
/// polygon must be star-shaped about its first point's centroid.
fn extrude(poly: &[[f32; 2]], thickness: f32, colour: [f32; 3]) -> Mesh {
    let mut m = Mesh::default();
    let n = poly.len();
    let c = poly.iter().fold([0.0, 0.0], |a, p| {
        [a[0] + p[0] / n as f32, a[1] + p[1] / n as f32]
    });
    let h = thickness * 0.5;
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        // Front and back caps, fanned from the centroid.
        m.flat(
            [[c[0], c[1], -h], [a[0], a[1], -h], [b[0], b[1], -h]],
            colour,
            |_| [0.0; 2],
        );
        m.flat(
            [[c[0], c[1], h], [b[0], b[1], h], [a[0], a[1], h]],
            colour,
            |_| [0.0; 2],
        );
        // The side.
        m.flat(
            [[a[0], a[1], -h], [a[0], a[1], h], [b[0], b[1], h]],
            colour,
            |_| [0.0; 2],
        );
        m.flat(
            [[a[0], a[1], -h], [b[0], b[1], h], [b[0], b[1], -h]],
            colour,
            |_| [0.0; 2],
        );
    }
    m
}

fn icosphere(subdivisions: u32) -> Vec<[f32; 3]> {
    let t = (1.0 + 5f32.sqrt()) / 2.0;
    let mut verts: Vec<[f32; 3]> = [
        [-1.0, t, 0.0],
        [1.0, t, 0.0],
        [-1.0, -t, 0.0],
        [1.0, -t, 0.0],
        [0.0, -1.0, t],
        [0.0, 1.0, t],
        [0.0, -1.0, -t],
        [0.0, 1.0, -t],
        [t, 0.0, -1.0],
        [t, 0.0, 1.0],
        [-t, 0.0, -1.0],
        [-t, 0.0, 1.0],
    ]
    .iter()
    .map(|v| normalize(*v))
    .collect();
    let mut faces: Vec<[u32; 3]> = vec![
        [0, 11, 5],
        [0, 5, 1],
        [0, 1, 7],
        [0, 7, 10],
        [0, 10, 11],
        [1, 5, 9],
        [5, 11, 4],
        [11, 10, 2],
        [10, 7, 6],
        [7, 1, 8],
        [3, 9, 4],
        [3, 4, 2],
        [3, 2, 6],
        [3, 6, 8],
        [3, 8, 9],
        [4, 9, 5],
        [2, 4, 11],
        [6, 2, 10],
        [8, 6, 7],
        [9, 8, 1],
    ];
    for _ in 0..subdivisions {
        let mut cache = std::collections::HashMap::new();
        let mut mid = |a: u32, b: u32, verts: &mut Vec<[f32; 3]>| -> u32 {
            let key = (a.min(b), a.max(b));
            *cache.entry(key).or_insert_with(|| {
                let (p, q) = (verts[a as usize], verts[b as usize]);
                verts.push(normalize([p[0] + q[0], p[1] + q[1], p[2] + q[2]]));
                verts.len() as u32 - 1
            })
        };
        let mut next = Vec::with_capacity(faces.len() * 4);
        for [a, b, c] in faces {
            let ab = mid(a, b, &mut verts);
            let bc = mid(b, c, &mut verts);
            let ca = mid(c, a, &mut verts);
            next.extend_from_slice(&[[a, ab, ca], [b, bc, ab], [c, ca, bc], [ab, bc, ca]]);
        }
        faces = next;
    }
    let mut flat = Vec::with_capacity(faces.len() * 3);
    for f in faces {
        for i in f {
            flat.push(verts[i as usize]);
        }
    }
    flat
}

fn sphere_with(
    subdivisions: u32,
    displace: impl Fn([f32; 3]) -> f32,
    colour: impl Fn([f32; 3]) -> [f32; 3],
) -> Mesh {
    let corners = icosphere(subdivisions.saturating_sub(COARSER.get()));
    let mut m = Mesh::default();
    let mut index = std::collections::HashMap::new();
    for p in corners {
        let key = (
            (p[0] * 1e5) as i32,
            (p[1] * 1e5) as i32,
            (p[2] * 1e5) as i32,
        );
        let i = *index.entry(key).or_insert_with(|| {
            let d = displace(p);
            m.vertex(
                [p[0] * d, p[1] * d, p[2] * d],
                colour(p),
                [0.0, (p[1] + 1.0) * 0.5],
            )
        });
        m.indices.push(i);
    }
    m.smooth()
}

/// Every model the scene draws, by name. The order is the order of the draw
/// ranges in [`Library`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Model {
    Sphere,
    Rock0,
    Rock1,
    Rock2,
    Cylinder,
    Cone,
    Box,
    Hex,
    Plane,
    Kelp,
    Tube,
    Capsule,
    Fish,
    Shark,
    Whale,
    Dolphin,
    Narwhal,
    Penguin,
    Mantle,
    Bell,
    Shaft,
    Capital,
    Break,
    Hull,
    Sail,
    Fan,
    Star,
    Prism,
    Keel,
    Shell,
    Sponge,
}

impl Model {
    pub const ALL: [Model; 31] = [
        Model::Sphere,
        Model::Rock0,
        Model::Rock1,
        Model::Rock2,
        Model::Cylinder,
        Model::Cone,
        Model::Box,
        Model::Hex,
        Model::Plane,
        Model::Kelp,
        Model::Tube,
        Model::Capsule,
        Model::Fish,
        Model::Shark,
        Model::Whale,
        Model::Dolphin,
        Model::Narwhal,
        Model::Penguin,
        Model::Mantle,
        Model::Bell,
        Model::Shaft,
        Model::Capital,
        Model::Break,
        Model::Hull,
        Model::Sail,
        Model::Fan,
        Model::Star,
        Model::Prism,
        Model::Keel,
        Model::Shell,
        Model::Sponge,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    fn build(self) -> Mesh {
        let white = |_: f32, _: f32| [1.0; 3];
        let none = |_: f32| [0.0, 0.0];
        match self {
            Model::Sphere => sphere_with(2, |_| 1.0, |_| [1.0; 3]),
            Model::Rock0 | Model::Rock1 | Model::Rock2 => {
                let salt = self.index() as u32 * 7 + 3;
                sphere_with(
                    2,
                    move |p| {
                        // Lumpy, with a flattened base so it sits on the sand.
                        let lumps = (p[0] * 3.1 + salt as f32).sin() * (p[1] * 2.7).cos() * 0.12
                            + (p[2] * 4.3 + salt as f32 * 0.7).sin() * 0.08;
                        let base = if p[1] < -0.3 { 0.75 } else { 1.0 };
                        (1.0 + lumps) * base
                    },
                    move |p| {
                        let v =
                            0.80 + 0.30 * jitter((p[0] * 50.0 + p[2] * 31.0) as i32 as u32, salt);
                        [v, v, v * 0.97]
                    },
                )
            }
            Model::Cylinder => lathe(
                &[(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)],
                12,
                |_, _| 1.0,
                white,
                none,
            ),
            Model::Cone => lathe(
                &[(0.0, 0.0), (1.0, 0.0), (0.0, 1.0)],
                12,
                |_, _| 1.0,
                white,
                none,
            ),
            Model::Box => extrude(
                &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
                2.0,
                [1.0; 3],
            ),
            Model::Hex => {
                let hex: Vec<[f32; 2]> = (0..6)
                    .map(|i| {
                        let a = i as f32 / 6.0 * TAU;
                        [a.cos(), a.sin()]
                    })
                    .collect();
                // Stood upright, so the prism's axis is y.
                extrude(&hex, 2.0, [1.0; 3])
                    .map(|p| [p[0], p[2] * 0.5 + 0.5, p[1]], |n| [n[0], n[2], n[1]])
            }
            Model::Plane => {
                let mut m = Mesh::default();
                let a = m.vertex([-1.0, 0.0, -1.0], [1.0; 3], [0.0; 2]);
                let b = m.vertex([1.0, 0.0, -1.0], [1.0; 3], [0.0; 2]);
                let c = m.vertex([1.0, 0.0, 1.0], [1.0; 3], [0.0; 2]);
                let d = m.vertex([-1.0, 0.0, 1.0], [1.0; 3], [0.0; 2]);
                m.quad(a, b, c, d);
                for v in &mut m.vertices {
                    v.normal = [0.0, -1.0, 0.0];
                }
                m
            }
            Model::Kelp => kelp(),
            Model::Tube => lathe(
                &[
                    (1.0, 0.0),
                    (0.85, 0.25),
                    (0.65, 0.5),
                    (0.45, 0.75),
                    (0.30, 0.9),
                    (0.0, 1.0),
                ],
                6,
                |_, _| 1.0,
                white,
                |y| [y, 0.0],
            ),
            Model::Capsule => {
                // Rounded ends of radius 1 on a shaft, from y = 0 to 1 once
                // scaled: limbs and arms are capsules stretched between joints.
                let mut p = vec![(0.0f32, 0.0f32)];
                for i in 1..=4 {
                    let a = i as f32 / 4.0 * PI * 0.5;
                    p.push((a.sin() * 0.25, 0.25 - a.cos() * 0.25));
                }
                for i in 1..4 {
                    let a = i as f32 / 4.0 * PI * 0.5;
                    p.push((a.cos() * 0.25, 0.75 + a.sin() * 0.25));
                }
                p.push((0.0, 1.0));
                lathe(&p, 10, |_, _| 1.0, white, none).scaled([4.0, 1.0, 4.0])
            }
            Model::Fish => fish(),
            Model::Shark => shark(),
            Model::Whale => whale(),
            Model::Dolphin => dolphin(false),
            Model::Narwhal => dolphin(true),
            Model::Penguin => penguin(),
            Model::Mantle => mantle(),
            Model::Bell => {
                let mut p = Vec::new();
                for i in 0..=8 {
                    let a = i as f32 / 8.0 * PI * 0.5;
                    // A dome with a flared, scalloped margin.
                    p.push((a.cos() * (1.0 + 0.12 * (1.0 - a.sin())), a.sin() * 0.7));
                }
                p.reverse();
                p.insert(0, (0.0, 0.7));
                p.dedup();
                p.reverse();
                lathe(
                    &p,
                    16,
                    |y, a| {
                        if y < 0.05 {
                            1.0 + 0.06 * (a * 8.0).sin()
                        } else {
                            1.0
                        }
                    },
                    |y, _| [1.0, 1.0, 1.0 - 0.4 * y],
                    |y| [0.0, y],
                )
            }
            Model::Shaft => {
                // A Doric shaft with twenty flutes and a slight entasis.
                let rings: Vec<(f32, f32)> = (0..=8)
                    .map(|i| {
                        let y = i as f32 / 8.0;
                        (1.0 - 0.16 * y + 0.03 * (y * PI).sin(), y)
                    })
                    .collect();
                let mut profile = vec![(0.0, 0.0)];
                profile.extend(rings);
                profile.push((0.0, 1.0));
                lathe(
                    &profile,
                    40,
                    |_, a| 1.0 - 0.05 * (a * 20.0).cos().abs().powf(0.5),
                    |_, a| {
                        let v = 0.90 + 0.10 * (a * 20.0).cos();
                        [v, v, v]
                    },
                    |y| [0.0, y],
                )
            }
            Model::Capital => lathe(
                &[
                    (0.0, 0.0),
                    (0.86, 0.0),
                    (1.05, 0.35),
                    (1.25, 0.6),
                    (1.25, 1.0),
                    (0.0, 1.0),
                ],
                16,
                |_, _| 1.0,
                white,
                none,
            ),
            Model::Break => {
                // The jagged top of a broken shaft.
                lathe(
                    &[(0.84, 0.0), (0.7, 0.15), (0.35, 0.05), (0.0, 0.2)],
                    14,
                    |y, a| 1.0 + 0.25 * ((a * 3.0).sin() * (a * 7.0).cos()) * y,
                    white,
                    none,
                )
                .map(
                    |p| {
                        [
                            p[0],
                            p[1] + 0.2 * (p[0] * 5.0).sin() * (p[2] * 4.0).cos(),
                            p[2],
                        ]
                    },
                    |n| n,
                )
            }
            Model::Hull => hull(),
            Model::Sail => sail(),
            Model::Fan => fan(),
            Model::Star => {
                let star: Vec<[f32; 2]> = (0..10)
                    .map(|i| {
                        let a = i as f32 / 10.0 * TAU + PI * 0.5;
                        let r = if i % 2 == 0 { 1.0 } else { 0.38 };
                        [a.cos() * r, a.sin() * r]
                    })
                    .collect();
                extrude(&star, 0.25, [1.0; 3]).laid_flat()
            }
            Model::Prism => extrude(&[[-1.0, 0.0], [1.0, 0.0], [0.0, 1.0]], 2.0, [1.0; 3]),
            Model::Keel => keel(),
            Model::Shell => {
                // One valve of a giant clam: a scalloped half-dome, ridged.
                let mut p = Vec::new();
                for i in 0..=6 {
                    let a = i as f32 / 6.0 * PI * 0.5;
                    p.push((a.cos(), a.sin() * 0.55));
                }
                p.reverse();
                p.insert(0, (0.0, 0.55));
                p.dedup();
                p.reverse();
                lathe(
                    &p,
                    18,
                    |_, a| 1.0 + 0.08 * (a * 6.0).sin().abs(),
                    |_, a| {
                        let v = 0.8 + 0.2 * (a * 12.0).cos();
                        [v, v, v * 0.95]
                    },
                    none,
                )
            }
            Model::Sponge => lathe(
                &[
                    (0.8, 0.0),
                    (1.0, 0.5),
                    (0.9, 0.95),
                    (0.75, 1.0),
                    (0.55, 0.85),
                    (0.4, 0.3),
                ],
                10,
                |_, _| 1.0,
                |y, _| if y > 0.95 { [0.4, 0.35, 0.3] } else { [1.0; 3] },
                |y| [0.0, y],
            ),
        }
    }
}

fn kelp() -> Mesh {
    // A stipe with blades on alternate sides, twisting as it climbs.
    let mut m = Mesh::default();
    let segments = fewer(28, 7);
    let mut previous: Option<(u32, u32)> = None;
    for i in 0..=segments {
        let y = i as f32 / segments as f32;
        let blade = (y * 26.0).sin();
        let width = 0.012 + 0.030 * blade.max(0.0).powf(1.5) * (0.4 + y);
        let side = blade.signum() * 0.010 * (0.4 + y);
        let twist = y * 4.0;
        let (s, c) = twist.sin_cos();
        let a = m.vertex(
            [(side - width) * c, y, (side - width) * s],
            [0.9 + 0.2 * y; 3],
            [0.0, y],
        );
        let b = m.vertex(
            [(side + width) * c, y, (side + width) * s],
            [0.9 + 0.2 * y; 3],
            [0.0, y],
        );
        if let Some((pa, pb)) = previous {
            m.quad(pa, pb, b, a);
        }
        previous = Some((a, b));
    }
    m.smooth()
}

/// Countershading: dark along the back, pale underneath.
fn countershade(back: f32, belly: f32) -> impl Fn(f32, f32) -> [f32; 3] {
    move |_, up| {
        let t = (up * 0.5 + 0.5).clamp(0.0, 1.0);
        let v = belly + (back - belly) * t;
        [v, v, v]
    }
}

fn fish() -> Mesh {
    // Length 2 from tail (-1) to snout (+1), so instances scale by half a length.
    let mut m = loft(
        -0.75,
        1.0,
        12,
        10,
        |x| {
            let t = (x + 0.75) / 1.75;
            let h = 0.26 * (t * PI).sin().powf(0.7) + 0.02;
            (0.0, h, h * 0.9, h * 0.45)
        },
        countershade(0.55, 1.45),
    );
    let tail = extrude(
        &[[-0.70, 0.0], [-1.05, 0.30], [-0.92, 0.0], [-1.05, -0.30]],
        0.01,
        [0.8; 3],
    );
    let dorsal = extrude(
        &[[-0.25, 0.20], [0.25, 0.22], [-0.15, 0.40]],
        0.01,
        [0.6; 3],
    );
    let anal = extrude(
        &[[-0.50, -0.12], [-0.25, -0.16], [-0.52, -0.28]],
        0.01,
        [0.9; 3],
    );
    let pectoral = extrude(
        &[[0.35, -0.05], [0.10, -0.08], [0.18, -0.20]],
        0.01,
        [0.9; 3],
    )
    .map(|p| [p[0], p[1], p[2] + 0.10], |n| n);
    for part in [tail, dorsal, anal, pectoral] {
        m.append(part);
    }
    m.with_aux(|p| [p[0], 0.0])
}

fn shark() -> Mesh {
    let mut m = loft(
        -0.80,
        1.0,
        14,
        10,
        |x| {
            let t = (x + 0.8) / 1.8;
            let h =
                0.17 * (t * PI).sin().powf(0.8) * (1.0 - 0.4 * ((x - 0.6).max(0.0) / 0.4)) + 0.015;
            (0.0, h, h * 0.85, h * 0.85)
        },
        countershade(0.75, 1.6),
    );
    // The long upper lobe of the tail, the tall first dorsal, rigid pectorals.
    let tail = extrude(
        &[[-0.75, 0.0], [-1.10, 0.36], [-0.95, 0.02], [-1.0, -0.17]],
        0.012,
        [0.75; 3],
    );
    let dorsal = extrude(
        &[[-0.10, 0.12], [0.20, 0.13], [-0.06, 0.40]],
        0.012,
        [0.7; 3],
    );
    let second = extrude(
        &[[-0.55, 0.06], [-0.45, 0.06], [-0.53, 0.13]],
        0.010,
        [0.7; 3],
    );
    let fin = extrude(
        &[[0.25, -0.05], [0.42, -0.06], [0.05, -0.30]],
        0.010,
        [0.9; 3],
    );
    let left = fin.clone().map(|p| [p[0], p[1], p[2] + 0.10], |n| n);
    let right = fin.map(|p| [p[0], p[1], p[2] - 0.10], |n| n);
    for part in [tail, dorsal, second, left, right] {
        m.append(part);
    }
    m.with_aux(|p| [p[0], 0.0])
}

/// A North Atlantic right whale, one unit long with the snout at +0.5: a third
/// of it head, a narrow arched rostrum, a huge bowed lower lip, broad paddle
/// flippers, no dorsal fin, and pale callosities on the head.
fn whale() -> Mesh {
    let mut m = loft(
        -0.47,
        0.50,
        22,
        14,
        |x| {
            let smooth = |a: f32, b: f32, v: f32| {
                let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
                t * t * (3.0 - 2.0 * t)
            };
            // The back: thin at the tail stock, broad over the body, dipping
            // behind the blowholes, then the narrow rostrum arching down.
            let mut top = 0.018 + (0.105 - 0.018) * smooth(-0.47, -0.10, x);
            top += (0.080 - top) * smooth(0.10, 0.22, x);
            if x > 0.22 {
                let a = (x - 0.22) / 0.30;
                top = 0.080 * (1.0 - a * a).max(0.0).sqrt().max(0.12);
            }
            // The belly, and under the head the great bowed lower lip.
            let mut bottom = 0.015 + (0.100 - 0.015) * smooth(-0.47, -0.10, x);
            let a = (x - 0.30) / 0.23;
            bottom = bottom.max(0.130 * (1.0 - a * a).max(0.0).sqrt());
            if x > 0.48 {
                bottom = bottom.min(0.03);
            }
            (
                0.0,
                top.max(0.010),
                bottom.max(0.010),
                (top + bottom) * 0.48,
            )
        },
        |x, up| {
            // Black, with the crusted pale callosities about the head.
            let callus = x > 0.25 && ((x * 90.0).sin() * (up * 13.0).cos() > 0.55) && up > 0.3;
            let chin = x > 0.32 && up < -0.6 && (x * 70.0).cos() > 0.3;
            if callus || chin {
                [5.5, 5.4, 4.8]
            } else {
                [0.9, 0.9, 0.95]
            }
        },
    );
    // Horizontal flukes, a broad paddle flipper on each side.
    let flukes = extrude(
        &[[-0.42, 0.0], [-0.56, 0.13], [-0.50, 0.0], [-0.56, -0.13]],
        0.010,
        [0.9; 3],
    )
    .laid_flat();
    let flipper = extrude(
        &[[0.20, 0.0], [0.08, 0.0], [0.04, -0.10], [0.12, -0.13]],
        0.012,
        [0.9; 3],
    );
    let left = flipper
        .clone()
        .map(|p| [p[0], p[1] - 0.05, p[2] + 0.09], |n| n);
    let right = flipper.map(|p| [p[0], p[1] - 0.05, p[2] - 0.09], |n| n);
    for part in [flukes, left, right] {
        m.append(part);
    }
    m.with_aux(|p| [p[0] * 2.0, 0.0])
}

fn dolphin(narwhal: bool) -> Mesh {
    let mut m = loft(
        -0.47,
        if narwhal { 0.46 } else { 0.52 },
        18,
        12,
        move |x| {
            let t = (x + 0.47) / 0.95;
            let mut h = 0.085 * (t * PI).sin().powf(0.6) + 0.008;
            if !narwhal && x > 0.38 {
                h = 0.020; // the beak
            }
            (0.0, h, h * 0.95, h * 0.9)
        },
        countershade(0.75, 1.5),
    );
    let flukes = extrude(
        &[[-0.42, 0.0], [-0.54, 0.10], [-0.49, 0.0], [-0.54, -0.10]],
        0.008,
        [0.8; 3],
    )
    .laid_flat();
    let flipper = extrude(
        &[[0.22, -0.04], [0.14, -0.05], [0.10, -0.12]],
        0.008,
        [0.9; 3],
    );
    let left = flipper.clone().map(|p| [p[0], p[1], p[2] + 0.06], |n| n);
    let right = flipper.map(|p| [p[0], p[1], p[2] - 0.06], |n| n);
    for part in [flukes, left, right] {
        m.append(part);
    }
    if narwhal {
        // The spiral tusk, as a long thin cone out of the upper lip.
        let tusk = lathe(
            &[(0.0, 0.0), (0.010, 0.0), (0.004, 0.46), (0.0, 0.48)],
            6,
            |_, _| 1.0,
            |_, _| [6.0, 5.7, 4.8],
            |_| [0.0; 2],
        )
        .map(
            |p| [p[1] + 0.44, p[0] + 0.005, p[2]],
            |n| [n[1], n[0], n[2]],
        );
        m.append(tusk);
    } else {
        let fin = extrude(
            &[[-0.08, 0.06], [0.07, 0.07], [-0.12, 0.175]],
            0.008,
            [0.75; 3],
        );
        m.append(fin);
    }
    m.with_aux(|p| [p[0] * 2.0, 0.0])
}

fn penguin() -> Mesh {
    let mut m = loft(
        -0.42,
        0.42,
        14,
        10,
        |x| {
            let t = (x + 0.42) / 0.84;
            let h = 0.12 * (t * PI).sin().powf(0.6) + 0.01;
            (0.0, h, h, h * 0.95)
        },
        |x, up| {
            // Black back and head, white front, a touch of gold at the neck.
            if up < -0.1 && x < 0.30 {
                [6.0, 6.0, 5.8]
            } else if x > 0.26 && x < 0.32 && up > -0.2 {
                [5.0, 3.6, 1.0]
            } else {
                [0.6, 0.6, 0.7]
            }
        },
    );
    let beak = lathe(
        &[(0.0, 0.0), (0.022, 0.0), (0.0, 0.12)],
        6,
        |_, _| 1.0,
        |_, _| [1.5; 3],
        |_| [0.0; 2],
    )
    .map(|p| [p[1] + 0.40, p[0] + 0.01, p[2]], |n| [n[1], n[0], n[2]]);
    m.append(beak);
    m.with_aux(|p| [p[0] * 2.0, 0.0])
}

/// A giant squid's mantle and fins, the tip at +0.4 and the head end at 0.
fn mantle() -> Mesh {
    let mut m = loft(
        -0.06,
        0.42,
        14,
        10,
        |x| {
            let t = (x + 0.06) / 0.48;
            let r = 0.07 * (1.0 - t * t * 0.9).max(0.05) * (t * 6.0).clamp(0.6, 1.0);
            (0.0, r, r, r)
        },
        |x, up| {
            let v = 0.85 + 0.25 * ((x * 60.0).sin() * (up * 9.0).cos());
            [v, v, v]
        },
    );
    let fins = extrude(
        &[[0.28, 0.0], [0.36, 0.09], [0.42, 0.0], [0.36, -0.09]],
        0.008,
        [0.9; 3],
    )
    .laid_flat();
    m.append(fins);
    m
}

fn hull() -> Mesh {
    // A galleon's hull, one unit long: full in the waist, a raked bow, a
    // square stern and a deep keel. Planks show as bands of tone.
    loft(
        -0.5,
        0.55,
        20,
        14,
        |x| {
            let t = ((x + 0.5) / 1.05).clamp(0.0, 1.0);
            let beam = 0.13
                * (1.0 - (2.0 * t - 0.9).powi(2))
                    .max(0.0)
                    .sqrt()
                    .max(if x < -0.45 { 0.10 } else { 0.0 });
            let depth = 0.12 * (1.0 - (2.0 * t - 1.0).powi(4)).max(0.0) + 0.01;
            (0.0, 0.05, depth, beam.max(0.01))
        },
        |_, up| {
            let band = ((up * 9.0).sin() * 0.5 + 0.5) * 0.25 + 0.8;
            [band, band * 0.95, band * 0.9]
        },
    )
}

fn sail() -> Mesh {
    // A torn, bellied square sail hanging from its yard: x across, y down.
    let mut m = Mesh::default();
    let n = 6;
    let mut grid = vec![vec![0u32; n + 1]; n + 1];
    for (j, row) in grid.iter_mut().enumerate() {
        for (i, cell) in row.iter_mut().enumerate() {
            let x = i as f32 / n as f32 * 2.0 - 1.0;
            let y = -(j as f32 / n as f32);
            let belly = (1.0 - x * x) * (-y) * 0.25;
            // The lower edge is in tatters.
            let torn = if j == n {
                -0.25 * jitter(i as u32, 77)
            } else {
                0.0
            };
            *cell = m.vertex(
                [x, y - torn, belly],
                [1.0 - 0.15 * jitter((i * 7 + j) as u32, 5); 3],
                [0.0, -y],
            );
        }
    }
    for j in 0..n {
        for i in 0..n {
            m.quad(
                grid[j][i],
                grid[j][i + 1],
                grid[j + 1][i + 1],
                grid[j + 1][i],
            );
        }
    }
    m.smooth()
}

fn fan() -> Mesh {
    // A gorgonian sea fan: a ribbed half-disc, stood on edge.
    let mut m = Mesh::default();
    let centre = m.vertex([0.0, 0.0, 0.0], [0.8; 3], [0.0, 0.0]);
    let rim = 18;
    let mut prev = None;
    for i in 0..=rim {
        let a = i as f32 / rim as f32 * PI;
        let r = 1.0 + 0.12 * (a * 7.0).sin();
        let rib = if i % 2 == 0 { 1.25 } else { 0.85 };
        let v = m.vertex([a.cos() * r, a.sin() * r, 0.0], [rib; 3], [0.0, a.sin()]);
        if let Some(p) = prev {
            m.tri(centre, p, v);
        }
        prev = Some(v);
    }
    for v in &mut m.vertices {
        v.normal = [0.0, 0.0, -1.0];
    }
    m
}

fn keel() -> Mesh {
    // A pressure ridge: slabs of broken ice jammed together, hanging down
    // from y = 0.
    let mut m = Mesh::default();
    for i in 0..7u32 {
        let x = (i as f32 / 6.0) * 2.0 - 1.0;
        let depth = 0.35 + 0.65 * (1.0 - x * x) * (0.6 + 0.4 * jitter(i, 3));
        let w = 0.18 + 0.12 * jitter(i, 9);
        let tilt = (jitter(i, 21) - 0.5) * 0.6;
        let slab = extrude(
            &[
                [-w, 0.0],
                [w, 0.0],
                [w * 0.8, -depth],
                [-w * 0.7, -depth * 0.9],
            ],
            0.5 + 0.4 * jitter(i, 4),
            [0.8 + 0.4 * jitter(i, 13); 3],
        );
        let (s, c) = tilt.sin_cos();
        m.append(slab.map(
            move |p| [p[0] * c - p[1] * s + x, p[0] * s + p[1] * c, p[2]],
            move |n| [n[0] * c - n[1] * s, n[0] * s + n[1] * c, n[2]],
        ));
    }
    m
}

/// All models in one vertex and index buffer, with each model's range.
pub struct Library {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
    /// Per model, its levels of detail, the full one first; as many as
    /// building it coarser makes a difference to.
    pub lods: Vec<Vec<Lod>>,
    /// Per model, whether it is sealed: its back faces are never seen.
    pub sealed: Vec<bool>,
}

/// One level of detail of a model, in the library's buffers.
#[derive(Clone, Copy, Debug)]
pub struct Lod {
    pub first: u32,
    pub count: u32,
    pub base: i32,
    /// How far its surface strays from the full model's, either way, in the
    /// model's own units.
    pub error: f32,
    /// How far its colours and normals stray from the full model's: see
    /// `Mesh::distance`.
    pub stray: f32,
    /// The longest edge of what it changed: how fine the colours and normals
    /// it carries there are.
    pub edge: f32,
}

impl Library {
    pub fn build() -> Self {
        let mut vertices = Vec::new();
        let mut indices = Vec::new();
        let mut lods = Vec::new();
        let mut sealed = Vec::new();
        for model in Model::ALL {
            let mut levels: Vec<Mesh> = Vec::new();
            for step in 0..LODS as u32 {
                COARSER.set(step);
                let mesh = model.build();
                COARSER.set(0);
                if levels
                    .last()
                    .is_some_and(|l| l.vertices.len() == mesh.vertices.len())
                {
                    break;
                }
                levels.push(mesh);
            }
            // Wound either way when built; the sealed ones are turned to face
            // out, the way back faces are culled. A model is sealed only if
            // every one of its levels is.
            let closed: Vec<_> = levels.iter().map(Mesh::closed).collect();
            for (mesh, closed) in levels.iter_mut().zip(&closed) {
                if *closed == Some(false) {
                    for t in mesh.indices.as_chunks_mut::<3>().0 {
                        t.swap(1, 2);
                    }
                }
            }
            sealed.push(closed.iter().all(Option::is_some));
            let mut list = Vec::new();
            for mesh in &levels {
                list.push(Lod {
                    first: indices.len() as u32,
                    count: mesh.indices.len() as u32,
                    base: vertices.len() as i32,
                    error: levels[0].distance(mesh).0.max(mesh.distance(&levels[0]).0),
                    stray: levels[0].distance(mesh).1,
                    edge: mesh.longest_new_edge(&levels[0]),
                });
                vertices.extend_from_slice(&mesh.vertices);
                indices.extend_from_slice(&mesh.indices);
            }
            lods.push(list);
        }
        Self {
            vertices,
            indices,
            lods,
            sealed,
        }
    }
}

impl Mesh {
    /// How far any of this mesh's vertices lies from `other`'s surface; and
    /// how far `other`'s colour and normal there stray from the vertex's own,
    /// the larger of the two, the normal's by the sine of the angle between.
    fn distance(&self, other: &Mesh) -> (f32, f32) {
        let triangles: Vec<[u32; 3]> = other.indices.as_chunks::<3>().0.to_vec();
        let corner = |t: &[u32; 3]| t.map(|i| other.vertices[i as usize].pos);
        let (mut far, mut stray) = (0.0f32, 0.0f32);
        for v in &self.vertices {
            let nearest: Vec<(f32, [u32; 3], [f32; 3])> = triangles
                .iter()
                .map(|t| {
                    let c = corner(t);
                    let w = closest_on_triangle(v.pos, c);
                    let q = [0, 1, 2].map(|k| c[0][k] * w[0] + c[1][k] * w[1] + c[2][k] * w[2]);
                    let d = sub(v.pos, q);
                    (dot(d, d).sqrt(), *t, w)
                })
                .collect();
            let best = nearest.iter().map(|n| n.0).fold(f32::MAX, f32::min);
            far = far.max(best);
            let blend = |t: [u32; 3], w: [f32; 3], f: fn(&Vertex) -> [f32; 3]| {
                let [a, b, c] = t.map(|i| f(&other.vertices[i as usize]));
                [0, 1, 2].map(|k| a[k] * w[0] + b[k] * w[1] + c[k] * w[2])
            };
            // Where creases meet at the nearest point, the side facing the
            // way this vertex does.
            let (t, w) = nearest
                .iter()
                .filter(|n| n.0 <= best + 1e-4)
                .map(|n| (n.1, n.2))
                .max_by(|a, b| {
                    let facing = |(t, w): &([u32; 3], [f32; 3])| {
                        dot(normalize(blend(*t, *w, |v| v.normal)), v.normal)
                    };
                    facing(a).total_cmp(&facing(b))
                })
                .expect("a triangle");
            let colour = blend(t, w, |v| v.colour);
            let normal = normalize(blend(t, w, |v| v.normal));
            // Colours relative to their brightness: countershading runs past 1.
            let tint = (0..3)
                .map(|k| (colour[k] - v.colour[k]).abs() / colour[k].max(v.colour[k]).max(0.1))
                .fold(0.0, f32::max);
            let n = cross(normal, v.normal);
            stray = stray.max(tint).max(dot(n, n).sqrt());
        }
        (far, stray)
    }

    /// The longest edge of the triangles this mesh has and `full` has not:
    /// how coarse the parts it changed are.
    fn longest_new_edge(&self, full: &Mesh) -> f32 {
        let key = |m: &Mesh, t: &[u32; 3]| {
            let mut k = t.map(|i| m.vertices[i as usize].pos.map(|c| (c * 1e4).round() as i64));
            k.sort();
            k
        };
        let old: std::collections::HashSet<_> = full
            .indices
            .as_chunks::<3>()
            .0
            .iter()
            .map(|t| key(full, t))
            .collect();
        let mut longest = 0.0f32;
        for t in self.indices.as_chunks::<3>().0 {
            if old.contains(&key(self, t)) {
                continue;
            }
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let d = sub(self.vertices[a as usize].pos, self.vertices[b as usize].pos);
                longest = longest.max(dot(d, d).sqrt());
            }
        }
        longest
    }

    /// Whether the mesh is closed, every edge shared by exactly two triangles
    /// running it opposite ways, so that from outside its back faces are
    /// always hidden behind its front ones; and if so, whether it is wound
    /// counter-clockwise seen from outside. Fins, sails and fronds are open.
    pub fn closed(&self) -> Option<bool> {
        use std::collections::HashMap;
        // Vertices at the same place are one, whatever their normals.
        let mut welded = HashMap::new();
        let ids: Vec<usize> = self
            .vertices
            .iter()
            .map(|v| {
                let key = v.pos.map(|c| (c * 1e4).round() as i64);
                let next = welded.len();
                *welded.entry(key).or_insert(next)
            })
            .collect();
        let mut edges: HashMap<(usize, usize), u32> = HashMap::new();
        let mut volume = 0.0f64;
        for t in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [0, 1, 2].map(|k| ids[t[k] as usize]);
            // Slivers where two corners meet, as at a sphere's poles, cover
            // nothing.
            if a == b || b == c || c == a {
                continue;
            }
            for edge in [(a, b), (b, c), (c, a)] {
                *edges.entry(edge).or_default() += 1;
            }
            let [p, q, r] = [0, 1, 2].map(|k| self.vertices[t[k] as usize].pos.map(f64::from));
            volume += p[0] * (q[1] * r[2] - q[2] * r[1])
                + p[1] * (q[2] * r[0] - q[0] * r[2])
                + p[2] * (q[0] * r[1] - q[1] * r[0]);
        }
        let closed = !edges.is_empty()
            && edges
                .iter()
                .all(|(&(a, b), &n)| n == 1 && edges.get(&(b, a)) == Some(&1));
        closed.then_some(volume > 0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closed_shapes_are_sealed_and_sheets_are_not() {
        let library = Library::build();
        let sealed = |m: Model| library.sealed[Model::ALL.iter().position(|&n| n == m).unwrap()];
        let names: Vec<String> = Model::ALL
            .iter()
            .filter(|&&m| sealed(m))
            .map(|m| format!("{m:?}"))
            .collect();
        println!("sealed: {}", names.join(" "));
        assert!(sealed(Model::Box), "a box is closed");
        assert!(!sealed(Model::Plane), "a plane has two sides");
    }

    #[test]
    fn every_model_builds_with_sane_normals() {
        let library = Library::build();
        assert_eq!(library.lods.len(), Model::ALL.len());
        for (model, lods) in Model::ALL.iter().zip(&library.lods) {
            println!(
                "{model:?}: {}",
                lods.iter()
                    .map(|l| format!(
                        "{} tris, error {:.3}, stray {:.3}, edge {:.3}",
                        l.count / 3,
                        l.error,
                        l.stray,
                        l.edge
                    ))
                    .collect::<Vec<_>>()
                    .join(" | ")
            );
            assert!(lods[0].error == 0.0, "{model:?}'s full detail is itself");
            for lod in lods {
                let (first, count, base) = (lod.first, lod.count, lod.base);
                assert!(count > 0 && count % 3 == 0, "{model:?} has no triangles");
                for &i in &library.indices[first as usize..(first + count) as usize] {
                    let v = library.vertices[(base as u32 + i) as usize];
                    let n = v.normal;
                    let length = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                    assert!(length.is_finite(), "{model:?} has a broken normal");
                }
            }
        }
    }
}
