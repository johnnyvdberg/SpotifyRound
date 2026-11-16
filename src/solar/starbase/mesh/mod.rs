//! The starbase's pieces as meshes (`models::Key::Base`), built from flat
//! quads rather than distance fields: a tunnel segment meshed by surface
//! nets would be some 50,000 triangles, and a score are in view; built
//! from its own rings and openings it is under 2,000.
//!
//! Every surface faces the way it is seen from, wound counter-clockwise
//! from that side, as every model is (`fs_model` culls the other). Sizes
//! are metres and the placing's scale 1, but for the craft and the world's
//! towers, which are a unit long and scaled to their length.
//!
//! A vertex carries its part (`models::part`, and `parts` here) and a tone
//! 0..255 the part's shading reads its own way (`base_surface`): which
//! tower a facade belongs to, a neon's colour.

use glam::Vec3;

use super::super::models::{Mesh, ShipVertex};

mod city;
mod craft;
mod entrance;
mod lock;
mod megastructure;
pub mod port;
mod service;
mod spire;
pub mod tunnel;

/// The starbase's pieces (`Key::Base { piece }`).
pub mod piece {
    /// The entrance: the stepped pit down from the mouth, its apron and
    /// the pylons round it.
    pub const ENTRANCE: u32 = 0;
    /// A segment of the tunnel.
    pub const SEGMENT: u32 = 1;
    /// The hub the tunnel comes out of into the port.
    pub const HUB: u32 = 2;
    /// The ship's own dock, and its clamps.
    pub const CRADLE: u32 = 3;
    pub const CLAMP: u32 = 4;
    /// The ship's own bow, seen from its cockpit.
    pub const BOW: u32 = 5;
    /// The megastructure the mouth is in: its rim, underside and stem,
    /// and what stands on its dish.
    pub const STRUCTURE: u32 = 6;
    /// A module of the port's catwalks, and a mooring along one.
    pub const CATWALK: u32 = 7;
    pub const MOORING: u32 = 8;
    /// Craft: a flying car, a freighter, a shuttle and a liner.
    pub const CAR: u32 = 9;
    pub const FREIGHTER: u32 = 10;
    pub const SHUTTLE: u32 = 11;
    pub const LINER: u32 = 12;
    /// The megastructure's dish, its upper face.
    pub const PLATING: u32 = 13;
    /// The airlock: its chamber, and a leaf of its doors.
    pub const LOCK: u32 = 14;
    pub const LEAF: u32 = 15;
    /// The buildings of the world inside: a tower for a town far off,
    /// three for one near, and a patch of low sprawl.
    pub const TOWER_FAR: u32 = 16;
    pub const TOWER_SLAB: u32 = 17;
    pub const TOWER_TAPER: u32 = 18;
    pub const TOWER_ROUND: u32 = 19;
    pub const SPRAWL: u32 = 20;
    /// The city's needles, blocks and works' sheds (`mesh::city`).
    pub const TOWER_NEEDLE: u32 = 21;
    pub const TOWER_BLOCK: u32 = 22;
    pub const SHED: u32 = 23;
    pub const COUNT: u32 = 24;
}

/// Whether `piece` is the megastructure or its entrance, shaded apart from
/// the rest (`models::BASE_SURFACE`).
pub fn outside(piece: u32) -> bool {
    piece == piece::STRUCTURE || piece == piece::ENTRANCE
}

/// Parts of the starbase's own, after `models::part`'s.
pub mod parts {
    /// A lamp, lit by itself.
    pub const LIGHT: u8 = 11;
    /// Yellow and black stripes.
    pub const HAZARD: u8 = 12;
    /// A block's or a tower's face: windows floor by floor, by its tone.
    pub const TOWER: u8 = 13;
    /// The megastructure's dish (`fs_plating`).
    pub const GROUND: u8 = 14;
    /// Neon, its colour by its tone (`neon`).
    pub const NEON: u8 = 15;
}

/// A neon's tone: its colour, or what it does.
pub mod neon {
    pub const CYAN: u8 = 20;
    /// The guide along the tunnel's deck, pulses flowing in.
    pub const GUIDE: u8 = 210;
    /// Red, steady (a tail light) and blinking (a beacon).
    pub const RED: u8 = 238;
    pub const BEACON: u8 = 249;
    /// A dock's status, from its placing (`Instance::look`).
    pub const STATUS: u8 = 255;
}

pub(super) struct Builder {
    vertices: Vec<ShipVertex>,
    indices: Vec<u32>,
    /// The tone of the vertices added from here on.
    pub tone: u8,
}

impl Builder {
    fn new() -> Self {
        Builder {
            vertices: Vec::new(),
            indices: Vec::new(),
            tone: 255,
        }
    }

    fn vertex(&mut self, p: Vec3, n: Vec3, part: u8) -> u32 {
        self.vertices.push(ShipVertex {
            position: p.to_array(),
            normal: n.normalize_or(Vec3::Z).to_array(),
            extra: [self.tone, part, 0, 255],
        });
        self.vertices.len() as u32 - 1
    }

    /// A quad through `q` in order round it, seen from the side its
    /// corners' normals `n` point to.
    pub fn quad(&mut self, q: [Vec3; 4], n: [Vec3; 4], part: u8) {
        let face = (q[1] - q[0]).cross(q[2] - q[0]) + (q[2] - q[0]).cross(q[3] - q[0]);
        let toward = n[0] + n[1] + n[2] + n[3];
        let v = [0, 1, 2, 3].map(|k| self.vertex(q[k], n[k], part));
        let order = if face.dot(toward) >= 0.0 {
            [0, 1, 2, 0, 2, 3]
        } else {
            [0, 2, 1, 0, 3, 2]
        };
        self.indices.extend(order.map(|k| v[k]));
    }

    pub fn flat(&mut self, q: [Vec3; 4], n: Vec3, part: u8) {
        self.quad(q, [n; 4], part);
    }

    /// A box from `lo` to `hi`, seen from outside.
    pub fn cuboid(&mut self, lo: Vec3, hi: Vec3, part: u8) {
        self.faces(lo, hi, [Some(part); 6]);
    }

    /// A box from `lo` to `hi`, a part a face (+x, -x, +y, -y, +z, -z),
    /// those with none left open.
    pub fn faces(&mut self, lo: Vec3, hi: Vec3, parts: [Option<u8>; 6]) {
        let centre = (lo + hi) * 0.5;
        let half = (hi - lo) * 0.5;
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        for (face, part) in parts.into_iter().enumerate() {
            let Some(part) = part else { continue };
            let axis = face / 2;
            let n = axes[axis] * if face % 2 == 0 { 1.0 } else { -1.0 };
            let (u, v) = (axes[(axis + 1) % 3], axes[(axis + 2) % 3]);
            let c = centre + n * half.dot(axes[axis]);
            let (hu, hv) = (u * half.dot(u), v * half.dot(v));
            self.flat(
                [c - hu - hv, c + hu - hv, c + hu + hv, c - hu + hv],
                n,
                part,
            );
        }
    }

    /// A block standing on the plane `z = from` and rising to `to` (toward
    /// -z, out of the base's surface): `half` across along `u` and along
    /// the way square to it, about `centre`. Its sides and its roof.
    pub fn block(
        &mut self,
        centre: Vec3,
        u: Vec3,
        half: [f32; 2],
        span: [f32; 2],
        side: u8,
        roof: u8,
    ) {
        let v = Vec3::Z.cross(u);
        let (a, b) = (u * half[0], v * half[1]);
        let at = |x: f32, y: f32, z: f32| Vec3::new(centre.x, centre.y, z) + a * x + b * y;
        let [from, to] = span;
        for (x0, y0, x1, y1, n) in [
            (1.0, -1.0, 1.0, 1.0, u),
            (-1.0, 1.0, -1.0, -1.0, -u),
            (1.0, 1.0, -1.0, 1.0, v),
            (-1.0, -1.0, 1.0, -1.0, -v),
        ] {
            self.flat(
                [
                    at(x0, y0, from),
                    at(x1, y1, from),
                    at(x1, y1, to),
                    at(x0, y0, to),
                ],
                n,
                side,
            );
        }
        self.flat(
            [
                at(-1.0, -1.0, to),
                at(1.0, -1.0, to),
                at(1.0, 1.0, to),
                at(-1.0, 1.0, to),
            ],
            -Vec3::Z,
            roof,
        );
    }

    fn mesh(self) -> Mesh {
        Mesh {
            vertices: self.vertices,
            indices: self.indices,
        }
    }
}

/// Round the tunnel's axis: the point at radius `r`, angle `a`, depth `z`,
/// and the way in toward the axis there.
pub(super) fn round(r: f32, a: f32, z: f32) -> Vec3 {
    Vec3::new(r * a.cos(), r * a.sin(), z)
}
pub(super) fn inward(a: f32) -> Vec3 {
    -Vec3::new(a.cos(), a.sin(), 0.0)
}

/// The mesh of piece `piece` (`piece::*`).
pub fn build(piece: u32) -> Mesh {
    let mut b = Builder::new();
    match piece {
        piece::ENTRANCE => entrance::entrance(&mut b),
        piece::SEGMENT => tunnel::segment(&mut b),
        piece::HUB => port::hub(&mut b),
        piece::CRADLE => service::cradle(&mut b),
        piece::CLAMP => service::clamp(&mut b),
        piece::BOW => service::bow(&mut b),
        piece::STRUCTURE => megastructure::structure(&mut b),
        piece::CATWALK => port::catwalk(&mut b),
        piece::MOORING => port::mooring(&mut b),
        piece::CAR => craft::car(&mut b),
        piece::FREIGHTER => craft::freighter(&mut b),
        piece::SHUTTLE => craft::shuttle(&mut b),
        piece::PLATING => megastructure::plating(&mut b),
        piece::LOCK => lock::chamber(&mut b),
        piece::LEAF => lock::leaf(&mut b),
        piece::TOWER_FAR => spire::far(&mut b),
        piece::TOWER_SLAB => spire::slab(&mut b),
        piece::TOWER_TAPER => spire::taper(&mut b),
        piece::TOWER_ROUND => spire::round(&mut b),
        piece::SPRAWL => spire::sprawl(&mut b),
        piece::TOWER_NEEDLE => city::needle(&mut b),
        piece::TOWER_BLOCK => city::block(&mut b),
        piece::SHED => city::shed(&mut b),
        _ => craft::liner(&mut b),
    }
    b.mesh()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_piece_is_light_on_triangles_and_faces_where_it_says() {
        for p in 0..piece::COUNT {
            let m = build(p);
            let triangles = m.indices.len() / 3;
            assert!(triangles > 0, "piece {p}");
            let most = 8000;
            assert!(triangles < most, "piece {p}: {triangles} triangles");
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
            // Wound counter-clockwise seen from the side it faces.
            for t in m.indices.chunks(3) {
                let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(m.vertices[t[k] as usize].position));
                let n = Vec3::from(m.vertices[t[0] as usize].normal);
                assert!(
                    (b - a).cross(c - a).dot(n) >= -1e-3,
                    "piece {p}: {a} {b} {c} {n}"
                );
            }
        }
    }
}
