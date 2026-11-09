// The solar tour's frame. Prepended with `noise.wgsl` and the noise volume's
// declarations (`noise3d.rs`).
//
// Many small pipelines, drawn one after another in a single render pass, each
// within the V3D's four-thread register budget; the tile buffer blends them on
// chip. They write linear light, premultiplied, through an sRGB view.
//
// - First the depth mask (`fs_mask`): each body's solid disc at its
//   distance rank, so everything after skips, before shading, whatever a
//   nearer body hides; the background sits at the far end of it.
// - The background, full screen: `fs_space_baked` (the galaxy and stars,
//   baked once a system by `fs_bake_stars`, and the star's glare), or
//   `fs_space` (the same, worked out per pixel) until the bake is done, or
//   `fs_warp` during a hyperjump, one pipeline per drive (`WARP`).
// - Then the bodies, far to near, each a fan of triangles round its own
//   silhouette (`vs_body`), shaders branching rather than discarding so the
//   V3D skips their work for pixel groups with nothing to draw:
//   `fs_star`; `fs_flat` for a planet whose maps are not in yet; for a planet
//   with maps `fs_ground`, `fs_clouds` and `fs_glow` (added, with the sea's
//   glint where it has seas);
//   `fs_moon` for a moon; `fs_limb` (added) for the air past any limb; and
//   `fs_ring` over a ringed planet. The mapped ones are built per map slot
//   (`SLOT`), so each reads one set of textures.
// - Shadows are passes of their own, multiplied into what is drawn:
//   `fs_eclipse`
//   (only when another body is near the line to the star) and
//   `fs_ring_shadow` (ringed planets) before the night side's glow.

struct Body {
    // xyz: centre relative to the camera; w: radius.
    place: vec4<f32>,
    // Rows taking world directions into the body's turning frame; rot[0].w
    // is the clouds' extra turn.
    rot: array<vec4<f32>, 3>,
    // x: kind (-1 for the star); y: map slot (-1 none); z: moon layer (-1 for
    // a planet); w: night mode (0 none, 1 cities, 2 glowing rock).
    look: vec4<f32>,
    // rgb: the air's optical depth; w: unused.
    air: vec4<f32>,
    // rgb: flat colour when no maps are loaded; w: band contrast for giants.
    colour: vec4<f32>,
    // Rings: inner and outer radius, opacity, seed.
    ring: vec4<f32>,
    ring_colour: vec4<f32>,
    // The quad, in clip space: min x, min y, max x, max y.
    rect: vec4<f32>,
    // The bodies that can cast a shadow on this one (their indices; -1 for
    // none), from the CPU, so eclipses loop over those, not every body.
    shade: vec4<f32>,
};

struct Params {
    // x: half the frame in pixels; y: tangent of half the field of view;
    // z: the angle a pixel spans; w: bodies in use.
    frame: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    // The star: centre relative to the camera, and radius.
    star: vec4<f32>,
    // Its disc's and glare's colour, and w: how much of its disc is in view.
    star_colour: vec4<f32>,
    // The light it casts on its planets, tinted and scaled by its kind.
    light: vec4<f32>,
    // Rows turning world directions into the sky map's frame.
    sky: array<vec4<f32>, 3>,
    // x: warp 0..1; y: flash; z: seconds; w: seconds into the jump.
    warp: vec4<f32>,
    bodies: array<Body, 16>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(1) var sky_map: texture_2d<f32>;
@group(0) @binding(2) var map_sampler: sampler;
@group(0) @binding(3) var a_day: texture_2d<f32>;
@group(0) @binding(4) var a_relief: texture_2d<f32>;
@group(0) @binding(5) var a_extra: texture_2d<f32>;
@group(0) @binding(6) var a_moon_day: texture_2d_array<f32>;
@group(0) @binding(7) var a_moon_relief: texture_2d_array<f32>;
@group(0) @binding(8) var b_day: texture_2d<f32>;
@group(0) @binding(9) var b_relief: texture_2d<f32>;
@group(0) @binding(10) var b_extra: texture_2d<f32>;
@group(0) @binding(11) var b_moon_day: texture_2d_array<f32>;
@group(0) @binding(12) var b_moon_relief: texture_2d_array<f32>;
// Each ringed planet's ring density by radius, a row per planet (ring.w).
@group(0) @binding(13) var ring_profile: texture_2d<f32>;
// The galaxy and stars baked into an equirectangular map, once a system.
@group(0) @binding(14) var starfield: texture_2d<f32>;

// Which map slot a mapped body's pipeline reads, and which drive a warp
// pipeline draws.
override SLOT: i32 = 0;
override WARP: i32 = 1;
// Which of the stacked cloud layers drawn in low orbit (from 1): each a shell
// a little higher, showing only the clouds dense enough to reach it.
override LAYER: i32 = 1;

const SKYLIGHT: vec3<f32> = vec3<f32>(0.035, 0.060, 0.110);
const SCALE_HEIGHT: f32 = 0.011;
const ATMOSPHERE: f32 = 1.075;
const BIAS: f32 = 0.45;

fn smoothstep01(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn airmass(mu: f32) -> f32 {
    let m = max(mu, -0.02);
    return 1.0 / (m + 0.025 + 0.045 * exp(-m * 30.0));
}

fn ray_at(position: vec2<f32>) -> vec3<f32> {
    let half = params.frame.x;
    let s = vec2<f32>((position.x - half) / half, (half - position.y) / half);
    return normalize(params.forward.xyz + (params.right.xyz * s.x + params.up.xyz * s.y) * params.frame.y);
}

// ---------------------------------------------------------------------------
// Background.
// ---------------------------------------------------------------------------

struct Full {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_background(@builtin(vertex_index) index: u32) -> Full {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Full;
    // At the far end of the depth mask: hidden wherever a body's disc is.
    out.position = vec4<f32>(corners[index], 1.0, 1.0);
    return out;
}

// One layer of stars: a star in some cells of a grid `scale` cells across,
// each a point a pixel or so wide, coloured by its temperature.
fn stars(dir: vec3<f32>, scale: f32, density: f32, brightness: f32, seed: i32, size: f32) -> vec3<f32> {
    let cell = vec3<i32>(floor(dir * scale));
    let h = hash3(cell + vec3<i32>(seed, 0, 0));
    if (h > density) {
        return vec3<f32>(0.0);
    }
    let centre = normalize(vec3<f32>(cell) + vec3<f32>(
        hash3(cell + vec3<i32>(seed, 1, 0)),
        hash3(cell + vec3<i32>(seed, 2, 0)),
        hash3(cell + vec3<i32>(seed, 3, 0)),
    ));
    let angle = length(cross(dir, centre));
    let b = h / density;
    let glow = exp(-angle * angle / (size * size)) * brightness * (0.15 + b * b * 1.6);
    let tint = mix(vec3<f32>(1.0, 0.72, 0.5), vec3<f32>(0.7, 0.82, 1.0), hash3(cell + vec3<i32>(seed, 4, 0)));
    return tint * glow;
}

fn sky_at(dir: vec3<f32>) -> vec3<f32> {
    let d = vec3<f32>(dot(params.sky[0].xyz, dir), dot(params.sky[1].xyz, dir), dot(params.sky[2].xyz, dir));
    let uv = vec2<f32>(0.5 + atan2(d.x, d.z) / TAU, 0.5 - asin(clamp(d.y, -1.0, 1.0)) / PI);
    return textureSampleLevel(sky_map, map_sampler, uv, 0.0).rgb;
}

// The star's glare: a halo falling off from its disc, where it is in view.
fn glare(dir: vec3<f32>) -> vec3<f32> {
    let s = params.star.xyz;
    let distance = length(s);
    let disc = params.star.w / max(distance, 1e-3);
    let angle = max(acos(clamp(dot(dir, s / max(distance, 1e-3)), -1.0, 1.0)), disc * 0.5);
    let g = disc / angle;
    return params.star_colour.rgb * params.star_colour.w * (g * g * 0.06 + g * 0.012);
}

@fragment
fn fs_space(in: Full) -> @location(0) vec4<f32> {
    let dir = ray_at(in.position.xy);
    var colour = sky_at(dir);
    colour += stars(dir, 60.0, 0.30, 1.3, 11, params.frame.z * 0.75);
    colour += stars(dir, 170.0, 0.10, 0.6, 23, params.frame.z * 0.75);
    colour += stars(dir, 400.0, 0.025, 0.4, 37, params.frame.z * 0.75);
    colour += glare(dir);
    return vec4<f32>(mix(colour, vec3<f32>(1.0), params.warp.y), 1.0);
}

// The starfield baked for the system: the same galaxy and stars as
// `fs_space`, into an equirectangular map in bands of rows, each star a texel
// wide (the map's texels are about 1.8 of the frame's pixels).
@fragment
fn fs_bake_stars(in: Full) -> @location(0) vec4<f32> {
    let uv = in.position.xy / vec2<f32>(4096.0, 2048.0);
    let lat = (0.5 - uv.y) * PI;
    let lon = (uv.x - 0.5) * TAU;
    let dir = vec3<f32>(cos(lat) * sin(lon), sin(lat), cos(lat) * cos(lon));
    // Half a texel wide, and dimmer by the area the texture magnifies them
    // to, so each star sends about the light the per-pixel stars did.
    let texel = TAU / 4096.0 * 0.5;
    let dim = 0.6;
    var colour = sky_at(dir);
    colour += stars(dir, 60.0, 0.30, 1.3 * dim, 11, texel);
    colour += stars(dir, 170.0, 0.10, 0.6 * dim, 23, texel);
    colour += stars(dir, 400.0, 0.025, 0.4 * dim, 37, texel);
    return vec4<f32>(colour, 1.0);
}

// The background from the baked starfield: one read, and the star's glare.
@fragment
fn fs_space_baked(in: Full) -> @location(0) vec4<f32> {
    let dir = ray_at(in.position.xy);
    let uv = vec2<f32>(0.5 + atan2(dir.x, dir.z) / TAU, 0.5 - asin(clamp(dir.y, -1.0, 1.0)) / PI);
    let colour = textureSampleLevel(starfield, map_sampler, uv, 0.0).rgb + glare(dir);
    return vec4<f32>(mix(colour, vec3<f32>(1.0), params.warp.y), 1.0);
}

// Streaks flying out from the heading: each lives in a cell of the screen's
// polar coordinates (angle around the heading, log of the distance from it),
// which scroll outward with time, so one cell lookup draws a moving streak.
fn streaks(theta: f32, logr: f32, count: f32, speed: f32, length_: f32, seed: i32) -> f32 {
    let x = vec2<f32>(theta / TAU * count, logr * 6.0 - params.warp.w * speed);
    let cell = vec2<i32>(floor(x));
    let h = hash3(vec3<i32>(cell, seed));
    if (h > 0.35) {
        return 0.0;
    }
    let f = x - vec2<f32>(cell);
    let along = hash3(vec3<i32>(cell, seed + 1));
    let across = abs(f.x - 0.5) * 2.0;
    let t = (f.y - along * (1.0 - length_)) / length_;
    let line = (1.0 - smoothstep01(0.0, 0.25, across)) * smoothstep01(0.0, 0.15, t) * (1.0 - smoothstep01(0.6, 1.0, t));
    return line * (0.4 + h * 1.7);
}

@fragment
fn fs_warp(in: Full) -> @location(0) vec4<f32> {
    let half = params.frame.x;
    let s = vec2<f32>((in.position.x - half) / half, (half - in.position.y) / half);
    let r = max(length(s), 1e-4);
    let theta = atan2(s.y, s.x);
    // Around the heading by cosine and sine, so noise has no seam where the
    // angle wraps.
    let around = vec2<f32>(cos(theta), sin(theta));
    let logr = log(r);
    let warp = params.warp.x;
    let t = params.warp.w;
    var colour = vec3<f32>(0.0);
    if (WARP == 1) {
        // Lumen Drift: blue-white streaks, a bright core, and broad sheets of
        // light fanning out to the sides.
        colour += vec3<f32>(0.55, 0.78, 1.0) * streaks(theta, logr, 240.0, 9.0, 0.8, 3) * smoothstep01(0.04, 0.35, r);
        colour += vec3<f32>(0.75, 0.88, 1.0) * streaks(theta, logr, 110.0, 6.5, 0.9, 7) * 0.7 * smoothstep01(0.1, 0.6, r);
        let sheet = exp(-abs(s.y) * 4.0 / (0.25 + r)) * (0.55 + 0.45 * noise(vec3<f32>(around * 3.0, logr * 2.0 - t * 1.5)));
        colour += vec3<f32>(0.34, 0.40, 0.90) * sheet * 0.9;
        colour += vec3<f32>(0.025, 0.035, 0.12) * (0.6 + 0.4 * noise(vec3<f32>(s * 2.0, t * 0.2)));
        colour += vec3<f32>(0.9, 0.95, 1.0) * exp(-r * 9.0) * 1.4 + vec3<f32>(0.4, 0.55, 1.0) * exp(-r * 3.0) * 0.4;
    } else if (WARP == 2) {
        // Tachyon Tunnel: rings rushing past with a twist, cyan and magenta.
        // Concentric rings flying outward, the tunnel's wall split into
        // panels that turn slowly as they pass.
        let rings = smoothstep01(0.8, 1.0, sin(logr * 16.0 - t * 11.0)) * smoothstep01(0.06, 0.45, r);
        let panels = 0.55 + 0.45 * smoothstep01(-0.2, 0.6, cos(theta * 8.0 + logr * 1.5 - t * 0.6));
        let hue = 0.5 + 0.5 * sin(logr * 3.0 - t * 2.0);
        colour += mix(vec3<f32>(0.1, 0.8, 1.0), vec3<f32>(1.0, 0.25, 0.85), hue) * rings * panels * 0.8;
        colour += vec3<f32>(0.6, 0.9, 1.0) * streaks(theta, logr, 160.0, 12.0, 0.8, 11) * 0.6;
        colour += vec3<f32>(0.02, 0.03, 0.08) * (1.0 + r) + vec3<f32>(0.8, 0.95, 1.0) * exp(-r * 12.0);
    } else if (WARP == 3) {
        // Fold Drive: space folding into a spiral, the sky bent around it.
        let twist = theta + logr * 2.5 - t * 0.8;
        let folds = 0.5 + 0.5 * sin(twist * 6.0);
        let bent = normalize(params.forward.xyz + (params.right.xyz * cos(twist) + params.up.xyz * sin(twist)) * r * params.frame.y);
        colour += sky_at(bent) * 6.0;
        colour += mix(vec3<f32>(0.55, 0.25, 0.9), vec3<f32>(1.0, 0.75, 0.3), folds) * smoothstep01(0.6, 1.0, folds) * smoothstep01(0.05, 0.7, r) * 0.45;
        colour += vec3<f32>(1.0, 0.9, 0.7) * exp(-r * 7.0);
    } else if (WARP == 4) {
        // Ion Slipstream: plasma curtains streaming by, teal and green.
        // Thin filaments where the noise crosses zero, drawn out along the
        // direction of flight.
        let flow = noise(vec3<f32>(around * 7.0, logr * 0.9 - t * 1.4));
        let filament = (1.0 - smoothstep01(0.0, 0.1, abs(flow))) * smoothstep01(0.1, 0.7, r);
        let glow = (1.0 - smoothstep01(0.0, 0.45, abs(flow))) * smoothstep01(0.1, 0.9, r) * 0.25;
        let tint = mix(vec3<f32>(0.05, 0.95, 0.6), vec3<f32>(0.2, 0.55, 1.0), 0.5 + 0.5 * noise(vec3<f32>(around * 2.0, t * 0.3)));
        colour += tint * (filament + glow) * 0.8;
        colour += vec3<f32>(0.6, 1.0, 0.9) * streaks(theta, logr, 120.0, 8.0, 0.4, 19) * 0.5;
        colour += vec3<f32>(0.7, 1.0, 0.95) * exp(-r * 10.0) * 0.8;
    } else {
        // Graviton Spiral: spiral arms around a dark core, like a disc of
        // infalling matter.
        let arm = sin(theta * 2.0 - logr * 5.0 - t * 3.0);
        let disc = smoothstep01(0.3, 1.0, arm) * smoothstep01(0.12, 0.25, r) * exp(-r * 1.2);
        let heat = noise(vec3<f32>(around * 4.0, logr * 4.0 - t * 2.0)) * 0.5 + 0.5;
        colour += mix(vec3<f32>(1.0, 0.35, 0.08), vec3<f32>(1.0, 0.85, 0.5), heat) * disc * 1.2;
        colour += vec3<f32>(1.0, 0.8, 0.55) * exp(-pow((r - 0.14) / 0.02, 2.0)) * 1.5;
        colour *= smoothstep01(0.08, 0.13, r);
        colour += vec3<f32>(1.0, 0.6, 0.4) * streaks(theta, logr, 80.0, 7.0, 0.5, 23) * 0.4 * smoothstep01(0.2, 0.6, r);
    }
    // In and out of the jump, the calm sky fades under the drive's light.
    let dir = ray_at(in.position.xy);
    let calm = (sky_at(dir) + stars(dir, 60.0, 0.30, 1.3, 11, params.frame.z * 0.75)) * (1.0 - warp);
    return vec4<f32>(mix(calm + colour * warp, vec3<f32>(1.0), params.warp.y), 1.0);
}

// ---------------------------------------------------------------------------
// Bodies.
// ---------------------------------------------------------------------------

struct Quad {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) body: u32,
};

// Each body is drawn as a fan of triangles round its silhouette, so pixels
// past its disc are not shaded at all: the circle where the cone of rays
// touching it (`air.w` round its centre: its air or rings included) meets the
// plane through its centre, as a polygon just outside it, projected. Close to
// the camera, where that polygon could reach behind it, the CPU's rectangle
// instead (the first two triangles; the rest collapse). At its depth rank
// (`ring_colour.w`) for the depth mask.
const SIDES: u32 = 16u;

fn project(p: vec3<f32>) -> vec2<f32> {
    let z = dot(p, params.forward.xyz);
    return vec2<f32>(dot(p, params.right.xyz), dot(p, params.up.xyz)) / (z * params.frame.y);
}

@vertex
fn vs_body(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Quad {
    var out: Quad;
    out.body = instance;
    let depth = params.bodies[instance].ring_colour.w;
    let c = params.bodies[instance].place.xyz;
    let bound = params.bodies[instance].air.w;
    let distance = length(c);
    let ahead = dot(c, params.forward.xyz);
    let angle = asin(clamp(bound / max(distance, 1e-6), 0.0, 1.0));
    // Out to the polygon's corners, and three pixels more for the soft edge.
    let spread = distance * tan(angle) / cos(PI / f32(SIDES)) + 3.0 * params.frame.z * distance;
    if (distance <= bound * 1.02 || ahead <= spread * 1.05) {
        let rect = params.bodies[instance].rect;
        var corners = array<vec2<f32>, 6>(
            vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
            vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        );
        let p = select(mix(rect.xy, rect.zw, corners[min(vertex, 5u)]), rect.xy, vertex >= 6u);
        out.position = vec4<f32>(p, depth, 1.0);
        return out;
    }
    let corner = vertex % 3u;
    var p = project(c);
    if (corner != 0u) {
        let axis = c / distance;
        let side = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(axis.y) < 0.9);
        let e1 = normalize(cross(axis, side));
        let e2 = cross(axis, e1);
        let a = f32(vertex / 3u + corner - 1u) * TAU / f32(SIDES);
        p = project(c + (e1 * cos(a) + e2 * sin(a)) * spread);
    }
    out.position = vec4<f32>(p, depth, 1.0);
    return out;
}

// The depth mask: where the body's solid disc fully covers the pixel.
@fragment
fn fs_mask(in: Quad) -> @location(0) vec4<f32> {
    let h = trace(in.body, in.position.xy);
    if (h.cover < 0.999) {
        discard;
    }
    return vec4<f32>(0.0);
}

// For the Pi estimate: shades nothing; counts the pixels a draw reaches.
@fragment
fn fs_probe(in: Quad) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

@fragment
fn fs_probe_full(in: Full) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

fn map_uv(n: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(0.5 + atan2(n.x, n.z) / TAU, 0.5 - asin(clamp(n.y, -1.0, 1.0)) / PI);
}

fn unwrap(g: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(g.x - round(g.x), g.y);
}

// The mip level a map of `size` texels needs at `uv`.
fn level(uv: vec2<f32>, size: vec2<f32>) -> f32 {
    let gx = unwrap(dpdx(uv)) * size;
    let gy = unwrap(dpdy(uv)) * size;
    return 0.5 * log2(max(max(dot(gx, gx), dot(gy, gy)), 1e-8)) + BIAS;
}

fn to_body(i: u32, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(params.bodies[i].rot[0].xyz, v), dot(params.bodies[i].rot[1].xyz, v), dot(params.bodies[i].rot[2].xyz, v));
}

fn from_body(i: u32, v: vec3<f32>) -> vec3<f32> {
    return params.bodies[i].rot[0].xyz * v.x + params.bodies[i].rot[1].xyz * v.y + params.bodies[i].rot[2].xyz * v.z;
}

// Bodies are read field by field from the uniform buffer, by index, never
// copied whole: a copy of a body's forty floats held registers enough to
// cost every body pipeline two threads.

// Where a pixel's ray meets a body, or comes closest to it.
struct Hit {
    dir: vec3<f32>,
    point: vec3<f32>,
    // Unit normal at the hit, or toward the closest approach.
    n: vec3<f32>,
    // Closest approach to the centre, in radii; whether it hit.
    closest: f32,
    hit: bool,
    distance: f32,
    // How much of the pixel the body covers.
    cover: f32,
};

fn trace(i: u32, position: vec2<f32>) -> Hit {
    return trace_shell(i, position, params.bodies[i].place.w);
}

// The same against a shell of `radius` round the body's centre.
fn trace_shell(i: u32, position: vec2<f32>, radius: f32) -> Hit {
    var h: Hit;
    h.dir = ray_at(position);
    let c = params.bodies[i].place.xyz;
    let along = max(dot(c, h.dir), 0.0);
    let nearest = h.dir * along;
    h.closest = length(nearest - c) / radius;
    let inside = 1.0 - h.closest * h.closest;
    h.hit = inside > 0.0;
    let t = along - sqrt(max(inside, 0.0)) * radius;
    h.point = select(nearest, h.dir * t, h.hit);
    h.n = normalize(h.point - c);
    h.distance = max(select(along, t, h.hit), 1e-4);
    let edge = (h.closest - 1.0) * radius / (h.distance * params.frame.z);
    h.cover = smoothstep01(-0.75, 0.75, -edge);
    return h;
}

// How much of the star other bodies hide from a point: eclipses, with a soft
// penumbra.
fn eclipse(self_index: u32, point: vec3<f32>, to_star: vec3<f32>) -> f32 {
    var lit = 1.0;
    for (var k = 0u; k < 4u; k++) {
        let index = params.bodies[self_index].shade[k];
        if (index < 0.0) {
            break;
        }
        let other = params.bodies[u32(index)].place;
        let rel = other.xyz - point;
        let along = dot(rel, to_star);
        if (along <= 0.0) {
            continue;
        }
        let miss = length(rel - to_star * along);
        let spread = along * params.star.w / max(length(params.star.xyz - point), 1.0);
        lit *= smoothstep01(other.w - spread, other.w + spread, miss);
    }
    return lit;
}

// Ring density at radius `r` (in planet radii): ringlets, gaps and a main
// division, from the profile row the planet's rings were painted into.
fn ring_density(r: f32, i: u32) -> f32 {
    let x = (r - params.bodies[i].ring.x) / (params.bodies[i].ring.y - params.bodies[i].ring.x);
    if (x < 0.0 || x > 1.0) {
        return 0.0;
    }
    let row = (params.bodies[i].ring.w + 0.5) / f32(textureDimensions(ring_profile, 0).y);
    return textureSampleLevel(ring_profile, map_sampler, vec2<f32>(x, row), 0.0).r;
}

// The ring's shadow on a planet's point.
fn ring_shadow(i: u32, point: vec3<f32>, sun: vec3<f32>) -> f32 {
    if (params.bodies[i].ring.z <= 0.0) {
        return 1.0;
    }
    let axis = params.bodies[i].rot[1].xyz;
    let denom = dot(sun, axis);
    if (abs(denom) < 1e-4) {
        return 1.0;
    }
    let s = dot(params.bodies[i].place.xyz - point, axis) / denom;
    if (s <= 0.0) {
        return 1.0;
    }
    return 1.0 - ring_density(length(point + sun * s - params.bodies[i].place.xyz) / params.bodies[i].place.w, i) * params.bodies[i].ring.z * 0.85;
}

fn star_light() -> vec3<f32> {
    return params.light.rgb * 1.25;
}

// The sky's blue, lit by the star: greyer under a red one.
fn skylight() -> vec3<f32> {
    return SKYLIGHT * params.light.rgb;
}

fn has_air(i: u32) -> bool {
    return max(max(params.bodies[i].air.r, params.bodies[i].air.g), params.bodies[i].air.b) > 0.0;
}

// Sunlight reaching the ground where the star stands at `mu`, through the air.
fn ground_light(i: u32, mu: f32) -> vec3<f32> {
    if (has_air(i)) {
        return star_light() * exp(-params.bodies[i].air.rgb * airmass(mu)) * smoothstep01(-0.035, 0.02, mu);
    }
    return star_light() * smoothstep01(-0.03, 0.03, mu);
}

// The air's transmittance toward the viewer.
fn through(i: u32, h: Hit) -> vec3<f32> {
    if (has_air(i)) {
        return exp(-params.bodies[i].air.rgb * airmass(clamp(dot(h.n, -h.dir), 0.0, 1.0)));
    }
    return vec3<f32>(1.0);
}

fn day_texel(uv: vec2<f32>, lod: f32) -> vec3<f32> {
    if (SLOT == 0) {
        return textureSampleLevel(a_day, map_sampler, uv, lod).rgb;
    }
    return textureSampleLevel(b_day, map_sampler, uv, lod).rgb;
}

fn relief_texel(uv: vec2<f32>, lod: f32) -> vec4<f32> {
    if (SLOT == 0) {
        return textureSampleLevel(a_relief, map_sampler, uv, lod);
    }
    return textureSampleLevel(b_relief, map_sampler, uv, lod);
}

fn extra_texel(uv: vec2<f32>, lod: f32) -> vec2<f32> {
    if (SLOT == 0) {
        return textureSampleLevel(a_extra, map_sampler, uv, lod).rg;
    }
    return textureSampleLevel(b_extra, map_sampler, uv, lod).rg;
}

fn map_size() -> vec2<f32> {
    if (SLOT == 0) {
        return vec2<f32>(textureDimensions(a_day, 0));
    }
    return vec2<f32>(textureDimensions(b_day, 0));
}

// The ground's tangent frame in the world, from the body-frame normal.
fn east_of(i: u32, nb: vec3<f32>) -> vec3<f32> {
    let cos_lat = sqrt(max(nb.x * nb.x + nb.z * nb.z, 1e-8));
    return from_body(i, vec3<f32>(nb.z, 0.0, -nb.x) / cos_lat);
}

// How far the clouds' shadows fall, in map coordinates.
fn shadow_shift(i: u32, nb: vec3<f32>, n: vec3<f32>, sun: vec3<f32>, mu_sun: f32) -> vec2<f32> {
    let east = east_of(i, nb);
    let cos_lat = sqrt(max(nb.x * nb.x + nb.z * nb.z, 1e-8));
    let reach = 0.006 / max(mu_sun, 0.12);
    return vec2<f32>(dot(sun, east) * reach / (TAU * max(cos_lat, 0.05)), -dot(sun, cross(n, east)) * reach / PI);
}

@fragment
fn fs_star(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let mu = sqrt(max(1.0 - h.closest * h.closest, 0.0));
        // look.yzw: the convection cells' scale and contrast (a red giant's few
        // huge ones, a red dwarf's spots), and how much the limb darkens.
        let style = params.bodies[i].look.yzw;
        let granules = 1.0 + style.y * noise(to_body(i, h.n) * style.x + vec3<f32>(params.warp.z * 0.05));
        // Its own colour, deepened, not blown out: a red giant's face is orange
        // and mottled, a white dwarf's searing white.
        let tint = params.star_colour.rgb / max(max(params.star_colour.r, params.star_colour.g), params.star_colour.b);
        let light = min(tint * tint * 1.15 * (1.0 - style.z + style.z * mu) * granules, vec3<f32>(1.0));
        return vec4<f32>(light * h.cover, h.cover);
    }
    discard;
    return vec4<f32>(0.0);
}

// A planet whose maps are not in yet: its colour, banded on giants, lit.
@fragment
fn fs_flat(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let nb = to_body(i, h.n);
        let sun = normalize(params.star.xyz - h.point);
        let mu = dot(h.n, sun);
        let albedo = params.bodies[i].colour.rgb * (1.0 + params.bodies[i].colour.w * sin(nb.y * 23.0 + sin(nb.y * 7.0) * 2.0) * 0.5);
        let light = ground_light(i, mu) * max(mu + 0.03, 0.0);
        return vec4<f32>(albedo * light * through(i, h) * h.cover, h.cover);
    }
    discard;
    return vec4<f32>(0.0);
}

// A mapped planet's ground: its colour, relief and the clouds' shadows, lit
// through its air, in eclipses and the rings' shadow.
@fragment
fn fs_ground(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let nb = to_body(i, h.n);
    let uv = map_uv(nb);
    let lod = level(uv, map_size());
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let n = h.n;
        let sun = normalize(params.star.xyz - h.point);
        let mu_sun = dot(n, sun);
        let east = east_of(i, nb);
        let relief = relief_texel(uv, lod + 0.4);
        let tilted = normalize(n + (east * (relief.r - 0.5) + cross(n, east) * (relief.g - 0.5)) * 0.55);
        let sky = select(vec3<f32>(0.0), skylight() * smoothstep01(-0.15, 0.4, mu_sun), has_air(i));
        var albedo = day_texel(uv, lod) * mix(1.0, 0.8, relief.b);
        let kind = i32(params.bodies[i].look.x);
        if (kind == 6 || kind == 7 || kind == 14) {
            // Giants darken toward the limb.
            albedo *= 0.55 + 0.45 * sqrt(clamp(dot(n, -h.dir), 0.0, 1.0));
        }
        let colour = albedo * (ground_light(i, mu_sun) * max(dot(tilted, sun) + 0.03, 0.0) + sky);
        return vec4<f32>(colour * through(i, h) * h.cover, h.cover);
    }
    discard;
    return vec4<f32>(0.0);
}

// How much close-up detail a mapped world's point gets: none until a pixel
// covers less than a texel of its map, so only low orbits pay for it.
fn closeness(i: u32, h: Hit) -> f32 {
    let footprint = length(h.point) * params.frame.z;
    let texel = params.bodies[i].place.w * TAU / map_size().x;
    return smoothstep01(texel * 1.5, texel * 0.2, footprint) * step(0.999, h.cover);
}

// Close-up detail over a mapped world's ground, multiplied in (as half the
// factor: the blend doubles it): the light that slopes finer than the map's
// texels catch, and a grain. One fetch of the noise volume: a second (a finer
// octave, or the sea's mask) would halve the threads it runs with.
@fragment
fn fs_detail(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let close = closeness(i, h);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(close <= 0.0)) {
        // All in the body's frame, so nothing turns back to the world's. No mask
        // for the sea: a second fetch halves the threads, and under the sea the
        // ground is near black, its glint and the sky's sheen added after this.
        let nb = to_body(i, h.n);
        let a = noised(nb * 1400.0);
        // The star is far enough that its direction from the body's centre does
        // for every point: no need to keep the hit point.
        let sun = to_body(i, normalize(params.star.xyz - params.bodies[i].place.xyz));
        let mu = dot(nb, sun);
        // The sun's height over the ground tilted by the slope, to first order.
        let tilted = mu - 0.42 * close * (dot(a.yzw, sun) - dot(a.yzw, nb) * mu);
        let shading = clamp((max(tilted, 0.0) + 0.03) / (max(mu, 0.0) + 0.03), 0.6, 1.6);
        let grain = 1.0 + a.x * 0.168 * close;
        return vec4<f32>(vec3<f32>(0.5 * grain * mix(1.0, shading, smoothstep01(-0.05, 0.1, mu))), 1.0);
    }
    discard;
    return vec4<f32>(0.0);
}


// The cloud deck over a mapped world, and the shadows it casts on the ground
// below, in one pass. One look at the map a little toward the sun serves
// both: the cloud there shades this cloud's far side (tops facing the sun
// bright, towers' far sides grey) and casts its shadow on the ground here.
// The shadow rides in the alpha: blended over, the ground below keeps
// (1 - cloud) x (1 - shadow) of itself. Thin edges glow when the star is
// behind them, and the deck is more opaque toward the limb, where the view
// crosses more of it.
@fragment
fn fs_clouds(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let nb = to_body(i, h.n);
    let uv = map_uv(nb);
    let cuv = vec2<f32>(uv.x + params.bodies[i].rot[0].w, uv.y);
    let lod = level(cuv, map_size());
    let cloud = extra_texel(cuv, lod).g;
    let n = h.n;
    let sun = normalize(params.star.xyz - h.point);
    let mu_sun = dot(n, sun);
    var toward = 0.0;
    if (mu_sun > -0.05) {
        toward = extra_texel(cuv + shadow_shift(i, nb, n, sun, mu_sun), lod + 1.0).g;
    }
    let shadow = 0.6 * toward * sunlit(mu_sun) * h.cover;
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(cloud * h.cover <= 0.002 && shadow <= 0.002)) {
        if (cloud * h.cover <= 0.002) {
            // Clear sky with a cloud's shadow on the ground below.
            return vec4<f32>(0.0, 0.0, 0.0, shadow);
        }
        var side = 1.0;
        if (mu_sun > -0.05) {
            side = clamp(1.0 - 2.0 * (toward - cloud), 0.4, 1.3);
        }
        var sun_high = star_light();
        if (has_air(i)) {
            sun_high = star_light() * exp(-params.bodies[i].air.rgb * airmass(mu_sun + 0.04) * 0.55);
        }
        // Forward scattering, a cheap stand-in for a Henyey-Greenstein lobe:
        // thin cloud between the camera and the star lights up.
        let toward_star = max(dot(h.dir, sun), 0.0);
        let lobe = toward_star * toward_star * toward_star * toward_star;
        let glow = (1.0 - cloud) * (0.2 + 1.2 * lobe * lobe);
        let sky = select(vec3<f32>(0.0), skylight() * smoothstep01(-0.15, 0.4, mu_sun), has_air(i));
        let lit = 0.85 * (sun_high * smoothstep01(-0.08, 0.04, mu_sun) * ((0.30 + 0.70 * sqrt(max(mu_sun, 0.0))) * side + glow) + sky * 1.5);
        // Toward the limb the view crosses more of the layer: more opaque.
        let thick = min(cloud * (2.2 - 1.2 * clamp(dot(n, -h.dir), 0.0, 1.0)), 1.0) * h.cover;
        return vec4<f32>(lit * through(i, h) * thick, 1.0 - (1.0 - thick) * (1.0 - shadow));
    }
    discard;
    return vec4<f32>(0.0);
}

// A stacked cloud layer, over the deck in low orbit: a shell a little higher
// for each LAYER, showing only the clouds dense enough to reach it. Seen
// from low down the shells slide past each other as the camera moves, so the
// clouds stand up from the ground. One fetch.
@fragment
fn fs_cloud_layer(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let lift = f32(LAYER) * 0.005;
    let h = trace_shell(i, in.position.xy, params.bodies[i].place.w * (1.0 + lift));
    let nb = to_body(i, h.n);
    let uv = map_uv(nb);
    let cuv = vec2<f32>(uv.x + params.bodies[i].rot[0].w, uv.y);
    let cloud = extra_texel(cuv, level(cuv, map_size())).g;
    // The denser the cloud, the higher it reaches.
    let reach = f32(LAYER) * 0.35;
    let alpha = smoothstep01(reach, reach + 0.25, cloud) * 0.75 * h.cover;
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(alpha <= 0.002)) {
        let sun = normalize(params.star.xyz - h.point);
        let mu_sun = dot(h.n, sun);
        var sun_high = star_light();
        if (has_air(i)) {
            sun_high = star_light() * exp(-params.bodies[i].air.rgb * airmass(mu_sun + 0.04) * 0.5);
        }
        // Higher tops catch more light; their sides at low sun stay grey.
        let lit = 0.85 * sun_high * smoothstep01(-0.1, 0.04, mu_sun)
            * (0.35 + 0.65 * sqrt(max(mu_sun + 0.05, 0.0))) * (0.92 + 0.08 * f32(LAYER))
            + skylight() * 1.2 * smoothstep01(-0.15, 0.4, mu_sun);
        return vec4<f32>(lit * through(i, h) * alpha, alpha);
    }
    discard;
    return vec4<f32>(0.0);
}

// Added over a mapped world: city lights or molten glow, the sea's glint
// and the sky's sheen on it (`SEA`), and the blue the air in front scatters
// toward the viewer. One pass for all the added light, after the clouds,
// so it shares one trace: the sea's light is held back under cloud itself.
override SEA: bool = false;

@fragment
fn fs_glow(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let nb = to_body(i, h.n);
    let uv = map_uv(nb);
    let lod = level(uv, map_size());
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let n = h.n;
        let sun = normalize(params.star.xyz - h.point);
        let mu_sun = dot(n, sun);
        var colour = vec3<f32>(0.0);
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu_sun);
        // The cloud over the point, read once: it holds back both the city
        // lights and the sea's light. The sea first, the air's transmittance
        // last, so few values are live at once (the other order halved the
        // threads).
        let cloud = extra_texel(vec2<f32>(uv.x + params.bodies[i].rot[0].w, uv.y), lod).g;
        if (SEA) {
            let water = relief_texel(uv, lod + 0.4).b * (1.0 - cloud);
            if (water > 0.0) {
                let fresnel = 0.02 + 0.98 * pow(1.0 - clamp(dot(n, -h.dir), 0.0, 1.0), 5.0);
                let log_nh = log2(max(dot(n, normalize(sun - h.dir)), 0.0));
                let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
                colour = (ground_light(i, mu_sun) * glint * (0.25 + fresnel)
                    + skylight() * 2.0 * fresnel * smoothstep01(-0.1, 0.3, mu_sun)) * water;
            }
        }
        if (params.bodies[i].look.w > 0.5 && params.bodies[i].look.w < 1.5 && darkness > 0.002) {
            let night = extra_texel(uv, lod + 0.6).r;
            let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, night));
            colour += tint * night * night * 2.2 * darkness * (1.0 - cloud * 0.85);
        } else if (params.bodies[i].look.w > 1.5) {
            // Molten rock glows on both sides, more visibly in the dark.
            let night = extra_texel(uv, lod + 0.6).r;
            let heat = mix(vec3<f32>(1.0, 0.25, 0.04), vec3<f32>(1.0, 0.75, 0.3), night * night);
            colour += heat * night * (0.15 + 0.55 * darkness);
        }
        let air = through(i, h);
        colour *= air;
        if (has_air(i)) {
            let sun_high = star_light() * exp(-params.bodies[i].air.rgb * airmass(mu_sun + 0.04) * 0.55);
            colour += sun_high * smoothstep01(-0.10, 0.08, mu_sun) * (vec3<f32>(1.0) - air) * 0.45;
        }
        return vec4<f32>(colour * h.cover, 0.0);
    }
    discard;
    return vec4<f32>(0.0);
}

// A moon, from its layer of the slot's moon maps: colour, relief, molten glow.
@fragment
fn fs_moon(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let nb = to_body(i, h.n);
    let uv = map_uv(nb);
    let lod = level(uv, vec2<f32>(512.0, 256.0));
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let layer = i32(params.bodies[i].look.z);
        var day: vec3<f32>;
        var relief: vec4<f32>;
        if (SLOT == 0) {
            day = textureSampleLevel(a_moon_day, map_sampler, uv, layer, lod).rgb;
            relief = textureSampleLevel(a_moon_relief, map_sampler, uv, layer, lod);
        } else {
            day = textureSampleLevel(b_moon_day, map_sampler, uv, layer, lod).rgb;
            relief = textureSampleLevel(b_moon_relief, map_sampler, uv, layer, lod);
        }
        let n = h.n;
        let sun = normalize(params.star.xyz - h.point);
        let mu_sun = dot(n, sun);
        let east = east_of(i, nb);
        let tilted = normalize(n + (east * (relief.r - 0.5) + cross(n, east) * (relief.g - 0.5)) * 0.55);
        var colour = day * ground_light(i, mu_sun) * max(dot(tilted, sun) + 0.03, 0.0);
        // A moon's third relief channel is molten glow.
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu_sun);
        colour += mix(vec3<f32>(1.0, 0.25, 0.04), vec3<f32>(1.0, 0.75, 0.3), relief.b) * relief.b * (0.15 + 0.55 * darkness);
        return vec4<f32>(colour * through(i, h) * h.cover, h.cover);
    }
    discard;
    return vec4<f32>(0.0);
}

// The air seen edge-on past a body's limb, added.
@fragment
fn fs_limb(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let r = h.closest;
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(r >= ATMOSPHERE || h.cover >= 1.0)) {
        let air = params.bodies[i].air.rgb;
        let height = max(r - 1.0, 0.0);
        let depth = air * sqrt(TAU * r / SCALE_HEIGHT) * exp(-height / SCALE_HEIGHT);
        let sun = normalize(params.star.xyz - h.point);
        let mu = dot(h.n, sun);
        let lit = star_light() * exp(-air * airmass(mu + 0.08) * 0.6) * smoothstep01(-0.30, 0.05, mu) * eclipse(in.body, h.point, sun);
        let glow = lit * (vec3<f32>(1.0) - exp(-depth)) * 0.9;
        return vec4<f32>(glow * (1.0 - h.cover), 0.0);
    }
    discard;
    return vec4<f32>(0.0);
}

// Rings, over their planet: where the ray crosses the equatorial plane in
// front of the planet or past it, lit from either face, in the planet's shadow.
@fragment
fn fs_ring(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    let c = params.bodies[i].place.xyz;
    let axis = params.bodies[i].rot[1].xyz;
    let denom = dot(h.dir, axis);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(abs(denom) < 1e-5)) {
        let t = dot(c, axis) / denom;
        // Behind the planet's disc, the planet hides it.
        if (t <= 0.0 || (h.hit && t > h.distance)) {
            discard;
        }
        let p = h.dir * t - c;
        let density = ring_density(length(p) / params.bodies[i].place.w, i) * params.bodies[i].ring.z;
        if (density <= 0.001) {
            discard;
        }
        let sun = normalize(params.star.xyz - (c + p));
        let along = dot(-p, sun);
        let miss = length(-p - sun * along);
        let shadowed = select(1.0, smoothstep01(params.bodies[i].place.w * 0.97, params.bodies[i].place.w * 1.03, miss), along > 0.0);
        let light = star_light() * (0.25 + 0.75 * sqrt(abs(dot(sun, axis)))) * shadowed;
        let colour = params.bodies[i].ring_colour.rgb * (light + skylight() * 0.3);
        return vec4<f32>(colour * density, density);
    }
    discard;
    return vec4<f32>(0.0);
}

// How much of a pixel's light comes straight from the star rather than the
// sky or the body's own glow: where shadows darken it.
fn sunlit(mu: f32) -> f32 {
    return smoothstep01(-0.05, 0.12, mu);
}

// Multiplied into a body: other bodies passing in front of the star.
@fragment
fn fs_eclipse(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let sun = normalize(params.star.xyz - h.point);
        let lit = eclipse(i, h.point, sun);
        let factor = mix(1.0, mix(1.0, lit, sunlit(dot(h.n, sun))), h.cover);
        return vec4<f32>(vec3<f32>(factor), 1.0);
    }
    discard;
    return vec4<f32>(0.0);
}

// Multiplied into a ringed planet: its rings' shadow.
@fragment
fn fs_ring_shadow(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.cover <= 0.0)) {
        let sun = normalize(params.star.xyz - h.point);
        let factor = mix(1.0, mix(1.0, ring_shadow(i, h.point, sun), sunlit(dot(h.n, sun))), h.cover);
        return vec4<f32>(vec3<f32>(factor), 1.0);
    }
    discard;
    return vec4<f32>(0.0);
}
