//! The rotating Earth on the idle screen, drawn on the GPU with Vulkan.
//!
//! wgpu renders the sphere as a fragment shader, lit through a model of the
//! atmosphere: sunlight is reddened by the air it crosses, so the terminator
//! and the cloud tops along it glow like a sunset; the air in front of the ground
//! adds a blue haze that thickens toward the limb, and the limb itself glows
//! where the sun reaches it. The oceans catch the sun with a Fresnel glint,
//! clouds cast shadows and drift slowly over the ground, and the planet turns on
//! its tilted axis. Lighting is done in linear light and encoded once.
//!
//! The maps are 4096 × 2048: on the 850-pixel disc that is still about one and
//! a half texels per pixel at the centre, at a quarter of the memory of the 8K
//! originals. Each is stored in the smallest format that holds it — the day map
//! as sRGB, so the GPU linearises it and builds a gamma-correct mip chain; lights
//! and clouds as one channel each; relief and the water mask together — and the
//! GPU builds every mip chain, which is what stops the city lights shimmering as
//! they are minified. Frames are drawn into a small ring of textures that Slint
//! samples directly, on the same device it renders the UI with, so a frame never
//! leaves the GPU.
//!
//! Maps, all distributed by <https://www.solarsystemscope.com/textures/> (CC BY 4.0),
//! reduced from 8K: day and night (NASA Blue Marble / Black Marble), a cloud
//! sheet, a normal map for terrain relief and a specular map for water.

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
pub const SIZE: u32 = 1080;
/// Seconds for one revolution of the planet.
const REVOLUTION: f32 = 192.0;
/// How much faster than the ground the cloud sheet turns, so the weather
/// visibly moves over the continents.
const CLOUD_DRIFT: f32 = 0.035;
/// How much brighter the city lights are than in the source map.
const NIGHT_GAIN: f32 = 2.2;
/// Albedo of the cloud sheet.
const CLOUD_ALBEDO: f32 = 0.85;
/// Height of the cloud deck in planet radii, which sets how far its shadows
/// fall. Exaggerated about fourfold, so they read at this scale.
const CLOUD_HEIGHT: f32 = 0.006;
/// Frames the readback is allowed to be behind, so the GPU is never waited on.
pub(crate) const SLOTS: usize = crate::visuals::SLOTS;
/// Extra mip levels to skip when sampling the maps. A slightly softer planet,
/// but it stops the city lights and the relief sparkling as the surface turns.
const DETAIL_BIAS: f32 = 0.45;
/// Planet radius as a fraction of the frame's half-size, leaving room for the
/// halo, which reaches the very edge of the panel.
const PLANET: f32 = 0.93;
/// Outer radius of the atmosphere, as a multiple of the planet's radius.
const ATMOSPHERE: f32 = 1.075;
/// Scale height of the air in planet radii: about eight times Earth's, so the
/// limb glow is a few pixels deep rather than one.
const SCALE_HEIGHT: f32 = 0.011;
/// Vertical optical depth of the air in red, green and blue: Rayleigh
/// scattering, strongest in the blue.
const AIR: [f32; 3] = [0.045, 0.100, 0.230];
/// Brightness of the sunlight, in linear units of the day map.
const SUNLIGHT: f32 = 1.25;
/// How strongly the normal map tilts the lighting.
const RELIEF: f32 = 0.55;
/// Tilt of the planet's axis: 23.4° toward the right, and a little toward the
/// viewer so the northern hemisphere is in view.
const OBLIQUITY: f64 = 23.44;
const NOD: f64 = 12.0;
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
    /// Bytes per texel: one for the single-channel sheets (the clouds and the
    /// city lights are stored as greyscale JPEGs), three for everything else.
    channels: u32,
    /// Texels exactly as decoded.
    pixels: Vec<u8>,
}

impl Map {
    fn decode(bytes: &[u8]) -> Result<Self> {
        let image = image::load_from_memory(bytes).context("decoding an Earth map")?;
        let (width, height) = (image.width(), image.height());
        // A greyscale sheet is kept as it is: expanding it to RGB only for
        // `into_grey` to fold it back costs two passes over 8 million texels
        // and gives the very same bytes.
        let (channels, pixels) = match image {
            image::DynamicImage::ImageLuma8(grey) => (1, grey.into_raw()),
            other => (3, other.into_rgb8().into_raw()),
        };
        Ok(Self {
            width,
            height,
            channels,
            pixels,
        })
    }

    /// Expand to RGBA for the GPU; the colour maps keep their channels.
    fn to_rgba(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.pixels.len() / self.channels as usize * 4);
        if self.channels == 1 {
            for &grey in &self.pixels {
                out.extend_from_slice(&[grey, grey, grey, 255]);
            }
            return out;
        }
        let (texels, _) = self.pixels.as_chunks::<3>();
        for texel in texels {
            out.extend_from_slice(&[texel[0], texel[1], texel[2], 255]);
        }
        out
    }

    /// The relief's two slopes with `water` alongside, for one texture that
    /// answers both "which way does the ground face" and "is this sea".
    fn pack_relief(&self, water: Map) -> Vec<u8> {
        let (texels, _) = self.pixels.as_chunks::<3>();
        let (water_width, water_height) = (water.width, water.height);
        let mask = water.into_grey();
        let mut out = Vec::with_capacity(texels.len() * 4);
        for (i, texel) in texels.iter().enumerate() {
            // The water map is the same size as the relief; were it not, the
            // nearest row and column would do.
            let x = (i as u32 % self.width) * water_width / self.width;
            let y = (i as u32 / self.width) * water_height / self.height;
            let sea = mask[(y * water_width + x) as usize];
            out.extend_from_slice(&[texel[0], texel[1], sea, 255]);
        }
        out
    }

    /// Luminance, for the cloud and water masks. A greyscale map already is.
    fn into_grey(self) -> Vec<u8> {
        if self.channels == 1 {
            return self.pixels;
        }
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
        let pixels = std::mem::take(&mut self.pixels);
        let filter = image::imageops::FilterType::Lanczos3;
        let resized = if self.channels == 1 {
            image::GrayImage::from_raw(self.width, self.height, pixels)
                .map(|source| image::imageops::resize(&source, width, height, filter).into_raw())
        } else {
            image::RgbImage::from_raw(self.width, self.height, pixels)
                .map(|source| image::imageops::resize(&source, width, height, filter).into_raw())
        };
        let Some(resized) = resized else {
            return;
        };
        tracing::warn!(
            from = format!("{}x{}", self.width, self.height),
            to = format!("{width}x{height}"),
            "the GPU cannot hold this map, shrinking it"
        );
        self.width = width;
        self.height = height;
        self.pixels = resized;
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
    /// x: surface rotation in turns; y: the cloud sheet's, a little ahead.
    spin: [f32; 4],
    /// x, y, z: sun direction in the planet's frame, where y is the axis.
    sun: [f32; 4],
    /// x, y, z: half vector between the sun and the viewer, in the planet's frame.
    gloss: [f32; 4],
    /// x, y, z: sun direction on screen, for the limb.
    sky: [f32; 4],
    /// Rows of the rotation from screen space into the planet's frame.
    to_planet: [[f32; 4]; 3],
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
        let limit = device.limits().max_texture_dimension_2d;

        // Decoding the five JPEG and TIFF sheets is most of the start-up time
        // and purely CPU work, so each gets a core of its own: on the Pi's four
        // A76s the planet comes up several times sooner. Holding all the
        // decoded sheets at once costs about 50 MB, briefly.
        let decoded = std::thread::scope(|scope| {
            [DAY, NIGHT, CLOUDS, NORMAL, SPECULAR]
                .map(|bytes| scope.spawn(move || Map::decode(bytes)))
                .map(|job| {
                    job.join()
                        .unwrap_or_else(|_| Err(anyhow::anyhow!("decoding an Earth map panicked")))
                })
        });
        let [day, night, clouds, relief, water] = decoded;

        // Bound in shader order: day, night, clouds, relief with the water mask.
        let mips = MipChain::new(device);
        let mut views = Vec::with_capacity(4);
        for (name, map) in [("day", day), ("night", night), ("clouds", clouds)] {
            let mut map = map?;
            map.fit(limit);
            let (width, height) = (map.width, map.height);
            let (format, texel_bytes, data) = if name == "day" {
                (wgpu::TextureFormat::Rgba8UnormSrgb, 4, map.to_rgba())
            } else {
                (wgpu::TextureFormat::R8Unorm, 1, map.into_grey())
            };
            views.push(upload(
                gpu,
                &mips,
                name,
                width,
                height,
                format,
                &data,
                texel_bytes,
            )?);
        }
        let relief = relief?;
        let water = water?;
        views.push(upload(
            gpu,
            &mips,
            "relief",
            relief.width,
            relief.height,
            wgpu::TextureFormat::Rgba8Unorm,
            &relief.pack_relief(water),
            4,
        )?);

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
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                loaded(6),
                loaded(7),
            ],
        });

        // What each pixel of the frame sees that never changes: where on the
        // maps it lies before the planet turns, and the air between it and the
        // viewer. Worked out once, below, instead of every frame: the Pi's GPU
        // is short of arithmetic, and the inverse trigonometry and exponentials
        // behind these were a fair share of each frame's.
        let target = |label, format| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        // Single floats: a 4096-texel map needs finer steps than half floats.
        let mapping = target("Globe mapping", MAPPING);
        let air = target("Globe air", AIR_FORMAT);

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
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 6,
                    resource: wgpu::BindingResource::TextureView(&mapping),
                },
                wgpu::BindGroupEntry {
                    binding: 7,
                    resource: wgpu::BindingResource::TextureView(&air),
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

        draw_geometry(gpu, &shader, &uniforms, &mapping, &air);

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
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms_at(seconds)));

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
        gpu.submit(encoder.finish());
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

/// A texture the shader reads texel by texel, never filtered.
fn loaded(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: false },
            view_dimension: wgpu::TextureViewDimension::D2,
            multisampled: false,
        },
        count: None,
    }
}

/// Formats of the per-pixel tables: map coordinates, and the air's
/// transmittance with the sea's Fresnel term.
const MAPPING: wgpu::TextureFormat = wgpu::TextureFormat::Rg32Float;
const AIR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;

/// Fill the per-pixel tables, once: `fs_geometry` over the whole frame, with
/// the uniforms of any moment, since none of what it reads ever changes.
fn draw_geometry(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    uniforms: &wgpu::Buffer,
    mapping: &wgpu::TextureView,
    air: &wgpu::TextureView,
) {
    let device = &gpu.device;
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Globe geometry bind group"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Globe geometry bind group"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: uniforms.as_entire_binding(),
        }],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("Globe geometry pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let target = |format| {
        Some(wgpu::ColorTargetState {
            format,
            blend: None,
            write_mask: wgpu::ColorWrites::ALL,
        })
    };
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("Globe geometry pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_main"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some("fs_geometry"),
            targets: &[target(MAPPING), target(AIR_FORMAT)],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    gpu.queue
        .write_buffer(uniforms, 0, bytemuck::bytes_of(&uniforms_at(0.0)));
    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("Globe geometry encoder"),
    });
    {
        let attachment = |view| {
            Some(wgpu::RenderPassColorAttachment {
                view,
                depth_slice: None,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store: wgpu::StoreOp::Store,
                },
            })
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Globe geometry pass"),
            color_attachments: &[attachment(mapping), attachment(air)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(0, &group, &[]);
        pass.draw(0..3, 0..1);
    }
    gpu.submit(encoder.finish());
    // The frames write the same uniforms: let this pass have its own first.
    if gpu.wait(std::time::Duration::from_secs(30)).is_none() {
        tracing::warn!("the globe's geometry took too long to draw");
    }
}

/// The shader's parameters for `seconds` of run time.
fn uniforms_at(seconds: f64) -> Uniforms {
    let half = SIZE as f32 / 2.0;
    let turns = (seconds / REVOLUTION as f64) as f32;
    let clouds = (seconds / REVOLUTION as f64 * (1.0 + CLOUD_DRIFT as f64)) as f32;
    let sky = sun_at(seconds);
    let to_planet = tilt();
    let sun = rotate(&to_planet, sky);
    let view = rotate(&to_planet, [0.0, 0.0, 1.0]);
    Uniforms {
        frame: [half, half, half * PLANET, half * PLANET * ATMOSPHERE],
        spin: [turns.rem_euclid(1.0), clouds.rem_euclid(1.0), 0.0, 0.0],
        sun: [sun[0], sun[1], sun[2], 0.0],
        gloss: half_vector(sun, view),
        sky: [sky[0], sky[1], sky[2], 0.0],
        to_planet: to_planet.map(|row| [row[0], row[1], row[2], 0.0]),
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

fn half_vector(sun: [f32; 3], view: [f32; 3]) -> [f32; 4] {
    let h = [sun[0] + view[0], sun[1] + view[1], sun[2] + view[2]];
    let length = (h[0] * h[0] + h[1] * h[1] + h[2] * h[2]).sqrt();
    [h[0] / length, h[1] / length, h[2] / length, 0.0]
}

/// The rotation from screen space into the planet's frame, whose y axis is the
/// axis of rotation: the transpose of tilting the axis by [`OBLIQUITY`] toward
/// the right and [`NOD`] toward the viewer.
fn tilt() -> [[f32; 3]; 3] {
    let (st, ct) = OBLIQUITY.to_radians().sin_cos();
    let (sk, ck) = NOD.to_radians().sin_cos();
    // Planet to screen: nod about x, then lean about z.
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

/// Upload a map at its native size and let the GPU build the mip chain.
#[allow(clippy::too_many_arguments)]
fn upload(
    gpu: &Graphics,
    mips: &MipChain,
    label: &str,
    width: u32,
    height: u32,
    format: wgpu::TextureFormat,
    data: &[u8],
    bytes_per_texel: u32,
) -> Result<wgpu::TextureView> {
    let device = &gpu.device;
    let queue = &gpu.queue;
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
    mips.build(gpu, &texture, levels, format);
    Ok(texture.create_view(&wgpu::TextureViewDescriptor::default()))
}

/// Builds mip chains on the GPU: every level is the previous one sampled with a
/// linear filter. Doing this here rather than on the CPU is what removes the
/// shimmer from the city lights when the map is minified.
struct MipChain {
    layout: wgpu::BindGroupLayout,
    pipelines: [(wgpu::TextureFormat, wgpu::RenderPipeline); 3],
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
                    wgpu::TextureFormat::Rgba8UnormSrgb,
                    make(wgpu::TextureFormat::Rgba8UnormSrgb),
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
        gpu.submit(encoder.finish());
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
    sky: vec4<f32>,
    to_planet: array<vec4<f32>, 3>,
}};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var day_map: texture_2d<f32>;
@group(0) @binding(2) var night_map: texture_2d<f32>;
@group(0) @binding(3) var cloud_map: texture_2d<f32>;
@group(0) @binding(4) var relief_map: texture_2d<f32>;
@group(0) @binding(5) var map_sampler: sampler;
// Per pixel, from `fs_geometry`: map coordinates before the planet turns, and
// the air's transmittance toward the viewer with the sea's Fresnel term.
@group(0) @binding(6) var mapping_map: texture_2d<f32>;
@group(0) @binding(7) var air_map: texture_2d<f32>;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const RELIEF: f32 = {relief};
const NIGHT_GAIN: f32 = {night_gain};
const BIAS: f32 = {bias};
const FRAME_EDGE: f32 = {frame_edge};
const CLOUD_ALBEDO: f32 = {cloud_albedo};
const CLOUD_HEIGHT: f32 = {cloud_height};
const SCALE_HEIGHT: f32 = {scale_height};
const AIR: vec3<f32> = vec3<f32>({air_r}, {air_g}, {air_b});
const SUNLIGHT: vec3<f32> = vec3<f32>({sun_r}, {sun_g}, {sun_b});
// `Theme.bg` (#07070a), the window colour behind the planet.
const BACKGROUND: vec3<f32> = vec3<f32>(0.0275, 0.0275, 0.0392);
// Light from the sky itself, which keeps shadowed ground and the twilight
// side from going black.
const SKYLIGHT: vec3<f32> = vec3<f32>(0.035, 0.060, 0.110);

fn smoothstep01(edge0: f32, edge1: f32, x: f32) -> f32 {{
    let t = clamp((x - edge0) / (edge1 - edge0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}}

// Air a ray crosses, in vertical thicknesses, leaving at `mu` (the cosine of
// its angle from straight up). Close to 1 / mu, but finite at the horizon.
fn airmass(mu: f32) -> f32 {{
    let m = max(mu, -0.02);
    return 1.0 / (m + 0.025 + 0.045 * exp(-m * 30.0));
}}

// Sunlight left after crossing the air down to a point where the sun stands at
// `mu`: reddened, then cut off as the sun sets.
fn sunlight(mu: f32) -> vec3<f32> {{
    return SUNLIGHT * exp(-AIR * airmass(mu)) * smoothstep01(-0.035, 0.02, mu);
}}

// One frame of the planet in linear light, premultiplied, encoded to sRGB.
fn encode(linear: vec3<f32>, alpha: f32) -> vec4<f32> {{
    // A soft shoulder instead of a hard clip, for the sun glint. Only the
    // glint ever reaches it; everywhere else this is the plain curve, so the
    // three exponentials are skipped.
    if (max(max(linear.r, linear.g), linear.b) <= 0.8) {{
        return vec4<f32>(pow(linear, vec3<f32>(1.0 / 2.2)), alpha);
    }}
    let over = max(linear - vec3<f32>(0.8), vec3<f32>(0.0));
    let shaped = min(linear, vec3<f32>(0.8)) + 0.2 * (vec3<f32>(1.0) - exp(-over * 5.0));
    return vec4<f32>(pow(shaped, vec3<f32>(1.0 / 2.2)), alpha);
}}

struct Varying {{
    @builtin(position) position: vec4<f32>,
}};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {{
    // A full-screen triangle; outside the planet and its halo the fragment
    // shader paints only the space behind it.
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}}

// The air seen edge-on past the limb, `r` planet radii from the centre: a
// glow where the sun lights it, thinning out exponentially with height. On the
// night side there is only the faint green line of the airglow.
fn limb(p: vec2<f32>, r: f32) -> vec4<f32> {{
    let h = max(r - 1.0, 0.0);
    // A grazing ray crosses about sqrt(2 pi r / H) vertical thicknesses.
    let depth = AIR * sqrt(TAU * r / SCALE_HEIGHT) * exp(-h / SCALE_HEIGHT);
    let tangent = vec3<f32>(p / max(r, 0.0001), 0.0);
    let mu = dot(tangent, params.sky.xyz);
    // High air stays lit for a while after the ground below it has gone dark.
    let lit = SUNLIGHT * exp(-AIR * airmass(mu + 0.08) * 0.6) * smoothstep01(-0.30, 0.05, mu);
    var glow = lit * (vec3<f32>(1.0) - exp(-depth)) * 0.9;
    let night = 1.0 - smoothstep01(-0.25, 0.05, mu);
    let band = (h - SCALE_HEIGHT * 1.4) / (SCALE_HEIGHT * 0.35);
    glow += vec3<f32>(0.10, 0.32, 0.12) * exp(-band * band) * night * 0.10;
    // Faded out before the square frame can cut it off. After encoding, which
    // would otherwise lift the faintest glow back into sight.
    let guard = 1.0 - smoothstep01(FRAME_EDGE * 0.92, FRAME_EDGE, r);
    let out = pow(glow, vec3<f32>(1.0 / 2.2)) * guard;
    return vec4<f32>(out, min(max(max(out.r, out.g), out.b), 1.0));
}}

// What lies behind the planet: the window colour and the faint air glow
// `ui/screens/idle.slint` drew as a Slint gradient, `circle` (half the
// frame's diagonal) from #4b86d8 at 0x14 through #244a7d at 0x10 by 52% to
// nothing by 76%, inside the round panel. Painted here, the frame is opaque
// and Slint has one layer to draw instead of three.
fn space(position: vec2<f32>) -> vec3<f32> {{
    let centre = params.frame.xy;
    let d = length(position - centre);
    let t = d / length(centre);
    let inner = vec4<f32>(vec3<f32>(0.2941, 0.5255, 0.8471) * 0.0784, 0.0784);
    let middle = vec4<f32>(vec3<f32>(0.1412, 0.2902, 0.4902) * 0.0627, 0.0627);
    var air = select(
        mix(middle, vec4<f32>(0.0), clamp((t - 0.52) / 0.24, 0.0, 1.0)),
        mix(inner, middle, t / 0.52),
        t < 0.52,
    );
    air *= step(d, centre.x);
    return air.rgb + BACKGROUND * (1.0 - air.a);
}}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {{
    // Laid over space the way Slint composited the frame when it was
    // transparent: Slint takes a wgpu texture for straight alpha.
    // Most of the frame is the opaque planet, which hides space entirely.
    let globe = shade(in);
    if (globe.a >= 1.0) {{
        return vec4<f32>(globe.rgb, 1.0);
    }}
    return vec4<f32>(globe.rgb * globe.a + space(in.position.xy) * (1.0 - globe.a), 1.0);
}}

struct Geometry {{
    @location(0) mapping: vec4<f32>,
    @location(1) air: vec4<f32>,
}};

// What never changes for a pixel, worked out once into the tables `shade`
// reads: where it lies on the maps before the planet turns, and what the air
// between it and the viewer lets through.
@fragment
fn fs_geometry(in: Varying) -> Geometry {{
    let p = vec2<f32>(
        (in.position.x - params.frame.x) / params.frame.z,
        (params.frame.y - in.position.y) / params.frame.z,
    );
    let distance = min(length(p), 1.0);
    let z = sqrt(max(1.0 - distance * distance, 0.0));
    let n_screen = vec3<f32>(p, z);
    let n = vec3<f32>(
        dot(params.to_planet[0].xyz, n_screen),
        dot(params.to_planet[1].xyz, n_screen),
        dot(params.to_planet[2].xyz, n_screen),
    );
    let lat = asin(clamp(n.y, -1.0, 1.0));
    let lon0 = atan2(n.x, n.z);
    var out: Geometry;
    out.mapping = vec4<f32>(0.5 + lon0 / TAU, 0.5 - lat / PI, 0.0, 0.0);
    let fresnel = 0.02 + 0.98 * pow(1.0 - z, 5.0);
    out.air = vec4<f32>(exp(-AIR * airmass(z)), fresnel);
    return out;
}}

fn shade(in: Varying) -> vec4<f32> {{
    // Screen position in planet radii, y up.
    let p = vec2<f32>(
        (in.position.x - params.frame.x) / params.frame.z,
        (params.frame.y - in.position.y) / params.frame.z,
    );
    let distance = length(p);
    let atmosphere = params.frame.w / params.frame.z;
    if (distance > atmosphere) {{
        return vec4<f32>(0.0);
    }}
    // Anti-aliasing at the limb: a pixel either side of the edge.
    let edge = (distance - 1.0) * params.frame.z;
    let coverage = smoothstep01(-0.75, 0.75, -edge);
    if (coverage <= 0.0) {{
        return limb(p, distance);
    }}

    // The point on the sphere, on screen and then in the planet's own frame.
    let z = sqrt(max(1.0 - distance * distance, 0.0));
    let n_screen = vec3<f32>(p, z);
    let n = vec3<f32>(
        dot(params.to_planet[0].xyz, n_screen),
        dot(params.to_planet[1].xyz, n_screen),
        dot(params.to_planet[2].xyz, n_screen),
    );
    let texel = vec2<i32>(in.position.xy);
    let mapped = textureLoad(mapping_map, texel, 0).xy;
    let uv = vec2<f32>(mapped.x + params.spin.x, mapped.y);
    let air = textureLoad(air_map, texel, 0);

    // Tangent frame, used to tilt the normal with the relief map and to cast
    // the clouds' shadows; the latitude's cosine and the longitude's sine and
    // cosine follow from the normal itself.
    let cos_lat = sqrt(max(n.x * n.x + n.z * n.z, 1e-8));
    let sin_lon = n.x / cos_lat;
    let cos_lon = n.z / cos_lat;
    let east = vec3<f32>(cos_lon, 0.0, -sin_lon);
    let north = vec3<f32>(-n.y * sin_lon, cos_lat, -n.y * cos_lon);
    let relief = textureSampleBias(relief_map, map_sampler, uv, BIAS + 0.4);
    let bump = relief.rg - vec2<f32>(0.5);
    let water = relief.b;
    let tilted = normalize(n + (east * bump.x + north * bump.y) * RELIEF);

    let sun = params.sun.xyz;
    let mu_sun = dot(n, sun);
    let mu_view = z;

    // Clouds turn a little ahead of the ground; their shadows fall away from
    // the sun, farther the lower it stands.
    let cloud_uv = vec2<f32>(uv.x + params.spin.y - params.spin.x, uv.y);
    let cloud = textureSampleBias(cloud_map, map_sampler, cloud_uv, BIAS).r;
    let reach = CLOUD_HEIGHT / max(mu_sun, 0.12);
    let shift = vec2<f32>(
        dot(sun, east) * reach / (TAU * max(cos_lat, 0.05)),
        -dot(sun, north) * reach / PI,
    );
    var shadow = 0.0;
    if (mu_sun > -0.05) {{
        shadow = textureSampleBias(cloud_map, map_sampler, cloud_uv + shift, BIAS + 1.0).r;
    }}

    // The ground, lit by reddened sunlight and the sky.
    let albedo = textureSampleBias(day_map, map_sampler, uv, BIAS).rgb * mix(1.0, 0.8, water);
    let sun_ground = sunlight(mu_sun);
    let direct = sun_ground * max(dot(tilted, sun) + 0.03, 0.0) * (1.0 - 0.6 * shadow);
    let sky = SKYLIGHT * smoothstep01(-0.15, 0.4, mu_sun);
    var colour = albedo * (direct + sky);

    // A calm sea mirrors the sun: a tight glint and a broad sheen, both
    // stronger at a grazing angle.
    let fresnel = air.a;
    let open_sea = water * (1.0 - cloud);
    // Land and overcast sea have no glint, so they skip it; over open water
    // both lobes share one logarithm.
    if (open_sea > 0.0) {{
        let log_nh = log2(max(dot(n, params.gloss.xyz), 0.0));
        // From orbit the glint is a soft patch, spread by the waves, not a mirror's disc.
        let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
        colour += sun_ground * glint * (0.25 + fresnel) * open_sea;
    }}
    colour += SKYLIGHT * 2.0 * fresnel * water * smoothstep01(-0.1, 0.3, mu_sun);

    // The cloud tops and the air above the ground stand above most of the
    // atmosphere, so they keep the light and its sunset colour a little longer
    // than the ground does.
    let sun_high = SUNLIGHT * exp(-AIR * airmass(mu_sun + 0.04) * 0.55);
    let sun_cloud = sun_high * smoothstep01(-0.08, 0.04, mu_sun);
    let cloud_lit = CLOUD_ALBEDO * (sun_cloud * (0.30 + 0.70 * sqrt(max(mu_sun, 0.0))) + sky * 1.5);
    colour = mix(colour, cloud_lit, cloud);

    // The night side: city lights, warm where faint and whiter where dense,
    // dimmed by cloud, and a faint bluish earthshine.
    let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu_sun);
    if (darkness > 0.002) {{
        let city = textureSampleBias(night_map, map_sampler, uv, BIAS + 0.6).r;
        let glow = city * city * NIGHT_GAIN;
        let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, city));
        colour += tint * glow * darkness * (1.0 - cloud * 0.85);
        colour += vec3<f32>(0.004, 0.006, 0.012) * (albedo + cloud) * darkness;
    }}

    // The air between the ground and the viewer: it dims what is behind it and
    // adds its own blue, thickening toward the limb.
    let through = air.rgb;
    let sun_air = sun_high * smoothstep01(-0.10, 0.08, mu_sun);
    colour = colour * through + sun_air * (vec3<f32>(1.0) - through) * 0.45;

    let planet = encode(colour, 1.0);
    if (coverage >= 1.0) {{
        return planet;
    }}
    return mix(limb(p, distance), planet, coverage);
}}
"#,
        relief = RELIEF,
        night_gain = NIGHT_GAIN,
        bias = DETAIL_BIAS,
        frame_edge = 1.0 / PLANET,
        cloud_albedo = CLOUD_ALBEDO,
        cloud_height = CLOUD_HEIGHT,
        scale_height = SCALE_HEIGHT,
        air_r = AIR[0],
        air_g = AIR[1],
        air_b = AIR[2],
        sun_r = SUNLIGHT,
        sun_g = SUNLIGHT * 0.98,
        sun_b = SUNLIGHT * 0.95,
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
    fn render(&mut self, gpu: &Graphics, seconds: f64, _seed: u64, frame: usize) -> Result<()> {
        Globe::render(self, gpu, seconds, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        Globe::texture(self, frame)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guards the halo geometry: the planet is round and only has a faint ring
    /// just outside it, over plain space. A units mix-up once painted the whole
    /// square instead, which showed as a visible box.
    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_frame_is_round() {
        let pixels = render_once(0.0).expect("a GPU adapter and an Earth frame");
        let half = SIZE / 2;
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };
        let brightness = |x: u32, y: u32| at(x, y)[..3].iter().map(|&c| c as u32).sum::<u32>();

        // Opaque everywhere, so Slint draws nothing under it.
        assert!(pixels.chunks(4).all(|pixel| pixel[3] == 255));
        // Outside the round panel there is only the window colour.
        assert_eq!(at(0, 0), [7, 7, 10, 255], "the corner should be plain space");
        assert_eq!(at(SIZE - 1, SIZE - 1), at(0, 0));

        // The sun starts on the left, so the halo is only on that side.
        let limb = (half as f32 * PLANET) as u32;
        let space = brightness(0, 0);
        let halo = brightness(half - limb - 3, half);
        assert!(halo > space + 6, "the halo should show, got {halo} over {space}");
        // Whatever the planet size, the glow has faded out by the frame edge: it
        // must never be cut off by the square.
        assert!(
            brightness(2, half) <= space + 9,
            "the glow should fade out before the frame edge, got {} over {space}",
            brightness(2, half)
        );
    }

    /// Time the globe on whatever adapter is present, from the moment the maps
    /// start decoding to a run of frames. Only useful relative to another run on
    /// the same machine.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let started = std::time::Instant::now();
        let mut globe = Globe::new(&gpu).expect("an Earth renderer");
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        println!(
            "globe setup: {:.0} ms",
            started.elapsed().as_secs_f64() * 1e3
        );
        Visual::render(&mut globe, &gpu, 0.0, 0, 0).expect("a frame");
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let frames = 240;
        let mut worst = 0.0f64;
        let started = std::time::Instant::now();
        for i in 0..frames {
            let begun = std::time::Instant::now();
            // Spread over a whole day, so the night side is timed too.
            let seconds = i as f64 * SUN_PERIOD as f64 / frames as f64;
            Visual::render(&mut globe, &gpu, seconds, 0, i % SLOTS).expect("a frame");
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            worst = worst.max(begun.elapsed().as_secs_f64());
        }
        let mean = started.elapsed().as_secs_f64() / frames as f64;
        println!(
            "globe frame: mean {:.3} ms, worst {:.3} ms",
            mean * 1e3,
            worst * 1e3
        );
        // Back to back with one wait at the end: what the GPU itself sustains,
        // without the per-frame round trip.
        let started = std::time::Instant::now();
        for i in 0..frames {
            let seconds = i as f64 * SUN_PERIOD as f64 / frames as f64;
            Visual::render(&mut globe, &gpu, seconds, 0, i % SLOTS).expect("a frame");
        }
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let sustained = started.elapsed().as_secs_f64() / frames as f64;
        println!("globe frame, back to back: {:.3} ms", sustained * 1e3);
    }

    /// Write frames through a whole day to `GLOBE_PREVIEW_DIR`, for design work.
    #[test]
    #[ignore = "writes design previews; set GLOBE_PREVIEW_DIR"]
    fn previews() {
        let Ok(dir) = std::env::var("GLOBE_PREVIEW_DIR") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let globe = Globe::new(&gpu).expect("an Earth renderer");
        for (k, share) in [0.0, 0.15, 0.3, 0.5, 0.75, 0.88].iter().enumerate() {
            let pixels = globe.read_back(&gpu, *share * SUN_PERIOD as f64);
            let path = std::path::Path::new(&dir).join(format!("globe-{k}.png"));
            image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                .expect("a preview image");
        }
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
