//! Labels on the worlds in view: a leader from the up-right of a planet's or
//! moon's limb, 45° up and then a short way across, with its name above the
//! line and its distance below.
//!
//! The nearest world in view always gets one, unless it fills more than a
//! quarter of the screen (then the HUD's own panels already name it); farther
//! ones get one too wherever there is room on the screen for theirs, nearest
//! first. A label that would run off the circle or into one of the HUD's
//! panels turns to the left instead, and one with room on neither side is
//! left out.

use super::SIZE;
use super::system::{AU, KM, Stop, System};
use super::tour::{View, sub, thousands};

/// The leader's run at 45°, each way, in pixels.
pub const RISE: f32 = 70.0;
/// From the limb to the leader's start, in pixels.
const GAP: f64 = 10.0;
/// The most labels shown at once.
const MOST: usize = 4;
/// The share of the screen a world may fill and keep its label.
const LARGEST: f64 = 0.25;
/// Room kept clear round each label, in pixels.
const SPACING: f64 = 22.0;

/// A label on a world: where its leader starts, in the frame's pixels, which
/// way it runs, and what it says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BodyLabel {
    pub at: [f32; 2],
    /// The leader runs up to the left instead of to the right.
    pub left: bool,
    pub name: String,
    pub distance: String,
}

/// A world as it falls on the screen.
struct Seen {
    name: String,
    /// The centre of its disc and its radius, in pixels.
    centre: [f64; 2],
    radius: f64,
    /// From the camera to its centre, and to its surface, in units.
    depth: f64,
    surface: f64,
}

/// The labels for the worlds of `system` the camera sees at `t`, kept clear
/// of `panels`: the HUD's panels that are up, in the frame's pixels, as
/// `[left, top, right, bottom]`.
pub fn labels(system: &System, view: &View, t: f64, panels: &[[f64; 4]]) -> Vec<BodyLabel> {
    if view.warp > 0.02 || view.reveal < 1.0 {
        return Vec::new();
    }
    let half = SIZE as f64 / 2.0;
    let right = cross(view.forward, view.up);
    let mut seen = Vec::new();
    for (p, planet) in system.planets.iter().enumerate() {
        let worlds = std::iter::once((None, planet.name.as_str(), planet.radius)).chain(
            planet
                .moons
                .iter()
                .enumerate()
                .map(|(m, moon)| (Some(m), moon.name.as_str(), moon.radius)),
        );
        for (moon, name, radius) in worlds {
            let rel = sub(system.centre(Stop { planet: p, moon }, t), view.position);
            let depth = length(rel);
            let ahead = dot(rel, view.forward);
            // Behind, or around and beside the camera: no disc to point at.
            if ahead <= radius || depth <= radius {
                continue;
            }
            let x = dot(rel, right) / ahead / view.half_fov;
            let y = dot(rel, view.up) / ahead / view.half_fov;
            let centre = [half + x * half, half - y * half];
            let radius_px = (radius / depth).asin().tan() / view.half_fov * half;
            if (centre[0] - half).hypot(centre[1] - half) >= half + radius_px {
                continue;
            }
            seen.push(Seen {
                name: name.to_string(),
                centre,
                radius: radius_px,
                depth,
                surface: depth - radius,
            });
        }
    }
    seen.sort_by(|a, b| a.surface.total_cmp(&b.surface));

    let mut placed: Vec<([f64; 4], [f64; 4], [f64; 2])> = Vec::new();
    let mut out = Vec::new();
    for world in &seen {
        if out.len() == MOST {
            break;
        }
        // Hidden behind a nearer world.
        if seen.iter().any(|o| {
            o.depth < world.depth
                && (o.centre[0] - world.centre[0]).hypot(o.centre[1] - world.centre[1]) < o.radius
        }) {
            continue;
        }
        // Filling the screen: the HUD names it already. (A disc smaller
        // than half the screen's radius cannot cover a quarter of it.)
        if world.radius > half * LARGEST.sqrt() && coverage(world) > LARGEST {
            continue;
        }
        let distance = distance(world.surface);
        for left in [false, true] {
            let (anchor, leader, text) = layout(world, left, &world.name, &distance);
            let clear = |b: [f64; 4]| {
                panels.iter().all(|p| !overlaps(b, *p, 0.0))
                    && placed
                        .iter()
                        .all(|(l, t, _)| !overlaps(b, *l, SPACING) && !overlaps(b, *t, SPACING))
            };
            let inside = |b: [f64; 4], margin: f64| {
                [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]]
                    .iter()
                    .all(|c| (c[0] - half).hypot(c[1] - half) < half - margin)
            };
            let apart = placed
                .iter()
                .all(|(_, _, a)| (a[0] - anchor[0]).hypot(a[1] - anchor[1]) > 2.0 * SPACING);
            if inside(text, 14.0) && inside(leader, 14.0) && clear(text) && clear(leader) && apart {
                placed.push((leader, text, anchor));
                out.push(BodyLabel {
                    at: [anchor[0] as f32, anchor[1] as f32],
                    left,
                    name: world.name.clone(),
                    distance,
                });
                break;
            }
        }
    }
    out
}

/// A label's leader start, the box round its leader and the box round its
/// text, in pixels.
fn layout(world: &Seen, left: bool, name: &str, distance: &str) -> ([f64; 2], [f64; 4], [f64; 4]) {
    let side = if left { -1.0 } else { 1.0 };
    let reach = (world.radius + GAP) * std::f64::consts::FRAC_1_SQRT_2;
    let anchor = [world.centre[0] + side * reach, world.centre[1] - reach];
    let rise = RISE as f64;
    let elbow = [anchor[0] + side * rise, anchor[1] - rise];
    let leader = [
        anchor[0].min(elbow[0]) - 5.0,
        elbow[1],
        anchor[0].max(elbow[0]) + 5.0,
        anchor[1] + 5.0,
    ];
    let width = text_width(name, distance);
    let text = if left {
        [elbow[0] - width, elbow[1] - 28.0, elbow[0], elbow[1] + 26.0]
    } else {
        [elbow[0], elbow[1] - 28.0, elbow[0] + width, elbow[1] + 26.0]
    };
    (anchor, leader, text)
}

/// About how wide a label's line runs: its wider text and a margin, at the
/// sizes `idle.slint` sets them (the name 17 px bold, spaced 2 px; the
/// distance 15 px).
pub fn text_width(name: &str, distance: &str) -> f64 {
    let name = name.chars().count() as f64 * 12.5;
    let distance = distance.chars().count() as f64 * 8.6;
    name.max(distance) + 20.0
}

/// From the camera to a world's surface, as the HUD gives distances: in
/// kilometres near by, in AU (as drawn) farther out.
fn distance(surface: f64) -> String {
    let km = surface.max(0.0) * KM;
    if km < 1.0e6 {
        format!("{} km", thousands(km))
    } else {
        format!("{:.2} AU", surface / AU)
    }
}

/// The share of the round screen a world's disc covers, sampled.
fn coverage(world: &Seen) -> f64 {
    const N: usize = 48;
    let half = SIZE as f64 / 2.0;
    let (mut screen, mut covered) = (0usize, 0usize);
    for j in 0..N {
        for i in 0..N {
            let x = (i as f64 + 0.5) / N as f64 * 2.0 * half;
            let y = (j as f64 + 0.5) / N as f64 * 2.0 * half;
            if (x - half).hypot(y - half) > half {
                continue;
            }
            screen += 1;
            if (x - world.centre[0]).hypot(y - world.centre[1]) < world.radius {
                covered += 1;
            }
        }
    }
    covered as f64 / screen.max(1) as f64
}

fn overlaps(a: [f64; 4], b: [f64; 4], margin: f64) -> bool {
    a[0] < b[2] + margin && b[0] < a[2] + margin && a[1] < b[3] + margin && b[1] < a[3] + margin
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn length(a: [f64; 3]) -> f64 {
    dot(a, a).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn world(centre: [f64; 2], radius: f64, surface: f64) -> Seen {
        Seen {
            name: "HIP 198317 d".into(),
            centre,
            radius,
            depth: surface + 1.0,
            surface,
        }
    }

    #[test]
    fn leader_rises_at_45_degrees_from_the_limb() {
        let w = world([400.0, 600.0], 100.0, 10.0);
        let (anchor, leader, text) = layout(&w, false, "HIP 198317 d", "0.02 AU");
        let reach = 110.0 * std::f64::consts::FRAC_1_SQRT_2;
        assert!((anchor[0] - (400.0 + reach)).abs() < 1e-9);
        assert!((anchor[1] - (600.0 - reach)).abs() < 1e-9);
        assert_eq!(leader[1], anchor[1] - RISE as f64);
        assert_eq!(text[0], anchor[0] + RISE as f64);
        let (flipped, _, text) = layout(&w, true, "HIP 198317 d", "0.02 AU");
        assert!(flipped[0] < 400.0 && text[2] < flipped[0]);
    }

    #[test]
    fn coverage_of_a_disc() {
        let half = SIZE as f64 / 2.0;
        // A disc half the screen's radius covers a quarter of it.
        let quarter = coverage(&world([half, half], half * 0.5, 1.0));
        assert!((quarter - 0.25).abs() < 0.02, "{quarter}");
        assert!(coverage(&world([half, half], half * 1.2, 1.0)) > 0.99);
    }

    /// Over whole tours: never more than `MOST`, never two crossing, never
    /// off the circle or under a panel, and the nearest world in view
    /// labelled whenever it can be.
    #[test]
    fn labels_over_whole_tours() {
        use super::super::system::System;
        use super::super::tour::Tour;
        let half = SIZE as f64 / 2.0;
        let panels = vec![[300.0, 66.0, 780.0, 142.0], [780.0, 415.0, 1030.0, 665.0]];
        let (mut frames, mut shown, mut many) = (0, 0, 0);
        for seed in [0u64, 5, 11] {
            let tour = Tour::new(System::generate(seed), System::generate(seed + 1));
            let mut t = 0.0;
            while t < tour.total {
                let view = tour.view(t);
                let found = labels(&tour.system, &view, t, &panels);
                frames += 1;
                shown += usize::from(!found.is_empty());
                many += usize::from(found.len() > 1);
                assert!(found.len() <= MOST);
                let boxes: Vec<_> = found
                    .iter()
                    .map(|l| {
                        let side = if l.left { -1.0 } else { 1.0 };
                        let elbow = [
                            l.at[0] as f64 + side * RISE as f64,
                            l.at[1] as f64 - RISE as f64,
                        ];
                        let w = text_width(&l.name, &l.distance);
                        let x0 = if l.left { elbow[0] - w } else { elbow[0] };
                        [x0, elbow[1] - 28.0, x0 + w, elbow[1] + 26.0]
                    })
                    .collect();
                for (i, b) in boxes.iter().enumerate() {
                    for c in [[b[0], b[1]], [b[2], b[1]], [b[0], b[3]], [b[2], b[3]]] {
                        assert!((c[0] - half).hypot(c[1] - half) < half, "{seed} {t}: {b:?}");
                    }
                    assert!(panels.iter().all(|p| !overlaps(*b, *p, 0.0)), "{seed} {t}");
                    for o in &boxes[i + 1..] {
                        assert!(!overlaps(*b, *o, 0.0), "{seed} {t}: {b:?} {o:?}");
                    }
                }
                t += 0.5;
            }
        }
        // Worlds are in view through much of a tour, and now and then
        // more than one has room.
        assert!(shown * 5 > frames, "{shown} of {frames}");
        assert!(many > 0, "never more than one label in {frames} frames");
    }

    #[test]
    fn distances_switch_to_au_far_out() {
        assert_eq!(distance(10.0), "63,710 km");
        assert_eq!(distance(AU * 0.5), "0.50 AU");
    }
}
