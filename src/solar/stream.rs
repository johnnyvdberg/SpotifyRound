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
//! of 8 rows is about 16k texels a pass.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use slint::wgpu_30::wgpu;

use super::system::{Kind, Moon, Planet, Rng};
use crate::gpu::Graphics;

pub const MAP_WIDTH: u32 = 2048;
pub const MAP_HEIGHT: u32 = 1024;
pub const MOON_WIDTH: u32 = 512;
pub const MOON_HEIGHT: u32 = 256;
pub const MOONS: u32 = 6;
pub const SKY_WIDTH: u32 = 1024;
pub const SKY_HEIGHT: u32 = 512;
const PROBE_WIDTH: u32 = 128;
const PROBE_HEIGHT: u32 = 64;

/// Identifies a planet across systems: the system's seed and its index.
pub type Key = (u64, usize);

const DAY: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;
const RELIEF: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const EXTRA: wgpu::TextureFormat = wgpu::TextureFormat::Rg8Unorm;
const HEIGHT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const PROBE: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
const SKY: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

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
        flags: [1.0, if kind.has_cities() { 1.0 } else { 0.0 }, 0.0, warmth],
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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Step {
    Probe,
    AwaitProbe,
    Terrain(u32),
    TerrainMips,
    Surface(u32),
    SurfaceMips,
    MoonHeight(u32),
    MoonSurface(u32),
    Done,
}

struct Job {
    slot: usize,
    key: Key,
    kind: Kind,
    params: GenParams,
    lowland: f64,
    moons: Vec<Moon>,
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

    // Shared machinery.
    terrain: wgpu::Texture,
    moon_terrain: wgpu::Texture,
    probe: wgpu::Texture,
    readback: wgpu::Buffer,
    buffers: HashMap<&'static str, wgpu::Buffer>,
    moon_buffers: Vec<wgpu::Buffer>,
    height_layout: wgpu::BindGroupLayout,
    surface_layout: wgpu::BindGroupLayout,
    probe_pipeline: wgpu::RenderPipeline,
    height_pipeline: wgpu::RenderPipeline,
    planet_pipeline: wgpu::RenderPipeline,
    moon_pipeline: wgpu::RenderPipeline,
    sky_pipeline: wgpu::RenderPipeline,
    terrain_sampler: wgpu::Sampler,
    mips: MipChain,
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
        let slot = |n: usize| SlotMaps {
            day: map_texture(
                device,
                &format!("Solar day {n}"),
                MAP_WIDTH,
                MAP_HEIGHT,
                1,
                DAY,
                true,
            ),
            relief: map_texture(
                device,
                &format!("Solar relief {n}"),
                MAP_WIDTH,
                MAP_HEIGHT,
                1,
                RELIEF,
                true,
            ),
            extra: map_texture(
                device,
                &format!("Solar extra {n}"),
                MAP_WIDTH,
                MAP_HEIGHT,
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

        let mut source = String::from(include_str!("noise.wgsl"));
        source.push_str(include_str!("generate.wgsl"));
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Solar generator"),
            source: wgpu::ShaderSource::Wgsl(source.into()),
        });
        let height_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar height"),
            entries: &[uniform_entry(0)],
        });
        let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Solar surface"),
            entries: &[
                uniform_entry(0),
                texture_entry(1, wgpu::TextureViewDimension::D2),
                sampler_entry(2),
            ],
        });
        let pipeline = |label: &str,
                        layout: &wgpu::BindGroupLayout,
                        entry: &str,
                        formats: &[wgpu::TextureFormat]| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
            let targets: Vec<_> = formats
                .iter()
                .map(|&format| {
                    Some(wgpu::ColorTargetState {
                        format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })
                })
                .collect();
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[],
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    targets: &targets,
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let probe_pipeline = pipeline("Solar probe", &height_layout, "fs_height", &[PROBE]);
        let height_pipeline = pipeline("Solar height", &height_layout, "fs_height", &[HEIGHT]);
        let planet_pipeline = pipeline(
            "Solar planet",
            &surface_layout,
            "fs_planet",
            &[DAY, RELIEF, EXTRA],
        );
        let moon_pipeline = pipeline("Solar moon", &surface_layout, "fs_moon", &[DAY, RELIEF]);
        let sky_pipeline = pipeline("Solar sky", &height_layout, "fs_sky", &[SKY]);

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
        let rows = std::env::var("SOLAR_GEN_ROWS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8u32)
            .clamp(1, MAP_HEIGHT);
        Self {
            maps: [slot(0), slot(1)],
            owner: [None, None],
            skies: [sky(0), sky(1)],
            sky_owner: [None, None],
            job: None,
            sky_job: None,
            version: 0,
            rows,
            terrain: map_texture(
                device,
                "Solar terrain",
                MAP_WIDTH,
                MAP_HEIGHT,
                1,
                HEIGHT,
                true,
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
            probe_pipeline,
            height_pipeline,
            planet_pipeline,
            moon_pipeline,
            sky_pipeline,
            terrain_sampler,
            mips: MipChain::new(device),
        }
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
        // A job for something no longer wanted is dropped.
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
                self.job = Some(Job {
                    slot,
                    key: *key,
                    kind: planet.kind,
                    params: params(planet.kind, planet.seed, planet.palette),
                    lowland: planet.kind.lowland(),
                    moons: planet.moons.iter().take(MOONS as usize).cloned().collect(),
                    step: if planet.kind.lowland() > 0.0 {
                        Step::Probe
                    } else {
                        Step::Terrain(0)
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

    /// Run one frame's share of painting, recorded into `encoder`.
    pub fn advance(&mut self, gpu: &Graphics, encoder: &mut wgpu::CommandEncoder) {
        self.advance_sky(gpu, encoder);
        let Some(mut job) = self.job.take() else {
            return;
        };
        let device = &gpu.device;
        let rows = self.rows;
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
                    &self.probe_pipeline,
                    &group,
                    &[&level_view(&self.probe, 0, 0)],
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
                    job.step = Step::Terrain(0);
                }
            }
            Step::Terrain(row) => {
                let mut p = job.params;
                p.size[0] = MAP_WIDTH as f32;
                p.size[1] = MAP_HEIGHT as f32;
                if job.lowland <= 0.0 {
                    p.size[2] = -10.0;
                }
                job.params = p;
                gpu.queue
                    .write_buffer(&self.buffers["planet"], 0, bytemuck::bytes_of(&p));
                let group = self.height_group(device, &self.buffers["planet"]);
                let count = rows.min(MAP_HEIGHT - row);
                draw(
                    encoder,
                    &self.height_pipeline,
                    &group,
                    &[&level_view(&self.terrain, 0, 0)],
                    row,
                    count,
                    MAP_WIDTH,
                    row == 0,
                );
                job.step = if row + count >= MAP_HEIGHT {
                    Step::TerrainMips
                } else {
                    Step::Terrain(row + count)
                };
            }
            Step::TerrainMips => {
                self.mips.build(device, encoder, &self.terrain, 0, HEIGHT);
                job.step = Step::Surface(0);
            }
            Step::Surface(row) => {
                gpu.queue
                    .write_buffer(&self.buffers["planet"], 0, bytemuck::bytes_of(&job.params));
                let group = self.surface_group(device, &self.buffers["planet"], &self.terrain);
                let maps = &self.maps[job.slot];
                let count = (rows / 2).max(1).min(MAP_HEIGHT - row);
                draw(
                    encoder,
                    &self.planet_pipeline,
                    &group,
                    &[
                        &level_view(&maps.day, 0, 0),
                        &level_view(&maps.relief, 0, 0),
                        &level_view(&maps.extra, 0, 0),
                    ],
                    row,
                    count,
                    MAP_WIDTH,
                    row == 0,
                );
                job.step = if row + count >= MAP_HEIGHT {
                    Step::SurfaceMips
                } else {
                    Step::Surface(row + count)
                };
            }
            Step::SurfaceMips => {
                let maps = &self.maps[job.slot];
                self.mips.build(device, encoder, &maps.day, 0, DAY);
                self.mips.build(device, encoder, &maps.relief, 0, RELIEF);
                self.mips.build(device, encoder, &maps.extra, 0, EXTRA);
                job.step = if job.moons.is_empty() {
                    Step::Done
                } else {
                    Step::MoonHeight(0)
                };
            }
            Step::MoonHeight(i) => {
                let moon = &job.moons[i as usize];
                let mut p = params(moon.kind, moon.seed, moon.palette);
                p.size = [
                    MOON_WIDTH as f32,
                    MOON_HEIGHT as f32,
                    if moon.kind == Kind::Barren {
                        -0.04
                    } else {
                        -10.0
                    },
                    moon.kind as u32 as f32,
                ];
                let buffer = &self.moon_buffers[i as usize];
                gpu.queue.write_buffer(buffer, 0, bytemuck::bytes_of(&p));
                let group = self.height_group(device, buffer);
                draw(
                    encoder,
                    &self.height_pipeline,
                    &group,
                    &[&level_view(&self.moon_terrain, 0, 0)],
                    0,
                    MOON_HEIGHT,
                    MOON_WIDTH,
                    true,
                );
                self.mips
                    .build(device, encoder, &self.moon_terrain, 0, HEIGHT);
                job.step = Step::MoonSurface(i);
            }
            Step::MoonSurface(i) => {
                let buffer = &self.moon_buffers[i as usize];
                let group = self.surface_group(device, buffer, &self.moon_terrain);
                let maps = &self.maps[job.slot];
                draw(
                    encoder,
                    &self.moon_pipeline,
                    &group,
                    &[
                        &level_view(&maps.moon_day, 0, i),
                        &level_view(&maps.moon_relief, 0, i),
                    ],
                    0,
                    MOON_HEIGHT,
                    MOON_WIDTH,
                    true,
                );
                self.mips.build(device, encoder, &maps.moon_day, i, DAY);
                self.mips
                    .build(device, encoder, &maps.moon_relief, i, RELIEF);
                job.step = if i + 1 < job.moons.len() as u32 {
                    Step::MoonHeight(i + 1)
                } else {
                    Step::Done
                };
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
            &self.sky_pipeline,
            &group,
            &[&level_view(&self.skies[job.index], 0, 0)],
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
            ],
        })
    }
}

/// A full-target pass limited to `count` rows from `row`.
#[allow(clippy::too_many_arguments)]
fn draw(
    encoder: &mut wgpu::CommandEncoder,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    targets: &[&wgpu::TextureView],
    row: u32,
    count: u32,
    width: u32,
    clear: bool,
) {
    let attachments: Vec<_> = targets
        .iter()
        .map(|view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
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
            })
        })
        .collect();
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some("Solar generator pass"),
        color_attachments: &attachments,
        depth_stencil_attachment: None,
        timestamp_writes: None,
        occlusion_query_set: None,
        multiview_mask: None,
    });
    pass.set_pipeline(pipeline);
    pass.set_bind_group(0, group, &[]);
    pass.set_scissor_rect(0, row, width, count);
    pass.draw(0..3, 0..1);
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
            pass.draw(0..3, 0..1);
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
}
