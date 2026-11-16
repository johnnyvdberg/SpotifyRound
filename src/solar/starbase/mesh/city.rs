//! The city's buildings beyond the towers of `spire`: a needle for the
//! tallest of a skyline, slender so a tall one stays within its lot; a
//! mid-rise block, set back under its roof; and a works' shed, its roof a
//! sawtooth of north lights. Each stands on its foot along +z, its
//! footprint along x and y as `city::extent` says.

use glam::Vec3;
use std::f32::consts::TAU;

use super::super::super::models::part;
use super::Builder;
use super::spire::{bands, beacon, tier};

/// A slender hexagon, a little tapered, under a spire and a beacon.
pub(super) fn needle(b: &mut Builder) {
    tier(
        b,
        [0.0, 0.0],
        6,
        (0.055, 0.0),
        (0.045, 0.6),
        0.0,
        1.0,
        bands,
        None,
    );
    tier(
        b,
        [0.0, 0.0],
        6,
        (0.045, 0.6),
        (0.032, 0.86),
        0.0,
        1.0,
        bands,
        Some(part::ACCENT),
    );
    tier(
        b,
        [0.0, 0.0],
        6,
        (0.02, 0.86),
        (0.003, 0.99),
        0.0,
        1.0,
        |_| part::DARK,
        None,
    );
    beacon(b, 0.97);
}

/// A block of a few floors: windows and hull by turns round it, a storey
/// set back on its roof.
pub(super) fn block(b: &mut Builder) {
    // Corners at an eighth of a turn: half a unit along x, 0.35 across.
    let r = 0.5 * std::f32::consts::SQRT_2;
    tier(
        b,
        [0.0, 0.0],
        4,
        (r, 0.0),
        (r, 0.86),
        TAU / 8.0,
        0.7,
        bands,
        Some(part::ACCENT),
    );
    tier(
        b,
        [0.08, 0.0],
        4,
        (r * 0.55, 0.86),
        (r * 0.55, 1.0),
        TAU / 8.0,
        0.7,
        |_| part::HULL,
        Some(part::DARK),
    );
}

/// A works' shed: low walls and a roof of four teeth, each a slope up to a
/// lit glazed face.
pub(super) fn shed(b: &mut Builder) {
    let r = 0.5 * std::f32::consts::SQRT_2;
    let (y, lo, hi) = (0.32, 0.22, 0.3);
    tier(
        b,
        [0.0, 0.0],
        4,
        (r, 0.0),
        (r, lo),
        TAU / 8.0,
        0.64,
        |_| part::HULL,
        None,
    );
    for i in 0..4 {
        let x0 = -0.5 + i as f32 * 0.25;
        let x1 = x0 + 0.25;
        b.flat(
            [
                Vec3::new(x0, -y, lo),
                Vec3::new(x1, -y, hi),
                Vec3::new(x1, y, hi),
                Vec3::new(x0, y, lo),
            ],
            Vec3::new(-(hi - lo), 0.0, x1 - x0),
            part::ACCENT,
        );
        b.flat(
            [
                Vec3::new(x1, -y, lo),
                Vec3::new(x1, y, lo),
                Vec3::new(x1, y, hi),
                Vec3::new(x1, -y, hi),
            ],
            Vec3::X,
            part::WINDOWS,
        );
        let (a, c) = (Vec3::new(x0, y, lo), Vec3::new(x0, -y, lo));
        b.flat(
            [a, a, Vec3::new(x1, y, hi), Vec3::new(x1, y, lo)],
            Vec3::Y,
            part::HULL,
        );
        b.flat(
            [c, c, Vec3::new(x1, -y, lo), Vec3::new(x1, -y, hi)],
            -Vec3::Y,
            part::HULL,
        );
    }
}
