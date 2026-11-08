//! An invented planet on the idle screen, generated from a seed and drawn on the
//! GPU with Vulkan.
//!
//! A sibling of [`crate::globe`], and for now a deliberate copy of much of it:
//! the same atmosphere model, sea glint, cloud shadows and night side. What
//! differs is where the maps come from and where the camera is.
//!
//! **The maps** are painted on the GPU when the scene is built, in the same
//! five layers the Earth uses: day colour, city lights, clouds, and relief
//! with the water mask. Every texel is a point on the unit sphere and every
//! layer reads 3D noise at that point, so nothing seams at the date line or
//! pinches at the poles.
//!
//! - *Height*: a handful of tectonic plates, some continental and some
//!   oceanic, set the large shapes; domain-warped fBm roughens the coasts;
//!   ridged noise raises mountain chains along the plate boundaries; and an
//!   "eroded" fBm, whose octaves are damped where the slope is already steep,
//!   carves valleys. Sea level is chosen so about 70% of the surface is ocean.
//! - *Climate*: temperature falls with latitude and height; moisture follows
//!   the three circulation cells (wet at the equator and near 60°, dry near
//!   30°), comes from the ocean upwind of the prevailing winds and is cut off
//!   behind mountains. A biome blend of the two picks colours sampled by eye
//!   from Blue Marble imagery.
//! - *Clouds*: fBm stretched along the latitudes, banded like the circulation
//!   cells, wound into cyclones that turn the right way in each hemisphere.
//! - *Lights*: towns and cities scattered as cellular noise, weighted toward
//!   coasts, temperate land and lowlands.
//!
//! **The camera** is ray traced per pixel instead of looked up from tables as
//! the globe does, so it can move: it shows the whole disc for a while, then
//! dives to low orbit and pitches up until the horizon and the thin blue line
//! of the air cross the panel, and climbs back out. Close to the ground,
//! where a map texel covers many pixels, procedural detail noise fades in.

use std::f32::consts::{PI, TAU};

use anyhow::Result;

use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;

/// Size of the rendered frame. The view draws it 1:1, so nothing is scaled.
pub const SIZE: u32 = 1080;
/// Size of the generated maps, as the Earth's.
const MAP_WIDTH: u32 = 4096;
const MAP_HEIGHT: u32 = 2048;
/// Size of the quick, small height map whose histogram sets the sea level.
const PROBE_WIDTH: u32 = 256;
const PROBE_HEIGHT: u32 = 128;
/// Share of the surface under water.
const OCEAN: f64 = 0.70;
/// Rows of a map painted per submission, so no single GPU job runs long
/// enough to trip a driver's watchdog on the Pi.
const BAND: u32 = 128;
/// Plates and storms the generator places; must match the shader.
const PLATES: usize = 12;
const STORMS: usize = 12;

/// Seconds for one revolution of the planet, seen from far away.
const REVOLUTION: f32 = 192.0;
/// How much slower the ground turns in low orbit, where the same angular speed
/// would sweep it past far too fast.
const NEAR_SLOW: f32 = 7.0;
/// How much faster than the ground the cloud sheet turns.
const CLOUD_DRIFT: f32 = 0.035;
/// How much brighter the city lights are than the generated map.
const NIGHT_GAIN: f32 = 2.2;
/// Albedo of the cloud sheet.
const CLOUD_ALBEDO: f32 = 0.85;
/// Height of the cloud deck in planet radii (exaggerated, as the globe's).
const CLOUD_HEIGHT: f32 = 0.006;
/// Extra mip levels to skip when sampling the maps.
const DETAIL_BIAS: f32 = 0.45;
/// Planet radius as a fraction of the frame's half-size, in the far view.
const PLANET: f32 = 0.93;
/// Scale height of the air in planet radii.
const SCALE_HEIGHT: f32 = 0.011;
/// Vertical optical depth of the air in red, green and blue.
const AIR: [f32; 3] = [0.045, 0.100, 0.230];
/// Brightness of the sunlight, in linear units of the day map.
const SUNLIGHT: f32 = 1.25;
/// How strongly the relief tilts the lighting.
const RELIEF: f32 = 0.55;
/// Tilt of the axis, as the globe's.
const OBLIQUITY: f64 = 23.44;
const NOD: f64 = 12.0;
/// The sun's direction at the start, in screen space.
const SUN: [f32; 3] = [-0.62, 0.22, 0.75];
/// Seconds for the sun to travel once around the planet.
const SUN_PERIOD: f32 = 1800.0;

/// Distance of the far camera, in planet radii: far enough to look almost
/// orthographic, like the globe.
const FAR: f32 = 40.0;
/// Height of the low orbit, in planet radii, above the air.
const ALTITUDE: f32 = 0.12;
/// Field of view in low orbit, corner to corner of the square's sides.
const NEAR_FOV: f32 = 56.0;
/// The camera's cycle: seconds far, diving, close, and climbing back.
const HOLD_FAR: f64 = 70.0;
const DIVE: f64 = 25.0;
const HOLD_NEAR: f64 = 90.0;
const CLIMB: f64 = 25.0;

/// One frame of the planet: a texture Slint samples directly.
struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Per-frame parameters, laid out as 16-byte aligned vectors.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// x: half the frame in pixels; y: tangent of half the field of view;
    /// z: the angle one pixel spans; w: how far into low orbit (0 far, 1 close).
    frame: [f32; 4],
    /// The camera's position in the planet's frame, in planet radii.
    camera: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    forward: [f32; 4],
    /// x: surface rotation in turns; y: the cloud sheet's.
    spin: [f32; 4],
    /// The sun's direction in the planet's frame.
    sun: [f32; 4],
    /// The planet's disc on screen, in clip space: min x, min y, max x, max y.
    disk: [f32; 4],
}

/// The generator's parameters, from the seed.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct GenParams {
    /// x, y: the target's size in texels; z: the sea level; w: unused.
    size: [f32; 4],
    /// Offsets into noise space, so each seed reads a different stretch of it.
    offsets: [[f32; 4]; 4],
    /// Plate centres on the sphere, and w: how continental each plate is.
    plates: [[f32; 4]; PLATES],
    /// Storm centres, and w: how far each winds its clouds, signed by hemisphere.
    storms: [[f32; 4]; STORMS],
}

/// Everything needed to draw one frame of the planet.
pub(crate) struct Planet {
    /// The frame's passes, for the far view and for low orbit.
    pipelines: [[wgpu::RenderPipeline; PASSES]; 2],
    /// The noise volume the frame's shaders read.
    noise: crate::noise3d::Noise3d,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    frames: Vec<Frame>,
    /// The generated maps: day, relief, night, clouds.
    #[allow(dead_code)]
    maps: Vec<wgpu::Texture>,
    /// The ground's rotation in turns, integrated, since its speed depends on
    /// where the camera is.
    spin: f64,
    /// The run time of the last frame, to integrate `spin` over.
    last: Option<f64>,
    /// How long the maps took to generate, for the benchmark.
    #[allow(dead_code)]
    generated: std::time::Duration,
}

impl Planet {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        let seed = std::env::var("PLANET_SEED")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or_else(rand::random::<u64>);
        Self::with_seed(gpu, seed)
    }

    pub(crate) fn with_seed(gpu: &Graphics, seed: u64) -> Result<Self> {
        let device = &gpu.device;
        let started = std::time::Instant::now();
        let noise = crate::noise3d::Noise3d::new(device, &gpu.queue);
        let maps = generate(gpu, seed, &noise)?;
        let generated = started.elapsed();
        tracing::info!(
            seed,
            ms = generated.as_secs_f64() * 1e3,
            "planet maps generated"
        );

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Planet sampler"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Planet uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Planet bind group"),
            entries: &[
                uniform_entry(0),
                texture(1),
                texture(2),
                texture(3),
                texture(4),
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Planet bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&maps.day),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&maps.night),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&maps.clouds),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&maps.relief),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Planet shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Planet pipeline layout"),
            bind_group_layouts: &[Some(&layout), Some(&noise.layout)],
            immediate_size: 0,
        });
        // Four small pipelines drawn in order (see the shader), for the far
        // view and again for low orbit (`NEAR` in the shader).
        let additive = wgpu::BlendState {
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
        let multiply = wgpu::BlendState {
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
        };
        let passes: [(&str, &str, &str, Option<wgpu::BlendState>); PASSES] = [
            ("background", "vs_main", "fs_background", None),
            (
                "ground",
                "vs_disk",
                "fs_ground",
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
            ("detail", "vs_disk", "fs_detail", Some(multiply)),
            ("sea", "vs_disk", "fs_sea", Some(additive)),
            (
                "clouds",
                "vs_disk",
                "fs_clouds",
                Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
            ),
            ("glow", "vs_disk", "fs_glow", Some(additive)),
        ];
        let pipelines = [0.0, 1.0].map(|near| {
            passes.map(|(name, vertex, fragment, blend)| {
                let constants = [("NEAR", near)];
                let options = wgpu::PipelineCompilationOptions {
                    constants: &constants,
                    ..Default::default()
                };
                let label = format!("Planet {} {name}", if near > 0.0 { "near" } else { "far" });
                device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                    label: Some(&label),
                    layout: Some(&pipeline_layout),
                    vertex: wgpu::VertexState {
                        module: &shader,
                        entry_point: Some(vertex),
                        buffers: &[],
                        compilation_options: options.clone(),
                    },
                    fragment: Some(wgpu::FragmentState {
                        module: &shader,
                        entry_point: Some(fragment),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8UnormSrgb,
                            blend,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: options,
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            })
        });

        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Planet frame"),
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
                // Drawn through an sRGB view, so the passes blend in linear
                // light and the frame Slint reads is encoded.
                let view = texture.create_view(&wgpu::TextureViewDescriptor {
                    format: Some(wgpu::TextureFormat::Rgba8UnormSrgb),
                    ..Default::default()
                });
                Frame { texture, view }
            })
            .collect();

        Ok(Self {
            pipelines,
            noise,
            uniforms,
            bind_group,
            frames,
            maps: maps.textures.clone(),
            spin: 0.0,
            last: None,
            generated,
        })
    }

    /// Draw the frame for `seconds` of run time into `frame` and submit it.
    fn render(&mut self, gpu: &Graphics, seconds: f64, frame: usize) -> Result<()> {
        let near = view_at(seconds);
        let dt = match self.last {
            Some(last) if seconds >= last && seconds - last < 1.0 => seconds - last,
            Some(_) => 0.0,
            None => 0.0,
        };
        self.last = Some(seconds);
        self.spin += dt / REVOLUTION as f64 * (1.0 + (1.0 / NEAR_SLOW as f64 - 1.0) * near as f64);
        self.draw(gpu, seconds, near, self.spin, frame);
        Ok(())
    }

    /// Draw one frame for `seconds`, `near` of the way into low orbit, with the
    /// ground turned `spin` turns.
    fn draw(&self, gpu: &Graphics, seconds: f64, near: f32, spin: f64, frame: usize) {
        gpu.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&uniforms_at(seconds, near, spin)),
        );
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Planet encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Planet pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frames[frame].view,
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
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_bind_group(1, &self.noise.group, &[]);
            let close = near > 0.001;
            let [background, ground, detail, sea, clouds, glow] =
                &self.pipelines[usize::from(close)];
            pass.set_pipeline(background);
            pass.draw(0..3, 0..1);
            for pipeline in [ground, detail, sea, clouds, glow] {
                // The detail only exists in low orbit.
                if std::ptr::eq(pipeline, detail) && !close {
                    continue;
                }
                pass.set_pipeline(pipeline);
                pass.draw(0..6, 0..1);
            }
        }
        gpu.submit(encoder.finish());
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    /// Read one frame back as RGBA pixels, for the previews.
    #[cfg(test)]
    fn read_back(&self, gpu: &Graphics, seconds: f64, near: f32, turned: f64) -> Vec<u8> {
        self.draw(gpu, seconds, near, turned + seconds / REVOLUTION as f64, 0);
        read_texture(gpu, &self.frames[0].texture, SIZE, SIZE, 4)
    }
}

const SLOTS: usize = crate::visuals::SLOTS;
/// The frame's pipelines, drawn in order: background, ground, close-up
/// detail (low orbit only), sea, clouds, glow.
const PASSES: usize = 6;

/// How far into low orbit the camera is at `seconds`: 0 for the whole disc,
/// 1 for the horizon. `PLANET_VIEW=globe` or `horizon` holds one of them.
fn view_at(seconds: f64) -> f32 {
    static FORCED: std::sync::OnceLock<Option<f32>> = std::sync::OnceLock::new();
    let forced = *FORCED.get_or_init(|| {
        match std::env::var("PLANET_VIEW")
            .ok()?
            .to_ascii_lowercase()
            .as_str()
        {
            "globe" | "far" => Some(0.0),
            "horizon" | "near" => Some(1.0),
            _ => None,
        }
    });
    if let Some(forced) = forced {
        return forced;
    }
    let cycle = HOLD_FAR + DIVE + HOLD_NEAR + CLIMB;
    let t = seconds.rem_euclid(cycle);
    let s = if t < HOLD_FAR {
        0.0
    } else if t < HOLD_FAR + DIVE {
        (t - HOLD_FAR) / DIVE
    } else if t < HOLD_FAR + DIVE + HOLD_NEAR {
        1.0
    } else {
        1.0 - (t - HOLD_FAR - DIVE - HOLD_NEAR) / CLIMB
    };
    s as f32
}

/// Ease in and out, with no jerk at either end.
fn smootherstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * x * (x * (x * 6.0 - 15.0) + 10.0)
}

fn smoothstep(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Where the camera is and which way it looks, `near` of the way to low orbit:
/// position, right, up, forward and the tangent of half the field of view.
fn camera(near: f32) -> ([f32; 3], [f32; 3], [f32; 3], [f32; 3], f32) {
    let e = smootherstep(near);
    let distance = (FAR.ln() + ((1.0 + ALTITUDE).ln() - FAR.ln()) * e).exp();
    let far_half = (1.0 / FAR).asin().tan() / PLANET;
    let near_half = (NEAR_FOV.to_radians() / 2.0).tan();
    let half = (far_half.ln() + (near_half.ln() - far_half.ln()) * e).exp();
    // Pitch up from looking straight down until the horizon sits a little
    // above the middle of the panel.
    let dip = (1.0 / (1.0 + ALTITUDE)).acos();
    let pitch_near = PI / 2.0 - dip - 0.42 * near_half.atan();
    let pitch = pitch_near * smoothstep(0.35, 1.0, e);

    let to_planet = tilt();
    let view = rotate(&to_planet, [0.0, 0.0, 1.0]);
    let right = rotate(&to_planet, [1.0, 0.0, 0.0]);
    let up = rotate(&to_planet, [0.0, 1.0, 0.0]);
    let forward = view.map(|v| -v);
    let (sin, cos) = pitch.sin_cos();
    let pitched_forward = [0, 1, 2].map(|i| forward[i] * cos + up[i] * sin);
    let pitched_up = [0, 1, 2].map(|i| up[i] * cos - forward[i] * sin);
    let position = view.map(|v| v * distance);
    (position, right, pitched_up, pitched_forward, half)
}

/// The shader's parameters for `seconds` of run time.
fn uniforms_at(seconds: f64, near: f32, spin: f64) -> Uniforms {
    let (position, right, up, forward, half) = camera(near);
    let sun = rotate(&tilt(), sun_at(seconds));
    let clouds = spin * (1.0 + CLOUD_DRIFT as f64);
    let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
    Uniforms {
        frame: [SIZE as f32 / 2.0, half, 2.0 * half / SIZE as f32, near],
        camera: v4(position),
        right: v4(right),
        up: v4(up),
        forward: v4(forward),
        spin: [
            spin.rem_euclid(1.0) as f32,
            clouds.rem_euclid(1.0) as f32,
            0.0,
            0.0,
        ],
        sun: v4(sun),
        disk: disk(position, right, up, forward, half),
    }
}

/// The planet's disc on screen, in clip space, from the camera: the
/// silhouette's circle projected, with a few pixels to spare, or the whole
/// frame when the planet surrounds the view.
fn disk(
    position: [f32; 3],
    right: [f32; 3],
    up: [f32; 3],
    forward: [f32; 3],
    half: f32,
) -> [f32; 4] {
    let full = [-1.0, -1.0, 1.0, 1.0];
    let dot = |a: [f32; 3], b: [f32; 3]| a[0] * b[0] + a[1] * b[1] + a[2] * b[2];
    let centre = position.map(|v| -v);
    let distance = dot(centre, centre).sqrt();
    if distance <= 1.0001 || dot(centre, forward) <= 1.0 {
        return full;
    }
    let axis = centre.map(|v| v / distance);
    let helper = if axis[1].abs() < 0.9 {
        [0.0, 1.0, 0.0]
    } else {
        [1.0, 0.0, 0.0]
    };
    let cross = |a: [f32; 3], b: [f32; 3]| {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    };
    let e1 = cross(axis, helper);
    let l = dot(e1, e1).sqrt();
    let e1 = e1.map(|v| v / l);
    let e2 = cross(axis, e1);
    let spread = distance * (1.0 / distance).asin().tan() / (PI / 16.0).cos();
    let (mut lo, mut hi) = ([f32::MAX; 2], [f32::MIN; 2]);
    for k in 0..16 {
        let a = k as f32 / 16.0 * TAU;
        let p = [0, 1, 2].map(|i| centre[i] + spread * (a.cos() * e1[i] + a.sin() * e2[i]));
        let z = dot(p, forward);
        if z <= 1e-5 {
            return full;
        }
        let x = dot(p, right) / z / half;
        let y = dot(p, up) / z / half;
        lo = [lo[0].min(x), lo[1].min(y)];
        hi = [hi[0].max(x), hi[1].max(y)];
    }
    let margin = 3.0 / (SIZE as f32 / 2.0);
    [
        (lo[0] - margin).max(-1.0),
        (lo[1] - margin).max(-1.0),
        (hi[0] + margin).min(1.0),
        (hi[1] + margin).min(1.0),
    ]
}

fn sun_at(seconds: f64) -> [f32; 3] {
    let angle = (seconds / SUN_PERIOD as f64) * std::f64::consts::TAU;
    let (sin, cos) = angle.sin_cos();
    let (x, y, z) = (SUN[0] as f64, SUN[1] as f64, SUN[2] as f64);
    [
        (x * cos - z * sin) as f32,
        y as f32,
        (x * sin + z * cos) as f32,
    ]
}

/// The rotation from screen space into the planet's frame, as the globe's.
fn tilt() -> [[f32; 3]; 3] {
    let (st, ct) = OBLIQUITY.to_radians().sin_cos();
    let (sk, ck) = NOD.to_radians().sin_cos();
    let nod = [[1.0, 0.0, 0.0], [0.0, ck, -sk], [0.0, sk, ck]];
    let lean = [[ct, st, 0.0], [-st, ct, 0.0], [0.0, 0.0, 1.0]];
    let mut to_screen = [[0.0f64; 3]; 3];
    for (r, row) in to_screen.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = (0..3).map(|k| lean[r][k] * nod[k][c]).sum();
        }
    }
    let mut to_planet = [[0.0f32; 3]; 3];
    for (r, row) in to_planet.iter_mut().enumerate() {
        for (c, cell) in row.iter_mut().enumerate() {
            *cell = to_screen[c][r] as f32;
        }
    }
    to_planet
}

fn rotate(m: &[[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [0, 1, 2].map(|r| m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2])
}

fn uniform_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
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

fn texture(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

// ---------------------------------------------------------------------------
// Generating the maps.
// ---------------------------------------------------------------------------

/// The generated maps, ready to sample.
struct Maps {
    /// The textures themselves, kept for the previews to read back.
    #[allow(dead_code)]
    textures: Vec<wgpu::Texture>,
    day: wgpu::TextureView,
    night: wgpu::TextureView,
    clouds: wgpu::TextureView,
    relief: wgpu::TextureView,
}

/// A small deterministic generator for the seed's parameters.
struct SplitMix(u64);

impl SplitMix {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// Uniform in [0, 1).
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }

    fn range(&mut self, low: f32, high: f32) -> f32 {
        low + (high - low) * self.unit()
    }

    /// A point on the unit sphere, uniformly.
    fn on_sphere(&mut self) -> [f32; 3] {
        let z = self.range(-1.0, 1.0);
        let phi = self.range(0.0, TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        [r * phi.cos(), z, r * phi.sin()]
    }
}

fn gen_params(seed: u64) -> GenParams {
    let mut rng = SplitMix(seed);
    let offsets = [(); 4].map(|_| {
        [
            rng.range(-60.0, 60.0),
            rng.range(-60.0, 60.0),
            rng.range(-60.0, 60.0),
            0.0,
        ]
    });
    // About two plates in five carry continents.
    let plates = [(); PLATES].map(|_| {
        let [x, y, z] = rng.on_sphere();
        let continental = rng.unit() < 0.42;
        let w = if continental {
            rng.range(0.35, 1.0)
        } else {
            rng.range(-1.0, -0.25)
        };
        [x, y, z, w]
    });
    // Most storms in the mid-latitudes' storm tracks, a few tropical ones;
    // wound anticlockwise in the north and clockwise in the south.
    let storms = [(); STORMS].map(|_| {
        let tropical = rng.unit() < 0.3;
        let lat = if tropical {
            rng.range(10.0, 24.0)
        } else {
            rng.range(35.0, 64.0)
        }
        .to_radians();
        let north = rng.unit() < 0.5;
        let lat = if north { lat } else { -lat };
        let lon = rng.range(0.0, TAU);
        let strength = rng.range(1.2, 2.6) * if north { 1.0 } else { -1.0 };
        [
            lat.cos() * lon.sin(),
            lat.sin(),
            lat.cos() * lon.cos(),
            strength,
        ]
    });
    GenParams {
        size: [0.0; 4],
        offsets,
        plates,
        storms,
    }
}

/// The height below which `share` of the sphere lies, from a probe map whose
/// rows are weighted by the area they cover.
fn sea_level(heights: &[f32], width: u32, height: u32, share: f64) -> f32 {
    let mut weighted: Vec<(f32, f64)> = Vec::with_capacity(heights.len());
    for y in 0..height {
        let lat = (0.5 - (y as f64 + 0.5) / height as f64) * std::f64::consts::PI;
        let area = lat.cos();
        for x in 0..width {
            weighted.push((heights[(y * width + x) as usize], area));
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

/// Paint the five maps for `seed` on the GPU.
fn generate(gpu: &Graphics, seed: u64, noise: &crate::noise3d::Noise3d) -> Result<Maps> {
    let device = &gpu.device;
    let mut params = gen_params(seed);
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Planet generator"),
        size: std::mem::size_of::<GenParams>() as u64,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("Planet generator shader"),
        source: wgpu::ShaderSource::Wgsl(generator_source().into()),
    });

    // Height first: uniforms only.
    let height_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Planet height"),
        entries: &[uniform_entry(0)],
    });
    let height_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Planet height"),
        layout: &height_layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: buffer.as_entire_binding(),
        }],
    });
    let pipeline = |label,
                    layout: &wgpu::BindGroupLayout,
                    entry,
                    formats: &[wgpu::TextureFormat],
                    blend: Option<wgpu::BlendState>| {
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(layout), Some(&noise.layout)],
            immediate_size: 0,
        });
        let targets: Vec<_> = formats
            .iter()
            .map(|&format| {
                Some(wgpu::ColorTargetState {
                    format,
                    blend,
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

    // 1. A small probe of the heights, read back to choose the sea level.
    let probe_pipeline = pipeline(
        "Planet probe",
        &height_layout,
        "fs_height",
        &[wgpu::TextureFormat::Rgba32Float],
        None,
    );
    let probe = target(
        device,
        "Planet probe",
        PROBE_WIDTH,
        PROBE_HEIGHT,
        1,
        wgpu::TextureFormat::Rgba32Float,
    );
    params.size = [PROBE_WIDTH as f32, PROBE_HEIGHT as f32, 0.0, 0.0];
    gpu.queue
        .write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
    // The probe leaves out the eroded detail, too small to move the sea.
    paint(
        gpu,
        &probe_pipeline,
        &height_group,
        &noise.group,
        &[&view_of(&probe, 0)],
        PROBE_WIDTH,
        PROBE_HEIGHT,
        true,
    );
    let bytes = read_texture(gpu, &probe, PROBE_WIDTH, PROBE_HEIGHT, 16);
    let heights: Vec<f32> = bytes
        .chunks_exact(16)
        .map(|texel| f32::from_le_bytes([texel[0], texel[1], texel[2], texel[3]]))
        .collect();
    let sea = sea_level(&heights, PROBE_WIDTH, PROBE_HEIGHT, OCEAN);

    // 2. The full height map, with a mip chain for the climate's wide looks.
    let mips = MipChain::new(device);
    let levels = MAP_WIDTH.max(MAP_HEIGHT).ilog2() + 1;
    let terrain_format = wgpu::TextureFormat::Rgba16Float;
    let terrain_pipeline = pipeline(
        "Planet terrain",
        &height_layout,
        "fs_height",
        &[terrain_format],
        None,
    );
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
    let erosion_pipeline = pipeline(
        "Planet erosion",
        &height_layout,
        "fs_erosion",
        &[terrain_format],
        Some(add),
    );
    let terrain = target(
        device,
        "Planet terrain",
        MAP_WIDTH,
        MAP_HEIGHT,
        levels,
        terrain_format,
    );
    params.size = [MAP_WIDTH as f32, MAP_HEIGHT as f32, sea, 0.0];
    gpu.queue
        .write_buffer(&buffer, 0, bytemuck::bytes_of(&params));
    for (pipeline, clear) in [(&terrain_pipeline, true), (&erosion_pipeline, false)] {
        paint(
            gpu,
            pipeline,
            &height_group,
            &noise.group,
            &[&view_of(&terrain, 0)],
            MAP_WIDTH,
            MAP_HEIGHT,
            clear,
        );
    }
    mips.build(gpu, &terrain, levels, terrain_format);

    // 3. Everything else, from the heights.
    let surface_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Planet surface"),
        entries: &[
            uniform_entry(0),
            texture(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
        ],
    });
    let terrain_view = terrain.create_view(&wgpu::TextureViewDescriptor::default());
    let terrain_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
        label: Some("Planet terrain sampler"),
        address_mode_u: wgpu::AddressMode::Repeat,
        address_mode_v: wgpu::AddressMode::ClampToEdge,
        mag_filter: wgpu::FilterMode::Linear,
        min_filter: wgpu::FilterMode::Linear,
        mipmap_filter: wgpu::MipmapFilterMode::Linear,
        ..Default::default()
    });
    let surface_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Planet surface"),
        layout: &surface_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&terrain_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&terrain_sampler),
            },
        ],
    });
    let formats = [
        wgpu::TextureFormat::Rgba8UnormSrgb,
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::R8Unorm,
        wgpu::TextureFormat::R8Unorm,
    ];
    // The climate first, into a scratch map the other surface passes read.
    let climate_format = wgpu::TextureFormat::Rgba8Unorm;
    let climate_pipeline = pipeline(
        "Planet climate",
        &surface_layout,
        "fs_climate",
        &[climate_format],
        None,
    );
    let climate = target(
        device,
        "Planet climate",
        MAP_WIDTH,
        MAP_HEIGHT,
        1,
        climate_format,
    );
    paint(
        gpu,
        &climate_pipeline,
        &surface_group,
        &noise.group,
        &[&view_of(&climate, 0)],
        MAP_WIDTH,
        MAP_HEIGHT,
        true,
    );
    let read_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Planet surface with climate"),
        entries: &[
            uniform_entry(0),
            texture(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture(3),
        ],
    });
    let climate_view = view_of(&climate, 0);
    let read_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Planet surface with climate"),
        layout: &read_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&terrain_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&terrain_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&climate_view),
            },
        ],
    });
    let relief_pipeline = pipeline(
        "Planet relief",
        &read_layout,
        "fs_relief",
        &formats[1..2],
        None,
    );
    let clouds_pipeline = pipeline(
        "Planet clouds",
        &read_layout,
        "fs_clouds",
        &formats[3..],
        None,
    );
    // The passes after the relief read it too.
    let late_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Planet surface with relief"),
        entries: &[
            uniform_entry(0),
            texture(1),
            wgpu::BindGroupLayoutEntry {
                binding: 2,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                count: None,
            },
            texture(3),
            texture(4),
        ],
    });
    let day_pipeline = pipeline("Planet day", &late_layout, "fs_day", &formats[..1], None);
    let lights_pipeline = pipeline(
        "Planet lights",
        &late_layout,
        "fs_lights",
        &formats[2..3],
        None,
    );
    let towns_pipeline = pipeline(
        "Planet towns",
        &late_layout,
        "fs_towns",
        &formats[2..3],
        Some(add),
    );
    let names = [
        "Planet day",
        "Planet relief",
        "Planet night",
        "Planet clouds",
    ];
    let maps: Vec<wgpu::Texture> = names
        .iter()
        .zip(formats)
        .map(|(name, format)| target(device, name, MAP_WIDTH, MAP_HEIGHT, levels, format))
        .collect();
    let views: Vec<wgpu::TextureView> = maps.iter().map(|map| view_of(map, 0)).collect();
    for (pipeline, target) in [(&relief_pipeline, &views[1]), (&clouds_pipeline, &views[3])] {
        paint(
            gpu,
            pipeline,
            &read_group,
            &noise.group,
            &[target],
            MAP_WIDTH,
            MAP_HEIGHT,
            true,
        );
    }
    let relief_view = view_of(&maps[1], 0);
    let late_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Planet surface with relief"),
        layout: &late_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: buffer.as_entire_binding(),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&terrain_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(&terrain_sampler),
            },
            wgpu::BindGroupEntry {
                binding: 3,
                resource: wgpu::BindingResource::TextureView(&climate_view),
            },
            wgpu::BindGroupEntry {
                binding: 4,
                resource: wgpu::BindingResource::TextureView(&relief_view),
            },
        ],
    });
    for (pipeline, target, clear) in [
        (&day_pipeline, &views[0], true),
        (&lights_pipeline, &views[2], true),
        (&towns_pipeline, &views[2], false),
    ] {
        paint(
            gpu,
            pipeline,
            &late_group,
            &noise.group,
            &[target],
            MAP_WIDTH,
            MAP_HEIGHT,
            clear,
        );
    }
    for (map, format) in maps.iter().zip(formats) {
        mips.build(gpu, map, levels, format);
    }
    if gpu.wait(std::time::Duration::from_secs(60)).is_none() {
        tracing::warn!("the planet's maps took too long to finish");
    }
    let full = |map: &wgpu::Texture| map.create_view(&wgpu::TextureViewDescriptor::default());
    Ok(Maps {
        day: full(&maps[0]),
        relief: full(&maps[1]),
        night: full(&maps[2]),
        clouds: full(&maps[3]),
        textures: maps,
    })
}

fn target(
    device: &wgpu::Device,
    label: &str,
    width: u32,
    height: u32,
    levels: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
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

fn view_of(texture: &wgpu::Texture, level: u32) -> wgpu::TextureView {
    texture.create_view(&wgpu::TextureViewDescriptor {
        base_mip_level: level,
        mip_level_count: Some(1),
        ..Default::default()
    })
}

/// Run a full-screen generator pass into `targets`, a band of rows at a time,
/// waiting for each so no single GPU job runs long.
fn paint(
    gpu: &Graphics,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
    noise_group: &wgpu::BindGroup,
    targets: &[&wgpu::TextureView],
    width: u32,
    height: u32,
    clear: bool,
) {
    let mut row = 0;
    while row < height {
        let rows = BAND.min(height - row);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Planet generator"),
            });
        {
            let attachments: Vec<_> = targets
                .iter()
                .map(|view| {
                    Some(wgpu::RenderPassColorAttachment {
                        view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: if row == 0 && clear {
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
                label: Some("Planet generator pass"),
                color_attachments: &attachments,
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, group, &[]);
            pass.set_bind_group(1, noise_group, &[]);
            pass.set_scissor_rect(0, row, width, rows);
            pass.draw(0..3, 0..1);
        }
        gpu.submit(encoder.finish());
        if gpu.wait(std::time::Duration::from_secs(30)).is_none() {
            tracing::warn!("a band of the planet's maps took too long");
        }
        row += rows;
    }
}

/// Copy a texture's first level back to the CPU, rows unpadded.
fn read_texture(
    gpu: &Graphics,
    texture: &wgpu::Texture,
    width: u32,
    height: u32,
    bytes_per_texel: u32,
) -> Vec<u8> {
    let row = width * bytes_per_texel;
    let padded =
        row.div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT) * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Planet readback"),
        size: padded as u64 * height as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu
        .device
        .create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Planet readback"),
        });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.submit(encoder.finish());
    gpu.wait(std::time::Duration::from_secs(30));
    let slice = buffer.slice(..);
    let mapped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = mapped.clone();
    slice.map_async(wgpu::MapMode::Read, move |_| {
        flag.store(true, std::sync::atomic::Ordering::SeqCst);
    });
    let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while !mapped.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < until {
        let _ = gpu.device.poll(wgpu::PollType::Poll);
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
    let mut out = Vec::with_capacity((row * height) as usize);
    if let Ok(data) = slice.get_mapped_range() {
        for y in 0..height as usize {
            let start = y * padded as usize;
            out.extend_from_slice(&data[start..start + row as usize]);
        }
    }
    buffer.unmap();
    out
}

/// Builds mip chains on the GPU, as the globe's, with half floats added for
/// the height map.
struct MipChain {
    layout: wgpu::BindGroupLayout,
    pipelines: Vec<(wgpu::TextureFormat, wgpu::RenderPipeline)>,
    sampler: wgpu::Sampler,
}

impl MipChain {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Planet mip chain"),
            entries: &[
                texture(0),
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Planet mip chain shader"),
            source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Planet mip chain layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Planet mip chain pipeline"),
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
            })
        };
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Planet mip chain sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let pipelines = [
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureFormat::Rgba8UnormSrgb,
            wgpu::TextureFormat::R8Unorm,
            wgpu::TextureFormat::Rgba16Float,
        ]
        .into_iter()
        .map(|format| (format, make(format)))
        .collect();
        Self {
            layout,
            pipelines,
            sampler,
        }
    }

    fn build(
        &self,
        gpu: &Graphics,
        texture: &wgpu::Texture,
        levels: u32,
        format: wgpu::TextureFormat,
    ) {
        let device = &gpu.device;
        let Some((_, pipeline)) = self.pipelines.iter().find(|(f, _)| *f == format) else {
            tracing::warn!(?format, "no mip pipeline for this map format");
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Planet mip chain"),
        });
        for level in 1..levels {
            let source = view_of(texture, level - 1);
            let target = view_of(texture, level);
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Planet mip chain"),
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
                label: Some("Planet mip chain pass"),
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
            pass.set_bind_group(0, &bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.submit(encoder.finish());
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

/// Hashing and value noise with analytic derivatives, shared by the
/// generator and the frame shader's close-up detail.
const NOISE: &str = r#"
const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;

fn hash3(c: vec3<i32>) -> f32 {
    var h = (u32(c.x) * 0x8da6b343u) ^ (u32(c.y) * 0xd8163841u) ^ (u32(c.z) * 0xcb1ab31fu);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// Turns each octave so the lattice never lines up from one to the next.
const OCTAVE: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.00, 0.80, 0.60),
    vec3<f32>(-0.80, 0.36, -0.48),
    vec3<f32>(-0.60, -0.48, 0.64),
);

fn fbm(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    for (var i = 0; i < octaves; i++) {
        sum += amplitude * noise(q);
        q = OCTAVE * q * 2.03;
        amplitude *= 0.5;
    }
    return sum;
}
"#;

/// The generator: heights first (`fs_height`, then `fs_erosion` added), then
/// the surface maps from them (`fs_day`, `fs_lights`, `fs_clouds`).
pub(crate) fn generator_source() -> String {
    let mut source = String::from(NOISE);
    source.push_str(crate::noise3d::WGSL);
    source.push_str(GENERATOR);
    source
}

const GENERATOR: &str = r#"
struct Gen {
    size: vec4<f32>,
    offsets: array<vec4<f32>, 4>,
    plates: array<vec4<f32>, 12>,
    storms: array<vec4<f32>, 12>,
};

@group(0) @binding(0) var<uniform> gen: Gen;
@group(0) @binding(1) var terrain: texture_2d<f32>;
@group(0) @binding(2) var terrain_sampler: sampler;

struct Varying {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

// The point on the unit sphere a texel of an equirectangular map stands for,
// the same mapping the frame shader reads it back with.
fn sphere_at(uv: vec2<f32>) -> vec3<f32> {
    let lon = (uv.x - 0.5) * TAU;
    let lat = (0.5 - uv.y) * PI;
    return vec3<f32>(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
}

fn ridged(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    var weight = 1.0;
    for (var i = 0; i < octaves; i++) {
        var r = 1.0 - abs(noise(q));
        r = r * r * weight;
        weight = clamp(r * 1.6, 0.0, 1.0);
        sum += amplitude * r;
        q = OCTAVE * q * 2.07;
        amplitude *= 0.5;
    }
    return sum;
}

// fBm whose octaves fade where the slope is already steep: valleys and
// smooth basins, as erosion leaves them.
fn eroded(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    var slope = vec3<f32>(0.0);
    var frequency = 1.0;
    for (var i = 0; i < octaves; i++) {
        let n = noised(q);
        // The slope so far, kept in this octave's frame: the octave turn is
        // orthonormal, so its length is the same as in the sphere's own, and
        // no matrix of turns has to be carried along.
        slope += n.yzw * frequency * amplitude;
        sum += amplitude * n.x / (1.0 + dot(slope, slope) * 0.35);
        q = OCTAVE * q * 2.01;
        slope = OCTAVE * slope;
        frequency *= 2.01;
        amplitude *= 0.5;
    }
    return sum;
}

// The generator runs as several small pipelines rather than two large ones,
// so each keeps the V3D's four threads: the large shapes, then the eroded
// detail added on top; then the day colour with the relief, the city lights
// and the clouds, each from the same climate.

// The planet's large shapes at `p`: the point warped, the plates' blend and
// the nearest boundary, and the base elevation.
struct Shape {
    q: vec3<f32>,
    boundary: f32,
    base: f32,
};

fn shape(p: vec3<f32>) -> Shape {
    let o = gen.offsets;
    let warp = vec3<f32>(
        fbm(p * 1.4 + o[0].xyz, 4),
        fbm(p * 1.4 + o[1].xyz, 4),
        fbm(p * 1.4 + o[2].xyz, 4),
    );
    var out: Shape;
    out.q = normalize(p + warp * 0.32);
    // Plates: a soft blend of their biases, and how close the nearest
    // boundary is.
    var weights = 0.0;
    var bias = 0.0;
    var d1 = 9.0;
    var d2 = 9.0;
    for (var i = 0; i < 12; i++) {
        let plate = gen.plates[i];
        let d = distance(out.q, plate.xyz);
        let w = exp(-d * d * 14.0);
        weights += w;
        bias += w * plate.w;
        if (d < d1) {
            d2 = d1;
            d1 = d;
        } else if (d < d2) {
            d2 = d;
        }
    }
    out.boundary = d2 - d1;
    out.base = 0.42 * bias / max(weights, 1e-5) + 0.55 * fbm(out.q * 2.2 + o[3].xyz, 6);
    return out;
}

// Raw elevation without the eroded detail (sea level not yet taken off),
// the mountain belts, the base, and 1.
@fragment
fn fs_height(in: Varying) -> @location(0) vec4<f32> {
    let p = sphere_at(in.position.xy / gen.size.xy);
    let o = gen.offsets;
    let s = shape(p);
    let landish = smoothstep(-0.15, 0.25, s.base);
    // Mountain chains along the plate boundaries, mostly on land; lower
    // ranges scattered inland.
    let belt = exp(-s.boundary * s.boundary / (0.07 * 0.07));
    let ridges = ridged(s.q * 5.5 + o[0].zxy, 6);
    let scattered = smoothstep(0.15, 0.55, fbm(s.q * 3.0 + o[1].zxy, 3));
    let mountains = ridges * (belt * (0.35 + 0.65 * landish) * 0.42 + scattered * 0.12 * landish);
    return vec4<f32>(s.base + mountains, mountains, s.base, 1.0);
}

// The eroded detail, added to the elevation by blending.
@fragment
fn fs_erosion(in: Varying) -> @location(0) vec4<f32> {
    let p = sphere_at(in.position.xy / gen.size.xy);
    let s = shape(p);
    let landish = smoothstep(-0.15, 0.25, s.base);
    let detail = eroded(s.q * 9.0 + gen.offsets[2].zxy, 7) * (0.05 + 0.10 * landish);
    return vec4<f32>(detail, 0.0, 0.0, 0.0);
}

fn srgb(r: f32, g: f32, b: f32) -> vec3<f32> {
    return pow(vec3<f32>(r, g, b) / 255.0, vec3<f32>(2.2));
}

// Elevation above sea level, wide-looking: the height map's mip `level`.
fn wide(uv: vec2<f32>, level: f32) -> f32 {
    return textureSampleLevel(terrain, terrain_sampler, uv, level).r - gen.size.z;
}

fn elevation_at(texel: vec2<i32>) -> f32 {
    // Wrapped across the date line by compare and select: the V3D has no
    // integer divide, and the emulated modulo held registers enough to cost
    // the relief pass two threads.
    let w = i32(gen.size.x);
    var x = select(texel.x, texel.x + w, texel.x < 0);
    x = select(x, x - w, x >= w);
    let t = vec2<i32>(x, clamp(texel.y, 0, i32(gen.size.y) - 1));
    return textureLoad(terrain, t, 0).r - gen.size.z;
}

// Light from towns and cities: cellular noise at `scale` cells per radius,
// each cell one settlement of random size, kept with probability `density`.
fn settlements(p: vec3<f32>, scale: f32, density: f32, seed: i32) -> f32 {
    let x = p * scale;
    let cell = vec3<i32>(floor(x));
    var light = 0.0;
    for (var k = 0; k < 27; k++) {
        let offset = vec3<i32>(k % 3 - 1, (k / 3) % 3 - 1, k / 9 - 1);
        let id = cell + offset;
        let keep = hash3(id + vec3<i32>(seed, 17, 3));
        if (keep > density) {
            continue;
        }
        let centre = vec3<f32>(id) + vec3<f32>(
            hash3(id + vec3<i32>(seed, 1, 0)),
            hash3(id + vec3<i32>(seed, 2, 0)),
            hash3(id + vec3<i32>(seed, 3, 0)),
        );
        let size = hash3(id + vec3<i32>(seed, 4, 0));
        let population = size * size * size;
        let radius = 0.08 + 0.42 * population;
        let d = distance(x, centre) / radius;
        light += (0.25 + 0.75 * population) * exp(-d * d * 2.5);
    }
    return light;
}

// Rotate `v` by `angle` about the unit `axis`.
fn turn_about(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

// What every surface pass needs to know about a texel.
struct Climate {
    p: vec3<f32>,
    uv: vec2<f32>,
    lat: f32,
    // Elevation above sea level, and whether it is land.
    e: f32,
    land: f32,
    warmth: f32,
    temperature: f32,
    moisture: f32,
    near_coast: f32,
};

// The climate, worked out once into a scratch map by `fs_climate` and read
// back by the passes after it.
@group(0) @binding(3) var climate_map: texture_2d<f32>;

fn climate(position: vec2<f32>) -> Climate {
    var c: Climate;
    c.uv = position / gen.size.xy;
    c.p = sphere_at(c.uv);
    c.e = textureLoad(terrain, vec2<i32>(position), 0).r - gen.size.z;
    c.land = smoothstep(-0.0015, 0.0015, c.e);
    c.lat = asin(clamp(c.p.y, -1.0, 1.0));
    let stored = textureLoad(climate_map, vec2<i32>(position), 0);
    c.temperature = stored.r * 2.0 - 0.5;
    c.moisture = stored.g;
    c.near_coast = stored.b;
    c.warmth = stored.a * 2.0 - 0.5;
    return c;
}

// Temperature, moisture, the coast's nearness and warmth, encoded for an
// 8-bit map (temperatures from -0.5 to 1.5).
@fragment
fn fs_climate(in: Varying) -> @location(0) vec4<f32> {
    let c = work_out_climate(in.position.xy);
    return vec4<f32>((c.temperature + 0.5) * 0.5, c.moisture, c.near_coast, (c.warmth + 0.5) * 0.5);
}

fn work_out_climate(position: vec2<f32>) -> Climate {
    var c: Climate;
    c.uv = position / gen.size.xy;
    c.p = sphere_at(c.uv);
    let o = gen.offsets;
    c.e = textureLoad(terrain, vec2<i32>(position), 0).r - gen.size.z;
    c.land = smoothstep(-0.0015, 0.0015, c.e);
    c.lat = asin(clamp(c.p.y, -1.0, 1.0));
    let cos_lat = max(cos(c.lat), 0.02);
    // Temperature: latitude, height, a little noise.
    c.warmth = pow(cos_lat, 0.85) + fbm(c.p * 4.0 + o[3].zyx, 4) * 0.10;
    c.temperature = c.warmth - max(c.e, 0.0) * 1.5;
    // Moisture: the circulation cells, then the air's path from the sea along
    // the prevailing wind, which mountains upwind wring dry. Trade winds and
    // polar easterlies come from the east; the westerlies from the west.
    let degrees = abs(c.lat) * 57.2958;
    let from_east = select(-1.0, 1.0, degrees < 30.0 || degrees > 62.0);
    var ocean_upwind = 0.0;
    var barrier = 0.0;
    for (var k = 1; k <= 5; k++) {
        let up = wide(c.uv + vec2<f32>(from_east * f32(k) * 0.0105 / cos_lat, 0.0), 4.5);
        ocean_upwind += (1.0 - smoothstep(-0.02, 0.03, up)) / f32(k);
        barrier = max(barrier, up - max(c.e, 0.0));
    }
    // The weights 1/k for k = 1..5 add up to 137/60.
    ocean_upwind *= 60.0 / 137.0;
    c.near_coast = 1.0 - smoothstep(0.0, 0.12, max(wide(c.uv, 5.5), 0.0));
    let interior = smoothstep(0.03, 0.25, wide(c.uv, 7.5));
    c.moisture = clamp(
        0.40 + 0.26 * cos(6.0 * c.lat) + 0.30 * ocean_upwind + 0.12 * c.near_coast - 0.18 * interior
            - 0.35 * smoothstep(0.05, 0.3, barrier) + 0.22 * fbm(c.p * 6.0 + o[2].yzx, 5),
        0.0,
        1.0,
    );
    return c;
}

// The relief's slopes toward the east and the north, as the normal map
// stores them: one unit of elevation stands for about 16 km, 0.0025 radii,
// exaggerated.
fn slopes(position: vec2<f32>, lat: f32) -> vec2<f32> {
    let texel = vec2<i32>(position);
    let east_step = max(cos(lat), 0.02) * TAU / gen.size.x;
    let north_step = PI / gen.size.y;
    let de = (max(elevation_at(texel + vec2<i32>(1, 0)), 0.0) - max(elevation_at(texel - vec2<i32>(1, 0)), 0.0)) / (2.0 * east_step);
    let dn = (max(elevation_at(texel - vec2<i32>(0, 1)), 0.0) - max(elevation_at(texel + vec2<i32>(0, 1)), 0.0)) / (2.0 * north_step);
    return clamp(-vec2<f32>(de, dn) * 0.0025 * 25.0, vec2<f32>(-0.45), vec2<f32>(0.45));
}

// The relief with the water mask, before the day map, which reads it.
@fragment
fn fs_relief(in: Varying) -> @location(0) vec4<f32> {
    let uv = in.position.xy / gen.size.xy;
    let lat = (0.5 - uv.y) * PI;
    let land = smoothstep(-0.0015, 0.0015, textureLoad(terrain, vec2<i32>(in.position.xy), 0).r - gen.size.z);
    return vec4<f32>(vec2<f32>(0.5) + slopes(in.position.xy, lat) * land, 1.0 - land, 1.0);
}

// The finished relief, for the passes after `fs_relief`.
@group(0) @binding(4) var relief_map: texture_2d<f32>;

fn steepness(position: vec2<f32>) -> f32 {
    return length(textureLoad(relief_map, vec2<i32>(position), 0).rg - vec2<f32>(0.5));
}

// The day map.
@fragment
fn fs_day(in: Varying) -> @location(0) vec4<f32> {
    let c = climate(in.position.xy);
    let o = gen.offsets;
    let p = c.p;
    let steep = steepness(in.position.xy);
    let moisture = c.moisture;
    let temperature = c.temperature;
    // Land colour: a blend of biomes across temperature and moisture, from
    // colours read off Blue Marble imagery.
    let sand = mix(srgb(218.0, 184.0, 136.0), srgb(170.0, 118.0, 78.0), 0.45 * smoothstep(-0.2, 0.3, fbm(p * 14.0 + o[0].yxz, 4)));
    let dry = mix(sand, srgb(150.0, 136.0, 94.0), smoothstep(0.10, 0.32, moisture));
    let warm_green = mix(srgb(122.0, 114.0, 66.0), srgb(26.0, 54.0, 20.0), smoothstep(0.48, 0.80, moisture));
    let green = mix(srgb(32.0, 48.0, 34.0), mix(srgb(48.0, 70.0, 32.0), warm_green, smoothstep(0.62, 0.85, temperature)), smoothstep(0.42, 0.58, temperature));
    var ground = mix(dry, green, smoothstep(0.25, 0.62, moisture));
    ground = mix(ground, srgb(108.0, 102.0, 88.0), smoothstep(0.46, 0.34, temperature));
    ground = mix(ground, srgb(100.0, 88.0, 76.0), max(smoothstep(0.12, 0.40, steep) * 0.75, smoothstep(0.16, 0.34, c.e) * 0.55));
    ground = mix(ground, srgb(236.0, 240.0, 245.0), smoothstep(0.26, 0.15, temperature));
    ground *= 0.86 + 0.28 * (fbm(p * 60.0 + o[1].yxz, 4) + 0.5);
    // The sea: turquoise over the shelves, darkening into the deep, frozen
    // where it is cold enough.
    let depth = max(-c.e, 0.0);
    var sea = mix(srgb(30.0, 82.0, 112.0), srgb(10.0, 28.0, 66.0), smoothstep(0.0, 0.035, depth));
    sea = mix(sea, srgb(5.0, 13.0, 40.0), smoothstep(0.035, 0.22, depth));
    let ice = smoothstep(0.17, 0.10, c.warmth + 0.04 + 0.04 * fbm(p * 20.0, 3));
    sea = mix(sea, srgb(218.0, 226.0, 236.0), ice);
    return vec4<f32>(mix(sea, ground, c.land), 1.0);
}

// How likely people are to live here: temperate, watered, low land, nearer
// the coast, not on steep slopes.
fn liveable(position: vec2<f32>) -> f32 {
    let c = climate(position);
    return c.land
        * smoothstep(0.30, 0.52, c.temperature) * (1.0 - smoothstep(0.30, 0.50, c.e))
        * smoothstep(0.10, 0.35, c.moisture) * (0.45 + 0.9 * c.near_coast)
        * smoothstep(-0.25, 0.20, fbm(c.p * 3.0 + gen.offsets[3].xzy, 3))
        * (1.0 - smoothstep(0.08, 0.25, steepness(position)));
}

// City lights: the cities, then (`fs_towns`, added) the towns between them.
@fragment
fn fs_lights(in: Varying) -> @location(0) vec4<f32> {
    let live = liveable(in.position.xy);
    var lights = 0.0;
    if (live > 0.01) {
        lights = live * settlements(sphere_at(in.position.xy / gen.size.xy), 120.0, 0.7, 11) * 1.8;
    }
    return vec4<f32>(clamp(lights, 0.0, 1.0), 0.0, 0.0, 1.0);
}

@fragment
fn fs_towns(in: Varying) -> @location(0) vec4<f32> {
    let live = liveable(in.position.xy);
    var lights = 0.0;
    if (live > 0.01) {
        lights = live * settlements(sphere_at(in.position.xy / gen.size.xy), 420.0, 0.6, 29) * 0.7;
    }
    return vec4<f32>(clamp(lights, 0.0, 1.0), 0.0, 0.0, 0.0);
}

// Clouds: wound into the storms, stretched along the latitudes, banded like
// the circulation cells, thinner over dry land.
@fragment
fn fs_clouds(in: Varying) -> @location(0) vec4<f32> {
    let cl = climate(in.position.xy);
    let o = gen.offsets;
    var c = cl.p;
    for (var i = 0; i < 12; i++) {
        let storm = gen.storms[i];
        let d = distance(c, storm.xyz);
        c = turn_about(c, storm.xyz, storm.w * exp(-d * d / (0.22 * 0.22)));
    }
    let stretched = vec3<f32>(c.x, c.y * 1.9, c.z);
    let cwarp = vec3<f32>(
        fbm(stretched * 2.0 + o[1].xyz, 3),
        fbm(stretched * 2.0 + o[2].xyz, 3),
        fbm(stretched * 2.0 + o[3].xyz, 3),
    );
    let sheet = fbm(stretched * 3.5 + cwarp * 0.9 + o[0].xyz, 9);
    let itcz = exp(-pow((cl.lat * 57.2958 - 4.0) / 7.0, 2.0));
    let cover = sheet + 0.10 * cos(6.0 * cl.lat) + 0.12 * itcz + 0.16 * (cl.moisture - 0.5) * cl.land - 0.10;
    var cloud = smoothstep(0.0, 0.42, cover);
    // Finer breakup at the edges.
    cloud *= smoothstep(-0.45, 0.25, fbm(c * 26.0 + o[3].yzx, 4) + cloud * 0.6);
    return vec4<f32>(clamp(cloud, 0.0, 1.0), 0.0, 0.0, 1.0);
}
"#;

/// The frame shader, with the fixed lighting numbers folded in.
pub(crate) fn shader_source() -> String {
    let mut source = String::from(NOISE);
    source.push_str(crate::noise3d::WGSL);
    let constants = format!(
        "const RELIEF: f32 = {relief};\n\
         const NIGHT_GAIN: f32 = {night_gain};\n\
         const BIAS: f32 = {bias};\n\
         const FRAME_EDGE: f32 = {frame_edge};\n\
         const CLOUD_ALBEDO: f32 = {cloud_albedo};\n\
         const CLOUD_HEIGHT: f32 = {cloud_height};\n\
         const SCALE_HEIGHT: f32 = {scale_height};\n\
         const MAP_TEXEL: f32 = {texel};\n\
         const AIR: vec3<f32> = vec3<f32>({air_r}, {air_g}, {air_b});\n\
         const SUNLIGHT: vec3<f32> = vec3<f32>({sun_r}, {sun_g}, {sun_b});\n",
        relief = RELIEF,
        night_gain = NIGHT_GAIN,
        bias = DETAIL_BIAS,
        frame_edge = 1.0 / PLANET,
        cloud_albedo = CLOUD_ALBEDO,
        cloud_height = CLOUD_HEIGHT,
        scale_height = SCALE_HEIGHT,
        texel = TAU / MAP_WIDTH as f32,
        air_r = AIR[0],
        air_g = AIR[1],
        air_b = AIR[2],
        sun_r = SUNLIGHT,
        sun_g = SUNLIGHT * 0.98,
        sun_b = SUNLIGHT * 0.95,
    );
    source.push_str(&constants);
    source.push_str(FRAME_SHADER);
    source
}

const FRAME_SHADER: &str = r#"
// The frame is drawn in four small pipelines, one after another in a single
// render pass, rather than one large one: each stays within the V3D's
// four-thread register budget, and the tile buffer blends them on chip.
// They write linear light, premultiplied, through an sRGB view of the
// frame, which encodes on the way out.
//
// - `fs_background`, full screen: space behind the planet and the air seen
//   edge-on past its limb.
// - `fs_ground`, over the planet's disc: the lit ground, its relief, the
//   clouds' shadows, the sea's glint, dimmed by the air in front of it.
// - `fs_clouds`, blended over it: the cloud deck.
// - `fs_glow`, added: city lights and earthshine on the night side, and the
//   blue the air in front of the planet scatters toward the viewer.
//
// `NEAR` builds the low-orbit versions of the ground and the clouds, with
// close-up detail where a map texel covers several pixels.

struct Uniforms {
    frame: vec4<f32>,
    camera: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    spin: vec4<f32>,
    sun: vec4<f32>,
    // The planet's disc on screen, in clip space: min x, min y, max x, max y.
    disk: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var day_map: texture_2d<f32>;
@group(0) @binding(2) var night_map: texture_2d<f32>;
@group(0) @binding(3) var cloud_map: texture_2d<f32>;
@group(0) @binding(4) var relief_map: texture_2d<f32>;
@group(0) @binding(5) var map_sampler: sampler;

override NEAR: bool = false;

// `Theme.bg` (#07070a), the window colour behind the planet, in linear light.
const BACKGROUND: vec3<f32> = vec3<f32>(0.0024, 0.0024, 0.0033);
const SKYLIGHT: vec3<f32> = vec3<f32>(0.035, 0.060, 0.110);
const ATMOSPHERE: f32 = 1.075;

fn smoothstep01(edge0: f32, edge1: f32, x: f32) -> f32 {
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn airmass(mu: f32) -> f32 {
    let m = max(mu, -0.02);
    return 1.0 / (m + 0.025 + 0.045 * exp(-m * 30.0));
}

fn sunlight(mu: f32) -> vec3<f32> {
    return SUNLIGHT * exp(-AIR * airmass(mu)) * smoothstep01(-0.035, 0.02, mu);
}

struct Varying {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

// A quad over the planet's disc, so the planet's pipelines shade nothing else.
@vertex
fn vs_disk(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    var out: Varying;
    out.position = vec4<f32>(mix(params.disk.xy, params.disk.zw, corners[index]), 0.0, 1.0);
    return out;
}

struct Ray {
    dir: vec3<f32>,
    // The ground point, or where the ray comes closest to the planet.
    n: vec3<f32>,
    // Where it crosses the cloud deck.
    nc: vec3<f32>,
    // Its closest approach to the centre, in radii.
    closest: f32,
    // Distance to the ground, or to the closest approach.
    distance: f32,
};

fn trace(position: vec2<f32>) -> Ray {
    let half = params.frame.x;
    let s = vec2<f32>((position.x - half) / half, (half - position.y) / half);
    let dir = normalize(params.forward.xyz + (params.right.xyz * s.x + params.up.xyz * s.y) * params.frame.y);
    let c = params.camera.xyz;
    let along = max(-dot(c, dir), 0.0);
    let nearest = c + dir * along;
    let b = length(nearest);
    let hit = 1.0 - b * b;
    let t = along - sqrt(max(hit, 0.0));
    var ray: Ray;
    ray.dir = dir;
    ray.closest = b;
    ray.distance = select(along, t, hit > 0.0);
    ray.n = select(nearest / max(b, 1e-6), normalize(c + dir * t), hit > 0.0);
    let rc = 1.0 + CLOUD_HEIGHT;
    let deck = rc * rc - b * b;
    let tc = along - sqrt(max(deck, 0.0));
    ray.nc = select(ray.n, normalize(c + dir * tc), hit > 0.0 && deck > 0.0);
    return ray;
}

// How much of the pixel the planet covers: a pixel either side of the limb.
fn coverage(ray: Ray) -> f32 {
    let edge = (ray.closest - 1.0) / max(ray.distance * params.frame.z, 1e-6);
    return smoothstep01(-0.75, 0.75, -edge);
}

fn map_uv(n: vec3<f32>, turns: f32) -> vec2<f32> {
    return vec2<f32>(0.5 + atan2(n.x, n.z) / TAU + turns, 0.5 - asin(clamp(n.y, -1.0, 1.0)) / PI);
}

// A screen derivative of map coordinates, across the date line's wrap.
fn unwrap(g: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(g.x - round(g.x), g.y);
}

// The mip level the maps need where their coordinates change by these per
// pixel: one level per sample instead of two gradients.
fn level(uv: vec2<f32>) -> f32 {
    let size = vec2<f32>(textureDimensions(day_map, 0));
    let gx = unwrap(dpdx(uv)) * size;
    let gy = unwrap(dpdy(uv)) * size;
    return 0.5 * log2(max(max(dot(gx, gx), dot(gy, gy)), 1e-8)) + BIAS;
}

// The ground's tangent frame at `n`.
fn east_of(n: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(n.z, 0.0, -n.x) / sqrt(max(n.x * n.x + n.z * n.z, 1e-8));
}

// How far the clouds' shadows fall, in map coordinates, from where the sun
// stands over `n`.
fn shadow_shift(n: vec3<f32>, mu_sun: f32) -> vec2<f32> {
    let sun = params.sun.xyz;
    let east = east_of(n);
    let cos_lat = sqrt(max(n.x * n.x + n.z * n.z, 1e-8));
    let reach = CLOUD_HEIGHT / max(mu_sun, 0.12);
    return vec2<f32>(dot(sun, east) * reach / (TAU * max(cos_lat, 0.05)), -dot(sun, cross(n, east)) * reach / PI);
}

// The air's transmittance toward the viewer from the ground at `mu_view`.
fn through(ray: Ray) -> vec3<f32> {
    return exp(-AIR * airmass(clamp(dot(ray.n, -ray.dir), 0.0, 1.0)));
}

// ---------------------------------------------------------------------------

fn space(position: vec2<f32>) -> vec3<f32> {
    let centre = vec2<f32>(params.frame.x);
    let d = length(position - centre);
    let t = d / length(centre);
    let inner = vec4<f32>(vec3<f32>(0.2941, 0.5255, 0.8471) * 0.0784, 0.0784);
    let middle = vec4<f32>(vec3<f32>(0.1412, 0.2902, 0.4902) * 0.0627, 0.0627);
    var air = select(
        mix(middle, vec4<f32>(0.0), clamp((t - 0.52) / 0.24, 0.0, 1.0)),
        mix(inner, middle, t / 0.52),
        t < 0.52,
    );
    air *= step(d, centre.x) * (1.0 - params.frame.w);
    // The gradient was a Slint one, laid in sRGB: back to linear.
    return pow(air.rgb + vec3<f32>(0.0275, 0.0275, 0.0392) * (1.0 - air.a), vec3<f32>(2.2));
}

@fragment
fn fs_background(in: Varying) -> @location(0) vec4<f32> {
    let ray = trace(in.position.xy);
    var colour = space(in.position.xy);
    let r = ray.closest;
    if (r < ATMOSPHERE) {
        // The air edge-on: a glow where the sun lights it, thinning out with
        // height; on the night side only the faint green airglow.
        let h = max(r - 1.0, 0.0);
        let depth = AIR * sqrt(TAU * r / SCALE_HEIGHT) * exp(-h / SCALE_HEIGHT);
        let mu = dot(ray.n, params.sun.xyz);
        let lit = SUNLIGHT * exp(-AIR * airmass(mu + 0.08) * 0.6) * smoothstep01(-0.30, 0.05, mu);
        var glow = lit * (vec3<f32>(1.0) - exp(-depth)) * 0.9;
        let night = 1.0 - smoothstep01(-0.25, 0.05, mu);
        let band = (h - SCALE_HEIGHT * 1.4) / (SCALE_HEIGHT * 0.35);
        glow += vec3<f32>(0.10, 0.32, 0.12) * exp(-band * band) * night * 0.03;
        // In the far view, faded out before the square frame can cut it off.
        let half = params.frame.x;
        let screen = length(in.position.xy - vec2<f32>(half)) / (half * 0.93);
        let guard = mix(1.0 - smoothstep01(FRAME_EDGE * 0.92, FRAME_EDGE, screen), 1.0, params.frame.w);
        colour += glow * guard;
    }
    return vec4<f32>(colour, 1.0);
}

// Close-up detail where a map texel covers several pixels: three octaves of
// value noise and their slope, fading in as the footprint shrinks.
fn detail(n: vec3<f32>) -> vec4<f32> {
    var sum = noised(n * 1500.0) * vec4<f32>(0.5, vec3<f32>(1500.0 * 0.5));
    let b = noised(OCTAVE * n * 3100.0);
    sum += vec4<f32>(b.x * 0.25, (transpose(OCTAVE) * b.yzw) * 3100.0 * 0.25);
    let c = noised(n.zxy * 6300.0);
    sum += vec4<f32>(c.x * 0.125, c.yzw.yzx * 6300.0 * 0.125);
    return sum;
}

// How much close-up detail a ray's ground point gets.
fn closeness(ray: Ray) -> f32 {
    return smoothstep01(MAP_TEXEL * 1.5, MAP_TEXEL * 0.25, ray.distance * params.frame.z);
}

@fragment
fn fs_ground(in: Varying) -> @location(0) vec4<f32> {
    let ray = trace(in.position.xy);
    let uv = map_uv(ray.n, params.spin.x);
    let lod = level(uv);
    let cover = coverage(ray);
    if (cover <= 0.0) {
        discard;
    }
    let n = ray.n;
    let sun = params.sun.xyz;
    let mu_sun = dot(n, sun);
    let east = east_of(n);
    let relief = textureSampleLevel(relief_map, map_sampler, uv, lod + 0.4);
    let water = relief.b;
    let bumped = n + (east * (relief.r - 0.5) + cross(n, east) * (relief.g - 0.5)) * RELIEF;
    var shadow = 0.0;
    if (mu_sun > -0.05) {
        let cloud_uv = vec2<f32>(uv.x + params.spin.y - params.spin.x, uv.y) + shadow_shift(n, mu_sun);
        shadow = textureSampleLevel(cloud_map, map_sampler, cloud_uv, lod + 1.0).r;
    }
    let sun_ground = sunlight(mu_sun);
    let sky = SKYLIGHT * smoothstep01(-0.15, 0.4, mu_sun);
    let day = textureSampleLevel(day_map, map_sampler, uv, lod).rgb;
    let colour = day * mix(1.0, 0.8, water)
        * (sun_ground * max(dot(normalize(bumped), sun) + 0.03, 0.0) * (1.0 - 0.6 * shadow) + sky);
    return vec4<f32>(colour * through(ray) * cover, cover);
}

// Low orbit only, multiplied into the ground: where a map texel covers
// several pixels, value noise breaks up its colour and tilts its light.
// Drawn as its own pass, so the ground keeps four threads.
@fragment
fn fs_detail(in: Varying) -> @location(0) vec4<f32> {
    let ray = trace(in.position.xy);
    let close = closeness(ray) * step(0.0, coverage(ray) - 0.999);
    if (close <= 0.0) {
        discard;
    }
    let n = ray.n;
    let uv = map_uv(n, params.spin.x);
    let water = textureSampleLevel(relief_map, map_sampler, uv, 0.0).b;
    let d = detail(n);
    let sun = params.sun.xyz;
    // The light the detail's slope adds or takes, against the smooth ground.
    let mu = dot(n, sun);
    let tilted = dot(normalize(n - (d.yzw - n * dot(d.yzw, n)) * 0.00022 * close * (1.0 - water)), sun);
    let shading = clamp((max(tilted, 0.0) + 0.03) / (max(mu, 0.0) + 0.03), 0.6, 1.6);
    let grain = 1.0 + d.x * close * mix(0.3, 0.08, water);
    return vec4<f32>(vec3<f32>(grain * mix(1.0, shading, smoothstep01(-0.05, 0.1, mu))), 1.0);
}

// The sea, added to the ground: a calm sea mirrors the sun, a tight glint
// and a broad sheen, both stronger at a grazing angle and hidden under
// cloud, and the sky's reflection.
@fragment
fn fs_sea(in: Varying) -> @location(0) vec4<f32> {
    let ray = trace(in.position.xy);
    let uv = map_uv(ray.n, params.spin.x);
    let lod = level(uv);
    let cover = coverage(ray);
    let water = textureSampleLevel(relief_map, map_sampler, uv, lod + 0.4).b * cover;
    if (water <= 0.0) {
        discard;
    }
    let n = ray.n;
    let sun = params.sun.xyz;
    let mu_sun = dot(n, sun);
    let fresnel = 0.02 + 0.98 * pow(1.0 - clamp(dot(n, -ray.dir), 0.0, 1.0), 5.0);
    let cloud = textureSampleLevel(cloud_map, map_sampler, map_uv(ray.nc, params.spin.y), lod).r;
    let log_nh = log2(max(dot(n, normalize(sun - ray.dir)), 0.0));
    let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
    let colour = sunlight(mu_sun) * glint * (0.25 + fresnel) * (1.0 - cloud)
        + SKYLIGHT * 2.0 * fresnel * smoothstep01(-0.1, 0.3, mu_sun);
    return vec4<f32>(colour * water * through(ray), 0.0);
}

@fragment
fn fs_clouds(in: Varying) -> @location(0) vec4<f32> {
    let ray = trace(in.position.xy);
    let cuv = map_uv(ray.nc, params.spin.y);
    let lod = level(cuv);
    // Everything that needs the whole ray, first, so only the two points
    // stay live past here.
    let cover = coverage(ray);
    let air = through(ray);
    let n = ray.n;
    let nc = ray.nc;
    var wisp = 0.0;
    if (NEAR) {
        let close = closeness(ray);
        if (close > 0.0) {
            // One fetch (a second would cost the pass two threads).
            wisp = noise(nc * 520.0) * 0.9 * close * 1.4;
        }
    }
    var cloud = textureSampleLevel(cloud_map, map_sampler, cuv, lod).r;
    cloud = clamp(cloud + wisp * cloud * (1.0 - cloud), 0.0, 1.0);
    let alpha = cloud * cover;
    if (alpha <= 0.002) {
        discard;
    }
    let mu_sun = dot(n, params.sun.xyz);
    // The deck a little toward the sun shades the clouds' far sides.
    var side = 1.0;
    if (mu_sun > -0.05) {
        let toward = textureSampleLevel(cloud_map, map_sampler, cuv + shadow_shift(n, mu_sun) * 0.35, lod).r;
        side = clamp(1.0 - 1.4 * (toward - cloud), 0.55, 1.15);
    }
    // The cloud tops stand above most of the air, so they keep the light and
    // its sunset colour a little longer than the ground.
    let sun_high = SUNLIGHT * exp(-AIR * airmass(mu_sun + 0.04) * 0.55);
    let sky = SKYLIGHT * smoothstep01(-0.15, 0.4, mu_sun);
    let lit = CLOUD_ALBEDO * (sun_high * smoothstep01(-0.08, 0.04, mu_sun) * (0.30 + 0.70 * sqrt(max(mu_sun, 0.0))) * side + sky * 1.5);
    return vec4<f32>(lit * air * alpha, alpha);
}

@fragment
fn fs_glow(in: Varying) -> @location(0) vec4<f32> {
    let ray = trace(in.position.xy);
    let uv = map_uv(ray.n, params.spin.x);
    let lod = level(uv);
    let cover = coverage(ray);
    if (cover <= 0.0) {
        discard;
    }
    let n = ray.n;
    let mu_sun = dot(n, params.sun.xyz);
    let air = through(ray);
    var colour = vec3<f32>(0.0);
    // The night side: city lights, warm where faint and whiter where dense,
    // dimmed by cloud, and a faint bluish earthshine.
    let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu_sun);
    if (darkness > 0.002) {
        let city = textureSampleLevel(night_map, map_sampler, uv, lod + 0.6).r;
        let cloud = textureSampleLevel(cloud_map, map_sampler, map_uv(ray.nc, params.spin.y), lod).r;
        let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, city));
        colour += tint * (city * city * NIGHT_GAIN * (1.0 - cloud * 0.85));
        let day = textureSampleLevel(day_map, map_sampler, uv, lod + 2.0).rgb;
        colour += vec3<f32>(0.004, 0.006, 0.012) * (day + cloud);
        colour *= darkness * air;
    }
    // The air in front of the ground adds its own blue, thickening toward
    // the limb.
    let sun_high = SUNLIGHT * exp(-AIR * airmass(mu_sun + 0.04) * 0.55);
    colour += sun_high * smoothstep01(-0.10, 0.08, mu_sun) * (vec3<f32>(1.0) - air) * 0.45;
    return vec4<f32>(colour * cover, 0.0);
}
"#;

impl Visual for Planet {
    fn render(&mut self, gpu: &Graphics, seconds: f64, _seed: u64, frame: usize) -> Result<()> {
        Planet::render(self, gpu, seconds, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        Planet::texture(self, frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sea_level_leaves_the_share_asked_for_under_water() {
        // A ramp from -1 to 1 down the rows, equal along each: the area-weighted
        // median is the equator's height.
        let (w, h) = (8, 64);
        let heights: Vec<f32> = (0..w * h)
            .map(|i| ((i / w) as f32 / (h - 1) as f32) * 2.0 - 1.0)
            .collect();
        let level = sea_level(&heights, w, h, 0.5);
        assert!(level.abs() < 0.05, "got {level}");
    }

    #[test]
    fn the_camera_dives_and_comes_back() {
        assert_eq!(view_at(0.0), 0.0);
        assert_eq!(view_at(HOLD_FAR + DIVE + 1.0), 1.0);
        let (far, ..) = camera(0.0);
        let (near, ..) = camera(1.0);
        let length = |v: [f32; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!((length(far) - FAR).abs() < 1e-3);
        assert!((length(near) - 1.0 - ALTITUDE).abs() < 1e-4);
    }

    /// Write previews to `PLANET_PREVIEW_DIR`: the far view through a day, the
    /// dive, the horizon, and the generated maps themselves.
    #[test]
    #[ignore = "writes design previews; set PLANET_PREVIEW_DIR"]
    fn previews() {
        let Ok(dir) = std::env::var("PLANET_PREVIEW_DIR") else {
            return;
        };
        let seed = std::env::var("PLANET_SEED")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let planet = Planet::with_seed(&gpu, seed).expect("a planet");
        println!(
            "maps generated in {:.0} ms",
            planet.generated.as_secs_f64() * 1e3
        );
        // Turned so seed 7's main continent is in view; `PLANET_SPIN` adds to it.
        let extra: f64 = std::env::var("PLANET_SPIN")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0);
        let shots: [(&str, f64, f32, f64); 7] = [
            ("globe-day", 0.0, 0.0, 0.33),
            ("globe-dusk", 0.18 * SUN_PERIOD as f64, 0.0, 0.33),
            ("globe-night", 0.5 * SUN_PERIOD as f64, 0.0, 0.33),
            ("dive", 20.0, 0.55, 0.2),
            ("horizon-day", 30.0, 1.0, 0.15),
            ("horizon-sunset", 0.2 * SUN_PERIOD as f64, 1.0, 0.15),
            ("horizon-night", 0.45 * SUN_PERIOD as f64, 1.0, 0.15),
        ];
        for (k, (name, channels)) in [("day", 4u32), ("relief", 4), ("night", 1), ("clouds", 1)]
            .into_iter()
            .enumerate()
        {
            let pixels = read_texture(&gpu, &planet.maps[k], MAP_WIDTH, MAP_HEIGHT, channels);
            let colour = if channels == 4 {
                image::ExtendedColorType::Rgba8
            } else {
                image::ExtendedColorType::L8
            };
            let path = std::path::Path::new(&dir).join(format!("map-{name}.png"));
            image::save_buffer(&path, &pixels, MAP_WIDTH, MAP_HEIGHT, colour).expect("a map image");
        }
        for (name, seconds, near, turned) in shots {
            let pixels = planet.read_back(&gpu, seconds, near, turned + extra);
            let path = std::path::Path::new(&dir).join(format!("planet-{name}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a preview image");
        }
    }

    /// Time the generator and the frames in both views.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let started = std::time::Instant::now();
        let planet = Planet::with_seed(&gpu, 7).expect("a planet");
        println!(
            "planet setup: {:.0} ms (maps {:.0} ms)",
            started.elapsed().as_secs_f64() * 1e3,
            planet.generated.as_secs_f64() * 1e3
        );
        for (label, near) in [("globe", 0.0f32), ("dive", 0.55), ("horizon", 1.0)] {
            planet.draw(&gpu, 0.0, near, 0.0, 0);
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            let frames = 60;
            let started = std::time::Instant::now();
            for i in 0..frames {
                let seconds = i as f64 * SUN_PERIOD as f64 / frames as f64;
                planet.draw(&gpu, seconds, near, seconds / REVOLUTION as f64, i % SLOTS);
            }
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            let mean = started.elapsed().as_secs_f64() / frames as f64;
            println!("planet {label} frame, back to back: {:.3} ms", mean * 1e3);
        }
    }
}
