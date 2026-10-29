// Small functions shared by the Nautilus' shaders.
//
// WESL links this module into each root shader at build time; the entry points
// and pipeline overrides stay in `scene.wesl`.

fn smooth01(a: f32, b: f32, v: f32) -> f32 {
    let t = clamp((v - a) / (b - a), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

// Like smoothstep, but defined when the edges are given high to low. The
// mask reads more naturally that way.
fn soft(edge0: f32, edge1: f32, x: f32) -> f32 {
    let low = min(edge0, edge1);
    let high = max(edge0, edge1);
    let t = clamp((x - low) / max(high - low, 0.00001), 0.0, 1.0);
    let v = t * t * (3.0 - 2.0 * t);
    return select(v, 1.0 - v, edge0 > edge1);
}

// Coverage of a signed distance, anti-aliased over `aa` either side of the edge.
fn fill(d: f32, aa: f32) -> f32 {
    return soft(aa, -aa, d);
}

fn ell(p: vec2<f32>, radius: vec2<f32>) -> f32 {
    return length(p / radius) - 1.0;
}

fn segment(p: vec2<f32>, a: vec2<f32>, b: vec2<f32>) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / max(dot(ba, ba), 0.00001), 0.0, 1.0);
    return length(pa - ba * h);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u = f * f * (3.0 - 2.0 * f);
    let a = hash21(i);
    let b = hash21(i + vec2<f32>(1.0, 0.0));
    let c = hash21(i + vec2<f32>(0.0, 1.0));
    let d = hash21(i + vec2<f32>(1.0, 1.0));
    return mix(mix(a, b, u.x), mix(c, d, u.x), u.y);
}
