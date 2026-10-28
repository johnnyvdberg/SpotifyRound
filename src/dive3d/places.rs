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
}

impl Place {
    fn new(name: impl Into<String>, leg: f64, seed: u64) -> Self {
        Self {
            name: name.into(),
            leg,
            seed,
        }
    }

    /// Seconds into the visit at which the Nautilus is here.
    pub fn seconds(&self) -> f64 {
        LEG / SPEED * self.leg
    }
}

const NAMES: [&str; BIOMES] = [
    "shallows", "forest", "wrecks", "atlantis", "cave", "abyss", "arctic",
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
    // Through the lava cave's tunnel and past its secrets, which the window
    // sees ahead of the boat: from the approach, looking in at the mouth,
    // along the passages, and out of the far mouth.
    for (name, at) in [
        ("approach", 3.55),
        ("tunnel-mouth", 3.70),
        ("entering", 3.80),
        ("passages-1", 3.90),
        ("passages-2", 3.98),
        ("passages-3", 4.10),
        ("far-mouth", 4.22),
    ] {
        places.push(Place::new(format!("4-cave-{name}"), at, 0));
    }
    places
}
