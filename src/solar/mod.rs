//! A grand tour of invented star systems, on the idle screen.
//!
//! Each visit is a generated system ([`system`]): a star, five to eight
//! planets of different kinds, and moons of the kinds our own system has. The
//! camera follows a timeline ([`tour`]): it drops out of a hyperjump, crosses
//! to three of the planets in turn, orbits each high up and then low over its
//! horizon, and after the last turns toward the next star and jumps into the
//! next system: two or three short legs, something met between each two
//! (an encounter, a star gate or a wormhole, [`voyage`]). The HUD names the
//! destination and shows the ETA, distances and real orbital speeds.
//!
//! **Built for the Pi 5's VideoCore VII.** The planet maps are painted on the
//! GPU while the scene runs, a few rows a frame ([`stream`]), into two slots:
//! the planet being visited and the next one, so new worlds stream in during
//! the crossing and the one left behind is overwritten. The frame is two
//! passes: a full-screen background of stars and the galaxy, then one small
//! quad per body, far to near, so a pixel only pays for the bodies over it.

mod bake;
mod base_mesh;
mod bodies;
pub mod console;
mod encounters;
mod frame;
mod interior;
pub mod labels;
pub mod layout;
pub mod models;
mod orders;
mod pipe;
pub mod rocks;
pub mod scan;
mod setup;
pub mod stream;
pub mod system;
pub mod tour;
pub mod voyage;

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;
use bake::*;
use bodies::*;
use encounters::*;
use layout::*;
use pipe::Pipe;
use stream::{Key, Streamer, sampler_entry, texture_entry, uniform_entry};
use system::{Kind, Planet, Stop, System, equatorial_to_system};
use tour::{Phase, Tour, View, add, normalize, scale, sub};

/// The frame's shader, linked from `shaders/solar` by `build.rs`.
pub(crate) const FRAME_SHADER: &str = include_str!(concat!(env!("OUT_DIR"), "/solar_frame.wgsl"));

/// Size of the rendered frame.
pub const SIZE: u32 = 1080;
/// Cloud layers stacked over the deck in low orbit: each a pass of about
/// 330 instructions where clouds are, so one (a second added little).
const CLOUD_LAYERS: u32 = 1;
const SLOTS: usize = crate::visuals::SLOTS;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Body {
    place: [f32; 4],
    rot: [[f32; 4]; 3],
    look: [f32; 4],
    air: [f32; 4],
    colour: [f32; 4],
    ring: [f32; 4],
    ring_colour: [f32; 4],
    rect: [f32; 4],
    /// Up to four bodies that can cast a shadow on this one, by index (-1:
    /// none).
    shade: [f32; 4],
    /// The patch of its grid a mapped world's mesh covers (`vs_globe`):
    /// longitude and latitude of its corner, then their spans, in radians.
    window: [f32; 4],
    /// The mesh's cells along each side, and how far it stands proud of the
    /// surface, in radii.
    mesh: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct Params {
    frame: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    star: [f32; 4],
    star_colour: [f32; 4],
    light: [f32; 4],
    sky: [[f32; 4]; 3],
    warp: [f32; 4],
    burn: [f32; 4],
    dust: [f32; 4],
    motion: [f32; 4],
    /// The star the jump makes for: xyz its direction, w how brightly it is
    /// marked; and its colour.
    goal: [f32; 4],
    goal_colour: [f32; 4],
    /// The drive spooling up: x how far through its preparation, y how much
    /// of its start-up effect shows.
    spool: [f32; 4],
    /// The stopover between legs of the jump, or what is seen inside one
    /// (the `EV_*` rows of [`layout`]).
    ev: [[f32; 4]; EV_ROWS],
    bodies: [Body; MAX_BODIES],
}

/// The starship meshes on the GPU: every one in one buffer, and where each
/// lies (`models::Library::ranges`).
struct Models {
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// The frame's placings of the models, grouped by mesh.
    instances: wgpu::Buffer,
    library: models::Library,
}

/// The frame's model placings and draws, kept from frame to frame so
/// sorting them allocates nothing once they have grown (`Solar::draw`).
#[derive(Default)]
struct ModelScratch {
    placed: Vec<(models::Key, models::Instance)>,
    nearest: std::collections::HashMap<models::Key, f64>,
    order: Vec<(f64, models::Key, f64, usize)>,
    flat: Vec<models::Instance>,
    model_draws: Vec<ModelDraw>,
}

/// A draw of one mesh's placings: its family, where the mesh lies
/// (`models::Library::ranges`), and which instances.
type ModelDraw = (u32, (u32, u32, i32), std::ops::Range<u32>);

/// The most model placings drawn in a frame.
const MODEL_INSTANCES: usize = 1024;

/// Rocks or debris drawn round the ship in a stopover (`vs_rock`).
const ROCKS: u32 = 260;
/// How many times coarser than the frame the leviathans' halos are drawn
/// (the scale `vs_leviathan_glow` reads them at), and in what.
const LEVIATHAN_HALO: u32 = 4;
const LEVIATHAN_HALO_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
/// A gate's guide buoys (two a step down the approach) and the most nodes
/// round its ring (`vs_glow`).
const GATE_BUOYS: u32 = 24;
const GATE_NODES: u32 = 9;
/// The most panels a Dyson swarm has: six rings of three rows, each of up
/// to 150 cells and one more where the ring's seam cuts one in two.
const DYSON_PANELS: u32 = 6 * 3 * 151;
/// A panel's quads (`DYSON_GRID` in `encounter.wesl`), two triangles each.
const DYSON_PANEL_VERTICES: u32 = 2 * 3 * 6;
/// Vertices of the ship's own effects' quad (`vs_overlay`).
const OVERLAY_VERTICES: u32 = 3;

/// How fast the dust drifts (box widths a second) and how long its streaks
/// grow at full throttle (in box widths).
const DUST_SPEED: f64 = 1.4;
const DUST_STREAK: f32 = 0.14;

/// The depth mask's format: ranks of up to 17 bodies need few bits.
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth16Unorm;
/// The baked starfield, a cube map: a texel about 1.8 of the frame's pixels
/// at a face's middle, so stars stay small; 24 MB each, two of them.
const STARFIELD_FACE: u32 = 1024;
/// The galaxy alone beside it, soft: baked whole as a starfield's starts.
const SKY_CUBE_FACE: u32 = 128;
/// Its rows, face after face: how far a bake has come.
const STARFIELD_HEIGHT: u32 = STARFIELD_FACE * 6;
/// Rows of it baked a frame: about a millisecond on the Pi.
const STARFIELD_ROWS: u32 = 64;
/// Vertices of a body's draw: a fan of 16 triangles round its silhouette.
const BODY_VERTICES: u32 = 48;
/// The inscribed depth mask's draw: `MASK_SIDES` triangles.
const MASK_VERTICES: u32 = MASK_SIDES * 3;
/// The ring's draw: `RING_SIDES` quads.
const RING_VERTICES: u32 = RING_SIDES * 6;
/// The limb's annulus: `LIMB_SIDES` sides of two quads.
const LIMB_RING_VERTICES: u32 = LIMB_SIDES * 2 * 6;
/// The star's glare polygon (`vs_glare`): a fan of `GLARE_SIDES` sides,
/// drawn as a list of triangles.
const GLARE_VERTICES: u32 = (GLARE_SIDES - 2) * 3;
/// Half a step of the 8-bit frame near black, in linear light: where the
/// glare's polygon ends (`GLARE_FLOOR`).
const GLARE_FLOOR: f64 = 1.5e-4;
/// How far round the sky from the star the glare may reach as a polygon, in
/// radians; farther (a star flyby), it is drawn over the whole frame.
const GLARE_POLYGON: f64 = 1.35;

/// A mapped world's mesh (`vs_globe`): grids of so many cells a side, all
/// with one stride, a vertex being `row * GRID_STRIDE + column` (no vertex
/// buffer). One index buffer holds each grid's triangles in turn
/// ([`grid_indices`], [`grid_range`]).
const GRIDS: [u32; 6] = [8, 16, 32, 64, 96, 128];
/// The fewest cells a side for a ringed world's mesh, whose rings' shadow
/// is worked out at the vertices.
const RINGED_GRID: u32 = 32;

/// The triangles of every grid in `GRIDS`, one after another: each cell two,
/// counter-clockwise seen from outside (its column runs east, its row north).
fn grid_indices() -> Vec<u16> {
    let mut indices = Vec::new();
    for cells in GRIDS {
        for row in 0..cells {
            for column in 0..cells {
                let a = row * GRID_STRIDE + column;
                let (east, north) = (a + 1, a + GRID_STRIDE);
                let corner = north + 1;
                indices.extend([a, east, corner, a, corner, north].map(|i| i as u16));
            }
        }
    }
    indices
}

/// Where the triangles of the grid of `cells` a side lie in the index
/// buffer: 8 at 0, 16 at 384, 32 at 1920, 64 at 8064, 96 at 32640 and 128
/// at 87936, 186240 in all.
fn grid_range(cells: u32) -> std::ops::Range<u32> {
    let mut start = 0;
    for n in GRIDS {
        let count = n * n * 6;
        if n >= cells {
            return start..start + count;
        }
        start += count;
    }
    start - 128 * 128 * 6..start
}

/// `SOLAR_PROCEDURAL_STARS`: draw the sky per pixel (`fs_space`) even once
/// the starfield is baked, to compare the two. Read once.
fn procedural_stars() -> bool {
    static SET: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *SET.get_or_init(|| std::env::var_os("SOLAR_PROCEDURAL_STARS").is_some())
}

/// Anisotropic filtering of the planet maps, for measuring on the Pi:
/// `SOLAR_ANISO=<2..16>` sharpens them toward the limb and the poles. Off by
/// default (on the Pi, `SOLAR_ANISO=4` cost 1.3-1.5 ms in orbit).
fn map_anisotropy() -> u16 {
    std::env::var("SOLAR_ANISO")
        .ok()
        .and_then(|v| v.parse::<u16>().ok())
        .map_or(1, |n| n.clamp(1, 16))
}

struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// What the HUD shows: a title, and lines of label and value.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Hud {
    pub title: String,
    pub lines: Vec<(String, String)>,
    /// The scanner's readings, while orbiting, and how far through the
    /// whole survey, 0..1.
    pub scans: Vec<scan::Scan>,
    pub survey: f32,
    /// The minimap of the system, while in one.
    pub map: Option<Minimap>,
    /// The manoeuvre under way and why, while not orbiting.
    pub manoeuvre: Option<tour::Manoeuvre>,
    /// The star the next jump makes for, where it is in view.
    pub target: Option<Target>,
    /// The ship's alarm between the stars, 0..1 (`voyage::Shot::alert`),
    /// or an incident's aboard.
    pub alert: f32,
    /// An incident's transmission, while it shows (`console`).
    pub notice: Option<console::Notice>,
}

/// The next jump's star, marked on the frame.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Target {
    /// Where it is, in the frame's pixels.
    pub at: [f32; 2],
    pub label: String,
    /// How brightly the mark shows, 0..1, and how far it has locked on, 0..1.
    pub shown: f32,
    pub lock: f32,
}

/// A small tilted map of the system: its orbits, its planets, the route
/// being flown and where the ship is, laid out in a `MAP` square.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Minimap {
    /// Path commands for every orbit.
    pub orbits: String,
    /// Path commands for the route still to fly.
    pub route: String,
    pub dots: Vec<MapDot>,
    pub ship: [f32; 2],
}

/// A planet, or the star, on the minimap.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MapDot {
    pub at: [f32; 2],
    pub size: f32,
    /// sRGB.
    pub colour: [u8; 3],
    pub label: String,
    /// The stop being visited or made for.
    pub current: bool,
}

/// The minimap's side, in its own units.
pub const MAP: f32 = 240.0;

pub(crate) struct Solar {
    /// The frame's small pipelines, by name: `space`, `spacebaked`,
    /// `glare`, `glarefull`, `warp1`..`warp5`, `dropout1`..`dropout5`,
    /// `globestar`, `limb`, `limbring`, `ring`, and per map slot `ground0`,
    /// `sea0`, `clouds0`, `glow0` (and `…1`), the worlds from afar as meshes
    /// `globe0…` (`SurfaceLook`), moons `globemoon…` and worlds whose maps are
    /// not in `globeflat…`.
    /// Each with its label, which the Pi estimate matches to the shader-db.
    pipelines: std::collections::HashMap<Pipe, (String, wgpu::RenderPipeline)>,
    /// A drive to use for every jump instead of each system's own, for the
    /// screenshots.
    drive_override: Option<u32>,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// A battle's effects this frame, by kind (`battle_effects`).
    effects: wgpu::Buffer,
    /// A Dyson swarm's panels this frame (`dyson_panels`).
    panels: wgpu::Buffer,
    /// The meshes' triangles, every grid's (`grid_indices`).
    grid: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    /// The leviathans' halos, drawn small before the frame's pass, and the
    /// bind group reading them in it (`fs_leviathan_glow`).
    leviathan_halos: (wgpu::TextureView, wgpu::BindGroup),
    /// The plasma storm or the remnant, drawn at half the frame's
    /// resolution before the frame's pass and laid over the sky from there
    /// (`fs_coarse`).
    coarse: (wgpu::TextureView, wgpu::BindGroup),
    /// The battle's starship meshes: on their way from the thread building
    /// (or loading) them, then on the GPU, with where each one lies.
    model_library: Option<std::sync::mpsc::Receiver<models::Library>>,
    models: Option<Models>,
    /// The model placings' working space (`ModelScratch`).
    scratch: ModelScratch,
    /// Each ringed planet's ring density by radius, a row per planet, and the
    /// system it was painted for.
    rings: wgpu::Texture,
    rings_for: u64,
    /// The rock shapes' surface and horizon maps (`rock_maps`), and the
    /// layout of their group.
    craters: (wgpu::Texture, wgpu::BindGroup),
    crater_layout: wgpu::BindGroupLayout,
    /// The nebula's gas and the plasma storm's (`bake_gas`, `bake_storm`).
    gas: [(wgpu::Texture, wgpu::BindGroup); 2],
    bound: (u64, usize, usize),
    /// The baked starfields (galaxy and stars, cube maps), for this system
    /// and the next, and whose they are: seed and rows done. Beside each,
    /// its galaxy alone, small and soft, for the drives that smear it.
    starfields: [wgpu::Texture; 2],
    sky_cubes: [wgpu::Texture; 2],
    starfield_owner: [Option<(u64, u32)>; 2],
    frames: Vec<Frame>,
    /// The depth mask: each body's distance rank over its solid disc, so the
    /// GPU skips shading whatever a nearer body hides (early depth test).
    depth: wgpu::TextureView,
    streamer: Streamer,
    base_seed: u64,
    visit: u64,
    visit_start: f64,
    tour: Tour,
    /// Orders from the ship's terminal, carried out on the next frame.
    pending: Vec<console::Order>,
    /// What the terminal shows, and the next stars' rows, for the visit they
    /// were listed from.
    console: console::Console,
    stars: (u64, Vec<(u64, console::Row)>),
    /// The incidents this visit has by themselves (`console::schedule`),
    /// for the visit and tour seed they were drawn for; one raised from the
    /// terminal, when (in the scene's seconds) and which; and the visit
    /// whose scheduled change of course has been made.
    incidents: (u64, u64, Vec<(f64, usize)>),
    raised: Option<(f64, usize)>,
    diverted: Option<(u64, usize)>,
    hud: Hud,
    hud_at: f64,
    /// The labels on the worlds in view, worked out every frame.
    labels: Vec<labels::BodyLabel>,
    /// How far the dust round the ship has drifted, in its box's units, and
    /// the visit time that was worked out for.
    dust: [f64; 3],
    dust_at: Option<f64>,
    /// A camera to use instead of the tour's, for the screenshots.
    view_override: Option<View>,
    /// A battle's one ship to show alone, for the screenshots: its fleet's
    /// design and which ship (0 a capital, 1 an escort).
    showcase: Option<(u32, u32)>,
    /// The last frame's background and draws, for the benchmarks.
    #[cfg(test)]
    drawn: std::cell::RefCell<String>,
}

fn system_for(base: u64, visit: u64) -> System {
    System::generate(base ^ visit.wrapping_mul(0x9e37_79b9_7f4a_7c15))
}

/// Whether visit `visit` is to a relay star, only crossed on the way to the
/// next real visit: by chance (`tour::RELAY_CHANCE`), never the first, and
/// never more than two in a row.
fn relay(base: u64, visit: u64) -> bool {
    let roll = |v: u64| {
        v > 0
            && system::Rng::new(base ^ v.wrapping_mul(0x51af_d7ed_558c_cd1b) ^ 0x4e1a).unit()
                < tour::RELAY_CHANCE
    };
    roll(visit) && !(roll(visit - 1) && roll(visit.saturating_sub(2)))
}

/// The tour of visit `visit`: a whole one or a relay's crossing, the jump at
/// its end making for the next visit's star, through it and any relays
/// after to the next real visit.
/// Until the model meshes are `ready`, its jump meets nothing drawn with
/// them. It arrives in the light `arrival` (the last trip's last leg's), or
/// what that trip would have had.
fn tour_for(base: u64, visit: u64, ready: bool, arrival: Option<u32>) -> Tour {
    let passage = |visit: u64, next: &System| {
        let drive = tour::drive_to(next).0;
        let plan = voyage::Passage::plan(base, visit, drive);
        if ready {
            plan
        } else {
            plan.without_models(drive)
        }
    };
    let mut route = Vec::new();
    let mut ahead = visit + 1;
    loop {
        route.push(system_for(base, ahead));
        if !relay(base, ahead) {
            break;
        }
        ahead += 1;
    }
    // The trip began at the last real visit (the first never a relay).
    let mut began = visit;
    while relay(base, began) {
        began -= 1;
    }
    let (leg, legs) = ((visit - began + 1) as usize, (ahead - began) as usize);
    let system = system_for(base, visit);
    // The light the ship arrives in: the last trip's last leg's.
    let arrival = arrival.unwrap_or_else(|| match visit {
        0 => tour::drive_to(&system).0,
        _ => passage(visit - 1, &system).arrival_look(),
    });
    let plan = passage(visit, &route[0]);
    Tour::plan(system, route, relay(base, visit), leg, legs, plan, arrival)
}

impl Solar {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        let seed = std::env::var("SOLAR_SEED")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(rand::random::<u64>);
        Ok(Self::with_seed(gpu, seed))
    }

    /// Move the tour on to the visit `seconds` falls in.
    fn catch_up(&mut self, seconds: f64) -> f64 {
        if seconds < self.visit_start {
            self.visit_start = seconds;
        }
        while seconds - self.visit_start >= self.tour.total {
            self.visit_start += self.tour.total;
            self.visit += 1;
            let arrival = self.tour.passage.arrival_look();
            self.tour = tour_for(
                self.base_seed,
                self.visit,
                self.models.is_some(),
                Some(arrival),
            );
            tracing::info!(
                star = %self.tour.system.star.name,
                relay = self.tour.transit,
                "arrived in a new system"
            );
        }
        seconds - self.visit_start
    }

    /// Tell the streamer what to have ready at `t` into the visit.
    fn stream(&mut self, t: f64) {
        let (segment, _) = self.tour.segment(t);
        let (current, second) = self.tour.wanted(t);
        let system = &self.tour.system;
        let mut wanted: Vec<(Key, Planet)> = [current, second]
            .into_iter()
            .flatten()
            .map(|s| ((system.seed, s.key()), system.world(s)))
            .collect();
        // Out to the jump point and on: the next visit's first stop, if it
        // has stops, and its sky.
        let leaving =
            matches!(segment.phase, Phase::Cruise { .. } | Phase::Spool) || segment.phase.deep();
        let departing = matches!(segment.phase, Phase::Depart(_));
        if (leaving || (departing && wanted.len() < 2))
            && let Some(first) = self.tour.next_first()
        {
            if wanted.len() >= 2 {
                wanted.pop();
            }
            let next = &self.tour.next;
            wanted.push(((next.seed, first.key()), next.world(first)));
        }
        let mut skies = vec![(system.seed, system.sky)];
        if leaving || departing {
            skies.push((self.tour.next.seed, self.tour.next.sky));
        }
        let wanted: Vec<(Key, &Planet)> = wanted.iter().map(|(k, p)| (*k, p)).collect();
        self.streamer.want(&wanted, &skies);
    }

    /// The camera at `t`, shaken by the engines through a burn.
    fn view(&self, t: f64) -> View {
        let mut view = self.view_override.unwrap_or_else(|| self.tour.view(t));
        // The clamps closing on the hull at a starbase, and letting go,
        // jolt it as a burn's thrust does.
        let jolt = view.base.map_or(0.0, |b| b.shake as f32 * 6.0);
        let throttle = self.throttle(t).max(jolt);
        if throttle > 0.0 {
            // The thrust shudders through the hull: a small, quick shake.
            let shake = |a: f64, b: f64| (t * a).sin() + 0.6 * (t * b + 1.3).sin();
            let amount = 0.0016 * throttle as f64;
            let right = cross(view.forward, view.up);
            view.forward = normalize(add(
                view.forward,
                add(
                    scale(right, shake(31.0, 47.0) * amount),
                    scale(view.up, shake(37.0, 53.0) * amount),
                ),
            ));
            // Square again, or rays and projections part (by a few pixels).
            view.up = normalize(cross(normalize(cross(view.forward, view.up)), view.forward));
        }
        view
    }

    /// How hard the engine fires at `t`, 0..1: zero but in a burn.
    fn throttle(&self, t: f64) -> f32 {
        self.tour.manoeuvre(t).map_or(0.0, |m| m.throttle())
    }

    /// Which way the ship is going at `t`, against the body it is at or
    /// making for (the jump's own way during it).
    fn motion(&self, t: f64) -> [f64; 3] {
        let dt = 0.05;
        let here = |t: f64| {
            let p = self.tour.view(t).position;
            match self.tour.current(t) {
                Some(stop) => sub(p, self.tour.system.centre(stop, t)),
                None => p,
            }
        };
        let d = sub(here(t + dt), here(t - dt));
        if length(d) > 1e-9 {
            normalize(d)
        } else {
            self.tour.view(t).forward
        }
    }

    /// Drift the dust past the ship while the engine fires.
    fn drift_dust(&mut self, t: f64) {
        let dt = self.dust_at.map_or(0.0, |last| (t - last).clamp(0.0, 0.1));
        self.dust_at = Some(t);
        if dt > 0.0 && self.throttle(t) > 0.0 {
            let m = self.motion(t);
            for (d, m) in self.dust.iter_mut().zip(m) {
                *d = (*d - m * DUST_SPEED * dt).rem_euclid(1.0);
            }
        }
    }

    /// The look of the light to draw for look `k` of the tour
    /// (`voyage::LOOKS`): the drives' overridden for the screenshots, a
    /// gate's conduit and a wormhole's throat as they are.
    fn look(&self, k: u32) -> u32 {
        match self.drive_override {
            Some(d) if (k as usize) <= tour::DRIVES.len() => d,
            _ => k.clamp(1, voyage::LOOKS.len() as u32),
        }
    }

    /// The drive this visit's jump uses: its number and name.
    fn drive(&self) -> (u32, &'static str) {
        match self.drive_override {
            Some(k) => (k, tour::DRIVES[(k as usize - 1) % tour::DRIVES.len()]),
            None => self.tour.drive(),
        }
    }

    fn update_hud(&mut self, t: f64) {
        if (t - self.hud_at).abs() < 0.25 {
            return;
        }
        self.hud_at = t;
        let (title, mut lines) = self.tour.hud(t);
        // A line that carries on the one before it (no label of its own)
        // takes that label: the HUD shows one line at a time.
        for k in 1..lines.len() {
            if lines[k].0.is_empty() {
                lines[k].0 = lines[k - 1].0.clone();
            }
        }
        self.hud = Hud {
            title,
            lines,
            scans: self.tour.scans(t),
            survey: self.tour.survey(t),
            map: self.minimap(t),
            manoeuvre: self.tour.manoeuvre(t),
            target: self.target(t),
            alert: self.tour.view(t).scene.map_or(0.0, |s| s.shot.alert as f32),
            notice: None,
        };
        self.update_incidents(t);
        self.service_repairs(t);
        self.update_console(t);
    }

    /// The next jump's star on the frame at `t`, while it is marked and in
    /// view.
    fn target(&self, t: f64) -> Option<Target> {
        let view = self.tour.view(t);
        if view.target_shown <= 0.0 || view.warp > 0.3 {
            return None;
        }
        let right = cross(view.forward, view.up);
        let z = dot(view.target, view.forward);
        if z < 0.05 {
            return None;
        }
        let x = dot(view.target, right) / (z * view.half_fov);
        let y = dot(view.target, view.up) / (z * view.half_fov);
        if x.abs() > 0.9 || y.abs() > 0.9 {
            return None;
        }
        let half = SIZE as f64 / 2.0;
        // Locked on once the alignment is done.
        let lock = match self.tour.manoeuvre(t) {
            Some(m) if m.drive > 0 => smoothstep(0.0, tour::SPOOL_STAGES[1], m.progress as f64),
            _ => 0.0,
        };
        Some(Target {
            at: [(half * (1.0 + x)) as f32, (half * (1.0 - y)) as f32],
            label: format!(
                "{} · {:.1} ly",
                self.tour.next.star.name, self.tour.next.distance
            ),
            shown: view.target_shown as f32,
            lock: if matches!(self.tour.segment(t).0.phase, Phase::Jump(_)) {
                1.0
            } else {
                lock as f32
            },
        })
    }

    /// The labels on the worlds in view at `t`, clear of the HUD's panels
    /// that are up (where `idle.slint` puts them, in the frame's pixels).
    fn update_labels(&mut self, t: f64) {
        let hud = &self.hud;
        let mut panels = Vec::new();
        if !hud.title.is_empty() {
            panels.push([260.0, 84.0, 820.0, 126.0]);
        }
        // The scanner or the manoeuvre under way: a half-height panel, the
        // burn's readout taller and the drive's taller still.
        let tall = match &hud.manoeuvre {
            _ if !hud.scans.is_empty() => Some(98.0),
            Some(m) if m.drive > 0 || m.dock => Some(224.0),
            Some(m) if m.dv > 0.0 => Some(196.0),
            Some(_) => Some(98.0),
            None => None,
        };
        if let Some(height) = tall {
            panels.push([240.0, 150.0, 840.0, 150.0 + height]);
        }
        // The next jump's star, its brackets and its name.
        if let Some(target) = &hud.target {
            let [x, y] = target.at.map(f64::from);
            panels.push([x - 130.0, y - 50.0, x + 130.0, y + 80.0]);
        }
        if hud.map.is_some() {
            panels.push([780.0, 415.0, 1030.0, 665.0]);
        }
        // An incident's transmission, at the left.
        if self.hud.notice.is_some() {
            panels.push([50.0, 440.0, 300.0, 640.0]);
        }
        // The now-playing card, when a track is up.
        panels.push([170.0, 784.0, 910.0, 920.0]);
        let view = self.view(t);
        self.labels = if view.inside {
            Vec::new()
        } else if view.deep {
            encounter_label(&view, &panels).into_iter().collect()
        } else {
            labels::labels(&self.tour.system, &view, t, &panels)
        };
    }

    /// The minimap at `t`: none during the jump, between systems.
    fn minimap(&self, t: f64) -> Option<Minimap> {
        if self.tour.segment(t).0.phase.deep() {
            return None;
        }
        let current = self.tour.current(t);
        let system = &self.tour.system;
        // Distances from the star on a log scale, so close-in planets and
        // the outermost giant all fit; the plane seen tilted, from above.
        let inner = system.planets.first().map_or(1.0, |p| p.orbit) * 0.5;
        let outer = system.planets.last().map_or(1.0, |p| p.orbit);
        let reach = (MAP as f64) * 0.45;
        let squash = 0.5;
        let radius = |r: f64| reach * (1.0 + r / inner).ln() / (1.0 + outer / inner).ln();
        let centre = MAP as f64 / 2.0;
        let project = |p: [f64; 3]| -> [f32; 2] {
            let flat = (p[0] * p[0] + p[2] * p[2]).sqrt();
            let r = radius(flat);
            let k = if flat > 1e-9 { r / flat } else { 0.0 };
            [
                (centre + p[0] * k) as f32,
                (centre + p[2] * k * squash - p[1] * k * 0.8) as f32,
            ]
        };
        let mut orbits = String::new();
        for planet in &system.planets {
            let (rx, ry) = (radius(planet.orbit), radius(planet.orbit) * squash);
            orbits.push_str(&format!(
                "M {:.1} {:.1} A {rx:.1} {ry:.1} 0 1 0 {:.1} {:.1} A {rx:.1} {ry:.1} 0 1 0 {:.1} {:.1} ",
                centre - rx,
                centre,
                centre + rx,
                centre,
                centre - rx,
                centre
            ));
        }
        let mut route = String::new();
        for (i, p) in self.tour.route(t).into_iter().enumerate() {
            let [x, y] = project(p);
            route.push_str(&format!(
                "{} {x:.1} {y:.1} ",
                if i == 0 { "M" } else { "L" }
            ));
        }
        let to_srgb = |c: [f32; 3], lift: f32| {
            c.map(|v| ((v.max(0.0).powf(1.0 / 2.2) * (1.0 - lift) + lift) * 255.0) as u8)
        };
        let mut dots = vec![MapDot {
            at: [centre as f32; 2],
            size: match system.star.kind {
                system::StarKind::RedGiant => 14.0,
                system::StarKind::WhiteDwarf => 5.0,
                system::StarKind::Main => 9.0,
            },
            colour: to_srgb(system.star.light.map(|c| c.min(1.0)), 0.3),
            label: String::new(),
            current: false,
        }];
        let visited: Vec<usize> = self.tour.itinerary.iter().map(|s| s.planet).collect();
        for (i, planet) in system.planets.iter().enumerate() {
            let colour = match planet.kind {
                Kind::Terran | Kind::Ocean | Kind::Jungle | Kind::Eyeball => [0.12, 0.3, 0.6],
                _ => planet.palette[0],
            };
            dots.push(MapDot {
                at: project(planet.position(t)),
                size: if planet.kind.is_giant() { 8.0 } else { 5.0 },
                colour: to_srgb(colour, 0.25),
                // The planet's letter, for the stops.
                label: if visited.contains(&i) {
                    planet.name.rsplit(' ').next().unwrap_or("").to_string()
                } else {
                    String::new()
                },
                current: current.is_some_and(|c| c.planet == i),
            });
        }
        // The jump point, out at the edge, once the ship is making for it.
        if matches!(
            self.tour.segment(t).0.phase,
            Phase::Depart(_) | Phase::Cruise { .. } | Phase::Spool
        ) {
            dots.push(MapDot {
                at: project(self.tour.jump_point()),
                size: 6.0,
                colour: [235, 240, 255],
                label: "JUMP".into(),
                current: true,
            });
        }
        Some(Minimap {
            orbits,
            route,
            dots,
            ship: project(self.tour.view(t).position),
        })
    }

    fn render(&mut self, gpu: &Graphics, seconds: f64, frame: usize) -> Result<()> {
        let mut t = self.catch_up(seconds);
        if !self.pending.is_empty() {
            for order in std::mem::take(&mut self.pending) {
                self.obey(order, seconds);
            }
            t = self.catch_up(seconds);
            self.hud_at = f64::NEG_INFINITY;
        }
        self.stream(t);
        self.draw(gpu, t, frame);
        self.update_hud(t);
        self.update_labels(t);
        Ok(())
    }
}

impl Visual for Solar {
    fn render(&mut self, gpu: &Graphics, seconds: f64, _seed: u64, frame: usize) -> Result<()> {
        Solar::render(self, gpu, seconds, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    fn hud(&self) -> Option<Hud> {
        Some(self.hud.clone())
    }

    fn labels(&self) -> Vec<labels::BodyLabel> {
        self.labels.clone()
    }

    fn console(&self) -> Option<console::Console> {
        Some(self.console.clone())
    }

    fn order(&mut self, page: i32, code: i32) {
        if let Some(order) = console::Order::from_ui(page, code, self.visit) {
            tracing::info!(?order, "an order from the ship's terminal");
            self.pending.push(order);
        }
    }
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
fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Which stage of the drive's preparation `progress` (`Params::spool`'s x) is
/// in: 0 aligning, before its start-up shows, then each of the pipelines
/// `fs_spool` has a stage (`STAGE`).
fn spool_stage(progress: f32) -> usize {
    tour::SPOOL_STAGES[1..]
        .iter()
        .filter(|&&s| progress >= s as f32)
        .count()
}

#[cfg(test)]
pub(crate) mod tests;
