# water: realistic visibility (V1) and the surface from below (V2)

Branch `water`, from `main` at `9e10613`. Measured on the Pi 5 (OpenGL ES), `hotspots`, the full
default (4 visits, 168 places, 6 frames), kiosk frozen by `deploy/bench-pi.sh`; every run of the
table below in one session.

## Result

| run | median | worst | over 11 ms |
|---|---:|---:|---:|
| base (`main` 9e10613, `water-before`) | 7.63 | 10.49 (forest s0 0.15) | 0 |
| V1 cull only (`water-v1`) | 6.61 | 9.39 | 0 |
| V1 with the reinvestment (`water-v1r`) | 6.76 | 9.59 | 0 |
| V1 + V2, final (`water-v2b`) | **6.86** | **10.25** (coral s0 0.15) | 0 |

Per sea, median / worst (ms):

| sea | base | V1 cull | V1 + reinvest | final |
|---|---|---|---|---|
| abyss | 6.91 / 9.89 | 5.96 / 7.22 | 6.06 / 7.37 | 6.04 / 7.39 |
| arctic | 8.28 / 10.02 | 7.27 / 8.93 | 7.27 / 9.32 | 7.26 / 9.29 |
| atlantis | 8.49 / 9.39 | 7.26 / 8.60 | 7.35 / 8.89 | 7.47 / 9.18 |
| bloom | 7.11 / 8.93 | 5.78 / 7.61 | 5.92 / 7.73 | 5.90 / 7.72 |
| brine | 6.92 / 8.95 | 6.03 / 7.98 | 6.45 / 8.09 | 6.21 / 8.63 |
| cave | 6.87 / 10.14 | 6.23 / 9.16 | 6.68 / 9.59 | 6.69 / 9.44 |
| city | 8.84 / 9.78 | 7.51 / 8.80 | 7.35 / 8.67 | 7.37 / 8.75 |
| coral | 8.63 / 10.41 | 7.95 / 9.39 | 8.08 / 9.53 | 8.54 / 10.25 |
| forest | 8.18 / 10.49 | 6.83 / 8.76 | 7.06 / 8.97 | 7.16 / 8.99 |
| sargasso | 8.29 / 10.26 | 7.47 / 9.05 | 7.62 / 9.58 | 7.54 / 9.50 |
| shallows | 7.82 / 10.02 | 7.10 / 8.73 | 7.20 / 8.75 | 7.49 / 9.66 |
| trench | 6.29 / 8.17 | 5.88 / 7.21 | 5.80 / 7.42 | 6.09 / 7.57 |
| vents | 7.58 / 9.38 | 5.76 / 7.74 | 5.96 / 7.96 | 5.93 / 8.06 |
| wrecks | 6.96 / 8.50 | 5.59 / 7.45 | 5.66 / 7.70 | 5.68 / 7.72 |

So the dive is 0.8 ms faster at the median and every sea is faster than before, with the
headroom partly spent on a finer seabed, finer model levels and the surface; the worst place is
the coral's 0.15 (10.25 ms), where the surface costs most.

## V1: every sea has a sight

`world::SIGHT` gives each sea a visibility distance (1 unit ~ 11 m): 13 under the ice, over the
trench and in Atlantis, 12 over the reef, 11 in the drowned city, 10 in the shallows and the
sargasso, 8 in the forest and the cave, 7.5 at the wrecks, 7 in the abyss, 6.5 over the brine,
6 at the vents, 5.5 in the bloom. The fog is `1 - exp(-2.4 d / sight)` scaled to reach exactly 1
at the sight (`world::fog`, `fog_of` in `scene.wesl`, `g.water.w` = 2.4 / sight), and at full fog
`fog_at`'s colour is exactly the open water behind it (the skyward tint fades out with the fog),
so what is not drawn beyond the sight is never missed.

- **Cull**: `World::visible` rejects anything whose nearest point is beyond the sight (plus the
  static cache's margin), `cells` stops placing there, the seabed's tiles go through `visible`
  too, and the static cache is rebuilt when the sight changes by a quarter unit (a border).
- **Looking up** the haze's way is up to 0.6 shorter (`world::UP_CLEAR`, `fog_toward`): a shape
  against the bright surface shows further than one against the dark water. Without it the
  sargasso lost its weed rafts overhead.
- **Lights carry twice as far** (`LIGHT_SIGHT`): the drowned city's towers are placed out to twice
  the sight (`visible_lit`, `lit_cells`) and their windows fade over that range while the walls are
  fully hazed: "on into the haze, where only their windows show" keeps working. Sun shafts and
  the vents' plumes fade over twice the sight too.
- **LOD**: the fog bias of `level_of` follows the new curve (1 / what the fog leaves).
- **Saving** (`part::NO_SIGHT_CULL`, bit 35, the same binary drawing everything as before, haze
  unchanged): median 7.46 -> 6.61 ms (-0.85), worst 10.04 -> 9.39; vents -1.71, bloom -1.21,
  atlantis -1.11, forest -1.25, wrecks -1.23, arctic -0.95, city -0.98, abyss -0.78, brine -1.06,
  sargasso -0.51, cave -0.47, coral -0.51, shallows -0.71, trench -0.44.
- **Reinvested**: the seabed's levels are 0.06/0.24/0.96 units apart instead of 0.08/0.32/1.28
  (the coarsest still reaches past every sight; `part::COARSE_SEABED`, bit 36, restores the old
  ones): +0.12 ms median. Models keep their finer levels further (LOD error 1.5 px, edge 4.5 px,
  from 2 and 6; `DIVE3D_LOD=2,6` is the old): +0.04 ms median. Atlantis' temple fronts are placed
  from four units abeam (six before), since the farther ones are now in the haze.

Pictures, base | final: `docs/handoff/img/water-murky.jpg` (vents, bloom, abyss, brine,
sargasso, Atlantis) and `docs/handoff/img/water-clear.jpg` (shallows, coral, forest, trench,
wrecks, city, arctic, cave).

- **Vents, bloom, abyss**: the hard dark silhouettes of far ridges against the water are gone;
  the ground dissolves into the water. The abyss's far wall of the drop is now inside the dark
  (its sight is 7): the window shows the near ground, the jellies and the smokers.
- **Brine**: the far crust no longer ends in a line against the water; the neighbouring city that
  showed across the brine's border at 20+ units is gone.
- **Sargasso**: hazier and golden; the rafts overhead stay (looking up), the far sargassum
  stands fade.
- **Atlantis**: the farthest temple fronts and ruins (beyond 13) are gone; the colonnades are
  unchanged.
- **Shallows, coral, forest, trench, arctic, cave**: nearly unchanged at mid range; the far
  distance is a little hazier.
- **City**: towers beyond 11 are their windows only; the far dark skyline silhouettes are gone.

## V2: the surface from below

`water_ceiling` (`scene.wesl`, `SKY_CEILING` and `SKY_SNELL`): the surface is tilted by the three
swells, two short chops across them (moving at 1.5-1.7 rad/s) and the caustic web's cells; per
pixel the ray's angle to the tilted normal decides between the **mirror** of the water below
(outside the critical angle; darker the steeper the reflection looks down) and **Snell's window**
(inside 48.6 degrees: the sky, tinted by depth and sun, brightest at the window's rim). The side
window looks at most ~20 degrees up, so the level window overhead is never in view; what shows is
what a diver sees looking up at a shallow angle, a silvery moving mirror with bright facets
flashing where a wave face turns toward the window. The caustic web stays as the light under it,
the whole washed 42 % toward the open water and dissolved into the haze at the sight.

- Switch: `part::SURFACE_FACETS` (bit 37, `DIVE3D_SKIP=137438953472`, shader switch
  `OLD_CEILING`), a row in `feature_costs::PASSES`.
- Shader: water #1 264 -> 473 instructions, still **4 threads** (32 temps, at the limit: worked as
  scalars behind `fenced()` so the waves are not hoisted beside the caustic fetch; a first version
  with vec3 intermediates was 2 threads). Water #3 350 -> 538, 2 threads as before.
- Cost: the first version cost +1.1 to +1.3 ms in atlantis, trench and wrecks, where the ceiling is
  mostly haze; now the ceiling returns the open water at once beyond the sight and the facets fade
  out between 0.7 and 0.9 of the haze and are skipped past it. Final, with against without (same
  binary): median 6.76 -> 6.86 (+0.10), shallows +0.26, coral +0.24, trench +0.37, wrecks +0.14,
  forest +0.22, sargasso 0, atlantis +0.05; worst 9.66 -> 10.25 (coral 0.15).
- Pictures: `docs/handoff/img/water-surface.jpg` (top halves of the coral and the shallows, base
  | final).

## Gates

`cargo test --release` (128 passed), `cargo clippy --all-targets -- -D warnings`,
`cargo fmt --all -- --check`, `deploy/v3d-shaderdb/check.sh .` ok with the baseline updated.

## Left

- The Atlantis volcano stands 26-44 units off and is beyond any sight now (it was 99 % hazed
  before too); if it should be seen, it needs to stand nearer or show as a glow (`LIGHT_SIGHT`).
- The abyss's "steep drop": its far wall used to show as a hard silhouette; with sight 7 it is
  dark. A searchlight-lit or glowing feature near the rim would bring it back.
- The facets' bright patches are smooth lenses at the Pi's resolution; a finer chop (a third
  wave, ~+25 instructions) would break them up, but water #1 is at 32 temps.
- Denser near placement was not done per placer (the "ground" task owns the placers); the
  saving left (median 6.86 vs 7.63 ms) is room for V3-V10.
- `docs/feature-costs.md`'s full table was regenerated (`water-features`), see there.
