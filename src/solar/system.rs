//! Star systems, made up from a seed: a star, its planets and their moons.
//!
//! Distances are in Earth radii (6371 km) throughout. Orbits are drawn ten
//! times closer than they would really be, so the next planet is a few minutes
//! away rather than days; the physical numbers the HUD shows (orbital speeds,
//! altitudes, light years) are worked out from the real ones.
//!
//! Which planets a star has follows roughly what is found around real stars:
//! scorched rock and lava close in, rocky worlds and oceans around the
//! habitable zone, deserts and ice beyond it, then gas giants and ice giants.
//! Moons come in the kinds our own system has: grey cratered ones like the
//! Moon and Callisto, icy cracked ones like Europa, volcanic ones like Io,
//! hazy ones like Titan, and small captured rocks like Phobos.

use std::f64::consts::TAU;

/// Kilometres in one unit of distance, the Earth's radius.
pub const KM: f64 = 6371.0;
/// Units in one astronomical unit as drawn: ten times closer than real.
pub const AU: f64 = 2348.0;
/// The real astronomical unit, in units.
pub const REAL_AU: f64 = 23481.0;
/// How much smaller stars are drawn than they would be, so they do not crowd
/// the compressed orbits.
const STAR_SCALE: f64 = 0.3;
/// The gravitational constant times the Earth's mass, in km³/s².
pub const GM_EARTH: f64 = 398_600.4;
/// The same for the Sun.
pub const GM_SUN: f64 = 1.327_124e11;

/// What a body is made of, which decides how its maps are painted. The
/// numbers are the shader's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Terran = 0,
    Desert = 1,
    Ocean = 2,
    Ice = 3,
    Lava = 4,
    Barren = 5,
    GasGiant = 6,
    IceGiant = 7,
    Volcanic = 8,
    IcyMoon = 9,
    Hazy = 10,
    Captured = 11,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Terran => "temperate world",
            Kind::Desert => "desert world",
            Kind::Ocean => "ocean world",
            Kind::Ice => "ice world",
            Kind::Lava => "lava world",
            Kind::Barren => "barren rock",
            Kind::GasGiant => "gas giant",
            Kind::IceGiant => "ice giant",
            Kind::Volcanic => "volcanic moon",
            Kind::IcyMoon => "icy moon",
            Kind::Hazy => "hazy moon",
            Kind::Captured => "captured asteroid",
        }
    }

    pub fn is_giant(self) -> bool {
        matches!(self, Kind::GasGiant | Kind::IceGiant)
    }

    /// Share of the surface below sea level, for kinds with a sea of
    /// something (water, lava, dark lowland plains).
    pub fn lowland(self) -> f64 {
        match self {
            Kind::Terran => 0.68,
            Kind::Ocean => 0.93,
            Kind::Desert => 0.30,
            Kind::Ice => 0.55,
            Kind::Lava => 0.22,
            Kind::Barren => 0.20,
            _ => 0.0,
        }
    }

    /// Vertical optical depth of the air in red, green and blue; zero for
    /// none.
    pub fn air(self) -> [f32; 3] {
        match self {
            Kind::Terran => [0.045, 0.100, 0.230],
            Kind::Ocean => [0.050, 0.110, 0.250],
            Kind::Desert => [0.050, 0.034, 0.022],
            Kind::Ice => [0.020, 0.040, 0.080],
            Kind::Lava => [0.140, 0.070, 0.030],
            Kind::GasGiant => [0.030, 0.036, 0.050],
            Kind::IceGiant => [0.020, 0.070, 0.090],
            Kind::Hazy => [0.260, 0.150, 0.050],
            _ => [0.0, 0.0, 0.0],
        }
    }

    /// Whether the night side shows city lights.
    pub fn has_cities(self) -> bool {
        matches!(self, Kind::Terran | Kind::Ocean)
    }

    /// Whether the night map glows on both sides: molten rock.
    pub fn glows(self) -> bool {
        matches!(self, Kind::Lava | Kind::Volcanic)
    }
}

/// A star: its spectral class and what follows from it.
#[derive(Clone, Debug)]
pub struct Star {
    pub name: String,
    pub class: char,
    /// Surface temperature in kelvin.
    pub temperature: f64,
    /// Radius as drawn, in units.
    pub radius: f64,
    /// Mass in suns.
    pub mass: f64,
    /// Colour of its light, linear, normalised to the brightest channel.
    pub colour: [f32; 3],
}

/// Rings around a planet, in its equatorial plane.
#[derive(Clone, Debug)]
pub struct Rings {
    /// Inner and outer radius, in planet radii.
    pub inner: f64,
    pub outer: f64,
    /// Linear colour and how opaque the densest ringlets are.
    pub colour: [f32; 3],
    pub opacity: f32,
    pub seed: f32,
}

#[derive(Clone, Debug)]
pub struct Moon {
    pub name: String,
    pub kind: Kind,
    /// Radius in units.
    pub radius: f64,
    /// Orbit radius around the planet, in units, as drawn.
    pub orbit: f64,
    /// Seconds for one orbit as drawn, time compressed.
    pub period: f64,
    pub phase: f64,
    /// Tilt of its orbit from the planet's equator, radians.
    pub inclination: f64,
    pub palette: [[f32; 3]; 4],
    pub seed: u64,
}

#[derive(Clone, Debug)]
pub struct Planet {
    pub name: String,
    pub kind: Kind,
    /// Radius in units (Earth radii).
    pub radius: f64,
    /// Mass in Earth masses.
    pub mass: f64,
    /// Orbit radius around the star as drawn, in units.
    pub orbit: f64,
    /// Angle along its orbit at the start of the visit.
    pub phase: f64,
    /// Seconds for one turn on its axis, as drawn.
    pub day: f64,
    /// Axial tilt, radians, and the direction it leans.
    pub tilt: f64,
    pub lean: f64,
    pub rings: Option<Rings>,
    pub moons: Vec<Moon>,
    pub palette: [[f32; 3]; 4],
    pub seed: u64,
}

impl Planet {
    /// Its position around the star at `seconds` into the visit. Planets crawl
    /// along their orbits; over a visit they hardly move.
    pub fn position(&self, seconds: f64) -> [f64; 3] {
        let years = (self.orbit / AU).powf(1.5);
        let angle = self.phase + seconds * TAU / (years * 7200.0);
        [self.orbit * angle.cos(), 0.0, self.orbit * angle.sin()]
    }

    /// The planet's rotation axis, in the system's frame.
    pub fn axis(&self) -> [f64; 3] {
        let (st, ct) = self.tilt.sin_cos();
        let (sl, cl) = self.lean.sin_cos();
        [st * cl, ct, st * sl]
    }

    /// Where moon `index` is relative to the planet at `seconds`.
    pub fn moon_offset(&self, index: usize, seconds: f64) -> [f64; 3] {
        let moon = &self.moons[index];
        let angle = moon.phase + seconds * TAU / moon.period;
        let (s, c) = angle.sin_cos();
        let (si, ci) = moon.inclination.sin_cos();
        // In the equatorial frame (axis up), then tilted with the planet.
        let local = [moon.orbit * c, moon.orbit * s * si, moon.orbit * s * ci];
        equatorial_to_system(self, local)
    }

    /// Real orbital speed at `distance` units from the centre, in km/s.
    pub fn orbital_speed(&self, distance: f64) -> f64 {
        (GM_EARTH * self.mass / (distance * KM)).sqrt()
    }
}

/// From a planet's equatorial frame (y along its axis) into the system's.
pub fn equatorial_to_system(planet: &Planet, v: [f64; 3]) -> [f64; 3] {
    let (st, ct) = planet.tilt.sin_cos();
    let (sl, cl) = planet.lean.sin_cos();
    // Tilt about z by `tilt`, then turn about y by `lean`.
    let x1 = v[0] * ct + v[1] * st;
    let y1 = -v[0] * st + v[1] * ct;
    let z1 = v[2];
    [x1 * cl - z1 * sl, y1, x1 * sl + z1 * cl]
}

#[derive(Clone, Debug)]
pub struct System {
    pub seed: u64,
    pub star: Star,
    pub planets: Vec<Planet>,
    /// Light years from the previous system.
    pub distance: f64,
    /// Direction of the sky's galactic band, and a seed for its look.
    pub sky: [f32; 4],
}

/// A small deterministic generator.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Self(seed ^ 0x5851_f42d_4c95_7f2d)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    pub fn range(&mut self, low: f64, high: f64) -> f64 {
        low + (high - low) * self.unit()
    }

    pub fn int(&mut self, low: u64, high: u64) -> u64 {
        low + self.next() % (high - low + 1)
    }

    pub fn chance(&mut self, p: f64) -> bool {
        self.unit() < p
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[(self.next() % items.len() as u64) as usize]
    }
}

/// Linear RGB of a black body at `kelvin`, brightest channel 1. A fit to the
/// Planckian locus good from about 2000 K to 40000 K.
pub fn blackbody(kelvin: f64) -> [f32; 3] {
    let t = kelvin / 100.0;
    let r = if t <= 66.0 {
        255.0
    } else {
        329.7 * (t - 60.0).powf(-0.1332)
    };
    let g = if t <= 66.0 {
        99.47 * t.ln() - 161.12
    } else {
        288.12 * (t - 60.0).powf(-0.0755)
    };
    let b = if t >= 66.0 {
        255.0
    } else if t <= 19.0 {
        0.0
    } else {
        138.52 * (t - 10.0).ln() - 305.04
    };
    let srgb = [r, g, b].map(|c| (c / 255.0).clamp(0.0, 1.0));
    let linear = srgb.map(|c| c.powf(2.2));
    let top = linear.iter().cloned().fold(0.0, f64::max).max(1e-6);
    linear.map(|c| (c / top) as f32)
}

fn srgb(r: u8, g: u8, b: u8) -> [f32; 3] {
    [r, g, b].map(|c| (c as f32 / 255.0).powf(2.2))
}

const SYLLABLES: &[&str] = &[
    "ka", "ve", "lo", "ri", "sa", "tor", "mi", "an", "el", "dra", "su", "na", "ish", "or", "ta",
    "len", "ce", "ru", "vo", "bel", "ky", "ne", "os", "ar", "thi", "qua", "zen", "il", "mo", "ra",
];

fn proper_name(rng: &mut Rng) -> String {
    let count = rng.int(2, 3);
    let mut name = String::new();
    for _ in 0..count {
        name.push_str(rng.pick(SYLLABLES));
    }
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => name,
    }
}

fn catalogue_name(rng: &mut Rng) -> String {
    let catalogue = *rng.pick(&[
        "HD", "Gliese", "Kepler", "TOI", "HIP", "Wolf", "Ross", "LHS",
    ]);
    match catalogue {
        "HD" | "HIP" => format!("{catalogue} {}", rng.int(1000, 229_999)),
        "Kepler" | "TOI" => format!("{catalogue}-{}", rng.int(10, 4999)),
        _ => format!("{catalogue} {}", rng.int(10, 1199)),
    }
}

/// Palettes, linear: gas giants' bands, and the tints of rocky kinds.
fn palette(kind: Kind, rng: &mut Rng) -> [[f32; 3]; 4] {
    match kind {
        Kind::GasGiant => *rng.pick(&[
            // Banded beige and rust, like Jupiter.
            [
                srgb(222, 204, 172),
                srgb(176, 120, 82),
                srgb(140, 96, 70),
                srgb(236, 226, 206),
            ],
            // Pale gold, like Saturn.
            [
                srgb(226, 206, 152),
                srgb(196, 166, 110),
                srgb(170, 146, 102),
                srgb(240, 228, 190),
            ],
            // A hot giant's blue-grey haze.
            [
                srgb(120, 140, 170),
                srgb(70, 86, 120),
                srgb(160, 170, 190),
                srgb(40, 50, 80),
            ],
            // Rose and cream.
            [
                srgb(214, 170, 160),
                srgb(160, 100, 96),
                srgb(236, 214, 196),
                srgb(120, 80, 84),
            ],
        ]),
        Kind::IceGiant => *rng.pick(&[
            [
                srgb(166, 214, 222),
                srgb(140, 196, 210),
                srgb(190, 228, 232),
                srgb(120, 180, 200),
            ],
            [
                srgb(70, 110, 200),
                srgb(54, 90, 180),
                srgb(110, 150, 220),
                srgb(40, 70, 150),
            ],
        ]),
        Kind::Desert => *rng.pick(&[
            [
                srgb(190, 110, 70),
                srgb(120, 70, 50),
                srgb(210, 150, 100),
                srgb(80, 60, 50),
            ],
            [
                srgb(200, 170, 120),
                srgb(150, 120, 90),
                srgb(226, 200, 150),
                srgb(96, 84, 70),
            ],
        ]),
        Kind::Barren | Kind::Captured => [
            srgb(130, 126, 120),
            srgb(80, 78, 76),
            srgb(170, 166, 160),
            srgb(60, 56, 52),
        ],
        Kind::Volcanic => [
            srgb(226, 206, 110),
            srgb(200, 130, 60),
            srgb(240, 236, 210),
            srgb(40, 30, 24),
        ],
        Kind::IcyMoon => [
            srgb(226, 220, 206),
            srgb(150, 110, 80),
            srgb(240, 240, 238),
            srgb(180, 170, 160),
        ],
        Kind::Hazy => [
            srgb(214, 150, 70),
            srgb(170, 110, 50),
            srgb(230, 180, 100),
            srgb(130, 80, 40),
        ],
        Kind::Ice => [
            srgb(232, 240, 246),
            srgb(150, 190, 220),
            srgb(250, 252, 255),
            srgb(110, 140, 170),
        ],
        Kind::Lava => [
            srgb(32, 26, 24),
            srgb(255, 110, 20),
            srgb(60, 50, 46),
            srgb(255, 200, 80),
        ],
        Kind::Terran | Kind::Ocean => [srgb(255, 255, 255); 4],
    }
}

fn moons_for(planet_kind: Kind, radius: f64, rng: &mut Rng) -> Vec<Moon> {
    let count = match planet_kind {
        Kind::GasGiant => rng.int(3, 6),
        Kind::IceGiant => rng.int(2, 4),
        Kind::Terran | Kind::Ocean => rng.int(0, 2),
        Kind::Desert => rng.int(0, 2),
        Kind::Ice | Kind::Barren => rng.int(0, 1),
        _ => 0,
    } as usize;
    let mut moons = Vec::with_capacity(count);
    let mut orbit = radius * if planet_kind.is_giant() { 3.4 } else { 4.0 };
    for i in 0..count {
        let (kind, moon_radius) = if planet_kind.is_giant() {
            // Giants: volcanic close in, icy and cratered further out, a hazy
            // one sometimes, small captured rocks at the edge.
            let kind = match (i, rng.unit()) {
                (0, x) if x < 0.55 => Kind::Volcanic,
                (_, x) if x < 0.35 => Kind::IcyMoon,
                (_, x) if x < 0.62 => Kind::Barren,
                (_, x) if x < 0.80 => Kind::Hazy,
                _ => Kind::Captured,
            };
            let r = match kind {
                Kind::Captured => rng.range(0.03, 0.08),
                _ => rng.range(0.22, 0.44),
            };
            (kind, r)
        } else if matches!(planet_kind, Kind::Terran | Kind::Ocean) && i == 0 {
            // A large grey companion, like ours.
            (Kind::Barren, radius * rng.range(0.2, 0.3))
        } else {
            (Kind::Captured, rng.range(0.02, 0.05))
        };
        let period = 160.0 * (orbit / (radius * 4.0)).powf(1.5) * rng.range(0.9, 1.1);
        moons.push(Moon {
            name: proper_name(rng),
            kind,
            radius: moon_radius,
            orbit,
            period,
            phase: rng.range(0.0, TAU),
            inclination: rng.range(-0.12, 0.12),
            palette: palette(kind, rng),
            seed: rng.next(),
        });
        orbit *= rng.range(1.45, 1.8);
    }
    moons
}

impl System {
    /// The system for `seed`.
    pub fn generate(seed: u64) -> Self {
        let mut rng = Rng::new(seed);
        // Mostly the common, cooler classes.
        let roll = rng.unit();
        let (class, temperature, radius, mass) = if roll < 0.30 {
            (
                'M',
                rng.range(2800.0, 3700.0),
                rng.range(0.35, 0.6),
                rng.range(0.2, 0.55),
            )
        } else if roll < 0.58 {
            (
                'K',
                rng.range(3900.0, 5200.0),
                rng.range(0.7, 0.9),
                rng.range(0.6, 0.85),
            )
        } else if roll < 0.85 {
            (
                'G',
                rng.range(5300.0, 6000.0),
                rng.range(0.9, 1.15),
                rng.range(0.9, 1.1),
            )
        } else if roll < 0.96 {
            (
                'F',
                rng.range(6100.0, 7400.0),
                rng.range(1.2, 1.5),
                rng.range(1.15, 1.45),
            )
        } else {
            (
                'A',
                rng.range(7600.0, 9800.0),
                rng.range(1.6, 2.2),
                rng.range(1.6, 2.4),
            )
        };
        let luminosity = radius * radius * (temperature / 5772.0).powi(4);
        let catalogue = catalogue_name(&mut rng);
        let star = Star {
            name: catalogue.clone(),
            class,
            temperature,
            radius: radius * 109.0 * STAR_SCALE,
            mass,
            // Eyes adapt to their star: its light is tinted, not saturated.
            colour: blackbody(temperature).map(|c| 0.45 + 0.55 * c),
        };

        // Orbits spaced geometrically from just outside the star, kinds by
        // where each lies against the habitable zone.
        let habitable = luminosity.sqrt();
        let count = rng.int(5, 8) as usize;
        let mut distance = habitable * rng.range(0.3, 0.45);
        let mut planets = Vec::with_capacity(count);
        let letters = ['b', 'c', 'd', 'e', 'f', 'g', 'h', 'i'];
        let mut giants = 0;
        for letter in letters.iter().take(count) {
            let x = distance / habitable;
            let kind = if x < 0.55 {
                *rng.pick(&[Kind::Lava, Kind::Lava, Kind::Barren])
            } else if x < 0.85 {
                *rng.pick(&[Kind::Desert, Kind::Barren, Kind::Lava])
            } else if x < 1.4 {
                *rng.pick(&[Kind::Terran, Kind::Terran, Kind::Ocean, Kind::Desert])
            } else if x < 2.6 {
                *rng.pick(&[Kind::Desert, Kind::Ice, Kind::Barren])
            } else if x < 9.0 || giants == 0 {
                if rng.chance(0.75) {
                    Kind::GasGiant
                } else {
                    Kind::Ice
                }
            } else {
                *rng.pick(&[Kind::IceGiant, Kind::IceGiant, Kind::Ice])
            };
            if kind.is_giant() {
                giants += 1;
            }
            let radius = match kind {
                Kind::Terran => rng.range(0.85, 1.35),
                Kind::Ocean => rng.range(1.0, 1.7),
                Kind::Desert => rng.range(0.5, 1.0),
                Kind::Ice => rng.range(0.4, 1.1),
                Kind::Lava => rng.range(0.6, 1.4),
                Kind::Barren => rng.range(0.3, 0.8),
                Kind::GasGiant => rng.range(7.5, 12.0),
                Kind::IceGiant => rng.range(3.5, 4.4),
                _ => 1.0,
            };
            let mass = match kind {
                Kind::GasGiant => rng.range(60.0, 400.0),
                Kind::IceGiant => rng.range(13.0, 20.0),
                _ => radius.powf(3.7),
            };
            let name = format!("{} {letter}", star.name);
            let rings = if (kind == Kind::GasGiant && rng.chance(0.65))
                || (kind == Kind::IceGiant && rng.chance(0.4))
            {
                let tint = palette(kind, &mut rng)[3];
                Some(Rings {
                    inner: rng.range(1.25, 1.55),
                    outer: rng.range(2.0, 2.5),
                    colour: [
                        0.55 + 0.45 * tint[0],
                        0.52 + 0.45 * tint[1],
                        0.48 + 0.45 * tint[2],
                    ],
                    opacity: rng.range(0.55, 0.9) as f32,
                    seed: rng.range(0.0, 100.0) as f32,
                })
            } else {
                None
            };
            let moons = moons_for(kind, radius, &mut rng);
            let palette = palette(kind, &mut rng);
            planets.push(Planet {
                name,
                kind,
                radius,
                mass,
                orbit: distance * AU,
                phase: rng.range(0.0, TAU),
                day: if kind.is_giant() {
                    rng.range(150.0, 260.0)
                } else {
                    rng.range(240.0, 600.0)
                },
                tilt: rng.range(0.0, 0.5),
                lean: rng.range(0.0, TAU),
                rings,
                moons,
                palette,
                seed: rng.next(),
            });
            distance *= rng.range(1.5, 1.95);
        }

        let sky = {
            let z = rng.range(-1.0, 1.0);
            let phi = rng.range(0.0, TAU);
            let r = (1.0 - z * z).sqrt();
            [
                (r * phi.cos()) as f32,
                z as f32,
                (r * phi.sin()) as f32,
                rng.range(0.0, 100.0) as f32,
            ]
        };
        System {
            seed,
            star,
            planets,
            distance: rng.range(4.0, 40.0),
            sky,
        }
    }

    /// The planets to visit, in the order they are visited: three of them, as
    /// different from each other as the system allows, inner to outer.
    pub fn itinerary(&self) -> Vec<usize> {
        let mut rng = Rng::new(self.seed ^ 0x1717);
        let mut chosen: Vec<usize> = Vec::new();
        // One of each broad family if there is one: rocky, giant, other.
        let families: [&dyn Fn(Kind) -> bool; 3] = [
            &|k| matches!(k, Kind::Terran | Kind::Ocean),
            &|k| k.is_giant(),
            &|k| !matches!(k, Kind::Terran | Kind::Ocean) && !k.is_giant(),
        ];
        for family in families {
            let candidates: Vec<usize> = (0..self.planets.len())
                .filter(|&i| family(self.planets[i].kind) && !chosen.contains(&i))
                .collect();
            if !candidates.is_empty() {
                chosen.push(*rng.pick(&candidates));
            }
        }
        while chosen.len() < 3.min(self.planets.len()) {
            let i = rng.int(0, self.planets.len() as u64 - 1) as usize;
            if !chosen.contains(&i) {
                chosen.push(i);
            }
        }
        chosen.sort_unstable();
        chosen
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn systems_are_reproducible_and_varied() {
        let a = System::generate(42);
        let b = System::generate(42);
        assert_eq!(a.planets.len(), b.planets.len());
        assert_eq!(a.star.name, b.star.name);
        let mut kinds = std::collections::HashSet::new();
        for seed in 0..64 {
            let system = System::generate(seed);
            assert!((5..=8).contains(&system.planets.len()));
            for planet in &system.planets {
                kinds.insert(planet.kind as u32);
                for moon in &planet.moons {
                    kinds.insert(moon.kind as u32);
                }
            }
            let tour = system.itinerary();
            assert_eq!(tour.len(), 3);
            assert!(tour.windows(2).all(|w| w[0] < w[1]));
        }
        // Every kind turns up somewhere across the seeds.
        assert_eq!(kinds.len(), 12, "{kinds:?}");
    }

    #[test]
    fn real_orbital_speeds() {
        let earth = Planet {
            name: String::new(),
            kind: Kind::Terran,
            radius: 1.0,
            mass: 1.0,
            orbit: AU,
            phase: 0.0,
            day: 1.0,
            tilt: 0.0,
            lean: 0.0,
            rings: None,
            moons: Vec::new(),
            palette: [[1.0; 3]; 4],
            seed: 0,
        };
        // Low Earth orbit, 400 km up: about 7.7 km/s.
        let v = earth.orbital_speed(1.0 + 400.0 / KM);
        assert!((v - 7.67).abs() < 0.05, "{v}");
    }

    #[test]
    fn the_sun_is_white_and_red_dwarfs_are_orange() {
        let sun = blackbody(5772.0);
        assert!(sun[2] > 0.7 && sun[0] == 1.0);
        let dwarf = blackbody(3000.0);
        assert!(dwarf[2] < 0.3);
    }
}
