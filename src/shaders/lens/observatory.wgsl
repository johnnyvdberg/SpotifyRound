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
    colour_a: vec4<f32>,
    colour_b: vec4<f32>,
    // Per kind: size, tilt, ring tilt or inclination, detail.
    shape: vec4<f32>,
    // Per kind: the moon's sun, or a galaxy's companion.
    extra: vec4<f32>,
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

// The Moon near its terminator: maria, craters lit low from the side.
fn moon(p: vec2<f32>, edge: f32) -> vec3<f32> {
    let q = p / u.shape.x;
    let d2 = dot(q, q);
    if (d2 > 1.0) {
        return SKY;
    }
    let z = sqrt(1.0 - d2);
    var n = vec3<f32>(q, z);
    let maria = smoothstep(0.45, 0.58, fbm(q * 2.2 + 5.0));
    var albedo = mix(0.32, 0.1, maria) * (0.85 + 0.3 * noise(q * 40.0));
    // Two sizes of crater, one per cell, each a bowl inside a raised rim.
    for (var k = 0; k < 2; k += 1) {
        let scale = select(9.0, 26.0, k == 1);
        let g = q * scale;
        let cell = floor(g);
        let centre = cell + 0.3 + 0.4 * hash22(cell + f32(k) * 31.0);
        // Not every cell has a crater, and few are large.
        let roll = hash21(cell + 4.0 + f32(k));
        let radius = select(0.0, 0.06 + 0.24 * roll * roll, hash21(cell + 9.0 + f32(k)) < 0.55);
        let v = g - centre;
        let r = length(v);
        let x = r / radius;
        var slope = 0.0;
        if (radius <= 0.0) {
            continue;
        }
        if (x < 1.0) {
            slope = 2.0 * x;
        } else {
            slope = -4.0 * (x - 1.0) * exp(-pow((x - 1.0) / 0.35, 2.0)) / 0.35;
        }
        // Maria flooded the smaller craters.
        let depth = select(0.35, 0.22 * (1.0 - 0.7 * maria), k == 1);
        n += vec3<f32>(v / max(r, 1e-4) * slope * depth, 0.0);
        albedo *= 1.0 + 0.25 * exp(-pow((x - 1.0) / 0.25, 2.0));
    }
    let light = max(dot(normalize(n), u.extra.xyz), 0.0);
    let earthshine = 0.004;
    let shown = 1.0 - smoothstep(1.0 - edge, 1.0, sqrt(d2));
    return mix(SKY, vec3<f32>(1.0, 0.97, 0.92) * albedo * (light * 3.2 + earthshine), shown);
}

// A planet: Jupiter's belts (variant 0), Saturn and its rings (1), or Mars (2).
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
                let spot = length((q - vec2<f32>(0.35, -0.38)) * vec2<f32>(1.0, 1.9));
                surface = mix(surface, vec3<f32>(0.62, 0.28, 0.16), (1.0 - smoothstep(0.08, 0.12, spot)) * 0.7);
            }
        } else {
            let dark = smoothstep(0.45, 0.65, fbm(q * 2.6 + 3.0));
            surface = mix(u.colour_a.rgb, u.colour_b.rgb, dark);
            surface = mix(surface, vec3<f32>(0.95, 0.93, 0.9), smoothstep(0.82, 0.86, lat));
        }
        // Limb darkening, and the edge softened by the focus.
        let lit = surface * (0.35 + 0.65 * pow(z, 0.45));
        let shown = 1.0 - smoothstep(1.0 - edge / u.shape.x, 1.0, sqrt(d2));
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
        let bulge = exp(-r / 0.035);
        var light = u.colour_a.rgb * bulge * 2.2 + u.colour_b.rgb * disc * lanes;
        // A companion, as the Whirlpool's.
        let c = length(p - u.extra.xy) / u.shape.x;
        light += u.colour_a.rgb * exp(-c / 0.03) * u.extra.z;
        return SKY + light * u.shape.w;
    }
    let e = vec2<f32>(q.x, q.y / u.shape.z);
    let r = length(e);
    let ring = exp(-pow((r - 0.62) / 0.2, 2.0)) * (0.8 + 0.4 * noise(e * 9.0));
    let fill = 0.35 * (1.0 - smoothstep(0.3, 0.7, r));
    let colour = mix(u.colour_b.rgb, u.colour_a.rgb, smoothstep(0.4, 0.8, r));
    return SKY + colour * (ring + fill) * u.shape.w;
}

// A cluster's unresolved glow and its crowd of faint stars: an open cluster
// in reflection nebulosity (variant 0), a globular (1), or a double star (2),
// whose stars are all in `stars`.
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
    return SKY;
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
    // The view fades through a slew, and darkens a little toward the stop.
    let r = length(p0);
    colour *= u.look.z * (1.0 - 0.3 * r * r * r * r);
    return encode(eyepiece(colour, p0, pixel));
}
