# Jellyfish: swimming, trailing arms and species

Branch `worktree-agent-afcc99977990fb724`, fast-forwarded onto `dive-visuals`
(5fed499). The code is in commit 6487b58; this note is in the commit after it.

## What changed

- **The swimming stroke is the vertex shader's** (`swim()` in `scene.wesl`,
  anim kinds `PULSE` 5 (redefined), `TRAIL` 10 and `GLIDE` 11 in `world::anim`).
  - The bell contracts on an asymmetric stroke: a quick squeeze over a quarter
    of the cycle that narrows the margin, lengthens the bell and flicks the lip
    in, then a slow relaxation.
  - The animal rises on each stroke and sinks between. The rise is added in
    world space after the instance matrix, in bell radii, so every part moves
    together. A slow wander rides on top.
  - The arms and tentacles take the same stroke late down their length
    (`aux.x`). The margin pulls their roots in, and a wave scaled by the
    part's stretch travels to the tips. Each thread's bearing staggers it.
  - Phase and rate vary per individual. `drifters` no longer reads `w.t`, so
    it moved into the static cache.
- **Three instances per medusa, all placed like the bell.** Before, a jelly
  was about 17 instances: 1 bell, 4 arms and about 12 tubes.
  - `Bell` has 16 lappets. The modelled stomach is gone and painted instead.
  - `JellyArm` is now all four frilled, twisting oral arms in one mesh.
  - `JellyFringe` is new: twelve 3-sided threads, long and short alternating.
  - **Old bugs fixed:** tentacles placed with `between()` had their writhe
    scaled by their radius, so they hung as rigid sticks. The arms' mesh ran
    along -y, so they stuck up through the crown.
- **`fs_glass`'s `jelly_glass()`**, shared by both profiles, paints:
  - radial canals or stripes, the moon jelly's four horseshoe gonads, a
    stomach, a bright margin and Fresnel edges;
  - bioluminescent spots with waves of light running round the rim;
  - brightening with the stroke, which runs down the arms and threads too.
  - Glow uses the unlit tint, so each species keeps its colour.
  - Two `Lit` slots are reused for swimming instances, which are never
    grounded: `floor_y` carries the contraction and `floor` the tint.
  - The full-HD glass pass now applies the placement fade to jellies.
- **Species** (the `MEDUSAE` table in `world.rs`): moon jelly, lion's mane
  (long red-orange tentacles), crystal jelly (rim lights), sea nettle and
  helmet jelly. The abyss favours helmet, crystal and comb jellies.
  - The bell's pattern is passed in `surface.xyz` via `Tweak::patterned`.
    Glass is never textured, and `surface.w` (the LOD axis) is kept.
- **New model `Comb`**: a comb jelly with eight rainbow cilia rows
  (`material::COMB` 10). It glides rather than pulses.
- **Registration**: `JellyFringe` and `Comb` are appended after `Tree` in the
  `Model` enum and `Model::ALL`, `model_bit` maps them to `JELLYFISH`, and the
  `MODEL_GROUPS` text is updated.
- **Animation check**: `DIVE3D_SHOTS_OFFSETS=0,0.3,0.6,0.9` reshoots each
  place that many seconds later, as `<name>+<o>s.png`.

## Budget

Triangles at full detail and the LODs:

| Part | Full | LOD 1 | LOD 2 |
|---|---|---|---|
| Bell | 960 (was 1048) | 480 | 260 |
| Arms (all four) | 336 | 168 | 120 |
| Fringe | 720 | 360 | 216 |
| Comb | 256 | 128 | 96 |

A full-detail jelly is about 2016 triangles, against about 2008 before.

`DIVE3D_STATS`, before → after:

| Place | Instances | Model tris | Model verts |
|---|---|---|---|
| bloom early | 1475 → 383 | 111050 → 72118 | 67635 → 45086 |
| bloom middle | 1473 → 445 | 111192 → 73220 | 69390 → 47496 |
| abyss middle | 812 → 285 | 68092 → 43066 | 40171 → 25354 |
| wrecks early | 596 → 291 | 54950 → 47450 | 32640 → 28089 |

Sprites are unchanged or fewer (the crystal jelly's tip sprites are gone).

Hotspots (seed 0) were run with the machine at load average 39. Every scene
is about 3 to 3.8 times slower than the quiet baseline, so absolute timings
mean nothing. The jelly scenes slowed least (bloom about ×2.9 against
×3.3 to 3.8 for the others), with instances down 50 to 70%. Re-time on a quiet
machine or the Pi.

## Screenshots

All under
`/private/tmp/claude-501/-Users-john-projects-SpotifyRound/1b823e08-a68e-457f-8b2c-48cbf3ae0c7c/scratchpad/jelly/`:

- `before/` and `v3/`: the before and after frames.
- `compare-*.png`: before on the left, after on the right.
- `v3/seq/*+{0,0.3,0.6,0.9}s.png` and `seq-*.png`: the animation sequence.

## Risks

- **Moving `drifters` into the static cache**: placement visibility and fades
  now step with the 0.2-unit cache cell, like the other static decor.
- **Reused slots**: `Lit.floor_y` and `floor`, and the glass `surface.xyz`,
  carry jelly data. Any future glass material that reads them, or any
  grounded swimming instance, would clash.
- **LOD**: the fringe's coarse LODs are chosen from the straight, unanimated
  threads, so distant threads bend with only 3 or 5 segments.
- **Not checked on the Pi.** The fragment cost per jelly pixel is a little
  higher: `atan2`, a few `pow`s and the rim-light math.
- **The `!full_hd` glass path** shares `jelly_glass()` but is otherwise
  untested, since it cannot be built.
