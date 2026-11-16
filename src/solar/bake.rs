//! What the frame bakes once rather than every frame: the rings' density
//! profiles, the rocks' crater maps and the gas of the nebula and the plasma
//! storm, each from the noise volume alone.

use super::*;

/// Texels along a ring profile, and rows (one per planet).
pub(super) const RING_TEXELS: u32 = 1024;
pub(super) const RING_ROWS: u32 = 8;

/// Smooth 1D value noise in [0, 1], for the ring profiles.
pub(super) fn noise1(x: f64, seed: u64) -> f64 {
    let hash = |i: i64| {
        let mut z = (i as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ seed;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        ((z ^ (z >> 31)) >> 11) as f64 / (1u64 << 53) as f64
    };
    let i = x.floor();
    let f = x - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    hash(i as i64) * (1.0 - u) + hash(i as i64 + 1) * u
}

/// Paint the ring density profiles of `system`'s ringed planets, one row
/// each: ringlets, gaps and a main division.
pub(super) fn paint_rings(gpu: &Graphics, texture: &wgpu::Texture, system: &System) {
    let mut data = vec![0u8; (RING_TEXELS * RING_ROWS) as usize];
    for (row, planet) in system.planets.iter().enumerate().take(RING_ROWS as usize) {
        let Some(rings) = &planet.rings else {
            continue;
        };
        let seed = rings.seed.to_bits() as u64;
        for t in 0..RING_TEXELS as usize {
            let x = (t as f64 + 0.5) / RING_TEXELS as f64;
            let n1 = noise1(x * 40.0, seed);
            let n2 = noise1(x * 160.0, seed ^ 1);
            let n3 = noise1(x * 9.0, seed ^ 2);
            let division = 1.0 - (-((x - 0.62) / 0.025).powi(2)).exp() * 0.9;
            let edges = smoothstep(0.0, 0.04, x) * smoothstep(1.0, 0.9, x);
            let density = ((0.25 + 0.75 * n3)
                * (0.45 + 0.55 * n1)
                * (0.6 + 0.4 * n2)
                * division
                * edges
                * 1.4)
                .clamp(0.0, 1.0);
            data[row * RING_TEXELS as usize + t] = (density * 255.0).round() as u8;
        }
    }
    gpu.queue.write_texture(
        texture.as_image_copy(),
        &data,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(RING_TEXELS),
            rows_per_image: Some(RING_ROWS),
        },
        wgpu::Extent3d {
            width: RING_TEXELS,
            height: RING_ROWS,
            depth_or_array_layers: 1,
        },
    );
}

/// Texels along each side of a face of the gas map (`bake_gas`), as
/// `GAS_MAP` in `encounter.wesl`.
pub(super) const GAS_MAP: u32 = 512;
/// Texels along each side of the storm's map (`bake_storm`), as `STORM_MAP`
/// in `encounter.wesl`.
pub(super) const STORM_MAP: u32 = 512;

/// The rock shapes' surface and horizon maps on the GPU (`rocks::Maps`,
/// `rock_maps` in the shader), and their group; a texel each until `maps`.
pub(super) fn rock_maps(
    gpu: &Graphics,
    layout: &wgpu::BindGroupLayout,
    maps: Option<&rocks::Maps>,
) -> (wgpu::Texture, wgpu::BindGroup) {
    let device = &gpu.device;
    let (size, mips) = match maps {
        Some(m) => (rocks::MAP as u32, m.levels.len() as u32),
        None => (1, 1),
    };
    let layers = 2 * models::ROCKS;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Solar rock maps"),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let flat = vec![128u8; 4 * layers as usize];
    let levels: Vec<&[u8]> = match maps {
        Some(m) => m.levels.iter().map(|l| l.as_slice()).collect(),
        None => vec![&flat],
    };
    for (mip, data) in levels.iter().enumerate() {
        let side = (size >> mip).max(1);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: mip as u32,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(side * 4),
                rows_per_image: Some(side),
            },
            wgpu::Extent3d {
                width: side,
                height: side,
                depth_or_array_layers: layers,
            },
        );
    }
    let view = texture.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Solar rock maps"),
        layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::TextureView(&view),
        }],
    });
    (texture, group)
}

/// The gas the nebula and the plasma storm are drawn from, baked once over
/// every direction, a cube map's six faces (`fs_bake_gas`): read through a
/// cube, its filtering has no seams.
pub(super) fn bake_gas(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    noise: &crate::noise3d::Noise3d,
) -> wgpu::Texture {
    bake(gpu, shader, noise, "Solar gas", "fs_bake_gas", GAS_MAP, 6)
}

/// The plasma storm's disc seen from above, baked once on the face toward
/// +z of a cube map (`fs_bake_storm`): read through the same binding as the
/// nebula's gas (a second would be a texture over the V3D's limit).
pub(super) fn bake_storm(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    noise: &crate::noise3d::Noise3d,
) -> wgpu::Texture {
    bake(
        gpu,
        shader,
        noise,
        "Solar storm",
        "fs_bake_storm",
        STORM_MAP,
        6,
    )
}

/// Texels across each of a starbase's maps (`bake_city`), and down it,
/// as `CITY_WIDTH` and `CITY_HEIGHT` in `starbase/world.wesl`.
pub(super) const CITY_WIDTH: u32 = 2048;
pub(super) const CITY_HEIGHT: u32 = 1024;
/// Its layers: the ground inside, the tile of blocks, the clouds inside,
/// the megastructure's dish, the ground's relief, and the metals
/// (`write_metals`).
const CITY_LAYERS: u32 = 6;

/// A starbase's maps, baked once (`fs_bake_city`, `starbase/world.wesl`):
/// the ground of the hollow world inside it, all the way round and tiling
/// along it, and the clouds over that; the city planet's surface round its
/// mouth; and a tile of blocks laid over both. With every mip level.
/// `fs_pane` reads the world's where the view's ray meets the cylinder,
/// each base from its own place along it; `fs_plating` the surface's.
///
/// Its cities come from a table worked out once (`starbase::city`), read
/// from a buffer of the bake's own (group 3: the frame's own groups are
/// not bound here).
pub(super) fn bake_city(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    noise: &crate::noise3d::Noise3d,
) -> wgpu::Texture {
    use wgpu::util::DeviceExt;
    let device = &gpu.device;
    let table = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        label: Some("Solar city table"),
        contents: &super::starbase::city::city().table(),
        usage: wgpu::BufferUsages::STORAGE,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("Solar city table"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        }],
    });
    let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("Solar city table"),
        layout: &layout,
        entries: &[wgpu::BindGroupEntry {
            binding: 0,
            resource: table.as_entire_binding(),
        }],
    });
    bake_sized(
        gpu,
        shader,
        noise,
        "Solar city",
        "fs_bake_city",
        (CITY_WIDTH, CITY_HEIGHT, CITY_LAYERS),
        wgpu::TextureFormat::Rgba8UnormSrgb,
        CITY_HEIGHT.ilog2() + 1,
        Some("CITY_LAYER"),
        Some((&layout, &group)),
    )
}

/// A map baked once from the noise volume alone: `layers` square layers of
/// `size`, `fs` drawn over each (`vs_bake_craters`).
pub(super) fn bake(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    noise: &crate::noise3d::Noise3d,
    label: &str,
    fs: &str,
    size: u32,
    layers: u32,
) -> wgpu::Texture {
    let format = wgpu::TextureFormat::Rgba16Float;
    bake_sized(
        gpu,
        shader,
        noise,
        label,
        fs,
        (size, size, layers),
        format,
        1,
        None,
        None,
    )
}

/// A map baked once: `layers` layers `width` by `height`, of `format`,
/// with `mips` levels, each drawn by `fs`. Its instance is the layer; or,
/// given `layered` (the name of an override the shader takes its layer
/// from), a pipeline is made a layer and the instance is the mip level.
#[allow(clippy::too_many_arguments)]
fn bake_sized(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    noise: &crate::noise3d::Noise3d,
    label: &str,
    fs: &str,
    (width, height, layers): (u32, u32, u32),
    format: wgpu::TextureFormat,
    mips: u32,
    layered: Option<&str>,
    table: Option<(&wgpu::BindGroupLayout, &wgpu::BindGroup)>,
) -> wgpu::Texture {
    let device = &gpu.device;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: layers,
        },
        mip_level_count: mips,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    // It reads the noise volume alone.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[None, Some(&noise.layout), None, table.map(|t| t.0)],
        immediate_size: 0,
    });
    let pipeline = |layer: u32| {
        let constants = layered.map(|name| [(name, layer as f64)]);
        let options = wgpu::PipelineCompilationOptions {
            constants: constants.as_ref().map_or(&[], |c| &c[..]),
            ..Default::default()
        };
        device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some(label),
            layout: Some(&layout),
            vertex: wgpu::VertexState {
                module: shader,
                entry_point: Some("vs_bake_craters"),
                buffers: &[],
                compilation_options: options.clone(),
            },
            fragment: Some(wgpu::FragmentState {
                module: shader,
                entry_point: Some(fs),
                targets: &[Some(format.into())],
                compilation_options: options,
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: gpu.pipeline_cache(),
        })
    };
    let pipelines: Vec<_> = match layered {
        Some(_) => (0..layers).map(pipeline).collect(),
        None => vec![pipeline(0)],
    };
    let mut encoder = device.create_command_encoder(&Default::default());
    for (layer, mip) in (0..layers).flat_map(|l| (0..mips).map(move |m| (l, m))) {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
            base_mip_level: mip,
            mip_level_count: Some(1),
            ..Default::default()
        });
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some(label),
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
        pass.set_pipeline(&pipelines[if layered.is_some() { layer as usize } else { 0 }]);
        pass.set_bind_group(1, &noise.group, &[]);
        if let Some((_, group)) = table {
            pass.set_bind_group(3, group, &[]);
        }
        let instance = if layered.is_some() { mip } else { layer };
        pass.draw(0..3, instance..instance + 1);
    }
    gpu.submit(encoder.finish());
    texture
}

/// The metals a starbase's structures are built of, written into the last
/// layer of its maps (`METAL_LAYER` in `starbase/world.wesl`: the frame has
/// no texture binding to spare for a texture of their own, 16 a stage on
/// the Pi). Three tiling photographs from ambientCG (CC0,
/// `assets/textures/CREDITS.md`), each near enough grey to keep as a shade
/// alone: plates in red, the riveted grid in green, diamond tread in blue,
/// each tiled to fill the layer, with every mip level.
pub(super) fn write_metals(gpu: &Graphics, maps: &wgpu::Texture) {
    const FILES: [&[u8]; 3] = [
        include_bytes!("../../assets/textures/metal-plates-008.jpg"),
        include_bytes!("../../assets/textures/metal-plates-004.jpg"),
        include_bytes!("../../assets/textures/diamond-plate-008c.jpg"),
    ];
    let shades = FILES.map(|bytes| {
        image::load_from_memory(bytes)
            .expect("a metal's photograph")
            .to_luma8()
    });
    let full = image::RgbaImage::from_fn(CITY_WIDTH, CITY_HEIGHT, |x, y| {
        let at = |k: usize| {
            let shade = &shades[k];
            shade.get_pixel(x % shade.width(), y % shade.height()).0[0]
        };
        image::Rgba([at(0), at(1), at(2), 255])
    });
    for mip in 0..maps.mip_level_count() {
        let (width, height) = ((CITY_WIDTH >> mip).max(1), (CITY_HEIGHT >> mip).max(1));
        let level =
            image::imageops::resize(&full, width, height, image::imageops::FilterType::Triangle);
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: maps,
                mip_level: mip,
                origin: wgpu::Origin3d {
                    x: 0,
                    y: 0,
                    z: CITY_LAYERS - 1,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &level,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(4 * width),
                rows_per_image: Some(height),
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
    }
}
