# Handoff: the drowned city (biome 13)

Branch `worktree-agent-a5ea2066e52e6a8dd`, fast-forwarded onto `dive-visuals` (5fed499) before the work.

## What it is

A fourteenth sea: a drowned art-deco city of brass and iron. In code and text it is "the drowned city",
and the README calls it a Milliard City, after Verne's *Propeller Island*.

- Id `CITY = 13`, at the end of every per-biome table. `places::biome_name` gives `"city"`.
- Canonical order: `[0, 8, 1, 9, 7, 2, 10, 3, 13, 11, 4, 12, 5, 6]`, so it comes after Atlantis and before
  the brine flats. It is leg 8 for seed 0.
- True seabed depth 8.0, visual depth 7.2. Cruise band (0.45, 1.30), no ceiling. Relief 0.08, and the far
  mountains are scaled by 0.3. The floor is level terraces with silted paving (`biome_shape` in world.rs,
  the shader and the test mirror).
- Water: a teal-green tint, haze 0.09 (clear, so silhouettes read), the sun cut by 60%, and no sky.
  `levels.w` carries the city's share. `water_at` adds a warm horizon glow from it.

## How it is built

- Six models, appended after `Tree`: `Tower`, `Rotunda`, `Dome`, `Walkway`, `Pipework` and `Sign`.
  - Each one is a single sealed instance with LODs.
  - Every part is tagged in `aux.x` (`mesh::city`: plain, windows, panes or neon).
  - `city_part` makes every part wind outward, so the whole model is sealed.
- New material `world::material::CITY = 11` with its own pipeline (10 became the comb jelly when the branches merged) (added to `MATERIALS`):
  - `vs_model` packs the part kind into the length of the local normal, and a per-instance seed into
    `surface.w`.
  - `city_light` in `scene.wesl` draws the window grid in the shader. Lit and flickering windows are
    seeded by hash. Far away the grid averages out.
  - It also draws the leaded lit glass in a world-space lattice and the buzzing, stuttering neon.
  - Neon colour per instance comes from `Instance.floor`. City models are never grounded.
  - Emissive light is added after fog at 1 - 0.8·haze, so far windows still glow.
- Placement:
  - The static `city` placer runs in `static_steps` and is cached. It uses two world-fixed grids, split at
    |across| = 12 so they never overlap:
    - near blocks of 3.2 (towers, rotundas and domes, plus walkways to neighbours, kelp, bubble streams
      as ember sprites, signs with glow sprites and flickering point lights);
    - far blocks of 6.0 (silhouettes out to 60 units).
  - Pipes run along the avenue at across 1.5 to 2.3.
  - Nothing tall is placed within `AVENUE = 2.3` plus the footprint of the corridor. Spires are capped at
    y = -0.9.
  - The dynamic `city_beams` step adds sweeping searchlights from about 30% of the near crowns.
  - Fish gates, rock tones and `tall_things` (the sister submarine rises over towers) are updated. Big
    boulders are kept out of the streets.

## Stats

Desktop, M1 Max, `DIVE3D_STATS=1`, city shots:

- Model triangles: 38k to 58k.
- Instances: 143 to 246.
- Sprites: about 600 to 760.
- Placers: about 0.5 ms on a cache rebuild and 0.2 ms otherwise.

Hotspots, seed 0: the city legs are not among the 20 most expensive.

- At 0.15 into the leg it measured about 2.0 ms mean in one run, against 5.35 ms for the heaviest place
  (shallows).
- In another run all three city places were under 1.37 ms.

It has not been measured on the Pi.

## Files

`src/voyage.rs`, `src/dive3d/{world,mesh,mod,terrain,places}.rs`, `src/shaders/scene.wesl`, `README.md`,
`docs/13-dive3d-city.png`, `docs/handoff/city.md`.

The `bubbles_never_pop` test moments moved: bloom encounter 3015 s to 3556 s, deep trail 3407 s to 3810 s.
The leg after Atlantis shifted every later leg by 300 s.

## Risks

- The extra material adds 6 pipelines, which costs compile time at startup on the Pi.
- The city fragment work (window hash per pixel) is untested on V3D. Towers cover a lot of the screen.
- Any other branch that adds a biome or a Model variant will conflict in the per-biome arrays, `ORDER`,
  `Model::ALL` (now 54) and `MATERIALS`.
- Inserting the leg after Atlantis moves the absolute times of every later leg. Screenshot places tied to
  seconds, and encounter names, may now land in different seas.
- Decoding the part kind from the normal's length relies on interpolated normals staying above about
  0.87 of their length. That holds for the current segment counts.
