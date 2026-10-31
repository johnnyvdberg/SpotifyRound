# Coral reef, second pass: handoff

This builds on `dive-visuals-finish` at 773da4e ("Measure the nook and city
pass on the Pi"). The work is in the commit that adds this file.

The request was: "The coral doesn't really look like a coral reef to me." The
reef used to read as small scattered objects on pale sand, between dark
boulders, under murky teal water. It now reads as a shallow tropical reef.

## What changed

### Seabed shape

`world::reef_field` is shared by `biome_shape` in `world.rs`, the test copy
of the shader and `biome_shape` in `scene.wesl`. It is the sum of three
crossing swells, each periodic over the heightmap's 64 units. From it:

- Flat-topped bommies and the ridges joining them rise about 0.42 units
  (`reef_mound`).
- Sand gutters wind between them, scoured 0.10 deeper.
- Coral covers about two thirds of the ground (`reef_cover`).

`Seabed::reef`/`World::reef` sample the field on the seabed's own grain.

### The ground as living reef

- **Baked colour (`terrain.rs`).** The coral floor's per-vertex colour
  shades from gutter sand to a mosaic of colony hues, by `reef_cover`.
  `world::reef_tone` picks from 9 muted reef colours: ochres and browns are
  the commonest, with purple, pink, olive, cream and blue-grey. Colony seams
  are darker and bommie tops brighter.
- **New texture layer 17, `Surface::Reef` (`textures.rs`, `paint_reef`).**
  A wrapped, warped mosaic of small colonies. Each colony is a low dome of
  one of four kinds (polyps, branch ends, meanders or a smooth crust) with
  its own slight tint. Some borders open into crevices.
  - The coral biome's terrain layer moves from sand (12) to 17, in
    `terrain::LAYERS` and the shader's `surface_of`.
- **Reef amount in the layer index.** The tile stores how much reef is
  present in the fraction of its layer index (`REEF_SHARE = 0.45`).
  - Full-HD `vs_terrain` reads it into a new `reef` varying.
  - `fs_terrain` uses it to fade the carpet's texture to a faint rubble in
    the gutters. That is one `mix`, with no extra sample.
  - Tests guard `REEF_LAYER`, `REEF_SHARE` and the layer table.
- **Bedded feet.** Grounded models are bedded into this same colour
  (`world::floor_colour`), not into the plain biome floor.
- **Gutter sand.** `FLOOR_ALBEDO` for coral is now pale gutter sand
  (0.80, 0.72, 0.58) in both `world.rs` and the shader.

### Colonies by species, at reef scale

- **Patches.** `reef_patch`/`reef_species` lay the reef out in warped
  patches about 1.3 units across. Each patch is one species (staghorn,
  table, brain, bush, finger or soft coral), with 15% strays of another
  kind. `colony_hue` gives a patch one hue, with small per-colony jitter.
- **`reef()` (small corals and far cards).** It grows only where coral
  covers the ground, so the gutters stay clean. Colonies are about 1.6×
  larger on bommie tops, and the far impostors follow the same species,
  scale and hue.
- **`coral_garden()` (the bommies' big colonies).**
  - Staghorn thickets of 4–6 heads.
  - Terraces of 2–3 stepped table corals.
  - Massive brain boulders, a metre or two across, with an anemone.
  - Stands of bush or finger coral.
  - Soft corals: sea fans set broadside to the onshore surge, sea whips
    and anemones.
  - In the gutters there is only the odd star or shell.
- **Colours.** `REEF_HUES` is richer: ochre, brown, purple, pink, green,
  cream, soft blue and a golden accent.
- **Rocks.** `rocks()` leaves out about 90% of the boulders in coral. The
  rest are tinted and textured as pale coral rock.

### Reef fish and big animals

- **Reef fish swarms (`reef_fish`).** Over most bommie colonies, 8–21 small
  fish hover in one swarm. Each swarm is one kind: orange anthias,
  blue-green chromis, yellow damsels or purple anthias.
  - Each fish is an impostor card, form 5, so it costs one quad, adds no
    model vertices and sits in the static cache.
  - `vs_impostor` swims each fish round its swarm centre by the clock. The
    fish darts and bobs, and its card is mirrored so the nose leads.
    `fs_impostor` draws a side-on oval body with a forked tail,
    counter-shading and an eye.
- **Turtles and mantas** now also come to the reef (`gliders`).

### Water and light (`globals` in `mod.rs`)

- The water is clearer, bluer and brighter: level (0.11, 0.46, 0.66), deep
  (0.025, 0.20, 0.38), and haze down from 0.13 to 0.085.
- The sun is ×1.70 instead of ×1.55.
- The light keeps its reds over the reef, so the corals stay saturated:
  tint red ×1.40, green ×0.94.
- Sun shafts are a little stronger: 0.45 instead of 0.38.

### LOD

`CoralBrain` gets an LOD bias of 2, since its grooves come from the texture.

## Stats

`DIVE3D_STATS`, last of 3 frames:

| shot | before | after |
| --- | --- | --- |
| 1-coral-early | 35 draws, 437 inst, 78.5k verts | 29 draws, 254 inst, 70.8k verts |
| 1-coral-middle | 33 draws, 495 inst, 115.9k verts | 31 draws, 356 inst, 118.3k verts |
| 1-coral-leaving | 38 draws, 400 inst, 112.4k verts | 37 draws, 326 inst, 118.1k verts |
| 12-coral-other-seabed | 31 draws, 523 inst, 87.2k verts | 29 draws, 325 inst, 76.7k verts |

- About 30k of the middle shot's vertices, and about 75k of the leaving
  shot's, are the neighbouring forest's trees.
- The fish swarms are impostor quads, so they are not counted above.
- Static rebuild placers take 0.67–0.79 ms, against 0.68–0.96 ms before.

Desktop hotspots proxy (Apple M1 Max; seeds 0 and 500000; 8 frames; second
run, since the first run after a build was noisy):

| coral row | before mean ms | after mean ms |
| --- | --- | --- |
| seed 0, 0.85 | 2.01 | 1.74 |
| seed 0, 0.15 | 1.63 | 1.56 |
| seed 0, 0.45 | 1.57 | 1.46 |
| seed 500000, 0.15 | 1.47 | 1.40 |
| seed 500000, 0.45 | 1.31 | 1.21 |
| seed 500000, 0.85 | 1.22 | 1.24 |

The Pi was not used.

## Screenshots

Not in the repository:

- Before: `/private/tmp/claude-501/-Users-john-projects-SpotifyRound/1b823e08-a68e-457f-8b2c-48cbf3ae0c7c/scratchpad/coral3/before/sheet.png`
- After: `/private/tmp/claude-501/-Users-john-projects-SpotifyRound/1b823e08-a68e-457f-8b2c-48cbf3ae0c7c/scratchpad/coral3/after/sheet.png`
- Full-size frames are in the same folders.

## Risks and follow-ups

- **Pi cost is unmeasured.** The changes that touch the GPU are:
  - one extra varying and a `mix` in the terrain;
  - a fish branch in `fs_impostor`;
  - up to several hundred extra impostor quads, which use discard (dithered
    edges).

  Re-run the Pi hotspots for the coral leg.
- **Tiling.** The reef carpet tile repeats every 0.77 units. The baked
  colony colours break it up, but at grazing angles near the window a
  regular cell pattern can still show.
- **Sea fans.** They are still solid half-discs. Larger than about 0.2 they
  read as walls, so they were kept small. A lattice fan model or texture
  would help.
- **Table corals.** Close to the window they read as broad flat plates.
- **Fish motion.** The swarms' motion was only seen in stills. Their loops
  are a few tenths of a unit and may look mechanical in motion. They pass
  through corals, with no avoidance.
- **Brain LOD.** The brain-coral LOD bias may show slightly faceted rims on
  large domes at mid distance.
- **Leg borders.** Near borders the coral layer is chosen by the dominant
  biome (`b.a`), so the reef texture can show a little into the neighbouring
  leg's floor.
