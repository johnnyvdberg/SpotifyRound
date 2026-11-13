// Between the legs of a jump: what the ship meets out in the deep (`voyage.rs`).
// Appended to `frame.wgsl`, sharing its parameters, textures and helpers.
//
// Each stopover is drawn in its own frame, the encounter frame: the ship at
// `EV_SHIP`, +z the way the jump goes, its axes in the world in rows 2-4. A
// pixel's ray is turned into that frame (`to_ev`) and everything is traced
// or glowed along it, analytically where it can be. One pipeline per event
// (`EVENT`), so each holds only its own code:
// - `fs_event`, over the sky (premultiplied): the objects and their light.
// - `fs_event_sky`, in place of the sky: the black hole, which lenses it.
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
    var kind = 0.0;
    var glow = 0.0;
    let t = ev_t();
    if (EVENT == 1) {
        kind = params.ev[9].x;
        if (instance < 3u) {
            let o = ev_obj(i32(instance));
            p = o.xyz;
            radius = o.w;
        } else {
            // A box of rocks round the ship's way, wrapping round as it flies
            // on, a corridor kept clear down the middle.
            let span = vec3<f32>(460.0, 280.0, 820.0);
            radius = mix(0.7, 13.0, h.w * h.w * h.w);
            var xy = (h.xy - 0.5) * span.xy;
            let clear = radius + 12.0 + 26.0 * h.z;
            if (length(xy) < clear) {
                xy = normalize(xy + vec2<f32>(1e-3, 0.0)) * clear;
            }
            let z = ship.z - 60.0 + fract((h.z * span.z - ship.z + 60.0) / span.z) * span.z;
            p = vec3<f32>(xy, z);
            // Rocks gather in clumps and thin out between them, as fields
            // do, rather than lying evenly spread.
            // (Hashed coarse cells: the noise volume is the fragment
            // stage's only.)
            let salt = vec3<f32>(params.ev[0].w * 17.0);
            let clump = hash_cell(floor(p / 150.0) + salt) * 0.6 + hash_cell(floor(p / 70.0 + 0.5) + salt) * 0.4;
            if (hash_cell(vec3<f32>(fi, 5.0, 3.0)) > params.ev[9].y * mix(0.35, 1.5, clump)) {
                radius = 0.0;
            }
            if (instance >= 200u) {
                // Grains of dust close round the ship, catching the light as
                // they stream past.
                let near = vec3<f32>(90.0, 60.0, 160.0);
                let g = (h.xyz - vec3<f32>(0.5, 0.5, 0.0)) * near;
                p = vec3<f32>(g.xy, ship.z - 8.0 + fract((g.z - ship.z) / near.z) * near.z);
                radius = mix(0.04, 0.12, h.w);
                kind = 4.0;
                glow = 0.35 + 0.65 * h.w;
            }
        }
    } else if (EVENT == 7) {
        // Debris round the broken ring, turning with it.
        let o = ev_obj(0);
        let axis = params.ev[8].xyz;
        var e1 = cross(axis, vec3<f32>(0.0, 1.0, 0.0));
        e1 = normalize(select(e1, vec3<f32>(1.0, 0.0, 0.0), length(e1) < 1e-3));
        let e2 = cross(axis, e1);
        let a = h.x * TAU + params.ev[8].w * (0.8 + 0.4 * h.z);
        let r = o.w * (0.75 + 0.5 * h.y);
        p = o.xyz + (e1 * cos(a) + e2 * sin(a)) * r + axis * (h.z - 0.5) * 120.0;
        p += vec3<f32>(sin(t * 0.1 + fi), cos(t * 0.13 + fi * 1.7), sin(t * 0.07 + fi * 2.3)) * 3.0;
        radius = mix(0.5, 5.5, h.w * h.w * h.w);
        kind = 3.0;
    } else if (EVENT == 13) {
        // The lost ship's wreckage, flung out from where it blew up.
        let kill = ev_obj(1).z;
        let wreck = battle_ship(battle_lost(), kill);
        let dir = normalize(vec3<f32>(h.x - 0.5, (h.y - 0.5) * 0.6, h.z - 0.5) + vec3<f32>(1e-3));
        let speed = mix(10.0, 34.0, h.z);
        let age = max(t - kill - 0.35, 0.0);
        p = wreck[0].xyz + dir * speed * age + wreck[1].xyz * age * 4.0;
        radius = select(0.0, mix(0.2, 1.6, h.w * h.w * h.w), t > kill + 0.35 && instance < 90u);
        kind = 3.0;
    } else {
        // Ice glittering round the ship as it crosses the comet's tail.
        let span = vec3<f32>(160.0, 110.0, 300.0);
        let xy = (h.xy - 0.5) * span.xy;
        let z = ship.z - 20.0 + fract((h.z * span.z - ship.z + 20.0) / span.z) * span.z;
        p = vec3<f32>(xy, z);
        radius = mix(0.15, 0.6, h.w);
        kind = 4.0;
        glow = 0.3 + 2.5 * params.ev[2].w;
    }
    let w = from_ev(p - ship);
    let cam = vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
    out.centre = vec4<f32>(cam, radius);
    out.look = vec4<f32>(fi + params.ev[0].w * 37.0, kind, glow, 0.0);
    if (radius <= 0.0 || cam.z < -radius * 1.3 || cam.z > 4000.0) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let tan_half = params.frame.y;
    let centre = cam.xy / (max(cam.z, 1e-3) * tan_half);
    // Out to the sphere's silhouette, and a few pixels more for the glow.
    var spread = radius * 1.5 / (sqrt(max(cam.z * cam.z - radius * radius, 1.0)) * tan_half) + 4.0 / params.frame.x;
    var centre_at = centre;
    if (cam.z < radius * 1.45 + 0.5) {
        // Close enough to fill much of the view, or round the camera: over
        // the whole frame, the fragment marching from the camera.
        spread = 1.0;
        centre_at = vec2<f32>(0.0);
    }
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex % 6u];
    out.position = vec4<f32>(centre_at + corner * spread, clamp(max(cam.z - radius, 0.5) / (max(cam.z - radius, 0.5) + 400.0), 0.0, 0.999), 1.0);
    return out;
}

// A crater's profile across `x`, its distance from the middle over its
// radius: a bowl, a raised rim, and the ground beyond.
fn crater_profile(x: f32) -> f32 {
    let bowl = (x * x - 1.0) * 0.35;
    let rim = 0.14 * exp(-sq((x - 1.0) / 0.32));
    return mix(bowl, 0.0, smoothstep01(0.85, 1.05, x)) + rim * (1.0 - smoothstep01(1.3, 1.8, x));
}

// Craters at one scale round point `q` of a rock (its surface direction
// scaled up): the ground's height from them, its slope, and how much of
// it is a fresh rim, from the nearest of the eight cells about it (each
// crater kept within half a cell of its own, so none is missed).
fn craters(q: vec3<f32>, seed: f32) -> array<vec4<f32>, 2> {
    let base = floor(q - 0.5);
    var height = 0.0;
    var slope = vec3<f32>(0.0);
    var fresh = 0.0;
    for (var i = 0; i < 8; i++) {
        let cell = base + vec3<f32>(f32(i & 1), f32((i >> 1) & 1), f32((i >> 2) & 1));
        let k = hash_cell(cell + vec3<f32>(seed, 0.0, 0.0));
        if (k > 0.7) {
            continue;
        }
        let jitter = vec3<f32>(hash_cell(cell + vec3<f32>(seed + 1.0, 3.0, 0.0)), hash_cell(cell + vec3<f32>(seed + 2.0, 0.0, 5.0)), hash_cell(cell + vec3<f32>(0.0, seed + 3.0, 7.0)));
        let centre = cell + 0.5 + (jitter - 0.5) * 0.4;
        let radius = mix(0.16, 0.42, k / 0.7);
        let v = q - centre;
        let dist = length(v);
        let x = dist / radius;
        if (x > 1.8) {
            continue;
        }
        let depth = radius;
        height += crater_profile(x) * depth;
        let dx = (crater_profile(x + 0.03) - crater_profile(x - 0.03)) / 0.06;
        slope += dx * v / max(dist, 1e-4);
        fresh = max(fresh, step(0.45, jitter.x) * bump((x - 1.05) / 0.3));
    }
    return array<vec4<f32>, 2>(vec4<f32>(height, slope), vec4<f32>(fresh, 0.0, 0.0, 0.0));
}

// A rock's shape in its own frame, radius about 1: a stretched ellipsoid,
// its surface pushed in and out by noise whose input is warped by the
// noise's own slope, so the lumps flow into each other like broken stone
// rather than sitting as round blobs.
fn rock_shape(p: vec3<f32>, seed: f32, stretch: vec3<f32>) -> f32 {
    let warp = noised(p * 1.2 + vec3<f32>(seed)).yzw * 0.22;
    let q = p + warp;
    let lumps = noise(q * 1.7 + vec3<f32>(seed * 0.37)) * 0.17 + noise(q * 4.1 + vec3<f32>(seed * 0.71)) * 0.055;
    return sd_ellipsoid(p, stretch) - lumps;
}

@fragment
fn fs_rock(in: Rock) -> @location(0) vec4<f32> {
    let s = half_frame(in.position);
    let d = normalize(vec3<f32>(s * params.frame.y, 1.0));
    let c = in.centre.xyz;
    let r = in.centre.w;
    let kind = in.look.y;
    let seed = in.look.x;
    let tca = dot(c, d);
    let pixel = tca * params.frame.z;
    if (kind > 3.5) {
        // A mote of ice or a grain of dust: a soft point.
        let across = length(c - d * tca);
        let g = exp(-sq(across / max(r, pixel * 1.2)));
        return vec4<f32>(vec3<f32>(0.6, 0.85, 1.0) * g * in.look.z, 0.0);
    }
    // The light, from the encounter frame into the camera's.
    var lw = from_ev(normalize(vec3<f32>(0.55, 0.6, -0.45)));
    if (EVENT == 1) {
        lw = from_ev(ev_light());
    }
    let l = vec3<f32>(dot(lw, params.right.xyz), dot(lw, params.up.xyz), dot(lw, params.forward.xyz));
    if (kind > 2.5) {
        return debris(in, d, l);
    }
    // The rock's own frame: tumbling about its own axis, stretched its own
    // way, the ray in units of its radius.
    let axis = normalize(vec3<f32>(fract(seed * 0.31) - 0.5, 1.0, fract(seed * 0.71) - 0.5));
    let spin = seed * 0.37 + ev_t() * (0.04 + 0.08 * fract(seed * 0.618));
    let stretch = vec3<f32>(0.62 + 0.3 * fract(seed * 0.131), 0.55 + 0.25 * fract(seed * 0.377), 0.85);
    let ro = turn(-c / r, axis, -spin);
    let rd = turn(d, axis, -spin);
    // Its size on the frame, in pixels (close by, as big as it gets).
    let reach = max(tca, r * 0.5);
    let rs = r / reach / params.frame.z;
    // Sphere-traced from where the ray enters its bounds.
    let bound = sphere_hits(ro, rd, vec3<f32>(0.0), 1.45);
    if (bound.y <= 0.0) {
        discard;
    }
    var t = max(bound.x, 0.0);
    var closest = 1e9;
    var near_t = t;
    var hit = false;
    let steps = select(36, 14, rs < 8.0);
    for (var i = 0; i < steps; i++) {
        let dist = rock_shape(ro + rd * t, seed, stretch);
        let ratio = dist / max(t, 1e-3);
        if (ratio < closest) {
            closest = ratio;
            near_t = t;
        }
        if (dist < 0.002 * t + 0.001) {
            hit = true;
            break;
        }
        t += dist * 0.6;
        if (t > bound.y) {
            break;
        }
    }
    // Missed, but close: a soft edge a pixel wide, no wider.
    let angle_px = params.frame.z * reach / r;
    var cover = 1.0;
    if (!hit) {
        cover = clamp(0.5 - closest / angle_px, 0.0, 1.0);
        if (cover <= 0.0) {
            discard;
        }
        // Shaded where the ray passed closest, not where it gave up.
        t = near_t;
    }
    let p = ro + rd * t;
    // The normal from the shape, sharpened by craters and fine grain, each
    // faded out where finer than a pixel.
    let e = 0.012;
    var n = normalize(vec3<f32>(
        rock_shape(p + vec3<f32>(e, 0.0, 0.0), seed, stretch) - rock_shape(p - vec3<f32>(e, 0.0, 0.0), seed, stretch),
        rock_shape(p + vec3<f32>(0.0, e, 0.0), seed, stretch) - rock_shape(p - vec3<f32>(0.0, e, 0.0), seed, stretch),
        rock_shape(p + vec3<f32>(0.0, 0.0, e), seed, stretch) - rock_shape(p - vec3<f32>(0.0, 0.0, e), seed, stretch),
    ));
    let dir = normalize(p);
    var bowl = 0.0;
    var fresh = 0.0;
    if (rs > 10.0) {
        let big = craters(dir * 3.2, seed);
        var dent = big[0];
        fresh = big[1].x;
        if (rs > 40.0) {
            let small = craters(dir * 9.0 + 31.0, seed + 5.0);
            dent += vec4<f32>(small[0].x * 0.35, small[0].yzw);
            fresh = max(fresh, small[1].x * 0.6);
        }
        let fade = smoothstep01(10.0, 30.0, rs);
        let g = dent.yzw - n * dot(dent.yzw, n);
        n = normalize(n - g * 0.9 * fade);
        bowl = dent.x * fade;
    }
    let grain = noised(p * 22.0 + vec3<f32>(seed));
    n = normalize(n - grain.yzw * 0.12 * smoothstep01(25.0, 80.0, rs));
    // Back to the camera's frame.
    let nc = turn(n, axis, spin);
    let lc = l;
    let ll = turn(lc, axis, -spin);
    // Regolith: Lommel-Seeliger blended with Lambert (a lunar-Lambert
    // model), so a lit rock is flat and bright to its terminator rather than
    // shading off like plastic.
    let mu0 = max(dot(nc, lc), 0.0);
    let mu = max(dot(nc, -d), 0.0);
    // (Its rise toward the limb damped, or small rocks ring like bubbles.)
    let ls = 2.0 * mu0 / (mu0 + mu + 0.15);
    let lunar = select(0.65, 0.25, kind > 0.5 && kind < 1.5);
    var brdf = mix(mu0, ls * 0.5, lunar);
    // Soft shadows cast by its own lumps, from the shape along the light.
    var shadow = 1.0;
    if (mu0 > 0.0 && rs > 12.0) {
        var ts = 0.04;
        for (var k = 0; k < 6; k++) {
            let h = rock_shape(p + ll * ts, seed, stretch);
            shadow = min(shadow, 10.0 * h / ts);
            ts += clamp(h, 0.04, 0.3);
        }
        shadow = smoothstep01(0.0, 1.0, shadow);
    }
    // Hollows hold less light: from the craters' bowls and the shape.
    let ao = clamp(1.0 + bowl * 2.5, 0.45, 1.0) * clamp(0.55 + rock_shape(p + n * 0.12, seed, stretch) / 0.12 * 0.45, 0.4, 1.0);
    // The rock's colour: its kind, mottled at two scales, crater rims fresh
    // and paler, fine specks.
    var albedo = vec3<f32>(0.075, 0.068, 0.062);
    var shine = 0.0;
    if (kind > 1.5) {
        albedo = vec3<f32>(0.2, 0.13, 0.085);
        shine = 0.35;
    } else if (kind > 0.5) {
        albedo = vec3<f32>(0.4, 0.45, 0.5);
        shine = 0.6;
    }
    let mottle = noise(p * 2.3 + vec3<f32>(seed * 1.9)) * 0.5 + noise(p * 7.0 + vec3<f32>(seed)) * 0.25;
    albedo *= 0.8 + 0.45 * mottle;
    albedo *= 1.0 + fresh * 0.9;
    let speck = step(0.93, hash_cell(floor(p * 70.0))) * smoothstep01(40.0, 120.0, rs);
    albedo *= 1.0 + speck * 0.8;
    var colour = albedo * brdf * shadow * 2.4;
    // Faint fill from the sky and the field's other rocks, so the night
    // side still reads as a shape.
    colour += albedo * vec3<f32>(0.07, 0.08, 0.1) * (0.35 + 0.25 * max(dot(nc, -lc), 0.0)) * ao;
    // No glint at grazing angles: backlit, it would ring every small rock.
    let spec = pow(max(dot(reflect(-lc, nc), -d), 0.0), 40.0) * shine * shadow * smoothstep01(0.1, 0.45, mu) * mu0;
    colour += vec3<f32>(spec);
    if (kind > 0.5 && kind < 1.5) {
        // Ice: light scattered through its edges.
        colour += vec3<f32>(0.25, 0.45, 0.6) * pow(1.0 - mu, 4.0) * max(dot(d, lc), 0.0) * 0.06 * smoothstep01(20.0, 60.0, rs);
    }
    colour *= mix(1.0, ao, 0.6);
    return vec4<f32>(colour * cover, cover);
}

// A ship's debris: flat plates and torn edges, the older lumpy impostor.
fn debris(in: Rock, d: vec3<f32>, l: vec3<f32>) -> vec4<f32> {
    let c = in.centre.xyz;
    let r = in.centre.w;
    let seed = in.look.x;
    let tca = dot(c, d);
    let pixel = tca * params.frame.z;
    let d2 = dot(c, c) - tca * tca;
    let toward = normalize(d * tca - c + vec3<f32>(1e-4, 0.0, 0.0));
    let lump = noise(toward * 1.7 + vec3<f32>(seed * 3.1, seed * 1.3, 0.0)) * 0.13 + noise(toward * 4.3 + seed) * 0.05;
    let rim = r * (0.86 + lump);
    let across = sqrt(max(d2, 0.0));
    if (across > rim + pixel) {
        discard;
    }
    let th = sqrt(max(rim * rim - d2, 0.0));
    let at = d * (tca - th);
    var n = normalize(at - c);
    n = normalize(floor(n * 2.2 + 0.5) + n * 0.15);
    let g2 = noised(n * 10.0 + vec3<f32>(seed * 0.13));
    let diffuse = max(dot(n, l), 0.0);
    let albedo = mix(vec3<f32>(0.42, 0.44, 0.47), vec3<f32>(0.5, 0.36, 0.25), step(0.6, fract(seed * 0.17))) * (0.75 + 0.25 * g2.x);
    let spec = pow(max(dot(reflect(-l, n), -d), 0.0), 28.0) * 0.6;
    var colour = albedo * (diffuse * 1.25 + 0.025) + vec3<f32>(spec);
    if (fract(seed * 0.0731) > 0.86) {
        colour += vec3<f32>(1.0, 0.35, 0.08) * smoothstep01(0.55, 0.8, g2.x) * (0.7 + 0.3 * sin(ev_t() * 4.0 + seed));
    }
    let a = clamp((rim - across) / pixel + 0.5, 0.0, 1.0);
    return vec4<f32>(colour * a, a);
}

// ---------------------------------------------------------------------------
// A hive cube: a vast cube of machinery, greebled at three scales, lit from
// inside through slits and bays in its colour, scarred, turning slowly, with
// small cubes keeping station and a tractor beam reaching for the ship.
// ---------------------------------------------------------------------------

// Where a ray from `o` along `d` (both in the box's own frame) meets the box
// of half-size `h`: in and out.
fn box_hits(o: vec3<f32>, d: vec3<f32>, h: f32) -> vec2<f32> {
    let inv = 1.0 / select(d, vec3<f32>(1e-6), abs(d) < vec3<f32>(1e-6));
    let t0 = (vec3<f32>(-h) - o) * inv;
    let t1 = (vec3<f32>(h) - o) * inv;
    let lo = min(t0, t1);
    let hi = max(t0, t1);
    return vec2<f32>(max(max(lo.x, lo.y), lo.z), min(min(hi.x, hi.y), hi.z));
}

// A face of the cube at `uv` (-1..1 across it), as albedo and light given
// off: panels, seams and pipes at three scales, slits and open bays glowing
// in its colour, scars burning at their edges. `fp` is a pixel's size in
// `uv`, so detail finer than that fades out.
fn cube_face(uv: vec2<f32>, face: f32, seed: f32, fp: f32, glow: vec3<f32>, damage: f32, p: vec3<f32>) -> array<vec3<f32>, 2> {
    let t = ev_t();
    var albedo = vec3<f32>(0.075, 0.08, 0.078);
    var light = vec3<f32>(0.0);
    var seams = 0.0;
    var k = 3.0;
    for (var level = 0; level < 3; level++) {
        let g = (uv * 0.5 + 0.5) * k;
        let id = floor(g);
        let f = g - id;
        let h = hash_cell(vec3<f32>(id, face * 7.0 + seed + f32(level) * 31.0));
        let detail = 1.0 - smoothstep01(0.08, 0.3, fp * k);
        let seam = (1.0 - smoothstep01(0.0, 0.06 + fp * k * 0.5, min(min(f.x, f.y), min(1.0 - f.x, 1.0 - f.y))));
        seams = max(seams, seam * detail);
        albedo *= mix(1.0, 0.75 + 0.5 * h, detail);
        if (h > 0.72) {
            // Pipes along the panel.
            let along = select(f.x, f.y, h > 0.86);
            albedo *= mix(1.0, 0.7 + 0.5 * abs(sin(along * 18.0)), detail);
        }
        if (level == 1 && h > 0.9) {
            // A glowing slit, flickering.
            let slit = (1.0 - smoothstep01(0.0, 0.08, abs(f.y - 0.5))) * step(0.15, f.x) * step(f.x, 0.85);
            light += glow * slit * (0.55 + 0.45 * sin(t * 3.0 + h * 40.0)) * 0.9 * detail;
        }
        if (level == 0 && h < 0.1) {
            // An open bay: the light inside, behind a grille.
            let grille = smoothstep01(0.3, 0.5, abs(sin(f.x * 40.0)) * abs(sin(f.y * 40.0)));
            light += glow * (0.35 + 0.25 * grille) * smoothstep01(0.05, 0.12, min(min(f.x, f.y), min(1.0 - f.x, 1.0 - f.y)));
        }
        if (level == 2 && h > 0.965) {
            light += glow * 0.8 * detail * (0.5 + 0.5 * step(0.5, fract(t * 0.7 + h * 9.0)));
        }
        k *= 3.0;
    }
    albedo *= 1.0 - 0.7 * seams;
    // Conduits of light along the largest seams.
    let g0 = fract((uv * 0.5 + 0.5) * 3.0);
    let conduit = 1.0 - smoothstep01(0.0, 0.012 + fp * 2.0, min(abs(g0.x - 0.5), abs(g0.y - 0.5)));
    light += glow * conduit * 0.25 * (0.6 + 0.4 * sin(t * 1.3 + face));
    // Scars: blasted dark, their edges smouldering.
    if (damage > 0.05) {
        let scar = noise(p * 0.06 + vec3<f32>(seed)) * 0.6 + noise(p * 0.17 + seed) * 0.4;
        let cut = 1.0 - damage * 0.9;
        let burnt = smoothstep01(cut - 0.08, cut, scar);
        albedo *= 1.0 - 0.75 * burnt;
        light *= 1.0 - 0.8 * burnt;
        light += vec3<f32>(1.0, 0.35, 0.06) * bump((scar - cut) / 0.025) * 0.6 * (0.7 + 0.3 * sin(t * 2.0 + scar * 30.0));
    }
    return array<vec3<f32>, 2>(albedo, light);
}

fn ev_cube(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let axis = normalize(params.ev[8].xyz);
    let angle = params.ev[8].w;
    let tone = params.ev[9].x;
    let glow = mix(hue_of(tone), vec3<f32>(1.0), 0.12) * 1.1;
    let damage = params.ev[9].y;
    let seed = params.ev[9].w;
    // The nearest of the cube and its escorts along the ray.
    var best = 1e9;
    var which = -1;
    var local_o = vec3<f32>(0.0);
    var local_d = vec3<f32>(0.0);
    for (var k = 0; k < 3; k++) {
        let o = ev_obj(k);
        if (o.w <= 0.0) {
            continue;
        }
        let lo = turn(ship - o.xyz, axis, -angle * (1.0 + f32(k) * 0.6));
        let ld = turn(d, axis, -angle * (1.0 + f32(k) * 0.6));
        let hits = box_hits(lo, ld, o.w);
        if (hits.x < hits.y && hits.x > 0.0 && hits.x < best) {
            best = hits.x;
            which = k;
            local_o = lo;
            local_d = ld;
        }
    }
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    let primary = ev_obj(0);
    if (which >= 0) {
        let o = ev_obj(which);
        let p = local_o + local_d * best;
        let q = p / o.w;
        let a = abs(q);
        var normal = vec3<f32>(sign(q.x), 0.0, 0.0);
        var uv = q.yz;
        var face = select(0.0, 1.0, q.x > 0.0);
        if (a.y >= a.x && a.y >= a.z) {
            normal = vec3<f32>(0.0, sign(q.y), 0.0);
            uv = q.xz;
            face = select(2.0, 3.0, q.y > 0.0);
        } else if (a.z >= a.x && a.z >= a.y) {
            normal = vec3<f32>(0.0, 0.0, sign(q.z));
            uv = q.xy;
            face = select(4.0, 5.0, q.z > 0.0);
        }
        let fp = best * params.frame.z / o.w;
        let surface = cube_face(uv, face, seed + f32(which) * 13.0, fp, glow, damage, p);
        let n = turn(normal, axis, angle * (1.0 + f32(which) * 0.6));
        let l = normalize(vec3<f32>(0.45, 0.75, -0.45));
        let lit = max(dot(n, l), 0.0) * 0.9 + 0.08 + max(dot(n, -l), 0.0) * 0.05;
        colour = surface[0] * lit * vec3<f32>(0.8, 0.9, 1.0) + surface[1] + glow * 0.012;
        // The edges catch the light.
        let edge = smoothstep01(0.985, 1.0, max(max(min(a.x, a.y), min(a.y, a.z)), min(a.x, a.z)));
        colour += vec3<f32>(0.6, 0.7, 0.75) * edge * 0.15 * lit;
        alpha = 1.0;
    }
    // The haze of its light round it.
    let v = primary.xyz - ship;
    let tca = dot(v, d);
    if (tca > 0.0) {
        let across = length(v - d * tca);
        colour += glow * exp(-max(across - primary.w * 1.05, 0.0) / (primary.w * 0.3)) * 0.025 * (1.0 - alpha);
    }
    // The tractor beam: from the face toward the ship to just below the
    // camera, widening as it nears, light running along it.
    let beam = params.ev[3].w;
    if (beam > 0.001) {
        let start = primary.xyz + normalize(ship - primary.xyz) * primary.w;
        let end = ship + vec3<f32>(0.0, -7.0, 9.0);
        let along = end - start;
        let span = length(along);
        let m = along / span;
        let near = ray_to_line(ship, d, start, m);
        let f = clamp(near.z / span, 0.0, 1.0);
        if (near.y > 0.0 && near.z > 0.0 && near.y < best) {
            let width = mix(primary.w * 0.2, 1.6, f * f);
            let core = exp(-sq(near.x / width));
            var stripes = 0.6 + 0.4 * sin(near.z * 0.35 - ev_t() * 14.0);
            if (params.ev[9].z > 0.5) {
                // A lattice of light, not a plain cone.
                stripes = 0.35 + 0.65 * smoothstep01(0.5, 0.95, abs(sin(near.x / width * 7.0 + near.z * 0.2 - ev_t() * 4.0)));
            }
            colour += glow * core * stripes * beam * (0.45 + 0.3 * f) * (1.0 - smoothstep01(0.9, 1.0, f));
        }
    }
    return vec4<f32>(colour, alpha);
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
    let bands = noise(vec3<f32>(cos(a) * rr * 0.5, sin(a) * rr * 0.5, rr * 1.7)) * 0.6
        + noise(vec3<f32>(cos(a) * rr * 1.6, sin(a) * rr * 1.6, rr * 4.0)) * 0.4;
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

@fragment
fn fs_event_sky(in: Full) -> @location(0) vec4<f32> {
    let d = to_ev(normalize(in.dir));
    let ship = ev_ship();
    let o = ev_obj(0);
    let hole = o.xyz;
    let rs = o.w;
    let v = hole - ship;
    var tca = dot(v, d);
    var closest = ship + d * max(tca, 0.0);
    let b = length(closest - hole);
    let critical = 2.6 * rs;
    let n = normalize(vec3<f32>(params.ev[9].x, 1.0, params.ev[9].y));
    // Bent toward the hole: weakly far off, ever harder near the photon
    // sphere.
    let reach = max(b, 1e-3);
    var bend = 2.0 * rs / reach * (1.0 + 1.6 * pow(critical / max(reach, critical), 3.0));
    if (tca < 0.0) {
        bend = 2.0 * rs / length(v) * 0.5;
    }
    bend = min(bend, 2.9);
    let away = normalize(closest - hole + vec3<f32>(1e-5));
    let bent = normalize(d * cos(bend) - away * sin(bend));
    let captured = tca > 0.0 && b < critical;
    var sky = vec3<f32>(0.0);
    if (!captured) {
        sky = ev_sky(bent);
        // Light piled up into an Einstein ring.
        sky *= 1.0 + 1.6 * exp(-sq((b - critical * 1.5) / (critical * 0.45)));
    }
    if (tca > 0.0) {
        sky += vec3<f32>(1.0, 0.86, 0.66) * bump((b - critical) / (rs * 0.07)) * 1.2;
    }
    // The disc on the straight run in, then on the bent run out (the far
    // side of it, seen over the shadow).
    let near = disc_at(ship, d, max(tca, 0.0), hole, rs, n);
    var far = vec4<f32>(0.0);
    if (!captured) {
        far = disc_at(closest, bent, 1e6, hole, rs, n);
    } else {
        far = disc_at(closest, d, length(closest - hole), hole, rs, n);
    }
    var colour = near.rgb + (1.0 - near.a) * (far.rgb + (1.0 - far.a) * sky);
    // The jets, out of its poles.
    for (var k = 0; k < 2; k++) {
        let pole = n * select(-1.0, 1.0, k == 0);
        let jet = ray_to_line(ship, d, hole, pole);
        if (jet.y > 0.0 && jet.z > rs * 2.0) {
            let width = rs * 0.25 + jet.z * 0.025;
            let knots = 0.6 + 0.4 * sin(jet.z / rs * 0.8 - ev_t() * 6.0);
            colour += vec3<f32>(0.45, 0.6, 1.0) * exp(-sq(jet.x / width)) * exp(-jet.z / (rs * 30.0)) * knots * 0.9 * (1.0 - near.a);
        }
    }
    let fade = params.ev[10].y;
    return vec4<f32>(colour * fade, fade);
}

// ---------------------------------------------------------------------------
// A nebula: glowing gas and dark lanes of dust marched through in a few
// widening steps, lit up from inside by lightning.
// ---------------------------------------------------------------------------

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
    var through = 1.0;
    var light = vec3<f32>(0.0);
    var dist = 14.0;
    for (var i = 0; i < 7; i++) {
        let p = ship + d * dist;
        let q = p * 0.004 + vec3<f32>(seed * 1.3, seed * 0.7, 0.0);
        let n1 = noise(q);
        let n2 = noise(q * 2.7 + 3.1);
        let gas = smoothstep01(0.2, 0.85, n1 * 0.75 + n2 * 0.35 + 0.1) * density;
        let dust = smoothstep01(0.2, 0.55, noise(q * 1.5 + 7.0));
        let span = dist * 0.8;
        let flash = exp(-length(p - strike.xyz) / 110.0) * strike.w * 0.8;
        let tint = mix(c1, c2, 0.5 + 0.5 * n2);
        light += through * (tint * gas * 0.0016 + vec3<f32>(0.85, 0.85, 1.0) * flash * gas * 0.01) * span;
        through *= exp(-(gas * 0.0007 + dust * 0.0025) * span);
        dist += span;
    }
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
            let local = turn(n, axis, -params.ev[8].w * 0.5);
            let band = bump((abs(lat) - 0.83 - 0.03 * noise(local * 4.0 + t * 0.05)) / 0.05);
            let curtain = smoothstep01(-0.2, 0.7, noise(local * vec3<f32>(18.0, 3.0, 18.0) + vec3<f32>(t * 0.15, 0.0, 0.0)));
            let hue = mix(vec3<f32>(0.15, 1.0, 0.45), vec3<f32>(0.9, 0.25, 0.6), params.ev[9].y * smoothstep01(0.86, 0.95, abs(lat)));
            let mu = abs(dot(n, d));
            colour += hue * band * curtain * 0.3 / max(mu, 0.2);
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
// A pulsar: a hard point, two beams sweeping round with it, the wind nebula
// it blows, a ring in its equator.
// ---------------------------------------------------------------------------

fn ev_pulsar(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let star = ev_obj(0).xyz;
    let beam = normalize(ev_obj(1).xyz);
    let axis = normalize(params.ev[8].xyz);
    let tint = mix(vec3<f32>(0.55, 0.7, 1.0), vec3<f32>(0.8, 0.55, 1.0), params.ev[9].x);
    let px = point_pixels(ship, d, star);
    var colour = vec3<f32>(0.9, 0.95, 1.0) * glint(px, 1.5) * 3.0 + tint * exp(-px * 0.02) * 0.25;
    // The beams, both ways out of the poles of its field.
    let line = ray_to_line(ship, d, star, beam);
    if (line.y > 0.0) {
        let width = 2.5 + abs(line.z) * 0.05;
        let core = exp(-sq(line.x / width)) + exp(-line.x / (width * 6.0)) * 0.15;
        colour += mix(tint, vec3<f32>(1.0), 0.4) * core * exp(-abs(line.z) / 1400.0) * 1.4 * params.ev[9].y;
    }
    // The wind nebula's ring, in its equator, wisps drifting out.
    let denom = dot(d, axis);
    if (abs(denom) > 1e-3) {
        let t = dot(star - ship, axis) / denom;
        if (t > 0.0) {
            let q = ship + d * t - star;
            let r = length(q);
            let wisps = 0.5 + 0.5 * noise(q * 0.03 + vec3<f32>(0.0, 0.0, ev_t() * 0.2));
            colour += tint * bump((r - 75.0) / 22.0) * wisps * 0.035 / max(abs(denom), 0.35);
        }
    }
    colour += tint * exp(-px * 0.01) * 0.012;
    return vec4<f32>(colour, 0.0);
}

// ---------------------------------------------------------------------------
// A derelict: a broken ring station, a band traced as a cylinder, its last
// windows lit, its breaks smouldering, a beacon blinking.
// ---------------------------------------------------------------------------

fn ev_derelict(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let centre = o.xyz;
    let radius = o.w;
    let height = ev_obj(1).x;
    let gaps = ev_obj(1).y;
    let axis = normalize(params.ev[8].xyz);
    let spin = params.ev[8].w;
    let seed = params.ev[9].w;
    let t = ev_t();
    var e1 = cross(axis, vec3<f32>(0.0, 1.0, 0.0));
    e1 = normalize(select(e1, vec3<f32>(1.0, 0.0, 0.0), length(e1) < 1e-3));
    let e2 = cross(axis, e1);
    let ro = ship - centre;
    let lo = vec3<f32>(dot(ro, e1), dot(ro, e2), dot(ro, axis));
    let ld = vec3<f32>(dot(d, e1), dot(d, e2), dot(d, axis));
    let a = ld.x * ld.x + ld.y * ld.y;
    let b = lo.x * ld.x + lo.y * ld.y;
    let c = lo.x * lo.x + lo.y * lo.y - radius * radius;
    let h = b * b - a * c;
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    var found = -1.0;
    var at = vec3<f32>(0.0);
    var phi = 0.0;
    var sector = 0.0;
    if (h > 0.0 && a > 1e-6) {
        let q = sqrt(h);
        for (var k = 0; k < 2; k++) {
            let tk = (-b + select(q, -q, k == 0)) / a;
            if (tk > 0.0 && found < 0.0) {
                let p = lo + ld * tk;
                if (abs(p.z) < height) {
                    let ph = atan2(p.y, p.x) - spin;
                    let sec = fract(ph / TAU * gaps + seed * 0.013);
                    let jag = 0.05 * noise(vec3<f32>(p.z * 0.12, ph * 14.0, seed));
                    if (sec < 0.8 + jag) {
                        found = tk;
                        at = p;
                        phi = ph;
                        sector = sec;
                    }
                }
            }
        }
    }
    if (found > 0.0) {
        let outward = normalize(vec3<f32>(at.x, at.y, 0.0));
        let inner = dot(outward, ld) > 0.0;
        let n = select(outward, -outward, inner);
        let l = normalize(vec3<f32>(dot(vec3<f32>(0.6, 0.5, -0.6), e1), dot(vec3<f32>(0.6, 0.5, -0.6), e2), dot(vec3<f32>(0.6, 0.5, -0.6), axis)));
        let lit = max(dot(n, l), 0.0);
        let around = phi * radius;
        let fp = found * params.frame.z;
        // Hull plates and their seams.
        let cell = vec2<f32>(around / 9.0, at.z / 5.0);
        let id = floor(cell);
        let f = cell - id;
        let plate = hash_cell(vec3<f32>(id, seed));
        let seam = 1.0 - smoothstep01(0.0, 0.05 + fp / 9.0, min(min(f.x, 1.0 - f.x), min(f.y, 1.0 - f.y)));
        var albedo = mix(vec3<f32>(0.15, 0.155, 0.165), vec3<f32>(0.2, 0.17, 0.14), params.ev[9].x) * (0.7 + 0.5 * plate) * (1.0 - 0.6 * seam);
        // Scorch and age.
        albedo *= 0.6 + 0.4 * smoothstep01(-0.4, 0.4, noise(vec3<f32>(around * 0.02, at.z * 0.05, seed)));
        colour = albedo * (lit * 1.1 + 0.03);
        // Ribs where it broke.
        let broke = smoothstep01(0.7, 0.8, sector);
        colour = mix(colour, albedo * 0.3 * (0.5 + 0.5 * abs(sin(around * 0.6))), broke);
        colour += vec3<f32>(1.0, 0.4, 0.08) * broke * smoothstep01(0.45, 0.8, noise(vec3<f32>(around * 0.1, at.z * 0.2, t * 0.6))) * 0.9;
        // Windows on the inner face, a few still lit, flickering.
        if (inner) {
            let w = vec2<f32>(around / 3.0, at.z / 2.6);
            let wid = floor(w);
            let wf = w - wid;
            let wh = hash_cell(vec3<f32>(wid, seed + 3.0));
            let pane = step(0.25, wf.x) * step(wf.x, 0.75) * step(0.3, wf.y) * step(wf.y, 0.7);
            let on = step(wh, params.ev[9].z * 0.35) * (0.7 + 0.3 * step(0.1, fract(t * (0.3 + wh * 2.0) + wh * 17.0)));
            let warm = mix(vec3<f32>(1.0, 0.72, 0.38), vec3<f32>(0.7, 0.86, 1.0), params.ev[9].y);
            colour += warm * pane * on * 1.4 * (1.0 - broke) * (1.0 - smoothstep01(0.3, 1.0, fp));
            colour += warm * on * 0.04 * (1.0 - broke);
        }
        let edge = (height - abs(at.z)) / fp;
        alpha = clamp(edge + 0.5, 0.0, 1.0);
        colour *= alpha;
    }
    // A beacon on the rim, blinking red.
    let ba = 0.6 - spin;
    let beacon = centre + (e1 * cos(ba) + e2 * sin(ba)) * radius + axis * height;
    let blink = step(0.85, fract(t * 0.7));
    colour += vec3<f32>(1.0, 0.15, 0.1) * glint(point_pixels(ship, d, beacon), 1.4) * blink * 1.5;
    return vec4<f32>(colour, alpha);
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
// ---------------------------------------------------------------------------

fn ev_leviathans(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let pod = ev_obj(0);
    let heading = normalize(ev_obj(1).xyz);
    let length_ = ev_obj(1).w;
    let seed = params.ev[9].z;
    let t = ev_t();
    let side = normalize(cross(heading, vec3<f32>(0.0, 1.0, 0.0)));
    let upward = cross(side, heading);
    let ca = hue_of(params.ev[9].x) * 0.6 + vec3<f32>(0.1, 0.35, 0.45);
    let cb = hue_of(params.ev[9].y) * 0.6 + vec3<f32>(0.3, 0.1, 0.4);
    var colour = vec3<f32>(0.0);
    let count = i32(pod.w);
    for (var c = 0; c < 5; c++) {
        if (c >= count) {
            break;
        }
        let fc = f32(c);
        let h = vec3<f32>(hash_cell(vec3<f32>(fc, seed, 1.0)), hash_cell(vec3<f32>(fc, seed, 2.0)), hash_cell(vec3<f32>(fc, seed, 3.0)));
        let size = mix(0.55, 1.15, h.x);
        let body = length_ * size;
        let head = pod.xyz + side * (h.y - 0.5) * 140.0 + upward * (h.z - 0.5) * 70.0 + heading * (h.x - 0.5) * 60.0;
        let phase = t * (1.1 + 0.3 * h.z) + h.y * TAU;
        for (var j = 0; j < 9; j++) {
            let f = f32(j) / 8.0;
            let wave = sin(phase - f * 4.2) * body * 0.07 * (0.25 + f);
            let p = head - heading * f * body + upward * wave + side * sin(phase * 0.5 - f * 2.0) * body * 0.03;
            let thick = body * 0.075 * (1.0 - 0.8 * f) * smoothstep01(-0.1, 0.12, f);
            let v = p - ship;
            let z = dot(v, d);
            if (z < 1.0) {
                continue;
            }
            let off = v - d * z;
            let across = length(off);
            let pulse_ = 0.5 + 0.5 * sin(t * 2.2 - f * 9.0 + h.z * 6.0);
            let tint = mix(ca, cb, f);
            colour += tint * (exp(-sq(across / thick)) * (0.3 + 0.7 * pulse_) * 0.55 + exp(-across / (thick * 3.0)) * 0.035);
            if (j == 2) {
                // The fins, wide sheets swept to each side, beating slowly.
                let beat = sin(phase * 0.8) * 0.5;
                let fin = exp(-sq((dot(off, upward) - dot(off, side) * beat) / (thick * 0.35))) * exp(-sq(dot(off, side) / (thick * 4.0)));
                colour += tint * fin * 0.35;
            }
        }
        // A trailing filament from the tail.
        let tail_from = head - heading * body;
        let trail = ray_to_line(ship, d, tail_from, -heading);
        if (trail.y > 0.0 && trail.z > 0.0 && trail.z < body * 0.8) {
            colour += cb * exp(-sq(trail.x / (body * 0.006 + 0.3))) * (1.0 - trail.z / (body * 0.8)) * 0.4;
        }
    }
    // Plankton glittering.
    let cell = floor(d * 230.0);
    let g = hash_cell(cell + vec3<f32>(seed));
    if (g > 0.996) {
        colour += ca * 0.5 * (0.5 + 0.5 * sin(t * 3.0 + g * 300.0));
    }
    return vec4<f32>(colour, 0.0);
}

// ---------------------------------------------------------------------------
// A supernova remnant: a shell of glowing filaments, red outside and teal
// within, brighter where it is seen edge on, flown through; the pulsar the
// star left at its heart.
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
    // The diffuse glow inside, and its heart.
    let inside = max(hits.y - max(hits.x, 0.0), 0.0) / radius;
    colour += vec3<f32>(0.25, 0.35, 0.9) * inside * 0.035;
    let px = point_pixels(ship, d, centre);
    colour += vec3<f32>(0.75, 0.85, 1.0) * glint(px, 1.4) * 1.5 + vec3<f32>(0.4, 0.55, 1.0) * exp(-px * 0.03) * 0.15;
    return vec4<f32>(colour, 0.0);
}

// ---------------------------------------------------------------------------
// A star gate: a ring (or a blocky ring with pylons, or two rings turning
// against each other) traced by marching its distance field; its nodes lit
// one by one as the ship lines up, its field opening in the middle, buoys
// chasing light down the approach.
// ---------------------------------------------------------------------------

// The gate's distance field, in its own frame (z its axis), and which node
// is nearest, as an angle step.
fn gate_field(p: vec3<f32>, radius: f32, style: f32, spin: f32) -> vec2<f32> {
    let thick = radius * 0.06;
    let nodes = select(9.0, 3.0, style > 0.5 && style < 1.5);
    let a = atan2(p.y, p.x);
    let step_ = TAU / nodes;
    let k = round(a / step_);
    let c = cos(-k * step_);
    let s = sin(-k * step_);
    let q = vec3<f32>(p.x * c - p.y * s, p.x * s + p.y * c, p.z);
    var dist = 1e9;
    if (style < 0.5) {
        // A ring and its nodes.
        dist = length(vec2<f32>(length(p.xy) - radius, p.z)) - thick;
        dist = min(dist, length(q - vec3<f32>(radius, 0.0, 0.0)) - thick * 1.9);
    } else if (style < 1.5) {
        // A blocky ring, three pylons out from it.
        let rr = length(p.xy) - radius;
        dist = max(abs(rr) - thick * 1.4, abs(p.z) - thick * 2.2);
        let pylon = abs(q - vec3<f32>(radius * 1.18, 0.0, 0.0)) - vec3<f32>(radius * 0.2, thick * 1.6, thick * 3.0);
        dist = min(dist, length(max(pylon, vec3<f32>(0.0))) + min(max(pylon.x, max(pylon.y, pylon.z)), 0.0));
    } else {
        // Two rings, turning against each other, nodes on the outer.
        let c2 = cos(spin * 2.0);
        let s2 = sin(spin * 2.0);
        let p2 = vec3<f32>(p.x * c2 - p.y * s2, p.x * s2 + p.y * c2, p.z);
        let wob = sin(atan2(p2.y, p2.x) * 12.0) * thick * 0.3;
        dist = length(vec2<f32>(length(p.xy) - radius, p.z - thick * 2.0)) - thick;
        dist = min(dist, length(vec2<f32>(length(p.xy) - radius * 0.86, p.z + thick * 2.0)) - thick * 0.8 - wob);
        dist = min(dist, length(q - vec3<f32>(radius, 0.0, thick * 2.0)) - thick * 1.6);
    }
    return vec2<f32>(dist, (k + nodes) % nodes);
}

fn ev_gate(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let centre = o.xyz;
    let radius = o.w;
    let normal = normalize(params.ev[8].xyz);
    let spin = params.ev[8].w;
    let style = params.ev[9].x;
    let hue = mix(vec3<f32>(1.0, 0.72, 0.3), select(vec3<f32>(0.75, 0.45, 1.0), vec3<f32>(0.4, 0.85, 1.0), params.ev[9].y < 0.66), step(0.33, params.ev[9].y));
    let u = params.ev[9].z;
    let t = ev_t();
    // The gate's frame: z along its axis, turning slowly round it.
    var e1 = normalize(cross(normal, vec3<f32>(0.0, 1.0, 0.0)));
    let e2 = cross(normal, e1);
    let c = cos(spin);
    let s = sin(spin);
    let f1 = e1 * c + e2 * s;
    let f2 = -e1 * s + e2 * c;
    let ro = ship - centre;
    let lo = vec3<f32>(dot(ro, f1), dot(ro, f2), dot(ro, normal));
    let ld = vec3<f32>(dot(d, f1), dot(d, f2), dot(d, normal));
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    var reach = 1e9;
    let bound = sphere_hits(lo, ld, vec3<f32>(0.0), radius * 1.45);
    if (bound.y > 0.0) {
        var tm = max(bound.x, 0.0);
        var node = 0.0;
        var hit = false;
        for (var i = 0; i < 56; i++) {
            let f = gate_field(lo + ld * tm, radius, style, spin);
            if (f.x < 0.0015 * tm + 0.02) {
                hit = true;
                node = f.y;
                break;
            }
            tm += f.x * 0.9;
            if (tm > bound.y) {
                break;
            }
        }
        if (hit) {
            reach = tm;
            let p = lo + ld * tm;
            let e = 0.02 * radius;
            let n = normalize(vec3<f32>(
                gate_field(p + vec3<f32>(e, 0.0, 0.0), radius, style, spin).x - gate_field(p - vec3<f32>(e, 0.0, 0.0), radius, style, spin).x,
                gate_field(p + vec3<f32>(0.0, e, 0.0), radius, style, spin).x - gate_field(p - vec3<f32>(0.0, e, 0.0), radius, style, spin).x,
                gate_field(p + vec3<f32>(0.0, 0.0, e), radius, style, spin).x - gate_field(p - vec3<f32>(0.0, 0.0, e), radius, style, spin).x,
            ));
            // Lit from the side and a little behind, so its curves show.
            let lw = normalize(vec3<f32>(0.8, 0.45, 0.4));
            let l = vec3<f32>(dot(lw, f1), dot(lw, f2), dot(lw, normal));
            let a = atan2(p.y, p.x);
            let panels = 0.7 + 0.3 * step(0.5, fract(a * radius / 5.0)) * step(0.3, fract(a * radius / 23.0));
            let albedo = vec3<f32>(0.1, 0.105, 0.115) * panels;
            let lit = max(dot(n, l), 0.0) * 1.3 + 0.02;
            let spec = pow(max(dot(reflect(-l, n), -ld), 0.0), 30.0) * 0.9;
            colour = albedo * lit + vec3<f32>(spec) * panels;
            // Running lights round its face, in its colour.
            let strip = bump((fract(a * radius / 7.0) - 0.5) / 0.08) * smoothstep01(0.6, 0.9, abs(n.z));
            colour += hue * strip * (0.15 + 0.6 * smoothstep01(0.2, 0.5, u)) * step(0.5, fract(a * radius / 7.0 * 0.5 + t * 0.4));
            // The nodes light one after another as it powers up, and the
            // ring's inner face takes their light.
            let nodes = select(9.0, 3.0, style > 0.5 && style < 1.5);
            let on = smoothstep01(0.25 + node / nodes * 0.25, 0.3 + node / nodes * 0.25, u);
            let node_at = vec3<f32>(cos(node * TAU / nodes), sin(node * TAU / nodes), 0.0) * radius * select(1.0, 1.18, style > 0.5 && style < 1.5);
            let near_node = smoothstep01(radius * 0.2, radius * 0.06, length(p - node_at));
            colour += hue * near_node * mix(0.06, 2.2 + 0.6 * sin(t * 6.0 + node), on);
            let inner_face = smoothstep01(0.0, -0.6, dot(n.xy, normalize(p.xy)));
            colour += hue * inner_face * smoothstep01(0.45, 0.65, u) * 0.35;
            alpha = 1.0;
        }
    }
    // The field opening across the middle: rippling light over the stars
    // seen through it, bent.
    if (abs(ld.z) > 1e-4) {
        let tf = -lo.z / ld.z;
        if (tf > 0.0 && tf < reach) {
            let q = lo + ld * tf;
            let r = length(q.xy) / radius;
            let open = smoothstep01(0.48, 0.62, u);
            let inside = 1.0 - smoothstep01(0.86, 0.9, r);
            if (inside > 0.0 && open > 0.0) {
                let a = atan2(q.y, q.x);
                let swirl = noise(vec3<f32>(cos(a + r * 3.0 - t * 0.8) * r * 3.0, sin(a + r * 3.0 - t * 0.8) * r * 3.0, t * 0.4));
                let ripple = 0.5 + 0.5 * sin(r * 30.0 - t * 7.0 + swirl * 3.0);
                let bent = normalize(d + (f1 * q.x + f2 * q.y) / radius * 0.25 * open);
                let behind = ev_sky(bent) * 2.0;
                let field = hue * (0.35 + 0.9 * ripple * ripple + 0.6 * swirl) * (0.45 + 0.9 * r * r * r) + vec3<f32>(1.0) * pow(1.0 - r, 5.0) * 0.9
                    + mix(hue, vec3<f32>(1.0), 0.5) * bump((r - 0.87) / 0.025) * 1.5;
                let cover = open * inside * 0.85;
                colour = colour * alpha + (1.0 - alpha) * (field + behind) * cover;
                alpha = max(alpha, cover);
            }
        }
    }
    // The nodes' light, seen from afar as glints.
    let nodes = select(9.0, 3.0, style > 0.5 && style < 1.5);
    for (var k = 0; k < 9; k++) {
        let fk = f32(k);
        if (fk >= nodes) {
            break;
        }
        let on = smoothstep01(0.25 + fk / nodes * 0.25, 0.3 + fk / nodes * 0.25, u);
        let ang = fk * TAU / nodes;
        let at = centre + (f1 * cos(ang) + f2 * sin(ang)) * radius * select(1.0, 1.18, style > 0.5 && style < 1.5);
        let px = point_pixels(ship, d, at);
        colour += hue * (exp(-px / 3.0) * 0.8 + exp(-px / 18.0) * 0.12) * on;
    }
    // Buoys down the approach, chasing light toward the gate.
    for (var k = 0; k < 12; k++) {
        let fk = f32(k);
        let z = centre.z - 40.0 - fk * 46.0;
        let chase = step(fract(t * 0.9 - fk * 0.083), 0.18);
        for (var side = 0; side < 2; side++) {
            let x = select(-1.0, 1.0, side == 0) * radius * 1.3;
            let buoy = vec3<f32>(centre.x + x, centre.y - radius * 0.35, z);
            let px = point_pixels(ship, d, buoy);
            colour += hue * glint(px, 1.2) * (0.25 + 1.4 * chase) * (1.0 - alpha);
        }
    }
    return vec4<f32>(colour, alpha);
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

// Hull plating: panels a cell of a lattice each, their own height 0..1 and
// sunk at their seams, so a hull's skin carries plates and panel lines in its
// shape, not only in its colour.
fn plating(q: vec3<f32>, scale: f32) -> f32 {
    let g = q * scale;
    let f = fract(g) - 0.5;
    let inner = 1.0 - smoothstep(0.4, 0.5, max(max(abs(f.x), abs(f.y)), abs(f.z)));
    return (0.35 + 0.65 * hash_cell(floor(g))) * inner;
}

fn sd_cylinder_z(p: vec3<f32>, r: f32, h: f32) -> f32 {
    let d = vec2<f32>(length(p.xy) - r, abs(p.z) - h);
    return min(max(d.x, d.y), 0.0) + length(max(d, vec2<f32>(0.0)));
}

// A wedge's tier: tip at `tip` ahead, `half_width` either side at the stern
// (z = -1), its deck `top` high, sloping down to its edges.
fn wedge_tier(p: vec3<f32>, tip: f32, half_width: f32, top: f32, floor_: f32) -> f32 {
    let m = vec2<f32>(abs(p.x), p.z);
    let n = normalize(vec2<f32>(1.0 + tip, half_width));
    let edge = dot(m - vec2<f32>(0.0, tip), n);
    let back = -p.z - 1.0;
    let deck = p.y - (top - abs(p.x) * 0.18);
    return max(max(max(edge, back), deck), floor_ - p.y);
}

// A hull of design `kind_`, in its own frame (x to starboard, y up, z
// ahead), spanning about -1..1 along z.
fn hull_sdf(p: vec3<f32>, kind_: i32) -> f32 {
    let m = vec3<f32>(abs(p.x), p.y, p.z);
    if (kind_ == 0) {
        // A saucer in two decks, a bridge dome on top and a rim stepped
        // below; on a neck, the engineering hull with its deflector dish
        // ahead and a shuttle bay astern; nacelles high and aft on thin
        // swept pylons, their bussard caps forward.
        let sc = p - vec3<f32>(0.0, 0.06, 0.42);
        var saucer = sd_ellipsoid(sc, vec3<f32>(0.56, 0.05, 0.56));
        saucer = min(saucer, sd_ellipsoid(sc - vec3<f32>(0.0, 0.035, 0.0), vec3<f32>(0.38, 0.045, 0.38)));
        saucer = min(saucer, sd_ellipsoid(sc - vec3<f32>(0.0, -0.03, 0.0), vec3<f32>(0.42, 0.04, 0.42)));
        saucer = min(saucer, sd_ellipsoid(sc - vec3<f32>(0.0, 0.085, 0.0), vec3<f32>(0.085, 0.035, 0.085)));
        saucer -= plating(p, 22.0) * 0.004;
        var hull = sd_capsule(p, vec3<f32>(0.0, -0.17, 0.1), vec3<f32>(0.0, -0.14, -0.52), 0.085);
        hull = max(hull, -sd_box(p - vec3<f32>(0.0, -0.13, -0.6), vec3<f32>(0.045, 0.035, 0.07)));
        hull = min(hull, sd_ellipsoid(p - vec3<f32>(0.0, -0.17, 0.17), vec3<f32>(0.065, 0.065, 0.025)));
        hull -= plating(p, 26.0) * 0.004;
        let neck = sd_box(p - vec3<f32>(0.0, -0.05, 0.13), vec3<f32>(0.03, 0.075, 0.11));
        let nacelle = sd_capsule(m, vec3<f32>(0.34, 0.17, -0.2), vec3<f32>(0.34, 0.17, -1.0), 0.05);
        let bussard = sd_ellipsoid(m - vec3<f32>(0.34, 0.17, -0.19), vec3<f32>(0.052, 0.052, 0.06));
        let pylon = sd_capsule(m, vec3<f32>(0.03, -0.11, -0.36), vec3<f32>(0.34, 0.15, -0.5), 0.016);
        return min(min(saucer, min(hull, neck)), min(min(nacelle, bussard), pylon));
    } else if (kind_ == 1) {
        // A wedge in three tiers, each plated, a dorsal spine; the bridge
        // tower at the stern with its two sensor globes; a bank of engines
        // across the stern.
        var d = wedge_tier(p, 1.0, 0.6, 0.06 + 0.07 * (1.0 - p.z) * 0.5, -0.05);
        d = min(d, wedge_tier(p, 0.25, 0.34, 0.15, 0.0));
        d = min(d, wedge_tier(p, -0.3, 0.2, 0.2, 0.0));
        d = min(d, sd_box(p - vec3<f32>(0.0, 0.08 - p.z * 0.04, -0.1), vec3<f32>(0.012, 0.03, 0.6)));
        d -= plating(p, 34.0) * 0.007;
        d = min(d, sd_box(p - vec3<f32>(0.0, 0.25, -0.8), vec3<f32>(0.05, 0.06, 0.05)));
        d = min(d, sd_box(p - vec3<f32>(0.0, 0.31, -0.81), vec3<f32>(0.19, 0.022, 0.04)));
        d = min(d, length(m - vec3<f32>(0.15, 0.355, -0.81)) - 0.026);
        let engines = min(sd_cylinder_z(m - vec3<f32>(0.0, 0.07, -1.02), 0.07, 0.04), sd_cylinder_z(m - vec3<f32>(0.2, 0.06, -1.02), 0.05, 0.04));
        return min(d * 0.7, min(engines, sd_cylinder_z(m - vec3<f32>(0.34, 0.04, -1.01), 0.03, 0.03)));
    } else if (kind_ == 2) {
        // A raider: an armoured head on a long neck, a body with its engine
        // block, and wings swept down, their trailing edges cut into
        // feathers, cannons at their tips.
        let head = sd_ellipsoid(p - vec3<f32>(0.0, 0.0, 0.78), vec3<f32>(0.11, 0.07, 0.17));
        let brow = sd_ellipsoid(p - vec3<f32>(0.0, 0.05, 0.74), vec3<f32>(0.07, 0.03, 0.1));
        let neck = sd_capsule(p, vec3<f32>(0.0, 0.03, 0.64), vec3<f32>(0.0, 0.06, 0.08), 0.042);
        var body = sd_ellipsoid(p - vec3<f32>(0.0, 0.05, -0.15), vec3<f32>(0.25, 0.09, 0.32));
        body = min(body, sd_box(p - vec3<f32>(0.0, 0.03, -0.42), vec3<f32>(0.12, 0.05, 0.08)));
        body -= plating(p, 24.0) * 0.006;
        let w = m - vec3<f32>(0.12, 0.0, -0.18);
        let c = cos(0.38);
        let sn = sin(0.38);
        let along = w.x * c + w.y * sn;
        let across = -w.x * sn + w.y * c;
        var wing = sd_box(vec3<f32>(along - 0.42, across, w.z + along * 0.12), vec3<f32>(0.44, 0.016, 0.22 - along * 0.12));
        let feather = abs(fract(along * 9.0) - 0.5);
        wing = max(wing, -(max(feather - 0.06, -(w.z + along * 0.12 + 0.12 - along * 0.12))));
        let root = sd_ellipsoid(w - vec3<f32>(0.08, 0.0, 0.0), vec3<f32>(0.1, 0.05, 0.16));
        let tip = sd_capsule(m, vec3<f32>(0.86, -0.36, 0.08), vec3<f32>(0.86, -0.36, -0.3), 0.024);
        return min(min(min(head, brow), min(neck, body)), min(min(wing, root), tip));
    } else if (kind_ == 3) {
        // A warbird: seen from the side, two great wings arching above and
        // below with open space between, broadest at their middles and
        // narrowing to the joins fore and aft, thin at their edges; a long
        // neck to the command section ahead, nacelles under the lower wing.
        let q = p - vec3<f32>(0.0, 0.0, -0.25);
        let ang = atan2(q.y * 1.15, q.z * 0.85);
        let around = length(vec2<f32>(q.y * 1.15, q.z * 0.85)) - 0.52;
        let span = 0.7 * (0.25 + 0.75 * abs(sin(ang)));
        let thin = 0.035 * (1.0 - 0.7 * sq(clamp(q.x / span, -1.0, 1.0)));
        var wings = sd_box2(vec2<f32>(around, q.x), vec2<f32>(thin, span)) * 0.75;
        wings -= plating(p, 20.0) * 0.004;
        let neck = sd_capsule(p, vec3<f32>(0.0, 0.0, 0.2), vec3<f32>(0.0, -0.01, 0.8), 0.045);
        let command = sd_ellipsoid(p - vec3<f32>(0.0, -0.01, 0.86), vec3<f32>(0.15, 0.075, 0.12));
        let dome = sd_ellipsoid(p - vec3<f32>(0.0, 0.05, 0.86), vec3<f32>(0.06, 0.03, 0.05));
        let nacelle = sd_capsule(m, vec3<f32>(0.5, -0.47, -0.1), vec3<f32>(0.5, -0.47, -0.55), 0.035);
        return min(min(wings, neck), min(min(command, dome), nacelle));
    } else if (kind_ == 4) {
        // A trader: a broad horseshoe in two decks, open ahead, its prongs
        // ending in claws; a raised bridge astern, engines behind it.
        let q = p - vec3<f32>(0.0, 0.0, -0.05);
        let open = sd_box(q - vec3<f32>(0.0, 0.0, 0.6), vec3<f32>(0.24, 0.3, 0.32));
        let lower = max((length(vec2<f32>((length(q.xz) - 0.52) / 0.25, q.y / 0.075)) - 1.0) * 0.07, -open);
        let upper = max((length(vec2<f32>((length(q.xz) - 0.46) / 0.17, (q.y - 0.075) / 0.05)) - 1.0) * 0.045, -open - 0.04);
        var d = min(lower, upper) - plating(p, 26.0) * 0.005;
        let claw = sd_ellipsoid(vec3<f32>(abs(q.x), q.y, q.z) - vec3<f32>(0.4, -0.01, 0.4), vec3<f32>(0.07, 0.05, 0.12));
        let bridge = sd_ellipsoid(q - vec3<f32>(0.0, 0.12, -0.42), vec3<f32>(0.2, 0.07, 0.14));
        let engines = sd_cylinder_z(vec3<f32>(abs(q.x), q.y, q.z) - vec3<f32>(0.14, 0.02, -0.66), 0.05, 0.05);
        return min(min(d, claw), min(bridge, engines));
    }
    // A cube, its faces cut into recessed panels at two scales.
    return sd_box(p, vec3<f32>(0.62)) + plating(p, 9.0) * 0.022 + plating(p, 23.0) * 0.008 - 0.02;
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

// Half the length of each design's capital ship and escorts, in units.
fn faction_size(f: i32, capital: bool) -> f32 {
    var big = array<f32, 6>(13.0, 32.0, 9.5, 27.0, 9.5, 40.0);
    var small = array<f32, 6>(7.0, 4.0, 2.4, 13.0, 6.5, 9.0);
    return select(small[clamp(f, 0, 5)], big[clamp(f, 0, 5)], capital);
}

fn ship_faction(i: i32) -> i32 {
    return i32(select(ev_obj(1).x, ev_obj(1).y, i >= 4));
}

fn battle_lost() -> i32 {
    return i32(ev_obj(2).w);
}

// Ship `i` of the battle at `t` (0-3 one fleet, 4-7 the other), as it
// would fly with nothing blowing up: where it is, its size; which way it
// heads.
fn battle_course(i: i32, t: f32) -> array<vec4<f32>, 2> {
    let o = ev_obj(0);
    let seed = params.ev[9].x;
    let fi = f32(i);
    let fleet = select(1.0, -1.0, i >= 4);
    let capital = i == 0 || i == 4;
    let h = vec4<f32>(
        hash_cell(vec3<f32>(fi, seed, 1.0)),
        hash_cell(vec3<f32>(fi, seed, 2.0)),
        hash_cell(vec3<f32>(fi, seed, 3.0)),
        hash_cell(vec3<f32>(fi, seed, 4.0)),
    );
    let size = faction_size(ship_faction(i), capital);
    // The capitals hold their fleet's side, slowly; escorts wheel round.
    let rate = select((0.06 + 0.05 * h.x) * fleet, 0.015 * fleet, capital);
    let a = t * rate + select(h.y * TAU, 0.0, capital) + select(0.0, PI, i >= 4) + 0.6;
    let r = select(o.w * (0.4 + 0.5 * h.z), o.w * 0.55, capital);
    let y = select((h.w - 0.5) * o.w * 0.45, (h.w - 0.5) * o.w * 0.15, capital) + sin(t * 0.4 + fi) * 3.0;
    let p = o.xyz + vec3<f32>(cos(a) * r, y, sin(a) * r * 0.65);
    var heading = normalize(vec3<f32>(-sin(a) * fleet, 0.05 * cos(t * 0.4 + fi), cos(a) * 0.65 * fleet));
    if (capital) {
        // Broadside on to the other fleet.
        heading = normalize(vec3<f32>(-sin(a), 0.0, cos(a)));
    }
    return array<vec4<f32>, 2>(vec4<f32>(p, size), vec4<f32>(heading, 0.0));
}

// The blast: where, when, and how far its shock has come at `t`.
fn blast_at() -> vec3<f32> {
    return params.ev[8].xyz;
}

fn shock_radius(t: f32) -> f32 {
    return max(t - ev_obj(1).z, 0.0) * params.ev[8].w;
}

// Ship `i` at `t`, as `battle_course`, the lost one flown to the blast as it
// dies, the rest shoved aside by its shock as it passes them; w of the
// second row, how hard the shock is striking it now, 0..1.
fn battle_ship(i: i32, t: f32) -> array<vec4<f32>, 2> {
    if (params.ev[9].y > 0.5) {
        // The screenshots' showcase: the one ship asked for, alone, ahead
        // of the camera at a distance by its size, seen three-quarters.
        let size = faction_size(ship_faction(i), i == 0 || i == 4);
        let shown = i == i32(params.ev[9].z);
        let at = select(ev_ship() - vec3<f32>(0.0, 0.0, 1e4), ev_ship() + vec3<f32>(0.0, 0.0, size * 3.3), shown);
        return array<vec4<f32>, 2>(vec4<f32>(at, size), vec4<f32>(normalize(vec3<f32>(-0.78, -0.18, -0.6)), 0.0));
    }
    var s = battle_course(i, t);
    let kill = ev_obj(1).z;
    let blast = blast_at();
    if (i == battle_lost()) {
        s[0] = vec4<f32>(s[0].xyz + blast - battle_course(i, kill)[0].xyz, s[0].w);
        return s;
    }
    let away = s[0].xyz - blast;
    let dist = length(away);
    let age = t - kill - dist / params.ev[8].w;
    let strength = 26.0 / (1.0 + s[0].w / 6.0) * exp(-dist / 260.0);
    let shove = normalize(away) * strength * smoothstep01(0.0, 2.2, age);
    let jolt = exp(-sq(age / 0.5)) * step(-1.0, age);
    // Thrown off its heading a little, and back.
    let heading = normalize(s[1].xyz + normalize(away) * 0.35 * jolt);
    return array<vec4<f32>, 2>(vec4<f32>(s[0].xyz + shove, s[0].w), vec4<f32>(heading, jolt));
}

// The lights each design carries: nacelles, engines, windows, wingtips,
// and the markings painted on it (a factor on its hull's colour).
fn hull_lights(q: vec3<f32>, kind_: i32, fire: vec3<f32>, t: f32) -> vec3<f32> {
    let m = vec3<f32>(abs(q.x), q.y, q.z);
    var light = vec3<f32>(0.0);
    // Rows of windows: lit in short runs along a band of the hull.
    let windows = step(0.55, fract(q.x * 60.0 + q.z * 60.0)) * step(0.35, hash_cell(floor(q * 18.0)));
    if (kind_ == 0) {
        let nacelle = sd_capsule(m, vec3<f32>(0.34, 0.17, -0.2), vec3<f32>(0.34, 0.17, -1.0), 0.056);
        if (nacelle < 0.01) {
            let inboard = smoothstep(0.2, 0.6, abs(q.x - sign(q.x) * 0.34) / 0.05);
            light += vec3<f32>(0.3, 0.6, 1.0) * (0.5 + 0.9 * step(0.5, fract(q.z * 20.0))) * inboard * step(q.z, -0.3) * step(-0.95, q.z);
            light += vec3<f32>(1.0, 0.25, 0.08) * 2.4 * smoothstep(-0.26, -0.17, q.z);
        }
        let rim = length(vec2<f32>(q.x, q.z - 0.42));
        light += vec3<f32>(1.0, 0.9, 0.7) * windows * (bump((rim - 0.5) / 0.012) + bump((rim - 0.34) / 0.01) * 0.6) * step(abs(q.y - 0.06), 0.06) * 0.9;
        light += vec3<f32>(0.4, 0.7, 1.0) * bump(length(q - vec3<f32>(0.0, -0.17, 0.19)) / 0.05) * 2.0;
        light += vec3<f32>(1.0, 0.15, 0.08) * bump(length(m - vec3<f32>(0.06, 0.07, -0.13)) / 0.025) * 1.5;
        light += vec3<f32>(1.0, 0.9, 0.7) * windows * step(abs(q.y + 0.15), 0.02) * step(q.z, 0.05) * step(-0.45, q.z) * 0.7;
    } else if (kind_ == 1) {
        light += vec3<f32>(0.65, 0.82, 1.0) * smoothstep(-1.04, -1.06, q.z) * 3.0;
        light += vec3<f32>(1.0, 0.95, 0.85) * step(0.97, hash_cell(floor(q * 70.0))) * 0.8;
        light += vec3<f32>(1.0, 0.95, 0.85) * windows * step(abs(q.y - 0.26), 0.012) * 0.8;
    } else if (kind_ == 2) {
        light += vec3<f32>(1.0, 0.3, 0.12) * smoothstep(-0.48, -0.51, q.z) * step(abs(q.x), 0.12) * 1.8;
        light += fire * bump(length(m - vec3<f32>(0.86, -0.36, 0.1)) / 0.04) * 2.0;
        light += vec3<f32>(1.0, 0.4, 0.2) * windows * step(abs(q.y), 0.02) * step(0.72, q.z) * 1.4;
    } else if (kind_ == 3) {
        light += vec3<f32>(0.3, 1.0, 0.6) * bump(length(q - vec3<f32>(0.0, -0.08, 0.86)) / 0.07) * 1.6;
        light += vec3<f32>(0.4, 1.0, 0.6) * smoothstep(-0.52, -0.56, q.z) * step(0.48, abs(q.x)) * step(q.y, -0.43) * 2.0;
        light += vec3<f32>(0.7, 1.0, 0.85) * windows * step(abs(q.y + 0.01), 0.02) * step(0.82, q.z) * 1.2;
    } else if (kind_ == 4) {
        let rim = length(q.xz - vec2<f32>(0.0, -0.05));
        light += vec3<f32>(1.0, 0.75, 0.3) * windows * bump((rim - 0.75) / 0.02) * step(abs(q.y), 0.03) * 1.2;
        light += vec3<f32>(1.0, 0.55, 0.2) * smoothstep(-0.71, -0.74, q.z) * step(abs(q.y - 0.02), 0.06) * 2.0;
        light += vec3<f32>(0.8, 1.0, 0.6) * bump(length(m - vec3<f32>(0.4, 0.0, 0.53)) / 0.03) * 1.5;
    } else {
        let g = floor((q + 0.62) * 18.0);
        let h = hash_cell(g + vec3<f32>(3.0));
        light += vec3<f32>(0.2, 1.0, 0.4) * step(0.9, h) * (0.6 + 0.4 * sin(t * 3.0 + h * 50.0));
    }
    return light;
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
    let s = battle_ship(i, ev_t());
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

// A bolt of light from `a` to `b`, `w` pixels wide.
fn bolt(o: vec3<f32>, d: vec3<f32>, a: vec3<f32>, b: vec3<f32>, w: f32) -> f32 {
    let span = length(b - a);
    let m = (b - a) / max(span, 1e-3);
    let near = ray_to_line(o, d, a, m);
    if (near.y <= 0.0 || near.z < 0.0 || near.z > span) {
        return 0.0;
    }
    let px = near.x / (near.y * params.frame.z);
    return exp(-px * px / (2.0 * w * w)) * 1.6 + exp(-px / (w * 5.0)) * 0.12;
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
    let ship = ev_ship();
    let t = ev_t();
    let seed = params.ev[9].x;
    let lost = battle_lost();
    let kill = ev_obj(1).z;
    let blast = blast_at();
    let l = normalize(vec3<f32>(0.5, 0.7, -0.35));
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    var reach = 1e9;
    // The shock: a shell racing out from the blast, bending the stars
    // behind its edge and glowing there, fading as it spreads.
    let wave = shock_radius(t);
    let wave_age = wave / (params.ev[8].w * 9.0);
    if (wave > 0.0 && wave_age < 1.0) {
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
    // The ships, far to near not needed: the nearest hit along each ray.
    for (var i = 0; i < 8; i++) {
        if (i == lost && t > kill) {
            continue;
        }
        let s = battle_ship(i, t);
        let c = s[0].xyz;
        let size = s[0].w;
        let faction = ship_faction(i);
        let kind_ = faction;
        let fwd = s[1].xyz;
        let side = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
        let up = cross(side, fwd);
        let bound = sphere_hits(ship, d, c, size * 1.15);
        // Traced only until the meshes are ready (`vs_ship`).
        if (bound.y > 0.0 && bound.x < reach && params.ev[9].w < 0.5) {
            let ro = (ship - c) / size;
            let lo = vec3<f32>(dot(ro, side), dot(ro, up), dot(ro, fwd));
            let ld = vec3<f32>(dot(d, side), dot(d, up), dot(d, fwd));
            var tm = max(bound.x, 0.0) / size;
            let end = bound.y / size;
            var hit = false;
            for (var k = 0; k < 72; k++) {
                let dist = hull_sdf(lo + ld * tm, kind_);
                if (dist < 0.0006 + tm * params.frame.z * 0.35) {
                    hit = true;
                    break;
                }
                tm += max(dist * 0.8, 0.0008);
                if (tm > end) {
                    break;
                }
            }
            if (hit && tm * size < reach) {
                reach = tm * size;
                let q = lo + ld * tm;
                let e = max(0.0012, tm * params.frame.z * 0.5);
                let nl = normalize(vec3<f32>(
                    hull_sdf(q + vec3<f32>(e, 0.0, 0.0), kind_) - hull_sdf(q - vec3<f32>(e, 0.0, 0.0), kind_),
                    hull_sdf(q + vec3<f32>(0.0, e, 0.0), kind_) - hull_sdf(q - vec3<f32>(0.0, e, 0.0), kind_),
                    hull_sdf(q + vec3<f32>(0.0, 0.0, e), kind_) - hull_sdf(q - vec3<f32>(0.0, 0.0, e), kind_),
                ));
                let n = side * nl.x + up * nl.y + fwd * nl.z;
                // Plates each a shade of their own, and hollows darker.
                let panels = 0.82 + 0.3 * hash_cell(floor(q * 22.0));
                let ao = clamp(hull_sdf(q + nl * 0.03, kind_) / 0.03, 0.35, 1.0);
                let lit = (max(dot(n, l), 0.0) * 1.15 + 0.035 + max(dot(n, -l), 0.0) * 0.04) * ao;
                colour = faction_hull(faction) * hull_markings(q, kind_) * panels * lit;
                if (kind_ == 5) {
                    // The collective's cubes: the hive cube's machinery.
                    let a = abs(q);
                    var uv = q.yz;
                    var face = select(0.0, 1.0, q.x > 0.0);
                    if (a.y >= a.x && a.y >= a.z) {
                        uv = q.xz;
                        face = select(2.0, 3.0, q.y > 0.0);
                    } else if (a.z >= a.x && a.z >= a.y) {
                        uv = q.xy;
                        face = select(4.0, 5.0, q.z > 0.0);
                    }
                    let surface = cube_face(uv / 0.62, face, seed + f32(i) * 7.0, tm * params.frame.z / 0.62, vec3<f32>(0.25, 1.0, 0.4), 0.25, q * size);
                    colour = surface[0] * lit * 1.4 + surface[1];
                }
                colour += vec3<f32>(pow(max(dot(reflect(-l, n), -d), 0.0), 26.0) * 0.45);
                colour += hull_lights(q, kind_, faction_fire(faction), t);
                // Lit orange by the blast as its fireball burns.
                let glare = exp(-max(t - kill, 0.0) / 1.5) * step(kill, t);
                colour += vec3<f32>(1.0, 0.5, 0.2) * max(dot(n, normalize(blast - c)), 0.0) * glare * 0.6 * exp(-length(blast - c) / 150.0);
                alpha = 1.0;
            }
        }
        // The shield: flaring where the shock strikes it, holding most of
        // it back, the struck side brightest and crackling.
        let jolt = s[1].w;
        if (jolt > 0.01) {
            let sh = sphere_hits(ship, d, c, size * 1.45);
            if (sh.x > 0.0 && sh.x < reach + size) {
                let at = ship + d * sh.x;
                let n = (at - c) / (size * 1.45);
                let facing = max(dot(n, normalize(blast - c)), 0.0);
                let v = c - ship;
                let edge = length(v - d * dot(v, d)) / (size * 1.45);
                let crackle = 0.6 + 0.4 * noise(n * 9.0 + vec3<f32>(t * 6.0));
                let flare = (pow(facing, 10.0) * 0.7 * crackle + pow(edge, 14.0) * 0.35) * jolt;
                colour += faction_shield(faction) * flare;
                alpha = max(alpha, min(flare * 0.15, 1.0));
            }
        }
        // Engines.
        let engine = point_pixels(ship, d, c - fwd * size * 1.02);
        colour += mix(faction_shield(faction), vec3<f32>(1.0), 0.5) * glint(engine, 1.0 + size * 0.03) * 0.9 * (1.0 - alpha * 0.8);
    }
    if (params.ev[9].y > 0.5) {
        return vec4<f32>(colour, alpha);
    }
    // Fire between the fleets: each its own way. Beams, or bolts in short
    // dashes for the wedge fleet.
    for (var k = 0; k < 7; k++) {
        let fk = f32(k);
        let clock_ = t * 1.6 + fk * 0.37;
        let bin = floor(clock_);
        let on = fract(clock_);
        if (on > 0.35) {
            continue;
        }
        let h = hash_cell(vec3<f32>(bin, fk, seed));
        let shooter = i32(h * 7.99);
        let aim = (shooter / 4 * 4 + 4 + i32(fract(h * 13.0) * 3.99)) % 8;
        if ((shooter == lost || aim == lost) && t > kill) {
            continue;
        }
        let a = battle_ship(shooter, t)[0];
        let b = battle_ship(aim, t)[0];
        let faction = ship_faction(shooter);
        let miss = (vec3<f32>(fract(h * 31.0), fract(h * 57.0), fract(h * 91.0)) - 0.5) * b.w * 3.0 * step(0.6, fract(h * 7.0));
        let from_hull = a.xyz + normalize(b.xyz - a.xyz) * a.w * 0.6;
        var beam = bolt(ship, d, from_hull, b.xyz + miss, select(1.1, 1.8, faction == 5)) * sin(on / 0.35 * PI);
        if (faction == 1 || faction == 2) {
            // Bolts: dashes flying along the line.
            let span = length(b.xyz + miss - from_hull);
            let near = ray_to_line(ship, d, from_hull, normalize(b.xyz + miss - from_hull));
            beam *= step(fract(near.z / span * 5.0 - on * 9.0), 0.3);
        }
        colour += mix(faction_fire(faction), vec3<f32>(1.0), 0.4) * beam * 1.4;
        // Where a beam lands, its target's shield flares.
        if (fract(h * 7.0) < 0.6 && on > 0.05) {
            let sh = sphere_hits(ship, d, b.xyz, b.w * 1.45);
            if (sh.x > 0.0) {
                let n = (ship + d * sh.x - b.xyz) / (b.w * 1.45);
                let facing = max(dot(n, normalize(a.xyz - b.xyz)), 0.0);
                colour += faction_shield(ship_faction(aim)) * pow(facing, 5.0) * 0.7 * sin(on / 0.35 * PI);
            }
        }
    }
    // Torpedoes: glowing slugs arcing over with their trails, bursting on
    // the target's shield.
    for (var k = 0; k < 5; k++) {
        let fk = f32(k);
        let clock_ = t / 2.6 + fk * 0.23;
        let bin = floor(clock_);
        let phase = fract(clock_);
        let h = hash_cell(vec3<f32>(bin, fk + 10.0, seed));
        let shooter = i32(h * 7.99);
        let aim = (shooter / 4 * 4 + 4 + i32(fract(h * 17.0) * 3.99)) % 8;
        if ((shooter == lost || aim == lost) && t > kill - 2.0) {
            continue;
        }
        let a = battle_ship(shooter, t)[0].xyz;
        let goal = battle_ship(aim, t)[0];
        let flight = clamp(phase / 0.7, 0.0, 1.0);
        let arc = vec3<f32>(0.0, 1.0, 0.0) * sin(flight * PI) * 25.0;
        let p = mix(a, goal.xyz, flight) + arc;
        let tint = mix(faction_fire(ship_faction(shooter)), vec3<f32>(1.0, 0.25, 0.1), 0.5);
        if (phase < 0.7) {
            let pulse_ = 0.75 + 0.25 * sin(t * 30.0 + fk);
            colour += tint * glint(point_pixels(ship, d, p), 2.0) * 1.6 * pulse_;
            let back = mix(a, goal.xyz, max(flight - 0.12, 0.0)) + vec3<f32>(0.0, 1.0, 0.0) * sin(max(flight - 0.12, 0.0) * PI) * 25.0;
            colour += tint * bolt(ship, d, back, p, 1.0) * 0.35;
        } else {
            let age = (phase - 0.7) / 0.3;
            let at = goal.xyz + normalize(a - goal.xyz) * goal.w * 1.45;
            colour += burst(ship, d, at, max(goal.w * 0.7, 4.0), age) * 0.6;
            let sh = sphere_hits(ship, d, goal.xyz, goal.w * 1.45);
            if (sh.x > 0.0) {
                let n = (ship + d * sh.x - goal.xyz) / (goal.w * 1.45);
                let facing = max(dot(n, normalize(a - goal.xyz)), 0.0);
                colour += faction_shield(ship_faction(aim)) * pow(facing, 4.0) * sq(1.0 - age) * 0.9;
            }
        }
    }
    // Fighters weaving round the wedge fleet, bolts flicking between them.
    if (i32(ev_obj(1).x) == 1 || i32(ev_obj(1).y) == 1) {
        let o = ev_obj(0);
        for (var k = 0; k < 10; k++) {
            let fk = f32(k);
            let h = hash_cell(vec3<f32>(fk, seed, 21.0));
            let a = t * (0.5 + 0.4 * h) + h * TAU;
            let p = o.xyz + vec3<f32>(cos(a) * o.w * (0.3 + 0.5 * h), sin(a * 1.7 + fk) * o.w * 0.25, sin(a) * o.w * 0.5 * (0.4 + 0.6 * fract(h * 7.0)));
            let vel = normalize(vec3<f32>(-sin(a) * o.w, cos(a * 1.7 + fk) * 1.7 * o.w * 0.25, cos(a) * o.w * 0.5));
            let tint = select(vec3<f32>(1.0, 0.5, 0.3), vec3<f32>(0.6, 0.85, 1.0), k < 5);
            colour += tint * glint(point_pixels(ship, d, p), 1.0) * 1.2;
            colour += tint * bolt(ship, d, p - vel * 8.0, p, 0.6) * 0.25;
            let shot_on = step(fract(t * 2.0 + h * 5.0), 0.15);
            colour += select(vec3<f32>(0.3, 1.0, 0.3), vec3<f32>(1.0, 0.25, 0.2), k < 5) * bolt(ship, d, p + vel * 2.0, p + vel * 30.0, 0.8) * shot_on;
        }
    }
    // The ship lost: a flash, a fireball and a ring of shock in its plane.
    let age = (t - kill) / 5.0;
    colour += burst(ship, d, blast, 30.0, age) * 1.4;
    colour += vec3<f32>(1.0, 0.95, 0.9) * glint(point_pixels(ship, d, blast), 6.0) * exp(-max(t - kill, 0.0) * 3.0) * step(kill, t) * 3.0;
    if (age > 0.0 && age < 1.0) {
        let denom = dot(d, vec3<f32>(0.0, 1.0, 0.0));
        if (abs(denom) > 1e-3) {
            let tp = (blast.y - ship.y) / denom;
            if (tp > 0.0 && tp < reach) {
                let q = ship + d * tp - blast;
                let ring = bump((length(q) - wave) / (4.0 + wave * 0.05));
                colour += vec3<f32>(0.7, 0.85, 1.0) * ring * sq(1.0 - age) * 0.7;
            }
        }
    }
    // Stray fire: a beam past the ship, a torpedo bursting close by.
    let laser_at = ev_obj(2).x;
    if (t > laser_at && t < laser_at + 0.6) {
        let a = battle_ship(4, t)[0].xyz;
        let near = ship + vec3<f32>(ev_obj(1).w * 14.0, 6.0, 28.0);
        let beam = bolt(ship, d, a, near + (near - a) * 0.5, 1.8) * sin((t - laser_at) / 0.6 * PI);
        colour += mix(faction_fire(ship_faction(4)), vec3<f32>(1.0), 0.5) * beam * 2.0;
    }
    let torpedo_at = ev_obj(2).y;
    if (t > torpedo_at && t < torpedo_at + 3.5) {
        let a = battle_ship(0, torpedo_at)[0].xyz;
        let near = ship + vec3<f32>(-ev_obj(1).w * 22.0, -8.0, 40.0);
        let flight = (t - torpedo_at) / 1.1;
        if (flight < 1.0) {
            let p = mix(a, near, flight);
            colour += vec3<f32>(1.0, 0.4, 0.2) * glint(point_pixels(ship, d, p), 2.4) * 2.0;
        } else {
            colour += burst(ship, d, near, 22.0, (t - torpedo_at - 1.1) / 2.4) * 1.2;
        }
    }
    return vec4<f32>(colour, alpha);
}

// ---------------------------------------------------------------------------
// A plasma storm: a vast storm cell seen from its edge, marched in a few
// widening steps through a disc of gas whose noise is turned round the eye,
// faster near it, so it spirals; its eyewall bright, lit from inside by
// lightning.
// ---------------------------------------------------------------------------

fn ev_storm(d: vec3<f32>, s: vec2<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let eye = o.xyz;
    let radius = o.w;
    let axis = normalize(params.ev[8].xyz);
    let tint = mix(vec3<f32>(0.25, 0.4, 1.0), mix(vec3<f32>(1.0, 0.45, 0.15), vec3<f32>(0.35, 1.0, 0.55), step(0.66, params.ev[9].x)), step(0.33, params.ev[9].x));
    let hot = mix(tint, vec3<f32>(1.0), 0.55);
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
    if (t1 <= t0) {
        return vec4<f32>(0.0);
    }
    var through = 1.0;
    var light = vec3<f32>(0.0);
    let n = 12;
    // Each pixel's steps offset by its own amount, so the steps blur into
    // grain rather than banding.
    let dither = hash_cell(vec3<f32>(floor(s * params.frame.x), 7.0));
    for (var i = 0; i < n; i++) {
        let f = (f32(i) + dither) / f32(n);
        let tt = mix(t0, t1, f * f);
        let span = (t1 - t0) * (2.0 * f) / f32(n);
        let p = ship + d * tt - eye;
        let x = dot(p, e1);
        let y = dot(p, e2);
        let h = dot(p, axis);
        let r = length(vec2<f32>(x, y)) / radius;
        // Spiralling: the angle wound on by the distance from the eye and
        // turning with time, faster inside.
        let a = atan2(y, x) + turn_way * (t * 0.05 / (r + 0.15) + log(r + 0.05) * 1.6);
        let q = vec3<f32>(cos(a) * r * 3.0, sin(a) * r * 3.0, h / radius * 4.0) + vec3<f32>(seed);
        let cloud = noise(q) * 0.65 + noise(q * 2.6 + 5.0) * 0.35;
        let profile = smoothstep01(0.08, 0.2, r) * (1.0 - smoothstep01(0.7, 1.05, r)) * (1.0 - smoothstep01(0.4, 1.0, abs(h) / thick));
        let wall = bump((r - 0.22) / 0.06);
        let gas = smoothstep01(0.0, 0.55, cloud + 0.25 * bump((r - 0.22) / 0.1)) * profile;
        let flash = exp(-length(p + eye - strike.xyz) / 90.0) * strike.w * 3.0;
        // Darker, denser bands in the arms; hot near the wall.
        let shade = mix(0.35, 1.0, smoothstep01(0.1, 0.8, noise(q * 5.0 + 9.0) * 0.5 + 0.5));
        let glow = tint * gas * 0.0045 * shade + hot * wall * gas * 0.03 + vec3<f32>(0.9, 0.9, 1.0) * flash * gas * 0.02;
        light += through * glow * span;
        through *= exp(-gas * 0.006 * span);
    }
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
// ---------------------------------------------------------------------------

fn ev_dyson(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let star = o.xyz;
    let radius = o.w;
    let seed = params.ev[9].x;
    let t = ev_t();
    let warm = vec3<f32>(1.0, 0.72, 0.42);
    // The star, its glow, dimmed by what covers it.
    let px = point_pixels(ship, d, star);
    let disc = star_disc(ship, d, star, radius * 0.11);
    var colour = vec3<f32>(0.0);
    var cover = 0.0;
    var front = vec3<f32>(0.0);
    var front_cover = 0.0;
    var back = vec3<f32>(0.0);
    for (var shell = 0; shell < 3; shell++) {
        let fs = f32(shell);
        let rr = radius * (0.7 + 0.2 * fs);
        let hits = sphere_hits(ship, d, star, rr);
        for (var side = 0; side < 2; side++) {
            let th = select(hits.y, hits.x, side == 0);
            if (th <= 0.0) {
                continue;
            }
            let n = normalize(ship + d * th - star);
            // Rings: each shell has two, about axes of their own.
            for (var k = 0; k < 2; k++) {
                let fk = f32(k) + fs * 2.0;
                let axis = normalize(vec3<f32>(hash_cell(vec3<f32>(fk, seed, 1.0)) - 0.5, 1.0, hash_cell(vec3<f32>(fk, seed, 2.0)) - 0.5) + vec3<f32>(0.0, 0.0, 0.6 * (hash_cell(vec3<f32>(fk, seed, 3.0)) - 0.5)));
                let lat = dot(n, axis);
                let width = 0.13 + 0.05 * hash_cell(vec3<f32>(fk, seed, 4.0));
                if (abs(lat) > width) {
                    continue;
                }
                var e1 = normalize(cross(axis, vec3<f32>(0.0, 0.0, 1.0)));
                let e2 = cross(axis, e1);
                let lon = atan2(dot(n, e2), dot(n, e1)) + t * (0.02 + 0.02 * fs) * select(1.0, -1.0, k == 1);
                let cells = 90.0 + 30.0 * fs;
                let g = lon / TAU * cells;
                let id = floor(g);
                let fx = fract(g) - 0.5;
                let rows = 3.0;
                let gy = (lat / width * 0.5 + 0.5) * rows;
                let fy = fract(gy) - 0.5;
                let present = step(0.15, hash_cell(vec3<f32>(id, floor(gy), fk + seed)));
                let panel = present * step(abs(fx), 0.38) * step(abs(fy), 0.36);
                if (panel <= 0.0) {
                    continue;
                }
                let edge = smoothstep01(0.3, 0.38, max(abs(fx), abs(fy) * 1.05));
                if (side == 0 && dot(n, -d) > 0.0) {
                    // The near side: the panels' unlit backs, a glint of
                    // the star on their rims.
                    front += vec3<f32>(0.012, 0.012, 0.014) + warm * edge * 0.25 * exp(-px * 0.004);
                    front_cover = max(front_cover, panel);
                } else {
                    // The far side: their faces, lit.
                    let facing = max(dot(-n, normalize(ship - star + d * 0.0)), 0.0);
                    back += warm * (0.35 + 0.65 * facing) * (0.6 + 0.4 * hash_cell(vec3<f32>(id, fk, 9.0))) * 0.8;
                    cover = max(cover, panel * 0.9);
                }
            }
        }
    }
    let glow = warm * (exp(-px * 0.02) * 0.5 + exp(-px * 0.004) * 0.12);
    colour = disc * (1.0 - cover * 0.6) + glow * (1.0 - cover * 0.4);
    colour = mix(colour, back, cover);
    colour = mix(colour, front, front_cover);
    let a = max(max(cover, front_cover), max(disc.r, 0.0) * 0.0);
    return vec4<f32>(colour, a);
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

// ---------------------------------------------------------------------------
// A crystal entity: a vast lattice folded from a few shapes by mirror
// symmetry, marched; the stars behind bent through it in rainbows (the sky
// read three times, once a colour, each bent its own amount), its faces
// reflecting the sky, its heart pulsing brighter as it notices the ship.
// ---------------------------------------------------------------------------

// A shard: a long bipyramid along unit `dir`, `length_` out from the
// middle, `width` across at its waist, pointed at its tip.
fn shard(p: vec3<f32>, dir: vec3<f32>, length_: f32, width: f32) -> f32 {
    let along = dot(p, dir);
    let rad = length(p - dir * along);
    let waist = length_ * 0.3;
    let taper = select(width * (1.0 - (along - waist) / (length_ - waist)), width * (0.5 + 0.5 * along / waist), along < waist);
    return max(rad - max(taper, 0.0), max(-along, along - length_)) * 0.7;
}

// The crystal: folded into one twenty-fourth of itself by its mirror
// symmetry, three families of shards there (to the faces, edges and corners
// of a cube: 26 in all), each with a smaller pair branching off it.
fn crystal_sdf(p0: vec3<f32>) -> f32 {
    var p = abs(p0);
    if (p.x < p.y) {
        p = p.yxz;
    }
    if (p.y < p.z) {
        p = p.xzy;
    }
    if (p.x < p.y) {
        p = p.yxz;
    }
    var d = shard(p, vec3<f32>(1.0, 0.0, 0.0), 1.0, 0.11);
    d = min(d, shard(p, normalize(vec3<f32>(1.0, 1.0, 0.0)), 0.72, 0.08));
    d = min(d, shard(p, normalize(vec3<f32>(1.0, 1.0, 1.0)), 0.85, 0.09));
    // Branches off the longest shards, two thirds of the way out.
    let b = p - vec3<f32>(0.6, 0.0, 0.0);
    d = min(d, shard(b, normalize(vec3<f32>(0.6, 1.0, 0.0)), 0.3, 0.04));
    d = min(d, shard(b, normalize(vec3<f32>(0.6, 0.0, 1.0)), 0.3, 0.04));
    return min(d, length(p0) - 0.16);
}

fn ev_crystal(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let o = ev_obj(0);
    let centre = o.xyz;
    let size = o.w;
    let axis = normalize(params.ev[8].xyz);
    let angle = params.ev[8].w;
    let notice = ev_obj(1).x;
    // Icy: cyan, violet or green-white.
    let tint = mix(vec3<f32>(0.55, 0.9, 1.0), mix(vec3<f32>(0.8, 0.65, 1.0), vec3<f32>(0.65, 1.0, 0.8), step(0.5, params.ev[9].x)), step(0.33, params.ev[9].x));
    let t = ev_t();
    let bound = sphere_hits(ship, d, centre, size * 1.05);
    if (bound.y <= 0.0) {
        // Its light round it.
        let px = point_pixels(ship, d, centre);
        return vec4<f32>(tint * exp(-px * 0.01) * (0.04 + 0.12 * notice), 0.0);
    }
    let lo = turn((ship - centre) / size, axis, -angle);
    let ld = turn(d, axis, -angle);
    var tm = max(bound.x, 0.0) / size;
    let end = bound.y / size;
    var hit = false;
    var closest = 1e9;
    for (var i = 0; i < 56; i++) {
        let dist = crystal_sdf(lo + ld * tm);
        closest = min(closest, dist);
        if (dist < 0.0015 + tm * params.frame.z * 0.4) {
            hit = true;
            break;
        }
        tm += max(dist * 0.8, 0.003);
        if (tm > end) {
            break;
        }
    }
    let px = point_pixels(ship, d, centre);
    let heart = tint * (exp(-px * 0.03) * (0.25 + 1.2 * notice) * (0.8 + 0.2 * sin(t * 3.0)));
    if (!hit) {
        // Close misses glow faintly, the light caught in the lattice; and
        // the glow round it, as outside its bounds.
        return vec4<f32>(tint * exp(-px * 0.01) * (0.04 + 0.12 * notice) + heart * 0.4 + tint * exp(-closest * 40.0) * 0.05, 0.0);
    }
    let q = lo + ld * tm;
    let e = 0.003;
    let nl = normalize(vec3<f32>(
        crystal_sdf(q + vec3<f32>(e, 0.0, 0.0)) - crystal_sdf(q - vec3<f32>(e, 0.0, 0.0)),
        crystal_sdf(q + vec3<f32>(0.0, e, 0.0)) - crystal_sdf(q - vec3<f32>(0.0, e, 0.0)),
        crystal_sdf(q + vec3<f32>(0.0, 0.0, e)) - crystal_sdf(q - vec3<f32>(0.0, 0.0, e)),
    ));
    let n = turn(nl, axis, angle);
    let mu = max(dot(n, -d), 0.0);
    let fresnel = 0.06 + 0.94 * pow(1.0 - mu, 5.0);
    // Through it, each colour bent its own amount: a rainbow at every edge.
    let rr = ev_sky(normalize(refract(d, n, 0.84))).r;
    let gg = ev_sky(normalize(refract(d, n, 0.8))).g;
    let bb = ev_sky(normalize(refract(d, n, 0.76))).b;
    let through = vec3<f32>(rr, gg, bb) * 2.5 * tint + tint * 0.05;
    let mirror = ev_sky(reflect(d, n)) * 2.0 + tint * 0.04;
    // Light running through it from its heart, brighter as it notices.
    let inner = tint * (0.04 + 0.5 * notice * (0.6 + 0.4 * sin(t * 4.0 - length(q) * 12.0))) * (1.0 - smoothstep01(0.0, 1.0, length(q)));
    let edge = pow(1.0 - mu, 3.0) * tint * 0.45;
    // Facets catching the faint light of the galaxy above.
    let facet = pow(max(dot(n, normalize(vec3<f32>(0.3, 1.0, 0.2))), 0.0), 12.0) * tint * 0.4;
    let colour = mix(through, mirror, fresnel) + inner + edge + facet + heart * 0.3;
    return vec4<f32>(colour, 0.85);
}

// Each event's draw over the sky.
@fragment
fn fs_event(in: Full) -> @location(0) vec4<f32> {
    let d = to_ev(normalize(in.dir));
    let s = half_frame(in.position);
    var c = vec4<f32>(0.0);
    if (EVENT == 1) {
        c = ev_asteroids(d);
    } else if (EVENT == 2) {
        c = ev_cube(d);
    } else if (EVENT == 4) {
        c = ev_nebula(d, s);
    } else if (EVENT == 5) {
        c = ev_rogue(d);
    } else if (EVENT == 6) {
        c = ev_pulsar(d);
    } else if (EVENT == 7) {
        c = ev_derelict(d);
    } else if (EVENT == 8) {
        c = ev_comet(d);
    } else if (EVENT == 9) {
        c = ev_leviathans(d);
    } else if (EVENT == 10) {
        c = ev_remnant(d);
    } else if (EVENT == 11) {
        c = ev_gate(d);
    } else if (EVENT == 12) {
        c = ev_wormhole(d);
    } else if (EVENT == 13) {
        c = ev_battle(d);
    } else if (EVENT == 14) {
        c = ev_storm(d, s);
    } else if (EVENT == 15) {
        c = ev_dyson(d);
    } else if (EVENT == 16) {
        c = ev_crystal(d);
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

@fragment
fn fs_overlay(in: Full) -> @location(0) vec4<f32> {
    let s = half_frame(in.position);
    let r = length(s);
    let alert = params.ev[10].x;
    let hit = params.ev[2].w;
    let beam = params.ev[3].w;
    let flash = params.ev[4].w;
    let t = params.burn.y;
    var colour = vec3<f32>(0.0);
    // The alarm round the round frame's rim.
    if (alert > 0.3) {
        let red = smoothstep01(0.6, 0.8, alert);
        let rate = mix(2.2, 5.5, red);
        let beat = 0.55 + 0.45 * sin(t * rate);
        let tint = mix(vec3<f32>(1.0, 0.55, 0.1), vec3<f32>(1.0, 0.1, 0.06), red);
        let band = smoothstep01(0.86, 0.985, r) * (1.0 - smoothstep01(0.99, 1.01, r));
        colour += tint * band * beat * smoothstep01(0.3, 0.55, alert) * 0.18;
    }
    // A hit: a ring of hexagons rippling out from it.
    if (hit > 0.01) {
        let at = params.ev[11].xy;
        let dist = length(s - at);
        let front = (1.0 - hit) * 0.8 + 0.06;
        let ring = bump((dist - front) / 0.05) * (1.0 - smoothstep01(0.97, 1.0, r));
        let g = s * 12.0;
        let row = floor(g.y / 0.866);
        let shifted = vec2<f32>(g.x + 0.5 * (row % 2.0), g.y / 0.866);
        let cell = fract(shifted) - 0.5;
        let hex = smoothstep01(0.36, 0.48, max(abs(cell.x), abs(cell.y) * 0.9 + abs(cell.x) * 0.35));
        colour += vec3<f32>(0.35, 0.75, 1.0) * ring * (0.2 + 0.8 * hex) * hit * 0.55;
        colour += vec3<f32>(0.6, 0.85, 1.0) * exp(-dist * 14.0) * hit * 0.2;
    }
    // The tractor beam's hold: its colour washing in, scan lines rolling.
    if (beam > 0.01 && EVENT == 0) {
        let tone = hue_of(params.ev[9].x);
        let lines = 0.5 + 0.5 * sin(in.position.y * 0.5 - t * 20.0);
        colour += tone * beam * (0.015 + 0.015 * lines) * smoothstep01(0.4, 0.95, r) * (1.0 - smoothstep01(0.99, 1.01, r));
    }
    // Squared: blended in linear light, a little white reads as a lot.
    let white = clamp(flash * flash, 0.0, 1.0);
    return vec4<f32>(colour * (1.0 - white) + vec3<f32>(1.0) * white, white);
}
