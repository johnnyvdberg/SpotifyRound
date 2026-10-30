# Coral reef refinement: handoff

Branch `worktree-agent-a4eb1e492f5d74a97`, fast-forwarded from `dive-visuals`
(5fed499). The work is in the commit that adds this file.

## What changed

- **Branching corals (`mesh.rs`)**: `coral()` and `coral_finger()` are now
  grown by `colony()`/`grow()` from a `Habit`. Trunks lean out of a low crust,
  fork recursively, curve upward and taper, with rounded tips. The colour runs
  from a shaded foot to a pale tip that differs per tip. The new `tube()`
  helper (parallel-transported rings, one smooth surface per limb) replaces
  `branch()`. LOD 1 keeps every fork and drops sides; LOD 2 also drops the last
  generation, and limbs reach further so the outline holds.
- **New models** (appended to `Model` and `ALL`, registered in `model_bit` under
  CORALS, in `MODEL_GROUPS` "coral", in `own_skin`, with LOD bias 4):
  - `CoralBush`: a bushy cauliflower or bird's-nest coral.
  - `CoralBrain`: a lumpy dome that wears the new `Surface::Brain`.
  - `Anemone`: a column with two rings of swaying tentacles.
  - `SeaWhip`: a clump of gorgonian whips.
- **`Surface::Brain` (`textures.rs`)**: the brain coral's meandering labyrinth is
  the zero line of a sum of integer-wavevector waves, so the tile wraps. Layer
  indices 8 and 15 are mirrored as `CORAL_LAYER`/`BRAIN_LAYER` in `scene.wesl`.
  A test guards that they match.
- **Table coral**: 28 segments with a lobed, irregular rim, a shaded and ribbed
  underside, and a darker stem.
- **`world.rs`**: `reef()` places 7 kinds (staghorn, bush, finger, table,
  brain, anemone, whip). Colours come from `reef_hue()` (8 muted hues,
  jittered per colony) and `brain_hue()`. `coral_garden()` mixes bush with
  staghorn, uses real brain corals with an anemone beside them instead of
  three squashed spheres, and adds a soft-coral roll (sea whips or tube
  sponges with an anemone).
- **Impostors (`scene.wesl`)**:
  - New silhouettes matching the models: staghorn with forked antlers, a
    thicker cupped table, uneven fingers, a brain dome with broad grooves, and
    a lumpy bush dome.
  - Pseudo-normal shading.
  - Edges dithered over about 1 px with interleaved gradient noise.
  - Cross-fade: cards are also drawn 1.1 units inside the 4-unit model switch.
    The vertex shader dissolves them, by the camera's current distance, over
    the unit after `stretch.z`.
- **Lighting**: the full-HD `fs_model` adds a cheap rim/subsurface glow to
  surfaces with the coral or brain layer.

## Stats (DIVE3D_STATS, last of 3 frames)

| shot | before | after |
| --- | --- | --- |
| 1-coral-early | 482 inst, 122.6k tris, 81.4k verts, 27 draws | 480 inst, 127.8k tris, 79.4k verts, 35 draws |
| 1-coral-middle | 493 inst, 122.5k tris, 89.7k verts, 23 draws | 491 inst, 128.0k tris, 87.9k verts, 30 draws |
| 9-coral-other-seabed | 532 inst, 126.8k tris, 96.7k verts, 27 draws | 539 inst, 143.1k tris, 101.0k verts, 36 draws |

In 1-coral-early, Coral uses 55.3k verts before and 29.9k after (plus 11.1k
for CoralBush). 1-coral-leaving is the forest (trees) and is essentially
unchanged.

Screenshots (scratchpad, not in the repo):
`/private/tmp/claude-501/-Users-john-projects-SpotifyRound/1b823e08-a68e-457f-8b2c-48cbf3ae0c7c/scratchpad/coralx/{before,after}/`

## Risks and follow-ups

- There are 7 to 9 more draw calls per coral frame (new models times their
  LOD levels). Vertex and instance counts are within budget, but the Pi's
  per-draw cost has not been measured.
- The impostor fragment shader now runs up to 15 capsule SDFs (staghorn) plus
  `fwidth`. The cards are small, but the GPU time on the Pi is untested.
- The dithered impostor edges and cross-fade use a screen-fixed pattern. They
  may shimmer slightly in motion.
- The model/impostor choice is still made at static-cache time, so it moves
  in 0.5-unit steps. The shader-side fade hides most of that.
- The anemone and sea-whip close-ups were not inspected in any shot (none came
  near the window), only checked by bounding box and triangle counts.
- Table coral tops still read as fairly flat slabs up close.
