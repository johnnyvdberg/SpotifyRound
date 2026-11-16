//! The port, out in the open of the base's hollow world: catwalks, and the
//! ships moored along them. A spine runs on from the tunnel's deck down
//! the world's axis, a second above it, masts between the two at the arms'
//! tips; arms reach
//! out either side of both at every station, and ships lie alongside the
//! arms, each at a mooring: freighters, liners and shuttles, and here and
//! there a warship of the fleets', the largest at the arms' ends. The
//! ship's own dock stands on the lower spine. What is moored where follows
//! the base's seed.

use glam::{Quat, Vec3};
use std::f32::consts::FRAC_PI_2;

use super::super::models::{Instance, Key};
use super::super::system::Rng;
use super::super::tour::BaseScene;
use super::mesh::piece;
use super::mesh::port::{CATWALK, MOORING};
use super::mesh::tunnel::deck;
use super::scene::{base, hall, instance, seen, ship};

/// The fleets' designs berthed and flying here: every one but the cube's.
pub const DESIGNS: u32 = 5;
/// How far the port runs on from the tunnel's end, metres.
pub const LENGTH: f32 = 2800.0;
/// The upper spine's height above the tunnel's line.
const UPPER: f32 = 250.0;
/// The stations arms reach out from: the first's depth into the port, the
/// spacing and how many; and an arm's length in modules.
const STATIONS: (f32, f32, usize) = (300.0, 300.0, 8);
const ARM: usize = 4;
/// Beyond this a fleet's ship is left out (its mesh is thousands of
/// triangles): the haze has all but hidden it.
const WARSHIPS_WITHIN: f32 = 2400.0;

/// A hull colour of the port's own.
pub(super) fn livery(rng: &mut Rng) -> [f32; 3] {
    *rng.pick(&[
        [1.0, 1.0, 1.0],
        [1.25, 1.1, 0.8],
        [0.75, 0.95, 1.3],
        [1.3, 0.75, 0.65],
        [0.8, 1.2, 0.95],
        [1.2, 1.2, 1.25],
    ])
}

/// A ship of the fleets', by a draw.
pub(super) fn warship(rng: &mut Rng, role: u32) -> Key {
    Key::Ship {
        design: (rng.next() % DESIGNS as u64) as u32,
        role,
        variant: (rng.next() % super::super::models::VARIANTS as u64) as u32,
    }
}

/// The catwalks and the ships moored along them.
pub(super) fn berths(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    let origin = hall();
    let eye = ship(b);
    let none = [0.0; 4];
    let (module, half) = CATWALK;
    let lower = deck().0;
    let walk = |out: &mut Vec<(Key, Instance)>, at: Vec3, turn: Quat, way: Vec3| {
        if seen(b, at + way * module * 0.5, module * 0.6) {
            out.push((base(piece::CATWALK), instance(at, 1.0, turn, none)));
        }
    };
    // The spines, and the masts between them.
    let modules = (LENGTH / module) as usize;
    for k in 0..modules {
        let z = module * k as f32;
        walk(
            out,
            origin + Vec3::new(0.0, lower, z),
            Quat::IDENTITY,
            Vec3::Z,
        );
        if k >= 2 {
            walk(
                out,
                origin + Vec3::new(0.0, UPPER, z),
                Quat::IDENTITY,
                Vec3::Z,
            );
        }
    }
    let (first, pitch, stations) = STATIONS;
    // Masts between the two levels, at the arms' tips: clear of the way in.
    let up = Quat::from_rotation_x(-FRAC_PI_2);
    for s in [0usize, 3, 6] {
        for side in [-1.0f32, 1.0] {
            let x = side * (half + module * ARM as f32 - half);
            for k in 0..((UPPER - lower) / module).ceil() as usize {
                let y = (lower + module * k as f32).min(UPPER - module);
                walk(
                    out,
                    origin + Vec3::new(x, y, first + pitch * s as f32),
                    up,
                    Vec3::Y,
                );
            }
        }
    }
    for s in 0..stations {
        let z = first + pitch * s as f32;
        for (l, level) in [lower, UPPER].into_iter().enumerate() {
            for (e, side) in [-1.0f32, 1.0].into_iter().enumerate() {
                let way = Vec3::X * side;
                let turn = Quat::from_rotation_y(side * FRAC_PI_2);
                for k in 0..ARM {
                    let at = origin + Vec3::new(side * (half + module * k as f32), level, z);
                    walk(out, at, turn, way);
                }
                // Two berths along the arm, on the side away from the
                // tunnel: a ship alongside each, bow out along the spine.
                for m in 0..2usize {
                    let n = (s * 8 + l * 4 + e * 2 + m) as u64;
                    let mut rng = Rng::new(b.seed ^ 0xd0c5 ^ n << 20);
                    let full = rng.chance(0.72);
                    let tint = livery(&mut rng);
                    let kind = rng.unit();
                    let size_draw = rng.unit() as f32;
                    let key = warship(&mut rng, if m == 1 { 0 } else { 1 });
                    let across = half + module * if m == 0 { 1.3 } else { 3.4 };
                    let post = origin + Vec3::new(side * across, level, z + half);
                    // The largest lie at the arms' ends, at every other
                    // station, so they clear each other.
                    let capital = m == 1 && (s + l + e) % 2 == 0;
                    if m == 1 && !capital && kind < 0.35 {
                        continue;
                    }
                    let (key, size, beam, rise, look) = if capital {
                        (key, 150.0 + 35.0 * size_draw, 0.36, 0.1, none)
                    } else if kind < 0.2 {
                        (key, 52.0 + 18.0 * size_draw, 0.45, 0.1, none)
                    } else if kind < 0.55 {
                        (
                            base(piece::FREIGHTER),
                            150.0 + 50.0 * size_draw,
                            0.14,
                            0.02,
                            [0.0, tint[0], tint[1], tint[2]],
                        )
                    } else if kind < 0.8 {
                        (
                            base(piece::LINER),
                            170.0 + 35.0 * size_draw,
                            0.08,
                            0.03,
                            [0.0, tint[0], tint[1], tint[2]],
                        )
                    } else {
                        (
                            base(piece::SHUTTLE),
                            60.0 + 20.0 * size_draw,
                            0.43,
                            0.06,
                            [0.0, tint[0], tint[1], tint[2]],
                        )
                    };
                    let berth = post + Vec3::new(0.0, size * rise, MOORING + size * beam);
                    if !seen(b, berth, size * 1.1) {
                        continue;
                    }
                    // Amber while a ship is worked on, green ready, red closed.
                    let status = if !full {
                        2.0
                    } else if kind.fract() * 7.0 % 1.0 < 0.5 {
                        0.0
                    } else {
                        1.0
                    };
                    // A mooring reaches +x: turned to reach along the spine.
                    out.push((
                        base(piece::MOORING),
                        instance(
                            post,
                            1.0,
                            Quat::from_rotation_y(-FRAC_PI_2),
                            [status, 0.0, 0.0, 0.0],
                        ),
                    ));
                    let warlike = matches!(key, Key::Ship { .. });
                    if full && !(warlike && (berth - eye).length() > WARSHIPS_WITHIN) {
                        out.push((
                            key,
                            instance(berth, size, Quat::from_rotation_y(side * FRAC_PI_2), look),
                        ));
                    }
                }
            }
        }
    }
}
