//! A moment of a starbase visit as the frame draws it: the shaders' rows
//! for it, and every mesh placed.

use glam::{Quat, Vec3};

use super::super::layout::*;
use super::super::models::{Instance, Key};
use super::super::tour::starbase::{
    BEND, BERTH, BORE, CRADLE, FUNNEL, LOCK, LOCK_SEGMENTS, SEGMENT, SEGMENTS, SERVICE,
    TUNNEL_LENGTH, bend, far_lock, far_wall, tunnel_end,
};
use super::super::tour::{BaseScene, BaseStage};
use super::mesh::piece;
use super::{deep, port, smoothstep, traffic, world};

/// Segments drawn either side of the camera: the fog hides the rest.
const REACH: i32 = 11;
/// Panes in a segment: three bands of two bays (`mesh::tunnel`).
pub const PANES: u32 = 6;
/// Vertices of a pane: four facets of two triangles round its band.
pub const PANE_VERTICES: u32 = 24;
/// The megastructure and its entrance are drawn from this far out, metres:
/// from the orbit, where it is a few pixels across.
const SEEN_FROM: f64 = 6_000_000.0;

/// The cradle, where the ship rests in the port.
pub(super) fn cradle() -> Vec3 {
    Vec3::new(BEND as f32, 0.0, (tunnel_end() + CRADLE) as f32)
}

/// The port: the tunnel's exit into it.
pub(super) fn hall() -> Vec3 {
    Vec3::new(BEND as f32, 0.0, tunnel_end() as f32)
}

pub(super) fn ship(b: &BaseScene) -> Vec3 {
    Vec3::new(b.ship[0] as f32, b.ship[1] as f32, b.ship[2] as f32)
}

/// Whether a ball of `radius` at `at` can show on the frame: in front of
/// the camera and within its field (the round frame's, and a margin).
pub(super) fn seen(b: &BaseScene, at: Vec3, radius: f32) -> bool {
    let heading = Vec3::new(
        b.heading[0] as f32,
        b.heading[1] as f32,
        b.heading[2] as f32,
    );
    let d = at - ship(b);
    let along = d.dot(heading);
    if along < -radius {
        return false;
    }
    let aside = (d.length_squared() - along * along).max(0.0).sqrt();
    aside < (along + radius) * 0.6 + radius * 1.2
}

/// The segments in view: the first and how many.
pub fn segments(b: &BaseScene) -> (u32, u32) {
    // None while the airlock's inner door is shut: nothing past it shows.
    if !b.inside || b.doors[1] <= 0.0 {
        return (0, 0);
    }
    let at = ((b.ship[2] - FUNNEL) / SEGMENT).floor() as i32;
    let first = (at - REACH).clamp(LOCK_SEGMENTS as i32, SEGMENTS as i32);
    let last = (at + REACH + 1).clamp(0, SEGMENTS as i32);
    (first as u32, (last - first).max(0) as u32)
}

/// Whether the world in the open shows round the camera (`vs_shell`):
/// from the tunnel's end on, while the way to it is open.
pub fn world_shown(b: &BaseScene) -> bool {
    b.inside && !b.far && b.doors[1] > 0.0 && b.ship[2] >= tunnel_end() - 2500.0
}

/// Whether the port is in view: in it, and until the crossing has left it
/// well behind.
pub fn port_shown(b: &BaseScene) -> bool {
    world_shown(b) && b.ship[2] < tunnel_end() + (CRADLE + port::LENGTH as f64) + 3000.0
}

/// Whether the ship's own bow is drawn, below the cockpit: while it
/// berths, is serviced and leaves.
pub(super) fn bow_shown(b: &BaseScene) -> bool {
    matches!(
        b.stage,
        BaseStage::Berth | BaseStage::Service | BaseStage::Undock
    )
}

/// The shaders' rows for the base (`EV_BASE` and the frame's own).
pub fn rows(b: &BaseScene, ev: &mut [[f32; 4]; EV_ROWS]) {
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
        // Drawn over its world, alone, or alone with the ship's own bow in
        // the frame (`vs_shell`).
        match (b.inside, bow_shown(b)) {
            (true, true) => 3.0,
            (true, false) => 2.0,
            _ => 1.0,
        },
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
    // The depth resolves a few hundred metres inside, kilometres above
    // the city.
    // above the megastructure, more the further out.
    let far = (b.ship[0].powi(2) + b.ship[1].powi(2) + b.ship[2].powi(2)).sqrt();
    let reach = if b.ship[2] < -100.0 {
        (far * 0.3).max(15_000.0)
    } else {
        400.0
    };
    ev[EV_BASE + 3] = v4(b.sun, reach as f32);
}

pub(super) fn instance(at: Vec3, size: f32, turn: Quat, look: [f32; 4]) -> Instance {
    Instance {
        place: [at.x, at.y, at.z, size],
        turn: turn.to_array(),
        look,
    }
}

pub(super) fn base(piece: u32) -> Key {
    Key::Base { piece }
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

/// Every mesh of the base placed for this moment, into `out`: the
/// megastructure and the entrance from outside and on the way down; the
/// airlock; the tunnel round the
/// camera and what flies past its windows; the port, its catwalks and the
/// ships along them.
pub fn instances(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    let z = b.ship[2];
    let at = ship(b);
    let none = [0.0; 4];
    let far = (b.ship[0].powi(2) + b.ship[1].powi(2) + z * z).sqrt();
    // The megastructure and the entrance, but from in the airlock with its
    // outer door shut.
    let sealed = b.doors[0] <= 0.0 && z > FUNNEL;
    if !sealed && z < FUNNEL + 400.0 && far < SEEN_FROM {
        for part in [piece::PLATING, piece::STRUCTURE, piece::ENTRANCE] {
            out.push((base(part), instance(Vec3::ZERO, 1.0, Quat::IDENTITY, none)));
        }
    }
    if !b.inside {
        return;
    }
    // The airlock: its chamber, the status lamps green once the inner door
    // is open, and its two doors' leaves, drawn apart as they open.
    if z < FUNNEL + 2500.0 {
        let status = if b.doors[1] >= 1.0 { 1.0 } else { 0.0 };
        out.push((
            base(piece::LOCK),
            instance(
                Vec3::new(0.0, 0.0, LOCK as f32),
                1.0,
                Quat::IDENTITY,
                [status, 0.0, 0.0, 0.0],
            ),
        ));
        let length = (SEGMENT * LOCK_SEGMENTS as f64) as f32;
        for (open, depth) in [
            (b.doors[0], FUNNEL as f32 + 1.5),
            (b.doors[1], FUNNEL as f32 + length - 1.5),
        ] {
            let slide = open as f32 * (BORE as f32 + 6.0);
            for (side, turn) in [
                (1.0f32, Quat::IDENTITY),
                (-1.0, Quat::from_rotation_z(std::f32::consts::PI)),
            ] {
                out.push((
                    base(piece::LEAF),
                    instance(Vec3::new(side * slide, 0.0, depth), 1.0, turn, none),
                ));
            }
        }
    }
    let (first, count) = segments(b);
    for s in first..first + count {
        let zc = FUNNEL + (s as f64 + 0.5) * SEGMENT;
        let (x, slope) = bend(zc);
        out.push((
            base(piece::SEGMENT),
            instance(
                Vec3::new(x as f32, 0.0, zc as f32),
                1.0,
                Quat::from_rotation_y(slope.atan() as f32),
                none,
            ),
        ));
    }
    if count > 0 {
        traffic::cavern(b, out);
    }
    if b.doors[1] > 0.0 && !b.far {
        world::towns(b, out);
    }
    deep::features(b, out);
    far_airlock(b, out);
    if !port_shown(b) {
        return;
    }
    out.push((
        base(piece::HUB),
        instance(hall(), 1.0, Quat::IDENTITY, none),
    ));
    port::berths(b, out);
    traffic::port(b, out);
    let c = cradle();
    let status = match b.stage {
        BaseStage::Service if b.local > SERVICE * 0.85 => 1.0,
        BaseStage::Undock => 1.0,
        _ => 0.0,
    };
    out.push((
        base(piece::CRADLE),
        instance(c, 1.0, Quat::IDENTITY, [status, 0.0, 0.0, 0.0]),
    ));
    // The clamps round the deck, swung out from their hinges until they
    // close on the hull.
    let open = (1.0 - clamped(b)) * 70f32.to_radians();
    for (sx, sz) in [(-1.0f32, -1.0f32), (-1.0, 1.0), (1.0, -1.0), (1.0, 1.0)] {
        let foot = c + Vec3::new(sx * 30.0, -34.0, sz * 35.0);
        let face = if sx < 0.0 { 0.0 } else { std::f32::consts::PI };
        out.push((
            base(piece::CLAMP),
            instance(
                foot,
                1.0,
                Quat::from_rotation_y(face) * Quat::from_rotation_z(open),
                none,
            ),
        ));
    }
    // The ship's own bow, below the cockpit.
    if bow_shown(b) {
        let heading =
            Vec3::new(b.heading[0] as f32, 0.0, b.heading[2] as f32).normalize_or(Vec3::Z);
        out.push((
            base(piece::BOW),
            instance(at, 1.0, Quat::from_rotation_arc(Vec3::Z, heading), none),
        ));
    }
}

/// The far wall's airlock, from the crossing until the scene turns: its
/// chamber turned end for end (the world's door first), its doors' leaves.
fn far_airlock(b: &BaseScene, out: &mut Vec<(Key, Instance)>) {
    if !b.inside || b.far || !matches!(b.stage, BaseStage::Crossing | BaseStage::Egress) {
        return;
    }
    // Where the wall is drawn, the scene drawn back along the world.
    let (wall, lock) = ((far_wall() - b.fold) as f32, (far_lock() - b.fold) as f32);
    if (b.ship[2] as f32) < wall - 6000.0 {
        return;
    }
    let x = BEND as f32;
    let status = if b.doors[0] >= 1.0 { 1.0 } else { 0.0 };
    out.push((
        base(piece::LOCK),
        instance(
            Vec3::new(x, 0.0, lock),
            1.0,
            Quat::from_rotation_y(std::f32::consts::PI),
            [status, 0.0, 0.0, 0.0],
        ),
    ));
    let length = (SEGMENT * LOCK_SEGMENTS as f64) as f32;
    for (open, depth) in [(b.doors[1], wall + 1.5), (b.doors[0], wall + length - 1.5)] {
        let slide = open as f32 * (BORE as f32 + 6.0);
        for (side, turn) in [
            (1.0f32, Quat::IDENTITY),
            (-1.0, Quat::from_rotation_z(std::f32::consts::PI)),
        ] {
            out.push((
                base(piece::LEAF),
                instance(Vec3::new(x + side * slide, 0.0, depth), 1.0, turn, [0.0; 4]),
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::test_scene;
    use super::*;

    #[test]
    fn the_tunnel_is_drawn_round_the_camera() {
        let mut b = test_scene(BaseStage::Tunnel, 5.0, [0.0, 0.0, FUNNEL + 3000.0]);
        let (first, count) = segments(&b);
        assert!(first > 0 && count == 2 * REACH as u32 + 1);
        let mut placed = Vec::new();
        instances(&b, &mut placed);
        let segments_placed = placed
            .iter()
            .filter(|(k, _)| *k == base(piece::SEGMENT))
            .count();
        assert_eq!(segments_placed, count as usize);
        // Ships and cars fly past its windows.
        assert!(placed.iter().any(|(k, _)| *k == base(piece::CAR)));
        assert!(placed.iter().any(|(k, _)| matches!(k, Key::Ship { .. })));
        b.inside = false;
        assert_eq!(segments(&b), (0, 0));
    }

    #[test]
    fn the_city_is_drawn_on_the_way_down_and_the_port_at_the_end() {
        let mut placed = Vec::new();
        let mut b = test_scene(BaseStage::Approach, 5.0, [0.0, 0.0, -50_000.0]);
        b.inside = false;
        instances(&b, &mut placed);
        assert!(placed.iter().any(|(k, _)| *k == base(piece::STRUCTURE)));
        assert!(placed.iter().any(|(k, _)| *k == base(piece::ENTRANCE)));
        placed.clear();
        let dock = [BEND, 0.0, tunnel_end() + CRADLE];
        instances(&test_scene(BaseStage::Service, 3.0, dock), &mut placed);
        let count = |p: u32| placed.iter().filter(|(k, _)| *k == base(p)).count();
        assert_eq!(count(piece::HUB), 1);
        assert_eq!(count(piece::CLAMP), 4);
        assert!(count(piece::CATWALK) > 10);
        assert!(count(piece::MOORING) >= 2);
        // Battle ships among what is moored, none of them the cube's.
        let ships: Vec<u32> = placed
            .iter()
            .filter_map(|(k, _)| match k {
                Key::Ship { design, .. } => Some(*design),
                _ => None,
            })
            .collect();
        assert!(ships.len() >= 3);
        assert!(ships.iter().all(|&d| d < port::DESIGNS));
        assert!(
            placed.len() < crate::solar::MODEL_INSTANCES,
            "{} placed",
            placed.len()
        );
    }
}
