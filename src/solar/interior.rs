//! A starbase's interior as the frame draws it (`tour::BaseScene`): the
//! shaders' rows for it, every mesh placed, and its lights and sparks.
//!
//! Everything is in the base's own frame, metres: the mouth at the origin,
//! +z in down the funnel and the tunnel to the bay, +y up, +x right. The
//! encounter shaders draw in that frame as they draw a stopover's (`EV_SHIP`
//! the camera, `EV_X..EV_Z` its axes), so the model pipeline (family 6,
//! `fs_model`), the panes (`fs_pane`) and the effects (`vs_fx_*`) need
//! nothing else of it but the `EV_BASE` rows.
//!
//! Seen from outside (the approach and the climb) only its lights are drawn
//! here: the ship given priority going in, and the corridor's beacons.

use glam::{Quat, Vec3};

use super::base_mesh::piece;
use super::encounters::{FX_BEAM, FX_BRIGHT, FX_DIM, FX_GLINT, Fx, fx_falloff};
use super::layout::*;
use super::models::{self, Instance, Key};
use super::tour::BaseScene;
use super::tour::BaseStage;
use super::tour::starbase::{
    BEND, BERTH, BORE, CRADLE, CRADLE_DROP, FUNNEL, SEGMENT, SEGMENTS, SERVICE, TUNNEL_LENGTH,
    bend, tunnel_end,
};

/// Segments drawn either side of the camera: the fog hides the rest.
const REACH: i32 = 11;
/// Panes in a segment: three bands of four bays (`base_mesh`).
pub const PANES: u32 = 12;
/// Vertices of a pane: four facets of two triangles round its band.
pub const PANE_VERTICES: u32 = 24;

/// The cradle, where the ship rests in the bay.
fn cradle() -> Vec3 {
    Vec3::new(BEND as f32, 0.0, (tunnel_end() + CRADLE) as f32)
}

/// The segments in view: the first and how many.
pub(super) fn segments(b: &BaseScene) -> (u32, u32) {
    if !b.inside {
        return (0, 0);
    }
    let at = ((b.ship[2] - FUNNEL) / SEGMENT).floor() as i32;
    let first = (at - REACH).clamp(0, SEGMENTS as i32);
    let last = (at + REACH + 1).clamp(0, SEGMENTS as i32);
    (first as u32, (last - first).max(0) as u32)
}

/// The shaders' rows for the base (`EV_BASE` and the frame's own).
pub(super) fn rows(b: &BaseScene, ev: &mut [[f32; 4]; EV_ROWS]) {
    let v4 = |v: [f64; 3], w: f32| [v[0] as f32, v[1] as f32, v[2] as f32, w];
    ev[EV_EVENT] = [0.0, b.clock as f32, 1.0, 0.0];
    ev[EV_SHIP] = v4(b.ship, 0.0);
    ev[EV_X] = v4(b.basis[0], 0.0);
    ev[EV_Y] = v4(b.basis[1], 0.0);
    ev[EV_Z] = v4(b.basis[2], 0.0);
    ev[EV_STATE] = [0.0, 1.0, 0.0, 0.0];
    ev[EV_HIT] = [0.0, 0.0, b.shake as f32, 0.0];
    let serviced = match b.stage {
        BaseStage::Service => smoothstep(SERVICE * 0.8, SERVICE * 0.9, b.local),
        BaseStage::Undock => 1.0,
        _ => 0.0,
    };
    ev[EV_BASE] = [
        if b.inside { 2.0 } else { 1.0 },
        b.clock as f32,
        (b.seed % 997) as f32,
        serviced as f32,
    ];
    ev[EV_BASE + 1] = [
        FUNNEL as f32,
        TUNNEL_LENGTH as f32,
        BEND as f32,
        BORE as f32,
    ];
    let (first, count) = segments(b);
    ev[EV_BASE + 2] = [
        SEGMENT as f32,
        first as f32,
        count as f32,
        tunnel_end() as f32,
    ];
    ev[EV_BASE + 3] = match arc(b) {
        Some((at, flash)) => [at.x, at.y, at.z, flash],
        None => [0.0; 4],
    };
}

/// A turn of `angle` about +y: a segment along the bend.
fn yaw(angle: f32) -> Quat {
    Quat::from_rotation_y(angle)
}

fn instance(at: Vec3, size: f32, turn: Quat, look: [f32; 4]) -> Instance {
    Instance {
        place: [at.x, at.y, at.z, size],
        turn: turn.to_array(),
        look,
    }
}

/// How far the clamps have closed, 0..1.
pub(super) fn clamped(b: &BaseScene) -> f32 {
    (match b.stage {
        BaseStage::Berth => smoothstep(BERTH - 2.2, BERTH - 1.2, b.local),
        BaseStage::Service => 1.0,
        BaseStage::Undock => 1.0 - smoothstep(0.1, 1.0, b.local),
        _ => 0.0,
    }) as f32
}

/// A service arm: where it hangs from, its tool (`piece::WELDER`,
/// `COUPLER`, `SCANNER`), and where its tool's tip is at a moment of the
/// service.
struct Arm {
    base: Vec3,
    tool: u32,
}

const UPPER: f32 = 34.0;
const LOWER: f32 = 30.0;
const TOOL: f32 = 6.6;

fn arms() -> [Arm; 3] {
    let c = cradle();
    [
        Arm {
            base: c + Vec3::new(-45.0, 45.0, 55.0),
            tool: piece::WELDER,
        },
        Arm {
            base: c + Vec3::new(0.0, 50.0, 62.0),
            tool: piece::COUPLER,
        },
        Arm {
            base: c + Vec3::new(45.0, 45.0, 55.0),
            tool: piece::SCANNER,
        },
    ]
}

/// The welds along the hull's seam, the coupling and the scan: where each
/// arm's tool tip goes `local` seconds into the service, and whether it is
/// at work there.
fn tip(arm: usize, local: f64) -> (Vec3, bool) {
    let c = cradle() + Vec3::new(0.0, -(CRADLE_DROP as f32), 0.0);
    let a = arms();
    let rest = a[arm].base + Vec3::new(0.0, -20.0, -6.0);
    // Out from rest over `reach` seconds from `t0`, back by `t1`.
    let span = |t0: f64, t1: f64| {
        (smoothstep(t0, t0 + 1.6, local) * (1.0 - smoothstep(t1 - 1.6, t1, local))) as f32
    };
    let (work, out, busy) = match arm {
        0 => {
            // A seam welded along the nose, left to right, then a second.
            let u = (((local - 1.5) / 5.5).rem_euclid(1.0)) as f32;
            let weld = c + Vec3::new(-12.0 + 24.0 * u, 6.0, 22.0);
            let on = (1.5..7.0).contains(&local) || (25.5..31.0).contains(&local);
            (weld, span(0.0, 8.0).max(span(24.0, 32.0)), on)
        }
        1 => (
            c + Vec3::new(0.0, 4.0, 15.0),
            span(8.0, 24.0),
            (9.6..22.4).contains(&local),
        ),
        _ => {
            let u = ((local - 24.0) / 8.0).clamp(0.0, 1.0) as f32;
            let at = c + Vec3::new(-16.0 + 32.0 * u, 12.0, 26.0);
            (at, span(24.0, 32.5), (24.8..31.6).contains(&local))
        }
    };
    (rest.lerp(work, out), busy && out > 0.95)
}

/// Where the arms' tips are now, for the stage: at rest but in the service.
fn tips(b: &BaseScene) -> [(Vec3, bool); 3] {
    let local = if b.stage == BaseStage::Service {
        b.local
    } else {
        -10.0
    };
    [0, 1, 2].map(|k| tip(k, local))
}

/// The arc light while a weld is struck: where and how bright, flickering.
pub(super) fn arc(b: &BaseScene) -> Option<(Vec3, f32)> {
    if !b.inside || b.stage != BaseStage::Service {
        return None;
    }
    let (at, on) = tips(b)[0];
    if !on {
        return None;
    }
    let t = b.clock as f32;
    let flicker = 0.6 + 0.4 * ((t * 37.0).sin() * (t * 23.0 + 1.3).sin()).abs();
    Some((at, flicker))
}

/// The two-bone reach from `base` toward `wrist`: the elbow, kept above
/// the line between them.
fn elbow(base: Vec3, wrist: Vec3) -> Vec3 {
    let span = (wrist - base).length().clamp(1.0, UPPER + LOWER - 0.5);
    let way = (wrist - base).normalize_or(Vec3::NEG_Y);
    let along = (UPPER * UPPER - LOWER * LOWER + span * span) / (2.0 * span);
    let out = (UPPER * UPPER - along * along).max(0.0).sqrt();
    let mut pole = Vec3::Y - way * Vec3::Y.dot(way);
    if pole.length() < 1e-3 {
        pole = Vec3::Z - way * Vec3::Z.dot(way);
    }
    base + way * along + pole.normalize() * out
}

/// Every mesh of the interior placed for this moment, into `out`.
pub(super) fn instances(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    if !b.inside {
        return;
    }
    let z = b.ship[2];
    let none = [0.0; 4];
    if z < FUNNEL + 2500.0 {
        out.push((
            Key::Base {
                piece: piece::FUNNEL,
            },
            instance(Vec3::ZERO, 1.0, Quat::IDENTITY, none),
        ));
    }
    let (first, count) = segments(b);
    for s in first..first + count {
        let zc = FUNNEL + (s as f64 + 0.5) * SEGMENT;
        let (x, slope) = bend(zc);
        out.push((
            Key::Base {
                piece: piece::SEGMENT,
            },
            instance(
                Vec3::new(x as f32, 0.0, zc as f32),
                1.0,
                yaw(slope.atan() as f32),
                none,
            ),
        ));
    }
    if z < tunnel_end() - 2500.0 {
        return;
    }
    let end = tunnel_end() as f32;
    let hall = Vec3::new(BEND as f32, 0.0, end);
    out.push((
        Key::Base { piece: piece::BAY },
        instance(hall, 1.0, Quat::IDENTITY, none),
    ));
    let c = cradle();
    out.push((
        Key::Base {
            piece: piece::CRADLE,
        },
        instance(c, 1.0, Quat::IDENTITY, none),
    ));
    // The clamps round the platform, swung out from their hinges until
    // they close on the hull.
    let open = (1.0 - clamped(b)) * 70f32.to_radians();
    for (sx, sz) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
        let at = c + Vec3::new(sx * 30.0, -34.0, sz * 35.0);
        let face = if sx < 0.0 { 0.0 } else { std::f32::consts::PI };
        out.push((
            Key::Base {
                piece: piece::CLAMP,
            },
            instance(at, 1.0, yaw(face) * Quat::from_rotation_z(open), none),
        ));
    }
    // Ships in the other berths, nose to the walls.
    for k in 0..4u64 {
        let side = if k % 2 == 0 { -1.0 } else { 1.0 };
        let at = hall + Vec3::new(side * 250.0, -70.0, 200.0 + 280.0 * (k / 2) as f32);
        let mut rng = super::system::Rng::new(b.seed ^ 0xbe47 ^ k);
        let key = Key::Ship {
            design: (rng.next() % models::DESIGNS as u64) as u32,
            role: 1,
            variant: (rng.next() % models::VARIANTS as u64) as u32,
        };
        let turn = yaw(side * std::f32::consts::FRAC_PI_2);
        out.push((key, instance(at, 45.0, turn, [k as f32, 0.0, 0.0, 0.0])));
    }
    // The service arms, reaching their tools to their work.
    for (arm, (tip, _)) in arms().iter().zip(tips(b)) {
        let way = (tip - arm.base).normalize_or(Vec3::NEG_Y);
        let wrist = tip - way * TOOL;
        let knee = elbow(arm.base, wrist);
        for (from, to, length) in [(arm.base, knee, UPPER), (knee, wrist, LOWER)] {
            let turn = Quat::from_rotation_arc(Vec3::Z, (to - from).normalize_or(Vec3::Z));
            out.push((
                Key::Base { piece: piece::LINK },
                instance(from, length, turn, none),
            ));
        }
        let turn = Quat::from_rotation_arc(Vec3::Z, (tip - wrist).normalize_or(Vec3::Z));
        out.push((
            Key::Base { piece: arm.tool },
            instance(wrist, 1.0, turn, none),
        ));
    }
}

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

/// A beam from `a` to `b`, `w` pixels wide, steady.
fn beam(out: &mut [Vec<Fx>; 6], a: Vec3, b: Vec3, w: f32, colour: [f32; 3]) {
    let peak = colour[0].max(colour[1]).max(colour[2]);
    if peak < FX_DIM {
        return;
    }
    let span = (b - a).length();
    let m = (b - a) / span.max(1e-3);
    let reach = |floor| {
        (5.0 * w * fx_falloff(0.12 * peak, floor))
            .max(w * (2.0 * fx_falloff(1.6 * peak, floor)).sqrt())
            + 1.0
    };
    out[FX_BEAM].push(Fx {
        a: [a.x, a.y, a.z, w],
        b: [m.x, m.y, m.z, span],
        colour: [colour[0], colour[1], colour[2], -1.0],
        reach: [reach(FX_DIM), 0.0, 0.0, reach(FX_BRIGHT) / reach(FX_DIM)],
    });
}

fn hash(n: u32) -> f32 {
    let mut h = n.wrapping_mul(0x9e37_79b9) ^ 0x85eb_ca6b;
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 65535.0
}

/// The interior's lights this frame, by kind (`FX_KINDS`): sparks off the
/// weld and its arc, the coupling's glow, the scanner's line, the guide
/// lights running in down the tunnel's floor, the bay's beacons; seen from
/// outside, the corridor's beacons running in to the mouth and the ship
/// given priority, its engines.
pub(super) fn effects(b: &BaseScene) -> [Vec<Fx>; 6] {
    let mut out: [Vec<Fx>; 6] = Default::default();
    let t = b.clock as f32;
    let ship = Vec3::new(b.ship[0] as f32, b.ship[1] as f32, b.ship[2] as f32);
    if !b.inside {
        // The corridor's beacons, two lines of them, a pulse running in.
        if ship.z > -90_000.0 && ship.z < 0.0 {
            for k in 1..=20 {
                let z = -2000.0 * k as f32;
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
    // Guide lights along the tunnel's floor, a run of them flowing in
    // ahead of the ship.
    if b.stage == BaseStage::Tunnel || b.stage == BaseStage::Outbound {
        let floor = -(BORE as f32) + 4.0;
        let sense = if b.stage == BaseStage::Tunnel {
            1.0
        } else {
            -1.0
        };
        for k in 1..=14 {
            let z = ship.z + sense * 45.0 * k as f32;
            if z < FUNNEL as f32 || z > tunnel_end() as f32 {
                continue;
            }
            let x = bend(z as f64).0 as f32;
            let pulse = (1.0 - ((t * 1.6 - k as f32 * 0.12).rem_euclid(1.0))).powi(4);
            glint(
                &mut out,
                Vec3::new(x, floor, z),
                1.6,
                [0.3 + 1.4 * pulse, 0.8 + 1.0 * pulse, 1.2 + 0.6 * pulse],
            );
        }
    }
    let c = cradle();
    if ship.z > (tunnel_end() - 600.0) as f32 {
        // The bay's beacons: amber at the cradle's corners while the clamps
        // work, green once the service is done.
        let done = matches!(b.stage, BaseStage::Undock)
            || (b.stage == BaseStage::Service && b.local > SERVICE * 0.85);
        let on = (t * 1.2).fract() < 0.5;
        let colour = if done {
            [0.2, 1.5, 0.4]
        } else {
            [1.6, 0.8, 0.1]
        };
        for (sx, sz) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
            if on {
                glint(
                    &mut out,
                    c + Vec3::new(sx * 36.0, -33.0, sz * 46.0),
                    1.8,
                    colour,
                );
            }
        }
    }
    if b.stage != BaseStage::Service {
        return out;
    }
    let tips = tips(b);
    // The weld: its arc, and sparks thrown off it falling and fading.
    if let Some((at, flash)) = arc(b) {
        glint(
            &mut out,
            at,
            4.0 * flash,
            [1.6 * flash, 1.8 * flash, 2.4 * flash],
        );
        for i in 0..44u32 {
            let life = 0.9;
            let age = (t / life + hash(i)).fract();
            let up = 0.3 + 0.7 * hash(i + 101);
            let around = std::f32::consts::TAU * hash(i + 211);
            let speed = 8.0 + 14.0 * hash(i + 307);
            let v = Vec3::new(
                around.cos() * (1.0 - up * 0.5),
                up,
                around.sin() * (1.0 - up * 0.5) - 0.3,
            ) * speed;
            let s = age * life;
            let p = at + v * s + Vec3::new(0.0, -9.8 * 0.5 * s * s * 4.0, 0.0);
            let heat = (1.0 - age) * (1.0 - age);
            glint(&mut out, p, 0.8, [2.0 * heat, 1.1 * heat, 0.35 * heat]);
        }
    }
    // The coupling's glow while fuel and coolant run.
    let (coupling, flowing) = tips[1];
    if flowing {
        let pulse = 0.6 + 0.4 * (t * 3.0).sin();
        glint(
            &mut out,
            coupling,
            2.2,
            [0.2 * pulse, 1.0 * pulse, 1.4 * pulse],
        );
    }
    // The scanner's line down onto the hull.
    let (scanning, on) = tips[2];
    if on {
        let hull = Vec3::new(scanning.x, c.y - CRADLE_DROP as f32 + 2.0, scanning.z - 4.0);
        beam(&mut out, scanning, hull, 1.2, [0.3, 1.6, 0.5]);
        let sweep = hull + Vec3::new(0.0, 0.0, ((t * 2.0).sin()) * 6.0);
        glint(&mut out, sweep, 2.0, [0.3, 1.4, 0.4]);
    }
    out
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_arms_reach_their_work_and_keep_their_lengths() {
        for local in [0.0, 4.0, 12.0, 20.0, 28.0, 36.0] {
            for (k, arm) in arms().iter().enumerate() {
                let (tip, _) = tip(k, local);
                let way = (tip - arm.base).normalize();
                let wrist = tip - way * TOOL;
                let knee = elbow(arm.base, wrist);
                assert!(
                    ((knee - arm.base).length() - UPPER).abs() < 0.05,
                    "arm {k} at {local}"
                );
                let reach = (wrist - arm.base).length();
                if reach < UPPER + LOWER - 0.5 {
                    assert!(
                        ((wrist - knee).length() - LOWER).abs() < 0.05,
                        "arm {k} at {local}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_weld_strikes_only_in_service() {
        let scene = |stage, local| BaseScene {
            ship: [BEND, -CRADLE_DROP, tunnel_end() + CRADLE],
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            stage,
            local,
            clock: 100.0,
            inside: true,
            seed: 5,
            traffic: None,
            shake: 0.0,
        };
        assert!(arc(&scene(BaseStage::Service, 3.0)).is_some());
        assert!(arc(&scene(BaseStage::Service, 15.0)).is_none());
        assert!(arc(&scene(BaseStage::Berth, 3.0)).is_none());
        assert_eq!(clamped(&scene(BaseStage::Service, 3.0)), 1.0);
        assert_eq!(clamped(&scene(BaseStage::Tunnel, 3.0)), 0.0);
        let fx = effects(&scene(BaseStage::Service, 3.0));
        let total: usize = fx.iter().map(|v| v.len()).sum();
        assert!(total > 20 && total <= super::super::encounters::FX_INSTANCES);
        // Every mesh within reach of the camera.
        let mut placed = Vec::new();
        instances(&scene(BaseStage::Service, 3.0), &mut placed);
        assert!(
            placed
                .iter()
                .any(|(k, _)| *k == Key::Base { piece: piece::BAY })
        );
        assert!(
            placed
                .iter()
                .filter(|(k, _)| *k
                    == Key::Base {
                        piece: piece::CLAMP
                    })
                .count()
                == 4
        );
    }

    #[test]
    fn the_tunnel_is_drawn_round_the_camera() {
        let mut b = BaseScene {
            ship: [0.0, 0.0, FUNNEL + 2000.0],
            basis: [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]],
            stage: BaseStage::Tunnel,
            local: 5.0,
            clock: 50.0,
            inside: true,
            seed: 1,
            traffic: None,
            shake: 0.0,
        };
        let (first, count) = segments(&b);
        assert!(first > 0 && count == 2 * REACH as u32 + 1);
        b.inside = false;
        assert_eq!(segments(&b), (0, 0));
    }
}
