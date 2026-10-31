//! The models of the 3D dive, generated when the scene is built.
//!
//! Nothing is loaded from disk: every model comes from a handful of generators —
//! surfaces of revolution, lofted bodies, extrusions and ribbons — so a fish, a
//! fluted column or a galleon's hull is a few dozen lines of numbers. Each
//! vertex carries a colour multiplier, for the countershading of a fish or the
//! planks of a hull, and two animation coordinates the vertex shader bends by:
//! `aux.x` runs along a body from tail (-1) to head (+1), or along an arm from
//! root (0) to tip (1); `aux.y` runs up a plant from root (0) to tip (1). A
//! ray's or a turtle's wings beat as a whale's flukes do, so for them `aux.x`
//! runs from the body (+1) out to the tip of a wing or flipper (-1).
//!
//! Each model is also built coarser, at up to [`LODS`] levels of detail, from
//! the same recipe with half the segments at each step: [`Lod`] records how
//! far each strays from the full model, so the renderer only draws it where
//! the difference is too small to see. `glam` does the vector math and
//! meshoptimizer reorders the finished vertices and triangles for the GPU's
//! fetch and post-transform caches.

use std::cell::Cell;
use std::f32::consts::{PI, TAU};

use glam::Vec3;

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

/// The GPU's vertex record: f32 positions (geometry and the tunnel need
/// them), snorm8 normals, unorm8 colour, f16 aux (it feeds animate(), so
/// 1/255 steps would show on a fish's body). 24 B instead of 44.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Packed {
    pub pos: [f32; 3],
    pub normal: [i8; 4],
    pub colour: [u8; 4],
    pub aux: [u16; 2],
}

impl From<&Vertex> for Packed {
    fn from(v: &Vertex) -> Self {
        Self {
            pos: v.pos,
            normal: [
                (v.normal[0].clamp(-1.0, 1.0) * 127.0).round() as i8,
                (v.normal[1].clamp(-1.0, 1.0) * 127.0).round() as i8,
                (v.normal[2].clamp(-1.0, 1.0) * 127.0).round() as i8,
                0,
            ],
            colour: [
                (v.colour[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                (v.colour[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                (v.colour[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                255,
            ],
            aux: [
                half::f16::from_f32(v.aux[0]).to_bits(),
                half::f16::from_f32(v.aux[1]).to_bits(),
            ],
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

/// The nearest point of triangle `t` to `p`, as barycentric weights of its
/// corners.
fn closest_on_triangle(p: [f32; 3], t: [[f32; 3]; 3]) -> [f32; 3] {
    let p = Vec3::from_array(p);
    let [a, b, c] = t.map(Vec3::from_array);
    let (ab, ac, ap) = (b - a, c - a, p - a);
    let (d1, d2) = (ab.dot(ap), ac.dot(ap));
    if d1 <= 0.0 && d2 <= 0.0 {
        return [1.0, 0.0, 0.0];
    }
    let bp = p - b;
    let (d3, d4) = (ab.dot(bp), ac.dot(bp));
    if d3 >= 0.0 && d4 <= d3 {
        return [0.0, 1.0, 0.0];
    }
    let vc = d1 * d4 - d3 * d2;
    if vc <= 0.0 && d1 >= 0.0 && d3 <= 0.0 {
        let v = d1 / (d1 - d3);
        return [1.0 - v, v, 0.0];
    }
    let cp = p - c;
    let (d5, d6) = (ab.dot(cp), ac.dot(cp));
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
        let mut normals = vec![Vec3::ZERO; self.vertices.len()];
        for t in self.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0] as usize, t[1] as usize, t[2] as usize];
            let n = (Vec3::from_array(self.vertices[b].pos)
                - Vec3::from_array(self.vertices[a].pos))
            .cross(Vec3::from_array(self.vertices[c].pos) - Vec3::from_array(self.vertices[a].pos));
            for i in [a, b, c] {
                normals[i] += n;
            }
        }
        for (v, n) in self.vertices.iter_mut().zip(normals) {
            v.normal = n.normalize_or_zero().to_array();
        }
        self
    }

    /// A triangle with its own vertices and a flat normal, for hard edges.
    fn flat(&mut self, p: [[f32; 3]; 3], colour: [f32; 3], aux: impl Fn([f32; 3]) -> [f32; 2]) {
        let n = (Vec3::from_array(p[1]) - Vec3::from_array(p[0]))
            .cross(Vec3::from_array(p[2]) - Vec3::from_array(p[0]))
            .normalize_or_zero()
            .to_array();
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

    /// A quad with its own vertices and one flat normal across both
    /// triangles: a facet, with a colour at each corner.
    fn facet(&mut self, p: [[f32; 3]; 4], colour: [[f32; 3]; 4], aux: [f32; 2]) {
        let n = (Vec3::from_array(p[2]) - Vec3::from_array(p[0]))
            .cross(Vec3::from_array(p[3]) - Vec3::from_array(p[1]))
            .normalize_or_zero()
            .to_array();
        let base = self.vertices.len() as u32;
        for (pos, colour) in p.into_iter().zip(colour) {
            self.vertices.push(Vertex {
                pos,
                normal: n,
                colour,
                aux,
            });
        }
        self.quad(base, base + 1, base + 2, base + 3);
    }

    /// A triangle with its own vertices, one flat normal and a colour at
    /// each corner.
    fn facet_tri(&mut self, p: [[f32; 3]; 3], colour: [[f32; 3]; 3], aux: [f32; 2]) {
        let n = (Vec3::from_array(p[1]) - Vec3::from_array(p[0]))
            .cross(Vec3::from_array(p[2]) - Vec3::from_array(p[0]))
            .normalize_or_zero()
            .to_array();
        let base = self.vertices.len() as u32;
        for (pos, colour) in p.into_iter().zip(colour) {
            self.vertices.push(Vertex {
                pos,
                normal: n,
                colour,
                aux,
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
            v.normal = Vec3::from_array(g(v.normal)).normalize_or_zero().to_array();
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
    lathe_down_to(profile, segments, 5, radius, colour, aux)
}

/// [`lathe`], coarsening to as few as `least` segments: for thin parts, which
/// still read as round from a little way off with three sides.
fn lathe_down_to(
    profile: &[(f32, f32)],
    segments: usize,
    least: usize,
    radius: impl Fn(f32, f32) -> f32,
    colour: impl Fn(f32, f32) -> [f32; 3],
    aux: impl Fn(f32) -> [f32; 2],
) -> Mesh {
    let segments = fewer(segments, least);
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
    .map(|v| Vec3::from_array(*v).normalize().to_array())
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
                verts.push(
                    (Vec3::from_array(p) + Vec3::from_array(q))
                        .normalize()
                        .to_array(),
                );
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
    Diver,
    DiverHelmet,
    DiverLimb,
    DiverBoot,
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
    Manta,
    Turtle,
    Submarine,
    Screw,
    /// A hero formation of large intergrown crystal points.
    Crystal,
    /// A cluster of crystal points fanning out of one pocket of rock.
    CrystalCluster,
    /// A branching reef coral: a foot of staghorn or elkhorn, its arms
    /// forking up and out, pale at the tips.
    Coral,
    /// A table coral: a broad, wavy-rimmed plate on a short stem.
    CoralTable,
    /// Finger coral: a clump of upright, blunt-tipped fingers.
    CoralFinger,
    /// A clump of sargassum weed: a flattened raft of fronds with tendrils
    /// trailing below it, floating just under the surface.
    Sargassum,
    /// A frilled oral arm hanging from a jellyfish's bell; the tentacles are
    /// [`Model::Tube`]s.
    JellyArm,
    /// A craggy hydrothermal chimney with a hollow, flared mouth. Scaled up,
    /// it is also the far volcano.
    Vent,
    /// A submarine forest tree: a leaning trunk, forking branches and a
    /// clustered canopy, all one model so a tree costs one instance.
    Tree,
    /// A bushy coral: short, stubby branches packed into a rounded head.
    CoralBush,
    /// A brain coral: a low boulder dome wearing meandering grooves.
    CoralBrain,
    /// A sea anemone: a column ringed by curling, swaying tentacles.
    Anemone,
    /// A clump of sea whips: long, thin gorgonian rods.
    SeaWhip,
    /// A jellyfish's fringe of tentacles, all in the bell's frame: one
    /// instance per animal, bent by the shader from the bell's pulse.
    JellyFringe,
    /// A comb jelly: a small ribbed egg of clear jelly whose comb rows the
    /// shader runs with rainbow light.
    Comb,
    /// A tower of the drowned city: square, stepped back twice in brass
    /// cornices, ribbed with piers and crowned with a spire, its walls a grid
    /// of windows (`city::WINDOWS`). One instance a tower.
    Tower,
    /// The drowned city's round tower: a ribbed drum stepped back once, its
    /// crown a lit glass cupola under a needle.
    Rotunda,
    /// A glass dome on a windowed drum, lit from within, with a lantern on
    /// top: the city's halls and gardens.
    Dome,
    /// A glass walkway between two towers: a tube of lit panes ringed with
    /// brass, from y = 0 to 1 with a radius of 1, for `world::between`.
    Walkway,
    /// A run of the city's great pipes, lying along x from -1 to 1 on its
    /// saddles, flanged, with a riser and a valve wheel.
    Pipework,
    /// A neon roundel on an iron plate, facing +z: a ring with a bar
    /// through it, in the instance's colour.
    Sign,
    /// A stepped art-deco pyramid: four windowed tiers behind brass cornices,
    /// neon at its corners and a beacon on its spire.
    Ziggurat,
    /// A low, wide arcade: two floors of windows, a colonnade along both
    /// faces, a lit glass canopy and a neon marquee above it.
    Arcade,
    /// A riveted works hall with a sawtooth roof of lit skylights, two
    /// smokestacks, pipes and a glowing furnace door.
    Factory,
    /// A round picture palace: a ribbed drum, neon fins round its lower
    /// wall, a projecting glass marquee ringed in light, and a lit cupola.
    Theatre,
}

impl Model {
    pub const ALL: [Model; 64] = [
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
        Model::Diver,
        Model::DiverHelmet,
        Model::DiverLimb,
        Model::DiverBoot,
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
        Model::Manta,
        Model::Turtle,
        Model::Submarine,
        Model::Screw,
        Model::Crystal,
        Model::CrystalCluster,
        Model::Coral,
        Model::CoralTable,
        Model::CoralFinger,
        Model::Sargassum,
        Model::JellyArm,
        Model::Vent,
        Model::Tree,
        Model::CoralBush,
        Model::CoralBrain,
        Model::Anemone,
        Model::SeaWhip,
        Model::JellyFringe,
        Model::Comb,
        Model::Tower,
        Model::Rotunda,
        Model::Dome,
        Model::Walkway,
        Model::Pipework,
        Model::Sign,
        Model::Ziggurat,
        Model::Arcade,
        Model::Factory,
        Model::Theatre,
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
            Model::Diver => diver_body(),
            Model::DiverHelmet => diver_helmet(),
            Model::DiverLimb => diver_limb(),
            Model::DiverBoot => diver_boot(),
            Model::Fish => fish(),
            Model::Shark => shark(),
            Model::Whale => whale(),
            Model::Dolphin => dolphin(false),
            Model::Narwhal => dolphin(true),
            Model::Penguin => penguin(),
            Model::Mantle => mantle(),
            Model::Bell => {
                // A medusa bell: a domed crown, a slight waist and a margin
                // flared into sixteen lappets, the rim the shader lights. The
                // inner shell, the subumbrella, gives the additive glass two
                // layers to glow through; the stomach and gonads are painted
                // by `fs_glass` from the local position rather than modelled,
                // which pays for the lappets' finer ring. `aux.y` runs from
                // the margin (0) to the crown (1): the pulse squeezes the
                // margin hardest (`animate`, kind 5).
                let profile = [
                    (0.97, 0.000),
                    (1.00, 0.035),
                    (0.975, 0.100),
                    (0.925, 0.190),
                    (0.85, 0.290),
                    (0.75, 0.400),
                    (0.62, 0.510),
                    (0.47, 0.610),
                    (0.31, 0.700),
                    (0.15, 0.770),
                    (0.00, 0.800),
                ];
                let radius = |y: f32, a: f32| {
                    let rim = (1.0 - y / 0.10).clamp(0.0, 1.0);
                    1.0 + 0.045 * (a * 16.0).cos() * rim
                };
                let colour = |y: f32, _: f32| {
                    let rim = (1.0 - y / 0.12).clamp(0.0, 1.0);
                    [1.0, 1.0 - 0.06 * rim, 1.0 - 0.02 * rim]
                };
                let mut m = lathe(&profile, 32, radius, colour, |_| [0.0, 0.0]);
                // The subumbrella, a little smaller and lifted, a shade
                // warmer, so the rim reads as two translucent layers.
                m.append(
                    lathe(
                        &profile,
                        16,
                        |_, _| 1.0,
                        |_, _| [1.0, 0.86, 0.90],
                        |_| [0.0, 0.0],
                    )
                    .scaled([0.86, 0.80, 0.86])
                    .map(|p| [p[0], p[1] + 0.04, p[2]], |n| n),
                );
                m.with_aux(|p| [0.0, (p[1] / 0.80).clamp(0.0, 1.0)])
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
            Model::Manta => manta(),
            Model::Turtle => turtle(),
            Model::Submarine => submarine(),
            Model::Screw => screw(),
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
            Model::Crystal => crystal(),
            Model::CrystalCluster => crystal_cluster(),
            Model::Coral => coral(),
            Model::CoralTable => coral_table(),
            Model::CoralFinger => coral_finger(),
            Model::Sargassum => sargassum(),
            Model::JellyArm => jelly_arm(),
            Model::Vent => vent(),
            Model::Tree => tree(),
            Model::CoralBush => coral_bush(),
            Model::CoralBrain => coral_brain(),
            Model::Anemone => anemone(),
            Model::SeaWhip => sea_whip(),
            Model::JellyFringe => jelly_fringe(),
            Model::Comb => comb(),
            Model::Tower => tower(),
            Model::Rotunda => rotunda(),
            Model::Dome => dome(),
            Model::Walkway => walkway(),
            Model::Pipework => pipework(),
            Model::Sign => sign(),
            Model::Ziggurat => ziggurat(),
            Model::Arcade => arcade(),
            Model::Factory => factory(),
            Model::Theatre => theatre(),
        }
    }
}

/// A crystal point from `base` to `tip`, `radius` thick at its foot: a
/// tapered, ridged prism with a pyramidal termination, its facets given their
/// own flat normals and their own colours. The colours run from a deep violet
/// foot through a saturated middle to a warm white tip, with each facet a
/// little brighter or cooler than its neighbours, so the additive glass
/// shader gets its facets from the vertices alone.
#[allow(clippy::too_many_arguments)]
fn crystal_point(
    m: &mut Mesh,
    base: [f32; 3],
    tip: [f32; 3],
    radius: f32,
    phase: f32,
    foot: [f32; 3],
    crown: [f32; 3],
    seed: u32,
) {
    // Twelve sides with every other one pulled in: six broad faces, each
    // broken by a ridge down its length so a big point never reads as a flat
    // oval. Coarser, they halve to eight.
    let sides = fewer(12, 8);
    let axis = Vec3::from_array(tip) - Vec3::from_array(base);
    let length = axis.length().max(1e-4);
    let up = axis / length;
    let reference = if up.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let across = up.cross(reference).normalize_or_zero();
    let through = across.cross(up);
    let mid = [
        (foot[0] * 1.8 + 0.12).min(1.0),
        foot[1] * 1.8 + 0.05,
        (foot[2] * 0.7 + 0.55).min(1.0),
    ];
    let shade = |t: f32, facet: usize| {
        let t = t.clamp(0.0, 1.0);
        let body = if t < 0.55 {
            mix3(foot, mid, t / 0.55)
        } else {
            mix3(
                mid,
                crown,
                ((t - 0.55) / 0.45).powi(2) * 0.75 + (t - 0.55) / 0.45 * 0.25,
            )
        };
        // Alternate facets catch the light differently: the refraction a real
        // crystal shows even where nothing else lights it.
        let grain = jitter(seed, 30 + facet as u32);
        let gain = 0.78 + 0.36 * grain + 0.07 * (facet as f32 * 2.4 + seed as f32).sin();
        let warm = (grain - 0.5) * 0.10;
        [
            (body[0] * gain + warm).clamp(0.0, 1.0),
            (body[1] * gain).clamp(0.0, 1.0),
            (body[2] * gain - warm * 0.5).clamp(0.0, 1.0),
        ]
    };
    let angle = |s: usize| phase + s as f32 / sides as f32 * TAU;
    let ridge = |s: usize| 1.0 - 0.07 * (s % 2) as f32 - 0.04 * jitter(seed, 40 + s as u32);
    // The point wanders a little off its axis as it grows, so its broad faces
    // break into bands of different normals instead of one long, flat plane.
    let wander = |t: f32| {
        (across * (t * 3.7 + seed as f32 * 0.6).sin()
            + through * (t * 4.3 + seed as f32 * 0.7).cos())
            * (radius * 0.09)
    };
    let ring = |t: f32, r: f32, s: usize| {
        let c = Vec3::from_array(base) + up * (length * t) + wander(t);
        let a = angle(s);
        let rr = radius * r * ridge(s);
        (c + (across * a.cos() + through * a.sin()) * rr).to_array()
    };
    // A foot a little inside the rock, growth rings along the body with a
    // bulge midway, then the point.
    let lower = -0.10;
    let top = 0.74 + 0.10 * jitter(seed, 6);
    let top_r = 0.78 + 0.06 * jitter(seed, 7);
    let rings = [
        (lower, 1.0),
        (0.10 + 0.06 * jitter(seed, 1), 1.0),
        (0.28 + 0.06 * jitter(seed, 2), 1.0),
        (0.46 + 0.06 * jitter(seed, 3), 0.98),
        (0.62 + 0.06 * jitter(seed, 4), 0.94),
        (top, top_r),
    ];
    for s in 0..sides {
        let t = (s + 1) % sides;
        for w in rings.windows(2) {
            let (t0, r0) = w[0];
            let (t1, r1) = w[1];
            let p = [
                ring(t0, r0, s),
                ring(t1, r1, s),
                ring(t1, r1, t),
                ring(t0, r0, t),
            ];
            let c = [shade(t0, s), shade(t1, s), shade(t1, t), shade(t0, t)];
            m.facet(p, c, [0.0, t1]);
        }
    }
    // The termination: a spire a little off the axis, so no two points end
    // alike.
    let lean = radius * 0.22;
    let point = Vec3::from_array(tip)
        + across * ((seed as f32 * 0.7 + phase).cos() * lean)
        + through * ((seed as f32 * 0.7 + phase).sin() * lean);
    for s in 0..sides {
        let t = (s + 1) % sides;
        let p = [ring(top, top_r, s), ring(top, top_r, t), point.to_array()];
        let c = [shade(top, s), shade(top, t), crown];
        m.facet_tri(p, c, [1.0, 1.0]);
    }
    // A cap buried in the rock, so the point is a closed solid.
    let centre = (Vec3::from_array(base) + up * (length * lower) + wander(lower)).to_array();
    for s in 0..sides {
        let t = (s + 1) % sides;
        let p = [centre, ring(lower, 1.0, t), ring(lower, 1.0, s)];
        m.facet_tri(p, [foot, foot, foot], [0.0, 0.0]);
    }
}

/// The gallery's hero: a large point intergrown with three smaller ones, the
/// tallest near the middle, so the formation reads as crystals rather than as
/// one broad face.
fn crystal() -> Mesh {
    let mut m = Mesh::default();
    let foot = [0.12, 0.035, 0.34];
    let crown = [1.0, 0.92, 0.78];
    crystal_point(
        &mut m,
        [0.0, -0.03, 0.0],
        [0.06, 0.97, -0.06],
        0.62,
        0.35,
        foot,
        crown,
        7,
    );
    crystal_point(
        &mut m,
        [0.32, -0.02, 0.12],
        [0.82, 0.60, 0.32],
        0.42,
        1.15,
        [0.10, 0.03, 0.30],
        [0.86, 0.82, 0.98],
        8,
    );
    crystal_point(
        &mut m,
        [-0.30, -0.02, 0.14],
        [-0.66, 0.48, 0.34],
        0.30,
        2.05,
        [0.09, 0.03, 0.26],
        [0.82, 0.80, 1.0],
        9,
    );
    crystal_point(
        &mut m,
        [0.08, -0.02, 0.24],
        [0.20, 0.34, 0.58],
        0.20,
        2.8,
        [0.08, 0.025, 0.24],
        [0.80, 0.80, 1.0],
        10,
    );
    m
}

/// The gallery's satellites: a handful of points of different sizes and
/// bearings growing from one pocket, the tallest near the middle.
fn crystal_cluster() -> Mesh {
    // A point's base, tip and thickness, its facet phase and seed, and the
    // colour of its termination.
    type Point = ([f32; 3], [f32; 3], f32, f32, u32, [f32; 3]);
    let mut m = Mesh::default();
    let foot = [0.07, 0.025, 0.22];
    let points: [Point; 7] = [
        (
            [0.0, -0.06, 0.0],
            [0.05, 0.94, 0.08],
            0.30,
            0.4,
            21,
            [1.0, 0.94, 0.86],
        ),
        (
            [-0.18, 0.0, 0.04],
            [-0.54, 0.50, -0.06],
            0.17,
            0.9,
            22,
            [0.86, 0.84, 1.0],
        ),
        (
            [0.17, 0.0, 0.05],
            [0.52, 0.66, 0.18],
            0.20,
            1.5,
            23,
            [1.0, 0.90, 0.82],
        ),
        (
            [0.01, 0.0, 0.17],
            [0.16, 0.40, 0.56],
            0.13,
            2.1,
            24,
            [0.80, 0.80, 1.0],
        ),
        (
            [-0.09, 0.0, -0.16],
            [-0.22, 0.34, -0.52],
            0.12,
            2.7,
            25,
            [0.84, 0.84, 1.0],
        ),
        (
            [0.25, 0.0, -0.12],
            [0.40, 0.32, 0.10],
            0.10,
            3.3,
            26,
            [0.90, 0.86, 1.0],
        ),
        (
            [-0.27, 0.0, -0.09],
            [-0.34, 0.25, 0.28],
            0.09,
            3.9,
            27,
            [0.84, 0.82, 1.0],
        ),
    ];
    for (base, tip, radius, phase, seed, crown) in points {
        crystal_point(&mut m, base, tip, radius, phase, foot, crown, seed);
    }
    m
}

/// Blend two colours.
fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
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
        8,
        7,
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

/// A tapered rod from `a` to `b`, closed at both ends: hoses, pipes and
/// valves. The radius is `r0` at `a` and `r1` at `b`.
fn rod(a: [f32; 3], b: [f32; 3], r0: f32, r1: f32, colour: [f32; 3]) -> Mesh {
    let d = Vec3::from_array(b) - Vec3::from_array(a);
    let length = d.length().max(1e-5);
    let y = d / length;
    let reference = if y.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let x = y.cross(reference).normalize_or_zero();
    let z = x.cross(y);
    lathe(
        &[(0.0, 0.0), (r0, 0.0), (r1, 1.0), (0.0, 1.0)],
        6,
        |_, _| 1.0,
        move |_, _| colour,
        |_| [0.0; 2],
    )
    .map(
        move |p| {
            let q = x * p[0] + y * (p[1] * length) + z * p[2];
            [a[0] + q.x, a[1] + q.y, a[2] + q.z]
        },
        move |n| {
            let q = x * n[0] + y * n[1] + z * n[2];
            [q.x, q.y, q.z]
        },
    )
}

/// One knot of a [`tube`]: where its centre is, how thick it is there, its
/// colour and its animation coordinates.
#[derive(Clone, Copy)]
struct Knot {
    at: Vec3,
    radius: f32,
    colour: [f32; 3],
    aux: [f32; 2],
}

/// A tube through `knots`, closed at its first and rounded off just past its
/// last, its rings turned along the path by parallel transport so a curved
/// limb does not twist: a coral's branch, an anemone's tentacle, a sea whip.
/// `segments` round, coarsening to `least`. Each ring is shared by the two
/// spans either side of it, so a bent limb is one smooth surface rather than
/// straight pieces meeting at a crease.
fn tube(knots: &[Knot], segments: usize, least: usize) -> Mesh {
    let segments = fewer(segments, least);
    let mut m = Mesh::default();
    let n = knots.len();
    let along = |i: usize| {
        let a = knots[i.saturating_sub(1)].at;
        let b = knots[(i + 1).min(n - 1)].at;
        (b - a).normalize_or(Vec3::Y)
    };
    let first = along(0);
    let reference = if first.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };
    let mut x = first.cross(reference).normalize();
    let ring = |m: &mut Mesh, centre: Vec3, t: Vec3, x: Vec3, r: f32, k: &Knot| {
        let z = x.cross(t);
        (0..segments)
            .map(|s| {
                let a = s as f32 / segments as f32 * TAU;
                let p = centre + (x * a.cos() + z * a.sin()) * r;
                m.vertex(p.to_array(), k.colour, k.aux)
            })
            .collect::<Vec<u32>>()
    };
    let foot = m.vertex(knots[0].at.to_array(), knots[0].colour, knots[0].aux);
    let mut rings = Vec::with_capacity(n + 1);
    let mut t = first;
    for (i, k) in knots.iter().enumerate() {
        t = along(i);
        x = (x - t * x.dot(t)).normalize_or(x);
        rings.push(ring(&mut m, k.at, t, x, k.radius, k));
    }
    // The rounded end: one narrower ring a little past the last knot, then
    // its pole.
    let last = knots[n - 1];
    rings.push(ring(
        &mut m,
        last.at + t * last.radius * 0.55,
        t,
        x,
        last.radius * 0.8,
        &last,
    ));
    let tip = m.vertex(
        (last.at + t * last.radius * 1.05).to_array(),
        last.colour,
        last.aux,
    );
    for s in 0..segments {
        let u = (s + 1) % segments;
        m.tri(foot, rings[0][s], rings[0][u]);
        for w in rings.windows(2) {
            m.quad(w[0][s], w[1][s], w[1][u], w[0][u]);
        }
        let end = &rings[rings.len() - 1];
        m.tri(end[s], tip, end[u]);
    }
    m.smooth()
}

/// How a branching coral grows: how many trunks leave its foot, how often
/// and how widely they fork, how they taper and how hard they turn up
/// toward the light.
struct Habit {
    /// Trunks out of the foot.
    trunks: usize,
    /// Forks below each trunk at full detail.
    depth: u32,
    /// Children at each fork.
    forks: usize,
    /// The chance a limb forks at all, so no two colonies' crowns match.
    fork_chance: f32,
    /// A trunk's length, and each generation's length as a share of its
    /// parent's.
    length: f32,
    shrink: f32,
    /// A trunk's radius at its foot, and the share it keeps to its end.
    radius: f32,
    taper: f32,
    /// How far the trunks lean out from upright, and the children from their
    /// parent, in radians.
    spread: f32,
    fork_angle: f32,
    /// How much each limb bends up toward the vertical along its length.
    rise: f32,
    /// Sides round a trunk, and round a twig at the tips.
    sides: usize,
    twig_sides: usize,
    salt: u32,
}

/// Grow one limb of a branching coral from `start` along `dir`, and its
/// children after it. Its colour runs from the colony's shaded `foot` to its
/// pale growing tips, each tip a little different. Coarser, the forks go
/// from the tips inward and each limb that no longer forks reaches as far as
/// its children would have, so the colony keeps its size and outline.
#[allow(clippy::too_many_arguments)]
fn grow(
    m: &mut Mesh,
    habit: &Habit,
    start: Vec3,
    dir: Vec3,
    length: f32,
    radius: f32,
    level: u32,
    foot: [f32; 3],
    count: &mut u32,
) {
    *count += 1;
    let id = *count;
    let random = |salt: u32| jitter(id, habit.salt.wrapping_add(salt));
    // The first step coarser keeps every fork and loses only sides; the
    // next loses the last generation of twigs.
    let deepest = habit.depth.saturating_sub(COARSER.get().saturating_sub(1));
    let forks = level < deepest && random(1) < habit.fork_chance;
    // A limb that stops forking early reaches as far as the generations it
    // stands for, a little short of the full sum so the crown stays round.
    let mut reach = length;
    if !forks {
        let mut child = length;
        for _ in level..habit.depth {
            child *= habit.shrink;
            reach += child * 0.8;
        }
    }
    let knots = if COARSER.get() >= 2 || level == habit.depth {
        2
    } else {
        3
    };
    let generations = (habit.depth + 1) as f32;
    // The colour of the growing tip: pale, and warmer or cooler by a shade.
    let shade = 0.88 + 0.12 * random(2);
    let tip = [
        shade,
        shade * (0.94 + 0.06 * random(3)),
        shade * (0.92 + 0.10 * random(4)),
    ];
    let mut path = Vec::with_capacity(knots);
    let mut at = start;
    let mut heading = dir;
    let wobble = Vec3::new(random(5) - 0.5, 0.0, random(6) - 0.5) * 0.25;
    let end_radius = radius
        * if forks {
            habit.taper
        } else {
            habit.taper * 0.8
        };
    for k in 0..knots {
        let f = k as f32 / (knots - 1) as f32;
        let age = ((level as f32 + f * reach / length) / generations).min(1.0);
        let pale = smooth(0.35, 1.0, age);
        let colour = [0, 1, 2].map(|c| foot[c] + (tip[c] - foot[c]) * pale);
        path.push(Knot {
            at,
            radius: radius + (end_radius - radius) * f,
            colour,
            aux: [0.0; 2],
        });
        heading = (heading + (Vec3::Y * habit.rise + wobble) / (knots - 1) as f32).normalize();
        at += heading * reach / (knots - 1) as f32;
    }
    let sides = if level == 0 {
        habit.sides
    } else {
        habit.twig_sides
    };
    m.append(tube(&path, sides, 3));
    if !forks {
        return;
    }
    let end = path[knots - 1].at;
    let out = (path[knots - 1].at - path[knots - 2].at).normalize();
    let side = out.cross(Vec3::Y).normalize_or(Vec3::X);
    for c in 0..habit.forks {
        // Children turn away from their parent's line about axes spaced
        // round it, the first set toward the outside of the colony.
        let round = c as f32 / habit.forks as f32 * TAU + random(7 + c as u32) * 0.8;
        let axis = glam::Quat::from_axis_angle(out, round) * side;
        let bend = habit.fork_angle * (0.75 + 0.5 * random(11 + c as u32));
        let child = glam::Quat::from_axis_angle(axis, bend) * out;
        grow(
            m,
            habit,
            end - out * end_radius * 0.6,
            child,
            length * habit.shrink * (0.85 + 0.3 * random(15 + c as u32)),
            end_radius * 0.9,
            level + 1,
            foot,
            count,
        );
    }
}

/// A coral colony of `habit` on a low, lumpy crust of its own dead skeleton,
/// which the shader beds into the sand.
fn colony(habit: &Habit, foot: [f32; 3], crust: [f32; 3]) -> Mesh {
    let salt = habit.salt;
    let mut m = sphere_with(
        1,
        move |p| 1.0 + 0.18 * (p[0] * 6.0 + salt as f32).sin() * (p[2] * 5.0).cos(),
        move |_| crust,
    )
    .scaled([0.12, 0.04, 0.11]);
    let mut count = 0;
    for k in 0..habit.trunks {
        let a = k as f32 / habit.trunks as f32 * TAU + jitter(k as u32, salt) * 0.9;
        let lean = habit.spread * (0.6 + 0.6 * jitter(k as u32, salt + 1));
        // The central trunks stand straighter.
        let lean = if k == 0 { lean * 0.3 } else { lean };
        let dir = Vec3::new(a.cos() * lean.sin(), lean.cos(), a.sin() * lean.sin());
        let from = Vec3::new(a.cos() * 0.04, 0.0, a.sin() * 0.04);
        let length = habit.length * (0.8 + 0.4 * jitter(k as u32, salt + 2));
        grow(
            &mut m,
            habit,
            from,
            dir,
            length,
            habit.radius,
            0,
            foot,
            &mut count,
        );
    }
    let top = m.vertices.iter().map(|v| v.pos[1]).fold(0.1, f32::max);
    m.with_aux(move |p| [0.0, (p[1] / top).clamp(0.0, 1.0)])
}

/// A branching reef coral, staghorn-like: a handful of trunks leaning out
/// of a low crust and forking twice into tapering, upturned antlers, pale at
/// every tip. About half a unit tall and as wide.
fn coral() -> Mesh {
    colony(
        &Habit {
            trunks: 5,
            depth: 2,
            forks: 2,
            fork_chance: 0.9,
            length: 0.20,
            shrink: 0.74,
            radius: 0.036,
            taper: 0.72,
            spread: 0.85,
            fork_angle: 0.60,
            rise: 0.16,
            sides: 6,
            twig_sides: 5,
            salt: 17,
        },
        [0.62, 0.55, 0.52],
        [0.55, 0.52, 0.48],
    )
}

/// A bushy coral, like cauliflower or bird's-nest coral: many short, stubby
/// branches packed into a rounded head, each forking into three blunt
/// twigs. Lower and denser than [`coral`], so a thicket of both reads as
/// more than one species.
fn coral_bush() -> Mesh {
    colony(
        &Habit {
            trunks: 7,
            depth: 1,
            forks: 3,
            fork_chance: 0.9,
            length: 0.16,
            shrink: 0.75,
            radius: 0.042,
            taper: 0.85,
            spread: 0.85,
            fork_angle: 0.50,
            rise: 0.35,
            sides: 5,
            twig_sides: 4,
            salt: 41,
        },
        [0.66, 0.58, 0.55],
        [0.58, 0.55, 0.50],
    )
}

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A table coral: a plate of acropora spread flat on a short stout stem,
/// its rim lobed, wavy and pale with new growth, its underside in shade and
/// ribbed with the radial branches it grew from. One unit across and about
/// two thirds of a unit tall.
fn coral_table() -> Mesh {
    // Cupped: the plate rises to its rim, so it is never a bare line
    // seen edge on.
    let profile = [
        (0.0, 0.0),
        (0.18, 0.0),
        (0.11, 0.20),
        (0.12, 0.38),
        (0.30, 0.47),
        (0.62, 0.53),
        (1.0, 0.66),
        (0.97, 0.72),
        (0.62, 0.62),
        (0.28, 0.56),
        (0.0, 0.54),
    ];
    lathe(
        &profile,
        28,
        |y, a| {
            // Lobes of every size, so the outline is no regular polygon.
            let wave = 0.10 * (a * 3.0 + 0.7).sin()
                + 0.07 * (a * 7.0 + 1.3).sin()
                + 0.035 * (a * 13.0 + 2.1).sin();
            1.0 + wave * smooth(0.3, 0.6, y)
        },
        |y, a| {
            let rim = smooth(0.62, 0.69, y) * (1.0 - smooth(0.70, 0.73, y));
            let ribs = 0.5 + 0.5 * (a * 31.0).sin();
            let v = 0.72 + 0.10 * ribs;
            let under = if y < 0.55 { 0.66 + 0.14 * ribs } else { 1.0 };
            let stem = 1.0 - smooth(0.25, 0.45, y);
            let top = [v * under, v * under * 0.95, v * under * 0.90];
            let top = [0, 1, 2].map(|c| top[c] * (1.0 - 0.25 * stem));
            [0, 1, 2].map(|c| top[c] + (1.0 - top[c]) * rim * 0.8)
        },
        |_| [0.0; 2],
    )
}

/// Finger coral: a clump of upright, blunt fingers, some forked once, out of
/// one low crust. Coarser, the forks and then a few fingers go.
fn coral_finger() -> Mesh {
    colony(
        &Habit {
            trunks: if COARSER.get() >= 2 { 6 } else { 9 },
            depth: 1,
            forks: 2,
            fork_chance: 0.35,
            length: 0.30,
            shrink: 0.55,
            radius: 0.048,
            taper: 0.92,
            spread: 0.38,
            fork_angle: 0.38,
            rise: 0.45,
            sides: 6,
            twig_sides: 5,
            salt: 63,
        },
        [0.70, 0.64, 0.60],
        [0.60, 0.56, 0.52],
    )
}

/// A brain coral: a low, lumpy boulder dome on a flat foot, a unit across
/// and three quarters of one tall. Its meandering
/// ridges and grooves are the [`Surface::Brain`] texture's, which the model
/// carries; the mesh adds the boulder's own swell and a shaded skirt.
fn coral_brain() -> Mesh {
    let mut m = sphere_with(
        3,
        |p| {
            1.0 + 0.07 * (p[0] * 4.0 + 1.0).sin() * (p[2] * 3.0).cos()
                + 0.04 * (p[1] * 5.0 + p[0] * 2.0).sin()
        },
        |p| {
            // In shade toward its foot.
            let v = 0.70 + 0.30 * smooth(-0.6, 0.5, p[1]);
            [v, v * 0.97, v * 0.94]
        },
    );
    // Flattened underneath, where it sits on the reef.
    for v in &mut m.vertices {
        if v.pos[1] < 0.0 {
            v.pos[1] *= 0.2;
        }
        v.pos[1] = v.pos[1] * 0.65 + 0.1;
        v.aux = [0.0, 0.0];
    }
    m.smooth()
}

/// A sea anemone: a short column under a disc, ringed by tentacles that curl
/// out and up, pale and bright at their tips. The tentacles carry `aux.y`
/// from root to tip, so the sway waves them and leaves the column still.
/// Coarser, the inner ring goes and then half the outer.
fn anemone() -> Mesh {
    let coarse = COARSER.get();
    let mut m = lathe(
        &[
            (0.0, 0.0),
            (0.13, 0.0),
            (0.11, 0.10),
            (0.13, 0.18),
            (0.17, 0.22),
            (0.0, 0.23),
        ],
        10,
        |_, _| 1.0,
        |y, _| {
            let v = 0.55 + 0.25 * smooth(0.0, 0.2, y);
            [v, v * 0.92, v * 0.9]
        },
        |_| [0.0; 2],
    );
    let rings: &[(usize, f32, f32, f32)] = match coarse {
        0 => &[(16, 0.15, 0.30, 0.95), (9, 0.08, 0.24, 0.35)],
        1 => &[(14, 0.15, 0.30, 0.90)],
        _ => &[(9, 0.15, 0.30, 0.90)],
    };
    for (r, &(count, from, length, lean)) in rings.iter().enumerate() {
        for k in 0..count {
            let i = (r * 32 + k) as u32;
            let a = (k as f32 + 0.5 * r as f32) / count as f32 * TAU + 0.2 * jitter(i, 5);
            let out = Vec3::new(a.cos(), 0.0, a.sin());
            let length = length * (0.8 + 0.4 * jitter(i, 6));
            let root = out * from + Vec3::Y * 0.22;
            // Out over the rim, then curling back up.
            let mid = root + (out * lean.sin() + Vec3::Y * lean.cos()) * length * 0.5;
            let end = mid + (out * 0.45 + Vec3::Y).normalize() * length * 0.5;
            let knots: Vec<Knot> = if coarse >= 2 {
                vec![(root, 0.0), (end, 1.0)]
            } else {
                vec![(root, 0.0), (mid, 0.5), (end, 1.0)]
            }
            .into_iter()
            .map(|(at, f)| {
                let pale = smooth(0.3, 1.0, f);
                let tip = 0.9 + 0.1 * jitter(i, 7);
                Knot {
                    at,
                    radius: 0.024 * (1.0 - 0.45 * f),
                    colour: [
                        0.78 + (tip - 0.78) * pale,
                        0.74 + (tip - 0.74) * pale,
                        0.74 + (tip - 0.74) * pale,
                    ],
                    aux: [0.0, 0.25 + 0.75 * f],
                }
            })
            .collect();
            m.append(tube(&knots, 4, 3));
        }
    }
    m
}

/// A clump of sea whips: long, thin gorgonian rods rising from one holdfast,
/// each gently curved and swaying from the top. Six of them, four coarser.
fn sea_whip() -> Mesh {
    let coarse = COARSER.get();
    let mut m = sphere_with(0, |_| 1.0, |_| [0.55, 0.50, 0.48]).scaled([0.05, 0.03, 0.05]);
    let whips = if coarse >= 2 { 4 } else { 6 };
    let knots = if coarse >= 1 { 3 } else { 5 };
    for k in 0..whips {
        let i = k as u32;
        let a = k as f32 / whips as f32 * TAU + jitter(i, 21);
        let lean = 0.10 + 0.30 * jitter(i, 22);
        let height = 0.65 + 0.35 * jitter(i, 23);
        let out = Vec3::new(a.cos(), 0.0, a.sin());
        let path: Vec<Knot> = (0..knots)
            .map(|j| {
                let f = j as f32 / (knots - 1) as f32;
                // Leaning out more toward the top, in a soft curve.
                let at = out * (0.02 + lean * f * f * height) + Vec3::Y * height * f;
                let v = 0.72 + 0.28 * smooth(0.2, 1.0, f);
                Knot {
                    at,
                    radius: 0.016 * (1.0 - 0.5 * f),
                    colour: [v, v * 0.96, v * 0.94],
                    aux: [0.0, f],
                }
            })
            .collect();
        m.append(tube(&path, 4, 3));
    }
    m
}

/// A clump of sargassum weed: a flattened, lobed raft of fronds with tendrils
/// trailing below it. It floats just under the surface, so its underside is
/// the side the window sees.
fn sargassum() -> Mesh {
    let weed = [0.72, 0.62, 0.28];
    let pale = [0.90, 0.82, 0.52];
    let mut m = sphere_with(
        2,
        |p| {
            let lobe = 1.0 + 0.22 * ((p[0] * 7.3).sin() * (p[2] * 6.1).cos());
            let underside = if p[1] < -0.1 { 0.75 } else { 1.0 };
            lobe * underside
        },
        |p| {
            if p[1] < -0.2 {
                [0.42, 0.36, 0.16]
            } else {
                [0.72, 0.62, 0.28]
            }
        },
    )
    .scaled([0.72, 0.16, 0.58]);
    for k in 0..10 {
        let a = k as f32 / 10.0 * TAU;
        let r = 0.12 + 0.34 * (k as f32 * 2.7).sin().mul_add(0.5, 0.5);
        let root = [a.cos() * r, -0.01, a.sin() * r];
        let len = 0.15 + 0.30 * (k as f32 * 1.3).cos().mul_add(0.5, 0.5);
        let mid = [
            root[0] + 0.05 * (a * 3.0).sin(),
            -len * 0.5,
            root[2] + 0.05 * (a * 2.0).cos(),
        ];
        let tip = [
            root[0] + 0.10 * (a * 2.0).sin(),
            -len,
            root[2] + 0.10 * (a * 3.0).cos(),
        ];
        m.append(rod(root, mid, 0.012, 0.008, weed));
        m.append(rod(mid, tip, 0.008, 0.002, pale));
    }
    m.with_aux(|_p| [0.0, 0.5])
}

/// A jellyfish's four frilled oral arms in one model, hanging from the mouth
/// under the bell in the bell's own frame, so one instance placed like the
/// bell carries all four and the shader bends them with it. Each arm is a
/// ribbon turned edge-on to the bell's axis, tapering and twisting as it
/// falls, its two edges ruffled into frills that the middle of the ribbon
/// does not share. `aux.x` runs from the root (0) to the tip (1): the
/// trailing wave (`animate`, kind 10) grows and lags down it.
fn jelly_arm() -> Mesh {
    let segments = fewer(14, 5);
    let across = 3;
    let mut m = Mesh::default();
    for k in 0..4u32 {
        let ang = (k as f32 + 0.5) / 4.0 * TAU;
        let (out, side) = ([ang.cos(), 0.0, ang.sin()], [-ang.sin(), 0.0, ang.cos()]);
        let len = 1.05 + 0.30 * jitter(k, 41);
        let mut rows = Vec::with_capacity(segments + 1);
        for i in 0..=segments {
            let t = i as f32 / segments as f32;
            // The arm's spine leaves the mouth and leans out a little.
            let r = 0.07 + 0.16 * t;
            let centre = [out[0] * r, 0.10 - t * len, out[2] * r];
            let w = 0.21 * (1.0 - 0.70 * t) * (0.80 + 0.20 * (t * 9.0 + k as f32).sin());
            // A slow twist down the arm: the ribbon's breadth turns from
            // the radial plane toward the tangent.
            let twist = t * 1.6 + k as f32 * 0.4;
            let (c, sn) = (twist.cos(), twist.sin());
            let breadth = [out[0] * c + side[0] * sn, 0.0, out[2] * c + side[2] * sn];
            let normal = [-out[0] * sn + side[0] * c, 0.0, -out[2] * sn + side[2] * c];
            let mut row = Vec::with_capacity(across + 1);
            for j in 0..=across {
                let u = j as f32 / across as f32 * 2.0 - 1.0;
                // The frill: the edges ruffle out of the ribbon's plane,
                // ever tighter toward the tip.
                let frill = 0.085
                    * (t * (20.0 + 6.0 * t) + u * 1.7 + k as f32).sin()
                    * u.abs().powf(1.5)
                    * (0.5 + t);
                let x = u * w;
                let pos = [
                    centre[0] + breadth[0] * x + normal[0] * frill,
                    centre[1] + 0.02 * u.abs() * (t * 13.0).sin(),
                    centre[2] + breadth[2] * x + normal[2] * frill,
                ];
                let pale = 1.0 - 0.18 * (1.0 - u.abs());
                row.push(m.vertex(pos, [pale, pale, 1.0], [t, 0.0]));
            }
            rows.push(row);
        }
        for i in 0..segments {
            for j in 0..across {
                m.quad(
                    rows[i][j],
                    rows[i][j + 1],
                    rows[i + 1][j + 1],
                    rows[i + 1][j],
                );
            }
        }
    }
    m.smooth()
}

/// A jellyfish's fringe of tentacles in one model: twelve fine threads
/// hanging from just inside the bell's margin, in the bell's frame, every
/// other one shorter. Placed like the bell with its height stretched to the
/// species' tentacle length, it costs one instance where a dozen tubes did,
/// and the shader can bend every thread from the bell's pulse: the margin
/// pulls the roots in and the stroke runs down them. Three-sided, which
/// reads as round at a thread's width. `aux.x` is how far down the longest
/// thread a vertex hangs, so the wave keeps step across long and short.
fn jelly_fringe() -> Mesh {
    const THREADS: u32 = 12;
    let segments = fewer(10, 3);
    let sides = 3;
    let mut m = Mesh::default();
    for k in 0..THREADS {
        let ang = (k as f32 + 0.5) / THREADS as f32 * TAU + 0.12 * (jitter(k, 43) - 0.5);
        let (cx, cz) = (ang.cos(), ang.sin());
        let len = if k % 2 == 0 {
            0.80 + 0.20 * jitter(k, 44)
        } else {
            0.45 + 0.25 * jitter(k, 45)
        };
        let mut rings = Vec::with_capacity(segments + 1);
        for i in 0..=segments {
            let t = i as f32 / segments as f32;
            let r = 0.94 + 0.10 * t;
            let y = 0.012 - t * len;
            let thick = 0.028 * (1.0 - 0.65 * t);
            let ring: Vec<u32> = (0..sides)
                .map(|s| {
                    let a = s as f32 / sides as f32 * TAU + ang;
                    // The cross-section in the thread's own horizontal frame.
                    let (dx, dz) = (a.cos() * thick, a.sin() * thick);
                    m.vertex([cx * r + dx, y, cz * r + dz], [1.0; 3], [t * len, 0.0])
                })
                .collect();
            rings.push(ring);
        }
        for w in rings.windows(2) {
            for s in 0..sides {
                let n = (s + 1) % sides;
                m.quad(w[0][s], w[1][s], w[1][n], w[0][n]);
            }
        }
    }
    m.smooth()
}

/// A comb jelly: a small ctenophore, an egg of clear jelly ribbed by eight
/// comb rows from pole to pole, along which `fs_glass` runs the cilia's
/// travelling rainbow. No bell and no pulse: it glides (`animate`, kind 11).
/// `aux.y` runs from the lower pole (0) to the upper (1).
fn comb() -> Mesh {
    let profile = [
        (0.00, 0.00),
        (0.30, 0.05),
        (0.50, 0.18),
        (0.60, 0.36),
        (0.62, 0.55),
        (0.56, 0.74),
        (0.42, 0.89),
        (0.22, 0.98),
        (0.00, 1.00),
    ];
    lathe_down_to(
        &profile,
        16,
        6,
        // The eight comb rows stand a little proud of the body.
        |y, a| 1.0 + 0.07 * (a * 8.0).cos().max(0.0).powi(4) * (y * PI).sin(),
        |y, _| [1.0, 0.94 + 0.06 * y, 1.0],
        |y| [0.0, y],
    )
}

/// A craggy hydrothermal chimney, hollow at the mouth. The outside climbs
/// in ledges and the lip flares before the wall turns back down into the
/// throat; mineral bands are baked into the vertex colour.
fn vent() -> Mesh {
    let profile = [
        (0.42, 0.00),
        (0.34, 0.10),
        (0.38, 0.22),
        (0.29, 0.36),
        (0.34, 0.50),
        (0.24, 0.64),
        (0.29, 0.78),
        (0.40, 0.89),
        (0.43, 0.97),
        (0.31, 1.00),
        (0.22, 0.96),
        (0.17, 0.89),
        (0.00, 0.86),
    ];
    lathe(
        &profile,
        20,
        |y, a| 1.0 + 0.10 * (a * 7.0 + y * 9.0).sin() + 0.05 * (a * 3.0 - y * 17.0).sin(),
        |y, a| {
            let band = 0.5 + 0.5 * (y * 19.0 + a * 2.0).sin();
            // Pale mineral crust gathers on the lip; the throat below it is
            // soot-black.
            let lip = ((y - 0.84) / 0.16).clamp(0.0, 1.0);
            [
                0.10 + 0.10 * band + 0.20 * lip,
                0.085 + 0.08 * band + 0.17 * lip,
                0.075 + 0.06 * band + 0.11 * lip,
            ]
        },
        |y| [0.0, y],
    )
}

/// A forest tree: a leaning trunk, six forking branches and a clustered
/// canopy. One instance per tree keeps the forest cheap while giving it a
/// silhouette with depth rather than a post with a ball on top.
fn tree() -> Mesh {
    let bark = [0.30, 0.20, 0.14];
    let leaf = [0.78, 0.82, 0.58];
    let mut m = Mesh::default();
    let base = [0.0, -0.04, 0.0];
    let middle = [0.035, 0.50, 0.015];
    let top = [-0.015, 0.97, 0.0];
    m.append(rod(base, middle, 0.105, 0.062, bark));
    m.append(rod(middle, top, 0.062, 0.020, bark));
    // Far off, a tree is a trunk, its boughs and a crown of a few clumps:
    // no buttresses, no forked twigs.
    let far = COARSER.get() >= 2;
    // A few buttresses, so the trunk grows out of the silt.
    for k in 0..if far { 0 } else { 3 } {
        let a = k as f32 * TAU / 3.0 + 0.5;
        m.append(rod(
            [a.cos() * 0.12, 0.0, a.sin() * 0.12],
            [a.cos() * 0.04, 0.22, a.sin() * 0.04],
            0.045,
            0.020,
            bark,
        ));
    }
    let mut leaf_clumps: Vec<([f32; 3], f32)> = Vec::new();
    for b in 0..6 {
        let angle = b as f32 / 6.0 * TAU + 0.35;
        let y = 0.30 + 0.085 * b as f32;
        let reach = 0.20 + 0.07 * (b as f32 * 1.7).sin().abs();
        let rise = 0.10 + 0.05 * (b as f32 * 2.3).cos().abs();
        let root = [middle[0] * (y / 0.5), y, middle[2] * (y / 0.5)];
        let tip = [
            root[0] + angle.cos() * reach,
            y + rise,
            root[2] + angle.sin() * reach,
        ];
        m.append(rod(root, tip, 0.032, 0.010, bark));
        leaf_clumps.push((tip, 0.10 + 0.025 * (b as f32 * 0.9).sin().abs()));
        if far {
            continue;
        }
        let fork = [
            tip[0] + (angle + 0.6).cos() * 0.11,
            tip[1] + 0.08,
            tip[2] + (angle + 0.6).sin() * 0.11,
        ];
        m.append(rod(tip, fork, 0.014, 0.005, bark));
        leaf_clumps.push((fork, 0.085));
    }
    // The crown: a broad, uneven collection of small clumps.
    let crown = if far { 3 } else { 5 };
    for k in 0..crown {
        let a = k as f32 / crown as f32 * TAU;
        let p = [
            top[0] + a.cos() * (0.10 + 0.04 * k as f32),
            top[1] - 0.02 + 0.04 * (k as f32 * 1.3).sin(),
            top[2] + a.sin() * (0.10 + 0.04 * k as f32),
        ];
        // Fewer clumps, each a little larger, so the crown keeps its bulk.
        leaf_clumps.push((p, if far { 0.13 } else { 0.105 }));
    }
    for (centre, size) in leaf_clumps {
        let clump = sphere_with(
            1,
            |p| 1.0 + 0.16 * (p[0] * 5.0 + p[2] * 4.0).sin(),
            |_| leaf,
        )
        .scaled([size * 1.15, size * 0.68, size]);
        m.append(clump.map(
            move |p| [p[0] + centre[0], p[1] + centre[1], p[2] + centre[2]],
            |n| n,
        ));
    }
    m.with_aux(|p| [0.0, p[1].clamp(0.0, 1.0)])
}

/// What each part of a drowned-city model is, carried in `aux.x` to
/// `city_light` in `scene.wesl`: the city's material (`world::material::CITY`)
/// lights windows, panes and neon by it.
pub mod city {
    pub const PLAIN: f32 = 0.0;
    pub const WINDOWS: f32 = 1.0;
    pub const PANES: f32 = 2.0;
    pub const NEON: f32 = 3.0;
}

/// Add `part` to `m`, turned to face outward and marked as `kind` of city
/// surface. Every part of a city model is closed, and wound counter-clockwise
/// seen from outside with its normals pointing out, so the whole model is
/// sealed and its back faces culled however its parts were built or mirrored.
fn city_part(m: &mut Mesh, mut part: Mesh, kind: f32) {
    let pos = |v: &Vertex| Vec3::from_array(v.pos);
    let mut volume = 0.0f32;
    for t in part.indices.as_chunks::<3>().0 {
        let [a, b, c] = t.map(|i| pos(&part.vertices[i as usize]));
        volume += a.dot(b.cross(c));
    }
    if volume < 0.0 {
        for t in part.indices.as_chunks_mut::<3>().0 {
            t.swap(1, 2);
        }
    }
    // Normals that were worked out from the old winding point in; turn
    // each to agree with the faces about it.
    let mut faces = vec![Vec3::ZERO; part.vertices.len()];
    for t in part.indices.as_chunks::<3>().0 {
        let [a, b, c] = t.map(|i| pos(&part.vertices[i as usize]));
        let n = (b - a).cross(c - a);
        for &i in t {
            faces[i as usize] += n;
        }
    }
    for (v, f) in part.vertices.iter_mut().zip(faces) {
        if f.dot(Vec3::from_array(v.normal)) < 0.0 {
            v.normal = v.normal.map(|c| -c);
        }
        v.aux = [kind, polish(v.colour)];
    }
    m.append(part);
}

/// How much a city part of `colour` shines like polished brass, in `aux.y`
/// for `city_light`: by how much warmer than blue it is, so the brass of
/// the cornices, piers and fins shines fully, the bronzed iron a little and
/// the crusted footings, verdigris, glass and neon not at all.
fn polish(colour: [f32; 3]) -> f32 {
    let warmth = (colour[0] - colour[2]) / colour[0].max(1e-3);
    ((warmth - 0.2) / 0.3).clamp(0.0, 1.0)
}

/// `m` moved by `d`.
fn moved(m: Mesh, d: [f32; 3]) -> Mesh {
    m.map(move |p| [p[0] + d[0], p[1] + d[1], p[2] + d[2]], |n| n)
}

/// `m` turned by `angle` about y.
fn turned(m: Mesh, angle: f32) -> Mesh {
    let (s, c) = angle.sin_cos();
    let spin = move |p: [f32; 3]| [c * p[0] - s * p[2], p[1], s * p[0] + c * p[2]];
    m.map(spin, spin)
}

/// `m` laid along x: its y axis turned to x. A mirror, which `city_part`
/// winds back out.
fn along_x(m: Mesh) -> Mesh {
    m.map(|p| [p[1], p[0], p[2]], |n| [n[1], n[0], n[2]])
}

/// An upright cylinder of radius `r` from `y0` to `y1`, with flat ends and
/// hard edges round them: `segments` round, coarsening to `least`.
fn drum(r: f32, y0: f32, y1: f32, segments: usize, least: usize, colour: [f32; 3]) -> Mesh {
    let n = fewer(segments, least);
    let mut m = Mesh::default();
    let round = |s: usize| {
        let a = s as f32 / n as f32 * TAU;
        (a.cos(), a.sin())
    };
    let push = |m: &mut Mesh, pos: [f32; 3], normal: [f32; 3]| {
        m.vertices.push(Vertex {
            pos,
            normal,
            colour,
            aux: [0.0; 2],
        });
        m.vertices.len() as u32 - 1
    };
    let side: Vec<(u32, u32)> = (0..n)
        .map(|s| {
            let (c, sn) = round(s);
            let normal = [c, 0.0, sn];
            (
                push(&mut m, [r * c, y0, r * sn], normal),
                push(&mut m, [r * c, y1, r * sn], normal),
            )
        })
        .collect();
    for s in 0..n {
        let t = (s + 1) % n;
        m.quad(side[s].0, side[s].1, side[t].1, side[t].0);
    }
    for (y, up) in [(y0, -1.0), (y1, 1.0)] {
        let centre = push(&mut m, [0.0, y, 0.0], [0.0, up, 0.0]);
        let rim: Vec<u32> = (0..n)
            .map(|s| {
                let (c, sn) = round(s);
                push(&mut m, [r * c, y, r * sn], [0.0, up, 0.0])
            })
            .collect();
        for s in 0..n {
            let t = (s + 1) % n;
            if up > 0.0 {
                m.tri(centre, rim[t], rim[s]);
            } else {
                m.tri(centre, rim[s], rim[t]);
            }
        }
    }
    m
}

/// A ring of tube: a torus about y of radius `big`, its tube `small` thick.
fn ring(big: f32, small: f32, segments: usize, colour: [f32; 3]) -> Mesh {
    let around = fewer(6, 4);
    let profile: Vec<(f32, f32)> = (0..=around)
        .map(|k| {
            let a = k as f32 / around as f32 * TAU;
            (big + small * a.cos(), small * a.sin())
        })
        .collect();
    lathe_down_to(
        &profile,
        segments,
        6,
        |_, _| 1.0,
        move |_, _| colour,
        |_| [0.0; 2],
    )
}

/// A pointed cap about y: a cone of radius `r` at its foot and `h` tall.
fn point(r: f32, h: f32, segments: usize, least: usize, colour: [f32; 3]) -> Mesh {
    lathe_down_to(
        &[(0.0, 0.0), (r, 0.0), (0.0, h)],
        segments,
        least,
        |_, _| 1.0,
        move |_, _| colour,
        |_| [0.0; 2],
    )
}

/// A dome about y, `r` across and `h` tall, closed underneath.
fn cupola(r: f32, h: f32, segments: usize, least: usize, colour: [f32; 3]) -> Mesh {
    let steps = fewer(8, 3);
    let mut profile = vec![(0.0, 0.0)];
    profile.extend((0..=steps).map(|k| {
        let a = k as f32 / steps as f32 * PI * 0.5;
        (r * a.cos(), h * a.sin())
    }));
    lathe_down_to(
        &profile,
        segments,
        least,
        |_, _| 1.0,
        move |_, _| colour,
        |_| [0.0; 2],
    )
}

/// A beacon: a small sphere of neon, `r` across, at `at`.
fn beacon(m: &mut Mesh, r: f32, at: [f32; 3]) {
    let ball = sphere_with(1, |_| 1.0, |_| WHITE).scaled([r; 3]);
    city_part(m, moved(ball, at), city::NEON);
}

/// Brass and iron, as the city's models are built of: bronzed iron walls,
/// brass cornices and piers, verdigris pipes and barnacled footings, and lit
/// glass. The instance's colour tints them all; neon is white here and takes
/// its colour from the instance.
const IRON: [f32; 3] = [0.44, 0.39, 0.31];
const BRASS: [f32; 3] = [0.78, 0.60, 0.32];
const PIER: [f32; 3] = [0.58, 0.46, 0.28];
const VERDIGRIS: [f32; 3] = [0.34, 0.50, 0.40];
const CRUST: [f32; 3] = [0.34, 0.38, 0.29];
const BARNACLE: [f32; 3] = [0.62, 0.60, 0.52];
const GLASS: [f32; 3] = [0.55, 0.70, 0.62];
const WHITE: [f32; 3] = [1.0; 3];

/// The barnacle clumps round a footing of half-width `r`, square or
/// `round`; only at full detail, as they are too small to miss further off.
fn barnacles(m: &mut Mesh, r: f32, round: bool) {
    if COARSER.get() > 0 {
        return;
    }
    for k in 0..8 {
        let a = k as f32 / 8.0 * TAU + 0.3;
        let reach = if round {
            r
        } else {
            r / a.cos().abs().max(a.sin().abs())
        };
        let size = 0.05 + 0.025 * ((k * 5) % 3) as f32;
        let clump = sphere_with(0, |_| 1.0, |_| BARNACLE).scaled([size, size * 0.7, size]);
        city_part(
            m,
            moved(
                clump,
                [
                    a.cos() * reach,
                    0.10 + 0.04 * (k % 2) as f32,
                    a.sin() * reach,
                ],
            ),
            city::PLAIN,
        );
    }
}

/// The drowned city's square tower, about four and three quarter units tall
/// at unit scale: three tiers of windowed wall, each stepped back behind a
/// brass cornice, ribbed with piers that rise past it into pinnacles; neon up
/// the middle tier's corners and round the crown; a stepped crown, a spire
/// and a beacon. Pipes climb its first tier and barnacles crust its foot. The
/// piers thin out at each coarser level, which is most of what it loses.
fn tower() -> Mesh {
    let mut m = Mesh::default();
    city_part(
        &mut m,
        block([0.64, 0.25, 0.64], [0.0, -0.13, 0.0], CRUST),
        city::PLAIN,
    );
    city_part(
        &mut m,
        block([0.515, 0.17, 0.515], [0.0, 0.27, 0.0], CRUST),
        city::PLAIN,
    );
    barnacles(&mut m, 0.52, false);
    let tiers = [
        (0.50, 0.10, 2.20, 4),
        (0.38, 2.20, 3.20, 3),
        (0.26, 3.20, 3.80, 2),
    ];
    for (k, &(hw, y0, y1, piers)) in tiers.iter().enumerate() {
        let mid = (y0 + y1) * 0.5;
        let half = (y1 - y0) * 0.5;
        city_part(
            &mut m,
            block([hw, half, hw], [0.0, mid, 0.0], IRON),
            city::WINDOWS,
        );
        city_part(
            &mut m,
            block([hw + 0.03, 0.03, hw + 0.03], [0.0, y1, 0.0], BRASS),
            city::PLAIN,
        );
        // Piers down each face, between the columns of windows.
        let n = fewer(piers, 1);
        for i in 0..n {
            let u = if n == 1 {
                0.0
            } else {
                -0.72 * hw + i as f32 * 1.44 * hw / (n - 1) as f32
            };
            for face in 0..4 {
                let pier = block([0.02, half, 0.014], [hw + 0.012, mid, u], PIER);
                city_part(&mut m, turned(pier, face as f32 * PI * 0.5), city::PLAIN);
            }
        }
        // Corner piers, rising past the cornice into pinnacles; on the middle
        // tier they are neon.
        let (corner, colour) = if k == 1 {
            (city::NEON, WHITE)
        } else {
            (city::PLAIN, BRASS)
        };
        for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
            city_part(
                &mut m,
                block(
                    [0.035, half + 0.06, 0.035],
                    [sx * hw, mid + 0.06, sz * hw],
                    colour,
                ),
                corner,
            );
        }
    }
    // Pipes up the first tier, either side of the middle pier.
    for z in [-0.27, 0.27] {
        city_part(
            &mut m,
            moved(drum(0.035, 0.10, 2.20, 8, 8, VERDIGRIS), [0.55, 0.0, z]),
            city::PLAIN,
        );
    }
    // The crown: a neon band, two brass steps, the spire and its beacon.
    city_part(
        &mut m,
        block([0.272, 0.022, 0.272], [0.0, 3.62, 0.0], WHITE),
        city::NEON,
    );
    city_part(
        &mut m,
        block([0.19, 0.06, 0.19], [0.0, 3.86, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        block([0.12, 0.06, 0.12], [0.0, 3.98, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        moved(point(0.06, 0.7, 8, 4, BRASS), [0.0, 4.04, 0.0]),
        city::PLAIN,
    );
    beacon(&mut m, 0.035, [0.0, 4.74, 0.0]);
    m
}

/// The drowned city's round tower: a ribbed drum of windows stepped back
/// once behind a brass cornice, a neon band round its upper drum, a lit
/// glass cupola and a needle with its beacon.
fn rotunda() -> Mesh {
    let mut m = Mesh::default();
    city_part(&mut m, drum(0.64, -0.38, 0.12, 20, 8, CRUST), city::PLAIN);
    city_part(&mut m, drum(0.475, 0.10, 0.42, 20, 8, CRUST), city::PLAIN);
    barnacles(&mut m, 0.48, true);
    for (r, y0, y1, ribs) in [(0.45, 0.10, 2.60, 12), (0.34, 2.60, 3.50, 8)] {
        city_part(&mut m, drum(r, y0, y1, 24, 8, IRON), city::WINDOWS);
        city_part(
            &mut m,
            drum(r + 0.05, y1 - 0.04, y1 + 0.04, 24, 8, BRASS),
            city::PLAIN,
        );
        let n = fewer(ribs, 4);
        for k in 0..n {
            let rib = block(
                [0.016, (y1 - y0) * 0.5, 0.02],
                [r + 0.008, (y0 + y1) * 0.5, 0.0],
                PIER,
            );
            city_part(
                &mut m,
                turned(rib, (k as f32 + 0.5) / n as f32 * TAU),
                city::PLAIN,
            );
        }
    }
    city_part(&mut m, drum(0.352, 3.26, 3.31, 24, 8, WHITE), city::NEON);
    for a in [0.8f32, -0.8] {
        city_part(
            &mut m,
            moved(
                drum(0.035, 0.10, 2.56, 8, 8, VERDIGRIS),
                [0.50 * a.cos(), 0.0, 0.50 * a.sin()],
            ),
            city::PLAIN,
        );
    }
    // The cupola: lit panes from the cornice up.
    city_part(
        &mut m,
        moved(cupola(0.30, 0.34, 20, 6, GLASS), [0.0, 3.54, 0.0]),
        city::PANES,
    );
    city_part(
        &mut m,
        moved(point(0.035, 0.75, 8, 4, BRASS), [0.0, 3.84, 0.0]),
        city::PLAIN,
    );
    beacon(&mut m, 0.03, [0.0, 4.58, 0.0]);
    m
}

/// A glass dome a unit across on a windowed drum, lit from within, with a
/// brass lantern and its beacon on top.
fn dome() -> Mesh {
    let mut m = Mesh::default();
    city_part(&mut m, drum(1.0, -0.35, 0.34, 28, 10, IRON), city::WINDOWS);
    city_part(&mut m, drum(1.04, 0.31, 0.38, 28, 10, BRASS), city::PLAIN);
    city_part(
        &mut m,
        moved(cupola(0.97, 0.80, 28, 8, GLASS), [0.0, 0.38, 0.0]),
        city::PANES,
    );
    city_part(&mut m, drum(0.12, 1.10, 1.30, 10, 5, BRASS), city::PLAIN);
    city_part(
        &mut m,
        moved(point(0.15, 0.2, 10, 5, BRASS), [0.0, 1.30, 0.0]),
        city::PLAIN,
    );
    beacon(&mut m, 0.04, [0.0, 1.54, 0.0]);
    m
}

/// A glass walkway: a tube of lit panes from y = 0 to 1, a unit in radius,
/// ringed with brass. Stretched between two towers by `world::between`.
fn walkway() -> Mesh {
    let mut m = Mesh::default();
    city_part(&mut m, drum(1.0, 0.0, 1.0, 14, 6, GLASS), city::PANES);
    let rings = fewer(8, 3);
    for k in 0..rings {
        let y = (k as f32 + 0.5) / rings as f32;
        city_part(
            &mut m,
            drum(1.14, y - 0.012, y + 0.012, 14, 6, BRASS),
            city::PLAIN,
        );
    }
    m
}

/// A run of the great pipes: one along x from -1 to 1, lying on two saddles
/// with its flanges, a riser with a cap, and a valve wheel on its stem.
fn pipework() -> Mesh {
    let mut m = Mesh::default();
    let r = 0.07;
    city_part(
        &mut m,
        moved(along_x(drum(r, -1.0, 1.0, 12, 6, VERDIGRIS)), [0.0, r, 0.0]),
        city::PLAIN,
    );
    for x in [-0.92, -0.35, 0.35, 0.92] {
        city_part(
            &mut m,
            moved(
                along_x(drum(r * 1.35, -0.025, 0.025, 12, 6, BRASS)),
                [x, r, 0.0],
            ),
            city::PLAIN,
        );
    }
    for x in [-0.62, 0.62] {
        city_part(
            &mut m,
            block([0.03, 0.04, 0.09], [x, 0.04, 0.0], IRON),
            city::PLAIN,
        );
    }
    city_part(
        &mut m,
        moved(drum(0.05, r, 0.75, 10, 5, VERDIGRIS), [0.62, 0.0, 0.0]),
        city::PLAIN,
    );
    city_part(
        &mut m,
        moved(drum(0.07, 0.72, 0.80, 10, 5, BRASS), [0.62, 0.0, 0.0]),
        city::PLAIN,
    );
    city_part(&mut m, drum(0.016, 0.12, 0.30, 6, 4, BRASS), city::PLAIN);
    city_part(
        &mut m,
        moved(ring(0.085, 0.012, 14, BRASS), [0.0, 0.30, 0.0]),
        city::PLAIN,
    );
    for a in [0.0, PI * 0.5] {
        city_part(
            &mut m,
            turned(block([0.085, 0.006, 0.006], [0.0, 0.30, 0.0], BRASS), a),
            city::PLAIN,
        );
    }
    m
}

/// A neon sign: a ring with a bar through it and a chevron under, on an
/// iron plate bracketed off a wall, facing +z.
fn sign() -> Mesh {
    let mut m = Mesh::default();
    city_part(
        &mut m,
        block([0.24, 0.30, 0.012], [0.0, 0.0, -0.02], [0.20, 0.18, 0.15]),
        city::PLAIN,
    );
    let roundel = ring(0.17, 0.014, 20, WHITE).map(|p| [p[0], p[2], p[1]], |n| [n[0], n[2], n[1]]);
    city_part(&mut m, roundel, city::NEON);
    city_part(
        &mut m,
        block([0.016, 0.27, 0.014], [0.0, 0.0, 0.0], WHITE),
        city::NEON,
    );
    for side in [-1.0f32, 1.0] {
        let (s, c) = (side * 0.5).sin_cos();
        let bar = block([0.07, 0.012, 0.012], [0.0, 0.0, 0.0], WHITE).map(
            move |p| {
                [
                    c * p[0] - s * p[1] + side * 0.06,
                    s * p[0] + c * p[1] - 0.20,
                    p[2],
                ]
            },
            move |n| [c * n[0] - s * n[1], s * n[0] + c * n[1], n[2]],
        );
        city_part(&mut m, bar, city::NEON);
        city_part(
            &mut m,
            block([0.02, 0.02, 0.04], [side * 0.18, 0.24, -0.06], IRON),
            city::PLAIN,
        );
    }
    m
}

/// The drowned city's stepped pyramid: four tiers of windowed wall, each set
/// back behind a brass cornice, with a neon band and corner piers on the
/// middle tiers, a stepped crown and a beacon. It fills a block with a lower,
/// broader silhouette than the towers and catches the eye from across the
/// avenue.
fn ziggurat() -> Mesh {
    let mut m = Mesh::default();
    city_part(
        &mut m,
        block([0.72, 0.05, 0.72], [0.0, -0.03, 0.0], CRUST),
        city::PLAIN,
    );
    barnacles(&mut m, 0.72, false);
    let tiers = [
        (0.66, 0.02, 0.60, 4),
        (0.53, 0.60, 1.16, 3),
        (0.40, 1.16, 1.68, 2),
        (0.27, 1.68, 2.16, 1),
        (0.15, 2.16, 2.52, 0),
    ];
    for (k, &(hw, y0, y1, piers)) in tiers.iter().enumerate() {
        let mid = (y0 + y1) * 0.5;
        let half = (y1 - y0) * 0.5;
        city_part(
            &mut m,
            block([hw, half, hw], [0.0, mid, 0.0], IRON),
            city::WINDOWS,
        );
        city_part(
            &mut m,
            block([hw + 0.028, 0.022, hw + 0.028], [0.0, y1, 0.0], BRASS),
            city::PLAIN,
        );
        // A neon string course on two tiers, and brass corner piers rising
        // past every cornice into small pinnacles.
        if k == 1 || k == 3 {
            city_part(
                &mut m,
                block(
                    [hw + 0.012, 0.010, hw + 0.012],
                    [0.0, y0 + 0.055, 0.0],
                    WHITE,
                ),
                city::NEON,
            );
        }
        let n = fewer(piers, 1);
        for i in 0..n {
            let u = if n == 1 {
                0.0
            } else {
                -0.72 * hw + i as f32 * 1.44 * hw / (n - 1) as f32
            };
            for face in 0..4 {
                let pier = block([0.018, half, 0.014], [hw + 0.012, mid, u], PIER);
                city_part(&mut m, turned(pier, face as f32 * PI * 0.5), city::PLAIN);
            }
        }
        let (corner, colour) = if k == 1 {
            (city::NEON, WHITE)
        } else {
            (city::PLAIN, BRASS)
        };
        for (sx, sz) in [(1.0, 1.0), (-1.0, 1.0), (-1.0, -1.0), (1.0, -1.0)] {
            city_part(
                &mut m,
                block(
                    [0.024, half + 0.055, 0.024],
                    [sx * hw, mid + 0.055, sz * hw],
                    colour,
                ),
                corner,
            );
        }
    }
    city_part(
        &mut m,
        block([0.175, 0.05, 0.175], [0.0, 2.58, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        block([0.115, 0.035, 0.115], [0.0, 2.66, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        moved(point(0.055, 0.60, 8, 4, BRASS), [0.0, 2.69, 0.0]),
        city::PLAIN,
    );
    beacon(&mut m, 0.03, [0.0, 3.32, 0.0]);
    m
}

/// The drowned city's arcade: a low, broad hall of two windowed floors, a
/// colonnade along both long faces under a lit glass canopy, a neon marquee
/// over the doors and a small lit cupola on the roof. It is the street's
/// frontage, between the towers.
fn arcade() -> Mesh {
    let mut m = Mesh::default();
    city_part(
        &mut m,
        block([0.98, 0.05, 0.62], [0.0, -0.03, 0.0], CRUST),
        city::PLAIN,
    );
    barnacles(&mut m, 0.95, false);
    city_part(
        &mut m,
        block([0.92, 0.42, 0.56], [0.0, 0.48, 0.0], IRON),
        city::WINDOWS,
    );
    city_part(
        &mut m,
        block([0.96, 0.024, 0.60], [0.0, 0.92, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        block([0.74, 0.16, 0.42], [0.0, 1.10, 0.0], IRON),
        city::WINDOWS,
    );
    city_part(
        &mut m,
        block([0.78, 0.020, 0.46], [0.0, 1.28, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        block([0.66, 0.030, 0.36], [0.0, 1.31, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        moved(cupola(0.15, 0.13, 14, 5, GLASS), [0.0, 1.34, 0.0]),
        city::PANES,
    );
    city_part(&mut m, drum(0.035, 1.46, 1.52, 8, 4, BRASS), city::PLAIN);
    beacon(&mut m, 0.022, [0.0, 1.55, 0.0]);
    // The colonnade and its glass canopy, on both long faces.
    for side in [-1.0f32, 1.0] {
        let cols = fewer(5, 3);
        for k in 0..cols {
            let u = if cols == 1 {
                0.0
            } else {
                -0.70 + k as f32 * 1.40 / (cols - 1) as f32
            };
            city_part(
                &mut m,
                moved(drum(0.034, 0.06, 0.88, 8, 4, PIER), [u, 0.0, side * 0.585]),
                city::PLAIN,
            );
            city_part(
                &mut m,
                block([0.055, 0.020, 0.055], [u, 0.88, side * 0.585], BRASS),
                city::PLAIN,
            );
        }
        city_part(
            &mut m,
            block([0.82, 0.018, 0.19], [0.0, 0.79, side * 0.665], GLASS),
            city::PANES,
        );
        city_part(
            &mut m,
            block([0.82, 0.010, 0.022], [0.0, 0.772, side * 0.755], WHITE),
            city::NEON,
        );
        // A dark marquee board with a neon border over the canopy.
        city_part(
            &mut m,
            block([0.50, 0.10, 0.018], [0.0, 1.04, side * 0.575], [0.16; 3]),
            city::PLAIN,
        );
        city_part(
            &mut m,
            block([0.46, 0.008, 0.024], [0.0, 1.13, side * 0.580], WHITE),
            city::NEON,
        );
        for sx in [-1.0f32, 1.0] {
            city_part(
                &mut m,
                block(
                    [0.018, 0.36, 0.018],
                    [sx * 0.735, 0.49, side * 0.575],
                    WHITE,
                ),
                city::NEON,
            );
        }
    }
    m
}

/// The drowned city's works: a riveted hall with a sawtooth roof of lit
/// skylights, two banded smokestacks, a run of great pipes and a glowing
/// furnace door. It is the city's industry, lower and dirtier than the
/// towers.
fn factory() -> Mesh {
    let mut m = Mesh::default();
    city_part(
        &mut m,
        block([0.95, 0.05, 0.65], [0.0, -0.03, 0.0], CRUST),
        city::PLAIN,
    );
    barnacles(&mut m, 0.92, false);
    city_part(
        &mut m,
        block([0.88, 0.38, 0.58], [0.0, 0.48, 0.0], IRON),
        city::WINDOWS,
    );
    city_part(
        &mut m,
        block([0.92, 0.025, 0.62], [0.0, 0.88, 0.0], BRASS),
        city::PLAIN,
    );
    city_part(
        &mut m,
        block([0.86, 0.030, 0.56], [0.0, 0.92, 0.0], IRON),
        city::PLAIN,
    );
    // A sawtooth of lit skylights and their brass frames.
    let teeth = fewer(4, 2);
    for k in 0..teeth {
        let u = -0.63 + k as f32 * 1.26 / (teeth - 1).max(1) as f32;
        city_part(
            &mut m,
            block([0.105, 0.11, 0.52], [u, 1.01, 0.0], GLASS),
            city::PANES,
        );
        city_part(
            &mut m,
            block([0.018, 0.13, 0.55], [u - 0.11, 1.03, 0.0], BRASS),
            city::PLAIN,
        );
    }
    // Two stacks with their bands and flared mouths.
    for (x, z, r, top) in [(-0.50, 0.26, 0.13, 2.55), (0.48, 0.10, 0.10, 2.20)] {
        city_part(&mut m, drum(r, 0.90, top, 12, 6, IRON), city::PLAIN);
        let bands = fewer(3, 2);
        for k in 0..bands {
            let y = 1.10 + k as f32 * (top - 1.25) / (bands.max(2) - 1) as f32;
            city_part(
                &mut m,
                drum(r + 0.018, y, y + 0.035, 12, 6, BRASS),
                city::PLAIN,
            );
        }
        city_part(
            &mut m,
            drum(r + 0.055, top - 0.06, top + 0.02, 12, 6, BRASS),
            city::PLAIN,
        );
        beacon(&mut m, 0.022, [x, top + 0.08, z]);
    }
    // A run of pipes down one side and a riser into the hall.
    city_part(
        &mut m,
        moved(
            along_x(drum(0.045, -0.92, 0.92, 10, 5, VERDIGRIS)),
            [0.0, 0.30, -0.675],
        ),
        city::PLAIN,
    );
    for x in [-0.86, -0.30, 0.30, 0.86] {
        city_part(
            &mut m,
            moved(
                along_x(drum(0.06, -0.025, 0.025, 10, 5, BRASS)),
                [x, 0.30, -0.675],
            ),
            city::PLAIN,
        );
    }
    city_part(
        &mut m,
        moved(
            drum(0.045, 0.30, 0.92, 10, 5, VERDIGRIS),
            [0.66, 0.0, -0.675],
        ),
        city::PLAIN,
    );
    // Furnace doors, glowing from within on both long faces.
    for side in [-1.0f32, 1.0] {
        city_part(
            &mut m,
            block(
                [0.30, 0.20, 0.020],
                [0.0, 0.34, side * 0.595],
                [0.20, 0.17, 0.14],
            ),
            city::PLAIN,
        );
        city_part(
            &mut m,
            block([0.24, 0.15, 0.028], [0.0, 0.34, side * 0.607], GLASS),
            city::PANES,
        );
        for x in [-0.12, 0.12] {
            city_part(
                &mut m,
                block([0.014, 0.16, 0.012], [x, 0.34, side * 0.625], IRON),
                city::PLAIN,
            );
        }
    }
    m
}

/// The drowned city's picture palace: a ribbed round drum with neon fins
/// round its lower wall, a wide glass marquee ringed in light, a narrower
/// upper drum and a lit cupola. It is the city's night out.
fn theatre() -> Mesh {
    let mut m = Mesh::default();
    city_part(&mut m, drum(0.86, -0.35, 0.08, 26, 9, CRUST), city::PLAIN);
    barnacles(&mut m, 0.84, true);
    city_part(&mut m, drum(0.74, 0.06, 1.10, 26, 10, IRON), city::WINDOWS);
    city_part(&mut m, drum(0.80, 1.08, 1.13, 26, 10, BRASS), city::PLAIN);
    city_part(&mut m, drum(0.52, 1.13, 1.50, 20, 8, IRON), city::WINDOWS);
    city_part(&mut m, drum(0.58, 1.48, 1.53, 20, 8, BRASS), city::PLAIN);
    // Brass fins up the lower wall, every other one carrying a thin neon
    // strip: enough to mark the picture palace without turning it into a
    // fairground hoop.
    let fins = fewer(10, 5);
    for k in 0..fins {
        let a = k as f32 / fins as f32 * TAU;
        let (s, c) = a.sin_cos();
        let lit = k % 2 == 0;
        city_part(
            &mut m,
            block(
                [0.014, 0.34, 0.018],
                [0.752 * c, 0.56, 0.752 * s],
                if lit { WHITE } else { PIER },
            ),
            if lit { city::NEON } else { city::PLAIN },
        );
    }
    // The marquee: a brass fascia, a glass canopy beneath it and a thin neon
    // rim, all standing out over the pavement rather than round the whole
    // drum like a carousel roof.
    city_part(&mut m, drum(0.94, 1.06, 1.11, 26, 10, BRASS), city::PLAIN);
    city_part(&mut m, drum(0.91, 1.01, 1.05, 26, 10, GLASS), city::PANES);
    city_part(
        &mut m,
        moved(ring(0.915, 0.010, 26, WHITE), [0.0, 1.03, 0.0]),
        city::NEON,
    );
    for k in 0..4 {
        let a = (k as f32 - 1.5) * 0.22;
        city_part(
            &mut m,
            block(
                [0.012, 0.34, 0.012],
                [0.91 * a.sin(), 1.02, 0.91 * a.cos()],
                BRASS,
            ),
            city::PLAIN,
        );
    }
    // The entrance: a projecting doorway and canopy on the avenue side.
    for (z, w, h) in [(0.78, 0.24, 0.50), (0.84, 0.30, 0.16)] {
        city_part(
            &mut m,
            block([w, h * 0.5, 0.07], [0.0, h * 0.5 + 0.06, z], IRON),
            city::WINDOWS,
        );
    }
    city_part(
        &mut m,
        block([0.34, 0.016, 0.16], [0.0, 0.62, 0.84], GLASS),
        city::PANES,
    );
    city_part(
        &mut m,
        block([0.34, 0.010, 0.018], [0.0, 0.61, 0.92], WHITE),
        city::NEON,
    );
    // The cupola and its lantern.
    city_part(
        &mut m,
        moved(cupola(0.42, 0.40, 20, 7, GLASS), [0.0, 1.53, 0.0]),
        city::PANES,
    );
    city_part(&mut m, drum(0.10, 1.90, 2.02, 10, 5, BRASS), city::PLAIN);
    city_part(
        &mut m,
        moved(point(0.07, 0.18, 8, 4, BRASS), [0.0, 2.02, 0.0]),
        city::PLAIN,
    );
    beacon(&mut m, 0.028, [0.0, 2.23, 0.0]);
    m
}

/// A plain box of half-extents `size` about `centre`: straps, buckles and
/// the frame the divers' reservoir hangs on.
fn block(size: [f32; 3], centre: [f32; 3], colour: [f32; 3]) -> Mesh {
    extrude(
        &[[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]],
        2.0,
        colour,
    )
    .scaled(size)
    .map(
        move |p| [p[0] + centre[0], p[1] + centre[1], p[2] + centre[2]],
        |n| n,
    )
}

/// A diver's canvas suit from the hips to the neck, with the corselet's belt,
/// the frame on his back and the air reservoir it carries: the feet are at
/// y = 0, the copper helmet is a separate model on top.
fn diver_body() -> Mesh {
    let canvas = |y: f32, a: f32| {
        // Coarse canvas, a little darker in the folds and at the seams.
        let weave = 0.90 + 0.10 * ((a * 9.0).sin() * (y * 43.0).cos());
        [0.46 * weave, 0.40 * weave, 0.31 * weave]
    };
    let mut m = lathe(
        &[
            (0.0, 0.400),
            (0.074, 0.400),
            (0.080, 0.445),
            (0.072, 0.515),
            (0.075, 0.580),
            (0.086, 0.645),
            (0.100, 0.695),
            (0.098, 0.728),
            (0.0, 0.755),
        ],
        10,
        |_, _| 1.0,
        canvas,
        |_| [0.0; 2],
    )
    .scaled([1.0, 1.0, 0.66]);
    // A leather belt and its brass buckle.
    m.append(
        lathe(
            &[(0.0, 0.442), (0.081, 0.442), (0.083, 0.472), (0.0, 0.472)],
            10,
            |_, _| 1.0,
            |_, _| [0.24, 0.20, 0.15],
            |_| [0.0; 2],
        )
        .scaled([1.0, 1.0, 0.69]),
    );
    m.append(block(
        [0.010, 0.016, 0.022],
        [0.074, 0.457, 0.0],
        [0.64, 0.42, 0.22],
    ));
    // The reservoir sits on a flat frame against the small of his back.
    m.append(block(
        [0.010, 0.125, 0.090],
        [-0.076, 0.610, 0.0],
        [0.22, 0.23, 0.22],
    ));
    m.append(
        lathe(
            &[
                (0.0, 0.455),
                (0.030, 0.455),
                (0.043, 0.472),
                (0.047, 0.505),
                (0.047, 0.730),
                (0.043, 0.758),
                (0.0, 0.775),
            ],
            8,
            |_, _| 1.0,
            |y, _| {
                // Two iron bands round the tank.
                let band = if (y * 70.0).sin() > 0.6 { 0.88 } else { 1.0 };
                [0.30 * band, 0.31 * band, 0.33 * band]
            },
            |_| [0.0; 2],
        )
        .map(|p| [p[0] - 0.120, p[1], p[2]], |n| n),
    );
    // Its valve, and the hose curling up into the back of the helmet.
    m.append(rod(
        [-0.120, 0.775, 0.0],
        [-0.120, 0.802, 0.0],
        0.014,
        0.009,
        [0.46, 0.38, 0.24],
    ));
    m.append(rod(
        [-0.110, 0.774, 0.030],
        [-0.074, 0.824, 0.036],
        0.014,
        0.011,
        [0.16, 0.16, 0.15],
    ));
    m.append(rod(
        [-0.074, 0.824, 0.036],
        [-0.044, 0.850, 0.026],
        0.011,
        0.008,
        [0.16, 0.16, 0.15],
    ));
    m
}

/// The copper helmet: a domed crown, the corselet over the shoulders, its
/// neck ring, the boss and dark glass of the round faceplate, and the valve
/// and exhaust on top and at the back.
fn diver_helmet() -> Mesh {
    let copper = |y: f32, a: f32| {
        // Beaten copper, with a warmer band where the lamp catches the brow.
        let sheen = 0.86 + 0.12 * (a * 4.0 + y * 17.0).sin();
        [0.78 * sheen, 0.42 * sheen, 0.20 * sheen]
    };
    let mut m = lathe(
        &[
            (0.0, 0.735),
            (0.100, 0.735),
            (0.126, 0.755),
            (0.130, 0.773),
            (0.124, 0.795),
            (0.0, 0.795),
        ],
        10,
        |_, a| 1.0 + 0.010 * (a * 8.0).sin(),
        copper,
        |_| [0.0; 2],
    )
    .scaled([1.0, 1.0, 0.82]);
    m.append(lathe(
        &[(0.0, 0.790), (0.060, 0.790), (0.060, 0.803), (0.0, 0.803)],
        8,
        |_, _| 1.0,
        |_, _| [0.52, 0.32, 0.16],
        |_| [0.0; 2],
    ));
    // The crown, sat a little forward of the neck as the old diver's was.
    m.append(
        lathe(
            &[
                (0.0, 0.795),
                (0.080, 0.795),
                (0.099, 0.812),
                (0.106, 0.840),
                (0.106, 0.870),
                (0.099, 0.900),
                (0.084, 0.927),
                (0.060, 0.948),
                (0.0, 0.968),
            ],
            10,
            |_, _| 1.0,
            copper,
            |_| [0.0; 2],
        )
        .scaled([1.0, 1.0, 0.97])
        .map(|p| [p[0] + 0.030, p[1], p[2]], |n| n),
    );
    // The faceplate: a copper boss with the dark glass set into its face.
    m.append(rod(
        [0.090, 0.876, 0.0],
        [0.145, 0.876, 0.0],
        0.056,
        0.052,
        [0.72, 0.42, 0.21],
    ));
    m.append(rod(
        [0.145, 0.876, 0.0],
        [0.154, 0.876, 0.0],
        0.042,
        0.040,
        [0.24, 0.42, 0.44],
    ));
    m.append(rod(
        [0.030, 0.966, 0.0],
        [0.030, 0.982, 0.0],
        0.018,
        0.012,
        [0.58, 0.34, 0.16],
    ));
    // The exhaust pipe at the back that the bubbles stream from.
    m.append(rod(
        [-0.055, 0.905, 0.030],
        [-0.086, 0.918, 0.044],
        0.012,
        0.008,
        [0.46, 0.30, 0.17],
    ));
    m
}

/// One canvas limb of a diver's suit: from `between`'s start (0) to its end
/// (1), with cuffs at wrist and ankle. The same mesh serves every arm and
/// leg of the party.
fn diver_limb() -> Mesh {
    lathe(
        &[
            (0.0, 0.0),
            (0.86, 0.0),
            (0.80, 0.50),
            (0.88, 0.94),
            (0.82, 1.0),
            (0.0, 1.0),
        ],
        8,
        |_, _| 1.0,
        |y, a| {
            if y < 0.06 || y > 0.94 {
                [0.30, 0.26, 0.20]
            } else {
                let fold = 0.90 + 0.10 * ((a * 5.0).sin() * (y * 27.0).cos());
                [0.46 * fold, 0.40 * fold, 0.31 * fold]
            }
        },
        |_| [0.0; 2],
    )
}

/// A weighted diving boot, the ankle at the origin and the toe toward +x: a
/// lead sole, a leather upper and a brass toe cap.
fn diver_boot() -> Mesh {
    let mut m = extrude(
        &[
            [-0.045, -0.028],
            [0.062, -0.028],
            [0.076, -0.006],
            [0.048, 0.012],
            [0.018, 0.020],
            [0.018, 0.038],
            [-0.038, 0.038],
            [-0.048, 0.004],
        ],
        0.085,
        [0.11, 0.11, 0.12],
    );
    m.append(extrude(
        &[
            [-0.050, -0.034],
            [0.080, -0.034],
            [0.080, -0.022],
            [-0.050, -0.022],
        ],
        0.098,
        [0.18, 0.18, 0.19],
    ));
    m.append(extrude(
        &[
            [0.040, -0.020],
            [0.080, -0.020],
            [0.080, 0.014],
            [0.048, 0.018],
        ],
        0.092,
        [0.52, 0.38, 0.22],
    ));
    m
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

/// A manta ray, two units across the wings with the head at +x: a thick body
/// drawn out into thin, swept wings, the two horns of its cephalic fins either
/// side of the mouth and a whip of a tail. Black above, white below.
fn manta() -> Mesh {
    let smooth = |a: f32, b: f32, v: f32| {
        let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
        t * t * (3.0 - 2.0 * t)
    };
    let mut m = loft(
        -0.34,
        0.34,
        16,
        20,
        move |x| {
            // Widest a little behind the middle, where the wings sweep back
            // to their tips; a blunt, broad head in front.
            let span = if x < -0.06 {
                (1.0 - ((x + 0.06) / 0.28).powi(2)).max(0.0).sqrt()
            } else {
                1.0 - 0.72 * ((x + 0.06) / 0.40).powf(1.3)
            };
            let t = (x + 0.34) / 0.68;
            let thick = 0.075 * (t * PI).sin().powf(0.5) + 0.004;
            (0.0, thick, thick * 0.55, span.max(0.03))
        },
        countershade(0.55, 6.5),
    )
    // The wings thin out away from the body.
    .map(
        move |p| {
            [
                p[0],
                p[1] * (1.0 - 0.8 * smooth(0.12, 0.55, p[2].abs())),
                p[2],
            ]
        },
        |n| n,
    )
    .smooth();
    let horn = extrude(
        &[[0.30, 0.0], [0.42, 0.01], [0.40, 0.03], [0.30, 0.03]],
        0.03,
        [0.6; 3],
    );
    let left = horn
        .clone()
        .map(|p| [p[0], p[1] - 0.02, p[2] + 0.22], |n| n);
    let right = horn.map(|p| [p[0], p[1] - 0.02, p[2] - 0.22], |n| n);
    let tail = lathe(
        &[(0.0, 0.0), (0.012, 0.0), (0.002, 0.55), (0.0, 0.56)],
        5,
        |_, _| 1.0,
        |_, _| [0.5; 3],
        |_| [0.0; 2],
    )
    .map(|p| [-0.30 - p[1], p[0], p[2]], |n| [-n[1], n[0], n[2]]);
    for part in [left, right, tail] {
        m.append(part);
    }
    m.with_aux(|p| {
        [
            1.0 - 2.0 * ((p[2].abs() - 0.15) / 0.85).clamp(0.0, 1.0),
            0.0,
        ]
    })
}

/// A green turtle, under a unit long with the head at +x: a high domed shell
/// of broad scutes, a short neck and a round head, long front flippers to row
/// with and small hind ones to steer, both drooping so they show side on.
fn turtle() -> Mesh {
    let mut m = loft(
        -0.36,
        0.50,
        20,
        12,
        |x| {
            if x < 0.26 {
                let t = (x + 0.36) / 0.62;
                let s = (t * PI).sin().powf(0.5);
                (0.0, 0.17 * s + 0.01, 0.05 * s + 0.01, 0.22 * s + 0.01)
            } else {
                // The neck, then the head swelling out round it.
                let t = (x - 0.26) / 0.24;
                let head = (((t - 0.35) / 0.65).clamp(0.0, 1.0) * PI).sin();
                let r = 0.040 + 0.030 * head;
                (0.015, r, r * 0.8, r * 0.95)
            }
        },
        |x, up| {
            if x >= 0.26 {
                [1.3, 1.2, 0.9]
            } else if up > 0.0 {
                // The scutes, pale at their seams.
                let seam = ((x * 15.0).sin() * (up * 5.0).cos()).abs();
                let v = if seam < 0.10 { 1.5 } else { 0.85 + 0.25 * seam };
                [v, v * 0.95, v * 0.8]
            } else {
                [2.6, 2.4, 1.6]
            }
        },
    );
    let front = extrude(
        &[
            [0.0, 0.0],
            [0.10, 0.06],
            [0.02, 0.48],
            [-0.08, 0.42],
            [-0.06, 0.05],
        ],
        0.018,
        [1.2, 1.15, 0.9],
    )
    .laid_flat();
    let hind = extrude(
        &[[0.0, 0.0], [0.05, 0.03], [-0.03, 0.17], [-0.10, 0.12]],
        0.015,
        [1.2, 1.15, 0.9],
    )
    .laid_flat();
    // Laid flat, the paddles reach out along -z, and droop by `droop`;
    // mirrored for the near side, and turned inside out again, as a mirror
    // turns their winding.
    let place = |part: &Mesh, x: f32, z: f32, droop: f32, mirror: bool| {
        let (s, c) = droop.sin_cos();
        let turn = move |v: [f32; 3]| [v[0], v[1] * c + v[2] * s, v[2] * c - v[1] * s];
        let flip = move |v: [f32; 3]| if mirror { [v[0], v[1], -v[2]] } else { v };
        let mut part = part.clone().map(
            move |p| {
                let q = flip(turn(p));
                [q[0] + x, q[1] - 0.01, q[2] + z]
            },
            move |n| flip(turn(n)),
        );
        if mirror {
            for t in part.indices.as_chunks_mut::<3>().0 {
                t.swap(1, 2);
            }
        }
        part
    };
    let parts = [
        place(&front, 0.14, -0.15, 0.45, false),
        place(&front, 0.14, 0.15, 0.45, true),
        place(&hind, -0.26, -0.13, 0.6, false),
        place(&hind, -0.26, 0.13, 0.6, true),
    ];
    for part in parts {
        m.append(part);
    }
    m.with_aux(|p| [1.0 - 2.0 * ((p[2].abs() - 0.18) / 0.4).clamp(0.0, 1.0), 0.0])
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

/// A sister of the Nautilus, one unit long with the spur at +0.5: a spindle of
/// iron plates, its stern drawn to a point for the screw's shaft, its bow to
/// the ram; a long platform along its back with the pilot's cage forward and
/// the lantern above it, a rudder and diving planes at the stern and inclined
/// planes either side of the bow. Its beam is 0.15, as three metres to twenty.
/// The screw, the glowing ports and the lantern's glass are placed with it.
pub const SUB_RADIUS: f32 = 0.075;
/// Where on the submarine the lantern's lens is, and its screw's hub.
pub const SUB_LANTERN: [f32; 3] = [0.205, SUB_RADIUS + 0.044, 0.0];
pub const SUB_SCREW: [f32; 3] = [-0.488, 0.0, 0.0];

fn submarine() -> Mesh {
    const R: f32 = SUB_RADIUS;
    let ogive = |t: f32| 1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(2);
    let mut m = loft(
        -0.47,
        0.50,
        40,
        16,
        move |x| {
            let r = if x < -0.20 {
                0.010 + (R - 0.010) * ogive((x + 0.47) / 0.27)
            } else if x < 0.20 {
                R
            } else if x < 0.42 {
                R - (R - 0.014) * ((x - 0.20) / 0.22).powf(1.5)
            } else {
                // The spur: a ram of steel, drawn to a point.
                0.014 * (1.0 - (x - 0.42) / 0.08).max(0.08)
            };
            (0.0, r, r, r)
        },
        // Darker below, where no light falls and the weed grows.
        |_, up| {
            let v = 0.82 + 0.18 * up;
            [v, v * 0.98, v * 0.95]
        },
    );
    let dark = [0.55; 3];
    // The platform along the back, a plank deck on the plating.
    m.append(extrude(
        &[
            [-0.22, R - 0.012],
            [0.26, R - 0.012],
            [0.26, R + 0.005],
            [-0.22, R + 0.005],
        ],
        0.050,
        [0.70, 0.62, 0.52],
    ));
    // The pilot's cage, its lenticular glass placed with the ports.
    m.append(extrude(
        &[
            [0.15, R - 0.005],
            [0.25, R - 0.005],
            [0.245, R + 0.022],
            [0.225, R + 0.032],
            [0.16, R + 0.032],
        ],
        0.036,
        [0.75; 3],
    ));
    // The lantern's housing, above the cage and behind its glass.
    m.append(extrude(
        &[
            [0.17, R + 0.03],
            [0.205, R + 0.03],
            [0.205, R + 0.058],
            [0.185, R + 0.062],
            [0.17, R + 0.055],
        ],
        0.026,
        dark,
    ));
    // The rudder above and below the shaft, forward of the screw.
    for up in [1.0f32, -1.0] {
        m.append(
            extrude(
                &[
                    [-0.465, 0.004],
                    [-0.40, 0.004],
                    [-0.425, 0.068],
                    [-0.465, 0.075],
                ],
                0.006,
                dark,
            )
            .map(
                move |p| [p[0], p[1] * up, p[2] * up],
                move |n| [n[0], n[1] * up, n[2] * up],
            ),
        );
    }
    // Diving planes at the stern and inclined planes at the bow, either side.
    for side in [1.0f32, -1.0] {
        let fin = |poly: &[[f32; 2]], root: f32| {
            extrude(poly, 0.006, dark).map(
                // Turned flat, about x: a rotation, so it stays wound outward.
                move |p| [p[0], -p[2] * side, p[1] * side + root * side],
                move |n| [n[0], -n[2] * side, n[1] * side],
            )
        };
        m.append(fin(
            &[[-0.462, 0.0], [-0.405, 0.0], [-0.43, 0.06], [-0.465, 0.065]],
            0.004,
        ));
        m.append(fin(
            &[[0.20, 0.0], [0.27, 0.0], [0.255, 0.045], [0.22, 0.05]],
            R * 0.85,
        ));
    }
    m.with_aux(|p| [p[0] * 2.0, 0.0])
}

/// The submarine's screw, a unit in radius about x: four broad blades,
/// pitched, on a hub. It turns as the instance is rolled.
fn screw() -> Mesh {
    let mut m = loft(
        -0.30,
        0.30,
        6,
        10,
        |x| {
            let r = 0.18 * (1.0 - (x / 0.30).powi(4)).max(0.0).sqrt() + 0.02;
            (0.0, r, r, r)
        },
        |_, _| [0.8; 3],
    );
    let blade = [
        [-0.10, 0.10],
        [0.12, 0.10],
        [0.32, 0.55],
        [0.22, 0.92],
        [-0.02, 1.0],
        [-0.22, 0.72],
    ];
    let pitch: f32 = 0.65;
    for k in 0..4 {
        let theta = k as f32 * TAU / 4.0;
        // Blade space is (across, out, through); turned about `out` by the
        // pitch, then set round the hub: rotations only, so it stays wound
        // outward.
        let (sp, cp) = pitch.sin_cos();
        let (st, ct) = theta.sin_cos();
        let turn = move |p: [f32; 3]| {
            let across = p[0] * cp - p[2] * sp;
            let through = p[0] * sp + p[2] * cp;
            [through, p[1] * ct + across * st, p[1] * st - across * ct]
        };
        m.append(extrude(&blade, 0.06, [1.0; 3]).map(turn, turn));
    }
    m
}

/// All models in one vertex and index buffer, with each model's range.
pub struct Library {
    pub vertices: Vec<Packed>,
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
    /// Vertices this level draws: what the vertex shader runs on.
    pub vertices: u32,
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

/// Reorder a mesh with meshoptimizer: pack the vertices actually referenced,
/// in the order the GPU fetches them, then order triangles for the post-
/// transform cache. The geometry and its winding are unchanged.
fn optimized(mut mesh: Mesh) -> Mesh {
    if mesh.indices.is_empty() || mesh.vertices.is_empty() {
        return mesh;
    }
    let kept = meshopt::optimize_vertex_fetch_in_place(&mut mesh.indices, &mut mesh.vertices);
    mesh.vertices.truncate(kept);
    meshopt::optimize_vertex_cache_in_place(&mut mesh.indices, mesh.vertices.len());
    mesh
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
                let mesh = optimized(model.build());
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
                    vertices: mesh.vertices.len() as u32,
                    error: levels[0].distance(mesh).0.max(mesh.distance(&levels[0]).0),
                    stray: levels[0].distance(mesh).1,
                    edge: mesh.longest_new_edge(&levels[0]),
                });
                vertices.extend(mesh.vertices.iter().map(Packed::from));
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
                    let q = Vec3::from_array(c[0]) * w[0]
                        + Vec3::from_array(c[1]) * w[1]
                        + Vec3::from_array(c[2]) * w[2];
                    ((Vec3::from_array(v.pos) - q).length(), *t, w)
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
                        Vec3::from_array(blend(*t, *w, |v| v.normal))
                            .normalize_or_zero()
                            .dot(Vec3::from_array(v.normal))
                    };
                    facing(a).total_cmp(&facing(b))
                })
                .expect("a triangle");
            let colour = blend(t, w, |v| v.colour);
            let normal = Vec3::from_array(blend(t, w, |v| v.normal)).normalize_or_zero();
            // Colours relative to their brightness: countershading runs past 1.
            let tint = (0..3)
                .map(|k| (colour[k] - v.colour[k]).abs() / colour[k].max(v.colour[k]).max(0.1))
                .fold(0.0, f32::max);
            let n = normal.cross(Vec3::from_array(v.normal));
            stray = stray.max(tint).max(n.length());
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
                let d = Vec3::from_array(self.vertices[a as usize].pos)
                    - Vec3::from_array(self.vertices[b as usize].pos);
                longest = longest.max(d.length());
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
                    let (x, y, z) = (n[0] as f32, n[1] as f32, n[2] as f32);
                    assert!(x * x + y * y + z * z > 0.0, "{model:?} has a broken normal");
                }
            }
        }
    }
}
