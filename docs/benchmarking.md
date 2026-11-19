# Benchmarking and visual checks

Everything here is an `#[ignore]`d test (it needs a GPU, writes files or takes minutes) or a script in
`deploy/` that drives one. Run a test with
`cargo test --release <path> -- --ignored --nocapture`, or on the Pi with the test binary from
`deploy/build-for-pi.sh --tests`: `./spotify-round-tests <path> --ignored --nocapture`.

Plain `cargo test` needs no GPU: it validates every shader with naga and checks that each uniform
block's Rust struct matches the shader's layout (`src/shader_check.rs`).

Results and the decisions they led to are in [perf/](perf/): [perf-bottlenecks.md](perf/perf-bottlenecks.md)
and [pipeline-optimization.md](perf/pipeline-optimization.md) for the Nautilus,
[solar-perf-plan.md](perf/solar-perf-plan.md) for the Solar tour, [kiosk-ui-cost.md](perf/kiosk-ui-cost.md)
for the UI and compositor, and [feature-costs.md](perf/feature-costs.md) for the per-feature table.

## What the Pi's GPU can afford

The Pi 5's VideoCore VII (V3D) is short of arithmetic: about 46 billion shader operations a second,
so a full-screen pass at 1080 × 1080 costs roughly `instructions per pixel × 0.025 ms`, and a shader
too large for its registers runs at half the thread count and pays more besides. Passes, blending
and buffer uploads are cheap by comparison. The Pi has no timer queries, so GPU time there is read
from the V3D's own counters, and on a desktop from wgpu-profiler (`SPOTIFY_ROUND_PROFILE=1`).

The display runs on Vulkan by default; the tests and benchmarks run headless on the Pi's OpenGL ES
driver, which is faster there because it has early-Z (see [architecture.md](architecture.md)).
`WGPU_BACKEND=vulkan` or `gl` picks one.

## Which tool for which question

| Question | Use |
| --- | --- |
| Did my shader change make anything slower on the Pi? | `deploy/v3d-shaderdb/check.sh` (no Pi needed) |
| How long will a solar frame take on the Pi? | `deploy/pi-estimate.sh` (no Pi needed) |
| How long does a frame really take on the Pi? | `deploy/live-pi.sh` (as the kiosk) |
| Is the Nautilus slower than before, anywhere? | `deploy/bench-pi.sh run` / `compare` (`hotspots`) |
| What does each Nautilus feature cost? | `deploy/bench-pi.sh features` (`feature_costs::features`) |
| Where does a solar jump's time go? | `solar::tests::jump_costs` |
| What did my change do to how it looks? | `deploy/shots.sh run` / `compare`, the solar screenshot tests |

## Before committing a shader change

**`deploy/v3d-shaderdb/check.sh`** compiles every pipeline of the globe, the dive and the Solar tour
with the Pi 5's own compiler (the Pi OS release of Mesa in Docker, with its V3D noop `drm-shim`), and
fails if any shader runs at fewer threads than in `deploy/v3d-shaderdb/baseline.txt` or starts
spilling. That is the cliff a change can fall off with no visible difference in the code.
`check.sh . --update` accepts a new baseline. `shaderdb.sh <checkout>` prints each shader's
instructions, threads and spills by pipeline label and entry point; `compare.py a.txt b.txt` shows
what changed between two checkouts. Its numbers match the Pi's own `V3D_DEBUG=shaderdb` shader for
shader.

The check gets its pipelines by running `globe::tests::the_frame_is_round`,
`dive3d::tests::previews` (one frame of every sea) and `solar::tests::frame_time` (every key
moment) under the shim, so those tests must keep drawing every pipeline the app uses.

## Estimating the Pi without a Pi

**`deploy/pi-estimate.sh`** runs `solar::tests::pi_estimate`, which counts every draw's fragments at
each key moment of the solar tour with pipeline statistics queries (Mesa's lavapipe has them), and
costs them with the shader-db baseline: milliseconds a frame per moment, the heaviest passes, and what
streaming maps adds. It prints a floor beside each estimate: on the V3D a `discard` does not stop the
shader, but work inside an `if` is skipped for 16-pixel groups that all leave it. `--ms-per-inst`
(0.025, from `gpu_cost` on the Pi) sets the scale; one real measurement calibrates it.

## Measuring on the Pi

| Test or script | Use it for |
| --- | --- |
| `gpu_cost::videocore_vii` | Asserting the headless adapter really is the Pi 5's V3D. Run it before trusting any Pi measurement. |
| `gpu_cost::gpu_cost` | The GPU's basic costs (per instruction, pass, blend, upload). Re-run after a Mesa upgrade to recalibrate `--ms-per-inst`. |
| `gpu_cost::adapter_report` | What the adapter offers: backend, features, limits, texture formats. Run once per backend after a driver or wgpu upgrade. |
| `v3d_perf::tests::v3d_perf_counters` | Lists the V3D's performance counters, when choosing which to read. |
| `deploy/live-pi.sh` | The real frame time and GPU queue time, running the build *as* the kiosk (a second client on top of cage would add compositing the kiosk never pays). |
| `deploy/bench-pi.sh run TAG` / `compare A B` | The Nautilus regression bench: `dive3d::tests::hotspots` at several seeds and places in every sea, CSV per run, two runs side by side. |
| `deploy/bench-pi.sh features TAG` | `dive3d::feature_costs::features`: every pass, model group, sprite and scenery element priced where it is in view; the table goes into [feature-costs.md](perf/feature-costs.md). |

## Timing on any machine

Desktop numbers are only good relative to another run on the same machine.

| Test | Use it for |
| --- | --- |
| `globe::tests::frame_time` | The globe: map decoding, then a run of frames. |
| `dive3d::tests::frame_time` | The Nautilus, a line per sea. Quick before/after check. `DIVE3D_SKIP`, `DIVE3D_SKIP_EFFECTS` and `DIVE3D_SKIP_ELEMENTS` leave parts out without a rebuild. |
| `dive3d::tests::hotspots` | The same at chosen seeds and places in each leg (`DIVE3D_HOTSPOT_*`), for regressions a leg average hides. |
| `dive3d::tests::effect_cost` | Every pass, probe and lighting path, timed in pairs against the full frame so drift cancels, with a noise-floor row. For alternative shader paths; `features` is for what the viewer sees. |
| `dive3d::tests::fixed_cost` | The CPU side of an empty frame: scene build, tiles, staging, submission. |
| `dive3d::tests::cpu_cost` | CPU work only (caustics, a visit's seabed, the scene build), no GPU. |
| `dive3d::tests::coverage` | How many pixels each part of the frame shades. |
| `solar::tests::frame_time` | Solar: streaming the first planet, then every key moment of seed `SWEEP_SEED` (default 5). With `SWEEP_STEP=<s>` it sweeps the whole visit instead (`SWEEP_FROM`, `SWEEP_TO`, `SWEEP_DRIVE`), printing each frame's phase and draws. |
| `solar::tests::jump_costs` | Each draw of a jump, encounter by encounter (`SOLAR_ENCOUNTERS`), with each part left out in turn. |
| `lens::tests::frame_time` | The observatory and the microscope, a frame every two seconds through several targets and slides. |
| `solar::stream::tests::gallery` | One world of each kind painted alone, with the frames and time each map pass took. |

`SPOTIFY_ROUND_SCENE=globe|nautilus|solar|observatory|microscope` with `SPOTIFY_ROUND_FULL_SPEED=1` runs the app on one
scene as fast as it goes and logs the frame rate; `DIVE3D_STATS=1` logs the dive's draw counts.

## Screenshots and reference sheets

| Test or script | Writes |
| --- | --- |
| `deploy/shots.sh run NAME` / `compare A B` | Every place of `dive3d::places::all` (`dive3d::tests::screenshots`), and an HTML page of before, after and the difference, most-changed first. `shots.sh pi` renders them on the Pi's driver. |
| `dive3d::tests::scenes_sheet` | `docs/dive3d-scenes.png` and `.html` (`DIVE3D_SHEET=docs/dive3d-scenes.png`). |
| `dive3d::tests::quality_sheet` | `docs/dive3d-lighting.png`, the same scenes under each lighting path. |
| `dive3d::tests::previews` | One frame of every sea to `DIVE3D_PREVIEW_DIR`; also what shader-db compiles from. |
| `dive3d::helm::survey::survey` | A log of the helm's voyages (`HELM_LOG_DIR`), for checking clearances. |
| `globe::tests::previews` | Globe frames to `GLOBE_PREVIEW_DIR`. |
| `observatory::tests::previews`, `microscope::tests::previews` | Every target and every slide (at both objectives), a cloudy night and each kind of passer, to `LENS_PREVIEW_DIR`. Sources of `docs/observatory.jpg` and `docs/microscope.jpg`. |
| `visuals::tests::picker_thumbnails` | The screensaver picker's round windows, a frame of each scene in its bezel, into `ui/images/scenes/`. Run it again when a scene's look changes. |
| `solar::tests::screenshots` | Every key moment of seeds `SOLAR_SHOTS_SEEDS` (default 0,1,2,5) with a contact sheet and the HUD in `hud.txt`; `SOLAR_SHOTS=low` filters by name. |
| `solar::tests::{warps, spools, drop_outs, relay_crossing}` | Each drive jumping, starting up and dropping out; a jump through a relay. Source of `docs/solar-warps.jpg`. |
| `solar::tests::{encounters, battles, fleets, hive_cubes, asteroid_fields, fluxes}` | Each encounter between the stars, a row each. Sources of the `docs/solar-*.jpg` sheets. |
| `solar::tests::planet_gallery` | Every kind of world, close up and from orbit. |
| `ui::preview::render_design_previews`, `render_console_previews` | The real UI and the ship's terminal through the software renderer, into `target/preview/`. |

Solar shots go to `SOLAR_SHOTS_DIR` (default `target/solar-shots`). Before-and-after comparisons
belong in a commit message or a PR, not in `docs/`.

## Shader experiments

`shader_lab::tests::dump` writes the linked shader sources to `SHADER_DUMP_DIR` as starting points;
`shader_lab::tests::compile` compiles the variants in `SHADER_LAB_DIR` so shader-db can cost them.
[perf/solar-lab/](perf/solar-lab/) holds two such variants and `run-lab.sh`.

## Merged or removed

- `dive3d::tests::frame_cost` is gone: `effect_cost` times the same variants, paired against the
  full frame so drift cancels. Its half-float sea comparison settled the sea's format long ago.
- `solar::tests::frame_sweep` is now `solar::tests::frame_time` with `SWEEP_STEP` set.
