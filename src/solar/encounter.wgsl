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
            if (hash_cell(vec3<f32>(fi, 5.0, 3.0)) > params.ev[9].y) {
                radius = 0.0;
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
        let speed = mix(3.0, 16.0, h.z);
        let age = max(t - kill, 0.0);
        p = wreck[0].xyz + dir * speed * age + wreck[1].xyz * age * 4.0;
        radius = select(0.0, mix(0.25, 2.2, h.w * h.w * h.w), t > kill && instance < 90u);
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
    if (radius <= 0.0 || cam.z < radius * 1.3 + 0.5 || cam.z > 4000.0) {
        out.position = vec4<f32>(2.0, 2.0, 2.0, 1.0);
        return out;
    }
    let tan_half = params.frame.y;
    let centre = cam.xy / (cam.z * tan_half);
    // Out to the sphere's silhouette, and a few pixels more for the glow.
    let spread = radius * 1.2 / (sqrt(cam.z * cam.z - radius * radius) * tan_half) + 4.0 / params.frame.x;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(-1.0, 1.0),
        vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    );
    let corner = corners[vertex % 6u];
    out.position = vec4<f32>(centre + corner * spread, clamp(cam.z / (cam.z + 400.0), 0.0, 0.999), 1.0);
    return out;
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
        // A mote of ice: a soft point.
        let across = length(c - d * tca);
        let g = exp(-sq(across / max(r, pixel * 1.2)));
        return vec4<f32>(vec3<f32>(0.6, 0.85, 1.0) * g * in.look.z, 0.0);
    }
    let d2 = dot(c, c) - tca * tca;
    // A lumpy silhouette: the radius out along the way to the closest point.
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
    // Bumps and craters from the noise's slope, fixed to the rock as it
    // tumbles.
    let spin = seed * 0.37 + ev_t() * (0.05 + 0.1 * fract(seed * 0.618));
    let q = turn(n, normalize(vec3<f32>(fract(seed * 0.31) - 0.5, 1.0, fract(seed * 0.71) - 0.5)), spin) * 2.4 + vec3<f32>(seed * 0.13);
    let g = noised(q * 1.6);
    let g2 = noised(q * 4.4 + 7.0);
    if (kind > 2.5) {
        // A ship's debris: flat plates and torn edges.
        n = normalize(floor(n * 2.2 + 0.5) + n * 0.15);
    } else {
        n = normalize(n - (g.yzw * 0.45 + g2.yzw * 0.2) * 0.5);
    }
    // The light, from the encounter frame into the camera's.
    var lw = from_ev(normalize(vec3<f32>(0.55, 0.6, -0.45)));
    if (EVENT == 1) {
        lw = from_ev(ev_light());
    }
    let l = vec3<f32>(dot(lw, params.right.xyz), dot(lw, params.up.xyz), dot(lw, params.forward.xyz));
    let diffuse = max(dot(n, l), 0.0);
    let crater = smoothstep01(0.35, 0.55, abs(g2.x)) * 0.4 + 0.6;
    var albedo = vec3<f32>(0.13, 0.11, 0.095);
    var shine = 0.0;
    if (kind > 2.5) {
        albedo = mix(vec3<f32>(0.42, 0.44, 0.47), vec3<f32>(0.5, 0.36, 0.25), step(0.6, fract(seed * 0.17)));
        shine = 0.6;
    } else if (kind > 1.5) {
        albedo = vec3<f32>(0.34, 0.2, 0.13);
        shine = 0.35;
    } else if (kind > 0.5) {
        albedo = vec3<f32>(0.62, 0.7, 0.78);
        shine = 0.9;
    }
    albedo *= crater * (0.8 + 0.4 * g.x);
    let spec = pow(max(dot(reflect(-l, n), -d), 0.0), 28.0) * shine;
    var colour = albedo * (diffuse * 1.25 + 0.025) + vec3<f32>(spec);
    // Light off the far side's edge, and the faint blue of the sky.
    colour += vec3<f32>(0.04, 0.06, 0.1) * pow(1.0 - max(dot(n, -d), 0.0), 3.0);
    if (kind > 2.5 && fract(seed * 0.0731) > 0.86) {
        // A smouldering edge.
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
// A battle between two fleets, watched from its edge: eight ships circling
// each other, lasers flicking between them, torpedoes arcing over with their
// trails and bursting on the shields, one ship lost in a fireball and a
// ring of shock, its wreckage spreading (`vs_rock`); stray fire passing the
// ship close.
// ---------------------------------------------------------------------------

// Ship `i` of the battle at `t` (0-3 one fleet, 4-7 the other): where it
// is, its size, and which way it heads.
fn battle_ship(i: i32, t: f32) -> array<vec4<f32>, 2> {
    let o = ev_obj(0);
    let seed = params.ev[9].x;
    let fi = f32(i);
    let fleet = select(1.0, -1.0, i >= 4);
    let h = vec4<f32>(
        hash_cell(vec3<f32>(fi, seed, 1.0)),
        hash_cell(vec3<f32>(fi, seed, 2.0)),
        hash_cell(vec3<f32>(fi, seed, 3.0)),
        hash_cell(vec3<f32>(fi, seed, 4.0)),
    );
    // The two fleets wheel round each other, each on its own side.
    let rate = (0.06 + 0.05 * h.x) * fleet;
    let a = t * rate + h.y * TAU + select(0.0, PI, i >= 4);
    let r = o.w * (0.35 + 0.55 * h.z);
    let y = (h.w - 0.5) * o.w * 0.45 + sin(t * 0.4 + fi) * 6.0;
    let p = o.xyz + vec3<f32>(cos(a) * r, y, sin(a) * r * 0.65);
    let heading = normalize(vec3<f32>(-sin(a) * fleet, 0.05 * cos(t * 0.4 + fi), cos(a) * 0.65 * fleet));
    // A capital ship each, the rest escorts.
    let size = select(7.0 + 5.0 * h.x, 22.0, i == 0 || i == 4);
    return array<vec4<f32>, 2>(vec4<f32>(p, size), vec4<f32>(heading, h.y));
}

fn fleet_colour(i: i32) -> vec3<f32> {
    return hue_of(select(ev_obj(1).x, ev_obj(1).y, i >= 4));
}

// Which ship was lost, and when.
fn battle_lost() -> i32 {
    return 1 + i32(hash_cell(vec3<f32>(params.ev[9].x, 5.0, 5.0)) * 2.99) + select(0, 4, hash_cell(vec3<f32>(params.ev[9].x, 6.0, 6.0)) > 0.5);
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
    let core = exp(-sq(across / (size * 0.55))) * fade;
    let shell = exp(-sq((across - size) / (size * 0.18))) * fade * 0.35;
    let hot = mix(vec3<f32>(1.0, 0.95, 0.85), vec3<f32>(1.0, 0.45, 0.1), smoothstep01(0.0, 0.5, age));
    return hot * (core * 2.2 + shell) + vec3<f32>(1.0, 0.6, 0.3) * exp(-across / r) * fade * 0.04;
}

fn ev_battle(d: vec3<f32>) -> vec4<f32> {
    let ship = ev_ship();
    let t = ev_t();
    let seed = params.ev[9].x;
    let lost = battle_lost();
    let kill = ev_obj(1).z;
    let l = normalize(vec3<f32>(0.5, 0.7, -0.35));
    var colour = vec3<f32>(0.0);
    var alpha = 0.0;
    var reach = 1e9;
    // The ships: each a long hull traced as an ellipsoid, banded, lit, its
    // engines glowing astern.
    for (var i = 0; i < 8; i++) {
        if (i == lost && t > kill) {
            continue;
        }
        let s = battle_ship(i, t);
        let c = s[0].xyz;
        let size = s[0].w;
        let fwd = s[1].xyz;
        let side = normalize(cross(fwd, vec3<f32>(0.0, 1.0, 0.0)));
        let up = cross(side, fwd);
        let shape = vec3<f32>(size * 0.3, size * 0.16, size);
        let ro = ship - c;
        let lo = vec3<f32>(dot(ro, side), dot(ro, up), dot(ro, fwd)) / shape;
        let ld = vec3<f32>(dot(d, side), dot(d, up), dot(d, fwd)) / shape;
        let hits = sphere_hits(lo, normalize(ld), vec3<f32>(0.0), 1.0);
        if (hits.x > 0.0) {
            let tl = hits.x / length(ld);
            if (tl < reach) {
                reach = tl;
                let q = lo + ld * tl;
                let n = normalize(side * q.x / shape.x + up * q.y / shape.y + fwd * q.z / shape.z);
                let bands = 0.75 + 0.25 * step(0.5, fract(q.z * 3.0 + q.x * 0.5));
                let tint = mix(vec3<f32>(0.3, 0.3, 0.32), fleet_colour(i) * 0.4 + vec3<f32>(0.15), 0.35);
                colour = tint * bands * (max(dot(n, l), 0.0) * 1.1 + 0.05);
                colour += vec3<f32>(pow(max(dot(reflect(-l, n), -d), 0.0), 24.0) * 0.5);
                // Windows and running lights in rows down the hull.
                let row = step(0.92, fract(q.z * 9.0)) * step(abs(q.y), 0.3);
                colour += fleet_colour(i) * row * 0.8;
                alpha = 1.0;
            }
        }
        // Engines.
        let engine = point_pixels(ship, d, c - fwd * size * 1.02);
        colour += mix(fleet_colour(i), vec3<f32>(1.0), 0.5) * glint(engine, 1.2 + size * 0.04) * 1.2 * (1.0 - alpha * 0.8);
    }
    // Lasers: short bursts between the fleets, now and then missing.
    for (var k = 0; k < 6; k++) {
        let fk = f32(k);
        let clock_ = t * 1.7 + fk * 0.37;
        let bin = floor(clock_);
        let on = fract(clock_);
        if (on > 0.3) {
            continue;
        }
        let h = hash_cell(vec3<f32>(bin, fk, seed));
        let from_i = i32(h * 7.99);
        var to_i = (from_i + 4 + i32(fract(h * 13.0) * 3.99)) % 8;
        if ((from_i == lost || to_i == lost) && t > kill) {
            continue;
        }
        let a = battle_ship(from_i, t)[0].xyz;
        let b = battle_ship(to_i, t)[0];
        let miss = (vec3<f32>(fract(h * 31.0), fract(h * 57.0), fract(h * 91.0)) - 0.5) * b.w * 3.0 * step(0.6, fract(h * 7.0));
        let beam = bolt(ship, d, a, b.xyz + miss, 1.3) * sin(on / 0.3 * PI);
        colour += mix(fleet_colour(from_i), vec3<f32>(1.0), 0.45) * beam * 1.4;
    }
    // Torpedoes: glowing slugs arcing over with their trails, bursting on
    // the target's shield.
    for (var k = 0; k < 5; k++) {
        let fk = f32(k);
        let clock_ = t / 2.6 + fk * 0.23;
        let bin = floor(clock_);
        let phase = fract(clock_);
        let h = hash_cell(vec3<f32>(bin, fk + 10.0, seed));
        let from_i = i32(h * 7.99);
        let to_i = (from_i + 4 + i32(fract(h * 17.0) * 3.99)) % 8;
        if ((from_i == lost || to_i == lost) && t > kill - 2.0) {
            continue;
        }
        let a = battle_ship(from_i, t)[0].xyz;
        let goal = battle_ship(to_i, t)[0];
        let flight = clamp(phase / 0.7, 0.0, 1.0);
        let arc = vec3<f32>(0.0, 1.0, 0.0) * sin(flight * PI) * 25.0;
        let p = mix(a, goal.xyz, flight) + arc;
        let tint = select(vec3<f32>(1.0, 0.35, 0.2), vec3<f32>(0.4, 0.75, 1.0), from_i >= 4);
        if (phase < 0.7) {
            let pulse_ = 0.75 + 0.25 * sin(t * 30.0 + fk);
            colour += tint * glint(point_pixels(ship, d, p), 2.0) * 1.6 * pulse_;
            let back = mix(a, goal.xyz, max(flight - 0.12, 0.0)) + vec3<f32>(0.0, 1.0, 0.0) * sin(max(flight - 0.12, 0.0) * PI) * 25.0;
            colour += tint * bolt(ship, d, back, p, 1.0) * 0.35;
        } else {
            let age = (phase - 0.7) / 0.3;
            colour += burst(ship, d, goal.xyz + normalize(a - goal.xyz) * goal.w * 1.5, goal.w * 1.2, age) * 0.6;
            // The shield flaring round the goal.
            let sh = sphere_hits(ship, d, goal.xyz, goal.w * 1.6);
            if (sh.x > 0.0) {
                let v = goal.xyz - ship;
                let edge = length(v - d * dot(v, d)) / (goal.w * 1.6);
                let shield = select(vec3<f32>(0.5, 0.85, 1.0), vec3<f32>(1.0, 0.75, 0.35), to_i >= 4);
                colour += shield * pow(edge, 8.0) * 0.7 * sq(1.0 - age);
            }
        }
    }
    // The ship lost: a fireball, a ring of shock in its plane.
    let wreck = battle_ship(lost, kill);
    let age = (t - kill) / 5.0;
    colour += burst(ship, d, wreck[0].xyz, wreck[0].w * 3.0, age) * 1.6;
    if (age > 0.0 && age < 1.0) {
        let denom = dot(d, vec3<f32>(0.0, 1.0, 0.0));
        if (abs(denom) > 1e-3) {
            let tp = (wreck[0].y - ship.y) / denom;
            if (tp > 0.0) {
                let q = ship + d * tp - wreck[0].xyz;
                let ring = bump((length(q) - age * wreck[0].w * 14.0) / (wreck[0].w * 0.8));
                colour += vec3<f32>(0.7, 0.85, 1.0) * ring * sq(1.0 - age) * 0.8;
            }
        }
    }
    // Stray fire: a laser past the ship, a torpedo bursting close by.
    let laser_at = ev_obj(2).x;
    if (t > laser_at && t < laser_at + 0.6) {
        let a = battle_ship(4, t)[0].xyz;
        let near = ship + vec3<f32>(ev_obj(1).w * 14.0, 6.0, 28.0);
        let beam = bolt(ship, d, a, near + (near - a) * 0.5, 1.8) * sin((t - laser_at) / 0.6 * PI);
        colour += mix(fleet_colour(4), vec3<f32>(1.0), 0.5) * beam * 2.0;
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
    return vec4<f32>(colour * select(1.0, 1.0, alpha > 0.0), alpha);
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
        colour += tint * band * beat * smoothstep01(0.3, 0.55, alert) * 0.25;
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
