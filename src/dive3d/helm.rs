//! How the Nautilus moves: a helmsman's hand on a heavy boat.
//!
//! The boat is twenty metres long and three high, about 1.8 by 0.27 world
//! units, and the salon window is on its port side, halfway along and halfway
//! up. [`crate::voyage`] says how far along the voyage it is and how high it
//! means to cruise; the helm turns that into a pose that moves like a hull of
//! that size and weight.
//!
//! It goes where it points. Its track is the corridor's meander with a slow
//! wander of the helmsman's own, never bending tighter than about ten hull
//! lengths' radius, and the nose follows the track; it banks a degree or two
//! into each bend. It climbs and dives on its ballast and planes at under a
//! metre a second, gathering way over a quarter of a minute, noses up or down
//! a few degrees as it does, and reads the ground several hull lengths ahead,
//! so it starts up over a ridge in good time and sails over a gully instead
//! of ducking into it. The whole hull, not just the window, keeps a metre of
//! water under it and inside the cave's tunnel. Near the surface the swell
//! lifts and rolls it a little, less the deeper it goes.
//!
//! A swipe's passage is the voyage fast-forwarded: the climbs and dives run
//! on the voyage's own clock, so the boat rides over the ground as it would
//! have at cruising speed, only quicker, behind the rush of bubbles, while
//! its attitude still turns no faster than a few times its cruising rate.
//!
//! In the lava cave, at dead slow, the helmsman sets the boat a little across
//! the tunnel so the window looks up it, no farther than the hull clears both
//! walls, and swings it straight again beyond the far mouth.
//!
//! Seen from the salon window, which looks out to port, the boat's pitch turns
//! the view about its own axis, its roll tips the view up and down, and its
//! heading pans it.

use std::f32::consts::TAU;

use super::world::{CAVE, Seabed, TUNNEL, TUNNEL_SHARE, corridor, corridor_slope};
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

/// The hull: a spindle twenty metres long and three round, in world units of
/// about eleven metres.
pub const LENGTH: f32 = 1.8;
pub const HEIGHT: f32 = 0.27;
const RADIUS: f32 = HEIGHT * 0.5;
/// The least water the helmsman leaves under the keel and between the hull
/// and the tunnel's walls: about a metre.
const KEEL: f32 = 0.09;
/// The fastest the boat climbs or dives on its ballast and planes, and how
/// quickly it gathers that way, in world units a second (of the voyage's own
/// clock): under a metre a second, reached over a quarter of a minute.
const CLIMB: f32 = 0.07;
const LIFT: f32 = 0.003;
/// How far ahead, in world units, the helmsman reads the ground.
const AHEAD: f32 = 5.0;
/// The most the boat noses up or down, and banks into a bend, in radians.
const PITCH: f32 = 0.10;
const BANK: f32 = 0.035;
/// How many times its cruising rate the boat's attitude may turn in a
/// swipe's passage.
const BRISK: f32 = 3.0;
/// How far round the helmsman would set the boat across the cave's tunnel to
/// look up it, and the fastest he swings it. The tunnel's walls allow less
/// where they close in or the boat rides high under the vault.
pub const LOOK: f32 = 0.55;
const CRAB_RATE: f32 = 0.008;

fn smooth(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A critically damped spring: no overshoot, no sudden start.
#[derive(Clone, Copy, Debug, Default)]
struct Spring {
    value: f32,
    rate: f32,
}

impl Spring {
    fn at(value: f32) -> Self {
        Self { value, rate: 0.0 }
    }

    fn follow(&mut self, target: f32, omega: f32, dt: f32, max_rate: f32) {
        let accel = omega * omega * (target - self.value) - 2.0 * omega * self.rate;
        self.rate = (self.rate + accel * dt).clamp(-max_rate, max_rate);
        self.value += self.rate * dt;
    }

    /// Head for `target` as a heavy boat would on its ballast: never faster
    /// than `most`, never gathering or losing way faster than `accel`, and
    /// easing off in time to arrive without overshooting.
    /// `drift` is how fast the target itself is moving, which the boat
    /// matches rather than trailing behind.
    /// Nor does it ever go so fast it could not stop short of `room`, the
    /// hard limits below and above.
    fn heave_to(&mut self, target: f32, drift: f32, room: (f32, f32), accel: f32, dt: f32) {
        let gap = target - self.value;
        let closing = gap.signum() * (gap.abs() * 0.2).min((1.6 * accel * gap.abs()).sqrt());
        let stop = |space: f32| (2.4 * accel * space.max(0.0)).sqrt();
        let wanted = (closing + drift)
            .clamp(-CLIMB, CLIMB)
            .clamp(-stop(self.value - room.0), stop(room.1 - self.value));
        // It gathers way gently, but floods or blows its tanks harder to stop.
        let change = (wanted - self.rate) * 0.5;
        let limit = if change * self.rate < 0.0 {
            2.0 * accel
        } else {
            accel
        };
        let change = change.clamp(-limit, limit);
        self.rate += change * dt;
        self.value += self.rate * dt;
    }
}

/// The hull's track at `x`: the middle of the corridor, with a wander of the
/// helmsman's own about it, and its slope dz/dx. Its tightest bend is about
/// twenty units' radius, so the hull's way stays clear of what stands beside
/// the corridor.
fn track(x: f32, phase: f32) -> (f32, f32) {
    let (a, b) = (x * 0.29 + phase, x * 0.67 + phase * 1.3);
    (
        corridor(x) + 0.07 * a.sin() + 0.035 * b.sin(),
        corridor_slope(x) + 0.07 * 0.29 * a.cos() + 0.035 * 0.67 * b.cos(),
    )
}

/// How far up the boat can come from rest within `seconds`, climbing as
/// briskly as it may, with a margin for the last few seconds' easing in.
fn reach(seconds: f32) -> f32 {
    let (most, accel) = (CLIMB * 0.8, LIFT * 0.6);
    let seconds = (seconds - 5.0).max(0.0);
    let gathering = most / accel;
    if seconds < gathering {
        0.5 * accel * seconds * seconds
    } else {
        most * (seconds - 0.5 * gathering)
    }
}

/// The hull's half-thickness `along` from its middle, a spindle's.
fn girth(along: f32) -> f32 {
    RADIUS * (1.0 - (2.0 * along / LENGTH).powi(2)).max(0.0).sqrt()
}

/// Stations along the hull from stern to bow, where it is checked against the
/// ground and the tunnel.
const STATIONS: [f32; 7] = [-0.9, -0.6, -0.3, 0.0, 0.3, 0.6, 0.9];

#[derive(Debug, Default)]
pub struct Helm {
    last: Option<f64>,
    /// Where the voyage was last frame, to tell a passage's pace.
    voyage_x: f64,
    last_pos: [f32; 3],
    /// The ground the cruise is reckoned from, filtered.
    cruise: f32,
    /// The height the boat was making for last frame, and how fast that
    /// is moving, filtered.
    target: f32,
    drift: f32,
    /// The hull's middle's height, the heading of its nose along the track,
    /// how far it is set across the track in the cave, its pitch and roll.
    height: Spring,
    course: Spring,
    crab: Spring,
    pitch: Spring,
    roll: Spring,
}

impl Helm {
    pub fn steer(&mut self, seabed: &Seabed, voyage: Position, seconds: f64) -> Pose {
        let t = seconds as f32;
        let phase = seabed.seed * 40.0;
        let fresh = match self.last {
            None => true,
            Some(s) => seconds < s || seconds - s > 0.5,
        };
        let dt = match self.last {
            Some(s) if !fresh => (seconds - s) as f32,
            _ => 0.0,
        };
        // How many times faster than cruising the voyage is going: in a
        // swipe's passage the boat's climbs and dives run on that clock.
        let warp = if dt > 0.0 {
            ((voyage.x - self.voyage_x).abs() as f32 / dt / SPEED as f32).clamp(1.0, 60.0)
        } else {
            1.0
        };
        self.voyage_x = voyage.x;
        let voyage_dt = dt * warp;
        let brisk = warp.min(BRISK);
        let attitude_dt = dt * brisk;

        // A heavy boat holds its way: only a slow swing of a few percent.
        let x = voyage.x as f32 + 0.025 * (t * TAU / 75.0 + phase).sin();
        let (z, slope) = track(x, phase);
        let heading = -slope.atan();
        // How fast the track bends, per unit along it, for the bank.
        let bend = -(track(x + 0.05, phase).1.atan() - track(x - 0.05, phase).1.atan()) / 0.1;

        let alt = (voyage.altitude as f32 * 0.8).max(RADIUS + KEEL + 0.05);
        let pitch = self.pitch.value;
        let (sy, cy) = (self.course.value + self.crab.value).sin_cos();
        // The hull's floor: the ground under every station from stern to
        // bow, across its beam, with its girth and its pitch.
        let mut floor = f32::MIN;
        for along in STATIONS {
            let r = girth(along);
            for across in [-RADIUS, 0.0, RADIUS] {
                let g = seabed.ground(x + along * cy + across * sy, z - along * sy + across * cy);
                floor = floor.max(g + r + KEEL - along * pitch.sin());
            }
        }
        // The ground about and just ahead sets the cruise; it is filtered, so
        // a gully is sailed over rather than followed down.
        let mut about = f32::MIN;
        for k in 0..=6 {
            let ax = x - 0.6 + k as f32 * 0.3;
            let az = track(ax, phase).0;
            for side in [-0.35, 0.0, 0.35] {
                about = about.max(seabed.ground(ax, az + side));
            }
        }
        // The roof over the whole hull, not just the window: where the cave's
        // floor falls into the massif its vault comes down over the bow
        // first. The ceiling keeps the window well clear of the vault, the
        // surface or the ice; the hull's slender ends may come a little
        // nearer, which is the hard limit.
        let overhead = STATIONS
            .iter()
            .step_by(2)
            .map(|&along| {
                let ceiling = seabed.ceiling(x + along * cy) - along * pitch.sin();
                (ceiling, ceiling + 0.2 - girth(along))
            })
            .fold((f32::MAX, f32::MAX), |a, b| (a.0.min(b.0), a.1.min(b.1)));
        let (overhead, vault) = overhead;
        let mut roof = overhead;
        // Ground farther ahead is a climb to start in good time, and a roof a
        // dive: the boat must be there by the time it arrives.
        let mut rise = f32::MIN;
        for k in 1..=((AHEAD - 0.9) / 0.3) as i32 {
            let d = 0.9 + k as f32 * 0.3;
            let ax = x + d;
            let az = track(ax, phase).0;
            let g = seabed.ground(ax, az - 0.3).max(seabed.ground(ax, az + 0.3));
            let time = d / SPEED as f32;
            rise = rise.max(g + alt - reach(time));
        }
        // The roof matters from the bow on, and closer at hand: a vault
        // coming down at the cave's mouth must be met in good time.
        for k in 1..=10 {
            let d = k as f32 * 0.3;
            roof = roof.min(seabed.ceiling(x + 0.9 + d) + 0.6 * reach(d / SPEED as f32));
        }

        if fresh {
            self.cruise = about;
            self.course = Spring::at(heading);
            self.pitch = Spring::default();
            self.roll = Spring::default();
            self.crab = Spring::default();
        } else {
            self.cruise += (about - self.cruise) * (1.0 - (-voyage_dt / 4.0).exp());
        }
        let target = (self.cruise + alt).max(rise).min(roof).max(floor);
        if fresh {
            self.height = Spring::at(target.min(overhead.max(floor)));
            self.target = target;
            self.drift = 0.0;
        } else {
            if voyage_dt > 0.0 {
                let drift = ((target - self.target) / voyage_dt).clamp(-CLIMB, CLIMB);
                self.drift += (drift - self.drift) * (1.0 - (-voyage_dt).exp());
            }
            self.target = target;
            // The hard limits leave the boat some of its margin, so it is
            // seldom caught by them and stopped short; it brakes for them.
            let (lowest, highest) = (floor - 0.25 * KEEL, vault.max(floor));
            let room = (floor - 0.1 * KEEL, highest);
            // In fine steps, as a passage runs the voyage's clock fast.
            let steps = (voyage_dt / 0.1).ceil().max(1.0);
            for _ in 0..steps as i32 {
                self.height
                    .heave_to(target, self.drift, room, LIFT, voyage_dt / steps);
            }
            let held = self.height.value.clamp(lowest, highest);
            if held != self.height.value {
                self.height.value = held;
                self.height.rate = 0.0;
            }

            // Nose up into a climb, down into a dive, with the rest on the
            // ballast.
            let climb = (self.height.rate / SPEED as f32).atan() * 0.6;
            self.pitch
                .follow(climb.clamp(-PITCH, PITCH), 0.35, attitude_dt, 0.012);
            self.course.follow(heading, 0.5, attitude_dt, 0.05);
            // Bank a little into the bend, as the sail heels a boat inward.
            let turning = bend * SPEED as f32 * warp;
            let bank = (-turning * 10.0).clamp(-BANK, BANK);
            self.roll.follow(bank, 0.3, attitude_dt, 0.01);

            // In the cave the helmsman sets the boat across the tunnel, so the
            // window looks up it, from a little before the mouth until the far
            // one, no farther than the hull fits.
            let cave = |at: f32| seabed.share(at, corridor(at), CAVE);
            let wanted = LOOK * smooth(0.3, 0.9, cave(x).max(cave(x + 1.5)));
            let fits = if wanted > 0.0 || self.crab.value > 0.0 {
                self.fits(seabed, x, z)
            } else {
                0.0
            };
            let before = self.crab.value;
            self.crab
                .follow(wanted.min(fits), 0.12, attitude_dt, CRAB_RATE);
            // Should the walls close in, straighten up at once, but no faster
            // than the boat turns; the keel's metre of water covers the lag.
            if self.crab.value > fits {
                let back = (before - CRAB_RATE * attitude_dt).max(fits);
                self.crab.rate = self.crab.rate.min(0.0);
                self.crab.value = back;
            }
        }

        // Near the surface the swell reaches down to the boat, fading by half
        // in each eleven metres or so: a heave of a few decimetres, and a
        // roll and pitch of a fraction of a degree.
        let swell = (0.69 * self.height.value.min(0.0)).exp();
        let wave = |period: f32, at: f32| (t * TAU / period + phase * at).sin();
        let heave = 0.045 * swell * (0.6 * wave(9.3, 1.0) + 0.4 * wave(13.7, 2.3));
        let roll = self.roll.value + 0.008 * swell * wave(11.1, 0.7);
        let pitch = self.pitch.value + 0.004 * swell * wave(9.3, 1.9);
        let basis = basis(self.course.value + self.crab.value, pitch, roll);
        // The window is on the hull's side, a girth out from its middle.
        let out = basis[2];
        let pos = [
            x + out[0] * RADIUS,
            self.height.value + heave + out[1] * RADIUS,
            z + out[2] * RADIUS,
        ];
        let vel = if dt > 0.0 {
            [
                (pos[0] - self.last_pos[0]) / dt,
                (pos[1] - self.last_pos[1]) / dt,
                (pos[2] - self.last_pos[2]) / dt,
            ]
        } else {
            [SPEED as f32, 0.0, 0.0]
        };
        self.last = Some(seconds);
        self.last_pos = pos;
        Pose {
            pos,
            basis,
            vel,
            depth: depth_reading(pos[1]),
        }
    }

    /// How far across the cave's tunnel the hull with its middle at (x, z)
    /// may be set, keeping a keel's water between it and the walls and vault.
    fn fits(&self, seabed: &Seabed, x: f32, z: f32) -> f32 {
        // Take the arch's narrowest, most ragged radius, and reckon with it
        // from well before the mouth, so the boat is never set across
        // farther than the tunnel will take it.
        let wide = TUNNEL * 0.91 - KEEL;
        let high = 0.85 * TUNNEL * 0.91 - KEEL;
        let course = self.course.value;
        let c0 = course.cos();
        let centres = STATIONS.map(|along| {
            let px = x + along * c0;
            (seabed.share(px, corridor(px), CAVE) >= TUNNEL_SHARE * 0.4)
                .then(|| seabed.tunnel_floor(px) + 0.25 * TUNNEL)
        });
        let clear = |angle: f32| {
            let (s, c) = (course + angle).sin_cos();
            STATIONS.iter().zip(centres).all(|(&along, centre)| {
                let Some(centre) = centre else {
                    return true;
                };
                let (px, pz) = (x + along * c, z - along * s);
                let r = girth(along);
                let across = pz - corridor(px);
                let up = self.height.value - centre + along * self.pitch.value.sin();
                [(r, 0.0), (-r, 0.0), (0.0, r)].iter().all(|&(dz, dy)| {
                    ((across + dz) / wide).powi(2) + ((up + dy) / high).powi(2) <= 1.0
                })
            })
        };
        if !clear(0.0) {
            return 0.0;
        }
        if clear(LOOK) {
            return LOOK;
        }
        let (mut lo, mut hi) = (0.0, LOOK);
        for _ in 0..6 {
            let mid = 0.5 * (lo + hi);
            if clear(mid) {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        lo
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
mod survey;

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
            }
            last = Some(pose);
        }
    }
}
