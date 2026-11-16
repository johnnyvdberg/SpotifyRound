// The tunnel's window panes, and the shell round the port: both show the
// world inside the base (`world.wesl`) along the view's ray, behind
// everything that flies or stands in front of them.

import package::solar::noise::{PI, TAU};
import package::solar::frame::params;
import package::solar::encounter::{ev_ship, from_ev, to_ev};
import package::solar::starbase::common::base_haze;
import package::solar::starbase::world::{
    BLOCKS_LAYER, CLOUDS_LAYER, RELIEF_LAYER, WORLD_AXIS, WORLD_CLOUDS, WORLD_GROUND, WORLD_TILE,
};
import constants::EV_BASE;

struct Pane {
    @builtin(position) clip: vec4<f32>,
    // Where on the pane, in the base's frame.
    @location(0) at: vec3<f32>,
};

// Where a segment's panes lie in its wall, and its end ribs and mullions
// (`starbase::mesh::tunnel`: `WALL`, `RIB`, `MULLION`).
const PANE_OUT: f32 = 2.0;
const PANE_RIB: f32 = 8.0;
const PANE_MULLION: f32 = 1.5;

// A point of the base's frame on the frame, at the far end of the depth.
fn far_clip(at: vec3<f32>) -> vec4<f32> {
    let w = from_ev(at - ev_ship());
    let cam = vec3<f32>(dot(w, params.right.xyz), dot(w, params.up.xyz), dot(w, params.forward.xyz));
    return vec4<f32>(cam.x / params.frame.y, cam.y / params.frame.y, 0.9995 * cam.z, cam.z);
}

// A pane: six to a segment (three bands of two bays, long and shallow in
// the wall: they are seen at a glance, down the tunnel), each four facets
// round its band, a little wider and longer than its opening so the wall
// hides its edges. The segment `instance / 6` past the first drawn.
@vertex
fn vs_pane(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Pane {
    let row = params.ev[EV_BASE + 1];
    let segs = params.ev[EV_BASE + 2];
    let s = segs.y + f32(instance / 6u);
    let k = instance % 6u;
    let band = f32(k / 2u);
    let bay = f32(k % 2u);
    let zc = row.x + (s + 0.5) * segs.x;
    let u = clamp((zc - row.x) / row.y, 0.0, 1.0);
    let xc = row.z * (u - sin(TAU * u) / TAU);
    let turn = atan(row.z / row.y * (1.0 - cos(TAU * u)));
    let half = segs.x * 0.5 - PANE_RIB;
    let bay_length = half;
    let middle = -half + bay_length * (bay + 0.5);
    let along = bay_length * 0.5 - PANE_MULLION + 1.0;
    let spread = PI * 3.0 / 16.0 + 0.012;
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[vertex % 6u];
    let facet = f32(vertex / 6u);
    let a = band * PI * 0.5 - spread + 2.0 * spread * (facet + c.x) / 4.0;
    let r = row.w + PANE_OUT;
    let local = vec3<f32>(r * cos(a), r * sin(a), middle + along * (2.0 * c.y - 1.0));
    let sn = sin(turn);
    let cs = cos(turn);
    let at = vec3<f32>(xc + local.x * cs + local.z * sn, local.y, zc - local.x * sn + local.z * cs);
    var out: Pane;
    out.clip = far_clip(at);
    out.at = at;
    return out;
}

// The shell round the port, where the world shows in the open: from in
// the tunnel, a square over its far end; from out in the port, the whole
// frame, but for the part the ship's own bow covers while it is docked (a
// notch up from the frame's foot, a little inside the bow's outline): on
// the Pi a pixel drawn behind the bow is paid for all the same.
@vertex
fn vs_shell(@builtin(vertex_index) vertex: u32) -> Pane {
    let eye = ev_ship();
    let end = params.ev[EV_BASE + 2].w;
    var at: vec3<f32>;
    if (eye.z < end) {
        var corners = array<vec2<f32>, 6>(
            vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
            vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
        );
        let c = corners[vertex % 6u];
        let reach = params.ev[EV_BASE + 1].w + 14.0;
        at = vec3<f32>(params.ev[EV_BASE + 1].z + c.x * reach, c.y * reach, end);
        if (vertex >= 6u) {
            at = vec3<f32>(params.ev[EV_BASE + 1].z, 0.0, end);
        }
    } else {
        // A fan round a point above the notch: the frame's outline, the
        // notch's foot at the frame's when there is no bow.
        let bow = params.ev[EV_BASE].x > 2.5;
        let foot = select(-1.0, -0.705, bow);
        let tip = select(-1.0, -0.3, bow);
        var outline = array<vec2<f32>, 6>(
            vec2<f32>(-1.0, 1.0), vec2<f32>(1.0, 1.0), vec2<f32>(1.0, foot),
            vec2<f32>(0.08, tip), vec2<f32>(-0.08, tip), vec2<f32>(-1.0, foot),
        );
        var s = vec2<f32>(0.0, 0.4);
        let k = vertex / 3u;
        if (vertex % 3u == 1u) {
            s = outline[k];
        } else if (vertex % 3u == 2u) {
            s = outline[(k + 1u) % 6u];
        }
        let ray = params.forward.xyz + (params.right.xyz * s.x + params.up.xyz * s.y) * params.frame.y;
        at = eye + to_ev(ray) * 300.0;
    }
    var out: Pane;
    out.clip = far_clip(at);
    out.at = at;
    return out;
}

// The base's maps, baked once (`bake::bake_city`, `world.wesl`), with
// their mip levels.
@group(2) @binding(4) var city_map: texture_2d_array<f32>;
@group(2) @binding(5) var city_sampler: sampler;

// What the world's far end fades into, and the haze of the port out in it.
const WORLD_HAZE: vec3<f32> = vec3<f32>(0.075, 0.095, 0.14);

// Where round the cylinder a point across it lies, 0..1: not its angle
// but near enough, for a division.
fn around(q: vec2<f32>) -> f32 {
    let s = q.x / (abs(q.x) + abs(q.y));
    return select(3.0 + s, 1.0 - s, q.y >= 0.0) * 0.25;
}

// How far the world's highest ground stands above its lowlands, metres.
const WORLD_RELIEF: f32 = 650.0;

// The world along the ray from `o`, `time` seconds into the visit: its
// clouds, drifting, and its tallest towers' tops; its ground under them,
// met nearer where it stands high (so hills hide what is behind them and
// slide against the lowlands as the ship moves), lights running along its
// highways; and the haze that far. `fine`: with the blocks laid over the
// ground where it is built up, for the tunnel's windows; out in the port,
// where the world fills the frame and is all far off, without (the open
// world was 5 ms of the port's 10.7 on the Pi).
fn world(o: vec3<f32>, d: vec3<f32>, seed: f32, time: f32, fine: bool) -> vec3<f32> {
    let o2 = vec2<f32>(o.x - WORLD_AXIS, o.y);
    let a = max(dot(d.xy, d.xy), 1e-6);
    let b = dot(o2, d.xy);
    let t0 = (-b + sqrt(b * b - a * (dot(o2, o2) - WORLD_GROUND * WORLD_GROUND))) / a;
    // The clouds' level along the same ray: in proportion, as the camera
    // is never far from the axis.
    let t1 = t0 * (WORLD_CLOUDS / WORLD_GROUND);
    let shift = vec2<f32>(0.0, seed * 0.0173);
    let level = log2(t0);
    // The mip level, from how far: the ground's texels are thirteen metres.
    let coarse = max(level - 14.0, 0.0);
    let hit = o2 + d.xy * t0;
    let relief = textureSampleLevel(city_map, city_sampler, vec2<f32>(around(hit), (o.z + d.z * t0) / WORLD_TILE) + shift, RELIEF_LAYER, coarse).rgb * 2.0;
    // Back along the ray to where it met ground that high: the faster the
    // ray is leaving the axis, the less far.
    let t = t0 - relief.r * WORLD_RELIEF * WORLD_GROUND / max(dot(d.xy, hit), 0.12 * WORLD_GROUND);
    let g = vec2<f32>(around(o2 + d.xy * t), (o.z + d.z * t) / WORLD_TILE) + shift;
    let c = vec2<f32>(around(o2 + d.xy * t1) + time * 0.0005, (o.z + d.z * t1) / WORLD_TILE) + shift;
    let ground = textureSampleLevel(city_map, city_sampler, g, 0, coarse);
    let clouds = textureSampleLevel(city_map, city_sampler, c, CLOUDS_LAYER, coarse);
    var land = ground.rgb;
    if (fine) {
        land += textureSampleLevel(city_map, city_sampler, g * 8.0, BLOCKS_LAYER, max(level - 10.6, 0.0)).rgb * ground.a;
    }
    let traffic = relief.g * step(0.72, fract(relief.b * 70.0 - time * 0.3));
    // The clouds' colour is stored already times their cover.
    let colour = (land * 2.0 + vec3<f32>(1.0, 0.85, 0.6) * traffic) * (1.0 - clouds.a) + clouds.rgb * 2.0;
    // The same haze as the buildings standing on it (`base_haze`).
    return mix(colour, WORLD_HAZE, t0 / (t0 + 9000.0));
}

// A pane: the world through its glass, a faint tint; and the tunnel's haze
// this side of it.
@fragment
fn fs_pane(in: Pane) -> @location(0) vec4<f32> {
    let eye = ev_ship();
    let colour = world(eye, normalize(in.at - eye), params.ev[EV_BASE].z, params.ev[EV_BASE].y, true);
    return vec4<f32>(base_haze(colour * 0.92 + vec3<f32>(0.008, 0.012, 0.018), length(in.at - eye), in.at, 0.0), 1.0);
}

// The shell round the port: the world in the open.
@fragment
fn fs_shell(in: Pane) -> @location(0) vec4<f32> {
    let eye = ev_ship();
    return vec4<f32>(world(eye, normalize(in.at - eye), params.ev[EV_BASE].z, params.ev[EV_BASE].y, false), 1.0);
}
