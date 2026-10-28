//! Places along the voyage worth a look, for debugging what the dive draws.
//!
//! `DIVE3D_SHOTS_DIR=<dir> cargo test --release dive3d::tests::screenshots --
//! --ignored` writes a full-resolution frame of each, a contact sheet of them
//! all and an `index.html` naming them; `DIVE3D_SHOTS=cave` keeps only the
//! places whose names contain `cave`. Add a place here when a feature needs
//! checking somewhere the legs' own shots do not reach.

use crate::voyage::{BIOMES, LEG, SPEED};

/// A moment on the voyage: `leg` legs from the start of a visit (a fraction
/// lands within a leg; each changes into the next from about 0.7 on), in the
/// visit of `seed`, which picks the seabed and where the circuit starts.
pub struct Place {
    pub name: String,
    pub leg: f64,
    pub seed: u64,
    /// Seconds since a swipe on to the next leg, for a shot mid-passage.
    pub swiped: Option<f64>,
}

impl Place {
    fn new(name: impl Into<String>, leg: f64, seed: u64) -> Self {
        Self {
            name: name.into(),
            leg,
            seed,
            swiped: None,
        }
    }

    /// Seconds into the visit at which the Nautilus is here.
    pub fn seconds(&self) -> f64 {
        LEG / SPEED * self.leg
    }
}

/// The legs of seed 0's circuit, in the order of `voyage::ORDER`.
const NAMES: [&str; BIOMES] = [
    "shallows", "forest", "trench", "wrecks", "atlantis", "cave", "abyss", "arctic",
];

/// Every place to shoot, in order.
pub fn all() -> Vec<Place> {
    let mut places = Vec::new();
    // Each leg as it settles, in the middle, and as it gives way to the next.
    for (n, name) in NAMES.iter().enumerate() {
        for (when, at) in [("early", 0.15), ("middle", 0.45), ("leaving", 0.85)] {
            places.push(Place::new(format!("{n}-{name}-{when}"), n as f64 + at, 0));
        }
    }
    // Another visit's seabed, to see that nothing depends on one seed.
    for (n, name) in NAMES.iter().enumerate() {
        places.push(Place::new(
            format!("{n}-{name}-other-seabed"),
            n as f64 + 0.45,
            7,
        ));
    }
    // How the Nautilus moves: setting itself across the cave's tunnel at its
    // mouth, well inside it and leaving it, and part way through a swipe's
    // passage, the rush of bubbles at its height.
    let cave = crate::voyage::Biome::leg_of(super::world::CAVE) as f64;
    for (name, at) in [("mouth", -0.05), ("deep", 0.4), ("far-mouth", 0.8)] {
        places.push(Place::new(format!("helm-cave-{name}"), cave + at, 0));
    }
    for (name, after) in [("gathering", 0.8), ("rushing", 1.75), ("settling", 3.0)] {
        let mut place = Place::new(format!("helm-swipe-{name}"), 1.5, 0);
        place.swiped = Some(after);
        places.push(place);
    }
    // Through the lava cave's tunnel and past its secrets, which the window
    // sees ahead of the boat: from the approach, looking in at the mouth,
    // along the passages, and out of the far mouth.
    for (name, at) in [
        ("approach", -0.45),
        ("tunnel-mouth", -0.30),
        ("entering", -0.20),
        ("passages-1", -0.10),
        ("passages-2", -0.02),
        ("passages-3", 0.10),
        ("far-mouth", 0.22),
    ] {
        places.push(Place::new(format!("{cave}-cave-{name}"), cave + at, 0));
    }
    // Another submarine met on the way: the encounters run on the visit's
    // clock (`submarines` in `world.rs`), so these are moments of visits 0
    // and 500000 when one is in view.
    let at = |seconds: f64| seconds / (LEG / SPEED);
    for (name, seconds, visit) in [
        ("encounter-alongside-shallows", 50.0, 0),
        ("encounter-emerge-atlantis", 201.0, 0),
        ("encounter-emerge-abyss", 313.0, 0),
        ("encounter-overtake-forest", 503.0, 0),
        ("encounter-emerge-wrecks", 612.0, 0),
        ("encounter-pass-shallows", 880.0, 0),
        ("encounter-alongside-wrecks", 1037.0, 0),
        ("encounter-far-abyss", 1176.0, 0),
        ("encounter-emerge-arctic", 1288.0, 0),
        ("encounter-pass-forest", 310.0, 500_000),
        ("encounter-pass-abyss", 1005.0, 500_000),
        ("encounter-alongside-arctic", 1491.0, 500_000),
    ] {
        places.push(Place::new(name, at(seconds), visit));
    }
    places
}
