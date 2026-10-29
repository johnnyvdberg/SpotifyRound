//! The Nautilus' voyage as a real 3D scene.
//!
//! Seven biomes in turn, with the swipes and the dive profile of
//! [`crate::voyage`], rasterised: a seabed built in
//! tiles as it comes into view ([`terrain`]), procedurally generated models placed and
//! animated each frame by [`world`], a depth buffer and one sample. A
//! pixel only pays for what actually covers it, instead of every pixel asking
//! every feature whether it is there.
//!
//! The window itself is not part of the 3D scene: the 1080 sea texture is
//! handed straight to Slint as the full panel.

mod heightmap;
mod helm;
mod mesh;
#[cfg(test)]
mod places;
mod terrain;
mod textures;
mod world;

use anyhow::Result;
use slint::wgpu_30::wgpu;

use crate::gpu::Graphics;
use crate::visuals::Visual;
use crate::voyage::{Biome, Voyage};
use helm::{Helm, Pose};
use mesh::{Library, Lod, Vertex};
use terrain::Terrain;
use world::{Frame, Instance, Seabed, Sprite, World};

/// Edge of the panel-sized sea.
pub const SIZE: u32 = 1080;

/// The sea is the panel's own pixels; there is no smaller profile.
const SEA: u32 = SIZE;
/// Samples per pixel. Multisampling quarters the pixels each of the Pi GPU's
/// tiles holds, and the tiles' fixed costs outweighed the smoother edges, which
/// the upscale softens anyway; the one sample keeps the fading models smooth by
/// blending their coverage over the water (see the fading pipeline).
const SAMPLES: u32 = 1;
const MULTISAMPLED: bool = SAMPLES > 1;
/// A coarser level of detail is drawn only where its surface strays from the
/// full model's by less than `LOD_ERROR`, in pixels of the sea, which is
/// drawn at `SEA` and no finer, so no difference smaller can be drawn; and, unless
/// its colours and normals stray by less than `LOD_STRAY` too, where the
/// triangles it changed are under `LOD_EDGE` pixels, so whatever shading it
/// dropped was finer than a couple of pixels.
const LOD_ERROR: f32 = 2.0;
const LOD_STRAY: f32 = 0.1;
const LOD_EDGE: f32 = 6.0;
/// The corner mask's triangles: six vertices to each of `MASK_SEGMENTS` in
/// `scene.wesl`.
const MASK_VERTICES: u32 = 6 * 64;
const NEAR: f32 = 0.05;
const MAX_INSTANCES: usize = 24_576;
const MAX_SPRITES: usize = 8_192;
const MAX_TUNNEL_VERTICES: usize = 16_384;
const MAX_TUNNEL_INDICES: usize = 98_304;
/// The sea, at half its brightness (`SEA_SCALE` in `scene.wesl`): ten bits a
/// channel are finer than the panel's eight, and half the size of half
/// floats, so the GPU's tiles hold twice the pixels.
const COLOUR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

/// The caustic tile, generated once on the CPU: two sines and a pow in Rust
/// instead of in every vertex shader.
/// The CPU side of the caustic precompute, split out so it can be timed without
/// a GPU.
fn caustic_texels() -> Vec<u8> {
    const SIZE: usize = 256;
    let mut texels = vec![0u8; SIZE * SIZE];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let p = [x as f32 / SIZE as f32, y as f32 / SIZE as f32];
            let a = (p[0] * std::f32::consts::TAU + (p[1] * 8.168).sin()).sin();
            let b = (p[1] * 6.912 + (p[0] * 5.655).sin()).sin();
            let c = (a + b).abs().mul_add(-0.5, 1.0).max(0.0).powf(7.0);
            texels[y * SIZE + x] = (c * 255.0) as u8;
        }
    }
    texels
}

fn caustic_texture(device: &wgpu::Device, queue: &wgpu::Queue) -> wgpu::Texture {
    const SIZE: u32 = 256;
    let texels = caustic_texels();
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Dive3d caustics"),
        size: wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        wgpu::TexelCopyTextureInfo {
            texture: &texture,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        &texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(SIZE),
            rows_per_image: Some(SIZE),
        },
        wgpu::Extent3d {
            width: SIZE,
            height: SIZE,
            depth_or_array_layers: 1,
        },
    );
    texture
}

/// Per-frame parameters, in the layout of `Globals` in `scene.wesl`.
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
    tint: [f32; 4],
    lights: [[f32; 4]; 8],
    light_colours: [[f32; 4]; 8],
}

pub(crate) struct Nautilus {
    background: wgpu::RenderPipeline,
    terrain_pipeline: wgpu::RenderPipeline,
    /// Solid models: one pipeline per entry of [`MATERIALS`] and grounded
    /// flag.
    models: Vec<wgpu::RenderPipeline>,
    /// The same for sealed models, their back faces culled.
    sealed: Vec<wgpu::RenderPipeline>,
    /// Marks the corners of the sea outside the round window as covered.
    mask: wgpu::RenderPipeline,
    /// The same, for models fading in or out, which only they may discard.
    fading: Vec<wgpu::RenderPipeline>,
    glass: wgpu::RenderPipeline,
    sprites: wgpu::RenderPipeline,
    shafts: wgpu::RenderPipeline,
    /// Searchlight beams of other submarines.
    beams: wgpu::RenderPipeline,
    globals: wgpu::Buffer,
    group: wgpu::BindGroup,
    #[cfg(test)]
    trilinear: wgpu::BindGroup,
    vertices: wgpu::Buffer,
    indices: wgpu::Buffer,
    /// Per model, its levels of detail, the full one first.
    lods: Vec<Vec<mesh::Lod>>,
    /// Per model, whether its back faces can be culled.
    closed: Vec<bool>,
    /// The seabed's tiles, a slot of [`terrain::VERTICES`] each, and the
    /// triangles every tile shares.
    ground: wgpu::Buffer,
    ground_indices: wgpu::Buffer,
    ground_count: u32,
    terrain: Terrain,
    /// The tunnel through the cave's massif, and which of its builds
    /// ([`world::Frame::tunnel_version`]) the buffers hold.
    tunnel: wgpu::Buffer,
    tunnel_indices: wgpu::Buffer,
    tunnel_uploaded: Option<u64>,
    instances: wgpu::Buffer,
    /// Kept from frame to frame so sorting the instances allocates nothing:
    /// where each goes, the instances in that order, and the draws over them.
    order: Vec<Staged>,
    staged: Vec<Instance>,
    draws: Vec<Draw>,
    sprite_buffer: wgpu::Buffer,
    colour: wgpu::TextureView,
    /// The sea texture itself, for the full-HD UI layer. The view above is
    /// the same texture when multisampling is off.
    #[allow(dead_code)]
    sea_texture: wgpu::Texture,
    #[allow(dead_code)]
    caustic: wgpu::Texture,
    depth: wgpu::TextureView,
    resolved: wgpu::TextureView,
    relief: wgpu::Texture,
    voyage: Voyage,
    helm: Helm,
    /// The seabed of the current visit, and the visit it was made for.
    seabed: Seabed,
    visit: Option<u64>,
    frame: Frame,
    seconds: f64,
    /// Parts of the scene a benchmark leaves out, to see what each costs.
    #[cfg(test)]
    skip: u16,
}

/// The materials models get a pipeline of their own for, as `MATERIAL` in
/// `scene.wesl`: plain first, then those of [`world::material`]. Anything else
/// is drawn by the last, -1, which reads the material per instance. Each has a
/// second pipeline for models that stand on the seabed (`GROUNDED`).
const MATERIALS: [i32; 8] = [
    0,
    world::material::LAVA as i32,
    world::material::COPPER as i32,
    world::material::SURFACE as i32,
    world::material::VAULT as i32,
    world::material::ICE as i32,
    world::material::GOLD as i32,
    -1,
];

/// The index into [`MATERIALS`] of the pipeline for an instance.
fn material_slot(i: &Instance) -> usize {
    let material = i.extra[0].round() as i32;
    MATERIALS
        .iter()
        .position(|&m| m == material)
        .unwrap_or(MATERIALS.len() - 1)
}

/// The pipeline for an instance: its material's, of the pair for things that
/// stand on the seabed or swim free.
fn pipeline_slot(i: &Instance) -> usize {
    material_slot(i) * 2 + usize::from(i.extra[1] > 0.5)
}

/// Which pipeline a run of instances is drawn with, in the order they are drawn.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Kind {
    /// Solid and fully there, and closed: its back faces are culled.
    Sealed,
    /// Solid and fully there.
    Solid,
    /// Solid, fading in or out by coverage.
    Fading,
    /// Translucent: jellyfish, bells, bubbles' shells.
    Glass,
}

/// One instance on its way to the GPU: which draw it joins, how far out it
/// is, and where it is in the frame's lists.
#[derive(Clone, Copy)]
struct Staged {
    kind: Kind,
    slot: u32,
    model: u32,
    lod: u32,
    depth: f32,
    index: u32,
}

/// One draw: a run of instances of one model at one level of detail, all
/// drawn by the same pipeline.
#[derive(Clone, Copy)]
struct Draw {
    kind: Kind,
    /// The pipeline's index, into [`Nautilus::models`] and its siblings.
    slot: usize,
    model: usize,
    lod: usize,
    first: u32,
    count: u32,
    /// How far out its nearest instance is.
    nearest: f32,
}

/// The parts of the scene `frame_cost` can leave out, as bits of `skip`.
#[cfg_attr(not(test), allow(dead_code))]
mod part {
    pub const SEA: u16 = 1;
    pub const TERRAIN: u16 = 2;
    pub const MODELS: u16 = 4;
    pub const GLASS: u16 = 8;
    pub const SPRITES: u16 = 16;
    pub const LIGHTS: u16 = 32;
    /// Not a part: draw into a quarter of the pixels, the geometry unchanged,
    /// to tell what the fragments cost from what the vertices do.
    pub const QUARTER: u16 = 64;
    /// Not parts either: every texture lookup a flat grey (only in a scene
    /// built [`Nautilus::with_plain_textures`]); no procedural noise;
    /// trilinear instead of anisotropic filtering.
    pub const PLAIN_TEXTURES: u16 = 128;
    pub const NO_NOISE: u16 = 256;
    pub const ANISOTROPY: u16 = 512;
    /// Every model at its full detail.
    pub const FULL_DETAIL: u16 = 1024;
    pub const SHAFTS: u16 = 2048;
    /// Not parts: the cheaper lighting/material paths the shader can switch
    /// to at run time, so a benchmark can price a feature without rebuilding.
    pub const CHEAP_MODELS: u16 = 4096;
    pub const CHEAP_TERRAIN: u16 = 8192;
    pub const CHEAP_BACKGROUND: u16 = 16384;
}

/// Shader switches for the benchmark, as `counts.y` bits (see `debug` in
/// `scene.wesl`), and the extra variant that changes the sampler.
#[cfg_attr(not(test), allow(dead_code))]
mod shader_switch {
    pub const NO_NOISE: u32 = 2;
    pub const CHEAP_MODELS: u32 = 4;
    pub const CHEAP_TERRAIN: u32 = 8;
    pub const CHEAP_BACKGROUND: u32 = 16;
}

impl Nautilus {
    pub(crate) fn new(gpu: &Graphics) -> Result<Self> {
        Self::with_colour(gpu, COLOUR)
    }

    /// The scene with its sea stored as `colour`: half floats, or anything
    /// that holds 0 to 1, at half brightness.
    fn with_colour(gpu: &Graphics, colour: wgpu::TextureFormat) -> Result<Self> {
        Self::build(gpu, colour, false)
    }

    /// The scene with every surface a flat grey, to see what texturing costs.
    #[cfg(test)]
    fn with_plain_textures(gpu: &Graphics) -> Result<Self> {
        Self::build(gpu, COLOUR, true)
    }

    fn build(gpu: &Graphics, colour: wgpu::TextureFormat, plain: bool) -> Result<Self> {
        let scale = if cfg!(feature = "full-hd") || colour == wgpu::TextureFormat::Rgba16Float {
            1.0
        } else {
            0.5
        };
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
        let ground = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Dive3d seabed"),
            size: (terrain::SLOTS * terrain::VERTICES * std::mem::size_of::<terrain::Point>())
                as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let ground_index = terrain::indices();
        let ground_indices = buffer(
            "Dive3d seabed indices",
            bytemuck::cast_slice(&ground_index),
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
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
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
        let caustic = caustic_texture(device, &gpu.queue);
        let caustic_view = caustic.create_view(&wgpu::TextureViewDescriptor::default());
        let caustic_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dive3d caustics"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let group_with = |sampler: &wgpu::Sampler| {
            device.create_bind_group(&wgpu::BindGroupDescriptor {
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
                        resource: wgpu::BindingResource::Sampler(sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 4,
                        resource: wgpu::BindingResource::TextureView(&caustic_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 5,
                        resource: wgpu::BindingResource::Sampler(&caustic_sampler),
                    },
                ],
            })
        };
        let group = group_with(&sampler);
        // The same with plain trilinear filtering, to measure what the
        // anisotropic filtering costs.
        #[cfg(test)]
        let trilinear = group_with(&device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Dive3d surfaces, trilinear"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            ..Default::default()
        }));

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
        let sea_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Dive3d sea"),
            size: wgpu::Extent3d {
                width: SEA,
                height: SEA,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: colour,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT
                | wgpu::TextureUsages::TEXTURE_BINDING
                | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let sea = sea_texture.create_view(&wgpu::TextureViewDescriptor::default());
        // Without multisampling the sea is drawn straight into the texture the
        // Slint samples; with it, into a multisampled target that resolves
        // into `sea`.
        let colour_target = if MULTISAMPLED {
            target(
                "Dive3d colour",
                colour,
                SAMPLES,
                wgpu::TextureUsages::RENDER_ATTACHMENT,
            )
        } else {
            sea.clone()
        };
        let depth = target(
            "Dive3d depth",
            DEPTH,
            SAMPLES,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        );
        let resolved = sea;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Dive3d shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!(concat!(env!("OUT_DIR"), "/scene.wgsl")).into(),
            ),
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
            10 => Float32x4, 11 => Float32x4,
        ];
        let terrain_attributes = wgpu::vertex_attr_array![
            0 => Float32x3, 1 => Float32x3, 2 => Float32x4, 3 => Float32x4, 4 => Float32x4
        ];
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
            array_stride: std::mem::size_of::<terrain::Point>() as u64,
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
                        coverage: bool,
                        write_alpha: bool,
                        constants: &[(&str, f64)],
                        cull: Option<wgpu::Face>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(label),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some(vs),
                    buffers,
                    // The material too, which the tunnel's vertices are read
                    // by (`vs_model`).
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants,
                        ..Default::default()
                    },
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(fs),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: colour,
                        blend,
                        // The sea is handed to Slint with its alpha channel:
                        // fading models blend their coverage as alpha, but
                        // must leave the channel opaque behind them.
                        write_mask: if write_alpha {
                            wgpu::ColorWrites::ALL
                        } else {
                            wgpu::ColorWrites::COLOR
                        },
                    })],
                    compilation_options: wgpu::PipelineCompilationOptions {
                        constants: &[
                            constants,
                            &[
                                ("SEA_SCALE", scale),
                                ("PLAIN_TEXTURES", f64::from(u8::from(plain))),
                            ],
                        ]
                        .concat(),
                        ..Default::default()
                    },
                }),
                // Fins, sails and fronds are single sheets: only sealed models
                // are culled. The view's basis mirrors the screen, so what
                // faces the camera turns clockwise on it.
                primitive: wgpu::PrimitiveState {
                    front_face: wgpu::FrontFace::Cw,
                    cull_mode: cull,
                    ..Default::default()
                },
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: DEPTH,
                    depth_write_enabled: Some(write_depth),
                    depth_compare: Some(compare),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: wgpu::MultisampleState {
                    count: SAMPLES,
                    // Fading models out by coverage while multisampled; at one
                    // sample the fragment shader blends them into the water.
                    alpha_to_coverage_enabled: coverage && MULTISAMPLED,
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
            // Only where the depth is still the clear value: nothing drawn.
            wgpu::CompareFunction::GreaterEqual,
            false,
            true,
            &[],
            None,
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
            true,
            &[],
            None,
        );
        // Models that are fully there, compiled without the discard so the GPU
        // can skip whatever is hidden before shading it; and those fading in or
        // out, which are blended over what is behind them, back to front.
        // Each in a pipeline per material and grounded flag.
        let per_material = |label,
                            coverage,
                            fading: f64,
                            blend: Option<wgpu::BlendState>,
                            write_depth: bool,
                            cull| {
            let mut pipelines = Vec::with_capacity(MATERIALS.len() * 2);
            for &material in &MATERIALS {
                for grounded in [0.0, 1.0] {
                    pipelines.push(pipeline(
                        label,
                        "vs_model",
                        "fs_model",
                        &model_buffers,
                        blend,
                        write_depth,
                        nearer,
                        coverage,
                        fading == 0.0,
                        &[
                            ("FADING", fading),
                            ("MATERIAL", f64::from(material)),
                            ("GROUNDED", grounded),
                        ],
                        cull,
                    ));
                }
            }
            pipelines
        };
        let blend = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::SrcAlpha,
                dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let models = per_material("Dive3d models", false, 0.0, None, true, None);
        let sealed = per_material(
            "Dive3d sealed models",
            false,
            0.0,
            None,
            true,
            Some(wgpu::Face::Back),
        );
        // Before anything else, the corners outside the round window are
        // marked as nearest of all, so nothing drawn there is shaded.
        let mask = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Dive3d corners"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_mask"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_mask"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: colour,
                    blend: None,
                    write_mask: wgpu::ColorWrites::empty(),
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(wgpu::DepthStencilState {
                format: DEPTH,
                depth_write_enabled: Some(true),
                depth_compare: Some(wgpu::CompareFunction::Always),
                stencil: Default::default(),
                bias: Default::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: SAMPLES,
                ..Default::default()
            },
            multiview_mask: None,
            cache: None,
        });
        // One sample has no alpha-to-coverage: blending the coverage over the
        // water is the dissolve. Multisampled, the coverage itself does it.
        let fading = per_material(
            "Dive3d fading models",
            true,
            1.0,
            (!MULTISAMPLED).then_some(blend),
            !MULTISAMPLED,
            None,
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
            true,
            &[],
            None,
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
            true,
            &[],
            None,
        );
        let shafts = pipeline(
            "Dive3d shafts",
            "vs_shaft",
            "fs_shaft",
            &sprite_buffers,
            Some(additive),
            false,
            nearer,
            false,
            true,
            &[],
            None,
        );
        let beams = pipeline(
            "Dive3d beams",
            "vs_beam",
            "fs_beam",
            &sprite_buffers,
            Some(additive),
            false,
            nearer,
            false,
            true,
            &[],
            None,
        );

        tracing::info!(
            models = library.lods.len(),
            vertices = library.vertices.len(),
            "3D dive ready"
        );
        Ok(Self {
            background,
            terrain_pipeline: terrain,
            models,
            sealed,
            mask,
            fading,
            glass,
            sprites,
            shafts,
            beams,
            globals,
            group,
            #[cfg(test)]
            trilinear,
            vertices,
            indices,
            lods: library.lods,
            closed: library.sealed,
            ground,
            ground_indices,
            ground_count: ground_index.len() as u32,
            terrain: Terrain::default(),
            tunnel,
            tunnel_indices,
            tunnel_uploaded: None,
            instances,
            order: Vec::new(),
            staged: Vec::new(),
            draws: Vec::new(),
            sprite_buffer,
            colour: colour_target,
            sea_texture,
            caustic,
            depth,
            resolved,
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
            #[cfg(test)]
            skip: 0,
        })
    }

    /// Whether a benchmark asked to leave `part` out. Never, outside the tests.
    #[cfg(test)]
    fn skips(&self, part: u16) -> bool {
        self.skip & part != 0
    }

    #[cfg(not(test))]
    fn skips(&self, _part: u16) -> bool {
        false
    }

    fn draw(&mut self, gpu: &Graphics, seconds: f64, visit: u64, _frame: usize) -> Result<()> {
        let seed = (visit % 1_000_003) as f32 / 1_000_003.0;
        if self.visit != Some(visit) {
            // A new visit: a new seabed, and the boat settles onto it afresh.
            self.seabed = Seabed::new(visit, seed);
            self.helm = Helm::default();
            self.terrain.clear();
            self.frame.forget_tunnel();
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
        let ground = &self.ground;
        let tiles = self.terrain.update(&world, |slot, points| {
            let at = slot as usize * terrain::VERTICES * std::mem::size_of::<terrain::Point>();
            gpu.queue
                .write_buffer(ground, at as u64, bytemuck::cast_slice(points));
        });
        let mut uniforms = globals(&world, &self.frame);
        if self.skips(part::LIGHTS) {
            uniforms.counts[0] = 0.0;
        }
        let mut switches = 0;
        if self.skips(part::NO_NOISE) {
            switches |= shader_switch::NO_NOISE;
        }
        if self.skips(part::CHEAP_MODELS) {
            switches |= shader_switch::CHEAP_MODELS;
        }
        if self.skips(part::CHEAP_TERRAIN) {
            switches |= shader_switch::CHEAP_TERRAIN;
        }
        if self.skips(part::CHEAP_BACKGROUND) {
            switches |= shader_switch::CHEAP_BACKGROUND;
        }
        uniforms.counts[1] = switches as f32;
        gpu.queue
            .write_buffer(&self.globals, 0, bytemuck::bytes_of(&uniforms));

        // The solid instances that are fully there, those fading in or out
        // (the ice always, as it fades by where it is), then the translucent
        // ones; each material's together, so its pipeline is bound once. The
        // solid ones go nearest first, within each draw and draw by draw, so
        // the depth test turns away what lies behind them before it is
        // shaded. The translucent ones keep the order they were placed in:
        // they add up, and in ten bits a channel even the order of adding
        // can show.
        let fades = |i: &Instance| i.extra[2] < 0.999 || i.extra[0] == world::material::ICE;
        // A mirrored instance turns its model inside out.
        let mirrored = |i: &Instance| {
            let [a, b, c] = i.rows.map(|r| [r[0], r[1], r[2]]);
            let det = a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0])
                + a[2] * (b[0] * c[1] - b[1] * c[0]);
            det < 0.0
        };
        let closed = &self.closed;
        let kind_of = |model: usize, i: &Instance| {
            if fades(i) {
                Kind::Fading
            } else if closed[model] && !mirrored(i) {
                Kind::Sealed
            } else {
                Kind::Solid
            }
        };
        let lods = &self.lods;
        let full = self.skips(part::FULL_DETAIL);
        let level_of = |model: usize, i: &Instance| {
            if full {
                0
            } else {
                level_of(&lods[model], i, &pose)
            }
        };
        let out = pose.basis[2];
        let depth = |i: &Instance| {
            (0..3)
                .map(|r| (i.rows[r][3] - pose.pos[r]) * out[r])
                .sum::<f32>()
        };
        let order = &mut self.order;
        order.clear();
        for (model, list) in self.frame.solid.iter().enumerate() {
            order.extend(list.iter().enumerate().map(|(index, i)| Staged {
                kind: kind_of(model, i),
                slot: pipeline_slot(i) as u32,
                model: model as u32,
                lod: level_of(model, i) as u32,
                depth: depth(i),
                index: index as u32,
            }));
        }
        // Glass has one shader for every material.
        for (model, list) in self.frame.glass.iter().enumerate() {
            order.extend(list.iter().enumerate().map(|(index, i)| Staged {
                kind: Kind::Glass,
                slot: 0,
                model: model as u32,
                lod: level_of(model, i) as u32,
                depth: 0.0,
                index: index as u32,
            }));
        }
        order.sort_unstable_by(|a, b| {
            let kind = a.kind.cmp(&b.kind);
            if kind != std::cmp::Ordering::Equal {
                return kind;
            }
            // Fading models blend back to front, so the farthest goes first;
            // solid models go nearest first so the depth test can reject what
            // lies behind them.
            if a.kind == Kind::Fading {
                b.depth
                    .total_cmp(&a.depth)
                    .then(a.slot.cmp(&b.slot))
                    .then(a.model.cmp(&b.model))
                    .then(a.lod.cmp(&b.lod))
                    .then(a.index.cmp(&b.index))
            } else {
                a.slot
                    .cmp(&b.slot)
                    .then(a.model.cmp(&b.model))
                    .then(a.lod.cmp(&b.lod))
                    .then(a.depth.total_cmp(&b.depth))
                    .then(a.index.cmp(&b.index))
            }
        });
        let all = &mut self.staged;
        let draws = &mut self.draws;
        all.clear();
        draws.clear();
        for s in order.iter().take(MAX_INSTANCES) {
            let lists = if s.kind == Kind::Glass {
                &self.frame.glass
            } else {
                &self.frame.solid
            };
            all.push(lists[s.model as usize][s.index as usize]);
            let (slot, model, lod) = (s.slot as usize, s.model as usize, s.lod as usize);
            match draws.last_mut() {
                Some(d) if (d.kind, d.slot, d.model, d.lod) == (s.kind, slot, model, lod) => {
                    d.count += 1
                }
                _ => draws.push(Draw {
                    kind: s.kind,
                    slot,
                    model,
                    lod,
                    first: all.len() as u32 - 1,
                    count: 1,
                    nearest: s.depth,
                }),
            }
        }
        // Stable, so the translucent draws stay in the order they came.
        draws.sort_by(|a, b| {
            let kind = a.kind.cmp(&b.kind);
            if kind != std::cmp::Ordering::Equal {
                return kind;
            }
            match a.kind {
                // The instances are already back to front; keep them so.
                Kind::Fading => b.nearest.total_cmp(&a.nearest),
                Kind::Glass => std::cmp::Ordering::Equal,
                _ => a.slot.cmp(&b.slot).then(a.nearest.total_cmp(&b.nearest)),
            }
        });
        // The tunnel is in world space already: one instance that moves nothing.
        let tunnel_count = self.frame.tunnel_indices.len().min(MAX_TUNNEL_INDICES) as u32;
        let tunnel_fits = tunnel_count > 0 && self.frame.tunnel.len() <= MAX_TUNNEL_VERTICES;
        let tunnel_drawn = tunnel_fits && all.len() < MAX_INSTANCES;
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
                floor: [0.0; 4],
            });
        }
        // It is only rebuilt when the boat has moved on by a ring of it.
        if tunnel_fits && self.tunnel_uploaded != Some(self.frame.tunnel_version) {
            gpu.queue
                .write_buffer(&self.tunnel, 0, bytemuck::cast_slice(&self.frame.tunnel));
            gpu.queue.write_buffer(
                &self.tunnel_indices,
                0,
                bytemuck::cast_slice(&self.frame.tunnel_indices[..tunnel_count as usize]),
            );
            self.tunnel_uploaded = Some(self.frame.tunnel_version);
        }
        let all = &self.staged;
        gpu.queue
            .write_buffer(&self.instances, 0, bytemuck::cast_slice(all));
        // The shafts of sunlight follow the sprites in the same buffer.
        let sprites = &self.frame.sprites[..self.frame.sprites.len().min(MAX_SPRITES)];
        let shafts = &self.frame.shafts[..self.frame.shafts.len().min(MAX_SPRITES - sprites.len())];
        gpu.queue
            .write_buffer(&self.sprite_buffer, 0, bytemuck::cast_slice(sprites));
        gpu.queue.write_buffer(
            &self.sprite_buffer,
            std::mem::size_of_val(sprites) as u64,
            bytemuck::cast_slice(shafts),
        );
        // The searchlight beams follow the shafts.
        let beams = &self.frame.beams[..self
            .frame
            .beams
            .len()
            .min(MAX_SPRITES - sprites.len() - shafts.len())];
        gpu.queue.write_buffer(
            &self.sprite_buffer,
            (std::mem::size_of_val(sprites) + std::mem::size_of_val(shafts)) as u64,
            bytemuck::cast_slice(beams),
        );

        if std::env::var_os("DIVE3D_STATS").is_some() {
            let tris: u64 = draws
                .iter()
                .map(|d| u64::from(lods[d.model][d.lod].count) * u64::from(d.count))
                .sum::<u64>()
                / 3;
            let mut by_lod = [0u64; mesh::LODS];
            for d in draws.iter() {
                by_lod[d.lod] +=
                    u64::from(lods[d.model][d.lod].count) / 3 * u64::from(d.count);
            }
            let verts: u64 = draws
                .iter()
                .map(|d| u64::from(lods[d.model][d.lod].vertices) * u64::from(d.count))
                .sum();
            eprintln!(
                "dive3d stats: {} draws, {} instances, {} model tris (lod {:?}), {} model verts, {} terrain tiles, {} sprites",
                draws.len(),
                all.len(),
                tris,
                by_lod,
                verts,
                tiles.len(),
                sprites.len() + shafts.len() + beams.len()
            );
        }

        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Dive3d encoder"),
            });
        let this = &*self;
        let result = gpu.profiled("Nautilus", &mut encoder, |encoder| -> Result<()> {
            if !this.skips(part::SEA) {
                let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("Dive3d sea pass"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: if MULTISAMPLED {
                            &this.colour
                        } else {
                            &this.resolved
                        },
                        depth_slice: None,
                        resolve_target: if MULTISAMPLED {
                            Some(&this.resolved)
                        } else {
                            None
                        },
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                            // Multisampled: only the resolved image is needed.
                            store: if MULTISAMPLED {
                                wgpu::StoreOp::Discard
                            } else {
                                wgpu::StoreOp::Store
                            },
                        },
                    })],
                    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                        view: &this.depth,
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
                #[cfg(test)]
                let group = if this.skips(part::ANISOTROPY) {
                    &this.trilinear
                } else {
                    &this.group
                };
                #[cfg(not(test))]
                let group = &this.group;
                pass.set_bind_group(0, group, &[]);
                if this.skips(part::QUARTER) {
                    let half = SEA as f32 / 2.0;
                    pass.set_viewport(0.0, 0.0, half, half, 0.0, 1.0);
                }

                pass.set_pipeline(&this.mask);
                pass.draw(0..MASK_VERTICES, 0..1);
                // What is fully there goes first, so the seabed behind it, and
                // every other model, can be skipped before it is shaded.
                pass.set_vertex_buffer(1, this.instances.slice(..));
                if tunnel_drawn && !this.skips(part::MODELS) {
                    pass.set_pipeline(&this.models[pipeline_slot(&all[tunnel_instance as usize])]);
                    pass.set_vertex_buffer(0, this.tunnel.slice(..));
                    pass.set_index_buffer(this.tunnel_indices.slice(..), wgpu::IndexFormat::Uint32);
                    pass.draw_indexed(0..tunnel_count, 0, tunnel_instance..tunnel_instance + 1);
                }
                let models = |pass: &mut wgpu::RenderPass, wanted: Kind| {
                    pass.set_vertex_buffer(0, this.vertices.slice(..));
                    pass.set_index_buffer(this.indices.slice(..), wgpu::IndexFormat::Uint32);
                    let mut bound = None;
                    for d in this.draws.iter().filter(|d| d.kind == wanted) {
                        if bound != Some(d.slot) {
                            pass.set_pipeline(match wanted {
                                Kind::Sealed => &this.sealed[d.slot],
                                Kind::Solid => &this.models[d.slot],
                                Kind::Fading => &this.fading[d.slot],
                                Kind::Glass => &this.glass,
                            });
                            bound = Some(d.slot);
                        }
                        let l = this.lods[d.model][d.lod];
                        pass.draw_indexed(
                            l.first..l.first + l.count,
                            l.base,
                            d.first..d.first + d.count,
                        );
                    }
                };
                if !this.skips(part::MODELS) {
                    models(&mut pass, Kind::Sealed);
                    models(&mut pass, Kind::Solid);
                }
                if !this.skips(part::TERRAIN) {
                    pass.set_pipeline(&this.terrain_pipeline);
                    pass.set_vertex_buffer(0, this.ground.slice(..));
                    pass.set_index_buffer(this.ground_indices.slice(..), wgpu::IndexFormat::Uint32);
                    for &slot in &tiles {
                        let base = slot as i32 * terrain::VERTICES as i32;
                        pass.draw_indexed(0..this.ground_count, base, 0..1);
                    }
                }
                // The open water, before the things that fade into it: it
                // shades only the pixels nothing solid has covered, and the
                // fading models blend over it.
                pass.set_pipeline(&this.background);
                pass.draw(0..3, 0..1);
                if !this.skips(part::MODELS) {
                    models(&mut pass, Kind::Fading);
                }
                if !this.skips(part::GLASS) {
                    models(&mut pass, Kind::Glass);
                }

                if !sprites.is_empty() && !this.skips(part::SPRITES) {
                    pass.set_pipeline(&this.sprites);
                    pass.set_vertex_buffer(0, this.sprite_buffer.slice(..));
                    pass.draw(0..6, 0..sprites.len() as u32);
                }
                if !shafts.is_empty() && !this.skips(part::SHAFTS) {
                    let first = sprites.len() as u32;
                    pass.set_pipeline(&this.shafts);
                    pass.set_vertex_buffer(0, this.sprite_buffer.slice(..));
                    pass.draw(0..6, first..first + shafts.len() as u32);
                }
                if !beams.is_empty() && !this.skips(part::SHAFTS) {
                    let first = (sprites.len() + shafts.len()) as u32;
                    pass.set_pipeline(&this.beams);
                    pass.set_vertex_buffer(0, this.sprite_buffer.slice(..));
                    pass.draw(0..6, first..first + beams.len() as u32);
                }
            }
            Ok(())
        });
        result?;
        gpu.submit(encoder.finish());
        Ok(())
    }

    #[cfg(test)]
    fn read_back(&mut self, gpu: &Graphics, seconds: f64, seed: u64) -> Vec<u8> {
        self.read_back_swiped(gpu, seconds, seed, None)
    }

    /// As [`Self::read_back`], with a swipe on to the next leg `swiped`
    /// seconds before the frame, if given.
    #[cfg(test)]
    fn read_back_swiped(
        &mut self,
        gpu: &Graphics,
        seconds: f64,
        seed: u64,
        swiped: Option<f64>,
    ) -> Vec<u8> {
        // A swipe leaves the voyage a leg on: start afresh before and after.
        if swiped.is_some() {
            self.voyage = Voyage::default();
        }
        // Sail the last twenty seconds first, so the helm has settled as it
        // would have on screen.
        self.draw(gpu, seconds - 20.0, seed, 0)
            .expect("a 3D dive frame");
        for i in 1..600 {
            let at = seconds - 20.0 + f64::from(i) / 30.0;
            if let Some(before) = swiped
                && at - 1.0 / 30.0 < seconds - before
                && at >= seconds - before
            {
                self.voyage.travel(1, at);
            }
            let voyage = self
                .voyage
                .advance(at, (seed % 1_000_003) as f32 / 1_000_003.0);
            self.helm.steer(&self.seabed, voyage, at);
        }
        self.draw(gpu, seconds, seed, 0).expect("a 3D dive frame");
        if swiped.is_some() {
            self.voyage = Voyage::default();
        }
        crate::gpu::read_texture(gpu, &self.sea_texture, SIZE)
    }

    /// A quick reference frame: a few draws to settle pipelines and the helm,
    /// then the composed frame. For the documentation sheet and visual
    /// checks, where the exact helm history of `read_back_swiped` is not
    /// needed.
    #[cfg(test)]
    fn read_back_fast(&mut self, gpu: &Graphics, seconds: f64, seed: u64) -> Vec<u8> {
        for (i, at) in [seconds - 3.0, seconds - 1.0, seconds]
            .into_iter()
            .enumerate()
        {
            self.draw(gpu, at, seed, i % crate::visuals::SLOTS)
                .expect("a 3D dive frame");
        }
        crate::gpu::read_texture(gpu, &self.sea_texture, SIZE)
    }
}

impl Visual for Nautilus {
    fn render(&mut self, gpu: &Graphics, seconds: f64, seed: u64, frame: usize) -> Result<()> {
        self.draw(gpu, seconds, seed, frame)
    }

    fn texture(&self, _frame: usize) -> wgpu::Texture {
        self.sea_texture.clone()
    }

    fn travel(&mut self, legs: i32) {
        self.voyage.travel(legs, self.seconds);
    }
}

/// The coarsest of a model's levels of detail that looks the same as the
/// full one for this instance, at its distance through the window.
fn level_of(lods: &[Lod], i: &Instance, pose: &Pose) -> usize {
    let d = [
        i.rows[0][3] - pose.pos[0],
        i.rows[1][3] - pose.pos[1],
        i.rows[2][3] - pose.pos[2],
    ];
    let out = pose.basis[2];
    let z = d[0] * out[0] + d[1] * out[1] + d[2] * out[2];
    // The longest of the instance's axes, as the model is stretched.
    let scale = (0..3)
        .map(|c| (0..3).map(|r| i.rows[r][c] * i.rows[r][c]).sum::<f32>())
        .fold(0.0, f32::max)
        .sqrt();
    // Pixels of the sea per unit of the model, its nearest point taken as
    // close as its centre less its own size.
    let near = z - scale * 1.5;
    if near <= NEAR {
        return 0;
    }
    let pixels = scale * (SEA as f32 * 0.5) / (world::VIEW * near);
    (1..lods.len())
        .rev()
        .find(|&l| {
            let lod = &lods[l];
            lod.error * pixels < LOD_ERROR
                && (lod.stray < LOD_STRAY || lod.edge * pixels < LOD_EDGE)
        })
        .unwrap_or(0)
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
    let blend = |values: [f32; crate::voyage::BIOMES]| {
        values[here.a] + (values[here.b] - values[here.a]) * here.w as f32
    };
    let tint = |c: [f32; 3]| {
        let c = mix3(c, [0.05, 0.22, 0.32], arctic * 0.75);
        mix3(c, [0.030, 0.012, 0.010], cave * 0.85)
    };
    let water = tint(water_tone(depth));
    let deep = tint(water_tone((depth + 0.30).min(1.1)));
    // Over the trench the water is clear, and daylight reaches its walls.
    let sun = (-depth * 4.3).exp() * (1.0 - 0.95 * cave) * (1.0 + 1.8 * share(world::TRENCH));
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
        let d = world::SUN;
        let l = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / l, d[1] / l, d[2] / l]
    };
    let cam = pose.pos;
    // Water takes red light first, then green, blue last: the deeper the
    // window, the bluer the daylight that reaches it.
    let below = (pose.depth as f32).max(0.0);
    let tint = [
        (-(below - 1.0) * 0.32).exp(),
        (-(below - 1.0) * 0.05).exp(),
        (-(below - 1.0) * 0.07).exp(),
    ];
    // The trench's water is clear, so its far wall shows across the gulf.
    let haze = blend([0.18, 0.22, 0.15, 0.18, 0.20, 0.22, 0.12, 0.11]);

    // The lights that matter most: nearest what the window looks at, a
    // little way out from it, weighed by their reach.
    let ahead = [
        cam[0] + out[0] * 1.5,
        cam[1] + out[1] * 1.5,
        cam[2] + out[2] * 1.5,
    ];
    let mut lights: Vec<_> = frame.lights.iter().collect();
    lights.sort_by(|a, b| {
        let score = |l: &&world::Light| {
            let d = [
                l.pos[0] - ahead[0],
                l.pos[1] - ahead[1],
                l.pos[2] - ahead[2],
            ];
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
        water: [water[0], water[1], water[2], haze],
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
        tint: v4(tint),
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

    /// Write a frame of every place in [`places::all`] to `DIVE3D_SHOTS_DIR`,
    /// with a contact sheet of them all and an `index.html` naming each.
    #[test]
    #[ignore = "writes debugging screenshots; set DIVE3D_SHOTS_DIR"]
    fn screenshots() {
        let Ok(dir) = std::env::var("DIVE3D_SHOTS_DIR") else {
            return;
        };
        let dir = std::path::Path::new(&dir);
        std::fs::create_dir_all(dir).expect("a directory for the screenshots");
        let only = std::env::var("DIVE3D_SHOTS").unwrap_or_default();
        let places: Vec<_> = places::all()
            .into_iter()
            .filter(|p| p.name.contains(only.as_str()))
            .collect();
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        const THUMB: u32 = 360;
        const ACROSS: u32 = 4;
        let rows = (places.len() as u32).div_ceil(ACROSS).max(1);
        let mut sheet = image::RgbaImage::new(THUMB * ACROSS, THUMB * rows);
        let mut html = String::from(
            "<!doctype html><meta charset=utf-8><title>Dive screenshots</title>\n\
             <style>body{background:#111;color:#ddd;font:14px sans-serif;display:flex;\
             flex-wrap:wrap;gap:12px}figure{margin:0}img{width:360px}</style>\n",
        );
        for (i, place) in places.iter().enumerate() {
            let pixels = scene.read_back_fast(&gpu, place.seconds(), place.seed);
            let frame = image::RgbaImage::from_raw(SIZE, SIZE, pixels).expect("a whole frame");
            let file = format!("{}.png", place.name);
            frame.save(dir.join(&file)).expect("a screenshot");
            let thumb = image::imageops::resize(
                &frame,
                THUMB,
                THUMB,
                image::imageops::FilterType::Triangle,
            );
            let (x, y) = (i as u32 % ACROSS * THUMB, i as u32 / ACROSS * THUMB);
            image::imageops::overlay(&mut sheet, &thumb, i64::from(x), i64::from(y));
            html += &format!(
                "<figure><a href=\"{file}\"><img src=\"{file}\"></a>\
                 <figcaption>{i}: {} (leg {:.2}, seed {})</figcaption></figure>\n",
                place.name, place.leg, place.seed
            );
            println!("{i:3}: {}", place.name);
        }
        sheet.save(dir.join("sheet.png")).expect("a contact sheet");
        std::fs::write(dir.join("index.html"), html).expect("an index");
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
            "shallows", "forest", "trench", "wrecks", "atlantis", "cave", "abyss", "arctic",
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
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_cost() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        println!("adapter: {}, profiler: {}", gpu.adapter, gpu.profiling());
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        // The same scene, its sea in half floats as it was before.
        let mut floats =
            Nautilus::with_colour(&gpu, wgpu::TextureFormat::Rgba16Float).expect("a 3D dive");
        // And with every surface a flat grey.
        let mut plain = Nautilus::with_plain_textures(&gpu).expect("a 3D dive");
        let variants = [
            ("everything", 0),
            ("no terrain", part::TERRAIN),
            ("no solid models", part::MODELS),
            ("no glass models", part::GLASS),
            ("no sprites", part::SPRITES),
            ("no sun shafts", part::SHAFTS),
            ("no point lights", part::LIGHTS),
            ("empty sea", part::SEA),
            ("quarter pixels", part::QUARTER),
            ("plain textures", part::PLAIN_TEXTURES),
            ("trilinear only", part::ANISOTROPY),
            ("no noise", part::NO_NOISE),
            ("full detail", part::FULL_DETAIL),
            ("cheap models", part::CHEAP_MODELS),
            ("cheap terrain", part::CHEAP_TERRAIN),
            ("cheap background", part::CHEAP_BACKGROUND),
            (
                "all cheap lights",
                part::CHEAP_MODELS | part::CHEAP_TERRAIN | part::CHEAP_BACKGROUND,
            ),
            ("half-float sea", 0),
        ];
        let legs = [1.0, 3.0, 4.0];
        let frames = 20;
        // Warm up, so the first variant does not pay for anything the rest skip.
        for scene in [&mut scene, &mut floats, &mut plain] {
            for i in 0..10 {
                scene
                    .draw(&gpu, leg(1.0) + f64::from(i), 0, 0)
                    .expect("a frame");
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            }
        }
        let mut everything = 0.0;
        for (name, skip) in variants {
            let scene = if name == "half-float sea" {
                &mut floats
            } else if skip == part::PLAIN_TEXTURES {
                &mut plain
            } else {
                &mut scene
            };
            scene.skip = skip;
            let mut drawn = 0.0;
            for &n in &legs {
                for i in 0..frames {
                    let seconds = leg(n) + i as f64 * 0.25;
                    let begun = std::time::Instant::now();
                    scene
                        .draw(&gpu, seconds, 0, i as usize % crate::visuals::SLOTS)
                        .expect("a frame");
                    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                    drawn += begun.elapsed().as_secs_f64();
                }
            }
            let mean = drawn / (legs.len() * frames as usize) as f64 * 1e3;
            if skip == 0 {
                everything = mean;
            }
            println!(
                "dive3d cost, {name:>16}: {mean:7.2} ms ({:+.2} ms)",
                mean - everything
            );
        }
    }

    /// How much of the panel each part of the frame actually covers: with the
    /// part left out, the difference is the pixels it was shading.
    #[test]
    #[ignore = "needs a GPU adapter"]
    fn coverage() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        let at = leg(1.0) + 2.0;
        let shot = |scene: &mut Nautilus, skip: u16| -> Vec<u8> {
            scene.skip = skip;
            for i in 0..8 {
                scene
                    .draw(&gpu, at - 0.8 + f64::from(i) * 0.1, 0, 0)
                    .expect("a frame");
            }
            scene.draw(&gpu, at, 0, 0).expect("a frame");
            crate::gpu::read_texture(&gpu, &scene.sea_texture, SIZE)
        };
        let all = shot(&mut scene, 0);
        println!("coverage of {} pixels", SIZE * SIZE);
        for (name, skip) in [
            ("terrain", part::TERRAIN),
            ("models", part::MODELS),
            ("glass", part::GLASS),
            ("sprites", part::SPRITES),
            ("shafts", part::SHAFTS),
        ] {
            let without = shot(&mut scene, skip);
            let differing = all
                .chunks_exact(4)
                .zip(without.chunks_exact(4))
                .filter(|(a, b)| a != b)
                .count();
            println!(
                "coverage {name:>10}: {differing:>7} px ({:.1}%)",
                differing as f64 * 100.0 / f64::from(SIZE * SIZE)
            );
        }
        scene.skip = 0;
    }

    /// Time the scene per leg, on whatever adapter is present.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn frame_time() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        println!("adapter: {}, profiler: {}", gpu.adapter, gpu.profiling());
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        // Same binary, different feature set: useful without rebuilding.
        if let Ok(skip) = std::env::var("DIVE3D_SKIP") {
            scene.skip = skip.parse().unwrap_or(0);
        }
        scene.draw(&gpu, 0.0, 0, 0).expect("a frame");
        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        let names = [
            "shallows", "forest", "trench", "wrecks", "atlantis", "cave", "abyss", "arctic",
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
    /// Time specific seeds of every scene at several places inside each leg:
    /// the regression bench for hotspots, not just the leg average.
    ///
    /// `DIVE3D_HOTSPOT_SEEDS=0,250000,500000,750000` chooses visits (the
    /// voyage seed is the visit modulo 1,000,003); `DIVE3D_HOTSPOT_PLACES`
    /// chooses fractions through each leg; `DIVE3D_HOTSPOT_FRAMES` sets the
    /// samples. Writes CSV to `DIVE3D_HOTSPOT_DIR` when set.
    #[test]
    #[ignore = "a benchmark; needs a working GPU adapter"]
    fn hotspots() {
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        println!("adapter: {}, profiler: {}", gpu.adapter, gpu.profiling());
        let list = |name: &str, default: &[f64]| -> Vec<f64> {
            std::env::var(name).map_or_else(
                |_| default.to_vec(),
                |text| {
                    text.split(',')
                        .filter_map(|v| v.trim().parse().ok())
                        .collect()
                },
            )
        };
        let seeds: Vec<u64> = list(
            "DIVE3D_HOTSPOT_SEEDS",
            &[0.0, 250_000.0, 500_000.0, 750_000.0],
        )
        .into_iter()
        .map(|v| v as u64)
        .collect();
        let places = list("DIVE3D_HOTSPOT_PLACES", &[0.15, 0.45, 0.85]);
        let frames = std::env::var("DIVE3D_HOTSPOT_FRAMES")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(6usize);
        let names = [
            "shallows", "forest", "trench", "wrecks", "atlantis", "cave", "abyss", "arctic",
        ];
        let span = crate::voyage::LEG / crate::voyage::SPEED;
        let mut rows = Vec::new();
        for &visit in &seeds {
            let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
            // Build the seabed and warm its pipelines before timing it.
            for i in 0..4 {
                scene
                    .draw(&gpu, leg(0.0) + f64::from(i) * 0.1, visit, 0)
                    .expect("a warm frame");
                gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
            }
            for (n, name) in names.iter().enumerate() {
                for &place in &places {
                    let seconds = leg(n as f64) + place.clamp(0.0, 0.99) * span;
                    for i in 0..3 {
                        scene
                            .draw(&gpu, seconds + f64::from(i) * 0.02, visit, i as usize)
                            .expect("a warm frame");
                        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                    }
                    let mut total = 0.0;
                    let mut worst: f64 = 0.0;
                    for i in 0..frames {
                        let begun = std::time::Instant::now();
                        scene
                            .draw(
                                &gpu,
                                seconds + i as f64 * 0.02,
                                visit,
                                i % crate::visuals::SLOTS,
                            )
                            .expect("a frame");
                        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                        let ms = begun.elapsed().as_secs_f64() * 1e3;
                        total += ms;
                        worst = worst.max(ms);
                    }
                    let count: usize = scene.frame.solid.iter().map(Vec::len).sum::<usize>()
                        + scene.frame.glass.iter().map(Vec::len).sum::<usize>();
                    rows.push((
                        visit,
                        n,
                        name.to_string(),
                        place,
                        total / frames as f64,
                        worst,
                        count,
                        scene.frame.sprites.len(),
                    ));
                }
            }
        }
        rows.sort_by(|a, b| b.4.total_cmp(&a.4));
        println!(
            "{:>8}  {:<9} {:>5} {:>8} {:>8} {:>7} {:>7}",
            "seed", "scene", "place", "mean_ms", "worst_ms", "inst", "sprite"
        );
        for r in rows.iter().take(20) {
            println!(
                "{:>8}  {:<9} {:>5.2} {:>8.2} {:>8.2} {:>7} {:>7}",
                r.0, r.2, r.3, r.4, r.5, r.6, r.7
            );
        }
        println!("{} hotspots measured", rows.len());
        if let Ok(dir) = std::env::var("DIVE3D_HOTSPOT_DIR") {
            let mut csv = String::from("seed,scene,place,mean_ms,worst_ms,instances,sprites\n");
            for r in &rows {
                csv.push_str(&format!(
                    "{},{},{:.2},{:.3},{:.3},{},{}\n",
                    r.0, r.2, r.3, r.4, r.5, r.6, r.7
                ));
            }
            std::fs::write(std::path::Path::new(&dir).join("hotspots.csv"), csv)
                .expect("a hotspot CSV");
        }
    }

    /// One tiled reference image of every place in `places::all()` — every
    /// scene, every moment of the voyage, and every effect we shoot — for the
    /// docs and for comparing a visual change at a glance. Regenerate with
    /// `DIVE3D_SHEET=docs/dive3d-scenes.png cargo test --release
    /// dive3d::tests::scenes_sheet -- --ignored`.
    #[test]
    #[ignore = "writes the documentation contact sheet; set DIVE3D_SHEET"]
    fn scenes_sheet() {
        let Ok(path) = std::env::var("DIVE3D_SHEET") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        let places = places::all();
        let tile: u32 = std::env::var("DIVE3D_SHEET_TILE")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(280);
        let across: u32 = std::env::var("DIVE3D_SHEET_COLS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(7);
        let rows = (places.len() as u32).div_ceil(across);
        let mut sheet = image::RgbaImage::new(tile * across, tile * rows);
        for (i, place) in places.iter().enumerate() {
            let pixels = scene.read_back_swiped(&gpu, place.seconds(), place.seed, place.swiped);
            let frame = image::RgbaImage::from_raw(SIZE, SIZE, pixels).expect("a whole frame");
            let thumb =
                image::imageops::resize(&frame, tile, tile, image::imageops::FilterType::Triangle);
            let (x, y) = (i as u32 % across * tile, i as u32 / across * tile);
            image::imageops::overlay(&mut sheet, &thumb, i64::from(x), i64::from(y));
            println!("{i:3}: {}", place.name);
        }
        let path = std::path::Path::new(&path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("a directory for the sheet");
        }
        sheet.save(path).expect("the documentation sheet");
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("dive3d-scenes.png");
        let mut html = format!(
            "<!doctype html><meta charset=utf-8><title>Dive3D scenes</title>\n\
             <style>body{{background:#111;color:#ddd;font:13px system-ui,sans-serif;margin:24px}}\
             h1{{font-size:18px}} .grid{{display:grid;grid-template-columns:repeat({across},280px);gap:10px}}\
             figure{{margin:0}} .tile{{width:280px;height:280px;background-image:url({file});\
             background-repeat:no-repeat}} figcaption{{padding:4px 0;color:#bbb}}\
             code{{color:#9cf}}</style>\n<h1>Nautilus: every scene and effect</h1>\n\
             <p>One sheet, regenerated by <code>DIVE3D_SHEET=docs/dive3d-scenes.png cargo test --release dive3d::tests::scenes_sheet -- --ignored</code>.</p>\n<div class=grid>\n"
        );
        for (i, place) in places.iter().enumerate() {
            let (x, y) = (i as u32 % across * tile, i as u32 / across * tile);
            html += &format!(
                "<figure><div class=tile style=\"background-position:-{x}px -{y}px\"></div>\
                 <figcaption>{i}: {name}</figcaption></figure>\n",
                name = place.name
            );
        }
        html += "</div>\n";
        std::fs::write(path.with_extension("html"), html).expect("the scene index");
        println!(
            "wrote {} ({} x {})",
            path.display(),
            sheet.width(),
            sheet.height()
        );
    }

    /// The same three demanding scenes under each lighting path, tiled for a
    /// visual comparison: rich, cheap models, cheap terrain, both, triplanar.
    /// Regenerate with `DIVE3D_QUALITY_SHEET=docs/dive3d-lighting.png ...`.
    #[test]
    #[ignore = "writes a lighting comparison sheet; set DIVE3D_QUALITY_SHEET"]
    fn quality_sheet() {
        let Ok(path) = std::env::var("DIVE3D_QUALITY_SHEET") else {
            return;
        };
        let gpu = crate::gpu::headless().expect("a GPU adapter");
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        let places = [
            (
                "shallows",
                places::all()
                    .into_iter()
                    .find(|p| p.name == "0-shallows-middle")
                    .unwrap(),
            ),
            (
                "forest",
                places::all()
                    .into_iter()
                    .find(|p| p.name == "1-forest-middle")
                    .unwrap(),
            ),
            (
                "atlantis",
                places::all()
                    .into_iter()
                    .find(|p| p.name == "4-atlantis-middle")
                    .unwrap(),
            ),
        ];
        let variants: [(&str, u16); 4] = [
            ("rich", 0),
            ("cheap model light", part::CHEAP_MODELS),
            ("cheap terrain light", part::CHEAP_TERRAIN),
            ("both cheap", part::CHEAP_MODELS | part::CHEAP_TERRAIN),
        ];
        let tile = 360;
        let across = places.len() as u32;
        let mut sheet = image::RgbaImage::new(tile * across, tile * variants.len() as u32);
        for (row, (name, skip)) in variants.iter().enumerate() {
            scene.skip = *skip;
            for (col, (_, place)) in places.iter().enumerate() {
                let pixels = scene.read_back_fast(&gpu, place.seconds(), place.seed);
                let frame = image::RgbaImage::from_raw(SIZE, SIZE, pixels).expect("a frame");
                let thumb = image::imageops::resize(
                    &frame,
                    tile,
                    tile,
                    image::imageops::FilterType::Triangle,
                );
                image::imageops::overlay(
                    &mut sheet,
                    &thumb,
                    i64::from(col as u32 * tile),
                    i64::from(row as u32 * tile),
                );
                println!("{name}: {}", place.name);
            }
        }
        scene.skip = 0;
        let path = std::path::Path::new(&path);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).expect("a directory for the sheet");
        }
        sheet.save(path).expect("the comparison sheet");
        let file = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("dive3d-lighting.png");
        let mut html = format!(
            "<!doctype html><meta charset=utf-8><title>Dive3D lighting</title>\n\
             <style>body{{background:#111;color:#ddd;font:13px system-ui,sans-serif;margin:24px}}\
             table{{border-collapse:collapse}} td{{padding:4px;vertical-align:top}}\
             .tile{{width:360px;height:360px;background-image:url({file});background-repeat:no-repeat}}\
             th{{text-align:left;padding:4px}} code{{color:#9cf}}</style>\n\
             <h1>Nautilus lighting paths</h1>\n<p>Rich, then each cheaper path, over the same three places.\
             Regenerate with <code>DIVE3D_QUALITY_SHEET=docs/dive3d-lighting.png cargo test --release dive3d::tests::quality_sheet -- --ignored</code>.</p>\n<table><tr><th></th>"
        );
        for (name, _) in &places {
            html += &format!("<th>{name}</th>");
        }
        html += "</tr>\n";
        for (row, (name, _)) in variants.iter().enumerate() {
            html += &format!("<tr><th>{name}</th>");
            for col in 0..places.len() {
                let (x, y) = (col as u32 * tile, row as u32 * tile);
                html += &format!(
                    "<td><div class=tile style=\"background-position:-{x}px -{y}px\"></div></td>"
                );
            }
            html += "</tr>\n";
        }
        html += "</table>\n";
        std::fs::write(path.with_extension("html"), html).expect("the lighting index");
        println!(
            "wrote {} ({} x {})",
            path.display(),
            sheet.width(),
            sheet.height()
        );
    }
    /// Price the work moved to (or proposed for) the CPU, without a GPU:
    /// caustic generation, a visit's seabed and the per-frame scene build.
    #[test]
    #[ignore = "a CPU benchmark; no GPU needed"]
    fn cpu_cost() {
        let begun = std::time::Instant::now();
        for _ in 0..100 {
            std::hint::black_box(caustic_texels());
        }
        println!(
            "cpu caustic tile (256x256): {:.3} ms",
            begun.elapsed().as_secs_f64() * 1e3 / 100.0
        );
        let begun = std::time::Instant::now();
        for visit in 0..20 {
            std::hint::black_box(Seabed::new(visit, visit as f32 / 20.0));
        }
        println!(
            "cpu new seabed: {:.3} ms",
            begun.elapsed().as_secs_f64() * 1e3 / 20.0
        );
        let seabed = Seabed::new(0, 0.0);
        let mut voyage = Voyage::default();
        let mut helm = Helm::default();
        let mut frame = Frame::new();
        let begun = std::time::Instant::now();
        for n in 0..8 {
            for i in 0..60 {
                let seconds = leg(n as f64) + f64::from(i) * 0.25;
                let at = voyage.advance(seconds, 0.0);
                let pose = helm.steer(&seabed, at, seconds);
                let world = World {
                    seabed: &seabed,
                    pose,
                    t: seconds as f32,
                };
                world::build(&world, &mut frame);
            }
        }
        println!(
            "cpu scene build: {:.3} ms",
            begun.elapsed().as_secs_f64() * 1e3 / (8.0 * 60.0)
        );
        // Building the seabed's tiles: each vertex now carries its own shading,
        // so this is what the GPU no longer has to work out per frame.
        let mut terrain = Terrain::default();
        let mut vertices = 0usize;
        let seconds = leg(1.0);
        let at = voyage.advance(seconds, 0.0);
        let pose = helm.steer(&seabed, at, seconds);
        let world = World {
            seabed: &seabed,
            pose,
            t: seconds as f32,
        };
        let begun = std::time::Instant::now();
        for _ in 0..10 {
            terrain.clear();
            let _ = terrain.update(&world, |_, points| vertices += points.len());
        }
        println!(
            "cpu seabed tiles: {:.3} ms a visit ({} vertices)",
            begun.elapsed().as_secs_f64() * 1e3 / 10.0,
            vertices / 10
        );
        // What a per-instance light pass would cost: the thing the GPU's
        // per-vertex point-light loop currently does for every model vertex.
        let instances: usize = frame.solid.iter().map(Vec::len).sum::<usize>()
            + frame.glass.iter().map(Vec::len).sum::<usize>();
        let lights = &frame.lights;
        let begun = std::time::Instant::now();
        for _ in 0..100 {
            let mut sum = 0.0f32;
            for list in frame.solid.iter().chain(frame.glass.iter()) {
                for instance in list {
                    let p = [
                        instance.rows[0][3],
                        instance.rows[1][3],
                        instance.rows[2][3],
                    ];
                    for light in lights.iter().take(4) {
                        let d = [
                            light.pos[0] - p[0],
                            light.pos[1] - p[1],
                            light.pos[2] - p[2],
                        ];
                        sum += light.colour[0]
                            / (1.0
                                + (d[0] * d[0] + d[1] * d[1] + d[2] * d[2])
                                    / (light.range * light.range));
                    }
                }
            }
            std::hint::black_box(sum);
        }
        println!(
            "cpu per-instance light ({instances} instances): {:.3} ms",
            begun.elapsed().as_secs_f64() * 1e3 / 100.0
        );
    }
}
