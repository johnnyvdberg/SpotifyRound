// The Nautilus' salon window, around a sea drawn by the 3D dive.
//
// Two passes share this file:
//
// - `fs_overlay` draws the brass bezel, the riveted casing and the gauge faces.
//   None of it moves, so it is drawn once when the scene is built.
// - `fs_composite` runs at the panel's full resolution: it bends the sea at the
//   edge of the glass, gives it an engraved finish, lays the bezel over it and
//   draws the gauge needles, which are the only moving part of the frame.
//
// Positions are in units of the frame's height, with the window's centre at
// the origin.

struct Uniforms {
    // x, y: output frame size in pixels; z: seconds of run time; w: world seed in 0..1.
    frame: vec4<f32>,
    // x: edge of the square the sea is rendered into, in pixels; y: how far the
    // Nautilus has come, in world units; z: its height above the seabed; w: its
    // depth below the surface.
    scene: vec4<f32>,
};

@group(0) @binding(0) var<uniform> params: Uniforms;
@group(0) @binding(1) var sea: texture_2d<f32>;
@group(0) @binding(2) var bezel: texture_2d<f32>;
@group(0) @binding(3) var sea_sampler: sampler;

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
// The panel's own bezel frames the window, so the brass rim runs out to the
// very edge of the circle and the glass fills everything inside it.
const GLASS: f32 = 0.464;
const PORT: f32 = 0.500;
// Half the side of the square the sea texture covers, a little wider than the
// glass so the upscale never samples past the edge.
const VIEW: f32 = 0.470;
// The log turns once per leg of the voyage; this mirrors `voyage::LEG`.
const LEG: f32 = 3.4;

// Gauges set into the bezel: the manometer on the left reads depth, the log on
// the right turns once per leg of the voyage.
const GAUGE_R: f32 = 0.050;
const GAUGE_RING: f32 = 0.442;
const DEPTH_GAUGE: f32 = -2.235;
const LOG_GAUGE: f32 = -0.907;

// How much the thick glass bends the view at its edge. The 3D dive renders a
// plain perspective view and sets this so the composite bends it here.
override GLASS_BEND: f32 = 0.0;
// What the sea's colours were scaled by when they were stored, to fit them
// in a format that holds no more than 1 (see `SEA_SCALE` in the 3D dive).
override SEA_SCALE: f32 = 1.0;

fn rot(a: f32) -> mat2x2<f32> {
    let c = cos(a);
    let s = sin(a);
    return mat2x2<f32>(c, -s, s, c);
}

fn hash21(p: vec2<f32>) -> f32 {
    var p3 = fract(vec3<f32>(p.x, p.y, p.x) * 0.1031);
    p3 += dot(p3, p3.yzx + 33.33);
    return fract((p3.x + p3.y) * p3.z);
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

fn gauge_centre(angle: f32) -> vec2<f32> {
    return vec2<f32>(cos(angle), sin(angle)) * GAUGE_RING;
}

// A gauge face: brass bezel, enamel dial, ticks over `sweep` radians and a red
// sector near the end of the scale.
fn gauge_face(p: vec2<f32>, centre: vec2<f32>, sweep: f32, ticks: f32, red: bool) -> vec4<f32> {
    let l = p - centre;
    let r = length(l);
    if (r > GAUGE_R + 0.008) {
        return vec4<f32>(0.0);
    }
    let a = atan2(l.x, l.y);
    let u = r / GAUGE_R;
    let aa = 0.0012;
    var col = vec3<f32>(0.86, 0.80, 0.66) * (0.92 + 0.08 * vnoise(l * 300.0));
    col *= 1.0 - 0.25 * soft(0.55, 1.0, u);

    let inside = abs(a) <= sweep * 0.5 + 0.02;
    let step_a = sweep / ticks;
    let k = floor((a + sweep * 0.5) / step_a + 0.5);
    let near_a = -sweep * 0.5 + k * step_a;
    let tick_d = abs(a - near_a) * r;
    let major = (i32(k) % 5) == 0;
    let reach = select(0.80, 0.68, major);
    let tick = fill(tick_d - select(0.0004, 0.0009, major), aa * 0.7) * soft(reach - 0.02, reach, u) * soft(0.92, 0.90, u);
    if (inside) {
        col = mix(col, vec3<f32>(0.10, 0.08, 0.06), tick);
    }
    if (red && a > sweep * 0.30 && a < sweep * 0.5) {
        col = mix(col, vec3<f32>(0.62, 0.10, 0.06), soft(0.70, 0.72, u) * soft(0.80, 0.78, u));
    }
    col = mix(col, vec3<f32>(0.10, 0.08, 0.06), fill(abs(u - 0.93) - 0.006, 0.01));

    // The brass bezel ring with its knurled edge.
    let bez = soft(0.96, 0.99, u);
    let light = 0.55 + 0.45 * dot(normalize(l + vec2<f32>(0.0001)), normalize(vec2<f32>(-0.5, 0.8))) * select(1.0, -1.0, u > 1.08);
    let knurl = 0.85 + 0.15 * sin(atan2(l.y, l.x) * 120.0);
    let brass = vec3<f32>(0.62, 0.42, 0.16) * (0.45 + 0.75 * light) * knurl;
    col = mix(col, brass, bez);
    let alpha = fill(r - GAUGE_R - 0.006, aa);
    return vec4<f32>(col * alpha, alpha);
}

fn brass_rim(p: vec2<f32>, rp: f32, angle: f32) -> vec3<f32> {
    let u = clamp((rp - GLASS) / (PORT - GLASS), 0.0, 1.0);
    let radial = vec2<f32>(cos(angle), sin(angle));
    // Two rounded beads with a flat engraved band between them.
    let profile = select(
        select(sin((u - 0.70) / 0.30 * PI), 0.0, u < 0.70),
        sin(u / 0.30 * PI),
        u < 0.30,
    );
    let slope = select(
        select(cos((u - 0.70) / 0.30 * PI), 0.0, u < 0.70),
        cos(u / 0.30 * PI),
        u < 0.30,
    );
    let normal = normalize(vec3<f32>(-radial * slope * 0.9, 0.5 + profile * 0.5));
    let lamplight = normalize(vec3<f32>(-0.45, 0.60, 0.70));
    let diffuse = clamp(dot(normal, lamplight), 0.0, 1.0);
    let brushed = fbm(vec2<f32>(angle * 40.0, rp * 300.0));
    var brass = vec3<f32>(0.42, 0.26, 0.09) * (0.22 + 0.85 * diffuse);
    brass += vec3<f32>(0.55, 0.40, 0.18) * pow(diffuse, 9.0) * 0.9;
    brass *= 0.86 + 0.24 * brushed;
    // Tarnish in the crevices.
    brass = mix(brass, vec3<f32>(0.10, 0.17, 0.13), soft(0.55, 0.85, fbm(p * 40.0)) * (1.0 - profile) * 0.25);

    // The engraved scale on the flat band: a tick every 5 degrees, longer every 30.
    let deg = (angle / TAU) * 360.0;
    let k = floor(deg / 5.0 + 0.5);
    let tick_d = abs(deg - k * 5.0) * TAU / 360.0 * rp;
    let major = (i32(abs(k)) % 6) == 0;
    let band = soft(0.33, 0.36, u) * soft(0.67, 0.64, u);
    let length_ok = select(soft(0.48, 0.46, u) * soft(0.36, 0.38, u), band, major);
    let engraved = fill(tick_d - select(0.00045, 0.0008, major), 0.0006) * length_ok;
    brass *= 1.0 - 0.55 * engraved;
    brass = mix(brass, brass * 0.75, band * 0.25);

    // Twelve hex-headed bolts on the outer bead.
    let sector = TAU / 12.0;
    let bolt_a = floor(angle / sector + 0.5) * sector;
    let bolt_c = vec2<f32>(cos(bolt_a), sin(bolt_a)) * (GLASS + (PORT - GLASS) * 0.85);
    let bl = rot(bolt_a * 3.0 + 0.3) * (p - bolt_c);
    let hex_q = abs(bl);
    let hex = max(hex_q.x * 0.866 + hex_q.y * 0.5, hex_q.y) - 0.0075;
    let bolt = fill(hex, 0.0008);
    let bolt_light = 0.55 + 0.45 * dot(normalize(bl + vec2<f32>(0.0001)), normalize(vec2<f32>(-0.5, 0.7)));
    var head = vec3<f32>(0.40, 0.27, 0.11) * (0.45 + 0.65 * bolt_light);
    head += vec3<f32>(0.50, 0.38, 0.18) * soft(0.006, 0.0, length(bl - vec2<f32>(-0.002, 0.003))) * 0.5;
    brass = mix(brass * (1.0 - 0.5 * soft(0.012, 0.007, length(p - bolt_c - vec2<f32>(0.0015, -0.0015)))), head, bolt);

    // Dark seams where the rim meets the glass and the casing.
    brass *= 1.0 - 0.70 * exp(-u * 30.0) - 0.60 * exp(-(1.0 - u) * 40.0);
    return brass;
}

// The Nautilus' monogram on a brass cartouche at the foot of the window.
fn cartouche(p: vec2<f32>) -> vec4<f32> {
    let c = vec2<f32>(0.0, -GAUGE_RING - 0.004);
    let l = p - c;
    let plate = ell(l, vec2<f32>(0.046, 0.030));
    if (plate > 0.4) {
        return vec4<f32>(0.0);
    }
    let shade = 0.55 + 0.45 * soft(-0.03, 0.03, l.y - l.x * 0.4);
    var col = vec3<f32>(0.55, 0.37, 0.13) * shade;
    col *= 1.0 - 0.45 * fill(abs(plate + 0.12) - 0.012, 0.01);
    let n = min(
        min(segment(l, vec2<f32>(-0.012, -0.014), vec2<f32>(-0.012, 0.014)), segment(l, vec2<f32>(0.012, -0.014), vec2<f32>(0.012, 0.014))),
        segment(l, vec2<f32>(-0.012, 0.014), vec2<f32>(0.012, -0.014)),
    );
    let serifs = min(
        min(segment(l, vec2<f32>(-0.017, -0.014), vec2<f32>(-0.007, -0.014)), segment(l, vec2<f32>(0.007, 0.014), vec2<f32>(0.017, 0.014))),
        segment(l, vec2<f32>(-0.017, 0.014), vec2<f32>(-0.012, 0.014)),
    );
    let glyph = fill(min(n - 0.0024, serifs - 0.0012), 0.0008);
    col = mix(col, vec3<f32>(0.14, 0.08, 0.03), glyph * 0.85);
    let alpha = fill(plate, 0.03);
    return vec4<f32>(col * alpha, alpha);
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
fn fs_overlay(@builtin(position) in: vec4<f32>) -> @location(0) vec4<f32> {
    let p = vec2<f32>(
        (in.x - params.frame.x * 0.5) / params.frame.y,
        (params.frame.y * 0.5 - in.y) / params.frame.y,
    );
    let rp = length(p);
    let angle = atan2(p.y, p.x);

    var out = vec4<f32>(0.0);
    if (rp > PORT) {
        // Riveted iron plates of the salon wall.
        let plate_a = TAU / 8.0;
        let seam_a = abs(fract(angle / plate_a + 0.5) - 0.5) * plate_a * rp;
        let iron = vec3<f32>(0.050, 0.046, 0.042) * (0.75 + 0.35 * fbm(p * 22.0)) * (0.8 + 0.3 * soft(-0.4, 0.4, p.y - p.x));
        var casing = iron;
        casing *= 1.0 - 0.6 * fill(seam_a - 0.0012, 0.0008);
        let ring_d = abs(rp - 0.478);
        casing *= 1.0 - 0.5 * fill(ring_d - 0.0010, 0.0008);
        let rivet_a = TAU / 48.0;
        let ra = floor(angle / rivet_a + 0.5) * rivet_a;
        let rc = vec2<f32>(cos(ra), sin(ra)) * 0.465;
        let rd = length(p - rc);
        casing = mix(casing, vec3<f32>(0.11, 0.09, 0.07) * (0.6 + 0.6 * soft(0.004, -0.004, dot(p - rc, vec2<f32>(0.7, -0.7)))), fill(rd - 0.0038, 0.0007));
        casing += vec3<f32>(0.090, 0.060, 0.028) * exp(-length(p - vec2<f32>(-0.34, 0.41)) * 7.0);
        casing *= 1.0 - 0.6 * exp(-(rp - PORT) * 110.0);
        out = vec4<f32>(casing, 1.0);
    } else if (rp > GLASS) {
        out = vec4<f32>(brass_rim(p, rp, angle), 1.0);
    } else {
        // The bezel's shadow on the glass, deepest under the top edge.
        let lip = soft(GLASS - 0.030, GLASS, rp) * (0.40 + 0.30 * soft(-0.3, 0.3, p.y));
        out = vec4<f32>(0.0, 0.0, 0.0, lip * 0.75);
    }

    let depth_face = gauge_face(p, gauge_centre(DEPTH_GAUGE), 4.7, 30.0, true);
    out = vec4<f32>(depth_face.rgb + out.rgb * (1.0 - depth_face.a), depth_face.a + out.a * (1.0 - depth_face.a));
    let log_face = gauge_face(p, gauge_centre(LOG_GAUGE), TAU - 0.0001, 40.0, false);
    out = vec4<f32>(log_face.rgb + out.rgb * (1.0 - log_face.a), log_face.a + out.a * (1.0 - log_face.a));
    let plate = cartouche(p);
    out = vec4<f32>(plate.rgb + out.rgb * (1.0 - plate.a), plate.a + out.a * (1.0 - plate.a));
    return out;
}

// A blued-steel needle with a counterweight, pointing at `angle` (0 = up,
// clockwise), plus its shadow on the dial.
fn needle(col_in: vec3<f32>, p: vec2<f32>, centre: vec2<f32>, angle: f32, tip: vec3<f32>) -> vec3<f32> {
    var col = col_in;
    let l = p - centre;
    if (dot(l, l) > GAUGE_R * GAUGE_R) {
        return col;
    }
    let dir = vec2<f32>(sin(angle), cos(angle));
    let shadow_l = l - vec2<f32>(0.0025, -0.0025);
    let reach = GAUGE_R * 0.80;
    let ds = segment(shadow_l, -dir * reach * 0.25, dir * reach) - 0.0016;
    col *= 1.0 - 0.35 * fill(ds, 0.002);
    let along = clamp(dot(l, dir) / reach, -0.25, 1.0);
    let width = mix(0.0022, 0.0006, max(along, 0.0));
    let d = segment(l, -dir * reach * 0.25, dir * reach) - width;
    let steel = mix(vec3<f32>(0.06, 0.07, 0.11), tip, soft(0.70, 0.78, along));
    col = mix(col, steel, fill(d, 0.0007));
    let cap = length(l) - 0.0055;
    col = mix(col, vec3<f32>(0.60, 0.42, 0.18) * (0.7 + 0.5 * soft(0.004, -0.004, l.x - l.y)), fill(cap, 0.0007));
    return col;
}

@fragment
fn fs_composite(@builtin(position) in: vec4<f32>) -> @location(0) vec4<f32> {
    let t = params.frame.z;
    let seed = params.frame.w;
    let p = vec2<f32>(
        (in.x - params.frame.x * 0.5) / params.frame.y,
        (params.frame.y * 0.5 - in.y) / params.frame.y,
    );
    let rp = length(p);

    let over = textureLoad(bezel, vec2<i32>(in.xy), 0);
    var col = vec3<f32>(0.0);
    if (rp < GLASS + 0.002 && over.a < 0.999) {
        var q = p;
        if (GLASS_BEND > 0.0) {
            q += normalize(p + vec2<f32>(0.0001)) * soft(0.62, 1.0, rp / GLASS) * GLASS_BEND;
        }
        let uv = vec2<f32>(q.x, -q.y) / (2.0 * VIEW) + 0.5;
        col = textureSampleLevel(sea, sea_sampler, uv, 0.0).rgb / SEA_SCALE;

        // An engraver's finish, after the plates of the 1871 edition: fine
        // hatching that only shows in the shadows, and a warm, inky grade.
        let lum = dot(col, vec3<f32>(0.30, 0.55, 0.15));
        let hatch_a = 0.5 + 0.5 * sin((in.x + in.y) * 1.25);
        let hatch_b = 0.5 + 0.5 * sin((in.x - in.y) * 1.25);
        let dark = soft(0.16, 0.02, lum);
        let lines = hatch_a * soft(0.0, 1.0, dark) + hatch_b * soft(0.5, 1.0, dark);
        col *= 1.0 - 0.22 * lines * dark;
        col = mix(col, col * vec3<f32>(1.08, 1.0, 0.88), soft(0.10, 0.50, lum));
    }
    col = col * (1.0 - over.a) + over.rgb;

    // The manometer reads on a logarithmic scale, so it moves as much for the
    // Nautilus rising a few fathoms in the shallows as for a long climb out of
    // the abyss.
    let reading = log(1.0 + params.scene.w) / log(46.0);
    col = needle(col, p, gauge_centre(DEPTH_GAUGE), (reading - 0.5) * 4.7 * 0.92, vec3<f32>(0.60, 0.10, 0.06));
    col = needle(col, p, gauge_centre(LOG_GAUGE), params.scene.y / LEG * TAU, vec3<f32>(0.06, 0.07, 0.11));

    // Dither away the banding in the dark water.
    col += (hash21(in.xy + fract(t) * 91.0) - 0.5) / 255.0;
    return vec4<f32>(col, 1.0);
}
