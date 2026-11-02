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

Measured 2026-11-01 on the Pi, branch `water` (V1/V2, base `main` 9e10613), tag `water-features`.
Every sea now stops drawing at its sight (`world::SIGHT`), which is why most rows are smaller
than the `tooling` table's (8.50 ms frame median then).

frame with everything: median 6.94 ms over 13938 paired frames

| feature | kind | seas where it appears (median ms there) | median ms | worst ms (sea, visit, place) |
|---|---|---|---|---|
| whole 3D pass (all of the below and the clear) | pass | abyss 5.36, arctic 6.26, atlantis 6.58, bloom 4.83, brine 5.57, cave 5.83, city 6.83, coral 7.61, forest 6.57, sargasso 6.11, shallows 6.53, trench 5.20, vents 5.18, wrecks 5.00 | 6.06 | 8.95 (coral 0 @0.15) |
| coral | model | brine 1.23, coral 1.40, shallows 0.81 | 1.40 | 2.15 (coral 500000 @0.15) |
| opaque models | pass | abyss 1.09, arctic 1.40, atlantis 2.54, bloom 0.36, brine 1.15, cave 1.64, city 2.45, coral 2.56, forest 2.04, sargasso 1.84, shallows 1.66, trench 1.05, vents 0.89, wrecks 0.66 | 1.36 | 3.65 (city 500000 @0.15) |
| open-water background | pass | abyss 1.19, arctic 1.88, atlantis 1.58, bloom 1.22, brine 1.25, cave 0.14, city 0.49, coral 1.70, forest 0.84, sargasso 1.10, shallows 2.30, trench 1.67, vents 1.66, wrecks 1.68 | 1.34 | 2.76 (coral 500000 @0.45) |
| sargassum | model | cave 1.49, forest 1.02, sargasso 1.32 | 1.32 | 2.23 (sargasso 500000 @0.45) |
| ruins | model | atlantis 1.33, sargasso 0.18, trench 1.13, vents 0.64 | 1.19 | 1.65 (trench 500000 @0.85) |
| trees | model | city 1.32, coral 1.16, forest 0.98, shallows 0.11, wrecks 0.10 | 1.06 | 1.46 (city 500000 @0.45) |
| far coral impostors | pass | brine 0.89, coral 0.93, shallows 0.72 | 0.93 | 1.33 (shallows 0 @0.85) |
| fading models: ice-sheet coverage | pass | abyss 0.81, arctic 0.82, wrecks 0.80 | 0.81 | 0.93 (arctic 0 @0.45) |
| fading models | pass | abyss 0.80, arctic 0.83, wrecks 0.79 | 0.81 | 0.93 (arctic 0 @0.45) |
| planes | model | abyss 0.81, arctic 0.81, wrecks 0.79 | 0.80 | 0.92 (arctic 0 @0.45) |
| terrain | pass | abyss 0.74, arctic 0.61, atlantis 0.88, bloom 0.58, brine 0.80, cave 0.91, city 0.95, coral 0.94, forest 0.71, sargasso 0.80, shallows 0.84, trench 0.72, vents 0.60, wrecks 0.56 | 0.70 | 2.07 (shallows 500000 @0.85) |
| jellyfish | model | abyss 0.48, arctic 0.43, bloom 1.09, cave 0.48, forest 0.25, sargasso 0.09, trench 0.43, wrecks 0.54 | 0.50 | 2.22 (arctic 500000 @0.85) |
| city blocks | model | atlantis 0.43, city 0.76, coral 0.48, forest 0.44 | 0.48 | 2.39 (city 500000 @0.15) |
| glass models | pass | abyss 0.48, arctic 0.43, atlantis 0.07, bloom 1.07, brine 0.09, cave 0.28, city 0.08, forest 0.25, sargasso 0.09, shallows 0.17, trench 0.40, wrecks 0.53 | 0.46 | 2.25 (arctic 500000 @0.85) |
| towers | model | atlantis 0.23, brine 0.19, city 0.96, coral 0.62, forest 0.19, vents 0.09 | 0.40 | 2.02 (city 0 @0.45) |
| sprites | pass | abyss 0.28, arctic 0.27, atlantis 0.20, bloom 0.24, brine 0.22, cave 0.28, city 0.34, coral 0.24, forest 0.27, sargasso 0.28, shallows 0.24, trench 0.23, vents 0.24, wrecks 0.23 | 0.25 | 0.48 (coral 0 @0.85) |
| point lights | pass | abyss 0.24, atlantis 0.19, bloom 0.20, brine 0.20, cave 0.28, city 0.22, coral 0.27, forest 0.17, sargasso 0.14, shallows 0.20, vents 0.18, wrecks 0.15 | 0.21 | 0.43 (brine 0 @0.85) |
| searchlight beams | pass | atlantis 0.27, brine 0.67, city 0.10, coral 0.25, vents 0.07, wrecks 0.16 | 0.20 | 0.67 (brine 500000 @0.15) |
| sun shafts | pass | abyss 0.30, arctic 0.28, atlantis 0.12, brine 0.16, cave 0.35, city 0.13, coral 0.13, forest 0.21, sargasso 0.34, shallows 0.28, trench 0.13, vents 0.20, wrecks 0.20 | 0.20 | 0.58 (forest 0 @0.85) |
| motes | sprite | abyss 0.19, arctic 0.21, atlantis 0.21, bloom 0.13, brine 0.17, cave 0.16, city 0.28, coral 0.22, forest 0.28, sargasso 0.31, shallows 0.25, trench 0.25, vents 0.16, wrecks 0.16 | 0.20 | 0.40 (city 500000 @0.85) |
| cave tunnel | pass | bloom 0.07, brine 0.44, cave 1.03, city 0.06, sargasso 0.07, shallows 0.19, vents 0.09 | 0.18 | 1.64 (shallows 500000 @0.85) |
| kelp | model | arctic 211.43, atlantis 0.07, city 0.10, coral 0.19, forest 0.17, sargasso 0.10, shallows 0.50, trench 0.08, vents 0.45 | 0.15 | 426.72 (shallows 500000 @0.45) |
| sponges | model | abyss 0.18, arctic 0.14, bloom 0.05, cave 0.08, sargasso 0.13, trench 0.09, wrecks 0.16 | 0.11 | 0.23 (arctic 0 @0.15) |
| wreck | model | forest 0.09, trench 0.06, wrecks 0.11 | 0.10 | 0.19 (trench 0 @0.85) |
| cave hexes | model | atlantis 0.08, brine 0.11, cave 0.02, city 0.09, shallows 0.18 | 0.10 | 0.56 (brine 0 @0.85) |
| divers | model | atlantis 0.10, cave 0.00, city 0.09, coral 0.09, forest 0.09 | 0.09 | 0.10 (atlantis 500000 @0.45) |
| rock 1 | model | abyss 0.07, arctic 0.08, atlantis 0.09, bloom 0.06, brine 0.05, cave 0.06, city 0.07, coral 0.04, forest 0.07, sargasso 0.10, shallows 0.10, trench 0.13, vents 0.07, wrecks 0.07 | 0.07 | 0.23 (sargasso 0 @0.85) |
| shells | model | brine 0.01, city 0.23, coral 0.07, forest 0.19, shallows 0.03 | 0.07 | 0.24 (city 500000 @0.45) |
| vents | model | abyss 0.25, bloom 0.04, brine 0.01, cave 0.04, sargasso 0.00, shallows 0.04, vents 0.34, wrecks 0.25 | 0.06 | 0.46 (vents 500000 @0.15) |
| rock 2 | model | abyss 0.06, arctic 0.07, atlantis 0.11, bloom 0.04, brine 0.03, cave 0.04, city 0.05, coral 0.03, forest 0.08, sargasso 0.07, shallows 0.08, trench 0.10, vents 0.04, wrecks 0.05 | 0.06 | 0.17 (trench 500000 @0.15) |
| cylinders | model | abyss 0.09, atlantis 0.31, bloom 0.02, brine 0.03, cave 0.05, city 0.01, coral 0.02, forest 0.04, sargasso 0.08, shallows 0.04, trench 0.08, vents 0.17, wrecks 0.08 | 0.05 | 0.46 (vents 0 @0.85) |
| tubes | model | abyss 0.07, arctic 0.08, bloom 0.05, cave 0.03 | 0.05 | 0.08 (bloom 0 @0.15) |
| fans | model | arctic 0.02, bloom 0.03, brine 0.03, cave 0.03, city 0.09, coral 0.05, forest 0.09, sargasso 0.03, shallows 0.11, trench 0.02, vents 0.11 | 0.05 | 0.12 (shallows 0 @0.45) |
| penguins | model | arctic 0.05 | 0.05 | 0.05 (arctic 500000 @0.45) |
| cones | model | atlantis 0.03, brine 0.05, cave -0.01, city 0.02, sargasso 0.15, shallows -0.00, trench 0.07 | 0.04 | 0.20 (sargasso 0 @0.85) |
| narwhals | model | arctic 0.04 | 0.04 | 0.04 (arctic 500000 @0.15) |
| rock 0 | model | abyss -421.74, arctic 0.05, atlantis 0.08, bloom -423.16, brine 0.01, cave -210.68, city 0.04, coral 0.02, forest 0.06, sargasso 0.08, shallows 0.07, trench 0.04, vents 0.06, wrecks -212.92 | 0.04 | 0.16 (city 500000 @0.45) |
| city glass | model | city 0.02, coral 0.03 | 0.03 | 0.04 (coral 500000 @0.85) |
| submarine | model | abyss 0.01, arctic 0.02, brine 0.03, cave 0.07, city 0.04, coral -0.03, sargasso 0.03, shallows 0.04, vents 0.03, wrecks 0.03 | 0.03 | 0.07 (cave 500000 @0.85) |
| spheres | model | abyss 0.02, arctic 0.02, atlantis 0.01, bloom 0.02, brine 0.04, cave 0.03, city 0.04, coral 0.03, forest 0.02, sargasso 0.04, shallows 0.02, trench 0.03, vents 0.05, wrecks 0.03 | 0.03 | 0.11 (abyss 500000 @0.45) |
| stars | model | abyss 0.06, arctic 0.07, atlantis 0.02, brine 0.02, city 0.02, coral 0.01, shallows 0.00, wrecks 0.05 | 0.03 | 0.08 (abyss 0 @0.45) |
| sharks | model | forest 0.01, trench 0.04, wrecks 0.02 | 0.02 | 0.04 (trench 0 @0.45) |
| city works | model | atlantis 0.05, city 0.02, coral -0.04 | 0.02 | 0.09 (city 0 @0.15) |
| boxes | model | arctic 0.02, atlantis 0.29, cave 0.02, city 0.00, coral 0.01, forest 0.01, sargasso 0.07, trench 0.06, vents 0.15, wrecks 0.03 | 0.02 | 0.30 (atlantis 0 @0.15) |
| turtles | model | city 0.00, coral 0.02, shallows 0.01 | 0.01 | 0.03 (coral 0 @0.15) |
| whales | model | arctic 0.00, vents 0.02 | 0.00 | 0.02 (vents 500000 @0.45) |
| mantas | model | coral 0.03, shallows 0.00, trench 0.02, vents -0.01, wrecks 0.00 | 0.00 | 0.03 (coral 0 @0.45) |
| squid | model | abyss 0.01, bloom 0.00 | 0.00 | 0.01 (abyss 0 @0.15) |
| embers | sprite | abyss 0.01, arctic 0.02, atlantis -0.02, brine -0.03, cave -0.02, city -0.03, coral -0.02, shallows -0.03, vents 0.00, wrecks 0.00 | -0.01 | 0.03 (abyss 0 @0.85) |
| glows | sprite | abyss 0.01, arctic 0.04, atlantis -0.03, bloom 0.03, brine 0.00, cave 0.03, city -0.00, coral 0.07, forest -0.04, sargasso -0.05, shallows 0.03, trench -0.04, vents -0.00, wrecks -0.03 | -0.01 | 0.20 (coral 0 @0.85) |
| capsules | model | cave -0.01 | -0.01 | -0.01 (cave 500000 @0.15) |
| bubbles | sprite | abyss -0.05, arctic -0.05, atlantis -0.03, cave -0.05, city -0.04, coral -0.02, forest -0.05, vents -0.02, wrecks -0.03 | -0.04 | -0.00 (coral 500000 @0.15) |
| ice keels | model | abyss -0.05, arctic -0.06, forest 0.02, wrecks -0.03 | -0.05 | 0.02 (forest 500000 @0.85) |
| fish | model | abyss -426.22, arctic -421.60, atlantis -423.19, bloom -422.16, brine -424.42, cave -424.04, city -423.57, coral -416.74, forest -423.13, sargasso -425.18, shallows -416.35, trench -423.73, vents -425.72, wrecks -424.89 | -423.69 | -398.11 (shallows 0 @0.15) |
| fading models: per-instance fade | pass | under 0.05 ms in every sea | | |
| corner mask | pass | under 0.05 ms in every sea | | |
| dolphins | model | not in view | | |

### The life, Atlantis and city groups' rows (V5–V8)

`features` on the Pi, medians where the feature shows (`docs/handoff/v5.md`, `v6.md`):

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| wildlife: `world::LIFE` and the reef's new shoals (V5) | `DIVE3D_SKIP` bit 40 (`part::LIFE`) | abyss 0.15, brine 0.23, trench 0.18, vents 0.21, the rest 0.07–0.16 | 0.15 | 0.48 (shallows) |
| crabs (V5) | `DIVE3D_SKIP_EFFECTS` bit 45 | abyss 0.15, city 0.13, atlantis 0.12 | 0.04 | 0.21 (city) |
| rays (V5) | bit 44 | arctic 0.05 | 0.02 | 0.05 |
| eels (V5) | bit 43 | trench 0.03 | 0.02 | 0.05 |
| seals and sea lions (V5) | bit 46 | shallows 0.03 | 0.01 | 0.03 |
| Atlantis ring walls (V6) | `DIVE3D_SKIP_ELEMENTS` bit 5 | atlantis 0.25, trench 0.39, vents 0.20 | 0.20 | 0.65 (trench) |
| Atlantis canals and quays (V6) | bit 6 | atlantis 0.17 | 0.12 | 0.33 |
| Atlantis amphora fields (V6) | bit 13 | trench 0.18 | 0.12 | 0.25 |
| Atlantis triremes (V6) | bit 12 | trench 0.15 | 0.11 | 0.20 |
| Atlantis aqueduct (V6) | bit 11 | atlantis 0.13 | 0.09 | 0.16 |
| Atlantis acropolis (V6) | bit 2 | trench 0.24 | 0.07 | 0.24 |
| Atlantis stair, colossus, gate, statues, stelae, mosaics, lighthouse (V6) | bits 3, 4, 7–10, 14 | under 0.05 everywhere | 0.01 or less | 0.06 |
| debris (V4, now an element) | bit 1 | trench 0.09 | 0.04 | 0.21 |
| Atlantis Doric temple (V7) | bit 15 | atlantis 0.18 (`hotspots` pairs: +0.20 median, +0.03 at the worst place) | 0.13 | 0.18 |
| the drowned city's galleria and its rooms (V8) | `DIVE3D_SKIP_ELEMENTS` bit 16 | city: median -0.53 (it hides towers), worst place +0.39 (`hotspots` pairs) | | +0.39 |

### The new seas' rows (V9)

`features v9 DIVE3D_HOTSPOT_SCENES=meadow,sponges,hole` on the Pi (`docs/handoff/v9.md`):

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| glass sponges | `DIVE3D_SKIP_EFFECTS` bit 50 | sponges 1.32 | 1.32 | 3.72 (sponges 500000 @0.15) |
| seagrass | bit 48 | meadow 1.08 | 1.08 | 1.46 (meadow 500000 @0.15) |
| mangroves | bit 49 | meadow 0.50 | 0.50 | 0.78 (meadow 0 @0.85) |
| the blue hole's wall | bit 51 | hole -0.33 (it hides what lies behind it) | -0.33 | 0.07 |
| the blue hole's stalactites | `DIVE3D_SKIP_ELEMENTS` bit 17 | hole 0.02 | 0.02 | 0.03 |
| the blue hole's halocline | bit 18 | hole 0.01 | 0.01 | 0.01 |

### Near detail (V10)

`run v10-off DIVE3D_SKIP_ELEMENTS=524288` against `run v10-on` on the Pi, all 204 places
(`docs/handoff/v10.md`):

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| near detail: pebbles, shells, stars, urchins, sea cucumbers, worm tubes | `DIVE3D_SKIP_ELEMENTS` bit 19 | every sea but the coral, 0.07-0.25 | 0.13 | 0.42 (arctic 0 @0.85) |

### Rocks, plants, sponges and coral redone (V10)

`run plants-c` against `run v10-on`, all 204 places (`docs/handoff/v10.md`): median 6.98 -> 7.17
ms, worst 10.19 -> 10.28 (hole 0.85), none over 11. Faceted rocks (flat-shaded, 960 vertices
against 642) are most of it, +0.1-0.3 ms in every sea; the new kelp +0.3 in the shallows; the
glass sponges are cheaper (their sea's worst 9.45 -> 8.71); coral +0.06. Seagrass (44 blades
for 40) moves nothing measurable (`plants-d`, meadow 7.86 / 9.98).

### The volcano, the abyss's glow and lights through the haze (V10)

`run far-on` against `run plants-c`, all 204 places: median 7.17 -> 7.17 ms, worst 10.28 ->
10.39 (atlantis 500000 @0.15), none over 11.

Not split per feature: the two runs differ by all three changes. Per sea (median / worst ms,
before -> after): atlantis 8.39 / 9.85 -> 8.47 / 10.39 (the volcano), abyss 6.35 / 8.92 -> 6.42 /
9.26 (the glowing colonies), every other sea within run-to-run noise (the glow sprites' fog).
The colonies' own switch is `part::ABYSS_GLOW`; the volcano's cone and crater are in the cones'
and vents' effect groups (`DIVE3D_SKIP_EFFECTS`).

### The forest's frond crowns (V10)

`run trees5` against `run far-on`, the forest's 12 places: median 7.78 -> 8.48 ms, worst 8.83 ->
9.43 (500000 @0.15), none over 11. The 80-triangle leaf spheres became sprays of 5 fronds (3 far),
each a closed bent diamond of 8 triangles, so it is pixels, not vertices: +0.6-1.0 ms where the
trees fill the view, nothing at 0.85 where the boat is above them. Tried first: 6 longer open
sheets (`trees2`, +0.6-1.2, and the tree loses back-face culling), the same as closed 20-triangle
shells (`trees3`, +2-3), 6 long diamonds (`trees4`, +0.9-1.5). Part of the trees' effect group
(`effect::TREES`); there is no separate switch, the old crown is gone.

### Three mangrove stands (V10)

`run mang-b` against `run far-on`, the meadow's 12 places (with the frond crowns, which the
meadow's edge shows): median 7.85 -> 8.03 ms, worst 9.98 -> 10.26 (0 @0.15), none over 11. The
mangrove is three models now (9, 11 and 13 roots, the same 11 on average), each stand leaning,
splayed unevenly and tinted; the two extra draws and the crowns are the +0.0-0.6. Switch: the
mangroves' effect group (`effect::MANGROVES`) covers all three.

### Imported models (M1)

`features` on the Pi, branch `m1-gltf` (`docs/handoff/m1.md`); `run m1-before2` against `run
m1-after3`, all 204 places: median 7.14 -> 7.27 ms, worst 10.39 -> 10.54 (noise at a place with
nothing new), none over 11; wrecks 6.80 -> 6.94.

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| Atlantis' bronze shark, manta and whale (M1) | `DIVE3D_SKIP_ELEMENTS` bit 20 | atlantis 0.04 | 0.04 | 0.12 |
| the wrecks' chests and barrels (M1) | `DIVE3D_SKIP_ELEMENTS` bit 21 | wrecks 0.09 | 0.09 | 0.14 (wrecks 0 @0.15) |

### Ships as wrecks (M2)

`run m2-before2` against `run m2-after2`, all 204 places (`docs/handoff/m2.md`): median 7.19 ->
7.30 ms, worst 10.38 -> 10.39, none over 11; wrecks 6.93 -> 7.03, their worst place 8.52 -> 8.91.

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| the wrecks' Dutch hulls and fallen masts (M2), against no hull | `DIVE3D_SKIP_ELEMENTS` bit 22 | wrecks 1.19 | 1.19 | 2.06 (wrecks 0 @0.15) |

### The ground group's rows (V3/V4)

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| sediment: sand line on slopes, silt drifts (V4) | `DIVE3D_SKIP` bit 38 (`part::SEDIMENT`), priced by `hotspots` pairs | coral 0.18, forest 0.14, cave 0.08, the rest within the noise | 0.04 | +0.04 at the worst place |
| debris round wrecks, ruins and outcrops (V4) | `DIVE3D_SKIP` bit 39 (`part::DEBRIS`) | under 0.05 in every sea (`features`, branch `ground`) | | |
| ice keels, rounded and lit through (V3) | `DIVE3D_SKIP_EFFECTS` (`effect::ICE_KEELS`) | under 0.05 in every sea (`features`, branch `ground`) | | |

### The water group's rows (V1/V2)

These change a shader switch or the seabed's tiles, which the alternating method above cannot
price (every other frame would recompile the pipelines or rebuild the seabed), so they are priced
by `hotspots` pairs in one session (four visits, 168 places; `docs/handoff/water.md`):

| feature | switch | seas (median ms there) | median ms | worst ms |
|---|---|---|---|---|
| water surface from below: mirror, wave facets, Snell's window (V2) | `DIVE3D_SKIP` bit 37 (`part::SURFACE_FACETS`) | shallows 0.26, coral 0.24, trench 0.37, forest 0.22, wrecks 0.14, atlantis 0.05, the rest under 0.05 | 0.10 | +0.59 at the worst place (coral s0 0.15: 9.66 -> 10.25) |
| finer seabed levels, 0.06/0.24/0.96 (V1 reinvestment) | `DIVE3D_SKIP` bit 36 (`part::COARSE_SEABED`) | cave 0.44, brine 0.42, forest 0.15, atlantis 0.07, the rest about 0.1 or noise | 0.12 | |
| finer model levels, LOD 1.5/4.5 px (V1 reinvestment) | `DIVE3D_LOD=2,6` | forest 0.22, atlantis 0.08, city 0.08, the rest within the noise (+-0.15) | 0.04 | |
| the sight's cull (V1) saves | `DIVE3D_SKIP` bit 35 (`part::NO_SIGHT_CULL`) draws everything as before | vents 1.71, forest 1.25, wrecks 1.23, bloom 1.21, atlantis 1.11, brine 1.06, city 0.98, arctic 0.95, abyss 0.78, shallows 0.71, coral 0.51, sargasso 0.51, cave 0.47, trench 0.44 | -0.85 | worst place 10.04 -> 9.39 |

## Reading the table

(Written for the `tooling` table of 2026-11-01, 8.50 ms frame median; the ranking still holds in
the `water` table above, with every row smaller where the sight now cuts the far distance.)

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
