# perf-b: city, glass, start-up, pacing (P5 to P8)

Branch `perf-b`, from `main` 8c2e64b. All Pi numbers: GL backend, `bench-pi.sh run` default (four
visits, 168 places, six frames a place), one session, kiosk frozen. Raw CSVs are in
`docs/handoff/perf-b/` (`hotspots-before.csv` is `main`, `-p6` is after P7+P6, `-p5` after P5's
first commit, `-after` the branch head).

## Result

| | median | worst | places over 11 ms |
|---|---:|---:|---:|
| `main` | 8.26 ms | 10.89 (city s0 0.45) | 0 |
| after P7+P6 | 8.16 | 10.97 | 0 |
| after P5 (first commit) | 8.16 | 10.65 | 0 |
| branch head | 8.17 | 10.65 | 0 |

Per-sea median / worst, `main` to head:

| sea | median | worst |
|---|---|---|
| abyss | 7.32 to 7.17 | 9.85 to 9.91 |
| arctic | 8.58 to 8.25 | 10.71 to 10.05 |
| atlantis | 8.81 to 8.73 | 9.81 to 9.43 |
| bloom | 7.45 to 7.13 | 9.88 to 8.89 |
| brine | 7.66 to 7.61 | 10.29 to 9.77 |
| cave | 8.69 to 8.30 | 10.71 to 10.30 |
| city | 9.22 to 9.12 | 10.89 to 9.75 |
| coral | 8.86 to 8.79 | 10.70 to 10.65 |
| forest | 9.12 to 9.13 | 10.57 to 10.57 |
| sargasso | 8.56 to 8.46 | 10.33 to 10.31 |
| shallows | 8.19 to 8.21 | 10.27 to 10.26 |
| trench | 6.80 to 6.57 | 8.49 to 8.49 |
| vents | 9.09 to 9.10 | 10.28 to 10.18 |
| wrecks | 8.02 to 7.74 | 8.78 to 9.31 |

Single places move by up to +-0.5 ms between runs (six frames each): vents s500000 0.45/0.85 and
wrecks s250000 0.85 read +0.5 at the head and are in seas nothing here touches. No sea's median
is worse by more than 0.02 ms.

Start-up (test binary, `Nautilus::new` plus four frames, kiosk frozen, three warm and two cold
runs, spread under 0.03 s warm):

| | `main` | head |
|---|---:|---:|
| warm (Mesa cache) | 6.67 s | 2.72 s |
| cold (`MESA_SHADER_CACHE_DISABLE=true`) | 12.60 s | 6.64 s |
| `mesh::Library::build` alone | 4.64 s | 1.23 s |

Pictures: `deploy/shots.sh compare before final` over 87 places: 74 identical, 13 differ by under
0.05 % of pixels (mean change at most 0.017/255, city only; a few pixels differ by up to 114/255
where a coplanar surface now loses a depth tie because the draw order inside a model changed).
No visible change. Example, city middle: `perf-b/8-city-middle-a.jpg` (before),
`-b.jpg` (after), `-diff.png` (difference x16).

## P5 (T5): the city by building part

Re-measured first: city s0 0.45 was still the slowest place of all (10.89 ms), 276 instances.

- `mesh::Library` sorts the triangles of each city model's levels by the part their vertices
  carry (`mesh::city`: plain 0, windows 1, panes 2, neon 3; the parts share no vertices, the order
  inside a part is meshopt's) and keeps the index count per part in `Lod::parts`
  (`Model::is_city`).
- `fs_model` has a `CITY_PART` override (`city_light` takes its `kind` from it). `compile()` makes
  8 city pipelines (solid, sealed, times four parts) in place of the 12 `material11` ones. The
  draw loop draws a run of city draws part by part: one pipeline bind per part, then each model's
  range of that part. City instances are always drawn with `pipeline_slot` of grounded=0, anim=0
  (nothing in the city animates; `every_instance_has_a_pipeline` asserts it).
- Shaderdb (fragment shaders): neon 128 instr and panes 160, both 4 threads (were part of one
  650-instruction, 2-thread shader); plain 180 instr, 4 threads after `city_env()` stopped
  holding the water tone for the whole function (33 to 31 temps); walls 561 instr, 43 temps,
  2 threads.
- Pi: city s0 0.45 10.89 to 9.75 (-1.14; target was -2), city s0 0.15 7.75 to 6.88, arctic worst
  10.71 to 10.05 (-0.66; target -1), city median 9.22 to 9.12 (-0.10; target -0.4).
  **Targets partly missed**: the windowed walls are still one 561-instruction, 43-temporary
  shader at two threads, and they are most of the city's pixels.
- Left: step 3 of the brief. Getting the walls to 32 temporaries needs either per-window state
  from a baked table and `city_room` from a baked atlas of rooms (a visible change: less
  variety, no pacing figure) or a restructure of `city_light`'s live ranges; three source
  reorderings gained 5 temporaries (48 to 43) and no thread. Try the atlas next.

## P6 (T7): glass by material and animation

Re-measured: the bloom's glass still cost real time (bloom median 7.45).

- `GLASS` in `mod.rs` lists the six (material, animation) pairs the voyage places as glass; one
  pipeline each, with `MATERIAL` and `ANIM` overrides (`fs_glass`/`jelly_glass` read the material
  through the override). Their order follows `mesh::Model`, which the unsorted additive draws keep,
  so the sums are added in the same order as before.
- Shaderdb: `fs_glass` 506 instr/2 threads becomes 57 (sponge, sea pen), 153 (crystal), 85 (arm and
  fringe) and 169 (comb jelly), all four threads; the bell stays 193 instr at two threads. Vertex
  shaders 1078/640 (binning) become 383 to 539 / 66 to 199.
- Pi: bloom median 7.45 to 7.13 (-0.3; target -0.5), the worst glass places -1.0 to -1.4
  (cave s0 0.85, arctic s500000 0.85, bloom 0.15, atlantis s250000 0.85; target -1).
- Left: the bell (`material 7`) at two threads.

## P7 (T8): start-up

- `Library::build` builds the models on up to three threads in model order (work taken from a
  shared counter), assembles the buffers serially; `the_parallel_library_is_the_serial_one`
  compares it byte for byte with `build_serial`.
- The `MATERIAL == -1` model pipelines (18) are gone: an unknown material falls back to plain
  and `every_instance_has_a_pipeline` fails the build instead. The fading pipelines are made for
  the ice only (it is the only thing `kind_of` sends there). Model pipelines: 144 become 86
  (36 solid, 36 sealed, 6 fading, 8 city); glass goes from 1 to 6.
- Not done: compiling only the slots a voyage uses (brief step c). The test samples only
  70 moments, so pruning by it would risk a missing pipeline mid-dive; compile-on-first-use would
  hitch the frame. Some of the remaining model slots are probably unused too, but proving it needs a
  data table that new content would have to maintain.
- The dead seabed relief path is removed after checking: `relief()`, `ground()`, `ground_parts()`
  had no callers (their `biome_shape`, `massif`, `trench_profile`, `hall`, `tunnel_wide` and
  constants went too; the Rust test of the biome shapes has its own copy of the formulas and
  never read the shader's), the `Rgba32Float` 512x512 texture, its binding 1, its bind group
  entry and the 4 MB `write_texture` on every visit are gone. The shader's copy of the wrecks'
  `rocks` thresholds had already drifted from the Rust one.

## P8 (T10): pace from the display

`SPOTIFY_ROUND_PACE=display` (default unset: the 60.00 Hz timer, unchanged). The visualizer
thread then renders the next frame as soon as the UI thread has run the closure that shows the
last one (`Takes` in `visuals.rs`), waiting at most 100 ms. At most one frame is ever waiting for
the UI, so it never takes two in one refresh (the first would be lost) and the beat follows the
UI's present rate, the panel's 60.07 Hz. The GPU still has one frame in flight, as before.
The animation clock already follows real time, so nothing else changes.

Offline: `visuals::tests::the_display_sets_the_rate_where_a_timer_drifts` runs both pacings
against a 60.07 Hz panel in virtual time for a minute (9 ms render, 1.5 ms jitter): timer 62
repeated refreshes and 58 lost frames, display pacing 0 and 0. It is a model of the UI taking one
posted frame per refresh, not a measurement; whether the real UI thread behaves that way is what
the kiosk check settles (how to run it: `docs/agent-tasks.md`, Notes for the user).

## Gates

`cargo test --release`, `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all -- --check`
and `deploy/v3d-shaderdb/check.sh` (baseline updated three times: P7/P6, P5, the plain part) pass.

Tooling note: the Docker VM disk was full (about 24 GB of anonymous volumes), so the Pi and
shaderdb builds used the shared main target volumes instead of one per checkout, with the
sources touched before every build and the checksum compared; the checkout's own volume scheme
in `build-for-pi.sh` and `shaderdb.sh` is unchanged.
