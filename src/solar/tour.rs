//! The grand tour of one system, as a timeline: where the camera is and what
//! the HUD says at any moment of it.
//!
//! A visit drops out of the previous hyperjump outside the system, crosses to
//! the first planet on the itinerary, and for each planet settles into a high
//! orbit with its moons and rings in view, dives to a low orbit over the
//! horizon, climbs back out and crosses to the next. A giant with a moon the
//! size of a planet is visited last, and the moon after it, a short hop out.
//! Crossings are not straight: each follows a spline through the closest
//! point of a flyby, swinging round the star or past a planet not otherwise
//! visited, and slows as it passes. After the last stop it turns toward the
//! next star and jumps: the jump is the visit's last segment, and the next
//! visit begins where it ends. Ten to twelve minutes in all.
//!
//! Everything here is a pure function of the time into the visit, so any
//! moment can be shown on its own (the previews do) and nothing drifts.

use super::system::{AU, KM, Planet, REAL_AU, Stop, System};

/// Seconds each part of the tour takes.
pub const TRANSFER: f64 = 45.0;
/// A transfer that swings past the star or another planet on the way.
pub const FLYBY: f64 = 64.0;
/// The short hop from a giant out to its moon.
pub const HOP: f64 = 32.0;
pub const HIGH: f64 = 35.0;
pub const DESCEND: f64 = 12.0;
pub const LOW: f64 = 28.0;
pub const CLIMB: f64 = 12.0;
pub const DEPART: f64 = 20.0;
pub const JUMP: f64 = 150.0;
/// The hyperjump drives, each its own look (`fs_warp` in `frame.wgsl`).
pub const DRIVES: [&str; 5] = [
    "Lumen Drift",
    "Tachyon Tunnel",
    "Fold Drive",
    "Ion Slipstream",
    "Graviton Spiral",
];

/// Field of view, the tangent of half of it.
const HALF_FOV: f64 = 0.4663; // 50°
/// Points along a transfer path its timing is worked out at.
const SAMPLES: usize = 400;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Crossing to stop `to`: from the drop-out point, or from another stop.
    Transfer {
        from: Option<Stop>,
        to: Stop,
    },
    High(Stop),
    Descend(Stop),
    Low(Stop),
    Climb(Stop),
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
    /// A flash as the jump drops out, 0..1.
    pub flash: f64,
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
    /// The share of the transfer's (eased) time spent by each sample along
    /// the spline, and the distance covered, both cumulative.
    time: Vec<f64>,
    length: Vec<f64>,
    flyby: Option<Flyby>,
    /// When, as a share of the transfer, it passes closest.
    closest: f64,
}

#[derive(Clone, Copy, Debug)]
enum Anchor {
    Fixed,
    Stop(Stop),
}

// Small vector helpers, in f64.
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
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
fn unsmootherstep(y: f64) -> f64 {
    let (mut lo, mut hi) = (0.0, 1.0);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        if smootherstep(mid) < y {
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
        hermite(
            self.knots[k],
            self.tangents[k],
            self.knots[k + 1],
            self.tangents[k + 1],
            s,
        )
    }

    /// Work out the timing: how much of the transfer each stretch takes,
    /// slower near `slow` (a point and a radius) if there is one.
    fn time(&mut self, slow: Option<([f64; 3], f64)>) {
        let span = (self.knots.len() - 1) as f64;
        let mut time = vec![0.0];
        let mut length = vec![0.0];
        let mut last = self.spline(0.0);
        for j in 1..=SAMPLES {
            let p = self.spline(span * j as f64 / SAMPLES as f64);
            let ds = length_of(sub(p, last));
            let mid = mix(p, last, 0.5);
            let weight = match slow {
                Some((centre, r)) => {
                    let d = length_of(sub(mid, centre)) / (r * 2.5);
                    1.0 + 4.0 * (-d * d).exp()
                }
                None => 1.0,
            };
            time.push(time[j - 1] + ds * weight);
            length.push(length[j - 1] + ds);
            last = p;
        }
        let total = time[SAMPLES].max(1e-9);
        for t in &mut time {
            *t /= total;
        }
        self.time = time;
        self.length = length;
        self.closest = if self.knots.len() > 2 {
            unsmootherstep(self.time[SAMPLES / 2])
        } else {
            0.5
        };
    }

    /// Where along the spline (`sigma`) the camera is, a share `u` of the
    /// way through the transfer.
    fn sigma(&self, u: f64) -> f64 {
        let tau = smootherstep(u);
        let j = self.time.partition_point(|&t| t < tau).clamp(1, SAMPLES);
        let (a, b) = (self.time[j - 1], self.time[j]);
        let f = if b > a { (tau - a) / (b - a) } else { 0.0 };
        (j as f64 - 1.0 + f) / SAMPLES as f64 * (self.knots.len() - 1) as f64
    }

    /// Distance along the path still to go from `sigma`, in units.
    fn remaining(&self, sigma: f64) -> f64 {
        let span = (self.knots.len() - 1) as f64;
        let j = ((sigma / span * SAMPLES as f64) as usize).min(SAMPLES);
        self.length[SAMPLES] - self.length[j]
    }
}

fn length_of(a: [f64; 3]) -> f64 {
    length(a)
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
            push(Phase::Descend(stop), DESCEND, &mut t);
            push(Phase::Low(stop), LOW, &mut t);
            push(Phase::Climb(stop), CLIMB, &mut t);
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
                // `orbiting`): arrive over the night side, cross sunrise
                // high up, and run the low orbit through the morning.
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
        tour.paths = (0..tour.itinerary.len())
            .map(|k| tour.build_path(k, flybys[k], &mut rng))
            .collect();
        tour
    }

    /// Which transfers swing past something on the way: the star, when the
    /// stops lie at different angles around it, or a planet not otherwise
    /// visited that lies near the way. At least one passes the star.
    fn choose_flybys(
        system: &System,
        itinerary: &[Stop],
        entry: [f64; 3],
        rng: &mut super::system::Rng,
    ) -> Vec<Option<Flyby>> {
        let visited: Vec<usize> = itinerary.iter().map(|s| s.planet).collect();
        let mut angles = Vec::new();
        let mut flybys: Vec<Option<Flyby>> = itinerary
            .iter()
            .enumerate()
            .map(|(k, stop)| {
                let a = if k == 0 {
                    entry
                } else {
                    system.centre(itinerary[k - 1], 0.0)
                };
                let b = system.centre(*stop, 0.0);
                let angle = dot(normalize(a), normalize(b)).clamp(-1.0, 1.0).acos();
                if stop.moon.is_some() {
                    angles.push(-1.0);
                    return None;
                }
                angles.push(angle);
                let chord = length(sub(b, a));
                let near = (0..system.planets.len())
                    .filter(|i| !visited.contains(i))
                    .map(|i| {
                        let c = system.planets[i].position(0.0);
                        (i, (length(sub(c, a)) + length(sub(b, c))) / chord)
                    })
                    .min_by(|x, y| x.1.total_cmp(&y.1));
                let roll = rng.unit();
                match near {
                    Some((i, detour)) if detour < 1.4 && roll < 0.45 => Some(Flyby::Planet(i)),
                    _ if roll < 0.85 && angle > 0.7 => Some(Flyby::Star),
                    _ => None,
                }
            })
            .collect();
        if !flybys.contains(&Some(Flyby::Star))
            && let Some(k) = (0..angles.len())
                .filter(|&k| angles[k] >= 0.0)
                .max_by(|&a, &b| angles[a].total_cmp(&angles[b]))
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
        let ends = |knots: &[[f64; 3]]| {
            let n = knots.len();
            let first = sub(knots[1], knots[0]);
            let last = sub(knots[n - 1], knots[n - 2]);
            (
                add(scale(first, 0.5), scale(away, length(first) * 0.4)),
                add(scale(last, 0.5), scale(into, length(last) * 0.25)),
            )
        };
        let mut path = Path {
            knots: vec![p0, p1],
            tangents: Vec::new(),
            anchors: vec![start_anchor, end_anchor],
            time: Vec::new(),
            length: Vec::new(),
            flyby,
            closest: 0.5,
        };
        // Through the closest point of a flyby: round the far side of the
        // star from where the stops lie, or past the planet on the side
        // toward the straight way.
        let lift = rng.range(0.1, 0.3) * if rng.chance(0.5) { 1.0 } else { -1.0 };
        let waypoint = |when: f64| -> ([f64; 3], [f64; 3], f64, Anchor) {
            match flyby {
                Some(Flyby::Planet(i)) => {
                    let planet = &self.system.planets[i];
                    let c = planet.position(when);
                    let chord = sub(p1, p0);
                    let along = (dot(sub(c, p0), chord) / dot(chord, chord)).clamp(0.0, 1.0);
                    let q = add(p0, scale(chord, along));
                    let mut side = sub(q, c);
                    if length(side) < 1e-6 {
                        side = cross(chord, [0.0, 1.0, 0.0]);
                    }
                    let side = normalize(add(normalize(side), [0.0, lift, 0.0]));
                    let reach = planet.radius
                        * match &planet.rings {
                            Some(r) => (r.outer * 1.25).max(3.0),
                            None if planet.kind.is_giant() => 3.0,
                            None => 3.2,
                        };
                    (
                        add(c, scale(side, reach)),
                        c,
                        reach,
                        Anchor::Stop(Stop::planet(i)),
                    )
                }
                _ => {
                    let (a, b) = (normalize(p0), normalize(p1));
                    let middle = add(a, b);
                    let dir = if length(middle) < 0.3 {
                        normalize(cross(a, [0.0, 1.0, 0.0]))
                    } else {
                        scale(normalize(middle), -1.0)
                    };
                    let dir = normalize(add(dir, [0.0, lift, 0.0]));
                    let reach = (self.system.star.radius * rng_reach(self.system.seed)).max(8.0);
                    (scale(dir, reach), [0.0; 3], reach, Anchor::Fixed)
                }
            }
        };
        let mut when = 0.5 * (t0 + t1);
        let iterations = if flyby.is_some() { 3 } else { 1 };
        for _ in 0..iterations {
            if flyby.is_some() {
                let (w, centre, reach, anchor) = waypoint(when);
                path.knots = vec![p0, w, p1];
                path.anchors = vec![start_anchor, (anchor, when), end_anchor];
                let (m0, m2) = ends(&path.knots);
                let radial = normalize(sub(w, centre));
                let through = add(normalize(sub(w, p0)), normalize(sub(p1, w)));
                let mut tangent = sub(through, scale(radial, dot(through, radial)));
                if length(tangent) < 1e-6 {
                    tangent = cross(radial, [0.0, 1.0, 0.0]);
                }
                let arm = length(sub(w, p0)).min(length(sub(p1, w)));
                path.tangents = vec![m0, scale(normalize(tangent), arm * 0.8), m2];
                path.time(Some((centre, reach)));
                when = t0 + path.closest * (t1 - t0);
            } else {
                let (m0, m1) = ends(&path.knots);
                path.tangents = vec![m0, m1];
                path.time(None);
            }
        }
        path
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
        let k = (super::system::Rng::new(self.next.seed ^ 0xd21e).next() % DRIVES.len() as u64)
            as usize;
        (k as u32 + 1, DRIVES[k])
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

    /// 0 in high orbit, 1 in low orbit, for `stop` at time `t`.
    fn lowness(&self, stop: Stop, t: f64) -> f64 {
        let (segment, local) = self.segment(t);
        match segment.phase {
            Phase::Descend(p) if p == stop => smootherstep(local / segment.duration),
            Phase::Low(p) if p == stop => 1.0,
            Phase::Climb(p) if p == stop => 1.0 - smootherstep(local / segment.duration),
            _ => 0.0,
        }
    }

    /// High and low orbit distances from the planet's centre.
    fn heights(planet: &Planet) -> (f64, f64) {
        let high = planet.radius * if planet.kind.is_giant() { 3.7 } else { 3.0 };
        let low = planet.radius * if planet.kind.is_giant() { 1.22 } else { 1.10 };
        (high, low)
    }

    /// Angle along the orbit `seconds` after arriving: quicker high up,
    /// slower near the ground so the land does not race past. The rate eases
    /// with the height, integrated numerically.
    fn orbit_angle(&self, stop: Stop, t: f64) -> f64 {
        let start = self.arrival(stop);
        let orbit = self.orbit_of(stop);
        let span = (t - start).max(0.0);
        let steps = 48;
        let dt = span / steps as f64;
        let rate = |tt: f64| {
            let a = self.lowness(stop, tt);
            std::f64::consts::TAU * (1.0 / 150.0 + (1.0 / 900.0 - 1.0 / 150.0) * a)
        };
        let mut sum = 0.0;
        for i in 0..steps {
            sum += rate(start + (i as f64 + 0.5) * dt) * dt;
        }
        // Before arrival (on the transfer's last stretch) keep moving at the
        // high rate so the hand-over is smooth.
        if t < start {
            sum = -(start - t) * std::f64::consts::TAU / 150.0;
        }
        orbit.angle + sum
    }

    /// Camera position, forward and up while orbiting `stop` at `t`.
    fn orbiting(&self, stop: Stop, t: f64) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let body = self.world(stop);
        let centre = self.system.centre(stop, t);
        let orbit = self.orbit_of(stop);
        let a = self.lowness(stop, t);
        let (high, low) = Self::heights(body);
        let distance = high + (low - high) * a;
        let angle = self.orbit_angle(stop, t);
        let (s, c) = angle.sin_cos();
        // The plane's axes follow the sun, which an inner planet's own orbit
        // swings round during a visit: angle 0 is noon below the camera.
        let sun = normalize(scale(centre, -1.0));
        let flat = sub(sun, scale(orbit.n, dot(sun, orbit.n)));
        let u = if length(flat) > 1e-6 {
            normalize(flat)
        } else {
            orbit.u
        };
        let v = cross(orbit.n, u);
        let radial = add(scale(u, c), scale(v, s));
        let prograde = add(scale(u, -s), scale(v, c));
        let position = add(centre, scale(radial, distance));
        // High up: the planet a little off centre. Low down: along the orbit,
        // just below the horizon so it crosses the upper part of the panel.
        let f_high = normalize(add(scale(radial, -0.97), scale(prograde, 0.24)));
        let dip = (body.radius / distance).clamp(-1.0, 1.0).acos();
        let below = dip + 0.32 * HALF_FOV.atan();
        let f_low = normalize(add(
            scale(prograde, below.cos()),
            scale(radial, -below.sin()),
        ));
        let forward = normalize(mix(f_high, f_low, smoothstep(0.25, 1.0, a)));
        let up = normalize(mix(orbit.n, radial, a));
        (position, forward, up)
    }

    /// Everything a transfer path can bump into: planets, moons and the
    /// star, with how close the camera may come.
    fn obstacles(&self, t: f64) -> Vec<([f64; 3], f64)> {
        let mut obstacles = Vec::new();
        for (i, p) in self.system.planets.iter().enumerate() {
            let centre = p.position(t);
            obstacles.push((centre, p.radius * if p.kind.is_giant() { 2.8 } else { 2.0 }));
            for m in 0..p.moons.len().min(super::stream::MOONS as usize) {
                let stop = Stop {
                    planet: i,
                    moon: Some(m),
                };
                obstacles.push((self.system.centre(stop, t), p.moons[m].radius * 2.0));
            }
        }
        obstacles.push(([0.0; 3], self.system.star.radius * 3.0));
        obstacles
    }

    /// The camera's position on transfer `k`'s path at `t`, and how far
    /// along the spline it is.
    fn on_path(&self, k: usize, segment: Segment, t: f64) -> ([f64; 3], f64) {
        let path = &self.paths[k];
        let u = ((t - segment.start) / segment.duration).clamp(0.0, 1.0);
        let sigma = path.sigma(u);
        let mut position = path.spline(sigma);
        // Each knot moves with its body, blended along the spline.
        let last = path.knots.len() - 1;
        let j = (sigma.floor() as usize).min(last - 1);
        let s = smoothstep(0.0, 1.0, sigma - j as f64);
        for (knot, weight) in [(j, 1.0 - s), (j + 1, s)] {
            if let (Anchor::Stop(stop), placed) = path.anchors[knot] {
                let drift = sub(self.system.centre(stop, t), self.system.centre(stop, placed));
                position = add(position, scale(drift, weight));
            }
        }
        // Swing around anything in the way: a soft floor on the distance
        // from every body.
        let fade = smoothstep(0.0, 0.02, u) * (1.0 - smoothstep(0.98, 1.0, u));
        for (centre, floor) in self.obstacles(t) {
            let rel = sub(position, centre);
            let d = length(rel);
            let k = floor * 0.2;
            let soft = 0.5 * (d + floor + ((d - floor) * (d - floor) + k * k).sqrt());
            position = add(position, scale(normalize(rel), (soft - d) * fade));
        }
        (position, sigma)
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
        let mut flash = 0.0;
        let (position, forward, up) = match segment.phase {
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) => {
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
                let (_, f1, up1) = self.orbiting(to, t1);
                let (position, sigma) = self.on_path(k, segment, t);
                let target = self.system.centre(to, t);
                let toward = normalize(sub(target, position));
                // Look ahead toward the destination, and round at what the
                // path swings past as it nears it, with the way it is
                // heading mixed in so the turn never flips.
                let ahead = {
                    let a = self.paths[k].spline((sigma + 0.02).min((self.paths[k].knots.len() - 1) as f64));
                    let b = self.paths[k].spline(sigma);
                    let d = sub(a, b);
                    if length(d) > 1e-9 { normalize(d) } else { toward }
                };
                let mut aim = toward;
                if let Some((centre, _, _)) = self.flyby_body(k, t) {
                    let near = (u - self.paths[k].closest) / 0.17;
                    let w = 0.92 * (-near * near).exp();
                    let at = normalize(sub(centre, position));
                    let mixed = add(add(scale(toward, 1.0 - w), scale(at, w)), scale(ahead, 0.25 * w));
                    aim = if length(mixed) > 0.2 { normalize(mixed) } else { ahead };
                }
                let mut forward = normalize(mix(f0, aim, smoothstep(0.0, 0.18, u)));
                forward = normalize(mix(forward, f1, smoothstep(0.82, 1.0, u)));
                let up = normalize(mix(up0, up1, smoothstep(0.1, 0.9, u)));
                if from.is_none() {
                    flash = 1.0 - smoothstep(0.0, 0.035, u);
                }
                (position, forward, up)
            }
            Phase::Depart(p) => {
                let (p0, f0, up0) = self.orbiting(p, segment.start);
                let centre = self.system.centre(p, t);
                let out = normalize(sub(p0, centre));
                let s = smootherstep(u);
                let distance = self.world(p).radius * 30.0;
                let position = add(p0, scale(normalize(add(out, self.exit)), distance * s * s));
                let forward = normalize(mix(f0, self.exit, smoothstep(0.0, 0.8, u)));
                let up = normalize(mix(up0, [0.0, 1.0, 0.0], s));
                warp = smoothstep(0.85, 1.0, u) * 0.25;
                (position, forward, up)
            }
            Phase::Jump => {
                // Out beyond the departure point, heading for the next star.
                let last = *self.itinerary.last().expect("a tour has stops");
                let (p0, ..) = self.orbiting(last, segment.start - DEPART);
                let start = add(p0, scale(self.exit, self.world(last).radius * 30.0));
                let position = add(start, scale(self.exit, local * 400.0));
                warp = 0.25 + 0.75 * smoothstep(0.0, 0.07, u);
                warp *= 1.0 - smoothstep(0.93, 1.0, u);
                flash = smoothstep(0.97, 1.0, u);
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
            flash,
        }
    }

    /// The stops whose maps should be resident at `t`: the one being visited
    /// or approached, and the next. At a moon, its giant stays resident, as
    /// it fills the moon's sky. During the jump, none: the next visit's
    /// first stop is what to stream.
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
            Phase::Transfer { to, .. } => (Some(to), second(to)),
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) => {
                (Some(p), second(p))
            }
            Phase::Depart(p) => (
                Some(p),
                p.moon.map(|_| Stop::planet(p.planet)),
            ),
            Phase::Jump => (None, None),
        }
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
                self.on_path(k, segment, tt).0
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
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) | Phase::Depart(p) => Some(p),
            Phase::Jump => None,
        }
    }

    /// The scanner's readings at `t`: while orbiting a stop, from the moment
    /// the camera settles into orbit; none otherwise.
    pub fn scans(&self, t: f64) -> Vec<super::scan::Scan> {
        let (segment, _) = self.segment(t);
        match segment.phase {
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) => {
                let survey = super::scan::Survey::of(self.world(p), &self.system.star);
                survey.at(t - self.arrival(p))
            }
            _ => Vec::new(),
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
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) => {
                let planet = self.world(p);
                let distance = length(sub(view.position, self.system.centre(p, t)));
                let altitude = (distance - planet.radius) * KM;
                let speed = planet.orbital_speed(distance);
                let title = match segment.phase {
                    Phase::Low(_) => "LOW ORBIT",
                    Phase::Descend(_) => "DESCENDING",
                    Phase::Climb(_) => "CLIMBING",
                    _ => "ORBIT",
                };
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
