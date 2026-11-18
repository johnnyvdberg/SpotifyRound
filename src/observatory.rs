//! The observatory: a night at the telescope, seen through its eyepiece.
//!
//! The tour holds each target for a little over a minute: the Moon along its
//! terminator, Jupiter with its four large moons, Saturn's rings, Mars, the
//! Orion Nebula, the Andromeda and Whirlpool galaxies, the Ring Nebula, the
//! great cluster in Hercules, the Pleiades and the double star Albireo. Between
//! them the telescope slews: the stars streak and the view goes dark, then the
//! next target comes up out of focus and sharpens. The air shimmers over
//! everything, more at the high power the planets need, and bright stars
//! twinkle. Each visit shuffles the order; a swipe slews on to the next target
//! or back.
//!
//! The whole view is one small fragment shader (`shaders/lens/observatory.wgsl`)
//! over one uniform block: no maps, no meshes. See [`crate::lens`].

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
    include_str!("shaders/lens/observatory.wgsl")
);

/// Seconds the telescope stays on a target once it has arrived.
const HOLD: f64 = 70.0;
/// Seconds of a slew: the old target streaks away and fades, the new one
/// comes up out of focus and sharpens.
const SLEW: f64 = 6.0;
/// The field stop's radius in pixels: just inside the panel, so the lip of the
/// eyepiece's barrel shows round the edge.
const FIELD: f32 = 520.0;

/// The uniform block (`Params` in the shader).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Params {
    frame: [f32; 4],
    view: [f32; 4],
    look: [f32; 4],
    slew: [f32; 4],
    colour_a: [f32; 4],
    colour_b: [f32; 4],
    shape: [f32; 4],
    extra: [f32; 4],
    stars: [[f32; 4]; 12],
}

/// How the shader draws a target (`look.x`), with its variant.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    Moon,
    /// 0 Jupiter, 1 Saturn, 2 Mars.
    Planet(u8),
    /// 0 an emission nebula, 1 a galaxy, 2 a planetary nebula.
    Diffuse(u8),
    /// 0 an open cluster, 1 a globular, 2 a double star.
    Cluster(u8),
}

impl Kind {
    fn code(self) -> [f32; 2] {
        match self {
            Kind::Moon => [0.0, 0.0],
            Kind::Planet(v) => [1.0, v as f32],
            Kind::Diffuse(v) => [2.0, v as f32],
            Kind::Cluster(v) => [3.0, v as f32],
        }
    }
}

/// One thing to look at, with what the HUD says about it.
struct Target {
    name: &'static str,
    kind: Kind,
    /// What it is, how far, how bright (visual magnitude), and the eyepiece.
    what: &'static str,
    distance: &'static str,
    magnitude: &'static str,
    eyepiece: &'static str,
    /// How much the air shimmers at this power.
    seeing: f32,
    /// The field stars' density, 0..1: the Milky Way's in Cygnus is high.
    stars: f32,
    colour_a: [f32; 3],
    colour_b: [f32; 3],
    shape: [f32; 4],
    extra: [f32; 4],
    /// The target's own bright stars: x, y, brightness, colour.
    bright: &'static [[f32; 4]],
}

/// The Pleiades' brightest, roughly as they stand, Alcyone in the middle.
const PLEIADES: [[f32; 4]; 9] = [
    [0.0, 0.0, 1.5, 0.12],     // Alcyone
    [-0.42, -0.04, 1.1, 0.12], // Atlas
    [-0.42, 0.06, 0.45, 0.1],  // Pleione
    [0.52, 0.16, 1.0, 0.1],    // Electra
    [0.32, 0.3, 0.9, 0.1],     // Maia
    [0.28, -0.12, 0.85, 0.1],  // Merope
    [0.4, 0.44, 0.75, 0.1],    // Taygeta
    [0.58, 0.36, 0.4, 0.1],    // Celaeno
    [0.44, 0.58, 0.3, 0.1],    // Asterope
];

/// The Trapezium at the heart of the Orion Nebula, and two stars beside it.
const TRAPEZIUM: [[f32; 4]; 6] = [
    [0.0, 0.0, 0.9, 0.05],
    [0.034, 0.012, 0.65, 0.05],
    [0.014, 0.032, 0.5, 0.05],
    [0.04, -0.022, 0.45, 0.05],
    [-0.3, -0.42, 0.7, 0.2],
    [0.18, 0.5, 0.5, 0.3],
];

/// Albireo: a gold giant and its blue companion, 35 arc seconds apart.
const ALBIREO: [[f32; 4]; 2] = [[-0.07, -0.05, 1.8, 0.95], [0.07, 0.06, 0.9, 0.0]];

const SATURN_MOONS: [[f32; 4]; 2] = [[0.78, 0.1, 0.45, 0.6], [-0.55, -0.06, 0.15, 0.5]];

const M13: [[f32; 4]; 3] = [
    [0.12, 0.08, 0.6, 0.8],
    [-0.2, 0.15, 0.45, 0.7],
    [0.05, -0.25, 0.4, 0.85],
];

const TARGETS: [Target; 11] = [
    Target {
        name: "The Moon",
        kind: Kind::Moon,
        what: "Along the terminator",
        distance: "384,400 km",
        magnitude: "−11",
        eyepiece: "25 mm · 80×",
        seeing: 0.9,
        stars: 0.0,
        colour_a: [0.0; 3],
        colour_b: [0.0; 3],
        shape: [1.7, 0.0, 0.0, 0.0],
        extra: [0.0; 4],
        bright: &[],
    },
    Target {
        name: "Jupiter",
        kind: Kind::Planet(0),
        what: "Io, Europa, Ganymede, Callisto",
        distance: "5.2 AU from the Sun",
        magnitude: "−2.5",
        eyepiece: "9 mm · 220×",
        seeing: 1.3,
        stars: 0.02,
        colour_a: [0.86, 0.77, 0.62],
        colour_b: [0.56, 0.4, 0.27],
        shape: [0.24, 0.06, 0.0, 22.0],
        extra: [0.0; 4],
        // The moons are placed per visit (`Observatory::stars`).
        bright: &[],
    },
    Target {
        name: "Saturn",
        kind: Kind::Planet(1),
        what: "The rings and Titan",
        distance: "9.5 AU from the Sun",
        magnitude: "+0.6",
        eyepiece: "9 mm · 220×",
        seeing: 1.3,
        stars: 0.02,
        colour_a: [0.84, 0.75, 0.53],
        colour_b: [0.7, 0.61, 0.43],
        shape: [0.17, -0.12, 0.34, 14.0],
        extra: [0.0; 4],
        bright: &SATURN_MOONS,
    },
    Target {
        name: "Mars",
        kind: Kind::Planet(2),
        what: "Syrtis Major and the polar cap",
        distance: "1.5 AU from the Sun",
        magnitude: "−1.4",
        eyepiece: "6 mm · 330×",
        seeing: 1.6,
        stars: 0.02,
        colour_a: [0.85, 0.44, 0.22],
        colour_b: [0.42, 0.22, 0.13],
        shape: [0.13, 0.35, 0.0, 0.0],
        extra: [0.0; 4],
        bright: &[],
    },
    Target {
        name: "M42 · Orion Nebula",
        kind: Kind::Diffuse(0),
        what: "Emission nebula",
        distance: "1,344 light years",
        magnitude: "+4.0",
        eyepiece: "32 mm · 62×",
        seeing: 0.5,
        stars: 0.3,
        colour_a: [0.36, 0.72, 0.62],
        colour_b: [0.8, 0.32, 0.45],
        shape: [0.85, 0.4, 0.0, 0.55],
        extra: [0.0; 4],
        bright: &TRAPEZIUM,
    },
    Target {
        name: "M31 · Andromeda Galaxy",
        kind: Kind::Diffuse(1),
        what: "Spiral galaxy, with M32",
        distance: "2.5 million light years",
        magnitude: "+3.4",
        eyepiece: "40 mm · 50×",
        seeing: 0.5,
        stars: 0.32,
        colour_a: [1.0, 0.86, 0.66],
        colour_b: [0.62, 0.66, 0.76],
        shape: [1.3, 0.62, 0.28, 0.55],
        extra: [0.2, -0.24, 0.35, 0.25],
        bright: &[],
    },
    Target {
        name: "M51 · Whirlpool Galaxy",
        kind: Kind::Diffuse(1),
        what: "Spiral galaxy, with NGC 5195",
        distance: "31 million light years",
        magnitude: "+8.4",
        eyepiece: "13 mm · 150×",
        seeing: 0.6,
        stars: 0.14,
        colour_a: [1.0, 0.88, 0.7],
        colour_b: [0.6, 0.68, 0.85],
        shape: [0.55, 1.0, 0.85, 0.45],
        extra: [0.34, 0.3, 0.5, 1.0],
        bright: &[],
    },
    Target {
        name: "M57 · Ring Nebula",
        kind: Kind::Diffuse(2),
        what: "Planetary nebula",
        distance: "2,300 light years",
        magnitude: "+8.8",
        eyepiece: "9 mm · 220×",
        seeing: 0.8,
        stars: 0.22,
        colour_a: [0.9, 0.38, 0.32],
        colour_b: [0.35, 0.8, 0.68],
        shape: [0.16, 0.5, 0.75, 0.5],
        extra: [0.0; 4],
        bright: &[[0.0, 0.0, 0.08, 0.0]],
    },
    Target {
        name: "M13 · Hercules Cluster",
        kind: Kind::Cluster(1),
        what: "Globular cluster",
        distance: "22,200 light years",
        magnitude: "+5.8",
        eyepiece: "13 mm · 150×",
        seeing: 0.6,
        stars: 0.18,
        colour_a: [1.0, 0.86, 0.62],
        colour_b: [0.0; 3],
        shape: [0.7, 0.0, 0.0, 0.18],
        extra: [0.0; 4],
        bright: &M13,
    },
    Target {
        name: "M45 · Pleiades",
        kind: Kind::Cluster(0),
        what: "Open cluster",
        distance: "444 light years",
        magnitude: "+1.6",
        eyepiece: "40 mm · 50×",
        seeing: 0.5,
        stars: 0.2,
        colour_a: [0.0; 3],
        colour_b: [0.35, 0.5, 0.95],
        shape: [1.0, 0.0, 0.0, 0.06],
        extra: [0.0; 4],
        bright: &PLEIADES,
    },
    Target {
        name: "Albireo · β Cygni",
        kind: Kind::Cluster(2),
        what: "Double star, gold and blue",
        distance: "430 light years",
        magnitude: "+3.1 and +5.1",
        eyepiece: "13 mm · 150×",
        seeing: 0.7,
        stars: 0.85,
        colour_a: [0.0; 3],
        colour_b: [0.0; 3],
        shape: [1.0, 0.0, 0.0, 0.0],
        extra: [0.0; 4],
        bright: &ALBIREO,
    },
];

/// The scene.
pub struct Observatory {
    lens: Lens,
    /// The visit's seed and its order of targets.
    seed: Option<u64>,
    order: Vec<usize>,
    /// The tour: the stop it set out from, where it was headed, and when.
    origin: i64,
    offset: i64,
    since: f64,
    /// Swipes not yet taken up by a frame.
    pending: i32,
    /// What the last frame showed, for the HUD.
    shown: usize,
    settled: bool,
}

impl Observatory {
    pub fn new(gpu: &Graphics) -> Result<Self> {
        let lens = Lens::new(
            gpu,
            "Observatory",
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
            shown: 0,
            settled: false,
        })
    }

    /// The uniforms for `seconds` into the visit with `seed`.
    fn params(&mut self, seconds: f64, seed: u64) -> Params {
        if self.seed != Some(seed) {
            self.seed = Some(seed);
            self.order = Dice::new(seed).order(TARGETS.len());
            self.origin = 0;
            self.offset = 0;
            // The visit opens halfway through a slew: in the dark, with the
            // first target coming into focus.
            self.since = seconds - SLEW * 0.5;
            self.pending = 0;
        }
        if self.pending != 0 {
            let (stop, arrived) = tour_at(seconds - self.since, self.offset, HOLD, SLEW);
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
        let (stop, arrived) = tour_at(seconds - self.since, self.offset, HOLD, SLEW);
        let stop = if arrived < 0.5 {
            self.previous(stop)
        } else {
            stop
        };
        let index = self.order[wrap(stop, TARGETS.len())];
        let target = &TARGETS[index];
        self.shown = index;
        self.settled = arrived >= 1.0;

        // Each stop of each visit is a little different: where on the Moon,
        // where Jupiter's moons are, which field stars, which way the slew.
        let mut dice = Dice::new(seed ^ (stop as u64).wrapping_mul(0x2545_f491_4f6c_dd1d));
        let heading = dice.range(0.0, std::f32::consts::TAU);
        let star_seed = dice.range(0.0, 200.0);

        // The slew: out along `heading`, dark in the middle, then in from the
        // other side, out of focus, sharpening as it settles.
        let a = arrived;
        let (fade, streak, travel, defocus) = if a < 0.5 {
            let x = smooth(0.0, 0.45, a);
            (1.0 - x, x * 0.03, x * 0.5, 0.0)
        } else {
            let x = smooth(0.55, 1.0, a);
            let focus = 1.0 - smooth(0.6, 1.0, a);
            (x, (1.0 - x) * 0.03, -(1.0 - x) * 0.5, focus * focus * 0.02)
        };
        let dir = [heading.cos(), heading.sin()];
        let t = seconds as f32;
        // The mount tracks well but not perfectly: a slow wander.
        let wander = [
            0.02 * (t * 0.031).sin() + 0.01 * (t * 0.07).cos(),
            0.02 * (t * 0.023).cos(),
        ];
        // Seeing: the image dances a pixel or two as the air moves.
        let dance = target.seeing / FIELD;
        let wander = [
            wander[0] + dance * ((t * 3.1).sin() + 0.6 * (t * 7.7).sin()),
            wander[1] + dance * ((t * 2.7).cos() + 0.6 * (t * 6.9).sin()),
        ];
        let mut aim = [0.0f32; 2];
        let mut extra = target.extra;
        if target.kind == Kind::Moon {
            // A spot near the limb, the terminator crossing the field.
            let angle = dice.range(-0.6, 0.6);
            let reach = dice.range(0.95, 1.15);
            aim = [reach * angle.cos(), reach * angle.sin()];
            let sun = normalize([1.0, 0.15 + angle * 0.4, -0.6]);
            let sun = rotate_xy(sun, angle * 0.6);
            extra = [sun[0], sun[1], sun[2], 0.0];
        }

        let mut stars = [[0.0f32; 4]; 12];
        let bright: Vec<[f32; 4]> = if target.kind == Kind::Planet(0) {
            jupiter_moons(&mut dice, target.shape)
        } else {
            target.bright.to_vec()
        };
        for (slot, star) in stars.iter_mut().zip(&bright) {
            *slot = *star;
        }

        let [kind, variant] = target.kind.code();
        Params {
            frame: [SIZE as f32 * 0.5, SIZE as f32 * 0.5, FIELD, t],
            view: [
                aim[0] + wander[0] + dir[0] * travel,
                aim[1] + wander[1] + dir[1] * travel,
                target.seeing,
                defocus,
            ],
            look: [kind, variant, fade, star_seed],
            slew: [dir[0] * streak, dir[1] * streak, target.stars, 1.0 / FIELD],
            colour_a: rgb(target.colour_a),
            colour_b: rgb(target.colour_b),
            shape: target.shape,
            extra,
            stars,
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

    /// The stop the tour came from on its way to `stop`.
    fn previous(&self, stop: i64) -> i64 {
        if stop == self.offset {
            self.origin
        } else {
            stop - 1
        }
    }
}

/// Jupiter's four large moons, strung along its equator at this visit's
/// phases; one behind the disc is hidden, and so is one crossing it, which a
/// small telescope barely shows.
fn jupiter_moons(dice: &mut Dice, shape: [f32; 4]) -> Vec<[f32; 4]> {
    let (radius, tilt) = (shape[0], shape[1]);
    [2.0f32, 2.7, 3.4, 4.0]
        .iter()
        .map(|reach| {
            let phase = dice.range(0.0, std::f32::consts::TAU);
            let x = reach * radius * phase.sin();
            let behind = x.abs() < radius * 1.1;
            let (s, c) = tilt.sin_cos();
            [x * c, x * s, if behind { 0.0 } else { 0.55 }, 0.55]
        })
        .collect()
}

fn rgb(c: [f32; 3]) -> [f32; 4] {
    [c[0], c[1], c[2], 1.0]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let l = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / l, v[1] / l, v[2] / l]
}

fn rotate_xy(v: [f32; 3], angle: f32) -> [f32; 3] {
    let (s, c) = angle.sin_cos();
    [c * v[0] - s * v[1], s * v[0] + c * v[1], v[2]]
}

impl Visual for Observatory {
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
        Some(TARGETS[self.shown].name.to_string())
    }

    fn hud(&self) -> Option<Hud> {
        let target = &TARGETS[self.shown];
        let mut lines = vec![(target.name.to_string(), target.what.to_string())];
        if self.settled {
            lines.push(("Distance".into(), target.distance.into()));
            lines.push(("Magnitude".into(), target.magnitude.into()));
            lines.push(("Eyepiece".into(), target.eyepiece.into()));
        } else {
            lines[0].1 = "Slewing".into();
        }
        Some(Hud {
            title: "OBSERVATORY".into(),
            lines,
            ..Hud::default()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_visit_sees_every_target() {
        let mut order = Dice::new(1).order(TARGETS.len());
        order.sort();
        assert_eq!(order, (0..TARGETS.len()).collect::<Vec<_>>());
    }

    /// Frames of every target, settled, to `LENS_PREVIEW_DIR`.
    #[test]
    #[ignore = "writes design previews; set LENS_PREVIEW_DIR"]
    fn previews() {
        let Ok(dir) = std::env::var("LENS_PREVIEW_DIR") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Observatory::new(&gpu).expect("an observatory");
        let seed = 5;
        let leg = HOLD + SLEW;
        // The visit starts at 0.
        scene.params(0.0, seed);
        for k in 0..TARGETS.len() {
            let seconds = 20.0 + k as f64 * leg;
            let pixels = scene.frame_at(&gpu, seconds, seed);
            let name = TARGETS[scene.shown]
                .name
                .split(' ')
                .next_back()
                .unwrap_or("x");
            let path = std::path::Path::new(&dir)
                .join(format!("observatory-{k:02}-{}.png", name.to_lowercase()));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a preview image");
        }
        // Mid-slew, streaking.
        let pixels = scene.frame_at(&gpu, leg + 1.6 - SLEW * 0.5, seed);
        let path = std::path::Path::new(&dir).join("observatory-slew.png");
        image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
            .expect("a preview image");
    }
}
