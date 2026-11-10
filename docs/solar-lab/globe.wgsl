// targets: rgba8unorm-srgb
// entries: fs_globe fs_glow
// vertex: vs_globe
// variants: SEA=1 CLOUDS=1 NIGHT=1 AIR=1 | SEA=1 CLOUDS=1 NIGHT=0 AIR=1 | CLOUDS=1 NIGHT=0 AIR=1 | NIGHT=2 AIR=1 | NIGHT=0 AIR=0 | NIGHT=2 AIR=0 | AIR=1 GIANT=1 | AIR=1 GIANT=1 RINGS=1 | AIR=1 | NIGHT=2 AIR=1 GIANT=1
//
// A prototype for docs/solar-perf-plan.md (brief S1): a mapped world drawn
// as a patch of its latitude-longitude grid. Everything that varies smoothly
// over the sphere is worked out at the vertices; the fragment shader keeps
// only what depends on a texel. `fs_globe` is the whole world in one pass;
// `fs_glow` is the added light of a world with seas as a second, added pass
// over `fs_globe` built with CLOUDS alone. Not wired into the app: compile it
// under the V3D shader-db with the shader lab (the plan says how).

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;

struct Body {
    place: vec4<f32>,
    rot: array<vec4<f32>, 3>,
    look: vec4<f32>,
    air: vec4<f32>,
    colour: vec4<f32>,
    ring: vec4<f32>,
    ring_colour: vec4<f32>,
    rect: vec4<f32>,
    shade: vec4<f32>,
    // The patch of the grid drawn: longitude and latitude of its corner,
    // then their spans, in radians.
    window: vec4<f32>,
    // x: cells along each side; y: how far the mesh stands proud of the
    // surface, in radii, so the soft edge has pixels to fade over.
    mesh: vec4<f32>,
};

struct Params {
    frame: vec4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    forward: vec4<f32>,
    star: vec4<f32>,
    star_colour: vec4<f32>,
    light: vec4<f32>,
    bodies: array<Body, 16>,
};

@group(0) @binding(0) var<uniform> params: Params;
@group(0) @binding(2) var map_sampler: sampler;
@group(0) @binding(3) var a_day: texture_2d<f32>;
@group(0) @binding(4) var a_relief: texture_2d<f32>;
@group(0) @binding(5) var a_extra: texture_2d<f32>;
@group(0) @binding(13) var ring_profile: texture_2d<f32>;

override SEA: bool = false;
override CLOUDS: bool = false;
override RINGS: bool = false;
override AIR: i32 = 0;
override NIGHT: i32 = 0;
override GIANT: i32 = 0;

const SKYLIGHT: vec3<f32> = vec3<f32>(0.035, 0.060, 0.110);
const BIAS: f32 = 0.45;
const GRID_STRIDE: u32 = 129u;

fn smoothstep01(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn airmass(mu: f32) -> f32 {
    let m = max(mu, -0.02);
    return 1.0 / (m + 0.025 + 0.045 * exp(-m * 30.0));
}

fn star_light() -> vec3<f32> {
    return params.light.rgb * 1.25;
}

fn skylight() -> vec3<f32> {
    return SKYLIGHT * params.light.rgb;
}

fn from_body(i: u32, v: vec3<f32>) -> vec3<f32> {
    return params.bodies[i].rot[0].xyz * v.x + params.bodies[i].rot[1].xyz * v.y + params.bodies[i].rot[2].xyz * v.z;
}

struct Globe {
    @builtin(position) position: vec4<f32>,
    // The map's coordinates, and the clouds' turned longitude.
    @location(0) uv: vec3<f32>,
    // The soft edge in pixels inside the limb (negative: inside), the sun's
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
    return out;
}

@fragment
fn fs_globe(in: Globe) -> @location(0) vec4<f32> {
    let mu = in.geo.y;
    let relief = textureSampleBias(a_relief, map_sampler, in.uv.xy, BIAS + 0.4);
    var cloud = 0.0;
    if (CLOUDS) {
        cloud = textureSampleBias(a_extra, map_sampler, in.uv.zy, BIAS).g;
    }
    var albedo = textureSampleBias(a_day, map_sampler, in.uv.xy, BIAS).rgb * (1.0 - 0.2 * relief.b);
    if (GIANT == 1) {
        albedo *= 0.55 + 0.45 * sqrt(in.geo.z);
    }
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
        let toward = textureSampleBias(a_extra, map_sampler, in.uv.zy + in.shift.xy, BIAS + 1.0).g;
        let shadow = 0.6 * toward * smoothstep01(-0.05, 0.12, mu);
        let side = select(1.0, clamp(1.0 - 2.0 * (toward - cloud), 0.4, 1.3), mu > -0.05);
        let lit_deck = in.deck * ((0.30 + 0.70 * sqrt(max(mu, 0.0))) * side + (1.0 - cloud) * in.slope.z) + sky * (sky_rise * 1.275);
        let thick = min(cloud * (2.2 - 1.2 * in.geo.z), 1.0);
        rgb = lit_deck * thick + rgb * ((1.0 - thick) * (1.0 - shadow));
    }
    if (AIR == 1) {
        rgb += in.sheen;
    }
    if (SEA) {
        let water = relief.b * (1.0 - cloud);
        let away = 1.0 - in.geo.z;
        let away2 = away * away;
        let fresnel = 0.02 + 0.98 * away2 * away2 * away;
        let log_nh = log2(max(in.geo.w, 0.0));
        let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
        rgb += (in.direct * (day * glint * (0.25 + fresnel)) + sky * (2.0 * fresnel * smoothstep01(-0.1, 0.3, mu))) * water;
    }
    if (NIGHT == 1) {
        let night = textureSampleBias(a_extra, map_sampler, in.uv.xy, BIAS + 0.6).r;
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu);
        let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, night));
        rgb += tint * in.thru * (night * night * 2.2 * darkness * (1.0 - cloud * 0.85));
    } else if (NIGHT == 2) {
        let night = textureSampleBias(a_extra, map_sampler, in.uv.xy, BIAS + 0.6).r;
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu);
        let heat = mix(vec3<f32>(1.0, 0.25, 0.04), vec3<f32>(1.0, 0.75, 0.3), night * night);
        rgb += heat * in.thru * (night * (0.15 + 0.55 * darkness));
    }
    if (RINGS) {
        let x = in.slope.w;
        let density = textureSampleLevel(ring_profile, map_sampler, vec2<f32>(x, in.shift.w), 0.0).r;
        rgb *= 1.0 - density * in.shift.z * step(0.0, x) * step(x, 1.0);
    }
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(rgb * cover, cover);
}

// The added light of a world with seas, over the ground and clouds: the
// sea's glint and sheen, the night side's lights, the air's own sheen.
@fragment
fn fs_glow(in: Globe) -> @location(0) vec4<f32> {
    let mu = in.geo.y;
    let cloud = textureSampleBias(a_extra, map_sampler, in.uv.zy, BIAS).g;
    var rgb = vec3<f32>(0.0);
    let water = textureSampleBias(a_relief, map_sampler, in.uv.xy, BIAS + 0.4).b * (1.0 - cloud);
    let away = 1.0 - in.geo.z;
    let away2 = away * away;
    let fresnel = 0.02 + 0.98 * away2 * away2 * away;
    let log_nh = log2(max(in.geo.w, 0.0));
    let glint = exp2(log_nh * 260.0) * 0.9 + exp2(log_nh * 45.0) * 0.06;
    rgb = (in.direct * (smoothstep01(-0.035, 0.02, mu) * glint * (0.25 + fresnel)) + in.thru * skylight() * (2.0 * fresnel * smoothstep01(-0.1, 0.3, mu))) * water;
    if (NIGHT == 1) {
        let night = textureSampleBias(a_extra, map_sampler, in.uv.xy, BIAS + 0.6).r;
        let darkness = 1.0 - smoothstep01(-0.10, 0.03, mu);
        let tint = mix(vec3<f32>(1.0, 0.55, 0.22), vec3<f32>(1.0, 0.88, 0.66), smoothstep01(0.2, 0.9, night));
        rgb += tint * in.thru * (night * night * 2.2 * darkness * (1.0 - cloud * 0.85));
    }
    rgb += in.sheen;
    let cover = smoothstep01(-0.75, 0.75, -in.geo.x);
    return vec4<f32>(rgb * cover, 0.0);
}
