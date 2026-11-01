# perf-a: tunnel shader, vent plumes, caustic ceiling, draw order (P1-P4)

Branch `perf-a`, four commits on top of `main` at `6c12ca6`. Measured on the Pi 5 on **OpenGL ES**
(early-Z on), `hotspots`, the full default (4 visits, 168 places, 6 frames), kiosk frozen by
`deploy/bench-pi.sh`. Absolute numbers drift by about +-0.05 ms between runs and single places by
up to +-0.5 ms, so each claim below is a pair run in one session, and the acceptance pair was run
twice.

## Result

Base commit's binary against the final one, run back to back (two pairs):

| | median | worst | places over 11 ms | over 10.5 ms |
|---|---:|---:|---:|---:|
| base (`6c12ca6`) | 8.27 / 8.27 | 10.88 / 10.89 | 0 / 0 | 5 / 5 |
| `perf-a` | **7.69 / 7.68** | **10.72 / 10.71** | 0 / 0 | 2 / 2 |

(The very first base run read 11.17 ms worst, city s0 0.45, one place over 11 ms; the repeats read
10.9. The worst place is the city's, which is P5's.)

Per-sea medians, base to `perf-a` (first pair; the second pair differs by under 0.2 ms):

| sea | before | after | | sea | before | after |
|---|---:|---:|---|---|---:|---:|
| cave | 8.72 | 7.01 (-1.71) | | atlantis | 8.83 | 8.47 (-0.36) |
| vents | 9.08 | 7.64 (-1.44) | | city | 9.22 | 8.84 (-0.38) |
| wrecks | 8.01 | 7.11 (-0.90) | | shallows | 8.21 | 7.83 (-0.38) |
| brine | 7.62 | 6.81 (-0.81) | | trench | 6.80 | 6.53 (-0.27) |
| forest | 9.12 | 8.32 (-0.80) | | coral | 8.83 | 8.64 (-0.18) |
| abyss | 7.48 | 7.04 (-0.44) | | sargasso | 8.51 | 8.34 (-0.17) |
| arctic | 8.62 | 8.57 (-0.05) | | bloom | 7.45 | 7.46 (+0.01) |

Biggest single places (base to final): cave 0.15 -3.0 to -3.5 ms at all four seeds, brine s750000
0.85 -3.0, forest s750000 0.85 -3.0. Largest loss anywhere: +0.34 (coral s500000 0.85, a place that
swings by that much between identical runs).

## P1: the tunnel's own 4-thread fragment shader (brief T3)

Premise re-checked: on GL with early-Z the tunnel still cost what the brief said (cave s250000 0.15
10.74 ms before, the slowest place of that run bar the city).

`fs_vault` (`scene.wesl`) is the entry point of the opaque vault pipelines (`models`, `sealed
models`; the fading ones never draw the tunnel and keep `fs_model`). `vs_model` computes, for the
vault only, what varies smoothly over a wall and hands it over in slots the vault does not use
elsewhere: the tap's uv and LOD (`local`), the floor bounce's `low` x breathing (`floor_y`), the
lamp's reach/cone falloff, `near`, the cracks' glow and the warm tint (new `Lit.vault`), the normal
turned toward the window (`normal`), the lamp direction (`world`) and the lamp/eye halfway vector
(new `Lit.halfway`), both left unnormalised. Per pixel: the tap, the bumped normal, two N.L terms,
the tone and the shine.

| shader | instructions | threads | temps |
|---|---:|---:|---:|
| `fs_model` for the vault (before) | 187 | 2 | 36 |
| `fs_vault` (Vulkan, shaderdb) | 105 | **4** | 31 |
| `fs_vault` on the Pi's GL driver (`V3D_DEBUG=shaderdb`, kiosk frozen) | 103 | **4** | 31 |

(31 temps is under the 32 a 4-thread shader may use; a later change that adds a varying here may
tip it.) The vault's vertex shader grew 574 to 637 instructions at 2 threads, which the tunnel's
at-most-16 k vertices do not feel.

`Lit` had 15 user varyings, the limit wgpu's GL backend enforces (16 with `front_facing`), so the
city's one-bit `grounded` varying now rides in `surface.w` (the window seed plus 4096 when the
building stands on the seabed); `city_light` reads it back. Pixel-identical (checked, city
screenshots mean 0.000 after fixing a first attempt that put it in `floor`, which grounded models
read).

Pi, P1 alone (one pair, same session): cave 0.15 -1.89 / -1.92 / -1.99 / -2.03 ms at the four
seeds (brief: -2 or better), brine 0.85 -1.80 / -1.73 (brief -1.5), forest s250000 0.85 -1.61,
shallows s500000 0.85 -1.52, cave median 8.64 to 7.69 (-0.95; brief -0.8), forest -0.55. Overall
median 8.27 to 8.20, worst 11.17 to 10.85. No sea's median worse by more than 0.09 (brine +0.09
is that sea's own 0.15 place moving +0.31, and atlantis s250000 0.45 read +0.44: places move by
that much between identical runs, and the acceptance pairs at the end show neither).

Pictures (`docs/handoff/img/perf-a-cave.jpg`: before, after, difference x8): the cave previews
differ by a mean of 0.38/255 at most (`10-cave-entering`; 41 at the single worst pixel, along one
silhouette edge of the tunnel, where the normal is turned toward the window per vertex instead of
per pixel); the rest of the cave frames 0.02-0.05. No facets of the per-vertex lamp are visible
in the closest frames.

## P2: one ribbon per vent plume (brief T4)

Premise re-checked: abyss s500000 0.85's glows are plumes too (it is one of the plume's gains).

`Frame.plumes`, drawn by their own pipeline `Dive3d plumes` (`vs_plume`/`fs_plume`), one ribbon of
4 quads along the plume's rise instead of 8 (chimneys) or 12 (vents field) additive `GLOW` discs.
The fragment shader adds up what the discs did: a Gaussian across, half-width growing from the
mouth to the top, fading `1 - rise`, with the puffs' rhythm as a ripple running up the column
(`2 exp(-380 w^2)` strong while the puffs are small and apart, gone where they merge) whose phase
the CPU supplies (`Frame::plume`); the ribbon wanders sideways a little. The pipeline is 40
instructions at 4 threads. Switch: `DIVE3D_SKIP` bit `part::PLUMES` (1 << 33 = 8589934592; it is
a `DIVE3D_SKIP` bit, not a `DIVE3D_SKIP_EFFECTS` one: the plumes are no longer `GLOW` sprites, so
`DIVE3D_SKIP_EFFECTS=8589934592` no longer prices them, and `effect_cost` lists "pass: vent
plumes").

Pi (P2 against P1, one session): vents median 9.10 to 7.67 (-1.43), wrecks -0.44, forest -0.35.
Places with a chimney near the window: vents s0 0.15 -2.56 and 0.45 -2.10, s500000 0.15 -1.80,
s250000 0.15 -1.57, s750000 0.15 -1.36; abyss s500000 0.85 -2.35; wrecks s0 0.85 -2.43; forest
s750000 0.85 -2.96; city s250000 0.85 -1.99. Brief: -1.5 ms at the vents' 0.15/0.45 places, met where a
plume is near (vents s250000 0.45 -1.08, s750000 0.45 -0.49; the places with no chimney near the
window moved by +-0.05). Median 8.20 to 8.05. The
`DIVE3D_SKIP_EFFECTS` "remaining glow cost <= 1 ms" check does not apply any more (see above).

Pictures (`docs/handoff/img/perf-a-vents.jpg`): the plume keeps its look, a soft widening column
rising and fading, wafting. It is **not pixel-identical**: vents frames differ by a mean of 0.7-2.3
(max 74 in the difference image, in the plume's body), the abyss's by 0.3. **This is the change the
user should look at** (best on the Pi's panel in motion: the puffs' ripple is the one thing that
is only an approximation of the old beads).

## P3: a cheaper caustic ceiling (brief T6)

Premise re-checked with shaderdb and previews: the ceiling is 89 of water #1's 318 instructions
(49 for the sheen and fine ripples, 31 for the swell, 9 for the rest). Dropped, as the brief's
step 1 does term by term against the previews:

- the sheen (`chop`, a `pow(.., 18)`) and the fine cells (two more sines), which the 42 % wash over
  open water leaves under 8/255 (previews of the shallow seas: mean 0.41, max 8); and
- the swell's outer sine (`sin(swell * 0.5)` became `0.25 * swell`; the sine was only read near its
  straight part), no measurable difference.

Dropping the swell entirely was tried and measured too: mean 0.88/255, over the brief's 0.5, so it
stays. Water #1 318 to 264 instructions and water #3 398 to 350, threads unchanged (4 and 2).
The baked-texture route of the brief (step 2) was not taken: the remaining sines would have to be
replaced by a pattern that cannot match them, so it would be a visible change.

Pi (P3 against P2, run twice): median 8.05 to 7.71 and 8.03 to 7.77 (-0.34 / -0.26); atlantis
-0.41 / -0.38, wrecks -0.48 / -0.42, trench -0.34 / -0.31, shallows -0.38 / -0.24, brine -0.39 /
-0.06, coral -0.27 / -0.11, arctic -0.07 / -0.01. **Partly met**: the median goal (-0.3) is met on
one pair and nearly on the other; the brief's -0.5 in each of six seas is met in none (best
-0.48). The rest of the ceiling's cost is the swell's three sines, the caustic fetch and the two
washes, which cannot go without a visible change. Worth knowing: arctic barely draws the ceiling.

Pictures (`docs/handoff/img/perf-a-water.jpg`): previews mean at most 0.44/255, max 8; side by
side the frames are indistinguishable (the submarine differs between the runs because it moves).

## P4: draw order for early-Z

The probes that already existed were re-measured on GL with early-Z (P2 binary, one session each;
`DIVE3D_SKIP` bits):

| order | median | verdict |
|---|---:|---|
| as is (models, seabed, water; tunnel first) | 8.05 | |
| `TERRAIN_FIRST` (134217728) | 8.04 | cave -0.47, but forest +0.66, shallows +0.51, coral +0.28: the kelp and corals hide the seabed, so the seabed first shades what they then cover. **Not adopted.** |
| `DEPTH_FIRST_DRAWS` (67108864), nearest-first across materials | 8.03 | every sea within +-0.16, places +-0.5 both ways. No gain, 8x the draws (PO section 7.4). **Not adopted.** |
| both | 8.10 | worse (2 places over 11 ms) |

What does pay is the tunnel: its lower walls lie under the cave's own floor, so drawn first they
are shaded and then covered by the seabed. Drawn after the seabed (always) the cave's places along
the axis gain 1.1-1.5 ms but the places at the mouths lose up to +0.94, where the walls hide the
seabed behind them. The rule that separates them in the data: the boat within 0.7 tunnel radii of
the tunnel's axis draws it after the seabed, everywhere else it goes first (`tunnel_late` in
`draw`). `DIVE3D_SKIP` bit 34 (17179869184, `part::TUNNEL_FIRST`) restores the old order.

Pi (new rule against `TUNNEL_FIRST` in the same binary, two pairs): cave median -0.55 / -0.54,
brine -0.40 / -0.56, bloom 0 / -0.25 (noise); cave 0.15 -1.70 / -1.45 / -1.19 / -1.06 and brine 0.85
-1.28 / -1.02; no sea worse than +0.12 in either pair. Overall median 7.74 to 7.70 (first pair; the second
7.85 to 7.70), worst 10.82 to 10.71. Screenshots of every place are **pixel-identical** (mean 0.000 over all 87).

Not tried: sorting the terrain tiles of one level nearest-first (levels already go finest first,
which is nearest first); not expected to matter since the seabed rarely overlaps itself.

## Gates

`deploy/v3d-shaderdb/check.sh .` ok, baseline updated (`fs_vault` rows, `Dive3d plumes`, water
#1/#3, the vault and city vertex shaders; the vault vertex shader 574 to 637, the fading models'
574 to 505 smaller because the dead `grounded` varying left them); `cargo test --release` 125
passed; `cargo clippy --release --all-targets -- -D warnings`; `cargo fmt --all -- --check`.

## Left, and things for the user

- Look at the vents (`docs/handoff/img/perf-a-vents.jpg`, or live): the plume is an approximation.
- The Docker VM's disk filled up twice during this work (builds died with "No space left on
  device"; 24 GB of unused volumes and 33 GB of images are reclaimable by `docker system prune`),
  and the Pi's `/tmp` (2 GB tmpfs, shared) is full of other runs' binaries and result directories:
  `bench-pi.sh` never cleans the binary it copies there (about 47 MB a run).
- `deploy/bench-pi.sh compare` still says "over 14 ms"; the budget is 11 ms.
- The remaining headroom for this group: the water's ceiling (about 0.5 ms median still, needs a
  visible change to go), the city's s0 0.45 (10.9 ms, the worst place, P5).
