//! The deeper sections of the world inside, past the port, which the ship
//! crosses on its way out (`BaseStage::Crossing`), two thousand kilometres
//! of them: a bulkhead at every seam between lengths of the world's map
//! (`sections`), a sill round the ground and a ring braced across the
//! cylinder on spokes, which the ship flies through; every eighth a great
//! gate, a wall a kilometre up from the ground all the way round; in each
//! section of eight, by its own numbers, a wheel of a station turning about
//! the axis and arcologies rising kilometres from the ground toward it;
//! and the wall closing the far end, the lock's mouth in it. Placed where
//! they truly stand and drawn back with the scene (`BaseScene::fold`).

use glam::{Mat3, Quat, Vec3};
use std::f32::consts::TAU;

use super::super::models::{Instance, Key};
use super::super::tour::BaseScene;
use super::super::tour::starbase::{BEND, far_wall};
use super::city::ALONG;
use super::mesh::piece;
use super::scene::{base, instance, seen, ship};
use super::sections::{self, SPAN, folded, seam, section_hash, varied};

/// The cylinder's axis across the base's frame, and its ground's radius.
const AXIS: f32 = 200.0;
const GROUND: f32 = 4200.0;
/// How far off they are drawn: hazed nearly to nothing beyond.
const FAR: f32 = 48_000.0;
/// Nothing deep stands within this of the far wall but the wall.
const CLEAR: f32 = 3000.0;

pub(super) fn features(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    if !super::scene::world_shown(b) {
        return;
    }
    let eye = ship(b);
    let shift = sections::shift(b);
    let fold = folded(b);
    let wall = (far_wall() - b.fold) as f32;
    let none = [0.0; 4];
    let mut put = |key: u32, at: Vec3, size: f32, turn: Quat, radius: f32, look: [f32; 4]| {
        if at.z < wall - CLEAR && (at - eye).length() < FAR + radius && seen(b, at, radius) {
            out.push((base(key), instance(at, size, turn, look)));
        }
    };
    let first = ((eye.z - FAR) / ALONG + shift).floor() as i32;
    let last = ((eye.z + FAR) / ALONG + shift).ceil() as i32;
    let on_axis = |z: f32| Vec3::new(AXIS, 0.0, z);
    // The bulkheads, at the seams.
    for n in first..=last {
        let z = seam(n, shift);
        if !varied(z) {
            continue;
        }
        if (n + fold).rem_euclid(SPAN) == 0 {
            put(piece::GATE, on_axis(z), 1.0, Quat::IDENTITY, GROUND, none);
        } else {
            put(piece::SPOKES, on_axis(z), 1.0, Quat::IDENTITY, GROUND, none);
            put(piece::SILL, on_axis(z), 1.0, Quat::IDENTITY, GROUND, none);
        }
    }
    // Each section's own: a station, mostly, and a few arcologies.
    for k in first.div_euclid(SPAN)..=last.div_euclid(SPAN) {
        let start = k * SPAN;
        if !varied(seam(start, shift)) {
            continue;
        }
        let own = k + fold / SPAN;
        let h = |salt: u32| section_hash(own, salt);
        // Into a length of the section, clear of the bulkheads.
        let along = |length: f32, part: f32| seam(start, shift) + (length + part) * ALONG;
        if h(1) < 0.65 {
            let z = along(1.0 + (h(2) * 5.99).floor(), 0.5);
            // It turns once in a few minutes.
            let spin = Quat::from_rotation_z(b.clock as f32 * 0.03 + h(3) * TAU);
            put(piece::STATION, on_axis(z), 1.0, spin, 1000.0, none);
        }
        let count = 2 + (h(4) * 4.99) as u32;
        for i in 0..count {
            let z = along((h(10 + i) * 7.99).floor(), 0.12 + 0.76 * h(20 + i));
            let angle = h(30 + i) * TAU;
            let height = 1800.0 + 1500.0 * h(40 + i);
            let (s, c) = angle.sin_cos();
            let up = -Vec3::new(c, s, 0.0);
            let foot = Vec3::new(AXIS + GROUND * c, GROUND * s, z);
            let east = Vec3::new(-s, c, 0.0);
            let turn = Quat::from_mat3(&Mat3::from_cols(east, up.cross(east), up));
            put(
                piece::ARCOLOGY,
                foot,
                height,
                turn,
                height,
                [0.0, 0.85, 0.88, 0.95],
            );
        }
    }
    // The far wall itself.
    if (wall - eye.z).abs() < FAR * 2.0 {
        out.push((
            base(piece::FAR_WALL),
            instance(Vec3::new(BEND as f32, 0.0, wall), 1.0, Quat::IDENTITY, none),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_scene;
    use super::*;
    use crate::solar::tour::BaseStage;
    use crate::solar::tour::starbase::{CRADLE, fold, tunnel_end};

    fn placed_at(z: f64) -> Vec<(Key, Instance)> {
        let mut b = test_scene(BaseStage::Crossing, 10.0, [BEND, 40.0, z]);
        b.fold = fold(z);
        b.ship[2] -= b.fold;
        let mut placed = Vec::new();
        features(&b, &mut placed);
        placed
    }

    fn count(placed: &[(Key, Instance)], p: u32) -> usize {
        placed.iter().filter(|(k, _)| *k == base(p)).count()
    }

    #[test]
    fn the_crossing_passes_bulkheads_gates_and_the_rest_to_the_wall() {
        let start = tunnel_end() + CRADLE;
        // Out from the port: bulkheads ahead.
        let near = placed_at(start + 3000.0);
        assert!(count(&near, piece::SPOKES) >= 2, "{}", near.len());
        // Deep in, every kind seen somewhere along the way, the scene
        // drawn back so it stays near.
        let (mut gates, mut stations, mut arcologies) = (0, 0, 0);
        for k in 0..40 {
            let z = start + 150_000.0 + k as f64 * 45_000.0;
            let placed = placed_at(z);
            gates += count(&placed, piece::GATE);
            stations += count(&placed, piece::STATION);
            arcologies += count(&placed, piece::ARCOLOGY);
            for (_, i) in &placed {
                assert!(i.place[2].abs() < 300_000.0, "{z} {:?}", i.place);
            }
            if z < far_wall() - 100_000.0 {
                assert_eq!(count(&placed, piece::FAR_WALL), 0);
            }
        }
        assert!(
            gates > 5 && stations > 5 && arcologies > 40,
            "{gates} {stations} {arcologies}"
        );
        // At the end, the wall, and nothing deep stands at it.
        let end = placed_at(far_wall() - 400.0);
        assert_eq!(count(&end, piece::FAR_WALL), 1);
        // The ship's line goes through every ring's hoop and the station's
        // hub, clear of their spokes' roots.
        let off = ((BEND as f32 - AXIS).powi(2) + 55f32.powi(2)).sqrt();
        assert!(off < 300.0, "{off}");
        // Turned to the far side, none of it.
        let mut far = test_scene(BaseStage::Crossing, 10.0, [BEND, 40.0, start + 3000.0]);
        far.far = true;
        let mut placed = Vec::new();
        features(&far, &mut placed);
        assert!(placed.is_empty());
    }
}
