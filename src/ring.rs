//! The backdrop of the player: the window colour, the ambient glow, the
//! vignette and the progress ring around the rim, drawn on the GPU.
//!
//! Slint's `Path` re-parses and re-tessellates its commands whenever they change,
//! and a stroke of this size is re-tessellated even when it does not. Both cost
//! tens of milliseconds per frame in a debug build. A single fragment shader
//! draws the track, the played sector and the head dot into a texture on the
//! shared wgpu device instead; Slint only composites that texture, so a frame is
//! one uniform write and one full-screen draw call.
//!
//! The same pass paints what lies under the ring, opaque. On the Pi's GPU,
//! which is short of arithmetic, every full-screen layer Slint blends is a
//! million pixels of shading; drawn as Slint gradients, the glow and the
//! vignette were two such layers and the ring, a mostly transparent texture, a
//! third. Here they are one.

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
// As far out as the head dot allows: the panel's own bezel frames the ring.
const RADIUS: f32 = 531.0;
const THICKNESS: f32 = 6.0;
/// Radius of the head dot, and the larger one shown while scrubbing.
const HEAD: f32 = 8.0;
const HEAD_SCRUB: f32 = 13.0;

/// `Theme.ink` (#f5f5f8) as sRGB, and the track's alpha
/// (`Theme.ink.transparentize(0.9)`).
const INK: [f32; 4] = [0.9608, 0.9608, 0.9725, 1.0];
const TRACK_ALPHA: f32 = 0.1;
/// `Theme.bg` (#07070a) as sRGB: the window colour under everything.
const BG: [f32; 4] = [0.0275, 0.0275, 0.0392, 1.0];

/// Everything the ring shows for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Input {
    /// 0..1, from the playback clock or from the finger on the rim.
    pub progress: f32,
    /// A finger is on the rim: the head dot grows.
    pub scrubbing: bool,
    /// Accent colour, following the artwork.
    pub accent: Rgb,
    /// How much of the ring shows, 0..1: it fades in with the first track and
    /// out when playback stops.
    pub ring: f32,
    /// Strength of the ambient glow, 0..1: full once connected, dimmer behind
    /// the login screens.
    pub glow: f32,
}

impl Input {
    /// The same, with the progress rounded to half a pixel along the rim and
    /// the fades to steps the panel can show: the head moves less than that
    /// between most ticks, and every change has the whole window drawn again.
    pub fn on_pixels(self) -> Self {
        let steps = RADIUS * std::f32::consts::TAU * 2.0;
        let levels = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() / 255.0;
        Self {
            progress: (self.progress * steps).round() / steps,
            ring: levels(self.ring),
            glow: levels(self.glow),
            ..self
        }
    }
}

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Uniforms {
    /// Centre x, centre y, radius, thickness.
    geometry: [f32; 4],
    /// Progress, head radius, how much of the ring shows, glow strength.
    state: [f32; 4],
    track: [f32; 4],
    accent: [f32; 4],
    ink: [f32; 4],
    background: [f32; 4],
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
                    // The shader paints every pixel, opaque; there is nothing
                    // to blend against.
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
                input.ring.clamp(0.0, 1.0),
                input.glow.clamp(0.0, 1.0),
            ],
            track: [INK[0], INK[1], INK[2], TRACK_ALPHA],
            accent: [
                input.accent.r as f32 / 255.0,
                input.accent.g as f32 / 255.0,
                input.accent.b as f32 / 255.0,
                1.0,
            ],
            ink: INK,
            background: BG,
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
    state: vec4<f32>,    // progress, head radius, ring shown, glow strength
    track: vec4<f32>,
    accent: vec4<f32>,
    ink: vec4<f32>,
    background: vec4<f32>,
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

// The ambient glow, as `ui/main.slint` drew it: `circle 780px at 540px 430px`,
// the accent at `Theme.glow`'s alpha in the middle, at `Theme.glow-soft`'s by
// 44% and gone by 74%.
const GLOW_CENTRE: vec2<f32> = vec2<f32>(540.0, 430.0);
const GLOW_RADIUS: f32 = 780.0;
const GLOW_INNER: f32 = 0.3333; // 0x55
const GLOW_OUTER: f32 = 0.1098; // 0x1c
// The vignette: clear out to 52% of 700px from the centre, then darkening to
// black at 0xa8 by the edge of that circle.
const VIGNETTE_RADIUS: f32 = 700.0;
const VIGNETTE_START: f32 = 0.52;
const VIGNETTE_ALPHA: f32 = 0.6588;

fn hash(p: vec2<f32>) -> f32 {
    return fract(sin(dot(p, vec2<f32>(12.9898, 78.233))) * 43758.5453);
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let p = in.position.xy;
    var colour = params.background.rgb;

    let g = length(p - GLOW_CENTRE) / GLOW_RADIUS;
    let glow = select(
        mix(GLOW_OUTER, 0.0, clamp((g - 0.44) / 0.30, 0.0, 1.0)),
        mix(GLOW_INNER, GLOW_OUTER, g / 0.44),
        g < 0.44,
    ) * params.state.w;
    colour = params.accent.rgb * glow + colour * (1.0 - glow);

    let v = clamp((length(p - params.geometry.xy) / VIGNETTE_RADIUS - VIGNETTE_START)
        / (1.0 - VIGNETTE_START), 0.0, 1.0);
    colour *= 1.0 - v * VIGNETTE_ALPHA;
    // Dither away the banding in the slow gradients.
    colour += (hash(p) - 0.5) / 255.0;

    // Laid over the backdrop the way Slint composited the ring when it was a
    // texture of its own: Slint takes a wgpu texture for straight alpha, so the
    // premultiplied ring was weighted by its alpha once more. That is how the
    // faint track has always looked on the panel.
    let rim = ring(p);
    let weight = rim.a * params.state.z;
    return vec4<f32>(rim.rgb * weight + colour * (1.0 - weight), 1.0);
}

// The track, the played arc and the head dot, premultiplied.
fn ring(p: vec2<f32>) -> vec4<f32> {
    let centre = params.geometry.xy;
    let radius = params.geometry.z;
    let half = params.geometry.w * 0.5;
    let progress = clamp(params.state.x, 0.0, 1.0);
    let head_radius = params.state.y;

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

    fn input(progress: f32, ring: f32) -> Input {
        Input {
            progress,
            scrubbing: false,
            accent: Rgb::new(30, 215, 96),
            ring,
            glow: 1.0,
        }
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn draws_the_track_the_arc_and_the_head() {
        let pixels = render_once(&input(0.5, 1.0)).unwrap();

        // The played half reaches from the top clockwise to the bottom.
        // The band's centre line, from the edge of the frame.
        let band = (CENTER - RADIUS) as u32;
        let top = pixel(&pixels, 540, band);
        assert!(
            top[1] > 150 && top[1] > top[0],
            "the arc does not use the accent: {top:?}"
        );
        // The unplayed half is the faint track, just lighter than beside it.
        let left = pixel(&pixels, band, 540);
        let beside = pixel(&pixels, band + 20, 540);
        assert!(
            left[0] > beside[0] && left[0] < 30,
            "the track is missing or too strong: {left:?} beside {beside:?}"
        );
        // The head dot sits at the end of the arc.
        let head = pixel(&pixels, 540, SIZE - band);
        assert!(head[0] > 200, "the head dot is missing: {head:?}");
        // Every pixel is opaque: Slint draws nothing under this frame.
        assert!(pixels.chunks(4).all(|pixel| pixel[3] == 255));
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn lays_the_glow_and_the_vignette_under_the_ring() {
        let pixels = render_once(&input(0.4, 1.0)).unwrap();
        // The glow tints the middle with the accent.
        let middle = pixel(&pixels, 540, 430);
        assert!(
            middle[1] > middle[0] + 20,
            "the glow should take the accent: {middle:?}"
        );
        // The vignette leaves the corners close to black.
        let corner = pixel(&pixels, 8, 8);
        assert!(corner[..3].iter().all(|&c| c < 8), "corner {corner:?}");
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn a_hidden_ring_leaves_only_the_backdrop() {
        let band = (CENTER - RADIUS) as u32;
        let hidden = render_once(&input(0.4, 0.0)).unwrap();
        let shown = render_once(&input(0.4, 1.0)).unwrap();
        let top = pixel(&hidden, 540, band);
        let below = pixel(&hidden, 540, band + 20);
        for c in 0..3 {
            assert!(top[c].abs_diff(below[c]) <= 3, "{top:?} against {below:?}");
        }
        assert_ne!(pixel(&shown, 540, band), top, "the ring should show");
    }
}
