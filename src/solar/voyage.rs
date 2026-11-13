//! The voyage between two stars: the hyperjump broken into short legs, with
//! something met between each two.
//!
//! A jump used to be two and a half minutes of the drive's light. Now it is
//! two or three legs of under half a minute, and between them the ship is
//! either torn out of the jump into an encounter (an asteroid field, a hive
//! cube, a black hole and the rest of [`Event`]), flies a star gate, or dives
//! into a wormhole. After an encounter the drive re-engages in a quick spool
//! of its own; a gate or a wormhole throws the ship into its next leg itself,
//! down a conduit or a throat with a look of its own (`GATE_LOOK`,
//! `THROAT_LOOK`). Now and then something happens inside a leg too, seen
//! through the drive's light ([`Flux`]).
//!
//! Each stopover is scripted here in its own space, the encounter frame: the
//! ship drops out at the origin heading along +z, the way the jump goes, +y
//! up and +x to the right. A script gives, at any moment, where the ship is
//! in that frame, which way it looks, the objects the shaders draw (in
//! `encounter.wgsl`) and the alarms on the HUD; like the rest of the tour, a
//! pure function of the time, so any moment can be shown on its own.
//!
//! Which encounter comes up is dealt from a shuffled deck of all of them, a
//! deck after another, so every one comes round and none twice in a row.

use std::f64::consts::{PI, TAU};

use super::system::Rng;

/// A leg of the jump: the last of a trip always this long, so the drive's
/// light slows into the next system as it always did (`DROP_OUT`).
pub const LEG: f64 = 28.0;
/// A leg when the trip has two stopovers.
pub const SHORT_LEG: f64 = 22.0;
/// A leg cut short by a shortcut through the drive's light (`Flux::Shortcut`).
pub const SHORTCUT_LEG: f64 = 17.0;
/// The drive re-engaging after an encounter, and after breaking away from a
/// hive cube.
pub const REJUMP: f64 = 9.0;
pub const EMERGENCY: f64 = 5.5;
/// The chance a trip has a second stopover.
pub const SECOND_STOP: f64 = 0.3;
/// The chance a leg has something seen from inside the jump.
pub const FLUX_CHANCE: f64 = 0.45;

/// Every look of the light in a jump: the five drives, then the conduit a
/// star gate throws the ship down and a wormhole's throat.
pub const LOOKS: [&str; 7] = [
    "Lumen Drift",
    "Tachyon Tunnel",
    "Fold Drive",
    "Ion Slipstream",
    "Bubble Drive",
    "Gate Conduit",
    "Wormhole Throat",
];
pub const GATE_LOOK: u32 = 6;
pub const THROAT_LOOK: u32 = 7;

/// What the ship meets between two legs of a jump.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    Asteroids = 1,
    HiveCube,
    BlackHole,
    Nebula,
    RoguePlanet,
    Pulsar,
    Derelict,
    Comet,
    Leviathans,
    Remnant,
    Gate,
    Wormhole,
    Battle,
}

impl Event {
    pub const ALL: [Event; 13] = [
        Event::Asteroids,
        Event::HiveCube,
        Event::BlackHole,
        Event::Nebula,
        Event::RoguePlanet,
        Event::Pulsar,
        Event::Derelict,
        Event::Comet,
        Event::Leviathans,
        Event::Remnant,
        Event::Gate,
        Event::Wormhole,
        Event::Battle,
    ];

    /// Its number in the shaders (`EVENT` in `encounter.wgsl`), from 1.
    pub fn id(self) -> u32 {
        self as u32
    }

    /// A short name, for files and `SOLAR_ENCOUNTER`.
    pub fn slug(self) -> &'static str {
        match self {
            Event::Asteroids => "asteroids",
            Event::HiveCube => "hive-cube",
            Event::BlackHole => "black-hole",
            Event::Nebula => "nebula",
            Event::RoguePlanet => "rogue-planet",
            Event::Pulsar => "pulsar",
            Event::Derelict => "derelict",
            Event::Comet => "comet",
            Event::Leviathans => "leviathans",
            Event::Remnant => "remnant",
            Event::Gate => "gate",
            Event::Wormhole => "wormhole",
            Event::Battle => "battle",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|e| e.slug() == slug)
    }

    /// Seconds from dropping out (or reaching it) to the drive re-engaging
    /// (or being thrown on).
    pub fn duration(self) -> f64 {
        match self {
            Event::Asteroids => 30.0,
            Event::HiveCube => 34.0,
            Event::BlackHole => 32.0,
            Event::Nebula => 28.0,
            Event::RoguePlanet => 30.0,
            Event::Pulsar => 28.0,
            Event::Derelict => 32.0,
            Event::Comet => 28.0,
            Event::Leviathans => 32.0,
            Event::Remnant => 30.0,
            Event::Gate => 26.0,
            Event::Wormhole => 24.0,
            Event::Battle => 32.0,
        }
    }

    /// A star gate or a wormhole: it throws the ship on itself, no drive.
    pub fn portal(self) -> bool {
        matches!(self, Event::Gate | Event::Wormhole)
    }

    /// How long the drive takes to re-engage after it.
    pub fn rejump(self) -> f64 {
        match self {
            Event::Gate | Event::Wormhole => 0.0,
            Event::HiveCube | Event::Battle => EMERGENCY,
            _ => REJUMP,
        }
    }

    /// The pipelines that draw it (see `Solar::draw`): whether it replaces
    /// the sky (lensing it), and whether it adds rocks or debris.
    pub fn replaces_sky(self) -> bool {
        matches!(self, Event::BlackHole)
    }

    pub fn rocks(self) -> bool {
        matches!(
            self,
            Event::Asteroids | Event::Derelict | Event::Comet | Event::Battle
        )
    }
}

/// Something seen from inside a leg of the jump, through the drive's light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flux {
    IonStorm = 1,
    Convoy,
    Echo,
    Shortcut,
}

impl Flux {
    pub const ALL: [Flux; 4] = [Flux::IonStorm, Flux::Convoy, Flux::Echo, Flux::Shortcut];

    pub fn id(self) -> u32 {
        self as u32
    }

    pub fn slug(self) -> &'static str {
        match self {
            Flux::IonStorm => "ion-storm",
            Flux::Convoy => "convoy",
            Flux::Echo => "echo",
            Flux::Shortcut => "shortcut",
        }
    }
}

/// One leg of the jump: its light, how long it lasts, what is seen inside it,
/// and its share of the trip's light years.
#[derive(Clone, Copy, Debug)]
pub struct Leg {
    pub look: u32,
    pub duration: f64,
    pub flux: Option<Flux>,
    pub share: f64,
}

impl Leg {
    /// How much of its flux shows `local` seconds into it, 0..1: rising
    /// after the leg's start, fading before its end.
    pub fn flux_amount(&self, local: f64) -> f64 {
        if self.flux.is_none() {
            return 0.0;
        }
        let u = local / self.duration;
        smoothstep(0.12, 0.28, u) * (1.0 - smoothstep(0.78, 0.92, u))
    }
}

/// What the ship meets between two legs, and the draw that makes this one
/// its own (a hive cube's size and colour, a gate's design, …).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stopover {
    pub event: Event,
    pub variant: u64,
}

/// The trip's whole jump: legs, and the stopovers between them.
#[derive(Clone, Debug)]
pub struct Passage {
    pub legs: Vec<Leg>,
    pub stops: Vec<Stopover>,
}

/// Which encounter is dealt `n`th from the decks shuffled from `base`.
fn dealt(base: u64, n: u64) -> Event {
    let count = Event::ALL.len() as u64;
    let deck = |d: u64| {
        let mut rng = Rng::new(base ^ d.wrapping_mul(0xa24b_aed4_963e_e407) ^ 0x0dec);
        let mut cards = Event::ALL;
        for i in (1..cards.len()).rev() {
            let j = (rng.next() % (i as u64 + 1)) as usize;
            cards.swap(i, j);
        }
        cards
    };
    let (d, k) = (n / count, (n % count) as usize);
    let mut cards = deck(d);
    // Never the last of one deck again first in the next.
    if d > 0 && cards[0] == deck(d - 1)[Event::ALL.len() - 1] {
        cards.swap(0, Event::ALL.len() / 2);
    }
    cards[k]
}

/// An encounter forced for every stopover, for the screenshots:
/// `SOLAR_ENCOUNTER=<slug>` (see [`Event::slug`]).
fn forced() -> Option<Event> {
    std::env::var("SOLAR_ENCOUNTER")
        .ok()
        .and_then(|s| Event::from_slug(&s))
}

impl Passage {
    /// The jump of trip `trip` (a visit's number) of the tour seeded `base`,
    /// with drive `drive`.
    pub fn plan(base: u64, trip: u64, drive: u32) -> Self {
        let mut rng = Rng::new(base ^ trip.wrapping_mul(0x2545_f491_4f6c_dd1d) ^ 0x7015);
        let count = if rng.chance(SECOND_STOP) { 2 } else { 1 };
        let stops: Vec<Stopover> = (0..count)
            .map(|k| Stopover {
                event: forced().unwrap_or_else(|| dealt(base, trip * 2 + k)),
                variant: rng.next(),
            })
            .collect();
        Self::with_stops(stops, drive, &mut rng)
    }

    /// A passage through exactly these stopovers.
    pub fn with_stops(stops: Vec<Stopover>, drive: u32, rng: &mut Rng) -> Self {
        let count = stops.len();
        let mut legs = Vec::with_capacity(count + 1);
        for i in 0..=count {
            let look = match i.checked_sub(1).map(|k| stops[k].event) {
                Some(Event::Gate) => GATE_LOOK,
                Some(Event::Wormhole) => THROAT_LOOK,
                _ => drive,
            };
            let last = i == count;
            let mut duration = if last || count < 2 { LEG } else { SHORT_LEG };
            let flux = rng.chance(FLUX_CHANCE).then(|| {
                // A shortcut cuts a leg short, so never the last.
                let kinds: &[Flux] = if last { &Flux::ALL[..3] } else { &Flux::ALL };
                *rng.pick(kinds)
            });
            if flux == Some(Flux::Shortcut) {
                duration = SHORTCUT_LEG;
            }
            legs.push(Leg {
                look,
                duration,
                flux,
                share: 0.0,
            });
        }
        // Light years shared by time, a shortcut's leg covering half as much
        // again as its time would.
        let weight = |l: &Leg| {
            l.duration
                * if l.flux == Some(Flux::Shortcut) {
                    2.6
                } else {
                    1.0
                }
        };
        let total: f64 = legs.iter().map(weight).sum();
        for l in &mut legs {
            l.share = weight(l) / total;
        }
        Passage { legs, stops }
    }

    /// The look of the light the trip arrives in.
    pub fn arrival_look(&self) -> u32 {
        self.legs.last().map_or(1, |l| l.look)
    }

    /// Seconds from the first leg to the last's end.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn duration(&self) -> f64 {
        self.legs.iter().map(|l| l.duration).sum::<f64>()
            + self
                .stops
                .iter()
                .map(|s| s.event.duration() + s.event.rejump())
                .sum::<f64>()
    }
}

/// A stopover's moment, as the shaders and the camera need it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Shot {
    /// The ship in the encounter frame, and which way it looks; the bank
    /// round that way, radians.
    pub ship: [f64; 3],
    pub look: [f64; 3],
    pub roll: f64,
    /// The alarm, 0..1: amber from 0.3, red from 0.7.
    pub alert: f64,
    /// A hit on the shield, 0..1 as it flares, and where on the frame
    /// (across the half frame).
    pub hit: f64,
    pub hit_at: [f64; 2],
    /// A tractor beam's hold, 0..1.
    pub beam: f64,
    /// A white flash over everything, 0..1.
    pub flash: f64,
    /// How hard the hull shakes, 0..1.
    pub shake: f64,
    /// The objects drawn, each a place and a size; a rotation (axis, angle);
    /// and four numbers each script uses its own way.
    pub objects: [[f64; 4]; 3],
    pub rot: [f64; 4],
    pub extra: [f64; 4],
}

// Small helpers, in f64.
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
fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}
fn normalize(a: [f64; 3]) -> [f64; 3] {
    scale(a, 1.0 / length(a).max(1e-12))
}
/// Directions `a` to `b`, a share `t` of the way: less of the way the
/// farther behind `b` is, so the camera never whips round to look back.
fn blend(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let t = t * smoothstep(-0.5, 0.3, dot(a, b));
    normalize(add(scale(a, 1.0 - t), scale(b, t)))
}
pub(crate) fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}
/// A pulse that rises over `rise` seconds at `at` and decays over `fall`.
fn pulse(local: f64, at: f64, rise: f64, fall: f64) -> f64 {
    if local < at - rise {
        0.0
    } else if local < at {
        smoothstep(at - rise, at, local)
    } else {
        (-(local - at) / fall).exp()
    }
}
/// `v` turned round unit `axis` by `angle`.
pub(crate) fn rotate(v: [f64; 3], axis: [f64; 3], angle: f64) -> [f64; 3] {
    let (s, c) = angle.sin_cos();
    add(
        add(scale(v, c), scale(cross(axis, v), s)),
        scale(axis, dot(axis, v) * (1.0 - c)),
    )
}
const AHEAD: [f64; 3] = [0.0, 0.0, 1.0];

/// What the ship sees of `stop` `local` seconds after reaching it; on past
/// its duration, through the drive re-engaging, turning back onto the
/// heading.
pub fn shot(stop: &Stopover, local: f64) -> Shot {
    let duration = stop.event.duration();
    let mut rng = Rng::new(stop.variant);
    let mut shot = match stop.event {
        Event::Asteroids => asteroids(&mut rng, local, duration),
        Event::HiveCube => hive_cube(&mut rng, local, duration),
        Event::BlackHole => black_hole(&mut rng, local, duration),
        Event::Nebula => nebula(&mut rng, local, duration),
        Event::RoguePlanet => rogue_planet(&mut rng, local, duration),
        Event::Pulsar => pulsar(&mut rng, local, duration),
        Event::Derelict => derelict(&mut rng, local, duration),
        Event::Comet => comet(&mut rng, local, duration),
        Event::Leviathans => leviathans(&mut rng, local, duration),
        Event::Remnant => remnant(&mut rng, local, duration),
        Event::Gate => gate(&mut rng, local, duration),
        Event::Wormhole => wormhole(&mut rng, local, duration),
        Event::Battle => battle(&mut rng, local, duration),
    };
    // Past the stopover, the drive re-engaging: back onto the heading, the
    // star the jump makes for, over its first seconds.
    if local > duration && !stop.event.portal() {
        let back = smoothstep(duration, duration + 0.4 * stop.event.rejump(), local);
        shot.look = blend(shot.look, AHEAD, back);
        shot.roll *= 1.0 - back;
    }
    shot
}

/// An asteroid field: the ship threading it, dodging three big rocks close
/// by, small ones pinging off the shield.
fn asteroids(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let speed = 55.0;
    let kind = rng.int(0, 2) as f64;
    let density = rng.range(0.65, 1.0);
    let light = [
        rng.range(-1.0, 1.0),
        rng.range(0.2, 0.9),
        rng.range(-0.3, 0.8),
    ];
    let mut objects = [[0.0; 4]; 3];
    let mut dodge = [0.0; 3];
    let mut alert: f64 = 0.15;
    let mut gaze = AHEAD;
    for (k, at) in [0.3, 0.56, 0.8].into_iter().enumerate() {
        let r = rng.range(26.0, 48.0);
        let z = speed * duration * at;
        let a = rng.range(0.0, TAU);
        let miss = r + rng.range(9.0, 15.0);
        let side = [a.cos(), a.sin() * 0.6, 0.0];
        objects[k] = [side[0] * miss, side[1] * miss, z, r];
        // Swerve away from it as it nears, by a little.
        let near = (-sq((local - z / speed) / 2.2)).exp();
        dodge = add(dodge, scale(side, -9.0 * near));
        alert = alert.max(0.75 * near);
        // Glance at it before it passes.
        let lead = smoothstep(-6.0, -1.5, local - z / speed)
            * (1.0 - smoothstep(-1.0, 0.5, local - z / speed));
        let to = normalize(sub(
            [objects[k][0], objects[k][1], z],
            [0.0, 0.0, speed * local],
        ));
        gaze = blend(gaze, to, 0.35 * lead);
    }
    let ship = add([0.0, 0.0, speed * local], dodge);
    let swerve = dodge[0];
    let hits = [rng.range(0.4, 0.5), rng.range(0.62, 0.72)];
    let hit = hits
        .iter()
        .map(|&h| pulse(local, h * duration, 0.05, 0.35))
        .fold(0.0, f64::max);
    Shot {
        ship,
        look: gaze,
        roll: -swerve * 0.025,
        alert,
        hit,
        hit_at: [rng.range(-0.5, 0.5), rng.range(-0.4, 0.4)],
        shake: 0.15 + 0.6 * hit,
        objects,
        extra: [kind, density, normalize(light)[0], normalize(light)[1]],
        rot: [normalize(light)[2], 0.0, 0.0, 0.0],
        ..Default::default()
    }
}

/// A hive cube: it pulls the ship out of the jump, locks a tractor beam on
/// it, and the ship tears free and jumps away. Every one its own: its size,
/// colour, wear, turning, escorts, which side it lies, its beam and how the
/// ship breaks away.
fn hive_cube(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let size = rng.range(26.0, 46.0);
    let hue = *rng.pick(&[0.33, 0.36, 0.30, 0.42, 0.27, 0.33, 0.02]);
    let damage = rng.range(0.0, 1.0);
    let escorts = rng.int(0, 2) as usize;
    let beam_style = rng.int(0, 1) as f64;
    let breakaway = rng.int(0, 1);
    let lock = rng.range(0.2, 0.3) * duration;
    let free = rng.range(0.72, 0.8) * duration;
    let side = *rng.pick(&[[-1.0, 0.15], [1.0, 0.1], [0.35, 1.0], [-0.6, -0.5]]);
    let axis = normalize([
        rng.range(-1.0, 1.0),
        rng.range(-1.0, 1.0),
        rng.range(-1.0, 1.0),
    ]);
    let spin = rng.range(0.012, 0.035) * if rng.chance(0.5) { 1.0 } else { -1.0 };
    let tilt = rng.range(0.0, TAU);
    let cube = [side[0] * size * 2.4, side[1] * size * 1.6, size * 6.5];
    // Escorts: small cubes keeping station off its faces.
    let mut objects = [[cube[0], cube[1], cube[2], size], [0.0; 4], [0.0; 4]];
    for (k, object) in objects.iter_mut().enumerate().skip(1).take(escorts) {
        let a = rng.range(0.0, TAU) + local * 0.04 * k as f64;
        let place = add(
            cube,
            [
                a.cos() * size * 2.2,
                a.sin() * size * 1.1,
                -size * rng.range(0.6, 1.6),
            ],
        );
        *object = [place[0], place[1], place[2], size * rng.range(0.1, 0.18)];
    }
    // Drifting in after the drop-out, dragged toward it under the beam,
    // then away along the heading.
    let held = smoothstep(lock, lock + 2.5, local) * (1.0 - smoothstep(free, free + 0.6, local));
    let drag = smoothstep(lock, free, local);
    let pulled = scale(sub(cube, [0.0; 3]), 0.28 * drag * drag);
    let gone = (local - free).max(0.0);
    // Surging away, at a steady speed once clear.
    let surge = if gone < 2.0 {
        14.0 * gone * gone
    } else {
        56.0 * (gone - 1.0)
    };
    let away = [0.0, 0.0, 6.0 * local + surge];
    let wrench = if breakaway == 0 { 0.0 } else { 1.0 };
    // The evasive break corkscrews; the overload jolts straight away.
    let corkscrew = if gone > 0.0 {
        scale(
            [(gone * 2.4).sin(), (gone * 2.4).cos() - 1.0, 0.0],
            6.0 * wrench * gone.min(2.0),
        )
    } else {
        [0.0; 3]
    };
    let ship = add(
        add(
            away,
            scale(pulled, 1.0 - smoothstep(free, free + 3.0, local)),
        ),
        corkscrew,
    );
    let to_cube = normalize(sub(cube, ship));
    let watch = smoothstep(1.0, 5.0, local) * (1.0 - smoothstep(free, free + 2.5, local));
    let look = blend(AHEAD, to_cube, 0.8 * watch);
    // The shield hit every couple of seconds under the beam.
    let beat = ((local - lock) / 2.3).fract();
    let struck = if local > lock + 1.0 && local < free {
        (-beat * 5.0).exp()
    } else {
        0.0
    };
    let snap = pulse(local, free, 0.15, 0.6);
    let alert = smoothstep(0.5, 3.0, local) * 0.6 + 0.4 * held;
    Shot {
        ship,
        look,
        roll: wrench * gone.min(3.0) * 0.9 + 0.05 * (local * 0.7).sin() * held,
        alert: alert.min(1.0),
        hit: struck.max(snap * 0.8),
        hit_at: [side[0] * 0.35, side[1] * 0.25],
        beam: held,
        flash: snap * 0.7,
        shake: (0.25 + 0.55 * held + 0.6 * snap).min(1.0),
        objects,
        rot: [axis[0], axis[1], axis[2], tilt + spin * local],
        extra: [hue, damage, beam_style, (rng.next() % 997) as f64],
    }
}

/// A black hole: its pull tears the ship out of the jump, swings it round
/// past the accretion disc and slings it back onto the heading.
fn black_hole(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let rs = rng.range(7.0, 12.0);
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let hole = [side * 70.0, rng.range(-15.0, 25.0), 300.0];
    let tilt = [rng.range(-0.5, 0.5), rng.range(-0.35, 0.35)];
    let temperature = rng.range(0.0, 1.0);
    let spin = if rng.chance(0.5) { 1.0 } else { -1.0 };
    // A curve: in toward it, round its far side and out again.
    let u = (local / duration).clamp(0.0, 1.2);
    let swing = smoothstep(0.15, 0.85, u);
    let around = side * (swing * 0.9 - 0.15);
    let reach = 130.0 - 55.0 * (PI * swing).sin();
    let ship = [
        hole[0]
            - side * reach * (around * PI * 0.5).cos() * (1.0 - swing)
            - side * 40.0 * swing * swing,
        hole[1] * 0.4,
        420.0 * u.powf(1.15),
    ];
    let to = normalize(sub(hole, ship));
    let watch = smoothstep(0.03, 0.2, u) * (1.0 - smoothstep(0.78, 1.0, u));
    let near = (PI * swing).sin();
    Shot {
        ship,
        look: blend(AHEAD, to, 0.75 * watch),
        roll: side * 0.25 * near,
        alert: 0.35 + 0.4 * near,
        shake: 0.1 + 0.35 * near * near,
        objects: [[hole[0], hole[1], hole[2], rs], [0.0; 4], [0.0; 4]],
        extra: [tilt[0], tilt[1], temperature, spin],
        ..Default::default()
    }
}

/// A nebula: the ship slips through glowing gas and dark lanes of dust,
/// lightning flickering deep in it.
fn nebula(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let palette = rng.int(0, 3) as f64;
    let density = rng.range(0.7, 1.0);
    let speed = 40.0;
    let ship = [
        8.0 * (local * 0.11).sin(),
        4.0 * (local * 0.07).sin(),
        speed * local,
    ];
    // Lightning: a few strikes, each somewhere in the gas ahead.
    let mut strike = [0.0; 4];
    let mut shake: f64 = 0.05;
    for k in 0..5 {
        let at = rng.range(0.12, 0.92) * duration;
        let place = [
            rng.range(-160.0, 160.0),
            rng.range(-90.0, 90.0),
            speed * at + rng.range(60.0, 260.0),
        ];
        let p = pulse(local, at, 0.05, 0.25 + 0.1 * k as f64);
        if p > strike[3] {
            strike = [place[0], place[1], place[2], p];
        }
        shake = shake.max(0.3 * p);
    }
    Shot {
        ship,
        look: normalize([
            0.12 * (local * 0.09).sin(),
            0.05 * (local * 0.13).sin(),
            1.0,
        ]),
        roll: 0.06 * (local * 0.1).sin(),
        alert: 0.2,
        shake,
        flash: strike[3] * 0.08,
        objects: [
            strike,
            [palette, density, (rng.next() % 101) as f64, 0.0],
            [0.0; 4],
        ],
        ..Default::default()
    }
}

/// A rogue planet: a world with no star, lit by its own aurorae and the glow
/// of its cracks, passed close.
fn rogue_planet(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let radius = 90.0;
    let kind = rng.int(0, 2) as f64;
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let speed = 34.0;
    let centre = [
        side * radius * 1.7,
        -radius * rng.range(0.2, 0.55),
        speed * duration * 0.86,
    ];
    let axis = normalize([rng.range(-0.4, 0.4), 1.0, rng.range(-0.4, 0.4)]);
    let ship = [0.0, 0.0, speed * local];
    let to = normalize(sub(centre, ship));
    let watch = smoothstep(0.0, 4.0, local) * (1.0 - smoothstep(0.92, 1.05, local / duration));
    Shot {
        ship,
        look: blend(AHEAD, to, 0.72 * watch),
        roll: -side * 0.12 * watch,
        alert: 0.15,
        objects: [
            [centre[0], centre[1], centre[2], radius],
            [0.0; 4],
            [0.0; 4],
        ],
        rot: [
            axis[0],
            axis[1],
            axis[2],
            local * 0.012 + rng.range(0.0, TAU),
        ],
        extra: [
            kind,
            rng.range(0.0, 1.0),
            rng.range(0.5, 1.0),
            (rng.next() % 113) as f64,
        ],
        ..Default::default()
    }
}

/// A pulsar: its beams sweep round like a lighthouse's, and every turn one
/// crosses the ship.
fn pulsar(rng: &mut Rng, local: f64, _duration: f64) -> Shot {
    let star = [rng.range(-80.0, 80.0), rng.range(10.0, 60.0), 420.0];
    let period = rng.range(2.1, 2.9);
    let tilt = rng.range(0.45, 0.8);
    let ship = [0.0, 0.0, 9.0 * local];
    // The beam's cone round the spin axis passes through the ship.
    let towards = normalize(sub(ship, star));
    let perp = normalize(cross(towards, [0.3, 1.0, 0.1]));
    let axis = rotate(towards, perp, tilt);
    let phase = TAU * local / period;
    let beam = rotate(towards, axis, phase);
    let aligned = dot(beam, towards).max(dot(scale(beam, -1.0), towards));
    let sweep = ((aligned - 0.995) / 0.005).clamp(0.0, 1.0).powi(2);
    let to = normalize(sub(star, ship));
    Shot {
        ship,
        look: blend(AHEAD, to, 0.65 * smoothstep(0.0, 4.0, local)),
        alert: 0.45 + 0.4 * sweep,
        hit: sweep,
        hit_at: [0.0, 0.0],
        flash: sweep * 0.35,
        shake: 0.1 + 0.7 * sweep,
        objects: [
            [star[0], star[1], star[2], 4.0],
            [beam[0], beam[1], beam[2], tilt],
            [0.0; 4],
        ],
        rot: [axis[0], axis[1], axis[2], phase],
        extra: [rng.range(0.0, 1.0), rng.range(0.6, 1.0), period, 0.0],
        ..Default::default()
    }
}

/// A derelict: a broken ring station turning slowly, flown through, its
/// last lights flickering, debris all round.
fn derelict(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let radius = rng.range(120.0, 160.0);
    let speed = 22.0;
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let centre = [
        side * radius * 0.55,
        rng.range(-30.0, 10.0),
        speed * duration * 0.6 + radius * 0.4,
    ];
    // Tilted well over, so its inner face is seen.
    let axis = normalize([
        side * rng.range(0.5, 0.9),
        rng.range(0.5, 0.9),
        rng.range(0.3, 0.6),
    ]);
    let ship = [0.0, 0.0, speed * local];
    let gaze = normalize(add(
        sub(centre, ship),
        [radius * 0.5 * (local * 0.08).sin(), radius * 0.35, 0.0],
    ));
    let watch = smoothstep(0.0, 3.0, local) * (1.0 - smoothstep(0.5, 0.62, local / duration));
    Shot {
        ship,
        look: blend(AHEAD, gaze, 0.55 * watch),
        roll: 0.18 * (local * 0.05).sin(),
        alert: 0.3,
        objects: [
            [centre[0], centre[1], centre[2], radius],
            [rng.range(24.0, 34.0), rng.int(2, 4) as f64, 0.0, 0.0],
            [0.0; 4],
        ],
        rot: [
            axis[0],
            axis[1],
            axis[2],
            local * rng.range(0.008, 0.02) + rng.range(0.0, TAU),
        ],
        extra: [
            rng.range(0.0, 1.0),
            rng.range(0.0, 1.0),
            rng.range(0.3, 1.0),
            (rng.next() % 127) as f64,
        ],
        ..Default::default()
    }
}

/// A comet far from any star: its nucleus and coma, a curved dust tail and
/// a straight blue ion tail, which the ship flies through.
fn comet(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let speed = 30.0;
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let nucleus = [-side * 160.0, rng.range(-30.0, 50.0), 250.0];
    let crossing = [0.0, 0.0, speed * duration * 0.62];
    let tail = normalize(sub(crossing, nucleus));
    let ship = [0.0, 0.0, speed * local];
    let to = normalize(sub(nucleus, ship));
    let watch = smoothstep(0.0, 4.0, local) * (1.0 - smoothstep(0.45, 0.6, local / duration));
    let inside = (-sq((local - duration * 0.62) / 1.6)).exp();
    Shot {
        ship,
        look: blend(AHEAD, to, 0.6 * watch),
        roll: side * 0.08 * smoothstep(0.0, 3.0, local),
        alert: 0.15 + 0.3 * inside,
        shake: 0.05 + 0.4 * inside,
        objects: [
            [nucleus[0], nucleus[1], nucleus[2], 3.5],
            [tail[0], tail[1], tail[2], 900.0],
            [0.0; 4],
        ],
        extra: [
            side * rng.range(0.25, 0.45),
            rng.range(0.0, 1.0),
            rng.range(0.8, 1.2),
            (rng.next() % 89) as f64,
        ],
        ..Default::default()
    }
}

/// Leviathans: a pod of vast glowing creatures drifting across the ship's
/// way, the ship slowed to watch them pass.
fn leviathans(rng: &mut Rng, local: f64, _duration: f64) -> Shot {
    let count = rng.int(3, 5) as f64;
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let heading = normalize([-side, rng.range(-0.15, 0.15), rng.range(-0.35, 0.1)]);
    let start = [side * 95.0, rng.range(-25.0, 15.0), 175.0];
    let pod = add(start, scale(heading, 7.5 * local));
    let ship = [0.0, 0.0, 4.0 * local];
    let to = normalize(sub(pod, ship));
    Shot {
        ship,
        look: blend(AHEAD, to, 0.85 * smoothstep(0.0, 5.0, local)),
        roll: 0.04 * (local * 0.2).sin(),
        alert: 0.0,
        objects: [
            [pod[0], pod[1], pod[2], count],
            [heading[0], heading[1], heading[2], rng.range(120.0, 170.0)],
            [0.0; 4],
        ],
        extra: [
            rng.range(0.0, 1.0),
            rng.range(0.0, 1.0),
            (rng.next() % 61) as f64,
            local,
        ],
        ..Default::default()
    }
}

/// A supernova remnant: a vast shell of glowing filaments, crossed with a
/// jolt as its shock goes by, and the pulsar it left at the heart.
fn remnant(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let radius = 520.0;
    let speed = 40.0;
    let cross_at = speed * duration * 0.5;
    let off = [rng.range(-170.0, 170.0), rng.range(-80.0, 80.0)];
    let depth = (radius * radius - off[0] * off[0] - off[1] * off[1]).sqrt();
    let centre = [off[0], off[1], cross_at + depth];
    let ship = [0.0, 0.0, speed * local];
    let shock = pulse(local, duration * 0.5, 0.4, 1.2);
    Shot {
        ship,
        look: normalize([
            0.15 * (local * 0.05).sin(),
            0.05 * smoothstep(0.0, 4.0, local),
            1.0,
        ]),
        roll: 0.1 * shock,
        alert: 0.2 + 0.5 * shock,
        hit: shock,
        flash: shock * 0.45,
        shake: 0.05 + 0.75 * shock,
        objects: [
            [centre[0], centre[1], centre[2], radius],
            [0.0; 4],
            [0.0; 4],
        ],
        extra: [
            rng.range(0.0, 1.0),
            rng.range(0.8, 1.3),
            0.0,
            (rng.next() % 79) as f64,
        ],
        ..Default::default()
    }
}

/// A star gate: a ring met at the end of a leg, lit node by node as the
/// ship lines up, its field opening, and flown through.
fn gate(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let distance = 560.0;
    let style = rng.int(0, 2) as f64;
    let hue = rng.range(0.0, 1.0);
    let normal = normalize([rng.range(-0.08, 0.08), rng.range(-0.06, 0.06), -1.0]);
    let u = (local / duration).clamp(0.0, 1.0);
    // Slowing on the drop-out, then speeding up all the way through.
    let z = distance * (0.25 * u + 0.75 * u * u * u);
    let ship = [6.0 * (1.0 - u) * (local * 0.5).sin(), 3.0 * (1.0 - u), z];
    let through = pulse(local, duration, 0.8, 0.5);
    Shot {
        ship,
        look: normalize([
            0.06 * (1.0 - u) * smoothstep(0.0, 2.0, local),
            -0.02 * (1.0 - u) * smoothstep(0.0, 2.0, local),
            1.0,
        ]),
        roll: 0.6
            * smoothstep(0.0, 0.12, u)
            * (1.0 - smoothstep(0.12, 0.7, u))
            * if rng.chance(0.5) { 1.0 } else { -1.0 },
        alert: 0.0,
        flash: through,
        shake: 0.05 + 0.4 * smoothstep(0.75, 1.0, u),
        objects: [[0.0, 0.0, distance, 42.0], [0.0; 4], [0.0; 4]],
        rot: [normal[0], normal[1], normal[2], local * 0.05],
        extra: [style, hue, u, (rng.next() % 71) as f64],
        ..Default::default()
    }
}

/// A wormhole: a sphere with another sky in it, the stars round it bent
/// into a ring, dived into.
fn wormhole(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let distance = 430.0;
    let radius = rng.range(24.0, 34.0);
    let u = (local / duration).clamp(0.0, 1.0);
    let z = (distance - radius) * (0.3 * u + 0.7 * u * u);
    let ship = [0.0, 0.0, z];
    let through = pulse(local, duration, 0.7, 0.4);
    Shot {
        ship,
        look: AHEAD,
        roll: 0.4 * (PI * u).sin() * u,
        alert: 0.1,
        flash: through * 0.8,
        shake: 0.05 + 0.5 * smoothstep(0.7, 1.0, u),
        objects: [[0.0, 0.0, distance, radius], [0.0; 4], [0.0; 4]],
        extra: [
            rng.range(0.0, 1.0),
            if rng.chance(0.5) { 1.0 } else { -1.0 },
            rng.range(0.7, 1.0),
            (rng.next() % 67) as f64,
        ],
        ..Default::default()
    }
}

/// A battle: dropped out on the edge of two fleets fighting, with lasers,
/// torpedoes and shields flaring, a ship lost; not the ship's fight, it
/// watches as it slips away, stray fire passing close, and jumps out.
/// The fleets' ships, their fire and the wreck are worked out in the shader
/// from the battle's centre, its draw and the time (`ev_battle`).
fn battle(rng: &mut Rng, local: f64, duration: f64) -> Shot {
    let side = if rng.chance(0.5) { 1.0 } else { -1.0 };
    let centre = [
        side * rng.range(70.0, 100.0),
        rng.range(-10.0, 30.0),
        rng.range(230.0, 260.0),
    ];
    let spread = rng.range(80.0, 105.0);
    let warm = rng.range(0.0, 0.12);
    let cool = rng.range(0.45, 0.62);
    let kill = rng.range(0.42, 0.55) * duration;
    let stray_laser = rng.range(0.3, 0.4) * duration;
    let stray_torpedo = rng.range(0.6, 0.68) * duration;
    // Drifting in on the drop-out, then easing away from the fight, off to
    // the far side and on along the heading, quicker as it goes.
    let away = smoothstep(0.15 * duration, duration, local);
    let ship = [
        -side * 45.0 * away * away,
        -10.0 * away,
        4.0 * local + 30.0 * away * away,
    ];
    let to = normalize(sub(centre, ship));
    let watch = smoothstep(0.5, 4.0, local) * (1.0 - smoothstep(0.9, 1.04, local / duration));
    let near_laser = pulse(local, stray_laser, 0.1, 0.5);
    let near_torpedo = pulse(local, stray_torpedo + 1.1, 0.08, 0.7);
    let blast = pulse(local, kill, 0.1, 1.2);
    Shot {
        ship,
        look: blend(AHEAD, to, 0.78 * watch),
        roll: side * 0.12 * watch + 0.08 * near_torpedo,
        alert: (0.55 + 0.45 * near_laser.max(near_torpedo)).min(1.0) * smoothstep(0.0, 1.5, local),
        hit: near_torpedo,
        hit_at: [-side * 0.3, -0.25],
        flash: 0.25 * near_torpedo + 0.12 * blast + 0.08 * near_laser,
        shake: 0.12 + 0.75 * near_torpedo + 0.3 * blast + 0.25 * near_laser,
        objects: [
            [centre[0], centre[1], centre[2], spread],
            [warm, cool, kill, side],
            [stray_laser, stray_torpedo, 0.0, 0.0],
        ],
        extra: [(rng.next() % 997) as f64, 0.0, 0.0, 0.0],
        ..Default::default()
    }
}

fn sq(x: f64) -> f64 {
    x * x
}

/// A gate's or a wormhole's name, from its draw.
pub fn name_of(stop: &Stopover) -> String {
    let mut rng = Rng::new(stop.variant ^ 0x0a3e);
    match stop.event {
        Event::Gate => {
            let names = [
                "Kestrel", "Aster", "Halcyon", "Meridian", "Corvid", "Tessera", "Lantern",
                "Osprey", "Vigil", "Cinder",
            ];
            format!("Gate {}-{}", rng.pick(&names), rng.int(2, 19))
        }
        Event::Wormhole => format!("Bridge ER-{}", rng.int(100, 999)),
        Event::HiveCube => format!("Cube {}-{:02}", rng.int(100, 999), rng.int(1, 99)),
        _ => String::new(),
    }
}

/// The manoeuvre panel's name, text and progress.
pub type Panel = (String, String, f64);

/// The HUD for `stop` `local` seconds in: a title and its lines, and the
/// manoeuvre panel's name, text and progress. `to` is the star the trip
/// makes for and `ly` the light years still to go.
pub fn hud(
    stop: &Stopover,
    local: f64,
    to: &str,
    ly: f64,
) -> (String, Vec<(String, String)>, Panel) {
    let duration = stop.event.duration();
    let u = (local / duration).clamp(0.0, 1.0);
    let shot = shot(stop, local);
    let mut rng = Rng::new(stop.variant ^ 0x4d0);
    let line = |a: &str, b: String| (a.to_string(), b);
    let range = |i: usize| {
        let o = shot.objects[i];
        length(sub([o[0], o[1], o[2]], shot.ship))
    };
    // Encounter units read as kilometres, scaled.
    let km = |d: f64| format!("{} km", super::tour::thousands(d * 10.0));
    let rest = line("Remaining", format!("{ly:.1} ly to {to}"));
    let left = line("Re-engage", super::tour::clock(duration - local));
    let (title, lines, panel) = match stop.event {
        Event::Asteroids => {
            let kinds = ["carbonaceous", "water ice", "nickel-iron"];
            let kind = kinds[shot.extra[0] as usize % 3];
            let nearest = (0..3).map(|i| range(i) - shot.objects[i][3]).fold(f64::MAX, f64::min);
            (
                "NAV HAZARD",
                vec![
                    line("Field", format!("{kind} · {} bodies/km³", (shot.extra[1] * 40.0).round())),
                    line("Closest", km(nearest.max(0.0))),
                    line("Shield", format!("{:.0}%", 100.0 - 6.0 * u - 8.0 * shot.hit)),
                    left,
                    rest,
                ],
                (
                    "ASTEROID FIELD",
                    "Dropped out to thread a field of rogue rock between the stars, the shield taking the small stuff.".into(),
                ),
            )
        }
        Event::HiveCube => {
            let name = name_of(stop);
            let beam = shot.beam;
            let phase = if local < duration * 0.2 {
                "CONTACT"
            } else if beam > 0.05 {
                "TRACTOR LOCK"
            } else if local < duration {
                "BREAKAWAY"
            } else {
                "EMERGENCY JUMP"
            };
            (
                "INTERDICTION",
                vec![
                    line("Vessel", format!("{name} · {} km a side", super::tour::thousands(shot.objects[0][3] * 20.0))),
                    line("Status", phase.to_string()),
                    line("Range", km(range(0) - shot.objects[0][3])),
                    line("Shield", format!("{:.0}%", 100.0 - 55.0 * smoothstep(0.2, 0.75, u))),
                    line("Escorts", ((shot.objects[1][3] > 0.0) as u32 + (shot.objects[2][3] > 0.0) as u32).to_string()),
                ],
                (
                    phase,
                    match phase {
                        "CONTACT" => "A vast cube pulled the ship out of the jump. Hails unanswered.".into(),
                        "TRACTOR LOCK" => "Its beam has the ship. Remodulating the shield, drive to overload.".into(),
                        _ => "Torn free of the beam: emergency jump before it locks again.".into(),
                    },
                ),
            )
        }
        Event::BlackHole => {
            let rs = shot.objects[0][3];
            let mass = rs * rng.range(0.9, 1.4);
            (
                "INTERDICTION",
                vec![
                    line("Object", format!("black hole · {mass:.1} solar masses")),
                    line("Horizon", km(rs * 0.3)),
                    line("Range", format!("{:.0} Rs", range(0) / rs)),
                    line("Time dilation", format!("{:.4}×", 1.0 / (1.0 - rs / range(0).max(rs * 1.5)).sqrt())),
                    rest,
                ],
                (
                    "GRAVITATIONAL SLINGSHOT",
                    "Its pull tore the drive's field open. Swinging round past the disc to sling back onto the heading.".into(),
                ),
            )
        }
        Event::Nebula => {
            let names = ["H-alpha emission", "oxygen-III emission", "reflection", "dark dust"];
            (
                "NEBULA",
                vec![
                    line("Cloud", names[shot.objects[1][0] as usize % 4].to_string()),
                    line("Density", format!("{:.0} atoms/cm³", 900.0 * shot.objects[1][1])),
                    line("Discharge", if shot.objects[0][3] > 0.2 { "STRIKE".into() } else { "static building".into() }),
                    left,
                    rest,
                ],
                (
                    "NEBULA TRANSIT",
                    "Too dense to hold the jump through: crossing on thrusters, lightning arcing through the gas.".into(),
                ),
            )
        }
        Event::RoguePlanet => {
            let kinds = ["ice world", "dark rock, molten below", "frozen ocean"];
            (
                "ROGUE WORLD",
                vec![
                    line("World", kinds[shot.extra[0] as usize % 3].to_string()),
                    line("Star", "none · ejected long ago".into()),
                    line("Surface", format!("{:.0} K", 30.0 + 30.0 * shot.extra[2])),
                    line("Altitude", km(range(0) - shot.objects[0][3])),
                    rest,
                ],
                (
                    "ROGUE PLANET FLYBY",
                    "A world with no star, lit only by its aurorae and the heat of its cracks.".into(),
                ),
            )
        }
        Event::Pulsar => (
            "INTERDICTION",
            vec![
                line("Object", "pulsar · 1.4 solar masses".into()),
                line("Period", format!("{:.0} ms (strobed {:.1} s)", 30.0 + 60.0 * shot.extra[0], shot.extra[2])),
                line("Radiation", format!("{:.0} Sv/h", 2.0 + 480.0 * shot.hit)),
                line("Range", km(range(0))),
                left,
            ],
            (
                "PULSAR SWEEP",
                "Its beams sweep the ship once a turn; hull shielding to full until the drive can re-engage.".into(),
            ),
        ),
        Event::Derelict => (
            "DERELICT",
            vec![
                line("Structure", format!("ring station · {} across", km(shot.objects[0][3] * 2.0))),
                line("Power", if shot.extra[2] > 0.6 { "faint".into() } else { "failing".into() }),
                line("Life signs", "none".into()),
                line("Range", km((range(0) - shot.objects[0][3]).abs())),
                left,
            ],
            (
                "DERELICT STATION",
                "A broken ring station adrift between the stars, its last lights still flickering.".into(),
            ),
        ),
        Event::Comet => (
            "COMET",
            vec![
                line("Nucleus", km(range(0))),
                line("Ion tail", km(shot.objects[1][3])),
                line("Dust", if shot.hit > 0.1 { "IMPACTS".into() } else { "light".into() }),
                left,
                rest,
            ],
            (
                "INTERSTELLAR COMET",
                "A comet adrift between stars: crossing its blue ion tail.".into(),
            ),
        ),
        Event::Leviathans => (
            "CONTACT",
            vec![
                line("Lifeforms", format!("{} · {} long", shot.objects[0][3] as u32, km(shot.objects[1][3]))),
                line("Signal", "bioluminescent, rhythmic".into()),
                line("Range", km(range(0))),
                left,
                rest,
            ],
            (
                "LEVIATHANS",
                "Drive idled so as not to startle them: a pod of vast living things crossing the void.".into(),
            ),
        ),
        Event::Remnant => (
            "REMNANT",
            vec![
                line("Shell", format!("{} across, expanding", km(shot.objects[0][3] * 2.0))),
                line("Shock", if shot.hit > 0.1 { "CROSSING".into() } else { format!("{:.0} km/s", 4000.0 + 3000.0 * shot.extra[1]) }),
                line("Range", km((range(0) - shot.objects[0][3]).abs())),
                left,
                rest,
            ],
            (
                "SUPERNOVA REMNANT",
                "The shell of a star that blew apart, ten thousand years on, flown through.".into(),
            ),
        ),
        Event::Gate => {
            let name = name_of(stop);
            let phase = if u < 0.3 {
                "HANDSHAKE"
            } else if u < 0.55 {
                "NODES LIGHTING"
            } else if u < 0.85 {
                "FIELD OPEN"
            } else {
                "TRANSIT"
            };
            (
                "STAR GATE",
                vec![
                    line("Gate", name.clone()),
                    line("Status", phase.to_string()),
                    line("Range", km(range(0))),
                    line("Throws to", format!("{to} · {ly:.1} ly")),
                ],
                (
                    "STAR GATE",
                    format!("{name}: lining up on the ring, the gate's field throws the ship on toward {to}."),
                ),
            )
        }
        Event::Battle => {
            let kill = shot.objects[1][2];
            let phase = if local < 3.0 {
                "CONTACT"
            } else if local < shot.objects[2][1] + 2.5 && local > shot.objects[2][0] - 0.5 {
                "STRAY FIRE"
            } else if local < duration * 0.8 {
                "RUNNING DARK"
            } else {
                "CLEAR"
            };
            (
                "COMBAT ZONE",
                vec![
                    line("Combatants", "two fleets · unidentified".into()),
                    line("Ships", if local > kill { "7 · one lost".into() } else { "8".into() }),
                    line("Status", phase.to_string()),
                    line("Range", km(range(0))),
                    line("Shield", format!("{:.0}%", 100.0 - 18.0 * smoothstep(0.6, 0.75, u))),
                ],
                (
                    phase,
                    match phase {
                        "CONTACT" => "Dropped out into someone else's war. Not our fight: running dark.".into(),
                        "STRAY FIRE" => "Stray fire! Shield up, holding course away from the line of battle.".into(),
                        "CLEAR" => "Clear of the fighting. Spooling the drive while nobody is looking.".into(),
                        _ => "Engines low, emissions masked, edging away from the fleets.".into(),
                    },
                ),
            )
        }
        Event::Wormhole => {
            let name = name_of(stop);
            (
                "WORMHOLE",
                vec![
                    line("Bridge", name.clone()),
                    line("Throat", km(shot.objects[0][3] * 2.0)),
                    line("Range", km(range(0) - shot.objects[0][3])),
                    line("Saves", format!("{:.1} ly", ly * 0.4)),
                ],
                (
                    "WORMHOLE",
                    format!("{name}: a shortcut through space, its far mouth near {to}. Diving in."),
                ),
            )
        }
    };
    (title.into(), lines, (panel.0.into(), panel.1, u))
}

/// The HUD's words for a flux, while it shows.
pub fn flux_hud(flux: Flux) -> (&'static str, &'static str) {
    match flux {
        Flux::IonStorm => (
            "ION STORM",
            "A storm in the drive's field: lightning along the walls, the light stuttering.",
        ),
        Flux::Convoy => (
            "CONVOY",
            "Other ships' jumps running alongside, then peeling away.",
        ),
        Flux::Echo => (
            "TACHYON ECHO",
            "The ship's own wake, seen a few seconds ahead of itself.",
        ),
        Flux::Shortcut => (
            "SHORTCUT",
            "The field folds through a fault in space: the leg cut short.",
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_encounter_comes_round_and_none_twice_running() {
        let mut seen = std::collections::HashSet::new();
        let mut last = None;
        for n in 0..240 {
            let e = dealt(17, n);
            assert_ne!(Some(e), last, "{e:?} twice in a row at {n}");
            last = Some(e);
            seen.insert(e);
        }
        assert_eq!(seen.len(), Event::ALL.len());
    }

    #[test]
    fn passages_are_shorter_than_the_old_jump() {
        for trip in 0..200 {
            let p = Passage::plan(3, trip, 1 + (trip % 5) as u32);
            assert!(!p.stops.is_empty() && p.stops.len() <= 2);
            assert_eq!(p.legs.len(), p.stops.len() + 1);
            assert!((p.legs.last().unwrap().duration - LEG).abs() < 1e-9);
            let shares: f64 = p.legs.iter().map(|l| l.share).sum();
            assert!((shares - 1.0).abs() < 1e-9);
            // The old jump was 150 s of light alone; most are well under.
            assert!(p.duration() < 175.0, "{}", p.duration());
        }
        let mean: f64 = (0..200)
            .map(|t| Passage::plan(3, t, 1).duration())
            .sum::<f64>()
            / 200.0;
        assert!(mean < 125.0, "mean {mean}");
    }

    #[test]
    fn shots_are_finite_and_continuous() {
        for event in Event::ALL {
            for variant in 0..12 {
                let stop = Stopover {
                    event,
                    variant: variant * 7919,
                };
                let end = event.duration() + event.rejump();
                let mut last: Option<Shot> = None;
                let mut t = 0.0;
                while t <= end {
                    let s = shot(&stop, t);
                    for v in s.ship.iter().chain(&s.look) {
                        assert!(v.is_finite(), "{event:?} at {t}");
                    }
                    assert!((length(s.look) - 1.0).abs() < 1e-6);
                    if let Some(l) = last {
                        // No jumps of the camera from one frame to the next
                        // (a breakaway's surge is quick, but no jump).
                        assert!(
                            length(sub(s.ship, l.ship)) < 12.0,
                            "{event:?} {variant} ship jumps at {t}"
                        );
                        assert!(
                            dot(s.look, l.look) > 0.995,
                            "{event:?} {variant} look jumps at {t}"
                        );
                    }
                    last = Some(s);
                    t += 1.0 / 30.0;
                }
            }
        }
    }

    #[test]
    fn hive_cubes_differ() {
        let shots: Vec<Shot> = (0..8)
            .map(|v| {
                shot(
                    &Stopover {
                        event: Event::HiveCube,
                        variant: v * 131 + 5,
                    },
                    12.0,
                )
            })
            .collect();
        for (i, a) in shots.iter().enumerate() {
            for b in &shots[i + 1..] {
                assert!((a.objects[0][3] - b.objects[0][3]).abs() > 1e-6 || a.extra != b.extra);
            }
        }
    }
}
