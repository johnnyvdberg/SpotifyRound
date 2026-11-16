// The starbase's surfaces (`starbase::mesh`), family 6 of the models: what
// lights each place in it, and each part's look.

import package::solar::frame::{hash_cell, params};
import package::solar::encounter::{MODEL, ModelOut, ev_ship, ev_t};
import package::solar::starbase::common::{base_bend, base_fog, base_haze};
import package::solar::starbase::pane::{city_map, city_sampler};
import package::solar::starbase::world::{METAL_LAYER, SURFACE_LAYER, SURFACE_SIDE};
import package::solar::starbase::pane::WORLD_HAZE;
import constants::EV_BASE;

// The light at `p` facing `n`, worked out at the meshes' vertices
// (`vs_model`): its colour, and how far the tunnel's ribs pool it (0 not
// at all, 1 wholly: `base_lit` does that part per pixel, as a segment's
// wall has vertices only at its ends). Above the mouth, the star and the
// sky's little; down the entrance, its rings of lights; in the tunnel the
// lit rings at each rib; outside the tunnel's wall, in the hollow, the
// city's glow, cyan and magenta by turns, and its lamps overhead; in the
// port its ceiling's panels, greener round the ship's dock once it is
// serviced.
fn base_light(p: vec3<f32>, n: vec3<f32>) -> vec4<f32> {
    let row = params.ev[EV_BASE + 1];
    let segs = params.ev[EV_BASE + 2];
    if (p.z < 0.0) {
        let sun = params.ev[EV_BASE + 3].xyz;
        return vec4<f32>(vec3<f32>(1.0, 0.96, 0.9) * max(dot(n, sun), 0.0) + vec3<f32>(0.035, 0.045, 0.07), 0.0);
    }
    if (p.z < row.x) {
        let r = vec3<f32>(p.x, p.y, 0.0);
        let toward = -r / max(length(r), 1.0);
        return vec4<f32>(vec3<f32>(0.72, 0.8, 0.95) * (0.2 + 0.4 * max(dot(n, toward), 0.0) + 0.25 * max(-n.z, 0.0)), 0.0);
    }
    // The world's own towers, far out from the axis: the strip suns' light
    // from the axis' side, so their roofs and the faces turned to it are
    // bright and the rest in shade.
    let out = vec3<f32>(p.x - 200.0, p.y, 0.0);
    if (dot(out, out) > 4e6) {
        return vec4<f32>(WORLD_HAZE * 0.6 + vec3<f32>(0.95, 0.94, 0.9) * (0.28 + 0.6 * max(-dot(n, normalize(out)), 0.0)), 0.0);
    }
    if (p.z < segs.w) {
        let r = vec3<f32>(p.x - base_bend(p.z), p.y, 0.0);
        let far = length(r);
        let toward = -r / max(far, 1.0);
        if (far < row.w + 14.0) {
            return vec4<f32>(vec3<f32>(0.85, 0.9, 1.0) * (0.3 + 0.5 * max(dot(n, toward), 0.0)), 1.0);
        }
        return vec4<f32>(WORLD_HAZE + vec3<f32>(0.9, 0.92, 1.0) * (0.25 + 0.25 * max(-dot(n, toward), 0.0) + 0.2 * abs(n.y)), 0.0);
    }
    // The port, out in the open: the strip suns' light from all round.
    let green = params.ev[EV_BASE].w * exp(-length(p - vec3<f32>(row.z, 0.0, segs.w + 520.0)) / 300.0);
    let sky = mix(vec3<f32>(0.95, 0.96, 1.0), vec3<f32>(0.55, 1.0, 0.7), green * 0.5);
    return vec4<f32>(WORLD_HAZE + sky * (0.3 + 0.3 * abs(n.y) + 0.2 * abs(n.x)), 0.0);
}

// The light on a pixel at `p` facing `n`, from its vertices' (`lit`): the
// tunnel's pooled round its ribs.
fn base_lit(lit: vec4<f32>, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let rib = abs(fract((p.z - params.ev[EV_BASE + 1].x) / params.ev[EV_BASE + 2].x) - 0.5) * 2.0;
    let light = lit.rgb * (1.0 - lit.w * 0.6 * (1.0 - rib * rib));
    return light;
}

// A neon's colour: cyan, magenta or amber.
fn neon(k: f32) -> vec3<f32> {
    return select(select(vec3<f32>(1.0, 0.25, 0.85), vec3<f32>(0.2, 0.9, 1.0), k < 1.0), vec3<f32>(1.0, 0.6, 0.15), k >= 2.0);
}

// Lights in cells `g`, a `share` of them lit, finer than a pixel their
// average: how bright here.
fn base_lights(g: vec2<f32>, share: f32, tone: f32) -> f32 {
    let f = abs(fract(g) - 0.5);
    let lit = step(hash_cell(vec3<f32>(floor(g), tone * 255.0)), share);
    let pixel = fwidth(g.x) + fwidth(g.y);
    return mix(step(max(f.x, f.y * 1.2), 0.3) * lit, 0.25 * share, clamp(pixel * 1.3 - 0.45, 0.0, 1.0));
}

const ARMOUR: vec3<f32> = vec3<f32>(0.1, 0.11, 0.135);
const LAMPLIGHT: vec3<f32> = vec3<f32>(1.0, 0.84, 0.6);

// The face of the megastructure's dish (`models::BASE_PLATING`), an entry
// point of its own as it reads the base's maps: the dish's map about the
// mouth (`surface_texel`), lit from its vertices' light, and its own
// lights.
@fragment
fn fs_plating(in: ModelOut) -> @location(0) vec4<f32> {
    let q = in.local.xy * in.size;
    let surface = textureSample(city_map, city_sampler, q / SURFACE_SIDE + 0.5, SURFACE_LAYER);
    let colour = surface.rgb * 2.0 * in.lit.rgb + vec3<f32>(0.7, 0.85, 1.0) * surface.a * 1.3;
    return vec4<f32>(base_fog(colour, length(in.toward), ev_ship() - in.toward), 1.0);
}

// The megastructure and its entrance (`models::
// BASE_SURFACE`), by part: their faces, and the rest plain: roofs and
// landings, lamps, hazard stripes and the beacons' red.
fn base_outside(q: vec3<f32>, n: vec3<f32>, part: i32, p: vec3<f32>, tone: f32, light: vec3<f32>, metal: f32) -> vec3<f32> {
    var albedo = ARMOUR * metal;
    var glow = vec3<f32>(0.0);
    if (part == 13) {
        // A face's windows: up it (out of the surface above the mouth and
        // down the entrance) and across it, a share of them lit.
        var across = q.y * n.x - q.x * n.y;
        var share = 0.12 + 0.3 * fract(tone * 7.3);
        if (p.z > 0.0) {
            // Round the pit or the megastructure: not the arc's length but
            // near enough.
            across = q.x * sign(q.y) * 1.57;
        }
        let white = mix(LAMPLIGHT, vec3<f32>(0.65, 0.85, 1.0), step(0.3, fract(tone * 3.1)));
        glow = white * base_lights(vec2<f32>(across / 11.0, -q.z / 6.0), share, tone) * (0.7 + 0.5 * fract(tone * 11.0));
        albedo *= 0.75 + 0.5 * fract(tone * 5.7);
    } else if (part == 0) {
        // A roof or a landing: plates, a shade each.
        let id = floor(q.xy / 90.0);
        albedo *= 0.85 + 0.35 * fract(id.x * 0.37 + id.y * 0.61 + tone);
    } else if (part == 11) {
        albedo = vec3<f32>(0.0);
        glow = vec3<f32>(1.1, 1.15, 1.25);
    } else if (part == 12) {
        albedo = mix(vec3<f32>(0.03), vec3<f32>(0.8, 0.6, 0.05), step(0.5, fract((q.x + q.y + q.z) * 0.02)));
    } else if (part == 15) {
        albedo = vec3<f32>(0.0);
        glow = vec3<f32>(1.8, 0.14, 0.08) * step(fract(ev_t() * 0.9 + p.x * 0.013 + p.y * 0.007), 0.3);
    } else {
        albedo *= select(0.25, 0.6, part == 1);
    }
    return albedo * light + glow;
}

// A structure of the starbase or a craft in it (`models::BASE`), by part:
// plating in panels, darker frames, lamps lit by themselves, hazard
// stripes, neon in its colour (`tone`: a colour, the tunnel's guide
// flowing in, a red beacon, or a berth's status from `look.x`), ships'
// windows and engines (lit when `look.x` says they are under way). `q` is
// the surface in its own frame, metres; `look.yzw` tints a craft's hull.
fn base_structure(q: vec3<f32>, n: vec3<f32>, part: i32, p: vec3<f32>, tone: f32, look: vec4<f32>, light: vec3<f32>, metal: f32) -> vec3<f32> {
    let t = ev_t();
    var albedo = vec3<f32>(0.0);
    var glow = vec3<f32>(0.0);
    if (part < 2) {
        let tint = select(look.yzw, vec3<f32>(1.0), look.y + look.z + look.w < 0.01);
        let plating = select(vec3<f32>(0.32, 0.33, 0.35) * tint, vec3<f32>(0.16, 0.165, 0.18), part == 1);
        albedo = plating * metal;
    } else if (part == 2) {
        albedo = vec3<f32>(0.05, 0.05, 0.06);
    } else if (part == 15) {
        if (tone > 0.99) {
            // A berth's status: amber, green or red.
            glow = select(select(vec3<f32>(1.5, 0.75, 0.1), vec3<f32>(0.2, 1.5, 0.4), look.x > 0.5), vec3<f32>(1.6, 0.12, 0.08), look.x > 1.5);
        } else if (tone > 0.9) {
            // Red: a tail light, or a beacon blinking.
            glow = vec3<f32>(1.6, 0.13, 0.08) * select(1.0, step(fract(t * 0.9 + p.x * 0.013), 0.3), tone > 0.96);
        } else if (tone > 0.77) {
            // The guide along the tunnel's deck: cyan, pulses flowing in.
            let pulse = 1.0 - fract(t * 1.6 - p.z / 375.0);
            glow = vec3<f32>(0.2, 0.8, 1.0) * (0.5 + 1.2 * pulse * pulse * pulse);
        } else {
            glow = neon(floor(tone * 4.0)) * 1.2;
        }
    } else if (part == 11) {
        glow = vec3<f32>(1.1, 1.15, 1.25);
    } else if (part == 12) {
        albedo = mix(vec3<f32>(0.03), vec3<f32>(0.8, 0.6, 0.05), step(0.5, fract((q.x + q.y + q.z) * 0.25)));
    } else if (part == 7) {
        // Windows in a row: a ship's, a cabin's, or a tower's (dimmer by
        // its tone, for a town far off; brighter in a city's core, its
        // `look.x` negative).
        let pane = step(0.2, fract(q.y * 0.2)) * step(0.25, fract((q.z + q.x) * 0.13));
        glow = vec3<f32>(1.0, 0.82, 0.55) * (0.12 + 0.7 * pane) * tone * (1.0 - 1.2 * min(look.x, 0.0));
        albedo = vec3<f32>(0.02);
    } else {
        // A ship's engines and lamps: dim at rest, bright under way.
        albedo = vec3<f32>(0.2, 0.21, 0.23);
        let lamp = select(select(vec3<f32>(0.3, 0.5, 0.8), vec3<f32>(0.9, 0.15, 0.08), part == 4), vec3<f32>(0.2, 0.9, 0.35), part == 8);
        glow = lamp * (0.25 + 1.3 * look.x) * select(1.0, 1.6, part == 6);
    }
    return albedo * light + glow;
}

// The metal at `q` (the surface in its own frame, metres) facing `n`, as
// a shade about 1, from photographs (ambientCG, CC0: `assets/textures/
// CREDITS.md`) kept a channel each in a layer of the base's maps (`bake::
// write_metals`): plates of many sizes, for walls and hulls; a riveted
// grid, for frames; diamond tread, for decks. Laid on the two axes most
// across the surface, a tile of the plates `span` metres.
fn base_metal(q: vec3<f32>, n: vec3<f32>, part: i32, span: f32) -> f32 {
    let a = abs(n);
    var uv = q.xz;
    if (a.x > a.y && a.x > a.z) {
        uv = q.yz;
    } else if (a.z > a.y) {
        uv = q.xy;
    }
    let deck = part == 0 && a.y > 0.7;
    let tile = select(select(span, span * 0.6, part == 1), span * 0.35, deck);
    // The layer holds each photograph four times across and twice down.
    let metal = textureSample(city_map, city_sampler, uv / tile * vec2<f32>(0.25, 0.5), METAL_LAYER).rgb;
    // Each one's own mean brought to 1.
    return select(select(metal.r * 3.6, metal.g * 9.5, part == 1), metal.b * 6.3, deck);
}

// A starbase's structures and craft, and the megastructure and its
// entrance (`models::BASE`, `BASE_SURFACE`): an entry point of their own,
// as they read the base's maps where the other models read the rocks'.
// Lit from its vertices' light; hazed by where it stands.
@fragment
fn fs_base(in: ModelOut) -> @location(0) vec4<f32> {
    let n = normalize(in.normal);
    let part = i32(in.part + 0.5);
    let at = ev_ship() - in.toward;
    let q = in.local * in.size;
    if (MODEL == 7) {
        // No photograph on the megastructure: it is seen from kilometres
        // off, and the read cost this shader the Pi's four threads.
        let colour = base_outside(q, n, part, at, in.ao, in.lit.rgb, 1.0);
        return vec4<f32>(base_fog(colour, length(in.toward), at), 1.0);
    }
    let metal = base_metal(q, n, part, 14.0);
    let colour = base_structure(q, n, part, at, in.ao, in.look, base_lit(in.lit, at, n), metal);
    // What is not within the tunnel's wall stands out in the world.
    return vec4<f32>(base_haze(colour, length(in.toward), at, 1.0 - in.lit.w), 1.0);
}
