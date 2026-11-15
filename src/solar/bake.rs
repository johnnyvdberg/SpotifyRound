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

/// Texels along each side of a rock shape's crater map (`CRATER_MAP` in the
/// shader).
pub(super) const CRATER_MAP: u32 = 512;
/// Texels along each side of a face of the gas map (`bake_gas`), as
/// `GAS_MAP` in `encounter.wesl`.
pub(super) const GAS_MAP: u32 = 512;
/// Texels along each side of the storm's map (`bake_storm`), as `STORM_MAP`
/// in `encounter.wesl`.
pub(super) const STORM_MAP: u32 = 512;

/// Bake each rock shape's small craters once, a layer a shape, over its
/// directions octahedrally (`fs_bake_craters`): their height and slope, with
/// the slope of the grain's middle octave, read by the rocks' shader in one
/// filtered fetch where it worked out eight cells and a noise octave a
/// pixel. (Each shape's craters lie where they lie on every rock of it;
/// they were drawn for each rock's size before.)
pub(super) fn bake_craters(
    gpu: &Graphics,
    shader: &wgpu::ShaderModule,
    noise: &crate::noise3d::Noise3d,
) -> wgpu::Texture {
    let label = "Solar rock craters";
    bake(
        gpu,
        shader,
        noise,
        label,
        "fs_bake_craters",
        CRATER_MAP,
        models::ROCKS,
    )
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
    let device = &gpu.device;
    let format = wgpu::TextureFormat::Rgba16Float;
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some(label),
        size: wgpu::Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: layers,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::RENDER_ATTACHMENT,
        view_formats: &[],
    });
    // It reads the noise volume alone.
    let layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some(label),
        bind_group_layouts: &[None, Some(&noise.layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some(label),
        layout: Some(&layout),
        vertex: wgpu::VertexState {
            module: shader,
            entry_point: Some("vs_bake_craters"),
            buffers: &[],
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: shader,
            entry_point: Some(fs),
            targets: &[Some(format.into())],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: gpu.pipeline_cache(),
    });
    let mut encoder = device.create_command_encoder(&Default::default());
    for layer in 0..layers {
        let view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2),
            base_array_layer: layer,
            array_layer_count: Some(1),
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
        pass.set_pipeline(&pipeline);
        pass.set_bind_group(1, &noise.group, &[]);
        pass.draw(0..3, layer..layer + 1);
    }
    gpu.submit(encoder.finish());
    texture
}
