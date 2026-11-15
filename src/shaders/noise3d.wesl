// The noise volume's declarations and the noise read from it (`noise3d.rs`):
// bind group 1, imported by the solar shaders.
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
