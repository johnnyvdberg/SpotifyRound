//! Places along the voyage worth a look, for debugging what the dive draws.
//!
//! `DIVE3D_SHOTS_DIR=<dir> cargo test --release dive3d::tests::screenshots --
//! --ignored` writes a full-resolution frame of each, a contact sheet of them
//! all and an `index.html` naming them; `DIVE3D_SHOTS=cave` keeps only the
//! places whose names contain `cave`. Add a place here when a feature needs
//! checking somewhere the legs' own shots do not reach.
//!
//! Every place is in the visit of a given seed: the seabed, the biome circuit
//! and the encounters are all functions of it. `DIVE3D_SHOTS_SEED` overrides
//! the seeds of the leg, seabed and cave shots, so an exact random circuit can
//! be photographed reproducibly.

use crate::voyage::{self, BIOMES, Biome, LEG, SPEED};

/// A moment on the voyage: `leg` legs from the start of a visit (a fraction
/// lands within a leg; each changes into the next from about 0.7 on), in the
/// visit of `seed`, which picks the seabed, the biome order and where the
/// circuit starts.
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

/// A screenshot seed from the environment, if one was asked for.
fn seeded() -> Option<u64> {
    std::env::var("DIVE3D_SHOTS_SEED")
        .ok()
        .and_then(|v| v.trim().parse().ok())
}

fn biome_name(id: usize) -> &'static str {
    match id {
        0 => "shallows",
        1 => "forest",
        2 => "wrecks",
        3 => "atlantis",
        4 => "cave",
        5 => "abyss",
        6 => "arctic",
        _ => "trench",
    }
}

/// Which biome a settled leg of `seed`'s circuit is in.
fn leg_biome(leg: usize, seed: u64) -> usize {
    Biome::at((leg as f64 + 0.5) * LEG, voyage::seed_of(seed)).a
}

/// Which leg of `seed`'s circuit is biome `id`.
fn leg_of(id: usize, seed: u64) -> usize {
    (0..BIOMES)
        .find(|&leg| leg_biome(leg, seed) == id)
        .unwrap_or(0)
}

/// Every place to shoot, in order.
pub fn all() -> Vec<Place> {
    let mut places = Vec::new();
    // Each leg as it settles, in the middle, and as it gives way to the next.
    let seed = seeded().unwrap_or(0);
    for n in 0..BIOMES {
        let name = biome_name(leg_biome(n, seed));
        for (when, at) in [("early", 0.15), ("middle", 0.45), ("leaving", 0.85)] {
            places.push(Place::new(
                format!("{n}-{name}-{when}"),
                n as f64 + at,
                seed,
            ));
        }
    }
    // Another visit's seabed, to see that nothing depends on one seed.
    let other = seeded().unwrap_or(7);
    for n in 0..BIOMES {
        places.push(Place::new(
            format!("{n}-{}-other-seabed", biome_name(leg_biome(n, other))),
            n as f64 + 0.45,
            other,
        ));
    }
    // How the Nautilus moves: setting itself across the cave's tunnel at its
    // mouth, well inside it and leaving it, and part way through a swipe's
    // passage, the rush of bubbles at its height.
    let cave = leg_of(super::world::CAVE, seed) as f64;
    for (name, at) in [("mouth", -0.05), ("deep", 0.4), ("far-mouth", 0.8)] {
        places.push(Place::new(format!("helm-cave-{name}"), cave + at, seed));
    }
    for (name, after) in [("gathering", 0.8), ("rushing", 1.75), ("settling", 3.0)] {
        let mut place = Place::new(format!("helm-swipe-{name}"), 1.5, seed);
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
        places.push(Place::new(format!("{cave}-cave-{name}"), cave + at, seed));
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
        // Named by the kind of meeting and its moment: the legs they were
        // found in move whenever the voyage's order does.
        let kind = name.rsplit_once('-').map_or(name, |(kind, _)| kind);
        places.push(Place::new(
            format!("{kind}-{seconds}s-visit{visit}"),
            at(seconds),
            visit,
        ));
    }
    places
}
