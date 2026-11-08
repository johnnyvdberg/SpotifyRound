//! The grand tour of one system, as a timeline: where the camera is and what
//! the HUD says at any moment of it.
//!
//! A visit drops out of the previous hyperjump outside the system, crosses to
//! the first planet on the itinerary, and for each planet settles into a high
//! orbit with its moons and rings in view, dives to a low orbit over the
//! horizon, climbs back out and crosses to the next. After the last it turns
//! toward the next star and jumps: the jump is the visit's last segment, and
//! the next visit begins where it ends. About nine and a half minutes in all.
//!
//! Everything here is a pure function of the time into the visit, so any
//! moment can be shown on its own (the previews do) and nothing drifts.

use super::system::{AU, KM, Planet, REAL_AU, System};

/// Seconds each part of the tour takes.
pub const TRANSFER: f64 = 45.0;
pub const HIGH: f64 = 35.0;
pub const DESCEND: f64 = 12.0;
pub const LOW: f64 = 28.0;
pub const CLIMB: f64 = 12.0;
pub const DEPART: f64 = 20.0;
pub const JUMP: f64 = 150.0;
/// Field of view, the tangent of half of it.
const HALF_FOV: f64 = 0.4663; // 50°

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Phase {
    /// Crossing to planet `to`: from the drop-out point, or from another planet.
    Transfer {
        from: Option<usize>,
        to: usize,
    },
    High(usize),
    Descend(usize),
    Low(usize),
    Climb(usize),
    Depart(usize),
    Jump,
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
    pub itinerary: Vec<usize>,
    pub segments: Vec<Segment>,
    pub total: f64,
    /// Per planet visited: the start angle of its orbit and its plane.
    orbits: Vec<Orbit>,
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

impl Tour {
    pub fn new(system: System, next: System) -> Self {
        let itinerary = system.itinerary();
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
        for &planet in &itinerary {
            push(Phase::Transfer { from, to: planet }, TRANSFER, &mut t);
            push(Phase::High(planet), HIGH, &mut t);
            push(Phase::Descend(planet), DESCEND, &mut t);
            push(Phase::Low(planet), LOW, &mut t);
            push(Phase::Climb(planet), CLIMB, &mut t);
            from = Some(planet);
        }
        let last = *itinerary.last().unwrap_or(&0);
        push(Phase::Depart(last), DEPART, &mut t);
        push(Phase::Jump, JUMP, &mut t);

        let mut rng = super::system::Rng::new(system.seed ^ 0x0b17);
        let orbits = itinerary
            .iter()
            .map(|&i| {
                let planet = &system.planets[i];
                // Inclined to the planet's equator, so rings and moons are
                // seen from a little above or below.
                let axis = planet.axis();
                let side = normalize(cross(axis, [0.3, 0.1, 1.0]));
                let tilt = rng.range(0.25, 0.55) * if rng.chance(0.5) { 1.0 } else { -1.0 };
                let n = normalize(add(scale(axis, tilt.cos()), scale(side, tilt.sin())));
                let u = normalize(cross(n, [0.0, 0.0, 1.0]));
                Orbit {
                    u,
                    n,
                    angle: rng.range(0.0, std::f64::consts::TAU),
                }
            })
            .collect();
        // Drop out above the plane of the system, beyond the first stop,
        // looking in toward the star.
        let first = &system.planets[itinerary[0]];
        let first_at = first.position(0.0);
        let out = normalize(add(normalize(first_at), [0.0, 0.35, 0.0]));
        let entry = scale(out, first.orbit * 1.6 + 400.0);
        let exit = normalize([
            rng.range(-1.0, 1.0),
            rng.range(0.2, 0.6),
            rng.range(-1.0, 1.0),
        ]);
        // Arrive on the side the camera comes from, so the approach never
        // passes through the planet.
        let mut orbits: Vec<Orbit> = orbits;
        for k in 0..itinerary.len() {
            let planet = &system.planets[itinerary[k]];
            let arrival = segments
                .iter()
                .find(|s| s.phase == Phase::High(itinerary[k]))
                .map_or(0.0, |s| s.start);
            let centre = planet.position(arrival);
            let from = if k == 0 {
                entry
            } else {
                system.planets[itinerary[k - 1]].position(arrival)
            };
            // Arrive on the morning side, a little short of noon, so the
            // high orbit crosses the lit face and the low one runs on toward
            // the terminator; transfers swing round the planet if they must.
            let _ = (from, centre);
            // Angles are measured from the sun's direction (see `orbiting`):
            // arrive over the night side, cross sunrise high up, and run the
            // low orbit through the morning toward noon.
            orbits[k].angle = -2.2;
        }
        let mut tour = Tour {
            system,
            next,
            itinerary,
            segments,
            total: t,
            orbits,
            entry,
            exit,
        };
        tour.stage_moons();
        tour
    }

    /// Turn each visited planet's largest moon to where the camera will see
    /// it: past the planet's limb, a little to one side, half way through
    /// the high orbit. Moons are small against their planets; left to chance
    /// they would mostly be behind the camera or lost in the glare.
    fn stage_moons(&mut self) {
        for &p in &self.itinerary.clone() {
            let Some(segment) = self
                .segments
                .iter()
                .find(|s| s.phase == Phase::High(p))
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
            let (camera, forward, up) = self.orbiting(p, t);
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

    fn orbit_of(&self, planet: usize) -> Orbit {
        let k = self
            .itinerary
            .iter()
            .position(|&p| p == planet)
            .unwrap_or(0);
        self.orbits[k]
    }

    /// When the camera reaches planet `planet`'s high orbit.
    fn arrival(&self, planet: usize) -> f64 {
        self.segments
            .iter()
            .find(|s| s.phase == Phase::High(planet))
            .map_or(0.0, |s| s.start)
    }

    /// 0 in high orbit, 1 in low orbit, for planet `planet` at time `t`.
    fn lowness(&self, planet: usize, t: f64) -> f64 {
        let (segment, local) = self.segment(t);
        match segment.phase {
            Phase::Descend(p) if p == planet => smootherstep(local / segment.duration),
            Phase::Low(p) if p == planet => 1.0,
            Phase::Climb(p) if p == planet => 1.0 - smootherstep(local / segment.duration),
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
    fn orbit_angle(&self, planet: usize, t: f64) -> f64 {
        let start = self.arrival(planet);
        let orbit = self.orbit_of(planet);
        let span = (t - start).max(0.0);
        let steps = 48;
        let dt = span / steps as f64;
        let rate = |tt: f64| {
            let a = self.lowness(planet, tt);
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

    /// Camera position, forward and up while orbiting `planet` at `t`.
    fn orbiting(&self, planet: usize, t: f64) -> ([f64; 3], [f64; 3], [f64; 3]) {
        let body = &self.system.planets[planet];
        let centre = body.position(t);
        let orbit = self.orbit_of(planet);
        let a = self.lowness(planet, t);
        let (high, low) = Self::heights(body);
        let distance = high + (low - high) * a;
        let angle = self.orbit_angle(planet, t);
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
                let t0 = segment.start;
                let t1 = segment.start + segment.duration;
                let (p0, f0, up0) = match from {
                    Some(p) => self.orbiting(p, t0),
                    None => {
                        let toward = normalize(scale(self.entry, -1.0));
                        (self.entry, toward, [0.0, 1.0, 0.0])
                    }
                };
                let (p1, f1, up1) = self.orbiting(to, t1);
                let target = self.system.planets[to].position(t);
                let span = length(sub(p1, p0));
                let away = match from {
                    Some(p) => normalize(sub(p0, self.system.planets[p].position(t0))),
                    None => normalize(sub(p1, p0)),
                };
                let into = normalize(sub(target, p1));
                let m0 = add(
                    scale(normalize(sub(p1, p0)), span * 0.5),
                    scale(away, span * 0.4),
                );
                let m1 = add(
                    scale(normalize(sub(p1, p0)), span * 0.5),
                    scale(into, span * 0.25),
                );
                let s = smootherstep(u);
                let (h00, h10, h01, h11) = (
                    2.0 * s * s * s - 3.0 * s * s + 1.0,
                    s * s * s - 2.0 * s * s + s,
                    -2.0 * s * s * s + 3.0 * s * s,
                    s * s * s - s * s,
                );
                // The planets keep moving along their orbits meanwhile: carry
                // the path with the one left at first and the one ahead at
                // the end, so it meets both exactly.
                let drift_from = match from {
                    Some(p) => sub(
                        self.system.planets[p].position(t),
                        self.system.planets[p].position(t0),
                    ),
                    None => [0.0; 3],
                };
                let drift_to = sub(target, self.system.planets[to].position(t1));
                let position = add(
                    add(
                        add(scale(p0, h00), scale(m0, h10)),
                        add(scale(p1, h01), scale(m1, h11)),
                    ),
                    mix(drift_from, drift_to, s),
                );
                // Swing around anything in the way: a soft floor on the
                // distance from every planet and the star.
                let mut position = position;
                let fade = smoothstep(0.0, 0.02, u) * (1.0 - smoothstep(0.98, 1.0, u));
                let mut obstacles: Vec<([f64; 3], f64)> = self
                    .system
                    .planets
                    .iter()
                    .map(|p| {
                        (
                            p.position(t),
                            p.radius * if p.kind.is_giant() { 2.8 } else { 2.0 },
                        )
                    })
                    .collect();
                obstacles.push(([0.0; 3], self.system.star.radius * 3.0));
                for (centre, floor) in obstacles {
                    let rel = sub(position, centre);
                    let d = length(rel);
                    let k = floor * 0.2;
                    let soft = 0.5 * (d + floor + ((d - floor) * (d - floor) + k * k).sqrt());
                    position = add(position, scale(normalize(rel), (soft - d) * fade));
                }
                let toward = normalize(sub(target, position));
                let mut forward = normalize(mix(f0, toward, smoothstep(0.0, 0.18, u)));
                forward = normalize(mix(forward, f1, smoothstep(0.82, 1.0, u)));
                let up = normalize(mix(up0, up1, smoothstep(0.1, 0.9, u)));
                if from.is_none() {
                    flash = 1.0 - smoothstep(0.0, 0.035, u);
                }
                (position, forward, up)
            }
            Phase::Depart(p) => {
                let (p0, f0, up0) = self.orbiting(p, segment.start);
                let centre = self.system.planets[p].position(t);
                let out = normalize(sub(p0, centre));
                let s = smootherstep(u);
                let distance = self.system.planets[p].radius * 30.0;
                let position = add(p0, scale(normalize(add(out, self.exit)), distance * s * s));
                let forward = normalize(mix(f0, self.exit, smoothstep(0.0, 0.8, u)));
                let up = normalize(mix(up0, [0.0, 1.0, 0.0], s));
                warp = smoothstep(0.85, 1.0, u) * 0.25;
                (position, forward, up)
            }
            Phase::Jump => {
                // Out beyond the departure point, heading for the next star.
                let last = *self.itinerary.last().unwrap_or(&0);
                let (p0, ..) = self.orbiting(last, segment.start - DEPART);
                let start = add(
                    p0,
                    scale(self.exit, self.system.planets[last].radius * 30.0),
                );
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

    /// The planets whose maps should be resident at `t`: the one being visited
    /// or approached, and the next on the itinerary. During the jump, none:
    /// the next visit's first planet is what to stream.
    pub fn wanted(&self, t: f64) -> (Option<usize>, Option<usize>) {
        let (segment, _) = self.segment(t);
        let next_after = |p: usize| {
            let k = self.itinerary.iter().position(|&q| q == p)?;
            self.itinerary.get(k + 1).copied()
        };
        match segment.phase {
            Phase::Transfer { to, .. } => (Some(to), next_after(to)),
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) => {
                (Some(p), next_after(p))
            }
            Phase::Depart(p) => (Some(p), None),
            Phase::Jump => (None, None),
        }
    }

    /// The HUD at `t`: a title and lines of label and value.
    pub fn hud(&self, t: f64) -> (String, Vec<(String, String)>) {
        let (segment, local) = self.segment(t);
        let left = segment.duration - local;
        let view = self.view(t);
        let star = &self.system.star;
        let describe = |p: &Planet| {
            let moons = match p.moons.len() {
                0 => String::new(),
                1 => " · 1 moon".to_string(),
                n => format!(" · {n} moons"),
            };
            let rings = if p.rings.is_some() { " · rings" } else { "" };
            format!("{} · {:.1}× Earth{moons}{rings}", p.kind.name(), p.radius)
        };
        match segment.phase {
            Phase::Transfer { to, .. } => {
                let planet = &self.system.planets[to];
                let target = planet.position(t);
                let remaining = length(sub(target, view.position)) / AU;
                // A transfer orbit's speed around the star, from the vis-viva
                // equation between where it started and where it is going.
                let r = (length(view.position) / AU).max(0.05) * REAL_AU * KM;
                let a = (r + planet.orbit / AU * REAL_AU * KM) / 2.0;
                let gm = super::system::GM_SUN * star.mass;
                let v = (gm * (2.0 / r - 1.0 / a)).max(0.0).sqrt();
                (
                    "TRANSFER".into(),
                    vec![
                        ("Destination".into(), planet.name.clone()),
                        ("".into(), describe(planet)),
                        ("ETA".into(), clock(left + 0.0)),
                        ("Distance".into(), format!("{remaining:.2} AU")),
                        ("Velocity".into(), format!("{v:.1} km/s")),
                    ],
                )
            }
            Phase::High(p) | Phase::Descend(p) | Phase::Low(p) | Phase::Climb(p) => {
                let planet = &self.system.planets[p];
                let distance = length(sub(view.position, planet.position(t)));
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
                        Phase::Transfer { to, .. } => Some(self.system.planets[to].name.clone()),
                        _ => None,
                    })
                    .unwrap_or_else(|| format!("jump to {}", self.next.star.name));
                (
                    title.into(),
                    vec![
                        ("Orbiting".into(), planet.name.clone()),
                        ("".into(), describe(planet)),
                        ("Orbital speed".into(), format!("{speed:.2} km/s")),
                        (
                            "Altitude".into(),
                            format!("{} km", thousands(altitude.max(0.0))),
                        ),
                        (
                            "Moons".into(),
                            if planet.moons.is_empty() {
                                "none".into()
                            } else {
                                planet
                                    .moons
                                    .iter()
                                    .map(|m| m.name.clone())
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            },
                        ),
                        ("Next".into(), format!("{next} in {}", clock(departs))),
                    ],
                )
            }
            Phase::Depart(_) => (
                "DEPARTURE".into(),
                vec![
                    (
                        "Destination".into(),
                        format!(
                            "{} ({}-type star)",
                            self.next.star.name, self.next.star.class
                        ),
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
                        (
                            "Destination".into(),
                            format!(
                                "{} ({}-type star)",
                                self.next.star.name, self.next.star.class
                            ),
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
            tour.total > 6.0 * 60.0 && tour.total < 12.0 * 60.0,
            "{}",
            tour.total
        );
    }

    #[test]
    fn the_camera_never_jumps_or_hits_a_planet() {
        for seed in 0..12 {
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
            }
            let star = length(view.position);
            assert!(
                star > tour.system.star.radius * 1.5,
                "inside the star at {t}: {star}"
            );
            // Never more than a fraction of the way between planets in one frame.
            let step = length(sub(view.position, last));
            assert!(step < 3000.0, "a jump of {step} at {t}");
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
