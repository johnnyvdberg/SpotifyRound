// The observatory (`src/observatory.rs`): the view through a telescope's
// eyepiece, one target at a time. Field units: the field stop at radius 1.

struct Params {
    // Centre x, y and field-stop radius in pixels; seconds.
    frame: vec4<f32>,
    // Where the telescope points, off the target, in field units; how much the
    // air shimmers; the defocus, as a blur radius in field units.
    view: vec4<f32>,
    // Kind (0 moon, 1 planet, 2 nebula or galaxy, 3 star cluster), its variant,
    // how much of the view shows (0 dark through a slew, 1 settled), and the
    // field stars' seed.
    look: vec4<f32>,
    // The slew's streak (half-vector, field units), the field stars' density
    // (0..1), and one pixel in field units.
    slew: vec4<f32>,
    // Per kind. A galaxy's: bulge colour and dust lane, disc colour and bulge
    // radius.
    colour_a: vec4<f32>,
    colour_b: vec4<f32>,
    // Per kind: size, tilt, ring tilt or inclination, detail.
    shape: vec4<f32>,
    // Per kind: the Moon's or a planet's sun (none: no phase) and where
    // Jupiter's Red Spot is, or a galaxy's companion.
    extra: vec4<f32>,
    // Something crossing the field (in screen field units, apart from where
    // the telescope points): where it is and its velocity a second; its kind
    // (0 none, 1 satellite, 2 meteor, 3 aircraft), brightness, and seconds
    // since it appeared.
    passer: vec4<f32>,
    passer_look: vec4<f32>,
    // The night: how much cloud, its drift, and the light it catches.
    weather: vec4<f32>,
    // The target's own bright stars: x, y, brightness, colour (0 blue, 1 gold).
    stars: array<vec4<f32>, 12>,
};

@group(0) @binding(0) var<uniform> u: Params;

const SKY: vec3<f32> = vec3<f32>(0.0016, 0.0019, 0.0030);
// A star's seeing disc: about 1.6 pixels at best focus.
const SEEING_DISC: f32 = 0.0032;

fn star_colour(t: f32) -> vec3<f32> {
    return mix(vec3<f32>(0.62, 0.76, 1.0), vec3<f32>(1.0, 0.74, 0.42), t);
}

// One point of light `d` away: its seeing disc, drawn out along the slew.
fn point(d: vec2<f32>, size: f32) -> f32 {
    let s = u.slew.xy;
    let h = clamp(dot(d, s) / max(dot(s, s), 1e-9), -1.0, 1.0);
    let q = (d - s * h) / size;
    return exp(-dot(q, q)) / (1.0 + length(s) / size);
}

// Stars on a grid of `scale` cells across the field's radius, at most one a
// cell and kept to its middle so its glow never reaches the next. `keep` is
// the share of cells with a star.
fn star_field(p: vec2<f32>, scale: f32, seed: f32, keep: f32, size: f32) -> vec3<f32> {
    let g = p * scale + seed;
    let cell = floor(g);
    if (hash21(cell + 0.5) > keep) {
        return vec3<f32>(0.0);
    }
    let at = (cell + 0.25 + 0.5 * hash22(cell) - seed) / scale;
    let m = hash21(cell + 7.1);
    let brightness = 0.035 + 0.6 * pow(m, 12.0);
    return star_colour(hash21(cell + 3.3)) * brightness * point(p - at, size);
}

// The Moon near its terminator. Craters are laid out on the sphere (so they
// foreshorten toward the limb), in five sizes, many small and few large, each
// worn to its own depth; their walls tilt the light and their rims cast
// shadows, long near the terminator. The maria flooded most of them.
fn moon(p: vec2<f32>, edge: f32) -> vec3<f32> {
    let q = p / u.shape.x;
    let d2 = dot(q, q);
    if (d2 > 1.0) {
        return SKY;
    }
    let z = sqrt(1.0 - d2);
    let up = vec3<f32>(q, z);
    // Where on the globe, and its east and north there.
    let lon = atan2(q.x, z);
    let lat = asin(clamp(q.y, -1.0, 1.0));
    let east = vec3<f32>(cos(lon), 0.0, -sin(lon));
    let north = vec3<f32>(-sin(lat) * sin(lon), cos(lat), -sin(lat) * cos(lon));
    let uv = vec2<f32>(lon * cos(lat), lat);
    // The sun over this spot: its bearing along the ground and its height.
    let sun = u.extra.xyz;
    let ground = vec2<f32>(dot(sun, east), dot(sun, north));
    let bearing = ground / max(length(ground), 1e-4);
    let height = dot(sun, up) / max(length(ground), 1e-3);

    let maria = smoothstep(0.42, 0.6, fbm(uv * 2.2 + 5.0));
    var albedo = mix(0.3, 0.11, maria) * (0.82 + 0.36 * fbm(uv * 30.0));
    var slope = vec2<f32>(0.0);
    var lit = 1.0;
    for (var k = 0; k < 5; k += 1) {
        let scale = 4.0 * pow(2.2, f32(k));
        let g = uv * scale + f32(k) * 17.3;
        let cell = floor(g);
        let keep = (0.45 + 0.08 * f32(k)) * (1.0 - (0.9 - 0.15 * f32(k)) * maria);
        if (hash21(cell + 9.7) > keep) {
            continue;
        }
        let roll = hash21(cell + 4.1);
        // Kept, with its rim's outer slope, inside its cell, so nothing is
        // cut off at the cell's edge.
        let radius = 0.1 + 0.2 * roll * roll;
        let centre = cell + 0.5 + (0.49 - 1.6 * radius) * (2.0 * hash22(cell) - 1.0);
        let v = (g - centre) / radius;
        let x = length(v);
        if (x > 1.6) {
            continue;
        }
        // A few craters are fresh, deep and sharp; most are old, worn shallow.
        let fresh = pow(hash21(cell + 2.2), 2.5);
        let depth = 0.12 + 0.3 * fresh;
        let rim = 0.25 * depth;
        let dir = v / max(x, 1e-4);
        if (x < 1.0) {
            // A bowl, with a central peak in the large ones.
            var s = 2.0 * depth * x;
            if (k == 0 && x < 0.22) {
                s -= 0.5 * depth;
            }
            slope += dir * s;
            // Shadow from the rim between this point and the sun. Heights
            // here are from the rim's crest.
            let a = dot(v, bearing);
            let b = v.x * bearing.y - v.y * bearing.x;
            let to_rim = sqrt(max(1.0 - b * b, 0.0)) - a;
            let below = (depth + rim) * (1.0 - x * x);
            lit = min(lit, smoothstep(-0.03, 0.03, to_rim * height - below));
        } else {
            let outside = exp(-pow((x - 1.0) / 0.25, 2.0)) * (1.0 - smoothstep(1.35, 1.6, x));
            slope -= dir * rim * 2.0 * (x - 1.0) / 0.0625 * outside;
            // The rim's shadow thrown on the ground behind it.
            let a = dot(v, bearing);
            let b = v.x * bearing.y - v.y * bearing.x;
            if (a < 0.0 && abs(b) < 1.0) {
                let to_rim = -a - sqrt(1.0 - b * b);
                lit = min(lit, smoothstep(-0.01, 0.02, to_rim * height - rim));
            }
            // Fresh craters lie in a bright blanket of ejecta.
            albedo *= 1.0 + 0.45 * fresh * fresh * outside;
        }
    }
    let n = normalize(up - east * slope.x - north * slope.y);
    let light = max(dot(n, sun), 0.0) * lit;
    let earthshine = 0.004;
    let shown = 1.0 - smoothstep(1.0 - edge, 1.0, sqrt(d2));
    return mix(SKY, vec3<f32>(1.0, 0.97, 0.92) * albedo * (light * 3.2 + earthshine), shown);
}

// A planet: Jupiter's belts (variant 0), Saturn and its rings (1), Mars (2),
// or a disc with hardly a feature, as Venus (3) and Uranus (4) show.
fn planet(p: vec2<f32>, edge: f32) -> vec3<f32> {
    let q = rotate(p, u.shape.y) / u.shape.x;
    let d2 = dot(q, q);
    let t = u.frame.w;
    var colour = SKY;
    // Saturn's rings, an ellipse of tilt `shape.z`: the far half behind the
    // globe, the near half over it.
    var ring = 0.0;
    var inner_edge = 0.0;
    if (u.shape.z > 0.0) {
        let e = vec2<f32>(q.x, q.y / u.shape.z);
        let r = length(e);
        inner_edge = exp(-pow((r - 1.24) / 0.03, 2.0));
        ring = smoothstep(1.22, 1.26, r) * (1.0 - smoothstep(2.22, 2.27, r));
        ring *= 0.55 + 0.45 * smoothstep(1.5, 1.7, r);
        ring *= 1.0 - 0.92 * smoothstep(1.92, 1.95, r) * (1.0 - smoothstep(1.99, 2.02, r));
        ring *= 0.85 + 0.15 * sin(r * 90.0);
        if (d2 < 1.0) {
            if (q.y > 0.0) {
                ring = 0.0;
            }
            inner_edge *= f32(q.y < 0.0);
        } else {
            inner_edge = 0.0;
        }
    }
    if (d2 < 1.0) {
        let z = sqrt(1.0 - d2);
        let lat = q.y;
        var surface = u.colour_a.rgb;
        if (u.look.y < 1.5) {
            let swirl = noise(vec2<f32>(q.x * 4.0 + t * 0.004, lat * 18.0)) - 0.5;
            let bands = sin(lat * u.shape.w + swirl * 1.6 + 0.9 * sin(lat * u.shape.w * 0.37));
            surface = mix(u.colour_a.rgb, u.colour_b.rgb, 0.5 + 0.5 * bands);
            // Jupiter's Great Red Spot.
            if (u.look.y < 0.5) {
                let spot = length((q - vec2<f32>(u.extra.w, -0.38)) * vec2<f32>(1.0, 1.9));
                surface = mix(surface, vec3<f32>(0.62, 0.28, 0.16), (1.0 - smoothstep(0.08, 0.12, spot)) * 0.7);
            }
        } else if (u.look.y < 2.5) {
            let dark = smoothstep(0.45, 0.65, fbm(q * 2.6 + 3.0));
            surface = mix(u.colour_a.rgb, u.colour_b.rgb, dark);
            surface = mix(surface, vec3<f32>(0.95, 0.93, 0.9), smoothstep(0.82, 0.86, lat));
        } else {
            surface = mix(u.colour_a.rgb, u.colour_b.rgb, noise(q * 2.0 + t * 0.002) * 0.4);
        }
        // Limb darkening, the phase where the planet has one, and the edge
        // softened by the focus.
        let lit = surface * (0.35 + 0.65 * pow(z, 0.45));
        var shown = 1.0 - smoothstep(1.0 - edge / u.shape.x, 1.0, sqrt(d2));
        // The night side is as dark as the sky behind it.
        if (dot(u.extra.xyz, u.extra.xyz) > 0.0) {
            shown *= smoothstep(-0.04, 0.1, dot(vec3<f32>(q, z), u.extra.xyz));
        }
        colour = mix(colour, lit, shown);
    }
    // The rings are brighter than the globe; where the near side crosses it,
    // its inner edge shows as a dark line.
    colour *= 1.0 - 0.6 * inner_edge;
    return mix(colour, vec3<f32>(0.95, 0.88, 0.7), ring * 0.9);
}

// Gas and dust: an emission nebula (variant 0), a galaxy (1) or a planetary
// nebula's ring (2).
fn diffuse(p: vec2<f32>) -> vec3<f32> {
    let q = rotate(p, u.shape.y) / u.shape.x;
    if (u.look.y < 0.5) {
        let warp = vec2<f32>(fbm(q * 1.7), fbm(q * 1.7 + 9.0));
        let gas = fbm(q * 2.4 + warp * 1.8);
        let core = exp(-dot(q, q) * 3.5);
        let dust = smoothstep(0.42, 0.58, fbm(q * 3.1 + 21.0));
        let density = pow(gas, 2.5) * core * (1.0 - 0.75 * dust);
        let colour = mix(u.colour_b.rgb, u.colour_a.rgb, smoothstep(0.08, 0.3, density));
        // The bright heart round the stars that light it.
        let heart = exp(-dot(p, p) / 0.012) * 0.35 * u.colour_a.rgb;
        return SKY + (colour * density * 1.6 + heart) * u.shape.w;
    }
    if (u.look.y < 1.5) {
        let e = vec2<f32>(q.x, q.y / u.shape.z);
        let r = length(e);
        let arms = 0.5 + 0.5 * cos(2.0 * (atan2(e.y, e.x) - log(max(r, 0.02)) * 2.6));
        let disc = exp(-r / 0.22) * mix(1.0, 0.35 + arms, u.extra.w);
        let lanes = 1.0 - 0.55 * smoothstep(0.5, 0.65, fbm(e * 7.0)) * smoothstep(0.08, 0.3, r);
        let bulge = exp(-length(vec2<f32>(q.x, q.y / max(u.shape.z, 0.6))) / u.colour_b.w);
        var light = u.colour_a.rgb * bulge * 2.2 + u.colour_b.rgb * disc * lanes;
        // An edge-on galaxy's dust lane, just off its middle, across the bulge.
        let lane = exp(-pow((q.y - 0.012) / 0.02, 2.0)) * (1.0 - smoothstep(0.4, 1.0, abs(q.x)));
        light *= 1.0 - u.colour_a.w * lane;
        // A companion, as the Whirlpool's.
        let c = length(p - u.extra.xy) / u.shape.x;
        light += u.colour_a.rgb * exp(-c / 0.03) * u.extra.z;
        return SKY + light * u.shape.w;
    }
    if (u.look.y < 2.5) {
        let e = vec2<f32>(q.x, q.y / u.shape.z);
        let r = length(e);
        let ring = exp(-pow((r - 0.62) / 0.2, 2.0)) * (0.8 + 0.4 * noise(e * 9.0));
        let fill = 0.35 * (1.0 - smoothstep(0.3, 0.7, r));
        let colour = mix(u.colour_b.rgb, u.colour_a.rgb, smoothstep(0.4, 0.8, r));
        return SKY + colour * (ring + fill) * u.shape.w;
    }
    if (u.look.y < 3.5) {
        // A comet: its coma, a broad dust tail curving away and a straight,
        // narrow ion tail, both streaming from the sun (toward -x).
        let coma = exp(-length(q) / 0.035);
        let back = max(-q.x, 0.0);
        let behind = smoothstep(0.0, 0.08, back) * exp(-back * 1.1);
        let dust_y = q.y - 0.18 * back * back;
        let dust = exp(-abs(dust_y) / (0.02 + back * 0.16)) * behind;
        let ion = exp(-abs(q.y + 0.01 * sin(back * 9.0 - u.frame.w * 0.3)) / (0.006 + back * 0.025)) * behind;
        let rays = 0.75 + 0.25 * noise(vec2<f32>(back * 6.0 - u.frame.w * 0.05, q.y * 60.0));
        return SKY + (u.colour_a.rgb * (coma * 1.4 + dust * 0.45) + u.colour_b.rgb * ion * 0.5 * rays) * u.shape.w;
    }
    // A supernova remnant: a cage of red filaments round a blue-white glow.
    let e = vec2<f32>(q.x, q.y / u.shape.z);
    let r = length(e);
    let shell = 1.0 - smoothstep(0.75, 1.0, r + 0.15 * noise(e * 3.0));
    let filaments = pow(1.0 - abs(2.0 * fbm(e * 5.0 + 2.0) - 1.0), 6.0);
    let glow = exp(-r * r / 0.18);
    return SKY + (u.colour_a.rgb * filaments * shell * 1.3 + u.colour_b.rgb * glow * 0.45) * u.shape.w;
}

// A cluster's unresolved glow and its crowd of faint stars: an open cluster
// in reflection nebulosity (variant 0), a globular (1), a double star (2),
// whose stars are all in `stars`, or a pair of rich open clusters (3), centred
// at `extra.xy` and `extra.zw`.
fn cluster(p: vec2<f32>, size: f32) -> vec3<f32> {
    let q = p / u.shape.x;
    let r2 = dot(q, q);
    if (u.look.y < 0.5) {
        let wisps = fbm(rotate(q, 0.5) * vec2<f32>(1.5, 6.0) + 4.0);
        return SKY + u.colour_b.rgb * pow(wisps, 3.0) * exp(-r2 * 0.6) * u.shape.w;
    }
    if (u.look.y < 1.5) {
        let crowd = star_field(p, 150.0, u.look.w + 40.0, 0.95 * exp(-r2 / 0.09), size * 0.8);
        let glow = u.colour_a.rgb * exp(-r2 / 0.02) * u.shape.w;
        return SKY + glow + crowd * 0.8;
    }
    if (u.look.y < 2.5) {
        return SKY;
    }
    let a = p - u.extra.xy;
    let b = p - u.extra.zw;
    let keep = 0.75 * (exp(-dot(a, a) / 0.05) + exp(-dot(b, b) / 0.05));
    return SKY + star_field(p, 46.0, u.look.w + 17.0, keep, size) * 6.0;
}

// Whatever crosses the field while the telescope looks: a satellite gliding
// through in sunlight, a meteor's streak, or an aircraft's lights, its strobe
// flashing. In screen field units.
fn passer(p0: vec2<f32>, size: f32) -> vec3<f32> {
    let kind = u.passer_look.x;
    let d = p0 - u.passer.xy;
    let v = u.passer.zw;
    if (kind < 1.5) {
        let flare = 1.0 + 0.6 * sin(u.passer_look.z * 0.7);
        return vec3<f32>(1.0, 0.97, 0.92) * u.passer_look.y * flare * exp(-dot(d, d) / (size * size));
    }
    if (kind < 2.5) {
        // The streak behind the head, along its path.
        let along = -dot(d, normalize(v));
        let across = length(d + normalize(v) * along);
        let trail = length(v) * 0.25;
        let body = exp(-pow(across / (size * 0.8), 2.0)) * (1.0 - smoothstep(0.0, trail, along)) * step(-size * 2.0, along);
        let fade = 1.0 - smoothstep(0.3, 0.9, u.passer_look.z);
        return vec3<f32>(0.75, 1.0, 0.85) * body * u.passer_look.y * fade;
    }
    // Nav lights at the wing tips, red to port and green to starboard, and a
    // white strobe flashing between them, all a little out of focus.
    let wing = normalize(vec2<f32>(-v.y, v.x)) * 0.09;
    let blur = size * 4.0 + 0.008;
    let red = exp(-dot(d - wing, d - wing) / (blur * blur));
    let green = exp(-dot(d + wing, d + wing) / (blur * blur));
    let strobe = exp(-dot(d, d) / (blur * blur)) * step(0.85, fract(u.passer_look.z * 0.9));
    return (vec3<f32>(1.0, 0.15, 0.1) * red + vec3<f32>(0.1, 1.0, 0.3) * green + vec3<f32>(1.0) * strobe * 2.0) * u.passer_look.y;
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let pixel = u.slew.w;
    let p0 = field_position(in.position.xy, u.frame);
    if (dot(p0, p0) > 1.1) {
        return encode(OUTSIDE);
    }
    let t = u.frame.w;
    // The air between the telescope and the target. The image dances as a
    // whole (in `view.xy`, from the CPU); over that, a faint boil of the
    // edges, under a pixel.
    let shimmer = 0.12 * u.view.z * pixel * vec2<f32>(
        sin(p0.y * 71.0 + t * 7.1) + sin(p0.y * 113.0 - t * 5.3),
        sin(p0.x * 67.0 + t * 6.3) + sin(p0.x * 109.0 + t * 4.7),
    );
    let p = p0 + u.view.xy + shimmer;
    let size = SEEING_DISC + u.view.w;
    let edge = pixel * 1.5 + u.view.w * 2.0;

    var colour = SKY;
    let kind = u.look.x;
    if (kind < 0.5) {
        colour = moon(p, edge);
    } else if (kind < 1.5) {
        colour = planet(p, edge);
    } else if (kind < 2.5) {
        colour = diffuse(p);
    } else {
        colour = cluster(p, size);
    }
    // The Moon drowns the stars behind it and around it.
    if (kind > 0.5) {
        // Two layers on grids of different pitch, so a rich field shows no grid.
        colour += star_field(p, 24.0, u.look.w, min(u.slew.z, 0.45), size);
        colour += star_field(p, 37.0, u.look.w + 61.0, 0.5 * u.slew.z, size);
        for (var i = 0; i < 12; i += 1) {
            let s = u.stars[i];
            if (s.z <= 0.0) {
                continue;
            }
            // Twinkling: a bright star's light comes and goes with the air.
            let twinkle = 1.0 + 0.18 * u.view.z * sin(t * (9.0 + f32(i)) + f32(i) * 2.1);
            let d = p - s.xy;
            let disc = size * (1.0 + 0.6 * s.z);
            // A bright star also shows the first ring of its diffraction
            // pattern and a soft glow, scattered in the optics.
            let r = length(d) / disc;
            let ring = 0.02 * exp(-pow(r - 3.4, 2.0) * 3.0) * u.look.z;
            let glow = 0.05 * exp(-length(d) / (SEEING_DISC * 9.0));
            colour += star_colour(s.w) * s.z * twinkle * (point(d, disc) + ring + glow);
        }
    }
    if (u.passer_look.x > 0.5) {
        colour += passer(p0, size);
    }
    // Cloud drifting over: it dims the sky and catches what light there is,
    // most round the Moon.
    if (u.weather.x > 0.0) {
        let cloud = smoothstep(0.35, 0.8, fbm(p0 * 1.2 + u.weather.yz)) * u.weather.x;
        colour = mix(colour, vec3<f32>(0.62, 0.64, 0.7) * u.weather.w, cloud * 0.9);
    }
    // The view fades through a slew, and darkens a little toward the stop.
    let r = length(p0);
    colour *= u.look.z * (1.0 - 0.3 * r * r * r * r);
    return encode(eyepiece(colour, p0, pixel));
}
