// The solar tour's frame. Prepended with `noise.wgsl` and constants.
//
// Two passes into one target:
// - `fs_background` over the whole frame: stars, the galaxy's band, the
//   star's glare, and the hyperjump's streaks and tunnel.
// - `fs_body`, one screen-space quad per body, drawn far to near and blended:
//   only the pixels a body covers pay for it. Each ray traces its body, its
//   atmosphere and its rings.

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
    // Its light, and w: how much of its disc is in view.
    star_colour: vec4<f32>,
    // Rows turning world directions into the sky map's frame.
    sky: array<vec4<f32>, 3>,
    // x: warp 0..1; y: flash; z: seconds; w: unused.
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

const SKYLIGHT: vec3<f32> = vec3<f32>(0.035, 0.060, 0.110);
const SCALE_HEIGHT: f32 = 0.011;
const ATMOSPHERE: f32 = 1.075;
const BIAS: f32 = 1.37;

fn smoothstep01(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn airmass(mu: f32) -> f32 {
    let m = max(mu, -0.02);
    return 1.0 / (m + 0.025 + 0.045 * exp(-m * 30.0));
}

// A soft shoulder for the brightest values, then gamma.
fn encode(linear: vec3<f32>) -> vec3<f32> {
    let l = max(linear, vec3<f32>(0.0));
    let over = max(l - vec3<f32>(0.8), vec3<f32>(0.0));
    let shaped = min(l, vec3<f32>(0.8)) + 0.2 * (vec3<f32>(1.0) - exp(-over * 5.0));
    return pow(shaped, vec3<f32>(1.0 / 2.2));
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
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

// One layer of stars: a star in some cells of a grid `scale` cells across,
// each a point a pixel or so wide, coloured by its temperature.
fn stars(dir: vec3<f32>, scale: f32, density: f32, brightness: f32, seed: i32) -> vec3<f32> {
    let x = dir * scale;
    let cell = vec3<i32>(floor(x));
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
    let size = params.frame.z * 0.75;
    let b = h / density;
    let glow = exp(-angle * angle / (size * size)) * brightness * (0.15 + b * b * 1.6);
    let t = hash3(cell + vec3<i32>(seed, 4, 0));
    let tint = mix(vec3<f32>(1.0, 0.72, 0.5), vec3<f32>(0.7, 0.82, 1.0), t);
    return tint * glow;
}

fn starfield(dir: vec3<f32>) -> vec3<f32> {
    return stars(dir, 60.0, 0.30, 1.3, 11) + stars(dir, 170.0, 0.10, 0.6, 23) + stars(dir, 400.0, 0.025, 0.4, 37);
}

fn sky_at(dir: vec3<f32>) -> vec3<f32> {
    let d = vec3<f32>(dot(params.sky[0].xyz, dir), dot(params.sky[1].xyz, dir), dot(params.sky[2].xyz, dir));
    let uv = vec2<f32>(0.5 + atan2(d.x, d.z) / TAU, 0.5 - asin(clamp(d.y, -1.0, 1.0)) / PI);
    return textureSampleLevel(sky_map, map_sampler, uv, 0.0).rgb;
}

@fragment
fn fs_background(in: Full) -> @location(0) vec4<f32> {
    let dir = ray_at(in.position.xy);
    let warp = params.warp.x;
    var colour = vec3<f32>(0.0);
    if (warp < 0.02) {
        colour = sky_at(dir) + starfield(dir);
    } else {
        // Streaks: the stars smeared along lines out from the heading, longer
        // the faster, and the galaxy washed toward violet.
        let forward = params.forward.xyz;
        let reach = warp * warp * 0.35;
        // Six taps of the two nearer layers: the faintest stars would only
        // add grain to the streaks, at half the cost again.
        var sum = vec3<f32>(0.0);
        for (var k = 0; k < 6; k++) {
            let d = normalize(dir - forward * reach * f32(k) / 6.0);
            sum += stars(d, 60.0, 0.30, 1.3, 11) + stars(d, 170.0, 0.10, 0.6, 23);
        }
        sum *= 8.0 / 6.0;
        let off_axis = length(cross(dir, forward));
        colour = sum * (0.35 + 1.2 * warp) / 3.0 * (0.6 + off_axis);
        colour += sky_at(dir) * (1.0 - warp);
        // The tunnel: bands of light rushing past, brighter toward the rim.
        let along = dot(dir, forward);
        let angle = atan2(dot(dir, params.up.xyz), dot(dir, params.right.xyz));
        // Around the heading by its cosine and sine, so there is no seam;
        // along it by depth, rushing past.
        let around = vec2<f32>(cos(angle), sin(angle)) * 7.0;
        let depth = 1.0 / max(1.0 - along, 0.02) * 0.5 - params.warp.z * 2.5;
        let lines = noise(vec3<f32>(around * 3.0, depth * 0.35));
        let streak = 1.0 - smoothstep01(0.0, 0.18, abs(lines));
        let tunnel = smoothstep01(0.25, 0.95, off_axis) * streak * (0.6 + 0.4 * noise(vec3<f32>(around, depth)));
        colour += mix(vec3<f32>(0.30, 0.45, 1.0), vec3<f32>(0.65, 0.45, 1.0), off_axis) * tunnel * warp * warp * 0.3;
        // The heading glows.
        colour += vec3<f32>(0.7, 0.8, 1.0) * exp(-off_axis * off_axis * 140.0) * warp * 0.6;
    }
    // The star's glare: a halo falling off from its disc, only where it is in
    // view.
    let s = params.star.xyz;
    let distance = length(s);
    let disc = params.star.w / max(distance, 1e-3);
    let cos_angle = dot(dir, s / max(distance, 1e-3));
    let angle = max(acos(clamp(cos_angle, -1.0, 1.0)), disc * 0.5);
    let glare = disc / angle;
    colour += params.star_colour.rgb * params.star_colour.w * (glare * glare * 0.06 + glare * 0.012) * (1.0 - warp);
    colour = mix(colour, vec3<f32>(1.0), params.warp.y);
    return vec4<f32>(encode(colour), 1.0);
}

// ---------------------------------------------------------------------------
// Bodies.
// ---------------------------------------------------------------------------

struct Quad {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) body: u32,
};

@vertex
fn vs_body(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Quad {
    let rect = params.bodies[instance].rect;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[vertex];
    var out: Quad;
    out.position = vec4<f32>(mix(rect.xy, rect.zw, c), 0.0, 1.0);
    out.body = instance;
    return out;
}

fn map_uv(n: vec3<f32>) -> vec2<f32> {
    return vec2<f32>(0.5 + atan2(n.x, n.z) / TAU, 0.5 - asin(clamp(n.y, -1.0, 1.0)) / PI);
}

fn unwrap(g: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(g.x - round(g.x), g.y);
}

fn to_body(b: Body, v: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(b.rot[0].xyz, v), dot(b.rot[1].xyz, v), dot(b.rot[2].xyz, v));
}

fn from_body(b: Body, v: vec3<f32>) -> vec3<f32> {
    return b.rot[0].xyz * v.x + b.rot[1].xyz * v.y + b.rot[2].xyz * v.z;
}

struct Texels {
    day: vec3<f32>,
    relief: vec4<f32>,
    // Night glow, cloud.
    extra: vec2<f32>,
    cloud_shadow: f32,
    cloud_side: f32,
};

fn sample_planet(slot: i32, uv: vec2<f32>, cuv: vec2<f32>, shift: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> Texels {
    var t: Texels;
    let g = BIAS;
    if (slot == 0) {
        t.day = textureSampleGrad(a_day, map_sampler, uv, gx * g, gy * g).rgb;
        t.relief = textureSampleGrad(a_relief, map_sampler, uv, gx * g * 1.3, gy * g * 1.3);
        t.extra = vec2<f32>(
            textureSampleGrad(a_extra, map_sampler, uv, gx * g * 1.5, gy * g * 1.5).r,
            textureSampleGrad(a_extra, map_sampler, cuv, gx * g, gy * g).g,
        );
        t.cloud_shadow = textureSampleGrad(a_extra, map_sampler, uv + shift, gx * g * 2.0, gy * g * 2.0).g;
        t.cloud_side = textureSampleGrad(a_extra, map_sampler, cuv + shift * 0.35, gx * g, gy * g).g;
    } else {
        t.day = textureSampleGrad(b_day, map_sampler, uv, gx * g, gy * g).rgb;
        t.relief = textureSampleGrad(b_relief, map_sampler, uv, gx * g * 1.3, gy * g * 1.3);
        t.extra = vec2<f32>(
            textureSampleGrad(b_extra, map_sampler, uv, gx * g * 1.5, gy * g * 1.5).r,
            textureSampleGrad(b_extra, map_sampler, cuv, gx * g, gy * g).g,
        );
        t.cloud_shadow = textureSampleGrad(b_extra, map_sampler, uv + shift, gx * g * 2.0, gy * g * 2.0).g;
        t.cloud_side = textureSampleGrad(b_extra, map_sampler, cuv + shift * 0.35, gx * g, gy * g).g;
    }
    return t;
}

fn sample_moon(slot: i32, layer: i32, uv: vec2<f32>, gx: vec2<f32>, gy: vec2<f32>) -> Texels {
    var t: Texels;
    let g = BIAS;
    if (slot == 0) {
        t.day = textureSampleGrad(a_moon_day, map_sampler, uv, layer, gx * g, gy * g).rgb;
        t.relief = textureSampleGrad(a_moon_relief, map_sampler, uv, layer, gx * g, gy * g);
    } else {
        t.day = textureSampleGrad(b_moon_day, map_sampler, uv, layer, gx * g, gy * g).rgb;
        t.relief = textureSampleGrad(b_moon_relief, map_sampler, uv, layer, gx * g, gy * g);
    }
    // A moon's third relief channel is molten glow, not water.
    t.extra = vec2<f32>(t.relief.b, 0.0);
    t.relief.b = 0.0;
    t.cloud_shadow = 0.0;
    t.cloud_side = 0.0;
    return t;
}

// How much of the star other bodies hide from a point: eclipses, with a soft
// penumbra.
fn eclipse(self_index: u32, point: vec3<f32>, to_star: vec3<f32>) -> f32 {
    var lit = 1.0;
    let count = u32(params.frame.w);
    for (var i = 0u; i < count; i++) {
        if (i == self_index) {
            continue;
        }
        let other = params.bodies[i];
        if (other.look.x < 0.0) {
            continue;
        }
        let rel = other.place.xyz - point;
        let along = dot(rel, to_star);
        if (along <= 0.0) {
            continue;
        }
        let miss = length(rel - to_star * along);
        let r = other.place.w;
        // The penumbra widens with distance, as the star's disc allows.
        let spread = along * params.star.w / max(length(params.star.xyz - point), 1.0);
        lit *= smoothstep01(r - spread, r + spread, miss);
    }
    return lit;
}

// Ring density at radius `r` (in planet radii): ringlets, gaps and a main
// division, from 1D value noise.
fn ring_density(r: f32, b: Body) -> f32 {
    let inner = b.ring.x;
    let outer = b.ring.y;
    let x = (r - inner) / (outer - inner);
    if (x < 0.0 || x > 1.0) {
        return 0.0;
    }
    let seed = b.ring.w;
    let n1 = noise(vec3<f32>(x * 40.0 + seed, 0.5, 0.5)) * 0.5 + 0.5;
    let n2 = noise(vec3<f32>(x * 160.0 + seed, 1.5, 0.5)) * 0.5 + 0.5;
    let n3 = noise(vec3<f32>(x * 9.0 + seed, 2.5, 0.5)) * 0.5 + 0.5;
    let division = 1.0 - exp(-pow((x - 0.62) / 0.025, 2.0)) * 0.9;
    let edges = smoothstep01(0.0, 0.04, x) * smoothstep01(1.0, 0.9, x);
    return clamp((0.25 + 0.75 * n3) * (0.45 + 0.55 * n1) * (0.6 + 0.4 * n2) * division * edges * 1.4, 0.0, 1.0);
}

struct Hit {
    colour: vec3<f32>,
    alpha: f32,
};

@fragment
fn fs_body(in: Quad) -> @location(0) vec4<f32> {
    let index = in.body;
    let b = params.bodies[index];
    let dir = ray_at(in.position.xy);
    let c = b.place.xyz;
    let radius = b.place.w;
    let along = max(dot(c, dir), 0.0);
    let nearest = dir * along;
    let closest = length(nearest - c) / radius;
    let hit = 1.0 - closest * closest;
    let t_ground = (along - sqrt(max(hit, 0.0)) * radius);
    let point = select(nearest, dir * t_ground, hit > 0.0);
    let n = normalize(point - c);
    // Body-frame coordinates and their screen derivatives, before branching.
    let nb = to_body(b, n);
    let uv = map_uv(nb);
    let cuv = vec2<f32>(uv.x + b.rot[0].w, uv.y);
    let gx = unwrap(dpdx(uv));
    let gy = unwrap(dpdy(uv));

    let pixel = params.frame.z;
    let distance = max(select(along, t_ground, hit > 0.0), 1e-4);
    let edge = (closest - 1.0) * radius / (distance * pixel);
    let coverage = smoothstep01(-0.75, 0.75, -edge);

    // The star: a bright disc, darker toward its limb, faintly mottled.
    if (b.look.x < 0.0) {
        if (coverage <= 0.0) {
            discard;
        }
        let mu = sqrt(max(hit, 0.0));
        let granules = 0.92 + 0.08 * noise(nb * 60.0 + vec3<f32>(params.warp.z * 0.05));
        let light = params.star_colour.rgb * 5.0 * (0.4 + 0.6 * mu) * granules;
        return vec4<f32>(encode(light) * coverage, coverage);
    }

    let to_star_point = params.star.xyz - point;
    let sun = normalize(to_star_point);
    let air = b.air.rgb;
    let has_air = max(max(air.r, air.g), air.b) > 0.0;
    let star_light = params.star_colour.rgb * 1.25;

    // Rings: where the ray crosses the equatorial plane.
    var ring_alpha = 0.0;
    var ring_colour = vec3<f32>(0.0);
    var t_ring = 1e30;
    if (b.ring.z > 0.0) {
        let axis = b.rot[1].xyz;
        let denom = dot(dir, axis);
        if (abs(denom) > 1e-5) {
            let t = dot(c, axis) / denom;
            if (t > 0.0) {
                let p = dir * t - c;
                let r = length(p) / radius;
                let density = ring_density(r, b) * b.ring.z;
                if (density > 0.001) {
                    t_ring = t;
                    // The planet's shadow across the rings.
                    let rel = -p;
                    let a2 = dot(rel, sun);
                    let miss = length(rel - sun * a2);
                    let shadowed = select(1.0, smoothstep01(radius * 0.97, radius * 1.03, miss), a2 > 0.0);
                    // Lit from either face, a little more when back-lit.
                    let facing = abs(dot(sun, axis));
                    let light = star_light * (0.25 + 0.75 * sqrt(facing)) * shadowed;
                    ring_colour = b.ring_colour.rgb * (light + SKYLIGHT * 0.3);
                    ring_alpha = density;
                }
            }
        }
    }

    var result = vec4<f32>(0.0);
    if (coverage > 0.0) {
        let mu_sun = dot(n, sun);
        let mu_view = clamp(dot(n, -dir), 0.0, 1.0);
        let slot = i32(b.look.y);
        let layer = i32(b.look.z);
        let kind = i32(b.look.x);

        // Tangent frame in the body, carried back to the world.
        let cos_lat = sqrt(max(nb.x * nb.x + nb.z * nb.z, 1e-8));
        let sin_lon = nb.x / cos_lat;
        let cos_lon = nb.z / cos_lat;
        let east = from_body(b, vec3<f32>(cos_lon, 0.0, -sin_lon));
        let north = from_body(b, vec3<f32>(-nb.y * sin_lon, cos_lat, -nb.y * cos_lon));
        let reach = 0.006 / max(mu_sun, 0.12);
        let shift = vec2<f32>(
            dot(sun, east) * reach / (TAU * max(cos_lat, 0.05)),
            -dot(sun, north) * reach / PI,
        );

        var t: Texels;
        if (slot >= 0 && layer < 0) {
            t = sample_planet(slot, uv, cuv, shift, gx, gy);
        } else if (slot >= 0) {
            t = sample_moon(slot, layer, uv, gx, gy);
        } else {
            // No maps: a flat colour, banded on giants.
            t.day = b.colour.rgb * (1.0 + b.colour.w * sin(nb.y * 23.0 + sin(nb.y * 7.0) * 2.0) * 0.5);
            t.relief = vec4<f32>(0.5, 0.5, 0.0, 1.0);
            t.extra = vec2<f32>(0.0);
            t.cloud_shadow = 0.0;
            t.cloud_side = 0.0;
        }
        let bump = t.relief.rg - vec2<f32>(0.5);
        let water = t.relief.b;
        let tilted = normalize(n + (east * bump.x + north * bump.y) * 0.55);
        let cloud = t.extra.y;

        var shade = eclipse(index, point, sun);
        // The rings' shadow on the planet.
        if (b.ring.z > 0.0) {
            let axis = b.rot[1].xyz;
            let denom = dot(sun, axis);
            if (abs(denom) > 1e-4) {
                let s = dot(c - point, axis) / denom;
                if (s > 0.0) {
                    let r = length(point + sun * s - c) / radius;
                    shade *= 1.0 - ring_density(r, b) * b.ring.z * 0.85;
                }
            }
        }

        var sun_ground = star_light * smoothstep01(-0.03, 0.03, mu_sun);
        if (has_air) {
            sun_ground = star_light * exp(-air * airmass(mu_sun)) * smoothstep01(-0.035, 0.02, mu_sun);
        }
        sun_ground *= shade;
        let sky = select(vec3<f32>(0.0), SKYLIGHT * smoothstep01(-0.15, 0.4, mu_sun), has_air);
        var albedo = t.day * mix(1.0, 0.8, water);
        if (kind == 6 || kind == 7) {
            // Giants darken toward the limb.
            albedo *= 0.55 + 0.45 * sqrt(mu_view);
        }
        let direct = sun_ground * max(dot(tilted, sun) + 0.03, 0.0) * (1.0 - 0.6 * t.cloud_shadow);
        var colour = albedo * (direct + sky);

        let fresnel = 0.02 + 0.98 * pow(1.0 - mu_view, 5.0);
        let open_sea = water * (1.0 - cloud);
        if (open_sea > 0.0) {
            let gloss = normalize(sun - dir);
            let log_nh = log2(max(dot(n, gloss), 0.0));
            let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
            colour += sun_ground * glint * (0.25 + fresnel) * open_sea;
            colour += SKYLIGHT * 2.0 * fresnel * water * smoothstep01(-0.1, 0.3, mu_sun);
        }

        var sun_high = star_light * shade;
        if (has_air) {
            sun_high = star_light * exp(-air * airmass(mu_sun + 0.04) * 0.55) * shade;
        }
        if (cloud > 0.0) {
            let sun_cloud = sun_high * smoothstep01(-0.08, 0.04, mu_sun);
            let side = clamp(1.0 - 1.4 * (t.cloud_side - cloud), 0.55, 1.15);
            let cloud_lit = 0.85 * (sun_cloud * (0.30 + 0.70 * sqrt(max(mu_sun, 0.0))) * side + sky * 1.5);
            colour = mix(colour, cloud_lit, cloud);
        }

        let night = t.extra.x;
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu_sun);
        if (b.look.w > 0.5 && b.look.w < 1.5 && darkness > 0.002) {
            let glow = night * night * 2.2;
            let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, night));
            colour += tint * glow * darkness * (1.0 - cloud * 0.85);
        } else if (b.look.w > 1.5) {
            // Molten rock glows on both sides, more visibly in the dark.
            let heat = mix(vec3<f32>(1.0, 0.25, 0.04), vec3<f32>(1.0, 0.75, 0.3), night * night);
            colour += heat * night * (0.15 + 0.55 * darkness);
        }

        if (has_air) {
            let through = exp(-air * airmass(mu_view));
            let sun_air = sun_high * smoothstep01(-0.10, 0.08, mu_sun);
            colour = colour * through + sun_air * (vec3<f32>(1.0) - through) * 0.45;
        }
        result = vec4<f32>(encode(colour) * coverage, coverage);
    }

    // The air seen edge-on, past the limb.
    if (has_air && coverage < 1.0 && closest < ATMOSPHERE) {
        let r = closest;
        let h = max(r - 1.0, 0.0);
        let depth = air * sqrt(TAU * r / SCALE_HEIGHT) * exp(-h / SCALE_HEIGHT);
        let tangent = n;
        let mu = dot(tangent, sun);
        let lit = star_light * exp(-air * airmass(mu + 0.08) * 0.6) * smoothstep01(-0.30, 0.05, mu) * eclipse(index, point, sun);
        let glow = encode(lit * (vec3<f32>(1.0) - exp(-depth)) * 0.9);
        let a = min(max(max(glow.r, glow.g), glow.b), 1.0) * (1.0 - coverage);
        result = vec4<f32>(result.rgb + glow * (1.0 - coverage), result.a + a);
    }

    // Rings in front of the planet, or seen past it.
    if (ring_alpha > 0.0 && (t_ring < t_ground || hit <= 0.0 || coverage < 1.0)) {
        let in_front = t_ring < t_ground || hit <= 0.0;
        let a = ring_alpha * select(1.0 - coverage, 1.0, in_front);
        result = vec4<f32>(encode(ring_colour) * a + result.rgb * (1.0 - a), a + result.a * (1.0 - a));
    }
    if (result.a <= 0.0) {
        discard;
    }
    return result;
}
