//! What stands deep in the world inside, past the port, for the crossing:
//! the wall that closes its far end, the lock's mouth in it; rings braced
//! across the whole cylinder on spokes, which the ship flies through; a
//! wheel of a station turning about the axis; and arcologies, spires rising
//! kilometres from the ground toward the axis. Metres, but for the spire,
//! a unit tall and scaled to its height.

use glam::Vec3;
use std::f32::consts::TAU;

use super::super::super::models::part;
use super::spire::{bands, beacon, tier};
use super::{Builder, parts};

/// The ground's radius and the axis' place across the frame, from the
/// lock's line (`WORLD_GROUND`, `WORLD_AXIS`, the port's `BEND`).
const GROUND: f32 = 4200.0;
const AXIS_FROM_LOCK: f32 = 200.0 - 400.0;
/// The lock's bore, and its mouth in the wall.
const MOUTH: f32 = 140.0;

/// A band round the z axis from radius `r0` to `r1` at depth `z`, facing
/// -z (toward the way the ship comes), `sides` facets.
fn annulus(b: &mut Builder, sides: usize, r0: f32, r1: f32, z: f32, part: u8) {
    for k in 0..sides {
        let (a0, a1) = (
            k as f32 * TAU / sides as f32,
            (k + 1) as f32 * TAU / sides as f32,
        );
        let p = |a: f32, r: f32| Vec3::new(r * a.cos(), r * a.sin(), z);
        b.flat([p(a0, r0), p(a1, r0), p(a1, r1), p(a0, r1)], -Vec3::Z, part);
    }
}

/// A ring about the z axis: a band of `sides` facets, `r` out, `depth`
/// along the axis and `thick` across, its four faces.
fn hoop(b: &mut Builder, sides: usize, r: f32, depth: f32, thick: f32, outer: u8, inner: u8) {
    let (z0, z1) = (-depth * 0.5, depth * 0.5);
    let (ri, ro) = (r - thick * 0.5, r + thick * 0.5);
    for k in 0..sides {
        let (a0, a1) = (
            k as f32 * TAU / sides as f32,
            (k + 1) as f32 * TAU / sides as f32,
        );
        let p = |a: f32, r: f32, z: f32| Vec3::new(r * a.cos(), r * a.sin(), z);
        let mid = (a0 + a1) * 0.5;
        let out = Vec3::new(mid.cos(), mid.sin(), 0.0);
        b.flat(
            [p(a0, ro, z0), p(a1, ro, z0), p(a1, ro, z1), p(a0, ro, z1)],
            out,
            outer,
        );
        b.flat(
            [p(a1, ri, z0), p(a0, ri, z0), p(a0, ri, z1), p(a1, ri, z1)],
            -out,
            inner,
        );
        b.flat(
            [p(a0, ri, z0), p(a1, ri, z0), p(a1, ro, z0), p(a0, ro, z0)],
            -Vec3::Z,
            part::DARK,
        );
        b.flat(
            [p(a1, ri, z1), p(a0, ri, z1), p(a0, ro, z1), p(a1, ro, z1)],
            Vec3::Z,
            part::DARK,
        );
    }
}

/// A beam out from the z axis at angle `a`, from radius `r0` to `r1`,
/// `w` wide; a lit line along its face toward -z.
fn spoke(b: &mut Builder, a: f32, r0: f32, r1: f32, w: f32) {
    let (out, side) = (
        Vec3::new(a.cos(), a.sin(), 0.0),
        Vec3::new(-a.sin(), a.cos(), 0.0),
    );
    let lo = Vec3::new(-w * 0.5, -w * 0.5, -w * 0.5);
    let hi = Vec3::new(w * 0.5, w * 0.5, w * 0.5);
    let corner = |r: f32, s: f32, z: f32| out * r + side * s + Vec3::Z * z;
    let (s0, s1, z0, z1) = (lo.x, hi.x, lo.z, hi.z);
    for (q, n, p) in [
        (
            [
                corner(r0, s0, z0),
                corner(r1, s0, z0),
                corner(r1, s1, z0),
                corner(r0, s1, z0),
            ],
            -Vec3::Z,
            part::HULL,
        ),
        (
            [
                corner(r0, s1, z1),
                corner(r1, s1, z1),
                corner(r1, s0, z1),
                corner(r0, s0, z1),
            ],
            Vec3::Z,
            part::HULL,
        ),
        (
            [
                corner(r0, s1, z0),
                corner(r1, s1, z0),
                corner(r1, s1, z1),
                corner(r0, s1, z1),
            ],
            side,
            part::DARK,
        ),
        (
            [
                corner(r0, s0, z1),
                corner(r1, s0, z1),
                corner(r1, s0, z0),
                corner(r0, s0, z0),
            ],
            -side,
            part::DARK,
        ),
    ] {
        b.flat(q, n, p);
    }
    // Lamps along it.
    let mut r = r0 + 120.0;
    while r < r1 - 60.0 {
        b.flat(
            [
                corner(r, -3.0, z0 - 0.5),
                corner(r + 14.0, -3.0, z0 - 0.5),
                corner(r + 14.0, 3.0, z0 - 0.5),
                corner(r, 3.0, z0 - 0.5),
            ],
            -Vec3::Z,
            parts::LIGHT,
        );
        r += 160.0;
    }
}

/// The far wall: a disc closing the cylinder, the lock's mouth in it on the
/// port's line (off the axis), plates in rings, a ring of lamps round the
/// mouth and others out across it, beams out from the mouth.
pub(super) fn wall(b: &mut Builder) {
    let centre = Vec3::new(AXIS_FROM_LOCK, 0.0, 0.0);
    let edge = GROUND + 150.0;
    let sides = 72;
    // From the mouth out to the cylinder's edge along each way round it.
    let reach = |a: f32| {
        let u = Vec3::new(a.cos(), a.sin(), 0.0);
        let uc = u.dot(centre);
        uc + (uc * uc - centre.length_squared() + edge * edge).sqrt()
    };
    let bandsr = [0.0, 0.03, 0.1, 0.22, 0.4, 0.62, 0.85, 1.0];
    for k in 0..sides {
        let (a0, a1) = (
            k as f32 * TAU / sides as f32,
            (k + 1) as f32 * TAU / sides as f32,
        );
        let p = |a: f32, f: f32| {
            let r = MOUTH + (reach(a) - MOUTH) * f;
            Vec3::new(r * a.cos(), r * a.sin(), 0.0)
        };
        for (i, f) in bandsr.windows(2).enumerate() {
            // Plated all over, a dark seam between bands and every six
            // facets round.
            b.flat(
                [p(a0, f[0]), p(a1, f[0]), p(a1, f[1]), p(a0, f[1])],
                -Vec3::Z,
                part::HULL,
            );
            let lift = Vec3::new(0.0, 0.0, -0.5);
            if i > 0 {
                let out = |a: f32| (p(a, f[0]) - centre).normalize() * 10.0;
                b.flat(
                    [
                        p(a0, f[0]) + lift,
                        p(a1, f[0]) + lift,
                        p(a1, f[0]) + lift + out(a1),
                        p(a0, f[0]) + lift + out(a0),
                    ],
                    -Vec3::Z,
                    part::DARK,
                );
            }
            if k % 6 == 0 {
                let side = Vec3::new(-a0.sin(), a0.cos(), 0.0) * 10.0;
                let (q0, q1) = (p(a0, f[0]) + lift, p(a0, f[1]) + lift);
                b.flat([q0, q1, q1 + side, q0 + side], -Vec3::Z, part::DARK);
            }
            // A lamp at each band's inner edge, every few facets.
            if k % 3 == 0 && i > 0 {
                let (q0, q1) = (p(a0, f[0]), p(a0 + 0.3 * (a1 - a0), f[0]));
                let lift = Vec3::new(0.0, 0.0, -1.0);
                let out = (q0 - centre).normalize() * 18.0;
                b.flat(
                    [q0 + lift, q1 + lift, q1 + lift + out, q0 + lift + out],
                    -Vec3::Z,
                    parts::LIGHT,
                );
            }
        }
    }
    annulus(b, 48, MOUTH, MOUTH + 12.0, -2.0, parts::LIGHT);
    // A cowl round the mouth, out toward the ship.
    tier(
        b,
        [0.0, 0.0],
        24,
        (MOUTH + 60.0, -40.0),
        (MOUTH + 8.0, -1.0),
        0.0,
        1.0,
        |_| part::ACCENT,
        None,
    );
    for k in 0..8 {
        let a = k as f32 * TAU / 8.0 + 0.2;
        let mut q = Builder::new();
        spoke(&mut q, a, MOUTH + 60.0, reach(a) * 0.95, 30.0);
        let shifted = q.vertices.iter().map(|v| {
            let mut v = *v;
            v.position[2] -= 16.0;
            v
        });
        let base = b.vertices.len() as u32;
        b.vertices.extend(shifted);
        b.indices.extend(q.indices.iter().map(|i| i + base));
    }
}

/// A ring braced across the cylinder: a hoop round the axis wide enough to
/// fly through, six spokes out to the ground, lamps along them.
pub(super) fn spokes(b: &mut Builder) {
    hoop(b, 32, 340.0, 60.0, 40.0, part::HULL, part::ACCENT);
    annulus(b, 32, 318.0, 362.0, -31.0, parts::LIGHT);
    for k in 0..6 {
        spoke(b, k as f32 * TAU / 6.0 + 0.26, 360.0, GROUND + 60.0, 34.0);
    }
}

/// A wheel of a station turning about the axis: a rim of habitats, its
/// windows lit, on four spokes from a hub the ship flies through.
pub(super) fn station(b: &mut Builder) {
    hoop(b, 40, 900.0, 140.0, 90.0, part::WINDOWS, part::WINDOWS);
    hoop(b, 40, 900.0, 160.0, 40.0, part::HULL, part::ACCENT);
    hoop(b, 32, 330.0, 90.0, 50.0, part::HULL, part::ACCENT);
    annulus(b, 40, 830.0, 970.0, -80.0, part::DARK);
    annulus(b, 32, 300.0, 360.0, -46.0, parts::LIGHT);
    for k in 0..4 {
        spoke(b, k as f32 * TAU / 4.0 + 0.4, 355.0, 855.0, 40.0);
    }
    // Docking masts on the rim, beacons on them.
    for k in 0..8 {
        let a = k as f32 * TAU / 8.0;
        let at = Vec3::new(a.cos(), a.sin(), 0.0) * 960.0;
        b.tone = super::neon::BEACON;
        b.cuboid(at - Vec3::splat(6.0), at + Vec3::splat(6.0), parts::NEON);
        b.tone = 255;
    }
}

/// An arcology: a spire from the ground most of the way to the axis, in
/// stepped sweeps of lit floors, a crown and a needle, a beacon on it.
pub(super) fn arcology(b: &mut Builder) {
    let tiers = [
        (0.075, 0.0, 0.066, 0.22),
        (0.06, 0.22, 0.048, 0.48),
        (0.044, 0.48, 0.032, 0.7),
        (0.029, 0.7, 0.018, 0.86),
    ];
    for (r0, z0, r1, z1) in tiers {
        tier(
            b,
            [0.0, 0.0],
            8,
            (r0, z0),
            (r1, z1),
            0.0,
            1.0,
            bands,
            Some(part::ACCENT),
        );
        tier(
            b,
            [0.0, 0.0],
            8,
            (r1 * 1.15, z1),
            (r1 * 1.15, z1 + 0.006),
            0.0,
            1.0,
            |_| parts::LIGHT,
            None,
        );
    }
    tier(
        b,
        [0.0, 0.0],
        8,
        (0.018, 0.866),
        (0.002, 0.99),
        0.0,
        1.0,
        |_| part::DARK,
        None,
    );
    beacon(b, 0.975);
    // Buttresses at its foot.
    for k in 0..4 {
        let a = k as f32 * TAU / 4.0 + TAU / 16.0;
        tier(
            b,
            [a.cos() * 0.09, a.sin() * 0.09],
            4,
            (0.02, 0.0),
            (0.004, 0.14),
            a,
            1.0,
            |_| part::HULL,
            None,
        );
    }
}

/// A band round the z axis from radius `r0` to `r1` at depth `z`, facing
/// +z (away from the way the ship comes).
fn annulus_back(b: &mut Builder, sides: usize, r0: f32, r1: f32, z: f32, part: u8) {
    for k in 0..sides {
        let (a0, a1) = (
            k as f32 * TAU / sides as f32,
            (k + 1) as f32 * TAU / sides as f32,
        );
        let p = |a: f32, r: f32| Vec3::new(r * a.cos(), r * a.sin(), z);
        b.flat([p(a1, r0), p(a0, r0), p(a0, r1), p(a1, r1)], Vec3::Z, part);
    }
}

/// A bulkhead's foot: a sill all the way round the ground, a few hundred
/// metres wide along the world, over the seam between two lengths of its
/// map; lamps along both its edges.
pub(super) fn sill(b: &mut Builder) {
    let sides = 120;
    hoop(b, sides, GROUND - 14.0, 380.0, 28.0, part::DARK, part::HULL);
    for z in [-170.0f32, 170.0] {
        let mut q = Builder::new();
        hoop(
            &mut q,
            sides,
            GROUND - 28.6,
            8.0,
            1.0,
            parts::LIGHT,
            parts::LIGHT,
        );
        let base = b.vertices.len() as u32;
        b.vertices.extend(q.vertices.iter().map(|v| {
            let mut v = *v;
            v.position[2] += z;
            v
        }));
        b.indices.extend(q.indices.iter().map(|i| i + base));
    }
}

/// A great gate, every eighth bulkhead: a wall a kilometre and more up
/// from the ground all the way round, rings of lamps on its faces, its rim
/// lit floors, beacons round it; a hoop on spokes about the axis for the
/// ship to fly through.
pub(super) fn gate(b: &mut Builder) {
    let (rim, edge, half) = (2900.0, GROUND + 60.0, 300.0);
    let sides = 120;
    annulus(b, sides, rim, edge, -half, part::HULL);
    annulus_back(b, sides, rim, edge, half, part::HULL);
    // Its rim: rows of lit floors, under a darker lip each side.
    b.tone = super::spire::WINDOWS;
    hoop(
        b,
        sides,
        rim + 20.0,
        half * 2.0 - 40.0,
        40.0,
        part::HULL,
        part::WINDOWS,
    );
    b.tone = 255;
    hoop(b, sides, rim - 5.0, 20.0, 50.0, part::ACCENT, part::ACCENT);
    for r in [3150.0f32, 3450.0, 3800.0] {
        annulus(b, sides, r, r + 14.0, -half - 1.0, parts::LIGHT);
        annulus_back(b, sides, r, r + 14.0, half + 1.0, parts::LIGHT);
        annulus(b, sides, r + 60.0, r + 70.0, -half - 1.0, part::DARK);
    }
    // Beacons round the rim, toward the ship.
    for k in 0..24 {
        let a = k as f32 * TAU / 24.0;
        let at = Vec3::new(a.cos(), a.sin(), 0.0) * (rim + 30.0) - Vec3::Z * (half + 8.0);
        b.tone = super::neon::BEACON;
        b.cuboid(at - Vec3::splat(9.0), at + Vec3::splat(9.0), parts::NEON);
        b.tone = 255;
    }
    hoop(b, 32, 340.0, 60.0, 40.0, part::HULL, part::ACCENT);
    annulus(b, 32, 318.0, 362.0, -31.0, parts::LIGHT);
    for k in 0..6 {
        spoke(b, k as f32 * TAU / 6.0 + 0.26, 360.0, rim, 40.0);
    }
}
