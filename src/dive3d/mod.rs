//! The Nautilus' voyage as a real 3D scene.
//!
//! Seven biomes in turn, with the swipes and the dive profile of
//! [`crate::voyage`], rasterised: a seabed
//! mesh lifted in the vertex shader, procedurally generated models placed and
//! animated each frame by [`world`], a depth buffer and 4× multisampling. A
//! pixel only pays for what actually covers it, instead of every pixel asking
//! every feature whether it is there.
//!
//! The window itself is not part of the 3D scene: the brass bezel, the gauges,
//! the engraved finish and the bending of the glass are full-screen passes of
//! their own, in [`crate::porthole::Porthole`].

mod heightmap;
mod helm;
mod mesh;
mod textures;
mod world;

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::porthole::Porthole;
use crate::visuals::Visual;
use crate::voyage::{Biome, Voyage};
use helm::{Helm, Pose};
use mesh::{Library, Vertex};
use world::{Frame, Instance, Seabed, Sprite, World};

pub use crate::porthole::SIZE;

/// Edge of the square the sea is rendered into. Geometry is cheap here, so it
/// is drawn a little sharper than the procedural sea.
const SEA: u32 = 640;
const SAMPLES: u32 = 4;
const NEAR: f32 = 0.05;
/// How far the glass bends the view at its edge.
const BEND: f32 = 0.008;
const MAX_INSTANCES: usize = 24_576;
const MAX_SPRITES: usize = 8_192;
const MAX_TUNNEL_VERTICES: usize = 16_384;
const MAX_TUNNEL_INDICES: usize = 98_304;
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// Per-frame parameters, in the layout of `Globals` in `scene.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct Globals {
    camera: [f32; 4],
    view: [f32; 4],
    right: [f32; 4],
    up: [f32; 4],
    out: [f32; 4],
    lamp_origin: [f32; 4],
    sun: [f32; 4],
    water: [f32; 4],
    deep: [f32; 4],
    lamp: [f32; 4],
    levels: [f32; 4],
    world: [f32; 4],
    counts: [f32; 4],
    lights: [[f32; 4]; 8],
    light_colours: [[f32; 4]; 8],
}

pub(crate) struct Nautilus {
    background: wgpu::RenderPipeline,
    terrain: wgpu::RenderPipeline,
    models: wgpu::RenderPipeline,
    glass: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    group: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    ranges: Vec<(u32, u32, i32)>,
    grid: wgpu::Buffer,
    grid_indices: wgpu::Buffer,
    grid_count: u32,
    /// The tunnel through the cave's massif, rebuilt each frame.
    tunnel: wgpu::Buffer,
    tunnel_indices: wgpu::Buffer,
    instances: wgpu::Buffer,
    sprite_buffer: wgpu::Buffer,
    colour: wgpu::TextureView,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
    porthole: Porthole,
    relief: wgpu::Texture,
    voyage: Voyage,
    helm: Helm,
    /// The seabed of the current visit, and the visit it was made for.
    seabed: Seabed,
    visit: Option<u64>,
    frame: Frame,
    seconds: f64,
}

impl Nautilus {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        let device = &gpu.device;
        let library = Library::build();
        let buffer = |label, contents: &[u8], usage| {
            let b = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: contents.len().max(4) as u64,
                usage: usage | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            gpu.queue.write_buffer(&b, 0, contents);
            b
        };
        let vertices = buffer(
            "Dive3d vertices",
            bytemuck::cast_slice(&library.vertices),
            wgpu::BufferUsages::VERTEX,
        );
        let indices = buffer(
            "Dive3d indices",
            bytemuck::cast_slice(&library.indices),
            wgpu::BufferUsages::INDEX,
        );
        let (grid_points, grid_index) = mesh::terrain_grid();
        let grid = buffer(
            "Dive3d seabed",
            bytemuck::cast_slice(&grid_points),
            wgpu::BufferUsages::VERTEX,
        );
        let grid_indices = buffer(
            "Dive3d seabed indices",
            bytemuck::cast_slice(&grid_index),
            wgpu::BufferUsages::INDEX,
        );
        let instances = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive3d instances"),
            size: (MAX_INSTANCES * std::mem::size_of::<Instance>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let sprite_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive3d sprites"),
            size: (MAX_SPRITES * std::mem::size_of::<Sprite>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tunnel = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive3d tunnel"),
            size: (MAX_TUNNEL_VERTICES * std::mem::size_of::<Vertex>()) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let tunnel_indices = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive3d tunnel indices"),
            size: (MAX_TUNNEL_INDICES * 4) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive3d globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Dive3d globals"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        // The seabed's relief, replaced on every visit; and the surfaces, painted once.
        let relief = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dive3d relief"),
            size: wgpu::Extent3d {
                width: heightmap::SIZE as u32,
                height: heightmap::SIZE as u32,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba32Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let painted = textures::paint_all();
        let layers = textures::Surface::ALL.len() as u32;
        let surfaces = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dive3d surfaces"),
            size: wgpu::Extent3d {
                width: textures::SIZE as u32,
                height: textures::SIZE as u32,
                depth_or_array_layers: layers,
            },
            mip_level_count: painted.len() as u32,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        for (level, mip) in painted.iter().enumerate() {
            let size = (textures::SIZE >> level).max(1) as u32;
            for (layer, texels) in mip.iter().enumerate() {
                gpu.queue.write_texture(
                    wgpu::TexelCopyTextureInfo {
                        texture: &surfaces,
                        mip_level: level as u32,
                        origin: wgpu::Origin3d {
                            x: 0,
                            y: 0,
                            z: layer as u32,
                        },
                        aspect: wgpu::TextureAspect::All,
                    },
                    texels,
                    wgpu::TexelCopyBufferLayout {
                        offset: 0,
                        bytes_per_row: Some(size * 4),
                        rows_per_image: Some(size),
                    },
                    wgpu::Extent3d {
                        width: size,
                        height: size,
                        depth_or_array_layers: 1,
                    },
                );
            }
        }
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dive3d surfaces"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 4,
            ..Default::default()
        });
        let relief_view = relief.create_view(&wgpu::TextureViewDescriptor::default());
        let surfaces_view = surfaces.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Dive3d globals"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: globals.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&relief_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&surfaces_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let target = |label, format, samples, usage| {
            device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d {
                        width: SEA,
                        height: SEA,
                        depth_or_array_layers: 1,
                    },
                    mip_level_count: 1,
                    sample_count: samples,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage,
                    view_formats: &[],
                })
                .create_view(&wgpu::TextureViewDescriptor::default())
        };
        let colour = target(
            "Dive3d colour",
            COLOUR,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let depth = target(
            "Dive3d depth",
            DEPTH,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolved = target(
            "Dive3d sea",
            COLOUR,
            1,
            wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        );

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dive3d shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("scene.wgsl").into()),
        });
        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Dive3d pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let model_attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x3, 3 => Float32x2,
        ];
        let instance_attributes = wgpu::vertex_attr_array![
            4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4, 9 => Float32x4,
            10 => Float32x4,
        ];
        let terrain_attributes = wgpu::vertex_attr_array![0 => Float32x3];
        let sprite_attributes =
            wgpu::vertex_attr_array![0 => Float32x4, 1 => Float32x4, 2 => Float32x4];
        let model_buffers = [
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Vertex>() as u64,
                step_mode: wgpu::VertexStepMode::Vertex,
                attributes: &model_attributes,
            }),
            Some(wgpu::VertexBufferLayout {
                array_stride: std::mem::size_of::<Instance>() as u64,
                step_mode: wgpu::VertexStepMode::Instance,
                attributes: &instance_attributes,
            }),
        ];
        let terrain_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: 12,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &terrain_attributes,
        })];
        let sprite_buffers = [Some(wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Sprite>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &sprite_attributes,
        })];
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::OVER,
        };
        let pipeline = |label,
                        vs,
                        fs,
                        buffers: &[Option<wgpu::VertexBufferLayout>],
                        blend: Option<wgpu::BlendState>,
                        write_depth: bool,
                        compare: wgpu::CompareFunction,
                        coverage: bool| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers,
                    compilation_options: Default::default(),
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: COLOUR,
                        blend,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),
                // Fins, sails and fronds are single sheets: nothing is culled.
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write_depth),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    // Fading models out by coverage, so they melt away smoothly.
                    alpha_to_coverage_enabled: coverage,
                    ..Default::default()
                },
                multiview_mask: None,
                cache: None,
            })
        };
        // Depth is reversed — near at 1, far toward 0 — for precision far out.
        let nearer = wgpu::CompareFunction::Greater;
        let background = pipeline(
            "Dive3d water",
            "vs_background",
            "fs_background",
            &[],
            None,
            false,
            wgpu::CompareFunction::Always,
            false,
        );
        let terrain = pipeline(
            "Dive3d seabed",
            "vs_terrain",
            "fs_terrain",
            &terrain_buffers,
            None,
            true,
            nearer,
            false,
        );
        let models = pipeline(
            "Dive3d models",
            "vs_model",
            "fs_model",
            &model_buffers,
            None,
            true,
            nearer,
            true,
        );
        let glass = pipeline(
            "Dive3d glass",
            "vs_model",
            "fs_glass",
            &model_buffers,
            Some(additive),
            false,
            nearer,
            false,
        );
        let sprites = pipeline(
            "Dive3d sprites",
            "vs_sprite",
            "fs_sprite",
            &sprite_buffers,
            Some(additive),
            false,
            nearer,
            false,
        );

        let porthole = Porthole::new(gpu, &resolved, SEA, BEND);
        tracing::info!(
            models = library.ranges.len(),
            vertices = library.vertices.len(),
            "3D dive ready"
        );
        Ok(Self {
            background,
            terrain,
            models,
            glass,
            sprites,
            globals,
            group,
            vertices,
            indices,
            ranges: library.ranges,
            grid,
            grid_indices,
            grid_count: grid_index.len() as u32,
            tunnel,
            tunnel_indices,
            instances,
            sprite_buffer,
            colour,
            depth,
            resolved,
            porthole,
            relief,
            voyage: Voyage::default(),
            helm: Helm::default(),
            seabed: Seabed {
                map: vec![[0.0; 4]; heightmap::SIZE * heightmap::SIZE],
                seed: 0.0,
            },
            visit: None,
            frame: Frame::new(),
            seconds: 0.0,
        })
    }

    fn draw(&mut self, gpu: &Graphics, seconds: f64, visit: u64, frame: usize) -> Result<()> {
        let seed = (visit % 1_000_003) as f32 / 1_000_003.0;
        if self.visit != Some(visit) {
            // A new visit: a new seabed, and the boat settles onto it afresh.
            self.seabed = Seabed::new(visit, seed);
            self.helm = Helm::default();
            self.visit = Some(visit);
            gpu.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.relief,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                bytemuck::cast_slice(&self.seabed.map),
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(heightmap::SIZE as u32 * 16),
                    rows_per_image: Some(heightmap::SIZE as u32),
                },
                wgpu::Extent3d {
                    width: heightmap::SIZE as u32,
                    height: heightmap::SIZE as u32,
                    depth_or_array_layers: 1,
                },
            );
        }
        let voyage = self.voyage.advance(seconds, seed);
        let pose = self.helm.steer(&self.seabed, voyage, seconds);
        self.seconds = seconds;
        let t = seconds.rem_euclid(6_000.0) as f32;
        let world = World {
            seabed: &self.seabed,
            pose,
            t,
        };
        world::build(&world, &mut self.frame);
        // The gauges read the boat's real depth.
        let at = crate::voyage::Position {
            x: f64::from(pose.pos[0]),
            altitude: voyage.altitude,
            depth: pose.depth,
        };
        gpu.queue.write_buffer(
            &self.globals,
            0,
            bytemuck::bytes_of(&globals(&world, &self.frame)),
        );

        // Solid instances first, model by model, then the translucent ones.
        let mut all: Vec<Instance> = Vec::with_capacity(4096);
        let mut draws: Vec<(usize, u32, u32, bool)> = Vec::new();
        for (glassy, lists) in [(false, &self.frame.solid), (true, &self.frame.glass)] {
            for (model, list) in lists.iter().enumerate() {
                let room = MAX_INSTANCES - all.len();
                let n = list.len().min(room);
                if n == 0 {
                    continue;
                }
                draws.push((model, all.len() as u32, n as u32, glassy));
                all.extend_from_slice(&list[..n]);
            }
        }
        // The tunnel is in world space already: one instance that moves nothing.
        let tunnel_count = self.frame.tunnel_indices.len().min(MAX_TUNNEL_INDICES) as u32;
        let tunnel_drawn = tunnel_count > 0
            && self.frame.tunnel.len() <= MAX_TUNNEL_VERTICES
            && all.len() < MAX_INSTANCES;
        let tunnel_instance = all.len() as u32;
        if tunnel_drawn {
            all.push(Instance {
                rows: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                ],
                colour: [0.15, 0.13, 0.12, 0.0],
                anim: [0.0; 4],
                extra: [world::material::VAULT, 0.0, 1.0, 0.0],
                surface: [textures::Surface::Basalt.layer(), 1.5, 0.0, 0.0],
            });
            gpu.queue
                .write_buffer(&self.tunnel, 0, bytemuck::cast_slice(&self.frame.tunnel));
            gpu.queue.write_buffer(
                &self.tunnel_indices,
                0,
                bytemuck::cast_slice(&self.frame.tunnel_indices[..tunnel_count as usize]),
            );
        }
        gpu.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(&all));
        let sprites = &self.frame.sprites[..self.frame.sprites.len().min(MAX_SPRITES)];
        gpu.queue
            .write_buffer(&self.sprite_buffer, 0, bytemuck::cast_slice(sprites));

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Dive3d encoder"),
            });
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Dive3d sea pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &self.colour,
                    depth_slice: None,
                    resolve_target: Some(&self.resolved),
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        // Only the resolved image is needed afterwards.
                        store: wgpu::StoreOp::Discard,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0.0),
                        store: wgpu::StoreOp::Discard,
                    }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.group, &[]);
            pass.set_pipeline(&self.background);
            pass.draw(0..3, 0..1);

            pass.set_pipeline(&self.terrain);
            pass.set_vertex_buffer(0, self.grid.slice(..));
            pass.set_index_buffer(self.grid_indices.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..self.grid_count, 0, 0..1);

            pass.set_vertex_buffer(1, self.instances.slice(..));
            if tunnel_drawn {
                pass.set_pipeline(&self.models);
                pass.set_vertex_buffer(0, self.tunnel.slice(..));
                pass.set_index_buffer(self.tunnel_indices.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..tunnel_count, 0, tunnel_instance..tunnel_instance + 1);
            }
            pass.set_vertex_buffer(0, self.vertices.slice(..));
            pass.set_index_buffer(self.indices.slice(..), wgpu::IndexFormat::Uint32);
            for glassy in [false, true] {
                pass.set_pipeline(if glassy { &self.glass } else { &self.models });
                for &(model, first, count, g) in &draws {
                    if g != glassy {
                        continue;
                    }
                    let (start, length, base) = self.ranges[model];
                    pass.draw_indexed(start..start + length, base, first..first + count);
                }
            }

            if !sprites.is_empty() {
                pass.set_pipeline(&self.sprites);
                pass.set_vertex_buffer(0, self.sprite_buffer.slice(..));
                pass.draw(0..6, 0..sprites.len() as u32);
            }
        }
        self.porthole
            .draw(gpu, &mut encoder, seconds, seed, at, frame);
        gpu.queue.submit(std::iter::once(encoder.finish()));
        Ok(())
    }

    #[cfg(test)]
    fn read_back(&mut self, gpu: &Graphics, seconds: f64, seed: u64) -> Vec<u8> {
        // Sail the last twenty seconds first, so the helm has settled as it
        // would have on screen.
        self.draw(gpu, seconds - 20.0, seed, 0)
            .expect("a 3D dive frame");
        for i in 1..600 {
            let at = seconds - 20.0 + f64::from(i) / 30.0;
            let voyage = self
                .voyage
                .advance(at, (seed % 1_000_003) as f32 / 1_000_003.0);
            self.helm.steer(&self.seabed, voyage, at);
        }
        self.draw(gpu, seconds, seed, 0).expect("a 3D dive frame");
        crate::porthole::read_texture(gpu, &self.porthole.texture(0))
    }
}

impl Visual for Nautilus {
    fn render(&mut self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        self.draw(gpu, seconds, seed, frame)
    }

    fn texture(&self, frame: usize) -> wgpu::Texture {
        self.porthole.texture(frame)
    }

    fn travel(&mut self, legs: i32) {
        self.voyage.travel(legs, self.seconds);
    }
}

fn soft(a: f32, b: f32, v: f32) -> f32 {
    let t = ((v - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn mix3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

/// The colour of the water at `depth` (0 at the surface, 1 far down the abyss),
/// the same ramp as the procedural dive's.
fn water_tone(d: f32) -> [f32; 3] {
    let mut c = mix3(
        [0.11, 0.47, 0.46],
        [0.022, 0.175, 0.200],
        soft(0.0, 0.36, d),
    );
    c = mix3(c, [0.006, 0.052, 0.078], soft(0.30, 0.72, d));
    mix3(c, [0.002, 0.009, 0.020], soft(0.66, 1.08, d))
}

fn globals(w: &World, frame: &Frame) -> Globals {
    let pose: Pose = w.pose;
    let depth = 1.0 - (-(pose.depth as f32) / 7.0).exp();
    let here = Biome::at(f64::from(pose.pos[0]), w.seabed.seed);
    let share = |id: usize| here.weight(id) as f32;
    let cave = share(world::CAVE);
    let arctic = share(world::ARCTIC);
    let blend =
        |values: [f32; 7]| values[here.a] + (values[here.b] - values[here.a]) * here.w as f32;
    let tint = |c: [f32; 3]| {
        let c = mix3(c, [0.05, 0.22, 0.32], arctic * 0.75);
        mix3(c, [0.030, 0.012, 0.010], cave * 0.85)
    };
    let water = tint(water_tone(depth));
    let deep = tint(water_tone((depth + 0.30).min(1.1)));
    let sun = (-depth * 4.3).exp() * (1.0 - 0.95 * cave);
    let lamp = 0.22 + 0.95 * soft(0.12, 0.70, depth);
    let [right, up, out] = pose.basis;
    let in_boat = |v: [f32; 3]| {
        [
            right[0] * v[0] + up[0] * v[1] + out[0] * v[2],
            right[1] * v[0] + up[1] * v[1] + out[1] * v[2],
            right[2] * v[0] + up[2] * v[1] + out[2] * v[2],
        ]
    };
    // The searchlight is mounted on the hull and turns with the boat.
    let lamp_dir = {
        let d = in_boat([0.30, -0.45, 1.0]);
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / l, d[1] / l, d[2] / l]
    };
    let mount = in_boat([-0.35, 0.40, 0.0]);
    let lamp_origin = [
        pose.pos[0] + mount[0],
        pose.pos[1] + mount[1],
        pose.pos[2] + mount[2],
    ];
    let sun_dir = {
        let d = [0.30f32, 1.0, 0.25];
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / l, d[1] / l, d[2] / l]
    };
    let cam = pose.pos;

    // The lights that matter most: nearest the window, weighed by their reach.
    let mut lights: Vec<_> = frame.lights.iter().collect();
    lights.sort_by(|a, b| {
        let score = |l: &&world::Light| {
            let d = [l.pos[0] - cam[0], l.pos[1] - cam[1], l.pos[2] - 1.5];
            (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() / l.range
        };
        score(a).total_cmp(&score(b))
    });
    let v4 = |v: [f32; 3]| [v[0], v[1], v[2], 0.0];
    let mut g = Globals {
        camera: [cam[0], cam[1], cam[2], w.t],
        view: [world::VIEW, NEAR, 0.0, 0.0],
        right: v4(right),
        up: v4(up),
        out: v4(out),
        lamp_origin: v4(lamp_origin),
        sun: [sun_dir[0], sun_dir[1], sun_dir[2], sun],
        water: [
            water[0],
            water[1],
            water[2],
            blend([0.18, 0.22, 0.15, 0.18, 0.20, 0.22, 0.12]),
        ],
        deep: [deep[0], deep[1], deep[2], pose.depth as f32],
        lamp: [lamp_dir[0], lamp_dir[1], lamp_dir[2], lamp],
        levels: [w.surface(), w.vault(), w.ice(), 0.0],
        world: [
            w.seabed.seed,
            share(world::ATLANTIS) + 1.8 * cave,
            cave,
            arctic,
        ],
        counts: [lights.len().min(8) as f32, 0.0, 0.0, 0.0],
        lights: [[0.0; 4]; 8],
        light_colours: [[0.0; 4]; 8],
    };
    for (i, l) in lights.iter().take(8).enumerate() {
        g.lights[i] = [l.pos[0], l.pos[1], l.pos[2], l.range];
        g.light_colours[i] = [l.colour[0], l.colour[1], l.colour[2], 0.0];
    }
    g
}

/// Render a single frame synchronously, for the design previews.
#[cfg(test)]
pub fn render_once(seconds: f64, seed: u64) -> Result<Vec<u8>> {
    let gpu = crate::gpu::headless()?;
    let mut scene = Nautilus::new(&gpu)?;
    Ok(scene.read_back(&gpu, seconds, seed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leg(n: f64) -> f64 {
        crate::voyage::LEG / crate::voyage::SPEED * (n + 0.35)
    }

    /// Write frames from every leg of the voyage to `DIVE3D_PREVIEW_DIR`.
    #[test]
    #[ignore = "writes design previews; set DIVE3D_PREVIEW_DIR"]
    fn previews() {
        let Ok(dir) = std::env::var("DIVE3D_PREVIEW_DIR") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        let names = [
            "shallows", "forest", "wrecks", "atlantis", "cave", "abyss", "arctic",
        ];
        let offsets: Vec<f64> = std::env::var("DIVE3D_PREVIEW_OFFSETS")
            .ok()
            .map(|list| {
                list.split(',')
                    .filter_map(|v| v.trim().parse().ok())
                    .collect()
            })
            .unwrap_or_else(|| vec![0.0]);
        for (n, name) in names.iter().enumerate() {
            for (k, offset) in offsets.iter().enumerate() {
                let pixels = scene.read_back(&gpu, leg(n as f64) + offset, 0);
                let path = std::path::Path::new(&dir).join(format!("{n}-{name}-{k}.png"));
                image::save_buffer(&path, &pixels, SIZE, SIZE, image::ExtendedColorType::Rgba8)
                    .expect("a preview image");
            }
        }
    }

    #[test]
    #[ignore = "needs a working GPU adapter"]
    fn the_porthole_shows_a_sea() {
        let pixels = render_once(leg(0.0), 0).expect("a GPU adapter and a frame");
        let at = |x: u32, y: u32| {
            let i = ((y * SIZE + x) * 4) as usize;
            [pixels[i], pixels[i + 1], pixels[i + 2]]
        };
        let centre = at(SIZE / 2, SIZE / 2);
        assert!(
            centre.iter().any(|&c| c > 10),
            "the sea should not be black: {centre:?}"
        );
        assert_ne!(at(0, 0), centre, "the casing and the sea should differ");
    }

    /// Time the scene per leg, on whatever adapter is present.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        scene.draw(&gpu, 0.0, 0, 0).expect("a frame");
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let names = [
            "shallows", "forest", "wrecks", "atlantis", "cave", "abyss", "arctic",
        ];
        let frames = 60;
        let mut total = 0.0;
        for (n, name) in names.iter().enumerate() {
            let (mut cpu, mut drawn) = (0.0, 0.0);
            for i in 0..frames {
                let seconds = leg(n as f64) + i as f64 * 0.25;
                let begun = std::time::Instant::now();
                let at = scene.voyage.advance(seconds, 0.0);
                let pose = scene.helm.steer(&scene.seabed, at, seconds);
                let world = World {
                    seabed: &scene.seabed,
                    pose,
                    t: seconds as f32,
                };
                world::build(&world, &mut scene.frame);
                cpu += begun.elapsed().as_secs_f64();
                // The frame proper, which builds the scene again itself.
                let begun = std::time::Instant::now();
                scene
                    .draw(&gpu, seconds, 0, i % crate::visuals::SLOTS)
                    .expect("a frame");
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                drawn += begun.elapsed().as_secs_f64();
            }
            let mean = drawn / frames as f64;
            total += mean;
            let count: usize = scene.frame.solid.iter().map(Vec::len).sum::<usize>()
                + scene.frame.glass.iter().map(Vec::len).sum::<usize>();
            println!(
                "dive3d frame, {name}: mean {:.3} ms, scene building {:.3} ms, {count} instances, {} sprites",
                mean * 1e3,
                cpu / frames as f64 * 1e3,
                scene.frame.sprites.len()
            );
        }
        println!(
            "dive3d frame: mean {:.3} ms",
            total / names.len() as f64 * 1e3
        );
    }
}
