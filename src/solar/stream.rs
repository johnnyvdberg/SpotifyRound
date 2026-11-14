//! Streaming the tour's maps: planets, their moons and the sky, painted on the
//! GPU a few rows per frame while the scene keeps drawing.
//!
//! There are two planet slots. The planet being visited holds one and the
//! next on the itinerary is painted into the other while the camera travels;
//! when the tour moves on, the slot of the planet left behind is overwritten
//! with the one after. Each slot also holds its planet's moons, one layer of
//! a texture array each. Skies come in a pair too: the next system's is
//! painted during the hyperjump.
//!
//! A job is a list of steps, one per frame: a small height probe whose
//! histogram sets the sea level (read back without waiting), the height map
//! a band of rows at a time, the surface maps a band at a time, the mip
//! chains, then each moon. `SOLAR_GEN_ROWS` sets the band; a 2048-texel band
//! of 8 rows is about 16k texels a pass, and by default the band keeps that
//! many texels at any map size.
//!
//! `SOLAR_MAP_SIZE` sets the planet maps' width (1024 to 8192, a power of
//! two; the height is half), for measuring on the Pi: 2048 by default. A
//! moon's maps stay 512 × 256: a moon is only ever a small disc beside its
//! planet, and one the tour visits is painted into a planet slot.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::wgpu_30::wgpu;

use super::system::{Kind, Moon, Planet, Rng};
use crate::gpu::Graphics;

/// The planet maps' width unless `SOLAR_MAP_SIZE` says otherwise.
pub const MAP_WIDTH: u32 = 2048;
pub const MOON_WIDTH: u32 = 512;
pub const MOON_HEIGHT: u32 = 256;
pub const MOONS: u32 = 6;
pub const SKY_WIDTH: u32 = 1024;
pub const SKY_HEIGHT: u32 = 512;
const PROBE_WIDTH: u32 = 128;
const PROBE_HEIGHT: u32 = 64;

/// The planet maps' width and height: `SOLAR_MAP_SIZE` (the width, rounded
/// to a power of two from 1024 to 8192), else `MAP_WIDTH`; never more than
/// the GPU's `limit`.
pub fn map_size(limit: u32) -> (u32, u32) {
    let width = std::env::var("SOLAR_MAP_SIZE")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .map_or(MAP_WIDTH, |w| {
            w.clamp(1024, 8192).next_power_of_two().min(8192)
        });
    let width = width.min(limit.max(1024));
    (width, width / 2)
}

/// Identifies a planet across systems: the system's seed and its index.
pub type Key = (u64, usize);

const DAY: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const RELIEF: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const EXTRA: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const HEIGHT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const PROBE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const SKY: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const CLIMATE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

/// The generator's parameters, as the shader's `Gen`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GenParams {
    size: [f32; 4],
    offsets: [[f32; 4]; 4],
    plates: [[f32; 4]; 12],
    storms: [[f32; 4]; 12],
    palette: [[f32; 4]; 4],
    flags: [f32; 4],
}

fn on_sphere(rng: &mut Rng) -> [f32; 3] {
    let z = rng.range(-1.0, 1.0);
    let phi = rng.range(0.0, std::f64::consts::TAU);
    let r = (1.0 - z * z).max(0.0).sqrt();
    [(r * phi.cos()) as f32, z as f32, (r * phi.sin()) as f32]
}

fn params(kind: Kind, seed: u64, palette: [[f32; 3]; 4]) -> GenParams {
    let mut rng = Rng::new(seed);
    let offsets = [(); 4].map(|_| {
        [
            rng.range(-60.0, 60.0) as f32,
            rng.range(-60.0, 60.0) as f32,
            rng.range(-60.0, 60.0) as f32,
            rng.range(0.0, 8.0) as f32,
        ]
    });
    let plates = [(); 12].map(|_| {
        let [x, y, z] = on_sphere(&mut rng);
        let w = if rng.chance(0.42) {
            rng.range(0.35, 1.0)
        } else {
            rng.range(-1.0, -0.25)
        };
        [x, y, z, w as f32]
    });
    let storms = [(); 12].map(|_| {
        let tropical = rng.chance(0.3);
        let lat = if tropical {
            rng.range(10.0, 24.0)
        } else {
            rng.range(35.0, 64.0)
        }
        .to_radians();
        let north = rng.chance(0.5);
        let lat = if north { lat } else { -lat };
        let lon = rng.range(0.0, std::f64::consts::TAU);
        let strength = rng.range(1.2, 2.6) * if north { 1.0 } else { -1.0 };
        [
            (lat.cos() * lon.sin()) as f32,
            lat.sin() as f32,
            (lat.cos() * lon.cos()) as f32,
            strength as f32,
        ]
    });
    let warmth = match kind {
        Kind::Ice | Kind::IcyMoon | Kind::Geyser | Kind::Nitrogen => -0.3,
        Kind::Desert => -0.05,
        Kind::Lava | Kind::Volcanic => 0.6,
        _ => 0.0,
    };
    GenParams {
        size: [0.0, 0.0, 0.0, kind as u32 as f32],
        offsets,
        plates,
        storms,
        palette: palette.map(|c| [c[0], c[1], c[2], 1.0]),
        flags: [
            0.0,
            if kind.has_cities() { 1.0 } else { 0.0 },
            if cratered(kind) { 1.0 } else { 0.0 },
            warmth,
        ],
    }
}

/// The height below which `share` of the sphere lies, rows weighted by area.
fn sea_level(heights: &[f32], width: u32, height: u32, share: f64) -> f32 {
    let mut weighted: Vec<(f32, f64)> = Vec::with_capacity(heights.len());
    for y in 0..height {
        let lat = (0.5 - (y as f64 + 0.5) / height as f64) * std::f64::consts::PI;
        for x in 0..width {
            weighted.push((heights[(y * width + x) as usize], lat.cos()));
        }
    }
    weighted.sort_by(|a, b| a.0.total_cmp(&b.0));
    let total: f64 = weighted.iter().map(|(_, w)| w).sum();
    let mut sum = 0.0;
    for (h, w) in &weighted {
        sum += w;
        if sum >= total * share {
            return *h;
        }
    }
    weighted.last().map_or(0.0, |(h, _)| *h)
}

/// One planet slot's textures, allocated once and repainted for each planet.
pub struct SlotMaps {
    pub day: wgpu::Texture,
    pub relief: wgpu::Texture,
    pub extra: wgpu::Texture,
    pub moon_day: wgpu::Texture,
    pub moon_relief: wgpu::Texture,
}

/// Where a pass draws.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Target {
    Terrain,
    Climate,
    Relief,
    Day,
    Extra,
    MoonTerrain,
    MoonClimate,
    MoonRelief(u32),
    MoonDay(u32),
}

/// One pass of a job: a pipeline drawn over a target a band of rows at a
/// time, and the mip chains to build once it is done.
#[derive(Clone, Debug)]
struct Pass {
    pipeline: &'static str,
    target: Target,
    /// Clear the target first, or draw over (blend into) what is there.
    clear: bool,
    /// For a moon's pass, which moon (its parameters and its layer).
    moon: Option<u32>,
    mips_after: Vec<Target>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Probe,
    AwaitProbe,
    /// The pass at this index of the plan, and the next row to draw.
    Pass(usize, u32),
    Done,
}

struct Job {
    slot: usize,
    key: Key,
    kind: Kind,
    params: GenParams,
    lowland: f64,
    moons: Vec<Moon>,
    plan: Vec<Pass>,
    step: Step,
    mapped: Option<Arc<AtomicBool>>,
}

struct SkyJob {
    index: usize,
    row: u32,
    params: GenParams,
}

/// Paints maps into slots on request, a step per frame.
pub struct Streamer {
    maps: [SlotMaps; 2],
    /// What each slot holds, and whether it is finished.
    owner: [Option<(Key, bool)>; 2],
    skies: [wgpu::Texture; 2],
    sky_owner: [Option<(u64, bool)>; 2],
    job: Option<Job>,
    sky_job: Option<SkyJob>,
    /// Bumped whenever what the frame shader should bind changes.
    pub version: u64,
    rows: u32,
    /// The planet maps' width and height (`map_size`).
    size: (u32, u32),

    // Shared machinery.
    terrain: wgpu::Texture,
    climate: wgpu::Texture,
    moon_terrain: wgpu::Texture,
    moon_climate: wgpu::Texture,
    /// Bound in place of a map a pass is drawing into.
    placeholder: wgpu::Texture,
    probe: wgpu::Texture,
    readback: wgpu::Buffer,
    buffers: HashMap<&'static str, wgpu::Buffer>,
    moon_buffers: Vec<wgpu::Buffer>,
    height_layout: wgpu::BindGroupLayout,
    surface_layout: wgpu::BindGroupLayout,
    pipelines: HashMap<&'static str, wgpu::RenderPipeline>,
    /// Each pipeline's label, for the Pi estimate's fragment counts.
    labels: HashMap<&'static str, String>,
    terrain_sampler: wgpu::Sampler,
    noise: crate::noise3d::Noise3d,
    mips: MipChain,
}

/// Airless and icy worlds, with craters.
fn cratered(k: Kind) -> bool {
    matches!(
        k,
        Kind::Barren
            | Kind::Captured
            | Kind::IcyMoon
            | Kind::Desert
            | Kind::Volcanic
            | Kind::Geyser
            | Kind::TwoFaced
            | Kind::Nitrogen
            | Kind::Rusty
    )
}

/// The pipeline that paints `kind`'s day map: one per family of kinds.
fn day_pipeline(kind: Kind) -> &'static str {
    match kind {
        Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter => "day0",
        Kind::Cloudy => "day1",
        Kind::Hazy => "day2",
        Kind::Terran | Kind::Ocean | Kind::Jungle | Kind::Eyeball => "day3",
        Kind::Desert => "day4",
        Kind::Ice => "day5",
        Kind::Lava => "day6",
        Kind::Volcanic => "day7",
        Kind::IcyMoon => "day8",
        Kind::Geyser => "day9",
        Kind::TwoFaced => "day10",
        Kind::Nitrogen => "day11",
        Kind::Barren | Kind::Captured | Kind::Rusty => "day12",
    }
}

/// The pipelines that fill the scratch climate map for `kind`, in order (the
/// first clears it): lava and ice worlds keep their crust there instead
/// (`fs_climate`).
fn climate_pipelines(kind: Kind) -> &'static [&'static str] {
    match kind {
        Kind::Lava => &["climate1", "climate5", "climate2"],
        Kind::Ice => &["climate3", "climate4"],
        Kind::Desert => &["climate", "climate6"],
        _ => &["climate"],
    }
}

/// The passes that paint a planet of `kind` and its `moons`, in order.
fn plan(kind: Kind, moons: &[Moon]) -> Vec<Pass> {
    let pass = |pipeline, target, clear| Pass {
        pipeline,
        target,
        clear,
        moon: None,
        mips_after: Vec::new(),
    };
    let ground = !matches!(
        kind,
        Kind::GasGiant | Kind::IceGiant | Kind::HotJupiter | Kind::Hazy | Kind::Cloudy
    );
    let mut plan = vec![pass("height", Target::Terrain, true)];
    if ground {
        plan.push(pass("erosion", Target::Terrain, false));
    }
    if matches!(
        kind,
        Kind::Terran | Kind::Ocean | Kind::Jungle | Kind::Eyeball
    ) {
        plan.push(pass("rivers", Target::Terrain, false));
    }
    if cratered(kind) {
        for craters in ["craters0", "craters1", "craters2"] {
            plan.push(pass(craters, Target::Terrain, false));
        }
    }
    if let Some(last) = plan.last_mut() {
        last.mips_after.push(Target::Terrain);
    }
    for (n, climate) in climate_pipelines(kind).iter().enumerate() {
        plan.push(pass(climate, Target::Climate, n == 0));
    }
    plan.push(pass("relief", Target::Relief, true));
    plan.push(pass(day_pipeline(kind), Target::Day, true));
    let lights = match kind {
        Kind::Terran | Kind::Ocean | Kind::Jungle | Kind::Eyeball => "lights0",
        Kind::Lava => "lights1",
        Kind::HotJupiter => "lights2",
        _ => "lights3",
    };
    plan.push(pass(lights, Target::Extra, true));
    if kind == Kind::Lava {
        plan.push(pass("lights4", Target::Extra, false));
    }
    if kind.has_cities() {
        plan.push(pass("towns", Target::Extra, false));
    }
    let clouds = match kind {
        Kind::Terran | Kind::Ocean | Kind::Jungle | Kind::Eyeball => "clouds0",
        _ => "clouds1",
    };
    let mut clouds = pass(clouds, Target::Extra, false);
    clouds.mips_after = vec![Target::Day, Target::Relief, Target::Extra];
    plan.push(clouds);

    for (i, moon) in moons.iter().enumerate() {
        let i = i as u32;
        let moon_pass = |pipeline, target, clear| Pass {
            pipeline,
            target,
            clear,
            moon: Some(i),
            mips_after: Vec::new(),
        };
        let start = plan.len();
        plan.push(moon_pass("height", Target::MoonTerrain, true));
        plan.push(moon_pass("erosion", Target::MoonTerrain, false));
        if cratered(moon.kind) {
            for craters in ["craters0", "craters1", "craters2"] {
                plan.push(moon_pass(craters, Target::MoonTerrain, false));
            }
        }
        if let Some(last) = plan.last_mut() {
            last.mips_after.push(Target::MoonTerrain);
        }
        for (n, climate) in climate_pipelines(moon.kind).iter().enumerate() {
            plan.push(moon_pass(climate, Target::MoonClimate, n == 0));
        }
        plan.push(moon_pass("relief", Target::MoonRelief(i), true));
        if moon.kind == Kind::Volcanic {
            plan.push(moon_pass("glow", Target::MoonRelief(i), false));
        }
        let mut day = moon_pass(day_pipeline(moon.kind), Target::MoonDay(i), true);
        day.mips_after = vec![Target::MoonDay(i), Target::MoonRelief(i)];
        plan.push(day);
        debug_assert!(plan.len() > start);
    }
    plan
}

fn map_texture(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    layers: u32,
    format: wgpu::TextureFormat,
    mips: bool,
) -> wgpu::Texture {
    let levels = if mips {
        width.max(height).ilog2() + 1
    } else {
        1
    };
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: levels,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// A single-level, single-layer view.
fn level_view(texture: &wgpu::Texture, level: u32, layer: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2),
        base_mip_level: level,
        mip_level_count: Some(1),
        base_array_layer: layer,
        array_layer_count: Some(1),
        ..Default::default()
    })
}

pub fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Uniform,
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

pub fn texture_entry(
    binding: u32,
    dimension: wgpu::TextureViewDimension,
) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dimension,
            multisampled: false,
        },
        count: None,
    }
}

pub fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

impl Streamer {
    pub fn new(gpu: &Graphics) -> Self {
        let device = &gpu.device;
        let (map_width, map_height) = map_size(device.limits().max_texture_dimension_2d);
        tracing::info!(map_width, map_height, "solar planet maps");
        let slot = |n: usize| SlotMaps {
            day: map_texture(
                device,
                &format!("Solar day {n}"),
                map_width,
                map_height,
                1,
                DAY,
                true,
            ),
            relief: map_texture(
                device,
                &format!("Solar relief {n}"),
                map_width,
                map_height,
                1,
                RELIEF,
                true,
            ),
            extra: map_texture(
                device,
                &format!("Solar extra {n}"),
                map_width,
                map_height,
                1,
                EXTRA,
                true,
            ),
            moon_day: map_texture(
                device,
                &format!("Solar moon day {n}"),
                MOON_WIDTH,
                MOON_HEIGHT,
                MOONS,
                DAY,
                true,
            ),
            moon_relief: map_texture(
                device,
                &format!("Solar moon relief {n}"),
                MOON_WIDTH,
                MOON_HEIGHT,
                MOONS,
                RELIEF,
                true,
            ),
        };
        let sky = |n: usize| {
            map_texture(
                device,
                &format!("Solar sky {n}"),
                SKY_WIDTH,
                SKY_HEIGHT,
                1,
                SKY,
                false,
            )
        };

        let noise = crate::noise3d::Noise3d::new(device, &gpu.queue);
        let mut source = String::from(include_str!("noise.wgsl"));
        source.push_str(crate::noise3d::WGSL);
        source.push_str(include_str!("generate.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar generator"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let height_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar height"),
            entries: &[uniform_entry(0)],
        });
        let d2 = wgpu::TextureViewDimension::D2;
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar surface"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, d2),
                sampler_entry(2),
                texture_entry(3, d2),
                texture_entry(4, d2),
            ],
        });
        let add = wgpu::BlendState {
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
        };
        let max = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Max,
            },
            alpha: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::Zero,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
        };
        let pipeline = |label: &str,
                        layout: &wgpu::BindGroupLayout,
                        entry: &str,
                        format: wgpu::TextureFormat,
                        blend: Option<wgpu::BlendState>,
                        write_mask: wgpu::ColorWrites,
                        group: i32| {
            let constants = [("GROUP", group as f64)];
            let options = wgpu::PipelineCompilationOptions {
                constants: &constants,
                ..Default::default()
            };
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout), Some(&noise.layout)],
                immediate_size: 0,
            });
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: options.clone(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        blend,
                        write_mask,
                    })],
                    compilation_options: options,
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let all = wgpu::ColorWrites::ALL;
        let h = &height_layout;
        let sl = &surface_layout;
        let mut pipelines = HashMap::new();
        let mut labels = HashMap::new();
        let red = wgpu::ColorWrites::RED;
        let green = wgpu::ColorWrites::GREEN;
        let rgb = wgpu::ColorWrites::COLOR;
        let alpha = wgpu::ColorWrites::ALPHA;
        let red_green = wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN;
        let blue_alpha = wgpu::ColorWrites::BLUE | wgpu::ColorWrites::ALPHA;
        for (name, layout, entry, format, blend, mask, group) in [
            ("probe", h, "fs_height", PROBE, None, all, 0),
            ("height", h, "fs_height", HEIGHT, None, all, 0),
            ("erosion", h, "fs_erosion", HEIGHT, Some(add), all, 0),
            ("rivers", h, "fs_erosion", HEIGHT, Some(add), all, 1),
            ("craters0", h, "fs_craters", HEIGHT, Some(add), all, 0),
            ("craters1", h, "fs_craters", HEIGHT, Some(add), all, 1),
            ("craters2", h, "fs_craters", HEIGHT, Some(add), all, 2),
            ("sky", h, "fs_sky", SKY, None, all, 0),
            ("climate", sl, "fs_climate", CLIMATE, None, all, 0),
            ("climate1", sl, "fs_climate", CLIMATE, None, rgb, 1),
            ("climate2", sl, "fs_climate", CLIMATE, None, alpha, 2),
            ("climate6", sl, "fs_climate", CLIMATE, None, rgb, 6),
            (
                "climate5",
                sl,
                "fs_climate",
                CLIMATE,
                None,
                wgpu::ColorWrites::BLUE,
                5,
            ),
            ("climate3", sl, "fs_climate", CLIMATE, None, red_green, 3),
            ("climate4", sl, "fs_climate", CLIMATE, None, blue_alpha, 4),
            ("relief", sl, "fs_relief", RELIEF, None, all, 0),
            ("day0", sl, "fs_day", DAY, None, all, 0),
            ("day1", sl, "fs_day", DAY, None, all, 1),
            ("day2", sl, "fs_day", DAY, None, all, 2),
            ("day3", sl, "fs_day", DAY, None, all, 3),
            ("day4", sl, "fs_day", DAY, None, all, 4),
            ("day5", sl, "fs_day", DAY, None, all, 5),
            ("day6", sl, "fs_day", DAY, None, all, 6),
            ("day7", sl, "fs_day", DAY, None, all, 7),
            ("day8", sl, "fs_day", DAY, None, all, 8),
            ("day9", sl, "fs_day", DAY, None, all, 9),
            ("day10", sl, "fs_day", DAY, None, all, 10),
            ("day11", sl, "fs_day", DAY, None, all, 11),
            ("day12", sl, "fs_day", DAY, None, all, 12),
            ("lights0", sl, "fs_lights", EXTRA, None, red, 0),
            ("lights1", sl, "fs_lights", EXTRA, None, red, 1),
            ("lights2", sl, "fs_lights", EXTRA, None, red, 2),
            ("lights3", sl, "fs_lights", EXTRA, None, red, 3),
            ("lights4", sl, "fs_lights", EXTRA, Some(max), red, 4),
            ("towns", sl, "fs_towns", EXTRA, Some(add), red, 0),
            ("clouds0", sl, "fs_clouds", EXTRA, None, green, 0),
            ("clouds1", sl, "fs_clouds", EXTRA, None, green, 1),
            (
                "glow",
                sl,
                "fs_glow",
                RELIEF,
                None,
                wgpu::ColorWrites::BLUE,
                0,
            ),
        ] {
            labels.insert(name, format!("Solar {name}"));
            pipelines.insert(
                name,
                pipeline(
                    &format!("Solar {name}"),
                    layout,
                    entry,
                    format,
                    blend,
                    mask,
                    group,
                ),
            );
        }

        let buffer = |label: &'static str| {
            device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: std::mem::size_of::<GenParams>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            })
        };
        let mut buffers = HashMap::new();
        for name in ["probe", "planet", "sky"] {
            buffers.insert(name, buffer(name));
        }
        let moon_buffers = (0..MOONS).map(|_| buffer("moon")).collect();
        let padded = (PROBE_WIDTH * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let readback = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Solar probe readback"),
            size: padded as u64 * PROBE_HEIGHT as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let terrain_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Solar terrain sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        });
        // By default as many texels a band as 8 rows of a 2048-wide map.
        let rows = std::env::var("SOLAR_GEN_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or((8 * MAP_WIDTH / map_width).max(1))
            .clamp(1, map_height);
        Self {
            maps: [slot(0), slot(1)],
            owner: [None, None],
            skies: [sky(0), sky(1)],
            sky_owner: [None, None],
            job: None,
            sky_job: None,
            version: 0,
            rows,
            size: (map_width, map_height),
            terrain: map_texture(
                device,
                "Solar terrain",
                map_width,
                map_height,
                1,
                HEIGHT,
                true,
            ),
            climate: map_texture(
                device,
                "Solar climate",
                map_width,
                map_height,
                1,
                CLIMATE,
                false,
            ),
            moon_terrain: map_texture(
                device,
                "Solar moon terrain",
                MOON_WIDTH,
                MOON_HEIGHT,
                1,
                HEIGHT,
                true,
            ),
            moon_climate: map_texture(
                device,
                "Solar moon climate",
                MOON_WIDTH,
                MOON_HEIGHT,
                1,
                CLIMATE,
                false,
            ),
            placeholder: map_texture(device, "Solar placeholder", 1, 1, 1, CLIMATE, false),
            probe: map_texture(
                device,
                "Solar probe",
                PROBE_WIDTH,
                PROBE_HEIGHT,
                1,
                PROBE,
                false,
            ),
            readback,
            buffers,
            moon_buffers,
            height_layout,
            surface_layout,
            pipelines,
            labels,
            terrain_sampler,
            noise,
            mips: MipChain::new(device),
        }
    }

    /// The noise volume, shared with the frame's shaders.
    pub fn noise(&self) -> &crate::noise3d::Noise3d {
        &self.noise
    }

    pub fn slot_maps(&self, slot: usize) -> &SlotMaps {
        &self.maps[slot]
    }

    pub fn sky(&self, index: usize) -> &wgpu::Texture {
        &self.skies[index]
    }

    /// The slot holding `key`, if it is finished.
    pub fn ready(&self, key: Key) -> Option<usize> {
        (0..2).find(|&i| self.owner[i] == Some((key, true)))
    }

    /// The finished sky for `seed`, if any.
    pub fn sky_ready(&self, seed: u64) -> Option<usize> {
        (0..2).find(|&i| self.sky_owner[i] == Some((seed, true)))
    }

    /// Whether anything is still being painted.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn busy(&self) -> bool {
        self.job.is_some() || self.sky_job.is_some()
    }

    /// Ask for these planets (most wanted first) and skies to be resident.
    /// Starts painting the first one missing, overwriting a slot nothing
    /// wants any more.
    pub fn want(&mut self, planets: &[(Key, &Planet)], skies: &[(u64, [f32; 4])]) {
        if let Some(job) = &self.job
            && !planets.iter().any(|(key, _)| *key == job.key)
        {
            let slot = job.slot;
            self.owner[slot] = None;
            self.job = None;
        }
        if self.job.is_none() {
            for (key, planet) in planets {
                if self.owner.iter().any(|o| o.map(|(k, _)| k) == Some(*key)) {
                    continue;
                }
                let free = (0..2).find(|&i| match self.owner[i] {
                    None => true,
                    Some((k, _)) => !planets.iter().any(|(w, _)| *w == k),
                });
                let Some(slot) = free else {
                    break;
                };
                self.owner[slot] = Some((*key, false));
                self.version += 1;
                tracing::debug!(?key, slot, kind = ?planet.kind, "streaming a planet");
                let moons: Vec<Moon> = planet.moons.iter().take(MOONS as usize).cloned().collect();
                self.job = Some(Job {
                    slot,
                    key: *key,
                    kind: planet.kind,
                    params: params(planet.kind, planet.seed, planet.palette),
                    lowland: planet.kind.lowland(),
                    plan: plan(planet.kind, &moons),
                    moons,
                    step: if planet.kind.lowland() > 0.0 {
                        Step::Probe
                    } else {
                        Step::Pass(0, 0)
                    },
                    mapped: None,
                });
                break;
            }
        }
        if self.sky_job.is_none() {
            for (seed, pole) in skies {
                if self
                    .sky_owner
                    .iter()
                    .any(|o| o.map(|(s, _)| s) == Some(*seed))
                {
                    continue;
                }
                let free = (0..2).find(|&i| match self.sky_owner[i] {
                    None => true,
                    Some((s, _)) => !skies.iter().any(|(w, _)| *w == s),
                });
                let Some(index) = free else {
                    break;
                };
                self.sky_owner[index] = Some((*seed, false));
                let mut p = params(Kind::Barren, *seed, [[pole[0], pole[1], pole[2]]; 4]);
                p.size = [SKY_WIDTH as f32, SKY_HEIGHT as f32, 0.0, 0.0];
                p.palette[0] = [pole[0], pole[1], pole[2], 0.0];
                self.sky_job = Some(SkyJob {
                    index,
                    row: 0,
                    params: p,
                });
                break;
            }
        }
    }

    /// The texture behind a target, and the layer it draws into.
    fn texture_of(&self, slot: usize, target: Target) -> (&wgpu::Texture, u32) {
        let maps = &self.maps[slot];
        match target {
            Target::Terrain => (&self.terrain, 0),
            Target::Climate => (&self.climate, 0),
            Target::Relief => (&maps.relief, 0),
            Target::Day => (&maps.day, 0),
            Target::Extra => (&maps.extra, 0),
            Target::MoonTerrain => (&self.moon_terrain, 0),
            Target::MoonClimate => (&self.moon_climate, 0),
            Target::MoonRelief(i) => (&maps.moon_relief, i),
            Target::MoonDay(i) => (&maps.moon_day, i),
        }
    }

    fn format_of(target: Target) -> wgpu::TextureFormat {
        match target {
            Target::Terrain | Target::MoonTerrain => HEIGHT,
            Target::Climate | Target::MoonClimate => CLIMATE,
            Target::Relief | Target::MoonRelief(_) => RELIEF,
            Target::Day | Target::MoonDay(_) => DAY,
            Target::Extra => EXTRA,
        }
    }

    /// Run one frame's share of painting, recorded into `encoder`.
    pub fn advance(&mut self, gpu: &Graphics, encoder: &mut wgpu::CommandEncoder) {
        self.advance_sky(gpu, encoder);
        let Some(mut job) = self.job.take() else {
            return;
        };
        let device = &gpu.device;
        match job.step {
            Step::Probe => {
                let mut p = job.params;
                p.size = [
                    PROBE_WIDTH as f32,
                    PROBE_HEIGHT as f32,
                    0.0,
                    job.kind as u32 as f32,
                ];
                gpu.queue
                    .write_buffer(&self.buffers["probe"], 0, bytemuck::bytes_of(&p));
                let group = self.height_group(device, &self.buffers["probe"]);
                draw(
                    encoder,
                    (self.labels["probe"].as_str(), &self.pipelines["probe"]),
                    &group,
                    &self.noise.group,
                    &level_view(&self.probe, 0, 0),
                    0,
                    PROBE_HEIGHT,
                    PROBE_WIDTH,
                    true,
                );
                let padded = (PROBE_WIDTH * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                    * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                encoder.copy_texture_to_buffer(
                    wgpu::TexelCopyTextureInfo {
                        texture: &self.probe,
                        mip_level: 0,
                        origin: wgpu::Origin3d::ZERO,
                        aspect: wgpu::TextureAspect::All,
                    },
                    wgpu::TexelCopyBufferInfo {
                        buffer: &self.readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(padded),
                            rows_per_image: Some(PROBE_HEIGHT),
                        },
                    },
                    wgpu::Extent3d {
                        width: PROBE_WIDTH,
                        height: PROBE_HEIGHT,
                        depth_or_array_layers: 1,
                    },
                );
                job.step = Step::AwaitProbe;
            }
            Step::AwaitProbe => {
                // The copy was submitted with last frame's work: ask for it,
                // then look again each frame without ever waiting.
                let flag = job.mapped.get_or_insert_with(|| {
                    let flag = Arc::new(AtomicBool::new(false));
                    let done = flag.clone();
                    self.readback
                        .slice(..)
                        .map_async(wgpu::MapMode::Read, move |_| {
                            done.store(true, Ordering::SeqCst);
                        });
                    flag
                });
                let _ = device.poll(wgpu::PollType::Poll);
                if flag.load(Ordering::SeqCst) {
                    let padded = (PROBE_WIDTH * 16).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
                        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
                    let mut heights = Vec::with_capacity((PROBE_WIDTH * PROBE_HEIGHT) as usize);
                    if let Ok(data) = self.readback.slice(..).get_mapped_range() {
                        for y in 0..PROBE_HEIGHT as usize {
                            let row = &data[y * padded as usize..][..(PROBE_WIDTH * 16) as usize];
                            for texel in row.chunks_exact(16) {
                                heights.push(f32::from_le_bytes([
                                    texel[0], texel[1], texel[2], texel[3],
                                ]));
                            }
                        }
                    }
                    self.readback.unmap();
                    job.params.size[2] =
                        sea_level(&heights, PROBE_WIDTH, PROBE_HEIGHT, job.lowland);
                    job.mapped = None;
                    job.step = Step::Pass(0, 0);
                }
            }
            Step::Pass(index, row) => {
                let pass = job.plan[index].clone();
                // Each pass's parameters: the planet's, or its moon's.
                let (buffer, width, height) = match pass.moon {
                    None => {
                        let mut p = job.params;
                        p.size[0] = self.size.0 as f32;
                        p.size[1] = self.size.1 as f32;
                        p.flags[0] = (self.size.0 as f32 / MAP_WIDTH as f32).log2();
                        if job.lowland <= 0.0 {
                            p.size[2] = -10.0;
                        }
                        gpu.queue
                            .write_buffer(&self.buffers["planet"], 0, bytemuck::bytes_of(&p));
                        (&self.buffers["planet"], self.size.0, self.size.1)
                    }
                    Some(i) => {
                        let moon = &job.moons[i as usize];
                        let mut p = params(moon.kind, moon.seed, moon.palette);
                        let sea = if moon.kind == Kind::Barren {
                            -0.04
                        } else {
                            -10.0
                        };
                        p.size = [
                            MOON_WIDTH as f32,
                            MOON_HEIGHT as f32,
                            sea,
                            moon.kind as u32 as f32,
                        ];
                        let buffer = &self.moon_buffers[i as usize];
                        gpu.queue.write_buffer(buffer, 0, bytemuck::bytes_of(&p));
                        (buffer, MOON_WIDTH, MOON_HEIGHT)
                    }
                };
                let moon = pass.moon.is_some();
                let terrain = if moon {
                    &self.moon_terrain
                } else {
                    &self.terrain
                };
                let climate = if moon {
                    &self.moon_climate
                } else {
                    &self.climate
                };
                let relief = match pass.moon {
                    Some(i) => level_view(&self.maps[job.slot].moon_relief, 0, i),
                    None => level_view(&self.maps[job.slot].relief, 0, 0),
                };
                let placeholder = level_view(&self.placeholder, 0, 0);
                let group = match pass.target {
                    Target::Terrain | Target::MoonTerrain => self.height_group(device, buffer),
                    Target::Climate | Target::MoonClimate => {
                        self.surface_group(device, buffer, terrain, &placeholder, &placeholder)
                    }
                    Target::Relief | Target::MoonRelief(_) => self.surface_group(
                        device,
                        buffer,
                        terrain,
                        &level_view(climate, 0, 0),
                        &placeholder,
                    ),
                    _ => self.surface_group(
                        device,
                        buffer,
                        terrain,
                        &level_view(climate, 0, 0),
                        &relief,
                    ),
                };
                // Moons are small: a whole pass a frame.
                let count = if moon {
                    height
                } else {
                    self.rows.min(height - row)
                };
                let (texture, layer) = self.texture_of(job.slot, pass.target);
                draw(
                    encoder,
                    (
                        self.labels[pass.pipeline].as_str(),
                        &self.pipelines[pass.pipeline],
                    ),
                    &group,
                    &self.noise.group,
                    &level_view(texture, 0, layer),
                    row,
                    count,
                    width,
                    row == 0 && pass.clear,
                );
                if row + count >= height {
                    for target in &pass.mips_after {
                        let (texture, layer) = self.texture_of(job.slot, *target);
                        self.mips
                            .build(device, encoder, texture, layer, Self::format_of(*target));
                    }
                    job.step = if index + 1 < job.plan.len() {
                        Step::Pass(index + 1, 0)
                    } else {
                        Step::Done
                    };
                } else {
                    job.step = Step::Pass(index, row + count);
                }
            }
            Step::Done => {}
        }
        if job.step == Step::Done {
            self.owner[job.slot] = Some((job.key, true));
            self.version += 1;
            tracing::debug!(key = ?job.key, slot = job.slot, "planet streamed in");
        } else {
            self.job = Some(job);
        }
    }

    fn advance_sky(&mut self, gpu: &Graphics, encoder: &mut wgpu::CommandEncoder) {
        let Some(mut job) = self.sky_job.take() else {
            return;
        };
        gpu.queue
            .write_buffer(&self.buffers["sky"], 0, bytemuck::bytes_of(&job.params));
        let group = self.height_group(&gpu.device, &self.buffers["sky"]);
        let count = (self.rows * 2).min(SKY_HEIGHT - job.row);
        draw(
            encoder,
            (self.labels["sky"].as_str(), &self.pipelines["sky"]),
            &group,
            &self.noise.group,
            &level_view(&self.skies[job.index], 0, 0),
            job.row,
            count,
            SKY_WIDTH,
            job.row == 0,
        );
        job.row += count;
        if job.row >= SKY_HEIGHT {
            if let Some((seed, _)) = self.sky_owner[job.index] {
                self.sky_owner[job.index] = Some((seed, true));
            }
            self.version += 1;
        } else {
            self.sky_job = Some(job);
        }
    }

    fn height_group(&self, device: &wgpu::Device, buffer: &wgpu::Buffer) -> wgpu::BindGroup {
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar height"),
            layout: &self.height_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            }],
        })
    }

    fn surface_group(
        &self,
        device: &wgpu::Device,
        buffer: &wgpu::Buffer,
        terrain: &wgpu::Texture,
        climate: &wgpu::TextureView,
        relief: &wgpu::TextureView,
    ) -> wgpu::BindGroup {
        let view = terrain.create_view(&wgpu::TextureViewDescriptor::default());
        device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Solar surface"),
            layout: &self.surface_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&self.terrain_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(climate),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(relief),
                },
            ],
        })
    }
}

/// A full-target pass limited to `count` rows from `row`.
#[allow(clippy::too_many_arguments)]
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    (label, pipeline): (&str, &wgpu::RenderPipeline),
    group: &wgpu::BindGroup,
    noise: &wgpu::BindGroup,
    target: &wgpu::TextureView,
    row: u32,
    count: u32,
    width: u32,
    clear: bool,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Solar generator pass"),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                load: if clear {
                    wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT)
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
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.set_bind_group(1, noise, &[]);
    pass.set_scissor_rect(0, row, width, count);
    crate::gpu::counted_draw(&mut pass, label, (width * count) as u64, 0..3, 0..1);
}

/// Builds mip chains on the GPU, one layer at a time.
struct MipChain {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    sampler: wgpu::Sampler,
}

impl MipChain {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar mip chain"),
            entries: &[
                texture_entry(0, wgpu::TextureViewDimension::D2),
                sampler_entry(1),
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar mip chain"),
            source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Solar mip chain"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipelines = [DAY, RELIEF, EXTRA, HEIGHT]
            .into_iter()
            .map(|format| {
                let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some("Solar mip chain"),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some("vs_main"),
                        buffers: &[],
                        compilation_options: Default::default(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some("fs_main"),
                        targets: &[Some(wgpu::ColorTargetState {
                            format,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: Default::default(),
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                });
                (format, pipeline)
            })
            .collect();
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Solar mip chain"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            pipelines,
            sampler,
        }
    }

    fn build(
        &self,
        device: &wgpu::Device,
        encoder: &mut wgpu::CommandEncoder,
        texture: &wgpu::Texture,
        layer: u32,
        format: wgpu::TextureFormat,
    ) {
        let Some((_, pipeline)) = self.pipelines.iter().find(|(f, _)| *f == format) else {
            return;
        };
        for level in 1..texture.mip_level_count() {
            let source = level_view(texture, level - 1, layer);
            let target = level_view(texture, level, layer);
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Solar mip chain"),
                layout: &self.layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                ],
            });
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Solar mip chain"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &group, &[]);
            let size = target.texture().size();
            let covered = ((size.width >> level).max(1) * (size.height >> level).max(1)) as u64;
            crate::gpu::counted_draw(&mut pass, "Solar mip chain", covered, 0..3, 0..1);
        }
    }
}

const MIP_SHADER: &str = r#"
struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    let corner = corners[index];
    var out: Varying;
    out.position = vec4<f32>(corner, 0.0, 1.0);
    out.uv = corner * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

@group(0) @binding(0) var source: texture_2d<f32>;
@group(0) @binding(1) var source_sampler: sampler;

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    return textureSample(source, source_sampler, in.uv);
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_level_splits_the_sphere() {
        let (w, h) = (8, 64);
        let heights: Vec<f32> = (0..w * h)
            .map(|i| ((i / w) as f32 / (h - 1) as f32) * 2.0 - 1.0)
            .collect();
        assert!(sea_level(&heights, w, h, 0.5).abs() < 0.05);
    }

    /// Level 0 of a single-layer map, tightly packed.
    fn read_map(gpu: &Graphics, texture: &wgpu::Texture, texel: u32) -> Vec<u8> {
        let size = texture.size();
        let row = size.width * texel;
        let padded =
            row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: padded as u64 * size.height as u64,
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
                    rows_per_image: Some(size.height),
                },
            },
            wgpu::Extent3d {
                width: size.width,
                height: size.height,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(Some(encoder.finish()));
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let data = slice.get_mapped_range().expect("mapped");
        let mut out = Vec::with_capacity((row * size.height) as usize);
        for y in 0..size.height as usize {
            out.extend_from_slice(&data[y * padded as usize..][..row as usize]);
        }
        out
    }

    /// One world of each kind (`SOLAR_GALLERY_KINDS`, comma separated, to
    /// pick some), painted alone at the map size in use (`SOLAR_MAP_SIZE`):
    /// its day, relief and extra maps as PNGs in `SOLAR_GALLERY_DIR` (default
    /// `target/solar-gallery`), and in `times.txt` the frames each took and
    /// the wall time of each pass (on this machine's GPU; relative only).
    #[test]
    #[ignore = "writes maps; needs a working GPU adapter"]
    fn gallery() {
        use super::super::system::{Stop, System};
        let dir =
            std::env::var("SOLAR_GALLERY_DIR").unwrap_or_else(|_| "target/solar-gallery".into());
        let only = std::env::var("SOLAR_GALLERY_KINDS").ok();
        std::fs::create_dir_all(&dir).expect("a gallery directory");
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut streamer = Streamer::new(&gpu);
        let mut notes = format!(
            "map {} x {}, {} rows a band\n",
            streamer.size.0, streamer.size.1, streamer.rows
        );
        for kind in Kind::ALL {
            let name = format!("{kind:?}");
            if only
                .as_deref()
                .is_some_and(|o| !o.split(',').any(|k| k.eq_ignore_ascii_case(&name)))
            {
                continue;
            }
            // The first world of this kind among the systems: planets first.
            let found = (0..400u64).find_map(|seed| {
                let system = System::generate(seed);
                let stops = (0..system.planets.len()).flat_map(|p| {
                    std::iter::once(Stop {
                        planet: p,
                        moon: None,
                    })
                    .chain((0..system.planets[p].moons.len()).map(move |m| {
                        Stop {
                            planet: p,
                            moon: Some(m),
                        }
                    }))
                });
                let stops: Vec<Stop> = stops.collect();
                stops
                    .into_iter()
                    .map(|s| system.world(s))
                    .find(|w| w.kind == kind)
                    .map(|w| (seed, w))
            });
            let Some((seed, mut world)) = found else {
                notes.push_str(&format!("{name}: none found\n"));
                continue;
            };
            world.moons.clear();
            let key = (seed, 1000 + kind as usize);
            let mut times: Vec<(&'static str, f64, u32)> = Vec::new();
            let mut frames = 0u32;
            let start = std::time::Instant::now();
            while streamer.ready(key).is_none() && frames < 200_000 {
                streamer.want(&[(key, &world)], &[]);
                let pass = match streamer.job.as_ref().map(|j| (j.step, &j.plan)) {
                    Some((Step::Pass(i, _), plan)) => plan[i].pipeline,
                    _ => "probe",
                };
                let t = std::time::Instant::now();
                let mut encoder = gpu.device.create_command_encoder(&Default::default());
                streamer.advance(&gpu, &mut encoder);
                gpu.queue.submit(Some(encoder.finish()));
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                let ms = t.elapsed().as_secs_f64() * 1e3;
                match times.iter_mut().find(|(p, _, _)| *p == pass) {
                    Some(entry) => {
                        entry.1 += ms;
                        entry.2 += 1;
                    }
                    None => times.push((pass, ms, 1)),
                }
                frames += 1;
            }
            let total = start.elapsed().as_secs_f64() * 1e3;
            notes.push_str(&format!(
                "{name} (seed {seed}, {}): {frames} frames, {total:.0} ms\n",
                world.name
            ));
            for (pass, ms, n) in &times {
                notes.push_str(&format!("    {pass:>9} {ms:8.0} ms over {n} frames\n"));
            }
            let slot = streamer.ready(key).expect("painted");
            let maps = &streamer.maps[slot];
            let (w, h) = streamer.size;
            let save = |suffix: &str, data: Vec<u8>, colour| {
                let path = std::path::Path::new(&dir).join(format!("{name}-{suffix}.png"));
                image::save_buffer(&path, &data, w, h, colour).expect("a map");
            };
            save(
                "day",
                read_map(&gpu, &maps.day, 4),
                image::ExtendedColorType::Rgba8,
            );
            save(
                "relief",
                read_map(&gpu, &maps.relief, 4),
                image::ExtendedColorType::Rgba8,
            );
            let extra = read_map(&gpu, &maps.extra, 2);
            let extra: Vec<u8> = extra
                .chunks_exact(2)
                .flat_map(|c| [c[0], c[1], 0])
                .collect();
            save("extra", extra, image::ExtendedColorType::Rgb8);
        }
        std::fs::write(std::path::Path::new(&dir).join("times.txt"), notes).expect("the times");
    }
}
