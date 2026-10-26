//! The progress ring around the rim, drawn on the GPU.
//!
//! Slint's `Path` re-parses and re-tessellates its commands whenever they change,
//! and a stroke of this size is re-tessellated even when it does not. Both cost
//! tens of milliseconds per frame in a debug build. A single fragment shader
//! draws the track, the played sector and the head dot into a texture on the
//! shared wgpu device instead; Slint only composites that texture, so a frame is
//! one uniform write and one full-screen draw call.

#[cfg(test)]
use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::artwork::Rgb;
use crate::gpu::Graphics;

/// Size of one ring frame. The ring is drawn in screen coordinates, so this
/// matches the window.
pub const SIZE: u32 = 1080;
/// Frames kept in flight, so the one Slint is compositing is never the one being
/// written.
pub const SLOTS: usize = 3;

/// Must match the rim geometry in `ui/main.slint` and `swipe_area.slint`.
const CENTER: f32 = 540.0;
const RADIUS: f32 = 520.0;
const THICKNESS: f32 = 6.0;
/// Radius of the head dot, and the larger one shown while scrubbing.
const HEAD: f32 = 8.0;
const HEAD_SCRUB: f32 = 13.0;

/// `Theme.ink` (#f5f5f8) as sRGB, and the track's alpha
/// (`Theme.ink.transparentize(0.9)`).
const INK: [f32; 4] = [0.9608, 0.9608, 0.9725, 1.0];
const TRACK_ALPHA: f32 = 0.1;

/// Everything the ring shows for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// 0..1, from the playback clock or from the finger on the rim.
    pub progress: f32,
    /// A finger is on the rim: the head dot grows.
    pub scrubbing: bool,
    /// Accent colour, following the artwork.
    pub accent: Rgb,
    /// A track is loaded. Without one the ring is hidden, so no frame is drawn.
    pub has_track: bool,
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// Centre x, centre y, radius, thickness.
    geometry: [f32; 4],
    /// Progress, head radius, whether to draw anything, unused.
    state: [f32; 4],
    track: [f32; 4],
    accent: [f32; 4],
    ink: [f32; 4],
}

struct Frame {
    texture: wgpu::Texture,
    view: wgpu::TextureView,
}

pub struct Ring {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    uniforms: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    frames: Vec<Frame>,
}

impl Ring {
    pub fn new(gpu: &Graphics) -> Self {
        let device = &gpu.device;
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Ring shader"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Ring layout"),
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
            label: Some("Ring pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Ring pipeline"),
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
                    // The shader writes premultiplied alpha into a cleared
                    // target; there is nothing to blend against.
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
        let uniforms = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ring uniforms"),
            size: std::mem::size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Ring bind group"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: uniforms.as_entire_binding(),
            }],
        });
        let frames = (0..SLOTS)
            .map(|_| {
                let texture = device.create_texture(&wgpu::TextureDescriptor {
                    label: Some("Ring frame"),
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
            device: gpu.device.clone(),
            queue: gpu.queue.clone(),
            pipeline,
            uniforms,
            bind_group,
            frames,
        }
    }

    /// The render targets, for handing to Slint once. They are wrapped as images
    /// there so the renderer caches them instead of seeing a new image a frame.
    pub fn textures(&self) -> Vec<wgpu::Texture> {
        self.frames
            .iter()
            .map(|frame| frame.texture.clone())
            .collect()
    }

    /// Draw one frame and submit it, without waiting for the GPU.
    pub fn render(&self, input: &Input, frame: usize) {
        let uniforms = Uniforms {
            geometry: [CENTER, CENTER, RADIUS, THICKNESS],
            state: [
                input.progress.clamp(0.0, 1.0),
                if input.scrubbing { HEAD_SCRUB } else { HEAD },
                if input.has_track { 1.0 } else { 0.0 },
                0.0,
            ],
            track: [INK[0], INK[1], INK[2], TRACK_ALPHA],
            accent: [
                input.accent.r as f32 / 255.0,
                input.accent.g as f32 / 255.0,
                input.accent.b as f32 / 255.0,
                1.0,
            ],
            ink: INK,
        };
        self.queue
            .write_buffer(&self.uniforms, 0, bytemuck::bytes_of(&uniforms));

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Ring encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Ring pass"),
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
        self.queue.submit(std::iter::once(encoder.finish()));
    }

    /// One frame as RGBA pixels. Only the design previews need this; the running
    /// display hands the texture to Slint instead.
    #[cfg(test)]
    fn read_back(&self, frame: usize) -> Vec<u8> {
        let padded_row = (SIZE * 4).div_ceil(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT)
            * wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Ring readback"),
            size: (padded_row as u64) * (SIZE as u64),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Ring readback"),
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
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |_| {});
        self.device.poll(wgpu::PollType::wait_indefinitely()).ok();
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

/// Render the ring once, for the design previews.
#[cfg(test)]
pub fn render_once(input: &Input) -> Result<Vec<u8>> {
    let gpu = crate::gpu::headless()?;
    let ring = Ring::new(&gpu);
    ring.render(input, 0);
    Ok(ring.read_back(0))
}

const SHADER: &str = r#"
struct Uniforms {
    geometry: vec4<f32>, // centre x, centre y, radius, thickness
    state: vec4<f32>,    // progress, head radius, drawing, unused
    track: vec4<f32>,
    accent: vec4<f32>,
    ink: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Uniforms;

const TAU: f32 = 6.283185307;

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

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    if (params.state.z < 0.5) {
        return vec4<f32>(0.0);
    }

    let centre = params.geometry.xy;
    let radius = params.geometry.z;
    let half = params.geometry.w * 0.5;
    let progress = clamp(params.state.x, 0.0, 1.0);
    let head_radius = params.state.y;

    let p = in.position.xy;
    let d = length(p - centre);
    // The head dot reaches `head_radius` past the band on both sides.
    if (abs(d - radius) > max(half, head_radius) + 1.0) {
        return vec4<f32>(0.0);
    }

    // Band coverage, antialiased over roughly one pixel.
    let band = 1.0 - smoothstep(half - 0.75, half + 0.75, abs(d - radius));

    // Angle from the top, clockwise, as a 0..1 fraction of a turn.
    let angle = fract(atan2(p.x - centre.x, centre.y - p.y) / TAU);
    // The played part runs from the top to `progress`; soften the leading edge
    // over about a pixel so a slow arc does not crawl in steps.
    let edge = 0.75 / (radius * TAU);
    let sector = (1.0 - smoothstep(progress - edge, progress + edge, angle))
        * step(0.0005, progress);

    let tip = progress * TAU;
    let tip_centre = centre + vec2<f32>(sin(tip), -cos(tip)) * radius;
    let head = (1.0 - smoothstep(head_radius - 0.75, head_radius + 0.75, length(p - tip_centre)))
        * step(0.004, progress);

    let track_alpha = params.track.a * band;
    let arc_alpha = params.accent.a * band * sector;
    let head_alpha = params.ink.a * head;

    let track_colour = params.track.rgb * track_alpha;
    let arc_colour = params.accent.rgb * arc_alpha;
    let head_colour = params.ink.rgb * head_alpha;

    // Source-over: the arc covers the track, the head covers both. Premultiplied.
    let under = arc_colour + track_colour * (1.0 - arc_alpha);
    let alpha = arc_alpha + track_alpha * (1.0 - arc_alpha);
    return vec4<f32>(head_colour + under * (1.0 - head_alpha), head_alpha + alpha * (1.0 - head_alpha));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 4] {
        let i = ((y * SIZE + x) * 4) as usize;
        [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn draws_the_track_the_arc_and_the_head() {
        let pixels = render_once(&Input {
            progress: 0.5,
            scrubbing: false,
            accent: Rgb::new(30, 215, 96),
            has_track: true,
        })
        .unwrap();

        // The played half reaches from the top clockwise to the bottom.
        let top = pixel(&pixels, 540, 20);
        assert!(top[3] > 200, "the arc is not opaque: {top:?}");
        assert!(
            top[1] > 150 && top[1] > top[0],
            "the arc does not use the accent: {top:?}"
        );
        // The unplayed half is the faint track.
        let left = pixel(&pixels, 20, 540);
        assert!(
            left[3] > 0 && left[3] < 60,
            "the track is missing or too strong: {left:?}"
        );
        // The head dot sits at the end of the arc.
        let head = pixel(&pixels, 540, 1060);
        assert!(head[3] > 200, "the head dot is missing: {head:?}");
        // Nothing is drawn inside the circle or outside the band.
        assert_eq!(pixel(&pixels, 540, 540)[3], 0);
        assert_eq!(pixel(&pixels, 8, 8)[3], 0);
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn without_a_track_the_frame_is_clear() {
        let pixels = render_once(&Input {
            progress: 0.4,
            scrubbing: false,
            accent: Rgb::new(30, 215, 96),
            has_track: false,
        })
        .unwrap();
        assert!(
            pixels.chunks(4).all(|pixel| pixel[3] == 0),
            "a hidden ring must be fully transparent"
        );
    }
}
