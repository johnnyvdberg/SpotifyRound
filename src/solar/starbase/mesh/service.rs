//! The ship's own dock and what works on it: the cradle it rests in and
//! its clamps, and the ship's own bow as its cockpit sees it.

use glam::Vec3;

use super::super::super::models::part;
use super::tunnel::deck;
use super::{Builder, neon, parts};

/// The cradle, about the point the ship rests at: a column up from the
/// port's spine, a deck with hazard-striped ends and guide lines, a gantry
/// over its far end, and floodlights under it.
pub(super) fn cradle(b: &mut Builder) {
    let v = Vec3::new;
    let (spine, _) = deck();
    b.cuboid(v(-7.0, spine, -7.0), v(7.0, -40.0, 7.0), part::ACCENT);
    b.cuboid(v(-26.0, -52.0, -26.0), v(26.0, -40.0, 26.0), part::ACCENT);
    b.cuboid(v(-40.0, -40.0, -50.0), v(40.0, -34.0, 62.0), part::HULL);
    for (z0, z1) in [(-50.0, -46.0), (58.0, 62.0)] {
        b.faces(
            v(-40.0, -34.0, z0),
            v(40.0, -33.6, z1),
            [None, None, Some(parts::HAZARD), None, None, None],
        );
    }
    b.tone = neon::CYAN;
    for x in [-22.0f32, 22.0] {
        b.flat(
            [
                v(x - 0.6, -33.7, -44.0),
                v(x + 0.6, -33.7, -44.0),
                v(x + 0.6, -33.7, 56.0),
                v(x - 0.6, -33.7, 56.0),
            ],
            Vec3::Y,
            parts::NEON,
        );
    }
    b.tone = 255;
    // The gantry: two posts and a beam.
    for x in [-48.0f32, 42.0] {
        b.cuboid(v(x, -34.0, 42.0), v(x + 6.0, 44.0, 54.0), part::ACCENT);
    }
    b.cuboid(v(-48.0, 36.0, 42.0), v(48.0, 46.0, 54.0), part::ACCENT);
    for x in [-36.0f32, -14.0, 14.0, 36.0] {
        b.faces(
            v(x - 4.0, 34.0, 44.0),
            v(x + 4.0, 36.0, 52.0),
            [
                Some(part::DARK),
                Some(part::DARK),
                None,
                Some(parts::LIGHT),
                Some(part::DARK),
                Some(part::DARK),
            ],
        );
    }
}

/// A clamp, hinged at its foot: an upright, and a jaw reaching in along
/// +x over the hull.
pub(super) fn clamp(b: &mut Builder) {
    let v = Vec3::new;
    b.cuboid(v(-2.0, 0.0, -3.0), v(2.0, 22.0, 3.0), part::ACCENT);
    b.cuboid(v(-2.0, 18.0, -3.0), v(10.0, 22.0, 3.0), part::ACCENT);
    b.cuboid(v(-2.2, 8.0, -3.2), v(2.2, 11.0, 3.2), parts::HAZARD);
}

/// The ship's own bow as seen from its cockpit, the camera at the origin
/// looking along +z: a deck tapering ahead below the eye, sloping sides, a
/// stripe down its spine and running lights at its shoulders.
pub(super) fn bow(b: &mut Builder) {
    let v = Vec3::new;
    let (near, far) = (2.0, 30.0);
    let (wide, narrow) = (9.0, 1.5);
    let (top, bottom) = (-4.0, -11.0);
    b.flat(
        [
            v(-wide, top, near),
            v(wide, top, near),
            v(narrow, top, far),
            v(-narrow, top, far),
        ],
        Vec3::Y,
        part::HULL,
    );
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
