//! Where the Nautilus is on its voyage, and how deep.
//!
//! The diving-bell scene is drawn entirely by `dive.wgsl`, but the voyage is
//! steered from here: how far the Nautilus has come, how high it cruises above
//! the seabed and how deep it is below the surface. Keeping that state on the
//! CPU lets a swipe carry the boat on to the next stretch of sea, and lets the
//! depth follow a dive profile at a believable rate of climb, rather than jump
//! with every change of biome.
//!
//! The biome sequence and the seabed depths mirror the constants at the top of
//! `dive.wgsl`; the two must agree, or the seabed would not be where the gauges
//! say it is.

use std::f64::consts::TAU;

/// World units the Nautilus covers per second; one unit is about eleven metres.
pub const SPEED: f64 = 0.055;
/// World units per leg of the voyage, and the share of a leg spent changing
/// into the next biome.
pub const LEG: f64 = 3.4;
const BLEND: f64 = 0.32;
pub const BIOMES: usize = 7;

/// Where a visit starts within its first leg: on a settled stretch, rather than
/// mid-transition.
const START: f64 = LEG * 0.12;
/// Where a swipe lands within a leg, for the same reason.
const LANDING: f64 = LEG * 0.15;
/// Seconds a swipe takes to carry the Nautilus to the next leg.
const PASSAGE: f64 = 3.5;

/// Depth of the seabed below the surface in each biome, in world units:
/// shallows, forest, wrecks, Atlantis, lava cave, abyss, polar ice.
const SEABED: [f64; 7] = [1.0, 3.2, 6.0, 9.0, 13.0, 40.0, 3.0];
/// The band the Nautilus cruises in above the seabed, per biome.
const CRUISE: [(f64, f64); 7] = [
    (0.25, 0.55),
    // Low among the trunks of the forest, as the divers walk it.
    (0.22, 0.55),
    (0.25, 1.20),
    (0.25, 1.00),
    (0.22, 0.55),
    (0.25, 2.00),
    // Under the ice the Nautilus keeps close beneath the pack.
    (1.25, 2.30),
];
/// The highest the Nautilus may rise above the seabed: a fathom under the
/// surface in the shallows, under the cave's vault, and clear of the ice.
const CEILING: [f64; 7] = [0.70, 99.0, 99.0, 99.0, 0.62, 99.0, 2.45];
/// Closest the Nautilus comes to the seabed.
const CLEARANCE: f64 = 0.15;
/// The lava cave, and the height of its vault above its floor (`CAVE_ROOF` in
/// `dive.wgsl`). The vault is drawn wherever the cave covers a third of the
/// view's biome mix, so from there on the Nautilus must already be under it.
const CAVE: usize = 4;
const CAVE_ROOF: f64 = 0.95;
const CAVE_SHOWS: f64 = 0.3;
// The highest the Nautilus may cruise in the cave must keep it under the vault.
const _: () = assert!(CEILING[CAVE] < CAVE_ROOF - 0.1);

/// What the shader needs to know about the voyage for one frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Position {
    /// World units travelled.
    pub x: f64,
    /// Height above the seabed.
    pub altitude: f64,
    /// Depth below the surface.
    pub depth: f64,
}

/// A swipe in progress: the voyage's offset eases from `from` to `to`.
#[derive(Clone, Copy, Debug)]
struct Passage {
    begun: f64,
    from: f64,
    to: f64,
}

#[derive(Debug, Default)]
pub struct Voyage {
    seed: Option<f32>,
    last: f64,
    /// How far swipes have moved the Nautilus on from where time alone would
    /// have taken it.
    offset: f64,
    passage: Option<Passage>,
    depth: f64,
}

impl Voyage {
    /// Advance to `seconds` into the visit. A new seed, or the clock running
    /// backwards, starts a fresh voyage; a jump in time puts the Nautilus
    /// straight on its dive profile instead of easing it there.
    pub fn advance(&mut self, seconds: f64, seed: f32) -> Position {
        let fresh = self.seed != Some(seed) || seconds < self.last;
        if fresh {
            *self = Self {
                seed: Some(seed),
                last: seconds,
                ..Self::default()
            };
        }
        let dt = seconds - self.last;
        self.last = seconds;
        if let Some(passage) = self.passage
            && seconds >= passage.begun + PASSAGE
        {
            self.offset = passage.to;
            self.passage = None;
        }

        let x = self.x(seconds);
        let here = Biome::at(x, seed);
        let seabed = here.mix(SEABED);
        let low = here.mix(CRUISE.map(|band| band.0));
        let high = here.mix(CRUISE.map(|band| band.1));
        let mut ceiling = here.mix(CEILING);
        // Blending "no limit" with the cave's limit would leave the boat above
        // the vault, so the stalactites hang from nothing. Bring the cave's
        // ceiling down as the cave begins, in inverse so it falls smoothly.
        let cave = here.weight(CAVE);
        if cave > 0.0 {
            let t = (cave / CAVE_SHOWS).clamp(0.0, 1.0);
            let t = t * t * (3.0 - 2.0 * t);
            ceiling =
                ceiling.min(1.0 / (1.0 / ceiling + (1.0 / CEILING[CAVE] - 1.0 / ceiling) * t));
        }

        // A slow dive profile: long swells up and down through the cruising band.
        let phase = f64::from(seed) * 40.0;
        let swell = 0.5
            + 0.30 * (seconds * TAU / 97.0 + phase).sin()
            + 0.20 * (seconds * TAU / 41.0 + phase * 1.7).sin();
        let target = seabed - (low + (high - low) * swell.clamp(0.0, 1.0));

        // The boat answers the planes slowly, except when a swipe hurries it on.
        let lag = if self.passage.is_some() { 0.8 } else { 5.0 };
        if fresh || dt > 0.5 {
            self.depth = target;
        } else {
            self.depth += (target - self.depth) * (1.0 - (-dt / lag).exp());
        }
        // Never into the seabed, and never through the surface, the vault or the ice.
        self.depth = self
            .depth
            .clamp(seabed - ceiling, seabed - CLEARANCE)
            .max(0.05);
        Position {
            x,
            altitude: seabed - self.depth,
            depth: self.depth,
        }
    }

    /// Carry the Nautilus `legs` legs on (or back, if negative), landing on a
    /// settled stretch of that leg. Swipes made during a passage add up.
    pub fn travel(&mut self, legs: i32, seconds: f64) {
        if self.seed.is_none() {
            return;
        }
        let current = match self.passage {
            Some(passage) => base(passage.begun + PASSAGE) + passage.to,
            None => self.x(seconds),
        };
        let leg = (current / LEG).floor() + f64::from(legs);
        let landing = leg * LEG + LANDING;
        let end = seconds + PASSAGE;
        self.passage = Some(Passage {
            begun: seconds,
            from: self.offset_at(seconds),
            to: landing - base(end),
        });
    }

    fn x(&self, seconds: f64) -> f64 {
        base(seconds) + self.offset_at(seconds)
    }

    fn offset_at(&self, seconds: f64) -> f64 {
        match self.passage {
            Some(passage) => {
                let t = ((seconds - passage.begun) / PASSAGE).clamp(0.0, 1.0);
                // Ease in and out, so the boat surges away and settles again.
                let eased = t * t * t * (t * (t * 6.0 - 15.0) + 10.0);
                passage.from + (passage.to - passage.from) * eased
            }
            None => self.offset,
        }
    }
}

/// Where time alone would have taken the Nautilus.
fn base(seconds: f64) -> f64 {
    seconds * SPEED + START
}

/// The biomes at one point of the voyage: `a` turning into `b` by `w`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Biome {
    pub a: usize,
    pub b: usize,
    pub w: f64,
}

impl Biome {
    /// The same sequence as `biome` and `leg` in `dive.wgsl`.
    pub fn at(x: f64, seed: f32) -> Self {
        let u = x / LEG;
        let k = u.floor();
        let start = (seed * BIOMES as f32) as i64;
        let id = |k: f64| (k as i64 + start).rem_euclid(BIOMES as i64) as usize;
        let t = ((u - k - (1.0 - BLEND)) / BLEND).clamp(0.0, 1.0);
        Self {
            a: id(k),
            b: id(k + 1.0),
            w: t * t * (3.0 - 2.0 * t),
        }
    }

    /// How much of biome `id` there is here, 0 to 1.
    pub fn weight(&self, id: usize) -> f64 {
        let mut weight = 0.0;
        if self.a == id {
            weight += 1.0 - self.w;
        }
        if self.b == id {
            weight += self.w;
        }
        weight
    }

    fn mix(&self, values: [f64; 7]) -> f64 {
        values[self.a] + (values[self.b] - values[self.a]) * self.w
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ABYSS: usize = 5;

    #[test]
    fn a_visit_starts_in_the_seeds_biome() {
        let mut voyage = Voyage::default();
        let at = voyage.advance(0.0, 0.0);
        assert_eq!(Biome::at(at.x, 0.0).a, 0);
        let at = voyage.advance(0.0, 0.5);
        assert_eq!(Biome::at(at.x, 0.5).a, 3);
    }

    #[test]
    fn a_swipe_carries_the_nautilus_to_the_next_leg() {
        let mut voyage = Voyage::default();
        let mut seconds = 10.0;
        let before = voyage.advance(seconds, 0.0);
        voyage.travel(1, seconds);
        let mut last = before.x;
        while seconds < 10.0 + PASSAGE + 0.1 {
            seconds += 1.0 / 60.0;
            let at = voyage.advance(seconds, 0.0);
            assert!(at.x >= last, "the passage should only go forward");
            last = at.x;
        }
        assert_eq!(Biome::at(last, 0.0).a, 1, "the next leg is the forest");
        // And the voyage carries on at its own pace afterwards.
        let later = voyage.advance(seconds + 10.0, 0.0);
        assert!((later.x - last - 10.0 * SPEED).abs() < 1e-6);
    }

    #[test]
    fn swipes_back_and_forth_add_up() {
        let mut voyage = Voyage::default();
        voyage.advance(30.0, 0.0);
        voyage.travel(1, 30.0);
        voyage.travel(1, 30.5);
        let at = voyage.advance(30.5 + PASSAGE + 0.1, 0.0);
        assert_eq!(Biome::at(at.x, 0.0).a, 2);
        voyage.travel(-1, 40.0);
        let at = voyage.advance(40.0 + PASSAGE + 0.1, 0.0);
        assert_eq!(Biome::at(at.x, 0.0).a, 1);
    }

    #[test]
    fn the_depth_changes_over_time_but_stays_off_the_seabed() {
        let mut voyage = Voyage::default();
        let mut lowest = f64::MAX;
        let mut highest = f64::MIN;
        let mut seconds = 0.0;
        while seconds < 60.0 {
            let at = voyage.advance(seconds, 0.0);
            let here = Biome::at(at.x, 0.0);
            if here.w == 0.0 {
                lowest = lowest.min(at.depth);
                highest = highest.max(at.depth);
                assert!(at.altitude >= CLEARANCE - 1e-9);
                assert!(at.altitude <= CEILING[here.a] + 1e-9);
            }
            seconds += 1.0 / 60.0;
        }
        assert!(highest - lowest > 0.05, "the Nautilus should rise and sink");
    }

    /// Wherever the cave's vault is drawn, the Nautilus is under it, however
    /// high it cruised before, and whether it drifts in or is swiped in.
    #[test]
    fn the_nautilus_is_always_under_the_vault() {
        for swipe in [false, true] {
            let mut voyage = Voyage::default();
            // Seed 0: leg 3 is Atlantis, where it cruises up to a whole unit high.
            let mut seconds = LEG / SPEED * 3.3;
            voyage.advance(seconds, 0.0);
            if swipe {
                voyage.travel(1, seconds);
            }
            for _ in 0..(60 * 90) {
                seconds += 1.0 / 60.0;
                let at = voyage.advance(seconds, 0.0);
                if Biome::at(at.x, 0.0).weight(CAVE) >= CAVE_SHOWS {
                    assert!(
                        at.altitude < CAVE_ROOF - 0.1,
                        "{:.2} above the seabed, under a vault at {CAVE_ROOF}",
                        at.altitude
                    );
                }
            }
        }
    }

    #[test]
    fn the_abyss_is_deep() {
        let mut voyage = Voyage::default();
        let seconds = LEG / SPEED * (ABYSS as f64 + 0.4);
        let at = voyage.advance(seconds, 0.0);
        assert_eq!(Biome::at(at.x, 0.0).a, ABYSS);
        assert!(at.depth > 30.0);
    }
}
