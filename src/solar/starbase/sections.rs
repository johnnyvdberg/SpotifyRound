//! The world's length in sections. Its map is one length of ground
//! (`city::ALONG`, 13 km) laid end to end two thousand kilometres; past the
//! port each length is laid as it is or mirrored round (the world's suns
//! stand mirror-wise, so they still line up), by a pattern that repeats
//! every eight (`tour::starbase::FOLD`, which the scene is drawn back by
//! when deep in the world, so the pattern is unchanged by it). A bulkhead
//! stands at each seam between lengths, over the break in the ground, and
//! every eighth a great gate; `pane.wesl` (`world_varied`) lays the map
//! the same way.

use super::super::tour::BaseScene;
use super::super::tour::starbase::{CRADLE, tunnel_end};
use super::city::ALONG;
use super::port::LENGTH;

/// Which of each eight lengths are mirrored, a bit each.
const MIRRORS: u32 = 0b1011_0100;
/// Lengths in a section of the pattern.
pub const SPAN: i32 = 8;
/// Past the port (metres past the tunnel's end, as `WORLD_VARIED` in
/// `pane.wesl`), the lengths vary and the bulkheads stand.
pub const VARIED_PAST_END: f32 = CRADLE as f32 + LENGTH + 1500.0;

/// The map's own place along this base, in lengths (`shift` in `pane`).
pub fn shift(b: &BaseScene) -> f32 {
    (b.seed % 997) as f32 * 0.0173
}

/// Where length `n` of the map begins, metres in the scene's depth.
pub fn seam(n: i32, shift: f32) -> f32 {
    (n as f32 - shift) * ALONG
}

/// Whether the lengths vary from a seam at `z` on.
pub fn varied(z: f32) -> bool {
    z > tunnel_end() as f32 + VARIED_PAST_END
}

/// Whether length `n` is laid mirrored round.
pub fn mirrored(n: i32, shift: f32) -> bool {
    varied(seam(n, shift)) && (MIRRORS >> (n & 7)) & 1 == 1
}

/// The lengths the scene is drawn back by (`BaseScene::fold`).
pub fn folded(b: &BaseScene) -> i32 {
    (b.fold / ALONG as f64).round() as i32
}

/// A number 0..1 of section `k`'s own (true, not as drawn), and of `salt`.
pub fn section_hash(k: i32, salt: u32) -> f32 {
    super::hash(
        (k as u32)
            .wrapping_mul(0x2f1)
            .wrapping_add(salt.wrapping_mul(0x9e5)),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::solar::tour::starbase::FOLD;

    #[test]
    fn the_fold_is_whole_sections_and_matches_the_shader() {
        assert_eq!(FOLD, SPAN as f64 * ALONG as f64);
        let pane = include_str!("../../shaders/solar/starbase/pane.wesl");
        let line = pane
            .lines()
            .find(|l| l.contains("const WORLD_VARIED"))
            .expect("WORLD_VARIED in pane.wesl");
        let value: f32 = line
            .split('=')
            .nth(1)
            .unwrap()
            .trim()
            .trim_end_matches(';')
            .parse()
            .unwrap();
        assert_eq!(value, VARIED_PAST_END);
        assert!(pane.contains(&format!("0x{MIRRORS:X}u")));
    }
}
