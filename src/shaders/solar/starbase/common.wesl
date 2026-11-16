// A starbase as the frame draws it (`solar/starbase`), in the base's own
// frame, metres: the mouth at the origin, +z in down the entrance and the
// tunnel to the port, +y up, +x right; above the mouth (z < 0) the city
// round it. What every part of it shares: its depth, its fog and the
// tunnel's line.
//
// Its meshes are family 6 of the models (`surface.wesl`); the tunnel's
// panes and the port's shell, the world inside behind them, are `pane.wesl`,
// reading the maps `world.wesl` bakes. Rows `EV_BASE..+3`: whether it is drawn (1 over
// the body it stands on, 2 alone), seconds into the visit, the skylines'
// seed and the port's green once serviced; the entrance's depth, the
// tunnel's length, its bend and its bore; a segment's length, the first
// segment with panes and how many, where the port begins;
// the way to the star, and the depth's reach.

import package::solar::noise::TAU;
import package::solar::frame::params;
import package::solar::starbase::pane::{WORLD_FOG, WORLD_HAZE};
import constants::{EV_BASE, EV_SHIP};

// Drawn over the moon-sized body it stands on (whose depth is its place in
// the bodies' order, not a distance): in front of every body.
fn base_over() -> bool {
    let mode = params.ev[EV_BASE].x;
    return mode > 0.5 && mode < 1.5;
}

// The share of the depth range its meshes and lights are drawn in: all of
// it, or, over the body, what lies in front of the nearest body's rank.
fn base_depth() -> f32 {
    return select(1.0, 0.9 / (params.frame.w + 1.0), base_over());
}

// How far the depth resolves well, metres: a view `z` away is at
// `z / (z + reach)` of the range. A few hundred metres inside, kilometres
// above the city.
fn base_reach() -> f32 {
    return select(400.0, params.ev[EV_BASE + 3].w, params.ev[EV_BASE].x > 0.5);
}

// The haze between the camera and `p` on the base's surface or down its
// entrance, `distance` away: only the part of the way below the mouth
// (above it is space).
const TUNNEL_FOG: vec3<f32> = vec3<f32>(0.012, 0.014, 0.024);
fn base_fog(colour: vec3<f32>, distance: f32, p: vec3<f32>) -> vec3<f32> {
    let oz = min(params.ev[EV_SHIP].z, 0.0);
    let through = distance * clamp(p.z / (p.z - oz + 1e-3), 0.0, 1.0);
    return mix(colour, TUNNEL_FOG, through / (through + 1500.0));
}

// The haze inside, for the base's structures and panes, which are only
// ever seen from inside: thinner out in the port and for what stands out
// in the world (`open`), and the world's there.
fn base_haze(colour: vec3<f32>, distance: f32, p: vec3<f32>, open: f32) -> vec3<f32> {
    let port = max(step(params.ev[EV_BASE + 2].w, p.z), open);
    let thick = select(distance / (distance + 1500.0), 1.0 - exp(-distance / WORLD_FOG), port > 0.5);
    return mix(colour, mix(TUNNEL_FOG, WORLD_HAZE, port), thick);
}

// The tunnel's line: how far right it is at depth `z` (`tour::starbase::bend`).
fn base_bend(z: f32) -> f32 {
    let row = params.ev[EV_BASE + 1];
    let u = clamp((z - row.x) / row.y, 0.0, 1.0);
    return row.z * (u - sin(TAU * u) / TAU);
}
