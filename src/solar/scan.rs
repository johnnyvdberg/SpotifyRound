//! The scanner: while the camera orbits a planet, a sequence of readings,
//! each with a progress bar while it runs and a result when it is done.
//!
//! The results are worked out from the planet as generated: its kind, mass,
//! radius, its star's light and its distance from it. Units are the ones a
//! survey would report: bar, kelvin, m/s², km/s, microtesla, millisievert per
//! day. The gases are what a world of that kind would plausibly hold (an
//! Earth-like mix, carbon dioxide on desert and greenhouse worlds, hydrogen
//! and helium on giants), varied a little per planet.

use super::system::{AU, Kind, Planet, Rng, Star};

/// One reading: its name, how far along it is (0..1), and its result, or a
/// note of what it is doing while it runs.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Scan {
    pub name: String,
    pub progress: f32,
    pub value: String,
}

/// When each reading starts and how long it takes, in seconds after the
/// camera settles into orbit. Overlapping, as several instruments run at once.
const SCHEDULE: [(&str, f64, f64, &str); 6] = [
    ("Atmosphere", 2.0, 14.0, "spectrometer: absorption lines"),
    ("Gravimetry", 8.0, 10.0, "tracking orbital perturbations"),
    ("Climate", 14.0, 14.0, "thermal infrared mapping"),
    ("Magnetometer", 24.0, 12.0, "sampling the field"),
    ("Radiation", 34.0, 10.0, "dosimeters integrating"),
    (
        "Biosignatures",
        42.0,
        22.0,
        "searching spectra for disequilibrium",
    ),
];

/// Readings to show at once: the latest few, the running ones last.
const SHOWN: usize = 4;

fn subscript(formula: &str) -> String {
    formula
        .chars()
        .map(|c| match c {
            '0'..='9' => char::from_u32(0x2080 + c as u32 - '0' as u32).unwrap_or(c),
            _ => c,
        })
        .collect()
}

/// A gas mixture as "N₂ 77% O₂ 21% Ar 0.9%", normalised to 100%.
fn mixture(gases: &[(&str, f64)]) -> String {
    let total: f64 = gases.iter().map(|(_, share)| share).sum();
    gases
        .iter()
        .map(|(gas, share)| {
            let percent = share / total * 100.0;
            let shown = if percent >= 10.0 {
                format!("{percent:.0}")
            } else if percent >= 1.0 {
                format!("{percent:.1}")
            } else {
                format!("{percent:.2}")
            };
            format!("{} {shown}%", subscript(gas))
        })
        .collect::<Vec<_>>()
        .join(" · ")
}

/// Everything the scanner reports about a planet, worked out once.
pub struct Survey {
    atmosphere: String,
    gravimetry: String,
    climate: String,
    magnetometer: String,
    radiation: String,
    biosignatures: String,
}

impl Survey {
    pub fn of(planet: &Planet, star: &Star) -> Self {
        let mut rng = Rng::new(planet.seed ^ 0x5ca7);
        let kind = planet.kind;
        let giant = kind.is_giant();

        // Pressure at the surface (or, on a giant, quoted at the one-bar level)
        // and the gases.
        let (pressure, gases): (f64, Vec<(&str, f64)>) = match kind {
            Kind::Terran => (
                rng.range(0.7, 1.4),
                vec![
                    ("N2", rng.range(72.0, 80.0)),
                    ("O2", rng.range(17.0, 24.0)),
                    ("Ar", 0.9),
                    ("CO2", 0.04),
                ],
            ),
            Kind::Ocean => (
                rng.range(1.0, 3.0),
                vec![
                    ("N2", rng.range(70.0, 78.0)),
                    ("O2", rng.range(16.0, 22.0)),
                    ("H2O", rng.range(2.0, 4.0)),
                    ("Ar", 1.0),
                ],
            ),
            Kind::Jungle => (
                rng.range(1.2, 2.0),
                vec![
                    ("N2", rng.range(66.0, 72.0)),
                    ("O2", rng.range(26.0, 31.0)),
                    ("Ar", 0.9),
                    ("CO2", 0.06),
                ],
            ),
            Kind::Eyeball => (
                rng.range(0.8, 1.5),
                vec![
                    ("N2", rng.range(80.0, 90.0)),
                    ("CO2", rng.range(4.0, 10.0)),
                    ("O2", rng.range(0.2, 1.0)),
                    ("Ar", 1.2),
                ],
            ),
            Kind::Desert => (
                rng.range(0.006, 0.05),
                vec![("CO2", 95.0), ("N2", 2.8), ("Ar", 2.0), ("O2", 0.17)],
            ),
            Kind::Ice => (
                rng.range(0.01, 0.4),
                vec![
                    ("N2", rng.range(85.0, 95.0)),
                    ("CH4", rng.range(2.0, 8.0)),
                    ("CO", 0.5),
                ],
            ),
            Kind::Lava => (
                rng.range(5.0, 60.0),
                vec![
                    ("CO2", rng.range(70.0, 90.0)),
                    ("SO2", rng.range(5.0, 20.0)),
                    ("Na", 1.0),
                ],
            ),
            Kind::Cloudy => (
                rng.range(60.0, 95.0),
                vec![("CO2", 96.5), ("N2", 3.5), ("SO2", 0.015)],
            ),
            Kind::GasGiant => (
                1.0,
                vec![
                    ("H2", rng.range(84.0, 90.0)),
                    ("He", rng.range(10.0, 15.0)),
                    ("CH4", 0.3),
                    ("NH3", 0.03),
                ],
            ),
            Kind::IceGiant => (
                1.0,
                vec![
                    ("H2", rng.range(78.0, 83.0)),
                    ("He", rng.range(15.0, 19.0)),
                    ("CH4", rng.range(1.5, 2.5)),
                ],
            ),
            Kind::HotJupiter => (
                1.0,
                vec![("H2", 85.0), ("He", 14.0), ("Na", 0.01), ("K", 0.001)],
            ),
            Kind::Hazy => (
                rng.range(1.2, 1.6),
                vec![
                    ("N2", rng.range(94.0, 98.0)),
                    ("CH4", rng.range(1.5, 5.0)),
                    ("H2", 0.1),
                ],
            ),
            _ => (0.0, Vec::new()),
        };
        let atmosphere = if gases.is_empty() {
            "none: exosphere below 1 nbar".to_string()
        } else if giant {
            format!("at 1 bar · {}", mixture(&gases))
        } else {
            let p = if pressure < 0.1 {
                format!("{:.1} mbar", pressure * 1000.0)
            } else {
                format!("{pressure:.2} bar")
            };
            format!("{p} · {}", mixture(&gases))
        };

        // Surface gravity and escape velocity from mass and radius, in Earth
        // units: 9.81 m/s² and 11.19 km/s.
        let g = 9.81 * planet.mass / (planet.radius * planet.radius);
        let escape = 11.19 * (planet.mass / planet.radius).sqrt();
        let gravimetry = format!(
            "{g:.1} m/s² · escape {escape:.1} km/s · {:.2} Earth masses",
            planet.mass
        );

        // Equilibrium temperature from the star's luminosity and the real
        // distance, with the kind's albedo, then its greenhouse warming.
        let luminosity = star.luminosity();
        let distance = planet.orbit / AU;
        let albedo = match kind {
            Kind::Terran => 0.30,
            Kind::Ocean => 0.25,
            Kind::Jungle => 0.20,
            Kind::Eyeball => 0.30,
            Kind::Desert => 0.25,
            Kind::Ice => 0.65,
            Kind::Lava => 0.10,
            Kind::Cloudy => 0.75,
            Kind::GasGiant => 0.50,
            Kind::IceGiant => 0.30,
            Kind::HotJupiter => 0.05,
            _ => 0.12,
        };
        let equilibrium =
            278.6 * luminosity.powf(0.25) / distance.sqrt() * (1.0f64 - albedo).powf(0.25);
        let greenhouse = match kind {
            Kind::Terran | Kind::Eyeball => rng.range(25.0, 40.0),
            Kind::Ocean | Kind::Jungle => rng.range(35.0, 50.0),
            Kind::Cloudy => rng.range(400.0, 500.0),
            Kind::Lava => rng.range(150.0, 400.0),
            Kind::Hazy => rng.range(5.0, 15.0),
            _ => 0.0,
        };
        let mean = equilibrium + greenhouse + planet.heat;
        let spread = if gases.is_empty() { 0.45 } else { 0.12 } * mean;
        let climate = if giant {
            format!(
                "cloud tops {:.0} K · effective {:.0} K",
                equilibrium * 0.9,
                equilibrium
            )
        } else if kind.locked() {
            format!(
                "dayside {:.0} K · nightside {:.0} K · locked",
                mean * 1.15,
                (mean * 0.55).max(40.0)
            )
        } else {
            format!(
                "mean {:.0} K ({:+.0} °C) · {:.0}–{:.0} K",
                mean,
                mean - 273.15,
                mean - spread,
                mean + spread
            )
        };

        // The field at the surface (cloud tops for giants), in microtesla.
        let field = match kind {
            Kind::Terran | Kind::Ocean | Kind::Jungle => rng.range(25.0, 65.0),
            Kind::Eyeball => rng.range(2.0, 15.0),
            Kind::GasGiant | Kind::HotJupiter => rng.range(400.0, 1400.0),
            Kind::IceGiant => rng.range(20.0, 110.0),
            Kind::Lava => rng.range(2.0, 20.0),
            Kind::Ice => rng.range(0.5, 5.0),
            _ => rng.range(0.0, 0.8),
        };
        let magnetometer = if field < 1.0 {
            format!("{:.0} nT · no global dipole", field * 1000.0)
        } else if field > 300.0 {
            format!("{field:.0} µT · intense radiation belts")
        } else if field > 20.0 {
            format!("{field:.0} µT dipole · auroral ovals")
        } else {
            format!("{field:.1} µT · weak, patchy")
        };

        // The daily dose at the surface: shielded by field and air, raised
        // close to a bright star and in a giant's belts.
        let shield = (1.0 + field / 10.0) * (1.0 + pressure.min(10.0) * 2.0);
        let mut dose = 0.6 * luminosity.sqrt() / distance.max(0.05) / shield;
        if giant {
            dose = rng.range(5.0, 3600.0);
        }
        let radiation = if dose < 1.0 {
            format!("{:.2} mSv/day at the surface", dose)
        } else if giant {
            format!("{dose:.0} mSv/day in orbit · lethal in hours")
        } else {
            format!("{dose:.1} mSv/day at the surface")
        };

        let biosignatures = match kind {
            Kind::Terran | Kind::Ocean | Kind::Jungle => format!(
                "POSITIVE ({:.0}%): O₂ with CH₄ out of equilibrium · red edge at {:.2} µm",
                rng.range(92.0, 99.5),
                rng.range(0.69, 0.73)
            ),
            Kind::Eyeball => format!(
                "POSSIBLE ({:.0}%): O₂ and pigments near-IR at the substellar ocean",
                rng.range(55.0, 80.0)
            ),
            Kind::Ice => format!(
                "inconclusive ({:.0}%): CH₄ from below the ice?",
                rng.range(15.0, 40.0)
            ),
            Kind::Hazy => "prebiotic: tholins, HCN, C₂H₂ in the haze".to_string(),
            Kind::Cloudy => format!(
                "inconclusive ({:.0}%): PH₃ in the cloud deck",
                rng.range(10.0, 35.0)
            ),
            _ => "none detected".to_string(),
        };

        Survey {
            atmosphere,
            gravimetry,
            climate,
            magnetometer,
            radiation,
            biosignatures,
        }
    }

    /// The readings `seconds` after settling into orbit: finished ones with
    /// their results, running ones with their progress.
    pub fn at(&self, seconds: f64) -> Vec<Scan> {
        let results = [
            &self.atmosphere,
            &self.gravimetry,
            &self.climate,
            &self.magnetometer,
            &self.radiation,
            &self.biosignatures,
        ];
        let started: Vec<Scan> = SCHEDULE
            .iter()
            .zip(results)
            .filter(|((_, start, _, _), _)| seconds >= *start)
            .map(|((name, start, length, doing), result)| {
                let progress = ((seconds - start) / length).clamp(0.0, 1.0) as f32;
                Scan {
                    name: (*name).to_string(),
                    progress,
                    value: if progress >= 1.0 {
                        result.clone()
                    } else {
                        format!("{doing}…")
                    },
                }
            })
            .collect();
        let skip = started.len().saturating_sub(SHOWN);
        started.into_iter().skip(skip).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::super::system::System;
    use super::*;

    #[test]
    fn an_earthlike_world_reads_like_earth() {
        // Find a temperate planet and check its readings are of the right size.
        for seed in 0..200 {
            let system = System::generate(seed);
            for planet in &system.planets {
                if planet.kind != Kind::Terran {
                    continue;
                }
                let survey = Survey::of(planet, &system.star);
                assert!(survey.atmosphere.contains("N₂"), "{}", survey.atmosphere);
                assert!(survey.atmosphere.contains("bar"), "{}", survey.atmosphere);
                let g: f64 = survey
                    .gravimetry
                    .split(' ')
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                assert!((3.0..30.0).contains(&g), "{g}");
                assert!(survey.biosignatures.starts_with("POSITIVE"));
                return;
            }
        }
        panic!("no temperate world in 200 systems");
    }

    #[test]
    fn readings_run_then_report() {
        let system = System::generate(1);
        let survey = Survey::of(&system.planets[0], &system.star);
        assert!(survey.at(0.0).is_empty());
        let running = survey.at(5.0);
        assert_eq!(running.len(), 1);
        assert!(running[0].progress > 0.0 && running[0].progress < 1.0);
        assert!(running[0].value.ends_with('…'));
        let late = survey.at(200.0);
        assert_eq!(late.len(), SHOWN);
        assert!(
            late.iter()
                .all(|s| s.progress >= 1.0 && !s.value.ends_with('…'))
        );
    }
}
