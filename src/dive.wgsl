struct Uniforms {
    frame: vec4<f32>,
    camera: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Uniforms;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const GLASS: f32 = 0.404;
const PORT: f32 = 0.449;

// Screen units travelled per second. The water and the far ridge scroll more
// slowly than the seabed, which is what gives the porthole its depth.
const TRAVEL: f32 = 0.045;

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn hash11(x: f32) -> f32 {
    return fract(sin(x * 127.1 + 311.7) * 43758.5453);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
}

fn hash22(p: vec2<f32>) -> vec2<f32> {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * vec3<f32>(0.1031, 0.1030, 0.0973));
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.xx + p3.yz) * p3.zy);
}

// Like smoothstep, but defined when the edges are given high to low. Several
// masks below read more naturally that way, and the WGSL builtin leaves a
// reversed range undefined.
fn soft(edge0: f32, edge1: f32, x: f32) -> f32 {
    let low = min(edge0, edge1);
    let high = max(edge0, edge1);
    let t = clamp((x - low) / max(high - low, 0.00001), 0.0, 1.0);
    let v = t * t * (3.0 - 2.0 * t);
    return select(v, 1.0 - v, edge0 > edge1);
}

fn box(p: vec2<f32>, half: vec2<f32>, radius: f32) -> f32 {
    let q = abs(p) - half + vec2<f32>(radius);
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;
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

fn triangle(p: vec2<f32>, p0: vec2<f32>, p1: vec2<f32>, p2: vec2<f32>) -> f32 {
    let e0 = p1 - p0;
    let e1 = p2 - p1;
    let e2 = p0 - p2;
    let v0 = p - p0;
    let v1 = p - p1;
    let v2 = p - p2;
    let pq0 = v0 - e0 * clamp(dot(v0, e0) / max(dot(e0, e0), 0.00001), 0.0, 1.0);
    let pq1 = v1 - e1 * clamp(dot(v1, e1) / max(dot(e1, e1), 0.00001), 0.0, 1.0);
    let pq2 = v2 - e2 * clamp(dot(v2, e2) / max(dot(e2, e2), 0.00001), 0.0, 1.0);
    let s = sign(e0.x * e2.y - e0.y * e2.x);
    let d0 = vec2<f32>(dot(pq0, pq0), s * (v0.x * e0.y - v0.y * e0.x));
    let d1 = vec2<f32>(dot(pq1, pq1), s * (v1.x * e1.y - v1.y * e1.x));
    let d2 = vec2<f32>(dot(pq2, pq2), s * (v2.x * e2.y - v2.y * e2.x));
    let d = min(min(d0, d1), d2);
    return -sqrt(d.x) * sign(d.y);
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

fn fbm(p: vec2<f32>) -> f32 {
    var value = 0.0;
    var amplitude = 0.5;
    var q = p;
    for (var i = 0; i < 3; i = i + 1) {
        value += amplitude * vnoise(q);
        q = q * 2.03 + vec2<f32>(17.1, 9.2);
        amplitude *= 0.5;
    }
    return value / 0.875;
}

fn terrain_height(wx: f32, seed: f32) -> f32 {
    let n = 0.72 * vnoise(vec2<f32>(wx * 3.1, seed * 11.7))
        + 0.28 * (0.5 + 0.5 * sin(wx * 4.4 + seed * 13.0));
    return -0.315 + 0.080 * n + 0.012 * sin(wx * 8.5 + seed * 9.0);
}

fn ridge_height(wx: f32, seed: f32) -> f32 {
    let n = 0.62 * vnoise(vec2<f32>(wx * 1.8, seed * 5.1))
        + 0.38 * vnoise(vec2<f32>(wx * 3.7, seed * 2.3));
    return -0.055 + 0.150 * n + 0.018 * sin(wx * 6.0 + 1.3);
}

fn caustic(p: vec2<f32>, t: f32) -> f32 {
    let a = sin(p.x + sin(p.y * 1.3 + t));
    let b = sin(p.y * 1.1 + sin(p.x * 0.9 - t * 0.7));
    let web = abs(a + b) * 0.5;
    return pow(max(1.0 - web, 0.0), 7.0);
}

fn motes(q: vec2<f32>, scale: f32, drift: vec2<f32>, aspect: vec2<f32>) -> f32 {
    let p = q * scale + drift;
    let cell = floor(p);
    let f = fract(p);
    let centre = 0.18 + 0.64 * hash22(cell);
    let d = length((f - centre) * aspect);
    let twinkle = 0.35 + 0.65 * hash11(cell.x * 7.3 + cell.y * 13.1);
    return soft(0.045, 0.0, d) * twinkle;
}

// A small fish seen from the side. The school supplies the heading; the body,
// tail and fins swim around that axis. x is the body, y the rim light, z the
// pale belly and w the eye.
fn fish(
    q: vec2<f32>,
    centre: vec2<f32>,
    size: f32,
    angle: f32,
    phase: f32,
    t: f32,
    variant: f32,
) -> vec4<f32> {
    let wiggle = sin(t * 7.5 + phase) * 0.16;
    var l = rot(angle) * (q - centre) / size;
    l.y -= wiggle * soft(0.35, -1.05, l.x) * 0.8;

    let body_h = 0.26 + 0.13 * variant;
    let body_l = 0.80 + 0.16 * variant;
    let body = ell(l, vec2<f32>(body_l, body_h * (0.55 + 0.45 * soft(-0.9, 0.6, l.x))));
    let p = l.x + 0.75;
    let tail = max(
        max(-p - 0.40, p - 0.04),
        abs(l.y - wiggle) - (0.05 + max(0.0, -p) * 0.55),
    );
    let dorsal = ell(l - vec2<f32>(0.04, body_h + 0.02), vec2<f32>(0.22, 0.11));
    let pectoral = ell(l - vec2<f32>(0.30, -0.22), vec2<f32>(0.16, 0.055));
    let shape = min(min(body, tail), min(dorsal, pectoral));
    let eye = length(l - vec2<f32>(0.62, 0.06)) - 0.045;

    let mask = soft(0.09, -0.02, shape) * (1.0 - soft(0.035, 0.0, abs(eye)));
    let rim = soft(0.14, -0.005, shape) * (1.0 - soft(-0.02, -0.10, shape));
    let belly = mask * soft(0.04, -0.16, l.y / body_h);
    return vec4<f32>(mask, rim, belly, eye);
}

fn dolphin(
    q: vec2<f32>,
    centre: vec2<f32>,
    size: f32,
    angle: f32,
    phase: f32,
    t: f32,
) -> vec4<f32> {
    let wiggle = sin(t * 6.0 + phase) * 0.18;
    var l = rot(angle) * (q - centre) / size;
    l.y -= wiggle * soft(0.30, -1.0, l.x) * 0.65;

    let body = ell(
        l - vec2<f32>(0.01, 0.0),
        vec2<f32>(0.76, 0.20 * (0.38 + 0.62 * soft(-0.7, 0.7, l.x))),
    );
    let melon = ell(l - vec2<f32>(0.66, 0.045), vec2<f32>(0.13, 0.09));
    let beak = ell(l - vec2<f32>(0.82, -0.005), vec2<f32>(0.050, 0.040));
    let ff = l - vec2<f32>(0.0, wiggle * 1.2);
    let fluke_a = triangle(
        ff,
        vec2<f32>(-0.76, 0.020),
        vec2<f32>(-1.05, 0.095),
        vec2<f32>(-0.90, -0.018),
    );
    let fluke_b = triangle(
        ff,
        vec2<f32>(-0.76, -0.020),
        vec2<f32>(-1.05, -0.095),
        vec2<f32>(-0.90, 0.018),
    );
    let dorsal = triangle(
        l,
        vec2<f32>(-0.11, 0.155),
        vec2<f32>(0.055, 0.155),
        vec2<f32>(-0.045, 0.305),
    );
    let flipper = ell(
        rot(-0.55) * (l - vec2<f32>(0.18, -0.13)),
        vec2<f32>(0.13, 0.025),
    );
    let shape = min(
        min(body, min(melon, beak)),
        min(min(fluke_a, fluke_b), min(dorsal, flipper)),
    );
    let eye = length(l - vec2<f32>(0.64, 0.060)) - 0.030;

    let mask = soft(0.08, -0.02, shape) * (1.0 - soft(0.030, 0.0, abs(eye)));
    let rim = soft(0.13, -0.005, shape) * (1.0 - soft(-0.02, -0.09, shape));
    let belly = mask * soft(0.00, -0.12, l.y / 0.19);
    return vec4<f32>(mask, rim, belly, eye);
}

// A North Atlantic right whale: a huge head, a curved mouth, no dorsal fin,
// broad flukes and the pale callosities that make the silhouette recognisable.
fn whale(
    q: vec2<f32>,
    centre: vec2<f32>,
    scale: f32,
    angle: f32,
    phase: f32,
    t: f32,
) -> vec4<f32> {
    let l = rot(angle) * (q - centre) / scale;
    let tail_move = sin(t * 0.55 + phase) * 0.04;

    let body_d = ell(l - vec2<f32>(0.04, 0.0), vec2<f32>(0.34, 0.070));
    let head_d = ell(l - vec2<f32>(0.26, 0.012), vec2<f32>(0.145, 0.082));
    let rostrum = ell(l - vec2<f32>(0.395, -0.018), vec2<f32>(0.065, 0.034));
    let back_d = ell(l - vec2<f32>(-0.18, 0.020), vec2<f32>(0.16, 0.045));
    let stock_d = ell(
        l - vec2<f32>(-0.37, 0.035 + tail_move * 0.5),
        vec2<f32>(0.10, 0.022),
    );
    let fl = l - vec2<f32>(0.0, tail_move);
    let fluke_a = triangle(
        fl,
        vec2<f32>(-0.40, 0.040),
        vec2<f32>(-0.57, 0.110),
        vec2<f32>(-0.50, 0.010),
    );
    let fluke_b = triangle(
        fl,
        vec2<f32>(-0.40, 0.030),
        vec2<f32>(-0.57, -0.030),
        vec2<f32>(-0.50, 0.060),
    );
    let flipper = ell(
        rot(0.50) * (l - vec2<f32>(0.10, -0.065)),
        vec2<f32>(0.105, 0.018),
    );
    let shape = min(
        min(min(body_d, head_d), min(rostrum, back_d)),
        min(min(stock_d, fluke_a), min(fluke_b, flipper)),
    );
    let mask = soft(0.05, -0.015, shape);
    let rim = soft(0.10, 0.0, abs(shape)) * soft(0.035, -0.02, l.y);

    let mouth1 = segment(l, vec2<f32>(0.445, -0.015), vec2<f32>(0.32, -0.055));
    let mouth2 = segment(l, vec2<f32>(0.32, -0.055), vec2<f32>(0.17, -0.070));
    let mouth = (soft(0.003, 0.0, mouth1) + soft(0.003, 0.0, mouth2)) * mask;
    let c1 = exp(-dot(l - vec2<f32>(0.33, 0.070), l - vec2<f32>(0.33, 0.070)) * 2200.0);
    let c2 = exp(-dot(l - vec2<f32>(0.245, 0.082), l - vec2<f32>(0.245, 0.082)) * 2400.0);
    let c3 = exp(-dot(l - vec2<f32>(0.375, 0.028), l - vec2<f32>(0.375, 0.028)) * 3000.0);
    let callosity = (c1 + c2 + c3 + mouth * 0.85) * mask;
    let chin = ell(l - vec2<f32>(0.390, -0.038), vec2<f32>(0.055, 0.016));
    let chin_patch = soft(0.025, -0.015, chin) * mask;
    let side = ell(l - vec2<f32>(0.060, -0.036), vec2<f32>(0.070, 0.030));
    let side_patch = soft(0.03, -0.02, side)
        * mask
        * (0.55 + 0.45 * fbm(l * 16.0 + 4.0));
    return vec4<f32>(mask, rim, callosity + chin_patch * 0.75, side_patch);
}

// A jellyfish: a pulsing translucent dome, scalloped rim, radial canals, oral
// arms and a fan of fine tentacles. Everything trails the pulse rather than
// hanging perfectly straight.
fn jellyfish(
    q: vec2<f32>,
    centre: vec2<f32>,
    scale: f32,
    phase: f32,
    t: f32,
) -> vec3<f32> {
    let pulse = 0.5 + 0.5 * sin(t * 1.35 + phase);
    let contract = pulse * pulse;
    let rx = 0.062 * scale * (1.0 - 0.18 * contract);
    let ry = 0.043 * scale * (1.0 + 0.16 * contract);
    let l = q - centre;
    let dome = ell(l - vec2<f32>(0.0, 0.012 * scale), vec2<f32>(rx, ry));
    let upper = soft(-0.010 * scale, 0.004 * scale, l.y);
    let bell = soft(0.14, -0.03, dome) * upper;

    let rim_angle = atan2(l.y, l.x);
    let rim_y = -0.010 * scale + 0.003 * scale * sin(rim_angle * 9.0 + t * 0.8);
    let rim = exp(-abs(l.y - rim_y) * 260.0 / scale)
        * soft(1.05, 0.92, length(l / vec2<f32>(rx, ry)));
    let beads = pow(abs(sin(rim_angle * 9.0 + t * 0.8)), 22.0)
        * exp(-abs(l.y - rim_y) * 300.0 / scale)
        * soft(1.05, 0.92, length(l / vec2<f32>(rx, ry)));
    let halo = exp(-dot(l, l) / (0.0045 * scale * scale));
    let canals = pow(abs(sin(rim_angle * 7.0 + phase)), 16.0)
        * soft(0.9, 0.15, length(l / vec2<f32>(rx, ry)))
        * bell;

    let down = centre.y - q.y;
    var arms = 0.0;
    for (var j = 0; j < 3; j = j + 1) {
        let fj = f32(j);
        let reach = 0.12 * scale;
        let dx = (fj - 1.0) * 0.018 * scale;
        let x = centre.x + dx
            + sin(down * 18.0 + t * 2.0 + fj * 1.7 + phase) * 0.010 * scale * (0.3 + down);
        let width = 0.0050 * scale * (1.0 + 0.4 * contract);
        arms += soft(width, 0.0, abs(q.x - x))
            * soft(0.0, 0.015, down)
            * soft(reach, reach * 0.30, down);
    }

    var tentacles = 0.0;
    for (var j = 0; j < 10; j = j + 1) {
        let fj = f32(j);
        let reach = (0.12 + 0.06 * hash11(fj * 2.3 + phase)) * scale;
        let anchor = centre.x + (fj / 9.0 - 0.5) * 0.11 * scale;
        let x = anchor
            + sin(down * 26.0 + t * 2.4 + fj * 1.1 + phase) * 0.008 * scale * (0.2 + down * 3.0);
        let width = 0.0009 * scale * (1.0 + down * 2.0);
        tentacles += soft(width, 0.0, abs(q.x - x))
            * soft(0.0, 0.012, down)
            * soft(reach, reach * 0.5, down);
    }

    let gonad_a = exp(-dot(l - vec2<f32>(0.024, -0.002), l - vec2<f32>(0.024, -0.002)) * 9000.0);
    let gonad_b = exp(-dot(l - vec2<f32>(-0.024, -0.002), l - vec2<f32>(-0.024, -0.002)) * 9000.0);
    let gonad_c = exp(-dot(l - vec2<f32>(0.0, 0.018), l - vec2<f32>(0.0, 0.018)) * 13000.0);
    let gonads = (gonad_a + gonad_b + gonad_c) * bell;

    let bell_col = vec3<f32>(0.34, 0.31, 0.78);
    let glow = vec3<f32>(0.12, 0.62, 0.72);
    return bell_col * (bell * 0.14 + rim * 0.44 + beads * 0.30 + canals * 0.11)
        + glow * (halo * 0.14 + arms * 0.24 + tentacles * 0.16)
        + vec3<f32>(0.62, 0.38, 0.72) * gonads * 0.16;
}

fn water(q: vec2<f32>, t: f32, seed: f32) -> vec3<f32> {
    let bx = q.x + t * 0.010;
    let surface = 0.265
        + 0.010 * sin(bx * 7.0 + t * 0.55)
        + 0.006 * sin(bx * 15.0 - t * 0.37);
    let depth = clamp((surface - q.y) / 0.95, 0.0, 1.0);

    let near = vec3<f32>(0.075, 0.39, 0.43);
    let middle = vec3<f32>(0.014, 0.13, 0.18);
    let abyss = vec3<f32>(0.003, 0.016, 0.028);
    var colour = mix(near, middle, soft(0.0, 0.42, depth));
    colour = mix(colour, abyss, soft(0.34, 1.0, depth));

    let above = soft(surface - 0.004, surface + 0.004, q.y);
    let sky = vec3<f32>(0.60, 0.94, 0.89)
        * (0.72 + 0.28 * sin(bx * 21.0 + t * 0.9));
    colour = mix(colour, sky, above);

    let murk = fbm(
        vec2<f32>(q.x + t * 0.022, q.y) * vec2<f32>(2.4, 3.2)
            + vec2<f32>(seed * 23.0, -t * 0.018),
    );
    colour *= 0.82 + 0.36 * murk;

    let ca = caustic(
        vec2<f32>(q.x + t * 0.030, q.y) * vec2<f32>(5.5, 8.0)
            + vec2<f32>(t * 0.22, -t * 0.06),
        t * 0.7,
    );
    let near_surface = exp(-max(surface - q.y, 0.0) * 5.2);
    colour += vec3<f32>(0.09, 0.30, 0.28) * ca * near_surface * 0.48;

    let down = max(surface - q.y, 0.0);
    let slant = (q.x + t * 0.012 + down * 0.58) * 12.0
        + 1.4 * sin(q.x * 3.1 + t * 0.08);
    let ray_a = pow(max(sin(slant + 1.1 * sin(down * 2.3 + t * 0.13)), 0.0), 6.0);
    let ray_b = pow(max(sin(slant * 1.73 + 2.1), 0.0), 12.0);
    let shafts = (ray_a * 0.62 + ray_b * 0.38) * exp(-down * 2.15);
    colour += vec3<f32>(0.13, 0.39, 0.37) * shafts * 0.76;

    colour += vec3<f32>(0.04, 0.14, 0.16) * exp(-down * 1.35);
    return colour;
}

fn seabed(colour_in: vec3<f32>, q: vec2<f32>, t: f32, seed: f32) -> vec3<f32> {
    var colour = colour_in;
    let travel = t * TRAVEL;
    let wx = q.x + travel;

    // A far ridge, then the near floor: two silhouettes give the water depth.
    let ridge = ridge_height(q.x + t * 0.018, seed);
    let ridge_mask = 1.0 - soft(ridge - 0.006, ridge + 0.006, q.y);
    colour = mix(colour, vec3<f32>(0.010, 0.058, 0.075), ridge_mask * 0.80);

    let ground = terrain_height(wx, seed);
    let floor_mask = 1.0 - soft(ground - 0.004, ground + 0.004, q.y);
    let grit = fbm(vec2<f32>(wx * 12.0, q.y * 18.0) + vec2<f32>(seed * 17.0, 1.3));
    let floor_col = mix(
        vec3<f32>(0.010, 0.034, 0.039),
        vec3<f32>(0.030, 0.085, 0.082),
        grit * 0.7,
    );
    colour = mix(colour, floor_col, floor_mask);

    let lift = exp(-abs(q.y - ground) * 55.0) * floor_mask;
    colour += vec3<f32>(0.07, 0.18, 0.17) * lift * 0.35;

    // Boulders arrive in world cells, so the seabed keeps producing new shapes.
    let bspacing = 0.20;
    let bbase = floor(travel / bspacing) - 3.0;
    for (var i = 0; i < 4; i = i + 1) {
        let cell = bbase + f32(i);
        let wxi = (cell + 0.5 + (hash11(cell * 1.7 + seed * 13.0) - 0.5) * 0.7) * bspacing;
        let sx = wxi - travel;
        if (abs(sx) < 0.62) {
            let base_y = terrain_height(wxi, seed) - 0.008;
            let stone = vec2<f32>(
                0.045 + hash11(cell * 2.7 + seed) * 0.045,
                0.030 + hash11(cell * 8.9 + seed) * 0.025,
            );
            let rd = length((q - vec2<f32>(sx, base_y)) / stone) - 1.0;
            let rock = soft(0.16, -0.04, rd);
            colour = mix(colour, vec3<f32>(0.006, 0.024, 0.028), rock * 0.82);
            colour += vec3<f32>(0.035, 0.105, 0.100) * soft(0.06, -0.02, rd) * 0.22;
        }
    }

    // Fan corals.
    let cspacing = 0.40;
    let cbase = floor(travel / cspacing) - 2.0;
    for (var i = 0; i < 2; i = i + 1) {
        let cell = cbase + f32(i);
        let wxi = (cell + 0.5 + (hash11(cell * 7.3 + seed * 9.0) - 0.5) * 0.5) * cspacing;
        let sx = wxi - travel;
        if (abs(sx) < 0.60) {
            let base_y = terrain_height(wxi, seed) + 0.006;
            let fl = q - vec2<f32>(sx, base_y);
            let fr = length(fl);
            let fa = atan2(fl.y, fl.x);
            let fan = soft(0.060, 0.042, fr) * soft(-0.02, 0.025, fl.y);
            let ribs = pow(abs(sin(fa * 7.0 + 0.6)), 12.0)
                * soft(0.070, 0.012, fr)
                * soft(0.0, 0.020, fl.y);
            colour += vec3<f32>(0.85, 0.26, 0.36) * fan * 0.10;
            colour += vec3<f32>(1.0, 0.52, 0.58) * ribs * 0.13;
        }
    }

    // Kelp grows from the moving floor in the same world cells.
    let kspacing = 0.17;
    let kbase = floor(travel / kspacing) - 4.0;
    for (var i = 0; i < 7; i = i + 1) {
        let cell = kbase + f32(i);
        let wxi = (cell + 0.5 + (hash11(cell * 2.3 + seed * 7.0) - 0.5) * 0.8) * kspacing;
        let sx = wxi - travel;
        if (abs(sx) < 0.65) {
            let base_y = terrain_height(wxi, seed) + 0.004;
            let height = 0.18 + hash11(cell * 9.3 + seed * 7.0) * 0.28;
            let y = q.y - base_y;
            let up = clamp(y / height, 0.0, 1.0);
            let sway = sin(y * 24.0 - t * 0.9 + cell) * 0.022
                + sin(y * 8.0 + t * 0.45 + cell * 1.7) * 0.012;
            let x = sx + sway * up;
            let width = 0.0048 * (1.0 - up * 0.55) * (0.70 + 0.45 * sin(y * 82.0 + cell));
            let strand = soft(width, 0.0, abs(q.x - x))
                * soft(0.0, 0.012, y)
                * soft(height + 0.015, height - 0.015, y);
            colour = mix(colour, vec3<f32>(0.012, 0.095, 0.070), strand * 0.72);
            colour += vec3<f32>(0.030, 0.150, 0.105) * strand * up * 0.45;
        }
    }

    // Hydrothermal vents, also on their own long-world cycle.
    let vspacing = 1.9;
    let vbase = floor(travel / vspacing) - 1.0;
    for (var i = 0; i < 2; i = i + 1) {
        let cell = vbase + f32(i);
        let chance = hash11(cell * 4.1 + seed * 17.0);
        if (chance > 0.35) {
            let wxi = (cell + 0.5 + (chance - 0.5) * 0.5) * vspacing;
            let sx = wxi - travel;
            if (abs(sx) < 0.55) {
                let base_y = terrain_height(wxi, seed) + 0.004;
                let chimney_d = length(vec2<f32>((q.x - sx) * 5.2, q.y - base_y)) - 0.050;
                let chimney = soft(0.10, -0.04, chimney_d) * soft(0.11, 0.03, q.y - base_y);
                let plume_y = max(q.y - base_y - 0.035, 0.0);
                let plume_x = sx
                    + sin(plume_y * 35.0 - t * 1.6 + cell) * 0.007 * (0.4 + plume_y * 6.0);
                let plume = exp(-abs(q.x - plume_x) * 46.0)
                    * exp(-plume_y * 7.5)
                    * soft(0.0, 0.03, plume_y);
                colour = mix(colour, vec3<f32>(0.018, 0.024, 0.024), chimney);
                colour += vec3<f32>(1.0, 0.44, 0.13) * plume * 0.55;
                colour += vec3<f32>(0.30, 0.13, 0.04)
                    * exp(-length(q - vec2<f32>(sx, base_y)) * 18.0) * 0.32;
            }
        }
    }

    // Wrecks: one per world cell, so a new hull arrives as the submarine travels.
    let wspacing = 1.35;
    let wbase = floor(travel / wspacing) - 1.0;
    for (var i = 0; i < 2; i = i + 1) {
        let cell = wbase + f32(i);
        let chance = hash11(cell * 5.3 + seed * 11.0);
        if (chance > 0.25) {
            let wxi = (cell + 0.5 + (chance - 0.5) * 0.7) * wspacing;
            let sx = wxi - travel;
            if (abs(sx) < 0.62) {
                let base_y = terrain_height(wxi, seed) + 0.075;
                let scale = 0.90 + hash11(cell * 3.1 + seed * 5.0) * 0.45;
                let tilt = -0.10 + (hash11(cell * 8.7 + seed * 19.0) - 0.5) * 0.18;
                let wl = rot(tilt) * (q - vec2<f32>(sx, base_y)) / scale;

                let hull_main = ell(wl - vec2<f32>(-0.010, -0.038), vec2<f32>(0.215, 0.040));
                let hull_deck = box(wl - vec2<f32>(-0.010, 0.000), vec2<f32>(0.205, 0.018), 0.008);
                let hull_bow = triangle(
                    wl,
                    vec2<f32>(0.170, 0.020),
                    vec2<f32>(0.295, -0.005),
                    vec2<f32>(0.170, -0.065),
                );
                let hull_stern = box(wl - vec2<f32>(-0.195, 0.005), vec2<f32>(0.055, 0.045), 0.015);
                var hull_shape = min(hull_main, min(hull_deck, min(hull_bow, hull_stern)));
                hull_shape = max(hull_shape, wl.y - 0.018);
                let hole = ell(wl - vec2<f32>(-0.035, -0.015), vec2<f32>(0.040, 0.025))
                    + (fbm(wl * 14.0 + seed * 3.0) - 0.5) * 0.25;
                let hull = soft(0.040, -0.012, hull_shape)
                    * (1.0 - soft(0.015, -0.008, hole));
                let deck = soft(0.003, 0.0, abs(wl.y - 0.018))
                    * soft(0.24, 0.16, abs(wl.x));
                let house = soft(
                    0.020,
                    -0.008,
                    box(wl - vec2<f32>(-0.020, 0.040), vec2<f32>(0.055, 0.022), 0.006),
                );
                let stern_castle = soft(
                    0.020,
                    -0.008,
                    box(wl - vec2<f32>(-0.165, 0.042), vec2<f32>(0.050, 0.024), 0.006),
                );
                let forecastle = soft(
                    0.020,
                    -0.008,
                    box(wl - vec2<f32>(0.135, 0.036), vec2<f32>(0.045, 0.018), 0.006),
                );
                let bowsprit = segment(wl, vec2<f32>(0.220, 0.020), vec2<f32>(0.330, 0.075));
                let planks = pow(abs(sin(wl.y * 95.0 + wl.x * 2.0)), 12.0) * hull;
                let lantern = exp(-dot(wl - vec2<f32>(-0.235, 0.062), wl - vec2<f32>(-0.235, 0.062)) * 9000.0);

                let mast1 = segment(wl, vec2<f32>(-0.015, 0.020), vec2<f32>(-0.035, 0.245));
                let mast2 = segment(wl, vec2<f32>(0.125, 0.020), vec2<f32>(0.155, 0.200));
                let mast3 = segment(wl, vec2<f32>(-0.155, 0.020), vec2<f32>(-0.185, 0.155));
                let yard1 = segment(wl, vec2<f32>(-0.140, 0.205), vec2<f32>(0.100, 0.215));
                let yard2 = segment(wl, vec2<f32>(-0.090, 0.165), vec2<f32>(0.070, 0.175));
                let yard3 = segment(wl, vec2<f32>(0.085, 0.155), vec2<f32>(0.235, 0.165));
                let rig1 = segment(wl, vec2<f32>(-0.035, 0.245), vec2<f32>(0.230, 0.020));
                let rig2 = segment(wl, vec2<f32>(-0.035, 0.245), vec2<f32>(-0.250, 0.020));
                let rig3 = segment(wl, vec2<f32>(0.155, 0.200), vec2<f32>(0.080, 0.170));
                let mast = soft(
                    0.0065,
                    0.0,
                    min(min(mast1, mast2), mast3),
                );
                let yard = soft(0.0050, 0.0, min(min(yard1, yard2), yard3));
                let rigging = soft(0.0026, 0.0, min(rig1, min(rig2, rig3)));

                let sp = wl - vec2<f32>(-0.020, 0.135);
                let sail_shape = max(
                    max(sp.x - 0.110, -sp.x - 0.020),
                    max(sp.y - 0.050, -sp.y - 0.075),
                );
                let sail = soft(0.020, -0.020, sail_shape)
                    * (0.35 + 0.65 * fbm(wl * 18.0 + seed * 7.0));
                let sail_tri = soft(0.012, -0.010, triangle(
                    wl,
                    vec2<f32>(-0.020, 0.185),
                    vec2<f32>(0.095, 0.185),
                    vec2<f32>(-0.045, 0.080),
                )) * (0.45 + 0.55 * fbm(wl * 21.0 + seed * 3.0));
                let sail_total = max(sail, sail_tri);
                let flag = triangle(
                    wl,
                    vec2<f32>(-0.035, 0.255),
                    vec2<f32>(0.075, 0.230),
                    vec2<f32>(-0.035, 0.200),
                );
                let sail_edge = soft(0.008, 0.0, abs(sail_shape))
                    * (0.5 + 0.5 * fbm(wl * 24.0 + seed * 13.0));

                let debris = max(
                    box(rot(0.2) * (wl - vec2<f32>(-0.270, -0.070)), vec2<f32>(0.050, 0.006), 0.002),
                    box(rot(-0.1) * (wl - vec2<f32>(0.270, -0.060)), vec2<f32>(0.035, 0.005), 0.002),
                );
                let debris_mask = soft(0.010, -0.010, debris);

                let wreck_rim = soft(0.06, 0.0, abs(hull_shape)) * soft(0.025, -0.005, wl.y);

                var lamps = 0.0;
                var lamp_halo = 0.0;
                for (var j = 0; j < 3; j = j + 1) {
                    let lp = wl - vec2<f32>(-0.10 + f32(j) * 0.09, -0.022);
                    lamps += exp(-dot(lp * vec2<f32>(1.0, 1.3), lp * vec2<f32>(1.0, 1.3)) * 8000.0);
                    lamp_halo += exp(-dot(lp * vec2<f32>(1.0, 1.4), lp * vec2<f32>(1.0, 1.4)) * 500.0);
                }

                let hull_col = mix(
                    vec3<f32>(0.020, 0.035, 0.038),
                    vec3<f32>(0.050, 0.076, 0.074),
                    soft(-0.070, 0.020, wl.y),
                );
                colour = mix(colour, hull_col, hull * 0.95);
                colour = mix(colour, vec3<f32>(0.042, 0.072, 0.070),
                    max(max(house, forecastle), max(stern_castle, max(deck, mast))) * 0.90);
                colour = mix(colour, vec3<f32>(0.055, 0.085, 0.082), yard * 0.80);
                colour = mix(colour, vec3<f32>(0.045, 0.075, 0.073), soft(0.004, 0.0, bowsprit) * 0.80);
                colour = mix(colour, vec3<f32>(0.075, 0.115, 0.110), rigging * 0.75);
                colour = mix(colour, vec3<f32>(0.065, 0.110, 0.108), sail_total * 0.38);
                colour += vec3<f32>(0.10, 0.17, 0.16) * sail_edge * 0.28;
                colour = mix(colour, vec3<f32>(0.090, 0.135, 0.130), soft(0.005, 0.0, flag) * 0.55);
                colour = mix(colour, vec3<f32>(0.040, 0.070, 0.068), debris_mask * 0.70);
                colour += vec3<f32>(0.10, 0.24, 0.23) * wreck_rim * 0.55;
                colour += vec3<f32>(0.08, 0.18, 0.17) * soft(0.02, 0.0, abs(planks)) * 0.14;
                colour += vec3<f32>(0.30, 0.14, 0.05) * lamp_halo * 0.22;
                colour += vec3<f32>(1.0, 0.62, 0.24) * lamps * 0.85;
                colour += vec3<f32>(1.0, 0.64, 0.28) * lantern * 0.70;
                colour += vec3<f32>(0.03, 0.09, 0.10)
                    * exp(-length(wl - vec2<f32>(-0.02, 0.03)) * 3.4) * 0.20;
            }
        }
    }

    return colour;
}

// The reference submarine: a long dark hull, a row of round warm windows, a
// cyan light strip, glass domes, marker lights and a turning propeller. A pod
// of dolphins rides just below the bow.
fn submarine_layer(colour_in: vec3<f32>, q: vec2<f32>, t: f32, seed: f32) -> vec3<f32> {
    var colour = colour_in;
    let crossing = fract(t / 56.0 + seed * 0.71 + 0.18);
    let sub_c = vec2<f32>(
        -0.88 + crossing * 1.76,
        0.112 + 0.014 * sin(t * 0.17 + seed),
    );
    let tilt = 0.02 * sin(t * 0.11 + seed);
    let l = rot(tilt) * (q - sub_c);

    let hull_lower = ell(l - vec2<f32>(0.02, -0.018), vec2<f32>(0.335, 0.045));
    let hull_deck = box(l - vec2<f32>(0.02, 0.026), vec2<f32>(0.315, 0.018), 0.012);
    let bowtip = triangle(
        l,
        vec2<f32>(0.300, 0.045),
        vec2<f32>(0.415, 0.0),
        vec2<f32>(0.300, -0.045),
    );
    let tail = ell(l - vec2<f32>(-0.305, -0.002), vec2<f32>(0.050, 0.030));
    let hull_shape = min(min(hull_lower, hull_deck), min(bowtip, tail));
    let hull = soft(0.045, -0.015, hull_shape);
    let deck = soft(
        0.018,
        -0.008,
        box(l - vec2<f32>(0.015, 0.040), vec2<f32>(0.230, 0.010), 0.008),
    );
    let sail_shape = box(l - vec2<f32>(0.075, 0.050), vec2<f32>(0.095, 0.020), 0.012);
    let sail = soft(0.020, -0.010, sail_shape);
    let sail_rim = soft(0.018, 0.0, abs(sail_shape));
    let fin_shape = triangle(
        l,
        vec2<f32>(-0.170, 0.030),
        vec2<f32>(-0.020, 0.030),
        vec2<f32>(-0.100, 0.150),
    );
    let fin = soft(0.020, -0.010, fin_shape);
    let fin_rim = soft(0.018, 0.0, abs(fin_shape));
    let mast = soft(
        0.008,
        -0.004,
        box(l - vec2<f32>(0.060, 0.090), vec2<f32>(0.003, 0.022), 0.002),
    );
    let plane = soft(
        0.015,
        -0.005,
        box(l - vec2<f32>(0.245, -0.030), vec2<f32>(0.060, 0.006), 0.004),
    );

    let dome1 = length((l - vec2<f32>(0.215, 0.048)) / vec2<f32>(0.034, 0.032)) - 1.0;
    let dome2 = length((l - vec2<f32>(-0.245, 0.042)) / vec2<f32>(0.024, 0.022)) - 1.0;
    let glass = max(soft(0.03, -0.01, dome1), soft(0.03, -0.01, dome2));
    let glass_rim = max(soft(0.006, 0.0, abs(dome1)), soft(0.006, 0.0, abs(dome2)));

    var windows = 0.0;
    var window_glow = 0.0;
    var window_halo = 0.0;
    var window_core = 0.0;
    for (var j = 0; j < 5; j = j + 1) {
        let lp = l - vec2<f32>(-0.175 + f32(j) * 0.083, -0.014);
        let pd = length(lp);
        let ring = soft(0.030, 0.026, pd) * (1.0 - soft(0.021, 0.017, pd));
        windows = max(windows, ring);
        let fill = soft(0.024, 0.018, pd);
        let table = soft(0.003, 0.0, abs(lp.y + 0.004)) * soft(0.016, 0.010, abs(lp.x));
        window_glow = max(window_glow, fill * (0.8 + 0.2 * sin(f32(j) * 2.1 + t * 0.1)) * (1.0 - table * 0.6));
        window_halo += exp(-dot(lp, lp) * 700.0);
        window_core = max(window_core, exp(-dot(lp, lp) * 30000.0));
    }

    let strip_d = max(abs(l.y - 0.020) - 0.0022, abs(l.x - 0.015) - 0.215);
    let strip = soft(0.0, -0.004, strip_d);
    let green = exp(-dot(l - vec2<f32>(-0.255, 0.050), l - vec2<f32>(-0.255, 0.050)) * 9000.0)
        + exp(-dot(l - vec2<f32>(0.165, 0.060), l - vec2<f32>(0.165, 0.060)) * 9000.0);
    let white = exp(-dot(l - vec2<f32>(0.245, 0.070), l - vec2<f32>(0.245, 0.070)) * 12000.0);

    let prop_c = l - vec2<f32>(-0.300, -0.005);
    let hub = soft(0.008, 0.0, length(prop_c));
    var blades = 0.0;
    for (var j = 0; j < 3; j = j + 1) {
        let blade = rot(f32(j) * PI / 3.0 + t * 7.0) * prop_c;
        blades = max(blades, soft(0.010, 0.0, length(vec2<f32>(blade.x * 2.3, blade.y * 0.4))));
    }
    let prop = max(hub, blades * soft(0.045, 0.015, length(prop_c)));
    let thruster = exp(-length(prop_c) * 90.0) * (0.6 + 0.4 * sin(t * 12.0));

    let hull_rim = soft(0.12, 0.0, abs(hull_shape)) * soft(0.03, -0.02, l.y);
    let panel = soft(0.0025, 0.0, abs(l.y - 0.006)) * soft(0.27, 0.20, abs(l.x));
    let dome_glint = exp(-dot(l - vec2<f32>(0.195, 0.064), l - vec2<f32>(0.195, 0.064)) * 1200.0);
    colour = mix(colour, vec3<f32>(0.008, 0.022, 0.038), hull * 0.96);
    colour = mix(colour, vec3<f32>(0.015, 0.032, 0.052), max(deck, sail) * 0.90);
    colour = mix(colour, vec3<f32>(0.018, 0.035, 0.055), max(fin, mast) * 0.90);
    colour = mix(colour, vec3<f32>(0.020, 0.040, 0.060), plane * 0.85);
    colour += vec3<f32>(0.08, 0.20, 0.28) * hull_rim * 0.38;
    colour += vec3<f32>(0.10, 0.22, 0.28) * (sail_rim + fin_rim) * 0.22;
    colour += vec3<f32>(0.03, 0.08, 0.10) * panel * 0.25;
    colour = mix(colour, vec3<f32>(0.06, 0.10, 0.14), glass * 0.50);
    colour += vec3<f32>(0.28, 0.55, 0.70) * glass * 0.20;
    colour += vec3<f32>(0.40, 0.68, 0.82) * glass_rim * 0.35;
    colour += vec3<f32>(0.65, 0.85, 0.95) * dome_glint * 0.35;
    colour += vec3<f32>(0.35, 0.18, 0.06) * window_halo * 0.12;
    colour += vec3<f32>(0.70, 0.38, 0.12) * window_glow * 0.90;
    colour += vec3<f32>(1.00, 0.78, 0.40) * window_core * 0.85;
    colour += vec3<f32>(0.50, 0.25, 0.08) * windows * 0.20;
    colour += vec3<f32>(0.05, 0.50, 0.90) * strip * 0.85;
    colour += vec3<f32>(0.08, 0.35, 0.60)
        * exp(-abs(l.y - 0.020) * 55.0)
        * soft(0.24, 0.19, abs(l.x)) * 0.25;
    colour += vec3<f32>(0.18, 0.90, 0.55) * green * 0.50;
    colour += vec3<f32>(0.70, 0.90, 1.0) * white * 0.35;
    colour = mix(colour, vec3<f32>(0.025, 0.040, 0.055), prop * 0.80);
    colour += vec3<f32>(0.15, 0.55, 0.85) * thruster * 0.30;

    let beam_l = l - vec2<f32>(0.415, -0.005);
    let along = max(beam_l.x, 0.0);
    let across = abs(beam_l.y + along * 0.18);
    let beam = soft(0.08, 0.0, across)
        * soft(0.34, 0.02, along)
        * soft(0.0, 0.02, along);
    colour += vec3<f32>(0.35, 0.65, 0.65) * beam * 0.07;

    for (var i = 0; i < 4; i = i + 1) {
        let fi = f32(i);
        let off = vec2<f32>(
            0.28 - fi * 0.16,
            -0.105 - hash11(fi * 3.7 + seed * 8.0) * 0.045,
        );
        let phase = hash11(fi * 7.1 + seed * 13.0) * TAU;
        let size = 0.050 + hash11(fi * 5.3 + seed * 21.0) * 0.030;
        let dcentre = sub_c + rot(tilt) * off + vec2<f32>(0.0, sin(t * 0.6 + phase) * 0.006);
        let dangle = tilt + 0.03 * sin(t * 0.5 + phase);
        let d = dolphin(q, dcentre, size, dangle, phase, t);
        colour = mix(colour, vec3<f32>(0.075, 0.145, 0.170), d.x);
        colour += vec3<f32>(0.16, 0.30, 0.33) * d.y * 0.34;
        colour += vec3<f32>(0.32, 0.42, 0.43) * d.z * 0.35;
    }

    return colour;
}

@vertex
fn vs_main(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    return vec4<f32>(corners[index], 0.0, 1.0);
}

@fragment
fn fs_main(@builtin(position) in: vec4<f32>) -> @location(0) vec4<f32> {
    let t = params.frame.z;
    let seed = params.frame.w;
    let p = vec2<f32>(
        (in.x - params.frame.x * 0.5) / params.frame.y,
        (params.frame.y * 0.5 - in.y) / params.frame.y,
    );
    let rp = length(p);
    let angle = atan2(p.y, p.x);

    if (rp > PORT + 0.003) {
        let ribs = 0.5 + 0.5 * sin(angle * 46.0 + fbm(p * 18.0) * 3.0);
        var casing = mix(
            vec3<f32>(0.008, 0.011, 0.013),
            vec3<f32>(0.040, 0.036, 0.030),
            ribs * 0.30,
        );
        casing += vec3<f32>(0.070, 0.042, 0.016)
            * exp(-length(p - vec2<f32>(-0.34, -0.41)) * 8.5);
        casing += vec3<f32>(0.040, 0.055, 0.060)
            * exp(-length(p - vec2<f32>(0.43, 0.38)) * 11.0);
        return vec4<f32>(casing, 1.0);
    }

    // The glass bends the view a little at its edge, and the submarine sways.
    let sway = rot(sin(t * 0.071) * 0.010) * p;
    let edge = soft(0.62, 1.0, rp / GLASS);
    var q = sway + normalize(p + vec2<f32>(0.0001, 0.0001)) * edge * 0.007;
    q += vec2<f32>(sin(t * 0.13) * 0.004, sin(t * 0.11 + 1.3) * 0.005);

    var colour = water(q, t, seed);
    colour = seabed(colour, q, t, seed);

    // The right whale crosses far behind the nearer life.
    let whale_cross = fract(t / 78.0 + seed * 0.43);
    let whale_c = vec2<f32>(
        -0.95 + whale_cross * 1.90,
        0.155 + 0.020 * sin(t * 0.21 + seed),
    );
    if (abs(whale_c.x) < 0.95) {
        let w = whale(
            q,
            whale_c,
            0.58,
            0.025 * sin(t * 0.17 + seed),
            seed * 11.0,
            t,
        );
        colour = mix(colour, vec3<f32>(0.016, 0.043, 0.058), w.x * 0.92);
        colour += vec3<f32>(0.06, 0.16, 0.18) * w.y * 0.30;
        colour += vec3<f32>(0.40, 0.52, 0.54) * (w.z * 0.38 + w.w * 0.17);
    }

    // Jellyfish drift in the middle distance, before the fish and the submarine.
    for (var i = 0; i < 3; i = i + 1) {
        let fi = f32(i);
        let phase = hash11(fi * 13.1 + seed * 7.3) * TAU;
        let scale = 0.55 + hash11(fi * 7.7 + seed * 19.0) * 0.85;
        let base = vec2<f32>(
            -0.28 + hash11(fi * 3.3 + seed * 4.0) * 0.60,
            -0.22 + hash11(fi * 5.9 + seed * 9.0) * 0.40,
        );
        let pulse = 0.5 + 0.5 * sin(t * 1.35 + phase);
        let centre = base
            + vec2<f32>(
                sin(t * 0.09 + phase) * 0.035,
                sin(t * 0.13 + phase * 2.0) * 0.030 + pulse * 0.010,
            );
        let tint = mix(
            vec3<f32>(0.78, 0.88, 1.0),
            vec3<f32>(1.0, 0.76, 0.90),
            hash11(fi * 17.3 + seed * 5.0),
        );
        colour += jellyfish(q, centre, scale, phase, t)
            * (0.70 + 0.20 * sin(t * 0.8 + phase))
            * tint;
    }

    // Two shoals. Each fish is drawn along the school's velocity, so the head
    // points where the fish is actually going.
    var fish_mask = 0.0;
    var fish_rim = 0.0;
    var fish_belly = 0.0;
    var fish_mask_b = 0.0;
    var fish_rim_b = 0.0;
    var fish_belly_b = 0.0;

    let cycle_a = fract(t / 55.0 + seed * 0.61);
    let sx_a = -0.88 + cycle_a * 1.76;
    let sy_a = 0.060 + 0.035 * sin(t * 0.11 + seed * 2.0);
    let vy_a = 0.035 * 0.11 * cos(t * 0.11 + seed * 2.0);
    let ang_a = atan2(vy_a, 1.76 / 55.0);
    for (var i = 0; i < 8; i = i + 1) {
        let fi = f32(i);
        let variant = hash11(fi * 1.7 + seed * 31.0);
        let off = vec2<f32>(
            (hash11(fi * 3.1 + seed * 10.0) - 0.5) * 0.30,
            (hash11(fi * 5.7 + seed * 24.0) - 0.5) * 0.14,
        );
        let wob = vec2<f32>(sin(t * 0.9 + fi * 2.0) * 0.012, cos(t * 1.2 + fi * 1.7) * 0.008);
        let fang = ang_a + sin(t * 0.8 + fi * 2.3) * 0.09 + (hash11(fi * 9.1 + seed) - 0.5) * 0.06;
        let size = 0.010 + variant * 0.010;
        let f = fish(
            q,
            vec2<f32>(sx_a, sy_a) + off + wob,
            size,
            fang,
            fi * 1.7 + seed * 3.0,
            t,
            variant,
        );
        fish_mask = max(fish_mask, f.x);
        fish_rim = max(fish_rim, f.y);
        fish_belly = max(fish_belly, f.z);
    }

    let cycle_b = fract(t / 70.0 + seed * 0.37);
    let sx_b = 0.88 - cycle_b * 1.76;
    let sy_b = 0.085 + 0.030 * sin(t * 0.13 + seed * 4.0);
    let vy_b = 0.030 * 0.13 * cos(t * 0.13 + seed * 4.0);
    let ang_b = atan2(vy_b, -1.76 / 70.0);
    for (var i = 0; i < 6; i = i + 1) {
        let fi = f32(i);
        let variant = hash11(fi * 2.9 + seed * 41.0);
        let off = vec2<f32>(
            (hash11(fi * 4.3 + seed * 12.0) - 0.5) * 0.25,
            (hash11(fi * 6.1 + seed * 29.0) - 0.5) * 0.11,
        );
        let wob = vec2<f32>(sin(t * 1.1 + fi * 1.7) * 0.010, cos(t * 1.4 + fi * 2.1) * 0.007);
        let fang = ang_b + sin(t * 0.9 + fi * 1.9) * 0.08 + (hash11(fi * 8.3 + seed) - 0.5) * 0.05;
        let size = 0.008 + variant * 0.008;
        let f = fish(
            q,
            vec2<f32>(sx_b, sy_b) + off + wob,
            size,
            fang,
            fi * 2.1 + seed * 5.0,
            t,
            variant,
        );
        fish_mask_b = max(fish_mask_b, f.x);
        fish_rim_b = max(fish_rim_b, f.y);
        fish_belly_b = max(fish_belly_b, f.z);
    }

    colour = mix(colour, vec3<f32>(0.012, 0.050, 0.060), fish_mask * 0.92);
    colour += vec3<f32>(0.16, 0.38, 0.40) * fish_rim * 0.20;
    colour += vec3<f32>(0.18, 0.32, 0.33) * fish_belly * 0.20;
    colour = mix(colour, vec3<f32>(0.025, 0.072, 0.076), fish_mask_b * 0.90);
    colour += vec3<f32>(0.18, 0.35, 0.36) * fish_rim_b * 0.18;
    colour += vec3<f32>(0.20, 0.32, 0.32) * fish_belly_b * 0.18;

    colour = submarine_layer(colour, q, t, seed);

    // Marine snow, strung out horizontally by the forward travel, then a drift
    // of bioluminescent plankton near the bottom.
    let snow = motes(
            q + vec2<f32>(-t * 0.012, -t * 0.006),
            40.0,
            vec2<f32>(seed * 9.0, 0.0),
            vec2<f32>(0.30, 1.25),
        )
        + 0.6 * motes(
            q + vec2<f32>(-t * 0.020, -t * 0.011),
            75.0,
            vec2<f32>(0.0, seed * 5.0),
            vec2<f32>(0.45, 1.25),
        );
    colour += vec3<f32>(0.34, 0.61, 0.63) * snow * 0.17;
    let bio = motes(
        q * 1.4 + vec2<f32>(seed * 13.0 - t * 0.010, 0.0),
        34.0,
        vec2<f32>(t * 0.004, -t * 0.003),
        vec2<f32>(1.0, 1.0),
    );
    let flourish = 0.45 + 0.55 * sin(t * 2.1 + hash11(seed * 27.0) * TAU);
    colour += vec3<f32>(0.10, 0.82, 0.66)
        * bio * soft(0.08, -0.36, q.y) * flourish * 0.30;

    // Bubbles rising past the glass, drifting back with the current.
    for (var i = 0; i < 10; i = i + 1) {
        let fi = f32(i);
        let speed = 0.025 + hash11(fi + seed * 17.0) * 0.045;
        let y = -0.52 + fract(hash11(fi * 2.3 + seed * 5.0) + t * speed);
        let x = -0.55 + fract(
            hash11(fi * 3.7 + seed * 23.0)
                - t * 0.015
                + sin(t * 1.5 + fi * 2.1) * 0.010
        ) * 1.10;
        let radius = 0.005 + hash11(fi * 4.1 + seed * 29.0) * 0.009;
        let d = length(q - vec2<f32>(x, y));
        let shell = soft(radius, radius - 0.0020, d)
            * (1.0 - soft(radius - 0.0055, radius - 0.0030, d));
        let glint = soft(
            radius * 0.38,
            0.0,
            length(q - vec2<f32>(x - radius * 0.36, y + radius * 0.36)),
        );
        let inner = soft(radius - 0.0045, radius - 0.0020, d);
        colour += vec3<f32>(0.38, 0.78, 0.82) * (shell * 0.42 + glint * 0.70 + inner * 0.03);
    }

    // Glass: a faint green tint, vignette and two reflections.
    colour *= vec3<f32>(0.92, 1.00, 0.97);
    colour *= 1.0 - 0.55 * soft(GLASS * 0.55, GLASS, rp);
    let grime = fbm(p * 24.0 + seed * 31.0);
    colour *= 0.95 + 0.05 * grime;
    let shear = dot(p, normalize(vec2<f32>(0.55, -0.83)));
    let sheen = exp(-pow((shear + 0.16) * 13.0, 2.0));
    colour += vec3<f32>(0.12, 0.20, 0.21) * sheen * 0.16;
    let arc = exp(-abs(rp - GLASS + 0.007) * 150.0)
        * soft(
            0.0,
            0.7,
            dot(
                normalize(p + vec2<f32>(0.0001, 0.0001)),
                normalize(vec2<f32>(-0.62, 0.78)),
            ),
        );
    colour += vec3<f32>(0.26, 0.48, 0.47) * arc * 0.18;

    // The brass rim, shaded as a rounded ring with a row of rivets.
    let u = clamp((rp - GLASS) / (PORT - GLASS), 0.0, 1.0);
    let radial = vec2<f32>(cos(angle), sin(angle));
    let round = u * 2.0 - 1.0;
    let normal = normalize(vec3<f32>(radial * round * 0.78, sqrt(max(1.0 - round * round, 0.02))));
    let lamplight = normalize(vec3<f32>(-0.45, 0.60, 0.78));
    let diffuse = clamp(dot(normal, lamplight), 0.0, 1.0);
    let brushed = fbm(vec2<f32>(angle * 5.0, rp * 95.0));
    var brass = vec3<f32>(0.39, 0.235, 0.085) * (0.26 + 0.78 * diffuse);
    brass += vec3<f32>(0.42, 0.29, 0.12) * pow(diffuse, 7.0);
    brass *= 0.88 + 0.24 * brushed;
    brass = mix(brass, brass * 0.56, soft(0.34, 0.0, abs(u - 0.16)));
    brass += vec3<f32>(0.22, 0.16, 0.07) * soft(0.90, 1.0, u) * 0.55;

    let sector = TAU / 18.0;
    let rivet_angle = floor(angle / sector + 0.5) * sector;
    let rivet_centre = vec2<f32>(cos(rivet_angle), sin(rivet_angle))
        * ((GLASS + PORT) * 0.5 + 0.004);
    let rivet_d = length(p - rivet_centre);
    brass = mix(brass, brass * 0.62, soft(0.0058, 0.0042, rivet_d));
    brass += vec3<f32>(0.48, 0.37, 0.20)
        * soft(0.0021, 0.0, length(p - rivet_centre - vec2<f32>(-0.0007, 0.0010)));

    let seam = exp(-abs(rp - GLASS) * 420.0);
    let rim_mask = soft(GLASS - 0.001, GLASS + 0.001, rp)
        * (1.0 - soft(PORT - 0.001, PORT + 0.001, rp));
    colour = mix(colour, brass, rim_mask);
    colour *= 1.0 - 0.72 * seam;
    colour += vec3<f32>(0.24, 0.17, 0.07) * seam * 0.35;
    return vec4<f32>(colour, 1.0);
}
