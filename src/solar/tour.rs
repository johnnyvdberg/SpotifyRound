//! The grand tour of one system, as a timeline: where the camera is and what
//! the HUD says at any moment of it.
//!
//! A visit drops out of the previous hyperjump outside the system, crosses to
//! the first planet on the itinerary, and for each planet settles into an
//! orbit with its moons and rings in view while the scanner surveys it, then
//! crosses to the next, never stopping: transfers leave and enter the orbits
//! at their speeds, the way they run. A giant with a moon the
//! size of a planet is visited last, and the moon after it, a short hop out.
//! Crossings are not straight: each follows a spline through the closest
//! point of a flyby, swinging round the star or past a planet not otherwise
//! visited, and slows as it passes. After the last stop it climbs out and
//! flies across the system to a jump point at its edge, clear of the star's
//! gravity well, turns to the next star and spools up its drive, each drive
//! in its own stages, then jumps: the jump is the visit's last segment, and
//! the next visit begins where it ends. Ten to twelve minutes in all.
//!
//! Now and then the next star is only a relay on the way to the
//! destination (`Tour::plan` with `transit`): the visit drops out at one
//! edge of the relay system, crosses it to a jump point at another, and
//! jumps on, a hop or two to the star it was making for.
//!
//! Everything here is a pure function of the time into the visit, so any
//! moment can be shown on its own (the previews do) and nothing drifts.

use std::f64::consts::{PI, TAU};

use super::system::{AU, KM, Planet, REAL_AU, Stop, System};
use super::voyage::{self, Event, Passage, Shot};

/// Seconds each part of the tour takes.
pub const TRANSFER: f64 = 45.0;
/// A transfer that swings past the star or another planet on the way.
pub const FLYBY: f64 = 64.0;
/// The short hop from a giant out to its moon.
pub const HOP: f64 = 32.0;
/// The orbit, unhurried: on the worlds the scanner surveys
/// (`scan::surveyed`), its survey takes the first ten seconds.
pub const HIGH: f64 = 70.0;
pub const DEPART: f64 = 20.0;
/// Out of the last stop's pull to the jump point at the system's edge.
pub const CRUISE: f64 = 48.0;
/// Across a relay system, from where the ship dropped out to the jump point.
pub const CROSSING: f64 = 64.0;
/// The drive's preparation at the jump point, in its stages (`SPOOL_STAGES`).
pub const SPOOL: f64 = 36.0;
/// The last leg of the jump (`voyage::LEG`): the jump is now two or three
/// short legs with something met between them (`voyage`).
pub const JUMP: f64 = voyage::LEG;
/// How fast the ship drifts on through a stopover between legs, units a
/// second (for the minimap and the motion only: nothing of the system shows).
const DEEP_DRIFT: f64 = 20.0;
/// The start-up effect fading into the light at a leg after a re-engage.
const REJUMP_FADE: f64 = 3.0;
/// Where each of the drive's preparation stages begins, as a share of
/// `SPOOL`: aligning on the target star, the drive's own three, and the
/// engagement, in which its light rises into the jump.
pub const SPOOL_STAGES: [f64; 5] = [0.0, 0.16, 0.4, 0.66, 0.88];
/// The jump's first seconds, in which the drive's start-up effect fades
/// into its light.
const SPOOL_FADE: f64 = 9.0;
/// How fast the ship is going into the jump: units a second.
const JUMP_SPEED: f64 = 400.0;
/// The chance a jump stops at a relay star on the way, and the most relays
/// in a row. Rarer than it was: the gates, wormholes and encounters between
/// the legs of every jump break the trip up now.
pub const RELAY_CHANCE: f64 = 0.18;
/// Where a jump point lies: beyond the outermost orbit by this share, and
/// this far beyond that again.
const EDGE: f64 = 1.25;
const EDGE_MARGIN: f64 = 600.0;
/// The jump's last seconds, in which the drive slows (`drive_clock`).
const SLOWING: f64 = 4.0;
/// The last second of the jump, in which a flare builds at the heading.
const FLARE: f64 = 1.0;
/// How long the drive's light takes to tear open on arrival, over the new
/// system (`View::reveal`).
pub const DROP_OUT: f64 = 2.4;
/// The hyperjump drives, each its own look (`fs_warp` in `frame.wgsl`); a
/// gate's conduit and a wormhole's throat follow them in `voyage::LOOKS`.
pub const DRIVES: [&str; 5] = [
    "Lumen Drift",
    "Tachyon Tunnel",
    "Fold Drive",
    "Ion Slipstream",
    "Bubble Drive",
];

/// Field of view, the tangent of half of it.
const HALF_FOV: f64 = 0.4663; // 50°
/// The stop just left keeps its maps until its radius is this share of half
/// the view: 1% is 5 pixels on the kiosk, where its flat colour passes.
const LEFT_BEHIND: f64 = 0.01;
/// Points along a transfer path its timing is worked out at.
const SAMPLES: usize = 400;
/// The shares of a transfer's time given over to slowing the approach, and
/// the climb away from the body left, in proportion to the nearness of the
/// body (see `Path::time`).
const APPROACH: f64 = 0.3;
const LEAVE: f64 = 0.12;
/// Where in a transfer the capture burn begins: as the approach slows, from
/// some fifteen radii out.
const CAPTURE: f64 = 0.8;
/// How close, in moon radii, the camera may come to a moon; how far either
/// side of now, in seconds, the keep-away looks for it; and how much each
/// look counts (a Hann window, summing to 1).
const MOON_FLOOR: f64 = 3.0;
const MOON_SMOOTH: f64 = 0.5;
const MOON_SPAN: [f64; 7] = [0.0366, 0.125, 0.2134, 0.25, 0.2134, 0.125, 0.0366];

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Crossing to stop `to`: from the drop-out point, or from another stop.
    Transfer {
        from: Option<Stop>,
        to: Stop,
    },
    High(Stop),
    Depart(Stop),
    /// Out across the system to the jump point at its edge: climbing away
    /// from the last stop, or (a relay) from where the ship dropped out.
    Cruise {
        from: Option<Stop>,
    },
    /// At the jump point, the drive spooling up toward the next star.
    Spool,
    /// A leg of the jump (from 0), out in the deep between the stars.
    Jump(u8),
    /// Between legs `k` and `k + 1`: what the ship meets there
    /// (`voyage::Event`), dropped out into, or a gate or wormhole flown.
    Stopover(u8),
    /// The drive re-engaging after stopover `k`.
    Rejump(u8),
}

impl Phase {
    /// Out of the system, in the jump or between its legs.
    pub fn deep(self) -> bool {
        matches!(self, Phase::Jump(_) | Phase::Stopover(_) | Phase::Rejump(_))
    }
}

/// A stopover at one moment, for the shaders: the event, how far into it
/// (on past its end while the drive re-engages), how much of it shows, its
/// script's shot, and the encounter frame's axes in the world.
#[derive(Clone, Copy, Debug)]
pub struct Scene {
    pub event: Event,
    pub local: f64,
    pub duration: f64,
    pub variant: f64,
    pub fade: f64,
    pub shot: Shot,
    pub basis: [[f64; 3]; 3],
}

/// What a transfer swings past on the way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Flyby {
    Star,
    Planet(usize),
}

#[derive(Clone, Copy, Debug)]
pub struct Segment {
    pub phase: Phase,
    pub start: f64,
    pub duration: f64,
}

/// The camera at one moment.
#[derive(Clone, Copy, Debug)]
pub struct View {
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
    pub half_fov: f64,
    /// 0 at rest, 1 at full hyperjump.
    pub warp: f64,
    /// The flare at the heading as the jump drops out, 0..1: building over
    /// the jump's last second, fading as the next visit begins.
    pub flare: f64,
    /// How far the drive's light has torn open over the new system, 0..1,
    /// in a visit's first seconds (`DROP_OUT`); 1 once it has, and always
    /// outside them.
    pub reveal: f64,
    /// The drive's own clock, seconds: what its light scrolls by. Slows
    /// over the jump's last seconds and on through the drop-out, so the
    /// two read as one.
    pub clock: f64,
    /// How far through the drive's preparation, 0..1 (1 once it is done).
    pub spool: f64,
    /// How much of the drive's start-up effect shows, 0..1: rising through
    /// the preparation, fading into its light as the jump begins.
    pub spool_light: f64,
    /// The way to the star the jump makes for, and how brightly it is
    /// marked, 0..1 (0: not shown).
    pub target: [f64; 3],
    pub target_shown: f64,
    /// The look of the drive's light (`voyage::LOOKS`, from 1) in a jump or
    /// spooling up, and of the light torn open as the ship drops out of one.
    pub look: u32,
    pub drop_look: u32,
    /// Out in the deep between stars: none of the system drawn.
    pub deep: bool,
    /// A stopover in view, and something seen inside a leg: its kind
    /// (`voyage::Flux`), how much shows, and how far into it.
    pub scene: Option<Scene>,
    pub flux: Option<(u32, f64, f64)>,
    /// A white flash over the frame, 0..1: thrown out of a gate or wormhole
    /// into a leg.
    pub flash: f64,
}

/// Each drive's preparation: its three stages between aligning and
/// engaging, named, and what each does, given the target star and the star
/// the ship is leaving; then what engaging it does.
fn drive_stages(drive: u32, target: &str, here: &str) -> ([(&'static str, String); 3], String) {
    match drive {
        1 => (
            [
                (
                    "PHOTON CHARGE",
                    format!("drinking {here}'s light into the lens ring"),
                ),
                (
                    "LENS FOCUS",
                    format!("the light drawn to a point on {target}"),
                ),
                ("CORONA", "a blue-white blaze round the bow".into()),
            ],
            format!("drifting out on the beam to {target}"),
        ),
        2 => (
            [
                (
                    "TACHYON SEED",
                    "faster-than-light particles cast ahead".into(),
                ),
                (
                    "RING LATTICE",
                    format!("tunnel rings forming toward {target}"),
                ),
                (
                    "TUNNEL SEAL",
                    "the rings spun up and locked in phase".into(),
                ),
            ],
            format!("into the tunnel to {target}"),
        ),
        3 => (
            [
                ("MASS LOCK", format!("clear of {here}'s gravity well")),
                (
                    "FOLD GEOMETRY",
                    format!("space creased between here and {target}"),
                ),
                ("CREASE", "the sky ahead drawn in to the fold".into()),
            ],
            format!("folding space onto {target}"),
        ),
        4 => (
            [
                ("ION INJECTION", "charging the hull's plasma sheath".into()),
                ("SHEATH", "plasma curtains streaming over the hull".into()),
                ("CURRENT", format!("the sheath aligned on {target}")),
            ],
            format!("into the slipstream to {target}"),
        ),
        _ => (
            [
                ("EXOTIC MATTER", "negative energy fed to the ring".into()),
                (
                    "INFLATION",
                    "a bubble of warped space round the ship".into(),
                ),
                ("SHELL", "the bubble's wall drawn tight and steady".into()),
            ],
            format!("space ahead contracting toward {target}"),
        ),
    }
}

/// What the drive's panel says it does as it engages, in a word.
fn engage_word(drive: u32) -> &'static str {
    match drive {
        1 => "DRIFT",
        2 => "TUNNEL",
        3 => "FOLD",
        4 => "SLIPSTREAM",
        _ => "WARP",
    }
}

/// The drive used for the jump to `system`: its number and name.
pub(crate) fn drive_to(system: &System) -> (u32, &'static str) {
    let k = (super::system::Rng::new(system.seed ^ 0xd21e).next() % DRIVES.len() as u64) as usize;
    (k as u32 + 1, DRIVES[k])
}

/// The drive's clock `local` seconds into a leg of `length` seconds:
/// running at full speed, then slowing to 0.4 of it over the last `SLOWING`
/// seconds.
fn drive_clock(local: f64, length: f64) -> f64 {
    let x = ((local - (length - SLOWING)) / SLOWING).clamp(0.0, 1.0);
    local.min(length - SLOWING) + SLOWING * (x - 0.3 * x * x)
}

pub struct Tour {
    pub system: System,
    /// The system the jump at the end goes to.
    pub next: System,
    pub itinerary: Vec<Stop>,
    /// Each stop's body, as a planet (see `System::world`).
    worlds: Vec<Planet>,
    pub segments: Vec<Segment>,
    pub total: f64,
    /// Per stop: the start angle of its orbit and its plane.
    orbits: Vec<Orbit>,
    /// Per stop: the path of the transfer that reaches it.
    paths: Vec<Path>,
    /// Where the visit drops out of the jump, and which way the jump leaves:
    /// the way to the next star.
    entry: [f64; 3],
    exit: [f64; 3],
    /// A relay: no stops, only the crossing from the drop-out to the jump
    /// point.
    pub transit: bool,
    /// The stars the jump at the end makes for, `next` first and the
    /// destination last (more than one when it hops through relays), and
    /// which of the trip's jumps it is, of how many (from 1).
    pub route: Vec<System>,
    pub leg: usize,
    pub legs: usize,
    /// The jump point at the system's edge, and the way the ship climbs out
    /// of the last stop's orbit toward it.
    jump_point: [f64; 3],
    outbound: [f64; 3],
    /// The crossing to the jump point: a spline through three knots.
    cruise: Cruise,
    /// The jump itself: its legs and what is met between them; the look of
    /// the light the ship arrived in; and the encounter frame's axes, +z
    /// the way the jump goes.
    pub passage: Passage,
    arrival: u32,
    basis: [[f64; 3]; 3],
}

/// The flight out to the jump point: a Hermite spline from where it begins
/// (the end of the climb out, or the drop-out) through a point clear of the
/// star to the jump point, eased from its start to a drift at the end.
#[derive(Clone, Copy, Debug)]
struct Cruise {
    knots: [[f64; 3]; 3],
    tangents: [[f64; 3]; 3],
    /// The ease's slopes at either end (`ease_between`).
    pace: [f64; 2],
}

/// The cubic from 0 to 1 over 0..1 leaving at slope `m0` and arriving at
/// `m1`: increasing throughout for slopes up to 3.
fn ease_between(u: f64, m0: f64, m1: f64) -> f64 {
    let u = u.clamp(0.0, 1.0);
    let (u2, u3) = (u * u, u * u * u);
    (u3 - 2.0 * u2 + u) * m0 + (-2.0 * u3 + 3.0 * u2) + (u3 - u2) * m1
}

impl Cruise {
    fn at(&self, u: f64) -> [f64; 3] {
        let sigma = 2.0 * ease_between(u, self.pace[0], self.pace[1]);
        let k = (sigma.floor() as usize).min(1);
        let s = sigma - k as f64;
        hermite(
            self.knots[k],
            self.tangents[k],
            self.knots[k + 1],
            self.tangents[k + 1],
            s,
        )
    }

    /// Distance left to fly from `u`, along the spline.
    fn remaining(&self, u: f64) -> f64 {
        let n = 32;
        let mut last = self.at(u);
        let mut d = 0.0;
        for i in 1..=n {
            let p = self.at(u + (1.0 - u) * i as f64 / n as f64);
            d += length(sub(p, last));
            last = p;
        }
        d
    }
}

#[derive(Clone, Copy, Debug)]
struct Orbit {
    /// The orbit plane: `u` lies in it (used when the sun is overhead), `n`
    /// is its normal.
    u: [f64; 3],
    n: [f64; 3],
    angle: f64,
}

/// A transfer's path: a Hermite spline through its knots (the orbit left,
/// the closest point of a flyby, the orbit reached). Each knot was placed
/// where its body was at one moment and moves with it, so the path meets
/// the orbits and passes the flyby's planet exactly however they move. The
/// camera slows down near what it flies past.
#[derive(Clone, Debug)]
struct Path {
    knots: Vec<[f64; 3]>,
    tangents: Vec<[f64; 3]>,
    /// Per knot: the stop or planet it moves with, and when it was placed.
    anchors: Vec<(Anchor, f64)>,
    /// Where each sample lies along the spline (`sigma`): crowding in on
    /// either end over the first and last stretches and evenly between, so
    /// the climb away and the approach are timed down to the orbits however
    /// long the path.
    places: Vec<f64>,
    /// The share of the transfer's (eased) time spent by each sample along
    /// the spline, and the distance covered, both cumulative.
    time: Vec<f64>,
    length: Vec<f64>,
    flyby: Option<Flyby>,
    /// The knot at a flyby's closest point.
    waypoint: Option<usize>,
    /// When, as a share of the transfer, it passes closest.
    closest: f64,
    /// What it slows past, as `time` was last given it.
    slow: Option<([f64; 3], f64, f64, f64)>,
    /// The bodies it leaves and arrives at, where each is then, and their
    /// radii: `time` slows the climb away and the approach in proportion
    /// to the nearness.
    leave: Option<([f64; 3], f64)>,
    arrive: Option<([f64; 3], f64)>,
    /// The spline's length as `time` weighs it: what one whole share of
    /// the transfer's time covers at its ends.
    weighted: f64,
    /// The pace at its two ends, as the slope of the eased time against
    /// plain time (see `ease`): the orbits' own speeds, so the camera never
    /// stops between orbit and transfer.
    pace: [f64; 2],
    /// The speeds at its ends (distance a second; negative: the path's
    /// average) and how long it lasts, which `time` sets `pace` from.
    speeds: [f64; 2],
    duration: f64,
    /// The directions at its ends: along the orbit left and the one
    /// reached. `time` lays the end tangents along them.
    heading: [Option<[f64; 3]>; 2],
    /// The orbital mechanics behind it, for the manoeuvre panel.
    burns: Burns,
}

/// The numbers of a transfer's manoeuvres, worked out from real masses and
/// orbits with the patched-conic approximation: km/s and degrees.
#[derive(Clone, Copy, Debug, Default)]
struct Burns {
    /// Leaving the first orbit onto the transfer: escape plus the excess
    /// speed the transfer needs.
    depart: f64,
    /// Being caught into the orbit reached.
    capture: f64,
    /// Degrees swept round the star.
    sweep: f64,
    /// Real distances from the star, AU, at the start and end.
    from_au: f64,
    to_au: f64,
    /// A gravity assist: speed relative to the planet, how far its gravity
    /// bends the path, and the speed gained (negative: shed).
    assist_vinf: f64,
    assist_turn: f64,
    assist_dv: f64,
    /// An Oberth manoeuvre: the burn, the speed at perihelion, and the burn
    /// far from the star that would give the same energy.
    oberth_burn: f64,
    oberth_speed: f64,
    oberth_equal: f64,
}

/// What the ship is doing at one moment, and why, for the manoeuvre panel.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Manoeuvre {
    pub name: String,
    pub text: String,
    /// How far through a burn or flyby, 0..1; negative while coasting.
    pub progress: f32,
    /// The burn's whole change of speed in km/s; 0 when the engine is off
    /// (coasting, or a flyby).
    pub dv: f32,
    /// A burn's stages (see `STAGES`), each named and what it does here;
    /// none while the engine is off.
    pub stages: Vec<(String, String)>,
    /// Where each stage begins, as a share of the whole: `STAGES` if empty.
    pub starts: Vec<f64>,
    /// The hyperdrive spooling up, its number (0: not a drive), and how
    /// charged it is, 0..1.
    pub drive: u32,
    pub charge: f32,
}

/// Where each of a burn's stages begins, as a share of it: turning to the
/// burn's attitude, lighting the engine, the burn itself, and the cut-off.
const STAGES: [f64; 4] = [0.0, 0.08, 0.16, 0.9];

impl Manoeuvre {
    /// A burn of `dv` km/s, `progress` of the way through, in its stages:
    /// the attitude it is flown in, what the burn does and what it leaves.
    fn burn(
        name: &str,
        progress: f64,
        dv: f64,
        attitude: &str,
        burn: String,
        cutoff: String,
    ) -> Option<Self> {
        Some(Manoeuvre {
            name: name.into(),
            text: String::new(),
            progress: progress as f32,
            dv: dv as f32,
            stages: vec![
                ("ATTITUDE".into(), attitude.into()),
                ("IGNITION".into(), "engine lit, throttle up".into()),
                ("BURN".into(), burn),
                ("CUTOFF".into(), cutoff),
            ],
            ..Default::default()
        })
    }

    /// Drive `drive` (from 1) spooling up, `progress` of the way through
    /// its preparation, for the jump to `target` from `here`.
    pub fn spool(
        name: &str,
        drive: u32,
        progress: f64,
        text: String,
        target: &str,
        here: &str,
    ) -> Option<Self> {
        let (own, engage) = drive_stages(drive, target, here);
        let mut stages = vec![("ALIGN".to_string(), format!("bow on {target}, locked"))];
        stages.extend(own.into_iter().map(|(name, what)| (name.to_string(), what)));
        stages.push((engage_word(drive).to_string(), engage));
        Some(Manoeuvre {
            name: name.to_uppercase(),
            text,
            progress: progress as f32,
            stages,
            starts: SPOOL_STAGES.to_vec(),
            drive,
            charge: smoothstep(SPOOL_STAGES[1], SPOOL_STAGES[4], progress) as f32,
            ..Default::default()
        })
    }

    /// What the ship does with the engine off, and why.
    fn coast(name: &str, text: String, progress: f64) -> Option<Self> {
        Some(Manoeuvre {
            name: name.into(),
            text,
            progress: progress as f32,
            ..Default::default()
        })
    }

    /// How hard the engine is firing, 0..1: off while the ship turns to the
    /// burn's attitude, run up as it lights, cut at the end.
    pub fn throttle(&self) -> f32 {
        if self.dv <= 0.0 || self.progress < 0.0 {
            return 0.0;
        }
        let p = self.progress as f64;
        (smoothstep(STAGES[1], STAGES[2], p) * (1.0 - smoothstep(STAGES[3], STAGES[3] + 0.06, p)))
            as f32
    }

    /// The stage under way, and how far through it.
    pub fn stage(&self) -> (usize, f32) {
        let starts: &[f64] = if self.starts.is_empty() {
            &STAGES
        } else {
            &self.starts
        };
        let p = (self.progress as f64).clamp(0.0, 1.0);
        let i = starts.iter().rposition(|&s| p >= s).unwrap_or(0);
        let end = starts.get(i + 1).copied().unwrap_or(1.0);
        (
            i,
            ((p - starts[i]) / (end - starts[i])).clamp(0.0, 1.0) as f32,
        )
    }

    /// The share of the burn's change of speed made so far: from halfway up
    /// the throttle's run-up to halfway down its cut.
    pub fn done(&self) -> f32 {
        let (on, off) = (0.5 * (STAGES[1] + STAGES[2]), STAGES[3] + 0.03);
        (((self.progress as f64) - on) / (off - on)).clamp(0.0, 1.0) as f32
    }
}

#[derive(Clone, Copy, Debug)]
enum Anchor {
    Fixed,
    Stop(Stop),
}

// Small vector helpers, in f64.
pub(crate) fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
pub(crate) fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
pub(crate) fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
pub fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
pub fn normalize(a: [f64; 3]) -> [f64; 3] {
    scale(a, 1.0 / length(a).max(1e-12))
}
fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    add(scale(a, 1.0 - t), scale(b, t))
}
/// Direction `v` as a heading round `up` and an elevation above the plane
/// square to it.
fn heading(v: [f64; 3], up: [f64; 3]) -> (f64, f64) {
    let (e1, e2) = level_axes(up);
    (
        dot(v, e2).atan2(dot(v, e1)),
        dot(v, up).clamp(-1.0, 1.0).asin(),
    )
}

fn level_axes(up: [f64; 3]) -> ([f64; 3], [f64; 3]) {
    let mut e1 = cross(up, [0.0, 0.0, 1.0]);
    if length(e1) < 1e-3 {
        e1 = cross(up, [1.0, 0.0, 0.0]);
    }
    let e1 = normalize(e1);
    (e1, cross(up, e1))
}

/// The change of heading from `a` to `b`, a whole turn either way.
fn heading_change(a: [f64; 3], b: [f64; 3], up: [f64; 3]) -> f64 {
    let d = heading(b, up).0 - heading(a, up).0;
    d - TAU * (d / TAU).round()
}

/// From direction `a` to direction `b`, a share `t` of the way, turning
/// round `up` and tilting. A straight mix of two directions far apart passes
/// near nothing and whips round there; and between two nearly opposite ones
/// either way round is as short, so which way is taken could flip from one
/// frame to the next. It goes the way round nearest `way`, a change of
/// heading fixed for the whole turn.
fn swing(a: [f64; 3], b: [f64; 3], t: f64, up: [f64; 3], way: f64) -> [f64; 3] {
    let (ya, pa) = heading(a, up);
    let (_, pb) = heading(b, up);
    let mut dy = heading_change(a, b, up);
    dy += TAU * ((way - dy) / TAU).round();
    let (yaw, pitch) = (ya + dy * t, pa + (pb - pa) * t);
    let (e1, e2) = level_axes(up);
    let flat = add(scale(e1, yaw.cos()), scale(e2, yaw.sin()));
    normalize(add(scale(flat, pitch.cos()), scale(up, pitch.sin())))
}
fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
fn smootherstep(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

/// A whole number with thousands separated: 182,930.
pub fn thousands(value: f64) -> String {
    let digits = format!("{:.0}", value.round());
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Format seconds as m:ss.
pub fn clock(seconds: f64) -> String {
    let s = seconds.max(0.0).round() as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

fn hermite(p0: [f64; 3], m0: [f64; 3], p1: [f64; 3], m1: [f64; 3], s: f64) -> [f64; 3] {
    let (h00, h10, h01, h11) = (
        2.0 * s * s * s - 3.0 * s * s + 1.0,
        s * s * s - 2.0 * s * s + s,
        -2.0 * s * s * s + 3.0 * s * s,
        s * s * s - s * s,
    );
    add(
        add(scale(p0, h00), scale(m0, h10)),
        add(scale(p1, h01), scale(m1, h11)),
    )
}

/// Inverse of `smootherstep` on [0, 1], by bisection.
/// The inverse of an increasing function on 0..1.
fn invert(f: impl Fn(f64) -> f64, y: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if f(mid) < y {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    0.5 * (lo + hi)
}

impl Path {
    /// The spline's point at `sigma` (0 at the first knot, 1 at the next…),
    /// as placed, before the knots move with their bodies.
    fn spline(&self, sigma: f64) -> [f64; 3] {
        let last = self.knots.len() - 1;
        let k = (sigma.floor() as usize).min(last - 1);
        let s = (sigma - k as f64).clamp(0.0, 1.0);
        // Tangents no longer than half again the segment: where knots crowd
        // in on a flyby, the ones sized for the longer stretches either side
        // would overshoot and loop the path back on itself.
        let span = length(sub(self.knots[k + 1], self.knots[k]));
        let cap = |m: [f64; 3]| {
            let l = length(m);
            if l > 1.5 * span {
                scale(m, 1.5 * span / l)
            } else {
                m
            }
        };
        hermite(
            self.knots[k],
            cap(self.tangents[k]),
            self.knots[k + 1],
            cap(self.tangents[k + 1]),
            s,
        )
    }

    /// Work out the timing: how much of the transfer each stretch takes.
    /// Near what it flies past (`slow`: a point, how near the path comes,
    /// how wide the slow stretch is in those, and the share of the time it
    /// gets), the camera lingers: the flyby gets that share of the crossing
    /// however long the crossing is, so a slingshot past a small planet is
    /// seen for seconds, not a blink.
    fn time(&mut self, slow: Option<([f64; 3], f64, f64, f64)>) {
        self.slow = slow;
        let last = self.tangents.len() - 1;
        for (i, heading) in [(0, self.heading[0]), (last, self.heading[1])] {
            if let Some(heading) = heading {
                self.tangents[i] = scale(heading, length(self.tangents[i]));
            }
        }
        let span = (self.knots.len() - 1) as f64;
        // Closer and closer to either end over the first and last stretch,
        // evenly between: on a path tens of thousands of radii long, an even
        // sample is longer than the whole approach.
        let tail = span.min(3.0) / 3.0;
        let (head, end) = (SAMPLES / 8, SAMPLES - SAMPLES / 4);
        let crowd = |k: usize, n: usize| 1e-5f64.powf(k as f64 / n as f64);
        self.places = (0..=SAMPLES)
            .map(|j| match j {
                0 => 0.0,
                SAMPLES => span,
                j if j <= head => tail * crowd(head - j, head),
                j if j <= end => {
                    tail + (span - 2.0 * tail) * (j - head) as f64 / (end - head) as f64
                }
                j => span - tail * crowd(j - end, SAMPLES - end),
            })
            .collect();
        let mut ds = Vec::with_capacity(SAMPLES);
        let mut near = Vec::with_capacity(SAMPLES);
        let mut close_in = Vec::with_capacity(SAMPLES);
        let mut close_out = Vec::with_capacity(SAMPLES);
        let mut last = self.spline(0.0);
        for j in 1..=SAMPLES {
            let p = self.spline(self.places[j]);
            let middle = mix(p, last, 0.5);
            ds.push(length_of(sub(p, last)));
            near.push(match slow {
                Some((centre, reach, width, _)) => {
                    let d = length_of(sub(middle, centre)) / (reach * width);
                    (-d * d).exp()
                }
                None => 0.0,
            });
            // Time in proportion to the nearness: the body grows at a
            // steady rate as it comes in, not doubling every second at
            // the end.
            let nearness = |body: Option<([f64; 3], f64)>| match body {
                Some((centre, radius)) => radius / length_of(sub(middle, centre)).max(radius),
                None => 0.0,
            };
            close_in.push(nearness(self.arrive));
            close_out.push(nearness(self.leave));
            last = p;
        }
        // Weight 1 + boost near the flyby, the boost chosen so the near
        // stretch takes its share of the time.
        let base: f64 = ds.iter().sum();
        let close: f64 = ds.iter().zip(&near).map(|(d, g)| d * g).sum();
        let boost = match slow {
            Some((_, _, _, share)) if close > 1e-9 => {
                (share / (1.0 - share) * (base - close) / close - 1.0).max(0.0)
            }
            _ => 0.0,
        };
        // The climb away and the approach get their shares of the time on
        // top.
        let flown: f64 = ds
            .iter()
            .zip(&near)
            .map(|(d, g)| d * (1.0 + boost * g))
            .sum();
        let share = |close: &[f64], share: f64| {
            let nearing: f64 = ds.iter().zip(close).map(|(d, g)| d * g).sum();
            if nearing > 1e-9 {
                share / (1.0 - APPROACH - LEAVE) * flown / nearing
            } else {
                0.0
            }
        };
        let (approach, leaving) = (share(&close_in, APPROACH), share(&close_out, LEAVE));
        let weight =
            |j: usize| 1.0 + boost * near[j] + approach * close_in[j] + leaving * close_out[j];
        let mut time = vec![0.0];
        let mut length = vec![0.0];
        for j in 0..SAMPLES {
            time.push(time[j] + ds[j] * weight(j));
            length.push(length[j] + ds[j]);
        }
        let total = time[SAMPLES].max(1e-9);
        self.weighted = total;
        if self.duration > 0.0 {
            let average = total / self.duration;
            // The slope of the eased time that moves the camera at `v`
            // where the spline is weighted `w`.
            let slope = |v: f64, w: f64| {
                let v = if v < 0.0 { average } else { v };
                (v * w * self.duration / total).clamp(0.0, 2.5)
            };
            self.pace = [
                slope(self.speeds[0], weight(0)),
                slope(self.speeds[1], weight(SAMPLES - 1)),
            ];
        }
        for t in &mut time {
            *t /= total;
        }
        self.time = time;
        self.length = length;
        self.closest = match self.waypoint {
            Some(w) => {
                let j = self.places.partition_point(|&s| s < w as f64);
                invert(|u| self.ease(u), self.time[j.min(SAMPLES)])
            }
            None => 0.5,
        };
    }

    /// The share of the eased time at a share `u` of the transfer: a cubic
    /// leaving and reaching at the paces of the orbits at either end, and
    /// increasing throughout (each slope at most 3 keeps it so).
    fn ease(&self, u: f64) -> f64 {
        let [m0, m1] = self.pace;
        let u = u.clamp(0.0, 1.0);
        let (u2, u3) = (u * u, u * u * u);
        (u3 - 2.0 * u2 + u) * m0 + (-2.0 * u3 + 3.0 * u2) + (u3 - u2) * m1
    }

    /// Where along the spline (`sigma`) the camera is, a share `u` of the
    /// way through the transfer.
    fn sigma(&self, u: f64) -> f64 {
        let tau = self.ease(u);
        let j = self.time.partition_point(|&t| t < tau).clamp(1, SAMPLES);
        let (a, b) = (self.time[j - 1], self.time[j]);
        let f = if b > a { (tau - a) / (b - a) } else { 0.0 };
        self.places[j - 1] + (self.places[j] - self.places[j - 1]) * f
    }

    /// Distance along the path still to go from `sigma`, in units.
    fn remaining(&self, sigma: f64) -> f64 {
        let j = self
            .places
            .partition_point(|&s| s <= sigma)
            .saturating_sub(1)
            .min(SAMPLES);
        self.length[SAMPLES] - self.length[j]
    }
}

fn length_of(a: [f64; 3]) -> f64 {
    length(a)
}

/// Distance from the star in the plane of the system.
fn flat_radius(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[2] * a[2]).sqrt()
}

/// The angle from `a` to `b` round the star the way the planets go (radians,
/// 0..2π).
fn prograde(a: [f64; 3], b: [f64; 3]) -> f64 {
    (b[2].atan2(b[0]) - a[2].atan2(a[0])).rem_euclid(TAU)
}

/// A point on a transfer arc round the star from `a` to `b`, a share `s` of
/// the way: the angle sweeps prograde evenly while the distance from the star
/// eases from one orbit to the other, level at both ends as on a Hohmann
/// ellipse, where the ship leaves and meets each orbit tangentially. An arc
/// that would go nearly all the way round goes the short way back instead.
fn arc(a: [f64; 3], b: [f64; 3], s: f64) -> [f64; 3] {
    let (ra, rb) = (flat_radius(a), flat_radius(b));
    let mut sweep = prograde(a, b);
    if sweep > 1.6 * PI {
        sweep -= TAU;
    }
    let ease = 0.5 - 0.5 * (PI * s).cos();
    let r = ra + (rb - ra) * ease;
    let angle = a[2].atan2(a[0]) + sweep * s;
    [
        r * angle.cos(),
        a[1] + (b[1] - a[1]) * ease,
        r * angle.sin(),
    ]
}

/// `n + 1` knots along the arc from `a` to `b`, crowded toward its end or
/// start for a tight bend there.
fn arc_knots(
    a: [f64; 3],
    b: [f64; 3],
    n: usize,
    dense_end: bool,
    dense_start: bool,
) -> Vec<[f64; 3]> {
    (0..=n)
        .map(|k| {
            let mut s = k as f64 / n as f64;
            if dense_end {
                s = 1.0 - (1.0 - s) * (1.0 - s);
            }
            if dense_start {
                s *= s;
            }
            match k {
                0 => a,
                _ if k == n => b,
                _ => arc(a, b, s),
            }
        })
        .collect()
}

/// Tangents through every knot, Catmull-Rom style.
fn catmull_rom(knots: &[[f64; 3]]) -> Vec<[f64; 3]> {
    let n = knots.len();
    (0..n)
        .map(|k| match k {
            0 => sub(knots[1], knots[0]),
            _ if k == n - 1 => sub(knots[n - 1], knots[n - 2]),
            _ => scale(sub(knots[k + 1], knots[k - 1]), 0.5),
        })
        .collect()
}

/// Anchors for `n` knots: the ends move with their stops, the rest stay put
/// round the star.
fn anchors(n: usize, first: (Anchor, f64), last: (Anchor, f64)) -> Vec<(Anchor, f64)> {
    let mut anchors = vec![(Anchor::Fixed, 0.0); n];
    anchors[0] = first;
    anchors[n - 1] = last;
    anchors
}

/// The jump's segments after the spool: each leg, and each stopover between
/// two with the drive re-engaging after it (none after a gate or wormhole,
/// which throw the ship on themselves).
fn push_passage(passage: &Passage, segments: &mut Vec<Segment>, t: &mut f64) {
    for (k, leg) in passage.legs.iter().enumerate() {
        let k8 = k as u8;
        segments.push(Segment {
            phase: Phase::Jump(k8),
            start: *t,
            duration: leg.duration,
        });
        *t += leg.duration;
        if let Some(stop) = passage.stops.get(k) {
            let duration = stop.event.duration();
            segments.push(Segment {
                phase: Phase::Stopover(k8),
                start: *t,
                duration,
            });
            *t += duration;
            let rejump = stop.event.rejump();
            if rejump > 0.0 {
                segments.push(Segment {
                    phase: Phase::Rejump(k8),
                    start: *t,
                    duration: rejump,
                });
                *t += rejump;
            }
        }
    }
}

/// The encounter frame's axes in the world: x to the right, y up and z the
/// way the jump goes (`exit`), so a camera looking along z with y up has x
/// on its right (`right = forward × up`).
fn frame_of(exit: [f64; 3]) -> [[f64; 3]; 3] {
    let z = normalize(exit);
    let mut x = cross(z, [0.0, 1.0, 0.0]);
    if length(x) < 1e-3 {
        x = cross(z, [1.0, 0.0, 0.0]);
    }
    let x = normalize(x);
    let y = cross(x, z);
    [x, y, z]
}

impl Tour {
    /// A whole visit to `system`, the jump at its end straight to `next`.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn new(system: System, next: System) -> Self {
        let passage = Passage::plan(system.seed, 0, drive_to(&next).0);
        let arrival = drive_to(&system).0;
        Self::plan(system, vec![next], false, 1, 1, passage, arrival)
    }

    /// A visit to `system`, the jump at its end making for `route[0]` on
    /// the way to the last of `route` (the trip's jump `leg` of `legs`).
    /// A relay (`transit`) has no stops: only the crossing to its jump
    /// point. The jump at the end is `passage`'s legs and stopovers, and the
    /// light the ship arrives in is look `arrival` (`voyage::LOOKS`).
    pub fn plan(
        system: System,
        route: Vec<System>,
        transit: bool,
        leg: usize,
        legs: usize,
        passage: Passage,
        arrival: u32,
    ) -> Self {
        let next = route.first().cloned().expect("a jump makes for a star");
        let itinerary = if transit {
            Vec::new()
        } else {
            system.itinerary()
        };
        let mut rng = super::system::Rng::new(system.seed ^ 0x0b17);
        // The jump point's own draws, so the rest of the tour is the same
        // whatever they are.
        let mut far = super::system::Rng::new(system.seed ^ 0x7e1d);
        let edge = system.planets.last().map_or(5.0 * AU, |p| p.orbit) * EDGE + EDGE_MARGIN;
        // Round the system's axis by `angle`, raised `rise`, at `radius`.
        let at_edge = |angle: f64, rise: f64, radius: f64| {
            scale(normalize([angle.cos(), rise, angle.sin()]), radius)
        };

        // Drop out above the plane of the system, beyond the first stop,
        // looking in toward the star; a relay at its edge, across from the
        // jump point.
        let exit_draw = |rng: &mut super::system::Rng| {
            normalize([
                rng.range(-1.0, 1.0),
                rng.range(0.2, 0.6),
                rng.range(-1.0, 1.0),
            ])
        };
        if transit {
            let exit = exit_draw(&mut rng);
            let toward = exit[2].atan2(exit[0]) + far.range(-0.6, 0.6);
            let jump_point = at_edge(toward, far.range(0.08, 0.3), edge);
            let back = toward + PI + far.range(-0.9, 0.9);
            let entry = at_edge(back, far.range(-0.25, 0.3), edge);
            let mut segments = Vec::new();
            let mut t = 0.0;
            for (phase, duration) in [
                (Phase::Cruise { from: None }, CROSSING),
                (Phase::Spool, SPOOL),
            ] {
                segments.push(Segment {
                    phase,
                    start: t,
                    duration,
                });
                t += duration;
            }
            push_passage(&passage, &mut segments, &mut t);
            let mut tour = Tour {
                system,
                next,
                itinerary,
                worlds: Vec::new(),
                segments,
                total: t,
                orbits: Vec::new(),
                paths: Vec::new(),
                entry,
                exit,
                transit,
                route,
                leg,
                legs,
                jump_point,
                outbound: exit,
                cruise: Cruise {
                    knots: [[0.0; 3]; 3],
                    tangents: [[0.0; 3]; 3],
                    pace: [1.0, 1.0],
                },
                basis: frame_of(exit),
                passage,
                arrival,
            };
            tour.cruise = tour.build_cruise();
            return tour;
        }
        let first = &system.planets[itinerary[0].planet];
        let out = normalize(add(normalize(first.position(0.0)), [0.0, 0.35, 0.0]));
        let entry = scale(out, first.orbit * 1.6 + 400.0);
        let exit = exit_draw(&mut rng);

        let flybys = Self::choose_flybys(&system, &itinerary, entry, &mut rng);
        let mut segments = Vec::new();
        let mut t = 0.0;
        let mut push = |phase, duration: f64, t: &mut f64| {
            segments.push(Segment {
                phase,
                start: *t,
                duration,
            });
            *t += duration;
        };
        let mut from = None;
        for (k, &stop) in itinerary.iter().enumerate() {
            let duration = if stop.moon.is_some() {
                HOP
            } else if flybys[k].is_some() {
                FLYBY
            } else {
                TRANSFER
            };
            push(Phase::Transfer { from, to: stop }, duration, &mut t);
            push(Phase::High(stop), HIGH, &mut t);
            from = Some(stop);
        }
        let last = *itinerary.last().expect("a tour has stops");
        push(Phase::Depart(last), DEPART, &mut t);
        push(Phase::Cruise { from: Some(last) }, CRUISE, &mut t);
        push(Phase::Spool, SPOOL, &mut t);
        push_passage(&passage, &mut segments, &mut t);

        let orbits = itinerary
            .iter()
            .map(|stop| {
                let planet = &system.planets[stop.planet];
                // Inclined to the planet's equator, so rings and moons are
                // seen from a little above or below.
                let axis = planet.axis();
                let side = normalize(cross(axis, [0.3, 0.1, 1.0]));
                let tilt = rng.range(0.25, 0.55) * if rng.chance(0.5) { 1.0 } else { -1.0 };
                let n = normalize(add(scale(axis, tilt.cos()), scale(side, tilt.sin())));
                let u = normalize(cross(n, [0.0, 0.0, 1.0]));
                let _ = rng.range(0.0, std::f64::consts::TAU);
                // Angles are measured from the sun's direction (see
                // `orbiting`): arrive over the night side if the transfer
                // allows (see `new`), and cross sunrise in orbit.
                Orbit { u, n, angle: -2.2 }
            })
            .collect();
        // The jump point: at the system's edge, past the last stop and out
        // toward the next star, so the ship leaves the way it is going and
        // never back across the system.
        let depart = segments
            .iter()
            .find(|s| matches!(s.phase, Phase::Depart(_)))
            .expect("a tour departs its last stop")
            .start;
        let leaving = system.centre(last, depart);
        let flat = |v: [f64; 3]| normalize([v[0], 0.0, v[2]]);
        let mut side = add(flat(leaving), flat(exit));
        if length(side) < 0.2 {
            side = flat(exit);
        }
        let toward = side[2].atan2(side[0]) + far.range(-0.4, 0.4);
        let radius = edge.max(length(leaving) * 1.5 + EDGE_MARGIN);
        let jump_point = at_edge(toward, far.range(0.1, 0.3), radius);
        // The climb out of the last orbit heads for it; from a moon, away
        // from its giant.
        let mut outbound = normalize(sub(jump_point, leaving));
        if last.moon.is_some() {
            let giant = normalize(sub(system.planets[last.planet].position(depart), leaving));
            let toward = dot(outbound, giant) + 0.5;
            if toward > 0.0 {
                outbound = normalize(sub(outbound, scale(giant, 2.0 * toward)));
            }
        }
        let worlds = itinerary.iter().map(|&s| system.world(s)).collect();
        let mut tour = Tour {
            system,
            next,
            itinerary,
            worlds,
            segments,
            total: t,
            orbits,
            paths: Vec::new(),
            entry,
            exit,
            transit,
            route,
            leg,
            legs,
            jump_point,
            outbound,
            cruise: Cruise {
                knots: [[0.0; 3]; 3],
                tangents: [[0.0; 3]; 3],
                pace: [1.0, 1.0],
            },
            basis: frame_of(exit),
            passage,
            arrival,
        };
        tour.stage_moons();
        // Each orbit is entered the way the transfer reaching it comes in,
        // keeping its momentum: where the orbit runs along the transfer's
        // last stretch, and the way round (the same tilted plane, turned
        // over) whose meeting point is nearer the preferred one over the
        // night side. Drawn again a few times, as where it arrives bends
        // the transfer's end.
        for k in 0..tour.itinerary.len() {
            let path = tour.build_path(k, flybys[k], &mut rng);
            tour.paths.push(path);
            let preferred = tour.orbits[k];
            for _ in 0..3 {
                let heading = tour.heading_in(k);
                let mut best = (f64::INFINITY, preferred);
                for sense in [1.0, -1.0] {
                    let mut orbit = preferred;
                    orbit.n = scale(preferred.n, sense);
                    let (u, v) = tour.plane_axes(tour.itinerary[k], orbit, tour.arrival_time(k));
                    // The prograde direction at angle a is -u sin a + v cos a.
                    orbit.angle = (-dot(heading, u)).atan2(dot(heading, v));
                    let off = (orbit.angle - preferred.angle + PI).rem_euclid(TAU) - PI;
                    if off.abs() < best.0 {
                        best = (off.abs(), orbit);
                    }
                }
                tour.orbits[k] = best.1;
                tour.paths[k] = tour.build_path(k, flybys[k], &mut rng);
            }
        }
        tour.cruise = tour.build_cruise();
        tour
    }

    /// When the first segment `matches` picks begins, seconds into the visit.
    pub fn start_of(&self, matches: impl Fn(Phase) -> bool) -> Option<f64> {
        self.segments
            .iter()
            .find(|s| matches(s.phase))
            .map(|s| s.start)
    }

    /// The look of the light the visit arrived in (`voyage::LOOKS`).
    pub fn arrival_light(&self) -> u32 {
        self.arrival
    }

    /// The segment of `phase`'s kind (the cruise, the spool or the jump).
    fn segment_of(&self, matches: impl Fn(Phase) -> bool) -> Segment {
        *self
            .segments
            .iter()
            .find(|s| matches(s.phase))
            .expect("every tour has its cruise, spool and jump")
    }

    fn cruise_segment(&self) -> Segment {
        self.segment_of(|p| matches!(p, Phase::Cruise { .. }))
    }

    /// The flight to the jump point: from the end of the climb out of the
    /// last stop, heading the way it climbs, or from where a relay's
    /// drop-out leaves the ship, on through a point clear of the star (a
    /// third of the way out to the edge at least) to the jump point,
    /// drifting by the end.
    fn build_cruise(&self) -> Cruise {
        let segment = self.cruise_segment();
        let start = segment.start;
        let (p0, heading, pace) = match self.itinerary.last() {
            Some(&last) => {
                let p0 = self.departing(last, start - DEPART, start);
                let before = self.departing(last, start - DEPART, start - 0.5);
                (p0, normalize(sub(p0, before)), 0.35)
            }
            None => (self.entry, normalize(sub(self.jump_point, self.entry)), 1.0),
        };
        let p2 = self.jump_point;
        let mut mid = mix(p0, p2, 0.5);
        let clear = 0.35 * length(p2);
        if length(mid) < clear {
            // Round the star, not past it: out to the side the chord lies.
            let mut away = mid;
            if length(away) < 1e-6 {
                away = cross(normalize(sub(p2, p0)), [0.0, 1.0, 0.0]);
            }
            mid = scale(normalize(away), clear);
        }
        let first = length(sub(mid, p0));
        let second = length(sub(p2, mid));
        Cruise {
            knots: [p0, mid, p2],
            tangents: [
                scale(heading, first),
                scale(normalize(sub(p2, p0)), 0.5 * (first + second)),
                scale(normalize(sub(p2, mid)), second),
            ],
            pace: [pace, 0.25],
        }
    }

    /// Where the camera is on the flight to the jump point at `t`: off the
    /// last stop, the climb's own motion carried on and let go over the
    /// first stretch; kept clear of whatever it passes.
    fn cruising(&self, t: f64) -> [f64; 3] {
        let segment = self.cruise_segment();
        let u = ((t - segment.start) / segment.duration).clamp(0.0, 1.0);
        let mut p = self.cruise.at(u);
        if let Phase::Cruise { from: Some(last) } = segment.phase {
            let carry = self.departing(last, segment.start - DEPART, t);
            p = mix(carry, p, smoothstep(0.0, 0.3, u));
        }
        let fade = smoothstep(0.0, 0.12, u) * (1.0 - smoothstep(0.9, 1.0, u));
        if fade > 0.0 {
            let push = |p: [f64; 3], centre: [f64; 3], floor: f64| {
                let rel = sub(p, centre);
                let d = length(rel);
                let k = floor * 0.2;
                let soft = 0.5 * (d + floor + ((d - floor) * (d - floor) + k * k).sqrt());
                scale(normalize(rel), soft - d)
            };
            let mut moved = [0.0; 3];
            for (centre, floor, _) in self.obstacles(t) {
                moved = add(moved, push(p, centre, floor * 1.5));
            }
            p = add(p, scale(moved, fade));
        }
        p
    }

    /// Which way the camera looks on the flight out at `t`: along the way
    /// it goes; leaving a stop, turning to it from the climb's heading; on
    /// a relay, at first toward its star too.
    fn cruise_forward(&self, t: f64) -> [f64; 3] {
        let segment = self.cruise_segment();
        let u = ((t - segment.start) / segment.duration).clamp(0.0, 1.0);
        let motion = |t: f64| normalize(sub(self.cruising(t + 0.25), self.cruising(t - 0.25)));
        let ahead = match segment.phase {
            Phase::Cruise { from: Some(_) } => {
                let settled = segment.start + 0.25 * segment.duration;
                let way = heading_change(self.outbound, motion(settled), [0.0, 1.0, 0.0]);
                swing(
                    self.outbound,
                    motion(t),
                    smoothstep(0.0, 0.25, u),
                    [0.0, 1.0, 0.0],
                    way,
                )
            }
            _ => {
                let star = normalize(scale(self.cruising(t), -1.0));
                normalize(add(
                    motion(t),
                    scale(star, 0.8 * (1.0 - smoothstep(0.5, 0.9, u))),
                ))
            }
        };
        // Over the second half, toward the next star too, its mark in view.
        normalize(add(ahead, scale(self.exit, 0.7 * smoothstep(0.35, 0.9, u))))
    }

    /// The camera at the jump point `local` seconds into the spool: drifting
    /// on as it arrived, slowing, then surging toward the next star as the
    /// drive engages, at the jump's own speed by its end.
    fn spooling(&self, local: f64) -> [f64; 3] {
        let end = self.cruise_segment();
        let t1 = end.start + end.duration;
        let arrived = scale(sub(self.cruising(t1), self.cruising(t1 - 0.2)), 1.0 / 0.2);
        let settle = 6.0;
        let drift = scale(arrived, settle * (1.0 - (-local / settle).exp()));
        let engage = SPOOL_STAGES[4] * SPOOL;
        let span = SPOOL - engage;
        let x = ((local - engage) / span).clamp(0.0, 1.0);
        let surge = scale(self.exit, JUMP_SPEED * span * x * x * 0.5);
        add(add(self.cruising(t1), drift), surge)
    }

    /// Which way the camera looks at the jump point: turning from the way
    /// it arrived onto the next star over the alignment, and holding it.
    fn spool_forward(&self, u: f64) -> [f64; 3] {
        let end = self.cruise_segment();
        let arrived = self.cruise_forward(end.start + end.duration);
        let up = [0.0, 1.0, 0.0];
        let way = heading_change(arrived, self.exit, up);
        swing(
            arrived,
            self.exit,
            smoothstep(0.0, SPOOL_STAGES[1], u),
            up,
            way,
        )
    }

    /// The first stop of the visit the jump arrives in: none if that is a
    /// relay.
    pub fn next_first(&self) -> Option<Stop> {
        (self.route.len() == 1).then(|| self.next.itinerary()[0])
    }

    /// The star the trip is making for, past any relays.
    pub fn destination(&self) -> &System {
        self.route.last().expect("a jump makes for a star")
    }

    /// The jump point, for the minimap.
    pub fn jump_point(&self) -> [f64; 3] {
        self.jump_point
    }

    /// Whether the camera, reaching stop `k`'s orbit at the end of its
    /// transfer, moves against the orbit: its motion relative to the body
    /// over the transfer's last seconds against the orbit's first seconds.
    #[cfg(test)]
    fn arrives_backwards(&self, k: usize) -> bool {
        let stop = self.itinerary[k];
        let segment = self.transfer_to(stop);
        let t1 = segment.start + segment.duration;
        let dt = 2.0;
        let rel = |p: [f64; 3], t: f64| sub(p, self.system.centre(stop, t));
        let before = sub(
            rel(self.on_path(k, segment, t1).0, t1),
            rel(self.on_path(k, segment, t1 - dt).0, t1 - dt),
        );
        let after = sub(
            rel(self.orbiting(stop, t1 + dt).0, t1 + dt),
            rel(self.orbiting(stop, t1).0, t1),
        );
        dot(before, after) < 0.0
    }

    /// The rough times of each transfer, `(leaves, arrives)`, for flyby
    /// choices `flybys` (none yet: every transfer the plain length).
    fn rough_times(itinerary: &[Stop], flybys: &[Option<Flyby>]) -> Vec<(f64, f64)> {
        let mut t = 0.0;
        itinerary
            .iter()
            .enumerate()
            .map(|(k, stop)| {
                let duration = if stop.moon.is_some() {
                    HOP
                } else if flybys.get(k).copied().flatten().is_some() {
                    FLYBY
                } else {
                    TRANSFER
                };
                let times = (t, t + duration);
                t += duration + HIGH;
                times
            })
            .collect()
    }

    /// How each transfer gets there, as a real mission would plan it:
    ///
    /// - a **gravity assist** when another planet (a giant, ideally)
    ///   lies along the way round the star: its pull bends the path and
    ///   trades speed for free;
    /// - an **Oberth manoeuvre** when the target lies most of the way round
    ///   the star, or now and then for the spectacle: dive to perihelion and
    ///   burn there, where thrust is worth most;
    /// - otherwise a plain **transfer ellipse** round the star, a Hohmann
    ///   transfer when it sweeps about half way round.
    ///
    /// Every system has at least one Oberth dive past its star.
    fn choose_flybys(
        system: &System,
        itinerary: &[Stop],
        entry: [f64; 3],
        rng: &mut super::system::Rng,
    ) -> Vec<Option<Flyby>> {
        let rolls: Vec<f64> = itinerary.iter().map(|_| rng.unit()).collect();
        let mut flybys: Vec<Option<Flyby>> = vec![None; itinerary.len()];
        let mut sweeps = vec![-1.0; itinerary.len()];
        // Twice: the second time with the lengths the first choice gives.
        for _ in 0..2 {
            let times = Self::rough_times(itinerary, &flybys);
            for (k, stop) in itinerary.iter().enumerate() {
                if stop.moon.is_some() {
                    flybys[k] = None;
                    continue;
                }
                let (t0, t1) = times[k];
                let a = if k == 0 {
                    entry
                } else {
                    system.centre(itinerary[k - 1], t0)
                };
                let b = system.centre(*stop, t1);
                let sweep = prograde(a, b);
                sweeps[k] = sweep;
                let (ra, rb) = (flat_radius(a), flat_radius(b));
                // Any planet along the way but the two ends will do.
                let ends = [
                    k.checked_sub(1).map(|j| itinerary[j].planet),
                    Some(stop.planet),
                ];
                let assist = (0..system.planets.len())
                    .filter(|i| !ends.contains(&Some(*i)))
                    .filter_map(|i| {
                        let planet = &system.planets[i];
                        let c = planet.position(0.5 * (t0 + t1));
                        let into = prograde(a, c);
                        let rc = flat_radius(c);
                        let between = into > 0.08 * sweep && into < 0.92 * sweep;
                        let near = rc > ra.min(rb) * 0.45 && rc < ra.max(rb) * 2.0;
                        (between && near).then_some((i, planet.mass))
                    })
                    .max_by(|x, y| x.1.total_cmp(&y.1));
                let roll = rolls[k];
                flybys[k] = match assist {
                    Some((i, _)) if sweep < 1.4 * PI && roll < 0.75 => Some(Flyby::Planet(i)),
                    _ if sweep > 1.3 * PI => Some(Flyby::Star),
                    _ if roll > 0.78 => Some(Flyby::Star),
                    _ => None,
                };
            }
        }
        if !flybys.contains(&Some(Flyby::Star))
            && let Some(k) = (0..sweeps.len())
                .filter(|&k| sweeps[k] >= 0.0 && flybys[k].is_none())
                .max_by(|&a, &b| sweeps[a].total_cmp(&sweeps[b]))
        {
            flybys[k] = Some(Flyby::Star);
        }
        flybys
    }

    /// The path of the transfer to stop `k`.
    fn build_path(&self, k: usize, flyby: Option<Flyby>, rng: &mut super::system::Rng) -> Path {
        let to = self.itinerary[k];
        let segment = self
            .segments
            .iter()
            .find(|s| matches!(s.phase, Phase::Transfer { to: t, .. } if t == to))
            .copied()
            .expect("every stop has a transfer");
        let from = if k == 0 {
            None
        } else {
            Some(self.itinerary[k - 1])
        };
        let (t0, t1) = (segment.start, segment.start + segment.duration);
        let p0 = match from {
            Some(f) => self.orbiting(f, t0).0,
            None => self.entry,
        };
        let p1 = self.orbiting(to, t1).0;
        let away = match from {
            Some(f) => normalize(sub(p0, self.system.centre(f, t0))),
            None => normalize(sub(p1, p0)),
        };
        let into = normalize(sub(self.system.centre(to, t1), p1));
        let start_anchor = (from.map_or(Anchor::Fixed, Anchor::Stop), t0);
        let end_anchor = (Anchor::Stop(to), t1);
        let mut path = Path {
            knots: vec![p0, p1],
            tangents: Vec::new(),
            anchors: vec![start_anchor, end_anchor],
            places: Vec::new(),
            time: Vec::new(),
            length: Vec::new(),
            flyby,
            waypoint: None,
            closest: 0.5,
            slow: None,
            leave: from.map(|f| (self.system.centre(f, t0), self.world(f).radius)),
            arrive: Some((self.system.centre(to, t1), self.world(to).radius)),
            weighted: 0.0,
            pace: [1.0, 1.0],
            // Leave and arrive moving along the orbits, the way they are
            // flown, at their speeds: the departure burn starts from the
            // orbit's own motion, and the capture burn leaves the ship on
            // the next, so the camera keeps going forward throughout.
            speeds: [
                from.map_or(-1.0, |f| self.orbit_speed(f, t0)),
                self.orbit_speed(to, t1),
            ],
            duration: t1 - t0,
            heading: [
                from.map(|f| normalize(add(self.orbit_motion(f, t0), scale(away, 0.3)))),
                Some(self.orbit_motion(to, t1)),
            ],
            burns: Burns::default(),
        };
        let lift = rng.range(0.1, 0.3) * if rng.chance(0.5) { 1.0 } else { -1.0 };
        let reach_star = (self.system.star.radius * rng_reach(self.system.seed)).max(8.0);

        if to.moon.is_some() {
            // A hop across a giant's moons: a short arc round the giant.
            let span = length(sub(p1, p0));
            let chord = normalize(sub(p1, p0));
            path.tangents = vec![
                add(scale(chord, span * 0.5), scale(away, span * 0.4)),
                add(scale(chord, span * 0.5), scale(into, span * 0.25)),
            ];
            path.time(None);
        } else {
            match flyby {
                None => {
                    // A transfer ellipse round the star, prograde.
                    path.knots = arc_knots(p0, p1, 12, false, false);
                    path.anchors = anchors(path.knots.len(), start_anchor, end_anchor);
                    path.tangents = catmull_rom(&path.knots);
                    path.time(None);
                }
                Some(Flyby::Planet(i)) => {
                    // Two arcs meeting at the closest point past the planet,
                    // their knots crowding in on it so the bend is tight.
                    let planet = &self.system.planets[i];
                    let reach = planet.radius
                        * match &planet.rings {
                            Some(r) => (r.outer * 1.25).max(3.0),
                            None if planet.kind.is_giant() => 3.0,
                            None => 3.2,
                        };
                    let mut when = 0.5 * (t0 + t1);
                    for _ in 0..3 {
                        let c = planet.position(when);
                        // Pass behind it (on its trailing side) to be pulled
                        // along with it, or in front of it to be slowed.
                        let ahead = normalize([-c[2], 0.0, c[0]]);
                        let outward = normalize([c[0], 0.0, c[2]]);
                        let gain = flat_radius(p1) > flat_radius(p0);
                        let side = if gain { -1.0 } else { 1.0 };
                        let dir = normalize(add(
                            add(scale(ahead, side), scale(outward, 0.4 * side)),
                            [0.0, lift, 0.0],
                        ));
                        let w = add(c, scale(dir, reach));
                        let mut knots = arc_knots(p0, w, 9, true, false);
                        knots.extend(arc_knots(w, p1, 9, false, true).into_iter().skip(1));
                        let n = knots.len();
                        path.anchors = anchors(n, start_anchor, end_anchor);
                        path.anchors[9] = (Anchor::Stop(Stop::planet(i)), when);
                        path.knots = knots;
                        path.waypoint = Some(9);
                        path.tangents = catmull_rom(&path.knots);
                        path.time(Some((c, reach, 12.0, 0.3)));
                        when = t0 + path.closest * (t1 - t0);
                    }
                }
                Some(Flyby::Star) => {
                    // A dive to perihelion round the far side of the star.
                    let (a, b) = (normalize(p0), normalize(p1));
                    let middle = add(a, b);
                    let dir = if length(middle) < 0.3 {
                        normalize(cross(a, [0.0, 1.0, 0.0]))
                    } else {
                        scale(normalize(middle), -1.0)
                    };
                    let dir = normalize(add(dir, [0.0, lift, 0.0]));
                    let w = scale(dir, reach_star);
                    path.knots = vec![p0, w, p1];
                    path.anchors = vec![start_anchor, (Anchor::Fixed, t0), end_anchor];
                    let first = sub(w, p0);
                    let last = sub(p1, w);
                    let m0 = add(scale(first, 0.5), scale(away, length(first) * 0.4));
                    let m2 = add(scale(last, 0.5), scale(into, length(last) * 0.25));
                    let through = add(normalize(first), normalize(last));
                    let mut tangent = sub(through, scale(dir, dot(through, dir)));
                    if length(tangent) < 1e-6 {
                        tangent = cross(dir, [0.0, 1.0, 0.0]);
                    }
                    let arm = length(first).min(length(last));
                    path.tangents = vec![m0, scale(normalize(tangent), arm * 0.8), m2];
                    path.waypoint = Some(1);
                    path.time(Some(([0.0; 3], reach_star, 3.0, 0.25)));
                }
            }
        }
        path.burns = self.burns(&path, from, to, t0, t1);
        path
    }

    /// The camera climbing out of `stop`'s orbit for the jump, which began
    /// at `start`, at `t`: the orbit carried on, and the climb out along the
    /// way the jump leaves, from the orbit's speed up.
    fn departing(&self, stop: Stop, start: f64, t: f64) -> [f64; 3] {
        let u = ((t - start) / DEPART).clamp(0.0, 1.0);
        let (p0, ..) = self.orbiting(stop, start);
        let out = normalize(sub(p0, self.system.centre(stop, start)));
        let distance = self.world(stop).radius * 30.0;
        let s = smootherstep(u);
        add(
            self.orbiting(stop, t).0,
            scale(normalize(add(out, self.outbound)), distance * s * s),
        )
    }

    /// The way to look from `position` at `t` to frame `stop` as its high
    /// orbit does (see `orbiting`): at the body, turned a little the way the
    /// orbit flown at `when` runs.
    fn framed(&self, stop: Stop, position: [f64; 3], t: f64, when: f64) -> [f64; 3] {
        let centre = self.system.centre(stop, t);
        let radial = normalize(sub(position, centre));
        let along = self.orbit_motion(stop, when);
        let prograde = sub(along, scale(radial, dot(along, radial)));
        if length(prograde) > 1e-6 {
            normalize(add(scale(radial, -0.97), scale(normalize(prograde), 0.24)))
        } else {
            scale(radial, -1.0)
        }
    }

    /// How fast the camera moves round `stop` at `t`, relative to the body:
    /// distance a second.
    fn orbit_speed(&self, stop: Stop, t: f64) -> f64 {
        let dt = 0.5;
        let rel = |t: f64| sub(self.orbiting(stop, t).0, self.system.centre(stop, t));
        length(sub(rel(t + dt), rel(t - dt))) / (2.0 * dt)
    }

    /// Which way the camera moves round `stop` just after `t`, relative to
    /// the body.
    fn orbit_motion(&self, stop: Stop, t: f64) -> [f64; 3] {
        let dt = 0.5;
        let rel = |t: f64| sub(self.orbiting(stop, t).0, self.system.centre(stop, t));
        normalize(sub(rel(t + dt), rel(t)))
    }

    /// Which way transfer `k`'s last stretch heads, relative to the body
    /// it reaches: from a quarter of the way before the end to a tenth.
    fn heading_in(&self, k: usize) -> [f64; 3] {
        let stop = self.itinerary[k];
        let segment = self.transfer_to(stop);
        let t1 = segment.start + segment.duration;
        let rel = |t: f64| sub(self.on_path(k, segment, t).0, self.system.centre(stop, t));
        normalize(sub(
            rel(t1 - 0.1 * segment.duration),
            rel(t1 - 0.25 * segment.duration),
        ))
    }

    /// When the transfer to stop `k` ends.
    fn arrival_time(&self, k: usize) -> f64 {
        let segment = self.transfer_to(self.itinerary[k]);
        segment.start + segment.duration
    }

    /// The axes of `orbit`'s plane round `stop` at `t`, as `orbiting` sets
    /// them: `u` toward the sun, `v` a quarter turn on.
    fn plane_axes(&self, stop: Stop, orbit: Orbit, t: f64) -> ([f64; 3], [f64; 3]) {
        let sun = normalize(scale(self.system.centre(stop, t), -1.0));
        let flat = sub(sun, scale(orbit.n, dot(sun, orbit.n)));
        let u = if length(flat) > 1e-6 {
            normalize(flat)
        } else {
            orbit.u
        };
        (u, cross(orbit.n, u))
    }

    /// The transfer segment that reaches `stop`.
    fn transfer_to(&self, stop: Stop) -> Segment {
        self.segments
            .iter()
            .find(|s| matches!(s.phase, Phase::Transfer { to, .. } if to == stop))
            .copied()
            .expect("every stop has a transfer")
    }

    /// The numbers behind a transfer's manoeuvres. Distances drawn are
    /// turned back into real ones (orbits ten times further, the HUD's AU),
    /// and speeds come from the vis-viva equation and patched conics.
    fn burns(&self, path: &Path, from: Option<Stop>, to: Stop, t0: f64, t1: f64) -> Burns {
        let star = &self.system.star;
        let gm_star = super::system::GM_SUN * star.mass;
        let km = |units: f64| (units / AU).max(0.002) * REAL_AU * KM;
        let a = path.knots[0];
        let b = *path.knots.last().expect("a path has knots");
        let (r0, r1) = (km(length(a)), km(length(b)));
        let semi = 0.5 * (r0 + r1);
        let transfer = |r: f64| (gm_star * (2.0 / r - 1.0 / semi)).max(0.0).sqrt();
        let circular = |r: f64| (gm_star / r).sqrt();
        // Leaving and being caught: the excess speed over a circular orbit,
        // added in quadrature to the planet's escape speed at the high orbit,
        // less the speed in that orbit.
        let at_planet = |stop: Stop, vinf: f64, t: f64, camera: [f64; 3]| {
            let world = self.world(stop);
            let mu = super::system::GM_EARTH * world.mass;
            let r = length(sub(camera, self.system.centre(stop, t))).max(world.radius) * KM;
            ((vinf * vinf + 2.0 * mu / r).sqrt() - (mu / r).sqrt()).max(0.1)
        };
        let mut burns = Burns {
            sweep: prograde(a, b).to_degrees(),
            from_au: length(a) / AU,
            to_au: length(b) / AU,
            ..Default::default()
        };
        if to.moon.is_some() {
            let giant = &self.system.planets[to.planet];
            let mu = super::system::GM_EARTH * giant.mass;
            let g = giant.position(t0);
            let (ra, rb) = (length(sub(a, g)) * KM, length(sub(b, g)) * KM);
            // A Hohmann hop between the two orbits round the giant.
            let semi = 0.5 * (ra + rb);
            let v = |r: f64| (mu * (2.0 / r - 1.0 / semi)).max(0.0).sqrt();
            burns.depart = (v(ra) - (mu / ra).sqrt()).abs();
            burns.capture = ((mu / rb).sqrt() - v(rb)).abs() + 0.2;
            return burns;
        }
        burns.depart = match from {
            Some(f) => at_planet(f, (transfer(r0) - circular(r0)).abs(), t0, a),
            None => 0.0,
        };
        burns.capture = at_planet(to, (transfer(r1) - circular(r1)).abs(), t1, b);
        match path.flyby {
            Some(Flyby::Planet(i)) => {
                let planet = &self.system.planets[i];
                let w = path.knots[path.waypoint.unwrap_or(0)];
                let c = planet.position(t0 + path.closest * (t1 - t0));
                let rc = km(length(c));
                let vinf = (transfer(rc) - circular(rc)).abs().max(3.0);
                let mu = super::system::GM_EARTH * planet.mass;
                let rp = length(sub(w, c)) * KM;
                // The turn of a hyperbolic flyby, and the speed it trades.
                let turn = 2.0 * (1.0 / (1.0 + rp * vinf * vinf / mu)).asin();
                let dv = 2.0 * vinf * (turn / 2.0).sin();
                let gain = flat_radius(b) > flat_radius(a);
                burns.assist_vinf = vinf;
                burns.assist_turn = turn.to_degrees();
                burns.assist_dv = if gain { dv } else { -dv };
            }
            Some(Flyby::Star) => {
                let w = path.knots[path.waypoint.unwrap_or(0)];
                let rp = km(length(w));
                let semi = 0.5 * (rp + r1.max(r0));
                let vp = (gm_star * (2.0 / rp - 1.0 / semi)).max(0.0).sqrt();
                let burn = 1.5 + (self.system.seed % 7) as f64 * 0.35;
                // The energy a burn adds is v·dv + dv²/2: at perihelion v is
                // large, so a small burn there equals a big one far out.
                burns.oberth_burn = burn;
                burns.oberth_speed = vp;
                burns.oberth_equal = (2.0 * vp * burn + burn * burn).sqrt();
            }
            None => {}
        }
        burns
    }

    /// Turn each visited planet's largest moon to where the camera will see
    /// it: past the planet's limb, a little to one side, half way through
    /// the high orbit. Moons are small against their planets; left to chance
    /// they would mostly be behind the camera or lost in the glare.
    fn stage_moons(&mut self) {
        for &stop in &self.itinerary.clone() {
            if stop.moon.is_some() {
                continue;
            }
            let p = stop.planet;
            let Some(segment) = self
                .segments
                .iter()
                .find(|s| s.phase == Phase::High(stop))
                .copied()
            else {
                continue;
            };
            let planet = &self.system.planets[p];
            let Some(largest) = (0..planet.moons.len())
                .max_by(|&a, &b| planet.moons[a].radius.total_cmp(&planet.moons[b].radius))
            else {
                continue;
            };
            let t = segment.start + segment.duration * 0.55;
            let (camera, forward, up) = self.orbiting(stop, t);
            let centre = planet.position(t);
            let right = normalize(cross(forward, up));
            // Beyond the planet, off to the side at about the edge of view.
            let wanted = normalize(add(
                scale(normalize(sub(centre, camera)), 1.0),
                add(scale(right, 0.42), scale(up, 0.12)),
            ));
            let mut best = (f64::MIN, 0.0);
            for k in 0..96 {
                let phase = k as f64 / 96.0 * std::f64::consts::TAU;
                let mut trial = planet.clone();
                trial.moons[largest].phase = phase;
                let offset = trial.moon_offset(largest, t);
                let at = add(centre, offset);
                let score = dot(normalize(sub(at, camera)), wanted);
                if score > best.0 {
                    best = (score, phase);
                }
            }
            self.system.planets[p].moons[largest].phase = best.1;
        }
    }

    /// The drive the jump at the end of this visit uses: its number (from 1)
    /// and name.
    pub fn drive(&self) -> (u32, &'static str) {
        drive_to(&self.next)
    }

    /// Where the camera is out in the deep at `t`, past the jump point: on
    /// along the jump's way, at the jump's speed in its legs and a drift
    /// between them.
    fn deep_position(&self, t: f64) -> [f64; 3] {
        let mut travelled = 0.0;
        for segment in self.segments.iter().filter(|s| s.phase.deep()) {
            let spent = (t - segment.start).clamp(0.0, segment.duration);
            let speed = if matches!(segment.phase, Phase::Jump(_)) {
                JUMP_SPEED
            } else {
                DEEP_DRIFT
            };
            travelled += spent * speed;
        }
        add(self.spooling(SPOOL), scale(self.exit, travelled))
    }

    /// The stopover `k` of the jump `local` seconds in (on past its end
    /// while the drive re-engages), as the shaders take it.
    pub fn scene(&self, k: usize, local: f64, fade: f64) -> Scene {
        let stop = self.passage.stops[k];
        Scene {
            event: stop.event,
            local,
            duration: stop.event.duration(),
            variant: (stop.variant % 1000) as f64 / 1000.0,
            fade,
            shot: voyage::shot(&stop, local),
            basis: self.basis,
        }
    }

    /// A direction in the encounter frame, in the world.
    fn frame_to_world(&self, v: [f64; 3]) -> [f64; 3] {
        let [x, y, z] = self.basis;
        add(add(scale(x, v[0]), scale(y, v[1])), scale(z, v[2]))
    }

    /// The camera in a stopover: looking where its shot looks, banked and
    /// shaken.
    fn stopover_frame(&self, shot: &Shot, t: f64) -> ([f64; 3], [f64; 3]) {
        let mut forward = self.frame_to_world(shot.look);
        let up0 = voyage::rotate(self.basis[1], forward, shot.roll);
        let right = normalize(cross(forward, up0));
        let up = cross(right, forward);
        if shot.shake > 0.0 {
            let n = |a: f64, b: f64, c: f64| {
                (t * a).sin() + 0.6 * (t * b + 1.3).sin() + 0.3 * (t * c + 0.4).sin()
            };
            let amount = 0.0045 * shot.shake;
            forward = normalize(add(
                forward,
                add(
                    scale(right, n(23.0, 41.0, 67.0) * amount),
                    scale(up, n(29.0, 47.0, 71.0) * amount),
                ),
            ));
        }
        (forward, up)
    }

    /// The segment at `t`, and how far into it.
    pub fn segment(&self, t: f64) -> (Segment, f64) {
        let t = t.clamp(0.0, self.total - 1e-6);
        for segment in &self.segments {
            if t < segment.start + segment.duration {
                return (*segment, t - segment.start);
            }
        }
        let last = *self.segments.last().expect("a tour has segments");
        (last, last.duration)
    }

    fn index_of(&self, stop: Stop) -> usize {
        self.itinerary.iter().position(|&s| s == stop).unwrap_or(0)
    }

    /// The body at a stop, as a planet.
    pub fn world(&self, stop: Stop) -> &Planet {
        &self.worlds[self.index_of(stop)]
    }

    fn orbit_of(&self, stop: Stop) -> Orbit {
        self.orbits[self.index_of(stop)]
    }

    /// When the camera reaches `stop`'s high orbit.
    fn arrival(&self, stop: Stop) -> f64 {
        self.segments
            .iter()
            .find(|s| s.phase == Phase::High(stop))
            .map_or(0.0, |s| s.start)
    }

    /// The orbit's distance from the body's centre.
    fn height(planet: &Planet) -> f64 {
        planet.radius * if planet.kind.is_giant() { 3.7 } else { 3.0 }
    }

    /// Angle along the orbit at `t`, a turn every 150 seconds from the
    /// arrival (and before it, on the transfer's last stretch, so the
    /// hand-over is smooth).
    fn orbit_angle(&self, stop: Stop, t: f64) -> f64 {
        self.orbit_of(stop).angle + (t - self.arrival(stop)) * std::f64::consts::TAU / 150.0
    }

    /// Camera position, forward and up while orbiting `stop` at `t`.
    fn orbiting(&self, stop: Stop, t: f64) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let body = self.world(stop);
        let centre = self.system.centre(stop, t);
        let orbit = self.orbit_of(stop);
        let distance = Self::height(body);
        let angle = self.orbit_angle(stop, t);
        let (s, c) = angle.sin_cos();
        // The plane's axes follow the sun, which an inner planet's own orbit
        // swings round during a visit: angle 0 is noon below the camera.
        let (u, v) = self.plane_axes(stop, orbit, t);
        let radial = add(scale(u, c), scale(v, s));
        let prograde = add(scale(u, -s), scale(v, c));
        let position = add(centre, scale(radial, distance));
        // The planet a little off centre.
        let forward = normalize(add(scale(radial, -0.97), scale(prograde, 0.24)));
        (position, forward, orbit.n)
    }

    /// Everything a transfer path can bump into: planets, moons and the
    /// star, with how close the camera may come, and whether it is a moon.
    fn obstacles(&self, t: f64) -> Vec<([f64; 3], f64, bool)> {
        let mut obstacles = Vec::new();
        for (i, p) in self.system.planets.iter().enumerate() {
            let centre = p.position(t);
            obstacles.push((
                centre,
                p.radius * if p.kind.is_giant() { 2.8 } else { 2.0 },
                false,
            ));
            for m in 0..p.moons.len().min(super::stream::MOONS as usize) {
                let stop = Stop {
                    planet: i,
                    moon: Some(m),
                };
                obstacles.push((
                    self.system.centre(stop, t),
                    p.moons[m].radius * MOON_FLOOR,
                    true,
                ));
            }
        }
        obstacles.push(([0.0; 3], self.system.star.radius * 3.0, false));
        obstacles
    }

    /// Where transfer `k`'s spline, carried along by the bodies at its
    /// ends, is at `t` before anything is dodged, and how far along it is.
    fn on_spline(&self, k: usize, segment: Segment, t: f64) -> ([f64; 3], f64) {
        let path = &self.paths[k];
        let u = ((t - segment.start) / segment.duration).clamp(0.0, 1.0);
        let sigma = path.sigma(u);
        let mut position = path.spline(sigma);
        // Each end knot moves with its body, blended along the spline.
        let last = path.knots.len() - 1;
        let j = (sigma.floor() as usize).min(last - 1);
        let s = smoothstep(0.0, 1.0, sigma - j as f64);
        for (knot, weight) in [(j, 1.0 - s), (j + 1, s)] {
            if Some(knot) == path.waypoint {
                continue;
            }
            if let (Anchor::Stop(stop), placed) = path.anchors[knot] {
                let drift = sub(
                    self.system.centre(stop, t),
                    self.system.centre(stop, placed),
                );
                position = add(position, scale(drift, weight));
            }
        }
        // A flyby's closest point moves with the planet passed over a span
        // of time round the pass, not of the spline: its knots crowd in on
        // it, and a blend over them would jerk the camera as the planet,
        // moving as fast as the camera, is taken on and let go.
        if let Some(w) = path.waypoint
            && let (Anchor::Stop(stop), placed) = path.anchors[w]
        {
            let drift = sub(
                self.system.centre(stop, t),
                self.system.centre(stop, placed),
            );
            let span = 0.15 * segment.duration;
            let x = (t - placed) / span;
            let weight = (1.0 - x * x).max(0.0).powi(2);
            position = add(position, scale(drift, weight));
        }
        (position, sigma)
    }

    /// The camera's position on transfer `k`'s path at `t`, and how far
    /// along the spline it is.
    fn on_path(&self, k: usize, segment: Segment, t: f64) -> ([f64; 3], f64) {
        let (position, sigma) = self.on_spline(k, segment, t);
        let mut position = add(position, self.keep_away(k, segment, t, Some(false)));
        // A moon is dodged as it is over a span round now, the camera with
        // it: a small moon passed at speed would otherwise shove the camera
        // aside in a moment. Far from every moon, where the push is nil,
        // what it is now will do.
        let near = self
            .obstacles(t)
            .iter()
            .filter(|o| o.2)
            .map(|&(centre, floor, _)| length(sub(position, centre)) / floor)
            .fold(f64::MAX, f64::min);
        let spread = 1.0 - smoothstep(20.0, 30.0, near);
        let now = self.keep_away(k, segment, t, Some(true));
        let mut dodge = scale(now, 1.0 - spread);
        if spread > 0.0 {
            for (n, weight) in MOON_SPAN.iter().enumerate() {
                let when = t + MOON_SMOOTH * (n as f64 - 3.0) / 4.0;
                let push = if n == 3 {
                    now
                } else {
                    self.keep_away(k, segment, when, Some(true))
                };
                dodge = add(dodge, scale(push, weight * spread));
            }
        }
        position = add(position, dodge);
        (position, sigma)
    }

    /// How far the camera on transfer `k`'s path at `t` swings round
    /// what is in the way (only the moons, all but them, or everything): a
    /// soft floor on the distance from every body. Toward each end the push
    /// its end point would get is taken off, so the push is nil where the
    /// path meets the orbits (which may lie within a neighbour's floor) and
    /// nothing snaps the camera back as it lets go.
    fn keep_away(&self, k: usize, segment: Segment, t: f64, moons: Option<bool>) -> [f64; 3] {
        let path = &self.paths[k];
        let u = ((t - segment.start) / segment.duration).clamp(0.0, 1.0);
        let (position, _) = self.on_spline(k, segment, t);
        let last = path.knots.len() - 1;
        let fade = smoothstep(0.0, 0.02, u) * (1.0 - smoothstep(0.98, 1.0, u));
        let ends: Vec<[f64; 3]> = [0, last]
            .iter()
            .map(|&knot| match path.anchors[knot] {
                (Anchor::Stop(stop), placed) => add(
                    path.knots[knot],
                    sub(
                        self.system.centre(stop, t),
                        self.system.centre(stop, placed),
                    ),
                ),
                (Anchor::Fixed, _) => path.knots[knot],
            })
            .collect();
        let push = |p: [f64; 3], centre: [f64; 3], floor: f64| {
            let rel = sub(p, centre);
            let d = length(rel);
            let k = floor * 0.2;
            let soft = 0.5 * (d + floor + ((d - floor) * (d - floor) + k * k).sqrt());
            scale(normalize(rel), soft - d)
        };
        let (w_start, w_end) = (1.0 - smoothstep(0.0, 0.4, u), smoothstep(0.6, 1.0, u));
        let mut moved = [0.0; 3];
        for (centre, floor, moon) in self.obstacles(t) {
            if moons.is_some_and(|moons| moon != moons) {
                continue;
            }
            moved = add(moved, push(position, centre, floor));
            moved = sub(moved, scale(push(ends[0], centre, floor), w_start));
            moved = sub(moved, scale(push(ends[1], centre, floor), w_end));
        }
        scale(moved, fade)
    }

    /// Where the flyby of transfer `k` passes, at `t`, and its name.
    fn flyby_body(&self, k: usize, t: f64) -> Option<([f64; 3], f64, String)> {
        match self.paths[k].flyby? {
            Flyby::Star => Some((
                [0.0; 3],
                self.system.star.radius,
                self.system.star.name.clone(),
            )),
            Flyby::Planet(i) => {
                let p = &self.system.planets[i];
                Some((p.position(t), p.radius, p.name.clone()))
            }
        }
    }

    /// The camera at `t` seconds into the visit.
    pub fn view(&self, t: f64) -> View {
        let (segment, local) = self.segment(t);
        let u = (local / segment.duration).clamp(0.0, 1.0);
        let mut warp = 0.0;
        let (mut flare, mut reveal, mut clock) = (0.0, 1.0, 0.0);
        let (mut spool, mut spool_light, mut target_shown) = (0.0, 0.0, 0.0);
        let (mut look, mut drop_look) = (self.drive().0, self.arrival);
        let (mut deep, mut scene, mut flux, mut flash) = (segment.phase.deep(), None, None, 0.0);
        // Dropping out: the jump's light tears open over the new system (or
        // a stopover, after a leg of `length` seconds), its drive slowing on
        // from 0.4 to 0.15.
        let drop_out = |local: f64, length: f64| {
            let p = local / DROP_OUT;
            (
                p,
                (1.0 - p).powi(3),
                drive_clock(length, length) + DROP_OUT * (0.4 * p - 0.125 * p * p),
            )
        };
        let (position, forward, up) = match segment.phase {
            Phase::High(p) => self.orbiting(p, t),
            Phase::Transfer { from, to } => {
                let k = self.index_of(to);
                let t0 = segment.start;
                let t1 = segment.start + segment.duration;
                let (_, f0, up0) = match from {
                    Some(p) => self.orbiting(p, t0),
                    None => {
                        let toward = normalize(scale(self.entry, -1.0));
                        (self.entry, toward, [0.0, 1.0, 0.0])
                    }
                };
                let (_, _, up1) = self.orbiting(to, t1);
                let (position, _) = self.on_path(k, segment, t);
                // Keep the destination in the middle of the view the whole
                // way: what the path swings past crosses the view on its own.
                let centre = self.system.centre(to, t);
                let aim = normalize(sub(centre, position));
                // Leave framing the body left, and settle into the framing
                // of the one reached, each worked out from where the camera
                // is: they are the orbits' own at either end, and keep the
                // bodies in view in between.
                let start = match from {
                    Some(p) => self.framed(p, position, t, t0),
                    None => f0,
                };
                // Which way round to turn from the one to the other, settled
                // once, halfway through the turn.
                let way = {
                    let at = t0 + 0.09 * segment.duration;
                    let (there, _) = self.on_path(k, segment, at);
                    let start = match from {
                        Some(p) => self.framed(p, there, at, t0),
                        None => f0,
                    };
                    heading_change(
                        start,
                        normalize(sub(self.system.centre(to, at), there)),
                        up0,
                    )
                };
                let mut forward = swing(start, aim, smoothstep(0.0, 0.18, u), up0, way);
                forward = normalize(mix(
                    forward,
                    self.framed(to, position, t, t1),
                    smoothstep(0.82, 1.0, u),
                ));
                let up = normalize(mix(up0, up1, smoothstep(0.1, 0.9, u)));
                if from.is_none() && local < DROP_OUT {
                    (reveal, flare, clock) = drop_out(local, JUMP);
                }
                (position, forward, up)
            }
            Phase::Depart(p) => {
                // Still going round as it climbs out, so it never stops.
                let (_, f0, up0) = self.orbiting(p, t);
                let s = smootherstep(u);
                let position = self.departing(p, segment.start, t);
                let forward = normalize(mix(f0, self.outbound, smoothstep(0.0, 0.8, u)));
                let up = normalize(mix(up0, [0.0, 1.0, 0.0], s));
                (position, forward, up)
            }
            Phase::Cruise { from } => {
                // Out to the jump point, the next star marked once clear.
                if from.is_none() && local < DROP_OUT {
                    (reveal, flare, clock) = drop_out(local, JUMP);
                }
                target_shown = smoothstep(0.05, 0.2, u);
                (self.cruising(t), self.cruise_forward(t), [0.0, 1.0, 0.0])
            }
            Phase::Spool => {
                // At the jump point, turned onto the next star while the
                // drive spools up: its start-up effect builds through the
                // stages, and its light rises over the engagement.
                spool = u;
                spool_light = smoothstep(SPOOL_STAGES[1] * 0.5, 1.0, u);
                target_shown = 1.0;
                warp = 0.25 * smoothstep(SPOOL_STAGES[4], 1.0, u);
                clock = local - SPOOL;
                (self.spooling(local), self.spool_forward(u), [0.0, 1.0, 0.0])
            }
            Phase::Jump(k) => {
                // A leg of the jump, out along its way toward the next star.
                let k = k as usize;
                let leg = self.passage.legs[k];
                look = leg.look;
                // At full light to the end: the next visit or stopover takes
                // it over (`reveal`), slowing, the flare at its brightest. Into
                // the first leg, and after a stopover, the drive's start-up
                // effect fades into it; out of a gate or a wormhole, the ship
                // is thrown into it at full light, in a flash.
                let thrown = k > 0 && self.passage.stops[k - 1].event.portal();
                if thrown {
                    warp = 1.0;
                    flash = (-local / 0.22).exp();
                } else {
                    warp = 0.25 + 0.75 * smoothstep(0.0, 0.07, u);
                    let fade = if k == 0 { SPOOL_FADE } else { REJUMP_FADE };
                    spool_light = 1.0 - smoothstep(0.0, fade, local);
                }
                spool = 1.0;
                target_shown = spool_light;
                flare = smoothstep(segment.duration - FLARE, segment.duration, local);
                clock = drive_clock(local, segment.duration);
                deep = k > 0;
                flux = leg.flux.map(|f| (f.id(), leg.flux_amount(local), local));
                (self.deep_position(t), self.exit, [0.0, 1.0, 0.0])
            }
            Phase::Stopover(k) => {
                // Dropped out of the jump (or come to a gate or a wormhole):
                // the stopover's own script, in its frame, the leg's light
                // torn open over it.
                let k = k as usize;
                let made = self.scene(k, local, 1.0);
                drop_look = self.passage.legs[k].look;
                look = drop_look;
                if local < DROP_OUT {
                    (reveal, flare, clock) = drop_out(local, self.passage.legs[k].duration);
                }
                let (forward, up) = self.stopover_frame(&made.shot, t);
                scene = Some(made);
                (self.deep_position(t), forward, up)
            }
            Phase::Rejump(k) => {
                // The drive re-engaging, quickly, in its stages: turned back
                // onto the next star, what was met still in view behind its
                // start-up, until its light rises over everything.
                let k = k as usize;
                let stop = self.passage.stops[k];
                // A stopover in place of the sky gives it back first, so the
                // drive's start-up shows; the others fade as its light rises.
                let fade = if stop.event.replaces_sky() {
                    1.0 - smoothstep(0.0, 0.3, u)
                } else {
                    1.0 - smoothstep(SPOOL_STAGES[4], 1.0, u)
                };
                let made = self.scene(k, stop.event.duration() + local, fade);
                spool = u;
                spool_light = smoothstep(SPOOL_STAGES[1] * 0.5, 1.0, u);
                target_shown = smoothstep(0.0, 0.3, u);
                warp = 0.25 * smoothstep(SPOOL_STAGES[4], 1.0, u);
                clock = local - segment.duration;
                let (forward, up) = self.stopover_frame(&made.shot, t);
                scene = Some(made);
                (self.deep_position(t), forward, up)
            }
        };
        // Keep the frame orthonormal.
        let right = normalize(cross(forward, up));
        let up = cross(right, forward);
        View {
            position,
            forward,
            up,
            half_fov: HALF_FOV,
            warp,
            flare,
            reveal,
            clock,
            spool,
            spool_light,
            target: self.exit,
            target_shown,
            look,
            drop_look,
            deep,
            scene,
            flux,
            flash,
        }
    }

    /// The stops whose maps should be resident at `t`: the one being visited
    /// or approached, and the next. At a moon, its giant stays resident, as
    /// it fills the moon's sky. Leaving a stop, it stays resident until it
    /// is a few pixels across (`LEFT_BEHIND`): until then it would show as
    /// its flat colour. During the jump, none: the next visit's first stop
    /// is what to stream.
    pub fn wanted(&self, t: f64) -> (Option<Stop>, Option<Stop>) {
        let (segment, _) = self.segment(t);
        let second = |p: Stop| {
            if p.moon.is_some() {
                return Some(Stop::planet(p.planet));
            }
            let k = self.itinerary.iter().position(|&q| q == p)?;
            self.itinerary.get(k + 1).copied()
        };
        match segment.phase {
            Phase::Transfer {
                from: Some(from),
                to,
            } if self.in_sight(from, t) => (Some(to), Some(from)),
            Phase::Transfer { to, .. } => (Some(to), second(to)),
            Phase::High(p) => (Some(p), second(p)),
            Phase::Depart(p) => (Some(p), p.moon.map(|_| Stop::planet(p.planet))),
            Phase::Cruise { from: Some(p) } if self.in_sight(p, t) => (Some(p), None),
            Phase::Cruise { .. }
            | Phase::Spool
            | Phase::Jump(_)
            | Phase::Stopover(_)
            | Phase::Rejump(_) => (None, None),
        }
    }

    /// Whether `stop` is still more than `LEFT_BEHIND` of the view across.
    fn in_sight(&self, stop: Stop, t: f64) -> bool {
        let view = self.view(t);
        let distance = length(sub(self.system.centre(stop, t), view.position));
        self.world(stop).radius / distance > LEFT_BEHIND * view.half_fov
    }

    /// The transfer path in view at `t`, from where the camera is to where it
    /// is going, as points: for the minimap.
    pub fn route(&self, t: f64) -> Vec<[f64; 3]> {
        let (segment, _) = self.segment(t);
        let steps = 48;
        if let Phase::Cruise { .. } = segment.phase {
            let end = segment.start + segment.duration;
            return (0..=steps)
                .map(|i| self.cruising(t + (end - t) * i as f64 / steps as f64))
                .collect();
        }
        let Phase::Transfer { to, .. } = segment.phase else {
            return Vec::new();
        };
        let k = self.index_of(to);
        (0..=steps)
            .map(|i| {
                let tt = t + (segment.start + segment.duration - t) * i as f64 / steps as f64;
                // Moons dodged as they are now: at the map's scale the
                // difference does not show, and the camera's spread costs.
                let (on, _) = self.on_spline(k, segment, tt);
                add(on, self.keep_away(k, segment, tt, None))
            })
            .collect()
    }

    /// When the transfer to `stop` passes closest to what it flies past.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn flyby_at(&self, stop: Stop) -> Option<f64> {
        let k = self.itinerary.iter().position(|&s| s == stop)?;
        self.paths[k].flyby?;
        let segment = self
            .segments
            .iter()
            .find(|s| matches!(s.phase, Phase::Transfer { to, .. } if to == stop))?;
        Some(segment.start + segment.duration * self.paths[k].closest)
    }

    /// The stop being visited or made for at `t`.
    pub fn current(&self, t: f64) -> Option<Stop> {
        match self.segment(t).0.phase {
            Phase::Transfer { to, .. } => Some(to),
            Phase::High(p) | Phase::Depart(p) => Some(p),
            Phase::Cruise { .. }
            | Phase::Spool
            | Phase::Jump(_)
            | Phase::Stopover(_)
            | Phase::Rejump(_) => None,
        }
    }

    /// The jump's way through relays, for the HUD: " via A, B" or nothing.
    fn via(&self) -> String {
        if self.route.len() < 2 {
            return String::new();
        }
        let relays: Vec<&str> = self.route[..self.route.len() - 1]
            .iter()
            .map(|s| s.star.name.as_str())
            .collect();
        format!(" via {}", relays.join(", "))
    }

    /// The star the jump makes for, and the destination when that is past
    /// a relay.
    fn making_for(&self) -> String {
        let next = &self.next.star;
        if self.route.len() > 1 {
            format!("{} (relay to {})", next.name, self.destination().star.name)
        } else {
            format!("{} ({})", next.name, next.title())
        }
    }

    /// The speed needed to escape the star from `position`, km/s: what the
    /// flight out climbs away from.
    fn escape_speed(&self, position: [f64; 3]) -> f64 {
        let r = (length(position) / AU).max(0.002) * REAL_AU * KM;
        (2.0 * super::system::GM_SUN * self.system.star.mass / r).sqrt()
    }

    /// The manoeuvre under way at `t`, named and explained: the burns that
    /// leave and enter orbits, the coast along a transfer ellipse, a gravity
    /// assist or an Oberth dive past the star, and the escape before a jump.
    /// None while the scanner has the panel (in orbit) or during the jump.
    pub fn manoeuvre(&self, t: f64) -> Option<Manoeuvre> {
        let (segment, local) = self.segment(t);
        let u = local / segment.duration;
        let star = &self.system.star.name;
        let fire = Manoeuvre::burn;
        let make = Manoeuvre::coast;
        match segment.phase {
            Phase::Transfer { from, to } => {
                let k = self.index_of(to);
                let path = &self.paths[k];
                let burns = &path.burns;
                let dest = &self.world(to).name;
                let origin = from.map(|f| self.world(f).name.clone());
                if to.moon.is_some() {
                    let giant = &self.system.planets[to.planet].name;
                    return if u < 0.15 {
                        fire(
                            "TRANSFER BURN",
                            u / 0.15,
                            burns.depart,
                            "prograde",
                            format!("+{:.2} km/s, raising our orbit round {giant}", burns.depart),
                            format!("on the ellipse out to {dest}"),
                        )
                    } else if u > 0.85 {
                        fire(
                            "ORBIT MATCH",
                            (u - 0.85) / 0.15,
                            burns.capture,
                            "retrograde",
                            format!("−{:.2} km/s to match {dest}'s pace", burns.capture),
                            format!("in orbit round {dest}"),
                        )
                    } else {
                        make(
                            "MOON TRANSFER",
                            format!(
                                "Coasting on a Hohmann ellipse across {giant}'s moon system: half an orbit round the giant, from our orbit out to {dest}'s."
                            ),
                            -1.0,
                        )
                    };
                }
                let window = 0.15;
                if let Some(origin) = &origin
                    && u < 0.12
                {
                    return fire(
                        "DEPARTURE BURN",
                        u / 0.12,
                        burns.depart,
                        "prograde",
                        format!("+{:.1} km/s to break free of {origin}", burns.depart),
                        format!("on a transfer orbit round {star}"),
                    );
                }
                if origin.is_none() && u < 0.1 {
                    return make(
                        "DROP-OUT",
                        format!(
                            "Out of the jump and falling sunward toward {dest}, engines quiet."
                        ),
                        -1.0,
                    );
                }
                if u > CAPTURE {
                    return fire(
                        "CAPTURE BURN",
                        (u - CAPTURE) / (1.0 - CAPTURE),
                        burns.capture,
                        "retrograde, engine first",
                        format!("−{:.1} km/s as we swing in close", burns.capture),
                        format!("captured into orbit round {dest}"),
                    );
                }
                let near = u - path.closest;
                match path.flyby {
                    Some(Flyby::Planet(i)) => {
                        let name = &self.system.planets[i].name;
                        let (verb, why) = if burns.assist_dv >= 0.0 {
                            ("gains", "flung outward")
                        } else {
                            ("sheds", "dropped inward")
                        };
                        if near.abs() < window {
                            make(
                                "GRAVITY ASSIST",
                                format!(
                                    "Slingshot past {name} at {:.1} km/s: its gravity bends our course {:.0}° and the ship {verb} {:.1} km/s round {star}, free, {why} toward {dest}.",
                                    burns.assist_vinf,
                                    burns.assist_turn,
                                    burns.assist_dv.abs()
                                ),
                                (near + window) / (2.0 * window),
                            )
                        } else if near < 0.0 {
                            make(
                                "ASSIST APPROACH",
                                if burns.assist_dv >= 0.0 {
                                    format!(
                                        "Aiming just behind {name}, on its trailing side: its gravity will pull us along and turn us toward {dest} without a burn."
                                    )
                                } else {
                                    format!(
                                        "Aiming just ahead of {name}: its gravity will brake us and turn us inward toward {dest} without a burn."
                                    )
                                },
                                -1.0,
                            )
                        } else {
                            make(
                                "COAST",
                                format!(
                                    "Coasting on to {dest} with the {:.1} km/s {name} {} us.",
                                    burns.assist_dv.abs(),
                                    if burns.assist_dv >= 0.0 {
                                        "lent"
                                    } else {
                                        "took from"
                                    }
                                ),
                                -1.0,
                            )
                        }
                    }
                    Some(Flyby::Star) => {
                        if near.abs() < window {
                            fire(
                                "OBERTH MANOEUVRE",
                                (near + window) / (2.0 * window),
                                burns.oberth_burn,
                                "prograde at perihelion",
                                format!(
                                    "+{:.1} km/s while moving at {:.0} km/s",
                                    burns.oberth_burn, burns.oberth_speed
                                ),
                                format!("worth {:.0} km/s far from {star}", burns.oberth_equal),
                            )
                        } else if near < 0.0 {
                            make(
                                "SUNDIVER",
                                format!(
                                    "Falling toward {star} to pass it close, where a burn counts for most (the Oberth effect), then on to {dest}."
                                ),
                                -1.0,
                            )
                        } else {
                            make(
                                "COAST",
                                format!(
                                    "Climbing away from {star}, the perihelion burn's energy carrying us out to {dest}."
                                ),
                                -1.0,
                            )
                        }
                    }
                    None => {
                        let (r0, r1) = (burns.from_au, burns.to_au);
                        let hohmann = (burns.sweep - 180.0).abs() < 35.0;
                        if origin.is_none() {
                            make(
                                "APPROACH",
                                format!(
                                    "Falling in from {r0:.2} AU toward {dest}'s orbit at {r1:.2} AU, {star}'s gravity doing the work."
                                ),
                                -1.0,
                            )
                        } else if hohmann {
                            make(
                                "HOHMANN TRANSFER",
                                format!(
                                    "Coasting on half an ellipse round {star}, from {r0:.2} AU out to {r1:.2} AU, meeting {dest}'s orbit tangentially: the cheapest way between two orbits."
                                ),
                                -1.0,
                            )
                        } else {
                            make(
                                "TRANSFER ORBIT",
                                format!(
                                    "Engines off, coasting {:.0}° round {star} on an ellipse from {r0:.2} AU to {dest}'s orbit at {r1:.2} AU.",
                                    burns.sweep
                                ),
                                -1.0,
                            )
                        }
                    }
                }
            }
            Phase::Depart(p) => {
                let world = self.world(p);
                let mu = super::system::GM_EARTH * world.mass;
                let r = length(sub(self.view(t).position, self.system.centre(p, t)))
                    .max(world.radius)
                    * KM;
                // From orbital speed to escape speed, at the orbit left.
                let r0 = Self::height(world) * KM;
                fire(
                    "ESCAPE BURN",
                    u,
                    (2.0f64.sqrt() - 1.0) * (mu / r0).sqrt(),
                    "prograde",
                    format!(
                        "past escape velocity, {:.1} km/s here",
                        (2.0 * mu / r).sqrt()
                    ),
                    format!("clear of {}, out to the jump point", world.name),
                )
            }
            Phase::Cruise { from } => {
                let (_, drive) = self.drive();
                let to = &self.next.star.name;
                let goal = &self.destination().star.name;
                if from.is_none() && u < 0.08 {
                    return make(
                        "DROP-OUT",
                        format!(
                            "Out of the jump at the edge of {star}, a relay star on the way to {goal}."
                        ),
                        -1.0,
                    );
                }
                let au = length(self.jump_point) / AU;
                match from {
                    Some(_) => make(
                        "TO THE JUMP POINT",
                        format!(
                            "Coasting out of {star}'s gravity well to a jump point {au:.2} AU out, where space lies flat enough for the {drive} to reach {to}."
                        ),
                        u,
                    ),
                    None => make(
                        "RELAY CROSSING",
                        format!(
                            "Crossing {star} to a jump point at its far edge, {au:.2} AU out: the {drive} hops on to {to}{}.",
                            if self.route.len() > 1 {
                                String::new()
                            } else {
                                ", the last hop".into()
                            }
                        ),
                        u,
                    ),
                }
            }
            Phase::Spool => {
                let (k, drive) = self.drive();
                let next = &self.next.star.name;
                let hop = if self.legs > 1 {
                    format!(" · hop {} of {}", self.leg, self.legs)
                } else {
                    String::new()
                };
                Manoeuvre::spool(
                    drive,
                    k,
                    u,
                    format!("{next} · {:.1} ly{hop}", self.next.distance),
                    next,
                    star,
                )
            }
            Phase::Rejump(k) => {
                let (n, drive) = self.drive();
                let next = &self.next.star.name;
                let stop = self.passage.stops[k as usize];
                let name = if matches!(stop.event, Event::HiveCube | Event::Battle) {
                    "Emergency jump"
                } else {
                    drive
                };
                Manoeuvre::spool(
                    name,
                    n,
                    u,
                    format!(
                        "{next} · {:.1} ly left · leg {} of {}",
                        self.ly_left(k as usize),
                        k + 2,
                        self.passage.legs.len()
                    ),
                    next,
                    "the deep",
                )
            }
            Phase::Stopover(k) => {
                let stop = self.passage.stops[k as usize];
                let (_, _, (name, text, progress)) =
                    voyage::hud(&stop, local, &self.next.star.name, self.ly_left(k as usize));
                make(&name, text, progress)
            }
            Phase::Jump(k) => {
                let leg = self.passage.legs[k as usize];
                let f = leg.flux.filter(|_| leg.flux_amount(local) > 0.05)?;
                let (name, text) = voyage::flux_hud(f);
                make(name, text.to_string(), leg.flux_amount(local))
            }
            _ => None,
        }
    }

    /// The scanner's readings at `t`: while orbiting a stop it surveys (one
    /// in ten), from the moment the camera settles into orbit; none otherwise.
    pub fn scans(&self, t: f64) -> Vec<super::scan::Scan> {
        let (segment, _) = self.segment(t);
        match segment.phase {
            Phase::High(p) if super::scan::surveyed(self.world(p)) => {
                let survey = super::scan::Survey::of(self.world(p), &self.system.star);
                survey.at(t - self.arrival(p))
            }
            _ => Vec::new(),
        }
    }

    /// How far through the survey of the world being orbited, 0..1 (0 when
    /// not orbiting).
    pub fn survey(&self, t: f64) -> f32 {
        let (segment, _) = self.segment(t);
        match segment.phase {
            Phase::High(p) if super::scan::surveyed(self.world(p)) => {
                super::scan::Survey::progress(t - self.arrival(p))
            }
            _ => 0.0,
        }
    }

    /// A stop's body in a line: its kind, size and what goes round it.
    fn describe(&self, stop: Stop) -> String {
        let p = self.world(stop);
        if stop.moon.is_some() {
            return format!(
                "{} · {:.1}× Earth · a giant's moon",
                p.kind.name(),
                p.radius
            );
        }
        let moons = match p.moons.len() {
            0 => String::new(),
            1 => " · 1 moon".to_string(),
            n => format!(" · {n} moons"),
        };
        let rings = if p.rings.is_some() { " · rings" } else { "" };
        format!("{} · {:.1}× Earth{moons}{rings}", p.kind.name(), p.radius)
    }

    /// The HUD at `t`: a title and lines of label and value.
    pub fn hud(&self, t: f64) -> (String, Vec<(String, String)>) {
        let (segment, local) = self.segment(t);
        let left = segment.duration - local;
        let view = self.view(t);
        let star = &self.system.star;
        match segment.phase {
            Phase::Transfer { to, .. } => {
                let k = self.index_of(to);
                let planet = self.world(to);
                let u = local / segment.duration;
                let (_, sigma) = self.on_path(k, segment, t);
                let remaining = self.paths[k].remaining(sigma);
                let mut lines = vec![
                    ("Destination".into(), planet.name.clone()),
                    ("".into(), self.describe(to)),
                    ("ETA".into(), clock(left)),
                ];
                let mut title = "TRANSFER".to_string();
                if to.moon.is_some() {
                    // A hop between a giant's moons: speeds around the giant.
                    let giant = &self.system.planets[to.planet];
                    let d = length(sub(view.position, giant.position(t)));
                    lines.push((
                        "Distance".into(),
                        format!("{} km", thousands(remaining * KM)),
                    ));
                    lines.push((
                        "Velocity".into(),
                        format!("{:.1} km/s around {}", giant.orbital_speed(d), giant.name),
                    ));
                    return ("MOON TRANSFER".into(), lines);
                }
                // A transfer orbit's speed around the star, from the vis-viva
                // equation between where it is and where it is going: it
                // climbs as the path dives past the star.
                let r = (length(view.position) / AU).max(0.002) * REAL_AU * KM;
                let a = (r + planet.orbit / AU * REAL_AU * KM) / 2.0;
                let gm = super::system::GM_SUN * star.mass;
                let v = (gm * (2.0 / r - 1.0 / a)).max(0.0).sqrt();
                lines.push(("Distance".into(), format!("{:.2} AU", remaining / AU)));
                lines.push(("Velocity".into(), format!("{v:.1} km/s")));
                if let Some((centre, radius, name)) = self.flyby_body(k, t) {
                    let d = length(sub(view.position, centre));
                    let closest = self.paths[k].closest;
                    let value = match self.paths[k].flyby {
                        Some(Flyby::Star) => format!("{name} · {:.1} star radii", d / radius),
                        _ => format!("{name} · {} km", thousands((d - radius).max(0.0) * KM)),
                    };
                    let value = if u < closest - 0.18 {
                        format!("{name} in {}", clock((closest - u) * segment.duration))
                    } else {
                        value
                    };
                    lines.push(("Flyby".into(), value));
                    if (u - closest).abs() < 0.18 {
                        title = match self.paths[k].flyby {
                            Some(Flyby::Star) => "STAR FLYBY".into(),
                            _ => "FLYBY".into(),
                        };
                    }
                }
                (title, lines)
            }
            Phase::High(p) => {
                let planet = self.world(p);
                let distance = length(sub(view.position, self.system.centre(p, t)));
                let altitude = (distance - planet.radius) * KM;
                let speed = planet.orbital_speed(distance);
                let title = "ORBIT";
                let departs = self
                    .segments
                    .iter()
                    .find(|s| {
                        s.start > t && matches!(s.phase, Phase::Transfer { .. } | Phase::Depart(_))
                    })
                    .map_or(0.0, |s| s.start - t);
                let next = self
                    .segments
                    .iter()
                    .find(|s| s.start > t && matches!(s.phase, Phase::Transfer { .. }))
                    .and_then(|s| match s.phase {
                        Phase::Transfer { to, .. } => Some(self.world(to).name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| format!("jump to {}", self.next.star.name));
                let moons = if p.moon.is_some() {
                    ("Orbits".into(), self.system.planets[p.planet].name.clone())
                } else if planet.moons.is_empty() {
                    ("Moons".into(), "none".into())
                } else {
                    (
                        "Moons".into(),
                        planet
                            .moons
                            .iter()
                            .map(|m| m.name.clone())
                            .collect::<Vec<_>>()
                            .join(", "),
                    )
                };
                (
                    title.into(),
                    vec![
                        ("Orbiting".into(), planet.name.clone()),
                        ("".into(), self.describe(p)),
                        ("Orbital speed".into(), format!("{speed:.2} km/s")),
                        (
                            "Altitude".into(),
                            format!("{} km", thousands(altitude.max(0.0))),
                        ),
                        moons,
                        ("Next".into(), format!("{next} in {}", clock(departs))),
                    ],
                )
            }
            Phase::Depart(_) => (
                "DEPARTURE".into(),
                vec![
                    ("Destination".into(), self.making_for()),
                    (
                        "Star".into(),
                        format!(
                            "{} K · {:.2} solar masses",
                            thousands(self.next.star.temperature),
                            self.next.star.mass
                        ),
                    ),
                    ("Distance".into(), format!("{:.1} ly", self.next.distance)),
                    ("Jump point in".into(), clock(left + CRUISE)),
                ],
            ),
            Phase::Cruise { from } => {
                let u = local / segment.duration;
                let remaining = self.cruise.remaining(u);
                let mut lines = vec![
                    (
                        "Jump point".into(),
                        format!("{:.2} AU · {}", remaining / AU, clock(left)),
                    ),
                    (
                        "Escape speed".into(),
                        format!(
                            "{:.1} km/s from {}",
                            self.escape_speed(view.position),
                            star.name
                        ),
                    ),
                    ("Next star".into(), self.making_for()),
                    ("Drive".into(), self.drive().1.to_string()),
                ];
                if self.legs > 1 {
                    lines.push((
                        "Route".into(),
                        format!(
                            "jump {} of {} to {}",
                            self.leg,
                            self.legs,
                            self.destination().star.name
                        ),
                    ));
                }
                let title = if from.is_some() {
                    "TO JUMP POINT"
                } else {
                    "RELAY"
                };
                (title.into(), lines)
            }
            Phase::Spool => {
                let manoeuvre = self.manoeuvre(t).unwrap_or_default();
                let (stage, through) = manoeuvre.stage();
                let stage = manoeuvre
                    .stages
                    .get(stage)
                    .map_or(String::new(), |s| s.0.clone());
                (
                    "DRIVE SPOOL".into(),
                    vec![
                        ("Drive".into(), self.drive().1.to_string()),
                        ("Stage".into(), format!("{stage} · {:.0}%", through * 100.0)),
                        ("Target".into(), self.making_for()),
                        (
                            "Distance".into(),
                            format!("{:.1} ly{}", self.next.distance, self.via()),
                        ),
                        ("Jump in".into(), clock(left)),
                    ],
                )
            }
            Phase::Jump(k) => {
                let k = k as usize;
                let leg = self.passage.legs[k];
                let years = self.next.distance;
                let before: f64 = self.passage.legs[..k].iter().map(|l| l.share).sum();
                let covered =
                    years * (before + leg.share * smoothstep(0.0, 1.0, local / segment.duration));
                // Light years per second of the leg, as a multiple of light speed.
                let c = years * leg.share * 365.25 * 86_400.0 / segment.duration;
                let eta = self.total - t;
                let mut lines = vec![
                    (
                        "Drive".into(),
                        voyage::LOOKS[leg.look as usize - 1].to_string(),
                    ),
                    ("Destination".into(), self.making_for()),
                    ("ETA".into(), clock(eta)),
                    ("Distance".into(), format!("{covered:.1} of {years:.1} ly")),
                    ("Speed".into(), format!("{:.1} million c", c / 1e6)),
                ];
                if self.passage.legs.len() > 1 {
                    let next = self.passage.stops.get(k).map_or_else(
                        || "drop-out".to_string(),
                        |s| match s.event {
                            Event::Gate | Event::Wormhole => voyage::name_of(s),
                            _ => "deep space".into(),
                        },
                    );
                    lines.push((
                        "Leg".into(),
                        format!("{} of {} · to {next}", k + 1, self.passage.legs.len()),
                    ));
                }
                if let Some(f) = leg.flux.filter(|_| leg.flux_amount(local) > 0.05) {
                    lines.push(("Alert".into(), voyage::flux_hud(f).0.to_string()));
                } else if self.legs > 1 {
                    lines.push((
                        "Route".into(),
                        format!(
                            "jump {} of {} to {}",
                            self.leg,
                            self.legs,
                            self.destination().star.name
                        ),
                    ));
                }
                ("HYPERJUMP".into(), lines)
            }
            Phase::Stopover(k) => {
                let stop = self.passage.stops[k as usize];
                let (title, lines, _) =
                    voyage::hud(&stop, local, &self.next.star.name, self.ly_left(k as usize));
                (title, lines)
            }
            Phase::Rejump(k) => {
                let manoeuvre = self.manoeuvre(t).unwrap_or_default();
                let (stage, through) = manoeuvre.stage();
                let stage = manoeuvre
                    .stages
                    .get(stage)
                    .map_or(String::new(), |s| s.0.clone());
                (
                    "DRIVE SPOOL".into(),
                    vec![
                        ("Drive".into(), self.drive().1.to_string()),
                        ("Stage".into(), format!("{stage} · {:.0}%", through * 100.0)),
                        ("Target".into(), self.making_for()),
                        (
                            "Remaining".into(),
                            format!("{:.1} ly", self.ly_left(k as usize)),
                        ),
                        (
                            "Leg".into(),
                            format!("{} of {}", k + 2, self.passage.legs.len()),
                        ),
                        ("Jump in".into(), clock(left)),
                    ],
                )
            }
        }
    }

    /// Light years still to go after leg `k` of the jump.
    fn ly_left(&self, k: usize) -> f64 {
        let done: f64 = self.passage.legs[..=k].iter().map(|l| l.share).sum();
        self.next.distance * (1.0 - done)
    }
}

/// How many star radii a star flyby passes at, from the system's seed.
fn rng_reach(seed: u64) -> f64 {
    super::system::Rng::new(seed ^ 0xf1b7).range(4.5, 6.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tour() -> Tour {
        Tour::new(System::generate(3), System::generate(4))
    }

    #[test]
    fn numbers_read_easily() {
        assert_eq!(thousands(182_930.4), "182,930");
        assert_eq!(thousands(286.0), "286");
        assert_eq!(clock(70.2), "1:10");
    }

    #[test]
    fn the_tour_takes_several_minutes() {
        let tour = tour();
        assert!(
            tour.total > 6.0 * 60.0 && tour.total < 14.0 * 60.0,
            "{}",
            tour.total
        );
    }

    #[test]
    fn the_camera_never_jumps_or_hits_a_planet() {
        for seed in 0..40 {
            camera_stays_clear(Tour::new(
                System::generate(seed),
                System::generate(seed + 1),
            ));
        }
    }

    /// The body the camera moves against at `t`, as a viewer sees it: the
    /// nearest, in its own radii, of the stop made for, the one left and
    /// what a transfer flies past. Its centre, as a function of time.
    fn frame_body(tour: &Tour, t: f64) -> Option<(usize, Box<dyn Fn(f64) -> [f64; 3] + '_>)> {
        let (segment, _) = tour.segment(t);
        let at = tour.view(t).position;
        let mut bodies: Vec<(usize, f64, Box<dyn Fn(f64) -> [f64; 3] + '_>)> = Vec::new();
        let stop_body = |stop: Stop| {
            let radius = tour.world(stop).radius;
            (
                stop.key() as usize,
                radius,
                Box::new(move |t: f64| tour.system.centre(stop, t)) as Box<dyn Fn(f64) -> [f64; 3]>,
            )
        };
        match segment.phase {
            Phase::Transfer { from, to } => {
                bodies.push(stop_body(to));
                if let Some(f) = from {
                    bodies.push(stop_body(f));
                }
                let k = tour.index_of(to);
                if let Some(Flyby::Planet(i)) = tour.paths[k].flyby {
                    let planet = &tour.system.planets[i];
                    bodies.push((
                        1000 + i,
                        planet.radius,
                        Box::new(move |t: f64| planet.position(t)),
                    ));
                }
            }
            Phase::Jump(_) | Phase::Stopover(_) | Phase::Rejump(_) => return None,
            _ => bodies.push(stop_body(tour.current(t)?)),
        }
        bodies
            .into_iter()
            .min_by(|a, b| {
                let d = |c: &(usize, f64, Box<dyn Fn(f64) -> [f64; 3] + '_>)| {
                    length(sub(at, (c.2)(t))) / c.1
                };
                d(a).total_cmp(&d(b))
            })
            .map(|(id, _, centre)| (id, centre))
    }

    #[test]
    fn the_camera_never_stops_or_turns_back() {
        let h = 1.0 / 30.0;
        let systems =
            std::env::var("SOLAR_TEST_SYSTEMS").map_or(40, |v| v.parse().expect("a count"));
        let mut turns = Vec::new();
        for seed in 0..systems {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let mut t = h;
            let mut last: Option<(usize, [f64; 3])> = None;
            while t < tour.total - JUMP - h {
                let Some((id, centre)) = frame_body(&tour, t) else {
                    break;
                };
                let rel = |t: f64| sub(tour.view(t).position, centre(t));
                let v = sub(rel(t + h), rel(t - h));
                assert!(length(v) > 1e-6, "seed {seed}: stopped at {t:.2}");
                if let Some((before, w)) = last
                    && before == id
                    && dot(normalize(v), normalize(w)) <= 0.0
                {
                    let (segment, local) = tour.segment(t);
                    let k = tour.current(t).map_or(0, |s| tour.index_of(s));
                    turns.push(format!(
                        "seed {seed} at {t:.2}, {:.3} through {:?} (flyby {:?})",
                        local / segment.duration,
                        segment.phase,
                        tour.paths[k].flyby
                    ));
                }
                last = Some((id, v));
                t += h;
            }
        }
        assert!(turns.is_empty(), "turned back:\n{}", turns.join("\n"));
    }

    #[test]
    fn orbits_are_entered_the_way_the_transfer_arrives() {
        for seed in 0..60 {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            for k in 0..tour.itinerary.len() {
                assert!(
                    !tour.arrives_backwards(k),
                    "seed {seed}: stop {k}'s orbit runs against its arrival"
                );
            }
        }
    }

    fn camera_stays_clear(tour: Tour) {
        let mut last = tour.view(0.0).position;
        let mut t = 0.0;
        while t < tour.total - JUMP {
            let view = tour.view(t);
            for (i, planet) in tour.system.planets.iter().enumerate() {
                let d = length(sub(view.position, planet.position(t)));
                assert!(d > planet.radius * 1.05, "inside planet {i} at {t}: {d}");
                for m in 0..planet.moons.len().min(crate::solar::stream::MOONS as usize) {
                    let stop = Stop {
                        planet: i,
                        moon: Some(m),
                    };
                    let d = length(sub(view.position, tour.system.centre(stop, t)));
                    assert!(
                        d > planet.moons[m].radius * 1.05,
                        "inside moon {m} of planet {i} at {t}: {d} (seed {})",
                        tour.system.seed
                    );
                }
            }
            let star = length(view.position);
            assert!(
                star > tour.system.star.radius * 1.5,
                "inside the star at {t}: {star}"
            );
            // Never more than a fraction of the way between planets in one frame.
            let step = length(sub(view.position, last));
            let extent = tour.system.planets.last().map_or(0.0, |p| p.orbit);
            assert!(step < 3000.0 + extent * 0.004, "a jump of {step} at {t}");
            assert!((length(view.forward) - 1.0).abs() < 1e-6);
            last = view.position;
            t += 1.0 / 30.0;
        }
    }

    #[test]
    fn the_hud_always_names_a_destination() {
        let tour = tour();
        let mut t = 0.0;
        while t < tour.total {
            let (title, lines) = tour.hud(t);
            assert!(!title.is_empty());
            assert!(lines.len() >= 3);
            t += 7.0;
        }
    }
}

#[cfg(test)]
mod flyby_tests {
    use super::*;

    #[test]
    fn every_system_flies_past_its_star_and_some_visit_big_moons() {
        let mut worlds = 0;
        let mut planet_flybys = 0;
        for seed in 0..40 {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            assert!(
                tour.paths.iter().any(|p| p.flyby == Some(Flyby::Star)),
                "seed {seed}"
            );
            planet_flybys += tour
                .paths
                .iter()
                .filter(|p| matches!(p.flyby, Some(Flyby::Planet(_))))
                .count();
            worlds += tour.itinerary.iter().filter(|s| s.moon.is_some()).count();
            // The star flyby passes close: within ten star radii.
            for (k, path) in tour.paths.iter().enumerate() {
                if path.flyby != Some(Flyby::Star) {
                    continue;
                }
                let segment = tour
                    .segments
                    .iter()
                    .find(|s| matches!(s.phase, Phase::Transfer { to, .. } if to == tour.itinerary[k]))
                    .copied()
                    .unwrap();
                let nearest = (0..200)
                    .map(|i| {
                        let t = segment.start + segment.duration * i as f64 / 200.0;
                        length(tour.view(t).position)
                    })
                    .fold(f64::MAX, f64::min);
                let r = tour.system.star.radius;
                assert!(
                    nearest < (r * 10.0).max(20.0),
                    "seed {seed}: {nearest} vs {r}"
                );
            }
        }
        assert!(worlds >= 8, "{worlds}");
        assert!(planet_flybys >= 3, "{planet_flybys}");
    }
}

#[cfg(test)]
mod manoeuvre_tests {
    use super::*;

    #[test]
    fn transfers_explain_their_manoeuvres() {
        let mut names = std::collections::BTreeSet::new();
        for seed in 0..40 {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let mut t = 0.0;
            while t < tour.total {
                let (segment, _) = tour.segment(t);
                let m = tour.manoeuvre(t);
                match segment.phase {
                    Phase::Transfer { .. }
                    | Phase::Depart(_)
                    | Phase::Cruise { .. }
                    | Phase::Spool => {
                        let m = m.expect("a manoeuvre while travelling");
                        // A coast says why in a sentence, a burn in its stages.
                        let said = if m.dv > 0.0 {
                            m.stages
                                .iter()
                                .map(|(name, what)| format!("{name}: {what}"))
                                .collect::<Vec<_>>()
                                .join(" / ")
                        } else {
                            m.text.clone()
                        };
                        if std::env::var("SOLAR_PRINT").is_ok_and(|v| v == seed.to_string())
                            && names.insert(format!("{seed}{}{said}", m.name))
                        {
                            println!("{t:6.1} {}: {said}", m.name);
                        }
                        assert!(!said.is_empty() && !said.contains("NaN"), "{m:?}");
                        assert!(
                            m.dv <= 0.0 || m.stages.iter().all(|(_, what)| !what.is_empty()),
                            "{m:?}"
                        );
                        names.insert(m.name);
                    }
                    // Between the stars: a stopover's panel, or an event in
                    // a leg's.
                    phase if phase.deep() => {
                        if let Some(m) = m {
                            assert!(
                                !m.name.is_empty() && !m.text.is_empty() && !m.text.contains("NaN"),
                                "{m:?}"
                            );
                        }
                    }
                    _ => assert!(m.is_none()),
                }
                t += 0.5;
            }
        }
        for name in [
            "DEPARTURE BURN",
            "CAPTURE BURN",
            "GRAVITY ASSIST",
            "OBERTH MANOEUVRE",
            "HOHMANN TRANSFER",
            "MOON TRANSFER",
            "ESCAPE BURN",
            "TO THE JUMP POINT",
        ] {
            assert!(names.contains(name), "{name} never happens: {names:?}");
        }
    }
}

#[cfg(test)]
mod jump_point_tests {
    use super::*;

    /// A relay tour of `seed`, hopping on to `seed + 1`.
    fn relay(seed: u64) -> Tour {
        {
            let next = System::generate(seed + 1);
            let passage = Passage::plan(seed, 1, drive_to(&next).0);
            Tour::plan(
                System::generate(seed),
                vec![next, System::generate(seed + 2)],
                true,
                2,
                3,
                passage,
                1,
            )
        }
    }

    #[test]
    fn every_jump_leaves_from_the_edge_toward_its_star() {
        for seed in 0..40 {
            for tour in [
                Tour::new(System::generate(seed), System::generate(seed + 1)),
                relay(seed),
            ] {
                let outer = tour.system.planets.last().map_or(0.0, |p| p.orbit);
                let spool = tour.segment_of(|p| p == Phase::Spool);
                // At the jump point, beyond the outermost orbit.
                let at = tour.view(spool.start).position;
                assert!(
                    length(at) > outer * 1.2,
                    "seed {seed}: {} inside {outer}",
                    length(at)
                );
                // Aligned on the next star once the first stage is done, and
                // its mark shown.
                let aligned = tour.view(spool.start + SPOOL * SPOOL_STAGES[1] + 0.1);
                assert!(dot(aligned.forward, tour.exit) > 0.999, "seed {seed}");
                assert_eq!(aligned.target_shown, 1.0);
                // The drive's light rises over the engagement, into the jump.
                assert_eq!(tour.view(spool.start + 1.0).warp, 0.0);
                let end = tour.view(spool.start + SPOOL - 1e-3);
                assert!(end.warp > 0.2 && end.spool > 0.99 && end.spool_light > 0.99);
            }
        }
    }

    #[test]
    fn relays_are_crossed_without_stops() {
        for seed in 0..40 {
            let tour = relay(seed);
            assert!(tour.itinerary.is_empty());
            assert_eq!(tour.next_first(), None);
            assert!(tour.total > CROSSING + SPOOL + JUMP - 1e-6);
            // It drops out at the start, over the relay's edge.
            assert!(tour.view(0.0).reveal < 0.01);
            let (title, lines) = tour.hud(CROSSING * 0.5);
            assert_eq!(title, "RELAY");
            assert!(
                lines.iter().any(|(_, v)| v.contains("jump 2 of 3")),
                "{lines:?}"
            );
            camera_moves_smoothly(&tour, 0.0);
        }
    }

    /// From `from` to the jump, the camera clear of every body, never
    /// jerked and never stopped.
    fn camera_moves_smoothly(tour: &Tour, from: f64) {
        let h = 1.0 / 30.0;
        let end = tour.segment_of(|p| p == Phase::Jump(0)).start + 2.0;
        let mut t = from + h;
        let at = |t: f64| tour.view(t).position;
        while t < end {
            let v0 = scale(sub(at(t), at(t - h)), 1.0 / h);
            let v1 = scale(sub(at(t + h), at(t)), 1.0 / h);
            let speed = length(v0).max(length(v1));
            assert!(
                length(v1) > 1e-3,
                "seed {}: stopped at {t:.2}",
                tour.system.seed
            );
            // No kick bigger than a fifth of the speed in a frame, or than
            // the surge into the jump gives.
            let kick = length(sub(v1, v0));
            assert!(
                kick < 0.2 * speed + JUMP_SPEED * 0.05,
                "seed {}: a kick of {kick} at {t:.2} ({:?}, speed {speed})",
                tour.system.seed,
                tour.segment(t).0.phase
            );
            for planet in &tour.system.planets {
                assert!(length(sub(at(t), planet.position(t))) > planet.radius * 1.05);
            }
            assert!(length(at(t)) > tour.system.star.radius * 1.5);
            t += h;
        }
    }

    #[test]
    fn the_flight_out_carries_on_from_the_last_orbit() {
        for seed in 0..40 {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let depart = tour.segment_of(|p| matches!(p, Phase::Depart(_)));
            camera_moves_smoothly(&tour, depart.start);
        }
    }

    #[test]
    fn every_drive_spools_up_in_its_own_stages() {
        let mut names = std::collections::BTreeSet::new();
        for drive in 1..=DRIVES.len() as u32 {
            let m = Manoeuvre::spool(
                DRIVES[drive as usize - 1],
                drive,
                0.5,
                "x".into(),
                "Vega",
                "Sol",
            )
            .unwrap();
            assert_eq!(m.stages.len(), SPOOL_STAGES.len());
            assert_eq!(m.stages[0].0, "ALIGN");
            assert_eq!(m.throttle(), 0.0, "a drive is no burn");
            assert_eq!(m.stage().0, 2);
            for (name, what) in &m.stages[1..] {
                assert!(!what.is_empty());
                names.insert(name.clone());
            }
        }
        // Four stages of each drive its own.
        assert_eq!(names.len(), 4 * DRIVES.len(), "{names:?}");
    }
}
