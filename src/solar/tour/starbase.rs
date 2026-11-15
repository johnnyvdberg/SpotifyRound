//! A visit to a starbase the size of a moon: in place of an orbit, the
//! ship calls approach control, is cleared to dock (or told to hold while
//! priority traffic goes in first), lines up on its corridor, flies in
//! through the mouth and down a tunnel to a bay, docks, is serviced, then
//! undocks, turns about, flies back out and climbs back into the orbit it
//! left, from where the tour goes on as after any other orbit.
//!
//! The way in is staged like a drive spooling up: one segment
//! (`BaseStage::Approach`) whose steps (contact, clearance, the hold,
//! alignment, final approach) the docking panel lists and lights in turn.
//! While it calls and holds, the ship goes on round its orbit; a hold is a
//! holding pattern, the orbit flown a while longer.
//!
//! Outside, the camera is placed round the base's centre as it moves; from
//! the corridor in, in the base's own frame, in metres: the mouth at the
//! origin, +z into the base (down the tunnel), +y up and +x to the right.
//! Like the rest of the tour, everything is a pure function of the time.

use super::super::system::{KM, Rng, Stop, System, forced_starbase};
use super::{
    Manoeuvre, Phase, Tour, add, clock, cross, dot, hermite, length, normalize, scale, smoothstep,
    sub, thousands,
};

/// The steps of the way in, seconds each: the call to approach control,
/// the clearance, the hold (only when traffic has priority), lining up on
/// the corridor and the final approach to the mouth.
pub const CONTACT: f64 = 8.0;
pub const CLEARANCE: f64 = 7.0;
pub const HOLD: f64 = 18.0;
pub const ALIGN: f64 = 12.0;
pub const FINAL: f64 = 12.0;
/// The rest of the visit, seconds each.
pub const TUNNEL: f64 = 25.0;
pub const BERTH: f64 = 15.0;
pub const SERVICE: f64 = 40.0;
pub const UNDOCK: f64 = 12.0;
pub const OUTBOUND: f64 = 22.0;
pub const CLIMB: f64 = 14.0;
/// The chance another ship is given priority and goes in first.
pub const TRAFFIC_CHANCE: f64 = 0.2;

/// A metre, in the tour's units (Earth radii).
const METRE: f64 = 1.0 / (KM * 1000.0);
/// Where the final approach begins, metres out from the mouth, and how
/// fast the ship passes through it, metres a second.
const CORRIDOR: f64 = 60_000.0;
const MOUTH_SPEED: f64 = 250.0;
/// The tunnel: its length and how far its S-bend carries it to the side,
/// metres; how fast the ship comes out of it into the bay.
const TUNNEL_LENGTH: f64 = 6000.0;
const BEND: f64 = 400.0;
const BAY_SPEED: f64 = 60.0;
/// The cradle: this far into the bay past the tunnel's end, and this far
/// below its line, metres; how far the ship backs out of it.
const CRADLE: f64 = 300.0;
const CRADLE_DROP: f64 = 15.0;
const BACK_OUT: f64 = 60.0;
/// How fast the ship leaves through the mouth, metres a second.
const EXIT_SPEED: f64 = 400.0;

/// Where in a starbase visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BaseStage {
    /// Calling in, cleared (or holding), lining up and flying the corridor
    /// to the mouth: one segment, in steps.
    Approach,
    /// Down the tunnel, the city through its windows.
    Tunnel,
    /// Into the bay and the cradle, clamped.
    Berth,
    /// Docked, the crews at work.
    Service,
    /// Released, backed out and turned about.
    Undock,
    /// Back up the tunnel to the mouth.
    Outbound,
    /// Out of the mouth, climbing back into orbit.
    Climb,
}

impl BaseStage {
    pub const ALL: [BaseStage; 7] = [
        BaseStage::Approach,
        BaseStage::Tunnel,
        BaseStage::Berth,
        BaseStage::Service,
        BaseStage::Undock,
        BaseStage::Outbound,
        BaseStage::Climb,
    ];

    /// A short name, for screenshots and tests.
    pub fn name(self) -> &'static str {
        match self {
            BaseStage::Approach => "approach",
            BaseStage::Tunnel => "tunnel",
            BaseStage::Berth => "berth",
            BaseStage::Service => "service",
            BaseStage::Undock => "undock",
            BaseStage::Outbound => "outbound",
            BaseStage::Climb => "climb",
        }
    }
}

/// A ship given priority on the way in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Traffic {
    pub name: &'static str,
    pub class: &'static str,
    /// Why it goes first.
    pub reason: &'static str,
}

/// The ships that may be given priority.
const TRAFFIC: [Traffic; 8] = [
    Traffic {
        name: "Kestrel",
        class: "medical frigate",
        reason: "casualties aboard",
    },
    Traffic {
        name: "Iron Psalm",
        class: "ore hauler",
        reason: "reactor running hot",
    },
    Traffic {
        name: "Vantablue",
        class: "courier",
        reason: "diplomatic seal",
    },
    Traffic {
        name: "Mother Hesper",
        class: "colony ark",
        reason: "too big to wait",
    },
    Traffic {
        name: "Saint Orla",
        class: "patrol cutter",
        reason: "back on fumes",
    },
    Traffic {
        name: "Halberd",
        class: "fleet tender",
        reason: "fleet priority",
    },
    Traffic {
        name: "Low Tide",
        class: "salvage tug",
        reason: "towing a wreck",
    },
    Traffic {
        name: "Aurelian",
        class: "liner",
        reason: "on schedule",
    },
];

/// The steps of the way in, as the docking panel lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    Contact,
    Clearance,
    Hold,
    Align,
    Final,
}

/// A system's starbase, as the tour visits it.
#[derive(Clone, Debug)]
pub struct Base {
    pub stop: Stop,
    pub name: String,
    pub bay: u32,
    pub corridor: u32,
    /// The ship given priority on the way in, if one is.
    pub traffic: Option<Traffic>,
}

impl Base {
    /// The visit to `stop`, the starbase of `system`.
    pub fn plan(system: &System, stop: Stop) -> Self {
        let mut rng = Rng::new(system.seed ^ 0xa7c0_d0c5);
        let bay = rng.int(3, 19) as u32;
        let corridor = rng.int(1, 4) as u32;
        let ship = *rng.pick(&TRAFFIC);
        let hold = match forced_starbase() {
            Some(Some(hold)) => hold,
            _ => rng.chance(TRAFFIC_CHANCE),
        };
        Self::with_traffic(system, stop, bay, corridor, hold.then_some(ship))
    }

    pub fn with_traffic(
        system: &System,
        stop: Stop,
        bay: u32,
        corridor: u32,
        traffic: Option<Traffic>,
    ) -> Self {
        let name = stop
            .moon
            .map(|m| system.planets[stop.planet].moons[m].name.clone())
            .unwrap_or_default();
        Base {
            stop,
            name,
            bay,
            corridor,
            traffic,
        }
    }

    /// The steps of the way in, each with when it begins, seconds into it.
    pub fn steps(&self) -> Vec<(Step, f64)> {
        let mut steps = vec![(Step::Contact, CONTACT), (Step::Clearance, CLEARANCE)];
        if self.traffic.is_some() {
            steps.push((Step::Hold, HOLD));
        }
        steps.push((Step::Align, ALIGN));
        steps.push((Step::Final, FINAL));
        let mut at = 0.0;
        steps
            .into_iter()
            .map(|(step, length)| {
                let start = at;
                at += length;
                (step, start)
            })
            .collect()
    }

    /// The way in, seconds.
    pub fn approach(&self) -> f64 {
        CONTACT + CLEARANCE + self.traffic.map_or(0.0, |_| HOLD) + ALIGN + FINAL
    }

    /// When the ship leaves its orbit for the corridor, seconds into the
    /// way in.
    fn align_at(&self) -> f64 {
        self.approach() - ALIGN - FINAL
    }

    /// The step under way `local` seconds into the way in.
    pub fn step(&self, local: f64) -> Step {
        self.steps()
            .into_iter()
            .rev()
            .find(|&(_, start)| local >= start)
            .map_or(Step::Contact, |(step, _)| step)
    }

    /// The visit's segments: each stage and how long it takes.
    pub fn stages(&self) -> Vec<(BaseStage, f64)> {
        BaseStage::ALL
            .into_iter()
            .map(|stage| {
                let length = match stage {
                    BaseStage::Approach => self.approach(),
                    BaseStage::Tunnel => TUNNEL,
                    BaseStage::Berth => BERTH,
                    BaseStage::Service => SERVICE,
                    BaseStage::Undock => UNDOCK,
                    BaseStage::Outbound => OUTBOUND,
                    BaseStage::Climb => CLIMB,
                };
                (stage, length)
            })
            .collect()
    }

    /// The whole visit, seconds.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn duration(&self) -> f64 {
        self.stages().iter().map(|&(_, d)| d).sum()
    }
}

/// The base's frame in the world: the way out through the mouth, and up
/// and right as the ship sees them flying in.
#[derive(Clone, Copy, Debug)]
struct Frame {
    out: [f64; 3],
    up: [f64; 3],
    right: [f64; 3],
    radius: f64,
}

/// The tunnel's line: how far to the right at depth `z`, metres, and how
/// fast that changes with depth. An S-bend, so its far end is never in view.
fn bend(z: f64) -> (f64, f64) {
    if z <= 0.0 {
        return (0.0, 0.0);
    }
    if z >= TUNNEL_LENGTH {
        return (BEND, 0.0);
    }
    let a = std::f64::consts::TAU * z / TUNNEL_LENGTH;
    (
        BEND * (z / TUNNEL_LENGTH - a.sin() / std::f64::consts::TAU),
        BEND / TUNNEL_LENGTH * (1.0 - a.cos()),
    )
}

/// A cubic from `p0` at speed `v0` to `p1` at speed `v1` over `length`
/// seconds: where it is `local` seconds in.
fn ease(p0: f64, v0: f64, p1: f64, v1: f64, length: f64, local: f64) -> f64 {
    let s = (local / length).clamp(0.0, 1.0);
    let (h00, h10, h01, h11) = (
        2.0 * s * s * s - 3.0 * s * s + 1.0,
        s * s * s - 2.0 * s * s + s,
        -2.0 * s * s * s + 3.0 * s * s,
        s * s * s - s * s,
    );
    h00 * p0 + h10 * v0 * length + h01 * p1 + h11 * v1 * length
}

/// Metres to the mouth `local` seconds into the final approach: falling
/// from `CORRIDOR` to nothing, reaching the mouth at `MOUTH_SPEED`.
fn closing(local: f64) -> f64 {
    let left = (FINAL - local).clamp(0.0, FINAL);
    MOUTH_SPEED * left + (CORRIDOR - MOUTH_SPEED * FINAL) * (left / FINAL).powi(3)
}

/// How fast the final approach begins, metres a second.
fn corridor_speed() -> f64 {
    MOUTH_SPEED + 3.0 * (CORRIDOR - MOUTH_SPEED * FINAL) / FINAL
}

/// From direction `a` toward `b`, a share `s` of the way, turning round
/// `fallback` when the two are opposite.
fn turn(a: [f64; 3], b: [f64; 3], s: f64, fallback: [f64; 3]) -> [f64; 3] {
    let c = dot(a, b).clamp(-1.0, 1.0);
    let mut axis = cross(a, b);
    if length(axis) < 1e-6 {
        axis = fallback;
    }
    let axis = normalize(axis);
    super::super::voyage::rotate(a, axis, c.acos() * s.clamp(0.0, 1.0))
}

/// Where the camera is and which way it looks, at a moment of a visit.
pub(super) struct Placed {
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
    pub inside: bool,
}

impl Tour {
    /// The starbase this visit docks at, if `stop` is it.
    fn base_at(&self, stop: Stop) -> &Base {
        self.base
            .as_ref()
            .filter(|b| b.stop == stop)
            .expect("a starbase stage at the starbase")
    }

    /// When the visit to the base begins.
    pub(super) fn base_start(&self, stop: Stop) -> f64 {
        self.start_of(|p| p == Phase::Base(stop, BaseStage::Approach))
            .unwrap_or(0.0)
    }

    /// When stage `stage` of the visit to the base begins and ends.
    fn base_span(&self, stop: Stop, stage: BaseStage) -> (f64, f64) {
        self.segments
            .iter()
            .find(|s| s.phase == Phase::Base(stop, stage))
            .map_or((0.0, 0.0), |s| (s.start, s.start + s.duration))
    }

    /// The orbit round the base, relative to its centre, and its velocity.
    fn orbit_rel(&self, stop: Stop, t: f64) -> [f64; 3] {
        sub(self.orbiting(stop, t).0, self.system.centre(stop, t))
    }
    fn orbit_velocity(&self, stop: Stop, t: f64) -> [f64; 3] {
        let h = 0.05;
        scale(
            sub(self.orbit_rel(stop, t + h), self.orbit_rel(stop, t - h)),
            0.5 / h,
        )
    }

    /// The base's frame: the mouth faces where the orbit is as the ship
    /// lines up on it, so it flies on and down into the corridor.
    fn base_frame(&self, base: &Base) -> Frame {
        let stop = base.stop;
        let t = self.base_start(stop) + base.align_at() + 0.7 * ALIGN;
        let (_, _, up) = self.orbiting(stop, t);
        let out = normalize(self.orbit_rel(stop, t));
        let up = normalize(sub(up, scale(out, dot(up, out))));
        let forward = scale(out, -1.0);
        Frame {
            out,
            up,
            right: normalize(cross(forward, up)),
            radius: self.world(stop).radius,
        }
    }

    /// A point inside the base, metres in its own frame, relative to its
    /// centre in the world; and a direction in its frame, in the world.
    fn inside_rel(frame: &Frame, p: [f64; 3]) -> [f64; 3] {
        add(
            scale(frame.out, frame.radius),
            scale(Self::inside_dir(frame, p), METRE),
        )
    }
    fn inside_dir(frame: &Frame, d: [f64; 3]) -> [f64; 3] {
        add(
            add(scale(frame.right, d[0]), scale(frame.up, d[1])),
            scale(frame.out, -d[2]),
        )
    }

    /// The camera `local` seconds into stage `stage` of the visit to the
    /// base at `stop` (`t` into the visit).
    pub(super) fn base_view(&self, stop: Stop, stage: BaseStage, local: f64, t: f64) -> Placed {
        let base = self.base_at(stop);
        let frame = self.base_frame(base);
        let centre = self.system.centre(stop, t);
        let inward = scale(frame.out, -1.0);
        let at = |rel: [f64; 3]| add(centre, rel);
        // In the base's frame: where, and which way it looks.
        let placed = |p: [f64; 3], d: [f64; 3]| Placed {
            position: at(Self::inside_rel(&frame, p)),
            forward: normalize(Self::inside_dir(&frame, d)),
            up: frame.up,
            inside: true,
        };
        let tunnel = |z: f64, sense: f64| {
            let (x, slope) = bend(z);
            placed([x, 0.0, z], [slope * sense, 0.0, sense])
        };
        let cradle_z = TUNNEL_LENGTH + CRADLE;
        match stage {
            BaseStage::Approach => {
                let align = base.align_at();
                if local < align {
                    // Calling in, cleared and perhaps holding: still in orbit.
                    let (position, forward, up) = self.orbiting(stop, t);
                    return Placed {
                        position,
                        forward,
                        up,
                        inside: false,
                    };
                }
                if local < align + ALIGN {
                    // Off the orbit, onto the corridor, turning to the mouth.
                    let t_a = self.base_start(stop) + align;
                    let s = (local - align) / ALIGN;
                    let p0 = self.orbit_rel(stop, t_a);
                    let v0 = scale(self.orbit_velocity(stop, t_a), ALIGN);
                    let p1 = scale(frame.out, frame.radius + CORRIDOR * METRE);
                    let v1 = scale(inward, corridor_speed() * METRE * ALIGN);
                    let (_, f0, u0) = self.orbiting(stop, t);
                    return Placed {
                        position: at(hermite(p0, v0, p1, v1, s)),
                        forward: turn(f0, inward, smoothstep(0.0, 0.7, s), frame.right),
                        up: normalize(turn(u0, frame.up, smoothstep(0.0, 1.0, s), frame.right)),
                        inside: false,
                    };
                }
                // Down the corridor's beacons to the mouth.
                let d = closing(local - align - ALIGN);
                Placed {
                    position: at(scale(frame.out, frame.radius + d * METRE)),
                    forward: inward,
                    up: frame.up,
                    inside: false,
                }
            }
            BaseStage::Tunnel => tunnel(
                ease(0.0, MOUTH_SPEED, TUNNEL_LENGTH, BAY_SPEED, TUNNEL, local),
                1.0,
            ),
            BaseStage::Berth => {
                let u = local / BERTH;
                let z = ease(TUNNEL_LENGTH, BAY_SPEED, cradle_z, 0.0, BERTH, local);
                placed(
                    [BEND, -CRADLE_DROP * smoothstep(0.5, 1.0, u), z],
                    [0.0, 0.0, 1.0],
                )
            }
            BaseStage::Service => placed([BEND, -CRADLE_DROP, cradle_z], [0.0, 0.0, 1.0]),
            BaseStage::Undock => {
                // Backed out of the cradle, then turned about, to the left.
                let u = local / UNDOCK;
                let back = smoothstep(0.0, 0.5, u);
                let yaw = std::f64::consts::PI * smoothstep(0.35, 1.0, u);
                placed(
                    [
                        BEND,
                        -CRADLE_DROP * (1.0 - smoothstep(0.0, 0.4, u)),
                        cradle_z - BACK_OUT * back,
                    ],
                    [-yaw.sin(), 0.0, yaw.cos()],
                )
            }
            BaseStage::Outbound => tunnel(
                ease(cradle_z - BACK_OUT, 0.0, 0.0, -EXIT_SPEED, OUTBOUND, local),
                -1.0,
            ),
            BaseStage::Climb => {
                // Out of the mouth and back into the orbit, as it would be
                // had the ship never left it.
                let (_, end) = self.base_span(stop, BaseStage::Climb);
                let s = local / CLIMB;
                let p0 = scale(frame.out, frame.radius);
                let v0 = scale(frame.out, EXIT_SPEED * METRE * CLIMB);
                let p1 = self.orbit_rel(stop, end);
                let v1 = scale(self.orbit_velocity(stop, end), CLIMB);
                let (_, f1, u1) = self.orbiting(stop, t);
                Placed {
                    position: at(hermite(p0, v0, p1, v1, s)),
                    forward: turn(frame.out, f1, smoothstep(0.25, 1.0, s), frame.right),
                    up: normalize(turn(frame.up, u1, smoothstep(0.0, 1.0, s), frame.right)),
                    inside: false,
                }
            }
        }
    }

    /// Metres to the mouth from where the camera is at `t`, outside.
    fn range_to_mouth(&self, base: &Base, t: f64) -> f64 {
        let frame = self.base_frame(base);
        let mouth = add(
            self.system.centre(base.stop, t),
            scale(frame.out, frame.radius),
        );
        length(sub(self.view(t).position, mouth)) / METRE
    }

    /// How fast the camera moves at `t`, metres a second.
    fn speed_at(&self, t: f64) -> f64 {
        let h = 0.05;
        length(sub(self.view(t + h).position, self.view(t - h).position)) / (2.0 * h) / METRE
    }

    /// What approach control says during step `step`, `into` seconds into
    /// it.
    fn atc_line(&self, base: &Base, step: Step, into: f64) -> String {
        let name = &base.name;
        match (step, base.traffic) {
            (Step::Contact, _) if into < CONTACT * 0.5 => {
                format!("“{name} approach, inbound to dock, requesting a bay.”")
            }
            (Step::Contact, _) => "“Read you. Stand by for a bay.”".into(),
            (Step::Clearance, None) => format!(
                "“Cleared to dock, bay {}. Corridor {}.”",
                base.bay, base.corridor
            ),
            (Step::Clearance, Some(ship)) => format!(
                "“Bay {} is yours, but hold your orbit: {} has priority.”",
                base.bay, ship.name
            ),
            (Step::Hold, Some(ship)) if into < HOLD * 0.6 => {
                format!("“{} going in ahead of you: {}.”", ship.name, ship.reason)
            }
            (Step::Hold, _) => "“Traffic is in. You are cleared, corridor's yours.”".into(),
            (Step::Align, _) => format!("“Line up on the corridor {} beacon.”", base.corridor),
            (Step::Final, _) => format!("“Guidance lights on. Welcome to {name}.”"),
        }
    }

    /// The docking panel `local` seconds into the way in.
    fn approach_panel(&self, base: &Base, local: f64, t: f64) -> Manoeuvre {
        let total = base.approach();
        let steps = base.steps();
        let step = base.step(local);
        let bay = base.bay;
        let stages = steps
            .iter()
            .map(|&(s, _)| {
                let (label, value) = match s {
                    Step::Contact => ("CONTACT", format!("hailing {} approach control", base.name)),
                    Step::Clearance => match base.traffic {
                        None => ("CLEARANCE", format!("cleared to dock, bay {bay}")),
                        Some(_) => ("CLEARANCE", format!("bay {bay}, after priority traffic")),
                    },
                    Step::Hold => {
                        let ship = base.traffic.expect("a hold for traffic");
                        ("HOLD", format!("{} in first, {}", ship.name, ship.class))
                    }
                    Step::Align => (
                        "ALIGNMENT",
                        format!("bow on the corridor {} beacon", base.corridor),
                    ),
                    Step::Final => (
                        "FINAL",
                        format!(
                            "{} km to the mouth",
                            thousands(self.range_to_mouth(base, t) / 1000.0)
                        ),
                    ),
                };
                (label.to_string(), value)
            })
            .collect();
        let tag = match step {
            Step::Contact => "HAIL",
            Step::Clearance => "CLEARED",
            Step::Hold => "HOLD",
            Step::Align => "ALIGN",
            Step::Final => "DOCK",
        };
        // The range closed, from the orbit's height to the mouth.
        let start = (Tour::height(self.world(base.stop)) - self.world(base.stop).radius) / METRE;
        let range = self.range_to_mouth(base, t);
        Manoeuvre {
            name: "DOCKING APPROACH".into(),
            text: format!("{} · bay {bay} · corridor {}", base.name, base.corridor),
            progress: (local / total) as f32,
            stages,
            starts: steps.iter().map(|&(_, s)| s / total).collect(),
            charge: (1.0 - range / start).clamp(0.0, 1.0) as f32,
            dock: true,
            tag: tag.into(),
            gauge: "RNG".into(),
            label: "BERTH".into(),
            hot: (steps.len() - 1) as u32,
            ..Default::default()
        }
    }

    /// The service panel `local` seconds into the service.
    fn service_panel(&self, base: &Base, local: f64) -> Manoeuvre {
        let u = local / SERVICE;
        let mut rng = Rng::new(self.system.seed ^ 0x5e41_11ce);
        let hull = rng.int(58, 91);
        let mass = rng.int(140, 620);
        let (_, drive) = self.drive();
        let stages = vec![
            (
                "HULL PLATING".to_string(),
                format!("welding the seams, {hull}% to 100%"),
            ),
            (
                "COOLANT".to_string(),
                "loops flushed and refilled".to_string(),
            ),
            (
                "REACTION MASS".to_string(),
                format!("{mass} t pumped aboard"),
            ),
            (
                "DRIVE COIL".to_string(),
                format!("the {drive} recalibrated"),
            ),
            (
                "NAV SYNC".to_string(),
                "charts and beacons brought up to date".to_string(),
            ),
        ];
        let tag = if u >= 0.8 { "READY" } else { "SERVICE" };
        Manoeuvre {
            name: "SERVICE".into(),
            text: format!("{} · bay {}", base.name, base.bay),
            progress: u as f32,
            stages,
            starts: vec![0.0, 0.2, 0.4, 0.6, 0.8],
            charge: u as f32,
            dock: true,
            tag: tag.into(),
            gauge: "SVC".into(),
            label: "BAY".into(),
            hot: 4,
            ..Default::default()
        }
    }

    /// The panel `local` seconds into stage `stage` at the base at `stop`.
    pub(super) fn base_manoeuvre(
        &self,
        stop: Stop,
        stage: BaseStage,
        local: f64,
        t: f64,
    ) -> Manoeuvre {
        let base = self.base_at(stop);
        let (start, end) = self.base_span(stop, stage);
        let u = (local / (end - start).max(1e-6)) as f32;
        let coast = |name: &str, text: String| Manoeuvre {
            name: name.into(),
            text,
            progress: u,
            ..Default::default()
        };
        let (name, bay, corridor) = (&base.name, base.bay, base.corridor);
        match stage {
            BaseStage::Approach => self.approach_panel(base, local, t),
            BaseStage::Tunnel => coast(
                "TRANSIT TUNNEL",
                format!(
                    "Down corridor {corridor} into {name}, the city's lights through the windows either side."
                ),
            ),
            BaseStage::Berth => coast(
                "BERTHING",
                format!("Guided into the cradle of bay {bay}, the clamps reaching for the hull."),
            ),
            BaseStage::Service => self.service_panel(base, local),
            BaseStage::Undock => coast(
                "UNDOCKING",
                format!(
                    "Clamps released: backing out of bay {bay} on thrusters and turning about."
                ),
            ),
            BaseStage::Outbound => coast(
                "OUTBOUND",
                format!("Back up corridor {corridor} to the mouth, cleared out by {name} control."),
            ),
            BaseStage::Climb => coast(
                "DEPARTURE",
                format!(
                    "Out of {name}, climbing back into orbit for {}.",
                    self.after_base(stop)
                ),
            ),
        }
    }

    /// Where the tour goes after the base.
    fn after_base(&self, stop: Stop) -> String {
        let (_, end) = self.base_span(stop, BaseStage::Climb);
        self.segments
            .iter()
            .find(|s| s.start >= end - 1e-6)
            .and_then(|s| match s.phase {
                Phase::Transfer { to, .. } => Some(self.world(to).name.clone()),
                _ => None,
            })
            .unwrap_or_else(|| format!("the jump to {}", self.next.star.name))
    }

    /// The HUD `local` seconds into stage `stage` at the base at `stop`.
    pub(super) fn base_hud(
        &self,
        stop: Stop,
        stage: BaseStage,
        local: f64,
        t: f64,
    ) -> (String, Vec<(String, String)>) {
        let base = self.base_at(stop);
        let (_, end) = self.base_span(stop, stage);
        let left = end - t;
        let bay = format!("bay {} · corridor {}", base.bay, base.corridor);
        let speed = || format!("{} m/s", thousands(self.speed_at(t)));
        let name = ("Starbase".to_string(), base.name.clone());
        match stage {
            BaseStage::Approach => {
                let step = base.step(local);
                let into = local
                    - base
                        .steps()
                        .into_iter()
                        .find(|&(s, _)| s == step)
                        .map_or(0.0, |(_, at)| at);
                let giant = &self.system.planets[stop.planet].name;
                let across = self.world(stop).radius * 2.0 * KM;
                let traffic = match base.traffic {
                    None => "clear".to_string(),
                    Some(ship) => match step {
                        Step::Contact | Step::Clearance => {
                            format!("{} · {} inbound", ship.name, ship.class)
                        }
                        Step::Hold => format!("{} entering ahead · {}", ship.name, ship.reason),
                        _ => format!("{} docked", ship.name),
                    },
                };
                (
                    "APPROACH CONTROL".into(),
                    vec![
                        name,
                        (
                            "".into(),
                            format!(
                                "Starbase · {} km across · orbits {giant}",
                                thousands(across)
                            ),
                        ),
                        (
                            "Range".into(),
                            format!(
                                "{} km to the mouth",
                                thousands(self.range_to_mouth(base, t) / 1000.0)
                            ),
                        ),
                        ("Berth".into(), bay),
                        ("Traffic".into(), traffic),
                        ("ATC".into(), self.atc_line(base, step, into)),
                    ],
                )
            }
            BaseStage::Tunnel => {
                let z = ease(0.0, MOUTH_SPEED, TUNNEL_LENGTH, BAY_SPEED, TUNNEL, local);
                (
                    "TRANSIT TUNNEL".into(),
                    vec![
                        name,
                        ("Depth".into(), format!("{} m", thousands(z))),
                        ("Speed".into(), speed()),
                        ("Berth".into(), bay),
                        ("Docking in".into(), clock(left + BERTH)),
                    ],
                )
            }
            BaseStage::Berth => (
                "BERTHING".into(),
                vec![
                    name,
                    ("Berth".into(), bay),
                    ("Speed".into(), speed()),
                    (
                        "Clamps".into(),
                        if left < 1.5 {
                            "locked".into()
                        } else {
                            "open".into()
                        },
                    ),
                ],
            ),
            BaseStage::Service => {
                let done = ((local / SERVICE) * 5.0).floor().min(5.0) as u32;
                (
                    "DOCKED".into(),
                    vec![
                        name,
                        ("Berth".into(), bay),
                        ("Service".into(), format!("{done} of 5 done")),
                        ("Undock in".into(), clock(left)),
                    ],
                )
            }
            BaseStage::Undock => (
                "UNDOCKING".into(),
                vec![
                    name,
                    ("Berth".into(), bay),
                    ("Cleared by".into(), format!("{} control", base.name)),
                ],
            ),
            BaseStage::Outbound => {
                let z = ease(
                    TUNNEL_LENGTH + CRADLE - BACK_OUT,
                    0.0,
                    0.0,
                    -EXIT_SPEED,
                    OUTBOUND,
                    local,
                );
                (
                    "OUTBOUND TUNNEL".into(),
                    vec![
                        name,
                        ("Mouth in".into(), format!("{} m", thousands(z.max(0.0)))),
                        ("Speed".into(), speed()),
                    ],
                )
            }
            BaseStage::Climb => (
                "DEPARTURE".into(),
                vec![
                    ("Leaving".into(), base.name.clone()),
                    ("Next".into(), self.after_base(stop)),
                    ("Speed".into(), speed()),
                ],
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base(traffic: Option<Traffic>) -> Base {
        let system = System::generate(5);
        let stop = Stop {
            planet: 0,
            moon: None,
        };
        Base::with_traffic(&system, stop, 7, 2, traffic)
    }

    #[test]
    fn the_way_in_holds_only_for_traffic() {
        let clear = base(None);
        let held = base(Some(TRAFFIC[0]));
        assert!(!clear.steps().iter().any(|&(s, _)| s == Step::Hold));
        assert!(held.steps().iter().any(|&(s, _)| s == Step::Hold));
        assert!((held.approach() - clear.approach() - HOLD).abs() < 1e-9);
        assert_eq!(held.step(CONTACT + CLEARANCE + 1.0), Step::Hold);
        assert_eq!(clear.step(CONTACT + CLEARANCE + 1.0), Step::Align);
        assert_eq!(clear.step(clear.approach() - 1.0), Step::Final);
        let total: f64 = clear.stages().iter().map(|&(_, d)| d).sum();
        assert!((total - clear.duration()).abs() < 1e-9);
    }

    #[test]
    fn the_corridor_reaches_the_mouth_at_speed() {
        assert!((closing(0.0) - CORRIDOR).abs() < 1e-6);
        assert!(closing(FINAL).abs() < 1e-6);
        let h = 1e-4;
        let speed = (closing(FINAL - h) - closing(FINAL)) / h;
        assert!((speed - MOUTH_SPEED).abs() < 1.0, "{speed}");
        let start = (closing(0.0) - closing(h)) / h;
        assert!((start - corridor_speed()).abs() < 2.0, "{start}");
        // The tunnel's bend starts and ends straight.
        assert_eq!(bend(0.0), (0.0, 0.0));
        assert!(bend(TUNNEL_LENGTH * 0.5).1 > 0.0);
        assert!((bend(TUNNEL_LENGTH - 1e-6).0 - BEND).abs() < 1e-3);
    }
}
