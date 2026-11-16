//! The craft that fill the base besides the fleets' own ships: a flying
//! car, a freighter, a shuttle and a liner, a few dozen triangles each, as
//! scores of them are in view at once. Each flies along +z. A car is in
//! metres; the others a unit long, scaled to their length by their placing.

use glam::Vec3;

use super::super::super::models::part;
use super::{Builder, neon, parts};

const NONE: Option<u8> = None;

/// A box whose rear face (-z) is `rear` and whose front is `front`, the
/// rest `body`.
fn hull(b: &mut Builder, lo: Vec3, hi: Vec3, body: u8, front: u8, rear: u8) {
    let s = Some(body);
    b.faces(lo, hi, [s, s, s, s, Some(front), Some(rear)]);
}

/// A flying car, nine metres long: its body, head lamps and tail lamps.
pub(super) fn car(b: &mut Builder) {
    let v = Vec3::new;
    b.tone = neon::RED;
    hull(
        b,
        v(-1.7, -0.7, -4.5),
        v(1.7, 0.6, 4.5),
        part::HULL,
        parts::LIGHT,
        parts::NEON,
    );
    b.tone = 255;
    b.cuboid(v(-1.3, 0.6, -2.0), v(1.3, 1.4, 1.6), part::DARK);
}

/// A freighter: a spine carrying three containers, a bridge at the bow
/// and two engines aft.
pub(super) fn freighter(b: &mut Builder) {
    let v = Vec3::new;
    hull(
        b,
        v(-0.07, -0.05, -0.42),
        v(0.07, 0.03, 0.34),
        part::ACCENT,
        part::ACCENT,
        part::DARK,
    );
    for k in 0..3 {
        let z = -0.36 + 0.22 * k as f32;
        b.cuboid(v(-0.13, -0.12, z), v(0.13, 0.11, z + 0.19), part::HULL);
    }
    hull(
        b,
        v(-0.1, -0.06, 0.34),
        v(0.1, 0.1, 0.5),
        part::HULL,
        part::WINDOWS,
        part::DARK,
    );
    for x in [-0.12f32, 0.03] {
        hull(
            b,
            v(x, -0.07, -0.5),
            v(x + 0.09, 0.04, -0.4),
            part::ACCENT,
            part::ACCENT,
            part::ENGINE,
        );
    }
    b.tone = neon::BEACON;
    b.cuboid(v(-0.01, 0.1, 0.4), v(0.01, 0.13, 0.42), parts::NEON);
    b.tone = 255;
}

/// A shuttle: a short body, stub wings, a windscreen and one engine.
pub(super) fn shuttle(b: &mut Builder) {
    let v = Vec3::new;
    hull(
        b,
        v(-0.16, -0.1, -0.5),
        v(0.16, 0.1, 0.3),
        part::HULL,
        part::ACCENT,
        part::ENGINE,
    );
    hull(
        b,
        v(-0.11, -0.08, 0.3),
        v(0.11, 0.06, 0.5),
        part::ACCENT,
        part::WINDOWS,
        part::DARK,
    );
    b.cuboid(v(-0.42, -0.03, -0.42), v(0.42, 0.0, -0.05), part::ACCENT);
    b.cuboid(v(-0.02, 0.1, -0.48), v(0.02, 0.28, -0.2), part::ACCENT);
    b.tone = neon::RED;
    b.cuboid(v(-0.43, -0.03, -0.3), v(-0.41, 0.0, -0.24), parts::NEON);
    b.tone = 255;
    b.faces(
        v(0.41, -0.03, -0.3),
        v(0.43, 0.0, -0.24),
        [Some(parts::LIGHT), NONE, NONE, NONE, NONE, NONE],
    );
}

/// A liner: a long hull, its decks rows of windows down both sides, a
/// bridge on top and three engines aft.
pub(super) fn liner(b: &mut Builder) {
    let v = Vec3::new;
    let w = Some(part::WINDOWS);
    let h = Some(part::HULL);
    b.faces(
        v(-0.075, -0.06, -0.44),
        v(0.075, 0.05, 0.42),
        [w, w, h, h, h, Some(part::DARK)],
    );
    hull(
        b,
        v(-0.05, -0.045, 0.42),
        v(0.05, 0.03, 0.5),
        part::HULL,
        part::WINDOWS,
        part::DARK,
    );
    hull(
        b,
        v(-0.04, 0.05, 0.1),
        v(0.04, 0.085, 0.3),
        part::ACCENT,
        part::WINDOWS,
        part::ACCENT,
    );
    b.cuboid(v(-0.012, 0.05, -0.36), v(0.012, 0.13, -0.24), part::ACCENT);
    for x in [-0.07f32, -0.02, 0.03] {
        hull(
            b,
            v(x, -0.045, -0.5),
            v(x + 0.04, 0.035, -0.44),
            part::ACCENT,
            part::ACCENT,
            part::ENGINE,
        );
    }
}
