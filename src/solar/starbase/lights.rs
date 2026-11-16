//! The base's lights that are not its meshes' own, drawn as the battles'
//! effects are: seen from outside, the corridor's beacons running in to
//! the mouth and the ship given priority, its engines; in the port, the
//! beacons at the ship's own dock.

use glam::Vec3;

use super::super::encounters::{FX_BRIGHT, FX_DIM, FX_GLINT, Fx, fx_falloff};
use super::super::tour::starbase::{SERVICE, tunnel_end};
use super::super::tour::{BaseScene, BaseStage};
use super::scene::{cradle, ship};

/// A glint at `p`, `size` pixels.
fn glint(out: &mut [Vec<Fx>; 6], p: Vec3, size: f32, colour: [f32; 3]) {
    let peak = colour[0].max(colour[1]).max(colour[2]);
    if peak < FX_DIM {
        return;
    }
    let reach = |floor| {
        (6.0 * size * fx_falloff(0.15 * peak, floor))
            .max(size * (2.0 * fx_falloff(2.0 * peak, floor)).sqrt())
            + 1.0
    };
    out[FX_GLINT].push(Fx {
        a: [p.x, p.y, p.z, size],
        colour: [colour[0], colour[1], colour[2], 0.0],
        reach: [reach(FX_DIM), 0.0, 0.0, reach(FX_BRIGHT) / reach(FX_DIM)],
        ..Default::default()
    });
}

/// The base's lights this frame, by kind (`FX_KINDS`).
pub fn effects(b: &BaseScene) -> [Vec<Fx>; 6] {
    let mut out: [Vec<Fx>; 6] = Default::default();
    let t = b.clock as f32;
    let eye = ship(b);
    if eye.z < 0.0 {
        // The corridor's beacons, two lines of them, a pulse running in.
        if eye.z > -90_000.0 {
            for k in 1..=20 {
                let z = -2000.0 * k as f32;
                if z < eye.z + 300.0 {
                    continue;
                }
                let pulse = (1.0 - ((t * 0.8 - k as f32 * 0.08).rem_euclid(1.0))).powi(6);
                for x in [-700.0f32, 700.0] {
                    glint(
                        &mut out,
                        Vec3::new(x, 0.0, z),
                        1.4,
                        [0.25 + 1.6 * pulse, 0.9 + 1.2 * pulse, 0.6 + 0.8 * pulse],
                    );
                }
            }
        }
        if let Some([x, y, z, shown]) = b.traffic {
            let at = Vec3::new(x as f32, y as f32, z as f32);
            let s = shown as f32;
            glint(&mut out, at, 2.6, [0.8 * s, 0.9 * s, 1.6 * s]);
            let blink = if (t * 1.5).fract() < 0.15 { 1.0 } else { 0.0 };
            glint(
                &mut out,
                at + Vec3::new(30.0, 0.0, 0.0),
                1.2,
                [1.4 * s * blink, 0.1 * s * blink, 0.05 * s * blink],
            );
        }
        return out;
    }
    if !b.inside || eye.z < (tunnel_end() - 600.0) as f32 {
        return out;
    }
    let c = cradle();
    // The beacons at the ship's own dock: amber at the cradle's corners
    // while the clamps work, green once the service is done.
    let done = matches!(b.stage, BaseStage::Undock)
        || (b.stage == BaseStage::Service && b.local > SERVICE * 0.85);
    let colour = if done {
        [0.2, 1.5, 0.4]
    } else {
        [1.6, 0.8, 0.1]
    };
    if (t * 1.2).fract() < 0.5 {
        for (sx, sz) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            glint(
                &mut out,
                c + Vec3::new(sx * 38.0, -33.0, sz * 48.0),
                1.8,
                colour,
            );
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::super::test_scene;
    use super::*;
    use crate::solar::tour::starbase::{BEND, CRADLE, CRADLE_DROP};

    #[test]
    fn the_dock_and_the_corridor_have_their_beacons() {
        let dock = [BEND, -CRADLE_DROP, tunnel_end() + CRADLE];
        let fx = effects(&test_scene(BaseStage::Service, 3.0, dock));
        let total: usize = fx.iter().map(|v| v.len()).sum();
        assert!((4..=crate::solar::encounters::FX_INSTANCES).contains(&total));
        let mut outside = test_scene(BaseStage::Approach, 30.0, [0.0, 0.0, -50_000.0]);
        outside.inside = false;
        assert!(!effects(&outside)[FX_GLINT].is_empty());
    }
}
