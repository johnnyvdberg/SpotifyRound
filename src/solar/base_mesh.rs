//! The starbase's interior as meshes (`models::Key::Base`), built from flat
//! quads rather than distance fields: the funnel down from the mouth, a
//! segment of the tunnel (repeated along its bend), the bay at its end, the
//! cradle and its clamps, and the service arms' links and tool heads.
//!
//! A tunnel segment as a distance field meshed by surface nets would be
//! some 50,000 triangles, and fourteen are in view; built from its own
//! rings and openings it is about 1,500. Every surface faces the way it is
//! seen from (inward, toward the axis or the bay's middle), wound
//! counter-clockwise from that side, as every model is (`fs_model` culls
//! the other). Sizes are metres, the placing's scale 1 but for the arm's
//! links (a link a metre long, scaled to its length).
//!
//! The window openings are holes: the panes behind them are drawn by
//! `fs_pane` (`interior.rs`), five metres into the wall's depth
//! (`PANE_OUT`), so the wall hides their margins.

use glam::Vec3;
use std::f32::consts::TAU;

use super::models::{Mesh, ShipVertex, part};
use super::tour::starbase::{BORE, FUNNEL, FUNNEL_TOP, SEGMENT};

/// The interior's pieces (`Key::Base { piece }`).
pub mod piece {
    pub const FUNNEL: u32 = 0;
    pub const SEGMENT: u32 = 1;
    pub const BAY: u32 = 2;
    pub const CRADLE: u32 = 3;
    pub const CLAMP: u32 = 4;
    pub const LINK: u32 = 5;
    pub const WELDER: u32 = 6;
    pub const COUPLER: u32 = 7;
    pub const SCANNER: u32 = 8;
    pub const BOW: u32 = 9;
    pub const COUNT: u32 = 10;
}

/// Parts of the interior's own, after `models::part`'s.
pub mod parts {
    /// A light strip or panel, lit by itself.
    pub const LIGHT: u8 = 11;
    /// Yellow and black stripes.
    pub const HAZARD: u8 = 12;
}

/// Divisions round the tunnel's bore, and which of them are windows: three
/// bands of eight (right, overhead and left), each 45° across.
pub const DIVISIONS: usize = 64;
/// The first division's angle: the right band centred on +x.
const FIRST: f32 = -4.0;
pub const BANDS: [(usize, usize); 3] = [(0, 8), (16, 24), (32, 40)];
/// The light strips between the bands, by division, and the guide strip
/// along the floor.
const STRIPS: [usize; 5] = [12, 28, 44, 52, 60];
/// The tunnel wall's depth, from the bore out (the panes lie halfway).
pub const WALL: f32 = 10.0;
/// Each segment: its end ribs, and four bays of a window each.
pub const RIB: f32 = 8.0;
pub const BAYS: usize = 4;
pub const MULLION: f32 = 5.0;
/// The bay at the tunnel's end, about its entrance: across, below and
/// above the tunnel's line, and deep.
pub const HALL: [f32; 4] = [350.0, -130.0, 200.0, 700.0];

/// The angle of division boundary `i`.
pub fn division_angle(i: usize) -> f32 {
    (i as f32 + FIRST) * TAU / DIVISIONS as f32
}

/// Whether division `i` is a window.
fn window(i: usize) -> bool {
    BANDS.iter().any(|&(a, b)| (a..b).contains(&i))
}

/// The bays' openings along a segment: where each begins and ends, metres
/// from its middle.
pub fn openings() -> [(f32, f32); BAYS] {
    let half = SEGMENT as f32 / 2.0 - RIB;
    let bay = 2.0 * half / BAYS as f32;
    std::array::from_fn(|k| {
        let start = -half + bay * k as f32;
        (start + MULLION, start + bay - MULLION)
    })
}

struct Builder {
    vertices: Vec<ShipVertex>,
    indices: Vec<u32>,
}

impl Builder {
    fn new() -> Self {
        Builder {
            vertices: Vec::new(),
            indices: Vec::new(),
        }
    }

    fn vertex(&mut self, p: Vec3, n: Vec3, part: u8) -> u32 {
        self.vertices.push(ShipVertex {
            position: p.to_array(),
            normal: n.normalize_or(Vec3::Z).to_array(),
            extra: [255, part, 0, 255],
        });
        self.vertices.len() as u32 - 1
    }

    /// A quad through `q` in order round it, seen from the side its
    /// corners' normals `n` point to.
    fn quad(&mut self, q: [Vec3; 4], n: [Vec3; 4], part: u8) {
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

    fn flat(&mut self, q: [Vec3; 4], n: Vec3, part: u8) {
        self.quad(q, [n; 4], part);
    }

    /// A box from `lo` to `hi`, seen from outside.
    fn cuboid(&mut self, lo: Vec3, hi: Vec3, part: u8) {
        let c = |x: f32, y: f32, z: f32| Vec3::new(x, y, z);
        let (a, b) = (lo, hi);
        self.flat(
            [
                c(b.x, a.y, a.z),
                c(b.x, b.y, a.z),
                c(b.x, b.y, b.z),
                c(b.x, a.y, b.z),
            ],
            Vec3::X,
            part,
        );
        self.flat(
            [
                c(a.x, a.y, a.z),
                c(a.x, a.y, b.z),
                c(a.x, b.y, b.z),
                c(a.x, b.y, a.z),
            ],
            -Vec3::X,
            part,
        );
        self.flat(
            [
                c(a.x, b.y, a.z),
                c(a.x, b.y, b.z),
                c(b.x, b.y, b.z),
                c(b.x, b.y, a.z),
            ],
            Vec3::Y,
            part,
        );
        self.flat(
            [
                c(a.x, a.y, a.z),
                c(b.x, a.y, a.z),
                c(b.x, a.y, b.z),
                c(a.x, a.y, b.z),
            ],
            -Vec3::Y,
            part,
        );
        self.flat(
            [
                c(a.x, a.y, b.z),
                c(b.x, a.y, b.z),
                c(b.x, b.y, b.z),
                c(a.x, b.y, b.z),
            ],
            Vec3::Z,
            part,
        );
        self.flat(
            [
                c(a.x, a.y, a.z),
                c(a.x, b.y, a.z),
                c(b.x, b.y, a.z),
                c(b.x, a.y, a.z),
            ],
            -Vec3::Z,
            part,
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
fn round(r: f32, a: f32, z: f32) -> Vec3 {
    Vec3::new(r * a.cos(), r * a.sin(), z)
}
fn inward(a: f32) -> Vec3 {
    -Vec3::new(a.cos(), a.sin(), 0.0)
}

/// The mesh of piece `piece` (`piece::*`).
pub fn mesh(piece: u32) -> Mesh {
    let mut b = Builder::new();
    match piece {
        piece::FUNNEL => funnel(&mut b),
        piece::SEGMENT => segment(&mut b),
        piece::BAY => bay(&mut b),
        piece::CRADLE => cradle(&mut b),
        piece::CLAMP => clamp(&mut b),
        piece::LINK => link(&mut b),
        piece::WELDER => head(&mut b, 0),
        piece::COUPLER => head(&mut b, 1),
        piece::BOW => bow(&mut b),
        _ => head(&mut b, 2),
    }
    b.mesh()
}

/// The funnel: from the mouth, `FUNNEL_TOP` in radius, down a bell `FUNNEL`
/// deep to the tunnel's bore; rings of lights dashed round it; a broad
/// apron round its rim, a hazard ring at its edge.
fn funnel(b: &mut Builder) {
    let depth = FUNNEL as f32;
    let top = FUNNEL_TOP as f32;
    let bore = BORE as f32;
    let radius = |z: f32| bore + (top - bore) * (1.0 - z / depth).powi(2);
    let slope = |z: f32| -2.0 * (top - bore) * (1.0 - z / depth) / depth;
    let rows = 24;
    for k in 0..rows {
        let (z0, z1) = (
            depth * k as f32 / rows as f32,
            depth * (k + 1) as f32 / rows as f32,
        );
        for i in 0..DIVISIONS {
            let (a0, a1) = (division_angle(i), division_angle(i + 1));
            let normal = |a: f32, z: f32| -Vec3::new(a.cos(), a.sin(), -slope(z));
            let lit = k % 4 == 2 && i % 4 < 2;
            let part = if k == 0 {
                parts::HAZARD
            } else if lit {
                parts::LIGHT
            } else {
                part::HULL
            };
            b.quad(
                [
                    round(radius(z0), a0, z0),
                    round(radius(z0), a1, z0),
                    round(radius(z1), a1, z1),
                    round(radius(z1), a0, z1),
                ],
                [
                    normal(a0, z0),
                    normal(a1, z0),
                    normal(a1, z1),
                    normal(a0, z1),
                ],
                part,
            );
        }
    }
    // The apron round the rim, facing out of the mouth.
    for i in 0..DIVISIONS {
        let (a0, a1) = (division_angle(i), division_angle(i + 1));
        for (r0, r1, part) in [
            (top, top + 60.0, parts::HAZARD),
            (top + 60.0, top * 1.8, part::HULL),
        ] {
            b.flat(
                [
                    round(r0, a0, 0.0),
                    round(r1, a0, 0.0),
                    round(r1, a1, 0.0),
                    round(r0, a1, 0.0),
                ],
                -Vec3::Z,
                part,
            );
        }
    }
}

/// A segment of the tunnel, `SEGMENT` long round its middle: the bore's
/// wall, its window openings (four bays to a band) with their reveals,
/// a rib at each end, and light strips between the bands.
fn segment(b: &mut Builder) {
    let r = BORE as f32;
    let half = SEGMENT as f32 / 2.0;
    let inner = half - RIB;
    let rib_r = r - 12.0;
    for i in 0..DIVISIONS {
        let (a0, a1) = (division_angle(i), division_angle(i + 1));
        let (n0, n1) = (inward(a0), inward(a1));
        // The ribs at either end: their inner face and their side.
        for sign in [-1.0f32, 1.0] {
            let (za, zb) = (sign * inner, sign * half);
            b.quad(
                [
                    round(rib_r, a0, za),
                    round(rib_r, a1, za),
                    round(rib_r, a1, zb),
                    round(rib_r, a0, zb),
                ],
                [n0, n1, n1, n0],
                part::ACCENT,
            );
            b.flat(
                [
                    round(r, a0, za),
                    round(r, a1, za),
                    round(rib_r, a1, za),
                    round(rib_r, a0, za),
                ],
                Vec3::new(0.0, 0.0, -sign),
                part::ACCENT,
            );
        }
        let wall = |b: &mut Builder, z0: f32, z1: f32, part: u8| {
            b.quad(
                [
                    round(r, a0, z0),
                    round(r, a1, z0),
                    round(r, a1, z1),
                    round(r, a0, z1),
                ],
                [n0, n1, n1, n0],
                part,
            );
        };
        if !window(i) {
            wall(b, -inner, inner, part::HULL);
            continue;
        }
        // A window band: mullions between the openings, and each opening's
        // reveals across its ends.
        let mut z = -inner;
        for (o0, o1) in openings() {
            wall(b, z, o0, part::DARK);
            for (at, facing) in [(o0, 1.0f32), (o1, -1.0)] {
                b.flat(
                    [
                        round(r, a0, at),
                        round(r, a1, at),
                        round(r + WALL, a1, at),
                        round(r + WALL, a0, at),
                    ],
                    Vec3::new(0.0, 0.0, facing),
                    part::DARK,
                );
            }
            z = o1;
        }
        wall(b, z, inner, part::DARK);
    }
    // The reveals along each band's sides.
    for &(first, last) in &BANDS {
        for (i, facing) in [(first, 1.0f32), (last, -1.0)] {
            let a = division_angle(i);
            let tangent = Vec3::new(-a.sin(), a.cos(), 0.0) * facing;
            for (o0, o1) in openings() {
                b.flat(
                    [
                        round(r, a, o0),
                        round(r + WALL, a, o0),
                        round(r + WALL, a, o1),
                        round(r, a, o1),
                    ],
                    tangent,
                    part::DARK,
                );
            }
        }
    }
    // Light strips along the tunnel between the bands, and the guide along
    // its floor.
    for &i in &STRIPS {
        let a = division_angle(i);
        let half_width = 1.6 / r;
        let (s0, s1) = (a - half_width, a + half_width);
        let depth = 3.0;
        b.quad(
            [
                round(r - depth, s0, -inner),
                round(r - depth, s1, -inner),
                round(r - depth, s1, inner),
                round(r - depth, s0, inner),
            ],
            [inward(a); 4],
            parts::LIGHT,
        );
        for (edge, facing) in [(s0, -1.0f32), (s1, 1.0)] {
            let tangent = Vec3::new(-edge.sin(), edge.cos(), 0.0) * facing;
            b.flat(
                [
                    round(r, edge, -inner),
                    round(r - depth, edge, -inner),
                    round(r - depth, edge, inner),
                    round(r, edge, inner),
                ],
                tangent,
                part::DARK,
            );
        }
    }
}

/// The bay: a hall round the tunnel's mouth into it (`HALL`), its entrance
/// wall fanned from the bore out to the hall's edges; floor, ceiling,
/// walls; gantries across under the ceiling, light panels in it, the
/// control rooms' lit windows along the walls.
fn bay(b: &mut Builder) {
    let [w, floor, ceiling, deep] = HALL;
    let r = BORE as f32;
    let v = Vec3::new;
    // The entrance wall, from the bore's edge out to the hall's: the
    // tunnel's divisions and the hall's corners, so its edges are exact.
    let corners = [
        v(w, floor, 0.0),
        v(w, ceiling, 0.0),
        v(-w, ceiling, 0.0),
        v(-w, floor, 0.0),
    ];
    let mut angles: Vec<f32> = (0..DIVISIONS).map(division_angle).collect();
    angles.extend(corners.iter().map(|c| c.y.atan2(c.x)));
    let wrap = |a: f32| a.rem_euclid(TAU);
    angles.sort_by(|a, b| wrap(*a).total_cmp(&wrap(*b)));
    let hit = |a: f32| {
        let d = glam::Vec2::new(a.cos(), a.sin());
        let tx = if d.x.abs() > 1e-6 {
            w / d.x.abs()
        } else {
            f32::MAX
        };
        let ty = if d.y > 1e-6 {
            ceiling / d.y
        } else if d.y < -1e-6 {
            floor / d.y
        } else {
            f32::MAX
        };
        let t = tx.min(ty);
        v(d.x * t, d.y * t, 0.0)
    };
    for k in 0..angles.len() {
        let (a0, a1) = (angles[k], angles[(k + 1) % angles.len()]);
        b.flat(
            [round(r, a0, 0.0), hit(a0), hit(a1), round(r, a1, 0.0)],
            Vec3::Z,
            part::HULL,
        );
    }
    // Floor, ceiling, sides and the far wall, in tiles.
    let tiles = 6;
    for i in 0..tiles {
        for j in 0..tiles {
            let (u0, u1) = (i as f32 / tiles as f32, (i + 1) as f32 / tiles as f32);
            let (s0, s1) = (j as f32 / tiles as f32, (j + 1) as f32 / tiles as f32);
            let x = |u: f32| -w + 2.0 * w * u;
            let y = |u: f32| floor + (ceiling - floor) * u;
            let z = |u: f32| deep * u;
            b.flat(
                [
                    v(x(u0), floor, z(s0)),
                    v(x(u1), floor, z(s0)),
                    v(x(u1), floor, z(s1)),
                    v(x(u0), floor, z(s1)),
                ],
                Vec3::Y,
                part::HULL,
            );
            b.flat(
                [
                    v(x(u0), ceiling, z(s0)),
                    v(x(u0), ceiling, z(s1)),
                    v(x(u1), ceiling, z(s1)),
                    v(x(u1), ceiling, z(s0)),
                ],
                -Vec3::Y,
                part::DARK,
            );
            for side in [-1.0f32, 1.0] {
                b.flat(
                    [
                        v(side * w, y(u0), z(s0)),
                        v(side * w, y(u1), z(s0)),
                        v(side * w, y(u1), z(s1)),
                        v(side * w, y(u0), z(s1)),
                    ],
                    Vec3::new(-side, 0.0, 0.0),
                    part::HULL,
                );
            }
            b.flat(
                [
                    v(x(u0), y(s0), deep),
                    v(x(u1), y(s0), deep),
                    v(x(u1), y(s1), deep),
                    v(x(u0), y(s1), deep),
                ],
                -Vec3::Z,
                part::HULL,
            );
        }
    }
    // Gantries across the hall, and light panels between them.
    for zc in [120.0, 300.0, 480.0, 620.0] {
        b.cuboid(
            v(-w, ceiling - 40.0, zc - 6.0),
            v(w, ceiling - 28.0, zc + 6.0),
            part::ACCENT,
        );
    }
    for i in 0..5 {
        for j in 0..4 {
            let x = -w + 70.0 + 140.0 * i as f32;
            let z = 60.0 + 170.0 * j as f32;
            b.cuboid(
                v(x - 30.0, ceiling - 2.0, z - 8.0),
                v(x + 30.0, ceiling, z + 8.0),
                parts::LIGHT,
            );
        }
    }
    // The control rooms' windows, a band along each side wall.
    for side in [-1.0f32, 1.0] {
        let x = side * (w - 0.5);
        b.flat(
            [
                v(x, 60.0, 150.0),
                v(x, 90.0, 150.0),
                v(x, 90.0, 560.0),
                v(x, 60.0, 560.0),
            ],
            Vec3::new(-side, 0.0, 0.0),
            part::WINDOWS,
        );
    }
    // Hazard stripes on the floor round the cradle's foot.
    let cz = super::tour::starbase::CRADLE as f32;
    for (x0, x1, z0, z1) in [
        (-60.0, 60.0, cz - 64.0, cz - 58.0),
        (-60.0, 60.0, cz + 58.0, cz + 64.0),
        (-64.0, -58.0, cz - 64.0, cz + 64.0),
        (58.0, 64.0, cz - 64.0, cz + 64.0),
    ] {
        b.flat(
            [
                v(x0, floor + 0.3, z0),
                v(x1, floor + 0.3, z0),
                v(x1, floor + 0.3, z1),
                v(x0, floor + 0.3, z1),
            ],
            Vec3::Y,
            parts::HAZARD,
        );
    }
}

/// The cradle the ship rests in: a column from the floor, a platform with
/// hazard-striped edges.
fn cradle(b: &mut Builder) {
    let v = Vec3::new;
    let floor = HALL[1];
    b.cuboid(v(-12.0, floor, -12.0), v(12.0, -40.0, 12.0), part::ACCENT);
    b.cuboid(v(-35.0, -40.0, -45.0), v(35.0, -34.0, 45.0), part::HULL);
    for (x0, x1, z0, z1) in [(-35.0, 35.0, -45.0, -42.0), (-35.0, 35.0, 42.0, 45.0)] {
        b.cuboid(v(x0, -34.0, z0), v(x1, -33.5, z1), parts::HAZARD);
    }
}

/// A clamp, hinged at its foot: an upright, and a jaw reaching in along
/// +x over the hull.
fn clamp(b: &mut Builder) {
    let v = Vec3::new;
    b.cuboid(v(-2.0, 0.0, -3.0), v(2.0, 22.0, 3.0), part::ACCENT);
    b.cuboid(v(-2.0, 18.0, -3.0), v(10.0, 22.0, 3.0), part::ACCENT);
    b.cuboid(v(-2.2, 8.0, -3.2), v(2.2, 11.0, 3.2), parts::HAZARD);
}

/// The ship's own bow as seen from its cockpit, the camera at the origin
/// looking along +z: a deck tapering ahead below the eye, sloping sides, a
/// stripe down its spine and running lights at its shoulders.
fn bow(b: &mut Builder) {
    let v = Vec3::new;
    let (near, far) = (2.0, 30.0);
    let (wide, narrow) = (9.0, 1.5);
    let (top, bottom) = (-4.0, -11.0);
    let deck = [
        v(-wide, top, near),
        v(wide, top, near),
        v(narrow, top, far),
        v(-narrow, top, far),
    ];
    b.flat(deck, Vec3::Y, part::HULL);
    b.flat(
        [
            v(-0.6, top + 0.05, near),
            v(0.6, top + 0.05, near),
            v(0.3, top + 0.05, far),
            v(-0.3, top + 0.05, far),
        ],
        Vec3::Y,
        part::ACCENT,
    );
    for side in [-1.0f32, 1.0] {
        let n = v(side * 0.8, 0.3, 0.1).normalize();
        b.flat(
            [
                v(side * wide, top, near),
                v(side * narrow, top, far),
                v(side * narrow * 0.5, bottom, far),
                v(side * wide * 0.8, bottom, near),
            ],
            n,
            part::HULL,
        );
        b.cuboid(
            v(side * wide - 0.4, top, near + 1.0),
            v(side * wide + 0.4, top + 0.6, near + 2.0),
            parts::LIGHT,
        );
    }
    b.flat(
        [
            v(-narrow, top, far),
            v(narrow, top, far),
            v(narrow * 0.5, bottom, far),
            v(-narrow * 0.5, bottom, far),
        ],
        Vec3::Z,
        part::ACCENT,
    );
}

/// An arm's link: a metre long along +z, scaled to its length, with a
/// knuckle at its root.
fn link(b: &mut Builder) {
    let v = Vec3::new;
    b.cuboid(v(-0.022, -0.022, 0.0), v(0.022, 0.022, 1.0), part::HULL);
    b.cuboid(v(-0.04, -0.04, -0.03), v(0.04, 0.04, 0.04), part::ACCENT);
}

/// A tool head, along +z from its root: a welder (0) with its glowing
/// tip, a fuel coupler (1) in hazard stripes, a scanner (2) with its bar.
fn head(b: &mut Builder, tool: u32) {
    let v = Vec3::new;
    b.cuboid(v(-1.5, -1.5, 0.0), v(1.5, 1.5, 4.0), part::ACCENT);
    match tool {
        0 => {
            b.cuboid(v(-0.4, -0.4, 4.0), v(0.4, 0.4, 6.0), part::DARK);
            b.cuboid(v(-0.25, -0.25, 6.0), v(0.25, 0.25, 6.6), parts::LIGHT);
        }
        1 => {
            b.cuboid(v(-1.0, -1.0, 4.0), v(1.0, 1.0, 7.0), parts::HAZARD);
        }
        _ => {
            b.cuboid(v(-5.0, -0.4, 4.0), v(5.0, 0.4, 5.0), parts::LIGHT);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_interior_is_light_on_triangles() {
        for p in 0..piece::COUNT {
            let m = mesh(p);
            let triangles = m.indices.len() / 3;
            assert!(triangles > 0, "piece {p}");
            assert!(triangles < 8000, "piece {p}: {triangles} triangles");
            assert!(m.indices.iter().all(|&i| (i as usize) < m.vertices.len()));
        }
        // The windows: three bands, and four bays between the ribs.
        let o = openings();
        assert!(o[0].0 > -(SEGMENT as f32) / 2.0 + RIB);
        assert!(o[BAYS - 1].1 < SEGMENT as f32 / 2.0 - RIB);
    }

    #[test]
    fn the_tunnel_wall_faces_its_axis() {
        let m = mesh(piece::SEGMENT);
        for t in m.indices.chunks(3) {
            let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(m.vertices[t[k] as usize].position));
            let n = Vec3::from(m.vertices[t[0] as usize].normal);
            // Wound counter-clockwise seen from the side it faces.
            assert!((b - a).cross(c - a).dot(n) >= -1e-3, "{a} {b} {c} {n}");
        }
    }
}
