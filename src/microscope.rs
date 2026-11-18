//! The microscope: a naturalist's slides under the objective.
//!
//! Most slides hold a drop of water and what lives in it: paramecia swimming on
//! curving paths with their cilia beating and vacuoles pulsing, Euglena
//! squirming behind their flagella, diatoms gliding back and forth, volvox
//! colonies rolling with daughter colonies inside, amoebae creeping and
//! reaching, Vorticella bells snapping back on their coiling stalks, crescents
//! of Closterium, filaments of Spirogyra wound with their chloroplasts, stars
//! of Pediastrum, a tardigrade lumbering through, bacteria darting about and
//! debris jiggling in Brownian motion. Others are stained tissue: onion skin
//! and a blood smear.
//!
//! The fine focus drifts slowly up and down through the drop, so one organism
//! sharpens as another blurs. Slides are lit in bright field, in dark field
//! (where only edges shine, and in Rheinberg lighting shine gold on blue), or
//! in phase contrast. Partway through a slide the nosepiece turns to a
//! stronger objective and the view comes back into focus closer in; the stage
//! drifts slowly across the slide. A slide changes every minute and a half, or
//! with a swipe: the view blurs and slides away, and the next comes into focus.
//! Each visit's slides hold a different number of each organism.
//!
//! Everything moves as a pure function of the clock and the visit's seed, so a
//! moment of a visit can be drawn again exactly. The view is one fragment
//! shader (`shaders/lens/microscope.wgsl`) over one uniform block; see
//! [`crate::lens`].

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::lens::{Dice, Lens, smooth, tour_at, wrap};
use crate::solar::Hud;
use crate::visuals::Visual;

pub use crate::lens::SIZE;

/// The shader, after the shared prelude.
pub const SHADER: &str = concat!(
    include_str!("shaders/lens/prelude.wgsl"),
    include_str!("shaders/lens/microscope.wgsl")
);

/// Seconds a slide stays under the objective.
const HOLD: f64 = 85.0;
/// Seconds to change slides.
const CHANGE: f64 = 5.0;
/// Seconds into the hold that the stronger objective swings in, and how long
/// the view takes to come back into focus.
const NOSEPIECE: f32 = 45.0;
const REFOCUS: f32 = 1.6;
/// The field stop's radius in pixels.
const FIELD: f32 = 520.0;
/// The organisms the shader has room for.
const CELLS: usize = 16;
/// Organisms live on a torus this wide, centred on the view, so one that swims
/// out of view comes back in on the other side, outside the stop. No wider
/// than it must be for the longest (`Kind::reach`), or most of a slide would
/// be out of view.
const TORUS: f32 = 3.0;
/// Seconds for the focus to drift down through the drop and back.
const FOCUS_PERIOD: f32 = 56.0;

/// The uniform block (`Params` in the shader).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Params {
    frame: [f32; 4],
    look: [f32; 4],
    stage: [f32; 4],
    light: [f32; 4],
    glow: [f32; 4],
    slide: [f32; 4],
    cells: [[f32; 4]; 2 * CELLS],
}

/// What lives on the slides, in the shader's `kind` order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Paramecium,
    Pennate,
    Centric,
    Volvox,
    Amoeba,
    Closterium,
    Euglena,
    Vorticella,
    Spirogyra,
    Pediastrum,
    Tardigrade,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Paramecium => "Paramecium",
            Kind::Pennate => "Navicula",
            Kind::Centric => "Cyclotella",
            Kind::Volvox => "Volvox",
            Kind::Amoeba => "Amoeba",
            Kind::Closterium => "Closterium",
            Kind::Euglena => "Euglena",
            Kind::Vorticella => "Vorticella",
            Kind::Spirogyra => "Spirogyra",
            Kind::Pediastrum => "Pediastrum",
            Kind::Tardigrade => "a tardigrade",
        }
    }

    /// Half-length in slide units, smallest and largest.
    fn size(self) -> (f32, f32) {
        match self {
            Kind::Paramecium => (0.15, 0.21),
            Kind::Pennate => (0.1, 0.22),
            Kind::Centric => (0.06, 0.11),
            Kind::Volvox => (0.12, 0.2),
            Kind::Amoeba => (0.13, 0.19),
            Kind::Closterium => (0.17, 0.22),
            Kind::Euglena => (0.08, 0.12),
            Kind::Vorticella => (0.09, 0.13),
            Kind::Spirogyra => (0.16, 0.23),
            Kind::Pediastrum => (0.07, 0.12),
            Kind::Tardigrade => (0.18, 0.24),
        }
    }

    /// How far it reaches from its centre, in its sizes: an amoeba's
    /// pseudopods, a flagellum, a stalk or a filament reach past 1. The
    /// torus must leave room for it (`organisms_stay_on_the_torus`).
    #[cfg(test)]
    fn reach(self) -> f32 {
        match self {
            Kind::Amoeba => 1.3,
            Kind::Euglena | Kind::Vorticella => 1.95,
            Kind::Spirogyra => 2.05,
            Kind::Pediastrum => 1.1,
            _ => 1.05,
        }
    }
}

/// How a slide is lit.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Light {
    /// The lamp straight through: pigment absorbs it.
    Bright([f32; 3]),
    /// Only light scattered by edges, on a ground of this colour, in this
    /// colour: black and white for dark field, blue and gold for Rheinberg.
    Dark([f32; 3], [f32; 3]),
    /// Phase contrast: what slows the light shows dark, haloed bright.
    Phase([f32; 3]),
}

impl Light {
    fn word(self) -> &'static str {
        match self {
            Light::Bright(_) => "Bright field",
            Light::Dark(ground, _) if ground.iter().any(|&c| c > 0.006) => "Rheinberg",
            Light::Dark(..) => "Dark field",
            Light::Phase(_) => "Phase contrast",
        }
    }
}

/// What fills the field behind the organisms.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Tissue {
    None,
    Onion,
    Blood,
}

/// One slide.
struct Slide {
    name: &'static str,
    light: Light,
    tissue: Tissue,
    /// The objectives, before and after the nosepiece turns, with their
    /// magnification over the first.
    objectives: [(&'static str, f32); 2],
    /// What is on it: each kind and the most of it there may be.
    life: &'static [(Kind, usize)],
    /// Bacteria (share of cells) and debris.
    bacteria: f32,
    debris: f32,
}

const POND: [f32; 3] = [0.95, 0.9, 0.78];
const BLACK: [f32; 3] = [0.0025, 0.0025, 0.003];
const WHITE: [f32; 3] = [0.95, 0.97, 1.0];

const SLIDES: [Slide; 14] = [
    Slide {
        name: "Pond water",
        light: Light::Bright(POND),
        tissue: Tissue::None,
        objectives: [("10× · NA 0.25", 1.0), ("20× · NA 0.40", 1.8)],
        life: &[
            (Kind::Paramecium, 3),
            (Kind::Amoeba, 2),
            (Kind::Closterium, 2),
            (Kind::Pennate, 4),
            (Kind::Centric, 4),
        ],
        bacteria: 0.0,
        debris: 0.28,
    },
    Slide {
        name: "Diatoms",
        light: Light::Dark(BLACK, WHITE),
        tissue: Tissue::None,
        objectives: [("20× · NA 0.40", 1.0), ("40× · NA 0.65", 1.9)],
        life: &[(Kind::Pennate, 10), (Kind::Centric, 6)],
        bacteria: 0.0,
        debris: 0.28,
    },
    Slide {
        name: "Diatoms, Rheinberg",
        light: Light::Dark([0.003, 0.008, 0.04], [1.0, 0.8, 0.35]),
        tissue: Tissue::None,
        objectives: [("20× · NA 0.40", 1.0), ("40× · NA 0.65", 1.9)],
        life: &[(Kind::Pennate, 9), (Kind::Centric, 7)],
        bacteria: 0.0,
        debris: 0.2,
    },
    Slide {
        name: "Volvox",
        light: Light::Bright([0.9, 0.92, 0.82]),
        tissue: Tissue::None,
        objectives: [("4× · NA 0.10", 1.0), ("10× · NA 0.25", 2.0)],
        life: &[(Kind::Volvox, 8), (Kind::Pennate, 4)],
        bacteria: 0.0,
        debris: 0.25,
    },
    Slide {
        name: "Desmids",
        light: Light::Bright([0.93, 0.92, 0.84]),
        tissue: Tissue::None,
        objectives: [("10× · NA 0.25", 1.0), ("20× · NA 0.40", 1.8)],
        life: &[
            (Kind::Closterium, 9),
            (Kind::Pediastrum, 3),
            (Kind::Pennate, 3),
        ],
        bacteria: 0.0,
        debris: 0.25,
    },
    Slide {
        name: "Hay infusion",
        light: Light::Phase([0.6, 0.63, 0.6]),
        tissue: Tissue::None,
        objectives: [("10× · Ph1", 1.0), ("20× · Ph1", 1.8)],
        life: &[(Kind::Paramecium, 10), (Kind::Amoeba, 3)],
        bacteria: 0.3,
        debris: 0.2,
    },
    Slide {
        name: "Amoebae",
        light: Light::Bright([0.94, 0.88, 0.8]),
        tissue: Tissue::None,
        objectives: [("20× · NA 0.40", 1.0), ("40× · NA 0.65", 1.8)],
        life: &[(Kind::Amoeba, 5), (Kind::Centric, 6), (Kind::Paramecium, 3)],
        bacteria: 0.15,
        debris: 0.3,
    },
    Slide {
        name: "Euglena bloom",
        light: Light::Bright([0.92, 0.93, 0.85]),
        tissue: Tissue::None,
        objectives: [("20× · NA 0.40", 1.0), ("40× · NA 0.65", 1.9)],
        life: &[(Kind::Euglena, 14), (Kind::Pennate, 2)],
        bacteria: 0.0,
        debris: 0.2,
    },
    Slide {
        name: "Vorticella on a twig",
        light: Light::Phase([0.58, 0.6, 0.6]),
        tissue: Tissue::None,
        objectives: [("10× · Ph1", 1.0), ("20× · Ph1", 1.8)],
        life: &[
            (Kind::Vorticella, 9),
            (Kind::Paramecium, 2),
            (Kind::Euglena, 3),
        ],
        bacteria: 0.35,
        debris: 0.3,
    },
    Slide {
        name: "Spirogyra",
        light: Light::Bright([0.92, 0.94, 0.86]),
        tissue: Tissue::None,
        objectives: [("10× · NA 0.25", 1.0), ("20× · NA 0.40", 2.0)],
        life: &[(Kind::Spirogyra, 7), (Kind::Euglena, 3), (Kind::Pennate, 3)],
        bacteria: 0.0,
        debris: 0.22,
    },
    Slide {
        name: "Moss water",
        light: Light::Dark(BLACK, [0.92, 0.95, 1.0]),
        tissue: Tissue::None,
        objectives: [("10× · NA 0.25", 1.0), ("20× · NA 0.40", 1.7)],
        life: &[(Kind::Tardigrade, 2), (Kind::Pennate, 5), (Kind::Amoeba, 2)],
        bacteria: 0.25,
        debris: 0.35,
    },
    Slide {
        name: "Pediastrum",
        light: Light::Dark([0.003, 0.016, 0.008], [1.0, 0.95, 0.75]),
        tissue: Tissue::None,
        objectives: [("20× · NA 0.40", 1.0), ("40× · NA 0.65", 1.9)],
        life: &[
            (Kind::Pediastrum, 8),
            (Kind::Closterium, 3),
            (Kind::Centric, 4),
        ],
        bacteria: 0.0,
        debris: 0.2,
    },
    Slide {
        name: "Onion skin, iodine",
        light: Light::Bright([0.96, 0.94, 0.88]),
        tissue: Tissue::Onion,
        objectives: [("10× · NA 0.25", 1.0), ("40× · NA 0.65", 3.0)],
        life: &[],
        bacteria: 0.0,
        debris: 0.08,
    },
    Slide {
        name: "Blood smear, Giemsa",
        light: Light::Bright([0.97, 0.95, 0.93]),
        tissue: Tissue::Blood,
        objectives: [("40× · NA 0.65", 1.0), ("100× · oil", 2.4)],
        life: &[],
        bacteria: 0.0,
        debris: 0.05,
    },
];

/// One organism on a slide: where it starts and how it goes.
#[derive(Clone, Copy, Debug)]
struct Cell {
    kind: Kind,
    at: [f32; 2],
    heading: f32,
    size: f32,
    depth: f32,
    phase: f32,
    hue: f32,
    /// Speed along its heading, slide units a second.
    speed: f32,
    /// Turn, radians a second.
    turn: f32,
}

impl Cell {
    fn new(kind: Kind, dice: &mut Dice) -> Self {
        let (small, large) = kind.size();
        let (speed, turn) = match kind {
            Kind::Paramecium => (dice.range(0.035, 0.06), dice.range(-0.04, 0.04)),
            Kind::Euglena => (dice.range(0.02, 0.035), dice.range(-0.06, 0.06)),
            Kind::Pennate | Kind::Centric | Kind::Vorticella => (0.0, 0.0),
            Kind::Volvox => (dice.range(0.008, 0.016), 0.0),
            Kind::Amoeba => (dice.range(0.003, 0.006), dice.range(-0.01, 0.01)),
            Kind::Closterium | Kind::Spirogyra | Kind::Pediastrum => {
                (0.002, dice.range(-0.006, 0.006))
            }
            Kind::Tardigrade => (dice.range(0.008, 0.012), dice.range(-0.02, 0.02)),
        };
        Self {
            kind,
            at: [dice.range(-0.5, 0.5) * TORUS, dice.range(-0.5, 0.5) * TORUS],
            heading: dice.range(0.0, std::f32::consts::TAU),
            size: dice.range(small, large),
            // Tardigrades crawl on the slide; Vorticella hold to debris on it.
            depth: match kind {
                Kind::Tardigrade | Kind::Vorticella => dice.range(-1.0, -0.4),
                _ => dice.range(-1.0, 1.0),
            },
            phase: dice.next(),
            hue: dice.next(),
            speed,
            turn,
        }
    }

    /// Where it is, which way it points, and its `hue` slot (a Vorticella's
    /// stalk length) `t` seconds in, wrapped round the view at `view`.
    fn pose(&self, t: f32, view: [f32; 2]) -> ([f32; 2], f32, f32) {
        let wobble =
            |rate: f32, by: f32| by * (t * rate + self.phase * std::f32::consts::TAU).sin();
        let (mut x, mut y, angle);
        let mut hue = self.hue;
        match self.kind {
            Kind::Pennate => {
                // Gliding back and forth along its raphe.
                let glide = wobble(0.11, 0.07);
                x = self.at[0] + self.heading.cos() * glide;
                y = self.at[1] + self.heading.sin() * glide;
                angle = self.heading;
            }
            Kind::Centric => {
                x = self.at[0] + wobble(0.05, 0.02);
                y = self.at[1] + wobble(0.04, 0.02);
                angle = self.heading + t * 0.01;
            }
            Kind::Vorticella => {
                // Every so often the stalk snaps short in a fraction of a
                // second, then stretches out again over several.
                let period = 9.0 + 6.0 * self.phase;
                let into = (t + self.phase * period).rem_euclid(period);
                let stretch = if into < 0.25 {
                    1.0 - into / 0.25
                } else {
                    smooth(0.25, 6.0, into)
                };
                hue = stretch;
                // Anchored at the stalk's foot; the bell moves with it.
                let length = (0.3 + 1.6 * stretch) * self.size;
                let swing = self.heading + wobble(0.3, 0.12);
                x = self.at[0] + swing.cos() * length;
                y = self.at[1] + swing.sin() * length;
                angle = swing;
            }
            _ => {
                // Along a circle of radius speed / turn (or a straight line),
                // swaying a little either side of it.
                let heading = self.heading + self.turn * t;
                let (along_x, along_y) = if self.turn.abs() > 1e-4 {
                    let r = self.speed / self.turn;
                    (
                        r * (heading.sin() - self.heading.sin()),
                        -r * (heading.cos() - self.heading.cos()),
                    )
                } else {
                    (
                        self.speed * t * heading.cos(),
                        self.speed * t * heading.sin(),
                    )
                };
                let sway = wobble(0.5, 0.012);
                x = self.at[0] + along_x - heading.sin() * sway;
                y = self.at[1] + along_y + heading.cos() * sway;
                angle = match self.kind {
                    Kind::Paramecium => heading + wobble(0.5, 0.15),
                    Kind::Euglena => heading + wobble(1.1, 0.2),
                    Kind::Tardigrade => heading + wobble(0.4, 0.08),
                    _ => heading,
                };
            }
        }
        let half = TORUS * 0.5;
        x = view[0] + (x - view[0] + half).rem_euclid(TORUS) - half;
        y = view[1] + (y - view[1] + half).rem_euclid(TORUS) - half;
        ([x, y], angle, hue)
    }
}

/// The organisms on one slide of one visit, nearest the coverslip last: each
/// kind between half its most and its most.
fn populate(slide: &Slide, dice: &mut Dice) -> Vec<Cell> {
    let mut kinds = Vec::new();
    for &(kind, most) in slide.life {
        let fewest = most.div_ceil(2);
        let count = fewest + ((most - fewest + 1) as f32 * dice.next()) as usize;
        kinds.extend(std::iter::repeat_n(kind, count.min(most)));
    }
    let mut cells: Vec<Cell> = kinds
        .into_iter()
        .take(CELLS)
        .map(|kind| Cell::new(kind, dice))
        .collect();
    cells.sort_by(|a, b| a.depth.total_cmp(&b.depth));
    cells
}

/// The scene.
pub struct Microscope {
    lens: Lens,
    seed: Option<u64>,
    order: Vec<usize>,
    origin: i64,
    offset: i64,
    since: f64,
    pending: i32,
    /// The slide on the stage, with its organisms.
    stop: Option<i64>,
    cells: Vec<Cell>,
    shown: usize,
    settled: bool,
    /// Which objective is in: 0 the first, 1 the stronger.
    objective: usize,
}

impl Microscope {
    pub fn new(gpu: &Graphics) -> Result<Self> {
        let lens = Lens::new(
            gpu,
            "Microscope",
            SHADER,
            std::mem::size_of::<Params>() as u64,
        );
        Ok(Self {
            lens,
            seed: None,
            order: Vec::new(),
            origin: 0,
            offset: 0,
            since: 0.0,
            pending: 0,
            stop: None,
            cells: Vec::new(),
            shown: 0,
            settled: false,
            objective: 0,
        })
    }

    fn params(&mut self, seconds: f64, seed: u64) -> Params {
        if self.seed != Some(seed) {
            self.seed = Some(seed);
            self.order = Dice::new(seed).order(SLIDES.len());
            self.origin = 0;
            self.offset = 0;
            // The first slide comes up into focus.
            self.since = seconds - CHANGE * 0.5;
            self.pending = 0;
            self.stop = None;
        }
        if self.pending != 0 {
            let (stop, arrived) = tour_at(seconds - self.since, self.offset, HOLD, CHANGE);
            let from = if arrived < 0.5 {
                self.previous(stop)
            } else {
                stop
            };
            self.origin = from;
            self.offset = from + self.pending as i64;
            self.since = seconds;
            self.pending = 0;
        }
        let (stop, arrived) = tour_at(seconds - self.since, self.offset, HOLD, CHANGE);
        let stop = if arrived < 0.5 {
            self.previous(stop)
        } else {
            stop
        };
        // Seconds into this slide's hold (negative while it slides in).
        let leg = HOLD + CHANGE;
        let into = (seconds - self.since).max(0.0) % leg;
        let held = if arrived < 0.5 { into + leg } else { into } as f32 - CHANGE as f32;

        let index = self.order[wrap(stop, SLIDES.len())];
        let slide = &SLIDES[index];
        let mut dice = Dice::new(seed ^ (stop as u64).wrapping_mul(0x2545_f491_4f6c_dd1d));
        let debris = dice.range(0.0, 100.0);
        let heading = dice.range(0.0, std::f32::consts::TAU);
        // The stage drifts slowly across the slide, as a hand follows
        // something on it.
        let pan_way = dice.range(0.0, std::f32::consts::TAU);
        let pan_speed = dice.range(0.0, 0.006);
        if self.stop != Some(stop) {
            self.stop = Some(stop);
            self.cells = populate(slide, &mut dice);
        }
        self.shown = index;
        self.settled = arrived >= 1.0;

        // A change of slide: the view blurs and the stage slides away, then
        // the next slide slides in and comes into focus.
        let a = arrived;
        let (fade, travel, mut blur) = if a < 0.5 {
            let x = smooth(0.0, 0.5, a);
            (1.0 - 0.7 * x, x * 1.2, x * 0.05)
        } else {
            let x = smooth(0.5, 1.0, a);
            (0.3 + 0.7 * x, -(1.0 - x) * 1.2, (1.0 - x) * 0.05)
        };
        // The nosepiece: the stronger objective swings in, out of focus, and
        // the view sharpens closer in.
        let turned = held - NOSEPIECE;
        self.objective = usize::from(turned >= 0.0);
        let zoom = slide.objectives[self.objective].1;
        if (0.0..REFOCUS).contains(&turned) {
            let x = 1.0 - turned / REFOCUS;
            blur += x * x * 0.03;
        }

        let t = seconds as f32;
        let drift = [0.006 * (t * 0.05).sin(), 0.006 * (t * 0.037).cos()];
        let pan = pan_speed * held.max(0.0);
        let mut view = [
            drift[0] + heading.cos() * travel + pan_way.cos() * pan,
            drift[1] + heading.sin() * travel + pan_way.sin() * pan,
        ];
        // Before the stronger objective goes in, the hand on the stage brings
        // the nearest organism to the middle, and keeps it there.
        if turned >= 0.0 {
            let then = t - turned;
            let base = view;
            let nearest = self
                .cells
                .iter()
                .map(|cell| {
                    let (at, _, _) = cell.pose(then, base);
                    let gap = (at[0] - base[0]).hypot(at[1] - base[1]);
                    (cell, gap)
                })
                .filter(|(_, gap)| *gap < 1.0)
                .min_by(|a, b| a.1.total_cmp(&b.1));
            if let Some((cell, _)) = nearest {
                let (at, _, _) = cell.pose(t, base);
                let follow = smooth(0.0, 3.0, turned);
                view = [
                    base[0] + (at[0] - base[0]) * follow,
                    base[1] + (at[1] - base[1]) * follow,
                ];
            }
        }
        let mut cells = [[0.0f32; 4]; 2 * CELLS];
        for (i, cell) in self.cells.iter().enumerate() {
            let (at, angle, hue) = cell.pose(t, view);
            cells[2 * i] = [at[0], at[1], angle, cell.size];
            cells[2 * i + 1] = [cell.kind as u8 as f32, cell.depth, cell.phase, hue];
        }
        // The focus drifts through the drop; on tissue, which lies flat,
        // hardly at all.
        let reach = if slide.tissue == Tissue::None {
            0.85
        } else {
            0.25
        };
        let focus = reach * (t * std::f32::consts::TAU / FOCUS_PERIOD).sin();
        let (mode, light, glow) = match slide.light {
            Light::Bright(c) => (0.0, c, c),
            Light::Dark(ground, edge) => (1.0, ground, edge),
            Light::Phase(c) => (2.0, c, c),
        };
        let tissue = match slide.tissue {
            Tissue::None => 0.0,
            Tissue::Onion => 1.0,
            Tissue::Blood => 2.0,
        };
        Params {
            frame: [SIZE as f32 * 0.5, SIZE as f32 * 0.5, FIELD, t],
            look: [mode, focus, fade, debris],
            stage: [view[0], view[1], blur, 1.0 / FIELD],
            light: [light[0], light[1], light[2], 1.0],
            glow: [glow[0], glow[1], glow[2], zoom],
            slide: [tissue, slide.bacteria, slide.debris, 1.0],
            cells,
        }
    }

    /// A frame of the slide named `name`, `held` seconds in, for the
    /// picker's thumbnails.
    #[cfg(test)]
    /// Of the first `seeds` seeds, the one with the most organisms in the
    /// middle of the field is drawn.
    pub(crate) fn render_slide(name: &str, held: f64, seeds: u64) -> Vec<u8> {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Microscope::new(&gpu).expect("a microscope");
        let wanted = SLIDES.iter().position(|s| s.name == name).expect("a slide");
        let seconds = held + CHANGE * 0.5;
        let prepare = |scene: &mut Microscope, seed: u64| {
            scene.seed = None;
            scene.params(0.0, seed);
            let at = scene.order.iter().position(|&i| i == wanted).unwrap();
            scene.order.swap(0, at);
            scene.stop = None;
            scene.params(seconds, seed)
        };
        let best = (0..seeds.max(1))
            .max_by_key(|&seed| {
                let params = prepare(&mut scene, seed);
                let view = [params.stage[0], params.stage[1]];
                (0..CELLS)
                    .filter(|&i| {
                        let c = params.cells[2 * i];
                        c[3] > 0.0 && (c[0] - view[0]).hypot(c[1] - view[1]) < 0.75
                    })
                    .count()
            })
            .unwrap_or(0);
        prepare(&mut scene, best);
        scene.frame_at(&gpu, seconds, best)
    }

    fn previous(&self, stop: i64) -> i64 {
        if stop == self.offset {
            self.origin
        } else {
            stop - 1
        }
    }

    #[cfg(test)]
    fn frame_at(&mut self, gpu: &Graphics, seconds: f64, seed: u64) -> Vec<u8> {
        let params = self.params(seconds, seed);
        self.lens
            .render(gpu, bytemuck::bytes_of(&params), 0)
            .expect("a frame");
        self.lens.read_back(gpu, 0)
    }
}

impl Visual for Microscope {
    fn render(&mut self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        let params = self.params(seconds, seed);
        self.lens.render(gpu, bytemuck::bytes_of(&params), frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        self.lens.texture(frame)
    }

    fn travel(&mut self, legs: i32) {
        self.pending += legs;
    }

    fn whereabouts(&self) -> Option<String> {
        Some(SLIDES[self.shown].name.to_string())
    }

    fn hud(&self) -> Option<Hud> {
        let slide = &SLIDES[self.shown];
        let mut lines = vec![(slide.name.to_string(), slide.light.word().to_string())];
        if self.settled {
            let mut names: Vec<&str> = Vec::new();
            for cell in &self.cells {
                if !names.contains(&cell.kind.name()) {
                    names.push(cell.kind.name());
                }
            }
            if !names.is_empty() {
                lines.push(("In view".into(), names.join(", ")));
            }
            lines.push((
                "Objective".into(),
                slide.objectives[self.objective].0.into(),
            ));
        } else {
            lines[0].1 = "Changing slides".into();
        }
        Some(Hud {
            title: "MICROSCOPE".into(),
            lines,
            ..Hud::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_slide_fits_the_shader() {
        for slide in &SLIDES {
            let count: usize = slide.life.iter().map(|(_, n)| n).sum();
            assert!(count <= CELLS, "{} has {count} organisms", slide.name);
        }
    }

    /// Organisms leave the torus round the view on one side and come back on
    /// the other, so they always lie on it, and wrap only where all of them
    /// is outside the field stop, at the strongest objective's zoom too.
    #[test]
    fn organisms_stay_on_the_torus() {
        let mut dice = Dice::new(9);
        for slide in &SLIDES {
            for cell in populate(slide, &mut dice) {
                for (t, view) in [
                    (0.0, [0.0, 0.0]),
                    (13.0, [0.4, -0.2]),
                    (3600.0, [-3.0, 7.0]),
                ] {
                    let (at, _, _) = cell.pose(t, view);
                    for k in 0..2 {
                        assert!((at[k] - view[k]).abs() <= TORUS * 0.5 + 1e-4);
                    }
                }
                assert!(
                    TORUS * 0.5 - cell.size * cell.kind.reach() > 1.0,
                    "{:?}",
                    cell.kind
                );
            }
        }
    }

    /// A Vorticella's stalk snaps short and stretches out again.
    #[test]
    fn vorticella_contract() {
        let mut dice = Dice::new(4);
        let cell = Cell::new(Kind::Vorticella, &mut dice);
        let lengths: Vec<f32> = (0..300)
            .map(|i| cell.pose(i as f32 * 0.1, [0.0; 2]).2)
            .collect();
        let shortest = lengths.iter().cloned().fold(1.0, f32::min);
        let longest = lengths.iter().cloned().fold(0.0, f32::max);
        assert!(shortest < 0.1 && longest > 0.9, "{shortest}..{longest}");
    }

    /// A frame of every slide at each objective, and one through a change, to
    /// `LENS_PREVIEW_DIR`.
    #[test]
    #[ignore = "writes design previews; set LENS_PREVIEW_DIR"]
    fn previews() {
        let Ok(dir) = std::env::var("LENS_PREVIEW_DIR") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Microscope::new(&gpu).expect("a microscope");
        let seed = 3;
        let leg = HOLD + CHANGE;
        scene.params(0.0, seed);
        for k in 0..SLIDES.len() {
            for (when, label) in [(30.0, "a"), (60.0, "b")] {
                let seconds = when + k as f64 * leg;
                let pixels = scene.frame_at(&gpu, seconds, seed);
                let name = SLIDES[scene.shown]
                    .name
                    .replace([' ', ','], "-")
                    .to_lowercase();
                let path =
                    std::path::Path::new(&dir).join(format!("microscope-{k:02}{label}-{name}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a preview image");
            }
        }
        let pixels = scene.frame_at(&gpu, leg + 1.8 - CHANGE * 0.5, seed);
        let path = std::path::Path::new(&dir).join("microscope-change.png");
        image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
            .expect("a preview image");
    }
}
