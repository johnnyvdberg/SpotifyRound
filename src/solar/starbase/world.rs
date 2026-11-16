//! The buildings of the world inside the base as meshes, so they slide
//! against the ground and each other as the ship moves where the maps
//! alone lie flat. Each stands on a lot of the cities `starbase::city`
//! works out (and `city.wesl` paints, its roof among them), listed once.
//!
//! Detail by distance: a building is kept while it stands about three
//! pixels tall, the tallest first when there are more than the frame can
//! take, and shrinks into its painted roof as it nears the cut rather than
//! popping. Beyond `NEAR` the towers are plain; the map alone beyond `FAR`.

use glam::{Mat3, Quat, Vec3};
use std::sync::{Once, OnceLock};

use super::super::models::{Instance, Key};
use super::super::tour::BaseScene;
use super::super::tour::starbase::FUNNEL;
use super::city::{self, ALONG, GROUND, ROUND, kind, shape};
use super::mesh::piece;
use super::scene::{base, instance, seen, ship};
use super::sections;

/// The cylinder's axis across the base's frame (`WORLD_AXIS`).
const AXIS: f32 = 200.0;
/// How tall for how far a building must stand to be drawn: three of the
/// frame's pixels (a 50° field over 1080), and full size from half as
/// tall again.
const SHOWN: f32 = 3.0 * 2.0 * 0.4663 / 1080.0;
const FADE: f32 = 1.5;
/// The most buildings placed for a frame.
const BUDGET: usize = 2200;
/// Within this, every building in its own shape; beyond, towers plain.
const NEAR: f32 = 6500.0;
const FAR: f32 = 20_000.0;
/// The tallest, the skylines' peaks, are drawn this far, and so tall.
const FAR_TALL: f32 = 32_000.0;
const TALL: f32 = 150.0;
/// The squares the buildings are kept in, round and along a tile.
const SQUARES: [usize; 2] = [24, 12];

/// Muted hulls, low in saturation, by district (the core's cool greys,
/// the works' dark, the high-rise's warm stone), two each.
const TINTS: [[[f32; 3]; 2]; 3] = [
    [[1.12, 1.18, 1.3], [1.25, 1.25, 1.3]],
    [[0.6, 0.6, 0.62], [0.7, 0.66, 0.62]],
    [[0.94, 0.88, 0.8], [0.88, 0.84, 0.82]],
];

/// A building ready to place: its way out from the axis, place along a
/// tile, height, meshes near and far, their scale for its height, its
/// turn and its tint.
struct Ready {
    out: [f32; 2],
    along: f32,
    height: f32,
    near: u32,
    far: u32,
    scale: f32,
    turn: Quat,
    /// Its turn where its length of the map is laid mirrored round
    /// (`sections::mirrored`), standing at `out` mirrored.
    mirrored: Quat,
    look: [f32; 4],
}

/// The buildings, a list a square of the grid.
fn ready() -> Option<&'static Vec<Vec<Ready>>> {
    static READY: OnceLock<Vec<Vec<Ready>>> = OnceLock::new();
    static START: Once = Once::new();
    // Worked out in the background the first time they are asked for.
    START.call_once(|| {
        std::thread::spawn(|| {
            let _ = READY.set(prepare());
        });
    });
    READY.get()
}

fn prepare() -> Vec<Vec<Ready>> {
    let mut squares: Vec<Vec<Ready>> = (0..SQUARES[0] * SQUARES[1]).map(|_| Vec::new()).collect();
    for b in city::city().buildings() {
        let theta = b.at[0] / GROUND;
        let (s, c) = theta.sin_cos();
        let up = -Vec3::new(c, s, 0.0);
        let east = Vec3::new(-s, c, 0.0);
        let x = east * b.turn.cos() + Vec3::Z * b.turn.sin();
        let turn = Quat::from_mat3(&Mat3::from_cols(x, up.cross(x), up));
        // Mirrored round: at the angle's negative, its way along the
        // ground reflected (a turn, not a reflection, of its mesh).
        let up_m = -Vec3::new(c, -s, 0.0);
        let x_m = -Vec3::new(s, c, 0.0) * b.turn.cos() + Vec3::Z * b.turn.sin();
        let mirrored = Quat::from_mat3(&Mat3::from_cols(x_m, up_m.cross(x_m), up_m));
        let tower = matches!(b.shape, shape::SLAB | shape::TAPER | shape::ROUND);
        let near = match b.shape {
            shape::BLOCK => piece::TOWER_BLOCK,
            shape::SHED => piece::SHED,
            shape::SLAB => piece::TOWER_SLAB,
            shape::TAPER => piece::TOWER_TAPER,
            shape::ROUND => piece::TOWER_ROUND,
            _ => piece::TOWER_NEEDLE,
        };
        let district = match b.kind {
            kind::CORE => 0,
            kind::WORKS => 1,
            _ => 2,
        };
        let tint = TINTS[district][(b.tone * 1.99) as usize];
        let gx = ((b.at[0] / ROUND * SQUARES[0] as f32) as usize).min(SQUARES[0] - 1);
        let gz = ((b.at[1] / ALONG * SQUARES[1] as f32) as usize).min(SQUARES[1] - 1);
        squares[gz * SQUARES[0] + gx].push(Ready {
            out: [c, s],
            along: b.at[1],
            height: b.height,
            near,
            far: if tower { piece::TOWER_FAR } else { near },
            scale: 1.0 / city::extent(b.shape)[2],
            turn,
            mirrored,
            // The core's windows lit brighter (`base_structure` reads a
            // negative `look.x` so; a ship's is 0 or more).
            look: [
                if district == 0 { -1.0 } else { 0.0 },
                tint[0],
                tint[1],
                tint[2],
            ],
        });
    }
    squares
}

pub(super) fn towns(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    if let Some(squares) = ready() {
        place(b, squares, out);
    }
}

fn place(b: &BaseScene, squares: &[Vec<Ready>], out: &mut Vec<(Key, Instance)>) {
    let eye = ship(b);
    // The map's own place along this base (`shift` in `pane.wesl`).
    let shift = sections::shift(b);
    let tiles = ((eye.z - FAR_TALL) / ALONG + shift).floor() as i32
        ..=((eye.z + FAR_TALL) / ALONG + shift).ceil() as i32;
    let size = [ROUND / SQUARES[0] as f32, ALONG / SQUARES[1] as f32];
    let mut seen_now: Vec<(f32, f32, Vec3, &Ready, bool)> = Vec::new();
    for n in tiles {
        let flip = if sections::mirrored(n, shift) {
            -1.0
        } else {
            1.0
        };
        for gz in 0..SQUARES[1] {
            for gx in 0..SQUARES[0] {
                let theta = (gx as f32 + 0.5) * size[0] / GROUND;
                let z = ((gz as f32 + 0.5) * size[1] / ALONG - shift + n as f32) * ALONG;
                let middle = Vec3::new(AXIS + GROUND * theta.cos(), flip * GROUND * theta.sin(), z);
                if (middle - eye).length() > FAR_TALL + size[0] || !seen(b, middle, size[0]) {
                    continue;
                }
                for r in &squares[gz * SQUARES[0] + gx] {
                    let z = (r.along / ALONG - shift + n as f32) * ALONG;
                    if z < FUNNEL as f32 + 200.0 {
                        continue;
                    }
                    let at = Vec3::new(AXIS + GROUND * r.out[0], flip * GROUND * r.out[1], z);
                    let away = (at - eye).length();
                    let tall = r.height / away;
                    if tall > SHOWN && (away < FAR || (away < FAR_TALL && r.height > TALL)) {
                        seen_now.push((tall, away, at, r, flip < 0.0));
                    }
                }
            }
        }
    }
    // The tallest in the view first, when there are more than the budget.
    let mut cut = SHOWN;
    if seen_now.len() > BUDGET {
        seen_now.select_nth_unstable_by(BUDGET, |a, b| b.0.total_cmp(&a.0));
        seen_now.truncate(BUDGET);
        cut = seen_now
            .iter()
            .map(|s| s.0)
            .fold(f32::MAX, f32::min)
            .max(SHOWN);
    }
    for (tall, away, at, r, mirrored) in seen_now {
        // Grown out of its roof from the cut, full size half as tall again.
        let t = ((tall - cut) / (cut * (FADE - 1.0))).clamp(0.0, 1.0);
        let grown = t * t * (3.0 - 2.0 * t);
        if grown < 0.02 {
            continue;
        }
        let key = if away > NEAR { r.far } else { r.near };
        out.push((
            base(key),
            instance(
                at,
                r.height * r.scale * grown,
                if mirrored { r.mirrored } else { r.turn },
                r.look,
            ),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_scene;
    use super::*;
    use crate::solar::tour::BaseStage;
    use crate::solar::tour::starbase::{BEND, CRADLE, tunnel_end};

    fn placed(ship: [f64; 3], stage: BaseStage, local: f64) -> Vec<(Key, Instance)> {
        let squares = prepare();
        let mut out = Vec::new();
        place(&test_scene(stage, local, ship), &squares, &mut out);
        out
    }

    #[test]
    fn buildings_stand_in_view_from_the_dock_and_from_the_tunnel() {
        let dock = placed(
            [BEND, -15.0, tunnel_end() + CRADLE],
            BaseStage::Service,
            3.0,
        );
        let tunnel = placed([0.0, 0.0, tunnel_end() * 0.6], BaseStage::Tunnel, 0.5);
        eprintln!(
            "{} from the dock, {} from the tunnel",
            dock.len(),
            tunnel.len()
        );
        let eye = Vec3::new(BEND as f32, -15.0, (tunnel_end() + CRADLE) as f32);
        let mut by = [0usize; 6];
        for (_, i) in &dock {
            let away = (Vec3::new(i.place[0], i.place[1], i.place[2]) - eye).length();
            by[((away - 4000.0) / 3000.0).clamp(0.0, 5.0) as usize] += 1;
        }
        let tallest = dock.iter().map(|(_, i)| i.place[3]).fold(0.0f32, f32::max);
        eprintln!(
            "from the dock by distance (4 km, then every 3 km): {by:?}, the tallest {tallest} m"
        );
        for (from, placed) in [("dock", &dock), ("tunnel", &tunnel)] {
            assert!(
                placed.len() > 200 && placed.len() <= BUDGET,
                "{} from the {from}",
                placed.len()
            );
            // Each on the ground, standing in toward the axis.
            for (_, i) in placed {
                let r = ((i.place[0] - AXIS).powi(2) + i.place[1].powi(2)).sqrt();
                assert!((r - GROUND).abs() < 1.0, "{r}");
                assert!(i.place[3] > 0.0 && i.place[3] < 1600.0);
                let up = Quat::from_array(i.turn) * Vec3::Z;
                let inward = -Vec3::new(i.place[0] - AXIS, i.place[1], 0.0).normalize();
                assert!(up.dot(inward) > 0.999);
            }
        }
    }

    #[test]
    fn a_building_stands_where_the_map_paints_its_roof() {
        // The map's texel for a building's place (`around`, as `fs_pane`
        // samples it) is, in ground metres, the building's own lot.
        let c = city::city();
        for b in c.buildings().iter().step_by(97) {
            let theta = b.at[0] / GROUND;
            let u = city::around([theta.cos(), theta.sin()]);
            let m = city::metres([u * 2048.0, b.at[1] / ALONG * 1024.0], 2048.0, 1024.0);
            let d = city::wrap([m[0] - b.at[0], m[1] - b.at[1]]);
            assert!(d[0].abs() < 0.5 && d[1].abs() < 0.5, "{b:?} {m:?}");
        }
    }
}
