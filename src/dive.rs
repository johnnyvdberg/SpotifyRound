//! A procedural dive seen through a diving bell's porthole.
//!
//! The whole scene is generated in `dive.wgsl`: the moving water and light shafts,
//! the scrolling seabed with kelp, vents and wrecks, two shoals, jellyfish, a right
//! whale, a reference-style submarine and its pod of dolphins. Nothing but a
//! handful of uniforms is uploaded per frame, so a dive costs a fraction of the
//! memory the Earth maps need.
//!
//! The renderer exposes the same interface as [`crate::globe`], so the idle
//! screen can pick either scene without knowing anything about how it is drawn.

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::{SLOTS, Visual};

/// Size of the rendered frame. The porthole fills the round panel edge to edge.
pub const SIZE: u32 = 1080;

/// Per-frame parameters, laid out as 16-byte aligned vectors.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// x, y: frame size in pixels; z: seconds of run time; w: world seed in 0..1.
    frame: [f32; 4],
    /// Spare slots so later scenes can grow the budget without re-laying the
    /// buffer out. x: reserved.
    camera: [f32; 4],
}

/// One frame of the bell: a texture Slint samples directly.
struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// Everything needed to draw the dive.
pub(crate) struct Bell {
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    frames: Vec<Frame>,
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

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dive bind group"),
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dive bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dive shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dive pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dive pipeline"),
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
                    // The shader writes straight alpha and the scene is opaque.
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
                    label: Some("Dive frame"),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // Slint samples it; COPY_SRC is only for the design previews.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                Frame { texture, view }
            })
            .collect();

        tracing::info!("diving bell scene ready");
        Ok(Self {
            pipeline,
            uniforms,
            bind_group,
            frames,
        })
    }

    fn draw(&self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        let seconds = seconds.rem_euclid(6_000.0);
        let seed = (seed % 1_000_003) as f32 / 1_000_003.0;
        let uniforms = Uniforms {
            frame: [SIZE as f32, SIZE as f32, seconds as f32, seed],
            camera: [0.0; 4],
        };
        gpu.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Dive encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Dive pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frames[frame].view,
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
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    pub(crate) fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    /// Read one frame back as RGBA pixels, for the design previews and tests.
    #[cfg(test)]
    fn read_back(&self, gpu: &Graphics, seconds: f64, seed: u64) -> Vec<u8> {
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
    fn render(&self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        self.draw(gpu, seconds, seed, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        Bell::texture(self, frame)
    }
}

/// Render a single frame synchronously, for the design previews.
#[cfg(test)]
pub fn render_once(seconds: f64, seed: u64) -> Result<Vec<u8>> {
    let gpu = crate::gpu::headless()?;
    let bell = Bell::new(&gpu)?;
    Ok(bell.read_back(&gpu, seconds, seed))
}

/// The scene shader.
///
/// It is one full-screen triangle. Everything outside the porthole is the bell's
/// casing; inside it the water is generated with a few noise and shape functions.
/// Colours are deliberately kept in a narrow, cold palette so the warm wreck
/// lights, the vents and the bioluminescence carry the eye.
const SHADER: &str = include_str!("dive.wgsl");

/// The renderer is driven at the panel's refresh rate.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_world_keeps_moving() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let bell = Bell::new(&gpu).expect("a dive renderer");
        let first = bell.read_back(&gpu, 8.0, 4_242);
        let second = bell.read_back(&gpu, 18.0, 4_242);
        assert_ne!(first, second, "the scene should change as the bell travels");
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
}
