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
//! visited, and slows as it passes. After the last stop it turns toward the
//! next star and jumps: the jump is the visit's last segment, and the next
//! visit begins where it ends. Ten to twelve minutes in all.
//!
//! Everything here is a pure function of the time into the visit, so any
//! moment can be shown on its own (the previews do) and nothing drifts.

use std::f64::consts::{PI, TAU};

use super::system::{AU, KM, Planet, REAL_AU, Stop, System};

/// Seconds each part of the tour takes.
pub const TRANSFER: f64 = 45.0;
/// A transfer that swings past the star or another planet on the way.
pub const FLYBY: f64 = 64.0;
/// The short hop from a giant out to its moon.
pub const HOP: f64 = 32.0;
/// The orbit, unhurried: the scanner's survey (`scan::SCHEDULE`) takes its
/// first half.
pub const HIGH: f64 = 70.0;
pub const DEPART: f64 = 20.0;
pub const JUMP: f64 = 150.0;
/// The jump's last seconds, in which the drive slows (`drive_clock`).
const SLOWING: f64 = 4.0;
/// The last second of the jump, in which a flare builds at the heading.
const FLARE: f64 = 1.0;
/// How long the drive's light takes to tear open on arrival, over the new
/// system (`View::reveal`).
pub const DROP_OUT: f64 = 2.4;
/// The hyperjump drives, each its own look (`fs_warp` in `frame.wgsl`).
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
    Jump,
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
}

/// The drive used for the jump to `system`: its number and name.
fn drive_to(system: &System) -> (u32, &'static str) {
    let k = (super::system::Rng::new(system.seed ^ 0xd21e).next() % DRIVES.len() as u64) as usize;
    (k as u32 + 1, DRIVES[k])
}

/// The drive's clock `local` seconds into a jump: running at full speed,
/// then slowing to 0.4 of it over the last `SLOWING` seconds.
fn drive_clock(local: f64) -> f64 {
    let x = ((local - (JUMP - SLOWING)) / SLOWING).clamp(0.0, 1.0);
    local.min(JUMP - SLOWING) + SLOWING * (x - 0.3 * x * x)
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
    /// Where the visit drops out of the jump, and which way the jump leaves.
    entry: [f64; 3],
    exit: [f64; 3],
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
}

/// Where each of a burn's stages begins, as a share of it: turning to the
/// burn's attitude, lighting the engine, the burn itself, and the cut-off.
const STAGES: [f64; 4] = [0.0, 0.08, 0.16, 0.9];

impl Manoeuvre {
    /// A burn of `dv` km/s, `progress` of the way through, in its stages:
    /// the attitude it is flown in, what the burn does and what it leaves.
    fn burn(name: &str, progress: f64, dv: f64, attitude: &str, burn: String, cutoff: String) -> Option<Self> {
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
        })
    }

    /// What the ship does with the engine off, and why.
    fn coast(name: &str, text: String, progress: f64) -> Option<Self> {
        Some(Manoeuvre {
            name: name.into(),
            text,
            progress: progress as f32,
            dv: 0.0,
            stages: Vec::new(),
        })
    }

    /// How hard the engine is firing, 0..1: off while the ship turns to the
    /// burn's attitude, run up as it lights, cut at the end.
    pub fn throttle(&self) -> f32 {
        if self.dv <= 0.0 || self.progress < 0.0 {
            return 0.0;
        }
        let p = self.progress as f64;
        (smoothstep(STAGES[1], STAGES[2], p) * (1.0 - smoothstep(STAGES[3], STAGES[3] + 0.06, p))) as f32
    }

    /// The stage under way, and how far through it.
    pub fn stage(&self) -> (usize, f32) {
        let p = (self.progress as f64).clamp(0.0, 1.0);
        let i = STAGES.iter().rposition(|&s| p >= s).unwrap_or(0);
        let end = STAGES.get(i + 1).copied().unwrap_or(1.0);
        (i, ((p - STAGES[i]) / (end - STAGES[i])).clamp(0.0, 1.0) as f32)
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
    (dot(v, e2).atan2(dot(v, e1)), dot(v, up).clamp(-1.0, 1.0).asin())
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
            if l > 1.5 * span { scale(m, 1.5 * span / l) } else { m }
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
                j if j <= end => tail + (span - 2.0 * tail) * (j - head) as f64 / (end - head) as f64,
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
        let flown: f64 = ds.iter().zip(&near).map(|(d, g)| d * (1.0 + boost * g)).sum();
        let share = |close: &[f64], share: f64| {
            let nearing: f64 = ds.iter().zip(close).map(|(d, g)| d * g).sum();
            if nearing > 1e-9 {
                share / (1.0 - APPROACH - LEAVE) * flown / nearing
            } else {
                0.0
            }
        };
        let (approach, leaving) = (share(&close_in, APPROACH), share(&close_out, LEAVE));
        let weight = |j: usize| 1.0 + boost * near[j] + approach * close_in[j] + leaving * close_out[j];
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
        let j = self.places.partition_point(|&s| s <= sigma).saturating_sub(1).min(SAMPLES);
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
    [r * angle.cos(), a[1] + (b[1] - a[1]) * ease, r * angle.sin()]
}

/// `n + 1` knots along the arc from `a` to `b`, crowded toward its end or
/// start for a tight bend there.
fn arc_knots(a: [f64; 3], b: [f64; 3], n: usize, dense_end: bool, dense_start: bool) -> Vec<[f64; 3]> {
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

impl Tour {
    pub fn new(system: System, next: System) -> Self {
        let itinerary = system.itinerary();
        let mut rng = super::system::Rng::new(system.seed ^ 0x0b17);

        // Drop out above the plane of the system, beyond the first stop,
        // looking in toward the star.
        let first = &system.planets[itinerary[0].planet];
        let out = normalize(add(normalize(first.position(0.0)), [0.0, 0.35, 0.0]));
        let entry = scale(out, first.orbit * 1.6 + 400.0);
        let mut exit = normalize([
            rng.range(-1.0, 1.0),
            rng.range(0.2, 0.6),
            rng.range(-1.0, 1.0),
        ]);

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
        push(Phase::Jump, JUMP, &mut t);

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
        // Leave a moon's system away from its giant.
        if last.moon.is_some() {
            let depart = segments[segments.len() - 2].start;
            let giant = normalize(sub(
                system.planets[last.planet].position(depart),
                system.centre(last, depart),
            ));
            let toward = dot(exit, giant) + 0.5;
            if toward > 0.0 {
                exit = normalize(sub(exit, scale(giant, 2.0 * toward)));
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
        tour
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
                let ends = [k.checked_sub(1).map(|j| itinerary[j].planet), Some(stop.planet)];
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
        add(self.orbiting(stop, t).0, scale(normalize(add(out, self.exit)), distance * s * s))
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
        normalize(sub(rel(t1 - 0.1 * segment.duration), rel(t1 - 0.25 * segment.duration)))
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
        let u = if length(flat) > 1e-6 { normalize(flat) } else { orbit.u };
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

    /// The drive the jump that arrived here used: the last visit's.
    pub fn arrival_drive(&self) -> (u32, &'static str) {
        drive_to(&self.system)
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
        self.itinerary
            .iter()
            .position(|&s| s == stop)
            .unwrap_or(0)
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
            obstacles.push((centre, p.radius * if p.kind.is_giant() { 2.8 } else { 2.0 }, false));
            for m in 0..p.moons.len().min(super::stream::MOONS as usize) {
                let stop = Stop {
                    planet: i,
                    moon: Some(m),
                };
                obstacles.push((self.system.centre(stop, t), p.moons[m].radius * MOON_FLOOR, true));
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
                let drift = sub(self.system.centre(stop, t), self.system.centre(stop, placed));
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
            let drift = sub(self.system.centre(stop, t), self.system.centre(stop, placed));
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
                let push = if n == 3 { now } else { self.keep_away(k, segment, when, Some(true)) };
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
                    sub(self.system.centre(stop, t), self.system.centre(stop, placed)),
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
            Flyby::Star => Some(([0.0; 3], self.system.star.radius, self.system.star.name.clone())),
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
        let (position, forward, up) = match segment.phase {
            Phase::High(p) => {
                self.orbiting(p, t)
            }
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
                    heading_change(start, normalize(sub(self.system.centre(to, at), there)), up0)
                };
                let mut forward = swing(start, aim, smoothstep(0.0, 0.18, u), up0, way);
                forward = normalize(mix(forward, self.framed(to, position, t, t1), smoothstep(0.82, 1.0, u)));
                let up = normalize(mix(up0, up1, smoothstep(0.1, 0.9, u)));
                if from.is_none() && local < DROP_OUT {
                    // Dropping out: the jump's light tears open over the new
                    // system, its drive slowing on from 0.4 to 0.15.
                    let p = local / DROP_OUT;
                    reveal = p;
                    flare = (1.0 - p).powi(3);
                    clock = drive_clock(JUMP) + DROP_OUT * (0.4 * p - 0.125 * p * p);
                }
                (position, forward, up)
            }
            Phase::Depart(p) => {
                // Still going round as it climbs out, so it never stops.
                let (_, f0, up0) = self.orbiting(p, t);
                let s = smootherstep(u);
                let position = self.departing(p, segment.start, t);
                let forward = normalize(mix(f0, self.exit, smoothstep(0.0, 0.8, u)));
                let up = normalize(mix(up0, [0.0, 1.0, 0.0], s));
                warp = smoothstep(0.85, 1.0, u) * 0.25;
                (position, forward, up)
            }
            Phase::Jump => {
                // Out beyond the departure point, heading for the next star.
                let last = *self.itinerary.last().expect("a tour has stops");
                let start = self.departing(last, segment.start - DEPART, segment.start);
                let position = add(start, scale(self.exit, local * 400.0));
                // At full light to the end: the next visit takes it over
                // (`reveal`), slowing, the flare at its brightest.
                warp = 0.25 + 0.75 * smoothstep(0.0, 0.07, u);
                flare = smoothstep(segment.duration - FLARE, segment.duration, local);
                clock = drive_clock(local);
                (position, self.exit, [0.0, 1.0, 0.0])
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
            Phase::Transfer { from: Some(from), to } if self.in_sight(from, t) => {
                (Some(to), Some(from))
            }
            Phase::Transfer { to, .. } => (Some(to), second(to)),
            Phase::High(p) => {
                (Some(p), second(p))
            }
            Phase::Depart(p) => (
                Some(p),
                p.moon.map(|_| Stop::planet(p.planet)),
            ),
            Phase::Jump => (None, None),
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
        let Phase::Transfer { to, .. } = segment.phase else {
            return Vec::new();
        };
        let k = self.index_of(to);
        let steps = 48;
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
            Phase::Jump => None,
        }
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
                        format!("Out of the jump and falling sunward toward {dest}, engines quiet."),
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
                                    if burns.assist_dv >= 0.0 { "lent" } else { "took from" }
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
                                format!("+{:.1} km/s while moving at {:.0} km/s", burns.oberth_burn, burns.oberth_speed),
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
                let r = length(sub(self.view(t).position, self.system.centre(p, t))).max(world.radius) * KM;
                // From orbital speed to escape speed, at the orbit left.
                let r0 = Self::height(world) * KM;
                fire(
                    "ESCAPE BURN",
                    u,
                    (2.0f64.sqrt() - 1.0) * (mu / r0).sqrt(),
                    "prograde",
                    format!("past escape velocity, {:.1} km/s here", (2.0 * mu / r).sqrt()),
                    format!("clear of {}'s gravity, ready to jump", world.name),
                )
            }
            _ => None,
        }
    }

    /// The scanner's readings at `t`: while orbiting a stop, from the moment
    /// the camera settles into orbit; none otherwise.
    pub fn scans(&self, t: f64) -> Vec<super::scan::Scan> {
        let (segment, _) = self.segment(t);
        match segment.phase {
            Phase::High(p) => {
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
            Phase::High(p) => super::scan::Survey::progress(t - self.arrival(p)),
            _ => 0.0,
        }
    }

    /// A stop's body in a line: its kind, size and what goes round it.
    fn describe(&self, stop: Stop) -> String {
        let p = self.world(stop);
        if stop.moon.is_some() {
            return format!("{} · {:.1}× Earth · a giant's moon", p.kind.name(), p.radius);
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
                        format!(
                            "{name} in {}",
                            clock((closest - u) * segment.duration)
                        )
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
                    ("Drive".into(), format!("{} spooling up", self.drive().1)),
                    (
                        "Destination".into(),
                        format!("{} ({})", self.next.star.name, self.next.star.title()),
                    ),
                    (
                        "Star".into(),
                        format!(
                            "{} K · {:.2} solar masses",
                            thousands(self.next.star.temperature),
                            self.next.star.mass
                        ),
                    ),
                    ("Distance".into(), format!("{:.1} ly", self.next.distance)),
                    ("Jump in".into(), clock(left)),
                ],
            ),
            Phase::Jump => {
                let years = self.next.distance;
                let covered = years * smoothstep(0.0, 1.0, local / segment.duration);
                // Light years per second of jump, as a multiple of light speed.
                let c = years * 365.25 * 86_400.0 / segment.duration;
                (
                    "HYPERJUMP".into(),
                    vec![
                        ("Drive".into(), self.drive().1.to_string()),
                        (
                            "Destination".into(),
                            format!("{} ({})", self.next.star.name, self.next.star.title()),
                        ),
                        ("ETA".into(), clock(left)),
                        (
                            "Distance".into(),
                            format!("{:.1} of {years:.1} ly", covered),
                        ),
                        ("Speed".into(), format!("{:.1} million c", c / 1e6)),
                    ],
                )
            }
        }
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
            (stop.key() as usize, radius, Box::new(move |t: f64| tour.system.centre(stop, t)) as Box<dyn Fn(f64) -> [f64; 3]>)
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
                    bodies.push((1000 + i, planet.radius, Box::new(move |t: f64| planet.position(t))));
                }
            }
            Phase::Jump => return None,
            _ => bodies.push(stop_body(tour.current(t)?)),
        }
        bodies
            .into_iter()
            .min_by(|a, b| {
                let d = |c: &(usize, f64, Box<dyn Fn(f64) -> [f64; 3] + '_>)| length(sub(at, (c.2)(t))) / c.1;
                d(a).total_cmp(&d(b))
            })
            .map(|(id, _, centre)| (id, centre))
    }

    #[test]
    fn the_camera_never_stops_or_turns_back() {
        let h = 1.0 / 30.0;
        let systems = std::env::var("SOLAR_TEST_SYSTEMS").map_or(40, |v| v.parse().expect("a count"));
        let mut turns = Vec::new();
        for seed in 0..systems {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let mut t = h;
            let mut last: Option<(usize, [f64; 3])> = None;
            while t < tour.total - JUMP - h {
                let Some((id, centre)) = frame_body(&tour, t) else { break };
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
                    let stop = Stop { planet: i, moon: Some(m) };
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
            assert!(tour.paths.iter().any(|p| p.flyby == Some(Flyby::Star)), "seed {seed}");
            planet_flybys += tour.paths.iter().filter(|p| matches!(p.flyby, Some(Flyby::Planet(_)))).count();
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
                assert!(nearest < (r * 10.0).max(20.0), "seed {seed}: {nearest} vs {r}");
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
                    Phase::Transfer { .. } | Phase::Depart(_) => {
                        let m = m.expect("a manoeuvre while travelling");
                        // A coast says why in a sentence, a burn in its stages.
                        let said = if m.dv > 0.0 {
                            m.stages.iter().map(|(name, what)| format!("{name}: {what}")).collect::<Vec<_>>().join(" / ")
                        } else {
                            m.text.clone()
                        };
                        if std::env::var("SOLAR_PRINT").is_ok_and(|v| v == seed.to_string())
                            && names.insert(format!("{seed}{}{said}", m.name))
                        {
                            println!("{t:6.1} {}: {said}", m.name);
                        }
                        assert!(!said.is_empty() && !said.contains("NaN"), "{m:?}");
                        assert!(m.dv <= 0.0 || m.stages.iter().all(|(_, what)| !what.is_empty()), "{m:?}");
                        names.insert(m.name);
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
        ] {
            assert!(names.contains(name), "{name} never happens: {names:?}");
        }
    }
}
