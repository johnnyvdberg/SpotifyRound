//! The microscope: a naturalist's slides of pond life under the objective.
//!
//! Each slide holds a drop of water and what lives in it: paramecia swimming
//! on curving paths with their cilia beating and vacuoles pulsing, diatoms
//! gliding back and forth, volvox colonies rolling with daughter colonies
//! inside, amoebae creeping and reaching, crescents of Closterium, and debris
//! jiggling in Brownian motion. The fine focus drifts slowly up and down
//! through the drop, so one organism sharpens as another blurs. Some slides are
//! lit in bright field, some in dark field, where only the glassy edges shine.
//! A slide changes every minute and a half, or with a swipe: the view blurs
//! and slides away, and the next comes up into focus.
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
/// The field stop's radius in pixels.
const FIELD: f32 = 520.0;
/// The organisms the shader has room for.
const CELLS: usize = 16;
/// Organisms live on a torus this wide, centred on the field, so one that
/// swims out of view comes back in on the other side, outside the stop. No
/// wider than it must be for the largest (an amoeba reaching out to 1.3 of
/// its size), or most of a slide would be out of view.
const TORUS: f32 = 2.6;
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
        }
    }

    /// Half-length in field units, smallest and largest.
    fn size(self) -> (f32, f32) {
        match self {
            Kind::Paramecium => (0.15, 0.21),
            Kind::Pennate => (0.1, 0.22),
            Kind::Centric => (0.06, 0.11),
            Kind::Volvox => (0.12, 0.2),
            Kind::Amoeba => (0.13, 0.19),
            Kind::Closterium => (0.17, 0.22),
        }
    }
}

/// One slide.
struct Slide {
    name: &'static str,
    dark_field: bool,
    light: [f32; 3],
    objective: &'static str,
    /// What is on it, and how many of each.
    life: &'static [(Kind, usize)],
}

const SLIDES: [Slide; 6] = [
    Slide {
        name: "Pond water",
        dark_field: false,
        light: [0.95, 0.9, 0.78],
        objective: "10× · NA 0.25",
        life: &[
            (Kind::Paramecium, 3),
            (Kind::Amoeba, 2),
            (Kind::Closterium, 2),
            (Kind::Pennate, 4),
            (Kind::Centric, 4),
        ],
    },
    Slide {
        name: "Diatoms",
        dark_field: true,
        light: [0.95, 0.97, 1.0],
        objective: "20× · NA 0.40",
        life: &[(Kind::Pennate, 10), (Kind::Centric, 6)],
    },
    Slide {
        name: "Volvox",
        dark_field: false,
        light: [0.9, 0.92, 0.82],
        objective: "4× · NA 0.10",
        life: &[(Kind::Volvox, 8), (Kind::Pennate, 4)],
    },
    Slide {
        name: "Desmids",
        dark_field: false,
        light: [0.93, 0.92, 0.84],
        objective: "10× · NA 0.25",
        life: &[(Kind::Closterium, 9), (Kind::Pennate, 4), (Kind::Amoeba, 1)],
    },
    Slide {
        name: "Hay infusion",
        dark_field: true,
        light: [0.85, 0.92, 1.0],
        objective: "10× · NA 0.25",
        life: &[(Kind::Paramecium, 10), (Kind::Amoeba, 3)],
    },
    Slide {
        name: "Amoebae",
        dark_field: false,
        light: [0.94, 0.88, 0.8],
        objective: "20× · NA 0.40",
        life: &[(Kind::Amoeba, 5), (Kind::Centric, 6), (Kind::Paramecium, 3)],
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
    /// Speed along its heading, field units a second.
    speed: f32,
    /// Turn, radians a second.
    turn: f32,
}

impl Cell {
    fn new(kind: Kind, dice: &mut Dice) -> Self {
        let (small, large) = kind.size();
        let (speed, turn) = match kind {
            Kind::Paramecium => (dice.range(0.035, 0.06), dice.range(-0.04, 0.04)),
            Kind::Pennate | Kind::Centric => (0.0, 0.0),
            Kind::Volvox => (dice.range(0.008, 0.016), 0.0),
            Kind::Amoeba => (dice.range(0.003, 0.006), dice.range(-0.01, 0.01)),
            Kind::Closterium => (0.002, dice.range(-0.008, 0.008)),
        };
        Self {
            kind,
            at: [dice.range(-0.5, 0.5) * TORUS, dice.range(-0.5, 0.5) * TORUS],
            heading: dice.range(0.0, std::f32::consts::TAU),
            size: dice.range(small, large),
            depth: dice.range(-1.0, 1.0),
            phase: dice.next(),
            hue: dice.next(),
            speed,
            turn,
        }
    }

    /// Where it is and which way it points `t` seconds in.
    fn pose(&self, t: f32) -> ([f32; 2], f32) {
        let wobble =
            |rate: f32, by: f32| by * (t * rate + self.phase * std::f32::consts::TAU).sin();
        let (mut x, mut y, angle);
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
                    _ => heading,
                };
            }
        }
        let half = TORUS * 0.5;
        x = (x + half).rem_euclid(TORUS) - half;
        y = (y + half).rem_euclid(TORUS) - half;
        ([x, y], angle)
    }
}

/// The organisms on one slide of one visit, nearest the coverslip last.
fn populate(slide: &Slide, dice: &mut Dice) -> Vec<Cell> {
    let mut cells: Vec<Cell> = slide
        .life
        .iter()
        .flat_map(|&(kind, count)| std::iter::repeat_n(kind, count))
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
        let index = self.order[wrap(stop, SLIDES.len())];
        let slide = &SLIDES[index];
        let mut dice = Dice::new(seed ^ (stop as u64).wrapping_mul(0x2545_f491_4f6c_dd1d));
        let debris = dice.range(0.0, 100.0);
        let heading = dice.range(0.0, std::f32::consts::TAU);
        if self.stop != Some(stop) {
            self.stop = Some(stop);
            self.cells = populate(slide, &mut dice);
        }
        self.shown = index;
        self.settled = arrived >= 1.0;

        // A change of slide: the view blurs and the stage slides away, then
        // the next slide slides in and comes into focus.
        let a = arrived;
        let (fade, travel, blur) = if a < 0.5 {
            let x = smooth(0.0, 0.5, a);
            (1.0 - 0.7 * x, x * 1.2, x * 0.05)
        } else {
            let x = smooth(0.5, 1.0, a);
            (0.3 + 0.7 * x, -(1.0 - x) * 1.2, (1.0 - x) * 0.05)
        };
        let t = seconds as f32;
        let drift = [0.006 * (t * 0.05).sin(), 0.006 * (t * 0.037).cos()];
        let mut cells = [[0.0f32; 4]; 2 * CELLS];
        for (i, cell) in self.cells.iter().enumerate() {
            let (at, angle) = cell.pose(t);
            cells[2 * i] = [at[0], at[1], angle, cell.size];
            cells[2 * i + 1] = [cell.kind as u8 as f32, cell.depth, cell.phase, cell.hue];
        }
        let focus = 0.85 * (t * std::f32::consts::TAU / FOCUS_PERIOD).sin();
        Params {
            frame: [SIZE as f32 * 0.5, SIZE as f32 * 0.5, FIELD, t],
            look: [
                if slide.dark_field { 1.0 } else { 0.0 },
                focus,
                fade,
                debris,
            ],
            stage: [
                drift[0] + heading.cos() * travel,
                drift[1] + heading.sin() * travel,
                blur,
                1.0 / FIELD,
            ],
            light: [slide.light[0], slide.light[1], slide.light[2], 1.0],
            cells,
        }
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
        let light = if slide.dark_field {
            "Dark field"
        } else {
            "Bright field"
        };
        let mut lines = vec![(slide.name.to_string(), light.to_string())];
        if self.settled {
            let mut names: Vec<&str> = slide.life.iter().map(|(k, _)| k.name()).collect();
            names.dedup();
            lines.push(("In view".into(), names.join(", ")));
            lines.push(("Objective".into(), slide.objective.into()));
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

    /// Organisms leave the torus on one side and come back on the other, so
    /// they always lie on it, and wrap only far outside the field stop.
    #[test]
    fn organisms_stay_on_the_torus() {
        let mut dice = Dice::new(9);
        for slide in &SLIDES {
            for cell in populate(slide, &mut dice) {
                for t in [0.0, 13.0, 600.0, 3600.0] {
                    let (at, _) = cell.pose(t);
                    assert!(at.iter().all(|v| v.abs() <= TORUS * 0.5 + 1e-4));
                }
                // Far enough out that its whole outline is past the stop.
                assert!(TORUS * 0.5 - cell.size * 1.3 > 1.0, "{:?}", cell.kind);
            }
        }
    }

    /// A frame of every slide, and one through a change, to `LENS_PREVIEW_DIR`.
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
            let seconds = 30.0 + k as f64 * leg;
            let pixels = scene.frame_at(&gpu, seconds, seed);
            let name = SLIDES[scene.shown].name.replace(' ', "-").to_lowercase();
            let path = std::path::Path::new(&dir).join(format!("microscope-{k:02}-{name}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a preview image");
        }
        let pixels = scene.frame_at(&gpu, leg + 1.8 - CHANGE * 0.5, seed);
        let path = std::path::Path::new(&dir).join("microscope-change.png");
        image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
            .expect("a preview image");
    }
}
