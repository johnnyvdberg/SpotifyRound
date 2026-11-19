// Shared by the observatory and the microscope (`src/lens.rs`): a full-screen
// triangle, hashes, value noise, the eyepiece's field stop and the encode.

const TAU: f32 = 6.2831853;
// `Theme.bg` (#07070a) in linear light: the window colour outside the glass.
const OUTSIDE: vec3<f32> = vec3<f32>(0.0021, 0.0021, 0.0030);

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

// Pseudo-random in 0..1 for a cell. Keep `p` within a few hundred: past that
// single floats lose the fraction this lives on.
fn hash21(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + 45.32);
    return fract(q.x * q.y);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    let n = hash21(p);
    return vec2<f32>(n, hash21(p + n + 17.17));
}

// Smooth value noise in 0..1.
fn noise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let w = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, w.x), mix(c, d, w.x), w.y);
}

// Three octaves of it, in 0..1.
fn fbm(p: vec2<f32>) -> f32 {
    return 0.55 * noise(p) + 0.3 * noise(p * 2.03 + 11.7) + 0.15 * noise(p * 4.11 + 3.1);
}

fn rotate(p: vec2<f32>, angle: f32) -> vec2<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return vec2<f32>(c * p.x + s * p.y, -s * p.x + c * p.y);
}

// The pixel in field units: the centre of the panel at 0, the field stop at 1,
// y up. `frame` is the centre in pixels and the field stop's radius.
fn field_position(position: vec2<f32>, frame: vec4<f32>) -> vec2<f32> {
    return vec2<f32>(position.x - frame.x, frame.y - position.y) / frame.z;
}

// The field stop: what the glass shows (`inside`), cut off softly at its edge
// over a dark barrel with a faint ring of light caught on its lip.
fn eyepiece(inside: vec3<f32>, p: vec2<f32>, pixel: f32) -> vec3<f32> {
    let r = length(p);
    let shown = 1.0 - smoothstep(1.0 - 2.5 * pixel, 1.0, r);
    let lip = exp(-pow((r - 1.025) / 0.008, 2.0)) * 0.004;
    return mix(OUTSIDE + vec3<f32>(lip, lip * 0.95, lip * 0.85), inside, shown);
}

// Linear light to sRGB, with a soft shoulder instead of a hard clip.
fn encode(linear: vec3<f32>) -> vec4<f32> {
    let shaped = vec3<f32>(1.0) - exp(-max(linear, vec3<f32>(0.0)) * 1.15);
    return vec4<f32>(pow(shaped, vec3<f32>(1.0 / 2.2)), 1.0);
}
