//! A segment of the tunnel, repeated along its bend: the bore's wall, three
//! bands of windows on the hollow outside (right, overhead and left), a
//! rib at each end with a ring of lamps round it, light strips between the
//! bands, and a deck along the bottom: a roadway with its lanes, a guide
//! line down its middle and lamp posts along its kerbs.
//!
//! The window openings are holes: the panes behind them are drawn by
//! `fs_pane`, two metres into the wall's depth, so the wall hides their
//! margins.

use glam::Vec3;
use std::f32::consts::TAU;

use super::super::super::models::part;
use super::super::super::tour::starbase::{BORE, SEGMENT};
use super::{Builder, inward, neon, parts, round};

/// Divisions round the tunnel's bore, and which of them are windows: three
/// bands of twelve (right, overhead and left), each 67.5° across, so more
/// of the wall is glass than not.
pub const DIVISIONS: usize = 64;
/// The first division's angle: the right band centred on +x.
const FIRST: f32 = -6.0;
pub const BANDS: [(usize, usize); 3] = [(0, 12), (16, 28), (32, 44)];
/// The light strips between the bands, by division, and the two along the
/// deck's kerbs.
const STRIPS: [usize; 4] = [14, 30, 46, 62];
/// The divisions the deck covers: the wall under it is left out.
pub const DECK: (usize, usize) = (48, 60);
/// The tunnel wall's depth, from the bore out (the panes lie halfway):
/// thin, as the windows are seen at a glance, down the tunnel, where a
/// deep reveal would hide them.
pub const WALL: f32 = 4.0;
/// Each segment: its end ribs, and two long bays of a window each, half a
/// mullion's width in from each end.
pub const RIB: f32 = 8.0;
pub const BAYS: usize = 2;
pub const MULLION: f32 = 1.5;

/// The angle of division boundary `i`.
pub fn division_angle(i: usize) -> f32 {
    (i as f32 + FIRST) * TAU / DIVISIONS as f32
}

/// Whether division `i` is a window.
fn window(i: usize) -> bool {
    BANDS.iter().any(|&(a, b)| (a..b).contains(&i))
}

/// The deck's height, and half its width.
pub fn deck() -> (f32, f32) {
    let a = division_angle(DECK.0);
    (BORE as f32 * a.sin(), (BORE as f32 * a.cos()).abs())
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

pub(super) fn segment(b: &mut Builder) {
    let r = BORE as f32;
    let half = SEGMENT as f32 / 2.0;
    let inner = half - RIB;
    let rib_r = r - 12.0;
    let v = Vec3::new;
    for i in 0..DIVISIONS {
        let (a0, a1) = (division_angle(i), division_angle(i + 1));
        let (n0, n1) = (inward(a0), inward(a1));
        let under = (DECK.0 + 1..DECK.1 - 1).contains(&i);
        // The ribs at either end: their inner face, a ring of lamps round
        // it, and their side.
        for sign in [-1.0f32, 1.0] {
            if under {
                continue;
            }
            let (za, zb) = (sign * inner, sign * half);
            let ring = |b: &mut Builder, z0: f32, z1: f32, at: f32, part: u8| {
                b.quad(
                    [
                        round(at, a0, z0),
                        round(at, a1, z0),
                        round(at, a1, z1),
                        round(at, a0, z1),
                    ],
                    [n0, n1, n1, n0],
                    part,
                );
            };
            ring(b, za, zb, rib_r, part::ACCENT);
            if i % 4 != 3 {
                ring(
                    b,
                    sign * (inner + 2.5),
                    sign * (inner + 5.0),
                    rib_r - 0.4,
                    parts::LIGHT,
                );
            }
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
        if (DECK.0..DECK.1).contains(&i) {
            continue;
        }
        if !window(i) {
            // Darker trim either side of each light strip.
            let trim = STRIPS.iter().any(|&s| i + 2 == s || i == s + 1);
            wall(
                b,
                -inner,
                inner,
                if trim { part::ACCENT } else { part::HULL },
            );
            continue;
        }
        // A window band: mullions between the openings, and each opening's
        // reveals across its ends.
        let mut z = -inner;
        for (o0, o1) in openings() {
            wall(b, z, o0, part::ACCENT);
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
        wall(b, z, inner, part::ACCENT);
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
    // Light strips along the tunnel between the bands and over the kerbs.
    for &i in &STRIPS {
        let a = division_angle(i);
        let half_width = 1.8 / r;
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
    // The deck: walkways either side of a roadway, its lanes dashed, the
    // guide line down its middle.
    let (y, w) = deck();
    let road = w - 20.0;
    let strip = |b: &mut Builder, x0: f32, x1: f32, z0: f32, z1: f32, lift: f32, part: u8| {
        let y = y + lift;
        b.flat(
            [v(x0, y, z0), v(x1, y, z0), v(x1, y, z1), v(x0, y, z1)],
            Vec3::Y,
            part,
        );
    };
    strip(b, -w, -road, -half, half, 0.0, part::ACCENT);
    strip(b, road, w, -half, half, 0.0, part::ACCENT);
    strip(b, -road, road, -half, half, 0.0, part::HULL);
    b.tone = neon::GUIDE;
    strip(b, -1.5, 1.5, -half, half, 0.25, parts::NEON);
    b.tone = 255;
    for k in 0..4 {
        let z = -half + SEGMENT as f32 * (k as f32 + 0.5) / 4.0;
        for x in [-road * 0.5, road * 0.5] {
            strip(b, x - 0.7, x + 0.7, z - 9.0, z + 9.0, 0.25, parts::LIGHT);
        }
        // A lamp post on each kerb.
        for side in [-1.0f32, 1.0] {
            let x = side * (road + 3.0);
            b.cuboid(
                v(x - 0.5, y, z - 0.5),
                v(x + 0.5, y + 11.0, z + 0.5),
                part::ACCENT,
            );
            b.faces(
                v(x - 2.0 - side * 2.0, y + 11.0, z - 1.5),
                v(x + 2.0 - side * 2.0, y + 12.0, z + 1.5),
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
    }
    // The kerbs.
    for side in [-1.0f32, 1.0] {
        let x = side * road;
        b.cuboid(v(x - 0.6, y, -half), v(x + 0.6, y + 1.2, half), part::DARK);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_windows_lie_between_the_ribs() {
        let o = openings();
        assert!(o[0].0 > -(SEGMENT as f32) / 2.0 + RIB);
        assert!(o[BAYS - 1].1 < SEGMENT as f32 / 2.0 - RIB);
        // The deck lies below the tunnel's line, inside its bore.
        let (y, w) = deck();
        assert!(y < -80.0 && y > -(BORE as f32));
        assert!((y * y + w * w).sqrt() <= BORE as f32 + 0.01);
    }
}
