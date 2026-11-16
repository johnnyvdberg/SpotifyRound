//! What flies through the base: past the tunnel's windows, out in the
//! hollow, the fleets' ships and freighters on lanes along the tunnel and
//! streams of cars along it and in rings round it; in the port, ships and
//! cars on circuits round its catwalks. Each a pure function of the time, as
//! the tour is.

use glam::{Quat, Vec3};
use std::f32::consts::{PI, TAU};

use super::super::models::{Instance, Key};
use super::super::system::Rng;
use super::super::tour::BaseScene;
use super::super::tour::starbase::{BORE, FUNNEL, SEGMENT, bend, tunnel_end};
use super::hash;
use super::mesh::piece;
use super::port::{LENGTH, livery, warship};
use super::scene::{base, hall, instance, seen, ship};

/// `x` wrapped into `-span / 2..span / 2`.
fn wrap(x: f32, span: f32) -> f32 {
    (x + span * 0.5).rem_euclid(span) - span * 0.5
}

/// A car's colours: its body's tint.
fn paint(n: u32) -> [f32; 4] {
    let tints = [
        [1.3, 0.5, 0.4],
        [0.5, 0.8, 1.4],
        [1.3, 1.1, 0.5],
        [1.0, 1.0, 1.0],
        [0.5, 1.2, 0.8],
    ];
    let t = tints[(hash(n) * 4.99) as usize];
    [1.0, t[0], t[1], t[2]]
}

/// A point beside the tunnel at depth `z`: `across` and `up` from its line,
/// and the turn that heads a craft along it, in or out.
fn beside(z: f32, across: f32, up: f32, inbound: bool) -> (Vec3, Quat) {
    let (x, slope) = bend(z as f64);
    let yaw = (slope as f32).atan() + if inbound { 0.0 } else { PI };
    (
        Vec3::new(x as f32 + across, up, z),
        Quat::from_rotation_y(yaw),
    )
}

/// The cavern's traffic near the camera.
pub(super) fn cavern(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    let t = b.clock as f32;
    let eye = ship(b);
    let (start, end) = (FUNNEL as f32 + 120.0, tunnel_end() as f32 - 120.0);
    let wall = BORE as f32 + 18.0;
    // Ships on lanes either side and overhead: every third a warship.
    let span = 2400.0;
    for k in 0..12u32 {
        let mut rng = Rng::new(b.seed ^ 0x1a9e ^ (k as u64) << 16);
        let inbound = k % 2 == 0;
        let speed = rng.range(35.0, 95.0) as f32 * if inbound { 1.0 } else { -1.0 };
        let z = eye.z + wrap(rng.unit() as f32 * span + speed * t - eye.z, span);
        let overhead = k % 4 == 3;
        let size = rng.range(28.0, 46.0) as f32;
        let (across, up) = if overhead {
            (
                rng.range(-70.0, 70.0) as f32,
                wall + size * 0.6 + rng.range(10.0, 70.0) as f32,
            )
        } else {
            let side = if k % 4 < 2 { 1.0 } else { -1.0 };
            (
                side * (wall + size * 0.5 + rng.range(10.0, 90.0) as f32),
                rng.range(-35.0, 75.0) as f32,
            )
        };
        let tint = livery(&mut rng);
        let warlike = k % 3 == 0;
        let key = if warlike {
            warship(&mut rng, 1)
        } else if k % 3 == 1 {
            base(piece::FREIGHTER)
        } else {
            base(piece::SHUTTLE)
        };
        if z < start || z > end {
            continue;
        }
        let (at, turn) = beside(z, across, up, inbound);
        if !seen(b, at, size * 1.3) {
            continue;
        }
        let (size, look) = if warlike {
            (size, [1.0, 0.0, 0.0, 0.0])
        } else {
            (size * 2.0, [1.0, tint[0], tint[1], tint[2]])
        };
        out.push((key, instance(at, size, turn, look)));
    }
    // Cars in streams along the tunnel, close by its windows.
    let span = 900.0;
    for i in 0..96u32 {
        let lane = i % 8;
        let inbound = lane % 2 == 0;
        let speed = (55.0 + 14.0 * lane as f32) * if inbound { 1.0 } else { -1.0 };
        let z = eye.z + wrap(hash(i * 7 + 1) * span + speed * t - eye.z, span);
        if z < start || z > end {
            continue;
        }
        let (across, up) = match lane / 2 {
            0 => (wall + 6.0 + 9.0 * (lane % 2) as f32, 22.0),
            1 => (-(wall + 6.0 + 9.0 * (lane % 2) as f32), -6.0),
            2 => (18.0 - 36.0 * (lane % 2) as f32, wall + 8.0),
            _ => {
                let side = if lane % 2 == 0 { 1.0 } else { -1.0 };
                (side * (wall + 40.0), 48.0 * side)
            }
        };
        let (at, turn) = beside(z, across, up + 5.0 * hash(i * 13 + 5), inbound);
        if seen(b, at, 12.0) {
            out.push((base(piece::CAR), instance(at, 1.0, turn, paint(i))));
        }
    }
    // And in rings round it at every other rib, over the top and down.
    let ring = 2.0 * SEGMENT as f32;
    let nearest = ((eye.z - FUNNEL as f32) / ring).round();
    for m in -2..=2 {
        let z = FUNNEL as f32 + (nearest + m as f32) * ring;
        if z < start || z > end {
            continue;
        }
        let n = (z / ring) as u32;
        let sense = if n.is_multiple_of(2) { 1.0 } else { -1.0 };
        let x = bend(z as f64).0 as f32;
        for i in 0..7u32 {
            let a = hash(n * 31 + i) * TAU + sense * t * 0.3;
            let radius = wall + 4.0;
            let at = Vec3::new(x + radius * a.cos(), radius * a.sin(), z);
            let tangent = Vec3::new(-a.sin(), a.cos(), 0.0) * sense;
            if seen(b, at, 12.0) {
                out.push((
                    base(piece::CAR),
                    instance(
                        at,
                        1.0,
                        Quat::from_rotation_arc(Vec3::Z, tangent),
                        paint(n * 31 + i),
                    ),
                ));
            }
        }
    }
}

/// The port's traffic: ships and cars on circuits round its catwalks,
/// level, wide of the arms and the ships along them.
pub(super) fn port(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    let t = b.clock as f32;
    let origin = hall();
    let centre = origin + Vec3::new(0.0, 0.0, LENGTH * 0.5 + 100.0);
    let circuit = |turns: f32, across: f32, along: f32, height: f32, sense: f32| {
        let a = turns * TAU;
        let at = centre + Vec3::new(across * a.cos(), height, along * a.sin());
        let way = Vec3::new(-across * a.sin(), 0.0, along * a.cos()) * sense;
        (
            at,
            Quat::from_rotation_arc(Vec3::Z, way.normalize_or(Vec3::Z)),
        )
    };
    for k in 0..9u32 {
        let mut rng = Rng::new(b.seed ^ 0x907c ^ (k as u64) << 16);
        let sense = if k % 2 == 0 { 1.0 } else { -1.0 };
        let across = rng.range(520.0, 900.0) as f32;
        let along = rng.range(1300.0, 1700.0) as f32;
        let height = [
            130.0, 380.0, -220.0, 60.0, 470.0, -330.0, -40.0, 320.0, -160.0,
        ][k as usize];
        let turns = rng.unit() as f32 + sense * t * rng.range(0.006, 0.011) as f32;
        let tint = livery(&mut rng);
        let size = rng.range(34.0, 52.0) as f32;
        let warlike = k % 3 == 0;
        let key = if warlike {
            warship(&mut rng, 1)
        } else if k % 3 == 1 {
            base(piece::FREIGHTER)
        } else {
            base(piece::SHUTTLE)
        };
        let (at, turn) = circuit(turns, across, along, height, sense);
        if !seen(b, at, size * 2.0) {
            continue;
        }
        let (size, look) = if warlike {
            (size, [1.0, 0.0, 0.0, 0.0])
        } else {
            (size * 2.2, [1.0, tint[0], tint[1], tint[2]])
        };
        out.push((key, instance(at, size, turn, look)));
    }
    for i in 0..70u32 {
        let lane = i % 5;
        let sense = if lane % 2 == 0 { 1.0 } else { -1.0 };
        let across = 500.0 + 40.0 * lane as f32;
        let along = 1250.0 + 110.0 * lane as f32;
        let height = [40.0, -170.0, 120.0, -260.0, 320.0][lane as usize] + 8.0 * hash(i * 3 + 2);
        let turns = hash(i * 17 + 3) + sense * t * (0.016 + 0.002 * lane as f32);
        let (at, turn) = circuit(turns, across, along, height, sense);
        if seen(b, at, 12.0) {
            out.push((base(piece::CAR), instance(at, 1.0, turn, paint(i + 500))));
        }
    }
}
