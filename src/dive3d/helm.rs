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
//! In open water the helmsman lets the boat crab the same way, slowly and a
//! good deal less: every couple of minutes he swings the nose off the track,
//! so the window looks a little ahead or a little astern, and the boat dips
//! a little as it looks ahead and lifts as it looks back. The scenery then
//! comes at the glass, or falls away from it, on the diagonal, rather than
//! only sliding past it, and the voyage is seen in depth as well as across
//! and up and down. He keeps the swing small among the forest's trunks and
//! the city's columns, which stand close beside the corridor, and takes it
//! out before the cave, where the tunnel's walls leave room only for his
//! look up it.
//!
//! Seen from the salon window, which looks out to port, the boat's pitch turns
//! the view about its own axis, its roll tips the view up and down, and its
//! heading pans it.

use std::f32::consts::TAU;

use super::world::{
    ATLANTIS, CAVE, FOREST, Seabed, TRENCH, TUNNEL, TUNNEL_SHARE, corridor, corridor_slope,
};
use crate::voyage::{Position, SPEED};

/// The boat's pose for one frame.
#[derive(Clone, Copy, Debug)]
pub struct Pose {
    pub pos: [f32; 3],
    /// The window's right, up and outward directions in the world.
    pub basis: [[f32; 3]; 3],
    /// How fast the boat is going, in world units a second.
    pub vel: [f32; 3],
    /// Level and outward, the way the window would look were the helmsman
    /// not swinging the boat off its track, in world x and z. Things that
    /// keep their distance from the track keep their distance along this
    /// however the boat swings, so what the window is kept clear of stays
    /// clear rather than vanishing as the boat swings towards it.
    pub abeam: [f32; 2],
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
            abeam: [0.0, 1.0],
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
/// Blowing its tanks, as when a shelf ahead rises faster than it can climb
/// on its planes, it comes up half as fast again, and gathers that way
/// twice as quickly.
const BLOWN: f32 = 1.5;
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
/// How far round the helmsman swings the boat off its track in open water,
/// either way, and the fastest he swings it: twenty degrees, so the window
/// sees a third of the boat's way coming at it or falling away from it, at a
/// quarter of a degree a second, so a whole swing from looking astern to
/// looking ahead takes a couple of minutes. With the corridor's own bends it
/// stays within the turning rate of a hull of its size.
pub const SWING: f32 = 0.4;
const SWING_RATE: f32 = 0.005;
/// How much lower than its cruise the boat runs while it looks ahead, and
/// how much higher while it looks astern, as a share of its height over the
/// ground: it dips into the scenery it is turning towards.
const DIP: f32 = 0.3;
/// How far the boat heels to port along the trench's rim, so the window
/// looks down over its edge: a few degrees, all a heavy hull will give.
pub const PEER: f32 = 0.07;

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
    fn heave_to(
        &mut self,
        target: f32,
        drift: f32,
        room: (f32, f32),
        (most, accel): (f32, f32),
        dt: f32,
    ) {
        let gap = target - self.value;
        let closing = gap.signum() * (gap.abs() * 0.2).min((1.6 * accel * gap.abs()).sqrt());
        let stop = |space: f32| (2.4 * accel * space.max(0.0)).sqrt();
        let wanted = (closing + drift)
            .clamp(-most, most)
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

/// How far the course weaves to either side of the corridor at a biome
/// boundary, and the wavelength of the weave: two legs, so every boundary falls
/// at the crest of one of its bends. The boat is therefore always turning as
/// one sea gives way to the next, rather than crossing an invisible line on a
/// straight course. A quarter of a unit either way is inside the corridor's
/// clearance and gives a heading swing of a few degrees.
const GATE_TURN: f32 = 0.13 * (crate::voyage::LEG as f32 / 5.0);
const GATE_PERIOD: f32 = 2.0 * crate::voyage::LEG as f32;

/// The weave that puts the turns at the biome boundaries: a cosine, whose
/// second derivative is greatest (and heading straight across the corridor)
/// exactly at each boundary.
fn gate(x: f32, open: f32) -> (f32, f32) {
    let k = TAU / GATE_PERIOD;
    (
        GATE_TURN * open * (k * x).cos(),
        -GATE_TURN * k * open * (k * x).sin(),
    )
}

/// The start of the nearest stretch of lava cave along the corridor, in
/// world x, if this visit has one. The tunnel is about five units long, and
/// the gate's weave eases out over the five before it.
fn cave_start(seabed: &Seabed, x: f32) -> Option<f32> {
    let leg = seabed.order.iter().position(|&b| b == CAVE)? as f32;
    let leg_length = crate::voyage::LEG as f32;
    let period = leg_length * crate::voyage::BIOMES as f32;
    let base = (leg - 0.05) * leg_length;
    Some(base + ((x - base) / period).round() * period)
}

/// The hull's track at `x`: the middle of the corridor, with a wander of the
/// helmsman's own about it and the turn into the next biome, and its slope
/// dz/dx. Its tightest bend is about ten units' radius, so the hull's way stays
/// clear of what stands beside the corridor.
fn track(x: f32, phase: f32, open: f32) -> (f32, f32) {
    let (a, b) = (x * 0.29 + phase, x * 0.67 + phase * 1.3);
    let (across, slope) = gate(x, open);
    (
        corridor(x) + 0.07 * a.sin() + 0.035 * b.sin() + across,
        corridor_slope(x) + 0.07 * 0.29 * a.cos() + 0.035 * 0.67 * b.cos() + slope,
    )
}

/// How open the water is at `x` for the gate's weave and the helmsman's
/// swing: nothing in the cave's tunnel, easing back over the five units
/// either side of it, a lane change the hull can make, rather than following
/// the biome blend's own short ramp into the tunnel.
fn open(seabed: &Seabed, x: f32) -> f32 {
    cave_start(seabed, x).map_or(1.0, |cave| {
        let out = (cave - x).max(x - (cave + 5.2)).max(0.0);
        smooth(0.0, 5.0, out)
    })
}

/// How far, as a share of [`SWING`], the boat may be swung off its track at
/// `x`. The forest's trunks and the city's columns stand within about half a
/// unit of the corridor, which the hull's ends would reach set farther
/// across, so there it is held nearly straight, and it is straight in the
/// cave's tunnel, where the crab takes over.
fn room(seabed: &Seabed, x: f32) -> f32 {
    let share = |id: usize| seabed.share(x, corridor(x), id);
    (1.0 - 0.7 * share(FOREST) - 0.9 * share(ATLANTIS)).max(0.0) * open(seabed, x)
}

/// Where the helmsman means to swing the boat at `x`, off its track, in
/// radians (positive turns the window ahead), and the most he will allow
/// there. The swing is a long, uneven weave of the voyage's own, so the same
/// stretch of sea is always seen the same way, saturating a little so it
/// holds its angle for a while between swings. He reads the room several
/// hull lengths ahead, a minute and more at cruising speed, so he has
/// straightened up by the time the trunks close in, rather than being
/// brought straight by them.
fn swing(seabed: &Seabed, x: f32, phase: f32) -> (f32, f32) {
    let weave = 0.8 * (x * TAU / 19.0 + phase * 0.9).sin()
        + 0.5 * (x * TAU / 8.3 + phase * 2.1).sin()
        + 0.1;
    let most = SWING * room(seabed, x);
    let ahead = [1.5, 3.0, 4.5]
        .iter()
        .fold(most, |least, &d| least.min(SWING * room(seabed, x + d)));
    ((SWING * (1.8 * weave).tanh()).clamp(-ahead, ahead), most)
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
    /// how far it is set across the track in the cave and swung off it in
    /// open water, its pitch and roll.
    height: Spring,
    course: Spring,
    crab: Spring,
    swing: Spring,
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
        // Ease the weave out over the five units before the mouth.
        let open = open(seabed, x);
        let (z, slope) = track(x, phase, open);
        let heading = -slope.atan();
        // How fast the track bends, per unit along it, for the bank.
        let bend =
            -(track(x + 0.05, phase, open).1.atan() - track(x - 0.05, phase, open).1.atan()) / 0.1;

        let (swung, most) = swing(seabed, x, phase);
        if fresh {
            self.swing = Spring::at(swung);
        }
        // Dipping as it swings to look ahead, lifting as it looks astern.
        let dip = 1.0 - DIP * self.swing.value / SWING;
        let alt = (voyage.altitude as f32 * 0.8 * dip).max(RADIUS + KEEL + 0.05);
        let pitch = self.pitch.value;
        let (sy, cy) = (self.course.value + self.crab.value + self.swing.value).sin_cos();
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
            let az = track(ax, phase, open).0;
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
            let az = track(ax, phase, open).0;
            // Across the whole width the hull sweeps, its ends swinging
            // out as far as half its length when it is set at an angle.
            let g = [-0.6, -0.3, 0.0, 0.3, 0.6]
                .iter()
                .fold(f32::MIN, |high, &dz| high.max(seabed.ground(ax, az + dz)));
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
            // The swing, slowly, and never farther than the place allows:
            // where a swipe's passage carries the boat into the forest or
            // towards the cave quicker than it could swing back, the bound
            // brings it straight, over the passage's own few seconds. While
            // the course is still coming round, after a passage or in the
            // corridor's tighter bends, the swing waits on it, so the two
            // together never turn the nose faster than the crab alone may.
            let spare = (CRAB_RATE - self.course.rate.abs()).clamp(0.1 * SWING_RATE, SWING_RATE);
            self.swing.follow(swung, 0.12, attitude_dt, spare);
            if self.swing.value.abs() > most {
                self.swing.value = self.swing.value.clamp(-most, most);
                self.swing.rate = 0.0;
            }
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
            // Should the ground ahead need the boat well above where it is,
            // more than its planes can make in time, it blows its tanks;
            // never to climb into a roof it must stay under.
            let blow = smooth(0.4, 1.2, rise.min(roof) - self.height.value);
            let pace = (CLIMB * (1.0 + (BLOWN - 1.0) * blow), LIFT * (1.0 + blow));
            let before = self.height.value;
            for _ in 0..steps as i32 {
                self.height
                    .heave_to(target, self.drift, room, pace, voyage_dt / steps);
            }
            // Caught by a hard limit, the boat goes as the limit goes, the
            // ground under it or the vault over it, rather than stopping dead.
            let held = self.height.value.clamp(lowest, highest);
            if held != self.height.value {
                self.height.value = held;
                self.height.rate = ((held - before) / voyage_dt).clamp(-pace.0, pace.0);
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
            let peer = PEER * smooth(0.3, 0.9, seabed.share(x, corridor(x), TRENCH));
            self.roll.follow(bank + peer, 0.3, attitude_dt, 0.01);

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
            // Where the walls close in ahead, the helmsman eases off in good
            // time, rather than straightening up with a lurch when he gets
            // there: the least the tunnel takes over the next few hull lengths.
            let ahead = if fits > 0.0 {
                [0.6, 1.2, 1.8].iter().fold(fits, |least, &d| {
                    least.min(self.fits(seabed, x + d, z + corridor(x + d) - corridor(x)))
                })
            } else {
                fits
            };
            let before = self.crab.value;
            self.crab
                .follow(wanted.min(ahead), 0.12, attitude_dt, CRAB_RATE);
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
        let basis = basis(
            self.course.value + self.crab.value + self.swing.value,
            pitch,
            roll,
        );
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
        let (sa, ca) = (self.course.value + self.crab.value).sin_cos();
        Pose {
            pos,
            basis,
            vel,
            abeam: [sa, ca],
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
        let course = self.course.value + self.swing.value;
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

    /// Every biome boundary falls in the middle of a real turn, and the turn
    /// is taken out where the cave's walls leave no room for it.
    #[test]
    fn the_course_turns_at_every_biome_boundary() {
        use crate::voyage::LEG;
        let phase = 0.7;
        // The turn is spread over a fixed fraction of the longer leg now, so
        // sample it over the same fraction; a fixed quarter unit would only
        // see the gentle top of a much longer bend.
        let span = (LEG as f32 / 5.0) * 0.25;
        for k in 1..8 {
            let x = k as f32 * LEG as f32;
            let before = track(x - span, phase, 1.0).1.atan();
            let after = track(x + span, phase, 1.0).1.atan();
            assert!(
                (after - before).abs() > 0.012,
                "the course runs straight through the boundary at leg {k}"
            );
            // The turn is spread over half a unit at each side, not a hinge.
            assert!(
                (after - before).abs() < 0.105,
                "the turn at leg {k} is too sharp for the hull"
            );
            // Without the gate the natural wander over a quarter unit is
            // small; the cave gate itself is tested by the cave survey.
            let straight = track(x - 0.25, phase, 0.0).1.atan();
            let still = track(x + 0.25, phase, 0.0).1.atan();
            assert!(
                (still - straight).abs() < (after - before).abs() * 0.8,
                "the cave must be entered straight"
            );
        }
    }

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
