//! The megastructure the base's mouth is in, standing thirty kilometres
//! over its world: a dish sixty across, the entrance at the middle of its
//! upper face; a rim of lit floors round it; its underside drawn in to a
//! stem that goes down to the ground. The dish's face is a map
//! (`surface_texel`, drawn by `fs_plating` on `plating`'s disc); the rest
//! is `structure`: the rim, the underside and the stem, floors of windows
//! all over them, bands of light where they join, and housings and masts
//! standing on the dish.
//!
//! The same at every base.

use glam::Vec3;
use std::f32::consts::TAU;

use super::super::super::models::part;
use super::super::super::system::Rng;
use super::super::super::tour::starbase::{CITY, ELEVATION};
use super::entrance::APRON;
use super::{Builder, neon, parts, round};

/// Round it, in divisions.
const AROUND: usize = 96;
/// The rim's depth below the dish's face; the stem's radius, and how far
/// down the underside meets it.
const RIM: f32 = 1800.0;
const STEM: (f32, f32) = (8000.0, 15_000.0);

/// The disc the dish's face is laid on, in rings out from the apron.
pub(super) fn plating(b: &mut Builder) {
    let rings = [
        APRON,
        2500.0,
        5000.0,
        9000.0,
        14_000.0,
        20_000.0,
        26_000.0,
        CITY as f32,
    ];
    let around = 48;
    for pair in rings.windows(2) {
        for i in 0..around {
            let (a0, a1) = (
                i as f32 * TAU / around as f32,
                (i + 1) as f32 * TAU / around as f32,
            );
            b.flat(
                [
                    round(pair[0], a0, 0.0),
                    round(pair[1], a0, 0.0),
                    round(pair[1], a1, 0.0),
                    round(pair[0], a1, 0.0),
                ],
                -Vec3::Z,
                parts::GROUND,
            );
        }
    }
}

/// A band round the structure from radius `r0` at depth `z0` to `r1` at
/// `z1`, seen from outside.
fn band(b: &mut Builder, (r0, z0): (f32, f32), (r1, z1): (f32, f32), part: u8) {
    // Outward, and along the slope's own normal.
    let (dr, dz) = (r1 - r0, z1 - z0);
    for i in 0..AROUND {
        let (a0, a1) = (
            i as f32 * TAU / AROUND as f32,
            (i + 1) as f32 * TAU / AROUND as f32,
        );
        let n = |a: f32| Vec3::new(a.cos() * dz, a.sin() * dz, -dr);
        b.quad(
            [
                round(r0, a0, z0),
                round(r0, a1, z0),
                round(r1, a1, z1),
                round(r1, a0, z1),
            ],
            [n(a0), n(a1), n(a1), n(a0)],
            part,
        );
    }
}

pub(super) fn structure(b: &mut Builder) {
    let radius = CITY as f32;
    let (stem, under) = STEM;
    let foot = ELEVATION as f32 + 8000.0;
    // The rim, a band of light under it, the underside in two sweeps and
    // the stem, a band of light at its head.
    b.tone = 41;
    band(b, (radius, 0.0), (radius, RIM), parts::TOWER);
    b.tone = 255;
    band(
        b,
        (radius, RIM),
        (radius - 500.0, RIM + 160.0),
        parts::LIGHT,
    );
    b.tone = 97;
    band(
        b,
        (radius - 500.0, RIM + 160.0),
        (radius * 0.55, under * 0.55),
        parts::TOWER,
    );
    b.tone = 153;
    band(
        b,
        (radius * 0.55, under * 0.55),
        (stem, under),
        parts::TOWER,
    );
    b.tone = 255;
    band(b, (stem, under), (stem, under + 220.0), parts::LIGHT);
    b.tone = 67;
    band(b, (stem, under + 220.0), (stem, foot), parts::TOWER);
    // Housings on the dish, low and broad, turned to ring the mouth; and
    // a few masts, a beacon on each.
    let mut rng = Rng::new(0x5ea1_d15c);
    let mut unit = move || rng.unit() as f32;
    for k in 0..46 {
        let (a, r) = (unit() * TAU, 2600.0 + (radius - 4200.0) * unit().sqrt());
        let (h0, h1, h2) = (unit(), unit(), unit());
        let at = round(r, a, 0.0);
        let u = Vec3::new(-a.sin(), a.cos(), 0.0);
        if k % 8 == 7 {
            let height = 900.0 + 1400.0 * h0;
            b.tone = 255;
            b.block(
                at,
                u,
                [26.0, 26.0],
                [0.0, -height],
                part::ACCENT,
                part::DARK,
            );
            b.tone = neon::BEACON;
            b.block(
                at,
                u,
                [44.0, 44.0],
                [-height, -height - 70.0],
                parts::NEON,
                parts::NEON,
            );
            continue;
        }
        let half = [300.0 + 900.0 * h0, 160.0 + 420.0 * h1];
        let height = 90.0 + 420.0 * h2 * h2;
        b.tone = (h1 * 199.0) as u8;
        b.block(at, u, half, [0.0, -height], parts::TOWER, part::HULL);
        if h2 > 0.5 {
            b.block(
                at,
                u,
                [half[0] * 0.5, half[1] * 0.6],
                [-height, -height * 1.8],
                parts::TOWER,
                part::HULL,
            );
        }
    }
    b.tone = 255;
}
