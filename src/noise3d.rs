//! A small tileable volume of random values that the planet and solar
//! shaders read their smooth noise from.
//!
//! Value noise worked out in the shader hashes the eight corners of its
//! lattice cell, and on the Pi's V3D those hashes and their blends keep
//! enough values live to halve the threads every noisy shader runs with.
//! Read from a texture instead, one filtered fetch does the blending: the
//! fraction is remapped with the quintic fade first, so the hardware's linear
//! filter gives the smooth curve. The volume repeats every `SIZE` cells,
//! which the shaders' octave rotations and seed offsets hide.
//!
//! Each texel also carries its lattice point's gradient (central differences
//! of the values around it), so one fetch gives the noise and its slope
//! together: the erosion noise and the close-up relief need both, and four
//! fetches for a finite difference cost as many registers as the hashing.
//!
//! Shaders take it as bind group 1: `noise_volume` at binding 0 and
//! `noise_sampler` at binding 1 (see `WGSL`).

use slint::wgpu_30::wgpu;

/// Cells along each side of the volume.
pub const SIZE: u32 = 64;

/// The declarations and the noise functions, for shaders to prepend.
pub const WGSL: &str = r#"
@group(1) @binding(0) var noise_volume: texture_3d<f32>;
@group(1) @binding(1) var noise_sampler: sampler;

// Value noise in [-1, 1]: one filtered fetch, the fraction faded first so
// the linear filter blends along the quintic curve.
fn noise(x: vec3<f32>) -> f32 {
    let i = floor(x);
    let f = x - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    return textureSampleLevel(noise_volume, noise_sampler, (i + u + 0.5) / 64.0, 0.0).r * 2.0 - 1.0;
}

// Value noise and its gradient in one fetch: the gradient is the lattice's,
// blended along the same curve.
fn noised(x: vec3<f32>) -> vec4<f32> {
    let i = floor(x);
    let f = x - i;
    let u = f * f * f * (f * (f * 6.0 - 15.0) + 10.0);
    return textureSampleLevel(noise_volume, noise_sampler, (i + u + 0.5) / 64.0, 0.0) * 2.0 - 1.0;
}
"#;

/// The volume, its bind group and the layout pipelines declare it with.
pub struct Noise3d {
    pub layout: wgpu::BindGroupLayout,
    pub group: wgpu::BindGroup,
}

/// An f32 in [0, 1] as half-float bits.
fn half(value: f32) -> u16 {
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 0xff) as i32 - 127 + 15;
    if value <= 0.0 || exponent <= 0 {
        return 0;
    }
    let mantissa = (bits >> 13) & 0x3ff;
    ((exponent as u32) << 10 | mantissa) as u16
}

impl Noise3d {
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Self {
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Noise volume"),
            size: wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: SIZE,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::Rgba16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        // Deterministic, so a seed paints the same planet on every machine.
        let mut state = 0x2545_f491_4f6c_dd1du64;
        let n = SIZE as usize;
        let values: Vec<f32> = (0..n * n * n)
            .map(|_| {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                z ^= z >> 31;
                (z >> 40) as f32 / (1u64 << 24) as f32
            })
            .collect();
        let at = |x: usize, y: usize, z: usize| values[((z % n) * n + (y % n)) * n + (x % n)];
        let mut data = Vec::with_capacity(n * n * n * 8);
        for z in 0..n {
            for y in 0..n {
                for x in 0..n {
                    // The noise is 2v - 1, so its slope is twice the values'
                    // central difference; stored as (slope + 1) / 2, as the
                    // shader reads every channel back as c * 2 - 1.
                    let slope = |a: f32, b: f32| ((a - b) + 1.0) * 0.5;
                    let texel = [
                        at(x, y, z),
                        slope(at(x + 1, y, z), at(x + n - 1, y, z)),
                        slope(at(x, y + 1, z), at(x, y + n - 1, z)),
                        slope(at(x, y, z + 1), at(x, y, z + n - 1)),
                    ];
                    for c in texel {
                        data.extend_from_slice(&half(c.clamp(0.0, 1.0)).to_le_bytes());
                    }
                }
            }
        }
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(SIZE * 8),
                rows_per_image: Some(SIZE),
            },
            wgpu::Extent3d {
                width: SIZE,
                height: SIZE,
                depth_or_array_layers: SIZE,
            },
        );
        let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Noise volume"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            address_mode_w: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Noise volume"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
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
        let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Noise volume"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });
        Self { layout, group }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn halves() {
        assert_eq!(super::half(0.0), 0);
        assert_eq!(super::half(1.0), 0x3c00);
        assert_eq!(super::half(0.5), 0x3800);
    }
}
