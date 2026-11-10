// targets: rgba8unorm-srgb
// entries: fs_limb_ring
// vertex: vs_limb_ring
//
// A prototype for docs/solar-perf-plan.md (brief S2): the air past a limb as an
// annulus round the silhouette, its light worked out at the vertices.

const TAU: f32 = 6.283185307;
const PI: f32 = 3.141592653;
const SCALE_HEIGHT: f32 = 0.011;
const ATMOSPHERE: f32 = 1.075;
const LIMB_SIDES: u32 = 128u;

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

fn smoothstep01(e0: f32, e1: f32, x: f32) -> f32 {
    let t = clamp((x - e0) / (e1 - e0), 0.0, 1.0);
    return t * t * (3.0 - 2.0 * t);
}

fn airmass(mu: f32) -> f32 {
    let m = max(mu, -0.02);
    return 1.0 / (m + 0.025 + 0.045 * exp(-m * 30.0));
}

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
    // Two bands of quads: from just inside the limb to a little out, and on
    // to the top of the air.
    let corner = vertex % 6u;
    let quad = vertex / 6u;
    let k = quad / 2u + select(0u, 1u, corner == 1u || corner == 4u || corner == 5u);
    let ring = quad % 2u + select(0u, 1u, corner == 2u || corner == 3u || corner == 5u);
    let pixel = params.frame.z * sqrt(max(distance * distance - radius * radius, 0.0)) / radius;
    var rho = 1.0 - 2.5 * pixel;
    if (ring == 1u) {
        rho = 1.02;
    } else if (ring == 2u) {
        rho = ATMOSPHERE / cos(PI / f32(LIMB_SIDES));
    }
    let a = f32(k) * TAU / f32(LIMB_SIDES);
    let out_dir = e1 * cos(a) + e2 * sin(a);
    let sin_t = min(rho * radius / distance, 0.999);
    let spread = distance * sin_t / sqrt(1.0 - sin_t * sin_t);
    let p = c + out_dir * spread;
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
    out.lit = params.light.rgb * 1.25 * exp(-air * airmass(mu + 0.08) * 0.6) * (0.9 * smoothstep01(-0.30, 0.05, mu) * eclipse(i, nearest, sun));
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
