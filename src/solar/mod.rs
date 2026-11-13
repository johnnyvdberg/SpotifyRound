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

pub mod labels;
pub mod scan;
pub mod stream;
pub mod system;
pub mod tour;
pub mod voyage;

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;
use stream::{Key, Streamer, sampler_entry, texture_entry, uniform_entry};
use system::{Kind, Planet, Stop, System, equatorial_to_system};
use tour::{Phase, Tour, View, add, normalize, scale, sub};

/// Size of the rendered frame.
pub const SIZE: u32 = 1080;
const MAX_BODIES: usize = 16;
/// Cloud layers stacked over the deck in low orbit: each a pass of about
/// 330 instructions where clouds are, so one (a second added little).
const CLOUD_LAYERS: u32 = 1;
const SLOTS: usize = crate::visuals::SLOTS;

#[repr(C)]
#[derive(Clone, Copy, Default, bytemuck::Pod, bytemuck::Zeroable)]
struct Body {
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
struct Params {
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
    target: [f32; 4],
    target_colour: [f32; 4],
    /// The drive spooling up: x how far through its preparation, y how much
    /// of its start-up effect shows.
    spool: [f32; 4],
    /// The stopover between legs of the jump, or what is seen inside one
    /// (`EV_*` in `encounter.wgsl`).
    ev: [[f32; 4]; EV_ROWS],
    bodies: [Body; MAX_BODIES],
}

/// Rows of the stopover's parameters (`Params::ev`).
const EV_ROWS: usize = 12;
/// Rocks or debris drawn round the ship in a stopover (`vs_rock`).
const ROCKS: u32 = 260;
/// Vertices of the ship's own effects' quad (`vs_overlay`).
const OVERLAY_VERTICES: u32 = 3;

/// Motes of dust streaking past in a burn (`DUST` in the shader), the sides
/// of the engines' glow round the rim (`RIM_SIDES`), how fast the dust
/// drifts (box widths a second) and how long its streaks grow at full
/// throttle (in box widths).
const DUST_MOTES: u32 = 160;
const RIM_SIDES: u32 = 48;
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
/// The inscribed depth mask's draw: `MASK_SIDES` triangles in `frame.wgsl`.
const MASK_VERTICES: u32 = 64 * 3;
/// The ring's draw: `RING_SIDES` quads in `frame.wgsl`.
const RING_VERTICES: u32 = 32 * 6;
/// The limb's annulus: `LIMB_SIDES` sides of two quads in `frame.wgsl`.
const LIMB_RING_VERTICES: u32 = 128 * 2 * 6;
/// The drop-out's annulus and disc: `DROP_SIDES` sides in `frame.wgsl`.
const DROP_SIDES: u32 = 64;
/// The star's glare polygon (`vs_glare`): a fan of `GLARE_SIDES` 32 sides,
/// drawn as a list of triangles.
const GLARE_VERTICES: u32 = (32 - 2) * 3;
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
const GRID_STRIDE: u32 = 129;
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
    /// The ship's alarm between the stars, 0..1 (`voyage::Shot::alert`).
    pub alert: f32,
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
    pipelines: std::collections::HashMap<String, (String, wgpu::RenderPipeline)>,
    /// A drive to use for every jump instead of each system's own, for the
    /// screenshots.
    drive_override: Option<u32>,
    layout: wgpu::BindGroupLayout,
    uniforms: wgpu::Buffer,
    /// The meshes' triangles, every grid's (`grid_indices`).
    grid: wgpu::Buffer,
    sampler: wgpu::Sampler,
    bind_group: Option<wgpu::BindGroup>,
    /// Each ringed planet's ring density by radius, a row per planet, and the
    /// system it was painted for.
    rings: wgpu::Texture,
    rings_for: u64,
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
fn tour_for(base: u64, visit: u64) -> Tour {
    let passage =
        |visit: u64, next: &System| voyage::Passage::plan(base, visit, tour::drive_to(next).0);
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
    let arrival = match visit {
        0 => tour::drive_to(&system).0,
        _ => passage(visit - 1, &system).arrival_look(),
    };
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

    pub(crate) fn with_seed(gpu: &Graphics, seed: u64) -> Self {
        let device = &gpu.device;
        let streamer = Streamer::new(gpu);
        let d2 = wgpu::TextureViewDimension::D2;
        let array = wgpu::TextureViewDimension::D2Array;
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar frame"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, d2),
                sampler_entry(2),
                texture_entry(3, d2),
                texture_entry(4, d2),
                texture_entry(5, d2),
                texture_entry(6, array),
                texture_entry(7, array),
                texture_entry(8, d2),
                texture_entry(9, d2),
                texture_entry(10, d2),
                texture_entry(11, array),
                texture_entry(12, array),
                texture_entry(13, d2),
                texture_entry(14, wgpu::TextureViewDimension::Cube),
                texture_entry(15, wgpu::TextureViewDimension::Cube),
            ],
        });
        let mut source = String::from(include_str!("noise.wgsl"));
        source.push_str(crate::noise3d::WGSL);
        source.push_str(include_str!("frame.wgsl"));
        source.push_str(include_str!("encounter.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar frame"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar frame"),
            bind_group_layouts: &[Some(&layout), Some(&streamer.noise().layout)],
            immediate_size: 0,
        });
        // Every frame pipeline tests the depth mask (see `draw`); the mask's
        // own pipeline writes it and nothing else. Only the meshes cull.
        let make_culled = |label: &str,
                           vs: &str,
                           fs: &str,
                           blend: Option<wgpu::BlendState>,
                           constants: &[(&str, f64)],
                           write_depth: Option<bool>,
                           writes: wgpu::ColorWrites,
                           strict: bool,
                           cull_mode: Option<wgpu::Face>| {
            let options = wgpu::PipelineCompilationOptions {
                constants,
                ..Default::default()
            };
            let made = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers: &[],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8UnormSrgb,
                        blend,
                        write_mask: writes,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState {
                    cull_mode,
                    ..Default::default()
                },
                // None: drawn outside the frame's pass (the starfield bake).
                depth_stencil: write_depth.map(|write| wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write),
                    // Strictly in front for passes that draw only off their
                    // own disc (the limb): its mask then rejects the disc.
                    depth_compare: Some(if strict {
                        wgpu::CompareFunction::Less
                    } else {
                        wgpu::CompareFunction::LessEqual
                    }),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            });
            (label.to_string(), made)
        };
        let make = |label: &str,
                    vs: &str,
                    fs: &str,
                    blend: Option<wgpu::BlendState>,
                    constants: &[(&str, f64)],
                    write_depth: Option<bool>,
                    writes: wgpu::ColorWrites,
                    strict: bool| {
            let depth = write_depth;
            make_culled(label, vs, fs, blend, constants, depth, writes, strict, None)
        };
        // A mapped world's mesh: its back faces culled (counter-clockwise
        // seen from outside, see `grid_indices`).
        let globe = |label: &str,
                     fs: &str,
                     blend: Option<wgpu::BlendState>,
                     constants: &[(&str, f64)],
                     writes: wgpu::ColorWrites| {
            let (vs, back) = ("vs_globe", Some(wgpu::Face::Back));
            let depth = Some(false);
            make_culled(label, vs, fs, blend, constants, depth, writes, false, back)
        };
        let pipeline = |label: &str,
                        vs: &str,
                        fs: &str,
                        blend: Option<wgpu::BlendState>,
                        constants: &[(&str, f64)]| {
            make(
                label,
                vs,
                fs,
                blend,
                constants,
                Some(false),
                wgpu::ColorWrites::ALL,
                false,
            )
        };
        let over = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let add = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let multiply = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Zero,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        // Twice the product: the shader outputs half the factor, so the 8-bit
        // target's clamp to 1 still lets it brighten (up to 2x) as well as
        // darken.
        let double_multiply = Some(wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Dst,
                dst_factor: wgpu::BlendFactor::Src,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        });
        let mut pipelines = std::collections::HashMap::new();
        // The depth mask: each body's solid disc, at its distance rank.
        let none = wgpu::ColorWrites::empty();
        pipelines.insert(
            "mask".to_string(),
            make(
                "Solar mask",
                "vs_body",
                "fs_mask",
                None,
                &[],
                Some(true),
                none,
                false,
            ),
        );
        pipelines.insert(
            "maskinscribed".to_string(),
            make(
                "Solar mask inscribed",
                "vs_mask_inscribed",
                "fs_mask_inscribed",
                None,
                &[],
                Some(true),
                none,
                false,
            ),
        );
        // For the Pi estimate only: what survives the mask, shaded by nothing.
        pipelines.insert(
            "probe".to_string(),
            make(
                "Solar probe",
                "vs_body",
                "fs_probe",
                None,
                &[],
                Some(false),
                none,
                false,
            ),
        );
        pipelines.insert(
            "probestrict".to_string(),
            make(
                "Solar probe strict",
                "vs_body",
                "fs_probe",
                None,
                &[],
                Some(false),
                none,
                true,
            ),
        );
        pipelines.insert(
            "probefull".to_string(),
            make(
                "Solar probe full",
                "vs_background",
                "fs_probe_full",
                None,
                &[],
                Some(false),
                none,
                false,
            ),
        );
        // The starfield, baked once a system, and the background reading it.
        pipelines.insert(
            "bake".to_string(),
            make(
                "Solar starfield bake",
                "vs_bake",
                "fs_bake_stars",
                None,
                &[],
                None,
                wgpu::ColorWrites::ALL,
                false,
            ),
        );
        pipelines.insert(
            "bakesky".to_string(),
            make(
                "Solar sky cube bake",
                "vs_bake",
                "fs_bake_sky",
                None,
                &[],
                None,
                wgpu::ColorWrites::ALL,
                false,
            ),
        );
        // The baked background reads the cube and nothing else, the star's
        // glare added over it round the star.
        pipelines.insert(
            "spacebaked".to_string(),
            pipeline(
                "Solar space baked",
                "vs_background",
                "fs_space_baked",
                None,
                &[],
            ),
        );
        for (name, label, vs) in [
            ("glare", "Solar glare", "vs_glare"),
            ("glarefull", "Solar glare full", "vs_glare_full"),
        ] {
            pipelines.insert(name.to_string(), pipeline(label, vs, "fs_glare", add, &[]));
        }
        let (label, vs, fs) = ("Solar probe glare", "vs_glare", "fs_probe_glare");
        let made = make(label, vs, fs, None, &[], Some(false), none, false);
        pipelines.insert("probeglare".to_string(), made);
        pipelines.insert(
            "space".to_string(),
            pipeline("Solar space", "vs_background", "fs_space", None, &[]),
        );
        pipelines.insert(
            "dust".to_string(),
            pipeline("Solar burn dust", "vs_dust", "fs_dust", add, &[]),
        );
        pipelines.insert(
            "rim".to_string(),
            pipeline("Solar burn rim", "vs_rim", "fs_rim", add, &[]),
        );
        // The baked sky with the next jump's star marked, out to the jump
        // point.
        pipelines.insert(
            "target".to_string(),
            pipeline(
                "Solar space target",
                "vs_background",
                "fs_space_target",
                None,
                &[],
            ),
        );
        // Each drive's light over the whole frame, and with the flare at the
        // heading for the jump's last second.
        for (k, name) in voyage::LOOKS.iter().enumerate() {
            let k = k as u32 + 1;
            // Its start-up at the jump point, over the sky, rising into its
            // light: the drives' only, a gate or wormhole throwing the ship
            // on itself.
            if (k as usize) <= tour::DRIVES.len() {
                let constants = [("WARP", k as f64)];
                let made = pipeline(
                    &format!("Solar spool {name}"),
                    "vs_background",
                    "fs_spool",
                    None,
                    &constants,
                );
                pipelines.insert(format!("spool{k}"), made);
            }
            for (key, label, flare) in [
                (format!("warp{k}"), format!("Solar warp {name}"), 0.0),
                (
                    format!("warpflare{k}"),
                    format!("Solar warp flare {name}"),
                    1.0,
                ),
            ] {
                let constants = [("WARP", k as f64), ("FLARE", flare)];
                let made = pipeline(&label, "vs_background", "fs_warp", None, &constants);
                pipelines.insert(key, made);
            }
            // Its light torn open over the next system as the jump drops
            // out: ahead of the front opaque, before everything else, and
            // writing the depth so nothing it hides is shaded; behind the
            // front blended over everything drawn.
            let constants = [("WARP", k as f64)];
            let (label, all) = (format!("Solar drop ahead {name}"), wgpu::ColorWrites::ALL);
            let ahead = make(
                &label,
                "vs_drop_ahead",
                "fs_drop_ahead",
                None,
                &constants,
                Some(true),
                all,
                false,
            );
            pipelines.insert(format!("dropahead{k}"), ahead);
            pipelines.insert(
                format!("dropout{k}"),
                pipeline(
                    &format!("Solar drop out {name}"),
                    "vs_drop_behind",
                    "fs_drop_out",
                    over,
                    &constants,
                ),
            );
        }
        // Between the legs of a jump: each stopover's own draw over the sky
        // (`fs_event`, or in place of it, `fs_event_sky`), rocks or debris
        // round the ship for some; what is seen inside a leg over the
        // drive's light (`fs_flux`); and the ship's own effects over
        // everything (`fs_overlay`): the alarm, hits on the shield and
        // flashes.
        for event in voyage::Event::ALL {
            let constants = [("EVENT", event.id() as f64)];
            let (key, label) = (
                format!("event{}", event.id()),
                format!("Solar event {}", event.slug()),
            );
            let (vs, fs, blend) = if event.replaces_sky() {
                ("vs_background", "fs_event_sky", None)
            } else {
                ("vs_background", "fs_event", over)
            };
            pipelines.insert(key, pipeline(&label, vs, fs, blend, &constants));
            if event.rocks() {
                let label = format!("Solar rocks {}", event.slug());
                let all = wgpu::ColorWrites::ALL;
                let made = make(
                    &label,
                    "vs_rock",
                    "fs_rock",
                    over,
                    &constants,
                    Some(true),
                    all,
                    false,
                );
                pipelines.insert(format!("rocks{}", event.id()), made);
            }
        }
        for flux in voyage::Flux::ALL {
            let constants = [("EVENT", flux.id() as f64)];
            let label = format!("Solar flux {}", flux.slug());
            pipelines.insert(
                format!("flux{}", flux.id()),
                pipeline(&label, "vs_background", "fs_flux", add, &constants),
            );
        }
        pipelines.insert(
            "overlay".to_string(),
            make(
                "Solar ship overlay",
                "vs_overlay",
                "fs_overlay",
                over,
                &[],
                Some(false),
                wgpu::ColorWrites::ALL,
                false,
            ),
        );
        for (name, entry, blend) in [
            ("eclipse", "fs_eclipse", multiply),
            ("ringshadow", "fs_ring_shadow", multiply),
        ] {
            pipelines.insert(
                name.to_string(),
                pipeline(&format!("Solar {name}"), "vs_body", entry, blend, &[]),
            );
        }
        // The star, worlds whose maps are not in and moons as meshes too
        // (`vs_globe`, `GLOBE`), named `globe…` so they are drawn as one.
        let all = wgpu::ColorWrites::ALL;
        const STAR_GLOBE: [(&str, f64); 2] = [("GLOBE", 3.0), ("AIR", 0.0)];
        pipelines.insert(
            "globestar".to_string(),
            globe("Solar globestar", "fs_globe_star", over, &STAR_GLOBE, all),
        );
        for kind in Kind::ALL {
            let ringed = matches!(kind, Kind::GasGiant | Kind::IceGiant);
            for rings in [false, true].into_iter().filter(|&r| ringed || !r) {
                let look = FlatLook::of(kind, rings);
                let name = look.pipeline();
                if pipelines.contains_key(&name) {
                    continue;
                }
                let label = format!("Solar {name}");
                let made = globe(&label, "fs_globe_flat", over, &look.constants(), all);
                pipelines.insert(name, made);
            }
        }
        for slot in 0..2 {
            for air in [false, true] {
                let name = moon_pipeline(slot, air);
                let air = f64::from(u8::from(air));
                let constants = [("GLOBE", 2.0), ("SLOT", slot as f64), ("AIR", air)];
                let label = format!("Solar {name}");
                let made = globe(&label, "fs_globe_moon", over, &constants, all);
                pipelines.insert(name, made);
            }
        }
        // The rings, drawn as an annulus (`vs_ring`).
        pipelines.insert(
            "ring".to_string(),
            pipeline("Solar ring", "vs_ring", "fs_ring_annulus", over, &[]),
        );
        // The limb, off its own disc only (a strict depth test): an annulus
        // lit at its vertices for a body wholly ahead, else traced.
        pipelines.insert(
            "limb".to_string(),
            make(
                "Solar limb",
                "vs_body",
                "fs_limb",
                add,
                &[],
                Some(false),
                wgpu::ColorWrites::ALL,
                true,
            ),
        );
        pipelines.insert(
            "limbring".to_string(),
            make(
                "Solar limb ring",
                "vs_limb_ring",
                "fs_limb_ring",
                add,
                &[],
                Some(false),
                wgpu::ColorWrites::ALL,
                true,
            ),
        );
        pipelines.insert(
            "probelimbring".to_string(),
            make(
                "Solar probe limb ring",
                "vs_limb_ring",
                "fs_probe",
                None,
                &[],
                Some(false),
                none,
                true,
            ),
        );
        pipelines.insert(
            "probeglobe".to_string(),
            globe("Solar probe globe", "fs_probe_globe", None, &[], none),
        );
        let mut seen = std::collections::HashSet::new();
        for slot in 0..2 {
            // A mapped world from afar as a mesh (`fs_globe`), one pipeline
            // for each set of features a kind has: only its own code, in as
            // few registers as it needs.
            let looks = Kind::ALL.into_iter().flat_map(|k| {
                let rings = matches!(k, Kind::GasGiant | Kind::IceGiant);
                [false, true]
                    .into_iter()
                    .filter(move |&r| rings || !r)
                    .map(move |r| SurfaceLook::of(k, r))
            });
            for look in looks {
                let mesh = look.globe(slot);
                if !seen.insert(mesh.clone()) {
                    continue;
                }
                let mut constants = vec![("SLOT", slot as f64)];
                constants.extend(look.constants());
                let label = format!("Solar {mesh}");
                let all = wgpu::ColorWrites::ALL;
                pipelines.insert(mesh, globe(&label, "fs_globe", over, &constants, all));
                if look.sea {
                    // A sea's added light, a pass of its own.
                    let glow = look.globe_glow(slot);
                    let label = format!("Solar {glow}");
                    let glow_pipeline = globe(&label, "fs_globe_glow", add, &constants, all);
                    pipelines.insert(glow, glow_pipeline);
                }
            }
            // The ground close up, its magnified map lookups made crisp.
            pipelines.insert(
                format!("grounddetail{slot}"),
                pipeline(
                    &format!("Solar ground detail {slot}"),
                    "vs_body",
                    "fs_ground",
                    over,
                    &[("SLOT", slot as f64), ("DETAIL", 1.0)],
                ),
            );
            // The added light with the sea's glint, for worlds with seas.
            pipelines.insert(
                format!("glowsea{slot}"),
                pipeline(
                    &format!("Solar glow sea {slot}"),
                    "vs_body",
                    "fs_glow",
                    add,
                    &[("SLOT", slot as f64), ("SEA", 1.0)],
                ),
            );
            for (name, entry, blend) in [
                ("ground", "fs_ground", over),
                ("detail", "fs_detail", double_multiply),
                ("clouds", "fs_clouds", over),
                ("glow", "fs_glow", add),
            ] {
                pipelines.insert(
                    format!("{name}{slot}"),
                    pipeline(
                        &format!("Solar {name} {slot}"),
                        "vs_body",
                        entry,
                        blend,
                        &[("SLOT", slot as f64)],
                    ),
                );
            }
        }
        // The stacked cloud layers drawn in low orbit, per slot.
        for slot in 0..2 {
            for layer in 1..=CLOUD_LAYERS {
                pipelines.insert(
                    format!("cloudlayer{slot}x{layer}"),
                    pipeline(
                        &format!("Solar cloud layer {slot} {layer}"),
                        "vs_body",
                        "fs_cloud_layer",
                        over,
                        &[("SLOT", slot as f64), ("LAYER", layer as f64)],
                    ),
                );
            }
        }
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar uniforms"),
            size: std::mem::size_of::<Params>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let grid = {
            use wgpu::util::DeviceExt;
            device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("Solar mesh grids"),
                contents: bytemuck::cast_slice(&grid_indices()),
                usage: wgpu::BufferUsages::INDEX,
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Solar maps"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // No anisotropy unless `SOLAR_ANISO` asks for it: the traced
            // shaders' coordinates jump a whole turn where the longitude
            // wraps, and the V3D sized its footprint from that (a dotted seam
            // down every planet; about 2 ms in a close orbit). The meshes
            // never wrap inside a triangle, so they can use it.
            anisotropy_clamp: map_anisotropy(),
            ..Default::default()
        });
        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar frame"),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                });
                // Drawn through an sRGB view: the passes blend in linear light
                // and the frame Slint reads is encoded.
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                    ..Default::default()
                });
                Frame { texture, view }
            })
            .collect();
        let depth = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("Solar depth mask"),
                size: wgpu::Extent3d {
                    width: SIZE,
                    height: SIZE,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: DEPTH,
                usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                view_formats: &[],
            })
            .create_view(&wgpu::TextureViewDescriptor::default());
        let tour = tour_for(seed, 0);
        let rings = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Solar ring profiles"),
            size: wgpu::Extent3d {
                width: RING_TEXELS,
                height: RING_ROWS,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        Self {
            pipelines,
            drive_override: None,
            layout,
            uniforms,
            grid,
            sampler,
            bind_group: None,
            rings,
            rings_for: u64::MAX,
            bound: (u64::MAX, usize::MAX, usize::MAX),
            starfields: [0, 1].map(|_| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar starfield"),
                    size: wgpu::Extent3d {
                        width: STARFIELD_FACE,
                        height: STARFIELD_FACE,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                })
            }),
            sky_cubes: [0, 1].map(|_| {
                device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Solar sky cube"),
                    size: wgpu::Extent3d {
                        width: SKY_CUBE_FACE,
                        height: SKY_CUBE_FACE,
                        depth_or_array_layers: 6,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[wgpu::TextureFormat::Rgba8UnormSrgb],
                })
            }),
            starfield_owner: [None, None],
            frames,
            depth,
            streamer,
            base_seed: seed,
            visit: 0,
            visit_start: 0.0,
            tour,
            hud: Hud::default(),
            hud_at: f64::NEG_INFINITY,
            labels: Vec::new(),
            dust: [0.0; 3],
            dust_at: None,
            view_override: None,
            showcase: None,
            #[cfg(test)]
            drawn: Default::default(),
        }
    }

    /// Move the tour on to the visit `seconds` falls in.
    fn catch_up(&mut self, seconds: f64) -> f64 {
        if seconds < self.visit_start {
            self.visit_start = seconds;
        }
        while seconds - self.visit_start >= self.tour.total {
            self.visit_start += self.tour.total;
            self.visit += 1;
            self.tour = tour_for(self.base_seed, self.visit);
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

    fn bind(&mut self, gpu: &Graphics) {
        if self.rings_for != self.tour.system.seed {
            paint_rings(gpu, &self.rings, &self.tour.system);
            self.rings_for = self.tour.system.seed;
        }
        let sky = self.streamer.sky_ready(self.tour.system.seed).unwrap_or(0);
        let stars = self.starfield_slot(self.tour.system.seed).unwrap_or(0);
        let wanted = (self.streamer.version, sky, stars);
        if self.bind_group.is_some() && self.bound == wanted {
            return;
        }
        self.bind_group = Some(self.group(gpu, sky, stars));
        self.bound = wanted;
    }

    /// The frame's bind group, with galaxy sky `sky` and baked starfield
    /// `stars`.
    fn group(&self, gpu: &Graphics, sky: usize, stars: usize) -> wgpu::BindGroup {
        let full = |t: &wgpu::Texture| t.create_view(&wgpu::TextureViewDescriptor::default());
        let array = |t: &wgpu::Texture| {
            t.create_view(&wgpu::TextureViewDescriptor {
                dimension: Some(wgpu::TextureViewDimension::D2Array),
                ..Default::default()
            })
        };
        let a = self.streamer.slot_maps(0);
        let b = self.streamer.slot_maps(1);
        let views = [
            full(self.streamer.sky(sky)),
            full(&a.day),
            full(&a.relief),
            full(&a.extra),
            array(&a.moon_day),
            array(&a.moon_relief),
            full(&b.day),
            full(&b.relief),
            full(&b.extra),
            array(&b.moon_day),
            array(&b.moon_relief),
        ];
        let mut entries = vec![
            wgpu::BindGroupEntry {
                binding: 0,
                resource: self.uniforms.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&views[0]),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&self.sampler),
            },
        ];
        for (i, view) in views.iter().enumerate().skip(1) {
            entries.push(wgpu::BindGroupEntry {
                binding: 2 + i as u32,
                resource: wgpu::BindingResource::TextureView(view),
            });
        }
        let rings = full(&self.rings);
        entries.push(wgpu::BindGroupEntry {
            binding: 13,
            resource: wgpu::BindingResource::TextureView(&rings),
        });
        // Read back through the sRGB view it was written through.
        let starfield = self.starfields[stars].create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 14,
            resource: wgpu::BindingResource::TextureView(&starfield),
        });
        let sky_cube = self.sky_cubes[stars].create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            dimension: Some(wgpu::TextureViewDimension::Cube),
            ..Default::default()
        });
        entries.push(wgpu::BindGroupEntry {
            binding: 15,
            resource: wgpu::BindingResource::TextureView(&sky_cube),
        });
        gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar frame"),
            layout: &self.layout,
            entries: &entries,
        })
    }

    /// The starfield slot finished for the system of `seed`, if any.
    fn starfield_slot(&self, seed: u64) -> Option<usize> {
        (0..2).find(|&k| self.starfield_owner[k] == Some((seed, STARFIELD_HEIGHT)))
    }

    /// Bake a band of the starfield of the system that needs one: the
    /// current one, or the next once its galaxy is in (during the jump).
    /// Into a slot neither system has finished, reading the galaxy map the
    /// streamer painted and drawing the stars a texel wide, so the frame's
    /// background is one texture read instead of three layers of hashed
    /// stars a pixel.
    fn bake_stars(&mut self, gpu: &Graphics, encoder: &mut wgpu::CommandEncoder) {
        let seeds = [self.tour.system.seed, self.tour.next.seed];
        let Some((seed, sky)) = seeds.iter().find_map(|&seed| {
            let done = self.starfield_slot(seed).is_some();
            let sky = self.streamer.sky_ready(seed)?;
            (!done).then_some((seed, sky))
        }) else {
            return;
        };
        let slot = match (0..2).find(|&k| self.starfield_owner[k].map(|o| o.0) == Some(seed)) {
            Some(k) => k,
            None => {
                // Not one either system still needs.
                let free = (0..2).find(|&k| match self.starfield_owner[k] {
                    None => true,
                    Some((s, _)) => !seeds.contains(&s),
                });
                let Some(k) = free else {
                    return;
                };
                self.starfield_owner[k] = Some((seed, 0));
                k
            }
        };
        let row = self.starfield_owner[slot].map_or(0, |o| o.1);
        let (face, top) = (row / STARFIELD_FACE, row % STARFIELD_FACE);
        let rows = STARFIELD_ROWS.min(STARFIELD_FACE - top);
        // The other slot's textures at bindings 14 and 15: this one's are
        // the targets.
        let group = self.group(gpu, sky, 1 - slot);
        if row == 0 {
            // The galaxy alone, small: all of it at once.
            for face in 0..6 {
                let view = self.sky_cubes[slot].create_view(&wgpu::TextureViewDescriptor {
                    format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                    dimension: Some(wgpu::TextureViewDimension::D2),
                    base_array_layer: face,
                    array_layer_count: Some(1),
                    ..Default::default()
                });
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Solar sky cube bake"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                });
                let (label, pipeline) = &self.pipelines["bakesky"];
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &group, &[]);
                pass.set_bind_group(1, &self.streamer.noise().group, &[]);
                let texels = (SKY_CUBE_FACE * SKY_CUBE_FACE) as u64;
                crate::gpu::counted_draw(&mut pass, label, texels, 0..3, face..face + 1);
            }
        }
        let view = self.starfields[slot].create_view(&wgpu::TextureViewDescriptor {
            format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: face,
            array_layer_count: Some(1),
            ..Default::default()
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar starfield bake"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: if top == 0 {
                            wgpu::LoadOp::Clear(wgpu::Color::BLACK)
                        } else {
                            wgpu::LoadOp::Load
                        },
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            let (label, pipeline) = &self.pipelines["bake"];
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            pass.set_scissor_rect(0, top, STARFIELD_FACE, rows);
            crate::gpu::counted_draw(
                &mut pass,
                label,
                (STARFIELD_FACE * rows) as u64,
                0..3,
                face..face + 1,
            );
        }
        self.starfield_owner[slot] = Some((seed, row + rows));
    }

    /// The camera at `t`, shaken by the engines through a burn.
    fn view(&self, t: f64) -> View {
        let mut view = self.view_override.unwrap_or_else(|| self.tour.view(t));
        let throttle = self.throttle(t);
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

    /// The shader's parameters at `t` into the visit, and the draws: which
    /// pipeline over which body, in order.
    fn params(&self, t: f64) -> (Params, Vec<(u32, String)>) {
        let view = self.view(t);
        let throttle = self.throttle(t);
        let system = &self.tour.system;
        let camera = view.position;
        let rel = |p: [f64; 3]| [p[0] - camera[0], p[1] - camera[1], p[2] - camera[2]];
        let f32x4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];

        let star_rel = rel([0.0; 3]);
        let hidden = view.warp > 0.3 || view.deep;
        let mut bodies: Vec<(f64, Body, Vec<String>)> = Vec::new();
        if !hidden {
            let mut star = Body {
                place: f32x4(star_rel, system.star.radius),
                // A star's look: -1, then its surface's cell scale, their
                // contrast and its limb darkening.
                look: [
                    -1.0,
                    system.star.surface[0],
                    system.star.surface[1],
                    system.star.surface[2],
                ],
                // Its own frame the world's (its mesh needs one; its
                // granules lie in it).
                rot: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                ..Default::default()
            };
            if let Some(rect) = rect(&view, star_rel, system.star.radius * 1.02) {
                star.rect = rect;
                // air.w: how far round the centre the body's draws reach.
                star.air[3] = (system.star.radius * 1.02) as f32;
                bodies.push((length(star_rel), star, vec!["globestar".into()]));
            }
            for (index, planet) in system.planets.iter().enumerate() {
                let centre = planet.position(t);
                let place = rel(centre);
                let slot = self
                    .streamer
                    .ready((system.seed, Stop::planet(index).key()));
                let mut body = Body {
                    place: f32x4(place, planet.radius),
                    rot: rotation(
                        planet,
                        spin_angle(planet, t, centre),
                        t / planet.day * 0.035,
                    ),
                    look: [
                        planet.kind as u32 as f32,
                        slot.map_or(-1.0, |s| s as f32),
                        -1.0,
                        night_mode(planet.kind),
                    ],
                    air: {
                        let a = planet.kind.air();
                        [a[0], a[1], a[2], 0.0]
                    },
                    colour: flat_colour(planet),
                    ..Default::default()
                };
                let mut bound = planet.radius * if body.air[0] > 0.0 { 1.08 } else { 1.0 };
                if let Some(rings) = &planet.rings {
                    // The ring profile's row is the planet's index.
                    body.ring = [
                        rings.inner as f32,
                        rings.outer as f32,
                        rings.opacity,
                        index as f32,
                    ];
                    body.ring_colour = [rings.colour[0], rings.colour[1], rings.colour[2], 1.0];
                    bound = planet.radius * rings.outer;
                }
                if let Some(rect) = rect(&view, place, bound) {
                    body.rect = rect;
                    body.air[3] = bound as f32;
                    let eclipsed = eclipsed(system, t, centre, planet.radius, Some(index));
                    let close = length(place) < planet.radius * 1.6;
                    let passes =
                        surface_passes(planet.kind, slot, eclipsed, planet.rings.is_some(), close);
                    bodies.push((length(place), body, passes));
                }
                // Moons: a moon the size of a planet in full once its own
                // maps are in, the others once their planet's are.
                for (m, moon) in planet.moons.iter().enumerate().take(stream::MOONS as usize) {
                    let stop = Stop {
                        planet: index,
                        moon: Some(m),
                    };
                    let world = if moon.world {
                        self.streamer.ready((system.seed, stop.key()))
                    } else {
                        None
                    };
                    if world.is_none() && slot.is_none() {
                        continue;
                    }
                    let at = system.centre(stop, t);
                    let place = rel(at);
                    let angle = moon.phase + t * std::f64::consts::TAU / moon.period;
                    let air = moon.kind.air();
                    let bound = moon.radius * if air[0] > 0.0 { 1.08 } else { 1.0 };
                    let Some(rect) = rect(&view, place, bound) else {
                        continue;
                    };
                    let eclipsed = eclipsed(system, t, at, moon.radius, None);
                    let (look, passes) = match (world, slot) {
                        (Some(w), _) => (
                            [
                                moon.kind as u32 as f32,
                                w as f32,
                                -1.0,
                                night_mode(moon.kind),
                            ],
                            surface_passes(
                                moon.kind,
                                Some(w),
                                eclipsed,
                                false,
                                length(place) < moon.radius * 1.6,
                            ),
                        ),
                        (None, Some(s)) => {
                            let mut passes = vec![moon_pipeline(s, air[0] > 0.0)];
                            if eclipsed {
                                passes.push("eclipse".into());
                            }
                            if air[0] > 0.0 {
                                passes.push("limb".into());
                            }
                            (
                                [
                                    moon.kind as u32 as f32,
                                    s as f32,
                                    m as f32,
                                    night_mode(moon.kind),
                                ],
                                passes,
                            )
                        }
                        (None, None) => continue,
                    };
                    bodies.push((
                        length(place),
                        Body {
                            place: f32x4(place, moon.radius),
                            // Locked to its planet, one face toward it; the
                            // clouds of a world drift on their own.
                            rot: rotation(
                                planet,
                                angle + std::f64::consts::PI,
                                t / moon.period * 0.2,
                            ),
                            look,
                            air: [air[0], air[1], air[2], bound as f32],
                            colour: [
                                moon.palette[0][0],
                                moon.palette[0][1],
                                moon.palette[0][2],
                                0.0,
                            ],
                            rect,
                            ..Default::default()
                        },
                        passes,
                    ));
                }
            }
        }
        // Far to near; the farthest dropped if there are too many.
        bodies.sort_by(|a, b| b.0.total_cmp(&a.0));
        if bodies.len() > MAX_BODIES {
            bodies.drain(0..bodies.len() - MAX_BODIES);
        }
        let count = bodies.len() as u32;
        let mut list = [Body::default(); MAX_BODIES];
        let mut draws = Vec::new();
        for (index, (slot, (_, body, passes))) in list.iter_mut().zip(bodies.iter()).enumerate() {
            *slot = *body;
            // The depth mask's value: nearer bodies (later, far to near) lower.
            slot.ring_colour[3] = 1.0 - (index as f32 + 1.0) / (count as f32 + 1.0);
            slot.shade = shadowers(&bodies, index, star_rel, system.star.radius);
            for pass in passes {
                draws.push((index as u32, pass.clone()));
            }
        }

        // How much of the star's disc the bodies leave in view.
        let star_distance = length(star_rel);
        let star_dir = normalize(star_rel);
        let star_angle = (system.star.radius / star_distance).min(1.0).asin();
        let mut visible = 1.0;
        for (distance, body, _) in &bodies {
            if body.look[0] < 0.0 || *distance > star_distance {
                continue;
            }
            let dir = normalize([
                body.place[0] as f64,
                body.place[1] as f64,
                body.place[2] as f64,
            ]);
            let separation = dot(dir, star_dir).clamp(-1.0, 1.0).acos();
            let radius = (body.place[3] as f64 / distance).min(1.0).asin();
            visible *= smoothstep(radius - star_angle, radius + star_angle, separation);
        }
        if hidden {
            visible = 0.0;
        }

        let sky = sky_rotation(system.sky);
        let right = cross(view.forward, view.up);
        let params = Params {
            frame: [
                SIZE as f32 / 2.0,
                view.half_fov as f32,
                (2.0 * view.half_fov / SIZE as f64) as f32,
                count as f32,
            ],
            right: f32x4(right, 0.0),
            up: f32x4(view.up, 0.0),
            forward: f32x4(view.forward, 0.0),
            star: f32x4(star_rel, system.star.radius),
            star_colour: [
                system.star.colour[0],
                system.star.colour[1],
                system.star.colour[2],
                visible as f32,
            ],
            light: [
                system.star.light[0],
                system.star.light[1],
                system.star.light[2],
                0.0,
            ],
            sky,
            warp: [
                view.warp as f32,
                view.flare as f32,
                view.reveal as f32,
                view.clock as f32,
            ],
            burn: [throttle, t as f32, 0.0, 0.0],
            dust: [
                self.dust[0] as f32,
                self.dust[1] as f32,
                self.dust[2] as f32,
                DUST_STREAK * throttle,
            ],
            motion: f32x4(self.motion(t), 0.0),
            target: f32x4(view.target, view.target_shown),
            target_colour: {
                let c = self.tour.next.star.colour;
                let top = c[0].max(c[1]).max(c[2]).max(1e-3);
                [c[0] / top, c[1] / top, c[2] / top, 0.0]
            },
            spool: [
                view.spool as f32,
                view.spool_light as f32,
                self.look(view.look) as f32,
                self.look(view.drop_look) as f32,
            ],
            ev: {
                let mut ev = encounter_rows(&view);
                if let Some((faction, index)) = self.showcase {
                    ev[9][1] = 1.0;
                    ev[9][2] = index as f32;
                    (ev[6][0], ev[6][1], ev[6][2]) = (faction as f32, faction as f32, 1e4);
                    ev[7][3] = -1.0;
                    (ev[10][0], ev[2][3], ev[3][3], ev[4][3]) = (0.0, 0.0, 0.0, 0.0);
                }
                ev
            },
            bodies: list,
        };
        // The meshes' windows and grids, from the frame's pixel size.
        let mut params = params;
        for (body, name) in &draws {
            if name.starts_with("globe") {
                let b = &mut params.bodies[*body as usize];
                (b.window, b.mesh) = globe_mesh(b, params.frame[2]);
                // The rings' shadow is not linear across a coarse cell.
                if b.ring[2] > 0.0 {
                    b.mesh[0] = b.mesh[0].max(RINGED_GRID as f32);
                }
            }
        }
        // The air past the limb of a body wholly ahead as an annulus.
        for (body, name) in &mut draws {
            if name == "limb" && wholly_ahead(&params.bodies[*body as usize], &params, 1.08) {
                *name = "limbring".into();
            }
        }
        (params, draws)
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

    fn draw(&mut self, gpu: &Graphics, t: f64, frame: usize) {
        self.drift_dust(t);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Solar encoder"),
            });
        self.streamer.advance(gpu, &mut encoder);
        self.bake_stars(gpu, &mut encoder);
        self.bind(gpu);
        let (params, draws) = self.params(t);
        let baked = self.starfield_slot(self.tour.system.seed).is_some()
            && std::env::var_os("SOLAR_PROCEDURAL_STARS").is_none();
        let (look, drop_look) = (params.spool[2] as u32, params.spool[3] as u32);
        let event = voyage::Event::ALL
            .into_iter()
            .find(|e| e.id() == params.ev[0][0] as u32);
        let deep = params.ev[1][3] > 0.5;
        // A stopover that replaces the sky, at full strength: nothing under it
        // to draw.
        let covered = event.is_some_and(|e| e.replaces_sky())
            && params.ev[10][1] >= 1.0
            && params.warp[2] >= 1.0;
        let background = if baked && params.spool[1] > 0.001 {
            // The drive starting up, and on into the jump's first seconds.
            format!("spool{}", self.drive().0)
        } else if params.warp[0] > 0.02 {
            let flare = if params.warp[1] > 0.0 { "flare" } else { "" };
            format!("warp{flare}{look}")
        } else if covered {
            String::new()
        } else if baked && params.target[3] > 0.0 {
            "target".to_string()
        } else if baked {
            "spacebaked".to_string()
        } else {
            "space".to_string()
        };
        // The star's glare over the baked background (`fs_space` draws its
        // own; the drives none).
        let glare = match background.as_str() {
            _ if deep => None,
            "spacebaked" | "target" => glare_pass(&params),
            b if b.starts_with("spool") && params.warp[0] < 0.3 => glare_pass(&params),
            _ => None,
        };
        #[cfg(test)]
        {
            *self.drawn.borrow_mut() = format!(
                "{background}{} {}",
                glare.map(|g| format!(" {g}")).unwrap_or_default(),
                draws
                    .iter()
                    .map(|(b, n)| format!("{b}:{n}"))
                    .collect::<Vec<_>>()
                    .join(" ")
            );
            if let Some(e) = event {
                let rocks = if e.rocks() {
                    format!(" rocks{}", e.id())
                } else {
                    String::new()
                };
                self.drawn
                    .borrow_mut()
                    .push_str(&format!(" event{}{rocks}", e.id()));
            }
            if params.warp[2] < 1.0 {
                let over = format!(" dropout{drop_look}");
                self.drawn.borrow_mut().push_str(&over);
            }
        }
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&params));
        let Some(group) = &self.bind_group else {
            return;
        };
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frames[frame].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                // Lives in the tile buffer only: never written to memory.
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, &self.streamer.noise().group, &[]);
            pass.set_index_buffer(self.grid.slice(..), wgpu::IndexFormat::Uint16);
            // First the depth mask: every body's solid disc at its rank, so
            // nothing a nearer body hides is shaded, the background included.
            let mut masked = Vec::new();
            for (body, _) in &draws {
                if !masked.contains(body) {
                    masked.push(*body);
                }
            }
            // When counting for the Pi estimate, a probe before each draw
            // counts the pixels that pass the mask: what the Pi shades.
            let counting = crate::gpu::counting();
            for body in &masked {
                if counting {
                    pass.set_pipeline(&self.pipelines["probe"].1);
                    crate::gpu::probe_draw(&mut pass, 0..BODY_VERTICES, *body..*body + 1);
                }
                // Wholly ahead, its silhouette's polygon; close by, traced.
                let b = &params.bodies[*body as usize];
                let (name, vertices) = if inscribable(b, &params) {
                    ("maskinscribed", MASK_VERTICES)
                } else {
                    ("mask", BODY_VERTICES)
                };
                let (label, mask) = &self.pipelines[name];
                pass.set_pipeline(mask);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, *body..*body + 1);
            }
            let full = (SIZE * SIZE) as u64;
            if counting {
                pass.set_pipeline(&self.pipelines["probefull"].1);
                crate::gpu::probe_draw(&mut pass, 0..3, 0..1);
            }
            // Dropping out of the jump: its light ahead of the front, in
            // front of everything and writing the depth, so the background
            // and the bodies it hides are not shaded.
            let dropping = params.warp[2] < 1.0;
            if dropping {
                let (label, pipeline) = &self.pipelines[&format!("dropahead{drop_look}")];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..DROP_SIDES * 6, 0..1);
            }
            if !background.is_empty() {
                let (label, pipeline) = &self.pipelines[&background];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
            }
            // A stopover: its own draw over the sky (or in place of it), and
            // the rocks round the ship, in front of everything.
            if let Some(e) = event.filter(|_| params.ev[10][1] > 0.001) {
                let (label, pipeline) = &self.pipelines[&format!("event{}", e.id())];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
                if e.rocks() {
                    let (label, pipeline) = &self.pipelines[&format!("rocks{}", e.id())];
                    pass.set_pipeline(pipeline);
                    crate::gpu::counted_draw(
                        &mut pass,
                        label,
                        (ROCKS * 300) as u64,
                        0..6,
                        0..ROCKS,
                    );
                }
            }
            // Something seen inside a leg, over the drive's light.
            let flux = params.ev[10][2] as u32;
            if flux > 0 && params.ev[10][3] > 0.001 && params.warp[0] > 0.5 {
                let (label, pipeline) = &self.pipelines[&format!("flux{flux}")];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..3, 0..1);
            }
            if let Some(name) = glare {
                let (vertices, probe) = match name {
                    "glare" => (GLARE_VERTICES, "probeglare"),
                    _ => (3, "probefull"),
                };
                if counting {
                    pass.set_pipeline(&self.pipelines[probe].1);
                    crate::gpu::probe_draw(&mut pass, 0..vertices, 0..1);
                }
                let (label, pipeline) = &self.pipelines[name];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, 0..1);
            }
            for (body, name) in &draws {
                // An eclipse only over where the shadows fall, if any does.
                let scissor = if name == "eclipse" {
                    match shadow_scissor(&params, *body as usize) {
                        Some(r) => Some(r),
                        None => continue,
                    }
                } else {
                    None
                };
                if let Some([x, y, w, h]) = scissor {
                    pass.set_scissor_rect(x, y, w, h);
                }
                let instances = *body..*body + 1;
                if name.starts_with("globe") {
                    // A mesh: its grid's triangles, probed with the same.
                    let indices = grid_range(params.bodies[*body as usize].mesh[0] as u32);
                    if counting {
                        pass.set_pipeline(&self.pipelines["probeglobe"].1);
                        let (probed, of) = (indices.clone(), instances.clone());
                        crate::gpu::probe_draw_indexed(&mut pass, probed, of);
                    }
                    let (label, pipeline) = &self.pipelines[name];
                    pass.set_pipeline(pipeline);
                    crate::gpu::counted_draw_indexed(&mut pass, label, 0, indices, instances);
                    continue;
                }
                let (vertices, probe, probed) = match name.as_str() {
                    "ring" => (RING_VERTICES, "probe", BODY_VERTICES),
                    "limb" => (BODY_VERTICES, "probestrict", BODY_VERTICES),
                    "limbring" => (LIMB_RING_VERTICES, "probelimbring", LIMB_RING_VERTICES),
                    _ => (BODY_VERTICES, "probe", BODY_VERTICES),
                };
                if counting {
                    pass.set_pipeline(&self.pipelines[probe].1);
                    crate::gpu::probe_draw(&mut pass, 0..probed, instances.clone());
                }
                let (label, pipeline) = &self.pipelines[name];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..vertices, instances);
                if scissor.is_some() {
                    pass.set_scissor_rect(0, 0, SIZE, SIZE);
                }
            }
            // An engine burn: the dust streaking past and the glow round
            // the rim, over everything.
            if params.burn[0] > 0.0 {
                let (label, pipeline) = &self.pipelines["dust"];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(
                    &mut pass,
                    label,
                    (DUST_MOTES * 400) as u64,
                    0..6,
                    0..DUST_MOTES,
                );
                let (label, pipeline) = &self.pipelines["rim"];
                pass.set_pipeline(pipeline);
                let ring = full - (full as f64 * 0.82 * 0.82 * std::f64::consts::FRAC_PI_4) as u64;
                crate::gpu::counted_draw(&mut pass, label, ring, 0..RIM_SIDES * 6, 0..1);
            }
            // And behind the front, over everything drawn.
            if dropping {
                let (label, pipeline) = &self.pipelines[&format!("dropout{drop_look}")];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, 0, 0..DROP_SIDES * 3, 0..1);
            }
            // The ship's own effects over everything: the alarm round the
            // rim, a hit on the shield, a flash.
            let alarm = params.ev[10][0]
                .max(params.ev[2][3])
                .max(params.ev[4][3])
                .max(params.ev[3][3]);
            if alarm > 0.01 {
                let (label, pipeline) = &self.pipelines["overlay"];
                pass.set_pipeline(pipeline);
                crate::gpu::counted_draw(&mut pass, label, full, 0..OVERLAY_VERTICES, 0..1);
            }
        }
        gpu.submit(encoder.finish());
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
        };
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
            Some(m) if m.drive > 0 => Some(224.0),
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
        // The now-playing card, when a track is up.
        panels.push([170.0, 784.0, 910.0, 920.0]);
        let view = self.view(t);
        self.labels = if view.deep {
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
        let t = self.catch_up(seconds);
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

/// Texels along a ring profile, and rows (one per planet).
const RING_TEXELS: u32 = 1024;
const RING_ROWS: u32 = 8;

/// Smooth 1D value noise in [0, 1], for the ring profiles.
fn noise1(x: f64, seed: u64) -> f64 {
    let hash = |i: i64| {
        let mut z = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let i = x.floor();
    let f = x - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    hash(i as i64) * (1.0 - u) + hash(i as i64 + 1) * u
}

/// Paint the ring density profiles of `system`'s ringed planets, one row
/// each: ringlets, gaps and a main division.
fn paint_rings(gpu: &Graphics, texture: &wgpu::Texture, system: &System) {
    let mut data = vec![0u8; (RING_TEXELS * RING_ROWS) as usize];
    for (row, planet) in system.planets.iter().enumerate().take(RING_ROWS as usize) {
        let Some(rings) = &planet.rings else {
            continue;
        };
        let seed = rings.seed.to_bits() as u64;
        for t in 0..RING_TEXELS as usize {
            let x = (t as f64 + 0.5) / RING_TEXELS as f64;
            let n1 = noise1(x * 40.0, seed);
            let n2 = noise1(x * 160.0, seed ^ 1);
            let n3 = noise1(x * 9.0, seed ^ 2);
            let division = 1.0 - (-((x - 0.62) / 0.025).powi(2)).exp() * 0.9;
            let edges = smoothstep(0.0, 0.04, x) * smoothstep(1.0, 0.9, x);
            let density = ((0.25 + 0.75 * n3)
                * (0.45 + 0.55 * n1)
                * (0.6 + 0.4 * n2)
                * division
                * edges
                * 1.4)
                .clamp(0.0, 1.0);
            data[row * RING_TEXELS as usize + t] = (density * 255.0).round() as u8;
        }
    }
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(RING_TEXELS),
            rows_per_image: Some(RING_ROWS),
        },
        wgpu::Extent3d {
            width: RING_TEXELS,
            height: RING_ROWS,
            depth_or_array_layers: 1,
        },
    );
}

/// Whether another body of `system` could pass in front of the star as seen
/// from a body of `radius` at `at`: only then is its eclipse pass drawn.
/// The label on a stopover's main object, where it is in view and clear of
/// the panels: its name and range, a leader up from its edge.
fn encounter_label(view: &View, panels: &[[f64; 4]]) -> Option<labels::BodyLabel> {
    let scene = view.scene.as_ref()?;
    if view.reveal < 1.0 || view.warp > 0.02 || scene.fade < 0.5 {
        return None;
    }
    let shot = &scene.shot;
    let o = shot.objects[0];
    let name = match scene.event {
        voyage::Event::Asteroids | voyage::Event::Nebula | voyage::Event::Remnant => return None,
        voyage::Event::HiveCube => "Hive cube".to_string(),
        voyage::Event::BlackHole => "Black hole".into(),
        voyage::Event::RoguePlanet => "Rogue planet".into(),
        voyage::Event::Pulsar => "Pulsar".into(),
        voyage::Event::Derelict => "Derelict station".into(),
        voyage::Event::Comet => "Comet nucleus".into(),
        voyage::Event::Leviathans => "Leviathan pod".into(),
        voyage::Event::Gate => "Star gate".into(),
        voyage::Event::Wormhole => "Wormhole".into(),
        voyage::Event::Battle => "Battle".into(),
        voyage::Event::PlasmaStorm => "Storm eye".into(),
        voyage::Event::DysonSwarm => "Dyson swarm".into(),
        voyage::Event::Crystal => "Crystal entity".into(),
    };
    let [x, y, z] = scene.basis;
    let rel = sub([o[0], o[1], o[2]], shot.ship);
    let world = add(add(scale(x, rel[0]), scale(y, rel[1])), scale(z, rel[2]));
    let depth = length(world);
    let size = match scene.event {
        voyage::Event::HiveCube => o[3] * 1.4,
        voyage::Event::Leviathans => 40.0,
        voyage::Event::Comet => 12.0,
        voyage::Event::Battle => 30.0,
        _ => o[3],
    };
    let ahead = dot(world, view.forward);
    if ahead <= size * 1.2 || depth <= size * 1.2 {
        return None;
    }
    let right = cross(view.forward, view.up);
    let half = SIZE as f64 / 2.0;
    let sx = dot(world, right) / ahead / view.half_fov;
    let sy = dot(world, view.up) / ahead / view.half_fov;
    let radius = (size / depth).asin().tan() / view.half_fov * half;
    // Too big to point at, or off the round frame.
    if radius > half * 0.55 || sx.hypot(sy) > 0.8 {
        return None;
    }
    let centre = [half + sx * half, half - sy * half];
    let left = centre[0] > half * 1.3;
    let lean = std::f64::consts::FRAC_1_SQRT_2 * radius;
    let at = [
        centre[0] + if left { -lean } else { lean },
        centre[1] - lean,
    ];
    if panels.iter().any(|p| {
        at[0] > p[0] - 20.0 && at[0] < p[2] + 20.0 && at[1] > p[1] - 70.0 && at[1] < p[3] + 20.0
    }) {
        return None;
    }
    Some(labels::BodyLabel {
        at: [at[0] as f32, at[1] as f32],
        left,
        name,
        distance: format!("{} km", tour::thousands((depth - size).max(0.0) * 10.0)),
    })
}

/// The stopover's parameters for the shaders (`EV_*` in `encounter.wgsl`):
/// the event, how far into it and its draw; the ship in its frame, and the
/// frame's axes; its objects, rotation and own numbers; the alarm, how much
/// of it shows and what is seen inside a leg; and where a hit lands.
fn encounter_rows(view: &View) -> [[f32; 4]; EV_ROWS] {
    let mut ev = [[0.0f32; 4]; EV_ROWS];
    let v4 = |v: [f64; 3], w: f64| [v[0] as f32, v[1] as f32, v[2] as f32, w as f32];
    let x4 = |v: [f64; 4]| v.map(|x| x as f32);
    if let Some(scene) = &view.scene {
        let shot = &scene.shot;
        ev[0] = [
            scene.event.id() as f32,
            scene.local as f32,
            scene.duration as f32,
            scene.variant as f32,
        ];
        ev[1] = v4(shot.ship, 1.0);
        ev[2] = v4(scene.basis[0], shot.hit);
        ev[3] = v4(scene.basis[1], shot.beam);
        ev[4] = v4(scene.basis[2], shot.flash);
        for (k, object) in shot.objects.iter().enumerate() {
            ev[5 + k] = x4(*object);
        }
        ev[8] = x4(shot.rot);
        ev[9] = x4(shot.extra);
        ev[10] = [shot.alert as f32, scene.fade as f32, 0.0, 0.0];
        ev[11] = [
            shot.hit_at[0] as f32,
            shot.hit_at[1] as f32,
            shot.shake as f32,
            0.0,
        ];
    }
    if let Some((kind, amount, local)) = view.flux {
        ev[10][2] = kind as f32;
        ev[10][3] = amount as f32;
        ev[11][3] = local as f32;
    }
    ev[4][3] = ev[4][3].max(view.flash as f32);
    ev[1][3] = if view.deep { 1.0 } else { 0.0 };
    ev
}

fn eclipsed(system: &System, t: f64, at: [f64; 3], radius: f64, planet: Option<usize>) -> bool {
    let to_star = normalize([-at[0], -at[1], -at[2]]);
    let star_distance = length(at);
    let star_angle = (system.star.radius / star_distance).min(1.0).asin();
    let check = |centre: [f64; 3], r: f64| {
        let rel = [centre[0] - at[0], centre[1] - at[1], centre[2] - at[2]];
        let d = length(rel);
        if d < 1e-6 || d > star_distance {
            return false;
        }
        let separation = dot(normalize(rel), to_star).clamp(-1.0, 1.0).acos();
        separation < (r / d).min(1.0).asin() + star_angle + (radius / d).min(1.0).asin()
    };
    for (index, other) in system.planets.iter().enumerate() {
        let centre = other.position(t);
        if Some(index) != planet && check(centre, other.radius) {
            return true;
        }
        for m in 0..other.moons.len().min(stream::MOONS as usize) {
            let offset = other.moon_offset(m, t);
            let c = [
                centre[0] + offset[0],
                centre[1] + offset[1],
                centre[2] + offset[2],
            ];
            if length([c[0] - at[0], c[1] - at[1], c[2] - at[2]]) > 1e-6
                && check(c, other.moons[m].radius)
            {
                return true;
            }
        }
    }
    false
}

/// The part of the frame, as a scissor rectangle in pixels, where the shadows
/// of body `r`'s shadowers (`shade`) can fall on it and be seen: for each,
/// where the line from the star through it meets body `r`, widened by its
/// size and its penumbra. None if no shadow falls on the side in view, so the
/// eclipse pass is skipped.
fn shadow_scissor(params: &Params, r: usize) -> Option<[u32; 4]> {
    let v3 = |a: [f32; 4]| [a[0] as f64, a[1] as f64, a[2] as f64];
    let body = &params.bodies[r];
    let centre = v3(body.place);
    let radius = body.place[3] as f64;
    let star = v3(params.star);
    let (right, up, forward) = (v3(params.right), v3(params.up), v3(params.forward));
    let tan_half = params.frame[1] as f64;
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    let mut any = false;
    for k in 0..4 {
        let j = body.shade[k];
        if j < 0.0 {
            break;
        }
        let other = &params.bodies[j as usize];
        let o = v3(other.place);
        let d = normalize([o[0] - star[0], o[1] - star[1], o[2] - star[2]]);
        // The point on the body nearest the shadow's axis.
        let rel = [centre[0] - o[0], centre[1] - o[1], centre[2] - o[2]];
        let along = dot(rel, d).max(0.0);
        let axis_point = [
            o[0] + d[0] * along,
            o[1] + d[1] * along,
            o[2] + d[2] * along,
        ];
        let towards = normalize([
            axis_point[0] - centre[0],
            axis_point[1] - centre[1],
            axis_point[2] - centre[2],
        ]);
        let p = [
            centre[0] + towards[0] * radius,
            centre[1] + towards[1] * radius,
            centre[2] + towards[2] * radius,
        ];
        // The shadow's reach on the surface: the shadower's size and its
        // penumbra, as an angle round the body's centre.
        let star_distance = length([p[0] - star[0], p[1] - star[1], p[2] - star[2]]).max(1.0);
        let reach = other.place[3] as f64 + along * params.star[3] as f64 / star_distance;
        let miss = length([
            axis_point[0] - centre[0],
            axis_point[1] - centre[1],
            axis_point[2] - centre[2],
        ]);
        if miss > radius + reach {
            continue;
        }
        let spread = (reach / radius).min(std::f64::consts::PI);
        let e1 = normalize(cross(
            towards,
            if towards[1].abs() < 0.9 {
                [0.0, 1.0, 0.0]
            } else {
                [1.0, 0.0, 0.0]
            },
        ));
        let e2 = cross(towards, e1);
        for n in 0..=12 {
            let (a, b) = if n == 12 {
                (0.0, 0.0)
            } else {
                let angle = n as f64 / 12.0 * std::f64::consts::TAU;
                (spread.min(1.5) * angle.cos(), spread.min(1.5) * angle.sin())
            };
            // A point of the shadow's footprint, round the sphere.
            let dir = normalize([
                towards[0] + e1[0] * a + e2[0] * b,
                towards[1] + e1[1] * a + e2[1] * b,
                towards[2] + e1[2] * a + e2[2] * b,
            ]);
            let q = [
                centre[0] + dir[0] * radius,
                centre[1] + dir[1] * radius,
                centre[2] + dir[2] * radius,
            ];
            // Only the side facing the camera can show it.
            if dot(dir, q) > radius * 0.05 && spread < 1.5 {
                continue;
            }
            let z = dot(q, forward);
            if z <= 1e-6 {
                return Some([0, 0, SIZE, SIZE]);
            }
            let x = dot(q, right) / z / tan_half;
            let y = dot(q, up) / z / tan_half;
            lo = [lo[0].min(x), lo[1].min(y)];
            hi = [hi[0].max(x), hi[1].max(y)];
            any = true;
        }
        if spread >= 1.5 {
            // A shadower as big as the body (or a giant's shadow): all of it.
            return Some([0, 0, SIZE, SIZE]);
        }
    }
    if !any {
        return None;
    }
    let half = SIZE as f64 / 2.0;
    let px = |v: f64| ((v * half + half).clamp(0.0, SIZE as f64)) as u32;
    let margin = 6;
    let x0 = px(lo[0]).saturating_sub(margin);
    let x1 = (px(hi[0]) + margin).min(SIZE);
    // Clip space y is up; the frame's rows go down.
    let y0 = (SIZE - px(hi[1])).saturating_sub(margin);
    let y1 = (SIZE - px(lo[1]) + margin).min(SIZE);
    (x1 > x0 && y1 > y0).then_some([x0, y0, x1 - x0, y1 - y0])
}

/// Which of `bodies` (camera-relative, far to near) can cast a shadow on body
/// `index`: those whose disc, seen from it, comes near the star's. Up to four,
/// as indices; -1 for the rest.
fn shadowers(
    bodies: &[(f64, Body, Vec<String>)],
    index: usize,
    star: [f64; 3],
    star_radius: f64,
) -> [f32; 4] {
    let mut out = [-1.0f32; 4];
    let place = bodies[index].1.place;
    let at = [place[0] as f64, place[1] as f64, place[2] as f64];
    let radius = place[3] as f64;
    let to_star = [star[0] - at[0], star[1] - at[1], star[2] - at[2]];
    let star_distance = length(to_star);
    let to_star = normalize(to_star);
    let star_angle = (star_radius / star_distance).min(1.0).asin();
    let mut n = 0;
    for (j, (_, other, _)) in bodies.iter().enumerate() {
        if j == index || other.look[0] < 0.0 || n == 4 {
            continue;
        }
        let rel = [
            other.place[0] as f64 - at[0],
            other.place[1] as f64 - at[1],
            other.place[2] as f64 - at[2],
        ];
        let d = length(rel);
        if d < 1e-6 || d > star_distance {
            continue;
        }
        let separation = dot(normalize(rel), to_star).clamp(-1.0, 1.0).acos();
        let reach =
            (other.place[3] as f64 / d).min(1.0).asin() + star_angle + (radius / d).min(1.0).asin();
        if separation < reach {
            out[n] = j as f32;
            n += 1;
        }
    }
    out
}

/// The passes that draw a planet-sized body of `kind` from map slot `slot`
/// (or flat, if its maps are not in), in order.
fn surface_passes(
    kind: Kind,
    slot: Option<usize>,
    eclipsed: bool,
    rings: bool,
    close: bool,
) -> Vec<String> {
    if let Some(s) = slot
        && !close
    {
        // From afar, the ground, clouds and added light as a mesh in one
        // pass (two for a world with seas: its added light over it). The
        // rings' shadow is in it too; the air past the limb is a pass of its
        // own (`limb`, made `limbring` in `params`).
        let look = SurfaceLook::of(kind, rings);
        let mut passes = vec![look.globe(s)];
        if look.sea {
            passes.push(look.globe_glow(s));
        }
        if eclipsed {
            passes.push("eclipse".into());
        }
        if kind.air()[0] > 0.0 {
            passes.push("limb".into());
        }
        if rings {
            passes.push("ring".into());
        }
        return passes;
    }
    let mut passes: Vec<String> = match slot {
        Some(s) => {
            // Close up, the ground with its detail (one pass); higher up the
            // map alone is fine enough.
            // Close up, the ground with its lookups made crisp, then the
            // detail the map is too coarse for; higher up the map alone.
            let mut passes = vec![if close && kind.has_ground() {
                format!("grounddetail{s}")
            } else {
                format!("ground{s}")
            }];
            if close && kind.has_ground() {
                passes.push(format!("detail{s}"));
            }
            if kind.has_clouds() {
                passes.push(format!("clouds{s}"));
                // In low orbit, clouds that stand up from the deck.
                if close {
                    for layer in 1..=CLOUD_LAYERS {
                        passes.push(format!("cloudlayer{s}x{layer}"));
                    }
                }
            }
            passes
        }
        // Its maps not in: as a mesh with its rings' shadow in it.
        None => vec![FlatLook::of(kind, rings).pipeline()],
    };
    if eclipsed {
        passes.push("eclipse".into());
    }
    if rings && slot.is_some() {
        passes.push("ringshadow".into());
    }
    if let Some(s) = slot {
        // The added light: night lights, the sea's glint, the air's sheen.
        passes.push(if kind.has_sea() {
            format!("glowsea{s}")
        } else {
            format!("glow{s}")
        });
    }
    if kind.air()[0] > 0.0 {
        passes.push("limb".into());
    }
    if rings {
        passes.push("ring".into());
    }
    passes
}

/// What a kind's world from afar needs drawn (`fs_globe`'s constants).
#[derive(Clone, Copy, PartialEq, Eq)]
struct SurfaceLook {
    sea: bool,
    clouds: bool,
    night: u8,
    air: bool,
    giant: bool,
    rings: bool,
}

impl SurfaceLook {
    /// For a world of `kind`, with rings or without.
    fn of(kind: Kind, rings: bool) -> Self {
        Self {
            sea: kind.has_sea(),
            clouds: kind.has_clouds(),
            night: night_mode(kind) as u8,
            air: kind.air()[0] > 0.0,
            giant: matches!(kind, Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter),
            rings,
        }
    }

    /// The look's flags after the slot: `sc0a--` and the like.
    fn flags(self, slot: usize) -> String {
        let flag = |on: bool, c: char| if on { c } else { '-' };
        format!(
            "{slot}{}{}{}{}{}{}",
            flag(self.sea, 's'),
            flag(self.clouds, 'c'),
            self.night,
            flag(self.air, 'a'),
            flag(self.giant, 'g'),
            flag(self.rings, 'r'),
        )
    }

    /// Its mesh's pipeline: `globe` and the flags.
    fn globe(self, slot: usize) -> String {
        format!("globe{}", self.flags(slot))
    }

    /// A sea's added light over its mesh (`fs_globe_glow`): `globeglow` and
    /// the same flags, so it is drawn as a mesh too.
    fn globe_glow(self, slot: usize) -> String {
        format!("globeglow{}", self.flags(slot))
    }

    fn constants(self) -> [(&'static str, f64); 6] {
        [
            ("SEA", f64::from(u8::from(self.sea))),
            ("CLOUDS", f64::from(u8::from(self.clouds))),
            ("NIGHT", f64::from(self.night)),
            ("AIR", f64::from(u8::from(self.air))),
            ("GIANT", f64::from(u8::from(self.giant))),
            ("RINGS", f64::from(u8::from(self.rings))),
        ]
    }
}

/// What a world whose maps are not in yet needs drawn as a mesh
/// (`fs_globe_flat`'s constants).
#[derive(Clone, Copy, PartialEq, Eq)]
struct FlatLook {
    air: bool,
    giant: bool,
    rings: bool,
}

impl FlatLook {
    fn of(kind: Kind, rings: bool) -> Self {
        Self {
            air: kind.air()[0] > 0.0,
            giant: matches!(kind, Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter),
            rings,
        }
    }

    /// `globeflat` and its flags, so it is drawn as a mesh.
    fn pipeline(self) -> String {
        let flag = |on: bool, c: char| if on { c } else { '-' };
        format!(
            "globeflat{}{}{}",
            flag(self.air, 'a'),
            flag(self.giant, 'g'),
            flag(self.rings, 'r'),
        )
    }

    fn constants(self) -> [(&'static str, f64); 4] {
        [
            ("GLOBE", 1.0),
            ("AIR", f64::from(u8::from(self.air))),
            ("GIANT", f64::from(u8::from(self.giant))),
            ("RINGS", f64::from(u8::from(self.rings))),
        ]
    }
}

/// A moon's mesh pipeline (`fs_globe_moon`) for map slot `slot`, with air or
/// without.
fn moon_pipeline(slot: usize, air: bool) -> String {
    format!("globemoon{slot}{}", if air { 'a' } else { '-' })
}

fn night_mode(kind: Kind) -> f32 {
    if kind.has_cities() {
        1.0
    } else if kind.glows() {
        2.0
    } else {
        0.0
    }
}

fn flat_colour(planet: &Planet) -> [f32; 4] {
    match planet.kind {
        Kind::Terran | Kind::Jungle | Kind::Eyeball => [0.06, 0.12, 0.22, 0.0],
        Kind::Ocean => [0.04, 0.10, 0.24, 0.0],
        Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter => {
            let c = planet.palette[0];
            [c[0], c[1], c[2], 0.25]
        }
        _ => {
            let c = planet.palette[0];
            [c[0], c[1], c[2], 0.0]
        }
    }
}

/// How far `planet` has turned on its axis at `t`. A world locked to its star
/// keeps its +z (longitude 0, where the generator puts its warm side) facing
/// the star instead.
fn spin_angle(planet: &Planet, t: f64, centre: [f64; 3]) -> f64 {
    if !planet.kind.locked() {
        return t * std::f64::consts::TAU / planet.day;
    }
    let to_star = normalize([-centre[0], -centre[1], -centre[2]]);
    let ex = equatorial_to_system(planet, [1.0, 0.0, 0.0]);
    let ez = equatorial_to_system(planet, [0.0, 0.0, 1.0]);
    dot(to_star, ex).atan2(dot(to_star, ez))
}

/// Rows taking system directions into a body's turning frame: the planet's
/// equatorial frame, turned `angle` about its axis. The clouds' extra turn
/// rides in the first row's w.
fn rotation(planet: &Planet, angle: f64, clouds: f64) -> [[f32; 4]; 3] {
    let ex = equatorial_to_system(planet, [1.0, 0.0, 0.0]);
    let ey = equatorial_to_system(planet, [0.0, 1.0, 0.0]);
    let ez = equatorial_to_system(planet, [0.0, 0.0, 1.0]);
    let (s, c) = angle.sin_cos();
    let row0 = [0, 1, 2].map(|i| c * ex[i] - s * ez[i]);
    let row2 = [0, 1, 2].map(|i| s * ex[i] + c * ez[i]);
    let f = |r: [f64; 3], w: f64| [r[0] as f32, r[1] as f32, r[2] as f32, w as f32];
    [f(row0, clouds.rem_euclid(1.0)), f(ey, 0.0), f(row2, 0.0)]
}

/// Rows turning world directions into the sky map's frame. The generator
/// already lays the galaxy's band around the system's own pole, so this is
/// the identity; it is here to turn the sky later without touching the map.
fn sky_rotation(_sky: [f32; 4]) -> [[f32; 4]; 3] {
    [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ]
}

/// For a sphere around or close beside the camera, whose outline reaches
/// behind it: the part of the frame whose rays hit it, from a grid of rays,
/// a cell wider all round. In low orbit that leaves out the sky above the
/// horizon, which every pass over the planet would otherwise trace for
/// nothing. None if no ray hits it.
fn close_rect(view: &View, centre: [f64; 3], radius: f64) -> Option<[f32; 4]> {
    const CELLS: usize = 32;
    let right = cross(view.forward, view.up);
    let inside = length(centre) <= radius;
    let (mut lo, mut hi) = ([usize::MAX; 2], [0usize; 2]);
    for gy in 0..=CELLS {
        for gx in 0..=CELLS {
            let x = gx as f64 / CELLS as f64 * 2.0 - 1.0;
            let y = gy as f64 / CELLS as f64 * 2.0 - 1.0;
            let dir = normalize(
                [0, 1, 2]
                    .map(|i| view.forward[i] + (right[i] * x + view.up[i] * y) * view.half_fov),
            );
            let along = dot(centre, dir);
            let miss = length([0, 1, 2].map(|i| centre[i] - dir[i] * along));
            if inside || (along > 0.0 && miss <= radius) {
                lo = [lo[0].min(gx), lo[1].min(gy)];
                hi = [hi[0].max(gx), hi[1].max(gy)];
            }
        }
    }
    if lo[0] == usize::MAX {
        return None;
    }
    let edge = |g: usize, grow: i64| {
        ((g as i64 + grow).clamp(0, CELLS as i64) as f64 / CELLS as f64 * 2.0 - 1.0) as f32
    };
    Some([
        edge(lo[0], -1),
        edge(lo[1], -1),
        edge(hi[0], 1),
        edge(hi[1], 1),
    ])
}

/// A mapped world's mesh (`vs_globe`): the patch of its latitude-longitude
/// grid the camera can see, as `Body::window`, and `Body::mesh`: the grid's
/// cells a side, few enough that a far body costs little and many enough
/// that the silhouette keeps within 0.2 px of the circle, and how far the
/// mesh stands proud, 1.5 px at the limb, for the soft edge. `pixel` is the
/// tangent a pixel spans (`frame.z`).
fn globe_mesh(body: &Body, pixel: f32) -> ([f32; 4], [f32; 4]) {
    use std::f64::consts::{FRAC_PI_2, PI};
    let c = [0, 1, 2].map(|i| f64::from(body.place[i]));
    let r = f64::from(body.place[3]);
    let d = length(c).max(r * 1.001);
    // The way to the camera in the body's turning frame.
    let row = |k: usize| [0, 1, 2].map(|i| f64::from(body.rot[k][i]));
    let a = [0, 1, 2].map(|k| -dot(row(k), c) / d);
    let lat_c = a[1].clamp(-1.0, 1.0).asin();
    let lon_c = a[0].atan2(a[2]);
    let tangent = (d * d - r * r).sqrt();
    let pixel = f64::from(pixel);
    // The limb's radius in pixels, and the widest cell that keeps the
    // silhouette (a cell's sag, about R c^2 / 2) within 0.2 px.
    let limb = r / (tangent * pixel);
    let cell = (0.4 / limb.max(1e-6)).sqrt();
    // The cap in view, two cells wider.
    let alpha = (r / d).acos() + 2.0 * cell;
    let lat_lo = (lat_c - alpha).max(-FRAC_PI_2);
    let lat_hi = (lat_c + alpha).min(FRAC_PI_2);
    let reach = alpha.sin() / lat_c.cos().max(1e-6);
    let pole = lat_c + alpha >= FRAC_PI_2 || lat_c - alpha <= -FRAC_PI_2 || reach >= 1.0;
    // With a pole in view, every longitude.
    let half = if pole { PI } else { reach.asin() };
    let (lon_lo, lon_span) = (lon_c - half, 2.0 * half);
    // The window's widest side as an arc, at the latitude nearest the equator.
    let mut widest = lat_lo.cos().max(lat_hi.cos());
    if lat_lo <= 0.0 && lat_hi >= 0.0 {
        widest = 1.0;
    }
    let span = (lat_hi - lat_lo).max(lon_span * widest);
    let cells = GRIDS
        .into_iter()
        .find(|&n| span / f64::from(n) <= cell)
        .unwrap_or(GRIDS[GRIDS.len() - 1]);
    let proud = 1.5 * pixel * tangent / r;
    let window = [lon_lo, lat_lo, lon_span, lat_hi - lat_lo];
    let mesh = [cells as f32, proud as f32, 0.0, 0.0];
    (window.map(|v| v as f32), mesh)
}

/// The quad covering a sphere of `radius` at `centre` (camera-relative), in
/// clip space, or none if it is out of view.
/// Whether a body's silhouette lies wholly ahead of the camera, well clear
/// of it, so its depth mask can be the polygon inscribed in it.
fn inscribable(body: &Body, params: &Params) -> bool {
    wholly_ahead(body, params, 1.0)
}

/// How the star's glare is drawn: as a polygon round the star out to where
/// it falls under `GLARE_FLOOR` (`glare`), over the whole frame when that
/// reaches too far round the sky (`glarefull`), or not at all when the
/// star's disc is hidden. The same sums as `vs_glare`.
fn glare_pass(params: &Params) -> Option<&'static str> {
    let shown = f64::from(params.star_colour[3]);
    let colour = (0..3).map(|i| f64::from(params.star_colour[i]) * shown);
    let bright = colour.fold(0.0, f64::max);
    if bright <= 0.0 {
        return None;
    }
    let distance = length([0, 1, 2].map(|i| f64::from(params.star[i]))).max(1e-3);
    let chord = f64::from(params.star[3]) / distance * 0.012 * bright / GLARE_FLOOR;
    let wide = chord >= 2.0 * (GLARE_POLYGON / 2.0).sin();
    Some(if wide { "glarefull" } else { "glare" })
}

/// Whether everything within `scale` of its radii of the body's centre is
/// wholly ahead of the camera, its silhouette a circle on the cone of
/// tangents (for `scale` 1.08, its air's: the limb's annulus).
fn wholly_ahead(body: &Body, params: &Params, scale: f64) -> bool {
    let c = [0, 1, 2].map(|i| f64::from(body.place[i]));
    let r = f64::from(body.place[3]) * scale;
    let distance = length(c);
    if distance <= r * 1.05 {
        return false;
    }
    let forward = [0, 1, 2].map(|i| f64::from(params.forward[i]));
    let spread = r * distance / (distance * distance - r * r).sqrt();
    dot(c, forward) - spread > distance * 0.05
}

fn rect(view: &View, centre: [f64; 3], radius: f64) -> Option<[f32; 4]> {
    let distance = length(centre);
    let ahead = dot(centre, view.forward);
    if distance <= radius * 1.02 || ahead <= radius {
        // Around or beside the camera: what of the frame its rays hit.
        return close_rect(view, centre, radius);
    }
    let right = cross(view.forward, view.up);
    // The silhouette: a circle around the centre, perpendicular to the line
    // of sight, on the cone of tangents.
    let axis = normalize(centre);
    let e1 = normalize(cross(
        axis,
        if axis[1].abs() < 0.9 {
            [0.0, 1.0, 0.0]
        } else {
            [1.0, 0.0, 0.0]
        },
    ));
    let e2 = cross(axis, e1);
    let spread = distance * (radius / distance).asin().tan();
    let (mut lo, mut hi) = ([f64::MAX; 2], [f64::MIN; 2]);
    for k in 0..16 {
        let a = k as f64 / 16.0 * std::f64::consts::TAU;
        let p = [0, 1, 2].map(|i| {
            centre[i]
                + spread * (a.cos() * e1[i] + a.sin() * e2[i]) / (std::f64::consts::PI / 16.0).cos()
        });
        let z = dot(p, view.forward);
        if z <= 1e-6 {
            return close_rect(view, centre, radius);
        }
        let x = dot(p, right) / z / view.half_fov;
        let y = dot(p, view.up) / z / view.half_fov;
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    }
    let margin = 3.0 / (SIZE as f64 / 2.0);
    let r = [
        (lo[0] - margin).max(-1.0),
        (lo[1] - margin).max(-1.0),
        (hi[0] + margin).min(1.0),
        (hi[1] + margin).min(1.0),
    ];
    if r[0] >= r[2] || r[1] >= r[3] {
        return None;
    }
    Some(r.map(|v| v as f32))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn read_texture(gpu: &Graphics, texture: &wgpu::Texture) -> Vec<u8> {
        let row = SIZE * 4;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: padded as u64 * SIZE as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let data = slice.get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((row * SIZE) as usize);
        for y in 0..SIZE as usize {
            out.extend_from_slice(&data[y * padded as usize..][..row as usize]);
        }
        out
    }

    /// Run the streamer until everything wanted at `t` is in.
    fn settle(solar: &mut Solar, gpu: &Graphics, t: f64) {
        for _ in 0..100_000 {
            // Asked again before looking: a planet finished in the last draw
            // leaves the streamer idle until the next one wanted is started.
            solar.stream(t);
            let baked = solar.starfield_slot(solar.tour.system.seed).is_some();
            if !solar.streamer.busy() && baked {
                break;
            }
            solar.draw(gpu, t, 0);
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        }
    }

    /// The moments of one visit worth a look: the start, middle and end of
    /// every crossing, each orbit high and low and the dive and climb between,
    /// the departure, and the jump as it spools up, at full speed and as it
    /// drops out.
    fn moments(tour: &Tour) -> Vec<(String, f64)> {
        let mut out = Vec::new();
        for segment in &tour.segments {
            let at = |u: f64| segment.start + segment.duration * u;
            let end = segment.start + segment.duration;
            match segment.phase {
                Phase::Transfer { from, to } => {
                    let from = from.map_or("dropout".to_string(), |p| p.to_string());
                    out.push((
                        format!("transfer-{from}-to-{to}-start"),
                        segment.start + 0.5,
                    ));
                    out.push((format!("transfer-{from}-to-{to}-departure-burn"), at(0.06)));
                    match tour.flyby_at(to) {
                        Some(closest) => {
                            out.push((
                                format!("transfer-{from}-to-{to}-flyby-near"),
                                closest - 3.0,
                            ));
                            out.push((format!("transfer-{from}-to-{to}-flyby"), closest));
                        }
                        None => out.push((format!("transfer-{from}-to-{to}-cruise"), at(0.5))),
                    }
                    out.push((format!("transfer-{from}-to-{to}-capture-burn"), at(0.94)));
                    out.push((format!("transfer-{from}-to-{to}-arrival"), end - 1.5));
                }
                Phase::High(p) => out.push((format!("{p}-high-orbit"), at(0.5))),
                Phase::Depart(_) => {
                    out.push(("depart-start".into(), segment.start + 1.0));
                    out.push(("depart-spool".into(), end - 1.0));
                }
                Phase::Cruise { .. } => {
                    out.push(("cruise-start".into(), segment.start + 1.0));
                    out.push(("cruise".into(), at(0.5)));
                    out.push(("cruise-arrival".into(), end - 1.0));
                }
                Phase::Spool => {
                    for (i, &start) in tour::SPOOL_STAGES.iter().enumerate() {
                        out.push((format!("spool-stage{i}"), at(start + 0.04)));
                    }
                    out.push(("spool-engage".into(), end - 0.5));
                }
                Phase::Jump(k) => {
                    // The last leg keeps the names the previews ask for.
                    let name = if k as usize + 1 == tour.passage.legs.len() {
                        "jump".to_string()
                    } else {
                        format!("jump{k}")
                    };
                    out.push((format!("{name}-entry"), at(0.04)));
                    out.push((format!("{name}-cruise"), at(0.5)));
                    if tour.passage.legs[k as usize].flux.is_some() {
                        out.push((format!("{name}-flux"), at(0.55)));
                    }
                    out.push((format!("{name}-dropout"), end - 2.0));
                }
                Phase::Stopover(k) => {
                    let slug = tour.passage.stops[k as usize].event.slug();
                    out.push((format!("{slug}-dropout"), segment.start + 0.8));
                    for (i, u) in [0.15, 0.4, 0.6, 0.8, 0.97].into_iter().enumerate() {
                        out.push((format!("{slug}-{i}"), at(u)));
                    }
                }
                Phase::Rejump(k) => {
                    let slug = tour.passage.stops[k as usize].event.slug();
                    out.push((format!("{slug}-rejump"), at(0.5)));
                    out.push((format!("{slug}-rejump-engage"), end - 0.4));
                }
            }
        }
        out
    }

    /// Some trips hop through relays, never more than two in a row, and
    /// every tour's route ends at the next real visit.
    #[test]
    fn some_jumps_hop_through_relays() {
        let base = 7;
        let relays: Vec<bool> = (0..200).map(|v| relay(base, v)).collect();
        assert!(!relays[0]);
        let count = relays.iter().filter(|&&r| r).count();
        assert!(count > 20 && count < 100, "{count}");
        assert!(relays.windows(3).all(|w| !w.iter().all(|&r| r)));
        assert!(
            relays.windows(2).any(|w| w[0] && w[1]),
            "a trip of three jumps now and then"
        );
        for visit in 0..60 {
            let tour = tour_for(base, visit);
            assert_eq!(tour.transit, relays[visit as usize]);
            let hops = tour.route.len();
            assert!(
                relays[visit as usize + 1..visit as usize + hops]
                    .iter()
                    .all(|&r| r)
            );
            assert!(!relays[visit as usize + hops]);
            assert!(
                tour.leg >= 1 && tour.leg + hops - 1 == tour.legs,
                "{visit}: {} {hops} {}",
                tour.leg,
                tour.legs
            );
        }
    }

    /// The index buffer's grids lie where `grid_range` says, and a mesh's
    /// grid follows the disc's size: fine in orbit, coarse far off.
    #[test]
    fn grids() {
        let indices = grid_indices();
        assert_eq!(grid_range(8), 0..384);
        assert_eq!(grid_range(96), 32640..87936);
        assert_eq!(grid_range(128), 87936..186240);
        assert_eq!(indices.len(), 186240);
        assert_eq!(indices[87936..].iter().max(), Some(&(128 * 129 + 128)));
        let pixel = (2.0 * 0.5 / f64::from(SIZE)) as f32;
        let body = |distance: f32| Body {
            place: [0.0, 0.0, distance, 1.0],
            rot: [0, 1, 2].map(|k| [0, 1, 2, 3].map(|i| f32::from(u8::from(i == k)))),
            ..Default::default()
        };
        // From 2.5 radii, a limb 470 px from the centre: cells under 1.7°, a
        // cap of 66 degrees round the side facing the camera (-z).
        let (window, mesh) = globe_mesh(&body(2.5), pixel);
        assert_eq!(mesh[0], 96.0);
        assert!(((window[0] + window[2] / 2.0).abs() - std::f32::consts::PI).abs() < 1e-4);
        assert!(window[3] > 2.0 * 66.4f32.to_radians() && window[3] < 2.0 * 75f32.to_radians());
        // Far off, a few pixels: the coarsest, every longitude.
        let (window, mesh) = globe_mesh(&body(2000.0), pixel);
        assert_eq!(mesh[0], 8.0);
        assert_eq!(window[2], std::f32::consts::TAU);
    }

    /// Screenshots at the critical moments of a visit, for each seed in
    /// `SOLAR_SHOTS_SEEDS` (comma separated, default 0,1,2,5: between them
    /// every kind of planet and moon is visited), written to
    /// `SOLAR_SHOTS_DIR` (default `target/solar-shots`): one PNG per moment,
    /// a contact sheet per seed, and the HUD at each moment in `hud.txt`.
    /// `SOLAR_SHOTS=low` keeps only the moments whose names contain `low`.
    /// The streamer is run until the moment's maps are in, so each shot shows
    /// the finished assets.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn screenshots() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        let seeds: Vec<u64> = std::env::var("SOLAR_SHOTS_SEEDS")
            .unwrap_or_else(|_| "0,1,2,5".into())
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let only = std::env::var("SOLAR_SHOTS").ok();
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut notes = String::new();
        for seed in seeds {
            let mut solar = Solar::with_seed(&gpu, seed);
            let system = &solar.tour.system;
            notes.push_str(&format!(
                "seed {seed}: {} ({}, {:.0} K), itinerary {:?}\n",
                system.star.name,
                system.star.title(),
                system.star.temperature,
                solar.tour.itinerary
            ));
            for (i, planet) in system.planets.iter().enumerate() {
                notes.push_str(&format!(
                    "  p{i} {} {:?} r={:.2} moons=[{}]{}\n",
                    planet.name,
                    planet.kind,
                    planet.radius,
                    planet
                        .moons
                        .iter()
                        .map(|m| format!("{} {:?}", m.name, m.kind))
                        .collect::<Vec<_>>()
                        .join(", "),
                    if planet.rings.is_some() { " rings" } else { "" }
                ));
            }
            let mut thumbs = Vec::new();
            for (k, (label, t)) in moments(&solar.tour).into_iter().enumerate() {
                if only.as_deref().is_some_and(|o| !label.contains(o)) {
                    continue;
                }
                settle(&mut solar, &gpu, t);
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let name = format!("seed{seed}-{k:02}-{label}");
                let path = std::path::Path::new(&dir).join(format!("{name}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a screenshot");
                solar.hud_at = f64::NEG_INFINITY;
                solar.update_hud(t);
                notes.push_str(&format!("  {name} (t={t:.1}s) {}\n", solar.hud.title));
                for (l, v) in &solar.hud.lines {
                    notes.push_str(&format!("      {l:>13}  {v}\n"));
                }
                if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                    thumbs.push(image::imageops::thumbnail(&image, 270, 270));
                }
            }
            // A contact sheet, six across.
            let across = 6u32;
            let rows = (thumbs.len() as u32).div_ceil(across);
            let mut sheet = image::RgbaImage::new(270 * across, 270 * rows.max(1));
            for (i, thumb) in thumbs.iter().enumerate() {
                let (x, y) = ((i as u32 % across) * 270, (i as u32 / across) * 270);
                image::imageops::overlay(&mut sheet, thumb, x as i64, y as i64);
            }
            sheet
                .save(std::path::Path::new(&dir).join(format!("seed{seed}-sheet.png")))
                .expect("a contact sheet");
        }
        std::fs::write(std::path::Path::new(&dir).join("hud.txt"), notes).expect("the HUD notes");
    }

    /// Every hyperjump drive at a few moments of a jump, to
    /// `SOLAR_SHOTS_DIR` (default `target/solar-shots`): `warp-<drive>-<n>.png`
    /// and a sheet of them all, a row per drive.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn warps() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, 11);
        let start = solar.tour.total - tour::JUMP;
        // The system's starfield baked, as it is long before any jump.
        settle(&mut solar, &gpu, start);
        let moments = [0.03, 0.12, 0.5, 0.85];
        let mut sheet =
            image::RgbaImage::new(270 * moments.len() as u32, 270 * tour::DRIVES.len() as u32);
        for (k, name) in tour::DRIVES.iter().enumerate() {
            solar.drive_override = Some(k as u32 + 1);
            for (m, at) in moments.iter().enumerate() {
                let t = start + tour::JUMP * at;
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let file = name.to_lowercase().replace(' ', "-");
                let path = std::path::Path::new(&dir).join(format!("warp-{file}-{m}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a screenshot");
                if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                    let thumb = image::imageops::thumbnail(&image, 270, 270);
                    image::imageops::overlay(&mut sheet, &thumb, 270 * m as i64, 270 * k as i64);
                }
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("warps-sheet.png"))
            .expect("a sheet");
    }

    /// Every drive starting up at the jump point, a frame in each stage of
    /// its preparation and two into the jump, to `SOLAR_SHOTS_DIR` (default
    /// `target/solar-shots`): `spool-<drive>-<n>.png` and `spools-sheet.png`,
    /// a row per drive.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn spools() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, 11);
        let start = solar.tour.total - tour::JUMP - tour::SPOOL;
        settle(&mut solar, &gpu, start);
        // Seconds into the spool: a little past each stage's start, the
        // engagement's end, and into the jump.
        let moments: Vec<f64> = tour::SPOOL_STAGES
            .iter()
            .map(|s| tour::SPOOL * (s + 0.12))
            .chain([tour::SPOOL - 0.3, tour::SPOOL + 2.0, tour::SPOOL + 6.0])
            .collect();
        let cell = 216;
        let mut sheet = image::RgbaImage::new(
            cell * moments.len() as u32,
            cell * tour::DRIVES.len() as u32,
        );
        for (k, name) in tour::DRIVES.iter().enumerate() {
            solar.drive_override = Some(k as u32 + 1);
            for (m, at) in moments.iter().enumerate() {
                solar.draw(&gpu, start + at, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let file = name.to_lowercase().replace(' ', "-");
                let path = std::path::Path::new(&dir).join(format!("spool-{file}-{m}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a screenshot");
                if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                    let thumb = image::imageops::thumbnail(&image, cell, cell);
                    image::imageops::overlay(
                        &mut sheet,
                        &thumb,
                        (cell * m as u32) as i64,
                        (cell * k as u32) as i64,
                    );
                }
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("spools-sheet.png"))
            .expect("a sheet");
    }

    /// A relay's crossing: dropping out at its edge, across it to the jump
    /// point and the drive starting up there, as `relay-<n>.png` and
    /// `relay-sheet.png` in `SOLAR_SHOTS_DIR`.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn relay_crossing() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, 11);
        let (system, next) = (solar.tour.next.clone(), system_for(11, 2));
        let passage = voyage::Passage::plan(11, 2, tour::drive_to(&next).0);
        solar.tour = Tour::plan(
            system,
            vec![next.clone(), system_for(11, 3)],
            true,
            1,
            2,
            passage,
            1,
        );
        settle(&mut solar, &gpu, 0.0);
        let moments = [0.5, 1.5, 3.0, 20.0, 40.0, 60.0, 78.0, 84.0, 100.0, 112.0];
        let cell = 216;
        let mut sheet = image::RgbaImage::new(cell * moments.len() as u32, cell);
        for (m, &t) in moments.iter().enumerate() {
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            let path = std::path::Path::new(&dir).join(format!("relay-{m}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a screenshot");
            if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                let thumb = image::imageops::thumbnail(&image, cell, cell);
                image::imageops::overlay(&mut sheet, &thumb, (cell * m as u32) as i64, 0);
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("relay-sheet.png"))
            .expect("a sheet");
    }

    /// The drop-out from each drive into the next system: the jump's last
    /// second, then the next visit's first seconds, as `dropout-<drive>-<n>.png`
    /// and `dropouts-sheet.png` (a row a drive) in `SOLAR_SHOTS_DIR`.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn drop_outs() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        // Seconds before the end of the jump (negative), then into the next visit.
        let moments = [-1.0, -0.4, -0.02, 0.0, 0.25, 0.5, 0.8, 1.2, 1.7, 2.6];
        let cell = 216;
        let mut sheet = image::RgbaImage::new(
            cell * moments.len() as u32,
            cell * tour::DRIVES.len() as u32,
        );
        for (k, name) in tour::DRIVES.iter().enumerate() {
            let mut solar = Solar::with_seed(&gpu, 11);
            solar.drive_override = Some(k as u32 + 1);
            let total = solar.tour.total;
            // The next system's starfield and first stop, as the jump has
            // them ready by its end.
            settle(&mut solar, &gpu, total - 1.0);
            let mut arrived = false;
            for (m, &at) in moments.iter().enumerate() {
                let t = if at < 0.0 {
                    total + at
                } else {
                    if !arrived {
                        solar.catch_up(total);
                        settle(&mut solar, &gpu, 0.0);
                        arrived = true;
                    }
                    at
                };
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let file = name.to_lowercase().replace(' ', "-");
                let path = std::path::Path::new(&dir).join(format!("dropout-{file}-{m}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a screenshot");
                if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
                    let thumb = image::imageops::thumbnail(&image, cell, cell);
                    let (x, y) = (cell * m as u32, cell * k as u32);
                    image::imageops::overlay(&mut sheet, &thumb, x as i64, y as i64);
                }
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("dropouts-sheet.png"))
            .expect("a sheet");
    }

    /// Visit 0 of the tour seeded `base`, its jump through `stops` alone.
    fn tour_through(base: u64, stops: Vec<voyage::Stopover>) -> Tour {
        let system = system_for(base, 0);
        let next = system_for(base, 1);
        let drive = tour::drive_to(&next).0;
        let mut rng = system::Rng::new(base ^ 0x5eed);
        let passage = voyage::Passage::with_stops(stops, drive, &mut rng);
        let arrival = tour::drive_to(&system).0;
        Tour::plan(system, vec![next], false, 1, 1, passage, arrival)
    }

    /// Saves frame `pixels` as `name` in `dir` and adds it to `sheet` at
    /// column `x`, row `y`, `cell` pixels a side.
    fn shoot(
        dir: &str,
        name: &str,
        pixels: Vec<u8>,
        sheet: &mut image::RgbaImage,
        cell: u32,
        x: u32,
        y: u32,
    ) {
        let path = std::path::Path::new(dir).join(format!("{name}.png"));
        image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
            .expect("a screenshot");
        if let Some(image) = image::RgbaImage::from_raw(SIZE, SIZE, pixels) {
            let thumb = image::imageops::thumbnail(&image, cell, cell);
            image::imageops::overlay(sheet, &thumb, (cell * x) as i64, (cell * y) as i64);
        }
    }

    /// Every encounter between the legs of a jump, gates and wormholes too:
    /// dropping out, five moments of it and the drive re-engaging (or the
    /// ship thrown on into the next leg), to `SOLAR_SHOTS_DIR` (default
    /// `target/solar-shots`): `encounter-<event>-<n>.png` and
    /// `encounters-sheet.png`, a row an event. `SOLAR_ENCOUNTERS=<slugs>`
    /// picks some.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn encounters() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let only = std::env::var("SOLAR_ENCOUNTERS").ok();
        let events: Vec<voyage::Event> = voyage::Event::ALL
            .into_iter()
            .filter(|e| {
                only.as_ref()
                    .is_none_or(|o| o.split(',').any(|s| s == e.slug()))
            })
            .collect();
        let variant: u64 = std::env::var("SOLAR_VARIANT")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        let moments = [0.6, 0.15, 0.4, 0.6, 0.8, 0.97, 1.03];
        let cell = 216;
        let mut sheet =
            image::RgbaImage::new(cell * moments.len() as u32, cell * events.len() as u32);
        let mut solar = Solar::with_seed(&gpu, 11);
        for (row, event) in events.iter().enumerate() {
            solar.tour = tour_through(
                11,
                vec![voyage::Stopover {
                    event: *event,
                    variant,
                }],
            );
            let stop = solar
                .tour
                .segments
                .iter()
                .find(|s| s.phase == Phase::Stopover(0))
                .copied()
                .expect("a stopover");
            settle(&mut solar, &gpu, stop.start);
            for (m, &at) in moments.iter().enumerate() {
                let t = if m == 0 {
                    stop.start + at
                } else if at > 1.0 {
                    // Re-engaging, or thrown on into the next leg.
                    stop.start
                        + stop.duration
                        + if event.portal() {
                            0.4
                        } else {
                            event.rejump() * 0.6
                        }
                } else {
                    stop.start + stop.duration * at
                };
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                shoot(
                    &dir,
                    &format!("encounter-{}-{m}", event.slug()),
                    pixels,
                    &mut sheet,
                    cell,
                    m as u32,
                    row as u32,
                );
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("encounters-sheet.png"))
            .expect("a sheet");
    }

    /// Hive cubes of eight draws at the same moment of their scripts, to
    /// show each is its own: `hive-cubes-sheet.png`.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn hive_cubes() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let cell = 270;
        let mut sheet = image::RgbaImage::new(cell * 4, cell * 2);
        let mut solar = Solar::with_seed(&gpu, 11);
        for v in 0..8u64 {
            let stop = voyage::Stopover {
                event: voyage::Event::HiveCube,
                variant: v * 7919 + 3,
            };
            solar.tour = tour_through(11, vec![stop]);
            let seg = solar
                .tour
                .segments
                .iter()
                .find(|s| s.phase == Phase::Stopover(0))
                .copied()
                .expect("a stopover");
            let t = seg.start + seg.duration * 0.5;
            settle(&mut solar, &gpu, t);
            solar.draw(&gpu, t, 0);
            let pixels = read_texture(&gpu, &solar.frames[0].texture);
            shoot(
                &dir,
                &format!("hive-cube-{v}"),
                pixels,
                &mut sheet,
                cell,
                (v % 4) as u32,
                (v / 4) as u32,
            );
        }
        sheet
            .save(std::path::Path::new(&dir).join("hive-cubes-sheet.png"))
            .expect("a sheet");
    }

    /// Asteroid fields of the three kinds of rock (carbon, ice, iron), each
    /// as a big rock nears and as it passes close, and the field at large:
    /// fixed moments, so shots before and after a change line up
    /// (`asteroids-sheet.png`, `asteroids-<kind>-<n>.png`).
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn asteroid_fields() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let cell = 360;
        let mut sheet = image::RgbaImage::new(cell * 3, cell * 3);
        let mut solar = Solar::with_seed(&gpu, 11);
        let mut found = 0;
        let mut variant = 0u64;
        while found < 3 {
            variant += 1;
            let stop = voyage::Stopover {
                event: voyage::Event::Asteroids,
                variant,
            };
            if voyage::shot(&stop, 0.0).extra[0] as usize != found {
                continue;
            }
            solar.tour = tour_through(11, vec![stop]);
            let seg = solar
                .tour
                .segments
                .iter()
                .find(|s| s.phase == Phase::Stopover(0))
                .copied()
                .expect("a stopover");
            settle(&mut solar, &gpu, seg.start);
            // The first big rock nearing and passing, and the field.
            let pass = voyage::shot(&stop, 0.0).objects[0][2] / 55.0;
            for (m, t) in [pass - 2.6, pass - 0.9, seg.duration * 0.45]
                .into_iter()
                .enumerate()
            {
                solar.draw(&gpu, seg.start + t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let kind = ["carbon", "ice", "iron"][found];
                shoot(
                    &dir,
                    &format!("asteroids-{kind}-{m}"),
                    pixels,
                    &mut sheet,
                    cell,
                    m as u32,
                    found as u32,
                );
            }
            found += 1;
        }
        sheet
            .save(std::path::Path::new(&dir).join("asteroids-sheet.png"))
            .expect("a sheet");
    }

    /// A planet of each of the main kinds with ground, the first such stop
    /// found in the tours seeded 0 up, each high in orbit and low over its
    /// horizon: fixed moments, so shots before and after a change line up
    /// (`planets-sheet.png`, `planet-<kind>-<n>.png`). `SOLAR_PLANET_KINDS`
    /// picks some (by name, as `Kind::name` gives them, lower case).
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn planet_gallery() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let only = std::env::var("SOLAR_PLANET_KINDS").ok();
        let kinds: Vec<Kind> = [
            Kind::Terran,
            Kind::Ocean,
            Kind::Jungle,
            Kind::Desert,
            Kind::Lava,
            Kind::Ice,
            Kind::Barren,
        ]
        .into_iter()
        .filter(|k| {
            only.as_ref()
                .is_none_or(|o| o.split(',').any(|s| s == k.name().to_lowercase()))
        })
        .collect();
        let cell = 360;
        let mut sheet = image::RgbaImage::new(cell * 3, cell * kinds.len() as u32);
        for (row, kind) in kinds.iter().enumerate() {
            let found = (0..400u64).find_map(|seed| {
                let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
                let stop =
                    tour.itinerary.iter().copied().find(|s| {
                        s.moon.is_none() && tour.system.planets[s.planet].kind == *kind
                    })?;
                Some((seed, stop))
            });
            let Some((seed, stop)) = found else { continue };
            let mut solar = Solar::with_seed(&gpu, seed);
            solar.tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let high = solar
                .tour
                .segments
                .iter()
                .find(|s| s.phase == Phase::High(stop))
                .copied()
                .expect("an orbit");
            let t = high.start + high.duration * 0.5;
            settle(&mut solar, &gpu, t);
            let centre = solar.tour.system.centre(stop, t);
            let radius = solar.tour.world(stop).radius;
            let sun = normalize(scale(centre, -1.0));
            let up = [0.0, 1.0, 0.0];
            let side = normalize(cross(sun, up));
            // From the day side, a little round toward the terminator: a lit
            // three-quarter disc; then low over the lit ground, looking along
            // the horizon.
            let from = normalize(add(scale(sun, 0.8), scale(side, 0.6)));
            let whole = tour::View {
                position: add(centre, scale(from, radius * 2.9)),
                forward: scale(from, -1.0),
                up,
                ..solar.tour.view(t)
            };
            let ground = normalize(add(
                scale(sun, 0.75),
                add(scale(side, 0.45), scale(up, 0.45)),
            ));
            let along = normalize(sub(
                scale(side, -1.0),
                scale(ground, dot(scale(side, -1.0), ground)),
            ));
            let low = tour::View {
                position: add(centre, scale(ground, radius * 1.18)),
                forward: normalize(add(along, scale(ground, -0.42))),
                up: ground,
                ..solar.tour.view(t)
            };
            let orbit = solar.tour.view(t);
            for (m, view) in [whole, low, orbit].into_iter().enumerate() {
                let mut view = view;
                view.up = normalize(cross(normalize(cross(view.forward, view.up)), view.forward));
                (view.warp, view.reveal, view.target_shown, view.spool_light) =
                    (0.0, 1.0, 0.0, 0.0);
                solar.view_override = Some(view);
                settle(&mut solar, &gpu, t);
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let name = kind.name().to_lowercase().replace(' ', "-");
                shoot(
                    &dir,
                    &format!("planet-{name}-{m}"),
                    pixels,
                    &mut sheet,
                    cell,
                    m as u32,
                    row as u32,
                );
            }
            solar.view_override = None;
        }
        sheet
            .save(std::path::Path::new(&dir).join("planets-sheet.png"))
            .expect("a sheet");
    }

    /// Each fleet's design alone, its capital ship and an escort, lit
    /// three-quarters: `fleets-sheet.png`, a row a design.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn fleets() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let cell = 360;
        let mut sheet = image::RgbaImage::new(cell * 2, cell * voyage::FACTIONS.len() as u32);
        let mut solar = Solar::with_seed(&gpu, 11);
        let stop = voyage::Stopover {
            event: voyage::Event::Battle,
            variant: 17,
        };
        solar.tour = tour_through(11, vec![stop]);
        let seg = solar
            .tour
            .segments
            .iter()
            .find(|s| s.phase == Phase::Stopover(0))
            .copied()
            .expect("a stopover");
        let t = seg.start + 3.0;
        settle(&mut solar, &gpu, t);
        let mut view = solar.tour.view(t);
        let basis = view.scene.expect("a stopover").basis;
        (view.forward, view.up, view.reveal) = (basis[2], basis[1], 1.0);
        solar.view_override = Some(view);
        for faction in 0..voyage::FACTIONS.len() as u32 {
            for (column, index) in [0u32, 1].into_iter().enumerate() {
                solar.showcase = Some((faction, index));
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                let name = voyage::FACTIONS[faction as usize].replace(' ', "-");
                shoot(
                    &dir,
                    &format!("fleet-{name}-{column}"),
                    pixels,
                    &mut sheet,
                    cell,
                    column as u32,
                    faction,
                );
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("fleets-sheet.png"))
            .expect("a sheet");
    }

    /// Battles of six draws (other fleets each), a row each: the fight,
    /// the blast, and its shock rippling out and shoving the ships aside:
    /// `battles-sheet.png`.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn battles() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let cell = 270;
        let after = [-3.0, 0.25, 0.9, 1.8];
        let mut sheet = image::RgbaImage::new(cell * after.len() as u32, cell * 6);
        let mut solar = Solar::with_seed(&gpu, 11);
        for v in 0..6u64 {
            let stop = voyage::Stopover {
                event: voyage::Event::Battle,
                variant: v * 104_729 + 17,
            };
            solar.tour = tour_through(11, vec![stop]);
            let seg = solar
                .tour
                .segments
                .iter()
                .find(|s| s.phase == Phase::Stopover(0))
                .copied()
                .expect("a stopover");
            let kill = voyage::shot(&stop, 0.0).objects[1][2];
            settle(&mut solar, &gpu, seg.start);
            for (m, at) in after.iter().enumerate() {
                let t = seg.start + kill + at;
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                shoot(
                    &dir,
                    &format!("battle-{v}-{m}"),
                    pixels,
                    &mut sheet,
                    cell,
                    m as u32,
                    v as u32,
                );
            }
        }
        sheet
            .save(std::path::Path::new(&dir).join("battles-sheet.png"))
            .expect("a sheet");
    }

    /// What is seen inside a leg, each through two drives' light:
    /// `fluxes-sheet.png`, a row a flux.
    #[test]
    #[ignore = "writes screenshots; needs a working GPU adapter"]
    fn fluxes() {
        let dir = std::env::var("SOLAR_SHOTS_DIR").unwrap_or_else(|_| "target/solar-shots".into());
        std::fs::create_dir_all(&dir).expect("a screenshot directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let cell = 270;
        let mut sheet = image::RgbaImage::new(cell * 4, cell * voyage::Flux::ALL.len() as u32);
        let mut solar = Solar::with_seed(&gpu, 11);
        let stop = voyage::Stopover {
            event: voyage::Event::Asteroids,
            variant: 1,
        };
        solar.tour = tour_through(11, vec![stop]);
        for (row, flux) in voyage::Flux::ALL.into_iter().enumerate() {
            solar.tour.passage.legs[0].flux = Some(flux);
            let leg = solar
                .tour
                .segments
                .iter()
                .find(|s| s.phase == Phase::Jump(0))
                .copied()
                .expect("a leg");
            settle(&mut solar, &gpu, leg.start);
            for (m, (drive, at)) in [(1, 0.4), (1, 0.6), (3, 0.5), (6, 0.5)]
                .into_iter()
                .enumerate()
            {
                solar.drive_override = Some(drive);
                if drive == 6 {
                    solar.drive_override = None;
                    solar.tour.passage.legs[0].look = 6;
                }
                let t = leg.start + leg.duration * at;
                solar.draw(&gpu, t, 0);
                let pixels = read_texture(&gpu, &solar.frames[0].texture);
                shoot(
                    &dir,
                    &format!("flux-{}-{m}", flux.slug()),
                    pixels,
                    &mut sheet,
                    cell,
                    m as u32,
                    row as u32,
                );
            }
            solar.tour.passage.legs[0].look = solar.tour.drive().0;
        }
        sheet
            .save(std::path::Path::new(&dir).join("fluxes-sheet.png"))
            .expect("a sheet");
    }

    /// One finished frame and its HUD at the first moment whose name holds
    /// `moment`, for the UI previews.
    pub(crate) fn render_once(seed: u64, moment: &str) -> (Vec<u8>, Hud, Vec<labels::BodyLabel>) {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, seed);
        let t = moments(&solar.tour)
            .into_iter()
            .find(|(label, _)| label.ends_with(moment))
            .map_or(60.0, |(_, t)| t);
        settle(&mut solar, &gpu, t);
        solar.draw(&gpu, t, 0);
        solar.hud_at = f64::NEG_INFINITY;
        solar.update_hud(t);
        solar.update_labels(t);
        (
            read_texture(&gpu, &solar.frames[0].texture),
            solar.hud.clone(),
            solar.labels.clone(),
        )
    }

    /// Fragment counts for the Pi estimate: every draw of one frame at each
    /// key moment of a visit, for each seed in `PI_ESTIMATE_SEEDS` (default
    /// 5,23), and every draw of each frame while the first planet streams in.
    /// Written to `target/pi-estimate/solar.tsv`; `deploy/pi-estimate.sh`
    /// turns them into milliseconds on the Pi.
    #[test]
    #[ignore = "counts fragments for the Pi estimate; needs a GPU adapter with pipeline statistics"]
    fn pi_estimate() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        assert!(
            crate::gpu::start_counting(&gpu.device),
            "this adapter cannot count fragments ({})",
            gpu.adapter
        );
        let seeds: Vec<u64> = std::env::var("PI_ESTIMATE_SEEDS")
            .unwrap_or_else(|_| "5,23".into())
            .split(',')
            .filter_map(|s| s.trim().parse().ok())
            .collect();
        let mut out = String::from("scene\tmoment\tframe\tlabel\tcovered\tkept\n");
        let mut push = |moment: &str, frame: usize, counts: Vec<(String, u64, u64)>| {
            for (label, covered, kept) in counts {
                out.push_str(&format!(
                    "solar\t{moment}\t{frame}\t{label}\t{covered}\t{kept}\n"
                ));
            }
        };
        for seed in seeds {
            let mut solar = Solar::with_seed(&gpu, seed);
            // Streaming the first planet in: every frame's draws.
            let mut frame = 0;
            loop {
                solar.stream(1.0);
                solar.draw(&gpu, 1.0, frame % SLOTS);
                push(
                    &format!("seed{seed} streaming"),
                    frame,
                    crate::gpu::take_counts(&gpu),
                );
                frame += 1;
                if !solar.streamer.busy() || frame > 4000 {
                    break;
                }
            }
            for (label, t) in moments(&solar.tour) {
                settle(&mut solar, &gpu, t);
                crate::gpu::take_counts(&gpu);
                solar.draw(&gpu, t, 0);
                push(
                    &format!("seed{seed} {label}"),
                    0,
                    crate::gpu::take_counts(&gpu),
                );
            }
        }
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/pi-estimate");
        std::fs::create_dir_all(&dir).expect("an output directory");
        std::fs::write(dir.join("solar.tsv"), out).expect("the counts");
    }

    /// Frame times through a whole visit, a frame every `SWEEP_STEP`
    /// seconds (default 4) from `SWEEP_FROM` to `SWEEP_TO` (default the
    /// whole tour) of seed `SWEEP_SEED` (default 11; `SWEEP_DRIVE` 1..5 for
    /// another jump drive), each with its phase,
    /// background and draws: where the time goes.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_sweep() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let seed = std::env::var("SWEEP_SEED")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(11);
        let mut solar = Solar::with_seed(&gpu, seed);
        solar.drive_override = std::env::var("SWEEP_DRIVE")
            .ok()
            .and_then(|s| s.parse().ok());
        let var = |k: &str, d: f64| {
            std::env::var(k)
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(d)
        };
        let end = var("SWEEP_TO", solar.tour.total);
        let step = var("SWEEP_STEP", 4.0);
        let mut t = var("SWEEP_FROM", 1.0);
        while t < end {
            settle(&mut solar, &gpu, t);
            let n = 10;
            let started = std::time::Instant::now();
            let mut cpu = 0.0;
            for i in 0..n {
                let drawn = std::time::Instant::now();
                solar.draw(&gpu, t + i as f64 * 0.05, i % SLOTS);
                cpu += drawn.elapsed().as_secs_f64();
            }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            println!(
                "sweep t={t:.0} {:.2} ms (cpu {:.2}) {:?} {}",
                started.elapsed().as_secs_f64() * 1e3 / n as f64,
                cpu * 1e3 / n as f64,
                solar.tour.segment(t).0.phase,
                solar.drawn.borrow()
            );
            t += step;
        }
    }

    /// Frame times in the heaviest views, and how long streaming a planet
    /// takes in frames.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut solar = Solar::with_seed(&gpu, 5);
        let started = std::time::Instant::now();
        let mut frames = 0;
        let t0 = 1.0;
        loop {
            solar.stream(t0);
            solar.draw(&gpu, t0, frames % SLOTS);
            frames += 1;
            if !solar.streamer.busy() || frames > 5000 {
                break;
            }
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        println!(
            "solar: first planet and sky streamed in {frames} frames, {:.0} ms",
            started.elapsed().as_secs_f64() * 1e3
        );
        for (label, t) in moments(&solar.tour) {
            settle(&mut solar, &gpu, t);
            let n = 30;
            let started = std::time::Instant::now();
            for i in 0..n {
                solar.draw(&gpu, t + i as f64 * 0.05, i % SLOTS);
            }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            println!(
                "solar {label}: {:.2} ms a frame",
                started.elapsed().as_secs_f64() * 1e3 / n as f64
            );
        }
    }
}
