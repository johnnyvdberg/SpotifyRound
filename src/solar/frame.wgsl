// The solar tour's frame. Prepended with `noise.wgsl` and the noise volume's
// declarations (`noise3d.rs`).
//
// Many small pipelines, drawn one after another in a single render pass, each
// within the V3D's four-thread register budget; the tile buffer blends them on
// chip. They write linear light, premultiplied, through an sRGB view.
//
// - First the depth mask: each body's solid disc at its distance rank, so
//   everything after skips, before shading, whatever a nearer body hides;
//   the background sits at the far end of it. A polygon inscribed in the
//   silhouette for a body wholly ahead (`vs_mask_inscribed`, nothing per
//   pixel), else traced (`fs_mask`).
// - The background, full screen: `fs_space_baked` (the galaxy and stars,
//   baked into a cube map once a system by `fs_bake_stars`, read along the
//   interpolated ray, and nothing else; `fs_space_flash` while the jump's
//   flash whitens it), or `fs_space` (the same and the star's glare, worked
//   out per pixel) until the bake is done, or during a hyperjump the drive's
//   light, one pipeline per drive (`WARP`, `fs_warp`).
// - Over the baked background, the star's glare: a polygon round the star
//   (`vs_glare`, or the whole frame in a flyby, `vs_glare_full`), added at
//   the far end of the depth range so every body's mask hides it.
// - Then the bodies, far to near. The star, every planet and moon as a mesh
//   of its own latitude-longitude grid (`vs_globe`), lit at its vertices:
//   `fs_globe_star`; `fs_globe_flat` for a planet whose maps are not in
//   yet; a mapped world from afar, `fs_globe` (and `fs_globe_glow` for one
//   with seas), built for each sort of world (`SurfaceLook` in `mod.rs`)
//   with its rings' shadow; `fs_globe_moon` for a moon. A mapped world
//   close up is a fan of triangles round its own silhouette (`vs_body`),
//   shaders branching rather than discarding so the V3D skips their work for
//   pixel groups with nothing to draw: `fs_ground`, `fs_detail`, `fs_clouds`
//   and `fs_glow`. Then the air past a
//   limb (added) as an annulus round the silhouette lit at its vertices
//   (`vs_limb_ring`, `fs_limb_ring`) for a body wholly
//   ahead, else traced over the fan (`fs_limb`); and the rings as an
//   annulus in their plane (`vs_ring`, `fs_ring_annulus`). The mapped ones
//   are built per map slot (`SLOT`), so each reads one set of textures.
// - Shadows multiplied into what is drawn: `fs_eclipse` (only when another
//   body is near the line to the star) and, close up, `fs_ring_shadow`.

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
    // The patch of its latitude-longitude grid a mapped world is drawn as
    // (`vs_globe`): longitude and latitude of its corner, then their spans,
    // in radians, from the CPU.
    window: vec4<f32>,
    // x: the grid's cells along each side; y: how far the mesh stands proud
    // of the surface, in radii, so the soft edge has pixels to fade over.
    mesh: vec4<f32>,
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
    // An engine burn: x the throttle 0..1, y the seconds, for its flicker.
    burn: vec4<f32>,
    // The dust drifting past the ship: xyz how far its field has moved, in
    // its box's units; w how long a streak is in them.
    dust: vec4<f32>,
    // xyz: the way the ship is going, in the world.
    motion: vec4<f32>,
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
@group(0) @binding(14) var starfield: texture_cube<f32>;
// The galaxy alone, small and soft (`fs_bake_sky`).
@group(0) @binding(15) var sky_cube: texture_cube<f32>;

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

// x squared: `pow(x, 2.0)` is undefined for negative x in WGSL.
fn sq(x: f32) -> f32 {
    return x * x;
}

// A smooth bump: 1 at 0, falling to 0 at |x| = 2.
fn bump(x: f32) -> f32 {
    let a = clamp(1.0 - x * x * 0.25, 0.0, 1.0);
    return a * a;
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
    // The ray through the pixel, not normalized: straight across the
    // screen, so interpolated from the corners instead of worked out per
    // pixel. For the starfield bake, the cube face.
    @location(0) dir: vec3<f32>,
    @location(1) @interpolate(flat) face: u32,
};

@vertex
fn vs_background(@builtin(vertex_index) index: u32) -> Full {
    return full_vertex(index);
}

fn full_vertex(index: u32) -> Full {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: Full;
    // At the far end of the depth mask: hidden wherever a body's disc is.
    out.position = vec4<f32>(corners[index], 1.0, 1.0);
    let s = corners[index];
    out.dir = params.forward.xyz + (params.right.xyz * s.x + params.up.xyz * s.y) * params.frame.y;
    return out;
}

// The starfield bake: a cube face (the instance) at a time.
@vertex
fn vs_bake(@builtin(vertex_index) index: u32, @builtin(instance_index) face: u32) -> Full {
    var out = full_vertex(index);
    out.face = face;
    return out;
}

// The direction through texel `uv` (0..1, down the rows) of cube face `face`
// (+x, -x, +y, -y, +z, -z), as the sampler reads it.
fn cube_dir(face: u32, uv: vec2<f32>) -> vec3<f32> {
    let a = uv.x * 2.0 - 1.0;
    let b = uv.y * 2.0 - 1.0;
    var d: vec3<f32>;
    switch face {
        case 0u: { d = vec3<f32>(1.0, -b, -a); }
        case 1u: { d = vec3<f32>(-1.0, -b, a); }
        case 2u: { d = vec3<f32>(a, 1.0, b); }
        case 3u: { d = vec3<f32>(a, -1.0, -b); }
        case 4u: { d = vec3<f32>(a, -b, 1.0); }
        default: { d = vec3<f32>(-a, -b, -1.0); }
    }
    return normalize(d);
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

// The same, the angle from the star taken as the chord to it: no arc cosine,
// and within a few percent where the glare is bright enough to see.
fn glare_near(dir: vec3<f32>) -> vec3<f32> {
    let s = params.star.xyz;
    let distance = length(s);
    let disc = params.star.w / max(distance, 1e-3);
    let angle = max(length(dir - s / max(distance, 1e-3)), disc * 0.5);
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
// `fs_space`, into a cube map a face and a band of rows at a time, each star
// a texel wide (the faces' texels are about 1.8 of the frame's pixels at
// their middle).
@fragment
fn fs_bake_stars(in: Full) -> @location(0) vec4<f32> {
    let dir = cube_dir(in.face, in.position.xy / f32(textureDimensions(starfield).x));
    // Half a texel wide, and dimmer by the area the texture magnifies them
    // to, so each star sends about the light the per-pixel stars did.
    let texel = PI * 0.5 / f32(textureDimensions(starfield).x) * 0.5;
    let dim = 0.6;
    var colour = sky_at(dir);
    colour += stars(dir, 60.0, 0.30, 1.3 * dim, 11, texel);
    colour += stars(dir, 170.0, 0.10, 0.6 * dim, 23, texel);
    colour += stars(dir, 400.0, 0.025, 0.4 * dim, 37, texel);
    return vec4<f32>(colour, 1.0);
}

// The galaxy alone into a small cube map, for the drives that smear it.
@fragment
fn fs_bake_sky(in: Full) -> @location(0) vec4<f32> {
    return vec4<f32>(sky_at(cube_dir(in.face, in.position.xy / f32(textureDimensions(sky_cube).x))), 1.0);
}

// The background from the baked starfield: one read along the interpolated
// ray and nothing else. The star's glare is a polygon of its own round the
// star (`vs_glare`), the jump's flash a pipeline of its own
// (`fs_space_flash`).
@fragment
fn fs_space_baked(in: Full) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(starfield, map_sampler, in.dir, 0.0).rgb, 1.0);
}

// The same, whitened by the flash as a jump drops out (`warp.y`), only
// while it is.
@fragment
fn fs_space_flash(in: Full) -> @location(0) vec4<f32> {
    let colour = textureSampleLevel(starfield, map_sampler, in.dir, 0.0).rgb;
    return vec4<f32>(mix(colour, vec3<f32>(1.0), params.warp.y), 1.0);
}

// The star's glare, added over the background as a polygon round the star
// out to where it falls under half a step of the 8-bit frame, with that
// floor taken off so its rim does not show. At the far end of the depth
// range, like the background, so every body's mask hides it. Over the
// whole frame instead (`vs_glare_full`) when it reaches too far round the
// sky for a polygon (a star flyby).
const GLARE_SIDES: u32 = 32u;
// Half a step of the 8-bit sRGB frame near black, in linear light.
const GLARE_FLOOR: f32 = 1.5e-4;

struct Glare {
    @builtin(position) position: vec4<f32>,
    // The ray through the pixel, not normalized (a point on the polygon's
    // plane, or across the screen).
    @location(0) dir: vec3<f32>,
    // xyz: the way to the star; w: its disc's radius, as an angle.
    @location(1) @interpolate(flat) star: vec4<f32>,
    // rgb: the glare's colour; w: where it falls under the floor, as `g`.
    @location(2) @interpolate(flat) tint: vec4<f32>,
};

fn glare_vertex(position: vec4<f32>, dir: vec3<f32>) -> Glare {
    var out: Glare;
    out.position = position;
    out.dir = dir;
    let distance = max(length(params.star.xyz), 1e-3);
    out.star = vec4<f32>(params.star.xyz / distance, params.star.w / distance);
    let colour = params.star_colour.rgb * params.star_colour.w;
    let bright = max(max(colour.r, colour.g), colour.b);
    // `g * 0.012 * bright` is the faint end's term (`glare_near`).
    out.tint = vec4<f32>(colour * (1.0 - params.warp.y), GLARE_FLOOR / max(0.012 * bright, 1e-12));
    return out;
}

@vertex
fn vs_glare(@builtin(vertex_index) index: u32) -> Glare {
    let distance = max(length(params.star.xyz), 1e-3);
    let ahead = params.star.xyz / distance;
    let colour = params.star_colour.rgb * params.star_colour.w;
    let bright = max(max(colour.r, colour.g), colour.b);
    // The chord to the star where the glare reaches the floor, as an angle
    // (kept under a right angle: wider, the frame takes `vs_glare_full`).
    let chord = params.star.w / distance * 0.012 * bright / GLARE_FLOOR;
    let angle = min(2.0 * asin(min(chord * 0.5, 1.0)), 1.4);
    // A fan of triangles as a list: the first corner, then each pair after.
    let corner = index % 3u;
    let k = select(index / 3u + corner, 0u, corner == 0u);
    let phi = f32(k) * TAU / f32(GLARE_SIDES);
    let side = normalize(cross(ahead, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(ahead.y) > 0.9)));
    let other = cross(ahead, side);
    // Round the circle, on the plane a unit out toward the star.
    let p = ahead + (side * cos(phi) + other * sin(phi)) * (tan(angle) / cos(PI / f32(GLARE_SIDES)));
    let z = dot(p, params.forward.xyz);
    // Just short of the far end (the background's), past every body's rank.
    let position = vec4<f32>(dot(p, params.right.xyz) / params.frame.y, dot(p, params.up.xyz) / params.frame.y, z * 0.9999, z);
    return glare_vertex(position, p);
}

@vertex
fn vs_glare_full(@builtin(vertex_index) index: u32) -> Glare {
    let full = full_vertex(index);
    return glare_vertex(full.position, full.dir);
}

@fragment
fn fs_glare(in: Glare) -> @location(0) vec4<f32> {
    let angle = max(length(normalize(in.dir) - in.star.xyz), in.star.w * 0.5);
    let g = in.star.w / angle;
    let faint = in.tint.w;
    // `glare_near`'s g² · 0.06 + g · 0.012, less its value at the floor.
    let k = max((g - faint) * (0.012 + 0.06 * (g + faint)), 0.0);
    return vec4<f32>(in.tint.rgb * k, 0.0);
}

@fragment
fn fs_probe_glare(in: Glare) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

// Streaks flying out from the heading: each lives in a cell of the screen's
// polar coordinates (angle around the heading, log of the distance from it),
// which scroll outward with time, so one cell lookup draws a moving streak.
fn streaks(theta: f32, logr: f32, count: f32, speed: f32, length_: f32, seed: i32) -> f32 {
    var x = vec2<f32>(theta / TAU * count, logr * 6.0 - params.warp.w * speed);
    let s = f32(seed);
    // Each column of cells shifted along by its own amount, so the streaks
    // do not line up into rings.
    x.y += hash_cell(vec3<f32>(floor(x.x), s, 5.0)) * 7.0;
    let cell = floor(x);
    let h = hash_cell(vec3<f32>(cell, s));
    if (h > 0.35) {
        return 0.0;
    }
    let f = x - cell;
    let along = hash_cell(vec3<f32>(cell, s + 1.0));
    let across = abs(f.x - 0.5) * 2.0;
    let t = (f.y - along * (1.0 - length_)) / length_;
    let line = (1.0 - smoothstep01(0.0, 0.25, across)) * smoothstep01(0.0, 0.15, t) * (1.0 - smoothstep01(0.6, 1.0, t));
    return line * (0.4 + h * 1.7);
}

// A hash of a cell's whole-number coordinates (up to some thousands) in
// float arithmetic: `hash3`'s 32-bit multiplies are many instructions each
// on the V3D, and every streak takes three.
fn hash_cell(p: vec3<f32>) -> f32 {
    var q = fract(p * vec3<f32>(0.1031, 0.1030, 0.0973));
    q += dot(q, q.yzx + 33.33);
    return fract((q.x + q.y) * q.z);
}

// The drive's light over the whole frame, the pixel's polar coordinates
// round the heading worked out here. (A polar fan of triangles carrying them
// as varyings was 1.1-3.1 ms slower on the Pi: some 42,000 small triangles
// cost the binner and the varyings more than `atan2` and `log` a pixel.)
@fragment
fn fs_warp(in: Full) -> @location(0) vec4<f32> {
    let half = params.frame.x;
    let s = vec2<f32>((in.position.x - half) / half, (half - in.position.y) / half);
    let r = max(length(s), 1e-4);
    // Around the heading by cosine and sine, so noise has no seam where the
    // angle wraps: the pixel's own direction from the heading.
    return warp_colour(r, atan2(s.y, s.x), log(r), s / r, in.dir);
}

// Each drive's light at a distance `r` from the heading (across the half
// frame), at angle `theta` round it (`atan2`'s), `logr` the log of `r` and
// `around` the unit way round; `dir` the ray, for the calm sky.
fn warp_colour(r: f32, theta: f32, logr: f32, around: vec2<f32>, dir: vec3<f32>) -> vec4<f32> {
    let s = around * r;
    let warp = params.warp.x;
    let t = params.warp.w;
    var colour = vec3<f32>(0.0);
    if (WARP == 1) {
        // Lumen Drift: a blue-white blaze. Fine rays of light from a white
        // core fill the frame (two octaves of noise round the heading, so
        // they shimmer and never repeat), fast streaks fly out along them,
        // and soft sheets of light fan out flat to the sides.
        // The noise before the streaks. The other way round it fits four
        // threads, yet on the Pi it ran 6.60 ms a frame against 5.96 this
        // way, at two.
        let fine = noise(vec3<f32>(around * 26.0, logr * 0.35 - t * 0.25));
        let broad = noise(vec3<f32>(around * 9.0, logr * 0.2 - t * 0.12));
        let rays = smoothstep01(0.05, 0.75, fine) * 0.7 + smoothstep01(-0.1, 0.8, broad) * 0.5;
        let falloff = exp(-r * 1.3);
        colour += vec3<f32>(0.10, 0.55, 0.95) * rays * falloff * 1.1;
        colour += vec3<f32>(0.70, 0.90, 1.0) * streaks(theta, logr, 260.0, 10.0, 0.85, 3) * smoothstep01(0.05, 0.4, r);
        colour += vec3<f32>(0.85, 0.95, 1.0) * streaks(theta, logr, 120.0, 7.0, 0.9, 7) * 0.6 * smoothstep01(0.1, 0.6, r);
        let sheet = exp(-abs(s.y) * 5.0 / (0.2 + r)) * (0.6 + 0.4 * broad);
        colour += vec3<f32>(0.40, 0.75, 1.0) * sheet * 0.6;
        // The deep blue the rays shine through, lighter toward the core.
        colour += mix(vec3<f32>(0.03, 0.22, 0.38), vec3<f32>(0.01, 0.06, 0.14), smoothstep01(0.0, 1.2, r));
        colour += vec3<f32>(0.95, 0.98, 1.0) * exp(-r * 10.0) * 1.6 + vec3<f32>(0.35, 0.7, 1.0) * exp(-r * 3.5) * 0.5;
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
        // Fold Drive: space itself warped. The sky ahead is drawn in toward
        // the heading and smeared outward (three looks along the radius),
        // stars stretch into streaks, white near the heading and redder
        // toward the edges as space shifts their light, and a faint ring of
        // bent light marks where space folds.
        let fold = 0.08 * exp(-sq((r - 0.3) / 0.06));
        // Two looks into the soft galaxy, written out: the V3D does not
        // unroll a loop and waits on each read in turn.
        let side = (params.right.xyz * around.x + params.up.xyz * around.y) * (r + fold) * params.frame.y;
        let smear = textureSampleLevel(sky_cube, map_sampler, params.forward.xyz + side * 0.38, 0.0).rgb
            + textureSampleLevel(sky_cube, map_sampler, params.forward.xyz + side * 0.66, 0.0).rgb;
        colour += smear * 1.35;
        let shift = mix(vec3<f32>(0.85, 0.92, 1.0), vec3<f32>(1.0, 0.62, 0.42), smoothstep01(0.2, 1.1, r));
        colour += shift * streaks(theta, logr, 300.0, 9.0, 0.6, 31) * smoothstep01(0.03, 0.3, r);
        colour += shift * streaks(theta, logr, 140.0, 6.0, 0.75, 37) * 0.8 * smoothstep01(0.05, 0.4, r);
        colour += vec3<f32>(1.0, 0.95, 0.9) * streaks(theta, logr, 90.0, 4.0, 0.85, 41) * 1.1 * smoothstep01(0.1, 0.5, r);
        colour += vec3<f32>(0.45, 0.6, 1.0) * fold * 1.2;
        colour += vec3<f32>(1.0, 0.97, 0.92) * exp(-r * 14.0);
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
        // Bubble Drive: the ship rides in a bubble of warped space. Its shell
        // is a shimmering ring, split into colour at its edge; inside it the
        // stars ahead are bunched toward the heading and bluer, outside it
        // space is stretched and redder.
        // The shell's wobble and ripples from sines, not the noise volume:
        // its fetches beside the sky's held twice the registers.
        let wobble = 0.012 * sin(theta * 3.0 + t * 0.7) * sin(theta * 5.0 - t * 0.4);
        let shell = 0.78 + wobble;
        let inside = 1.0 - smoothstep01(shell - 0.02, shell + 0.02, r);
        // The shell's light: one ring per colour, a little apart, and ripples
        // running round it. Smooth bumps, not exponentials: four of those
        // beside the rest held twice the registers.
        let ripple = 0.75 + 0.25 * sin(theta * 12.0 - t * 2.3) * sin(theta * 7.0 + t * 1.1);
        let d = (r - shell) / 0.012;
        let ring = vec3<f32>(bump(d - 1.0), bump(d), bump(d + 1.0));
        let shell_light = (ring * 1.3 + vec3<f32>(0.3, 0.45, 0.9) * bump(d * 0.3) * 0.3) * ripple;
        // Inside, look out through the bubble at space squeezed toward the
        // heading; outside, at space stretched away from it.
        let bend = mix(r * 1.45, r * 0.55, inside);
        let dir_out = normalize(params.forward.xyz + (params.right.xyz * around.x + params.up.xyz * around.y) * bend * params.frame.y);
        let tint = mix(vec3<f32>(1.0, 0.55, 0.4), vec3<f32>(0.6, 0.8, 1.0), inside);
        // The stars and the galaxy from the baked starfield: one read (worked
        // out per pixel beside the streaks, they held twice the registers).
        colour += textureSampleLevel(starfield, map_sampler, dir_out, 0.0).rgb * 1.6 * tint * mix(0.6, 1.0, inside);
        colour += shell_light;
        colour += vec3<f32>(0.85, 0.92, 1.0) * exp(-r * 12.0) * 0.8;
    }
    // In and out of the jump, the calm sky fades under the drive's light.
    // At full speed none of it shows: a branch the whole frame takes alike.
    var calm = vec3<f32>(0.0);
    if (warp < 0.999) {
        calm = textureSampleLevel(starfield, map_sampler, dir, 0.0).rgb * (1.0 - warp);
    }
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
// touching it (its disc and air) meets the
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
    let air = params.bodies[instance].air;
    let disc = params.bodies[instance].place.w * select(1.0, 1.08, max(max(air.r, air.g), air.b) > 0.0);
    return body_vertex(vertex, instance, min(disc, air.w));
}

// The polygon round everything within `bound` of the body's centre.
fn body_vertex(vertex: u32, instance: u32, bound: f32) -> Quad {
    var out: Quad;
    out.body = instance;
    let depth = params.bodies[instance].ring_colour.w;
    let c = params.bodies[instance].place.xyz;
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

// The rings drawn as what they are, an annulus in their plane: a polygon
// just outside the outer edge with one just inside the inner edge cut out,
// so the pixels between the rings and the planet's poles cost nothing, and
// each pixel knows its point on the plane without a ray of its own. The
// sides wholly past the plane through the planet's centre are drawn just
// behind the planet's depth, so its mask hides them where its disc is in
// front. Edge on it covers the pixels a ray per pixel would find the rings
// in, which are next to none; what reaches behind the camera is clipped.
const RING_SIDES: u32 = 32u;

struct RingQuad {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) body: u32,
    // The point on the ring's plane, from the planet's centre.
    @location(1) local: vec3<f32>,
};

@vertex
fn vs_ring(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> RingQuad {
    let c = params.bodies[instance].place.xyz;
    let radius = params.bodies[instance].place.w;
    let axis = params.bodies[instance].rot[1].xyz;
    let outer = radius * params.bodies[instance].ring.y * 1.01 / cos(PI / f32(RING_SIDES));
    let side = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(axis.y) < 0.9);
    let e1 = normalize(cross(axis, side));
    let e2 = cross(axis, e1);
    // Each side a quad: outer, outer next, inner; inner, outer next, inner next.
    let corner = vertex % 6u;
    let first = vertex / 6u;
    let k = first + select(0u, 1u, corner == 1u || corner == 4u || corner == 5u);
    let inside = corner == 2u || corner == 3u || corner == 5u;
    let r = select(outer, radius * params.bodies[instance].ring.x * 0.99, inside);
    let a = f32(k) * TAU / f32(RING_SIDES);
    let local = (e1 * cos(a) + e2 * sin(a)) * r;
    // Past the centre's plane at both of the side's ends: behind the planet.
    let a0 = f32(first) * TAU / f32(RING_SIDES);
    let a1 = f32(first + 1u) * TAU / f32(RING_SIDES);
    let behind = dot(e1 * cos(a0) + e2 * sin(a0), c) > 0.0 && dot(e1 * cos(a1) + e2 * sin(a1), c) > 0.0;
    let depth = params.bodies[instance].ring_colour.w + select(0.0, 0.5 / (params.frame.w + 1.0), behind);
    var out: RingQuad;
    out.body = instance;
    out.local = local;
    // Homogeneous, w the depth along the view: `local` is interpolated with
    // the perspective, exactly the point on the plane, and the GPU clips
    // what lies behind the camera.
    let q = c + local;
    let z = dot(q, params.forward.xyz);
    out.position = vec4<f32>(dot(q, params.right.xyz) / params.frame.y, dot(q, params.up.xyz) / params.frame.y, depth * z, z);
    return out;
}

// The rings over the annulus: the point on their plane comes in, so no ray
// is traced, only whether the planet stands in front of it.
@fragment
fn fs_ring_annulus(in: RingQuad) -> @location(0) vec4<f32> {
    let i = in.body;
    let c = params.bodies[i].place.xyz;
    let radius = params.bodies[i].place.w;
    let p = in.local;
    let q = c + p;
    let distance = length(q);
    let along = dot(c, q) / distance;
    let inside = radius * radius - (dot(c, c) - along * along);
    // Nothing, not a discard, where the planet stands in front or the rings
    // are clear: over its premultiplied blend that changes nothing, and a
    // shader that can discard turns the V3D's early depth test off for the
    // whole frame, every draw in it.
    if (inside > 0.0 && along - sqrt(inside) < distance) {
        return vec4<f32>(0.0);
    }
    let density = ring_density(length(p) / radius, i) * params.bodies[i].ring.z;
    return vec4<f32>(ring_light(i, p) * density, density);
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

// The depth mask for a body wholly ahead of the camera, for nothing per
// pixel: a polygon inscribed in its silhouette, a pixel and a half inside,
// so every pixel it covers is fully the body's. The few between it and the
// soft edge are shaded behind the body as well, which costs less than
// tracing every pixel of the disc.
const MASK_SIDES: u32 = 64u;
const MASK_VERTICES: u32 = MASK_SIDES * 3u;

@vertex
fn vs_mask_inscribed(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Quad {
    var out: Quad;
    out.body = instance;
    let c = params.bodies[instance].place.xyz;
    let distance = length(c);
    let r = params.bodies[instance].place.w;
    // The silhouette: where the cone of tangent rays meets the plane through
    // the centre, inset.
    let spread = r * distance / sqrt(max(distance * distance - r * r, 1e-12)) - 1.5 * params.frame.z * distance;
    let corner = vertex % 3u;
    var p = project(c);
    if (corner != 0u) {
        let axis = c / distance;
        let side = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(axis.y) < 0.9);
        let e1 = normalize(cross(axis, side));
        let e2 = cross(axis, e1);
        let a = f32(vertex / 3u + corner - 1u) * TAU / f32(MASK_SIDES);
        p = project(c + (e1 * cos(a) + e2 * sin(a)) * max(spread, 0.0));
    }
    out.position = vec4<f32>(p, params.bodies[instance].ring_colour.w, 1.0);
    return out;
}

@fragment
fn fs_mask_inscribed(in: Quad) -> @location(0) vec4<f32> {
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

// A world's features, fixed for a pipeline that draws only one sort of
// world (`fs_globe`), or -1 to read them from the body.
override AIR: i32 = -1;
override NIGHT: i32 = -1;
override GIANT: i32 = -1;

fn has_air(i: u32) -> bool {
    if (AIR >= 0) {
        return AIR == 1;
    }
    return max(max(params.bodies[i].air.r, params.bodies[i].air.g), params.bodies[i].air.b) > 0.0;
}

// Sunlight reaching the ground where the star stands at `mu`, through the air.
fn ground_light(i: u32, mu: f32) -> vec3<f32> {
    if (has_air(i)) {
        return star_light() * exp(-params.bodies[i].air.rgb * airmass(mu)) * smoothstep01(-0.035, 0.02, mu);
    }
    return star_light() * smoothstep01(-0.03, 0.03, mu);
}

// What the air lets through over `mass` air masses: 1 without air.
fn thin(i: u32, mass: f32) -> vec3<f32> {
    return exp(-params.bodies[i].air.rgb * mass);
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

// Close up (`DETAIL`, low orbit and the dive to it), the map's lookups are
// made crisp: where it is magnified dozens of times, each texel's blend into
// the next is narrowed and its edge bent into a wavy line (a few sines of
// the texel coordinates: arithmetic, no fetch; the noise volume as a third
// fetch halved the threads), so coastlines, ridges and snow lines are sharp
// edges instead of a smear. The detail pass adds the fine slope and grain.
override DETAIL: bool = false;

fn crisp_uv(uv: vec2<f32>, size: vec2<f32>, close: f32) -> vec2<f32> {
    let t = uv * size - 0.5;
    let cell = floor(t);
    let f = t - cell;
    let wobble = vec2<f32>(
        sin(t.y * 2.7 + sin(t.x * 1.9 + t.y * 0.7) * 1.3),
        sin(t.x * 2.3 + sin(t.y * 1.7 - t.x * 0.6) * 1.1),
    );
    let edge = vec2<f32>(0.5) + wobble * 0.28;
    let crisp = smoothstep(edge - vec2<f32>(0.3), edge + vec2<f32>(0.3), f);
    // Not all the way: fully crisp, the texel grid shows in its steps.
    return (cell + mix(f, crisp, close * 0.75) + 0.5) / size;
}

// A mapped planet's ground: its colour and relief, lit through its air.
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
        // The maps first, so little is live across their reads. Crisp by how
        // far the map is magnified (its level, below the bias).
        var at = uv;
        if (DETAIL) {
            at = crisp_uv(uv, map_size(), smoothstep01(BIAS - 0.5, BIAS - 2.5, lod));
        }
        let relief = relief_texel(at, lod + 0.4);
        var albedo = day_texel(at, lod) * mix(1.0, 0.8, relief.b);
        let n = h.n;
        let sun = normalize(params.star.xyz - h.point);
        let mu_sun = dot(n, sun);
        let east = east_of(i, nb);
        let tilted = normalize(n + (east * (relief.r - 0.5) + cross(n, east) * (relief.g - 0.5)) * 0.55);
        let sky = select(vec3<f32>(0.0), skylight() * smoothstep01(-0.15, 0.4, mu_sun), has_air(i));
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

// The air seen edge-on past a body's limb, added: `limb_glow`, in the
// eclipses' shadows. Traced per pixel over the whole fan: only for a body
// not wholly ahead of the camera (else `fs_limb_ring`).
@fragment
fn fs_limb(in: Quad) -> @location(0) vec4<f32> {
    let i = in.body;
    let h = trace(i, in.position.xy);
    // A branch, not a discard: the V3D skips the work for 16-pixel groups
    // that all leave it, but runs on through a discard.
    if (!(h.closest >= ATMOSPHERE || h.cover >= 1.0)) {
        let sun = normalize(params.star.xyz - h.point);
        return vec4<f32>(limb_glow(i, h, sun) * eclipse(in.body, h.point, sun), 0.0);
    }
    discard;
    return vec4<f32>(0.0);
}

// The glow of the air past a body's limb along the ray of `h`, with the
// star toward `sun`, for what of the pixel the body leaves.
fn limb_glow(i: u32, h: Hit, sun: vec3<f32>) -> vec3<f32> {
    let r = h.closest;
    let air = params.bodies[i].air.rgb;
    let height = max(r - 1.0, 0.0);
    let depth = air * sqrt(TAU * r / SCALE_HEIGHT) * exp(-height / SCALE_HEIGHT);
    let mu = dot(h.n, sun);
    let lit = star_light() * exp(-air * airmass(mu + 0.08) * 0.6) * smoothstep01(-0.30, 0.05, mu);
    return lit * (vec3<f32>(1.0) - exp(-depth)) * 0.9 * (1.0 - h.cover);
}

// The air past the limb of a body wholly ahead of the camera, as an annulus
// round its silhouette instead of a fan over the whole disc: `LIMB_SIDES`
// sides in two bands, from 2.5 px inside the limb to 1.02 radii, then on to
// the top of the air. Its light (`limb_glow`'s, eclipses included) is worked
// out at the vertices, as at each ray's closest point to the body; per pixel
// only the air's depth along the ray and the soft edge. Added, strictly in
// front of the depth mask as `fs_limb` is, so the mask rejects the disc.
const LIMB_SIDES: u32 = 128u;

struct Limb {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) body: u32,
    // The ray's closest approach in radii, and the soft edge in pixels.
    @location(1) reach: vec2<f32>,
    @location(2) lit: vec3<f32>,
};

@vertex
fn vs_limb_ring(@builtin(vertex_index) vertex: u32, @builtin(instance_index) i: u32) -> Limb {
    var out: Limb;
    out.body = i;
    let c = params.bodies[i].place.xyz;
    let radius = params.bodies[i].place.w;
    let distance = length(c);
    let axis = c / distance;
    let side = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(axis.y) < 0.9);
    let e1 = normalize(cross(axis, side));
    let e2 = cross(axis, e1);
    // Each side two quads, one a band: inner, inner next, outer; outer,
    // inner next, outer next.
    let corner = vertex % 6u;
    let quad = vertex / 6u;
    let k = quad / 2u + select(0u, 1u, corner == 1u || corner == 4u || corner == 5u);
    let ring = quad % 2u + select(0u, 1u, corner == 2u || corner == 3u || corner == 5u);
    // A pixel at the limb, in radii.
    let pixel = params.frame.z * sqrt(max(distance * distance - radius * radius, 0.0)) / radius;
    var rho = max(1.0 - 2.5 * pixel, 0.0);
    if (ring == 1u) {
        rho = 1.02;
    } else if (ring == 2u) {
        rho = ATMOSPHERE / cos(PI / f32(LIMB_SIDES));
    }
    let a = f32(k) * TAU / f32(LIMB_SIDES);
    let out_dir = e1 * cos(a) + e2 * sin(a);
    // On the cone of rays passing `rho` radii from the centre, in the plane
    // through it.
    let sin_t = min(rho * radius / distance, 0.999);
    let spread = distance * sin_t / sqrt(1.0 - sin_t * sin_t);
    let p = c + out_dir * spread;
    // Homogeneous, w the depth along the view, as `vs_ring`: interpolated
    // with the perspective, at the body's rank.
    let z = dot(p, params.forward.xyz);
    out.position = vec4<f32>(dot(p, params.right.xyz) / params.frame.y, dot(p, params.up.xyz) / params.frame.y, params.bodies[i].ring_colour.w * z, z);
    out.reach = vec2<f32>(rho, (rho - 1.0) / pixel);
    // Lit as at the ray's closest point to the body.
    let dir = normalize(p);
    let nearest = dir * dot(c, dir);
    let n = normalize(nearest - c);
    let sun = normalize(params.star.xyz - nearest);
    let mu = dot(n, sun);
    let air = params.bodies[i].air.rgb;
    out.lit = star_light() * exp(-air * airmass(mu + 0.08) * 0.6) * (0.9 * smoothstep01(-0.30, 0.05, mu) * eclipse(i, nearest, sun));
    return out;
}

@fragment
fn fs_limb_ring(in: Limb) -> @location(0) vec4<f32> {
    let r = in.reach.x;
    let air = params.bodies[in.body].air.rgb;
    let depth = air * (sqrt(TAU * r / SCALE_HEIGHT) * exp(-max(r - 1.0, 0.0) / SCALE_HEIGHT));
    let cover = smoothstep01(-0.75, 0.75, -in.reach.y);
    return vec4<f32>(in.lit * (vec3<f32>(1.0) - exp(-depth)) * (1.0 - cover), 0.0);
}

// The rings' colour at `p` on their plane (from the planet's centre): lit
// from either face, in the planet's shadow.
fn ring_light(i: u32, p: vec3<f32>) -> vec3<f32> {
    let axis = params.bodies[i].rot[1].xyz;
    let sun = normalize(params.star.xyz - (params.bodies[i].place.xyz + p));
    let along = dot(-p, sun);
    let miss = length(-p - sun * along);
    let shadowed = select(1.0, smoothstep01(params.bodies[i].place.w * 0.97, params.bodies[i].place.w * 1.03, miss), along > 0.0);
    let light = star_light() * (0.25 + 0.75 * sqrt(abs(dot(sun, axis)))) * shadowed;
    return params.bodies[i].ring_colour.rgb * (light + skylight() * 0.3);
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

// Whether the world has a cloud deck.
override CLOUDS: bool = false;
// Whether the world has rings, whose shadow falls on it.
override RINGS: bool = false;

// ---------------------------------------------------------------------------
// A mapped world seen from afar as a mesh: a patch of its own latitude-
// longitude grid (`window`, `mesh` on the body, from the CPU), so everything
// that varies smoothly over the sphere is worked out once a vertex and the
// fragment shader keeps only what depends on a texel (docs/solar-perf-plan.md,
// brief S1). The grid's vertex is its index, `row * GRID_STRIDE + column`
// (no vertex buffer), from a static index buffer holding the triangles for
// a few sizes of grid; back faces are culled. The mesh stands a pixel and a
// half proud of the surface at the limb, and each vertex carries the soft
// edge of the ray through it against the true sphere, so the edge fades as
// the traced one does. The map's level is the GPU's own: the mesh never
// crosses the longitude's wrap inside a triangle. Built per look
// (`SurfaceLook`): only its own code, in as few registers as it needs.
// ---------------------------------------------------------------------------

const GRID_STRIDE: u32 = 129u;

// What the mesh is of: 0 a mapped world (`fs_globe`); 1 a world whose maps
// are not in yet (`fs_globe_flat`: its colour folded into `direct`, its
// bands' contrast in `uv.z`); 2 a moon (`fs_globe_moon`: its layer of the
// moon maps in `uv.z`); 3 the star (`fs_globe_star`: its body-frame normal in
// `direct`, its surface's style in `thru`). The fragment shaders of those
// have no body index, so what they need of the body rides in varyings the
// mapped world's shading uses and theirs does not.
override GLOBE: i32 = 0;

struct Globe {
    @builtin(position) position: vec4<f32>,
    // The map's coordinates, and the clouds' turned longitude.
    @location(0) uv: vec3<f32>,
    // The soft edge in pixels past the limb (negative: inside), the sun's
    // and the viewer's height over the ground, and the sea's glint angle.
    @location(1) geo: vec4<f32>,
    // The sun along east and north (x 0.55, the relief's strength), the
    // clouds' forward-scattering lobe, and where the rings' shadow falls.
    @location(2) slope: vec4<f32>,
    // The clouds' shadow shift in map coordinates; the rings' shadow's
    // strength and row.
    @location(3) shift: vec4<f32>,
    // Sunlight on the ground through the air both ways.
    @location(4) direct: vec3<f32>,
    // The air toward the viewer.
    @location(5) thru: vec3<f32>,
    // Sunlight on the cloud deck through the air above it and to the viewer.
    @location(6) deck: vec3<f32>,
    // The air's own sheen.
    @location(7) sheen: vec3<f32>,
};

@vertex
fn vs_globe(@builtin(vertex_index) vertex: u32, @builtin(instance_index) i: u32) -> Globe {
    var out: Globe;
    let cells = params.bodies[i].mesh.x;
    let at = vec2<f32>(f32(vertex % GRID_STRIDE), f32(vertex / GRID_STRIDE)) / cells;
    let window = params.bodies[i].window;
    let lon = window.x + window.z * at.x;
    let lat = window.y + window.w * at.y;
    let cos_lat = cos(lat);
    let nb = vec3<f32>(cos_lat * sin(lon), sin(lat), cos_lat * cos(lon));
    let n = from_body(i, nb);
    let east = from_body(i, vec3<f32>(cos(lon), 0.0, -sin(lon)));
    let north = cross(n, east);
    let c = params.bodies[i].place.xyz;
    let radius = params.bodies[i].place.w;
    let point = c + n * radius * (1.0 + params.bodies[i].mesh.y);
    let distance = length(point);
    let dir = point / distance;
    // The ray through the vertex against the true sphere: its soft edge.
    let closest = length(c - dir * dot(c, dir)) / radius;
    let edge = (closest - 1.0) * radius / (distance * params.frame.z);
    // Homogeneous, w the depth along the view, as `vs_ring`: the values are
    // interpolated with the perspective, and what is behind is clipped.
    let z = dot(point, params.forward.xyz);
    out.position = vec4<f32>(dot(point, params.right.xyz) / params.frame.y, dot(point, params.up.xyz) / params.frame.y, params.bodies[i].ring_colour.w * z, z);

    let sun = normalize(params.star.xyz - point);
    let mu = dot(n, sun);
    let mu_view = clamp(dot(n, -dir), 0.0, 1.0);
    let se = dot(sun, east);
    let sn = dot(sun, north);
    out.uv = vec3<f32>(0.5 + lon / TAU, 0.5 - lat / PI, 0.5 + lon / TAU + params.bodies[i].rot[0].w);
    out.geo = vec4<f32>(edge, mu, mu_view, dot(n, normalize(sun - dir)));
    let toward_star = max(dot(dir, sun), 0.0);
    let lobe = toward_star * toward_star * toward_star * toward_star;
    out.slope = vec4<f32>(se * 0.55, sn * 0.55, 0.2 + 1.2 * lobe * lobe, 0.0);
    let reach = 0.006 / max(mu, 0.12);
    out.shift = vec4<f32>(se * reach / (TAU * max(cos_lat, 0.05)), -sn * reach / PI, 0.0, 0.0);
    if (RINGS) {
        let axis = params.bodies[i].rot[1].xyz;
        let denom = dot(sun, axis);
        let s = dot(c - point, axis) / denom;
        let hit = point + sun * s - c;
        let x = (length(hit) / radius - params.bodies[i].ring.x) / (params.bodies[i].ring.y - params.bodies[i].ring.x);
        out.slope.w = clamp(x, -1.0, 2.0);
        out.shift.z = select(0.0, params.bodies[i].ring.z * 0.85 * smoothstep01(-0.05, 0.12, mu), abs(denom) >= 1e-4 && s > 0.0);
        // The profile's rows (`RING_ROWS`): the texture is not bound to the
        // vertex stage.
        out.shift.w = (params.bodies[i].ring.w + 0.5) / 8.0;
    }
    out.direct = star_light();
    out.thru = vec3<f32>(1.0);
    out.deck = star_light() * (0.85 * smoothstep01(-0.08, 0.04, mu));
    out.sheen = vec3<f32>(0.0);
    if (AIR == 1) {
        let air = params.bodies[i].air.rgb;
        let thru = exp(-air * airmass(mu_view));
        let high = star_light() * exp(-air * (airmass(mu + 0.04) * 0.55));
        out.thru = thru;
        out.direct = star_light() * exp(-air * airmass(mu)) * thru;
        out.deck = high * thru * (0.85 * smoothstep01(-0.08, 0.04, mu));
        out.sheen = high * (vec3<f32>(1.0) - thru) * (0.45 * smoothstep01(-0.10, 0.08, mu));
    }
    if (GLOBE == 1) {
        out.direct *= params.bodies[i].colour.rgb;
        out.uv.z = params.bodies[i].colour.w;
    } else if (GLOBE == 2) {
        out.uv.z = params.bodies[i].look.z;
    } else if (GLOBE == 3) {
        out.direct = nb;
        out.thru = params.bodies[i].look.yzw;
    }
    return out;
}

// The maps of the slot a pipeline is built for, at the GPU's own level.
fn globe_day(uv: vec2<f32>, bias: f32) -> vec3<f32> {
    if (SLOT == 0) {
        return textureSampleBias(a_day, map_sampler, uv, bias).rgb;
    }
    return textureSampleBias(b_day, map_sampler, uv, bias).rgb;
}

fn globe_relief(uv: vec2<f32>, bias: f32) -> vec4<f32> {
    if (SLOT == 0) {
        return textureSampleBias(a_relief, map_sampler, uv, bias);
    }
    return textureSampleBias(b_relief, map_sampler, uv, bias);
}

fn globe_extra(uv: vec2<f32>, bias: f32) -> vec2<f32> {
    if (SLOT == 0) {
        return textureSampleBias(a_extra, map_sampler, uv, bias).rg;
    }
    return textureSampleBias(b_extra, map_sampler, uv, bias).rg;
}

// The world over the mesh: the ground, the cloud deck and its shadows and
// the added light, its rings' shadow included. A world with
// seas (`SEA`) is two passes: here only its ground and cloud deck, in few
// enough registers for four threads, then its added light (the sea's glint
// and sheen, the cities, the air's sheen) in `fs_globe_glow`.
@fragment
fn fs_globe(in: Globe) -> @location(0) vec4<f32> {
    let mu = in.geo.y;
    let relief = globe_relief(in.uv.xy, BIAS + 0.4);
    var cloud = 0.0;
    if (CLOUDS) {
        cloud = globe_extra(in.uv.zy, BIAS).g;
    }
    var albedo = globe_day(in.uv.xy, BIAS) * (1.0 - 0.2 * relief.b);
    if (GIANT == 1) {
        albedo *= 0.55 + 0.45 * sqrt(in.geo.z);
    }
    // The relief's tilt of the normal, along the interpolated east and north.
    let d = relief.rg - 0.5;
    let lit = (mu + dot(d, in.slope.xy)) * inverseSqrt(1.0 + 0.3025 * dot(d, d));
    var day = smoothstep01(-0.03, 0.03, mu);
    var sky = vec3<f32>(0.0);
    if (AIR == 1) {
        day = smoothstep01(-0.035, 0.02, mu);
        sky = in.thru * skylight();
    }
    let sky_rise = smoothstep01(-0.15, 0.4, mu);
    var rgb = albedo * (in.direct * (day * max(lit + 0.03, 0.0)) + sky * sky_rise);
    if (CLOUDS) {
        let toward = globe_extra(in.uv.zy + in.shift.xy, BIAS + 1.0).g;
        let shadow = 0.6 * toward * smoothstep01(-0.05, 0.12, mu);
        let side = select(1.0, clamp(1.0 - 2.0 * (toward - cloud), 0.4, 1.3), mu > -0.05);
        let lit_deck = in.deck * ((0.30 + 0.70 * sqrt(max(mu, 0.0))) * side + (1.0 - cloud) * in.slope.z) + sky * (sky_rise * 1.275);
        let thick = min(cloud * (2.2 - 1.2 * in.geo.z), 1.0);
        rgb = lit_deck * thick + rgb * ((1.0 - thick) * (1.0 - shadow));
    }
    // A world with seas adds its light in `fs_globe_glow`.
    if (AIR == 1 && !SEA) {
        rgb += in.sheen;
    }
    if (NIGHT == 1 && !SEA) {
        rgb += city_lights(globe_extra(in.uv.xy, BIAS + 0.6).r, mu, cloud) * in.thru;
    } else if (NIGHT == 2) {
        let night = globe_extra(in.uv.xy, BIAS + 0.6).r;
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu);
        let heat = mix(vec3<f32>(1.0, 0.25, 0.04), vec3<f32>(1.0, 0.75, 0.3), night * night);
        rgb += heat * in.thru * (night * (0.15 + 0.55 * darkness));
    }
    if (RINGS) {
        rgb *= globe_ring_shadow(in);
    }
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(rgb * cover, cover);
}

// The rings' shadow, from the coordinate and strength `vs_globe` worked out.
fn globe_ring_shadow(in: Globe) -> f32 {
    let x = in.slope.w;
    let density = textureSampleLevel(ring_profile, map_sampler, vec2<f32>(x, in.shift.w), 0.0).r;
    return 1.0 - density * in.shift.z * step(0.0, x) * step(x, 1.0);
}

// The night side's city lights, before the air toward the viewer.
fn city_lights(night: f32, mu: f32, cloud: f32) -> vec3<f32> {
    let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu);
    let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, night));
    return tint * (night * night * 2.2 * darkness * (1.0 - cloud * 0.85));
}

// The added light of a world with seas, over `fs_globe` built with the same
// constants (`add` blend): the sea's glint and sheen, the night side's
// lights, the air's own sheen. Its own pass so that neither half needs more
// than 32 registers, which on the V3D is four threads, not two.
@fragment
fn fs_globe_glow(in: Globe) -> @location(0) vec4<f32> {
    let mu = in.geo.y;
    var cloud = 0.0;
    if (CLOUDS) {
        cloud = globe_extra(in.uv.zy, BIAS).g;
    }
    let water = globe_relief(in.uv.xy, BIAS + 0.4).b * (1.0 - cloud);
    var day = smoothstep01(-0.03, 0.03, mu);
    var sky = vec3<f32>(0.0);
    if (AIR == 1) {
        day = smoothstep01(-0.035, 0.02, mu);
        sky = in.thru * skylight();
    }
    let away = 1.0 - in.geo.z;
    let away2 = away * away;
    let fresnel = 0.02 + 0.98 * away2 * away2 * away;
    let log_nh = log2(max(in.geo.w, 0.0));
    let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
    var rgb = (in.direct * (day * glint * (0.25 + fresnel)) + sky * (2.0 * fresnel * smoothstep01(-0.1, 0.3, mu))) * water;
    if (NIGHT == 1) {
        rgb += city_lights(globe_extra(in.uv.xy, BIAS + 0.6).r, mu, cloud) * in.thru;
    }
    if (AIR == 1) {
        rgb += in.sheen;
    }
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(rgb * cover, 0.0);
}

// A world whose maps are not in yet, over the mesh (`GLOBE` 1): its colour
// lit through the air (`direct`), banded on a giant by its latitude, in its
// rings' shadow.
@fragment
fn fs_globe_flat(in: Globe) -> @location(0) vec4<f32> {
    let mu = in.geo.y;
    var day = smoothstep01(-0.03, 0.03, mu);
    if (AIR == 1) {
        day = smoothstep01(-0.035, 0.02, mu);
    }
    var rgb = in.direct * (day * max(mu + 0.03, 0.0));
    if (GIANT == 1) {
        let y = sin((0.5 - in.uv.y) * PI);
        rgb *= 1.0 + in.uv.z * sin(y * 23.0 + sin(y * 7.0) * 2.0) * 0.5;
    }
    if (RINGS) {
        rgb *= globe_ring_shadow(in);
    }
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(rgb * cover, cover);
}

// A moon over the mesh (`GLOBE` 2), from its layer of the slot's moon maps:
// colour, relief, molten glow.
@fragment
fn fs_globe_moon(in: Globe) -> @location(0) vec4<f32> {
    let mu = in.geo.y;
    let layer = i32(in.uv.z + 0.5);
    var day: vec3<f32>;
    var relief: vec4<f32>;
    if (SLOT == 0) {
        day = textureSampleBias(a_moon_day, map_sampler, in.uv.xy, layer, BIAS).rgb;
        relief = textureSampleBias(a_moon_relief, map_sampler, in.uv.xy, layer, BIAS);
    } else {
        day = textureSampleBias(b_moon_day, map_sampler, in.uv.xy, layer, BIAS).rgb;
        relief = textureSampleBias(b_moon_relief, map_sampler, in.uv.xy, layer, BIAS);
    }
    let d = relief.rg - 0.5;
    let lit = (mu + dot(d, in.slope.xy)) * inverseSqrt(1.0 + 0.3025 * dot(d, d));
    var sunup = smoothstep01(-0.03, 0.03, mu);
    if (AIR == 1) {
        sunup = smoothstep01(-0.035, 0.02, mu);
    }
    var rgb = day * in.direct * (sunup * max(lit + 0.03, 0.0));
    // A moon's third relief channel is molten glow.
    let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu);
    let heat = mix(vec3<f32>(1.0, 0.25, 0.04), vec3<f32>(1.0, 0.75, 0.3), relief.b);
    rgb += heat * in.thru * (relief.b * (0.15 + 0.55 * darkness));
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(rgb * cover, cover);
}

// The star over the mesh (`GLOBE` 3): its limb darkened by the viewer's
// height over its surface, its granules one noise fetch along the
// interpolated body-frame normal.
@fragment
fn fs_globe_star(in: Globe) -> @location(0) vec4<f32> {
    // thru: the convection cells' scale and contrast, and how much the limb
    // darkens (the star's look.yzw).
    let style = in.thru;
    let granules = 1.0 + style.y * noise(in.direct * style.x + vec3<f32>(params.warp.z * 0.05));
    let tint = params.star_colour.rgb / max(max(params.star_colour.r, params.star_colour.g), params.star_colour.b);
    let light = min(tint * tint * 1.15 * (1.0 - style.z + style.z * in.geo.z) * granules, vec3<f32>(1.0));
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(light * cover, cover);
}

// For the Pi estimate: the mesh's pixels, shaded by nothing.
@fragment
fn fs_probe_globe(in: Globe) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

// ---------------------------------------------------------------------------
// An engine burn, seen from the cockpit: dust in a box round the ship
// streaking past against the way it goes, and the engines' glow welling up
// round the rim of the view. Both small and added over the frame, drawn only
// while the engine fires.
// ---------------------------------------------------------------------------

// Motes of dust in the box.
const DUST: u32 = 160u;
// Sides of the ring the glow is drawn over.
const RIM_SIDES: u32 = 48u;
// Where the glow starts, as a share of the view's radius.
const RIM_INNER: f32 = 0.82;

struct Streak {
    @builtin(position) position: vec4<f32>,
    // x: 0 at the head, 1 at the tail; y: across, -1..1; z: brightness.
    @location(0) along: vec3<f32>,
};

// A point relative to the camera, on screen in clip space, and its depth.
fn project_dust(p: vec3<f32>) -> vec3<f32> {
    let z = dot(p, params.forward.xyz);
    let x = dot(p, params.right.xyz) / z / params.frame.y;
    let y = dot(p, params.up.xyz) / z / params.frame.y;
    return vec3<f32>(x, y, z);
}

@vertex
fn vs_dust(@builtin(vertex_index) index: u32, @builtin(instance_index) mote: u32) -> Streak {
    var out: Streak;
    let id = i32(mote);
    let seed = vec3<f32>(hash3(vec3<i32>(id, 1, 7)), hash3(vec3<i32>(id, 2, 11)), hash3(vec3<i32>(id, 3, 13)));
    // The box wraps round the camera, so the motes never run out.
    let head = fract(seed + params.dust.xyz) * 2.0 - 1.0;
    let tail = head + params.motion.xyz * params.dust.w;
    // Fade near the box's faces, where a mote wraps to the far side.
    let edge = 1.0 - max(max(abs(head.x), abs(head.y)), abs(head.z));
    let near = 1.0 - smoothstep01(0.4, 1.6, length(head));
    let brightness = params.burn.x * smoothstep01(0.0, 0.15, edge) * near;
    var a = project_dust(head);
    var b = project_dust(tail);
    if (a.z < 0.05 || brightness <= 0.0) {
        out.position = vec4<f32>(0.0, 0.0, 0.0, 1.0);
        out.along = vec3<f32>(0.0);
        return out;
    }
    if (b.z < 0.05) {
        // Cut where the tail goes behind the camera.
        let t = (a.z - 0.05) / max(a.z - b.z, 1e-4);
        b = project_dust(mix(head, tail, t));
    }
    // A thin quad from head to tail, a little over a pixel wide.
    let pixel = 1.0 / params.frame.x;
    var d = b.xy - a.xy;
    if (length(d) < pixel) {
        d = vec2<f32>(pixel, 0.0);
    }
    let dir = normalize(d);
    let across = vec2<f32>(-dir.y, dir.x) * pixel * 1.3;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index];
    let at = mix(a.xy, a.xy + d, c.x) + across * c.y;
    out.position = vec4<f32>(at, 0.0, 1.0);
    out.along = vec3<f32>(c.x, c.y, brightness);
    return out;
}

@fragment
fn fs_dust(in: Streak) -> @location(0) vec4<f32> {
    let fade = (1.0 - in.along.x) * (1.0 - in.along.y * in.along.y);
    // Lit by the engines' glow from behind: warm white.
    return vec4<f32>(vec3<f32>(1.0, 0.86, 0.66) * fade * in.along.z * 1.6, 0.0);
}

struct Rim {
    @builtin(position) position: vec4<f32>,
    @location(0) at: vec2<f32>,
};

@vertex
fn vs_rim(@builtin(vertex_index) index: u32) -> Rim {
    let side = index / 6u;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index % 6u];
    let angle = (f32(side) + c.x) * TAU / f32(RIM_SIDES);
    // Out past the panel's circle, so the polygon's flats cover it.
    let radius = mix(RIM_INNER * 0.98, 1.0 / cos(PI / f32(RIM_SIDES)), c.y);
    let at = vec2<f32>(cos(angle), sin(angle)) * radius;
    var out: Rim;
    out.position = vec4<f32>(at, 0.0, 1.0);
    out.at = at;
    return out;
}

@fragment
fn fs_rim(in: Rim) -> @location(0) vec4<f32> {
    let r = length(in.at);
    let t = params.burn.y;
    // The exhaust's light thrown forward round the hull: strongest at the
    // bottom, where the engines sit, and flickering with the burn.
    let well = smoothstep01(RIM_INNER, 1.0, r);
    let below = clamp(-in.at.y / max(r, 1e-4), 0.0, 1.0);
    let flicker = 0.86 + 0.08 * sin(t * 37.0) + 0.06 * sin(t * 23.0 + in.at.x * 3.0);
    let glow = vec3<f32>(1.0, 0.42, 0.10) * well * well * well * (0.04 + 0.22 * below * below) * flicker * params.burn.x;
    return vec4<f32>(glow, 0.0);
}
