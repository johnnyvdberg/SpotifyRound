//! The buildings of the world inside the base, standing up from the
//! cylinder's ground toward its axis: towers of three shapes, none of them
//! a box (a slab with a raked roof, a tapering hexagon under a needle, a
//! round tower under a dome), a plain one for towns far off, and a patch
//! of low sprawl. A tower is a unit tall along +z from its foot and scaled
//! to its height; a patch of sprawl a unit in radius. Their faces are
//! their hull (tinted by the placing, muted: a town is seen from
//! kilometres off) between bands of dim windows.

use glam::Vec3;
use std::f32::consts::TAU;

use super::super::super::models::part;
use super::super::super::system::Rng;
use super::{Builder, neon, parts};

/// How bright a tower's windows are, of 255: dim, for a skyline far off.
pub(super) const WINDOWS: u8 = 140;

/// A tier round the axis `at` (x, y): `sides` of them, from radius `r0` at
/// height `z0` to `r1` at `z1`, turned by `turn`, squashed to `flat` across
/// one way; its faces by `face(k)`. And its top.
#[allow(clippy::too_many_arguments)]
pub(super) fn tier(
    b: &mut Builder,
    at: [f32; 2],
    sides: usize,
    (r0, z0): (f32, f32),
    (r1, z1): (f32, f32),
    turn: f32,
    flat: f32,
    face: impl Fn(usize) -> u8,
    top: Option<u8>,
) {
    let corner = |k: usize, r: f32, z: f32| {
        let a = turn + k as f32 * TAU / sides as f32;
        Vec3::new(at[0] + r * a.cos(), at[1] + r * a.sin() * flat, z)
    };
    for k in 0..sides {
        let mid = turn + (k as f32 + 0.5) * TAU / sides as f32;
        let n = Vec3::new(
            mid.cos() * flat,
            mid.sin(),
            (r0 - r1) / (z1 - z0).max(1e-3) * flat,
        );
        let part = face(k);
        b.tone = if part == part::WINDOWS { WINDOWS } else { 255 };
        b.flat(
            [
                corner(k, r0, z0),
                corner(k + 1, r0, z0),
                corner(k + 1, r1, z1),
                corner(k, r1, z1),
            ],
            n,
            part,
        );
    }
    b.tone = 255;
    if let Some(part) = top {
        let centre = Vec3::new(at[0], at[1], z1);
        for k in 0..sides {
            b.flat(
                [centre, centre, corner(k, r1, z1), corner(k + 1, r1, z1)],
                Vec3::Z,
                part,
            );
        }
    }
}

pub(super) fn beacon(b: &mut Builder, z: f32) {
    b.tone = neon::BEACON;
    b.cuboid(
        Vec3::new(-0.012, -0.012, z),
        Vec3::new(0.012, 0.012, z + 0.03),
        parts::NEON,
    );
    b.tone = 255;
}

/// Hull and windows by turns round a tier.
pub(super) fn bands(k: usize) -> u8 {
    if k.is_multiple_of(2) {
        part::WINDOWS
    } else {
        part::HULL
    }
}

/// A slab: broad one way and thin the other, a band of windows down each
/// broad face, set back near its top under a raked roof.
pub(super) fn slab(b: &mut Builder) {
    let face = |k: usize| {
        if k.is_multiple_of(2) {
            part::WINDOWS
        } else {
            part::HULL
        }
    };
    tier(
        b,
        [0.0, 0.0],
        4,
        (0.15, 0.0),
        (0.15, 0.7),
        TAU / 8.0,
        0.5,
        face,
        Some(part::ACCENT),
    );
    tier(
        b,
        [0.02, 0.0],
        4,
        (0.11, 0.7),
        (0.08, 0.93),
        TAU / 8.0,
        0.5,
        face,
        Some(part::DARK),
    );
    tier(
        b,
        [0.05, 0.0],
        3,
        (0.035, 0.93),
        (0.004, 1.0),
        0.0,
        0.5,
        |_| part::ACCENT,
        None,
    );
}

/// A tapering hexagon in three sweeps, a needle and a beacon on it.
pub(super) fn taper(b: &mut Builder) {
    tier(
        b,
        [0.0, 0.0],
        6,
        (0.12, 0.0),
        (0.095, 0.45),
        0.0,
        1.0,
        bands,
        None,
    );
    tier(
        b,
        [0.0, 0.0],
        6,
        (0.095, 0.45),
        (0.055, 0.78),
        0.0,
        1.0,
        bands,
        None,
    );
    tier(
        b,
        [0.0, 0.0],
        6,
        (0.055, 0.78),
        (0.012, 0.92),
        0.0,
        1.0,
        |_| part::ACCENT,
        None,
    );
    tier(
        b,
        [0.0, 0.0],
        3,
        (0.012, 0.92),
        (0.002, 0.985),
        0.0,
        1.0,
        |_| part::DARK,
        None,
    );
    beacon(b, 0.97);
}

/// A round tower, a lit ring at its shoulder and a dome over it.
pub(super) fn round(b: &mut Builder) {
    tier(
        b,
        [0.0, 0.0],
        8,
        (0.13, 0.0),
        (0.13, 0.62),
        0.0,
        1.0,
        bands,
        None,
    );
    tier(
        b,
        [0.0, 0.0],
        8,
        (0.15, 0.62),
        (0.15, 0.66),
        0.0,
        1.0,
        |_| parts::LIGHT,
        Some(part::ACCENT),
    );
    tier(
        b,
        [0.0, 0.0],
        8,
        (0.13, 0.66),
        (0.09, 0.86),
        0.0,
        1.0,
        |_| part::HULL,
        None,
    );
    tier(
        b,
        [0.0, 0.0],
        8,
        (0.09, 0.86),
        (0.0, 1.0),
        0.0,
        1.0,
        |_| part::ACCENT,
        None,
    );
}

/// A tower for a town far off: four faces leaning in, and its roof.
pub(super) fn far(b: &mut Builder) {
    tier(
        b,
        [0.0, 0.0],
        4,
        (0.13, 0.0),
        (0.07, 1.0),
        TAU / 8.0,
        1.0,
        bands,
        Some(part::ACCENT),
    );
}

/// A patch of sprawl: a dozen low buildings of five and six sides, each
/// its own size and turn, scattered over a unit's radius.
pub(super) fn sprawl(b: &mut Builder) {
    let mut rng = Rng::new(0x5b_a71);
    let mut unit = move || rng.unit() as f32;
    for i in 0..13 {
        let (a, r) = (unit() * TAU, unit().sqrt());
        let size = 0.08 + 0.16 * unit();
        let height = 0.06 + 0.32 * unit() * unit();
        let sides = [5, 6, 4][i % 3];
        let face = move |k: usize| {
            if (k + i).is_multiple_of(3) {
                part::WINDOWS
            } else {
                part::HULL
            }
        };
        tier(
            b,
            [a.cos() * r, a.sin() * r],
            sides,
            (size, 0.0),
            (size * 0.9, height),
            unit() * TAU,
            0.6 + 0.4 * unit(),
            face,
            Some(part::ACCENT),
        );
    }
}
