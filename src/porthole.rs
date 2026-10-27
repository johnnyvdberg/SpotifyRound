//! The Nautilus' salon window, around the 3D dive's sea.
//!
//! The brass bezel, the riveted casing and the gauge faces never change, so
//! they are drawn once, into a texture Slint lays over the sea. The gauge
//! needles, the only moving part, are two small Slint elements turned to the
//! angles [`dials`] gives (`ui/screens/idle.slint`).
//!
//! This used to be a full-resolution composite every frame that also bent the
//! sea at the edge of the glass and gave it an engraved finish: on the Pi's
//! GPU, which is short of arithmetic, that pass alone cost about a third of
//! the frame.
//!
//! The shader is in `porthole.wgsl`, a small file of its own.

use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::voyage::Position;

/// Size of the window's frame. The porthole fills the round panel edge to edge.
pub const SIZE: u32 = 1080;
/// Half the side of the square the sea covers, as a fraction of [`SIZE`]: a
/// little wider than the glass. Mirrors `VIEW` in `porthole.wgsl`.
pub const VIEW: f32 = 0.470;
/// The log turns once per leg of the voyage; this mirrors `voyage::LEG`.
const LEG: f64 = 3.4;
/// The manometer's scale: the depth at its far end, logarithmic in between,
/// and the arc it sweeps. Mirrors the depth gauge's face in `porthole.wgsl`.
const DEEPEST: f64 = 45.0;
const SWEEP: f64 = 4.7 * 0.92;

/// Per-frame parameters, laid out as 16-byte aligned vectors.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// x, y: frame size in pixels.
    frame: [f32; 4],
}

/// The window: the bezel and the gauge faces, drawn once.
pub(crate) struct Porthole {
    overlay: wgpu::Texture,
}

impl Porthole {
    pub(crate) fn new(gpu: &Graphics) -> Self {
        let device = &gpu.device;
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Porthole uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Porthole bind group"),
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
            label: Some("Porthole bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let overlay = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Porthole overlay"),
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
        let view = overlay.create_view(&wgpu::TextureViewDescriptor::default());
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Porthole shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Porthole pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Porthole overlay pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_overlay"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: wgpu::TextureFormat::Rgba8Unorm,
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

        gpu.queue.write_buffer(
            &uniforms,
            0,
            bytemuck::bytes_of(&Uniforms {
                frame: [SIZE as f32, SIZE as f32, 0.0, 0.0],
            }),
        );
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("Porthole overlay encoder"),
        });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Porthole overlay pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
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
            pass.set_pipeline(&pipeline);
            pass.set_bind_group(0, &group, &[]);
            pass.draw(0..3, 0..1);
        }
        gpu.submit(encoder.finish());
        Self { overlay }
    }

    /// The bezel, the casing and the gauge faces, in straight alpha as Slint
    /// takes a wgpu texture, transparent over the glass.
    pub(crate) fn overlay(&self) -> wgpu::Texture {
        self.overlay.clone()
    }
}

/// The needles at `at`, in radians clockwise from straight up: the manometer,
/// on a logarithmic scale so it moves as much for a few fathoms in the
/// shallows as for a long climb out of the abyss, and the log, which turns
/// once per leg of the voyage.
pub(crate) fn dials(at: Position) -> [f32; 2] {
    let reading = (1.0 + at.depth.max(0.0)).ln() / (1.0 + DEEPEST).ln();
    let depth = (reading - 0.5) * SWEEP;
    let log = (at.x / LEG).rem_euclid(1.0) * std::f64::consts::TAU;
    [depth as f32, log as f32]
}

/// Read a texture back as RGBA pixels, for the design previews and tests.
#[cfg(test)]
pub(crate) fn read_texture(gpu: &Graphics, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let padded_row = (width * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
        * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("Preview readback"),
        size: (padded_row as u64) * (height as u64),
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
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
    );
    gpu.queue.submit(std::iter::once(encoder.finish()));
    let slice = buffer.slice(..);
    slice.map_async(wgpu::MapMode::Read, |_| {});
    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    {
        let data = slice.get_mapped_range().expect("a mapped frame");
        for row in 0..height as usize {
            let start = row * padded_row as usize;
            pixels.extend_from_slice(&data[start..start + (width * 4) as usize]);
        }
    }
    buffer.unmap();
    pixels
}

/// The window as Slint shows it, for the design previews and tests: the sea,
/// `sea` pixels square, scaled up to cover `VIEW` either side of the centre,
/// and the overlay laid over it. The needles are left out.
#[cfg(test)]
pub(crate) fn composite(sea: &[u8], sea_size: u32, overlay: &[u8]) -> Vec<u8> {
    let side = 2.0 * VIEW * SIZE as f32;
    let origin = (SIZE as f32 - side) / 2.0;
    let texel = |x: i32, y: i32, c: usize| {
        let x = x.clamp(0, sea_size as i32 - 1) as usize;
        let y = y.clamp(0, sea_size as i32 - 1) as usize;
        f32::from(sea[(y * sea_size as usize + x) * 4 + c])
    };
    let mut pixels = vec![0u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let i = ((y * SIZE + x) * 4) as usize;
            let u = (x as f32 + 0.5 - origin) / side * sea_size as f32 - 0.5;
            let v = (y as f32 + 0.5 - origin) / side * sea_size as f32 - 0.5;
            let inside = (-0.5..sea_size as f32 - 0.5).contains(&u)
                && (-0.5..sea_size as f32 - 0.5).contains(&v);
            let alpha = f32::from(overlay[i + 3]) / 255.0;
            for c in 0..3 {
                let under = if inside {
                    let (x0, y0) = (u.floor() as i32, v.floor() as i32);
                    let (fx, fy) = (u - u.floor(), v - v.floor());
                    let top = texel(x0, y0, c) * (1.0 - fx) + texel(x0 + 1, y0, c) * fx;
                    let bottom =
                        texel(x0, y0 + 1, c) * (1.0 - fx) + texel(x0 + 1, y0 + 1, c) * fx;
                    top * (1.0 - fy) + bottom * fy
                } else {
                    0.0
                };
                let over = f32::from(overlay[i + c]);
                pixels[i + c] = (over * alpha + under * (1.0 - alpha)).round() as u8;
            }
            pixels[i + 3] = 255;
        }
    }
    pixels
}

/// The bezel, the casing and the gauge faces.
const SHADER: &str = include_str!("porthole.wgsl");
