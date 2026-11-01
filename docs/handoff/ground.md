# Ground: V3 (ice less blocky) and V4 (terrain integration, placement realism)

Branch `ground`, from `main` 9e10613. Pictures are before | after, from `deploy/shots.sh run`
(`ground-before` on 9e10613, the after run on this branch).

## V3: the polar ice

- **Keels** (`mesh::keel`): the extruded slabs are now pressure ridges of rubble worn round: nine
  overlapping flat-bellied lobes of different depths on a subdivided icosphere (`mesh::blob`,
  2562 / 642 / 162 vertices at its three levels of detail, so the full shape only near the
  window), a ragged outline where it meets the sheet, smooth normals.
- **Lit through**: the vertices carry the light scattered in the ice, pale blue-white where it is
  thin (by the sheet, at the edges) and glacial blue down the thick middle; keels are drawn in the
  ice's own pipeline (`material::ICE`, marked `world::ICE_KEEL`), at the sheet's brightness, a
  little brighter at the silhouette, and with the sheet's thinner haze. Before they were dark
  silhouettes against the sheet. The ice pipeline's vertex shader skips the lights entirely (the
  sheet never read them), its frost texture runs round and down a keel, not streaked from above.
- Now and then (14 % of keels, off the lane by 3 units) a **berg**: broader (1.6 to 2.5) and
  deeper (0.65 to 0.9, kept above the sisters' ceiling under the ice).
- **Floes** (`world::floe_light`, in the ice bake): the pack is cells of different thickness,
  rounded where they ground together, with brighter refrozen seams; the bake only works its
  fields out where the arctic's share reaches 0.3 (cheaper than before outside the arctic).
- **Texture** (`textures::Surface::Ice`): melt scallops with bright thin rims, frost platelets,
  bubbles and fractures, softer than before.

![arctic](img/ground-arctic.jpg)
![a berg as the arctic comes in](img/ground-arctic-berg.jpg)

## V4: bedded into the seabed, rules as data

- `world::Bed` and the rows of `world::bed` (`STONE`, `OUTCROP`, `FLAT`, `GROWN`, `WRECK`, `DRUM`,
  `BUILT`): how much of a thing is sunk, how far it leans with the slope, the steepest ground it is
  found on and how far the ground may fall away under its footprint. `world::seat` samples the
  ground at the thing's middle and four points round it and returns its height and slope, or
  `None` where it would hang off a lip or lie on too steep a bank. `Seat::tilt(yaw)` gives the
  pitch and roll that lay it along the slope.
- **Stones** sink about a third and lie with the slope, never on lips (the rocks perched on
  Atlantis' terrace edges are gone). Per-sea tables `STONES` (how stony the bottom is) and
  `OUTCROPS` (how many boulders are bedrock): stony abyss, trench and vents; few stones on the
  Sargasso's silt, the bloom's ooze and the brine's crust (which now reads as a crust plain);
  almost none on the reef (as before); no outcrops in Atlantis' paving. Seas keeping 0.45 keep
  exactly their old stone cells.
- **Outcrops**: long, half sunk, tilted with the slope plus a dip of their strata, with three
  scree stones below them down the slope.
- **Wrecks** settle along the slope (pitch and list from the ground), sunk to the bilge, in the
  first berth near their cell where `bed::WRECK` lets them lie (`wreck_berth`, shared with the
  sisters' `tall_things`), with **scour pits** at bow and stern and **debris** down-current:
  planks, a bronze gun, ballast stones (`wreck_debris`).
- **Ruins**: fallen drums roll to rest along the slope with chips beside them; temple fronts stand
  only on ground level enough to build on (`bed::BUILT`), with fallen drums and entablature blocks
  in front.
- **Trench** slabs are thick and bedded back into the rim, only their lip over the drop (they read
  as floating discs before); the sediment spills from the new lip.
- **Starfish** lie along the slope; polar sponges and the shallows' fans keep off ground they
  could not root on.
- **Shader** (`vs_model`, grounded pipelines only): the sand line follows the slope under the
  model (`world::ground_under` packs the slope into `floor.w`), so a stone on a bank is buried
  evenly instead of having a level band through it; silt drifts against the side facing the
  current (`CURRENT`). The per-frame grounding pass now skips the cached scenery, which found its
  ground when the cache was built (it was redone every frame).
- The divers' obstacles follow the same stone rules.

![shallows](img/ground-shallows.jpg)
![wrecks (brightened)](img/ground-wrecks.jpg)
![atlantis](img/ground-atlantis.jpg)
![brine](img/ground-brine.jpg)
![trench](img/ground-trench.jpg)

## Switches and costs

- `part::SEDIMENT` (bit 38, `DIVE3D_SKIP=274877906944`; shader switch `NO_SEDIMENT`, 256): the slope-following sand line and silt drifts.
- `part::DEBRIS` (bit 39, `DIVE3D_SKIP=549755813888`): everything marked `world::DEBRIS` (scour, planks, guns, ballast, chips, rubble,
  scree).
- Keels are still `effect::ICE_KEELS`.
- shaderdb: grounded vertex shaders +37 to +46 instructions (still two threads, as before); the
  ice's fragment shader 97 to 154 (four threads); its vertex shader 146 to 188 (four threads). The
  grounded ice pipelines (never used) are no longer built. Baseline updated.

## Measurements

On the Pi 5 (OpenGL ES), `hotspots`, four visits, 168 places, kiosk frozen; each table's runs in one
session. (The bits were 35/36 on the branch; the merge with `water` moved them to 38/39 and the
shader switch from 128 to 256.)

**The branch, before merging** (2026-11-01, coordinator): `ground` 546da81 against `main` dc04018.
`ground` was branched from 9e10613, before `water`'s sight cull, so this pair mostly shows the cull;
against its own base (9e10613, `docs/handoff/water.md`'s base column) the branch is level: median
7.63 -> 7.58, arctic 8.28 -> 8.19 (the keels' own shader, 8be63da, took back the 8.98 of the run
before it), abyss 6.91 -> 7.08.

**Merged into `main`** (dfd5809), against `main` before the merge (dc04018), two sessions:

| run | median | worst | over 11 ms |
|---|---:|---:|---:|
| `main` dc04018 | 6.88 / 6.89 | 10.29 / 10.27 (coral s0 0.15) | 0 |
| merged dfd5809 | 6.82 / 6.84 | 10.21 / 10.18 (coral s0 0.15) | 0 |
| merged, `part::SEDIMENT` off | 6.80 | 10.14 | 0 |

Per sea, median / worst, second session (ms):

| sea | main | merged | merged without sediment |
|---|---|---|---|
| abyss | 6.08 / 7.89 | 6.20 / 7.47 | 6.21 / 7.42 |
| arctic | 7.28 / 9.31 | 7.27 / 9.17 | 7.21 / 9.22 |
| atlantis | 7.49 / 9.23 | 7.14 / 9.09 | 7.13 / 9.07 |
| bloom | 5.94 / 7.72 | 5.95 / 7.60 | 5.95 / 7.62 |
| brine | 6.25 / 8.67 | 6.14 / 8.55 | 6.13 / 8.64 |
| cave | 6.71 / 9.47 | 6.87 / 9.30 | 6.79 / 9.33 |
| city | 7.39 / 8.76 | 7.29 / 8.73 | 7.25 / 8.75 |
| coral | 8.57 / 10.27 | 8.67 / 10.18 | 8.49 / 10.14 |
| forest | 7.19 / 9.01 | 7.19 / 9.00 | 7.05 / 9.10 |
| sargasso | 7.59 / 9.58 | 7.40 / 9.31 | 7.40 / 9.35 |
| shallows | 7.53 / 9.72 | 7.41 / 9.61 | 7.38 / 9.61 |
| trench | 6.13 / 7.57 | 6.20 / 7.10 | 6.23 / 7.10 |
| vents | 5.98 / 8.09 | 6.18 / 7.75 | 6.14 / 7.84 |
| wrecks | 5.71 / 7.74 | 5.76 / 7.33 | 5.78 / 7.33 |

- The arctic is back to `main`'s time; Atlantis (-0.35, its terrace rocks gone) and the sargasso
  (-0.19) are faster.
- The abyss is +0.12 at the median and -0.42 at its worst place: visit 500000 is 0.15-0.25 slower,
  the others level or faster. Setting its stone share back from 0.55 to 0.45 was measured (6.23 ->
  6.20, within the noise) and not kept.
- **Sediment** costs 0.04 ms at the median (coral 0.18, forest 0.14, the rest within the noise). The
  alternating `features` method cannot price it (the switch recompiles the pipelines, so it read
  "under 0.05 everywhere"); these are `hotspots` pairs.
- **Debris** is under 0.05 ms in every sea (`bench-pi.sh features`, on the branch).
- Ice keels: under 0.05 ms in every sea (`features`), from +0.7 ms in the arctic before 8be63da.

Pictures of the merge (`deploy/shots.sh compare`, before | after), most changed first: the arctic's
keels, the cave's approach and the brine's crust (fewer, bedded stones), the abyss's border with
the ice, the trench's lip. Every sea was looked at.

## Left

- The rock meshes themselves are still smooth lumpy spheres; a rock with facets and strata would
  sell the outcrops more.
- Vents' chimneys and the city's props are not seated by the rules (the vents' plumes and lights
  are tied to their cells; the city's streets are flat).
- No scour geometry in the terrain itself (the heightmap must match the shader); the pits are
  dark lenses on the sand.
- The ordinary keels near the lane can still hang deeper than the boat's ceiling under the ice
  (as before this branch).
