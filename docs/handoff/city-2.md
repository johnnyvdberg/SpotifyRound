# Handoff: the drowned city, cheap fidelity (second pass)

Branch `worktree-agent-ae42607726b1b47c1`, on `dive-visuals-finish` at 773da4e ("Measure the nook and
city pass on the Pi"). The work is commit **bfd0589**; this note is the commit after it.

The user said the city "looks great" and asked for more fidelity from rendering tricks that suit the
Pi's VideoCore VII. The rule for every trick was the tiler's: no new render pass or render target, no
full-screen post, no new texture reads (dependent or otherwise), no discard, no new blended overdraw
over large areas. What was added is arithmetic in the city's own fragment shader, data baked on the CPU
into vertices that already exist, and a few small sprites.

## What was missing (first renders)

- Lit windows were flat coloured squares, with no depth behind them.
- The walls were evenly lit from the plinth to the crown. Nothing grounded a building on the seabed.
- The seabed round the buildings was the same dark silt everywhere, though thousands of windows looked
  down on it.
- Glass and brass did not reflect anything; unlit windows were plain dark squares.
- Neon was a flat colour, with no glow.
- Dark towers merged with the dark water behind them.

## The tricks

All but the last two live in `city_light` and `city_room` in `src/shaders/scene.wesl`, which both
fragment shaders (`full_hd`, which the Pi runs, and the 448 profile) call. The vertex shader adds one
flat varying (`Lit.eye`, location 12) and reuses one free slot (`floor_y`).

1. **Interior mapping (`city_room`).** Each lit window follows the view ray into an analytic box room
   (a window wide, a floor high, about as deep). It shades whichever wall, floor or ceiling the ray
   reaches first, plus a lamp hotspot and a depth falloff.
   - The camera is moved into model space once per vertex (cofactors over the determinant, from the
     columns the normal transform already builds) and passed flat. The fragment rebuilds the ray as
     `local - eye`, exactly.
   - About 40 ALU operations, with no texture.
   - It runs only for lit panes nearer than 45 units (the old far averaging takes over beyond). The
     branch is coherent over a window's pixels, so whole 16-lane groups skip it.
   - *Why it suits V3D:* the QPUs are comparatively strong at arithmetic and weak at texture
     latency. A room atlas or cube map would be a dependent read per pixel.
2. **Per-window variation, from the window's existing hash.** Rose, sage or cream wallpaper, the lamp to
   one side, curtains half drawn (red or green, lit through), slatted blinds half down. In 8% of rooms
   a figure paces the back wall.
3. **Lifts.** About half of the faces have one column whose windows are dark, with a cold-lit car riding
   slowly up and down it, pausing at each end. A compare and a smoothstep, only in that column.
4. **Fake reflections (`city_mirror`).** An analytic environment keyed to the reflected ray: the
   deep-to-surface water gradient, plus the city's warm horizon band (`levels.w`).
   - Unlit window glass mirrors it with a cubic Fresnel. Lit glass mirrors it faintly. Dome, cupola and
     walkway panes take it on their leaded glass.
   - Brass takes it as a matcap with a highlight band, so the cornices and fins glint as the boat
     moves. How much a part shines is baked per vertex into the free `aux.y` from its colour's warmth
     (`mesh::polish`), and the vertex shader passes it in `floor_y` (city models are never grounded).
   - No cube map and no extra pass.
5. **Baked ambient occlusion at the foot.** Buildings that stand on the seabed are darkened over the
   plinth and first floor, from the model's own height. That is one smoothstep. They are flagged by
   `floor.w = 1` in `place_building`, so signs, walkways and pipes are untouched.
6. **Rim light from the haze.** Every city surface takes the water's fog colour at grazing angles, so
   dark silhouettes separate from dark water. A cube of `1 - n·v` and a multiply-add.
7. **Neon tubes with a hot core.** The tube is whiter where it faces the window and keeps its colour at
   the edges. A couple of multiplies, no bloom pass.
8. **Lamplight pooled on the seabed, plus contact shadows, baked into terrain vertices**
   (`world::Spill`, `world::city_spills`, `terrain::build`, `vs_terrain`).
   - When a seabed tile is built, the city's buildings that can reach it are found with the same
     deterministic `building`/`street_shop` functions the placer uses: near grid, far grid and shops.
   - Each building throws a warm pool, tinted by its neon and as strong as it is tall, falling off from
     its footing. A rounded-square or round distance in its own yaw gives a dark contact line where
     wall meets silt.
   - The light goes into the tile's existing `glow` attribute, marked by a negative `glow.a`.
     `vs_terrain` adds it to the light on the floor (`CITY_SPILL`) instead of to the albedo. The lava
     cave keeps the slot wherever both are present.
   - *Why it suits V3D:* a vertex-coloured decal is free. Point lights per pool, or blended decal
     quads, would cost overdraw or per-pixel loops. The finest seabed grid is 0.08 units, so the pools
     are smooth.
   - CPU cost, desktop: finding the spills is about 5 µs a tile, and a whole city tile builds in about
     78 µs.
9. **Faked bloom (`world::crown_glow`).** One small additive GLOW sprite over every visible building's
   beacon, and a soft halo over the crown neon band of towers and round towers. These are 0.2 to 0.6
   units across, through the existing sprite pipeline, and cover only the pixels that glow.

Not done, on purpose:

- Far skyline cards: the far grid already reaches 60 units.
- Lamp cones: `city_beams` already sweeps searchlights.
- Bloom on lit windows: too many sprites.
- Light spill on facades: the walls' quads have no interior vertices to carry it.

## Stats (`DIVE3D_STATS=1`, the four city shots, desktop)

| shot | draws | instances | model tris | model verts | sprites before → after |
| --- | --- | --- | --- | --- | --- |
| 8-city-early | 30 | 154 | 43 322 | 61 656 | 954 → 988 (+3.6%) |
| 8-city-middle | 25 | 128 | 35 608 | 51 846 | 874 → 903 (+3.3%) |
| 8-city-leaving | 29 | 148 | 41 894 | 64 211 | 771 → 816 (+5.8%) |
| 8-city-other-seabed | 29 | 145 | 38 152 | 53 366 | 936 → 965 (+3.1%) |

Draws, instances, triangles and vertices are unchanged. Sprites rise 3–6%, well inside the +25% budget.
There are no new pipelines, passes or render targets.

Hotspots, `DIVE3D_HOTSPOT_SEEDS=0`, 30 frames, desktop M1 Max. The desktop was shared with other
agents' GPU work, so absolute times drifted between runs. The city's share of the run's mean frame is
the steadier figure:

| | city 0.15 / 0.45 / 0.85 (ms) | mean of all 42 places | city / mean |
| --- | --- | --- | --- |
| before (773da4e) | 0.72 / 0.74 / 0.74 | 0.91 | 0.81 |
| after | 1.06 / 1.46 / 1.25 | 1.54 | 0.81 |

**The Pi was not measured** (it was out of bounds for this task). The fragment work added is on city
surfaces only:

- about 15 ALU operations on every city pixel (rim, mirror, foot);
- about 40 more on lit, near window panes (the room).

That is a guess of 0.5–1.0 ms on the Pi's city leg (8.89 ms mean, worst place 10.85 ms, against a
16.6 ms budget). Please run the usual Pi hotspots (`docs/handoff/pi/README.md`) to confirm.

## Renders

In the scratch directory
`/private/tmp/claude-501/-Users-john-projects-SpotifyRound/1b823e08-a68e-457f-8b2c-48cbf3ae0c7c/scratchpad/city3/`:

- `before/` and `after/`: the four city shots and their `sheet.png`;
- `before-after-sheet.png`: the two sheets stacked, before above after.

## Risks

- **Pi cost unmeasured.** `city_room` is the longest piece. If the city leg grows too much on V3D,
  lower its cut-off (`far < 0.999` in `city_light`, i.e. 45 units), for example to 30 units.
- **Varyings.** `Lit` gains a flat `vec4` (location 12) for every model pipeline: 34 interpolated
  components in all, well under V3D's limit. Pipelines other than the city's write zeros.
- **`floor_y` is reused** for the city's brass weight. That is safe only while city models are never
  `grounded()`.
- **Terrain `glow.a < 0`** now means city lamplight. Anything else that reads `glow.a` as the cave's
  share must use `max(glow.a, 0)`, as `vs_terrain` does.
- **Spill placement mirrors the placer.** If the placer's `building`/`street_shop` rules change, the
  pools follow automatically. A new kind of building needs a footprint in `Spill::of`.
- **Only the `full_hd` terrain shader** (the default and the Pi's) shows the pools. The 448 profile's
  `vs_terrain` does not read the baked attributes.
- **Brightness tuning** was judged on desktop renders only. The pools (`CITY_SPILL = 2.2`) make the
  dense early shot's floor fairly amber. Lower it if it reads too warm on the panel.
