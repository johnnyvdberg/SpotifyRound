# What each part and effect costs

The frame-time price of every pass, model group and sprite kind of the dive, measured on the Pi 5
(OpenGL ES, Mesa 26.2.2, V3D 7.1) with `dive3d::feature_costs::features`. Every later task that
adds a part or effect adds its row here (rule 8 of `docs/agent-tasks.md`: a pass is a row of
`PASSES` in `src/dive3d/feature_costs.rs` and a bit of `dive3d::part`; a model group or sprite
kind is an entry of `dive3d::effect::MODEL_GROUPS`/`SPRITE_GROUPS`, which this benchmark picks up
by itself).

## Regenerate

```sh
find src -type f -exec touch {} + && deploy/build-for-pi.sh --tests
PI=<user>@<pi> deploy/bench-pi.sh features TAG     # about four minutes; freezes the kiosk meanwhile
# target/bench/TAG-features.md is the table below, TAG-features.csv has every feature at every place
```

`DIVE3D_HOTSPOT_SCENES=coral,city`, `DIVE3D_FEATURE_VISITS=0,500000`,
`DIVE3D_FEATURE_PLACES=0.15,0.45,0.85` and `DIVE3D_FEATURE_FRAMES=6` narrow it (pass them as
`NAME=VALUE` after the tag). The same test runs on the Mac with `DIVE3D_FEATURE_DIR=<dir> cargo
test --release dive3d::feature_costs -- --ignored --nocapture`, but a desktop GPU prices things
very differently; only the Pi's numbers go in this document.

## How to read it

- A feature's cost at one place is the median over 6 frames of (frame with everything) minus (frame
  with the feature left out), the two drawn back to back in alternating order so drift cancels. It
  is what the frame would save by dropping the feature there, with everything else unchanged:
  features overlap (early-Z lets a nearer layer hide the pixels of a farther one), so costs do not
  add up to the frame.
- A model group or sprite kind is measured only at places where a frame holds an instance of it;
  "seas where it appears" lists those seas with the median cost there. A pass is measured
  everywhere and listed in the seas where it costs 0.05 ms or more.
- **median** is over every place where the feature is present, **worst** the largest single place.
  Places are the 14 seas at 0.15/0.45/0.85 of their leg in two visits (0 and 500000), 84 places.
- Noise is about +-0.05 ms: negative and 0.00-0.05 rows are zero. The whole 3D pass row is the sum
  of everything beneath it; the rest of the frame (about 1.2 ms) is the CPU's placement, uploads
  and the readback the benchmark waits for.
- "Cheap" and probe switches (`QUARTER`, `CHEAP_*`, `NO_NOISE` ...) are not features and stay in
  `effect_cost`.

## Latest table

Measured 2026-11-01 on the Pi, branch `tooling`, base `main` 6c12ca6, tag `tooling-features`.

frame with everything: median 8.50 ms over 13872 paired frames

| feature | kind | seas where it appears (median ms there) | median ms | worst ms (sea, visit, place) |
|---|---|---|---|---|
| whole 3D pass (all of the below and the clear) | pass | abyss 6.68, arctic 7.31, atlantis 7.56, bloom 6.01, brine 7.00, cave 8.28, city 7.84, coral 7.98, forest 7.86, sargasso 7.15, shallows 7.14, trench 5.85, vents 8.32, wrecks 7.31 | 7.30 | 10.24 (city 0 @0.45) |
| opaque models | pass | abyss 1.36, arctic 1.51, atlantis 3.05, bloom 0.68, brine 1.82, cave 3.78, city 3.18, coral 3.17, forest 2.98, sargasso 2.39, shallows 1.94, trench 1.07, vents 1.47, wrecks 1.25 | 1.77 | 5.19 (cave 0 @0.15) |
| sargassum | model | cave 1.85, forest 1.41, sargasso 1.74 | 1.74 | 3.05 (sargasso 500000 @0.45) |
| coral | model | brine 1.21, coral 1.48, shallows 0.89 | 1.48 | 1.98 (coral 500000 @0.15) |
| ruins | model | atlantis 1.55, city 0.07, sargasso 0.52, trench 0.97, vents 0.85 | 1.42 | 1.99 (trench 500000 @0.85) |
| open-water background | pass | abyss 1.04, arctic 1.46, atlantis 1.65, bloom 1.01, brine 1.47, cave 0.13, city 0.51, coral 1.20, forest 0.87, sargasso 1.12, shallows 1.65, trench 1.53, vents 1.40, wrecks 1.73 | 1.26 | 2.04 (coral 500000 @0.45) |
| trees | model | city 1.80, coral 1.49, forest 1.07, shallows 0.16, wrecks 0.38 | 1.22 | 2.00 (city 500000 @0.45) |
| terrain | pass | abyss 0.91, arctic 1.25, atlantis 1.01, bloom 0.97, brine 1.07, cave 1.05, city 1.05, coral 1.21, forest 1.10, sargasso 1.11, shallows 1.42, trench 0.91, vents 1.29, wrecks 0.96 | 1.11 | 1.96 (cave 0 @0.15) |
| far coral impostors | pass | brine 0.95, coral 0.95, shallows 0.74 | 0.95 | 1.37 (shallows 0 @0.85) |
| fading models: ice-sheet coverage | pass | abyss 0.79, arctic 0.83, wrecks 0.80 | 0.82 | 0.94 (arctic 0 @0.45) |
| fading models | pass | abyss 0.80, arctic 0.83, wrecks 0.81 | 0.81 | 0.95 (arctic 0 @0.45) |
| planes | model | abyss 0.80, arctic 0.82, wrecks 0.79 | 0.80 | 0.94 (arctic 0 @0.45) |
| jellyfish | model | abyss 0.76, arctic 0.64, bloom 1.59, cave 0.64, forest 0.40, sargasso 0.12, trench 0.68, wrecks 0.94 | 0.77 | 3.76 (arctic 500000 @0.85) |
| glass models | pass | abyss 0.76, arctic 0.63, atlantis 0.12, bloom 1.57, brine 0.18, cave 0.10, city 0.13, forest 0.40, sargasso 0.14, shallows 0.24, trench 0.66, wrecks 0.93 | 0.69 | 3.82 (arctic 500000 @0.85) |
| towers | model | atlantis 0.43, bloom 0.03, brine 0.43, cave 0.13, city 1.38, coral 0.78, forest 0.53, sargasso 0.07, trench 0.10, vents 0.13, wrecks 0.29 | 0.40 | 2.67 (city 0 @0.45) |
| sprites | pass | abyss 0.61, arctic 0.30, atlantis 0.24, bloom 0.40, brine 0.26, cave 0.34, city 0.31, coral 0.24, forest 0.29, sargasso 0.32, shallows 0.27, trench 0.26, vents 1.58, wrecks 0.29 | 0.30 | 3.52 (vents 0 @0.15) |
| point lights | pass | abyss 0.33, arctic 0.18, atlantis 0.19, bloom 0.30, brine 0.11, cave 0.26, city 0.23, coral 0.27, forest 0.20, sargasso 0.23, shallows 0.23, trench 0.11, vents 0.33, wrecks 0.25 | 0.26 | 0.73 (brine 0 @0.85) |
| searchlight beams | pass | atlantis 0.29, brine 0.67, city 0.14, coral 0.30, vents 0.07, wrecks 0.17 | 0.23 | 0.67 (brine 500000 @0.15) |
| motes | sprite | abyss 0.19, arctic 0.22, atlantis 0.21, bloom 0.17, brine 0.18, cave 0.19, city 0.29, coral 0.24, forest 0.31, sargasso 0.32, shallows 0.26, trench 0.24, vents 0.16, wrecks 0.15 | 0.22 | 0.44 (sargasso 500000 @0.45) |
| sun shafts | pass | abyss 0.30, arctic 0.27, atlantis 0.14, brine 0.18, cave 0.36, city 0.15, coral 0.12, forest 0.25, sargasso 0.36, shallows 0.27, trench 0.09, vents 0.22, wrecks 0.20 | 0.21 | 0.63 (forest 0 @0.85) |
| cave hexes | model | atlantis 0.13, brine 0.22, cave 0.13, city 0.12, shallows 0.24 | 0.20 | 1.12 (brine 0 @0.85) |
| cave tunnel | pass | bloom 0.06, brine 0.57, cave 1.98, city 0.08, sargasso 0.07, shallows 0.19, vents 0.09 | 0.19 | 4.69 (cave 0 @0.15) |
| kelp | model | arctic 0.46, atlantis 0.05, city 0.12, coral 0.08, forest 0.19, sargasso 0.07, shallows 0.43, trench 0.06, vents 0.96, wrecks 0.09 | 0.15 | 1.00 (vents 500000 @0.85) |
| sponges | model | abyss 0.19, arctic 0.17, bloom 0.06, cave 0.11, sargasso 0.12, trench 0.10, wrecks 0.17 | 0.14 | 0.24 (arctic 0 @0.15) |
| rock 1 | model | abyss 0.15, arctic 0.10, atlantis 0.11, bloom 0.13, brine 0.08, cave 0.08, city 0.13, coral 0.05, forest 0.14, sargasso 0.16, shallows 0.14, trench 0.16, vents 0.14, wrecks 0.11 | 0.13 | 0.23 (abyss 500000 @0.45) |
| rock 0 | model | abyss 0.12, arctic 0.09, atlantis 0.13, bloom 0.14, brine 0.10, cave 0.08, city 0.11, coral 0.03, forest 0.12, sargasso 0.14, shallows 0.10, trench 0.10, vents 0.14, wrecks 0.13 | 0.11 | 0.20 (city 500000 @0.45) |
| city blocks | model | arctic 0.06, atlantis 0.06, brine 0.12, cave 0.03, city 1.05, coral 0.62, forest 0.76, trench 0.04, wrecks 0.04 | 0.11 | 2.49 (city 500000 @0.15) |
| rock 2 | model | abyss 0.11, arctic 0.10, atlantis 0.13, bloom 0.12, brine 0.07, cave 0.05, city 0.09, coral 0.03, forest 0.13, sargasso 0.13, shallows 0.10, trench 0.11, vents 0.12, wrecks 0.10 | 0.10 | 0.23 (atlantis 0 @0.45) |
| fish | model | abyss 0.02, arctic 0.20, atlantis 0.18, bloom 0.05, brine 0.12, cave 0.06, city 0.21, coral 0.12, forest 0.07, sargasso 0.25, shallows 0.53, trench 0.04, vents 0.16, wrecks 0.04 | 0.10 | 1.14 (sargasso 500000 @0.15) |
| divers | model | atlantis 0.13, cave 0.00, city 0.08, coral 0.10, forest 0.09 | 0.08 | 0.13 (atlantis 500000 @0.45) |
| shells | model | brine 0.01, city 0.26, coral 0.07, forest 0.24, shallows 0.03 | 0.08 | 0.30 (city 500000 @0.45) |
| cylinders | model | abyss 0.19, arctic 0.02, atlantis 0.41, bloom 0.06, brine 0.02, cave 0.06, city 0.02, coral 0.02, forest 0.03, sargasso 0.08, shallows 0.03, trench 0.09, vents 0.34, wrecks 0.04 | 0.06 | 0.52 (vents 0 @0.85) |
| tubes | model | abyss 0.08, arctic 0.09, bloom 0.06, cave 0.03 | 0.06 | 0.11 (bloom 0 @0.15) |
| wreck | model | city 0.04, forest 0.06, trench -0.00, wrecks 0.09 | 0.06 | 0.12 (trench 0 @0.85) |
| cones | model | atlantis 0.03, brine 0.04, cave 0.02, city 0.03, sargasso 0.21, shallows -0.00, trench 0.09 | 0.04 | 0.23 (sargasso 500000 @0.85) |
| fans | model | arctic 0.04, bloom 0.03, brine 0.02, cave 0.02, city 0.10, coral 0.04, forest 0.09, sargasso 0.03, shallows 0.10, trench 0.02, vents 0.12 | 0.04 | 0.13 (vents 500000 @0.45) |
| vents | model | abyss 0.34, arctic 0.00, atlantis 0.03, bloom 0.08, brine 0.03, cave 0.05, city 0.03, sargasso 0.04, shallows 0.03, trench 0.03, vents 0.43, wrecks 0.47 | 0.04 | 0.64 (abyss 500000 @0.45) |
| city glass | model | city 0.05, coral 0.03 | 0.04 | 0.05 (city 0 @0.45) |
| penguins | model | arctic 0.04 | 0.04 | 0.04 (arctic 500000 @0.45) |
| narwhals | model | arctic 0.03 | 0.03 | 0.03 (arctic 0 @0.15) |
| spheres | model | abyss 0.03, arctic 0.02, atlantis 0.03, bloom 0.01, brine 0.02, cave 0.00, city 0.05, coral 0.02, forest 0.03, sargasso 0.04, shallows 0.03, trench 0.03, vents 0.08, wrecks 0.03 | 0.03 | 0.14 (vents 500000 @0.15) |
| stars | model | abyss 0.05, arctic 0.06, atlantis 0.03, brine 0.02, city 0.02, coral 0.02, shallows 0.00, wrecks 0.06 | 0.03 | 0.07 (arctic 0 @0.15) |
| boxes | model | arctic 0.03, atlantis 0.34, cave 0.18, city 0.02, coral -0.00, forest -0.01, sargasso 0.10, trench 0.02, vents 0.22, wrecks 0.01 | 0.03 | 0.43 (atlantis 0 @0.45) |
| submarine | model | abyss 0.03, arctic -0.02, brine 0.01, cave 0.05, city 0.06, coral 0.00, sargasso 0.03, shallows 0.03, vents 0.02, wrecks 0.04 | 0.03 | 0.07 (abyss 0 @0.45) |
| city works | model | atlantis 0.05, city 0.04, coral -0.06 | 0.02 | 0.30 (city 0 @0.15) |
| turtles | model | city 0.04, coral 0.02, shallows 0.02 | 0.02 | 0.04 (city 500000 @0.45) |
| whales | model | arctic 0.01, vents 0.02 | 0.02 | 0.02 (arctic 0 @0.85) |
| mantas | model | coral 0.04, shallows 0.02, trench 0.00, vents 0.02, wrecks 0.03 | 0.02 | 0.04 (coral 0 @0.45) |
| glows | sprite | abyss 0.49, arctic -0.01, atlantis 0.02, bloom 0.15, brine 0.01, cave 0.03, city 0.02, coral 0.07, forest -0.01, sargasso -0.03, shallows 0.01, trench -0.00, vents 2.60, wrecks -0.00 | 0.02 | 3.27 (vents 0 @0.15) |
| sharks | model | forest 0.02, trench 0.04, wrecks 0.00 | 0.02 | 0.04 (trench 0 @0.45) |
| capsules | model | cave 0.00 | 0.00 | 0.01 (cave 500000 @0.15) |
| squid | model | abyss 0.01, bloom -0.01 | -0.00 | 0.01 (bloom 0 @0.85) |
| embers | sprite | abyss 0.01, arctic 0.02, atlantis -0.01, brine -0.02, cave -0.02, city -0.02, coral -0.03, shallows -0.02, vents -0.02, wrecks -0.00 | -0.02 | 0.05 (abyss 0 @0.85) |
| bubbles | sprite | abyss -0.04, arctic -0.04, atlantis -0.02, cave -0.02, city -0.03, coral -0.03, forest -0.02, vents -0.01, wrecks -0.02 | -0.02 | -0.01 (vents 0 @0.85) |
| ice keels | model | abyss -0.05, arctic -0.04, forest 0.01, wrecks -0.02 | -0.02 | 0.01 (forest 500000 @0.85) |
| fading models: per-instance fade | pass | under 0.05 ms in every sea | | |
| corner mask | pass | under 0.05 ms in every sea | | |
| dolphins | model | not in view | | |

## Reading the table

- The dive is dominated by the sea pass's two largest parts: opaque models (1.8 ms median, 5.2 ms
  worst, in the cave at 0.15 of its leg) and the full-screen open
  water (1.3 ms median). The terrain costs 1.1 ms.
- The expensive single groups are **sargassum** (1.7 ms median where present, 3.1 worst),
  **coral** (1.5, 2.0), **ruins** (1.4), **trees** (1.2) and the **far coral impostors** (0.95);
  **jellyfish** and glass peak at 3.8 ms in the arctic and bloom views.
- In the vents the glow sprites cost 2.6 ms (3.3 ms worst) and all sprites 1.6 ms (3.5 worst):
  the P2 brief (one column per plume).
- The cave tunnel costs 2.0 ms in the cave and 4.7 ms at its worst place.
- Under 0.05 ms everywhere: the corner mask, per-instance fades, dolphins (not met in these
  places), squid, ice keels, bubbles, embers.

## After perf-b (2026-11-01)

The table above was measured before `perf-b` (city split by part, glass by material, see
`docs/handoff/perf-b.md`) and is not regenerated here. What moved, from the same 168-place
`hotspots` runs (not the leave-one-out method of this table): the bloom's median place
7.45 to 7.13 ms, the worst glass places 1.0 to 1.4 ms faster, the city's worst place 10.89 to
9.75 ms. Nothing new was added to `dive3d::part` or `dive3d::effect`, so no row is new; regenerate
the table (`bench-pi.sh features`) when the next task needs it.
