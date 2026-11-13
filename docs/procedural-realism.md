# Procedural realism on a small GPU

Notes on what makes the solar tour's procedural shaders look real for what they cost on the Pi 5's
V3D, with where each is used. Before and after shots of changes are kept in `docs/compare/`.

## What gives the most for the cost

| Technique | What it fixes | Cost | Used in |
|---|---|---|---|
| **Domain warping** — `f(p + warp(p))`, here the warp taken from the noise volume's own gradient (`noised`), one extra fetch ([Quilez, *warp*](https://iquilezles.org/articles/warp)) | Noise reads as round blobs; warped, lumps flow into each other like broken stone or turbulent gas | 1 fetch per evaluation | `rock_shape` |
| **Noise with derivatives** — value and slope from one fetch, the slope used for normals and to shape the sum ([Quilez, *morenoise*](https://iquilezles.org/articles/morenoise)) | Normals by finite differences cost four times the noise; erosion-like detail | Free with `noised` (gradient stored in the volume) | `rock_shape`, rock grain, planets |
| **Crater profiles from a small Voronoi search** — bowl, raised rim, fresh ejecta, from the eight cells about the point (craters kept within half a cell) | Raw Voronoi has no rims and looks like cells; a profile with a rim reads as impacts | 8 hashes per scale, only on rocks over ~10 px | `craters`, `crater_profile` |
| **Regolith reflectance** — Lommel–Seeliger blended with Lambert (lunar-Lambert), its limb rise damped | Lambert on rock looks like plastic; airless, dusty surfaces stay bright to the terminator ([JPL asteroid rendering](https://robotics.jpl.nasa.gov/media/documents/Villa_ea_Image_Rendering_AAS_GNC_2023.pdf)) | A division | `fs_rock` |
| **Soft shadows from the distance field** — the darkest `k·h/t` along a short march toward the light ([Quilez, *rmshadows*](https://iquilezles.org/articles/rmshadows)) | Lumps and crater walls casting no shade | 6 shape evaluations, only on rocks over ~12 px | `fs_rock` |
| **One-sample ambient occlusion** — the shape's distance a little way out along the normal | Hollows lit as brightly as crests | 1 shape evaluation | `fs_rock` |
| **Fading detail by pixel footprint** | Shimmer and aliasing from detail finer than a pixel | A smoothstep per octave | craters, grain, specks, hive cubes |
| **Level of detail by size on screen** — fewer march steps, no craters or shadows on small rocks | Paying full cost for rocks a few pixels across | Saves most of the cost of a field | `fs_rock` (`rs`) |
| **No grazing glints** — specular faded toward the limb | Backlit small rocks ringing like bubbles | — | `fs_rock` |
| **Clustered placement** — density modulated at two coarse scales | Evenly sprinkled fields look synthetic | 2 hashes per instance (vertex) | `vs_rock` |

## Pitfalls met

- The noise volume is bound to the fragment stage only, so vertex shaders use hashed cells.
- When a ray misses a marched shape, the soft edge must be shaded where the ray passed closest,
  not where the march gave up, or every small object gets a bright ring.
- Lommel–Seeliger's `2μ₀/(μ₀+μ)` goes to 2 at the limb; damp the denominator.
- Whites blended over the frame in linear light look far stronger on screen: square a flash.

## Next candidates

- Bake per-rock shapes into a small 3D distance texture once a field, and march that instead of
  the noise (fewer fetches per step).
- Blue-noise dither on the final colour, against banding in dark gradients.
- Analytic anti-aliasing of the warp drives' streaks by their pixel width.

Sources: [Quilez, domain warping](https://iquilezles.org/articles/warp) ·
[Quilez, noise derivatives](https://iquilezles.org/articles/morenoise) ·
[Quilez, soft shadows](https://iquilezles.org/articles/rmshadows) ·
[Villa et al., JPL, image rendering for small bodies](https://robotics.jpl.nasa.gov/media/documents/Villa_ea_Image_Rendering_AAS_GNC_2023.pdf) ·
[Procedural generation of asteroids, Uni Bremen](https://cgvr.cs.uni-bremen.de/research/procedural_asteroid/) ·
[Book of Shaders, fBm](https://thebookofshaders.com/13/)
