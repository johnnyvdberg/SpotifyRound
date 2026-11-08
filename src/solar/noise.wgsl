const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;

fn hash3(c: vec3<i32>) -> f32 {
    var h = (u32(c.x) * 0x8da6b343u) ^ (u32(c.y) * 0xd8163841u) ^ (u32(c.z) * 0xcb1ab31fu);
    h ^= h >> 16u;
    h *= 0x7feb352du;
    h ^= h >> 15u;
    h *= 0x846ca68bu;
    h ^= h >> 16u;
    return f32(h >> 8u) * (1.0 / 16777216.0);
}

// Turns each octave so the lattice never lines up from one to the next.
const OCTAVE: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(0.00, 0.80, 0.60),
    vec3<f32>(-0.80, 0.36, -0.48),
    vec3<f32>(-0.60, -0.48, 0.64),
);

fn fbm(p: vec3<f32>, octaves: i32) -> f32 {
    var q = p;
    var sum = 0.0;
    var amplitude = 0.5;
    for (var i = 0; i < octaves; i++) {
        sum += amplitude * noise(q);
        q = OCTAVE * q * 2.03;
        amplitude *= 0.5;
    }
    return sum;
}
