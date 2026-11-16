//! The airlock at the head of the tunnel: a chamber the length of two of
//! the tunnel's segments, windowless, the tunnel's deck through it, rings
//! of lamps along it and a ring of status lamps at each end (their colour
//! from the placing); and a leaf of its doors, half a disc across the
//! bore, two to a door, drawn apart to open it.

use glam::Vec3;
use std::f32::consts::PI;

use super::super::super::models::part;
use super::super::super::tour::starbase::{BORE, LOCK_SEGMENTS, SEGMENT};
use super::tunnel::{DECK, DIVISIONS, deck, division_angle};
use super::{Builder, inward, neon, parts, round};

/// The chamber, about its middle.
pub(super) fn chamber(b: &mut Builder) {
    let r = BORE as f32;
    let half = SEGMENT as f32 * LOCK_SEGMENTS as f32 * 0.5;
    let v = Vec3::new;
    // Bands along it: hazard by each door, status lamps, plating between
    // ribs with rings of lamps.
    let mut bands: Vec<(f32, f32, f32, u8)> = Vec::new();
    for sign in [-1.0f32, 1.0] {
        bands.push((sign * half, sign * (half - 10.0), r, parts::HAZARD));
        bands.push((
            sign * (half - 10.0),
            sign * (half - 16.0),
            r - 1.0,
            parts::NEON,
        ));
    }
    let ribs = 5;
    let span = 2.0 * (half - 16.0);
    for k in 0..ribs {
        let z0 = -(half - 16.0) + span * k as f32 / ribs as f32;
        let z1 = z0 + span / ribs as f32;
        bands.push((z0, z0 + 6.0, r - 8.0, part::ACCENT));
        bands.push((z0 + 6.0, z0 + 8.5, r - 8.4, parts::LIGHT));
        bands.push((
            z0 + 8.5,
            z1,
            r,
            if k % 2 == 0 { part::HULL } else { part::ACCENT },
        ));
    }
    for i in 0..DIVISIONS {
        if (DECK.0..DECK.1).contains(&i) {
            continue;
        }
        let (a0, a1) = (division_angle(i), division_angle(i + 1));
        for &(z0, z1, at, part) in &bands {
            b.tone = if part == parts::NEON {
                neon::STATUS
            } else {
                255
            };
            b.quad(
                [
                    round(at, a0, z0),
                    round(at, a1, z0),
                    round(at, a1, z1),
                    round(at, a0, z1),
                ],
                [inward(a0), inward(a1), inward(a1), inward(a0)],
                part,
            );
        }
    }
    b.tone = 255;
    let (y, w) = deck();
    b.flat(
        [
            v(-w, y, -half),
            v(w, y, -half),
            v(w, y, half),
            v(-w, y, half),
        ],
        Vec3::Y,
        part::HULL,
    );
    b.tone = neon::GUIDE;
    b.flat(
        [
            v(-1.5, y + 0.25, -half),
            v(1.5, y + 0.25, -half),
            v(1.5, y + 0.25, half),
            v(-1.5, y + 0.25, half),
        ],
        Vec3::Y,
        parts::NEON,
    );
    b.tone = 255;
}

/// A door's leaf: the half of a disc across the bore on its +x side, seen
/// from both sides, a hazard band and a line of lamps down its meeting
/// edge.
pub(super) fn leaf(b: &mut Builder) {
    let r = BORE as f32 + 3.0;
    let arcs = 16;
    for facing in [-1.0f32, 1.0] {
        let z = facing * 1.5;
        let n = Vec3::Z * facing;
        for i in 0..arcs {
            let (a0, a1) = (
                -PI / 2.0 + PI * i as f32 / arcs as f32,
                -PI / 2.0 + PI * (i + 1) as f32 / arcs as f32,
            );
            let (y0, y1) = (r * a0.sin(), r * a1.sin());
            let (x0, x1) = (r * a0.cos(), r * a1.cos());
            // Lamps down the meeting edge, a hazard band beside them, then
            // its panels: each as far as the leaf reaches at this height.
            let at = |x: f32, y: f32, reach: f32| Vec3::new(x.min(reach), y, z);
            let panel = if i % 4 < 2 { part::HULL } else { part::ACCENT };
            for (from, to, part) in [
                (0.0, 2.0, parts::LIGHT),
                (2.0, 9.0, parts::HAZARD),
                (9.0, r, panel),
            ] {
                b.flat(
                    [
                        at(from, y0, x0),
                        at(to, y0, x0),
                        at(to, y1, x1),
                        at(from, y1, x1),
                    ],
                    n,
                    part,
                );
            }
        }
    }
}
