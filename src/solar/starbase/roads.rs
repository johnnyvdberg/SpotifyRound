//! The traffic on the world's roads, as lights: on each road piece of the
//! city (`city::City::segments`) cars a lane each way, headlights warm white
//! one way, tail lights red the other, running at their own pace along the
//! network. The nearest in view are kept, within the effects' budget, a
//! glint each, hazed as the ground is; the map's own streaks carry the rest
//! (`pane.wesl`).

use glam::Vec3;

use super::super::tour::BaseScene;
use super::super::tour::starbase::{FUNNEL, far_wall};
use super::city::{self, ALONG, GROUND};
use super::scene::{seen, ship, world_shown};
use super::sections;

/// The cylinder's axis across the base's frame (`WORLD_AXIS`).
const AXIS: f32 = 200.0;
/// The most cars lit a frame (the effects hold `FX_INSTANCES` in all).
pub(super) const BUDGET: usize = 96;
/// Metres between cars on a lane, highways and others, their speeds
/// (m/s), and how far a lane lies from the road's middle.
const SPACING: [f32; 2] = [230.0, 520.0];
const SPEED: [f32; 2] = [32.0, 15.0];
const LANE: f32 = 4.0;
/// How far off a car's light is drawn.
const FAR: f32 = 14_000.0;

/// A car's light: where, how big (pixels), its colour.
pub(super) struct Car {
    pub at: Vec3,
    pub size: f32,
    pub colour: [f32; 3],
}

/// The cars lit this frame, the nearest first.
pub(super) fn cars(b: &BaseScene) -> Vec<Car> {
    if !world_shown(b) {
        return Vec::new();
    }
    let eye = ship(b);
    let t = b.clock as f32;
    // The map's own place along this base (`shift` in `pane.wesl`).
    let shift = sections::shift(b);
    let (near_end, far_end) = (FUNNEL as f32 + 200.0, (far_wall() - b.fold) as f32 - 200.0);
    let tiles = ((eye.z - FAR) / ALONG + shift).floor() as i32
        ..=((eye.z + FAR) / ALONG + shift).ceil() as i32;
    let c = city::city();
    let place = |m: [f32; 2], n: i32| {
        // Mirrored round in the lengths laid so (`sections`).
        let flip = if sections::mirrored(n, shift) {
            -1.0
        } else {
            1.0
        };
        let theta = flip * m[0] / GROUND;
        let (s, co) = theta.sin_cos();
        // Lifted a few metres off the ground, toward the axis.
        let r = GROUND - 3.0;
        Vec3::new(
            AXIS + r * co,
            r * s,
            (m[1] / ALONG - shift + n as f32) * ALONG,
        )
    };
    let mut found: Vec<(f32, Car)> = Vec::new();
    for n in tiles {
        for (s, r) in c.segments.iter().zip(&c.roads) {
            let (a, e) = (place([s[0], s[1]], n), place([s[2], s[3]], n));
            let middle = (a + e) * 0.5;
            let half = r[3] * 0.5;
            if (middle - eye).length() > FAR + half || !seen(b, middle, half + 50.0) {
                continue;
            }
            let kind = if r[1] > 0.5 { 0 } else { 1 };
            let way = [
                (s[2] - s[0]) / r[3].max(1e-3),
                (s[3] - s[1]) / r[3].max(1e-3),
            ];
            // A lane each way: along the network, and back.
            for (sign, colour) in [(1.0f32, [1.5, 1.3, 0.95]), (-1.0, [1.4, 0.12, 0.06])] {
                let side = [-way[1] * LANE * sign, way[0] * LANE * sign];
                // Cars stand where `along - sign * speed * t` is a whole
                // number of spacings (the way back half a spacing over):
                // the first on this piece, from its start.
                let offset = if sign < 0.0 { 0.5 * SPACING[kind] } else { 0.0 };
                let mut d = (offset + sign * SPEED[kind] * t - r[2]).rem_euclid(SPACING[kind]);
                while d < r[3] {
                    let f = d / r[3];
                    let m = [
                        s[0] + (s[2] - s[0]) * f + side[0],
                        s[1] + (s[3] - s[1]) * f + side[1],
                    ];
                    let at = place(m, n);
                    d += SPACING[kind];
                    if at.z < near_end || at.z > far_end {
                        continue;
                    }
                    let away = (at - eye).length();
                    if away > FAR {
                        continue;
                    }
                    // Hazed as the ground under it is (`world` in `pane`).
                    let clear = 1.0 - away / (away + 9000.0);
                    let k = clear * if kind == 0 { 1.0 } else { 0.7 };
                    found.push((
                        away,
                        Car {
                            at,
                            size: 1.0,
                            colour: [colour[0] * k, colour[1] * k, colour[2] * k],
                        },
                    ));
                }
            }
        }
    }
    if found.len() > BUDGET {
        found.select_nth_unstable_by(BUDGET, |a, b| a.0.total_cmp(&b.0));
        found.truncate(BUDGET);
    }
    found.sort_by(|a, b| a.0.total_cmp(&b.0));
    found.into_iter().map(|(_, car)| car).collect()
}

#[cfg(test)]
mod tests {
    use super::super::test_scene;
    use super::*;
    use crate::solar::tour::BaseStage;
    use crate::solar::tour::starbase::{BEND, CRADLE, CRADLE_DROP, tunnel_end};

    #[test]
    fn the_roads_have_their_traffic_in_view_and_it_moves() {
        let dock = [BEND, -CRADLE_DROP, tunnel_end() + CRADLE];
        let mut b = test_scene(BaseStage::Service, 3.0, dock);
        b.doors = [1.0, 1.0];
        let now = cars(&b);
        assert!(now.len() > 20 && now.len() <= BUDGET, "{}", now.len());
        b.clock += 2.0;
        let later = cars(&b);
        assert!(
            now.iter()
                .zip(&later)
                .any(|(a, b)| (a.at - b.at).length() > 1.0)
        );
        // Turned to the far side, none.
        b.far = true;
        assert!(cars(&b).is_empty());
    }
}
