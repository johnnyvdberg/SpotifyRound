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
use super::{Phase, Tour, add, cross, dot, hermite, length, normalize, scale, smoothstep, sub};

mod hud;

/// The steps of the way in, seconds each: the call to approach control,
/// the clearance, the hold (only when traffic has priority), lining up on
/// the corridor and the final approach to the mouth.
pub const CONTACT: f64 = 8.0;
pub const CLEARANCE: f64 = 7.0;
pub const HOLD: f64 = 18.0;
pub const ALIGN: f64 = 12.0;
pub const FINAL: f64 = 12.0;
/// The rest of the visit, seconds each.
/// The airlock: down the entrance to a stop in the lock, the outer door
/// shut behind and the inner one opened ahead.
pub const AIRLOCK: f64 = 14.0;
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
/// The shaft from the mouth down to the tunnel, metres deep; its width at
/// the mouth is `FUNNEL_TOP` across the middle.
pub const FUNNEL: f64 = 1000.0;
pub const FUNNEL_TOP: f64 = 1000.0;
/// The tunnel: segments of `SEGMENT` metres, `SEGMENTS` of them, round a
/// bore `BORE` metres in radius; how far its S-bend carries it to the side.
pub const SEGMENT: f64 = 160.0;
pub const SEGMENTS: u32 = 38;
pub const TUNNEL_LENGTH: f64 = SEGMENT * SEGMENTS as f64;
pub const BORE: f64 = 120.0;
pub const BEND: f64 = 400.0;
/// The airlock, at the head of the tunnel: the segments it takes the place
/// of, and where the ship stops in it, metres in from the mouth.
pub const LOCK_SEGMENTS: u32 = 2;
pub const LOCK: f64 = FUNNEL + SEGMENT * LOCK_SEGMENTS as f64 * 0.5;
/// How high over its world the base's mouth stands, metres: the top of
/// the megastructure it is in (`starbase::mesh::megastructure`).
pub const ELEVATION: f64 = 30_000.0;
/// How fast the ship comes out of the tunnel into the bay, metres a second.
const BAY_SPEED: f64 = 60.0;
/// The cradle: this far into the bay past the tunnel's end, and this far
/// below its line, metres; how far the ship backs out of it.
pub const CRADLE: f64 = 520.0;
pub const CRADLE_DROP: f64 = 15.0;
pub const BACK_OUT: f64 = 60.0;
/// The base's surface round the mouth as a mesh, this far out from it,
/// metres: armour plating and the blocks on it (`starbase::mesh::city`).
pub const CITY: f64 = 32_000.0;
/// On the way in the base's own meshes are all that is drawn (and the
/// body they stand on not) from this many metres out from the mouth: the
/// whole final approach, through which that surface fills the view, so the
/// body under it is never paid for as well. On the way out, until this far.
const INSIDE_FROM: f64 = CORRIDOR + 2000.0;
const INSIDE_UNTIL: f64 = 2000.0;
/// How fast the ship leaves through the mouth, metres a second.
const EXIT_SPEED: f64 = 400.0;

/// Where in a starbase visit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BaseStage {
    /// Calling in, cleared (or holding), lining up and flying the corridor
    /// to the mouth: one segment, in steps.
    Approach,
    /// Down the entrance into the airlock, and through it.
    Airlock,
    /// Along the tunnel, the world inside through its windows.
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
    pub const ALL: [BaseStage; 8] = [
        BaseStage::Approach,
        BaseStage::Airlock,
        BaseStage::Tunnel,
        BaseStage::Berth,
        BaseStage::Service,
        BaseStage::Undock,
        BaseStage::Outbound,
        BaseStage::Climb,
    ];

    /// Inside the base, from the mouth in and back out to it.
    pub fn inner(self) -> bool {
        !matches!(self, BaseStage::Approach | BaseStage::Climb)
    }

    /// A short name, for screenshots and tests.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            BaseStage::Approach => "approach",
            BaseStage::Airlock => "airlock",
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
                    BaseStage::Airlock => AIRLOCK,
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

/// Where the tunnel ends and the bay begins, metres in from the mouth.
pub const fn tunnel_end() -> f64 {
    FUNNEL + TUNNEL_LENGTH
}

/// The tunnel's line: how far to the right at depth `z`, metres, and how
/// fast that changes with depth. Straight down the funnel, then an S-bend
/// along the tunnel, so its far end is never in view.
pub fn bend(z: f64) -> (f64, f64) {
    let z = z - FUNNEL;
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

/// Seconds into the airlock stage: the ship stopped in the lock, the outer
/// door shut, and the inner door begun to open and open.
const LOCK_STOPPED: f64 = 7.0;
const LOCK_SHUT: f64 = 9.0;
const LOCK_OPENING: f64 = 10.5;

/// Metres in from the mouth `local` seconds into the airlock stage: down
/// the entrance, slowing to a stop in the lock.
pub(super) fn lock_depth(local: f64) -> f64 {
    ease(0.0, MOUTH_SPEED, LOCK, 0.0, LOCK_STOPPED, local)
}

/// How far open the airlock's doors are, 0..1, the outer and the inner:
/// the outer open and the inner shut until the ship is in; shut behind it,
/// then the inner opened, and so through the visit; both open for the way
/// out.
pub fn doors(stage: BaseStage, local: f64) -> [f64; 2] {
    match stage {
        BaseStage::Approach | BaseStage::Climb => [1.0, 0.0],
        BaseStage::Airlock => [
            1.0 - smoothstep(LOCK_STOPPED - 1.0, LOCK_SHUT, local),
            smoothstep(LOCK_OPENING, AIRLOCK - 0.5, local),
        ],
        BaseStage::Outbound => [1.0, 1.0],
        _ => [0.0, 1.0],
    }
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

/// Where the camera is and which way it looks.
struct Pose {
    position: [f64; 3],
    forward: [f64; 3],
    up: [f64; 3],
}

/// Where the camera is and which way it looks, at a moment of a visit.
pub(super) struct Placed {
    pub position: [f64; 3],
    pub forward: [f64; 3],
    pub up: [f64; 3],
    pub scene: BaseScene,
}

/// A moment of a visit to a starbase as its interior is drawn
/// (`interior.rs`): where the camera is in the base's frame, metres (the
/// mouth at the origin, +z in, +y up, +x right), and that frame's axes in
/// the world; the stage, how far into it and into the whole visit; whether
/// the interior is drawn in place of the system; and what moves in it.
#[derive(Clone, Copy, Debug)]
pub struct BaseScene {
    pub ship: [f64; 3],
    /// Which way the ship points, in the base's frame.
    pub heading: [f64; 3],
    pub basis: [[f64; 3]; 3],
    pub stage: BaseStage,
    pub local: f64,
    pub clock: f64,
    pub inside: bool,
    /// Whether the sky shows from inside: looking out through the mouth.
    pub sky: bool,
    /// The way to the star, in the base's frame.
    pub sun: [f64; 3],
    pub seed: u64,
    /// The ship given priority, while it is seen going in: where (metres,
    /// in the base's frame) and how brightly its engines show, 0..1.
    pub traffic: Option<[f64; 4]>,
    /// The hull's shudder as the clamps close and let go, 0..1.
    pub shake: f64,
    /// How far open the airlock's doors are, the outer and the inner.
    pub doors: [f64; 2],
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
            radius: self.world(stop).radius + ELEVATION * METRE,
        }
    }

    /// The starbase's attitude, held still rather than turning as a moon
    /// does: rows turning world directions into its body's frame, in which
    /// its maps lie and its mouth faces +z (`MOUTH` in `generate.wesl`).
    /// The mouth faces the corridor the ship flies in by, its up the base's.
    pub fn base_attitude(&self) -> Option<(Stop, [[f64; 3]; 3])> {
        let base = self.base.as_ref()?;
        let frame = self.base_frame(base);
        Some((
            base.stop,
            [normalize(cross(frame.up, frame.out)), frame.up, frame.out],
        ))
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
    fn base_place(&self, stop: Stop, stage: BaseStage, local: f64, t: f64) -> Pose {
        let base = self.base_at(stop);
        let frame = self.base_frame(base);
        let centre = self.system.centre(stop, t);
        let inward = scale(frame.out, -1.0);
        let at = |rel: [f64; 3]| add(centre, rel);
        // In the base's frame: where, and which way it looks.
        let placed = |p: [f64; 3], d: [f64; 3]| Pose {
            position: at(Self::inside_rel(&frame, p)),
            forward: normalize(Self::inside_dir(&frame, d)),
            up: frame.up,
        };
        let tunnel = |z: f64, sense: f64| {
            let (x, slope) = bend(z);
            placed([x, 0.0, z], [slope * sense, 0.0, sense])
        };
        let cradle_z = tunnel_end() + CRADLE;
        match stage {
            BaseStage::Approach => {
                let align = base.align_at();
                if local < align {
                    // Calling in, cleared and perhaps holding: still in orbit.
                    let (position, forward, up) = self.orbiting(stop, t);
                    return Pose {
                        position,
                        forward,
                        up,
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
                    return Pose {
                        position: at(hermite(p0, v0, p1, v1, s)),
                        forward: turn(f0, inward, smoothstep(0.0, 0.7, s), frame.right),
                        up: normalize(turn(u0, frame.up, smoothstep(0.0, 1.0, s), frame.right)),
                    };
                }
                // Down the corridor's beacons to the mouth.
                let d = closing(local - align - ALIGN);
                Pose {
                    position: at(scale(frame.out, frame.radius + d * METRE)),
                    forward: inward,
                    up: frame.up,
                }
            }
            BaseStage::Airlock => tunnel(lock_depth(local), 1.0),
            BaseStage::Tunnel => {
                tunnel(ease(LOCK, 0.0, tunnel_end(), BAY_SPEED, TUNNEL, local), 1.0)
            }
            BaseStage::Berth => {
                let u = local / BERTH;
                let z = ease(tunnel_end(), BAY_SPEED, cradle_z, 0.0, BERTH, local);
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
                Pose {
                    position: at(hermite(p0, v0, p1, v1, s)),
                    forward: turn(frame.out, f1, smoothstep(0.25, 1.0, s), frame.right),
                    up: normalize(turn(frame.up, u1, smoothstep(0.0, 1.0, s), frame.right)),
                }
            }
        }
    }

    /// The camera `local` seconds into stage `stage` of the visit to the
    /// base at `stop` (`t` into the visit), and the interior as drawn then.
    pub(super) fn base_view(&self, stop: Stop, stage: BaseStage, local: f64, t: f64) -> Placed {
        let base = self.base_at(stop);
        let frame = self.base_frame(base);
        let pose = self.base_place(stop, stage, local, t);
        let ship = self.to_base(&frame, base.stop, pose.position, t);
        // Inside from where the funnel fills the view, down and back up.
        let reach = if stage == BaseStage::Climb {
            INSIDE_UNTIL
        } else {
            INSIDE_FROM
        };
        let inside = stage.inner()
            || (ship[2] > -reach && ship[0].hypot(ship[1]) < FUNNEL_TOP + 0.2 * ship[2].abs());
        let heading = {
            let f = pose.forward;
            [dot(f, frame.right), dot(f, frame.up), -dot(f, frame.out)]
        };
        let centre = self.system.centre(stop, t);
        let to_star = normalize(scale(centre, -1.0));
        let sun = [
            dot(to_star, frame.right),
            dot(to_star, frame.up),
            -dot(to_star, frame.out),
        ];
        let start = self.base_start(stop);
        // The shudder of the clamps closing at the end of the berthing, and
        // letting go as the undocking begins.
        let thud = |at: f64| {
            let x = local - at;
            if x < 0.0 { 0.0 } else { (-x / 0.35).exp() }
        };
        let shake = match stage {
            BaseStage::Berth => thud(BERTH - 1.2),
            BaseStage::Undock => 0.6 * thud(0.3),
            _ => 0.0,
        };
        Placed {
            position: pose.position,
            forward: pose.forward,
            up: pose.up,
            scene: BaseScene {
                ship,
                heading,
                basis: [frame.right, frame.up, scale(frame.out, -1.0)],
                stage,
                local,
                clock: t - start,
                inside,
                sky: inside
                    && heading[2] < 0.3
                    && ship[2] < FUNNEL + 1500.0
                    && doors(stage, local)[0] > 0.0,
                sun,
                seed: self.system.seed,
                traffic: self.traffic_at(base, &frame, stage, local, t),
                shake,
                doors: doors(stage, local),
            },
        }
    }

    /// A point in the world at `t`, in the base's frame (metres from its
    /// mouth).
    fn to_base(&self, frame: &Frame, stop: Stop, p: [f64; 3], t: f64) -> [f64; 3] {
        let mouth = add(self.system.centre(stop, t), scale(frame.out, frame.radius));
        let rel = scale(sub(p, mouth), 1.0 / METRE);
        [
            dot(rel, frame.right),
            dot(rel, frame.up),
            -dot(rel, frame.out),
        ]
    }

    /// The ship given priority, while it is seen: through the hold, from
    /// halfway between the camera and the mouth to the corridor's start and
    /// down it into the mouth, its engines fading as it goes in.
    fn traffic_at(
        &self,
        base: &Base,
        frame: &Frame,
        stage: BaseStage,
        local: f64,
        t: f64,
    ) -> Option<[f64; 4]> {
        base.traffic?;
        if stage != BaseStage::Approach {
            return None;
        }
        let hold = base
            .steps()
            .into_iter()
            .find(|&(s, _)| s == Step::Hold)
            .map(|(_, at)| at)?;
        let u = (local - hold) / (HOLD * 0.9);
        if !(-0.15..=1.0).contains(&u) {
            return None;
        }
        let start = self.base_start(base.stop);
        let seen_from = self.to_base(
            frame,
            base.stop,
            self.orbiting(base.stop, start + hold).0,
            t,
        );
        let from = scale(seen_from, 0.5);
        let corridor = [0.0, 0.0, -CORRIDOR];
        let at = if u < 0.6 {
            let s = smoothstep(0.0, 0.6, u.max(0.0));
            add(scale(from, 1.0 - s), scale(corridor, s))
        } else {
            let s = smoothstep(0.6, 1.0, u);
            scale(corridor, 1.0 - s)
        };
        let shown = smoothstep(-0.15, 0.0, u) * (1.0 - smoothstep(0.9, 1.0, u));
        Some([at[0], at[1], at[2], shown])
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
        assert_eq!(bend(FUNNEL), (0.0, 0.0));
        assert!(bend(FUNNEL + TUNNEL_LENGTH * 0.5).1 > 0.0);
        assert!((bend(tunnel_end() - 1e-6).0 - BEND).abs() < 1e-3);
    }
}
