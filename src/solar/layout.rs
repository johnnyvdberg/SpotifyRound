//! The layout the solar scene shares with its shaders: the sizes of the
//! uniform block's arrays, the rows of its stopover parameters and the vertex
//! counts of the draws the shaders lay out themselves.
//!
//! `build.rs` includes this file and hands every constant in [`WESL`] to the
//! shaders as a WESL constant (`import constants::NAME`), so each number lives
//! here once. A test reflects the linked shader and checks the uniform
//! block's size against [`super::Params`].

/// Bodies drawn in a frame (`Params::bodies`).
pub const MAX_BODIES: usize = 16;

/// Ships in a battle, placed once a frame into `Params::ev` from
/// [`EV_FLEET`], two rows each (`fleet_rows`, read by `battle_ship`).
pub const BATTLE_SHIPS: usize = 8;

/// Rows of the stopover's parameters (`Params::ev`, `encounter_rows`).
///
/// The event: its id, seconds into it, its duration and its variant.
pub const EV_EVENT: usize = 0;
/// The ship in the encounter frame; w 1 out in the deep.
pub const EV_SHIP: usize = 1;
/// The encounter frame's x, y and z axes in the world. Their w: the hit on
/// the shield, the tractor beam's hold and the flash.
pub const EV_X: usize = 2;
pub const EV_Y: usize = 3;
pub const EV_Z: usize = 4;
/// The event's objects, three rows (place and size).
pub const EV_OBJECT: usize = 5;
/// A rotation: axis and angle.
pub const EV_ROT: usize = 8;
/// The event's own four numbers.
pub const EV_EXTRA: usize = 9;
/// The alarm, how much of the stopover shows, the flux and how much of it.
pub const EV_STATE: usize = 10;
/// Where a hit lands on the frame (across the half frame), the shake, and
/// seconds into the flux.
pub const EV_HIT: usize = 11;
/// A battle's ships this frame, two rows each.
pub const EV_FLEET: usize = 12;
/// A starbase (`starbase::scene`), four rows: whether it is drawn (0 not,
/// 1 over the world it stands on, 2 alone, 3 alone with the ship's own bow
/// in the frame), seconds into the visit, its maps' seed and the green of
/// the dock's lights once serviced; the entrance's depth, the tunnel's
/// length, its bend and its bore; a segment's length, the first segment
/// with panes drawn, how many, and where the port begins; the way to the
/// star in the base's frame, and how far its depth resolves well.
pub const EV_BASE: usize = EV_FLEET + 2 * BATTLE_SHIPS;
/// Every row.
pub const EV_ROWS: usize = EV_BASE + 4;

/// Sides of the star's glare polygon (`vs_glare`).
pub const GLARE_SIDES: u32 = 32;
/// Sides of the drop-out's annulus and disc (`vs_drop_ahead`).
pub const DROP_SIDES: u32 = 64;
/// Sides of the polygon round the display's circle (`vs_corners`).
pub const CORNER_SIDES: u32 = 32;
/// Quads of the ring's annulus (`vs_ring`).
pub const RING_SIDES: u32 = 32;
/// Triangles of the inscribed depth mask (`vs_mask_inscribed`).
pub const MASK_SIDES: u32 = 64;
/// Sides of the limb's annulus (`vs_limb_ring`).
pub const LIMB_SIDES: u32 = 128;
/// A mapped world's mesh: vertex `row * GRID_STRIDE + column` (`vs_globe`).
pub const GRID_STRIDE: u32 = 129;
/// Motes of dust streaking past in a burn (`vs_dust`).
pub const DUST: u32 = 160;
/// Sides of the engines' glow round the rim (`vs_rim`).
pub const RIM_SIDES: u32 = 48;
/// Slices of the frame each leviathan's fins are bounded in (`vs_leviathan`).
pub const FIN_SLICES: u32 = 16;
/// Sides of a gate's field (`vs_gate_field`).
pub const GATE_SIDES: u32 = 48;
/// Sides of the ring a hit on the shield is drawn on (`vs_overlay_hit`).
pub const HIT_SIDES: u32 = 32;
/// How many times coarser than the frame the plasma storm and the remnant
/// are drawn (`fs_coarse`).
pub const COARSE: u32 = 2;

/// Every constant above, by the name the shaders import it as (read by
/// `build.rs` and the shader tests).
#[cfg_attr(not(test), allow(dead_code))]
pub const WESL: &[(&str, i64)] = &[
    ("MAX_BODIES", MAX_BODIES as i64),
    ("BATTLE_SHIPS", BATTLE_SHIPS as i64),
    ("EV_EVENT", EV_EVENT as i64),
    ("EV_SHIP", EV_SHIP as i64),
    ("EV_X", EV_X as i64),
    ("EV_Y", EV_Y as i64),
    ("EV_Z", EV_Z as i64),
    ("EV_OBJECT", EV_OBJECT as i64),
    ("EV_ROT", EV_ROT as i64),
    ("EV_EXTRA", EV_EXTRA as i64),
    ("EV_STATE", EV_STATE as i64),
    ("EV_HIT", EV_HIT as i64),
    ("EV_FLEET", EV_FLEET as i64),
    ("EV_BASE", EV_BASE as i64),
    ("EV_ROWS", EV_ROWS as i64),
    ("GLARE_SIDES", GLARE_SIDES as i64),
    ("DROP_SIDES", DROP_SIDES as i64),
    ("CORNER_SIDES", CORNER_SIDES as i64),
    ("RING_SIDES", RING_SIDES as i64),
    ("MASK_SIDES", MASK_SIDES as i64),
    ("LIMB_SIDES", LIMB_SIDES as i64),
    ("GRID_STRIDE", GRID_STRIDE as i64),
    ("DUST", DUST as i64),
    ("RIM_SIDES", RIM_SIDES as i64),
    ("FIN_SLICES", FIN_SLICES as i64),
    ("GATE_SIDES", GATE_SIDES as i64),
    ("HIT_SIDES", HIT_SIDES as i64),
    ("COARSE", COARSE as i64),
];

#[cfg(test)]
mod tests {
    use super::WESL;

    /// The linked shaders hold the values this file has now: `build.rs` ran
    /// with them, and nothing in the shaders declares them over again.
    #[test]
    fn the_shaders_were_linked_with_these_constants() {
        let shaders = [
            crate::solar::FRAME_SHADER,
            crate::solar::stream::GENERATE_SHADER,
        ];
        for (name, value) in WESL {
            let declared = format!("const {name} = {value};");
            let found = shaders.iter().filter(|s| s.contains(&declared)).count();
            let any = shaders.iter().any(|s| s.contains(&format!("const {name}")));
            assert!(
                !any || found > 0,
                "`{name}` in the linked shader is not {value}: rebuild"
            );
        }
        assert!(
            crate::solar::FRAME_SHADER.contains(&format!("const EV_ROWS = {};", super::EV_ROWS))
        );
    }
}
