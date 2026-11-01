//! What each part and effect of the dive costs on the machine running it.
//!
//! `features` times the frame with and without every entry of
//! [`super::part`] (the passes in [`PASSES`]) and [`super::effect`] (every
//! model group and sprite kind, found from `MODEL_GROUPS` and
//! `SPRITE_GROUPS`, and every scenery element of `world::ELEMENTS`, so a new
//! group or element is measured with no change here), at the
//! places where it is in view, and writes `docs/feature-costs.md`'s table.
//! Run it on the Pi with `deploy/bench-pi.sh features TAG`; see that
//! document for how to read the table.
//!
//! A feature's cost at a place is the median over frames of the difference
//! between a frame with everything and a frame with it left out, the two
//! drawn back to back (in alternating order) so that drift cancels. A group
//! counts as in view at a place when a frame holds an instance or sprite of
//! it; a pass is measured everywhere, and counts as present in the seas
//! where it costs 0.05 ms or more.

use super::*;
use std::collections::BTreeMap;

/// The pass-level switches that remove something the viewer sees. Probes and
/// alternative shader paths (`QUARTER`, `CHEAP_*`, `NO_NOISE`, ...) are not
/// features and stay in `effect_cost`. A new pass gets its row here.
///
/// Not here: switches that change a shader switch (`part::SURFACE_FACETS`)
/// or rebuild the seabed (`part::COARSE_SEABED`): alternated frame by frame,
/// each frame would recompile the pipelines or rebuild every tile, so they
/// are priced by `hotspots` pairs instead (`docs/feature-costs.md`).
const PASSES: &[(&str, u64)] = &[
    ("whole 3D pass (all of the below and the clear)", part::SEA),
    ("terrain", part::TERRAIN),
    ("opaque models", part::MODELS),
    ("glass models", part::GLASS),
    ("fading models", part::FADING),
    ("fading models: ice-sheet coverage", part::FADING_ICE),
    ("fading models: per-instance fade", part::FADING_PROPS),
    ("far coral impostors", part::IMPOSTORS),
    ("sprites", part::SPRITES),
    ("point lights", part::LIGHTS),
    ("sun shafts", part::SUN_SHAFTS),
    ("searchlight beams", part::BEAMS),
    ("corner mask", part::MASK),
    ("cave tunnel", part::TUNNEL),
    ("open-water background", part::BACKGROUND),
    ("sediment: sand line on slopes, silt drifts", part::SEDIMENT),
    (
        "wildlife (V5): world::LIFE and the reef's shoals",
        part::LIFE,
    ),
    (
        "the abyss's glowing colonies on its walls (V10)",
        part::ABYSS_GLOW,
    ),
];

/// Below this a pass is not counted as present in a sea.
const PRESENT_MS: f64 = 0.05;

struct Row {
    feature: String,
    kind: &'static str,
    sea: String,
    visit: u64,
    place: f64,
    ms: f64,
    in_view: bool,
}

fn leg_start(n: f64) -> f64 {
    crate::voyage::LEG / crate::voyage::SPEED * (n + 0.35)
}

fn median(values: &mut [f64]) -> f64 {
    values.sort_by(f64::total_cmp);
    let m = values.len() / 2;
    if values.len().is_multiple_of(2) {
        (values[m - 1] + values[m]) / 2.0
    } else {
        values[m]
    }
}

fn list(name: &str, default: &[f64]) -> Vec<f64> {
    std::env::var(name).map_or_else(
        |_| default.to_vec(),
        |text| {
            text.split(',')
                .filter_map(|v| v.trim().parse().ok())
                .collect()
        },
    )
}

/// `DIVE3D_FEATURE_VISITS` (default `0,500000`), `DIVE3D_FEATURE_PLACES`
/// (`0.15,0.45,0.85`), `DIVE3D_FEATURE_FRAMES` (6) and
/// `DIVE3D_HOTSPOT_SCENES` choose what is sampled; `DIVE3D_FEATURE_DIR`
/// receives `features.csv` (every feature at every place) and `features.md`
/// (the summary table).
#[test]
#[ignore = "a benchmark; needs a working GPU adapter"]
fn features() {
    let gpu = crate::gpu::headless().expect("a GPU adapter");
    println!("adapter: {}, profiler: {}", gpu.adapter, gpu.profiling());
    let visits: Vec<u64> = list("DIVE3D_FEATURE_VISITS", &[0.0, 500_000.0])
        .into_iter()
        .map(|v| v as u64)
        .collect();
    let places = list("DIVE3D_FEATURE_PLACES", &[0.15, 0.45, 0.85]);
    let only: Vec<String> = std::env::var("DIVE3D_HOTSPOT_SCENES")
        .map(|t| t.split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let frames: usize = std::env::var("DIVE3D_FEATURE_FRAMES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6);
    let span = crate::voyage::LEG / crate::voyage::SPEED;

    let mut rows: Vec<Row> = Vec::new();
    let mut baselines: Vec<f64> = Vec::new();
    for &visit in &visits {
        let mut scene = Nautilus::new(&gpu).expect("a 3D dive");
        for i in 0..4 {
            scene
                .draw(&gpu, leg_start(0.0) + f64::from(i) * 0.1, visit, 0)
                .expect("a warm frame");
            gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
        }
        for n in 0..crate::voyage::BIOMES {
            let sea = places::biome_name(
                Biome::at((n as f64 + 0.5) * crate::voyage::LEG, seed_of(visit)).a,
            );
            if !only.is_empty() && !only.iter().any(|s| s == sea) {
                continue;
            }
            for &place in &places {
                let seconds = leg_start(n as f64) + place.clamp(0.0, 0.99) * span;
                scene.skip = 0;
                scene.skip_effects = 0;
                scene.skip_elements = 0;
                // Warm, then see what is in view over the sampled frames.
                let mut models = 0u64;
                let mut sprites = 0u64;
                let mut elements = 0u64;
                for i in 0..3 + frames {
                    scene
                        .draw(
                            &gpu,
                            seconds + i as f64 * 0.02,
                            visit,
                            i % crate::visuals::SLOTS,
                        )
                        .expect("a frame");
                    gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                    if i >= 3 {
                        for lists in [&scene.frame.solid, &scene.frame.glass] {
                            for (model, list) in lists.iter().enumerate() {
                                if !list.is_empty() {
                                    models |= model_bit(model);
                                }
                            }
                        }
                        for sprite in &scene.frame.sprites {
                            sprites |= sprite_bit(sprite.tint[3]);
                        }
                        for lists in [&scene.frame.solid, &scene.frame.glass] {
                            for i in lists.iter().flatten() {
                                elements |= 1 << world::element_of(i);
                            }
                        }
                    }
                }
                // (name, kind, part mask, effect mask, element mask, in view)
                let mut variants: Vec<(String, &'static str, u64, u64, u64, bool)> = Vec::new();
                for &(name, bit) in PASSES {
                    variants.push((name.to_string(), "pass", bit, 0, 0, true));
                }
                for &(name, bit, _, _) in effect::MODEL_GROUPS {
                    variants.push((name.to_string(), "model", 0, bit, 0, models & bit != 0));
                }
                for &(name, bit, _) in effect::SPRITE_GROUPS {
                    variants.push((name.to_string(), "sprite", 0, bit, 0, sprites & bit != 0));
                }
                for &(e, name) in world::ELEMENTS {
                    let bit = 1u64 << e;
                    variants.push((name.to_string(), "element", 0, 0, bit, elements & bit != 0));
                }
                // Only what is in view is timed; a group nothing of which is
                // in view is recorded as not in view, at no cost.
                let mut deltas: Vec<Vec<f64>> = vec![Vec::new(); variants.len()];
                let time =
                    |scene: &mut Nautilus, at: f64, slot: usize, (p, e, m): (u64, u64, u64)| {
                        scene.skip = p;
                        scene.skip_effects = e;
                        scene.skip_elements = m;
                        let begun = std::time::Instant::now();
                        scene.draw(&gpu, at, visit, slot).expect("a frame");
                        gpu.device.poll(wgpu::PollType::wait_indefinitely()).ok();
                        begun.elapsed().as_secs_f64() * 1e3
                    };
                for i in 0..frames {
                    let at = seconds + i as f64 * 0.02;
                    let slot = i % crate::visuals::SLOTS;
                    for (v, (_, _, p, e, m, in_view)) in variants.iter().enumerate() {
                        if !in_view {
                            continue;
                        }
                        let none = (0, 0, 0);
                        let (base, taken) = if (i + v) % 2 == 0 {
                            let b = time(&mut scene, at, slot, none);
                            (b, time(&mut scene, at, slot, (*p, *e, *m)))
                        } else {
                            let t = time(&mut scene, at, slot, (*p, *e, *m));
                            (time(&mut scene, at, slot, none), t)
                        };
                        baselines.push(base);
                        deltas[v].push(base - taken);
                    }
                }
                for (v, (name, kind, _, _, _, in_view)) in variants.iter().enumerate() {
                    let ms = if *in_view {
                        median(&mut deltas[v])
                    } else {
                        0.0
                    };
                    rows.push(Row {
                        feature: name.clone(),
                        kind,
                        sea: sea.to_string(),
                        visit,
                        place,
                        ms,
                        in_view: *in_view,
                    });
                }
                println!("{sea} visit {visit} place {place}: measured");
            }
        }
    }
    summarise(&rows, &mut baselines);
}

/// What one feature cost where it was present.
struct Acc {
    kind: &'static str,
    seas: BTreeMap<String, Vec<f64>>,
    costs: Vec<f64>,
    worst: (f64, String),
}

fn summarise(rows: &[Row], baselines: &mut [f64]) {
    let mut features: Vec<(String, Acc)> = Vec::new();
    for r in rows {
        let at = features
            .iter()
            .position(|(n, _)| *n == r.feature)
            .unwrap_or_else(|| {
                features.push((
                    r.feature.clone(),
                    Acc {
                        kind: r.kind,
                        seas: BTreeMap::new(),
                        costs: Vec::new(),
                        worst: (f64::MIN, String::new()),
                    },
                ));
                features.len() - 1
            });
        let acc = &mut features[at].1;
        if !r.in_view || (r.kind == "pass" && r.ms < PRESENT_MS) {
            continue;
        }
        acc.seas.entry(r.sea.clone()).or_default().push(r.ms);
        acc.costs.push(r.ms);
        if r.ms > acc.worst.0 {
            acc.worst = (r.ms, format!("{} {} @{:.2}", r.sea, r.visit, r.place));
        }
    }
    let medians: Vec<f64> = features
        .iter()
        .map(|(_, a)| {
            if a.costs.is_empty() {
                f64::MIN
            } else {
                median(&mut a.costs.clone())
            }
        })
        .collect();
    let mut order: Vec<usize> = (0..features.len()).collect();
    order.sort_by(|&a, &b| medians[b].total_cmp(&medians[a]));
    let mut table = String::from(
        "| feature | kind | seas where it appears (median ms there) | median ms | worst ms (sea, visit, place) |\n\
         |---|---|---|---|---|\n",
    );
    for i in order {
        let (name, acc) = &mut features[i];
        if acc.costs.is_empty() {
            let why = if acc.kind == "pass" {
                "under 0.05 ms in every sea"
            } else {
                "not in view"
            };
            table += &format!("| {name} | {} | {why} | | |\n", acc.kind);
            continue;
        }
        let seas: Vec<String> = acc
            .seas
            .iter_mut()
            .map(|(sea, v)| format!("{sea} {:.2}", median(v)))
            .collect();
        table += &format!(
            "| {name} | {} | {} | {:.2} | {:.2} ({}) |\n",
            acc.kind,
            seas.join(", "),
            median(&mut acc.costs),
            acc.worst.0,
            acc.worst.1
        );
    }
    let head = format!(
        "frame with everything: median {:.2} ms over {} paired frames\n\n",
        median(baselines),
        baselines.len()
    );
    println!("{head}{table}");
    if let Ok(dir) = std::env::var("DIVE3D_FEATURE_DIR") {
        let dir = std::path::Path::new(&dir);
        let mut csv = String::from("feature,kind,sea,visit,place,in_view,ms\n");
        for r in rows {
            csv += &format!(
                "\"{}\",{},{},{},{:.2},{},{:.3}\n",
                r.feature, r.kind, r.sea, r.visit, r.place, r.in_view, r.ms
            );
        }
        std::fs::write(dir.join("features.csv"), csv).expect("a features CSV");
        std::fs::write(dir.join("features.md"), head + &table).expect("a features table");
    }
}
