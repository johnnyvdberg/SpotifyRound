//! What the two eyepiece scenes, the [observatory](crate::observatory) and the
//! [microscope](crate::microscope), share: each is one fragment shader over a
//! full-screen triangle, fed by one uniform block the CPU fills every frame,
//! drawn into a small ring of textures that Slint samples directly.
//!
//! Neither scene has maps to upload or passes to chain: everything is worked
//! out per pixel from the uniforms, so a frame is one buffer write and one draw.
//! The field of view is round by nature, an eyepiece's field stop, so the
//! round panel *is* the instrument and nothing is cut off at its rim. Both
//! shaders start with `shaders/lens/prelude.wgsl`: the full-screen triangle,
//! hashes and value noise, the field stop and the encode.

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;

/// Size of the rendered frame: the panel's, drawn 1:1.
pub const SIZE: u32 = 1080;
/// Render targets in the ring, as for every scene.
const SLOTS: usize = crate::visuals::SLOTS;

struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

/// One full-screen shader and the frames it draws into.
pub struct Lens {
    label: &'static str,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    frames: Vec<Frame>,
}

impl Lens {
    /// Compile `shader` (with `fs_main` as its fragment entry point) and make
    /// a uniform buffer of `uniform_size` bytes for it at binding 0.
    pub fn new(gpu: &Graphics, label: &'static str, shader: &str, uniform_size: u64) -> Self {
        let device = &gpu.device;
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some(label),
            source: wgpu::ShaderSource::Wgsl(shader.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some(label),
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
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some(label),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // Every pixel is painted, opaque.
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: gpu.pipeline_cache(),
        });
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: uniform_size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some(label),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format: wgpu::TextureFormat::Rgba8Unorm,
                    // Slint samples it; COPY_SRC is only used by the previews.
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                        | wgpu::TextureUsages::TEXTURE_BINDING
                        | wgpu::TextureUsages::COPY_SRC,
                    view_formats: &[],
                });
                let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
                Frame { texture, view }
            })
            .collect();
        Self {
            label,
            pipeline,
            uniforms,
            bind_group,
            frames,
        }
    }

    /// Write `uniforms` and draw them into `frame`, without waiting for the GPU.
    pub fn render(&self, gpu: &Graphics, uniforms: &[u8], frame: usize) -> Result<()> {
        gpu.queue.write_buffer(&self.uniforms, 0, uniforms);
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some(self.label),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some(self.label),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.frames[frame].view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        // Painted over in full: nothing to clear or keep.
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
        gpu.submit(encoder.finish());
        Ok(())
    }

    /// The frame as a wgpu texture, which is what crosses to the UI thread.
    pub fn texture(&self, frame: usize) -> wgpu::Texture {
        self.frames[frame].texture.clone()
    }

    /// One frame read back as RGBA pixels, for the previews and tests.
    #[cfg(test)]
    pub fn read_back(&self, gpu: &Graphics, frame: usize) -> Vec<u8> {
        let padded_row = (SIZE * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Lens readback"),
            size: (padded_row as u64) * (SIZE as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Lens readback"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &self.frames[frame].texture,
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

/// A small deterministic generator for the scenes' layouts, so the same seed
/// always gives the same sky and the same slide.
#[derive(Clone)]
pub struct Dice(u64);

impl Dice {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    /// The next value in 0..1.
    pub fn next(&mut self) -> f32 {
        // SplitMix64.
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 40) as f32 / (1u64 << 24) as f32
    }

    /// A value between `lo` and `hi`.
    pub fn range(&mut self, lo: f32, hi: f32) -> f32 {
        lo + (hi - lo) * self.next()
    }

    /// `0..n` in an order shuffled by the dice.
    pub fn order(&mut self, n: usize) -> Vec<usize> {
        let mut order: Vec<usize> = (0..n).collect();
        for i in (1..n).rev() {
            let j = ((self.next() * (i + 1) as f32) as usize).min(i);
            order.swap(i, j);
        }
        order
    }
}

/// Where a tour that holds each stop `hold` seconds after a move of
/// `move_time` seconds to it is, `since` seconds after it set out from stop
/// `offset` (the first, moved on or back by swipes): the stop, and how far
/// into the move to it, from 0 (setting out) to 1 (settled). A swipe starts a
/// fresh move to its stop, so `since` counts from the last one.
pub fn tour_at(since: f64, offset: i64, hold: f64, move_time: f64) -> (i64, f32) {
    let leg = hold + move_time;
    let elapsed = since.max(0.0);
    let legs = (elapsed / leg).floor();
    let into = elapsed - legs * leg;
    let stop = offset + legs as i64;
    let arrived = (into / move_time).min(1.0) as f32;
    (stop, arrived)
}

/// `a` round into `0..n`.
pub fn wrap(a: i64, n: usize) -> usize {
    a.rem_euclid(n as i64) as usize
}

/// The smoothstep curve from 0 at `a` to 1 at `b`.
pub fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dice_repeat_for_a_seed() {
        let a: Vec<f32> = (0..8).scan(Dice::new(7), |d, _| Some(d.next())).collect();
        let b: Vec<f32> = (0..8).scan(Dice::new(7), |d, _| Some(d.next())).collect();
        assert_eq!(a, b);
        assert!(a.iter().all(|v| (0.0..1.0).contains(v)));
        let mut order = Dice::new(3).order(11);
        order.sort();
        assert_eq!(order, (0..11).collect::<Vec<_>>());
    }

    #[test]
    fn a_tour_moves_on_after_each_hold() {
        assert_eq!(tour_at(0.0, 0, 60.0, 5.0), (0, 0.0));
        assert_eq!(tour_at(30.0, 0, 60.0, 5.0), (0, 1.0));
        let (stop, arrived) = tour_at(67.5, 0, 60.0, 5.0);
        assert_eq!(stop, 1);
        assert!((arrived - 0.5).abs() < 1e-6);
        assert_eq!(tour_at(0.0, -2, 60.0, 5.0).0, -2);
        assert_eq!(wrap(-2, 11), 9);
    }

    /// Time both eyepiece scenes on whatever adapter is present, over a few
    /// targets and slides each. Only useful against another run on the same
    /// machine.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        use crate::visuals::Visual;
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let scenes: [(&str, Box<dyn Visual>); 2] = [
            (
                "observatory",
                Box::new(crate::observatory::Observatory::new(&gpu).expect("an observatory")),
            ),
            (
                "microscope",
                Box::new(crate::microscope::Microscope::new(&gpu).expect("a microscope")),
            ),
        ];
        for (name, mut scene) in scenes {
            let frames = 240;
            let mut worst = 0.0f64;
            let started = std::time::Instant::now();
            for i in 0..frames {
                let begun = std::time::Instant::now();
                // A frame every two seconds: through several stops.
                scene
                    .render(&gpu, i as f64 * 2.0, 1, i % SLOTS)
                    .expect("a frame");
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                worst = worst.max(begun.elapsed().as_secs_f64());
            }
            let mean = started.elapsed().as_secs_f64() / frames as f64;
            println!(
                "{name} frame: mean {:.3} ms, worst {:.3} ms",
                mean * 1e3,
                worst * 1e3
            );
        }
    }
}
