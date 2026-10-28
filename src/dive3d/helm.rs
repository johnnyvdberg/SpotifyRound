//! How the Nautilus moves: a helmsman's hand on a heavy boat.
//!
//! [`crate::voyage`] says where along the voyage the boat should be and how
//! high it means to cruise; the helm turns that into a pose that moves like a
//! submarine. It follows the corridor's meander across the seabed and surges
//! a little in speed. It keeps its height above the highest ground about it,
//! filtered slowly, so it sails over a gully instead of ducking into it and
//! climbs before a ridge rather than at it. It noses up as it climbs and down
//! as it dives, banks gently into each change of heading, and every couple of
//! minutes slows and turns right round on its axis, so the whole sea goes past
//! the window. Every motion goes through a critically damped spring, so nothing
//! starts or stops abruptly — not a swipe to the next sea, not the drop over
//! the shelf into the abyss.
//!
//! In the lava cave it swings the window round to look up the tunnel ahead.
//!
//! Seen from the salon window, which looks out to port, the boat's pitch turns
//! the view about its own axis, its roll tips the view up and down, and its
//! heading pans it.

use std::f32::consts::{PI, TAU};

use super::world::{CAVE, Seabed, TRENCH, corridor, corridor_slope, hash};
use crate::voyage::{Position, SPEED};

/// The boat's pose for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: [f32; 3],
    /// The window's right, up and outward directions in the world.
    pub basis: [[f32; 3]; 3],
    /// How fast the boat is going, in world units a second.
    pub vel: [f32; 3],
    /// Depth below the surface, for the manometer, in the voyage's units.
    pub depth: f64,
}

impl Pose {
    /// A level boat at (x, y), for tests that need a pose without a voyage.
    #[cfg(test)]
    pub fn level(x: f32, y: f32) -> Self {
        Self {
            pos: [x, y, corridor(x)],
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            vel: [0.0; 3],
            depth: depth_reading(y),
        }
    }
}

/// The 3D seabed's depths are compressed so the abyss is a drop rather than a
/// gulf; the gauge undoes that, reading the depth the voyage means.
pub fn depth_reading(y: f32) -> f64 {
    (f64::from(-y) / 2.5).exp() - 1.0
}

/// A critically damped spring: no overshoot, no sudden start.
#[derive(Clone, Copy, Debug, Default)]
struct Spring {
    value: f32,
    rate: f32,
}

impl Spring {
    fn follow(&mut self, target: f32, omega: f32, dt: f32, max_rate: f32) {
        let accel = omega * omega * (target - self.value) - 2.0 * omega * self.rate;
        self.rate = (self.rate + accel * dt).clamp(-max_rate, max_rate);
        self.value += self.rate * dt;
    }
}

/// How far round the window turns to look up the cave's tunnel.
pub const LOOK: f32 = 0.9;
/// How far the boat heels to port along the trench's rim, so the window
/// looks down over its edge.
pub const PEER: f32 = 0.2;

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Seconds between the boat's turns about, and how long one takes.
const TURN_EVERY: f32 = 150.0;
const TURN_FOR: f32 = 55.0;
/// How far the boat falls behind the voyage through a turn, to catch up after.
const TURN_LAG: f32 = 0.9;
/// Closest the hull comes to the ground below and about it.
const CLEARANCE: f32 = 0.2;

#[derive(Debug, Default)]
pub struct Helm {
    last: Option<(f64, [f32; 3])>,
    /// The cruise height, filtered, before the spring.
    cruise: f32,
    height: Spring,
    pitch: Spring,
    roll: Spring,
    yaw: Spring,
    /// Full turns completed since the visit began, so the heading keeps
    /// counting up through each turn instead of unwinding.
    turns: f32,
    /// The turn under way, if one is: its number, and how far it has gone.
    turning: Option<(i32, f32)>,
}

impl Helm {
    pub fn steer(&mut self, seabed: &Seabed, voyage: Position, seconds: f64) -> Pose {
        let t = seconds as f32;
        let phase = seabed.seed * 40.0;

        let fresh = match self.last {
            None => true,
            Some((s, _)) => seconds < s || seconds - s > 0.5,
        };
        let dt = match self.last {
            Some((s, _)) if !fresh => (seconds - s) as f32,
            _ => 0.0,
        };
        if fresh {
            self.turns = 0.0;
            self.turning = None;
        }

        // A turn about every so often, though never in the tunnel. Whether a
        // turn happens is decided for the whole of it from where it starts.
        let cycle = (t + phase * 3.0) / TURN_EVERY;
        let n = cycle.floor() as i32;
        let into = cycle.fract() * TURN_EVERY;
        let start_x = voyage.x as f32 - into * SPEED as f32;
        let clear = (0..=6).all(|i| {
            let x = start_x + i as f32 * 0.6;
            seabed.share(x, corridor(x), CAVE) < 0.02
        });
        let progress = if into < TURN_FOR && clear && hash(n, 0, 900) > 0.2 {
            let f = into / TURN_FOR;
            f * f * f * (f * (f * 6.0 - 15.0) + 10.0)
        } else {
            0.0
        };
        if let Some((m, done)) = self.turning
            && (m != n || progress == 0.0)
        {
            // The last turn ended: count it, so the heading stays put.
            if done > 0.5 {
                self.turns += 1.0;
            }
            self.turning = None;
        }
        if progress > 0.0 {
            self.turning = Some((n, progress));
        }
        let lag = TURN_LAG * (PI * progress).sin().powi(2);

        // Along the corridor, with a slow surge in speed.
        let surge =
            0.15 * (t * TAU / 80.0 + phase).sin() + 0.05 * (t * TAU / 23.0 + phase * 1.3).sin();
        let x = voyage.x as f32 + surge - lag;
        let z = corridor(x) + 0.12 * (t * 0.07 + phase).sin();

        // The highest ground about and ahead sets the cruise, so a gully is
        // sailed over rather than followed down. The faster the boat goes, the
        // farther ahead it looks, so a swipe never meets a ridge unawares.
        let pace = match self.last {
            Some((s, p)) if !fresh && seconds > s => ((x - p[0]) / (seconds - s) as f32).abs(),
            _ => SPEED as f32,
        };
        let reach = (pace / SPEED as f32 * 0.35).clamp(1.0, 4.0);
        let mut highest = f32::MIN;
        let mut near = f32::MIN;
        // Samples stay close however far it looks, so no ridge slips between.
        let samples = (3.0 * reach / 0.25).ceil() as i32;
        for a in 0..=samples {
            let along = -0.6 * reach + a as f32 * 0.25;
            for side in [-0.5f32, 0.0, 0.5, 1.0] {
                // Along the corridor's own curve, not straight on into whatever
                // it bends away from.
                let ax = x + along;
                let g = seabed.ground(ax, corridor(ax) + (z - corridor(x)) + side);
                highest = highest.max(g);
                // The hard floor is only the ground under and just ahead of the
                // hull; looking far ahead is the cruise's business.
                if (-0.1..=0.4).contains(&along) && side.abs() <= 0.5 {
                    near = near.max(g);
                }
            }
        }
        let floor = near + CLEARANCE;
        // The lowest roof ahead steers the boat down in good time; only the
        // roof overhead is a hard limit.
        let mut ceiling = f32::MAX;
        for a in 0..=(1.5 * reach / 0.25).ceil() as i32 {
            ceiling = ceiling.min(seabed.ceiling(x + a as f32 * 0.25));
        }
        let overhead = seabed.ceiling(x);
        let cruise = highest + voyage.altitude as f32 * 0.8;

        if fresh {
            self.cruise = cruise;
            let start = cruise.max(floor).min(ceiling.max(floor));
            self.height = Spring {
                value: start,
                rate: 0.0,
            };
            self.pitch = Spring::default();
            self.roll = Spring::default();
            self.yaw = Spring {
                value: -corridor_slope(x).atan(),
                rate: 0.0,
            };
        }
        let vx = match self.last {
            Some((_, p)) if dt > 0.0 => (x - p[0]) / dt,
            _ => SPEED as f32,
        };
        if !fresh {
            // The cruise height drifts toward where it should be, and the
            // boat follows that, never the ground's every bump.
            self.cruise += (cruise - self.cruise) * (1.0 - (-dt / 2.5).exp());
            let target = self.cruise.max(floor).min(ceiling);
            let climb = 0.08 + vx.abs() * 0.5;
            // Stiffer in a fast passage, when there is more ground to follow;
            // the rush of bubbles past the glass covers the quicker motion.
            let omega = 0.4 + vx.abs().min(3.0) * 0.6;
            self.height.follow(target, omega, dt, climb);
            let height = self.height.value.clamp(floor - 0.08, overhead.max(floor));
            if height != self.height.value {
                self.height.value = height;
                self.height.rate *= 0.5;
            }
            // Nose up into a climb, down into a dive.
            let pitch = (self.height.rate.atan2(vx.abs().max(0.03)) * 0.8).clamp(-0.18, 0.18);
            self.pitch.follow(pitch, 0.8, dt, 0.2);
            // Along the corridor, a helmsman's small corrections, and the turns.
            let course = -corridor_slope(x).atan();
            let wander = 0.03 * (t * 0.05 + phase).sin() + 0.015 * (t * 0.13 + phase * 0.7).sin();
            // In the cave the helmsman swings the window round to look up the
            // tunnel, from a little before the mouth until the far one.
            let cave = |at: f32| seabed.share(at, corridor(at), CAVE);
            let look = LOOK * smooth(0.3, 0.9, cave(x).max(cave(x + 1.2)));
            let heading = course + wander + look + TAU * (self.turns + progress);
            self.yaw.follow(heading, 0.7, dt, 0.4);
            let bank = (-self.yaw.rate * 0.5 + 0.010 * (t * 0.21 + phase).sin()).clamp(-0.08, 0.08);
            let peer = PEER * smooth(0.3, 0.9, seabed.share(x, corridor(x), TRENCH));
            self.roll.follow(bank + peer, 0.9, dt, 0.2);
        }

        let pos = [x, self.height.value, z];
        let vel = match self.last {
            Some((_, p)) if dt > 0.0 => [
                (pos[0] - p[0]) / dt,
                (pos[1] - p[1]) / dt,
                (pos[2] - p[2]) / dt,
            ],
            _ => [SPEED as f32, 0.0, 0.0],
        };
        self.last = Some((seconds, pos));
        Pose {
            pos,
            basis: basis(self.yaw.value, self.pitch.value, self.roll.value),
            vel,
            depth: depth_reading(pos[1]),
        }
    }
}

/// The window's axes for a boat at `yaw`, `pitch` and `roll`: the boat's nose
/// is +x, its keel -y and the window looks along +z.
fn basis(yaw: f32, pitch: f32, roll: f32) -> [[f32; 3]; 3] {
    let m = super::world::place([0.0; 3], yaw, pitch, roll, [1.0; 3]);
    [
        [m[0][0], m[1][0], m[2][0]],
        [m[0][1], m[1][1], m[2][1]],
        [m[0][2], m[1][2], m[2][2]],
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::voyage::Voyage;

    #[test]
    fn the_boat_moves_smoothly_and_stays_off_the_seabed() {
        let seabed = Seabed::new(7, 0.0);
        let mut voyage = Voyage::default();
        let mut helm = Helm::default();
        let mut seconds = 5.0;
        let mut last: Option<Pose> = None;
        let mut turned = 0.0f32;
        for frame in 0..(60 * 600) {
            seconds += 1.0 / 60.0;
            if frame == 60 * 200 {
                voyage.travel(2, seconds);
            }
            let at = voyage.advance(seconds, 0.0);
            let pose = helm.steer(&seabed, at, seconds);
            let under = seabed.ground(pose.pos[0], pose.pos[2]);
            assert!(
                pose.pos[1] > under + 0.05,
                "into the seabed at {seconds:.1} s"
            );
            assert!(
                pose.pos[1] <= seabed.ceiling(pose.pos[0]) + 0.05,
                "through the roof at {seconds:.1} s"
            );
            if let Some(p) = last {
                // No jumps from one frame to the next: gentle when cruising,
                // quicker but still smooth in a swipe's fast passage.
                // In a fast passage the rush of bubbles past the glass hides
                // the boat following the seabed's steepest shelves.
                let fast = pose.vel[0].abs() > 0.3;
                let most = if fast { 0.08 } else { 0.01 };
                assert!(
                    (pose.pos[1] - p.pos[1]).abs() < most,
                    "a lurch at {seconds:.1} s"
                );
                for (a, b) in pose.basis.iter().zip(p.basis.iter()) {
                    for k in 0..3 {
                        assert!((a[k] - b[k]).abs() < 0.01, "a jolt at {seconds:.1} s");
                    }
                }
                turned +=
                    (pose.basis[2][0] * p.basis[2][2] - pose.basis[2][2] * p.basis[2][0]).abs();
            }
            last = Some(pose);
        }
        assert!(
            turned > TAU,
            "the boat should have turned right round, turned {turned:.2}"
        );
    }
}
