//! The rotating Earth on the idle screen, drawn on the GPU with Vulkan.
//!
//! wgpu renders the sphere as a fragment shader. The maps are uploaded at their
//! native resolution — nothing is resampled on the CPU — and the GPU builds a mip
//! chain for each, which is what stops the city lights shimmering as they are
//! minified. Frames are drawn into a small ring of textures that Slint samples
//! directly, on the same device it renders the UI with, so a frame never leaves the
//! GPU: no readback and no upload.
//!
//! Maps, all distributed by <https://www.solarsystemscope.com/textures/> (CC BY 4.0):
//! day and night (NASA Blue Marble / Black Marble), a cloud sheet, a normal map for
//! terrain relief and a specular map for sun glint on water.

use anyhow::{Context, Result};

use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;

const DAY: &[u8] = include_bytes!("../assets/earth/day.jpg");
const NIGHT: &[u8] = include_bytes!("../assets/earth/night.jpg");
const CLOUDS: &[u8] = include_bytes!("../assets/earth/clouds.jpg");
const NORMAL: &[u8] = include_bytes!("../assets/earth/normal.tif");
const SPECULAR: &[u8] = include_bytes!("../assets/earth/specular.tif");

/// Size of the rendered frame. The view draws it 1:1, so nothing is scaled.
pub const SIZE: u32 = 920;
/// Seconds for one revolution of the planet.
const REVOLUTION: f32 = 192.0;
/// How much brighter the city lights are than in the source map.
const NIGHT_GAIN: f32 = 1.4;
/// Faint fill on the night side, so a fully dark hemisphere is still a planet.
const NIGHTSHINE: f32 = 0.04;
/// Albedo of the cloud sheet: white, so it stays bright where the ground is dark.
const CLOUD_ALBEDO: f32 = 0.9;
/// How much brighter the cloud tops are than the ground once it is dark.
const CLOUD_NIGHT: f32 = 0.05;
/// How strongly the air glows along the terminator.
const DUSK_GAIN: f32 = 0.16;
/// Frames the readback is allowed to be behind, so the GPU is never waited on.
pub(crate) const SLOTS: usize = crate::visuals::SLOTS;
/// Extra mip levels to skip when sampling the maps. A slightly softer planet,
/// but it stops the city lights and the relief sparkling as the surface turns.
const DETAIL_BIAS: f32 = 0.7;
/// Planet radius as a fraction of the frame's half-size, leaving room for the halo.
const PLANET: f32 = 0.95;
/// Outer radius of the atmosphere, as a multiple of the planet's radius.
const ATMOSPHERE: f32 = 1.17;
/// Colour of the atmosphere.
const HALO: [f32; 3] = [0.32, 0.58, 1.0];
/// Colour of the light scattered along the terminator.
const DUSK: [f32; 3] = [1.0, 0.62, 0.34];
/// How strongly the normal map tilts the lighting.
const RELIEF: f32 = 0.55;
/// Width of the terminator, in dot-product units. A wide one reads as an
/// atmosphere rather than a switch.
const TERMINATOR: f32 = 0.42;
/// How far sunlight carries past the terminator, as a fraction of full daylight.
const TWILIGHT_CARRY: f32 = 0.22;
/// Width of the twilight band around the terminator, in dot-product units.
const TWILIGHT: f32 = 0.55;
/// Direction of the sun at the start, in screen space: front-left and a little
/// high. It circles from here, see `SUN_PERIOD`.
const SUN: [f32; 3] = [-0.62, 0.22, 0.75];
/// Seconds for the sun to travel once around the planet, so the visible side
/// drifts from full day to full night and back over half an hour.
const SUN_PERIOD: f32 = 1800.0;

/// One decoded equirectangular map, still on the CPU.
struct Map {
    width: u32,
    height: u32,
    /// RGB, three bytes per texel, exactly as decoded.
    pixels: Vec<u8>,
}

impl Map {
    fn decode(bytes: &[u8]) -> Result<Self> {
        let image = image::load_from_memory(bytes)
            .context("decoding an Earth map")?
            .to_rgb8();
        Ok(Self {
            width: image.width(),
            height: image.height(),
            pixels: image.into_raw(),
        })
    }

    /// Expand to RGBA for the GPU; the colour maps keep their channels.
    fn to_rgba(&self) -> Vec<u8> {
        let (texels, _) = self.pixels.as_chunks::<3>();
        let mut out = Vec::with_capacity(texels.len() * 4);
        for texel in texels {
            out.extend_from_slice(&[texel[0], texel[1], texel[2], 255]);
        }
        out
    }

    /// Luminance, for the cloud and water masks.
    fn to_grey(&self) -> Vec<u8> {
        let (texels, _) = self.pixels.as_chunks::<3>();
        texels
            .iter()
            .map(|texel| {
                ((texel[0] as u32 * 77 + texel[1] as u32 * 150 + texel[2] as u32 * 29) >> 8) as u8
            })
            .collect()
    }

    /// Shrink the map if the GPU cannot hold it at its native size. This is the
    /// only place anything is resampled, and it never happens on the Pi.
    fn fit(&mut self, limit: u32) {
        if self.width <= limit && self.height <= limit {
            return;
        }
        let scale = limit as f32 / self.width.max(self.height) as f32;
        let width = ((self.width as f32 * scale) as u32).clamp(1, limit);
        let height = ((self.height as f32 * scale) as u32).clamp(1, limit);
        let Some(source) =
            image::RgbImage::from_raw(self.width, self.height, std::mem::take(&mut self.pixels))
        else {
            return;
        };
        let resized = image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::Lanczos3,
        );
        tracing::warn!(
            from = format!("{}x{}", self.width, self.height),
            to = format!("{width}x{height}"),
            "the GPU cannot hold this map, shrinking it"
        );
        self.width = width;
        self.height = height;
        self.pixels = resized.into_raw();
    }
}

/// One frame of the planet: a texture Slint samples directly.
struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Per-frame parameters, laid out as 16-byte aligned vectors.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// x, y: centre in pixels; z: planet radius in pixels; w: atmosphere radius in pixels.
    frame: [f32; 4],
    /// x: surface rotation in turns. The clouds are part of the same surface, so
    /// they turn with it.
    spin: [f32; 4],
    /// x, y, z: sun direction.
    sun: [f32; 4],
    /// x, y, z: half vector between the sun and the viewer.
    gloss: [f32; 4],
}

/// Everything needed to draw one frame of the planet.
pub(crate) struct Globe {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    /// A ring of render targets, so the frame Slint is drawing is never the one
    /// being written: nothing in the loop ever waits.
    frames: Vec<Frame>,
}

impl Globe {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        let device = &gpu.device;
        let queue = &gpu.queue;
        let limit = device.limits().max_texture_dimension_2d;

        // Bound in shader order: day, night, clouds, relief, water.
        let sources = [
            ("day", DAY, true),
            ("night", NIGHT, true),
            ("clouds", CLOUDS, false),
            ("relief", NORMAL, true),
            ("water", SPECULAR, false),
        ];
        let mips = MipChain::new(device);
        let mut views = Vec::with_capacity(sources.len());
        // One map at a time, so the 8K sheets never all sit in RAM at once.
        for (name, bytes, colour) in sources {
            let mut map = Map::decode(bytes)?;
            map.fit(limit);
            let (format, texel_bytes, data) = if colour {
                (wgpu::TextureFormat::Rgba8Unorm, 4, map.to_rgba())
            } else {
                (wgpu::TextureFormat::R8Unorm, 1, map.to_grey())
            };
            views.push(upload(
                device,
                queue,
                &mips,
                name,
                map.width,
                map.height,
                format,
                &data,
                texel_bytes,
            )?);
        }

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Earth sampler"),
            // Longitude wraps around; latitude clamps at the poles.
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            // Keeps the grazing view near the limb from aliasing. Drivers without
            // anisotropy (the Pi's V3D) simply clamp this to 1.
            anisotropy_clamp: 16,
            ..Default::default()
        });

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Globe uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Globe bind group"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                texture(1),
                texture(2),
                texture(3),
                texture(4),
                texture(5),
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Globe bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&views[0]),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&views[1]),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::TextureView(&views[2]),
                },
                wgpu::BindGroupEntry {
                    binding: 4,
                    resource: wgpu::BindingResource::TextureView(&views[3]),
                },
                wgpu::BindGroupEntry {
                    binding: 5,
                    resource: wgpu::BindingResource::TextureView(&views[4]),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Globe shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source().into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Globe pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Globe pipeline"),
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
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // One triangle covers the whole frame, so nothing blends; the
                    // shader premultiplies, which is what Slint's RGBA images want.
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

        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Globe frame"),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // Slint needs to sample it; COPY_SRC is only used by the previews.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                Frame { texture, view }
            })
            .collect();

        tracing::info!("Earth maps uploaded");
        Ok(Self {
            pipeline,
            uniforms,
            bind_group,
            frames,
        })
    }

    /// Draw the frame for `seconds` of run time into `frame` and submit it,
    /// without waiting for the GPU.
    fn render(&self, gpu: &Graphics, seconds: f64, frame: usize) -> Result<()> {
        let half = self.size() as f32 / 2.0;
        let turns = (seconds / REVOLUTION as f64) as f32;
        let sun = sun_at(seconds);
        let uniforms = Uniforms {
            frame: [half, half, half * PLANET, half * PLANET * ATMOSPHERE],
            spin: [turns.rem_euclid(1.0), 0.0, 0.0, 0.0],
            sun: [sun[0], sun[1], sun[2], 0.0],
            gloss: half_vector(sun),
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Globe encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Globe pass"),
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    /// The frame as a wgpu texture, which is what crosses to the UI thread: an
    /// image wrapping it is then made there, with no copy in between.
    fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    /// Read one frame back as RGBA pixels. Only the design previews need this: the
    /// running display hands the texture to Slint instead.
    #[cfg(test)]
    fn read_back(&self, gpu: &Graphics, seconds: f64) -> Vec<u8> {
        self.render(gpu, seconds, 0).expect("a frame");
        let padded_row = (SIZE * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Preview readback"),
            size: (padded_row as u64) * (SIZE as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Preview readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.frames[0].texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded_row),
                    rows_per_image: Some(SIZE),
                },
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: 1,
            },
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let mut pixels = Vec::with_capacity((SIZE * SIZE * 4) as usize);
        {
            let data = slice.get_mapped_range().expect("a mapped frame");
            for row in 0..SIZE as usize {
                let start = row * padded_row as usize;
                pixels.extend_from_slice(&data[start..start + (SIZE * 4) as usize]);
            }
        }
        buffer.unmap();
        pixels
    }

    fn size(&self) -> u32 {
        SIZE
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

/// The sun's direction at `seconds` of run time.
///
/// It travels once around the vertical axis every [`SUN_PERIOD`], so the lit side
/// slides across the disc: the terminator sweeps in, the city lights take over,
/// and quarter of an hour later daylight comes back round.
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

fn half_vector(sun: [f32; 3]) -> [f32; 4] {
    let length = (sun[0] * sun[0] + sun[1] * sun[1] + (sun[2] + 1.0) * (sun[2] + 1.0)).sqrt();
    [
        sun[0] / length,
        sun[1] / length,
        (sun[2] + 1.0) / length,
        0.0,
    ]
}

/// Upload a map at its native size and let the GPU build the mip chain.
#[allow(clippy::too_many_arguments)]
fn upload(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    mips: &MipChain,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    data: &[u8],
    bytes_per_texel: u32,
) -> Result<wgpu::TextureView> {
    let levels = (width.max(height) as f32).log2().floor() as u32 + 1;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
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
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_DST
            | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(width * bytes_per_texel),
            rows_per_image: Some(height),
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    mips.build(device, queue, &texture, levels, format);
    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Builds mip chains on the GPU: every level is the previous one sampled with a
/// linear filter. Doing this here rather than on the CPU is what removes the
/// shimmer from the city lights when the map is minified.
struct MipChain {
    layout: wgpu::BindGroupLayout,
    pipelines: [(wgpu::TextureFormat, wgpu::RenderPipeline); 2],
    sampler: wgpu::Sampler,
}

impl MipChain {
    fn new(device: &wgpu::Device) -> Self {
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Mip chain"),
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
            label: Some("Mip chain shader"),
            source: wgpu::ShaderSource::Wgsl(MIP_SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Mip chain layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let make = |format: wgpu::TextureFormat| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("Mip chain pipeline"),
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
            label: Some("Mip chain sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        Self {
            layout,
            pipelines: [
                (
                    wgpu::TextureFormat::Rgba8Unorm,
                    make(wgpu::TextureFormat::Rgba8Unorm),
                ),
                (
                    wgpu::TextureFormat::R8Unorm,
                    make(wgpu::TextureFormat::R8Unorm),
                ),
            ],
            sampler,
        }
    }

    fn build(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        texture: &wgpu::Texture,
        levels: u32,
        format: wgpu::TextureFormat,
    ) {
        let Some((_, pipeline)) = self.pipelines.iter().find(|(f, _)| *f == format) else {
            tracing::warn!(?format, "no mip pipeline for this map format");
            return;
        };
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Mip chain"),
        });
        for level in 1..levels {
            let source = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level - 1,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let target = texture.create_view(&wgpu::TextureViewDescriptor {
                base_mip_level: level,
                mip_level_count: Some(1),
                ..Default::default()
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Mip chain"),
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
                label: Some("Mip chain pass"),
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
        queue.submit(std::iter::once(encoder.finish()));
    }
}

/// The mip chain shader: a full-screen triangle that copies one level into the
/// next with a linear filter.
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

/// The globe shader, with the fixed lighting numbers folded in.
fn shader_source() -> String {
    format!(
        r#"
struct Uniforms {{
    frame: vec4<f32>,
    spin: vec4<f32>,
    sun: vec4<f32>,
    gloss: vec4<f32>,
}};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var day_map: texture_2d<f32>;
@group(0) @binding(2) var night_map: texture_2d<f32>;
@group(0) @binding(3) var cloud_map: texture_2d<f32>;
@group(0) @binding(4) var relief_map: texture_2d<f32>;
@group(0) @binding(5) var water_map: texture_2d<f32>;
@group(0) @binding(6) var map_sampler: sampler;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const HALO: vec3<f32> = vec3<f32>({halo_r}, {halo_g}, {halo_b});
const RELIEF: f32 = {relief};
const TERMINATOR: f32 = {terminator};
const NIGHT_GAIN: f32 = {night_gain};
const NIGHTSHINE: f32 = {nightshine};
const BIAS: f32 = {bias};
const FRAME_EDGE: f32 = {frame_edge};
const DUSK: vec3<f32> = vec3<f32>({dusk_r}, {dusk_g}, {dusk_b});
const DUSK_GAIN: f32 = {dusk_gain};
const CLOUD_ALBEDO: f32 = {cloud_albedo};
const CLOUD_NIGHT: f32 = {cloud_night};
const TWILIGHT: f32 = {twilight};
const TWILIGHT_CARRY: f32 = {twilight_carry};

fn smoothstep01(edge0: f32, edge1: f32, x: f32) -> f32 {{
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}}

struct Varying {{
    @builtin(position) position: vec4<f32>,
}};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {{
    // A full-screen triangle; the fragment shader discards everything outside the
    // planet and its halo.
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {{
    // Screen position in planet radii, y up.
    let p = vec2<f32>(
        (in.position.x - params.frame.x) / params.frame.z,
        (params.frame.y - in.position.y) / params.frame.z,
    );
    let distance = length(p);
    // Outer radius of the atmosphere, in the same units as `distance`.
    let atmosphere = params.frame.w / params.frame.z;
    if (distance > atmosphere) {{
        discard;
    }}
    // If the planet is drawn big enough, the atmosphere would run past the square
    // frame and be cut off at its edges: fade the glow out before that can happen.
    // With a smaller planet this band sits beyond the atmosphere and does nothing.
    let guard = 1.0 - smoothstep01(FRAME_EDGE * 0.90, FRAME_EDGE, distance);

    // Outside the limb: the atmosphere, only where the sun reaches it.
    let edge = (distance - 1.0) * params.frame.z;
    if (edge > 0.0) {{
        let falloff = 1.0 - (distance - 1.0) / (atmosphere - 1.0);
        let limb = normalize(vec3<f32>(p.x, p.y, 0.35));
        let lit = smoothstep01(-0.25, 0.35, dot(limb, params.sun.xyz));
        let alpha = falloff * falloff * 0.55 * smoothstep01(0.0, 1.5, edge) * lit * guard;
        return vec4<f32>(HALO * alpha, alpha);
    }}

    let z = sqrt(max(1.0 - distance * distance, 0.0));
    let normal = vec3<f32>(p.x, p.y, z);
    let lat = asin(clamp(p.y, -1.0, 1.0));
    let lon = atan2(p.x, z) + params.spin.x * TAU;
    let u = 0.5 + lon / TAU;
    let v = 0.5 - lat / PI;

    // Tangent frame, used to tilt the normal with the relief map.
    let sin_lon = sin(lon);
    let cos_lon = cos(lon);
    let sin_lat = sin(lat);
    let cos_lat = cos(lat);
    let east = vec3<f32>(cos_lon, 0.0, -sin_lon);
    let north = vec3<f32>(-sin_lat * sin_lon, cos_lat, -sin_lat * cos_lon);
    let bump =
        textureSampleBias(relief_map, map_sampler, vec2<f32>(u, v), BIAS + 0.4).rg
        - vec2<f32>(0.5);
    let tilted = normalize(normal + (east * bump.x + north * bump.y) * RELIEF);

    let direct = dot(tilted, params.sun.xyz);
    let facing = dot(normal, params.sun.xyz);
    let light = smoothstep01(-TERMINATOR, TERMINATOR, direct);
    let darkness = 1.0 - smoothstep01(-TERMINATOR, TERMINATOR, facing);

    let surface = textureSampleBias(day_map, map_sampler, vec2<f32>(u, v), BIAS).rgb;
    // Clouds are part of the surface, so they turn with it.
    let cloud = textureSampleBias(cloud_map, map_sampler, vec2<f32>(u, v), BIAS).r;

    // Sunlight diffuses through the air a long way past the terminator, so the
    // ground fades out over a wide band instead of stopping at a line. The band
    // reaches further into the night than into the day, and is warm: what a sunset
    // on the limb looks like from orbit.
    let twilight =
        (1.0 - smoothstep01(0.0, TWILIGHT, abs(facing))) * smoothstep01(-0.45, 0.1, facing);
    let daylight = max(light, TWILIGHT_CARRY * twilight);
    let shade = NIGHTSHINE + (1.0 - NIGHTSHINE) * daylight;

    // Ground first, then the cloud sheet over it, both under the same light.
    let ground = surface * shade;
    var colour = ground + (vec3<f32>(CLOUD_ALBEDO * shade) - ground) * cloud;

    let glossy = pow(max(dot(tilted, params.gloss.xyz), 0.0), 130.0) * light;
    if (glossy > 0.01) {{
        let water = textureSampleBias(water_map, map_sampler, vec2<f32>(u, v), BIAS).r;
        colour += vec3<f32>(water * glossy * (1.0 - cloud * 0.9) * 0.5 * shade);
    }}

    if (darkness > 0.002) {{
        // The lights are the highest-frequency thing on the map, so they get the
        // most bias: a pixel that flickers at night is far more noticeable than
        // one that is a touch soft. Clouds sit above them and hide them.
        let city = textureSampleBias(night_map, map_sampler, vec2<f32>(u, v), BIAS + 0.6).rgb;
        colour +=
            city * NIGHT_GAIN * darkness * vec3<f32>(1.0, 0.82, 0.52) * (1.0 - cloud * 0.85);
        // The tops of the clouds still catch a little light of their own.
        colour += vec3<f32>(CLOUD_NIGHT) * cloud * darkness;
    }}

    // The glow of the air along the terminator.
    colour += DUSK * twilight * DUSK_GAIN;

    // Haze right at the limb, where the line of sight grazes the air.
    colour += HALO * pow(1.0 - z, 7.0) * (light + DUSK_GAIN * twilight * 0.5) * 0.45;

    let coverage = smoothstep01(-0.75, 0.75, -edge);
    return vec4<f32>(colour * coverage, coverage);
}}
"#,
        halo_r = HALO[0],
        halo_g = HALO[1],
        halo_b = HALO[2],
        relief = RELIEF,
        terminator = TERMINATOR,
        night_gain = NIGHT_GAIN,
        nightshine = NIGHTSHINE,
        bias = DETAIL_BIAS,
        frame_edge = 1.0 / PLANET,
        dusk_r = DUSK[0],
        dusk_g = DUSK[1],
        dusk_b = DUSK[2],
        dusk_gain = DUSK_GAIN,
        cloud_albedo = CLOUD_ALBEDO,
        cloud_night = CLOUD_NIGHT,
        twilight = TWILIGHT,
        twilight_carry = TWILIGHT_CARRY,
    )
}

/// Render a single frame synchronously, for the design previews.
///
/// This brings up its own adapter and textures, so it is only used by the
/// `preview` test and never by the running display.
#[cfg(test)]
pub fn render_once(seconds: f64) -> Result<Vec<u8>> {
    let gpu = crate::gpu::headless()?;
    let globe = Globe::new(&gpu)?;
    Ok(globe.read_back(&gpu, seconds))
}

impl Visual for Globe {
    fn render(&self, gpu: &Graphics, seconds: f64, _seed: u64, frame: usize) -> Result<()> {
        Globe::render(self, gpu, seconds, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        Globe::texture(self, frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the halo geometry: the frame is only opaque inside the planet and
    /// only has a faint ring just outside it. A units mix-up once painted the
    /// whole square instead, which Slint then drew as a visible box.
    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_frame_is_round() {
        let pixels = render_once(0.0).expect("a GPU adapter and an Earth frame");
        let half = SIZE / 2;
        let alpha = |x: u32, y: u32| pixels[((y * SIZE + x) * 4 + 3) as usize];

        assert_eq!(alpha(0, 0), 0, "the corner should be transparent");
        assert_eq!(
            alpha(SIZE - 1, SIZE - 1),
            0,
            "the corner should be transparent"
        );
        assert_eq!(alpha(half, half), 255, "the middle should be opaque");

        // The sun starts on the left, so the halo is only on that side.
        let limb = (half as f32 * PLANET) as u32;
        assert_eq!(alpha(half - limb + 4, half), 255, "inside the limb");
        let halo = alpha(half - limb - 3, half);
        assert!(
            halo > 0 && halo < 255,
            "the halo should be translucent, got {halo}"
        );
        // Whatever the planet size, the glow has faded out by the frame edge: it
        // must never be cut off by the square.
        assert!(
            alpha(2, half) <= 2,
            "the glow should fade out before the frame edge, got {}",
            alpha(2, half)
        );
    }

    /// The sun travels once round the planet every half hour, so the lit side
    /// sweeps across: half way through, the disc is almost entirely night.
    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_day_drifts_to_night() {
        let day = render_once(0.0).expect("a frame");
        let night = render_once(SUN_PERIOD as f64 / 2.0).expect("a frame");
        let half = SIZE / 2;
        let luminance = |pixels: &[u8]| {
            let mut total = 0u32;
            let mut count = 0u32;
            for y in (half - 120)..(half + 120) {
                for x in (half - 120)..(half + 120) {
                    let i = ((y * SIZE + x) * 4) as usize;
                    total += pixels[i] as u32 + pixels[i + 1] as u32 + pixels[i + 2] as u32;
                    count += 3;
                }
            }
            total / count
        };
        let (lit, dark) = (luminance(&day), luminance(&night));
        assert!(
            lit > dark * 3,
            "the night side should be far darker: day {lit}, night {dark}"
        );
    }
}
