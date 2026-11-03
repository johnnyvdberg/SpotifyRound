//! The drowned museum (biome [`MUSEUM`]): three grand halls in line, which
//! the Nautilus enters by the doors in the front, tours and leaves by the
//! doors at the back.
//!
//! - The natural history hall, after the Natural History Museum's great
//!   hall: arcades of buff and slate terracotta either side under a vault
//!   with a skylight along its crown, galleries over the arcades, a great
//!   whale's skeleton hung on cables from the vault over an island of the
//!   largest skeletons, the rest of the specimens on plinths and in cases
//!   along the aisles, and the natural history paintings on the walls. The
//!   boat loops once round the island, the window turned in on it.
//! - The mineral hall: dark walls and lit glass cases on pedestals, each
//!   mineral on its stand inside, under lamps that turn from white light to
//!   ultraviolet and back, so the fluorescent ones flare in their own
//!   colours.
//! - The picture gallery: the grand masters hung salon-fashion down its
//!   long wall round the largest, in gilt frames on cords from a picture
//!   rail, each with its label.
//!
//! The building is the visit's own ([`Museum::near`]): the halls' order and
//! proportions, round or pointed arches, galleries or none, the loop's
//! radius, how the mineral cases stand, and which specimen and painting go
//! where. The helm reads the same layout ([`tour`]), so the loop fits
//! whatever hall it is given. The specimens and paintings are models and
//! textures the renderer streams in as the museum comes near and lets go
//! once it is behind (`Model::is_streamed`).

use std::f32::consts::FRAC_PI_2;

use super::*;
use crate::dive3d::paintings::{PAINTINGS, Painting};

/// The stretch of its leg the building takes, in units from the leg's
/// start: clear of the blend in from the last leg and, with the shift
/// `voyage_x` gives what stands off the corridor, of the blend out.
const FRONT: f32 = 2.0;
const BACK: f32 = 13.4;
/// How far before the front door and after the back door the boat's way
/// leaves its track and rejoins it.
const APPROACH: f32 = 1.4;
/// How far to starboard of the boat's lane the starboard walls stand.
const STARBOARD: f32 = 0.75;
/// How far above the open seabed round it the museum's floor is laid, over
/// the museum's slight relief.
const PLINTH: f32 = 0.1;
/// The doors on the lane: half their width and their height.
const DOOR: f32 = 0.62;
const DOOR_HIGH: f32 = 1.3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HallKind {
    Natural,
    Minerals,
    Masters,
}

/// One hall, in the museum's frame: `u` along its axis from the front door,
/// `v` across it to port from the boat's lane.
#[derive(Clone, Copy, Debug)]
pub struct Hall {
    pub kind: HallKind,
    pub from: f32,
    pub long: f32,
    /// The port wall's distance from the lane.
    pub port: f32,
    /// The walls' height to the vault's springing, and the vault's rise.
    pub high: f32,
    pub rise: f32,
    pub bays: usize,
    /// Pointed arches and vault rather than round ones.
    pub pointed: bool,
    /// Galleries over the arcades.
    pub galleries: bool,
    pub wall: [f32; 3],
    pub trim: [f32; 3],
}

impl Hall {
    fn to(&self) -> f32 {
        self.from + self.long
    }

    fn middle(&self) -> f32 {
        0.5 * (self.port - STARBOARD)
    }

    fn half(&self) -> f32 {
        0.5 * (self.port + STARBOARD)
    }

    fn bay(&self) -> f32 {
        self.long / self.bays as f32
    }

    /// The vault's height over `v`, barrel or pointed.
    fn vault(&self, v: f32) -> f32 {
        let s = ((v - self.middle()) / self.half()).clamp(-1.0, 1.0);
        let lift = if self.pointed {
            1.0 - s.abs().powf(1.6)
        } else {
            (1.0 - s * s).sqrt()
        };
        self.high + self.rise * lift
    }

    /// How high the arcades spring, under the galleries if there are any.
    fn spring(&self) -> f32 {
        self.high * if self.galleries { 0.52 } else { 0.8 }
    }

    /// The galleries' floor.
    fn gallery(&self) -> f32 {
        self.spring() + 0.62 * self.bay()
    }
}

/// The museum of one visit, laid out along the chord of the corridor over
/// its stretch of the leg.
#[derive(Clone, Copy, Debug)]
pub struct Museum {
    /// The world (x, z) of the front door, on the lane.
    pub origin: [f32; 2],
    /// Unit vectors along the halls and across them to port.
    pub axis: [f32; 2],
    pub across: [f32; 2],
    pub long: f32,
    pub floor: f32,
    pub halls: [Hall; 3],
    /// The loop round the natural history hall's island: its middle along
    /// the axis, and its radius (the island is that far to port).
    pub loop_at: f32,
    pub radius: f32,
    pub seed: u32,
}

impl Museum {
    /// The museum nearest `x` along the voyage, if this visit has one there.
    pub fn near(order: &[usize; BIOMES], seed: f32, x: f32) -> Option<Self> {
        let leg = crate::voyage::LEG as f32;
        let k0 = (x / leg).floor() as i32;
        let k = (k0 - 1..=k0 + 1)
            .filter(|&k| order[k.rem_euclid(BIOMES as i32) as usize] == MUSEUM)
            .min_by(|a, b| {
                let d = |k: i32| ((k as f32 + 0.5) * leg - x).abs();
                d(*a).total_cmp(&d(*b))
            })?;
        Some(Self::of_leg(k, seed))
    }

    fn of_leg(k: i32, seed: f32) -> Self {
        let leg = crate::voyage::LEG as f32;
        let salt = seed.to_bits() ^ (k as u32).wrapping_mul(0x9e37_79b9);
        let mut h = Hasher::new(salt, 2400);
        let (x0, x1) = (k as f32 * leg + FRONT, k as f32 * leg + BACK);
        let (z0, z1) = (corridor(x0), corridor(x1));
        let long = (x1 - x0).hypot(z1 - z0);
        let axis = [(x1 - x0) / long, (z1 - z0) / long];
        let across = [-axis[1], axis[0]];
        // The natural history hall is as long as its loop needs; the
        // picture gallery takes the larger share of what is left.
        let radius = 1.9 + 0.1 * h.next();
        let natural = 2.0 * radius + 2.6;
        let rest = long - natural;
        let gallery = 0.55 + 0.1 * h.next();
        let mut kinds = [HallKind::Natural, HallKind::Minerals, HallKind::Masters];
        h.shuffle(&mut kinds);
        let mut from = 0.0;
        let halls = kinds.map(|kind| {
            let long = match kind {
                HallKind::Natural => natural,
                HallKind::Minerals => rest * (1.0 - gallery),
                HallKind::Masters => rest * gallery,
            };
            let (port, high, rise, wall, trim) = match kind {
                HallKind::Natural => (
                    2.0 * radius + 1.35,
                    1.75 + 0.35 * h.next(),
                    0.7 + 0.4 * h.next(),
                    [0.80, 0.68, 0.50],
                    [0.50, 0.57, 0.63],
                ),
                HallKind::Minerals => (
                    1.7 + 0.5 * h.next(),
                    1.5 + 0.3 * h.next(),
                    0.45 + 0.3 * h.next(),
                    [0.20, 0.22, 0.26],
                    [0.55, 0.50, 0.42],
                ),
                // The Rijksmuseum's deep red, or the green of older
                // galleries.
                HallKind::Masters => (
                    1.9 + 0.4 * h.next(),
                    1.8 + 0.3 * h.next(),
                    0.5 + 0.35 * h.next(),
                    if h.next() < 0.6 {
                        [0.46, 0.13, 0.12]
                    } else {
                        [0.18, 0.30, 0.22]
                    },
                    [0.82, 0.74, 0.58],
                ),
            };
            let bay = 0.95 + 0.3 * h.next();
            let hall = Hall {
                kind,
                from,
                long,
                port,
                high,
                rise,
                bays: ((long / bay).round() as usize).max(2),
                pointed: h.next() < 0.4,
                galleries: kind == HallKind::Natural && h.next() < 0.75,
                wall,
                trim,
            };
            from += long;
            hall
        });
        let natural = halls.iter().find(|h| h.kind == HallKind::Natural);
        let loop_at = natural.map_or(0.0, |h| h.from + 0.5 * h.long);
        Self {
            origin: [x0, z0],
            axis,
            across,
            long,
            floor: -SEABED[MUSEUM] + PLINTH,
            halls,
            loop_at,
            radius,
            seed: salt,
        }
    }

    /// The world point `u` along and `v` across, at height `y`.
    pub fn at(&self, u: f32, v: f32, y: f32) -> [f32; 3] {
        [
            self.origin[0] + self.axis[0] * u + self.across[0] * v,
            y,
            self.origin[1] + self.axis[1] * u + self.across[1] * v,
        ]
    }

    /// The museum's (u, v) of a world point.
    pub fn local(&self, x: f32, z: f32) -> (f32, f32) {
        let (dx, dz) = (x - self.origin[0], z - self.origin[1]);
        (
            dx * self.axis[0] + dz * self.axis[1],
            dx * self.across[0] + dz * self.across[1],
        )
    }

    /// The world direction of (du, dv), level.
    fn direction(&self, du: f32, dv: f32) -> [f32; 2] {
        [
            self.axis[0] * du + self.across[0] * dv,
            self.axis[1] * du + self.across[1] * dv,
        ]
    }

    /// The yaw that turns a model's +x along the halls (and its +z across
    /// them to port).
    fn yaw(&self) -> f32 {
        yaw_along(self.axis[0], self.axis[1])
    }

    /// How far the lane at world x is inside the building, with `margin`
    /// either end: 1 inside, easing to 0.
    pub fn along(&self, x: f32, margin: f32) -> f32 {
        let (u, _) = self.local(x, corridor(x));
        smooth(-margin, 0.0, u) * smooth(self.long + margin, self.long, u)
    }

    /// The boat's way through the museum at voyage `x`, if it is on it.
    pub fn tour(&self, x: f32) -> Option<Tour> {
        // Where the voyage's x crosses the ends of the way and the doors.
        let x_at = |u: f32| self.origin[0] + self.axis[0] * u;
        let (s0, s1) = (x_at(-APPROACH), x_at(self.long + APPROACH));
        if !(s0..s1).contains(&x) {
            return None;
        }
        // Outside the doors the way runs at the voyage's pace; inside it
        // runs quicker, by as much as it must to go round the loop in the
        // same time, easing between the two over a unit about each door.
        let (in0, in1) = (x_at(0.0), x_at(self.long));
        let inside = |s: f32| smooth(in0 - 0.5, in0 + 0.5, s) * smooth(in1 + 0.5, in1 - 0.5, s);
        let integral = |to: f32| {
            let steps = 64;
            let dx = (to - s0) / steps as f32;
            (0..steps)
                .map(|i| inside(s0 + (i as f32 + 0.5) * dx) * dx)
                .sum::<f32>()
        };
        let way = self.long + 2.0 * APPROACH + TAU * self.radius;
        let extra = (way - (s1 - s0)) / integral(s1).max(1e-3);
        let a = (x - s0) + extra * integral(x);
        let (u, v) = self.on_way(a);
        // The heading some way ahead, about four seconds' run, so the hull,
        // whose turn lags behind its helm, follows the loop rather than
        // swinging wide of it.
        let lead = 4.0 * crate::voyage::SPEED as f32 * (1.0 + extra * inside(x));
        let (u1, v1) = self.on_way(a + lead);
        let (u2, v2) = self.on_way(a + lead + 0.05);
        let d = self.direction(u2 - u1, v2 - v1);
        let p = self.at(u, v, 0.0);
        Some(Tour {
            x: p[0],
            z: p[2],
            heading: yaw_along(d[0], d[1]),
            weight: smooth(s0, s0 + 1.0, x) * smooth(s1, s1 - 1.0, x),
        })
    }

    /// The point `a` along the way, in (u, v).
    fn on_way(&self, a: f32) -> (f32, f32) {
        let start = self.loop_at + APPROACH;
        let lap = TAU * self.radius;
        if a < start {
            (a - APPROACH, 0.0)
        } else if a < start + lap {
            // Once round to port: the window, on the port side, looks in at
            // the island all the way round.
            let t = (a - start) / self.radius;
            (
                self.loop_at + self.radius * t.sin(),
                self.radius * (1.0 - t.cos()),
            )
        } else {
            (a - lap - APPROACH, 0.0)
        }
    }
}

/// Where the tour puts the boat's middle.
#[derive(Clone, Copy, Debug)]
pub struct Tour {
    pub x: f32,
    pub z: f32,
    /// The heading, as the helm's: a yaw, its nose along (cos, -sin).
    pub heading: f32,
    /// How far the tour has taken over from the track: 0 to 1 over the
    /// unit after the way leaves it, and back before it rejoins it.
    pub weight: f32,
}

/// The tour at voyage `x`, if the boat is on one.
pub fn tour(seabed: &Seabed, x: f32) -> Option<Tour> {
    Museum::near(&seabed.order, seabed.seed, x)?.tour(x)
}

/// Whether the museum is near enough at voyage `x` that its models and
/// paintings should be in: from well before its approach until a little
/// after the boat is out of its back door.
pub fn museum_ahead(seabed: &Seabed, x: f32) -> bool {
    let leg = crate::voyage::LEG as f32;
    Museum::near(&seabed.order, seabed.seed, x).is_some_and(|m| {
        let (u, _) = m.local(x, corridor(x));
        (-APPROACH - 0.6 * leg..m.long + APPROACH + 0.15 * leg).contains(&u)
    })
}

/// How far the voyage at `x` is outside the museum's way, along the lane,
/// if a museum is near: 0 on it.
pub fn museum_reach(seabed: &Seabed, x: f32) -> Option<f32> {
    let m = Museum::near(&seabed.order, seabed.seed, x)?;
    let (u, _) = m.local(x, corridor(x));
    Some((-APPROACH - u).max(u - m.long - APPROACH).max(0.0))
}

/// The museum's roof over the boat's lane at `x`, the hull's margin under
/// the door heads, and how far it takes over, for `Seabed::ceiling`.
pub fn roof(order: &[usize; BIOMES], seed: f32, x: f32) -> Option<(f32, f32)> {
    let m = Museum::near(order, seed, x)?;
    let k = m.along(x, 1.0);
    (k > 0.0).then_some((m.floor + DOOR_HIGH - 0.35, k))
}

/// How each specimen is shown.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Mount {
    /// On the island's low stepped platform.
    Platform,
    /// On a tall stone plinth.
    Plinth,
    /// In a glass case on a pedestal.
    Case,
}

/// A specimen: the model, how long it stands in the hall along its longest
/// side, how long that side is in the model (each is cast a unit tall), and
/// its mount.
type Specimen = (Model, f32, f32, Mount);

const SPECIMENS: [Specimen; 18] = [
    (Model::Mammoth, 0.85, 1.47, Mount::Platform),
    (Model::Triceratops, 0.8, 2.10, Mount::Platform),
    (Model::GroundSloth, 0.45, 2.34, Mount::Plinth),
    (Model::Megalodon, 0.45, 1.0, Mount::Platform),
    (Model::PilotWhale, 0.4, 2.05, Mount::Plinth),
    (Model::WalrusWhale, 0.36, 1.34, Mount::Plinth),
    (Model::Tortoise, 0.4, 2.24, Mount::Plinth),
    (Model::StegosaurusSpike, 0.4, 4.39, Mount::Plinth),
    (Model::DeerSkull, 0.28, 1.87, Mount::Case),
    (Model::Diplocaulus, 0.26, 1.44, Mount::Case),
    (Model::Allosaurus, 0.26, 1.48, Mount::Case),
    (Model::Dinictis, 0.26, 1.55, Mount::Case),
    (Model::Archaeopteryx, 0.24, 1.21, Mount::Case),
    (Model::Trilobite, 0.18, 1.24, Mount::Case),
    (Model::Murex, 0.2, 3.02, Mount::Case),
    (Model::Porcupinefish, 0.22, 2.91, Mount::Case),
    (Model::KingCrab, 0.26, 1.86, Mount::Case),
    (Model::Seadragon, 0.3, 3.44, Mount::Case),
];

/// A mineral: the model, its colour, whether it is a crystal lit through
/// (glass), and the colour it fluoresces under the ultraviolet, if it does.
type Mineral = (Model, [f32; 3], bool, Option<[f32; 3]>);

const MINERALS: [Mineral; 14] = [
    (Model::CrystalCluster, [1.0, 1.0, 1.0], true, None),
    (
        Model::CrystalCluster,
        [0.85, 0.45, 1.0],
        true,
        Some([0.5, 0.2, 1.0]),
    ),
    (
        Model::Crystal,
        [0.45, 1.0, 0.55],
        true,
        Some([0.2, 1.0, 0.3]),
    ),
    (Model::Crystal, [1.0, 0.75, 0.30], true, None),
    (
        Model::CrystalCluster,
        [0.40, 0.75, 1.0],
        true,
        Some([0.3, 0.5, 1.0]),
    ),
    (Model::Crystal, [1.0, 0.55, 0.65], true, None),
    (
        Model::CrystalCluster,
        [0.55, 1.0, 0.95],
        true,
        Some([0.2, 1.0, 0.8]),
    ),
    (Model::Pyrite, [1.0; 3], false, None),
    (Model::Bismuth, [1.0; 3], false, None),
    (Model::Malachite, [1.0; 3], false, None),
    (Model::Geode, [1.0; 3], false, Some([0.7, 0.3, 1.0])),
    (Model::Tourmaline, [1.0; 3], false, None),
    (Model::Geode, [0.9, 1.0, 0.9], false, Some([1.0, 0.35, 0.2])),
    (
        Model::Malachite,
        [0.8, 0.9, 1.1],
        false,
        Some([0.1, 1.0, 0.4]),
    ),
];

const STONE: [f32; 3] = [0.74, 0.70, 0.62];
const GILT: [f32; 3] = [0.86, 0.66, 0.30];
const BRASS: [f32; 3] = [0.80, 0.62, 0.32];
const WIRE: [f32; 3] = [0.30, 0.30, 0.32];
const DARK_WOOD: [f32; 3] = [0.24, 0.15, 0.10];
const CARD: [f32; 3] = [0.92, 0.88, 0.76];

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// A box of `half` extents at `centre`, square to the halls, if it is in
/// view.
fn slab(w: &World, f: &mut Frame, m: &Museum, centre: [f32; 3], half: [f32; 3], colour: [f32; 3]) {
    if w.visible(centre, half[0].max(half[1]).max(half[2]) * 1.8) {
        f.put(Model::Box, place(centre, m.yaw(), 0.0, 0.0, half), colour)
            .element(element::MUSEUM);
    }
}

/// A rod from `a` to `b`.
fn rod(f: &mut Frame, a: [f32; 3], b: [f32; 3], r: f32, colour: [f32; 3]) {
    f.put(Model::Cylinder, between(a, b, r), colour)
        .element(element::MUSEUM);
}

/// The building, its exhibits and its paintings. Static: nothing here reads
/// the clock; what moves with it is [`museum_lights`].
pub fn museum(w: &World, f: &mut Frame) {
    let Some(m) = Museum::near(&w.seabed.order, w.seabed.seed, w.camera()) else {
        return;
    };
    if !w.visible(m.at(0.5 * m.long, 1.5, m.floor + 1.0), 0.6 * m.long + 3.0) {
        return;
    }
    for (i, hall) in m.halls.iter().enumerate() {
        building(w, f, &m, hall, i == 0);
        match hall.kind {
            HallKind::Natural => natural_history(w, f, &m, hall),
            HallKind::Minerals => mineral_cases(w, f, &m, hall),
            HallKind::Masters => masters(w, f, &m, hall),
        }
    }
    facade(w, f, &m);
}

/// One hall's floor, long walls with their pilasters, frieze and dado, the
/// vault's ribs and panels round its skylight, and its end walls.
fn building(w: &World, f: &mut Frame, m: &Museum, hall: &Hall, first: bool) {
    let y = m.floor;
    let (u0, u1) = (hall.from, hall.to());
    let (vm, half, bay) = (hall.middle(), hall.half(), hall.bay());
    let thick = 0.06;
    // The floor, polished stone, laid bay by bay so each is culled alone.
    for i in 0..hall.bays {
        let c = m.at(u0 + (i as f32 + 0.5) * bay, vm, y - 0.05);
        if w.visible(c, half.max(0.5 * bay) * 1.2) {
            f.put(
                Model::Box,
                place(c, m.yaw(), 0.0, 0.0, [0.5 * bay, 0.05, half]),
                [0.62, 0.58, 0.52],
            )
            .textured(Surface::Marble, 2.0)
            .element(element::MUSEUM);
        }
    }
    for (v, out) in [(-STARBOARD, -1.0f32), (hall.port, 1.0)] {
        for i in 0..hall.bays {
            let uc = u0 + (i as f32 + 0.5) * bay;
            let wall = [0.5 * bay, 0.5 * hall.high, thick];
            slab(
                w,
                f,
                m,
                m.at(uc, v + out * thick, y + 0.5 * hall.high),
                wall,
                hall.wall,
            );
            // A frieze along the top and a dark dado along the foot.
            let frieze = [0.5 * bay, 0.06, 1.6 * thick];
            slab(
                w,
                f,
                m,
                m.at(uc, v, y + hall.high - 0.06),
                frieze,
                hall.trim,
            );
            let dado = mix3(hall.wall, [0.1; 3], 0.5);
            slab(
                w,
                f,
                m,
                m.at(uc, v, y + 0.08),
                [0.5 * bay, 0.08, 1.3 * thick],
                dado,
            );
        }
        for i in 0..=hall.bays {
            let u = u0 + i as f32 * bay;
            let pilaster = [0.05, 0.5 * hall.high, 1.8 * thick];
            slab(
                w,
                f,
                m,
                m.at(u, v, y + 0.5 * hall.high),
                pilaster,
                hall.trim,
            );
        }
    }
    // The vault: a rib over each bay's line, and between them panels up
    // from either side to the skylight along the crown, open to the water.
    let pieces = 10;
    let arc = |k: usize, u: f32| {
        let v = -STARBOARD + k as f32 / pieces as f32 * 2.0 * half;
        m.at(u, v, y + hall.vault(v))
    };
    let roof_seen = |u: f32| {
        w.visible(
            m.at(u, vm, y + hall.high + 0.5 * hall.rise),
            half + hall.rise,
        )
    };
    for i in 0..=hall.bays {
        let u = u0 + i as f32 * bay;
        if roof_seen(u) {
            for k in 0..pieces {
                rod(f, arc(k, u), arc(k + 1, u), 0.035, hall.trim);
            }
        }
    }
    let panel = mix3(hall.wall, [0.85, 0.80, 0.70], 0.45);
    for i in 0..hall.bays {
        let uc = u0 + (i as f32 + 0.5) * bay;
        if !roof_seen(uc) {
            continue;
        }
        for k in 0..pieces {
            if k == pieces / 2 - 1 || k == pieces / 2 {
                continue;
            }
            let (a, b) = (arc(k, uc), arc(k + 1, uc));
            let mid = [
                0.5 * (a[0] + b[0]),
                0.5 * (a[1] + b[1]),
                0.5 * (a[2] + b[2]),
            ];
            let step = 2.0 * half / pieces as f32;
            // Rolled so its width climbs with the vault toward port.
            let slope = (b[1] - a[1]).atan2(step);
            let wide = 0.5 * step.hypot(b[1] - a[1]) + 0.01;
            f.put(
                Model::Box,
                place(mid, m.yaw(), 0.0, -slope, [0.5 * bay, 0.02, wide]),
                panel,
            )
            .element(element::MUSEUM);
        }
        // The skylight's glazing bars, across it and along its ridge.
        let (a, b) = (arc(pieces / 2 - 1, uc), arc(pieces / 2 + 1, uc));
        rod(f, a, b, 0.01, [0.25; 3]);
        let ridge = a[1].max(b[1]) + 0.12;
        let (a, b) = (
            m.at(uc - 0.5 * bay, vm, ridge),
            m.at(uc + 0.5 * bay, vm, ridge),
        );
        rod(f, a, b, 0.01, [0.25; 3]);
    }
    // An end wall at each end, but one between two halls, which serves both.
    if first {
        end_wall(w, f, m, hall, u0);
    }
    end_wall(w, f, m, hall, u1);
}

/// An end wall across the hall at `u`, with the doorway on the lane, the arch
/// round its head, and the gable up into the vault.
fn end_wall(w: &World, f: &mut Frame, m: &Museum, hall: &Hall, u: f32) {
    let y = m.floor;
    let thick = 0.08;
    let span = |a: f32, b: f32, h0: f32, h1: f32, f: &mut Frame| {
        slab(
            w,
            f,
            m,
            m.at(u, 0.5 * (a + b), y + 0.5 * (h0 + h1)),
            [thick, 0.5 * (h1 - h0), 0.5 * (b - a)],
            hall.wall,
        );
    };
    span(-STARBOARD, -DOOR, 0.0, hall.high, f);
    span(DOOR, hall.port, 0.0, hall.high, f);
    span(-DOOR, DOOR, DOOR_HIGH, hall.high, f);
    // The arch round the door's head, its crown at the lintel.
    let door = m.at(u, 0.0, y + DOOR_HIGH - 1.2 * DOOR);
    if w.visible(door, 1.0) {
        f.put(
            Model::MuseumArch,
            place(
                door,
                m.yaw() + FRAC_PI_2,
                0.0,
                0.0,
                [DOOR, DOOR * 0.98, 1.0],
            ),
            [1.0; 3],
        )
        .element(element::MUSEUM);
    }
    // The gable, in courses stepping in with the vault.
    let courses = 6;
    for c in 0..courses {
        let (t0, t1) = (c as f32 / courses as f32, (c + 1) as f32 / courses as f32);
        let s = if hall.pointed {
            (1.0 - t1).powf(1.0 / 1.6)
        } else {
            (1.0 - t1 * t1).sqrt()
        };
        let reach = (hall.half() * s).max(0.06);
        span(
            hall.middle() - reach,
            hall.middle() + reach,
            hall.high + hall.rise * t0,
            hall.high + hall.rise * t1,
            f,
        );
    }
}

/// The front: twin towers with spires either side of a deep portal, as the
/// Natural History Museum's front stands on the Cromwell Road, and broad
/// steps up to it.
fn facade(w: &World, f: &mut Frame, m: &Museum) {
    let y = m.floor;
    let first = m.halls[0];
    let buff = [0.80, 0.68, 0.50];
    let slate = [0.48, 0.55, 0.62];
    let tower = first.high + first.rise + 0.8;
    for v in [-STARBOARD - 0.25, first.port + 0.25] {
        if !w.visible(m.at(-0.05, v, y + 0.5 * tower), tower) {
            continue;
        }
        slab(
            w,
            f,
            m,
            m.at(-0.05, v, y + 0.5 * tower),
            [0.3, 0.5 * tower, 0.3],
            buff,
        );
        // Courses of slate between the buff.
        for k in 1..5 {
            let at = m.at(-0.05, v, y + tower * k as f32 / 5.0);
            slab(w, f, m, at, [0.32, 0.03, 0.32], slate);
        }
        // Tall narrow windows, dark.
        for k in 0..3 {
            let at = m.at(-0.355, v, y + 0.5 + k as f32 * 0.55);
            slab(w, f, m, at, [0.01, 0.16, 0.06], [0.05; 3]);
        }
        f.put(
            Model::Cone,
            place(
                m.at(-0.05, v, y + tower),
                m.yaw(),
                0.0,
                0.0,
                [0.3, 0.9, 0.3],
            ),
            slate,
        )
        .element(element::MUSEUM);
    }
    // The portal: a deep arch over the front door.
    let portal = m.at(-0.12, 0.0, y + DOOR_HIGH - 1.2 * DOOR);
    if w.visible(portal, 2.0) {
        f.put(
            Model::MuseumArch,
            place(
                portal,
                m.yaw() + FRAC_PI_2,
                0.0,
                0.0,
                [DOOR * 1.08, DOOR * 1.06, 2.0],
            ),
            [1.0; 3],
        )
        .element(element::MUSEUM);
    }
    // Steps up to the doors, broad and shallow, the lowest the deepest.
    for k in 0..3 {
        let deep = 0.35 - k as f32 * 0.1;
        slab(
            w,
            f,
            m,
            m.at(-deep, first.middle(), y - 0.1 + 0.035 * k as f32),
            [deep, 0.035, first.half() + 0.4],
            STONE,
        );
    }
}

/// The natural history hall: its arcades and galleries, the island in the
/// loop with the three largest skeletons on it and the whale over it, the
/// rest of the specimens in the aisles and the natural history paintings on
/// the walls.
fn natural_history(w: &World, f: &mut Frame, m: &Museum, hall: &Hall) {
    let y = m.floor;
    let mut h = Hasher::new(m.seed, 2500);
    let aisle = 0.55;
    let lines = [-STARBOARD + aisle, hall.port - aisle];
    let bay = hall.bay();
    let spring = hall.spring();
    for &v in &lines {
        // The piers with their capitals, and an arch from each to the next.
        for i in 0..=hall.bays {
            let u = hall.from + i as f32 * bay;
            if !w.visible(m.at(u, v, y + 0.5 * spring), spring) {
                continue;
            }
            f.put(
                Model::Shaft,
                place(m.at(u, v, y), 0.0, 0.0, 0.0, [0.07, spring - 0.08, 0.07]),
                hall.wall,
            )
            .element(element::MUSEUM);
            f.put(
                Model::Capital,
                place(m.at(u, v, y + spring - 0.08), 0.0, 0.0, 0.0, [0.08; 3]),
                hall.trim,
            )
            .element(element::MUSEUM);
            if i < hall.bays {
                let r = 0.5 * bay;
                let rise = if hall.pointed { 1.3 } else { 1.0 };
                f.put(
                    Model::MuseumArch,
                    place(
                        m.at(u + r, v, y + spring),
                        m.yaw(),
                        0.0,
                        0.0,
                        [0.97 * r, 0.97 * r * rise, 0.5],
                    ),
                    [1.0; 3],
                )
                .element(element::MUSEUM);
            }
        }
        // The gallery over the arcade, back to the wall, with a balustrade
        // along its edge.
        if hall.galleries {
            let wall = if v < hall.middle() {
                -STARBOARD
            } else {
                hall.port
            };
            let floor = hall.gallery();
            slab(
                w,
                f,
                m,
                m.at(hall.from + 0.5 * hall.long, 0.5 * (v + wall), y + floor),
                [0.5 * hall.long, 0.04, 0.5 * (wall - v).abs() + 0.05],
                hall.trim,
            );
            let rail = floor + 0.16;
            let middle = m.at(hall.from + 0.5 * hall.long, v, y + rail);
            if w.visible(middle, 0.5 * hall.long) {
                let (a, b) = (m.at(hall.from, v, y + rail), m.at(hall.to(), v, y + rail));
                rod(f, a, b, 0.016, BRASS);
                for i in 0..(hall.long / 0.18) as usize {
                    let u = hall.from + 0.09 + i as f32 * 0.18;
                    rod(
                        f,
                        m.at(u, v, y + floor),
                        m.at(u, v, y + rail),
                        0.008,
                        hall.trim,
                    );
                }
            }
        }
    }

    // The island: a round platform of two steps, a brass rail on posts
    // round its edge.
    let centre = m.at(m.loop_at, m.radius, y);
    let island = m.radius - 0.75;
    if w.visible([centre[0], y + 1.0, centre[2]], island + 1.5) {
        for (r, top, colour) in [
            (island, 0.05, STONE),
            (island - 0.08, 0.1, [0.66, 0.62, 0.55]),
        ] {
            f.put(
                Model::Cylinder,
                place(centre, 0.0, 0.0, 0.0, [r, top, r]),
                colour,
            )
            .element(element::MUSEUM);
        }
        let posts = 16;
        let edge = |k: usize, up: f32| {
            let a = k as f32 / posts as f32 * TAU;
            let r = island - 0.04;
            [centre[0] + r * a.cos(), y + up, centre[2] + r * a.sin()]
        };
        for k in 0..posts {
            rod(f, edge(k, 0.05), edge(k, 0.2), 0.006, BRASS);
            rod(f, edge(k, 0.2), edge(k + 1, 0.2), 0.005, BRASS);
        }
    }
    let mut specimens = SPECIMENS;
    h.shuffle(&mut specimens);
    let (big, rest): (Vec<_>, Vec<_>) = specimens.into_iter().partition(|s| s.3 == Mount::Platform);
    // In a line down the middle of the island, along the hall, the largest
    // in the middle: each square to the hall and a hand's breadth clear of
    // the next, the line centred on the island, the labels at its rim on
    // the lane's side.
    let mut big = big;
    big.sort_by(|a, b| b.1.total_cmp(&a.1));
    big.swap(0, 1);
    // How far each reaches along the hall from its middle: its length, or
    // for the flat tooth, its breadth (it is turned to face across).
    let reach = |&(model, size, longest, _): &Specimen| {
        if model == Model::Megalodon {
            0.5 * 0.84 * size / longest
        } else {
            0.5 * size
        }
    };
    let gap = 0.12;
    let span: f32 = big.iter().map(reach).sum::<f32>() * 2.0 + gap * (big.len() - 1) as f32;
    let mut along = -0.5 * span;
    for specimen in &big {
        let &(model, size, longest, _) = specimen;
        along += reach(specimen);
        let p = m.at(m.loop_at + along, m.radius, y + 0.1);
        along += reach(specimen) + gap;
        let s = size / longest;
        // The tooth is scanned point down: it stands turned over, its root
        // on the platform and its point up, its face across the hall.
        let rows = if model == Model::Megalodon {
            place([p[0], p[1] + s, p[2]], m.yaw() + FRAC_PI_2, 0.0, PI, [s; 3])
        } else {
            place(p, m.yaw(), 0.0, 0.0, [s; 3])
        };
        if !w.visible([p[0], p[1] + 0.5 * s, p[2]], size) {
            continue;
        }
        f.put(model, rows, [1.0; 3]).element(element::MUSEUM);
        let (u, _) = m.local(p[0], p[2]);
        let foot = m.at(u, m.radius - island + 0.12, y + 0.1);
        let out = m.direction(0.0, -1.0);
        label(f, foot, out[0].atan2(out[1]), 0.8);
    }
    // The whale, hung over the island on cables from the vault, diving a
    // little, a little across the hall.
    let length = 2.0 + 0.4 * h.next();
    // Low enough that the window, which looks out level, sees it over the
    // island, and clear above the boat's hull.
    let whale = [centre[0], y + 1.35, centre[2]];
    let tilt = 0.08;
    let yaw = m.yaw() + 0.4 * (h.next() - 0.5);
    if w.visible(whale, 0.6 * length) {
        f.put(
            Model::WhaleSkeleton,
            place(whale, yaw, -tilt, 0.0, [length; 3]),
            [1.0; 3],
        )
        .element(element::MUSEUM);
        let (s, c) = yaw.sin_cos();
        for k in 0..5 {
            let along = (k as f32 / 4.0 - 0.5) * 0.75 * length;
            let p = [
                whale[0] + c * along,
                whale[1] - along * tilt.sin() + 0.02,
                whale[2] - s * along,
            ];
            let (cu, cv) = m.local(p[0], p[2]);
            rod(f, p, m.at(cu, cv, y + hall.vault(cv) - 0.04), 0.004, WIRE);
        }
    }

    // The aisles: the rest of the specimens between the piers, each on its
    // plinth or in its case, turned to the hall.
    let mut slots = Vec::new();
    for &v in &lines {
        let starboard = v < hall.middle();
        let behind = if starboard { v - 0.27 } else { v + 0.27 };
        for i in 0..hall.bays {
            slots.push((hall.from + (i as f32 + 0.5) * bay, behind, starboard));
        }
    }
    h.shuffle(&mut slots);
    for (&(model, size, longest, mount), &(u, v, starboard)) in rest.iter().zip(&slots) {
        let p = m.at(u, v, y);
        // A model's +z is to port: the starboard aisle's face that way.
        let toward = if starboard { 0.0 } else { PI };
        let yaw = m.yaw() + toward + 0.5 * (h.next() - 0.5);
        if w.visible([p[0], y + 0.5, p[2]], 0.7) {
            stand(f, p, model, [size, size / longest], mount, yaw);
        }
    }

    // The paintings: on the aisles' walls behind the specimens and above
    // them, on the walls over the galleries, and on the end walls.
    let mut walls = Vec::new();
    for (v, face) in [(-STARBOARD + 0.07, 1.0f32), (hall.port - 0.07, -1.0)] {
        for i in 0..hall.bays {
            let u = hall.from + (i as f32 + 0.5) * bay;
            let high = if hall.galleries {
                hall.gallery() - 0.1
            } else {
                spring + 0.45 * bay
            };
            let wall = |low: f32, high: f32| Wall {
                u,
                v,
                along: [1.0, 0.0],
                face: [0.0, face],
                width: 0.8 * bay,
                low,
                high,
            };
            walls.push(wall(0.8, high));
            if hall.galleries && hall.high - hall.gallery() > 0.7 {
                walls.push(wall(hall.gallery() + 0.35, hall.high - 0.15));
            }
        }
    }
    for (u, face) in [(hall.from + 0.09, 1.0f32), (hall.to() - 0.09, -1.0)] {
        walls.push(Wall {
            u,
            v: 0.5 * (DOOR + hall.port - aisle),
            along: [0.0, 1.0],
            face: [face, 0.0],
            width: 0.85 * (hall.port - aisle - DOOR),
            low: 0.3,
            high: spring - 0.1,
        });
    }
    h.shuffle(&mut walls);
    let mut paintings: Vec<usize> = (0..PAINTINGS.len())
        .filter(|&i| !PAINTINGS[i].masters)
        .collect();
    h.shuffle(&mut paintings);
    let how = Hang {
        scale: NATURAL_SCALE,
        rows: 1,
        gap: 0.08,
    };
    hang(w, f, m, &walls, &paintings, how);
}

/// A stretch of wall to hang paintings on: its middle at (u, v), the
/// directions along it and out of it in (u, v), its width, and the band of
/// it they hang in, as heights above the floor.
struct Wall {
    u: f32,
    v: f32,
    along: [f32; 2],
    face: [f32; 2],
    width: f32,
    low: f32,
    high: f32,
}

/// How a hall hangs its paintings: `scale` is about how wide a painting of
/// a square metre hangs, in `rows` to a wall, `gap` apart.
#[derive(Clone, Copy)]
struct Hang {
    scale: f32,
    rows: usize,
    gap: f32,
}

/// Hangs `paintings`, in turn, on `walls`, in turn, each row filled from the
/// left with what fits and centred.
fn hang(w: &World, f: &mut Frame, m: &Museum, walls: &[Wall], paintings: &[usize], how: Hang) {
    let mut next = paintings.iter().copied().peekable();
    for wall in walls {
        let row_high = (wall.high - wall.low) / how.rows as f32;
        for row in 0..how.rows {
            let mut items = Vec::new();
            let mut used = 0.0;
            while let Some(i) = next.peek().copied() {
                let (pw, ph) = size_of(&PAINTINGS[i], how.scale, 0.84 * row_high);
                if used + pw + how.gap > wall.width {
                    if items.is_empty() {
                        // Too wide for the wall: left in store.
                        next.next();
                        continue;
                    }
                    break;
                }
                used += pw + how.gap;
                items.push((i, pw, ph));
                next.next();
            }
            let mid = wall.low + row_high * (how.rows - 1 - row) as f32 + 0.5 * row_high;
            let mut along = -0.5 * used + 0.5 * how.gap;
            for (i, pw, ph) in items {
                let c = along + 0.5 * pw;
                along += pw + how.gap;
                let (u, v) = (wall.u + wall.along[0] * c, wall.v + wall.along[1] * c);
                let p = m.at(u, v, m.floor + mid);
                if w.visible(p, pw.max(ph)) {
                    let face = m.direction(wall.face[0], wall.face[1]);
                    painting(f, p, face, i, [pw, ph], m.floor + wall.high + 0.1);
                }
            }
        }
        if next.peek().is_none() {
            return;
        }
    }
}

/// How large the halls hang their paintings ([`Hang::scale`]): the natural
/// history hall's, the picture gallery's, and the gallery's centrepiece.
const NATURAL_SCALE: f32 = 0.28;
const MASTERS_SCALE: f32 = 0.30;
const CENTREPIECE_SCALE: f32 = 0.42;

/// How big a painting hangs: its canvas's shape, and a size that grows with
/// the real one's area but far more slowly, so the cabinet pieces are not
/// lost and the Night Watch does not fill the hall; no taller than `most`.
fn size_of(p: &Painting, scale: f32, most: f32) -> (f32, f32) {
    let area = p.height * p.width / 10_000.0;
    let side = scale * (0.75 + 0.55 * (1.0 + area).ln());
    let aspect = (p.width / p.height).sqrt();
    let (w, h) = (side * aspect, side / aspect);
    let k = (most / h).min(1.0);
    (w * k, h * k)
}

/// A painting of `size` centred at `p`, facing the world direction `face`:
/// its canvas, the gilt frame round it and a dark liner inside that, the two
/// cords it hangs by from a hook on the rail at height `rail`, and its
/// label.
fn painting(f: &mut Frame, p: [f32; 3], face: [f32; 2], layer: usize, size: [f32; 2], rail: f32) {
    // A model's +z turned along `face`; its +x is then along the wall.
    let yaw = face[0].atan2(face[1]);
    let right = [face[1], -face[0]];
    let at = |r: f32, up: f32, out: f32| {
        [
            p[0] + right[0] * r + face[0] * out,
            p[1] + up,
            p[2] + right[1] * r + face[1] * out,
        ]
    };
    let [width, height] = size;
    f.put(
        Model::Canvas,
        place(at(0.0, 0.0, 0.012), yaw, 0.0, 0.0, [width, height, 1.0]),
        [1.0; 3],
    )
    .material(material::CANVAS)
    .glowing(0.35)
    .patterned([layer as f32, 0.0, 0.0])
    .element(element::MUSEUM);
    // The frame's four mouldings, overlapping at the corners, and the
    // liner along the top and foot just inside them.
    let band = 0.03 + 0.04 * width.max(height);
    let (hw, hh) = (0.5 * width, 0.5 * height);
    let liner = [0.18, 0.14, 0.10];
    for (r, up, half, depth, colour) in [
        (0.0, hh + 0.5 * band, [hw + band, 0.5 * band], 0.02, GILT),
        (0.0, -hh - 0.5 * band, [hw + band, 0.5 * band], 0.02, GILT),
        (hw + 0.5 * band, 0.0, [0.5 * band, hh], 0.02, GILT),
        (-hw - 0.5 * band, 0.0, [0.5 * band, hh], 0.02, GILT),
        (0.0, hh + 0.01, [hw + 0.01, 0.01], 0.014, liner),
        (0.0, -hh - 0.01, [hw + 0.01, 0.01], 0.014, liner),
    ] {
        let rows = place(at(r, up, depth), yaw, 0.0, 0.0, [half[0], half[1], depth]);
        let piece = f.put(Model::Box, rows, colour);
        if colour == GILT {
            piece.material(material::COPPER);
        }
        piece.element(element::MUSEUM);
    }
    // The cords, from the top corners of the frame to one hook on the rail.
    let hook = at(0.0, rail - p[1], 0.006);
    for side in [-1.0, 1.0] {
        rod(
            f,
            at(side * 0.4 * width, hh + band, 0.02),
            hook,
            0.0025,
            WIRE,
        );
    }
    // The label, a card on the wall by the frame's lower right.
    f.put(
        Model::Box,
        place(
            at(hw + band + 0.07, -hh + 0.04, 0.005),
            yaw,
            0.0,
            0.0,
            [0.04, 0.025, 0.004],
        ),
        CARD,
    )
    .element(element::MUSEUM);
}

/// A specimen at `p` on the floor on its mount, a plinth or a glass case on
/// a pedestal, turned to `yaw`, with its label at the mount's front: `size`
/// is its longest side and its height.
fn stand(f: &mut Frame, p: [f32; 3], model: Model, size: [f32; 2], mount: Mount, yaw: f32) {
    let [long, height] = size;
    let block = |f: &mut Frame, up: f32, half: [f32; 3], colour: [f32; 3]| {
        f.put(
            Model::Box,
            place([p[0], p[1] + up, p[2]], yaw, 0.0, 0.0, half),
            colour,
        )
        .element(element::MUSEUM);
    };
    let foot = match mount {
        Mount::Platform => {
            block(f, 0.02, [0.45, 0.02, 0.32], STONE);
            block(f, 0.06, [0.38, 0.02, 0.27], [0.30, 0.22, 0.16]);
            0.08
        }
        Mount::Plinth => {
            let (tall, w) = (0.38, (0.4 * long).clamp(0.09, 0.2));
            block(f, 0.5 * tall, [w, 0.5 * tall, w], STONE);
            let cap = [w + 0.02, 0.012, w + 0.02];
            block(f, tall + 0.012, cap, [0.55, 0.52, 0.46]);
            tall + 0.024
        }
        Mount::Case => {
            let (tall, w) = (0.42, (0.5 * long + 0.03).clamp(0.1, 0.2));
            block(f, 0.5 * tall, [w, 0.5 * tall, w], DARK_WOOD);
            vitrine(f, [p[0], p[1] + tall, p[2]], yaw, w, height + 0.1);
            // A short brass stand under the specimen, so it is held up to
            // be seen, not set down.
            f.put(
                Model::Cylinder,
                place(
                    [p[0], p[1] + tall, p[2]],
                    0.0,
                    0.0,
                    0.0,
                    [0.012, 0.03, 0.012],
                ),
                BRASS,
            )
            .element(element::MUSEUM);
            tall + 0.03
        }
    };
    f.put(
        model,
        place([p[0], p[1] + foot, p[2]], yaw, 0.0, 0.0, [height; 3]),
        [1.0; 3],
    )
    .element(element::MUSEUM);
    // The label, on the side toward the hall (the model's +z).
    let (s, c) = yaw.sin_cos();
    label(f, [p[0] + s * 0.26, p[1], p[2] + c * 0.26], yaw, 0.6);
}

/// A glass case `w` square and `high` tall standing on `base`, with a thin
/// brass edge round its top.
fn vitrine(f: &mut Frame, base: [f32; 3], yaw: f32, w: f32, high: f32) {
    f.glass(
        Model::Box,
        place(
            [base[0], base[1] + 0.5 * high, base[2]],
            yaw,
            0.0,
            0.0,
            [w, 0.5 * high, w],
        ),
        [0.10, 0.13, 0.14],
    )
    .glowing(0.04)
    .element(element::MUSEUM);
    let (s, c) = yaw.sin_cos();
    let corner = |a: f32, b: f32| {
        [
            base[0] + (c * a + s * b) * w,
            base[1] + high,
            base[2] + (-s * a + c * b) * w,
        ]
    };
    let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
    for k in 0..4 {
        let (a, b) = (corners[k], corners[(k + 1) % 4]);
        rod(f, corner(a.0, a.1), corner(b.0, b.1), 0.004, BRASS);
    }
}

/// A label card on a slanted brass stand at `p` on the floor, its face
/// turned to `yaw`'s +z.
fn label(f: &mut Frame, p: [f32; 3], yaw: f32, size: f32) {
    let top = 0.14 * size;
    rod(f, p, [p[0], p[1] + top, p[2]], 0.006, BRASS);
    let (s, c) = yaw.sin_cos();
    // Tipped back about its own x, toward the reader.
    let card = place(
        [p[0] + 0.01 * s, p[1] + top + 0.01, p[2] + 0.01 * c],
        yaw,
        0.0,
        -0.6,
        [0.06 * size, 0.04 * size, 0.004],
    );
    f.put(Model::Box, card, CARD).element(element::MUSEUM);
}

/// Where the mineral hall's cases stand: in rows, in a ring round the
/// middle, or along the port wall with a row nearer the lane, as the visit
/// has it; each (u, v) with the mineral in it.
fn case_plan(m: &Museum, hall: &Hall) -> Vec<(f32, f32, usize)> {
    let mut h = Hasher::new(m.seed, 2600);
    let mut minerals: Vec<usize> = (0..MINERALS.len()).collect();
    h.shuffle(&mut minerals);
    let (near, far) = (0.75, hall.port - 0.35);
    let (u0, long) = (hall.from + 0.3, hall.long - 0.6);
    let mut slots = Vec::new();
    match (h.next() * 3.0) as u32 {
        0 => {
            let rows = (((far - near) / 0.5).floor() as usize).clamp(1, 3);
            let per = ((long / 0.5).floor() as usize).max(1);
            for r in 0..rows {
                let v = near + (r as f32 + 0.5) * (far - near) / rows as f32;
                for i in 0..per {
                    slots.push((u0 + (i as f32 + 0.5) * long / per as f32, v));
                }
            }
        }
        1 => {
            let c = (hall.from + 0.5 * hall.long, 0.5 * (near + far));
            let r = (0.5 * (far - near)).min(0.5 * long).max(0.35);
            let n = ((TAU * r / 0.42) as usize).clamp(5, 10);
            for k in 0..n {
                let a = k as f32 / n as f32 * TAU;
                slots.push((c.0 + r * a.cos(), c.1 + r * a.sin()));
            }
        }
        _ => {
            let per = ((long / 0.4).floor() as usize).max(1);
            for i in 0..per {
                let u = u0 + (i as f32 + 0.5) * long / per as f32;
                slots.push((u, far));
                if i % 2 == 0 {
                    slots.push((u, near + 0.1));
                }
            }
        }
    }
    slots
        .into_iter()
        .zip(minerals.into_iter().cycle())
        .map(|((u, v), k)| (u, v, k))
        .collect()
}

/// The mineral hall's cases on their pedestals, each with the lamp housing
/// over it and the stand inside it the mineral sits on (static); the
/// minerals, whose look turns with the lamps, are [`museum_lights`]'.
fn mineral_cases(w: &World, f: &mut Frame, m: &Museum, hall: &Hall) {
    let y = m.floor;
    let (side, tall, high) = (0.13, 0.40, 0.34);
    for (u, v, _) in case_plan(m, hall) {
        let p = m.at(u, v, y);
        if !w.visible([p[0], y + 0.5, p[2]], 0.6) {
            continue;
        }
        let up = |h: f32| [p[0], y + h, p[2]];
        let pedestal = [side, 0.5 * tall, side];
        f.put(
            Model::Box,
            place(up(0.5 * tall), m.yaw(), 0.0, 0.0, pedestal),
            [0.12, 0.11, 0.11],
        )
        .element(element::MUSEUM);
        let cap = [side + 0.015, 0.012, side + 0.015];
        f.put(Model::Box, place(up(tall), m.yaw(), 0.0, 0.0, cap), BRASS)
            .material(material::COPPER)
            .element(element::MUSEUM);
        vitrine(f, up(tall + 0.012), m.yaw(), side, high);
        let lamp = [side, 0.02, side];
        f.put(
            Model::Box,
            place(up(tall + high + 0.03), m.yaw(), 0.0, 0.0, lamp),
            [0.10; 3],
        )
        .element(element::MUSEUM);
        let stand = [0.035, 0.03, 0.035];
        f.put(
            Model::Cylinder,
            place(up(tall + 0.012), 0.0, 0.0, 0.0, stand),
            [0.10; 3],
        )
        .element(element::MUSEUM);
    }
}

/// The picture gallery: the largest of the masters in the middle of the
/// port wall, the rest hung salon-fashion three deep either side and on the
/// end walls, a picture rail along the top, and benches before them.
fn masters(w: &World, f: &mut Frame, m: &Museum, hall: &Hall) {
    let mut h = Hasher::new(m.seed, 2700);
    let mut paintings: Vec<usize> = (0..PAINTINGS.len())
        .filter(|&i| PAINTINGS[i].masters)
        .collect();
    h.shuffle(&mut paintings);
    let area = |i: usize| PAINTINGS[i].height * PAINTINGS[i].width;
    let largest = paintings
        .iter()
        .copied()
        .max_by(|&a, &b| area(a).total_cmp(&area(b)))
        .expect("the masters");
    paintings.retain(|&i| i != largest);
    let rail = m.floor + hall.high - 0.12;
    let wall_v = hall.port - 0.07;
    let middle = hall.from + 0.5 * hall.long;
    let (bw, bh) = size_of(&PAINTINGS[largest], CENTREPIECE_SCALE, hall.high - 0.6);
    let p = m.at(middle, wall_v, m.floor + 0.35 + 0.5 * bh);
    if w.visible(p, bw) {
        painting(f, p, m.direction(0.0, -1.0), largest, [bw, bh], rail);
    }
    let side = 0.5 * (hall.long - bw - 0.45);
    let mut walls = Vec::new();
    for u in [hall.from + 0.5 * side + 0.08, hall.to() - 0.5 * side - 0.08] {
        walls.push(Wall {
            u,
            v: wall_v,
            along: [1.0, 0.0],
            face: [0.0, -1.0],
            width: side,
            low: 0.22,
            high: hall.high - 0.25,
        });
    }
    for (u, face) in [(hall.from + 0.09, 1.0f32), (hall.to() - 0.09, -1.0)] {
        walls.push(Wall {
            u,
            v: 0.5 * (DOOR + hall.port),
            along: [0.0, 1.0],
            face: [face, 0.0],
            width: 0.85 * (hall.port - DOOR),
            low: 0.3,
            high: hall.high - 0.25,
        });
    }
    let how = Hang {
        scale: MASTERS_SCALE,
        rows: 3,
        gap: 0.07,
    };
    hang(w, f, m, &walls, &paintings, how);
    if w.visible(m.at(middle, wall_v, rail), 0.5 * hall.long) {
        let a = m.at(hall.from + 0.05, wall_v - 0.005, rail);
        let b = m.at(hall.to() - 0.05, wall_v - 0.005, rail);
        rod(f, a, b, 0.012, BRASS);
    }
    // Upholstered benches down the middle, clear of the boat's lane.
    for k in 0..(hall.long / 1.2) as usize {
        let u = hall.from + 0.6 + k as f32 * 1.2;
        let p = m.at(u, 0.5 * hall.port + 0.15, m.floor);
        if !w.visible(p, 0.4) {
            continue;
        }
        let seat = place(
            [p[0], p[1] + 0.1, p[2]],
            m.yaw(),
            0.0,
            0.0,
            [0.28, 0.025, 0.09],
        );
        f.put(Model::Box, seat, [0.35, 0.10, 0.12])
            .element(element::MUSEUM);
        let base = place(
            [p[0], p[1] + 0.04, p[2]],
            m.yaw(),
            0.0,
            0.0,
            [0.26, 0.04, 0.07],
        );
        f.put(Model::Box, base, DARK_WOOD).element(element::MUSEUM);
    }
}

/// What in the museum moves with the clock: daylight down through the
/// skylights, the lamps, and the minerals under lamps that turn from white
/// light to ultraviolet and back.
pub fn museum_lights(w: &World, f: &mut Frame) {
    let Some(m) = Museum::near(&w.seabed.order, w.seabed.seed, w.camera()) else {
        return;
    };
    if !w.visible(m.at(0.5 * m.long, 1.5, m.floor + 1.0), 0.6 * m.long + 3.0) {
        return;
    }
    let y = m.floor;
    for hall in &m.halls {
        let bay = hall.bay();
        for i in 0..hall.bays {
            let u = hall.from + (i as f32 + 0.5) * bay;
            // A shaft of daylight down through the skylight, breathing as
            // the surface moves far overhead.
            let top = m.at(u, hall.middle(), y + hall.vault(hall.middle()));
            let foot = m.at(u + 0.3, hall.middle() + 0.2, y);
            if w.visible(m.at(u, hall.middle(), y + 0.5 * hall.high), hall.high) {
                let k = 0.35 * (0.7 + 0.3 * (w.t * 0.2 + i as f32 * 1.7).sin());
                f.shafts.push(Sprite {
                    place: [top[0], top[1], top[2], 0.16],
                    tint: [0.55 * k, 0.75 * k, 0.70 * k, 0.0],
                    stretch: [foot[0] - top[0], foot[1] - top[1], foot[2] - top[2], 0.0],
                });
            }
            // Lamps hung on chains, warm; in the natural history hall over
            // the aisles only, the island having its own light.
            for v in [0.2, hall.port - 0.45] {
                if hall.kind == HallKind::Natural && v < 1.0 {
                    continue;
                }
                let p = m.at(u, v, y + hall.high - 0.25);
                if !w.visible(p, 0.6) {
                    continue;
                }
                let lit = if hall.kind == HallKind::Minerals {
                    0.5
                } else {
                    1.0
                };
                f.put(
                    Model::Sphere,
                    place(p, 0.0, 0.0, 0.0, [0.05; 3]),
                    [1.0, 0.85, 0.6],
                )
                .glowing(1.2 * lit);
                let chain = m.at(u, v, y + hall.vault(v) - 0.02);
                f.put(Model::Cylinder, between(p, chain, 0.004), WIRE);
                let glow = [0.40 * lit, 0.30 * lit, 0.16 * lit];
                f.glow_sprite(p, 0.30, glow, i as f32, 1.0, 1.0);
            }
        }
        let light = match hall.kind {
            HallKind::Natural => m.at(m.loop_at, m.radius, y + 1.6),
            _ => m.at(
                hall.from + 0.5 * hall.long,
                0.6 * hall.port,
                y + hall.high - 0.2,
            ),
        };
        f.light(light, 3.5, [1.0, 0.86, 0.66]);
        if hall.kind != HallKind::Minerals {
            continue;
        }
        // The lamps over the cases run through white light, a dimming, the
        // ultraviolet and back, each case a little behind the last, so a
        // wave runs down the hall; under the ultraviolet the fluorescent
        // minerals flare in their own colours and the rest go dark.
        for (k, &(u, v, mineral)) in case_plan(&m, hall).iter().enumerate() {
            let p = m.at(u, v, y);
            if !w.visible([p[0], y + 0.6, p[2]], 0.5) {
                continue;
            }
            let (model, tint, glassy, fluoresces) = MINERALS[mineral];
            let uv = smooth(0.2, 0.6, (w.t * TAU / 24.0 - k as f32 * 0.35).sin());
            let white = 1.0 - uv;
            let base = [p[0], y + 0.40 + 0.012 + 0.03, p[2]];
            let size = if model == Model::Geode { 0.2 } else { 0.16 };
            let yaw = m.yaw() + FRAC_PI_2 + 0.6 * (k as f32 * 1.7).sin();
            let glow = fluoresces.map_or(0.0, |_| uv);
            let colour = match fluoresces {
                Some(c) => mix3(tint, c, uv),
                None => tint.map(|t| t * (0.25 + 0.75 * white)),
            };
            let rows = place(base, yaw, 0.0, 0.0, [size; 3]);
            if glassy {
                f.glass(model, rows, colour)
                    .material(material::CRYSTAL)
                    .glowing(0.3 + 1.2 * glow);
            } else {
                f.put(model, rows, colour).glowing(1.5 * glow);
            }
            // The lamp's own light in the case, white or violet, a glow from
            // what fluoresces, and a sparkle on the crystals under the white.
            let lamp = mix3([0.9, 0.85, 0.75], [0.35, 0.10, 0.85], uv);
            let over = [p[0], y + 0.70, p[2]];
            f.glow_sprite(over, 0.22, lamp.map(|c| c * 0.35), k as f32, 1.0, 1.0);
            let inside = [base[0], base[1] + 0.08, base[2]];
            if let Some(c) = fluoresces {
                f.glow_sprite(inside, 0.24, c.map(|c| c * 0.5 * uv), k as f32, 1.0, 1.0);
            }
            if glassy && white > 0.5 {
                let tip = [base[0], base[1] + 0.12, base[2]];
                let spark = [0.9 * white, 0.9 * white, white];
                f.glow_sprite(tip, 0.05, spark, 2.3 * k as f32, 3.0, 0.0);
            }
            if k % 3 == 0 {
                f.light([base[0], base[1] + 0.2, base[2]], 0.9, lamp);
            }
        }
    }
}

/// A seeded stream of numbers in 0..1, for the layout's choices.
struct Hasher {
    seed: u32,
    salt: i32,
    n: i32,
}

impl Hasher {
    fn new(seed: u32, salt: i32) -> Self {
        Self { seed, salt, n: 0 }
    }

    fn next(&mut self) -> f32 {
        self.n += 1;
        hash(self.n, self.salt, self.seed)
    }

    fn shuffle<T>(&mut self, items: &mut [T]) {
        for i in (1..items.len()).rev() {
            let j = ((self.next() * (i + 1) as f32) as usize).min(i);
            items.swap(i, j);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No painting hangs larger than its texture can fill sharply: across
    /// its width and its height, at the largest it can be hung, the layer it
    /// is drawn from and the image that layer was made from both have at
    /// least [`SHARP`] texels a unit (a unit is a little over half the
    /// window's width at the distance the boat passes the walls).
    #[test]
    fn every_painting_hangs_sharp() {
        use crate::dive3d::paintings::SIZE;
        const SHARP: f32 = 400.0;
        let area = |p: &Painting| p.height * p.width;
        let largest = PAINTINGS
            .iter()
            .filter(|p| p.masters)
            .max_by(|a, b| area(a).total_cmp(&area(b)))
            .unwrap();
        for p in &PAINTINGS {
            let scale = if std::ptr::eq(p, largest) {
                CENTREPIECE_SCALE
            } else if p.masters {
                MASTERS_SCALE
            } else {
                NATURAL_SCALE
            };
            let (w, h) = size_of(p, scale, f32::MAX);
            for (side, texels, pixels) in [("wide", w, p.pixels[0]), ("tall", h, p.pixels[1])] {
                let layer = SIZE as f32 / texels;
                let source = pixels as f32 / texels;
                assert!(
                    layer >= SHARP && source >= SHARP,
                    "{} by {} hangs {texels:.2} {side}: {layer:.0} texels a unit in its \
                     layer, {source:.0} in its image",
                    p.title,
                    p.painter,
                );
            }
        }
    }

    /// Every visit's museum differs, and every museum's tour goes in by the
    /// front door, once round the loop and out by the back, smoothly.
    #[test]
    fn the_tour_loops_through_every_layout() {
        let leg = crate::voyage::LEG as f32;
        let mut layouts = Vec::new();
        for visit in 0..12u64 {
            let seed = crate::voyage::seed_of(visit);
            let order = crate::voyage::order_for(seed);
            let k = order.iter().position(|&b| b == MUSEUM).unwrap() as f32;
            let m = Museum::near(&order, seed, (k + 0.5) * leg).expect("a museum");
            layouts.push(format!("{:.3} {:?}", m.radius, m.halls.map(|h| h.kind)));
            let sum: f32 = m.halls.iter().map(|h| h.long).sum();
            assert!((m.long - sum).abs() < 1e-3);
            let mut last: Option<Tour> = None;
            let mut turned = 0.0;
            let mut x = k * leg;
            while x < (k + 1.0) * leg {
                match (m.tour(x), last) {
                    (Some(t), Some(l)) => {
                        let step = (t.x - l.x).hypot(t.z - l.z);
                        assert!(step < 0.05, "visit {visit}: a jump of {step} at {x}");
                        let mut d = t.heading - l.heading;
                        d -= TAU * (d / TAU).round();
                        assert!(d.abs() < 0.05, "visit {visit}: the heading snaps at {x}");
                        turned += d;
                        last = Some(t);
                    }
                    (t, _) => last = t,
                }
                x += 0.01;
            }
            // Once round to port, which is a falling yaw (`helm`).
            assert!(
                turned < -0.95 * TAU,
                "visit {visit} turns {turned} all told"
            );
        }
        layouts.sort();
        layouts.dedup();
        assert!(layouts.len() > 6, "the layouts hardly differ: {layouts:?}");
    }
}
