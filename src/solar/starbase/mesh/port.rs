//! The port at the tunnel's end, out in the open of the base's hollow
//! world: the hub the tunnel comes out of, and the catwalks ships moor
//! along (`starbase::port` lays them out).
//!
//! The hub is in its own frame, the tunnel's exit at its origin. A
//! catwalk is a module a hundred metres long along +z from its origin, its
//! deck's top at y = 0. A mooring stands at a catwalk's edge and reaches
//! +x to the ship beside it.

use glam::Vec3;

use super::super::super::models::part;
use super::super::super::tour::starbase::BORE;
use super::tunnel::{DECK, DIVISIONS, deck, division_angle};
use super::{Builder, neon, parts, round};

/// The hub's radius and its depth back along the tunnel.
pub const HUB: (f32, f32) = (620.0, 90.0);
/// A catwalk module's length, and half its deck's width.
pub const CATWALK: (f32, f32) = (100.0, 9.0);
/// How far a mooring reaches from the catwalk's edge to the ship's side.
pub const MOORING: f32 = 26.0;

/// The hub: a drum round the tunnel's end. Its face to the port in rings
/// of plating, windows and lamps, a lit collar round the tunnel's exit;
/// its rim; and its back, seen through the tunnel's last windows.
pub(super) fn hub(b: &mut Builder) {
    let (radius, depth) = HUB;
    let r = BORE as f32;
    let rings: [(f32, f32, u8); 7] = [
        (r, r + 14.0, parts::LIGHT),
        (r + 14.0, 200.0, part::HULL),
        (200.0, 236.0, part::WINDOWS),
        (236.0, 380.0, part::HULL),
        (380.0, 392.0, parts::LIGHT),
        (392.0, 560.0, part::ACCENT),
        (560.0, radius, part::HULL),
    ];
    for i in 0..DIVISIONS {
        let (a0, a1) = (division_angle(i), division_angle(i + 1));
        for &(r0, r1, part) in &rings {
            // Hazard between the collar's lamps.
            let part = if part == parts::LIGHT && r0 == r && i % 4 >= 2 {
                parts::HAZARD
            } else {
                part
            };
            b.flat(
                [
                    round(r0, a0, 0.0),
                    round(r1, a0, 0.0),
                    round(r1, a1, 0.0),
                    round(r0, a1, 0.0),
                ],
                Vec3::Z,
                part,
            );
        }
        let back = if i % 8 < 5 { part::HULL } else { part::ACCENT };
        b.flat(
            [
                round(r + 5.0, a0, -depth),
                round(radius, a0, -depth),
                round(radius, a1, -depth),
                round(r + 5.0, a1, -depth),
            ],
            -Vec3::Z,
            back,
        );
        let out = |a: f32| Vec3::new(a.cos(), a.sin(), 0.0);
        b.quad(
            [
                round(radius, a0, -depth),
                round(radius, a1, -depth),
                round(radius, a1, 0.0),
                round(radius, a0, 0.0),
            ],
            [out(a0), out(a1), out(a1), out(a0)],
            if i % 2 == 0 {
                part::ACCENT
            } else {
                part::WINDOWS
            },
        );
    }
    // The end of the tunnel's deck, closing the bore below it.
    let (floor, _) = deck();
    for i in DECK.0..DECK.1 {
        let (p0, p1) = (
            round(r, division_angle(i), 0.0),
            round(r, division_angle(i + 1), 0.0),
        );
        b.flat(
            [
                p0,
                p1,
                Vec3::new(p1.x, floor, 0.0),
                Vec3::new(p0.x, floor, 0.0),
            ],
            Vec3::Z,
            part::ACCENT,
        );
    }
}

/// A strip between two points, `wide` across, seen from both sides.
fn strut(b: &mut Builder, from: Vec3, to: Vec3, wide: f32, part: u8) {
    let along = (to - from).normalize_or(Vec3::Z);
    let across = along.cross(Vec3::Y).normalize_or(Vec3::X) * wide * 0.5;
    let n = across.cross(along).normalize_or(Vec3::Y);
    for n in [n, -n] {
        b.flat(
            [from - across, from + across, to + across, to - across],
            n,
            part,
        );
    }
    let across = along.cross(Vec3::X).normalize_or(Vec3::Y) * wide * 0.5;
    let n = across.cross(along).normalize_or(Vec3::X);
    for n in [n, -n] {
        b.flat(
            [from - across, from + across, to + across, to - across],
            n,
            part,
        );
    }
}

/// A catwalk module: a deck with lamps along both edges, handrails, a
/// truss slung under it, and a lamp post at its middle.
pub(super) fn catwalk(b: &mut Builder) {
    let v = Vec3::new;
    let (length, half) = CATWALK;
    let s = Some(part::HULL);
    b.faces(
        v(-half, -1.6, 0.0),
        v(half, 0.0, length),
        [s, s, s, Some(part::ACCENT), None, None],
    );
    for side in [-1.0f32, 1.0] {
        let x = side * (half - 0.9);
        b.flat(
            [
                v(x - 0.5, 0.05, 0.0),
                v(x + 0.5, 0.05, 0.0),
                v(x + 0.5, 0.05, length),
                v(x - 0.5, 0.05, length),
            ],
            Vec3::Y,
            parts::LIGHT,
        );
        // The handrail and its posts.
        let x = side * half;
        strut(b, v(x, 3.2, 0.0), v(x, 3.2, length), 0.5, part::ACCENT);
        for k in 0..2 {
            let z = length * (0.25 + 0.5 * k as f32);
            strut(b, v(x, 0.0, z), v(x, 3.2, z), 0.5, part::ACCENT);
        }
        // The truss: braces down to the chord under the deck's middle.
        for k in 0..2 {
            let z = length * 0.5 * k as f32;
            strut(
                b,
                v(x, -1.6, z),
                v(0.0, -15.0, z + length * 0.25),
                1.2,
                part::ACCENT,
            );
            strut(
                b,
                v(0.0, -15.0, z + length * 0.25),
                v(x, -1.6, z + length * 0.5),
                1.2,
                part::ACCENT,
            );
        }
    }
    strut(
        b,
        v(0.0, -15.0, 0.0),
        v(0.0, -15.0, length),
        2.4,
        part::ACCENT,
    );
    // The lamp post.
    let z = length * 0.5;
    strut(
        b,
        v(half - 2.0, 0.0, z),
        v(half - 2.0, 12.0, z),
        0.7,
        part::ACCENT,
    );
    b.faces(
        v(half - 6.0, 12.0, z - 1.5),
        v(half - 1.0, 13.0, z + 1.5),
        [
            Some(part::DARK),
            Some(part::DARK),
            Some(part::DARK),
            Some(parts::LIGHT),
            Some(part::DARK),
            Some(part::DARK),
        ],
    );
}

/// A mooring: a gangway out to the ship's side, a clamp arm either end of
/// it, a cabin where it leaves the catwalk, and a mast with the berth's
/// status lamp (its colour from the placing).
pub(super) fn mooring(b: &mut Builder) {
    let v = Vec3::new;
    let reach = MOORING;
    b.cuboid(v(0.0, -1.0, -2.5), v(reach, 2.5, 2.5), part::HULL);
    b.cuboid(v(reach - 3.0, -4.0, -6.0), v(reach, 6.0, 6.0), part::ACCENT);
    for z in [-34.0f32, 34.0] {
        strut(b, v(0.0, 0.0, z * 0.4), v(reach, 4.0, z), 1.6, part::ACCENT);
        b.cuboid(
            v(reach - 2.0, 1.0, z - 3.0),
            v(reach, 7.0, z + 3.0),
            parts::HAZARD,
        );
    }
    // The cabin.
    let w = Some(part::WINDOWS);
    b.faces(
        v(-7.0, 0.0, -9.0),
        v(0.0, 6.0, 9.0),
        [w, w, Some(part::HULL), None, w, w],
    );
    // The mast and its lamp.
    strut(b, v(-3.5, 6.0, 0.0), v(-3.5, 20.0, 0.0), 0.8, part::ACCENT);
    b.tone = neon::STATUS;
    b.cuboid(v(-5.5, 20.0, -2.0), v(-1.5, 24.0, 2.0), parts::NEON);
    b.tone = 255;
}
