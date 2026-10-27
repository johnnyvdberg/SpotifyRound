//! A voyage seen through the Nautilus' salon window.
//!
//! The whole scene is generated in `dive.wgsl`. The Nautilus travels through
//! seven stretches of sea in turn: sunlit kelp shallows, the submarine forest of
//! Crespo with its helmeted divers, the galleons of Vigo Bay, the ruins of
//! Atlantis under their volcano, a lava cave, the abyss where the giant squid
//! lives and the underside of the polar ice. The light and the life change with
//! each leg; [`crate::voyage`] steers the boat, rising and sinking over each
//! seabed, and a swipe carries it on to the next leg or back to the last. Gauges
//! in the brass bezel follow the dive.
//!
//! A frame is drawn in two passes to fit a 60 fps budget on a Raspberry Pi 5. The
//! sea goes into a smaller texture, since nothing under water is sharp; a cheap
//! full-resolution pass scales it up and lays the bezel over it. The bezel
//! itself never changes, so it is drawn once when the scene is built. Nothing
//! but a handful of uniforms is uploaded per frame.
//!
//! The renderer exposes the same interface as [`crate::globe`], so the idle
//! screen can pick either scene without knowing anything about how it is drawn.

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::{SLOTS, Visual};
use crate::voyage::{BIOMES, Biome, Position, Voyage};

/// Size of the rendered frame. The porthole fills the round panel edge to edge.
pub const SIZE: u32 = 1080;

/// Edge of the square the sea is drawn into. It covers just the glass (about
/// 870 px across at full size), so this is roughly two thirds of the panel's
/// resolution and under a quarter of its pixels.
const SEA: u32 = 576;

/// Per-frame parameters, laid out as 16-byte aligned vectors.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// x, y: frame size in pixels; z: seconds of run time; w: world seed in 0..1.
    frame: [f32; 4],
    /// x: edge of the sea texture in pixels; y: world units travelled; z: height
    /// above the seabed; w: depth below the surface.
    scene: [f32; 4],
}

/// One frame of the window: a texture Slint samples directly.
struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Everything needed to draw the voyage.
pub(crate) struct Bell {
    /// Indexed by the biome of the leg the Nautilus is on.
    sea_pipelines: Vec<wgpu::RenderPipeline>,
    composite_pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    sea_group: wgpu::BindGroup,
    composite_group: wgpu::BindGroup,
    sea_view: wgpu::TextureView,
    frames: Vec<Frame>,
    voyage: Voyage,
    /// The clock of the frame drawn last, so a swipe starts from where the
    /// Nautilus is now.
    seconds: f64,
}

impl Bell {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        let device = &gpu.device;

        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        // The sea and the bezel only read the uniforms; the composite also reads
        // both textures.
        let sea_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dive sea bind group"),
            entries: &[uniform_entry],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dive composite bind group"),
            entries: &[
                uniform_entry,
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let sea_texture = render_target(device, "Dive sea", SEA, wgpu::TextureFormat::Rgba16Float);
        let sea_view = sea_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let bezel = render_target(device, "Dive bezel", SIZE, wgpu::TextureFormat::Rgba8Unorm);
        let bezel_view = bezel.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dive sea sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });

        let sea_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dive sea bind group"),
            layout: &sea_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let composite_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dive composite bind group"),
            layout: &composite_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&sea_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&bezel_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dive shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline = |label,
                        layout: &wgpu::BindGroupLayout,
                        entry_point,
                        format,
                        constants: &[(&str, f64)]| {
            let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(label),
                bind_group_layouts: &[Some(layout)],
                immediate_size: 0,
            });
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
                    entry_point: Some(entry_point),
                    targets: &[Some(wgpu::ColorTargetState {
                        format,
                        // Every pass covers its whole target.
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let bezel_pipeline = pipeline(
            "Dive bezel pipeline",
            &sea_layout,
            "fs_overlay",
            wgpu::TextureFormat::Rgba8Unorm,
            &[],
        );
        // One sea pipeline per biome, able to draw only it and its neighbours on
        // the voyage: nothing else can be in view, and a shader carrying all seven
        // biomes' code is markedly slower even where most of it never runs.
        let sea_pipelines = (0..BIOMES)
            .map(|biome| {
                let shown = [biome + BIOMES - 1, biome, biome + 1]
                    .iter()
                    .fold(0u32, |mask, id| mask | 1 << (id % BIOMES));
                // The driver compiles each variant here: slow on the Pi, so say
                // how slow.
                let begun = std::time::Instant::now();
                let made = pipeline(
                    "Dive sea pipeline",
                    &sea_layout,
                    "fs_scene",
                    wgpu::TextureFormat::Rgba16Float,
                    &[("BIOMES_SHOWN", f64::from(shown))],
                );
                tracing::info!(
                    biome,
                    seconds = begun.elapsed().as_secs_f32(),
                    "dive sea pipeline compiled"
                );
                made
            })
            .collect();
        let composite_pipeline = pipeline(
            "Dive composite pipeline",
            &composite_layout,
            "fs_composite",
            wgpu::TextureFormat::Rgba8Unorm,
            &[],
        );

        let frames = (0..SLOTS)
            .map(|_| {
                let texture =
                    render_target(device, "Dive frame", SIZE, wgpu::TextureFormat::Rgba8Unorm);
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                Frame { texture, view }
            })
            .collect();

        // The bezel is drawn once; every frame after this only reads it.
        let start = Position {
            x: 0.0,
            altitude: 0.3,
            depth: 1.0,
        };
        gpu.queue.write_buffer(
            &uniforms,
            0,
            bytemuck::bytes_of(&uniforms_at(0.0, 0.0, start)),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Dive bezel encoder"),
        });
        full_screen(
            &mut encoder,
            "Dive bezel pass",
            &bezel_view,
            &bezel_pipeline,
            &sea_group,
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));

        tracing::info!("nautilus scene ready");
        Ok(Self {
            sea_pipelines,
            composite_pipeline,
            uniforms,
            sea_group,
            composite_group,
            sea_view,
            frames,
            voyage: Voyage::default(),
            seconds: 0.0,
        })
    }

    fn draw(&mut self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        let seed = (seed % 1_000_003) as f32 / 1_000_003.0;
        let at = self.voyage.advance(seconds, seed);
        self.seconds = seconds;
        gpu.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&uniforms_at(seconds, seed, at)),
        );

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Dive encoder"),
            });
        full_screen(
            &mut encoder,
            "Dive sea pass",
            &self.sea_view,
            &self.sea_pipelines[Biome::at(at.x, seed).a],
            &self.sea_group,
        );
        full_screen(
            &mut encoder,
            "Dive composite pass",
            &self.frames[frame].view,
            &self.composite_pipeline,
            &self.composite_group,
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    pub(crate) fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    /// Read one frame back as RGBA pixels, for the design previews and tests.
    #[cfg(test)]
    fn read_back(&mut self, gpu: &Graphics, seconds: f64, seed: u64) -> Vec<u8> {
        self.draw(gpu, seconds, seed, 0).expect("a dive frame");
        let padded_row = (SIZE * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive preview readback"),
            size: (padded_row as u64) * (SIZE as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Dive preview readback"),
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

impl Visual for Bell {
    fn render(&mut self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        self.draw(gpu, seconds, seed, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        Bell::texture(self, frame)
    }

    fn travel(&mut self, legs: i32) {
        self.voyage.travel(legs, self.seconds);
    }
}

fn uniforms_at(seconds: f64, seed: f32, at: Position) -> Uniforms {
    uniforms_for(seconds, seed, at, SEA)
}

fn uniforms_for(seconds: f64, seed: f32, at: Position, sea: u32) -> Uniforms {
    // The shader's clock only drives animation, so it can wrap; where the
    // Nautilus is comes from the voyage.
    let seconds = seconds.rem_euclid(6_000.0);
    Uniforms {
        frame: [SIZE as f32, SIZE as f32, seconds as f32, seed],
        scene: [sea as f32, at.x as f32, at.altitude as f32, at.depth as f32],
    }
}

fn render_target(
    device: &wgpu::Device,
    label: &str,
    size: u32,
    format: wgpu::TextureFormat,
) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        // Slint or the composite samples it; COPY_SRC is only for the design previews.
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    })
}

/// Draw one full-screen triangle into `target`.
fn full_screen(
    encoder: &mut wgpu::CommandEncoder,
    label: &str,
    target: &wgpu::TextureView,
    pipeline: &wgpu::RenderPipeline,
    group: &wgpu::BindGroup,
) {
    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
        label: Some(label),
        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
            view: target,
            depth_slice: None,
            resolve_target: None,
            ops: wgpu::Operations {
                // Every pixel is written, so there is nothing to load or clear.
                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
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
    pass.draw(0..3, 0..1);
}

/// The Nautilus' window for a sea drawn by someone else: the brass bezel and
/// gauges, drawn once, and the composite that lays them over the sea, gives it
/// the engraved finish and bends the view at the edge of the glass. The 3D
/// dive uses this; the procedural dive above has the same passes built in.
pub(crate) struct Porthole {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    frames: Vec<Frame>,
    sea: u32,
}

impl Porthole {
    /// `sea` is the texture the sea is drawn into, `size` pixels square and
    /// covering the same view as the procedural dive's; `bend` is how far the
    /// glass bends the view at its edge.
    pub(crate) fn new(gpu: &Graphics, sea: &wgpu::TextureView, size: u32, bend: f32) -> Self {
        let device = &gpu.device;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Porthole uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let uniform_entry = wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        };
        let texture_entry = |binding| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let bezel_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Porthole bezel bind group"),
            entries: &[uniform_entry],
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Porthole bind group"),
            entries: &[
                uniform_entry,
                texture_entry(1),
                texture_entry(2),
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bezel = render_target(
            device,
            "Porthole bezel",
            SIZE,
            wgpu::TextureFormat::Rgba8Unorm,
        );
        let bezel_view = bezel.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Porthole sea sampler"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let bezel_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Porthole bezel bind group"),
            layout: &bezel_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Porthole bind group"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniforms.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(sea),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&bezel_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Porthole shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline =
            |label, layout: &wgpu::BindGroupLayout, entry_point, constants: &[(&str, f64)]| {
                let pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some(label),
                        bind_group_layouts: &[Some(layout)],
                        immediate_size: 0,
                    });
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
                        entry_point: Some(entry_point),
                        targets: &[Some(wgpu::ColorTargetState {
                            format: wgpu::TextureFormat::Rgba8Unorm,
                            blend: None,
                            write_mask: wgpu::ColorWrites::ALL,
                        })],
                        compilation_options: wgpu::PipelineCompilationOptions {
                            constants,
                            ..Default::default()
                        },
                    }),
                    primitive: wgpu::PrimitiveState::default(),
                    depth_stencil: None,
                    multisample: wgpu::MultisampleState::default(),
                    multiview_mask: None,
                    cache: None,
                })
            };
        let bezel_pipeline = pipeline("Porthole bezel pipeline", &bezel_layout, "fs_overlay", &[]);
        let composite = pipeline(
            "Porthole composite pipeline",
            &layout,
            "fs_composite",
            &[("GLASS_BEND", f64::from(bend))],
        );
        let start = Position {
            x: 0.0,
            altitude: 0.3,
            depth: 1.0,
        };
        gpu.queue.write_buffer(
            &uniforms,
            0,
            bytemuck::bytes_of(&uniforms_for(0.0, 0.0, start, size)),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Porthole bezel encoder"),
        });
        full_screen(
            &mut encoder,
            "Porthole bezel pass",
            &bezel_view,
            &bezel_pipeline,
            &bezel_group,
        );
        gpu.queue.submit(std::iter::once(encoder.finish()));

        let frames = (0..SLOTS)
            .map(|_| {
                let texture = render_target(
                    device,
                    "Porthole frame",
                    SIZE,
                    wgpu::TextureFormat::Rgba8Unorm,
                );
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                Frame { texture, view }
            })
            .collect();
        Self {
            pipeline: composite,
            uniforms,
            group,
            frames,
            sea: size,
        }
    }

    /// Lay the window over the sea into `frame`, with the gauges at `at`.
    pub(crate) fn draw(
        &self,
        gpu: &Graphics,
        encoder: &mut wgpu::CommandEncoder,
        seconds: f64,
        seed: f32,
        at: Position,
        frame: usize,
    ) {
        gpu.queue.write_buffer(
            &self.uniforms,
            0,
            bytemuck::bytes_of(&uniforms_for(seconds, seed, at, self.sea)),
        );
        full_screen(
            encoder,
            "Porthole composite pass",
            &self.frames[frame].view,
            &self.pipeline,
            &self.group,
        );
    }

    pub(crate) fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }
}

/// Read a frame texture back as RGBA pixels, for the design previews and tests.
#[cfg(test)]
pub(crate) fn read_texture(gpu: &Graphics, texture: &wgpu::Texture) -> Vec<u8> {
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
            texture,
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

/// Render a single frame synchronously, for the design previews.
#[cfg(test)]
pub fn render_once(seconds: f64, seed: u64) -> Result<Vec<u8>> {
    let gpu = crate::gpu::headless()?;
    let mut bell = Bell::new(&gpu)?;
    Ok(bell.read_back(&gpu, seconds, seed))
}

/// The scene shader: the bezel, the sea and the composite, one entry point each.
///
/// Colours are deliberately kept in a narrow, cold palette so the brass, the
/// lamps, the volcano and the bioluminescence carry the eye.
const SHADER: &str = include_str!("dive.wgsl");

#[cfg(test)]
mod tests {
    use super::*;

    /// Seconds into a visit at which leg `n` of the voyage is well under way.
    fn leg(n: f64) -> f64 {
        crate::voyage::LEG / crate::voyage::SPEED * (n + 0.35)
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_world_keeps_moving() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut bell = Bell::new(&gpu).expect("a dive renderer");
        let first = bell.read_back(&gpu, 8.0, 4_242);
        let second = bell.read_back(&gpu, 18.0, 4_242);
        assert_ne!(
            first, second,
            "the scene should change as the Nautilus travels"
        );
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_porthole_fills_the_frame() {
        let pixels = render_once(12.0, 7_321).expect("a GPU adapter and a dive frame");
        let index = |x: u32, y: u32| ((y * SIZE + x) * 4) as usize;
        let pixel = |x: u32, y: u32| -> [u8; 4] {
            let i = index(x, y);
            [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
        };

        assert_eq!(pixel(0, 0)[3], 255, "the casing should cover the corner");
        assert_eq!(
            pixel(SIZE / 2, SIZE / 2)[3],
            255,
            "the glass should be opaque"
        );
        assert_ne!(
            pixel(0, 0),
            pixel(SIZE / 2, SIZE / 2),
            "the casing and the underwater view should not be the same colour"
        );
    }

    /// The abyss is far darker than the sunlit shallows: the voyage really dives.
    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_sea_darkens_with_depth() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut bell = Bell::new(&gpu).expect("a dive renderer");
        // Seed 0 joins the circuit at the shallows, so leg 5 is the abyss.
        let mut brightness = |seconds: f64| -> f64 {
            let pixels = bell.read_back(&gpu, seconds, 0);
            let mut sum = 0u64;
            let mut count = 0u64;
            for y in (SIZE / 2 - 200)..(SIZE / 2 + 200) {
                for x in (SIZE / 2 - 200)..(SIZE / 2 + 200) {
                    let i = ((y * SIZE + x) * 4) as usize;
                    sum += pixels[i] as u64 + pixels[i + 1] as u64 + pixels[i + 2] as u64;
                    count += 3;
                }
            }
            sum as f64 / count as f64
        };
        let shallows = brightness(leg(0.0));
        let abyss = brightness(leg(5.0));
        assert!(
            abyss * 2.0 < shallows,
            "the abyss ({abyss:.1}) should be much darker than the shallows ({shallows:.1})"
        );
    }

    /// Write one frame per leg of the voyage to `DIVE_PREVIEW_DIR`, for design work.
    #[test]
    #[ignore = "writes design previews; set DIVE_PREVIEW_DIR"]
    fn previews() {
        let Ok(dir) = std::env::var("DIVE_PREVIEW_DIR") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut bell = Bell::new(&gpu).expect("a dive renderer");
        let names = [
            "shallows", "forest", "wrecks", "atlantis", "cave", "abyss", "arctic",
        ];
        let offsets: Vec<f64> = std::env::var("DIVE_PREVIEW_OFFSETS")
            .ok()
            .map(|list| {
                list.split(',')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0]);
        for (n, name) in names.iter().enumerate() {
            for (k, offset) in offsets.iter().enumerate() {
                let pixels = bell.read_back(&gpu, leg(n as f64) + offset, 0);
                let path = std::path::Path::new(&dir).join(format!("{n}-{name}-{k}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a preview image");
            }
        }
    }

    /// Time the scene on whatever adapter is present, per leg of the voyage.
    /// Only useful relative to another run on the same machine.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut bell = Bell::new(&gpu).expect("a dive renderer");
        // The first frame pays for pipeline compilation.
        bell.draw(&gpu, 0.0, 0, 0).expect("a frame");
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let names = [
            "shallows", "forest", "wrecks", "atlantis", "cave", "abyss", "arctic",
        ];
        let frames = 60;
        let mut total = 0.0;
        for (n, name) in names.iter().enumerate() {
            let mut worst = 0.0f64;
            let started = std::time::Instant::now();
            for i in 0..frames {
                let begun = std::time::Instant::now();
                bell.draw(&gpu, leg(n as f64) + i as f64 * 0.25, 0, i % SLOTS)
                    .expect("a frame");
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                worst = worst.max(begun.elapsed().as_secs_f64());
            }
            let mean = started.elapsed().as_secs_f64() / frames as f64;
            total += mean;
            println!(
                "dive frame, {name}: mean {:.3} ms, worst {:.3} ms",
                mean * 1e3,
                worst * 1e3
            );
        }
        println!(
            "dive frame: mean {:.3} ms",
            total / names.len() as f64 * 1e3
        );
    }
}
