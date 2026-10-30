# Lava cave and tunnel: handoff

Branch `cave-handoff` (copy of `worktree-agent-ae5bd40c7e85ad16b`),
fast-forwarded onto `dive-visuals` (5fed499) before the work. The code is
`f846c1d`; this note and the last brightness tweak are the commit after it.
The original agent hit its session limit after the tests passed but before it
could write this note or commit the tweak.

## What changed

- **A baked vault layer (`textures.rs`).** `Surface::Vault` is not a colour:
  its RGBA texels hold the rock's tone, the slope of its relief across the
  tile, the slope up it, and how molten its cracks are, all painted by
  `paint_vault()`. The pattern is lava laid down flow on flow: ledged flows,
  pillow lava, cooling joints that do not all open, vesicles, and pale
  flowstone streaks. Every field wraps. `paint_all()` now keeps four channels
  at `RANGE` for all layers, so the one texture keeps its size and mips.
- **The tunnel's vertex pass (`vs_model`).** The arch takes no sun and no
  searchlight in the vertex stage: its walls get the point lights (lava pools,
  passages) summed once a vertex, a per-vertex heat mask (broad hot patches,
  strongest low down), and a warm fog at the far end of the passage so the
  tunnel reads deep instead of opening onto cold blue water. The arch's
  inward-facing normals are turned toward the window first.
- **`vault_hd()` (`scene.wesl`).** The full-HD tunnel is shaded from the one
  texture tap: the stored slopes tilt the smooth normal into ledges, joints
  and vesicles; the searchlight is a soft cone from its mount, strongest on
  what faces it and falling off with distance; lava light comes up from the
  floor and is caught by downward faces; cracks pulse; wet rock glints where
  the beam strikes; rock right at the glass sits in the boat's shadow so a
  near wall is darker than the passage beyond it. No noise and no light loop
  in the fragment path.
- **The 448 profile** keeps the cheap `vault()` path, now reading only the
  tone channel of the same layer; the full-HD and low-cost paths share the
  data, not the per-pixel work.
- **Geometry and cliffs.**
  - The tunnel wall grows lava benches: two wandering shelves where the flow
    stood, on the walls only, so the arch is unchanged and the hull still
    fits (`helm.rs` was not touched).
  - Stalactite tips are paler, so they catch the light instead of reading as
    black spikes.
  - The massif's cliff faces take the vault layer as soon as any cave is
    present, and shade its relief and molten cracks; the cliff over the
    tunnel mouth is basalt rather than a hill of the previous sea's sand.
- **A test** guards that `VAULT_LAYER` in the shader matches
  `Surface::Vault as usize`.

## Budget

One texture tap, as before, and no per-pixel noise in the cave. Draws,
instances and triangles in the cave shots are unchanged (14–22 draws, 88–110
instances, 4k–18k model triangles). The tunnel's benches add vertices to the
tunnel mesh only.

Hotspots on the desktop with the machine loaded (medians of 40 frames, seed
0), new versus the 5fed499 baseline:

| place | new mean / worst | baseline mean / worst |
| --- | --- | --- |
| cave 0.85 | 1.41 / 2.17 ms | 1.50 / 2.39 ms |
| cave 0.85 | 1.47 / 2.78 ms | 1.42 / 2.87 ms |

The two runs disagree by more than the change, so this says "no measurable
regression", not "faster". Re-measure on a quiet desktop and on the Pi.

## Screenshots

Before and after sheets, with all fourteen cave places:
`/private/tmp/claude-501/-Users-john-projects-SpotifyRound/1b823e08-a68e-457f-8b2c-48cbf3ae0c7c/scratchpad/cavevis/{before,after2}/sheet.png`.

The difference is largest at the wall-filling frames (`9-cave-middle`,
`9-cave-passages-*`, `helm-cave-deep`), which were flat red-brown sheets and
now show flow ledges, joints, glowing seams and a darker near wall against a
warmer passage.

## Risks and follow-ups

- The relief normal is derived from a tiled height field, so it cannot follow
  the tunnel's own curvature; it is a detail layer over the smooth normal, not
  a replacement. Very close walls are still mostly texture and tone.
- `sample.a` is now 1.0 for every non-vault layer; only the vault and the
  cave cliff read alpha, so the change should be invisible elsewhere, but it
  is a shared texture format change worth a look in the other biomes.
- The ember brightness and the tunnel's final point-light/bounce levels were
  tuned last (this commit): point lights 2.5 → 1.2 and lava bounce 1.5 → 1.1.
  The sheet in `after2/` is the tuned version.
- Nothing has been measured on the Pi. The shader is longer even though it
  does the same number of taps; the Pi's fragment cost is the open question.
