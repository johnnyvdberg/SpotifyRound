//! The Nautilus' salon window, around the 3D dive's sea.
//!
//! The brass bezel, the riveted casing and the gauge faces never change, so
//! they are drawn once when the window is built. Each frame a full-resolution
//! composite bends the sea at the edge of the glass, gives it an engraved
//! finish, lays the bezel over it and turns the gauge needles: the manometer
//! reads the boat's depth and the log turns once per leg of the voyage.
//!
//! The shaders are in `porthole.wgsl`, a small file of their own.

use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::SLOTS;
use crate::voyage::Position;

/// Size of the rendered frame. The porthole fills the round panel edge to edge.
pub const SIZE: u32 = 1080;

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

/// The window: the bezel and gauges, drawn once, and the composite that lays
/// them over the sea.
pub(crate) struct Porthole {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    group: wgpu::BindGroup,
    frames: Vec<Frame>,
    sea: u32,
}

impl Porthole {
    /// `sea` is the texture the sea is drawn into, `size` pixels square and
    /// covering the glass with a little to spare; `bend` is how far the glass
    /// bends the view at its edge.
    /// `scale` is what the sea's colours were multiplied by when stored.
    pub(crate) fn new(
        gpu: &Graphics,
        sea: &wgpu::TextureView,
        size: u32,
        bend: f32,
        scale: f32,
    ) -> Self {
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
            &[
                ("GLASS_BEND", f64::from(bend)),
                ("SEA_SCALE", f64::from(scale)),
            ],
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

/// The bezel and the composite, one entry point each.
const SHADER: &str = include_str!("porthole.wgsl");
