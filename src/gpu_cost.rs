//! What the basic operations of a frame cost on the GPU at hand: passes,
//! full-screen fills, blending, a stencil attachment and buffer uploads, all at
//! the panel's 1080 × 1080. Run on the Pi with
//! `spotify-round-tests gpu_cost --ignored --nocapture`.
//!
//! On a Raspberry Pi the V3D driver's own accounting is read as well
//! (`gpu_stats` in sysfs): how many jobs each case made, and how long the GPU
//! spent on them, apart from any waiting on the CPU.

use std::time::Instant;

use slint::wgpu_30::wgpu;

const SIZE: u32 = 1080;
const FRAMES: u32 = 120;

const SHADER: &str = r#"
@group(0) @binding(0) var image: texture_2d<f32>;
@group(0) @binding(1) var image_sampler: sampler;

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> Varying {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Varying;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    out.uv = corners[index] * vec2<f32>(0.5, -0.5) + 0.5;
    return out;
}

@fragment
fn fs_solid() -> @location(0) vec4<f32> {
    return vec4<f32>(0.2, 0.3, 0.4, 0.5);
}

@fragment
fn fs_image(in: Varying) -> @location(0) vec4<f32> {
    return textureSample(image, image_sampler, in.uv) * 0.5;
}
"#;

/// The V3D queues' (jobs, nanoseconds busy), where the driver reports them.
fn v3d_stats() -> Option<Vec<(String, u64, u64)>> {
    let path = std::fs::read_dir("/sys/devices/platform/axi")
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("gpu_stats"))
        .find(|path| path.exists())?;
    let text = std::fs::read_to_string(path).ok()?;
    Some(
        text.lines()
            .skip(1)
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                Some((
                    fields.first()?.to_string(),
                    fields.get(2)?.parse().ok()?,
                    fields.get(3)?.parse().ok()?,
                ))
            })
            .collect(),
    )
}

struct Bench {
    gpu: crate::gpu::Graphics,
    target: wgpu::TextureView,
    stencil: wgpu::TextureView,
    group: wgpu::BindGroup,
    solid: wgpu::RenderPipeline,
    image: wgpu::RenderPipeline,
    blended: wgpu::RenderPipeline,
    stenciled: wgpu::RenderPipeline,
    upload: wgpu::Buffer,
}

impl Bench {
    fn new() -> Self {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let device = &gpu.device;
        let texture = |format, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: None,
                    size: wgpu::Extent3d {
                        width: SIZE,
                        height: SIZE,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let target = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );
        let stencil = texture(
            wgpu::TextureFormat::Stencil8,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let source = texture(
            wgpu::TextureFormat::Rgba8Unorm,
            wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: None,
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: None,
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&source),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: None,
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let pipeline = |entry: &str, blend: Option<wgpu::BlendState>, stencil: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: None,
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
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: stencil.then(|| wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Stencil8,
                    depth_write_enabled: Some(false),
                    depth_compare: Some(wgpu::CompareFunction::Always),
                    stencil: wgpu::StencilState::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let premultiplied = Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING);
        let solid = pipeline("fs_solid", None, false);
        let image = pipeline("fs_image", None, false);
        let blended = pipeline("fs_image", premultiplied, false);
        let stenciled = pipeline("fs_image", premultiplied, true);
        let upload = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: 256 * 1024,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        Self {
            gpu,
            target,
            stencil,
            group,
            solid,
            image,
            blended,
            stenciled,
            upload,
        }
    }

    /// One frame: one submission holding `passes` passes, after `uploads`
    /// buffer writes, each pass drawing `layers` full-screen triangles.
    #[allow(clippy::too_many_arguments)]
    fn frame(
        &self,
        passes: u32,
        load: wgpu::LoadOp<wgpu::Color>,
        stencil: Option<(wgpu::LoadOp<u32>, wgpu::StoreOp)>,
        pipeline: Option<&wgpu::RenderPipeline>,
        layers: u32,
        uploads: u32,
    ) {
        for _ in 0..uploads {
            self.gpu
                .queue
                .write_buffer(&self.upload, 0, &[0u8; 16 * 1024]);
        }
        let mut encoder = self
            .gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor::default());
        for _ in 0..passes {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: None,
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: stencil.map(|(load, store)| {
                    wgpu::RenderPassDepthStencilAttachment {
                        view: &self.stencil,
                        depth_ops: None,
                        stencil_ops: Some(wgpu::Operations { load, store }),
                    }
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            if let Some(pipeline) = pipeline {
                pass.set_pipeline(pipeline);
                pass.set_bind_group(0, &self.group, &[]);
                for _ in 0..layers {
                    pass.draw(0..3, 0..1);
                }
            }
        }
        self.gpu.queue.submit(std::iter::once(encoder.finish()));
    }

    fn measure(&self, name: &str, frame: impl Fn(&Self)) {
        // Warm up: pipelines compile and memory settles on first use.
        for _ in 0..5 {
            frame(self);
        }
        self.gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let before = v3d_stats();
        let begun = Instant::now();
        for _ in 0..FRAMES {
            frame(self);
        }
        self.gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let wall = begun.elapsed().as_secs_f64() * 1e3 / f64::from(FRAMES);
        let after = v3d_stats();
        let mut line = format!("gpu cost, {name:>34}: {wall:6.2} ms a frame");
        if let (Some(before), Some(after)) = (before, after) {
            for ((queue, jobs0, ns0), (_, jobs1, ns1)) in before.iter().zip(after.iter()) {
                let jobs = (jobs1 - jobs0) as f64 / f64::from(FRAMES);
                if jobs > 0.0 {
                    let busy = (ns1 - ns0) as f64 / 1e6 / f64::from(FRAMES);
                    line.push_str(&format!(" | {queue} {jobs:.1} jobs {busy:.2} ms"));
                }
            }
        }
        println!("{line}");
    }
}

/// The same through femtovg, the renderer Slint draws the window with: what
/// one full-screen layer of each kind of paint costs, and the player as it was
/// composed before and after its backdrop became one texture.
#[test]
#[ignore = "a benchmark; needs a working GPU adapter"]
fn femtovg_cost() {
    use femtovg::{Color, ImageFlags, Paint, Path, PixelFormat};

    let b = Bench::new();
    let gpu = b.gpu.clone();
    let renderer = femtovg::renderer::WGPURenderer::new(gpu.device.clone(), gpu.queue.clone());
    let canvas = std::cell::RefCell::new(femtovg::Canvas::new(renderer).expect("a canvas"));
    canvas.borrow_mut().set_size(SIZE, SIZE, 1.0);
    let full = SIZE as f32;
    let image = |size: usize| {
        canvas
            .borrow_mut()
            .create_image_empty(size, size, PixelFormat::Rgba8, ImageFlags::empty())
            .expect("an image")
    };
    let backdrop = image(SIZE as usize);
    let art = image(440);
    let output = || femtovg::renderer::WGPURenderOutput {
        view: b.target.clone(),
        width: SIZE,
        height: SIZE,
        format: wgpu::TextureFormat::Rgba8Unorm,
    };
    let rect = |x: f32, y: f32, w: f32, h: f32, r: f32| {
        let mut path = Path::new();
        path.rounded_rect(x, y, w, h, r);
        path
    };
    let glow = Paint::radial_gradient_stops(
        540.0,
        430.0,
        0.0,
        780.0,
        [
            (0.0, Color::rgba(30, 215, 96, 85)),
            (0.44, Color::rgba(30, 215, 96, 28)),
            (0.74, Color::rgba(0, 0, 0, 0)),
        ],
    );
    let vignette = Paint::radial_gradient_stops(
        540.0,
        540.0,
        0.0,
        700.0,
        [(0.52, Color::rgba(0, 0, 0, 0)), (1.0, Color::rgba(0, 0, 0, 168))],
    );
    let halo = Paint::radial_gradient(540.0, 370.0, 0.0, 190.0, Color::rgba(30, 215, 96, 85), Color::rgba(0, 0, 0, 0));
    let solid = Paint::color(Color::rgba(30, 215, 96, 128));
    let backdrop_paint = Paint::image(backdrop, 0.0, 0.0, full, full, 0.0, 1.0);
    let art_paint = Paint::image(art, 320.0, 150.0, 440.0, 440.0, 0.0, 1.0);

    let frame = |draw: &dyn Fn(&mut femtovg::Canvas<femtovg::renderer::WGPURenderer>)| {
        let mut canvas = canvas.borrow_mut();
        canvas.clear_rect(0, 0, SIZE, SIZE, Color::rgb(7, 7, 10));
        draw(&mut canvas);
        if let Some(commands) = canvas.flush_to_output(output()) {
            gpu.queue.submit(std::iter::once(commands));
        }
    };
    let screen = rect(0.0, 0.0, full, full, 0.0);
    let disc = {
        let mut path = Path::new();
        path.circle(540.0, 540.0, 540.0);
        path
    };
    b.measure("femtovg clear", |_| frame(&|_| {}));
    b.measure("femtovg solid layer", |_| frame(&|c| c.fill_path(&screen, &solid)));
    b.measure("femtovg gradient layer", |_| frame(&|c| c.fill_path(&screen, &glow)));
    b.measure("femtovg image layer", |_| {
        frame(&|c| c.fill_path(&screen, &backdrop_paint))
    });
    b.measure("femtovg round gradient (path)", |_| {
        frame(&|c| c.fill_path(&disc, &glow))
    });
    b.measure("femtovg player, before", |_| {
        frame(&|c| {
            c.fill_path(&screen, &glow);
            c.fill_path(&screen, &vignette);
            c.fill_path(&rect(254.0, 84.0, 572.0, 572.0, 286.0), &halo);
            c.fill_path(&rect(320.0, 150.0, 440.0, 440.0, 31.0), &art_paint);
            c.fill_path(&screen, &backdrop_paint);
        })
    });
    b.measure("femtovg player, after", |_| {
        frame(&|c| {
            c.fill_path(&screen, &backdrop_paint);
            c.fill_path(&rect(254.0, 84.0, 572.0, 572.0, 286.0), &halo);
            c.fill_path(&rect(320.0, 150.0, 440.0, 440.0, 31.0), &art_paint);
        })
    });
}

#[test]
#[ignore = "a benchmark; needs a working GPU adapter"]
fn gpu_cost() {
    use wgpu::LoadOp::{Clear, Load};
    let b = Bench::new();
    let black = Clear(wgpu::Color::BLACK);
    let keep = Some((Load, wgpu::StoreOp::Store));
    let drop = Some((Clear(0), wgpu::StoreOp::Discard));

    b.measure("empty pass, clear", |b| b.frame(1, black, None, None, 0, 0));
    b.measure("empty pass, load", |b| b.frame(1, Load, None, None, 0, 0));
    b.measure("empty pass, load + stencil kept", |b| {
        b.frame(1, Load, keep, None, 0, 0)
    });
    b.measure("empty pass, clear + stencil dropped", |b| {
        b.frame(1, black, drop, None, 0, 0)
    });
    b.measure("solid fill", |b| b.frame(1, black, None, Some(&b.solid), 1, 0));
    b.measure("textured fill", |b| b.frame(1, black, None, Some(&b.image), 1, 0));
    b.measure("blended textured fill", |b| {
        b.frame(1, black, None, Some(&b.blended), 1, 0)
    });
    b.measure("4 blended textured layers", |b| {
        b.frame(1, black, None, Some(&b.blended), 4, 0)
    });
    b.measure("4 blended layers, stencil kept", |b| {
        b.frame(1, Load, keep, Some(&b.stenciled), 4, 0)
    });
    b.measure("4 blended layers, stencil dropped", |b| {
        b.frame(1, black, drop, Some(&b.stenciled), 4, 0)
    });
    b.measure("2 passes of 2 blended layers", |b| {
        b.frame(2, Load, None, Some(&b.blended), 2, 0)
    });
    b.measure("1 buffer upload, empty pass", |b| {
        b.frame(1, black, None, None, 0, 1)
    });
    b.measure("4 buffer uploads, empty pass", |b| {
        b.frame(1, black, None, None, 0, 4)
    });
}
