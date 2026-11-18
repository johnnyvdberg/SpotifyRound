// The microscope (`src/microscope.rs`): a drop of pond water on a slide.
// Field units: the field stop at radius 1.

struct Params {
    // Centre x, y and field-stop radius in pixels; seconds.
    frame: vec4<f32>,
    // Illumination (0 bright field, 1 dark field), the focal plane (-1..1),
    // how much of the view shows, the debris' seed.
    look: vec4<f32>,
    // The stage's offset (field units), extra defocus while a slide is
    // changed, and one pixel in field units.
    stage: vec4<f32>,
    // The light: the bright field's colour, or the dark field's.
    light: vec4<f32>,
    // Per organism, two rows: x, y, angle, size (0: none); then kind, depth
    // (-1..1), phase, hue. Drawn in order, far ones first.
    cells: array<vec4<f32>, 32>,
};

@group(0) @binding(0) var<uniform> u: Params;

// What an organism looks like at a point: the signed distance to its outline
// in field units (negative inside), how much fine structure shows there, 0..1,
// and its colour.
struct Look {
    d: f32,
    detail: f32,
    pigment: vec3<f32>,
};

// `q` is in the organism's own frame, its half-length 1; `t` its clock.
fn organism(q: vec2<f32>, kind: f32, t: f32, hue: f32) -> Look {
    var o: Look;
    if (kind < 0.5) {
        // Paramecium: a slipper with an oral groove, beating cilia, granular
        // cytoplasm and two contractile vacuoles filling and emptying.
        let b = 0.3 * (1.0 - 0.2 * q.x);
        var d = (length(vec2<f32>(q.x, q.y / b)) - 1.0) * b;
        d += select(0.0, 0.05 * exp(-pow((q.x + 0.05) / 0.2, 2.0)), q.y > 0.0);
        d += 0.0035 * sin(atan2(q.y, q.x) * 90.0 - t * 22.0);
        let pulse = 0.6 + 0.4 * sin(t * 1.4);
        let vacuoles = max(
            1.0 - smoothstep(0.07 * pulse, 0.1 * pulse, length(q - vec2<f32>(0.55, 0.02))),
            1.0 - smoothstep(0.07, 0.1, length(q - vec2<f32>(-0.55, -0.02))) * (1.0 - pulse),
        );
        o.d = d;
        o.detail = 0.45 * smoothstep(0.45, 0.85, noise(q * 26.0 + t * 0.05)) + vacuoles;
        o.pigment = vec3<f32>(0.82, 0.86, 0.78);
    } else if (kind < 1.5) {
        // A pennate diatom: a glass boat with rows of pores across it and the
        // raphe down its middle.
        let b = 0.16 * sqrt(max(1.0 - q.x * q.x, 0.0));
        o.d = max(abs(q.y) - b, abs(q.x) - 1.0);
        let striae = smoothstep(0.2, 0.9, 0.5 + 0.5 * sin(q.x * 80.0)) * smoothstep(0.015, 0.03, abs(q.y));
        let raphe = exp(-pow(q.y / 0.012, 2.0)) * (1.0 - exp(-pow(q.x / 0.06, 2.0)));
        o.detail = 0.55 * striae + raphe;
        o.pigment = mix(vec3<f32>(0.78, 0.62, 0.3), vec3<f32>(0.6, 0.66, 0.32), hue);
    } else if (kind < 2.5) {
        // A centric diatom: a pill box of radial ribs and rings of pores.
        let r = length(q);
        o.d = r - 1.0;
        let ribs = 0.5 + 0.5 * sin(atan2(q.y, q.x) * 36.0);
        let rings = 0.5 + 0.5 * sin(r * 48.0);
        o.detail = 0.5 * ribs * rings * smoothstep(0.15, 0.3, r) + 0.4 * (1.0 - smoothstep(0.1, 0.16, r));
        o.pigment = mix(vec3<f32>(0.8, 0.66, 0.34), vec3<f32>(0.66, 0.6, 0.36), hue);
    } else if (kind < 3.5) {
        // Volvox: a rolling sphere of a few thousand cells, daughter colonies
        // inside.
        let r = length(q);
        o.d = r - 1.0;
        let g = rotate(q, t * 0.15) * 15.0;
        let dots = 1.0 - smoothstep(0.12, 0.28, length(fract(g) - 0.5));
        var daughters = 0.0;
        for (var k = 0; k < 3; k += 1) {
            let a = f32(k) * 2.1 + t * 0.15;
            let c = vec2<f32>(cos(a), sin(a)) * 0.45;
            daughters = max(daughters, 1.0 - smoothstep(0.16, 0.22, length(q - c)));
        }
        o.detail = 0.45 * dots + 0.9 * daughters;
        o.pigment = vec3<f32>(0.42, 0.7, 0.32);
    } else if (kind < 4.5) {
        // Amoeba: a creeping blob, its pseudopods slowly reaching out.
        let r = length(q);
        let dir = q / max(r, 1e-4);
        let reach = 0.8 + 0.5 * noise(dir * 1.7 + vec2<f32>(t * 0.06, hue * 9.0));
        o.d = (r - reach) * 0.6;
        let nucleus = 1.0 - smoothstep(0.14, 0.2, length(q - vec2<f32>(0.15, 0.1)));
        o.detail = 0.45 * smoothstep(0.45, 0.85, noise(q * 28.0 - t * 0.1)) + 0.7 * nucleus;
        o.pigment = vec3<f32>(0.8, 0.82, 0.76);
    } else {
        // Closterium, a desmid: a green crescent with clear tips, where
        // crystals dance.
        let v = q - vec2<f32>(0.0, -1.5);
        let r = length(v);
        let along = atan2(v.x, v.y) / 0.62;
        let thick = 0.17 * sqrt(max(1.0 - along * along, 0.0));
        o.d = max(abs(r - 1.5) - thick, (abs(along) - 1.0) * 0.9);
        let tip = smoothstep(0.72, 0.85, abs(along));
        let ridges = 0.5 + 0.5 * sin((r - 1.5) / max(thick, 0.02) * 7.0);
        o.detail = (1.0 - tip) * 0.5 * ridges + tip * 0.5 * step(0.8, noise(q * 40.0 + t * 2.0));
        o.pigment = mix(vec3<f32>(0.45, 0.72, 0.3), vec3<f32>(0.9, 0.9, 0.86), tip);
    }
    return o;
}

// The light a dark field scatters off a glassy edge: a rainbow round a
// diatom, by its hue.
fn sheen(hue: f32) -> vec3<f32> {
    return 0.6 + 0.4 * cos(TAU * (hue + vec3<f32>(0.0, 0.33, 0.67)));
}

@fragment
fn fs_main(in: Varying) -> @location(0) vec4<f32> {
    let pixel = u.stage.w;
    let p0 = field_position(in.position.xy, u.frame);
    if (dot(p0, p0) > 1.1) {
        return encode(OUTSIDE);
    }
    let t = u.frame.w;
    let p = p0 + u.stage.xy;
    let dark = u.look.x > 0.5;
    let r0 = dot(p0, p0);

    // The lamp: even light, a little warmer and dimmer toward the stop.
    var colour = select(u.light.rgb * (1.0 - 0.18 * r0), u.light.rgb * 0.0025, dark);

    // Debris and bacteria, jiggling in Brownian motion, each at its own depth.
    let g = p * 34.0 + u.look.w;
    let cell = floor(g);
    let h = hash22(cell);
    if (hash21(cell + 2.3) < select(0.28, 0.12, dark)) {
        let jiggle = 0.06 * vec2<f32>(sin(t * 1.3 + h.x * 40.0), cos(t * 1.1 + h.y * 30.0));
        let d = length(g - (cell + 0.3 + 0.4 * h + jiggle)) / 34.0;
        let size = 0.0015 + 0.004 * hash21(cell + 5.1);
        let w = pixel + 0.01 * abs(hash21(cell + 8.0) * 2.0 - 1.0 - u.look.y) + u.stage.z;
        let speck = 1.0 - smoothstep(size - w, size + w, d);
        let sharp = clamp(2.0 * pixel / w, 0.15, 1.0);
        colour = select(colour * (1.0 - 0.45 * speck * sharp), colour + u.light.rgb * 0.08 * speck * sharp, dark);
    }

    for (var i = 0; i < 16; i += 1) {
        let a = u.cells[2 * i];
        if (a.w <= 0.0) {
            continue;
        }
        let q = rotate(p - a.xy, a.z) / a.w;
        if (dot(q, q) > 4.5) {
            continue;
        }
        let b = u.cells[2 * i + 1];
        let o = organism(q, b.x, t + b.z * 10.0, b.w);
        // Out of the focal plane an outline spreads and its structure fades.
        let w = 1.2 * pixel + 0.016 * abs(b.y - u.look.y) + u.stage.z;
        let d = o.d * a.w;
        let inside = 1.0 - smoothstep(-w, w, d);
        let rim = exp(-pow(d / max(w, 0.0025), 2.0));
        let sharp = clamp(3.0 * pixel / w, 0.1, 1.0);
        let detail = o.detail * sharp * inside;
        if (dark) {
            let glow = select(vec3<f32>(1.0), sheen(b.w + q.x * 0.15), b.x > 0.5 && b.x < 2.5);
            colour += u.light.rgb * glow * o.pigment * (rim * 0.9 * sharp + inside * 0.03 + detail * 0.22);
        } else {
            // Absorbed by the pigment, darkest at the outline, with the bright
            // halo a refracting edge throws just outside it.
            let halo = exp(-pow((d - 3.0 * max(w, 0.0025)) / max(w, 0.0025), 2.0));
            colour *= mix(vec3<f32>(1.0), o.pigment * (1.0 - 0.35 * detail), inside * 0.85);
            colour *= 1.0 - 0.6 * rim * sharp;
            colour += u.light.rgb * 0.06 * halo * sharp;
        }
    }
    colour *= u.look.z;
    return encode(eyepiece(colour, p0, pixel));
}
