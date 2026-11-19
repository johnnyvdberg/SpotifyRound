// The microscope (`src/microscope.rs`): a drop of pond water on a slide.
// Field units: the field stop at radius 1.

struct Params {
    // Centre x, y and field-stop radius in pixels; seconds.
    frame: vec4<f32>,
    // Illumination (0 bright field, 1 dark field, 2 phase contrast), the
    // focal plane (-1..1), how much of the view shows, the debris' seed.
    look: vec4<f32>,
    // The stage's offset (slide units), extra defocus while a slide or an
    // objective is changed, and one pixel in field units.
    stage: vec4<f32>,
    // The background: the lamp's colour in bright field and phase contrast,
    // the dark (or, for Rheinberg lighting, coloured) ground in dark field.
    light: vec4<f32>,
    // The light an edge scatters in dark field; the objective's zoom.
    glow: vec4<f32>,
    // The slide's tissue (0 none, 1 onion skin, 2 a blood smear), how many
    // bacteria swarm, how much debris, and the tissue's stain strength.
    slide: vec4<f32>,
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
    if (kind > 5.5) {
        return organism2(q, kind, t, hue);
    }
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

// The newer organisms, kinds 6 to 10.
fn organism2(q: vec2<f32>, kind: f32, t: f32, hue: f32) -> Look {
    var o: Look;
    if (kind < 6.5) {
        // Euglena: a green spindle squirming as it swims, chloroplasts inside,
        // a red eyespot at the front and a whipping flagellum ahead of it.
        let squirm = 1.0 + 0.18 * sin(q.x * 3.0 - t * 2.2);
        let b = 0.24 * squirm * sqrt(max(1.0 - q.x * q.x, 0.0)) * (1.0 - 0.3 * max(-q.x, 0.0));
        var d = max(abs(q.y) - b, abs(q.x) - 1.0) * 0.8;
        // The flagellum: a thin line waving ahead.
        let fx = q.x - 1.0;
        if (fx > 0.0 && fx < 0.9) {
            let wave = 0.12 * fx * sin(fx * 9.0 - t * 18.0);
            d = min(d, abs(q.y - wave) - 0.012);
        }
        let plastids = smoothstep(0.55, 0.7, noise(q * vec2<f32>(9.0, 20.0) + 3.0));
        let eyespot = 1.0 - smoothstep(0.05, 0.08, length(q - vec2<f32>(0.72, 0.05)));
        o.d = d;
        o.detail = 0.6 * plastids + eyespot;
        o.pigment = mix(vec3<f32>(0.45, 0.75, 0.32), vec3<f32>(0.85, 0.2, 0.12), eyespot);
    } else if (kind < 7.5) {
        // Vorticella: a bell on a stalk anchored behind it (toward -x), the
        // stalk `hue` long, coiling as it snaps short.
        let bell = vec2<f32>(q.x - 0.25, q.y);
        var d = max(length(bell * vec2<f32>(1.0, 1.3)) - 0.3, -(q.x - 0.5));
        let mouth = exp(-pow((q.x - 0.5) / 0.04, 2.0)) * step(abs(q.y), 0.26);
        let stalk = 0.3 + 1.6 * hue;
        if (q.x < 0.0 && q.x > -stalk) {
            let coil = (1.0 - hue) * 0.12 * sin(q.x * 30.0);
            d = min(d, abs(q.y - coil) - 0.025);
        }
        let cilia = 0.5 + 0.5 * sin(q.y * 60.0 + t * 25.0);
        o.d = d;
        o.detail = 0.5 * smoothstep(0.5, 0.8, noise(q * 22.0)) + mouth * cilia;
        o.pigment = vec3<f32>(0.85, 0.86, 0.8);
    } else if (kind < 8.5) {
        // Spirogyra: a filament of cells, each wound with a ribbon of
        // chloroplast in a spiral.
        o.d = max(abs(q.y) - 0.11, abs(q.x) - 2.0);
        let walls = exp(-pow((fract(q.x * 2.6 + hue) - 0.5) / 0.04, 2.0));
        let ribbon = smoothstep(0.55, 0.8, 0.5 + 0.5 * sin(q.x * 14.0 + q.y / 0.11 * 1.6 + hue * 9.0));
        o.detail = 0.8 * walls + 0.6 * ribbon;
        o.pigment = mix(vec3<f32>(0.75, 0.88, 0.7), vec3<f32>(0.3, 0.68, 0.22), ribbon);
    } else if (kind < 9.5) {
        // Pediastrum: a flat star of cells in rings, the outer ones horned.
        let r = length(q);
        let a = atan2(q.y, q.x);
        let ring = floor(r * 3.2);
        let count = max(1.0, ring * 6.0);
        let cell_angle = (fract(a / TAU * count + ring * 0.3) - 0.5) / count * TAU * r;
        let cell_r = fract(r * 3.2) - 0.5;
        let cell = length(vec2<f32>(cell_angle * 3.2, cell_r));
        let horns = select(0.0, 0.08 * abs(sin(a * count * 0.5)), r > 0.85);
        o.d = r - 1.0 - horns;
        o.detail = smoothstep(0.32, 0.42, cell) * 0.9;
        o.pigment = vec3<f32>(0.42, 0.72, 0.3);
    } else {
        // A tardigrade: a plump body of segments on four pairs of stubby
        // clawed legs, lumbering.
        var d = (length(q * vec2<f32>(1.0, 2.3)) - 1.0) * 0.42;
        // The head, a rounded snout.
        d = min(d, length((q - vec2<f32>(0.92, 0.0)) * vec2<f32>(1.0, 1.4)) - 0.2);
        // Three pairs of legs along the sides, swinging as it walks, and the
        // last pair pointing back.
        for (var i = 0; i < 3; i += 1) {
            let x = -0.45 + f32(i) * 0.42;
            let swing = 0.07 * sin(t * 3.0 + f32(i) * 2.1);
            for (var side = -1.0; side < 2.0; side += 2.0) {
                let foot = vec2<f32>(x + swing * side, 0.62 * side);
                let hip = vec2<f32>(x, 0.3 * side);
                let along = clamp(dot(q - hip, foot - hip) / dot(foot - hip, foot - hip), 0.0, 1.0);
                d = min(d, length(q - mix(hip, foot, along)) - 0.1 + 0.03 * along);
            }
        }
        for (var side = -1.0; side < 2.0; side += 2.0) {
            let hip = vec2<f32>(-0.8, 0.2 * side);
            let foot = vec2<f32>(-1.12, 0.36 * side);
            let along = clamp(dot(q - hip, foot - hip) / dot(foot - hip, foot - hip), 0.0, 1.0);
            d = min(d, length(q - mix(hip, foot, along)) - 0.09);
        }
        let segments = 0.5 + 0.5 * sin(q.x * 11.0);
        let gut = 1.0 - smoothstep(0.12, 0.22, length(q * vec2<f32>(0.8, 2.6) + vec2<f32>(0.1, 0.0)));
        o.d = d;
        o.detail = 0.3 * segments + 0.6 * gut + 0.3 * smoothstep(0.6, 0.8, noise(q * 20.0));
        o.pigment = mix(vec3<f32>(0.86, 0.72, 0.52), vec3<f32>(0.55, 0.5, 0.3), gut);
    }
    return o;
}

// Stained tissue under the cover slip, filling the field: onion skin (1), its
// long cells walled and their nuclei stained with iodine, or a blood smear
// (2), red cells with pale middles and the odd white cell stained purple.
// Returns how much light it lets through, per channel.
fn tissue(p: vec2<f32>, w: f32) -> vec3<f32> {
    let stain = u.slide.w;
    if (u.slide.x < 1.5) {
        let warp = p + 0.03 * vec2<f32>(noise(p * 4.0), noise(p * 4.0 + 7.0));
        let g = warp * vec2<f32>(2.4, 7.0);
        let row = floor(g.y);
        let cell_x = g.x + hash21(vec2<f32>(row, 3.0));
        let f = vec2<f32>(fract(cell_x), fract(g.y));
        let id = vec2<f32>(floor(cell_x), row);
        let wall = min(min(f.x, 1.0 - f.x) / 2.4, min(f.y, 1.0 - f.y) / 7.0);
        let walls = exp(-pow(wall / (0.0035 + w), 2.0));
        let h = hash22(id);
        let nucleus_at = vec2<f32>(0.25 + 0.5 * h.x, 0.3 + 0.4 * h.y);
        let nd = length((f - nucleus_at) * vec2<f32>(1.0 / 2.4, 1.0 / 7.0) * vec2<f32>(1.0, 1.4));
        let nucleus = 1.0 - smoothstep(0.024 - w, 0.03 + w, nd);
        let cytoplasm = vec3<f32>(1.0, 0.93, 0.7);
        var through = mix(vec3<f32>(1.0), cytoplasm, stain);
        through *= 1.0 - 0.55 * walls * vec3<f32>(0.8, 0.9, 1.0);
        through *= mix(vec3<f32>(1.0), vec3<f32>(0.62, 0.42, 0.2), nucleus * stain);
        return through;
    }
    let g = p * 13.0;
    let cell = floor(g);
    let h = hash22(cell);
    let kind = hash21(cell + 5.5);
    let at = cell + 0.38 + 0.24 * h;
    let d = length(g - at) / 13.0;
    var through = vec3<f32>(1.0);
    if (kind < 0.012) {
        // A white cell: pale cytoplasm round a lobed purple nucleus.
        let r = 0.034;
        let inside = 1.0 - smoothstep(r - w, r + w, d);
        let lobes = 0.022 * (1.0 + 0.3 * sin(atan2(g.y - at.y, g.x - at.x) * 3.0 + h.x * 6.0));
        let nucleus = 1.0 - smoothstep(lobes - w, lobes + w, d);
        through *= mix(vec3<f32>(1.0), vec3<f32>(0.88, 0.82, 0.95), inside);
        through *= mix(vec3<f32>(1.0), vec3<f32>(0.45, 0.25, 0.6), nucleus * stain);
    } else if (kind < 0.86) {
        // A red cell: a biconcave disc, its middle thinner and paler.
        let r = 0.03 + 0.003 * h.y;
        let inside = 1.0 - smoothstep(r - w, r + w, d);
        let pallor = exp(-pow(d / (r * 0.4), 2.0));
        let density = inside * (1.0 - 0.35 * pallor);
        through *= mix(vec3<f32>(1.0), vec3<f32>(0.9, 0.42, 0.45), density * stain);
        through *= 1.0 - 0.25 * exp(-pow((d - r) / (0.002 + w), 2.0));
    } else if (kind < 0.9) {
        // A platelet.
        let inside = 1.0 - smoothstep(0.004 - w, 0.006 + w, d);
        through *= mix(vec3<f32>(1.0), vec3<f32>(0.7, 0.55, 0.8), inside);
    }
    return through;
}

// Bacteria: rods darting about, each in its own cell of the grid.
fn bacteria(p: vec2<f32>, w: f32) -> f32 {
    let g = p * 60.0 + u.look.w * 1.7;
    let cell = floor(g);
    if (hash21(cell + 1.9) > u.slide.y) {
        return 0.0;
    }
    let h = hash22(cell);
    let t = u.frame.w;
    let at = cell + 0.5 + 0.28 * vec2<f32>(sin(t * (1.0 + 2.0 * h.x) + h.y * 20.0), cos(t * (1.2 + 1.5 * h.y) + h.x * 20.0));
    let v = rotate(g - at, h.x * TAU + t * (h.y - 0.5) * 2.0);
    let rod = length(vec2<f32>(max(abs(v.x) - 0.12, 0.0), v.y)) - 0.05;
    return 1.0 - smoothstep(-w * 60.0, w * 60.0, rod);
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
    // The objective's zoom: the slide under it, in slide units.
    let zoom = u.glow.w;
    let p = p0 / zoom + u.stage.xy;
    let px = pixel / zoom;
    let mode = u.look.x;
    let dark = mode > 0.5 && mode < 1.5;
    let phase = mode > 1.5;
    let r0 = dot(p0, p0);

    // The lamp: even light, a little dimmer toward the stop; in dark field,
    // the ground.
    var colour = select(u.light.rgb * (1.0 - 0.18 * r0), u.light.rgb, dark);

    // Stained tissue fills the field, at the focal plane's middle.
    if (u.slide.x > 0.5) {
        let w = px + 0.01 * abs(u.look.y) + u.stage.z;
        colour *= tissue(p, w);
    }

    // Debris, jiggling in Brownian motion, each speck at its own depth.
    let g = p * 34.0 + u.look.w;
    let cell = floor(g);
    let h = hash22(cell);
    if (hash21(cell + 2.3) < u.slide.z * select(1.0, 0.45, dark)) {
        let jiggle = 0.06 * vec2<f32>(sin(t * 1.3 + h.x * 40.0), cos(t * 1.1 + h.y * 30.0));
        let d = length(g - (cell + 0.3 + 0.4 * h + jiggle)) / 34.0;
        let size = 0.0015 + 0.004 * hash21(cell + 5.1);
        let w = px + 0.01 * abs(hash21(cell + 8.0) * 2.0 - 1.0 - u.look.y) + u.stage.z;
        let speck = 1.0 - smoothstep(size - w, size + w, d);
        let sharp = clamp(2.0 * px / w, 0.15, 1.0);
        colour = select(colour * (1.0 - 0.45 * speck * sharp), colour + u.glow.rgb * 0.08 * speck * sharp, dark);
    }
    if (u.slide.y > 0.0) {
        let w = px + 0.006 * abs(0.3 - u.look.y) + u.stage.z;
        let rods = bacteria(p, w);
        colour = select(colour * (1.0 - 0.35 * rods), colour + u.glow.rgb * 0.05 * rods, dark);
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
        let w = 1.2 * px + 0.016 * abs(b.y - u.look.y) + u.stage.z;
        let d = o.d * a.w;
        let inside = 1.0 - smoothstep(-w, w, d);
        let rim = exp(-pow(d / max(w, 0.0025), 2.0));
        let sharp = clamp(3.0 * px / w, 0.1, 1.0);
        let detail = o.detail * sharp * inside;
        let halo = exp(-pow((d - 3.0 * max(w, 0.0025)) / max(w, 0.0025), 2.0));
        if (dark) {
            let glass = b.x > 0.5 && b.x < 2.5;
            let glow = select(vec3<f32>(1.0), sheen(b.w + q.x * 0.15), glass && u.glow.r > u.glow.b);
            colour += u.glow.rgb * glow * o.pigment * (rim * 0.9 * sharp + inside * 0.03 + detail * 0.22);
        } else if (phase) {
            // Phase contrast: what slows the light shows dark, with a bright
            // halo round it, whatever its colour.
            let grey = dot(o.pigment, vec3<f32>(0.3, 0.5, 0.2));
            colour *= mix(1.0, 0.3 + 0.2 * grey - 0.2 * detail, inside * 0.95);
            colour *= 1.0 - 0.45 * rim * sharp;
            colour += u.light.rgb * 0.8 * halo * sharp;
        } else {
            // Absorbed by the pigment, darkest at the outline, with the bright
            // halo a refracting edge throws just outside it.
            colour *= mix(vec3<f32>(1.0), o.pigment * (1.0 - 0.35 * detail), inside * 0.85);
            colour *= 1.0 - 0.6 * rim * sharp;
            colour += u.light.rgb * 0.06 * halo * sharp;
        }
    }
    colour *= u.look.z;
    return encode(eyepiece(colour, p0, pixel));
}
