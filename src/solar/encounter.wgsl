// Between the legs of a jump: what the ship meets out in the deep (`voyage.rs`).
// Appended to `frame.wgsl`, sharing its parameters, textures and helpers.
//
// Each stopover is drawn in its own frame, the encounter frame: the ship at
// `EV_SHIP`, +z the way the jump goes, its axes in the world in rows 2-4. A
// pixel's ray is turned into that frame (`to_ev`) and everything is traced
// or glowed along it, analytically where it can be. One pipeline per event
// (`EVENT`), so each holds only its own code:
// - `fs_event`, over the sky (premultiplied): the objects and their light.
//   Not the gate's or the Dyson swarm's: they are drawn bounded, a draw a
//   part over only the pixels it lights (`vs_glow`, `vs_gate_field`,
//   `vs_dyson_panel`, `vs_dyson_star`).
// - `fs_coarse`, in place of the sky: the black hole, which lenses it, its
//   disc and jets drawn at half the resolution first (`hole_layers`).
// - `vs_rock` / `fs_rock`, instanced: rocks round the ship (the asteroid
//   field), debris (the derelict) or ice (the comet), each a lumpy sphere on
//   a quad, writing the depth so near ones hide far ones.
// - `fs_flux`, over the drive's light (added): something seen inside a leg.
// - `vs_overlay` / `fs_overlay`, over everything: the alarm round the rim,
//   hits on the shield, the tractor beam's hold and flashes.
//
// Rows of `params.ev` (`encounter_rows` in `mod.rs`):
//   0: event, seconds into it, its duration, its draw 0..1
//   1: the ship in the frame, w 1 out in the deep
//   2-4: the frame's x, y and z in the world; w the hit, the beam, the flash
//   5-7: the event's objects (place, size)
//   8: a rotation (axis, angle)
//   9: the event's own four numbers
//   10: the alarm, how much of the stopover shows, the flux and how much of it
//   11: where a hit lands on the frame (across the half frame), the shake,
//       and seconds into the flux
//   12-27: a battle's ships this frame, two rows each (`battle_ship`)

override EVENT: i32 = 0;

fn ev_t() -> f32 {
    return params.ev[0].y;
}

fn ev_u() -> f32 {
    return clamp(params.ev[0].y / max(params.ev[0].z, 1e-3), 0.0, 1.5);
}

fn ev_ship() -> vec3<f32> {
    return params.ev[1].xyz;
}

fn ev_obj(k: i32) -> vec4<f32> {
    return params.ev[5 + k];
}

// A world direction in the encounter frame, and back.
fn to_ev(d: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(dot(d, params.ev[2].xyz), dot(d, params.ev[3].xyz), dot(d, params.ev[4].xyz));
}

fn from_ev(d: vec3<f32>) -> vec3<f32> {
    return params.ev[2].xyz * d.x + params.ev[3].xyz * d.y + params.ev[4].xyz * d.z;
}

// A fully saturated colour round the wheel.
fn hue_of(h: f32) -> vec3<f32> {
    return clamp(abs(fract(vec3<f32>(h) + vec3<f32>(0.0, 2.0 / 3.0, 1.0 / 3.0)) * 6.0 - 3.0) - 1.0, vec3<f32>(0.0), vec3<f32>(1.0));
}

// `v` turned round unit `axis` by `angle`.
fn turn(v: vec3<f32>, axis: vec3<f32>, angle: f32) -> vec3<f32> {
    let c = cos(angle);
    let s = sin(angle);
    return v * c + cross(axis, v) * s + axis * dot(axis, v) * (1.0 - c);
}

// The baked sky along a direction in the encounter frame.
fn ev_sky(d: vec3<f32>) -> vec3<f32> {
    return textureSampleLevel(starfield, map_sampler, from_ev(d), 0.0).rgb;
}

// Where a ray from `o` along unit `d` meets the sphere at `c` of radius `r`:
// near and far distances, or both negative if it misses.
fn sphere_hits(o: vec3<f32>, d: vec3<f32>, c: vec3<f32>, r: f32) -> vec2<f32> {
    let v = o - c;
    let b = dot(v, d);
    let k = dot(v, v) - r * r;
    let h = b * b - k;
    if (h < 0.0) {
        return vec2<f32>(-1.0, -1.0);
    }
    let q = sqrt(h);
    return vec2<f32>(-b - q, -b + q);
}

// The closest approach of the ray from `o` along unit `d` to the line
// through `p` along unit `m`: the distance, how far along the ray, and how
// far along the line.
fn ray_to_line(o: vec3<f32>, d: vec3<f32>, p: vec3<f32>, m: vec3<f32>) -> vec3<f32> {
    let w = o - p;
    let b = dot(d, m);
    let dd = dot(d, w);
    let e = dot(m, w);
    let denom = max(1.0 - b * b, 1e-5);
    let sc = (b * e - dd) / denom;
    let tc = (e - b * dd) / denom;
    return vec3<f32>(length(w + d * sc - m * tc), sc, tc);
}

// A point's glow on the frame: its distance across the ray from `o` along
// `d`, over how far ahead, in pixels.
fn point_pixels(o: vec3<f32>, d: vec3<f32>, p: vec3<f32>) -> f32 {
    let v = p - o;
    let z = dot(v, d);
    if (z <= 0.0) {
        return 1e6;
    }
    return length(v - d * z) / (z * params.frame.z);
}

// The pixel across the half frame.
fn half_frame(position: vec4<f32>) -> vec2<f32> {
    let half = params.frame.x;
    return vec2<f32>((position.x - half) / half, (half - position.y) / half);
}

// A glint at a point: a hard core of a pixel or two and a soft halo.
fn glint(px: f32, size: f32) -> f32 {
    return exp(-px * px / (2.0 * size * size)) * 2.0 + exp(-px / (6.0 * size)) * 0.15;
}

// ---------------------------------------------------------------------------
// An asteroid field: the far star the rocks are lit by, and the dust between
// them glowing round it. The rocks themselves are `fs_rock`.
// ---------------------------------------------------------------------------

fn ev_light() -> vec3<f32> {
    return normalize(vec3<f32>(params.ev[9].z, params.ev[9].w, params.ev[8].x));
}

fn ev_asteroids(d: vec3<f32>) -> vec4<f32> {
    let l = ev_light();
    let mu = dot(d, l);
    let kind = params.ev[9].x;
    let tint = select(select(vec3<f32>(1.0, 0.8, 0.55), vec3<f32>(0.7, 0.85, 1.0), kind > 0.5), vec3<f32>(1.0, 0.55, 0.35), kind > 1.5);
    // The far star: a hard point and its glare.
    let px = acos(clamp(mu, -1.0, 1.0)) / params.frame.z;
    var colour = vec3<f32>(1.0, 0.95, 0.88) * glint(px, 1.4) * 1.5;
    colour += tint * exp(-px * 0.012) * 0.05;
    // The dust, forward-scattering its light, drifting past.
    let ship = ev_ship();
    let dust = noise(d * 3.0 + ship * 0.002) * 0.5 + noise(d * 7.1 - ship * 0.003) * 0.5;
    let scatter = 0.25 + 1.5 * pow(max(mu, 0.0), 8.0);
    colour += tint * smoothstep01(-0.2, 0.8, dust) * scatter * 0.035 * params.ev[9].y;
    return vec4<f32>(colour, 0.0);
}

// ---------------------------------------------------------------------------
// Rocks, debris and ice round the ship, instanced (`ROCKS` in `mod.rs`).
// ---------------------------------------------------------------------------

struct Rock {
    @builtin(position) position: vec4<f32>,
    // The rock's centre in camera space (right, up, ahead), and its radius.
    @location(0) @interpolate(flat) centre: vec4<f32>,
    // Its seed, kind (0 carbon, 1 ice, 2 iron, 3 a ship's debris, 4 a mote
    // of glowing ice), how brightly a mote glows.
    @location(1) @interpolate(flat) look: vec4<f32>,
};

@vertex
fn vs_rock(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Rock {
    var out: Rock;
    let ship = ev_ship();
    let fi = f32(instance);
    let h = vec4<f32>(
        hash_cell(vec3<f32>(fi, 1.0, 7.0)),
        hash_cell(vec3<f32>(fi, 2.0, 11.0)),
        hash_cell(vec3<f32>(fi, 3.0, 13.0)),
        hash_cell(vec3<f32>(fi, 4.0, 17.0)),
    );
    var p = vec3<f32>(0.0);
    var radius = 0.0;
    var glow = 0.0;
    if (EVENT == 1) {
        // Grains of dust close round the ship, catching the light as they
        // stream past.
        let near = vec3<f32>(90.0, 60.0, 160.0);
        let g = (h.xyz - vec3<f32>(0.5, 0.5, 0.0)) * near;
        p = vec3<f32>(g.xy, ship.z - 8.0 + fract((g.z - ship.z) / near.z) * near.z);
        radius = select(0.0, mix(0.04, 0.12, h.w), instance < 70u);
        glow = 0.35 + 0.65 * h.w;
    } else {
        // Ice glittering round the ship as it crosses the comet's tail.
        let span = vec3<f32>(160.0, 110.0, 300.0);
        let xy = (h.xy - 0.5) * span.xy;
        let z = ship.z - 20.0 + fract((h.z * span.z - ship.z + 20.0) / span.z) * span.z;
        p = vec3<f32>(xy, z);
        radius = mix(0.15, 0.6, h.w);
        glow = 0.3 + 2.5 * params.ev[2].w;
    }
    let w = from_ev(p - ship);
    let cam = vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
    out.centre = vec4<f32>(cam, radius);
    // Kept out of the camera's own space, faded in with the distance: so
    // close, a grain would cover half the frame.
    out.look = vec4<f32>(fi, 4.0, glow * smoothstep01(6.0, 16.0, cam.z), 0.0);
    if (radius <= 0.0 || cam.z < 6.0 || cam.z > 4000.0) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let tan_half = params.frame.y;
    let centre = cam.xy / (cam.z * tan_half);
    let spread = radius * 1.5 / (cam.z * tan_half) + 4.0 / params.frame.x;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex % 6u];
    out.position = vec4<f32>(centre + corner * spread, clamp(cam.z / (cam.z + 400.0), 0.0, 0.999), 1.0);
    return out;
}

// A mote of ice or a grain of dust: a soft point, added.
@fragment
fn fs_rock(in: Rock) -> @location(0) vec4<f32> {
    let s = half_frame(in.position);
    let d = normalize(vec3<f32>(s * params.frame.y, 1.0));
    let c = in.centre.xyz;
    let tca = dot(c, d);
    let across = length(c - d * tca);
    let g = exp(-sq(across / max(in.centre.w, tca * params.frame.z * 1.2)));
    return vec4<f32>(vec3<f32>(0.6, 0.85, 1.0) * g * in.look.z, 0.0);
}

// ---------------------------------------------------------------------------
// A hive cube: a vast cube of machinery, greebled at three scales, lit from
// inside through slits and bays in its colour, scarred, turning slowly, with
// small cubes keeping station and a tractor beam reaching for the ship.
// ---------------------------------------------------------------------------

// The cube itself is a mesh (`fs_model`, family 3): here only the haze of
// its light round it and its tractor beam, each drawn bounded (`PART` 1 and
// 2, `vs_cube_glow`) over only the frame its light reaches.

fn cube_glow() -> vec3<f32> {
    return mix(hue_of(params.ev[9].x), vec3<f32>(1.0), 0.12) * 1.1;
}

// The haze along unit `d`: out to some 2.6 of the cube's radii.
fn cube_haze(d: vec3<f32>) -> vec3<f32> {
    let primary = ev_obj(0);
    let v = primary.xyz - ev_ship();
    let tca = dot(v, d);
    if (tca <= 0.0) {
        return vec3<f32>(0.0);
    }
    let across = length(v - d * tca);
    return cube_glow() * exp(-max(across - primary.w * 1.05, 0.0) / (primary.w * 0.3)) * 0.025;
}

// The tractor beam's ends: from the cube's face toward the ship to just
// under its bow.
fn beam_ends() -> array<vec3<f32>, 2> {
    let ship = ev_ship();
    let primary = ev_obj(0);
    return array<vec3<f32>, 2>(primary.xyz + normalize(ship - primary.xyz) * primary.w, ship + vec3<f32>(0.0, -7.0, 9.0));
}

// The beam along unit `d`: from its root `start` (xyz, and its length) along
// unit `way` (xyz, and the cube's radius), in `colour` (rgb, and its style).
fn cube_beam(d: vec3<f32>, start: vec4<f32>, way: vec4<f32>, colour: vec4<f32>) -> vec3<f32> {
    let near = ray_to_line(ev_ship(), d, start.xyz, way.xyz);
    let f = clamp(near.z / start.w, 0.0, 1.0);
    if (near.y <= 0.0 || near.z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let width = mix(way.w * 0.2, 1.6, f * f);
    let core = exp(-sq(near.x / width));
    var stripes = 0.6 + 0.4 * sin(near.z * 0.35 - ev_t() * 14.0);
    if (colour.w > 0.5) {
        stripes = 0.35 + 0.65 * smoothstep01(0.5, 0.95, abs(sin(near.x / width * 7.0 + near.z * 0.2 - ev_t() * 4.0)));
    }
    return colour.rgb * core * stripes * (0.45 + 0.3 * f) * (1.0 - smoothstep01(0.9, 1.0, f));
}

struct CubeGlow {
    @builtin(position) position: vec4<f32>,
    // The ray through the pixel, in the encounter frame.
    @location(0) ray: vec3<f32>,
    // The beam's (`cube_beam`), worked out once.
    @location(1) @interpolate(flat) start: vec4<f32>,
    @location(2) @interpolate(flat) way: vec4<f32>,
    @location(3) @interpolate(flat) colour: vec4<f32>,
};

@vertex
fn vs_cube_glow(@builtin(vertex_index) vertex: u32) -> CubeGlow {
    var out: CubeGlow;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex % 6u];
    let primary = ev_obj(0);
    var at = vec2<f32>(4.0);
    if (PART == 1) {
        // The haze falls under `FAINT` past 1.05 + 0.3 ln(0.025 x 1.1 /
        // FAINT) radii across the ray: under 2.65.
        let box = sphere_box(ev_camera_point(primary.xyz), primary.w * 2.65);
        if (box.x < 3.0) {
            at = mix(box.xy, box.zw, corner * 0.5 + 0.5);
        }
    }
    // The haze at the far end of the depth mask, as the sky is: only where
    // the cube (drawn first) is not.
    var depth = 1.0;
    if (PART == 2 && params.ev[3].w > 0.001) {
        // The beam, a Gaussian across at most a fifth of the radius; at the
        // depth of its root on the cube's face, in front of all of the cube
        // but its nearer edges.
        let ends = beam_ends();
        let reach = primary.w * 0.2 * gauss_reach(1.2);
        let root = ev_camera_point(ends[0]);
        at = segment_corner(root, ev_camera_point(ends[1]), reach, corner);
        depth = clamp(root.z / (root.z + 400.0), 0.0, 0.999);
        let span = length(ends[1] - ends[0]);
        out.start = vec4<f32>(ends[0], span);
        out.way = vec4<f32>((ends[1] - ends[0]) / span, primary.w);
        out.colour = vec4<f32>(cube_glow() * params.ev[3].w * params.ev[10].y, params.ev[9].z);
    }
    if (at.x > 3.0) {
        return out;
    }
    at = clamp(at, vec2<f32>(-1.0), vec2<f32>(1.0));
    out.position = vec4<f32>(at, depth, 1.0);
    out.ray = to_ev(params.forward.xyz + (params.right.xyz * at.x + params.up.xyz * at.y) * params.frame.y);
    return out;
}

@fragment
fn fs_cube_glow(in: CubeGlow) -> @location(0) vec4<f32> {
    let d = normalize(in.ray);
    if (PART == 1) {
        return vec4<f32>(cube_haze(d) * params.ev[10].y, 0.0);
    }
    return vec4<f32>(cube_beam(d, in.start, in.way, in.colour), 0.0);
}

// ---------------------------------------------------------------------------
// A black hole, in place of the sky: the stars bent round its shadow, its
// photon ring, and the accretion disc seen both in front and, bent over the
// top and bottom of the shadow, from behind; its jets.
// ---------------------------------------------------------------------------

// The disc's light where a ray from `o` along `d` crosses its plane before
// `limit`: colour (premultiplied) and cover.
fn disc_at(o: vec3<f32>, d: vec3<f32>, limit: f32, hole: vec3<f32>, rs: f32, n: vec3<f32>) -> vec4<f32> {
    let denom = dot(d, n);
    if (abs(denom) < 1e-4) {
        return vec4<f32>(0.0);
    }
    let t = dot(hole - o, n) / denom;
    if (t <= 0.0 || t > limit) {
        return vec4<f32>(0.0);
    }
    let q = o + d * t - hole;
    let r = length(q);
    let x = (r - 3.0 * rs) / (13.0 * rs);
    if (x < 0.0 || x > 1.0) {
        return vec4<f32>(0.0);
    }
    var e1 = normalize(cross(n, vec3<f32>(0.0, 0.0, 1.0)));
    let e2 = cross(n, e1);
    let spin = params.ev[9].w;
    // Inner rings turn faster.
    let a = atan2(dot(q, e2), dot(q, e1)) - spin * ev_t() * 2.5 * pow(3.0 * rs / r, 1.5);
    let rr = r / rs;
    // Each octave faded out where a pixel of the half-resolution layer
    // (`hole_layers`) spans a band, seen glancingly or from far: there it
    // would alias. Value noise averages to nothing.
    let footprint = (length(o - ev_ship()) + t) * params.frame.z * COARSE / max(abs(denom), 0.08) / rs;
    let bands = noise(vec3<f32>(cos(a) * rr * 0.5, sin(a) * rr * 0.5, rr * 1.7)) * 0.6 * (1.0 - smoothstep01(0.18, 0.45, footprint * 1.7))
        + noise(vec3<f32>(cos(a) * rr * 1.6, sin(a) * rr * 1.6, rr * 4.0)) * 0.4 * (1.0 - smoothstep01(0.18, 0.45, footprint * 4.0));
    let density = smoothstep01(0.0, 0.06, x) * (1.0 - smoothstep01(0.55, 1.0, x)) * (0.55 + 0.45 * bands);
    let hot = params.ev[9].z;
    let inner = mix(vec3<f32>(1.0, 0.92, 0.8), vec3<f32>(0.8, 0.88, 1.0), hot) * 3.2;
    let outer = mix(vec3<f32>(1.0, 0.42, 0.1), vec3<f32>(1.0, 0.65, 0.35), hot) * 1.1;
    let colour = mix(inner, outer, sqrt(x));
    // The side coming toward the camera brighter: Doppler beaming.
    let velocity = normalize(cross(n, q)) * spin;
    let beaming = pow(1.0 + 0.6 * dot(velocity, -d), 3.0);
    let cover = clamp(density * 1.1, 0.0, 0.96);
    return vec4<f32>(colour * density * beaming * 0.13, cover);
}

// How a ray along unit `d` from the ship passes the hole: bent toward it,
// weakly far off, ever harder near the photon sphere; or captured.
struct Lensed {
    // The ray's way out after it passes, and its closest approach.
    bent: vec3<f32>,
    closest: vec3<f32>,
    // How far it passes from the hole, and how far along to its closest.
    b: f32,
    tca: f32,
    captured: bool,
};

fn lens(d: vec3<f32>) -> Lensed {
    var out: Lensed;
    let ship = ev_ship();
    let o = ev_obj(0);
    let rs = o.w;
    let v = o.xyz - ship;
    out.tca = dot(v, d);
    out.closest = ship + d * max(out.tca, 0.0);
    out.b = length(out.closest - o.xyz);
    let critical = 2.6 * rs;
    let reach = max(out.b, 1e-3);
    var bend = 2.0 * rs / reach * (1.0 + 1.6 * pow(critical / max(reach, critical), 3.0));
    if (out.tca < 0.0) {
        bend = 2.0 * rs / length(v) * 0.5;
    }
    bend = min(bend, 2.9);
    let away = normalize(out.closest - o.xyz + vec3<f32>(1e-5));
    out.bent = normalize(d * cos(bend) - away * sin(bend));
    out.captured = out.tca > 0.0 && out.b < critical;
    return out;
}

fn hole_normal() -> vec3<f32> {
    return normalize(vec3<f32>(params.ev[9].x, 1.0, params.ev[9].y));
}

// The black hole's disc and jets along unit `d`, drawn at half the frame's
// resolution (soft, they lose nothing there): their light, and how much of
// the bent sky shows through them (`fs_coarse` lays the sky under).
fn hole_layers(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let hole = o.xyz;
    let rs = o.w;
    let n = hole_normal();
    let ray = lens(d);
    // The disc on the straight run in, then on the bent run out (the far
    // side of it, seen over the shadow).
    let near = disc_at(ship, d, max(ray.tca, 0.0), hole, rs, n);
    var far = vec4<f32>(0.0);
    if (!ray.captured) {
        far = disc_at(ray.closest, ray.bent, 1e6, hole, rs, n);
    } else {
        far = disc_at(ray.closest, d, length(ray.closest - hole), hole, rs, n);
    }
    var colour = near.rgb + (1.0 - near.a) * far.rgb;
    // The jets, out of its poles.
    for (var k = 0; k < 2; k++) {
        let pole = n * select(-1.0, 1.0, k == 0);
        let jet = ray_to_line(ship, d, hole, pole);
        if (jet.y > 0.0 && jet.z > rs * 2.0) {
            let width = rs * 0.25 + jet.z * 0.025;
            let knots = 0.6 + 0.4 * sin(jet.z / rs * 0.8 - ev_t() * 6.0);
            let base = smoothstep01(2.0, 3.0, jet.z / rs);
            colour += vec3<f32>(0.45, 0.6, 1.0) * exp(-sq(jet.x / width)) * exp(-jet.z / (rs * 30.0)) * knots * 0.9 * base * (1.0 - near.a);
        }
    }
    return vec4<f32>(colour, (1.0 - near.a) * (1.0 - far.a));
}

// The bent sky behind the black hole, sharp, at the frame's resolution:
// the stars, piled into an Einstein ring round the shadow, and the thin
// photon ring at its edge.
fn hole_sky(d: vec3<f32>) -> vec3<f32> {
    let ray = lens(d);
    let rs = ev_obj(0).w;
    let critical = 2.6 * rs;
    var sky = vec3<f32>(0.0);
    if (!ray.captured) {
        sky = ev_sky(ray.bent) * (1.0 + 1.6 * exp(-sq((ray.b - critical * 1.5) / (critical * 0.45))));
    }
    if (ray.tca > 0.0) {
        sky += vec3<f32>(1.0, 0.86, 0.66) * bump((ray.b - critical) / (rs * 0.07)) * 1.2;
    }
    return sky;
}

// ---------------------------------------------------------------------------
// A nebula: glowing gas and dark lanes of dust, lit up from inside by
// lightning. The gas is space-3d's (wwwtyro/space-3d, released into the
// public domain under the Unlicense): noise displaced by noise six times
// over, baked once over every direction (`fs_bake_gas`), each layer made
// sparse with a power. Two layers here: one far off, fixed on the sky, and
// one on a sphere round the ship's path that it flies through, so the gas
// near it streams past.
// ---------------------------------------------------------------------------

// The near layer's sphere: round the middle of the ship's path, wider than
// the path (`nebula` in `voyage.rs` flies 40 a second for 28 s).
const NEBULA_MIDDLE: vec3<f32> = vec3<f32>(0.0, 0.0, 560.0);
const NEBULA_REACH: f32 = 900.0;

// The gas map along unit `d` mirrored on the axes `flip` gives (±1 each):
// each nebula, and each of its layers, its own part of the map.
fn gas_along(d: vec3<f32>, flip: vec3<f32>) -> vec4<f32> {
    return textureSampleLevel(gas_map, map_sampler, d * flip, 0.0);
}

// Eight ways to mirror the gas map, one for each `seed` mod 8.
fn gas_flip(seed: f32) -> vec3<f32> {
    let k = u32(seed) & 7u;
    return vec3<f32>(f32(k & 1u), f32((k >> 1u) & 1u), f32((k >> 2u) & 1u)) * 2.0 - 1.0;
}

// space-3d's layer: `min(1, n * intensity)` to the power `falloff`.
fn gas_layer(n: f32, intensity: f32, falloff: f32) -> f32 {
    return pow(min(n * intensity, 1.0), falloff);
}

fn ev_nebula(d: vec3<f32>, s: vec2<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let palette = params.ev[6].x;
    let density = params.ev[6].y;
    let seed = params.ev[6].z;
    var c1 = vec3<f32>(0.95, 0.16, 0.36);
    var c2 = vec3<f32>(0.35, 0.12, 0.65);
    if (palette > 2.5) {
        c1 = vec3<f32>(0.9, 0.42, 0.14);
        c2 = vec3<f32>(0.25, 0.14, 0.3);
    } else if (palette > 1.5) {
        c1 = vec3<f32>(0.28, 0.48, 1.0);
        c2 = vec3<f32>(0.72, 0.32, 0.95);
    } else if (palette > 0.5) {
        c1 = vec3<f32>(0.1, 0.82, 0.72);
        c2 = vec3<f32>(0.95, 0.68, 0.22);
    }
    let strike = ev_obj(0);
    let flip = gas_flip(seed);
    let far = gas_along(d, flip);
    // Where the ray leaves the near layer's sphere (the ship is inside it).
    let rel = ship - NEBULA_MIDDLE;
    let b = dot(rel, d);
    let out = -b + sqrt(max(b * b - dot(rel, rel) + NEBULA_REACH * NEBULA_REACH, 0.0));
    let near = gas_along((rel + d * out) / NEBULA_REACH, -flip.yzx);
    let glow1 = gas_layer(far.r, 1.35 * density, 5.0);
    let glow2 = gas_layer(far.g, 1.4 * density, 6.0);
    let wisp = gas_layer(near.r, 1.4 * density, 6.0);
    let dust = smoothstep01(0.55, 0.8, near.b) * density;
    // The strike lights the gas round it.
    let toward = normalize(strike.xyz - ship);
    let flash = exp(-length(d - toward) * 5.0) * strike.w;
    let gas = glow1 + glow2 + wisp;
    let through = exp(-(gas * 0.45 + dust * 1.2));
    var light = (c1 * glow1 * 0.9 + c2 * glow2 * 0.75) * exp(-(wisp * 0.6 + dust * 1.2))
        + mix(c1, c2, near.a) * wisp * 0.8
        + vec3<f32>(0.85, 0.85, 1.0) * flash * (gas + 0.1) * 1.4;
    // The strike's bolt, jagged across the frame.
    let at = from_ev(strike.xyz - ship);
    let z = dot(at, params.forward.xyz);
    if (z > 0.0 && strike.w > 0.02) {
        let c = vec2<f32>(dot(at, params.right.xyz), dot(at, params.up.xyz)) / (z * params.frame.y);
        let k = s - c;
        let wobble = noise(vec3<f32>(k.y * 9.0, strike.x * 0.1, floor(ev_t() * 12.0))) * 0.05
            + noise(vec3<f32>(k.y * 31.0, strike.y * 0.1, 3.0)) * 0.015;
        let reach = 1.0 - smoothstep01(0.05, 0.35, abs(k.y));
        let bolt = exp(-abs(k.x - wobble) * params.frame.x * 0.5) * reach;
        light += vec3<f32>(0.85, 0.8, 1.0) * (bolt * 1.6 + exp(-length(k) * 9.0) * 0.4) * strike.w;
    }
    return vec4<f32>(light, 1.0 - through);
}

// ---------------------------------------------------------------------------
// A rogue planet: no star, lit only by the faint sky, the glow of its cracks
// and its aurorae round the poles.
// ---------------------------------------------------------------------------

fn ev_rogue(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let centre = o.xyz;
    let radius = o.w;
    let axis = normalize(params.ev[8].xyz);
    let kind = params.ev[9].x;
    let seed = params.ev[9].w;
    let t = ev_t();
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    let ground = sphere_hits(ship, d, centre, radius);
    var reach = 1e9;
    if (ground.x > 0.0) {
        reach = ground.x;
        let p = ship + d * ground.x;
        let n = (p - centre) / radius;
        let local = turn(n, axis, -params.ev[8].w);
        let lat = dot(n, axis);
        var albedo = vec3<f32>(0.5, 0.58, 0.68);
        var crack_colour = vec3<f32>(0.2, 0.75, 1.0);
        var stretch = vec3<f32>(1.0);
        if (kind > 1.5) {
            albedo = vec3<f32>(0.1, 0.17, 0.27);
            crack_colour = vec3<f32>(0.55, 0.85, 1.0);
            stretch = vec3<f32>(1.0, 3.0, 1.0);
        } else if (kind > 0.5) {
            albedo = vec3<f32>(0.085, 0.075, 0.07);
            crack_colour = vec3<f32>(1.0, 0.36, 0.06) * 1.6;
        }
        let terrain = noise(local * 3.0 + seed) * 0.5 + noise(local * 9.0 + seed) * 0.3 + noise(local * 27.0) * 0.2;
        albedo *= 0.7 + 0.5 * terrain;
        let ridge = 1.0 - abs(noise(local * stretch * 5.0 + seed * 0.3));
        let fine = 1.0 - abs(noise(local * stretch * 14.0 + 11.0));
        let crack = smoothstep01(0.955, 0.992, ridge) + smoothstep01(0.97, 0.995, fine) * 0.5 + smoothstep01(0.8, 0.99, ridge) * 0.08;
        let heat = 0.75 + 0.25 * sin(t * 0.9 + terrain * 12.0);
        // Faint light from the galaxy above, the cracks', the aurorae's.
        let sky_light = 0.025 + 0.05 * max(dot(n, normalize(vec3<f32>(0.3, 0.8, 0.2))), 0.0);
        let aurora_glow = bump((abs(lat) - 0.86) / 0.06) * vec3<f32>(0.1, 0.9, 0.45) * 0.08;
        colour = albedo * sky_light + crack_colour * crack * heat * params.ev[9].z + albedo * aurora_glow * 6.0;
        colour += vec3<f32>(0.15, 0.25, 0.5) * pow(1.0 - max(dot(n, -d), 0.0), 4.0) * 0.25;
        let edge = (radius - length(ship + d * dot(centre - ship, d) - centre)) / (ground.x * params.frame.z);
        alpha = clamp(edge + 0.5, 0.0, 1.0);
        colour *= alpha;
    }
    // The aurorae: curtains in a shell above the poles, seen through.
    let shell = sphere_hits(ship, d, centre, radius * 1.07);
    for (var k = 0; k < 2; k++) {
        let t_k = select(shell.y, shell.x, k == 0);
        if (t_k > 0.0 && t_k < reach) {
            let p = ship + d * t_k;
            let n = (p - centre) / (radius * 1.07);
            let lat = dot(n, axis);
            // The band is nothing more than 0.13 off its latitude (the bump
            // ends at 2, its wobble moves it 0.03 at most): no noise there.
            if (abs(abs(lat) - 0.83) < 0.13) {
                let local = turn(n, axis, -params.ev[8].w * 0.5);
                let band = bump((abs(lat) - 0.83 - 0.03 * noise(local * 4.0 + t * 0.05)) / 0.05);
                let curtain = smoothstep01(-0.2, 0.7, noise(local * vec3<f32>(18.0, 3.0, 18.0) + vec3<f32>(t * 0.15, 0.0, 0.0)));
                let hue = mix(vec3<f32>(0.15, 1.0, 0.45), vec3<f32>(0.9, 0.25, 0.6), params.ev[9].y * smoothstep01(0.86, 0.95, abs(lat)));
                let mu = abs(dot(n, d));
                colour += hue * band * curtain * 0.3 / max(mu, 0.2);
            }
        }
    }
    // Airglow round the limb.
    let v = centre - ship;
    let tca = dot(v, d);
    if (tca > 0.0) {
        let across = length(v - d * tca);
        colour += vec3<f32>(0.2, 0.5, 0.35) * bump((across - radius * 1.012) / (radius * 0.012)) * 0.25 * (1.0 - alpha);
    }
    return vec4<f32>(colour, alpha);
}

// ---------------------------------------------------------------------------
// A derelict: a broken ring station, a band traced as a cylinder, its last
// windows lit, its breaks smouldering, a beacon blinking.
// ---------------------------------------------------------------------------

fn ev_derelict(d: vec3<f32>) -> vec4<f32> {
    // The station itself is a mesh (`fs_model`, family 5): here only its
    // beacon, blinking red on the rim as it turns.
    let ship = ev_ship();
    let o = ev_obj(0);
    let axis = normalize(params.ev[8].xyz);
    var e1 = cross(axis, vec3<f32>(0.0, 1.0, 0.0));
    e1 = normalize(select(e1, vec3<f32>(1.0, 0.0, 0.0), length(e1) < 1e-3));
    let e2 = cross(axis, e1);
    let ba = 0.6 + params.ev[8].w;
    let beacon = o.xyz + (e1 * cos(ba) + e2 * sin(ba)) * o.w + axis * o.w * 0.13;
    let blink = step(0.85, fract(ev_t() * 0.7));
    return vec4<f32>(vec3<f32>(1.0, 0.15, 0.1) * glint(point_pixels(ship, d, beacon), 1.4) * blink * 1.5, 0.0);
}

// ---------------------------------------------------------------------------
// A comet: its nucleus and coma, the straight blue ion tail and the broader,
// curving dust tail.
// ---------------------------------------------------------------------------

fn tail_glow(d: vec3<f32>, start: vec3<f32>, way: vec3<f32>, length_: f32, width0: f32, widen: f32, streaky: f32) -> f32 {
    let ship = ev_ship();
    let line = ray_to_line(ship, d, start, way);
    if (line.y <= 0.0 || line.z <= 0.0 || line.z > length_) {
        return 0.0;
    }
    let width = width0 + line.z * widen;
    let along = line.z / length_;
    let stria = mix(1.0, 0.55 + 0.45 * noise(vec3<f32>(line.z * 0.012 - ev_t() * 0.35, line.x / width * 1.5, 3.0)), streaky);
    // Seen from inside it, not all at once.
    let near = smoothstep01(0.0, 25.0, line.y);
    return exp(-sq(line.x / width)) * (1.0 - along) * sqrt(1.0 - along) * stria * near;
}

fn ev_comet(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let nucleus = ev_obj(0).xyz;
    let tail = normalize(ev_obj(1).xyz);
    let length_ = ev_obj(1).w;
    let bright = params.ev[9].z;
    let px = point_pixels(ship, d, nucleus);
    let v = nucleus - ship;
    let across = length(v - d * max(dot(v, d), 0.0));
    var colour = vec3<f32>(1.0, 0.98, 0.92) * glint(px, 1.6) * 2.0;
    colour += vec3<f32>(0.45, 0.95, 0.8) * (exp(-across / 9.0) * 0.6 + exp(-across / 40.0) * 0.25) * bright;
    let ion = tail_glow(d, nucleus, tail, length_, 3.0, 0.045, 1.0);
    colour += mix(vec3<f32>(0.3, 0.55, 1.0), vec3<f32>(0.45, 0.4, 1.0), params.ev[9].y) * ion * 0.9 * bright;
    let side = normalize(cross(tail, vec3<f32>(0.0, 1.0, 0.0)));
    let dust_way = normalize(tail + side * params.ev[9].x);
    let dust = tail_glow(d, nucleus, dust_way, length_ * 0.7, 8.0, 0.16, 0.3);
    colour += vec3<f32>(1.0, 0.88, 0.66) * dust * 0.55 * bright;
    return vec4<f32>(colour, 0.0);
}

// ---------------------------------------------------------------------------
// Leviathans: a pod of vast translucent creatures, each a spine of soft
// glowing lobes rippling as it swims, fins at the shoulders, its light
// pulsing down its length; plankton glittering round them.
//
// Drawn in pieces, each on quads bounded to where its light falls under
// `FAINT` (`PART`; the draws in `Solar::draw`): each lobe's core (1), the
// fins at each creature's shoulders (2), the filament trailing from its tail
// (3), and every lobe's wide halo (4), small, into a texture of its own
// (`LEVIATHAN_HALO` in `mod.rs`), added over the sky with the plankton by
// `fs_leviathan_glow`. Each creature and lobe is worked out once a quad, in
// the vertex shader; the pixel only measures its ray against it.
// ---------------------------------------------------------------------------

// Which piece a leviathan pipeline draws: `PART` (declared with the gate's,
// below), set by every one.
const LOG2_E: f32 = 1.442695;

struct Leviathan {
    @builtin(position) position: vec4<f32>,
    // The pixel's ray in camera space (right, up, ahead), over its z.
    @location(0) ray: vec2<f32>,
    // In camera space: the lobe's centre and how fast its light falls with
    // the distance from it (squared for the cores, log2 e over the scale), or
    // the filament's start and its length.
    @location(1) @interpolate(flat) centre: vec4<f32>,
    // Its light at the peak; w the fins' fall across them, or the filament's
    // width.
    @location(2) @interpolate(flat) colour: vec4<f32>,
    // The fins' axes, side and the normal to their sheet, each with the
    // shoulder's offset along it; or the filament's way.
    @location(3) @interpolate(flat) side: vec4<f32>,
    @location(4) @interpolate(flat) normal: vec4<f32>,
};

// A vector in the encounter frame in camera space.
fn ev_camera(v: vec3<f32>) -> vec3<f32> {
    let w = from_ev(v);
    return vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
}

// How far out a light of `peak` falling as exp(-x²) stays over `FAINT`, in x.
fn gauss_reach(peak: f32) -> f32 {
    return sqrt(log(max(peak / FAINT, 1.0)));
}

// The frame's box round a sphere at `c` (camera space) of radius `r`: its
// tangents' slopes, across the half frame (low x, low y, high x, high y).
// The whole frame if the sphere reaches behind the camera, none if it is
// wholly behind.
fn sphere_box(c: vec3<f32>, r: f32) -> vec4<f32> {
    let k = c.z * c.z - r * r;
    if (c.z < -r) {
        return vec4<f32>(4.0);
    }
    if (c.z <= r || k <= 1e-3 * c.z * c.z) {
        return vec4<f32>(-1.0, -1.0, 1.0, 1.0);
    }
    let x = (c.x * c.z + vec2<f32>(-1.0, 1.0) * r * sqrt(c.x * c.x + k)) / k;
    let y = (c.y * c.z + vec2<f32>(-1.0, 1.0) * r * sqrt(c.y * c.y + k)) / k;
    return vec4<f32>(x.x, y.x, x.y, y.y) / params.frame.y;
}

// The frame within distance `r` of the segment from `a` to `b` (camera
// space): the box round both ends' spheres' boxes, turned along it. What of
// it is nearer the camera's plane than 2r is left out: unless the segment
// passes within 4r of the camera (then the whole frame), it is at least 60°
// off the view, too far from any ray in the frame to light it.
fn segment_corner(a: vec3<f32>, b: vec3<f32>, r: f32, corner: vec2<f32>) -> vec2<f32> {
    let ab = b - a;
    if (length(a + ab * clamp(-dot(a, ab) / max(dot(ab, ab), 1e-6), 0.0, 1.0)) < r * 4.0) {
        return corner;
    }
    let near = r * 2.0;
    if (max(a.z, b.z) < near) {
        return vec2<f32>(4.0);
    }
    let p = select(a, mix(a, b, (near - a.z) / (b.z - a.z)), a.z < near);
    let q = select(b, mix(b, a, (near - b.z) / (a.z - b.z)), b.z < near);
    let way = q.xy / q.z - p.xy / p.z;
    let along = select(vec2<f32>(1.0, 0.0), normalize(way), length(way) > 1e-6);
    let across = vec2<f32>(-along.y, along.x);
    var lo = vec2<f32>(1e9);
    var hi = vec2<f32>(-1e9);
    for (var e = 0; e < 2; e++) {
        let box = sphere_box(select(q, p, e == 0), r);
        for (var k = 0; k < 4; k++) {
            let c = vec2<f32>(select(box.x, box.z, (k & 1) != 0), select(box.y, box.w, (k & 2) != 0));
            let uv = vec2<f32>(dot(c, along), dot(c, across));
            lo = min(lo, uv);
            hi = max(hi, uv);
        }
    }
    let at = mix(lo, hi, corner * 0.5 + 0.5);
    return along * at.x + across * at.y;
}

// Slices of the frame each creature's fins are bounded in, a quad each
// (`LEVIATHAN_FIN_SLICES` in `mod.rs`).
const FIN_SLICES: u32 = 16u;

// The fins' light falls under `FAINT` where the ray's offset from the
// shoulder `v`, off, has |off·m| over `width` (m the fins' normal). For the
// ray along e = (x, y, 1) tan, off·m = -Q(e) / |e|² with
// Q(e) = (e·m)(e·v) - (v·m)|e|²: the fins lie in the band |Q| < width |e|²
// between two conics. Where it crosses the frame's line at `u` (x = u, or
// y = u if `by_y`): the lowest and highest it reaches along it, within the
// frame; low over high if it misses.
fn fin_cross(v: vec3<f32>, m: vec3<f32>, width: f32, u: f32, by_y: bool) -> vec2<f32> {
    let tan_half = params.frame.y;
    var lo = 1e9;
    var hi = -1e9;
    var ends = vec3<f32>(0.0);
    for (var k = 0; k < 2; k++) {
        let s = select(-width, width, k == 1);
        // Q - s|e|² along the line, a quadratic in w from three points.
        var g = vec3<f32>(0.0);
        for (var i = 0; i < 3; i++) {
            let w = f32(i - 1);
            let e = vec3<f32>(select(vec2<f32>(u, w), vec2<f32>(w, u), by_y) * tan_half, 1.0);
            g[i] = dot(e, m) * dot(e, v) - (dot(v, m) + s) * dot(e, e);
            ends[i] = width * dot(e, e);
        }
        let c = g.y;
        let b = (g.z - g.x) * 0.5;
        let a = (g.z + g.x) * 0.5 - c;
        var roots = vec2<f32>(2.0);
        if (abs(a) > 1e-6 * (abs(b) + abs(c))) {
            let disc = b * b - 4.0 * a * c;
            if (disc >= 0.0) {
                roots = (-b + vec2<f32>(-1.0, 1.0) * sqrt(disc)) / (2.0 * a);
            }
        } else if (abs(b) > 0.0) {
            roots = vec2<f32>(-c / b);
        }
        for (var i = 0; i < 2; i++) {
            if (abs(roots[i]) <= 1.0) {
                lo = min(lo, roots[i]);
                hi = max(hi, roots[i]);
            }
        }
        // The line's ends, if inside the band: Q there is g + s|e|².
        if (k == 1) {
            let q = g.xz + vec2<f32>(s) * ends.xz / width;
            if (abs(q.x) <= ends.x) {
                lo = -1.0;
                hi = max(hi, -1.0);
            }
            if (abs(q.y) <= ends.z) {
                hi = 1.0;
                lo = min(lo, 1.0);
            }
        }
    }
    return vec2<f32>(lo, hi);
}

// The quad bounding a creature's fins in slice `slice` of the frame: across
// the band where it crosses the slice's two sides, and where it leaves the
// frame between them.
fn fin_corner(v: vec3<f32>, m: vec3<f32>, width: f32, slice: u32, corner: vec2<f32>) -> vec2<f32> {
    // Sliced across the way the band runs past the shoulder.
    let t = cross(v, m);
    let run = select(t.xy, t.xy * v.z - v.xy * t.z, v.z > 0.0);
    let by_y = abs(run.y) > abs(run.x);
    let step = 2.0 / f32(FIN_SLICES);
    let u = -1.0 + f32(slice) * step + vec2<f32>(0.0, step);
    let a = fin_cross(v, m, width, u.x, by_y);
    let b = fin_cross(v, m, width, u.y, by_y);
    var lo = min(a.x, b.x);
    var hi = max(a.y, b.y);
    for (var k = 0; k < 2; k++) {
        let edge = select(-1.0, 1.0, k == 1);
        let leaves = fin_cross(v, m, width, edge, !by_y);
        if (leaves.x <= u.y && leaves.y >= u.x) {
            lo = min(lo, edge);
            hi = max(hi, edge);
        }
    }
    if (lo > hi) {
        return vec2<f32>(4.0);
    }
    // A little over, for the band's curve between the sides.
    let margin = step * 0.25 + 2.0 / params.frame.x;
    let w = mix(lo - margin, hi + margin, corner.y * 0.5 + 0.5);
    let along = mix(u.x, u.y, corner.x * 0.5 + 0.5);
    return select(vec2<f32>(along, w), vec2<f32>(w, along), by_y);
}

@vertex
fn vs_leviathan(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Leviathan {
    var out: Leviathan;
    let ship = ev_ship();
    let pod = ev_obj(0);
    let heading = normalize(ev_obj(1).xyz);
    let length_ = ev_obj(1).w;
    let seed = params.ev[9].z;
    let t = ev_t();
    let fade = params.ev[10].y;
    let side = normalize(cross(heading, vec3<f32>(0.0, 1.0, 0.0)));
    let upward = cross(side, heading);
    let ca = hue_of(params.ev[9].x) * 0.6 + vec3<f32>(0.1, 0.35, 0.45);
    let cb = hue_of(params.ev[9].y) * 0.6 + vec3<f32>(0.3, 0.1, 0.4);
    // A quad a lobe for the cores and halos, a slice of the frame a creature
    // for the fins, a creature for the filaments.
    let lobes = PART == 1 || PART == 4;
    let c = select(select(instance, instance / FIN_SLICES, PART == 2), instance / 9u, lobes);
    let fc = f32(c);
    let h = vec3<f32>(hash_cell(vec3<f32>(fc, seed, 1.0)), hash_cell(vec3<f32>(fc, seed, 2.0)), hash_cell(vec3<f32>(fc, seed, 3.0)));
    let size = mix(0.55, 1.15, h.x);
    let body = length_ * size;
    let head = pod.xyz + side * (h.y - 0.5) * 140.0 + upward * (h.z - 0.5) * 70.0 + heading * (h.x - 0.5) * 60.0;
    let phase = t * (1.1 + 0.3 * h.z) + h.y * TAU;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex % 6u];
    var at = vec2<f32>(0.0);
    if (PART == 3) {
        // The trailing filament.
        let tail = ev_camera(head - heading * body - ship);
        let way = ev_camera(-heading);
        let width = body * 0.006 + 0.3;
        let colour = cb * 0.4 * fade;
        out.centre = vec4<f32>(tail, body * 0.8);
        out.colour = vec4<f32>(colour, width);
        out.side = vec4<f32>(way, 0.0);
        let reach = width * gauss_reach(max(colour.r, max(colour.g, colour.b)));
        at = segment_corner(tail, tail + way * body * 0.8, reach, corner);
    } else {
        // A lobe: the shoulder's for the fins.
        let j = select(2u, instance % 9u, lobes);
        let f = f32(j) / 8.0;
        let wave = sin(phase - f * 4.2) * body * 0.07 * (0.25 + f);
        let p = head - heading * f * body + upward * wave + side * sin(phase * 0.5 - f * 2.0) * body * 0.03;
        let thick = body * 0.075 * (1.0 - 0.8 * f) * smoothstep01(-0.1, 0.12, f);
        let tint = mix(ca, cb, f);
        let v = ev_camera(p - ship);
        if (PART == 2) {
            // The fins, wide sheets swept to each side, beating slowly: their
            // light falls across `normal` (upward, swept by the beat) fast and
            // along `side` slowly.
            let beat = sin(phase * 0.8) * 0.5;
            let colour = tint * 0.35 * fade;
            let s = ev_camera(side);
            let m = ev_camera(upward) - s * beat;
            out.centre = vec4<f32>(v, LOG2_E / sq(thick * 4.0));
            out.colour = vec4<f32>(colour, LOG2_E / sq(thick * 0.35));
            out.side = vec4<f32>(s, dot(v, s));
            out.normal = vec4<f32>(m, dot(v, m));
            let reach = gauss_reach(max(colour.r, max(colour.g, colour.b)));
            at = fin_corner(v, m, thick * 0.35 * reach, instance % FIN_SLICES, corner);
        } else {
            var colour = tint * 0.035;
            var reach = thick * 3.0 * log(max(0.035 * max(tint.r, max(tint.g, tint.b)) * fade / FAINT, 1.0));
            out.centre = vec4<f32>(v, LOG2_E / (thick * 3.0));
            if (PART == 1) {
                let pulse_ = 0.5 + 0.5 * sin(t * 2.2 - f * 9.0 + h.z * 6.0);
                colour = tint * (0.3 + 0.7 * pulse_) * 0.55 * fade;
                reach = thick * gauss_reach(max(colour.r, max(colour.g, colour.b)));
                out.centre.w = LOG2_E / sq(thick);
            }
            out.colour = vec4<f32>(colour, 0.0);
            let box = clamp(sphere_box(v, reach), vec4<f32>(-1.0), vec4<f32>(1.0));
            at = mix(box.xy, box.zw, corner * 0.5 + 0.5);
        }
    }
    // At the far end of the depth mask, as the sky.
    out.position = vec4<f32>(at, 1.0, 1.0);
    out.ray = at * params.frame.y;
    return out;
}

@fragment
fn fs_leviathan(in: Leviathan) -> @location(0) vec4<f32> {
    let e = vec3<f32>(in.ray, 1.0);
    let ee = dot(e, e);
    let v = in.centre.xyz;
    var light = 0.0;
    if (PART == 3) {
        let trail = ray_to_line(vec3<f32>(0.0), e * inverseSqrt(ee), v, in.side.xyz);
        let length_ = in.centre.w;
        if (trail.y > 0.0 && trail.z > 0.0 && trail.z < length_) {
            light = exp(-sq(trail.x / in.colour.w)) * (1.0 - trail.z / length_);
        }
    } else {
        // How far ahead the ray passes the lobe, times |e|.
        let ve = dot(v, e);
        if (PART == 2) {
            // The ray's offset from the shoulder along each of the fins' axes.
            let k = ve / ee;
            let along = in.side.w - k * dot(e, in.side.xyz);
            let across = in.normal.w - k * dot(e, in.normal.xyz);
            light = exp2(-(sq(across) * in.colour.w + sq(along) * in.centre.w));
        } else {
            // Its distance squared from the lobe's centre: |v × e|² / |e|².
            let c = cross(v, e);
            let off = dot(c, c) / ee;
            if (PART == 1) {
                light = exp2(-off * in.centre.w);
            } else {
                light = exp2(-sqrt(off) * in.centre.w);
            }
        }
        // Nothing nearer than a unit ahead.
        light = select(light, 0.0, ve * abs(ve) < ee);
    }
    return vec4<f32>(in.colour.rgb * light, 0.0);
}

// For the Pi estimate only: the pixels a piece covers, shaded by nothing.
@fragment
fn fs_probe_leviathan(in: Leviathan) -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}

// Drawn coarser than the frame before its pass, in a group of their own:
// the leviathans' halos, or the plasma storm or the remnant (`fs_coarse`).
@group(2) @binding(0) var leviathan_halo: texture_2d<f32>;
@group(2) @binding(1) var halo_sampler: sampler;

struct LeviathanGlow {
    @builtin(position) position: vec4<f32>,
    // The ray through the pixel in the encounter frame, not normalized.
    @location(0) dir: vec3<f32>,
    // The plankton's colour, with the stopover's fade; and the seed, the
    // glitter's phase and the fade.
    @location(1) @interpolate(flat) plankton: vec3<f32>,
    @location(2) @interpolate(flat) look: vec3<f32>,
};

@vertex
fn vs_leviathan_glow(@builtin(vertex_index) index: u32) -> LeviathanGlow {
    let full = full_vertex(index);
    var out: LeviathanGlow;
    out.position = full.position;
    out.dir = to_ev(full.dir);
    let fade = params.ev[10].y;
    let ca = hue_of(params.ev[9].x) * 0.6 + vec3<f32>(0.1, 0.35, 0.45);
    out.plankton = ca * 0.5 * fade;
    out.look = vec3<f32>(params.ev[9].z, ev_t() * 3.0, fade);
    return out;
}

// Over the sky: the lobes' halos, and the plankton glittering.
@fragment
fn fs_leviathan_glow(in: LeviathanGlow) -> @location(0) vec4<f32> {
    let uv = in.position.xy / (2.0 * params.frame.x);
    var colour = textureSampleLevel(leviathan_halo, halo_sampler, uv, 0.0).rgb * in.look.z;
    let g = hash_cell(floor(normalize(in.dir) * 230.0) + vec3<f32>(in.look.x));
    if (g > 0.996) {
        colour += in.plankton * (0.5 + 0.5 * sin(in.look.y + g * 300.0));
    }
    return vec4<f32>(colour, 0.0);
}

// ---------------------------------------------------------------------------
// A supernova remnant: a shell of glowing filaments, red outside and teal
// within, brighter where it is seen edge on, flown through; the neutron star
// left at its heart.
// ---------------------------------------------------------------------------

fn ev_remnant(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let centre = o.xyz;
    let radius = o.w;
    let scale = params.ev[9].y;
    let seed = params.ev[9].w;
    let t = ev_t();
    var colour = vec3<f32>(0.0);
    let hits = sphere_hits(ship, d, centre, radius);
    for (var k = 0; k < 2; k++) {
        let tk = select(hits.y, hits.x, k == 0);
        if (tk > 0.0) {
            let n = (ship + d * tk - centre) / radius;
            let mu = abs(dot(n, d));
            let limb = min(1.0 / max(mu, 0.08), 9.0);
            let drift = vec3<f32>(seed * 0.1 + t * 0.003);
            let warp_ = noise(n * 2.0 + drift) * 0.35;
            let f1 = 1.0 - abs(noise(n * 5.5 * scale + warp_ + drift));
            let f2 = 1.0 - abs(noise(n * 13.0 * scale + warp_ * 2.0 + 5.0));
            let filaments = pow(f1, 14.0) * 1.1 + pow(f2, 18.0) * 0.7;
            let mixer = smoothstep01(-0.35, 0.45, noise(n * 1.8 + seed));
            let tint = mix(vec3<f32>(1.0, 0.22, 0.32), vec3<f32>(0.18, 0.92, 0.85), mixer * (0.4 + 0.6 * params.ev[9].x));
            colour += tint * filaments * limb * 0.09 / (1.0 + tk / 2500.0);
        }
    }
    // The diffuse glow inside.
    let inside = max(hits.y - max(hits.x, 0.0), 0.0) / radius;
    colour += vec3<f32>(0.25, 0.35, 0.9) * inside * 0.035;
    return vec4<f32>(colour, 0.0);
}

// The neutron star at the remnant's heart: drawn at the frame's own
// resolution (`fs_coarse`).
fn remnant_star(d: vec3<f32>) -> vec3<f32> {
    let px = point_pixels(ev_ship(), d, ev_obj(0).xyz);
    return vec3<f32>(0.75, 0.85, 1.0) * glint(px, 1.4) * 1.5 + vec3<f32>(0.4, 0.55, 1.0) * exp(-px * 0.03) * 0.15;
}

// ---------------------------------------------------------------------------
// A star gate: a ring (or a blocky ring with pylons, or two rings turning
// against each other), its nodes lit one by one as the ship lines up, its
// field opening in the middle, buoys chasing light down the approach. The
// ring is a mesh (`fs_model`, family 2), drawn over all this; the rest is
// drawn bounded, each part over only the pixels it lights, one pipeline a
// part (`PART`, in this order in `draw`):
// - `vs_gate_field` / `fs_gate_field`: the field, a polygon in the ring's
//   plane round its opening, over the sky (premultiplied).
// - `vs_glow` / `fs_gate_glint`: the buoys (1), then the nodes (2), a quad
//   each, added over it.
// ---------------------------------------------------------------------------

struct Gate {
    centre: vec3<f32>,
    radius: f32,
    // Its frame: across the ring, turning slowly round its axis, and the axis.
    f1: vec3<f32>,
    f2: vec3<f32>,
    normal: vec3<f32>,
    style: f32,
    hue: vec3<f32>,
    // How far through the stopover.
    u: f32,
};

fn gate() -> Gate {
    var g: Gate;
    let o = ev_obj(0);
    g.centre = o.xyz;
    g.radius = o.w;
    g.normal = normalize(params.ev[8].xyz);
    let spin = params.ev[8].w;
    g.style = params.ev[9].x;
    g.hue = mix(vec3<f32>(1.0, 0.72, 0.3), select(vec3<f32>(0.75, 0.45, 1.0), vec3<f32>(0.4, 0.85, 1.0), params.ev[9].y < 0.66), step(0.33, params.ev[9].y));
    g.u = params.ev[9].z;
    let e1 = normalize(cross(g.normal, vec3<f32>(0.0, 1.0, 0.0)));
    let e2 = cross(g.normal, e1);
    let c = cos(spin);
    let s = sin(spin);
    g.f1 = e1 * c + e2 * s;
    g.f2 = -e1 * s + e2 * c;
    return g;
}

// The faintest light drawn: half a level of the sRGB frame over black. A
// glow's quad (or a leviathan's piece) reaches as far as its light stays
// above it.
const FAINT: f32 = 0.5 / 255.0 / 12.92;

// A point of the encounter frame in the camera's space (right, up, ahead).
fn ev_camera_point(p: vec3<f32>) -> vec3<f32> {
    let w = from_ev(p - ev_ship());
    return vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
}

// A point in the camera's space on the frame, at a fixed depth (0..1).
fn camera_clip(c: vec3<f32>, depth: f32) -> vec4<f32> {
    return vec4<f32>(c.x / params.frame.y, c.y / params.frame.y, depth * c.z, c.z);
}

// A corner (0..5, two triangles) of the square round the cone of rays
// within `reach` pixels (`point_pixels`) of the point `c` in the camera's
// space: its projection holds every pixel the point's glow reaches, even
// where the point is off the frame.
fn cone_corner(c: vec3<f32>, reach: f32, corner: u32, depth: f32) -> vec4<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let k = corners[corner];
    let u = normalize(c);
    let a = normalize(cross(u, select(vec3<f32>(0.0, 1.0, 0.0), vec3<f32>(1.0, 0.0, 0.0), abs(u.y) > 0.9)));
    let b = cross(u, a);
    let t = min(reach * params.frame.z, 50.0);
    return camera_clip(u + (a * k.x + b * k.y) * t, depth);
}

// The field at `q` on the ring's plane (across it, in the ring's radius),
// seen along unit `d`: rippling light over the stars seen through it, bent
// by `bend` (premultiplied, unclamped: dark in its swirls), and its cover;
// nothing off it. `open`, how far it is open, in `hue.w`.
fn gate_field(q: vec2<f32>, d: vec3<f32>, bend: vec3<f32>, hue: vec4<f32>) -> vec4<f32> {
    let t = ev_t();
    let r = length(q);
    let inside = 1.0 - smoothstep01(0.86, 0.9, r);
    if (inside <= 0.0) {
        return vec4<f32>(0.0);
    }
    // Round the middle at angle `a`, turned on by `r * 3 - t * 0.8`: `q`
    // turned so, without the angle itself.
    let turn = r * 3.0 - t * 0.8;
    let c = cos(turn);
    let s = sin(turn);
    let swirl = noise(vec3<f32>((q.x * c - q.y * s) * 3.0, (q.x * s + q.y * c) * 3.0, t * 0.4));
    let ripple = 0.5 + 0.5 * sin(r * 30.0 - t * 7.0 + swirl * 3.0);
    let behind = ev_sky(d + bend) * 2.0;
    let m = 1.0 - r;
    let field = hue.rgb * (0.35 + 0.9 * ripple * ripple + 0.6 * swirl) * (0.45 + 0.9 * r * r * r) + vec3<f32>(1.0) * m * m * m * m * m * 0.9
        + mix(hue.rgb, vec3<f32>(1.0), 0.5) * bump((r - 0.87) / 0.025) * 1.5;
    let cover = hue.w * inside * 0.85;
    return vec4<f32>((field + behind) * cover, cover);
}

// The sides of the field's polygon, round its circle.
const GATE_SIDES: u32 = 48u;

struct GateField {
    @builtin(position) position: vec4<f32>,
    // Where on the ring's plane, across it in its radius, from the ship,
    // and how the light there is bent: all linear across the plane.
    @location(0) q: vec2<f32>,
    @location(1) ray: vec3<f32>,
    @location(2) bend: vec3<f32>,
    @location(3) @interpolate(flat) hue: vec4<f32>,
};

// The field: a fan of triangles as a list, out to just past where it ends
// (0.9 of the ring's radius), in the ring's plane.
@vertex
fn vs_gate_field(@builtin(vertex_index) vertex: u32) -> GateField {
    var out: GateField;
    let g = gate();
    let open = smoothstep01(0.48, 0.62, g.u);
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    if (open <= 0.0) {
        return out;
    }
    let corner = vertex % 3u;
    var q = vec2<f32>(0.0);
    if (corner > 0u) {
        let a = f32(vertex / 3u + corner - 1u) * TAU / f32(GATE_SIDES);
        q = vec2<f32>(cos(a), sin(a)) * 0.9 / cos(PI / f32(GATE_SIDES));
    }
    let p = g.centre + (g.f1 * q.x + g.f2 * q.y) * g.radius;
    out.position = camera_clip(ev_camera_point(p), 0.9999);
    out.q = q;
    out.ray = p - ev_ship();
    out.bend = (g.f1 * q.x + g.f2 * q.y) * 0.25 * open;
    out.hue = vec4<f32>(g.hue, open);
    return out;
}

@fragment
fn fs_gate_field(in: GateField) -> @location(0) vec4<f32> {
    return gate_field(in.q, normalize(in.ray), in.bend, in.hue) * params.ev[10].y;
}

struct Glow {
    @builtin(position) position: vec4<f32>,
    // The ray through the pixel, in the encounter frame.
    @location(0) ray: vec3<f32>,
    // The point in the encounter frame.
    @location(1) @interpolate(flat) at: vec3<f32>,
    // Its colour, at full strength.
    @location(2) @interpolate(flat) tint: vec3<f32>,
    // The gate's frame (`Gate`), the ship in it in the ring's radius, and
    // the field's colour and how far it is open, for the field under it.
    @location(3) @interpolate(flat) f1: vec3<f32>,
    @location(4) @interpolate(flat) f2: vec3<f32>,
    @location(5) @interpolate(flat) normal: vec3<f32>,
    @location(6) @interpolate(flat) ship: vec3<f32>,
    @location(7) @interpolate(flat) hue: vec4<f32>,
};

override PART: i32 = 0;

@vertex
fn vs_glow(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Glow {
    var out: Glow;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    let g = gate();
    let fade = params.ev[10].y;
    let peak = max(g.hue.r, max(g.hue.g, g.hue.b)) * fade;
    var strength = 0.0;
    var reach = 0.0;
    if (PART == 1) {
        // A buoy: a pair a step down the approach, the light chasing along
        // them. Its glint's halo, then its core, fades out past `FAINT`.
        let fk = f32(instance / 2u);
        let chase = step(fract(ev_t() * 0.9 - fk * 0.083), 0.18);
        let x = select(-1.0, 1.0, instance % 2u == 0u) * g.radius * 1.3;
        out.at = vec3<f32>(g.centre.x + x, g.centre.y - g.radius * 0.35, g.centre.z - 40.0 - fk * 46.0);
        strength = 0.25 + 1.4 * chase;
        let light = strength * peak / FAINT;
        reach = max(7.2 * log(max(0.15 * light, 1.0)), 1.2 * sqrt(2.0 * log(max(2.0 * light, 1.0))));
    } else {
        // A node round the ring, lit in turn.
        let nodes = select(9.0, 3.0, g.style > 0.5 && g.style < 1.5);
        let fk = f32(instance);
        if (fk >= nodes) {
            return out;
        }
        strength = smoothstep01(0.25 + fk / nodes * 0.25, 0.3 + fk / nodes * 0.25, g.u);
        let ang = fk * TAU / nodes;
        out.at = g.centre + (g.f1 * cos(ang) + g.f2 * sin(ang)) * g.radius * select(1.0, 1.18, g.style > 0.5 && g.style < 1.5);
        let light = strength * peak / FAINT;
        reach = max(18.0 * log(max(0.12 * light, 1.0)), 3.0 * log(max(0.8 * light, 1.0)));
    }
    if (reach <= 0.0) {
        return out;
    }
    out.tint = g.hue * strength * fade;
    let corner = cone_corner(ev_camera_point(out.at), reach, vertex % 6u, 0.9999);
    out.position = corner;
    out.ray = to_ev(params.right.xyz * corner.x * params.frame.y + params.up.xyz * corner.y * params.frame.y + params.forward.xyz * corner.w);
    let ro = (ev_ship() - g.centre) / g.radius;
    out.f1 = g.f1;
    out.f2 = g.f2;
    out.normal = g.normal;
    out.ship = vec3<f32>(dot(ro, g.f1), dot(ro, g.f2), dot(ro, g.normal));
    out.hue = vec4<f32>(g.hue, smoothstep01(0.48, 0.62, g.u));
    return out;
}

// Drawn over the field, which dims the buoys behind it and, dark in its
// swirls, swallows a glow's faint light there: what the glow adds to it.
@fragment
fn fs_gate_glint(in: Glow) -> @location(0) vec4<f32> {
    let d = normalize(in.ray);
    let px = point_pixels(ev_ship(), d, in.at);
    // The field where the ray crosses the ring's plane ahead, if open.
    var field = vec4<f32>(0.0);
    let ld = vec3<f32>(dot(d, in.f1), dot(d, in.f2), dot(d, in.normal));
    let tf = -in.ship.z / ld.z;
    if (in.hue.w > 0.0 && abs(ld.z) > 1e-4 && tf > 0.0) {
        let q = in.ship.xy + ld.xy * tf;
        field = gate_field(q, d, (in.f1 * q.x + in.f2 * q.y) * 0.25 * in.hue.w, in.hue);
    }
    var light = in.tint * (exp(-px / 3.0) * 0.8 + exp(-px / 18.0) * 0.12);
    if (PART == 1) {
        light = in.tint * glint(px, 1.2) * (1.0 - field.a);
    }
    let under = field.rgb * params.ev[10].y;
    return vec4<f32>(max(under + light, vec3<f32>(0.0)) - max(under, vec3<f32>(0.0)), 0.0);
}

// ---------------------------------------------------------------------------
// A wormhole: a sphere with another sky in it, swirling, the stars round it
// bent into a bright ring.
// ---------------------------------------------------------------------------

fn ev_wormhole(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let centre = o.xyz;
    let radius = o.w;
    let tint = hue_of(params.ev[9].x) * 0.5 + vec3<f32>(0.5);
    let swirl = params.ev[9].y;
    let strength = params.ev[9].z;
    let t = ev_t();
    let v = centre - ship;
    let tca = dot(v, d);
    if (tca <= 0.0) {
        return vec4<f32>(0.0);
    }
    let closest = ship + d * tca;
    let b = length(closest - centre);
    let away = normalize(closest - centre + vec3<f32>(1e-5));
    let cover = 1.0 - smoothstep01(2.6, 3.6, b / radius);
    if (cover <= 0.0) {
        return vec4<f32>(0.0);
    }
    var colour = vec3<f32>(0.0);
    if (b < radius) {
        // The far mouth's sky: the other side, turned and swirled.
        let x = b / radius;
        let axis = normalize(v);
        var other = turn(reflect(d, away), axis, swirl * (1.0 - x) * 3.0 + t * 0.3 * swirl);
        other = turn(other, normalize(vec3<f32>(0.3, 0.9, 0.1)), 2.1);
        let galaxy = textureSampleLevel(sky_cube, map_sampler, from_ev(other), 0.0).rgb;
        colour = ev_sky(other) * 1.6 * tint + galaxy * tint * 2.5;
        colour += tint * pow(x, 7.0) * 1.2;
    } else {
        // Out of the mouth, the sky bent round it.
        let bend = min(strength * 2.4 * radius / b, 2.6);
        var bent = normalize(d * cos(bend) - away * sin(bend));
        bent = turn(bent, normalize(v), swirl * sq(radius / b) * 0.8);
        colour = ev_sky(bent);
        colour += mix(tint, vec3<f32>(1.0), 0.5) * bump((b / radius - 1.04) / 0.05) * 1.2;
    }
    return vec4<f32>(colour * cover, cover);
}

// ---------------------------------------------------------------------------
// A battle between two fleets, watched from its edge. Each fleet is one of
// six designs (`voyage::FACTIONS`), its ships traced by marching their
// distance fields inside a bounding sphere: saucer cruisers on nacelles,
// vast wedge destroyers with fighters weaving round them, winged raiders,
// crescent warbirds, horseshoe traders and cubes, each at its own scale
// (taking a unit as some 25 m). Each fleet fires its own way: beams,
// pulsed bolts, torpedoes bursting on shields that flare where they are
// struck. A ship is lost: a fireball, its wreckage (`vs_rock`), and a shock
// that ripples out through space, bending the stars behind it, shoving the
// other ships aside as it passes while their shields flare on the side it
// strikes, and rocking the ship.
// ---------------------------------------------------------------------------

fn sd_ellipsoid(p: vec3<f32>, r: vec3<f32>) -> f32 {
    let k0 = length(p / r);
    let k1 = length(p / (r * r));
    return k0 * (k0 - 1.0) / max(k1, 1e-5);
}

fn sd_capsule(p: vec3<f32>, a: vec3<f32>, b: vec3<f32>, r: f32) -> f32 {
    let pa = p - a;
    let ba = b - a;
    let h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
    return length(pa - ba * h) - r;
}

fn sd_box(p: vec3<f32>, b: vec3<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec3<f32>(0.0))) + min(max(q.x, max(q.y, q.z)), 0.0);
}

fn sd_box2(p: vec2<f32>, b: vec2<f32>) -> f32 {
    let q = abs(p) - b;
    return length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0);
}

// Each design's hull colour, weapons' colour and shield's colour.
fn faction_hull(f: i32) -> vec3<f32> {
    var c = array<vec3<f32>, 6>(
        vec3<f32>(0.55, 0.57, 0.6), vec3<f32>(0.5, 0.5, 0.52), vec3<f32>(0.26, 0.29, 0.22),
        vec3<f32>(0.24, 0.36, 0.27), vec3<f32>(0.46, 0.32, 0.18), vec3<f32>(0.11, 0.12, 0.11),
    );
    return c[clamp(f, 0, 5)];
}

fn faction_fire(f: i32) -> vec3<f32> {
    var c = array<vec3<f32>, 6>(
        vec3<f32>(1.0, 0.5, 0.12), vec3<f32>(0.3, 1.0, 0.3), vec3<f32>(0.45, 1.0, 0.25),
        vec3<f32>(0.3, 1.0, 0.55), vec3<f32>(1.0, 0.8, 0.2), vec3<f32>(0.25, 1.0, 0.4),
    );
    return c[clamp(f, 0, 5)];
}

fn faction_shield(f: i32) -> vec3<f32> {
    var c = array<vec3<f32>, 6>(
        vec3<f32>(0.4, 0.7, 1.0), vec3<f32>(0.7, 0.8, 1.0), vec3<f32>(0.7, 1.0, 0.4),
        vec3<f32>(0.3, 1.0, 0.5), vec3<f32>(1.0, 0.7, 0.3), vec3<f32>(0.3, 1.0, 0.4),
    );
    return c[clamp(f, 0, 5)];
}

fn ship_faction(i: i32) -> i32 {
    return i32(select(ev_obj(1).x, ev_obj(1).y, i >= 4));
}

fn battle_lost() -> i32 {
    return i32(ev_obj(2).w);
}

// The blast: where, when, and how far its shock has come at `t`.
fn blast_at() -> vec3<f32> {
    return params.ev[8].xyz;
}

fn shock_radius(t: f32) -> f32 {
    return max(t - ev_obj(1).z, 0.0) * params.ev[8].w;
}

// Ship `i` of the battle now (0-3 one fleet, 4-7 the other), placed once a
// frame by `fleet_rows` in `mod.rs`: where it is and its size; which way it
// heads, and in w how hard the blast's shock is striking it, 0..1. The lost
// one flies on to the blast as it dies; the rest are shoved aside by the
// shock as it passes them.
fn battle_ship(i: i32) -> array<vec4<f32>, 2> {
    return array<vec4<f32>, 2>(params.ev[12 + 2 * i], params.ev[13 + 2 * i]);
}

// Painted markings, a factor on a hull's colour: the warbird's feathers.
fn hull_markings(q: vec3<f32>, kind_: i32) -> vec3<f32> {
    if (kind_ == 3) {
        let qq = q - vec3<f32>(0.0, 0.0, -0.25);
        let ang = atan2(qq.y, qq.z);
        let feathers = smoothstep(0.35, 0.5, abs(fract(abs(qq.x) * 6.0 - ang * 1.2) - 0.5));
        let belly = step(qq.y, -0.2);
        return mix(vec3<f32>(1.0), mix(vec3<f32>(0.55, 0.85, 0.65), vec3<f32>(1.25, 1.2, 0.9), feathers), 0.6 * belly + 0.25);
    }
    return vec3<f32>(1.0);
}

// ---------------------------------------------------------------------------
// The battle's ships as meshes (`ships.rs`): each vertex in its ship's own
// frame, placed where the battle has the ship (`battle_ship`), in the
// camera's frame, with a perspective divide and the depth the rocks use.
// ---------------------------------------------------------------------------

struct ShipIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    // Occlusion, part (`ships::part`, over 255), window flag, unused.
    @location(2) extra: vec4<f32>,
};

struct ShipOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) extra: vec4<f32>,
    @location(3) @interpolate(flat) ship: vec4<f32>,
    @location(4) toward: vec3<f32>,
    @location(5) blast_way: vec3<f32>,
    // Which part a triangle is of: whole, never blended between parts.
    @location(6) @interpolate(flat) part: f32,
};

@vertex
fn vs_ship(v: ShipIn, @builtin(instance_index) instance: u32) -> ShipOut {
    var out: ShipOut;
    let i = i32(instance);
    let s = battle_ship(i);
    let c = s[0].xyz;
    let size = s[0].w;
    let fwd = s[1].xyz;
    let side = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
    let up = cross(side, fwd);
    let rel = (side * v.position.x + up * v.position.y + fwd * v.position.z) * size + c - ev_ship();
    let w = from_ev(rel);
    let cam = vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
    let depth = clamp(cam.z / (cam.z + 400.0), 0.0, 0.999);
    out.clip = vec4<f32>(cam.x / params.frame.y, cam.y / params.frame.y, depth * cam.z, cam.z);
    out.local = v.position;
    out.normal = side * v.normal.x + up * v.normal.y + fwd * v.normal.z;
    out.extra = v.extra;
    out.part = v.extra.y * 255.0;
    out.ship = vec4<f32>(f32(i), f32(ship_faction(i)), size, s[1].w);
    out.toward = normalize(-rel);
    out.blast_way = blast_at() - (rel + ev_ship());
    return out;
}

// Each part's colour, by the ship's design.
fn ship_glow(part: i32, faction: i32) -> vec3<f32> {
    if (part == 3) {
        return select(vec3<f32>(0.35, 0.6, 1.0), vec3<f32>(0.4, 1.0, 0.6), faction == 3) * 1.6;
    } else if (part == 4) {
        return vec3<f32>(1.0, 0.22, 0.06) * 2.2;
    } else if (part == 5) {
        return vec3<f32>(0.35, 0.7, 1.0) * 2.0;
    } else if (part == 6) {
        return mix(faction_shield(faction), vec3<f32>(1.0), 0.35) * 2.6;
    } else if (part == 8) {
        return vec3<f32>(0.3, 1.0, 0.45) * 0.8;
    }
    return vec3<f32>(0.0);
}

@fragment
fn fs_ship(in: ShipOut) -> @location(0) vec4<f32> {
    let faction = i32(in.ship.y);
    let part = i32(in.part + 0.5);
    let ao = in.extra.x;
    let n = normalize(in.normal);
    let v = normalize(in.toward);
    let l = normalize(vec3<f32>(0.5, 0.7, -0.35));
    let q = in.local;
    // Plating: panels a shade each, their seams darker, laid out on the
    // two axes most across the surface.
    let a = abs(n);
    var uv = q.xz;
    if (a.x > a.y && a.x > a.z) {
        uv = q.yz;
    } else if (a.z > a.y) {
        uv = q.xy;
    }
    let pixel = length(fwidth(q)) + 1e-5;
    let g = uv * vec2<f32>(26.0, 14.0);
    let f = abs(fract(g) - 0.5);
    let seam = (1.0 - smoothstep01(0.44, 0.5, max(f.x, f.y))) * 0.25 * (1.0 - smoothstep01(0.01, 0.04, pixel));
    let panel = 0.82 + 0.3 * hash_cell(vec3<f32>(floor(g), f32(part)));
    var albedo = faction_hull(faction) * hull_markings(q, faction) * panel * (1.0 - seam);
    if (part == 1) {
        albedo *= vec3<f32>(0.82, 0.84, 0.88);
    } else if (part == 2) {
        albedo = vec3<f32>(0.05, 0.05, 0.055);
    } else if (faction == 5) {
        albedo = vec3<f32>(0.1, 0.11, 0.1) * panel;
    }
    let diffuse = max(dot(n, l), 0.0);
    let spec = pow(max(dot(reflect(-l, n), v), 0.0), 30.0) * 0.4 * diffuse;
    var colour = albedo * (diffuse * 1.15 + 0.03 + max(dot(n, -l), 0.0) * 0.03) * mix(1.0, ao, 0.7) + vec3<f32>(spec);
    colour += ship_glow(part, faction);
    if (part == 3) {
        // Nacelle grilles: bands of glow.
        colour *= 0.55 + 0.45 * step(0.4, fract(q.z * 40.0));
    }
    if (part == 7) {
        // Windows: small lit panes in rows, some dark.
        let w = q * vec3<f32>(90.0, 60.0, 90.0);
        let pane = step(0.5, fract(w.x + w.z)) * step(0.35, fract(w.y));
        let lit = step(0.35, hash_cell(floor(w * 0.5)));
        let warm = select(vec3<f32>(1.0, 0.85, 0.6), vec3<f32>(0.6, 1.0, 0.75), faction == 3 || faction == 5);
        colour += warm * pane * lit * 0.9 * (1.0 - smoothstep01(0.004, 0.012, pixel));
    }
    // Lit orange by the blast as its fireball burns.
    let kill = ev_obj(1).z;
    let t = ev_t();
    let glare = exp(-max(t - kill, 0.0) / 1.5) * step(kill, t);
    colour += vec3<f32>(1.0, 0.5, 0.2) * max(dot(n, normalize(in.blast_way)), 0.0) * glare * 0.6 * exp(-length(in.blast_way) / 150.0);
    return vec4<f32>(colour, 1.0);
}

// ---------------------------------------------------------------------------
// Every other model (`models.rs`), instanced: each vertex in its model's own
// frame, turned by its instance's quaternion, scaled and placed in the
// encounter frame. One pipeline per family (`MODEL`): 1 rocks, 2 star
// gates, 3 hive cubes, 4 ring stations, 5 debris.
// ---------------------------------------------------------------------------

override MODEL: i32 = 0;

struct ModelIn {
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) extra: vec4<f32>,
    @location(3) place: vec4<f32>,
    @location(4) turn: vec4<f32>,
    @location(5) look: vec4<f32>,
};

struct ModelOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) local: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) ao: f32,
    @location(3) @interpolate(flat) part: f32,
    @location(4) @interpolate(flat) look: vec4<f32>,
    @location(5) toward: vec3<f32>,
    @location(6) @interpolate(flat) size: f32,
    // Rocks: the light's direction in the rock's own frame, where their
    // maps and `toward` are too, so nothing is turned per pixel. Unused by
    // the rest.
    @location(7) @interpolate(flat) turn: vec4<f32>,
    // Rocks: their ground's shade, mottled (`vs_model`).
    @location(8) tone: f32,
};

fn quat_rotate(q: vec4<f32>, v: vec3<f32>) -> vec3<f32> {
    let t = 2.0 * cross(q.xyz, v);
    return v + q.w * t + cross(q.xyz, t);
}

@vertex
fn vs_model(v: ModelIn) -> ModelOut {
    var out: ModelOut;
    let rel = quat_rotate(v.turn, v.position) * v.place.w + v.place.xyz - ev_ship();
    let w = from_ev(rel);
    let cam = vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
    let depth = clamp(cam.z / (cam.z + 400.0), 0.0, 0.999);
    out.clip = vec4<f32>(cam.x / params.frame.y, cam.y / params.frame.y, depth * cam.z, cam.z);
    out.local = v.position;
    out.normal = quat_rotate(v.turn, v.normal);
    out.ao = v.extra.x;
    out.part = v.extra.y * 255.0;
    out.look = v.look;
    out.toward = -rel;
    out.size = v.place.w;
    out.turn = v.turn;
    if (MODEL == 3) {
        // Which face of the cube a block's top lies across (0..5: +x, -x,
        // +y, ...), or 6 for a wall: the same at a quad's every corner.
        let a = abs(v.position);
        let an = abs(v.normal);
        var axis = 0.0;
        if (a.y > a.x && a.y > a.z) {
            axis = 1.0;
        } else if (a.z > a.x && a.z > a.y) {
            axis = 2.0;
        }
        let along = select(select(an.x, an.y, axis == 1.0), an.z, axis == 2.0);
        let negative = select(select(v.position.x, v.position.y, axis == 1.0), v.position.z, axis == 2.0) < 0.0;
        out.turn = vec4<f32>(select(6.0, axis * 2.0 + select(0.0, 1.0, negative), along > 0.5), axis, 0.0, 0.0);
    }
    if (MODEL == 1) {
        let back = vec4<f32>(-v.turn.xyz, v.turn.w);
        out.normal = v.normal;
        out.toward = quat_rotate(back, -rel);
        // Its light in its own frame, and its kind of stone and shape
        // (`model_instances`).
        out.turn = vec4<f32>(v.look.xyz, 0.0);
        let kind = floor(v.look.w / f32(ROCK_SHAPES));
        out.look = vec4<f32>(kind, 0.0, 0.0, v.look.w - kind * f32(ROCK_SHAPES));
        // Its mottling, which changes slowly across it: worked out at its
        // vertices.
        out.tone = 0.8 + 0.27 * noise(v.position * 2.3 + vec3<f32>(v.place.w * 1.7));
    }
    return out;
}

// Texels along each side of a rock shape's maps (`rocks::MAP`), and the
// rock shapes (`models::ROCKS`): its surface map in layer `shape`, its
// horizon map in `shape + ROCK_SHAPES`.
const ROCK_MAP: f32 = 512.0;
const ROCK_SHAPES: u32 = 8u;

// Where unit `d` lies on an octahedral map, 0..1 each way.
fn octahedral(d: vec3<f32>) -> vec2<f32> {
    let p = d / (abs(d.x) + abs(d.y) + abs(d.z));
    var xy = p.xy;
    if (p.z < 0.0) {
        xy = (1.0 - abs(p.yx)) * select(vec2<f32>(-1.0), vec2<f32>(1.0), p.xy >= vec2<f32>(0.0));
    }
    return xy * 0.5 + 0.5;
}

struct CraterBake {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) layer: u32,
};

// The crater map of rock shape `instance` (`bake_craters` draws one
// triangle a layer over the whole of it).
@vertex
fn vs_bake_craters(@builtin(vertex_index) index: u32, @builtin(instance_index) layer: u32) -> CraterBake {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: CraterBake;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    out.layer = layer;
    return out;
}

// The gas map's size (`GAS_MAP` in `mod.rs`).
const GAS_MAP: f32 = 512.0;

fn noise01(p: vec3<f32>) -> f32 {
    return 0.5 * noise(p) + 0.5;
}

// space-3d's nebula (wwwtyro/space-3d, `nebula.glsl`, Unlicense): noise
// displaced by noise six times over, from the finest octave to the
// coarsest, along unit `p` times its scale.
fn space3d_nebula(p: vec3<f32>) -> f32 {
    var scale = 64.0;
    var displace = vec3<f32>(0.0);
    for (var i = 0; i < 6; i++) {
        displace = vec3<f32>(
            noise01(p * scale + displace),
            noise01(p.yzx * scale + displace),
            noise01(p.zxy * scale + displace),
        );
        scale *= 0.5;
    }
    return noise01(p * scale + displace);
}

// The storm's map's size (`STORM_MAP` in `mod.rs`), and how far out it
// reaches, in the storm's radii.
const STORM_MAP: f32 = 512.0;
const STORM_REACH: f32 = 1.1;

// The storm's gas at `q`, a point of the disc turned round the eye by its
// spiral (in the storm's radii), in layer `k` of it: the nebula's gas
// there, each layer from its own part of it, with a finer octave.
fn storm_gas(q: vec2<f32>, k: f32) -> f32 {
    let p = vec3<f32>(q, k * 5.3);
    return space3d_nebula(p * 0.1 + vec3<f32>(3.1, 7.7, 1.3)) * 0.65 + noise01(p * 8.0 + vec3<f32>(19.0)) * 0.35;
}

// The plasma storm's disc from above, out to `STORM_REACH` radii, on the
// face of a cube map toward +z: its gas wound into a spiral round the eye,
// thinning out to its edge and in to the eye, at its bottom (r), middle (g)
// and top (b), each its own; and the shade of its arms, darker, denser
// bands (a). Its eyewall is worked out as it is drawn.
@fragment
fn fs_bake_storm(in: CraterBake) -> @location(0) vec4<f32> {
    if (in.layer != 4u) {
        return vec4<f32>(0.0);
    }
    // The face toward +z, as `cube_dir` has it: x across, y up.
    let uv = in.position.xy / STORM_MAP * 2.0 - 1.0;
    let p = vec2<f32>(uv.x, -uv.y) * STORM_REACH;
    let r = length(p);
    // Wound tighter near the eye, where it turned faster.
    let a = log(r + 0.05) * 1.6 + 0.35 / (r + 0.15);
    let q = vec2<f32>(p.x * cos(a) - p.y * sin(a), p.x * sin(a) + p.y * cos(a));
    let profile = smoothstep01(0.08, 0.2, r) * (1.0 - smoothstep01(0.7, 1.05, r));
    // The noise's gas lies close round a half: spread out to -1..1.
    let ring = 0.25 * bump((r - 0.22) / 0.1);
    let layers = vec3<f32>(storm_gas(q, 0.0), storm_gas(q, 1.0), storm_gas(q, 2.0));
    let gas = smoothstep(vec3<f32>(0.0), vec3<f32>(0.55), (layers - 0.55) * 3.0 + ring) * profile;
    let shade = space3d_nebula(vec3<f32>(q, 0.0) * 0.07 + vec3<f32>(11.4, 2.9, 5.2));
    return vec4<f32>(gas, mix(0.35, 1.0, smoothstep01(0.35, 0.65, shade)));
}

// The gas along each direction, a texel each of a cube face (the layer):
// two layers of it at their own scales (r, g), dust (b), and a finer one
// for tints (a).
@fragment
fn fs_bake_gas(in: CraterBake) -> @location(0) vec4<f32> {
    let dir = cube_dir(in.layer, in.position.xy / GAS_MAP);
    return vec4<f32>(
        space3d_nebula(dir * 0.5 + vec3<f32>(3.1, 7.7, 1.3)),
        space3d_nebula(dir * 0.35 + vec3<f32>(11.4, 2.9, 5.2)),
        space3d_nebula(dir * 0.7 + vec3<f32>(6.6, 13.1, 9.8)),
        noise01(dir * 6.0 + vec3<f32>(19.0)),
    );
}

// Panels and their seams on a model's skin, on the two axes most across it:
// a shade each, the seams darker, faded where finer than a pixel.
fn skin(q: vec3<f32>, n: vec3<f32>, scale: f32, seed: f32) -> f32 {
    let a = abs(n);
    var uv = q.xz;
    if (a.x > a.y && a.x > a.z) {
        uv = q.yz;
    } else if (a.z > a.y) {
        uv = q.xy;
    }
    let pixel = length(fwidth(q)) * scale + 1e-5;
    let g = uv * scale;
    let f = abs(fract(g) - 0.5);
    let seam = (1.0 - smoothstep01(0.44, 0.5, max(f.x, f.y))) * 0.3 * (1.0 - smoothstep01(0.05, 0.3, pixel));
    return (0.82 + 0.3 * hash_cell(vec3<f32>(floor(g), seed))) * (1.0 - seam);
}

@fragment
fn fs_model(in: ModelOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let v = normalize(in.toward);
    let q = in.local;
    let part = i32(in.part + 0.5);
    let look = in.look;
    let t = ev_t();
    var colour = vec3<f32>(0.0);
    if (MODEL == 1) {
        // Rocks (`rocks.rs`): the mesh is only their outline. Their
        // ground, craters, boulders and grain are in their surface map (its
        // normal and how fresh it is), and how high the ground rises round
        // each point in their horizon map (its mean, and which way it leans
        // higher), which casts their shadows and darkens their hollows. Lit
        // by Hapke's model of regolith: dark grains scattering mostly back
        // toward the light, brightening sharply as the light comes from
        // behind the viewer (the opposition surge), multiply scattered by
        // the H functions.
        let kind = look.x;
        let l = in.turn.xyz;
        let d = normalize(q);
        // The mip where a texel is about a pixel: a texel of the finest
        // spans some 1.95 / ROCK_MAP radians round the middle.
        let pixel = max(length(dpdx(q)), length(dpdy(q))) / max(length(q), 0.05);
        let lod = max(log2(pixel * ROCK_MAP * 0.51), 0.0);
        // Clear of the map's outermost half texels at this mip, which
        // would blend with the far side of its fold.
        let edge = 0.5 * exp2(ceil(lod)) / ROCK_MAP;
        let at = clamp(octahedral(d), vec2<f32>(edge), vec2<f32>(1.0 - edge));
        let shape = i32(look.w + 0.5);
        let ground = textureSampleLevel(rock_maps, map_sampler, at, shape, lod);
        let h = textureSampleLevel(rock_maps, map_sampler, at, shape + i32(ROCK_SHAPES), lod) * 2.0 - 1.0;
        let nn = normalize(ground.xyz * 2.0 - 1.0);
        // The horizon toward the light, against how high the light stands
        // over the plane across the radial direction.
        let across = l - d * dot(l, d);
        let horizon = h.x + dot(h.yzw, across) / max(length(across), 1e-4);
        let soft = 0.03 + 0.02 * lod;
        let lit = smoothstep01(-soft, soft, dot(l, d) - horizon);
        let fresh = ground.w;
        let ao = clamp(1.0 - max(h.x, 0.0) * 1.6, 0.2, 1.0);
        // Single-scattering albedo, by kind: carbonaceous, icy, iron-rich.
        var albedo = 0.09;
        var tint = vec3<f32>(1.0, 0.95, 0.9);
        var shine = 0.0;
        if (kind > 1.5) {
            albedo = 0.22;
            tint = vec3<f32>(1.0, 0.72, 0.5);
            shine = 0.35;
        } else if (kind > 0.5) {
            albedo = 0.6;
            tint = vec3<f32>(0.85, 0.93, 1.0);
            shine = 0.5;
        }
        // Fresh ground is less weathered: paler, and bluer than the rest.
        albedo = min(albedo * in.tone * (1.0 + fresh * 0.9), 0.95);
        tint = mix(tint, tint * vec3<f32>(0.9, 0.97, 1.12), fresh);
        let mu0 = max(dot(nn, l), 0.0);
        let mu = max(dot(nn, v), 0.02);
        let cos_g = clamp(dot(l, v), -1.0, 1.0);
        // Henyey-Greenstein, backscattering; the surge's width 0.06.
        let xi = -0.3;
        let phase = (1.0 - xi * xi) / pow(1.0 + 2.0 * xi * cos_g + xi * xi, 1.5);
        let surge = 1.0 / (1.0 + sqrt(max(1.0 - cos_g, 0.0) / max(1.0 + cos_g, 1e-3)) / 0.06);
        let gamma = sqrt(1.0 - albedo);
        let h0 = (1.0 + 2.0 * mu0) / (1.0 + 2.0 * mu0 * gamma);
        let h1 = (1.0 + 2.0 * mu) / (1.0 + 2.0 * mu * gamma);
        let hapke = albedo * 0.25 * mu0 / (mu0 + mu) * ((1.0 + surge) * phase + h0 * h1 - 1.0);
        colour = tint * hapke * PI * 1.8 * lit;
        // What little the sky and the dust give back, into its hollows least.
        colour += tint * albedo * vec3<f32>(0.03, 0.035, 0.045) * ao * (0.4 + 0.6 * max(dot(nn, -l), 0.0));
        colour += vec3<f32>(pow(max(dot(reflect(-l, nn), v), 0.0), 40.0) * shine * mu0 * lit);
    } else if (MODEL == 2) {
        // A star gate: dark plated metal lit from the side; its nodes lit
        // one after another as it powers up, its inner face glowing as the
        // field opens.
        let style = look.x;
        let hue = mix(vec3<f32>(1.0, 0.72, 0.3), select(vec3<f32>(0.75, 0.45, 1.0), vec3<f32>(0.4, 0.85, 1.0), look.y < 0.66), step(0.33, look.y));
        let u = look.z;
        let l = normalize(vec3<f32>(0.8, 0.45, 0.4));
        let albedo = vec3<f32>(0.24, 0.25, 0.27) * skin(q, n, 40.0, 3.0) * select(1.0, 0.7, part == 1);
        let lit = max(dot(n, l), 0.0) * 1.3 + 0.025;
        colour = albedo * lit * mix(1.0, in.ao, 0.7) + vec3<f32>(pow(max(dot(reflect(-l, n), v), 0.0), 30.0) * 0.8 * lit);
        // Its edges catch the field's light from inside.
        colour += hue * pow(1.0 - max(dot(n, v), 0.0), 3.0) * 0.12 * (0.3 + smoothstep01(0.3, 0.6, u));
        let nodes = select(9.0, 3.0, style > 0.5 && style < 1.5);
        let node = (round(atan2(q.y, q.x) / (TAU / nodes)) + nodes) % nodes;
        let on = smoothstep01(0.25 + node / nodes * 0.25, 0.3 + node / nodes * 0.25, u);
        if (part == 9) {
            colour += hue * mix(0.05, 2.4 + 0.6 * sin(t * 6.0 + node), on);
        } else if (part == 7) {
            colour += hue * smoothstep01(0.45, 0.65, u) * (0.25 + 0.15 * sin(atan2(q.y, q.x) * 36.0 - t * 4.0));
        }
    } else if (MODEL == 3) {
        // A hive cube (`hive_mesh`): blocks of dark machinery, the finest a
        // panel each here (`skin`), light in its colour from glowing slits
        // among the finest, conduits along some of the large blocks'
        // seams and open bays behind grilles; flickering. Scars blasted
        // dark with smouldering edges.
        let glow = mix(hue_of(look.x), vec3<f32>(1.0), 0.12) * 1.1;
        let l = normalize(vec3<f32>(0.45, 0.75, -0.45));
        var albedo = vec3<f32>(0.075, 0.08, 0.078) * skin(q, n, 27.0, 5.0);
        if (part == 2) {
            albedo *= 0.4;
        }
        let lit = max(dot(n, l), 0.0) * 0.9 + 0.08 + max(dot(n, -l), 0.0) * 0.05;
        // Its own light's haze over it (`cube_haze`, drawn only round it).
        colour = albedo * lit * vec3<f32>(0.8, 0.9, 1.0) * mix(1.0, in.ao, 0.8) + glow * 0.037;
        // On a block's top, across its face (`vs_model`): the finest blocks
        // 54 to a side, the large ones 6.
        let face = in.turn.x;
        var uv = q.yz;
        if (in.turn.y > 1.5) {
            uv = q.xy;
        } else if (in.turn.y > 0.5) {
            uv = q.xz;
        }
        let big = (uv + 1.0) * 3.0;
        let cell = hash_cell(floor(q * 9.0));
        let flicker = (0.5 + 0.4 * sin(t * 3.0 + cell * 40.0)) * in.ao;
        if (part == 8) {
            colour += glow * flicker;
        } else if (part == 10) {
            // An open bay: light deep inside behind a grille.
            let bars = abs(fract(fract(big) * 18.0) - 0.5);
            colour = mix(glow * flicker, albedo * lit * 0.4, step(min(bars.x, bars.y), 0.08) * 0.85);
        } else if (face < 5.5 && part == 0) {
            // A slit among the finest blocks, and the conduits along some of
            // the large blocks' seams, while each is over a pixel.
            let pixel = length(fwidth(big));
            let slit = step(0.94, hash_cell(vec3<f32>(floor(big * 9.0), face + look.z * 7.0))) * step(pixel, 0.12);
            let g = abs(fract(big + 0.5) - 0.5);
            let seam = step(min(g.x, g.y), 0.02) * step(0.5, hash_cell(vec3<f32>(floor(big), face + 77.0))) * step(pixel, 0.03);
            colour += glow * flicker * max(slit, seam);
        }
        let damage = look.y;
        if (damage > 0.05) {
            let scar = noise(q * 2.2 * 6.0 + vec3<f32>(look.x * 10.0)) * 0.6 + noise(q * 17.0) * 0.4;
            let cut = 1.0 - damage * 0.9;
            let burnt = smoothstep01(cut - 0.08, cut, scar);
            colour *= 1.0 - 0.75 * burnt;
            colour += vec3<f32>(1.0, 0.35, 0.06) * bump((scar - cut) / 0.025) * 0.6 * (0.7 + 0.3 * sin(t * 2.0 + scar * 30.0));
        }
    } else if (MODEL == 4) {
        // A ring station: plated hull aged and scorched, ribs darker, rows
        // of windows on its inner face with a few still lit and flickering,
        // its breaks smouldering.
        let l = normalize(vec3<f32>(0.6, 0.5, -0.6));
        var albedo = mix(vec3<f32>(0.15, 0.155, 0.165), vec3<f32>(0.2, 0.17, 0.14), look.x) * skin(q, n, 60.0, 9.0);
        albedo *= 0.6 + 0.4 * smoothstep01(-0.4, 0.4, noise(q * 6.0));
        if (part == 1) {
            albedo *= 0.6;
        }
        let lit = max(dot(n, l), 0.0) * 1.1 + 0.03;
        colour = albedo * lit * mix(1.0, in.ao, 0.7);
        if (part == 7) {
            let a = atan2(q.y, q.x);
            let w = vec2<f32>(a * 260.0, q.z * 160.0);
            let wid = floor(w);
            let wf = fract(w);
            let wh = hash_cell(vec3<f32>(wid, 3.0));
            let pane = step(0.25, wf.x) * step(wf.x, 0.75) * step(0.3, wf.y) * step(wf.y, 0.7);
            let on = step(wh, look.z * 0.35) * (0.7 + 0.3 * step(0.1, fract(t * (0.3 + wh * 2.0) + wh * 17.0)));
            let warm = mix(vec3<f32>(1.0, 0.72, 0.38), vec3<f32>(0.7, 0.86, 1.0), look.y);
            let pixel = length(fwidth(w));
            // Panes finer than a pixel blur to their average.
            let shown = mix(pane, 0.25, smoothstep01(0.3, 1.0, pixel));
            colour += warm * (shown * on * 0.9 + on * 0.03);
        } else if (part == 4) {
            colour += vec3<f32>(1.0, 0.4, 0.08) * smoothstep01(0.4, 0.8, noise(q * 14.0 + vec3<f32>(t * 0.6))) * 0.9;
        }
    } else {
        // Debris: torn plates of hull, the derelict's grey or a fleet's own
        // colour, edges smouldering, hotter the newer.
        let l = normalize(vec3<f32>(0.55, 0.6, -0.45));
        var albedo = vec3<f32>(0.3, 0.31, 0.33);
        if (look.x >= 0.0) {
            albedo = faction_hull(i32(look.x + 0.5));
        }
        albedo *= skin(q, n, 6.0, in.size);
        let lit = max(dot(n, l), 0.0) * 1.2 + 0.03;
        colour = albedo * lit * mix(1.0, in.ao, 0.6) + vec3<f32>(pow(max(dot(reflect(-l, n), v), 0.0), 26.0) * 0.4);
        if (part == 4) {
            colour += vec3<f32>(1.0, 0.35, 0.08) * (0.25 + 1.2 * look.y) * (0.7 + 0.3 * sin(t * 4.0 + in.size * 9.0));
        }
    }
    return vec4<f32>(colour, 1.0);
}

// A burst at `c` of radius `r`, `age` 0..1 through it: a white core going
// orange, a soft shell.
fn burst(o: vec3<f32>, d: vec3<f32>, c: vec3<f32>, r: f32, age: f32) -> vec3<f32> {
    if (age <= 0.0 || age >= 1.0) {
        return vec3<f32>(0.0);
    }
    let v = c - o;
    let z = dot(v, d);
    if (z <= 0.0) {
        return vec3<f32>(0.0);
    }
    let across = length(v - d * z);
    let size = r * (0.25 + 0.75 * sqrt(age));
    let fade = sq(1.0 - age);
    let churn = 0.75 + 0.25 * noise(vec3<f32>((v - d * z) / size * 2.0) + vec3<f32>(age * 3.0));
    let core = exp(-sq(across / (size * 0.55))) * fade * churn;
    let shell = exp(-sq((across - size) / (size * 0.18))) * fade * 0.35;
    let hot = mix(vec3<f32>(1.0, 0.95, 0.85), vec3<f32>(1.0, 0.42, 0.08), smoothstep01(0.0, 0.5, age));
    return hot * (core * 2.2 + shell) + vec3<f32>(1.0, 0.6, 0.3) * exp(-across / r) * fade * 0.04;
}

fn ev_battle(d: vec3<f32>) -> vec4<f32> {
    // The background alone: the shock bending the stars behind its edge.
    // Everything lit in the battle is its effects (`vs_fx_*`), drawn after
    // the meshes.
    let ship = ev_ship();
    let t = ev_t();
    let blast = blast_at();
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    let wave = shock_radius(t);
    let wave_age = wave / (params.ev[8].w * 9.0);
    if (wave > 0.0 && wave_age < 1.0 && params.ev[9].y < 0.5) {
        let v = blast - ship;
        let tca = dot(v, d);
        if (tca > 0.0) {
            let b = length(v - d * tca);
            let width = 4.0 + wave * 0.04;
            let x = (b - wave) / width;
            let band = bump(x) * sq(1.0 - wave_age);
            if (band > 0.002) {
                // The stars behind its edge pushed outward, as through a
                // lens; a thin bright front.
                let away = normalize(d * tca - v);
                let bent = normalize(d + away * 0.035 * sin(x * 1.6) * band);
                let rim = mix(vec3<f32>(0.75, 0.88, 1.0), vec3<f32>(1.0, 0.65, 0.35), wave_age);
                // Close to the camera its front fills the view: then only the
                // stars' bending shows, no glow.
                let far = smoothstep01(40.0, 160.0, tca);
                let px = (b - wave) / (tca * params.frame.z);
                let front = exp(-sq(px / 2.5)) * 1.2 + exp(-abs(px) / 14.0) * 0.12;
                colour = ev_sky(bent) * band * 0.8 + rim * front * sq(1.0 - wave_age) * far;
                alpha = band * mix(0.35, 0.8, far);
            }
        }
    }
    return vec4<f32>(colour, alpha);
}

// ---------------------------------------------------------------------------
// The battle's effects: engines, shields flaring, beams and bolts, torpedoes
// and their bursts, fighters, the lost ship's blast and its shock, stray
// fire. Worked out on the CPU each frame (`battle_effects` in `mod.rs`), each
// drawn on geometry bounded to where its light falls under half a step of
// the 8-bit frame, after the meshes: tested against them at its own depth
// (no `frag_depth`, so the depth test runs before shading), added. A ship in
// front hides a laser; one behind does not. Light too dim to matter (under
// `FX_BRIGHT`) is drawn apart, behind everything (`FX_FAR`), so a glow's
// faint reach never shows over a mesh. Each kind its own pipelines, the
// instance's four rows read its own way (`Fx` in `mod.rs`), the last how far
// its light reaches: pixels round it, units round it, how far its front
// stands towards the camera, and the part of that reach its bright light
// keeps within (drawn at its own depth):
// - `vs_fx_line`: a beam or bolt from `a` along unit `m` for `span`, a strip
//   widened across the frame to its glow's reach (pixels), cut into pieces
//   along it so the depth follows it.
// - `vs_fx_spot`: a glint, burst or shield round its centre: a quad on the
//   frame over the cone its light reaches (pixels, or units round it), at
//   the depth of its front.
// - `vs_fx_ring`: the blast's shock wave, an annulus in its plane.
// ---------------------------------------------------------------------------

struct FxIn {
    @location(0) a: vec4<f32>,
    @location(1) b: vec4<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) reach: vec4<f32>,
};

struct FxOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) @interpolate(flat) a: vec4<f32>,
    @location(1) @interpolate(flat) b: vec4<f32>,
    @location(2) @interpolate(flat) colour: vec4<f32>,
};

// Nearest a battle effect's geometry comes to the camera, in units.
const FX_NEAR: f32 = 0.5;

// Whether a pipeline draws the effects' dim light, at the back of the depth
// range, or their bright light at their own depths; and the light between.
override FX_FAR: bool = false;
const FX_BRIGHT: f32 = 0.015;

// The part of an effect's reach this pipeline draws over.
fn fx_part(reach: vec4<f32>) -> f32 {
    return select(reach.w, 1.0, FX_FAR);
}

// An effect's light if this pipeline draws it.
fn fx_keep(light: vec3<f32>) -> vec4<f32> {
    let bright = max(light.x, max(light.y, light.z)) > FX_BRIGHT;
    return vec4<f32>(select(vec3<f32>(0.0), light * params.ev[10].y, bright != FX_FAR), 0.0);
}

// A point in the encounter frame in the camera's frame.
fn fx_cam(p: vec3<f32>) -> vec3<f32> {
    let w = from_ev(p - ev_ship());
    return vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
}

// A point in the camera's frame, `across` added on the frame (in its
// tangents), clipped as the meshes are (`vs_ship`); behind the camera, a z
// the clipper cuts at the near side. Dim light at the back.
fn fx_clip(c: vec3<f32>, across: vec2<f32>) -> vec4<f32> {
    let depth = clamp(c.z / (c.z + 400.0), 0.0, 0.999);
    let xy = c.xy / params.frame.y + across * c.z;
    return vec4<f32>(xy, select(select(c.z, depth * c.z, c.z > 0.0), c.z, FX_FAR), c.z);
}

fn fx_out(clip: vec4<f32>, v: FxIn) -> FxOut {
    var out: FxOut;
    out.clip = clip;
    out.a = v.a;
    out.b = v.b;
    out.colour = v.colour;
    return out;
}

// The pixel's ray in the encounter frame.
fn fx_ray(position: vec4<f32>) -> vec3<f32> {
    return to_ev(ray_at(position.xy));
}

// The two triangles of a quad: (along, across) of each corner, 0..1.
fn fx_corner(k: u32) -> vec2<f32> {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    return corners[k];
}

// Pieces a beam is cut into along it.
const FX_PIECES: u32 = 8u;

@vertex
fn vs_fx_line(@builtin(vertex_index) index: u32, v: FxIn) -> FxOut {
    var ca = fx_cam(v.a.xyz);
    var cb = fx_cam(v.a.xyz + v.b.xyz * v.b.w);
    if (max(ca.z, cb.z) < FX_NEAR) {
        return fx_out(vec4<f32>(0.0, 0.0, -1.0, 1.0), v);
    }
    // Only the part in front of the camera.
    if (ca.z < FX_NEAR) {
        ca = mix(ca, cb, (FX_NEAR - ca.z) / (cb.z - ca.z));
    } else if (cb.z < FX_NEAR) {
        cb = mix(cb, ca, (FX_NEAR - cb.z) / (ca.z - cb.z));
    }
    let k = fx_corner(index % 6u);
    let s = (f32(index / 6u) + k.x) / f32(FX_PIECES);
    let p = mix(ca, cb, s);
    // Its way across the frame there, and the strip's sides at its reach,
    // its ends pushed out as far.
    let run = (cb.xy - ca.xy) * p.z - p.xy * (cb.z - ca.z);
    let along = select(vec2<f32>(1.0, 0.0), normalize(run), dot(run, run) > 1e-12);
    let side = vec2<f32>(-along.y, along.x);
    let end = select(0.0, select(-1.0, 1.0, s > 0.5), s < 0.001 || s > 0.999);
    let px = (side * (k.y * 2.0 - 1.0) + along * end) * v.reach.x * fx_part(v.reach);
    return fx_out(fx_clip(p, px / params.frame.x), v);
}

// A beam or bolt: rows `a` (from, its width in pixels), `b` (unit way,
// span), colour (w: dashes' phase, or -1 for a steady beam).
@fragment
fn fs_fx_line(in: FxOut) -> @location(0) vec4<f32> {
    let d = fx_ray(in.clip);
    let near = ray_to_line(ev_ship(), d, in.a.xyz, in.b.xyz);
    if (near.y <= 0.0 || near.z < 0.0 || near.z > in.b.w) {
        return vec4<f32>(0.0);
    }
    let w = in.a.w;
    let px = near.x / (near.y * params.frame.z);
    var light = exp(-px * px / (2.0 * w * w)) * 1.6 + exp(-px / (w * 5.0)) * 0.12;
    if (in.colour.w >= 0.0) {
        // Bolts: dashes flying along the line.
        light *= step(fract(near.z / in.b.w * 5.0 - in.colour.w), 0.3);
    }
    return fx_keep(in.colour.rgb * light);
}

@vertex
fn vs_fx_spot(@builtin(vertex_index) index: u32, v: FxIn) -> FxOut {
    let c = fx_cam(v.a.xyz);
    let dist = length(c);
    // The cone its light reaches: pixels round it, or units round it; all
    // round if the camera is inside.
    let reach = v.reach.xy * fx_part(v.reach);
    let cone = reach.x * params.frame.z + asin(min(reach.y / max(dist, 1e-3), 1.0)) + 1.5 * params.frame.z;
    let u = c / max(dist, 1e-3);
    var lo = vec2<f32>(-1.45);
    var hi = vec2<f32>(1.45);
    if (reach.y < dist * 0.999) {
        // Its sides as seen in each of the frame's two planes through the
        // camera's axis, wider off them.
        let sx = asin(min(sin(cone) / sqrt(max(1.0 - u.y * u.y, 1e-6)), 1.0));
        let sy = asin(min(sin(cone) / sqrt(max(1.0 - u.x * u.x, 1e-6)), 1.0));
        let mid = vec2<f32>(atan2(c.x, c.z), atan2(c.y, c.z));
        lo = max(mid - vec2<f32>(sx, sy), lo);
        hi = min(mid + vec2<f32>(sx, sy), hi);
        if (any(lo >= hi)) {
            return fx_out(vec4<f32>(0.0, 0.0, -1.0, 1.0), v);
        }
    }
    // At the depth of its front, a step towards the camera from its centre.
    let z = max((dist - v.reach.z) * u.z, FX_NEAR);
    let k = fx_corner(index);
    let at = tan(mix(lo, hi, k));
    return fx_out(fx_clip(vec3<f32>(0.0, 0.0, z), at / params.frame.y), v);
}

// A glint: rows `a` (where, its size), colour.
@fragment
fn fs_fx_glint(in: FxOut) -> @location(0) vec4<f32> {
    let px = point_pixels(ev_ship(), fx_ray(in.clip), in.a.xyz);
    return fx_keep(in.colour.rgb * glint(px, in.a.w));
}

// A burst: rows `a` (where, its radius), `b` (w: its age 0..1), colour.
@fragment
fn fs_fx_burst(in: FxOut) -> @location(0) vec4<f32> {
    let light = burst(ev_ship(), fx_ray(in.clip), in.a.xyz, in.a.w, in.b.w);
    return fx_keep(in.colour.rgb * light);
}

// A shield flaring where it is struck: rows `a` (its centre and radius), `b`
// (unit way to what strikes it, how tightly round that it flares), colour.
@fragment
fn fs_fx_shield(in: FxOut) -> @location(0) vec4<f32> {
    let ship = ev_ship();
    let d = fx_ray(in.clip);
    let sh = sphere_hits(ship, d, in.a.xyz, in.a.w);
    if (sh.x <= 0.0) {
        return vec4<f32>(0.0);
    }
    let n = (ship + d * sh.x - in.a.xyz) / in.a.w;
    let facing = max(dot(n, in.b.xyz), 0.0);
    return fx_keep(in.colour.rgb * pow(facing, in.b.w));
}

// A shield struck by the blast's shock: holding most of it back, the struck
// side brightest and crackling, its edge lit. Rows as `fs_fx_shield`, the
// colour scaled by how hard the shock strikes.
@fragment
fn fs_fx_jolt(in: FxOut) -> @location(0) vec4<f32> {
    let ship = ev_ship();
    let d = fx_ray(in.clip);
    let sh = sphere_hits(ship, d, in.a.xyz, in.a.w);
    if (sh.x <= 0.0) {
        return vec4<f32>(0.0);
    }
    let n = (ship + d * sh.x - in.a.xyz) / in.a.w;
    let facing = max(dot(n, in.b.xyz), 0.0);
    let v = in.a.xyz - ship;
    let edge = length(v - d * dot(v, d)) / in.a.w;
    let crackle = 0.6 + 0.4 * noise(n * 9.0 + vec3<f32>(ev_t() * 6.0));
    let flare = pow(facing, 10.0) * 0.7 * crackle + pow(edge, 14.0) * 0.35;
    return fx_keep(in.colour.rgb * flare);
}

// Pieces round the shock wave's annulus.
const FX_RING_PIECES: u32 = 64u;

@vertex
fn vs_fx_ring(@builtin(vertex_index) index: u32, v: FxIn) -> FxOut {
    let k = fx_corner(index % 6u);
    let turn_ = (f32(index / 6u) + k.x) / f32(FX_RING_PIECES) * TAU;
    // Outer edge out past the circle's chords.
    let wave = v.a.w;
    let reach = 2.0 * v.b.x * fx_part(v.reach);
    let r = select(max(wave - reach, 0.0), (wave + reach) / cos(PI / f32(FX_RING_PIECES)), k.y > 0.5);
    let p = v.a.xyz + vec3<f32>(cos(turn_), 0.0, sin(turn_)) * r;
    return fx_out(fx_clip(fx_cam(p), vec2<f32>(0.0)), v);
}

// The shock wave in the blast's plane: rows `a` (the blast, how far the wave
// has come), `b` (x: its width), colour.
@fragment
fn fs_fx_ring(in: FxOut) -> @location(0) vec4<f32> {
    let ship = ev_ship();
    let d = fx_ray(in.clip);
    if (abs(d.y) <= 1e-3) {
        return vec4<f32>(0.0);
    }
    let tp = (in.a.y - ship.y) / d.y;
    if (tp <= 0.0) {
        return vec4<f32>(0.0);
    }
    let q = ship + d * tp - in.a.xyz;
    let ring = bump((length(q) - in.a.w) / in.b.x);
    return fx_keep(in.colour.rgb * ring);
}

// ---------------------------------------------------------------------------
// A plasma storm: a vast storm cell seen from its edge, marched in a few
// widening steps through a disc of gas wound in a spiral round the eye, its
// eyewall bright, lit from inside by lightning. The gas is the nebula's,
// wound and baked once from above (`fs_bake_storm`): the disc turns as a
// whole.
// ---------------------------------------------------------------------------

fn ev_storm(d: vec3<f32>, s: vec2<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let eye = o.xyz;
    let radius = o.w;
    let axis = normalize(params.ev[8].xyz);
    let seed = params.ev[9].y;
    let turn_way = params.ev[9].z;
    let strike = ev_obj(1);
    let t = ev_t();
    var e1 = normalize(cross(axis, vec3<f32>(0.0, 0.0, 1.0)));
    let e2 = cross(axis, e1);
    // Through the storm's slab only: where the ray is within its thickness.
    let thick = radius * 0.22;
    let h0 = dot(ship - eye, axis);
    let dv = dot(d, axis);
    var t0 = 0.0;
    var t1 = 2600.0;
    if (abs(dv) > 1e-4) {
        let a = (-thick - h0) / dv;
        let b = (thick - h0) / dv;
        t0 = max(min(a, b), 0.0);
        t1 = min(max(a, b), 2600.0);
    } else if (abs(h0) > thick) {
        return vec4<f32>(0.0);
    }
    // The ray in the storm's frame, in its radii.
    let rel = ship - eye;
    let start = vec3<f32>(dot(rel, e1), dot(rel, e2), dot(rel, axis)) / radius;
    let step_ = vec3<f32>(dot(d, e1), dot(d, e2), dot(d, axis)) / radius;
    // And within the disc's edge: no gas beyond it.
    let along = dot(step_.xy, step_.xy);
    let toward = dot(start.xy, step_.xy);
    let edge = toward * toward - along * (dot(start.xy, start.xy) - 1.05 * 1.05);
    if (edge <= 0.0) {
        return vec4<f32>(0.0);
    }
    t0 = max(t0, (-toward - sqrt(edge)) / along);
    t1 = min(t1, (-toward + sqrt(edge)) / along);
    if (t1 <= t0) {
        return vec4<f32>(0.0);
    }
    var through = 1.0;
    // The gas's, the eyewall's and the lightning's light, kept apart as
    // amounts and coloured after the march.
    var lit = vec3<f32>(0.0);
    let n = 8;
    // Each pixel's steps offset by its own amount, so the steps blur into
    // grain rather than banding.
    let dither = hash_cell(vec3<f32>(floor(s * params.frame.x), 7.0));
    // Its distance from the strike, as scalars along it.
    let from_strike = ship - strike.xyz;
    let strike_near = dot(from_strike, d);
    let strike_far = dot(from_strike, from_strike);
    // The disc turns as a whole, faster for a smaller storm, its spiral
    // the way it turns; each storm its own part of the map.
    let spin = turn_way * t * 0.08 + seed * 0.61;
    let turn = vec2<f32>(cos(spin), sin(spin)) / STORM_REACH;
    for (var i = 0; i < n; i++) {
        let f = (f32(i) + dither) / f32(n);
        let tt = mix(t0, t1, f * f);
        let span = (t1 - t0) * (2.0 * f) / f32(n);
        let p = start + step_ * tt;
        let h = p.z * radius / thick;
        let at = vec2<f32>(p.x * turn.x - p.y * turn.y, (p.x * turn.y + p.y * turn.x) * turn_way);
        let g = textureSampleLevel(gas_map, map_sampler, vec3<f32>(at, 1.0), 0.0);
        // The gas between the disc's layers, thinning toward its bottom and
        // top.
        let gas = select(mix(g.g, g.r, -h), mix(g.g, g.b, h), h > 0.0) * saturate(1.6 - 1.6 * abs(h));
        let wall = bump((length(p.xy) - 0.22) / 0.06);
        let shade = g.a;
        let flash = exp(-sqrt(max(strike_far + tt * (2.0 * strike_near + tt), 0.0)) / 90.0) * strike.w * 3.0;
        lit += through * span * gas * vec3<f32>(0.0065 * shade, 0.02 * wall, 0.02 * flash);
        through *= exp(-gas * 0.006 * span);
    }
    let tint = mix(vec3<f32>(0.25, 0.4, 1.0), mix(vec3<f32>(1.0, 0.45, 0.15), vec3<f32>(0.35, 1.0, 0.55), step(0.66, params.ev[9].x)), step(0.33, params.ev[9].x));
    let hot = mix(tint, vec3<f32>(1.0), 0.55);
    var light = tint * lit.x + hot * lit.y + vec3<f32>(0.9, 0.9, 1.0) * lit.z;
    // The strike's bolt on the frame.
    let at = from_ev(strike.xyz - ship);
    let z = dot(at, params.forward.xyz);
    if (z > 0.0 && strike.w > 0.02) {
        let c = vec2<f32>(dot(at, params.right.xyz), dot(at, params.up.xyz)) / (z * params.frame.y);
        let k = s - c;
        let wobble = noise(vec3<f32>(k.x * 8.0, strike.x * 0.1, floor(t * 14.0))) * 0.06 + noise(vec3<f32>(k.x * 27.0, 3.0, strike.z * 0.1)) * 0.015;
        let reach = 1.0 - smoothstep01(0.04, 0.3, abs(k.x));
        light += hot * (exp(-abs(k.y - wobble) * params.frame.x * 0.4) * reach * 1.5 + exp(-length(k) * 8.0) * 0.3) * strike.w;
    }
    return vec4<f32>(light, 1.0 - through);
}

// ---------------------------------------------------------------------------
// A Dyson swarm: a dim star, and round it rings of collector panels on
// three shells, each ring turning about its own tilted axis. Panels between
// the camera and the star are dark against it, catching its light on their
// edges; those beyond it shine with its light.
//
// The panels are geometry, an instance each (`vs_dyson_panel`), placed for
// the frame on the CPU from the same hashes a pixel's trace once found them
// by; the near side's face the camera, the far side's away, so culling
// sorts out those on the limb. Drawn in
// this order (`PART`, `draw` in `mod.rs`), each over only its own pixels,
// at depths of their own just short of the background's: the near side's
// at `DYSON_NEAR`, the far side's at `DYSON_FAR`, the star behind both.
// - 1: the near panels darken what is behind them, once, writing the depth
//   so nothing further is shaded there.
// - 2: each near panel's unlit back and its rim glint, added.
// - 3: the far panels darken the sky, once, writing the depth, and lay down
//   what light of the star shows through them.
// - 4: each far panel's lit face, added.
// - `vs_dyson_star` / `fs_dyson_star`: the star's glow over the rest, on
//   the square it reaches across (1), and its disc on its own (2).
// ---------------------------------------------------------------------------

const DYSON_NEAR: f32 = 0.996;
const DYSON_FAR: f32 = 0.997;
// A panel's quads, along and across its ring.
const DYSON_GRID: vec2<u32> = vec2<u32>(2u, 3u);
const DYSON_WARM: vec3<f32> = vec3<f32>(1.0, 0.72, 0.42);

struct Panel {
    @builtin(position) position: vec4<f32>,
    // Across the panel from its middle, in its cell's widths and heights.
    @location(0) cell: vec2<f32>,
    // How brightly its face is lit.
    @location(1) lit: f32,
    // The ray through the pixel, in the encounter frame.
    @location(2) ray: vec3<f32>,
};

// A panel's place, worked out for the frame by `dyson_panels` in `mod.rs`.
// What was hashed (its ring's axis and width, whether it is there, how
// bright its face) is hashed here, as the trace did: the CPU's sums differ
// from the GPU's in the last bits, enough to move a panel.
struct PanelIn {
    // Its span round the ring in cells, the ring's cells, and how far the
    // ring has turned.
    @location(0) span: vec4<f32>,
    // Its id round the ring, its row, its ring's number and the swarm's
    // seed.
    @location(1) cell: vec4<f32>,
};

@vertex
fn vs_dyson_panel(@builtin(vertex_index) vertex: u32, v: PanelIn) -> Panel {
    var out: Panel;
    out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
    if (hash_cell(vec3<f32>(v.cell.x, v.cell.y, v.cell.z + v.cell.w)) < 0.15) {
        return out;
    }
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    // A grid of quads, close enough to the sphere that its edges fall
    // where the traced ones did.
    let quad = vertex / 6u;
    let corner = (corners[vertex % 6u] + vec2<f32>(f32(quad % DYSON_GRID.x), f32(quad / DYSON_GRID.x))) / vec2<f32>(DYSON_GRID);
    let g = mix(v.span.x, v.span.y, corner.x);
    let gy = v.cell.y + mix(0.14, 0.86, corner.y);
    let fk = v.cell.z;
    let seed = v.cell.w;
    let lat = (gy / 3.0 * 2.0 - 1.0) * (0.13 + 0.05 * hash_cell(vec3<f32>(fk, seed, 4.0)));
    let lon = g / v.span.z * TAU - v.span.w;
    let shell = floor(fk * 0.5);
    let axis = normalize(vec3<f32>(hash_cell(vec3<f32>(fk, seed, 1.0)) - 0.5, 1.0, hash_cell(vec3<f32>(fk, seed, 2.0)) - 0.5) + vec3<f32>(0.0, 0.0, 0.6 * (hash_cell(vec3<f32>(fk, seed, 3.0)) - 0.5)));
    let e1 = normalize(cross(axis, vec3<f32>(0.0, 0.0, 1.0)));
    let n = (e1 * cos(lon) + cross(axis, e1) * sin(lon)) * sqrt(1.0 - lat * lat) + axis * lat;
    let o = ev_obj(0);
    let p = o.xyz + n * o.w * (0.7 + 0.2 * shell);
    let depth = select(DYSON_FAR, DYSON_NEAR, PART <= 2) - select(0.0, 0.0005, PART % 2 == 0);
    out.position = camera_clip(ev_camera_point(p), depth);
    out.ray = p - ev_ship();
    out.cell = vec2<f32>(g - v.cell.x - 0.5, gy - v.cell.y - 0.5);
    let facing = max(dot(-n, normalize(ev_ship() - o.xyz)), 0.0);
    out.lit = (0.35 + 0.65 * facing) * (0.6 + 0.4 * hash_cell(vec3<f32>(v.cell.x, v.cell.z, 9.0))) * 0.8;
    return out;
}

// The star's glow along `d`, and (`disc` > 0) its disc.
fn dyson_star(d: vec3<f32>, disc: f32, glow: f32) -> vec3<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let px = point_pixels(ship, d, o.xyz);
    var light = DYSON_WARM * (exp(-px * 0.02) * 0.5 + exp(-px * 0.004) * 0.12) * glow;
    if (disc > 0.0) {
        light += star_disc(ship, d, o.xyz, o.w * 0.11) * disc;
    }
    return light;
}

@fragment
fn fs_dyson_panel(in: Panel) -> @location(0) vec4<f32> {
    let fade = params.ev[10].y;
    var colour = vec4<f32>(0.0);
    if (PART == 1) {
        colour = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    } else if (PART == 2) {
        let px = point_pixels(ev_ship(), normalize(in.ray), ev_obj(0).xyz);
        let edge = smoothstep01(0.3, 0.38, max(abs(in.cell.x), abs(in.cell.y) * 1.05));
        colour = vec4<f32>(vec3<f32>(0.012, 0.012, 0.014) + DYSON_WARM * edge * 0.25 * exp(-px * 0.004), 0.0);
    } else if (PART == 3) {
        // The star through the panels: its disc dimmed more than its glow.
        colour = vec4<f32>(dyson_star(normalize(in.ray), 0.46, 0.64) * 0.1, 0.9);
    } else {
        colour = vec4<f32>(DYSON_WARM * in.lit * 0.9, 0.0);
    }
    return colour * fade;
}

struct DysonStar {
    @builtin(position) position: vec4<f32>,
    // The ray through the pixel, in the encounter frame.
    @location(0) ray: vec3<f32>,
};

// The star's glow (`PART` 1) on the square it reaches across, and its disc
// (2) on the square its halo does.
@vertex
fn vs_dyson_star(@builtin(vertex_index) vertex: u32) -> DysonStar {
    var out: DysonStar;
    let o = ev_obj(0);
    // Where the glow's tail, or the disc's own halo, falls past `FAINT`.
    let light = params.ev[10].y / FAINT;
    var reach = log(max(0.12 * light, 1.0)) / 0.004;
    if (PART == 2) {
        let r = o.w * 0.11;
        let far = length(o.xyz - ev_ship());
        reach = r * (1.0 + log(max(0.6 * light, 1.0)) / 6.0) / max(far - r, 1e-3) / params.frame.z;
    }
    let corner = cone_corner(ev_camera_point(o.xyz), reach, vertex % 6u, 0.998);
    out.position = corner;
    out.ray = to_ev(params.right.xyz * corner.x * params.frame.y + params.up.xyz * corner.y * params.frame.y + params.forward.xyz * corner.w);
    return out;
}

@fragment
fn fs_dyson_star(in: DysonStar) -> @location(0) vec4<f32> {
    let d = normalize(in.ray);
    var light = vec3<f32>(0.0);
    if (PART == 1) {
        light = dyson_star(d, 0.0, 1.0);
    } else {
        light = star_disc(ev_ship(), d, ev_obj(0).xyz, ev_obj(0).w * 0.11);
    }
    return vec4<f32>(light * params.ev[10].y, 0.0);
}

// The star's disc: limb-darkened, its surface churning.
fn star_disc(o: vec3<f32>, d: vec3<f32>, c: vec3<f32>, r: f32) -> vec3<f32> {
    let hits = sphere_hits(o, d, c, r);
    let v = c - o;
    let across = length(v - d * dot(v, d)) / r;
    if (hits.x <= 0.0) {
        return vec3<f32>(1.0, 0.7, 0.4) * exp(-max(across - 1.0, 0.0) * 6.0) * 0.6;
    }
    let mu = sqrt(max(1.0 - across * across, 0.0));
    let n = normalize(o + d * hits.x - c);
    let cells = noise(n * 9.0 + vec3<f32>(ev_t() * 0.1)) * 0.5 + 0.5;
    return vec3<f32>(1.0, 0.68, 0.38) * (0.5 + 0.5 * mu) * (0.85 + 0.3 * cells) * 2.2;
}

// The nebula (4) and the plasma storm (14) over the sky, from the gas map:
// a pipeline layout of their own (`gas_pipeline`), so apart from `fs_event`.
//
// The storm is drawn at half the frame's resolution, before the frame's
// pass (`COARSE`), and laid over the sky from there (`fs_coarse`).
@fragment
fn fs_gas(in: Full) -> @location(0) vec4<f32> {
    let d = to_ev(normalize(in.dir));
    var position = in.position;
    if (EVENT == 14) {
        position = vec4<f32>(in.position.xy * COARSE, in.position.zw);
    }
    let s = half_frame(position);
    var c = vec4<f32>(0.0);
    if (EVENT == 4) {
        c = ev_nebula(d, s);
    } else {
        c = ev_storm(d, s);
    }
    return c * params.ev[10].y;
}

// How many times coarser than the frame the plasma storm and the remnant
// are drawn (`COARSE` in `mod.rs`).
const COARSE: f32 = 2.0;

// The plasma storm (14), the remnant (10) or the black hole's disc (3) over
// the sky, from where it was drawn at half the frame's resolution
// (`fs_gas`, `fs_event`); the remnant's star, a pixel or two across, sharp
// over it; the black hole's bent sky, sharp under it.
@fragment
fn fs_coarse(in: Full) -> @location(0) vec4<f32> {
    let uv = in.position.xy / (params.frame.x * 2.0);
    var c = textureSampleLevel(leviathan_halo, halo_sampler, uv, 0.0);
    if (EVENT == 3) {
        // The black hole, in place of the sky: its bent sky sharp under its
        // disc and jets.
        let fade = params.ev[10].y;
        return vec4<f32>((c.rgb + c.a * hole_sky(to_ev(normalize(in.dir)))) * fade, fade);
    }
    if (EVENT == 10) {
        c += vec4<f32>(remnant_star(to_ev(normalize(in.dir))), 0.0) * params.ev[10].y;
    }
    return c;
}

// Each event's draw over the sky.
@fragment
fn fs_event(in: Full) -> @location(0) vec4<f32> {
    let d = to_ev(normalize(in.dir));
    let s = half_frame(in.position);
    var c = vec4<f32>(0.0);
    if (EVENT == 1) {
        c = ev_asteroids(d);
    } else if (EVENT == 5) {
        c = ev_rogue(d);
    } else if (EVENT == 7) {
        c = ev_derelict(d);
    } else if (EVENT == 8) {
        c = ev_comet(d);
    } else if (EVENT == 3) {
        // Its disc and jets, and how much shows through them: unfaded.
        return hole_layers(d);
    } else if (EVENT == 10) {
        c = ev_remnant(d);
    } else if (EVENT == 12) {
        c = ev_wormhole(d);
    } else if (EVENT == 13) {
        c = ev_battle(d);
    }
    return c * params.ev[10].y;
}

// ---------------------------------------------------------------------------
// Inside a leg: something seen through the drive's light, added over it.
// ---------------------------------------------------------------------------

@fragment
fn fs_flux(in: Full) -> @location(0) vec4<f32> {
    let s = half_frame(in.position);
    let r = max(length(s), 1e-4);
    let theta = atan2(s.y, s.x);
    let logr = log(r);
    let amount = params.ev[10].w;
    let t = params.ev[11].w;
    var colour = vec3<f32>(0.0);
    if (EVENT == 1) {
        // An ion storm: lightning running out along the field's walls, a
        // violet flicker over everything.
        for (var k = 0; k < 4; k++) {
            let fk = f32(k);
            let beat = floor(t * 2.3 + fk * 0.37);
            let seed = hash_cell(vec3<f32>(beat, fk, 9.0));
            let flash = exp(-fract(t * 2.3 + fk * 0.37) * 4.0) * step(0.2, seed);
            let angle = seed * TAU * 4.0;
            let jag = noise(vec3<f32>(logr * 6.0, beat, fk)) * 0.12 + noise(vec3<f32>(logr * 19.0, beat, fk + 4.0)) * 0.04;
            let off = abs(((theta - angle - jag + PI) % TAU + TAU) % TAU - PI) * r;
            let px = off * params.frame.x;
            let bolt = (exp(-px * 0.4) * 3.0 + exp(-px * 0.05) * 0.35) * smoothstep01(0.06, 0.3, r);
            colour += vec3<f32>(0.75, 0.6, 1.0) * bolt * flash + vec3<f32>(0.35, 0.2, 0.6) * flash * 0.08;
        }
    } else if (EVENT == 2) {
        // A convoy: other ships in jumps of their own alongside, each a
        // bright point and its trail back toward the heading, peeling away.
        for (var k = 0; k < 6; k++) {
            let fk = f32(k);
            let h = hash_cell(vec3<f32>(fk, 3.0, 5.0));
            let angle = h * TAU;
            let place = 0.35 + 0.3 * hash_cell(vec3<f32>(fk, 7.0, 1.0)) + 0.04 * sin(t * 0.5 + fk) + sq(max(t - 12.0 - fk * 0.6, 0.0)) * 0.02;
            let p = vec2<f32>(cos(angle), sin(angle)) * place;
            let along = dot(s, p) / place;
            let across = abs(s.x * p.y - s.y * p.x) / place;
            let trail = exp(-across * params.frame.x * 0.25) * smoothstep01(place * 0.25, place, along) * step(along, place);
            let tint = mix(vec3<f32>(1.0, 0.75, 0.45), vec3<f32>(0.5, 0.9, 1.0), h);
            colour += tint * (trail * 1.1 + glint(length(s - p) * params.frame.x, 2.6) * 3.0);
        }
    } else if (EVENT == 3) {
        // A tachyon echo: the ship's own wake ahead of it, rings closing in
        // on the heading split into colour, a ghost of the drop-out's flare.
        let phase = fract(t * 0.35);
        let front = 1.2 * (1.0 - phase);
        let dd = (r - front) / 0.02;
        colour += vec3<f32>(bump(dd - 1.4), bump(dd), bump(dd + 1.4)) * (1.0 - phase) * 1.3;
        let ghost = vec2<f32>(0.08 * sin(t * 0.7), 0.05 * cos(t * 0.9));
        colour += vec3<f32>(0.6, 0.85, 1.0) * glint(length(s - ghost) * params.frame.x, 4.0) * (1.2 + 0.6 * sin(t * 5.0));
        colour += vec3<f32>(0.5, 0.7, 1.0) * exp(-abs(s.y - ghost.y) * 120.0) * exp(-abs(s.x - ghost.x) * 3.0) * 0.3;
    } else if (EVENT == 5) {
        // A rift: a jagged tear across the field, opening and closing,
        // white light pouring through it, its edges split into colour.
        // Tearing open and closing again every seven seconds, each time at a
        // new angle.
        let cycle = fract(t / 7.0);
        let open = smoothstep01(0.05, 0.3, cycle) * (1.0 - smoothstep01(0.7, 0.95, cycle));
        let angle = 0.6 + hash_cell(vec3<f32>(floor(t / 7.0), 3.0, 1.0)) * 2.0;
        let along = s.x * cos(angle) + s.y * sin(angle);
        let across = -s.x * sin(angle) + s.y * cos(angle);
        let jag = noise(vec3<f32>(along * 5.0, t * 0.3, 1.0)) * 0.08 + noise(vec3<f32>(along * 17.0, t * 0.5, 2.0)) * 0.025;
        let width = (0.004 + 0.03 * open) * (1.0 - smoothstep01(0.4, 1.0, abs(along)));
        let k = abs(across - jag) / max(width, 1e-4);
        let core = exp(-k * k) * 2.5;
        let fringe = vec3<f32>(bump((k - 1.6) / 0.5), bump((k - 2.0) / 0.5), bump((k - 2.4) / 0.5));
        let glow = exp(-abs(across - jag) * 9.0) * 0.25 * open * (1.0 - smoothstep01(0.3, 1.0, abs(along)));
        colour += vec3<f32>(1.0, 0.98, 0.95) * core * open + fringe * 0.7 * open + vec3<f32>(0.7, 0.8, 1.0) * glow;
        // Sparks thrown off it.
        colour += vec3<f32>(1.0, 0.9, 0.7) * streaks(theta, logr, 140.0, 9.0, 0.3, 61) * glow * 3.0;
    } else if (EVENT == 6) {
        // Warp whales: creatures of the jump, swimming along its walls
        // beside the ship, each a chain of soft glowing lobes rippling as it
        // swims, its light pulsing down its length.
        for (var k = 0; k < 3; k++) {
            let fk = f32(k);
            let h = hash_cell(vec3<f32>(fk, 41.0, 7.0));
            let lane = h * TAU + sin(t * 0.3 + fk) * 0.3;
            let head = 0.55 + 0.15 * sin(t * 0.25 + fk * 2.0);
            for (var j = 0; j < 7; j++) {
                let f = f32(j) / 6.0;
                let rr = head * (1.0 - f * 0.45);
                let th = lane + sin(t * 1.4 - f * 4.0 + fk) * 0.06 * (0.3 + f);
                let p = vec2<f32>(cos(th), sin(th)) * rr;
                let size = (0.12 - 0.075 * f) * head;
                let dd = length(s - p) / size;
                let tint = mix(vec3<f32>(0.3, 0.95, 1.0), vec3<f32>(0.8, 0.4, 1.0), f);
                let pulse_ = 0.5 + 0.5 * sin(t * 2.5 - f * 8.0 + fk);
                colour += tint * (exp(-dd * dd) * (0.45 + 0.75 * pulse_) + exp(-dd * 0.5) * 0.05);
                if (j == 1) {
                    // Fins swept back to either side.
                    let side = vec2<f32>(-sin(th), cos(th));
                    let w = s - p;
                    let fin = exp(-sq(dot(w, side) / (size * 3.2))) * exp(-sq((dot(w, vec2<f32>(cos(th), sin(th))) + abs(dot(w, side)) * 0.5) / (size * 0.4)));
                    colour += tint * fin * 0.35 * (0.6 + 0.4 * sin(t * 1.6 + fk));
                }
            }
        }
    } else if (EVENT == 7) {
        // A gravity wave: rings of strained space rolling outward down the
        // field, each split into colour, the field brightening as each
        // passes.
        for (var k = 0; k < 3; k++) {
            let phase = fract(t * 0.22 + f32(k) / 3.0);
            let front = 0.08 + phase * 1.4;
            let wobble = 0.015 * sin(theta * 4.0 + t * 2.0 + f32(k));
            let dd = (r - front - wobble) / (0.02 + 0.04 * phase);
            let fade = sin(phase * PI);
            colour += vec3<f32>(bump(dd + 0.8), bump(dd), bump(dd - 0.8)) * fade * 0.7;
            colour += vec3<f32>(0.5, 0.6, 1.0) * sin(clamp(dd, -3.0, 3.0) * 1.0) * 0.05 * fade;
        }
    } else {
        // A shortcut: the field folding through a fault, a spiral of light
        // turning in on the heading.
        let arms = sin(theta * 3.0 + logr * 6.0 - t * 4.0);
        let spiral = (smoothstep01(0.86, 1.0, arms) + smoothstep01(0.5, 1.0, arms) * 0.15) * smoothstep01(0.05, 0.4, r);
        let hue = mix(vec3<f32>(0.4, 1.0, 0.9), vec3<f32>(1.0, 0.5, 0.9), 0.5 + 0.5 * sin(logr * 2.0 + t));
        colour += hue * spiral * 0.75 + vec3<f32>(0.9, 1.0, 1.0) * exp(-r * 8.0) * 0.6;
    }
    return vec4<f32>(colour * amount, 0.0);
}

// ---------------------------------------------------------------------------
// The ship's own effects, over everything: the alarm pulsing round the rim
// (amber, then red), a hit on the shield rippling out from where it landed
// in hexagons, the tractor beam's hold, a flash.
// ---------------------------------------------------------------------------

@vertex
fn vs_overlay(@builtin(vertex_index) index: u32) -> Full {
    var out = full_vertex(index);
    out.position.z = 0.0;
    return out;
}

// The overlay over the ring round the rim alone (`rim_vertex`): with only
// the alarm showing, nothing inside the ring is lit.
@vertex
fn vs_overlay_rim(@builtin(vertex_index) index: u32) -> Full {
    var out: Full;
    out.position = vec4<f32>(rim_vertex(index), 0.0, 1.0);
    out.dir = vec3<f32>(0.0);
    out.face = 0u;
    return out;
}

// The tractor beam's hold lights nothing inside this much of the radius:
// with neither a hit nor a flash, the overlay over the ring outside it
// alone (`fs_overlay_beam`).
const BEAM_INNER: f32 = 0.4;

@vertex
fn vs_overlay_beam(@builtin(vertex_index) index: u32) -> Full {
    var out: Full;
    out.position = vec4<f32>(ring_vertex(index, BEAM_INNER), 0.0, 1.0);
    out.dir = vec3<f32>(0.0);
    out.face = 0u;
    return out;
}

// The alarm round the round frame's rim.
fn overlay_alarm(r: f32, t: f32) -> vec3<f32> {
    let alert = params.ev[10].x;
    if (alert <= 0.3) {
        return vec3<f32>(0.0);
    }
    let red = smoothstep01(0.6, 0.8, alert);
    let rate = mix(2.2, 5.5, red);
    let beat = 0.55 + 0.45 * sin(t * rate);
    let tint = mix(vec3<f32>(1.0, 0.55, 0.1), vec3<f32>(1.0, 0.1, 0.06), red);
    let band = smoothstep01(0.86, 0.985, r) * (1.0 - smoothstep01(0.99, 1.01, r));
    return tint * band * beat * smoothstep01(0.3, 0.55, alert) * 0.18;
}

// The tractor beam's hold: its colour washing in, scan lines rolling.
fn overlay_beam(y: f32, r: f32, t: f32) -> vec3<f32> {
    let beam = params.ev[3].w;
    if (beam <= 0.01 || EVENT != 0) {
        return vec3<f32>(0.0);
    }
    let tone = hue_of(params.ev[9].x);
    let lines = 0.5 + 0.5 * sin(y * 0.5 - t * 20.0);
    return tone * beam * (0.015 + 0.015 * lines) * smoothstep01(BEAM_INNER, 0.95, r) * (1.0 - smoothstep01(0.99, 1.01, r));
}

// The overlay with a flash over it. Squared: blended in linear light, a
// little white reads as a lot.
// A flash's white: over everything first (`fs_overlay_flash`), the rest of
// the overlay added over it dimmed as much (`overlay_flash`), as one draw
// of both would have it.
fn overlay_white() -> f32 {
    return clamp(sq(params.ev[4].w), 0.0, 1.0);
}

fn overlay_flash(colour: vec3<f32>) -> vec4<f32> {
    return vec4<f32>(colour * (1.0 - overlay_white()), 0.0);
}

@fragment
fn fs_overlay_flash(in: Full) -> @location(0) vec4<f32> {
    return vec4<f32>(overlay_white());
}

// A hit on the shield lights only round where it struck: its ring of
// hexagons, `bump`'s width either side of its front, and its glow, out to
// where it falls under `FAINT`. Two instances of a polygon of `HIT_SIDES`
// round it: a disc for the glow and the ring, or while the glow reaches the
// ring's inside, one disc for both and nothing.
const HIT_SIDES: u32 = 32u;

@vertex
fn vs_overlay_hit(@builtin(vertex_index) index: u32, @builtin(instance_index) part: u32) -> Full {
    let hit = params.ev[2].w;
    let at = params.ev[11].xy;
    let front = (1.0 - hit) * 0.8 + 0.06;
    let glow = log(max(hit * 0.2 / FAINT, 1.0)) / 14.0;
    let flat = cos(PI / f32(HIT_SIDES));
    let ring = vec2<f32>(max(front - 0.1, 0.0), front + 0.1);
    // Apart while the disc's corners stay inside the ring's flats.
    let apart = glow / flat < ring.x * flat;
    var inner = 0.0;
    var outer = select(max(ring.y, glow), glow, apart);
    if (part == 1u) {
        inner = select(0.0, ring.x, apart);
        outer = select(0.0, ring.y, apart);
    }
    let side = index / 6u;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(0.0, 1.0),
        vec2<f32>(0.0, 1.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
    );
    let c = corners[index % 6u];
    let angle = (f32(side) + c.x) * TAU / f32(HIT_SIDES);
    // The inner edge's chords inside its circle, the outer's outside.
    let radius = mix(inner, outer / flat, c.y);
    var out: Full;
    out.position = vec4<f32>(at + vec2<f32>(cos(angle), sin(angle)) * radius, 0.0, 1.0);
    out.dir = vec3<f32>(0.0);
    out.face = 0u;
    return out;
}

@fragment
fn fs_overlay_beam(in: Full) -> @location(0) vec4<f32> {
    let r = length(half_frame(in.position));
    let t = params.burn.y;
    return overlay_flash(overlay_alarm(r, t) + overlay_beam(in.position.y, r, t));
}

@fragment
fn fs_overlay_hit(in: Full) -> @location(0) vec4<f32> {
    let s = half_frame(in.position);
    let r = length(s);
    let hit = params.ev[2].w;
    // A ring of hexagons rippling out from it.
    let at = params.ev[11].xy;
    let dist = length(s - at);
    let front = (1.0 - hit) * 0.8 + 0.06;
    let ring = bump((dist - front) / 0.05) * (1.0 - smoothstep01(0.97, 1.0, r));
    // A true hexagonal lattice: the nearer of two offset grids' cell
    // middles, the hexagon's own distance to it; only its edges lit.
    let g = s * 14.0;
    let pitch = vec2<f32>(1.0, 1.732);
    let a = g - pitch * floor(g / pitch) - pitch * 0.5;
    let b = (g - pitch * 0.5) - pitch * floor((g - pitch * 0.5) / pitch) - pitch * 0.5;
    let local = select(b, a, dot(a, a) < dot(b, b));
    let hex_d = max(abs(local.x) * 0.866 + abs(local.y) * 0.5, abs(local.y));
    let edge = smoothstep01(0.4, 0.48, hex_d);
    var colour = vec3<f32>(0.35, 0.75, 1.0) * ring * (0.1 + 0.9 * edge) * hit * 0.55;
    colour += vec3<f32>(0.6, 0.85, 1.0) * exp(-dist * 14.0) * hit * 0.2;
    return overlay_flash(colour);
}
