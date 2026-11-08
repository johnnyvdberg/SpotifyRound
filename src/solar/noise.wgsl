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

// Value noise in [-1, 1] and its gradient (Quilez), quintic between lattice
// points so the gradient is continuous too.
fn noised(x: vec3<f32>) -> vec4<f32> {
    let i = vec3<i32>(floor(x));
    let w = fract(x);
    let u = w * w * w * (w * (w * 6.0 - 15.0) + 10.0);
    let du = 30.0 * w * w * (w * (w - 2.0) + 1.0);
    let a = hash3(i + vec3<i32>(0, 0, 0)) * 2.0 - 1.0;
    let b = hash3(i + vec3<i32>(1, 0, 0)) * 2.0 - 1.0;
    let c = hash3(i + vec3<i32>(0, 1, 0)) * 2.0 - 1.0;
    let d = hash3(i + vec3<i32>(1, 1, 0)) * 2.0 - 1.0;
    let e = hash3(i + vec3<i32>(0, 0, 1)) * 2.0 - 1.0;
    let f = hash3(i + vec3<i32>(1, 0, 1)) * 2.0 - 1.0;
    let g = hash3(i + vec3<i32>(0, 1, 1)) * 2.0 - 1.0;
    let h = hash3(i + vec3<i32>(1, 1, 1)) * 2.0 - 1.0;
    let k1 = b - a;
    let k2 = c - a;
    let k3 = e - a;
    let k4 = a - b - c + d;
    let k5 = a - c - e + g;
    let k6 = a - b - e + f;
    let k7 = -a + b + c - d + e - f - g + h;
    let value = a + k1 * u.x + k2 * u.y + k3 * u.z + k4 * u.x * u.y + k5 * u.y * u.z
        + k6 * u.z * u.x + k7 * u.x * u.y * u.z;
    let gradient = du * vec3<f32>(
        k1 + k4 * u.y + k6 * u.z + k7 * u.y * u.z,
        k2 + k5 * u.z + k4 * u.x + k7 * u.z * u.x,
        k3 + k6 * u.x + k5 * u.y + k7 * u.x * u.y,
    );
    return vec4<f32>(value, gradient);
}

fn noise(x: vec3<f32>) -> f32 {
    return noised(x).x;
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
