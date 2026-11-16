//! A starbase as the frame draws it (`tour::BaseScene`): a megastructure
//! standing over a living world, the mouth in the dish at its top; the
//! stepped entrance down from the mouth; the airlock; the tunnel
//! along the axis of the base's hollow interior (a world on the inside of
//! a cylinder kilometres across, seen through the tunnel's windows, ships
//! and cars flying past them); and the port at the tunnel's end, out in
//! the open of that world: catwalks, ships moored along them, one berth
//! the ship's own.
//!
//! Everything is in the base's own frame, metres: the mouth at the origin,
//! +z in down the entrance and the tunnel to the port, +y up, +x right.
//! The encounter shaders draw in that frame as they draw a stopover's
//! (`EV_SHIP` the camera, `EV_X..EV_Z` its axes), so the model pipeline
//! (family 6, `starbase/surface.wesl`), the panes (`starbase/pane.wesl`)
//! and the effects (`vs_fx_*`) need nothing else of it but the `EV_BASE`
//! rows.
//!
//! - `mesh`: its pieces, built from flat quads.
//! - `scene`: the shaders' rows and every mesh placed for a moment.
//! - `port`: the port's catwalks and what is moored along them.
//! - `traffic`: the ships and cars flying past the tunnel and round the port.
//! - `city`: the cities of the world inside, the same as `city.wesl` paints them.
//! - `world`: their buildings, placed for the moment.
//! - `lights`: its beacons, sparks and beams.

pub mod city;
mod lights;
pub mod mesh;
mod port;
mod scene;
mod traffic;
mod world;

pub(super) use lights::effects;
pub(super) use scene::{PANE_VERTICES, PANES, instances, port_shown, rows, segments};

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A number 0..1 of `n`'s own.
fn hash(n: u32) -> f32 {
    let mut h = n.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

#[cfg(test)]
pub(super) fn test_scene(
    stage: super::tour::BaseStage,
    local: f64,
    ship: [f64; 3],
) -> super::tour::BaseScene {
    super::tour::BaseScene {
        ship,
        heading: [0.0, 0.0, 1.0],
        basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
        stage,
        local,
        clock: 100.0,
        inside: true,
        sky: false,
        sun: [0.0, 0.6, -0.8],
        seed: 5,
        traffic: None,
        shake: 0.0,
        doors: super::tour::starbase::doors(stage, local),
    }
}
