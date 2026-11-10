# The Nautilus on the Pi 5: where the frame goes, and what to do about it

Status: **complete** (2026-10-31). Every finding below is measured on the Pi unless it says
otherwise; estimates are marked as such, with how they were made.

Measured at `e5af5ea` (main) on the Pi 5 (V3D 7.1.7 at 960 MHz, 4 slices × 4 QPUs, Mesa
26.2.2-1~bpo13+0~rpt1, kiosk frozen by the bench scripts), `hotspots` over four visits × fourteen
legs × three places (168 places), six frames a place. In this session the baseline reads
**median 9.46 ms, worst 12.61 ms, 0 places over 14 ms, 25 over 11 ms**; render engine median
7.33 ms, bin engine median 0.87 ms. (Absolute numbers drift between sessions by ±0.5 ms; every
comparison below is within one session.)

`hotspots` times a frame serially: CPU build and staging, submission, binning, rendering and the
wait. In the kiosk the CPU and the binning of one frame overlap the rendering of the one before,
and the UI's own frame and its presentation share the render engine (§3.8), so the number that
limits the kiosk is **the render engine's time per frame plus ~2 ms of UI work**.

## 1. Summary (ranked)

| # | Bottleneck | Measured cost | Where | Fix (brief) |
|---|---|---|---|---|
| 1 | **No early-Z at all on the Pi 5**: Mesa 26.2.2's v3dv never enables the V3D 7.1 early-Z unit, and the corner mask's `Always` depth test would disable it anyway | **0.79 ms median, 0.80 ms worst** (measured with both fixed), 0.7–1.1 ms in every sea; 25 → 4 places over 11 ms | every frame, render engine (rasteriser/TLB throughput) | mask `GreaterEqual` (T1) + v3dv patch (T2); §3.1 |
| 2 | **The cave's tunnel**: `fs_model` for the vault, 187 instructions at 2 threads, over the whole window | **4.4–5.5 ms at the seven slowest tunnel views**, 2.2 ms on the cave's median place; all of it the fragment shader | cave 0.15; brine, forest, shallows 0.85 | `fs_vault` at 4 threads (T3); §3.2 |
| 3 | **The city's buildings**: `fs_model` for `CITY_LIGHTS`, 650–667 instructions at 2 threads, for every part of every building | **5.1 ms at city s0 0.45** (the slowest place once early-Z is on), 2.3 ms at arctic s750000 0.15, 1.0 ms on the city's median | city, and legs that look at it | split by part (T5); §3.3 |
| 4 | **The UI's present path**, outside `hotspots`: three render jobs and a 1.4 ms TFU conversion a frame after the visual's own; very likely not cage, which appears to scan the app's buffers out | **≈2.0 ms render engine + ≈1.4 ms TFU a displayed frame** | every kiosk frame, every scene | attribute, then drop the copy (T9); §3.8 |
| 5 | **Open water above the horizon**: the caustic ceiling (seven `sin`s, a `pow`, a fetch per pixel) and the glint | **0.84–1.01 ms median**, 1.0–1.4 ms in six seas; the ceiling ~0.7 ms of it, the glint 0.10 | every sea, most near the surface | cheaper ceiling (T6); §3.4 |
| 6 | **The vents' plumes**: 8–12 stacked additive `GLOW` discs per chimney | **up to 3.75 ms**; 2.5–3.4 ms at vents 0.15/0.45, 3.2 ms at abyss s500000 0.85 | vents; legs that look into them | one column per plume (T4); §3.6 |
| 7 | **Glass**: `fs_glass` 506 instructions at 2 threads; its `vs_model` compiled for any animation, 1078/640 instructions | **1.5 ms on the bloom's median**, up to 3.7 ms; half of it the fragment shader | bloom, abyss, wrecks, arctic | specialise by material and animation (T7); §3.7 |
| 8 | **Seabed**: mostly quads hidden behind the tunnel and the models | 1.58 ms median, up to 3.4 ms; early-Z removes 1.4–1.6 ms of the worst | every sea | T1/T2, then re-measure; §3.5 |
| 9 | **Start-up**: the mesh library rebuilt on one core at every start; 161 pipelines compiled eagerly | **4.65 s of the 5.9 s warm start**; ~9.5 s of compiling in the 15.4 s cold start | boot, first dive | parallel library, fewer pipelines (T8); §3.10 |
| 10 | **Frame pacing**: a 60.00 Hz CPU timer against a 60.07 Hz panel | a repeated frame every ~14 s | every kiosk frame | pace from the display (T10); §3.9 |
| — | CPU | 0.76 ms median, 1.58 ms max a frame, overlapped with the GPU in the kiosk | — | none needed; §3.9 |

**Where this leaves the targets.** No place is over 14 ms today. With T1 + T2 (measured) the median
is 8.59 ms, the worst 11.80 ms and four places are over 11 ms: city s0 0.45 (11.79), arctic s750000
0.15 (11.43), cave s250000 0.15 (11.20) and vents s250000 0.15 (11.03). Each of the four has its own
brief: the city (T5, ~2–3 ms there; it is also 2.3 ms of the arctic place, which looks back at the
city, and T6 takes ~0.5–0.9 ms of its 2.4 ms of water), the tunnel (T3, ~2–3 ms) and the plumes (T4,
~1.5–2.5 ms). Those estimates put every place under ~10 ms serial. In the kiosk the render engine is
the limit: 10.1 ms at the worst place after T1/T2 plus ~2 ms of UI is ~12.1 ms of a 16.7 ms refresh,
and T9 is the brief that shrinks the 2 ms. The CPU is not a hidden limiter (0.8–1.6 ms a frame,
overlapped).

## 2. Method and raw evidence

### Tools

- Pi timing: `find src -type f -exec touch {} +` and `deploy/build-for-pi.sh --tests` on the Mac,
  then `PI=<user>@<pi> deploy/bench-pi.sh run TAG [NAME=VALUE…]` and
  `deploy/bench-pi.sh compare A B`. The run now holds `/tmp/nautilus-bench.lock` on the Pi, so runs
  from several checkouts queue instead of overlapping (added with this analysis).
- Pass attribution: `DIVE3D_SKIP=<bits>` (the `part` bits in `src/dive3d/mod.rs`) leaves a pass
  out; the saving is the pass's cost. `DIVE3D_SKIP_EFFECTS=<bits>` leaves out one model group or
  sprite kind (`effect` bits).
- Hardware counters: `DIVE3D_COUNTERS=a,b,…` (≤ 32 of the 93) in `hotspots`, read from a global
  kernel perfmon (`src/v3d_perf.rs`).
- Engine accounting: `/sys/class/drm/card*/device/gpu_stats` (per queue: jobs, busy ns), read
  before and after a run, or twice over ten seconds of the live kiosk (read-only).
- Shader statistics without the Pi: `deploy/v3d-shaderdb/shaderdb.sh <checkout>`; the gate is
  `deploy/v3d-shaderdb/check.sh <checkout>`.
- Mesa source: the `v3d-mesa-src` Docker image built by `deploy/v3d-shaderdb` holds the exact
  Mesa 26.2.2 tree (`/mesa`); `docker run --rm v3d-mesa-src sh -c '…'` reads it.
- Display state, read-only: `kmsprint` on the Pi (planes, framebuffer ids, mode), and
  `DRM_IOCTL_MODE_GETFB2` on a framebuffer id for its format, pitch and modifier (no DRM master
  needed; a short Python `fcntl.ioctl` with the system `python3`).

### Baseline counters (168 places, per frame, stock driver)

| counter | median | max |
|---|---:|---:|
| `FEP-valid-quads` | 2 279 704 | 3 662 703 |
| `FEP-clipped-quads` (early-Z rejections) | **0** | **0** |
| `TLB-quads-not-passing-z-and-stencil-test` | 1 700 144 | 3 160 307 |
| `TLB-quads-passing-z-and-stencil-test` | 503 087 | 1 093 327 |
| `TLB-memory-writes` | 18 225 | 18 225 |
| `PTB-primitives-binned` | 104 387 | 321 076 |
| `QPU-total-active-clk-cycles-fragment-shading` | 16.05 M | 29.48 M |
| `QPU-active` (instructions executed) | 20.09 M | 33.29 M |
| `QPU-stalls` (all causes) | 2.14 M | — |
| `TMU-total-text-quads-access` | 8.33 M | 18.71 M |
| `L2T-total-cache-miss` | 23 700 | — |
| `core-memory-reads` | 40 100 | — |

`TLB-memory-writes` is exactly 1080 × 1080 × 4 B / 256 B: the colour target is stored once and the
depth buffer is not stored at all (its `StoreOp::Discard` works), so there is no wasted tile
traffic on the store side.

What the counters say about the kind of bound: the QPUs execute ~20 M instructions a frame, which
is ~1.3 M cycles per QPU, or **~1.4 ms of the 8.2 ms the GPU is busy**; stalls (TMU, uniforms,
VPM, TLB) are ~10 % of that, the L2 texture cache misses and core memory reads are small. The
GPU is not ALU-bound, not bandwidth-bound and not TMU-bound in aggregate. What scales the time is
**how many quads each pass puts through the rasteriser and the TLB, and how long each fragment
thread lives**: a 2-thread shader holds half the threads in flight to cover its varyings and
texture latency, and the frame's cost follows instruction counts and thread counts far more than
the ALU total suggests. The skip sweep below is therefore the primary evidence, the counters only
explain it.

### Pass attribution (skip sweep, stock driver, same session)

Each row is one `hotspots` run (168 places) with that pass left out; the saving is base − skipped.

| pass left out | median saved | mean | max | bin median | render median | mean over the 10 slowest |
|---|---:|---:|---:|---:|---:|---:|
| whole sea pass (`SEA`) | 8.67 | 8.72 | 12.15 | 0.86 | 7.33 | 11.53 |
| models, tunnel included (`MODELS`) | 1.78 | 2.06 | 5.98 | 0.42 | 1.20 | 4.72 |
| — tunnel only (`TUNNEL`) | 0.00 (cave 2.20) | 0.31 | 5.49 | 0.00 | 0.00 | 2.55 |
| open water (`BACKGROUND`) | 1.74 | 1.65 | 3.03 | 0.02 | 1.72 | 1.09 |
| seabed (`TERRAIN`) | 1.58 | 1.59 | 3.43 | 0.20 | 1.35 | 2.67 |
| sprites (`SPRITES`) | 0.43 | 0.68 | 4.10 | 0.09 | 0.31 | 0.93 |
| glass (`GLASS`) | 0.04 (bloom 1.51) | 0.41 | 3.71 | 0.01 | 0.01 | 0.36 |
| sun shafts and beams (`SHAFTS`) | 0.16 | 0.21 | 1.39 | 0.01 | 0.14 | 0.09 |
| fading models (`FADING`) | 0.01 (arctic 0.90) | 0.13 | 1.23 | 0.00 | 0.00 | 0.10 |
| far coral cards (`IMPOSTORS`) | 0.00 (coral 0.67) | 0.09 | 1.67 | 0.00 | 0.00 | 0.24 |
| corner mask (`MASK`) | **−0.58** | −0.61 | −0.08 | 0.01 | −0.61 | −0.92 |
| model draws issued empty (`DRAW_CALLS`) | 1.98 | 2.13 | 5.15 | 0.47 | 1.40 | 2.50 |
| models into a quarter viewport (`QUARTER_MODELS`) | 0.80 | 1.08 | 4.44 | 0.01 | 0.78 | 3.16 |
| everything into a quarter viewport (`QUARTER`) | 4.47 | 4.66 | 7.55 | 0.09 | 4.35 | 6.69 |

How to read it:

- With the whole sea pass left out a frame still takes **0.75 ms** (median; 0.41–1.68): the CPU
  build, staging, encoding, submission and wait. The CPU is a small part of the serial frame and
  none of the kiosk's (§3.9).
- Drawing everything into a quarter of the pixels saves 4.47 ms, so **about 6 ms of the 9.46 ms
  frame is per-pixel work** (rasteriser, TLB and fragment shading), ~1.3 ms is geometry (both
  vertex passes and the binner) and ~0.8 ms is CPU.
- The models split into ~0.8 ms of pixels (`QUARTER_MODELS`) and ~1.0 ms of geometry; issuing
  the draws with no instances (`DRAW_CALLS`) saves as much as leaving the models out, so the draw
  calls themselves cost nothing measurable (as `docs/pipeline-optimization.md` §65 found).
- The corner mask already pays for itself: without it the frame is 0.58 ms slower, because the
  21.5 % of the square outside the window would be shaded. With early-Z it is cheaper still.

Median saving per sea (ms):

| pass | abyss | arctic | atlantis | bloom | brine | cave | city | coral | forest | sargasso | shallows | trench | vents | wrecks |
|---|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|---:|
| models | 1.29 | 2.24 | 2.35 | 0.69 | 1.60 | **3.55** | **2.86** | 2.63 | 2.80 | 2.44 | 1.59 | 0.77 | 1.45 | 0.99 |
| tunnel | 0.01 | 0.01 | −0.01 | 0.01 | 0.04 | **2.20** | 0.00 | −0.01 | 0.02 | −0.01 | 0.01 | 0.00 | 0.00 | 0.00 |
| water | 1.57 | 2.25 | 2.39 | 1.46 | 1.82 | 0.35 | 1.61 | 1.96 | 0.81 | 1.56 | 1.95 | 2.16 | 1.79 | 2.19 |
| seabed | 1.39 | 1.52 | 1.39 | 1.31 | 1.39 | 1.88 | 1.68 | 1.76 | 1.74 | 1.45 | 1.86 | 1.32 | 1.76 | 1.38 |
| sprites | 0.74 | 0.46 | 0.39 | 0.56 | 0.36 | 0.49 | 0.48 | 0.38 | 0.46 | 0.42 | 0.37 | 0.36 | **1.69** | 0.33 |
| glass | 0.71 | 0.57 | 0.00 | **1.51** | 0.07 | 0.05 | 0.00 | −0.01 | 0.02 | 0.04 | 0.00 | 0.16 | 0.01 | 0.72 |
| fading | 0.34 | 0.90 | 0.00 | 0.01 | 0.01 | 0.01 | 0.00 | 0.00 | 0.01 | 0.01 | 0.01 | 0.00 | 0.02 | 0.02 |
| impostors | 0.00 | 0.00 | −0.01 | 0.00 | 0.00 | 0.00 | −0.01 | 0.67 | 0.01 | −0.02 | 0.00 | −0.01 | 0.00 | 0.00 |

The ten slowest places and what each pass costs there (ms saved by leaving it out):

| place | frame | models | tunnel | seabed | water | sprites | glass | quarter |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| cave s250000 0.15 | 12.61 | 5.44 | 5.30 | 3.36 | 0.32 | 0.48 | 1.05 | 7.27 |
| city s0 0.45 | 12.43 | 5.15 | −0.02 | 1.45 | 2.07 | 0.71 | 0.00 | 7.01 |
| brine s0 0.85 | 12.32 | 4.97 | 4.45 | 3.23 | 0.37 | 0.81 | 1.06 | 6.81 |
| arctic s750000 0.15 | 12.29 | 4.20 | 0.03 | 1.84 | 2.37 | 0.56 | 1.12 | 6.57 |
| cave s0 0.15 | 12.24 | 5.98 | 5.49 | 3.43 | 0.31 | 1.12 | 0.10 | 7.55 |
| coral s0 0.15 | 12.12 | 3.90 | 0.02 | 2.44 | 1.74 | 0.38 | −0.01 | 5.38 |
| cave s750000 0.15 | 12.12 | 5.73 | 5.43 | 3.37 | 0.37 | 1.15 | 0.08 | 7.46 |
| vents s250000 0.15 | 11.91 | 3.05 | −0.01 | 1.87 | 1.37 | 2.95 | 0.01 | 6.11 |
| coral s500000 0.15 | 11.84 | 3.21 | −0.06 | 2.46 | 1.70 | 0.31 | −0.02 | 5.61 |
| brine s750000 0.85 | 11.80 | 5.56 | 4.84 | 3.26 | 0.28 | 0.79 | 0.22 | 7.10 |

(The scene name is the leg's biome at its middle; a leg's 0.85 place can already look into the
next leg's sea, which is why brine 0.85 shows the cave's tunnel.)

### The kiosk's GPU outside the dive (read-only sample of the live kiosk)

`gpu_stats` over ten seconds of the running kiosk (the globe was on screen; the scene is picked
at random per visit, so the dive could not be chosen without touching the panel):

| queue | jobs/s | busy ms/s |
|---|---:|---:|
| render | 240.5 | 733.3 |
| bin | 240.4 | 13.1 |
| tfu | 120.1 | 84.2 |
| cache_clean | 60.0 | 1.4 |

Kiosk threads: UI thread 85.6 ms/s, visualizer 29.9 ms/s; cage 15.9 ms/s. The globe alone
(`globe::tests::frame_time`, kiosk frozen) is one render job and ~10.2 ms of render engine a
frame; its uniform upload is a TFU job of ~40 µs. So the other **three render jobs a frame cost
≈2.0 ms** of render engine, and besides the globe's upload there is **one TFU job a frame of
≈1.4 ms**: the UI's present path (§3.8; cage appears to scan the app's buffers out). For reference
(`gpu_cost::gpu_cost`, kiosk frozen, 1080²): a pass that only clears and stores costs 0.40 ms, one
that loads and stores 0.77 ms, a clear plus one full-screen textured quad 0.79 ms.

A dive frame itself is one bin job, one render job, one cache clean and ~14 TFU jobs (every
`queue.write_buffer` is a TFU copy on v3dv, ~8 µs each, 0.12 ms a frame in all).

## 3. Bottlenecks

### 3.1 Early-Z never runs on the Pi 5

**Root cause, part 1 (driver).** In Mesa 26.2.2 v3dv, the per-job early-Z bookkeeping
(`cmd_buffer_update_ez_state` in `src/broadcom/vulkan/v3dvx_cmd_buffer.c`) is compiled and called
only under `#if V3D_VERSION == 42` (the Pi 4). It is the only place that sets
`job->has_ez_draws`. On V3D 7.1 that flag therefore stays false, and `set_rcl_early_z_config`
writes `early_z_disable = true` into every render pass's `TILE_RENDERING_MODE_CFG_COMMON`.
(V3D 7.1 dropped the per-draw `Early Z enable` bits from `CFG_BITS`; early-Z is configured only in
that RCL packet, so the 7.1 path must still compute the job's direction.) The Gallium GL driver
(`v3d_update_job_ez`) runs its equivalent on every version, so GL apps get early-Z on the Pi 5
and Vulkan apps do not.

**Root cause, part 2 (the app).** The corner mask (`Dive3d corners`, the first draw of the sea
pass) uses `CompareFunction::Always` with depth writes. v3dv classes `ALWAYS` as an
*incompatible* early-Z test (`v3dv_compute_ez_state` → `V3D_EZ_DISABLED`,
`incompatible_ez_test = true`) and, because it writes depth, disables early-Z for the rest of the
job. Even a fixed driver would get nothing from this pass. `GreaterEqual` gives the same pixels
(depth is cleared to 0.0 and the mask writes 1.0, so it passes everywhere) and is a GT/GE test
like every other draw in the pass.

**Evidence.** `FEP-clipped-quads` ("Early-Z/Near/Far clipped quads") is 0 at all 168 places. A
throwaway v3dv build of the same 26.2.2 source, with the 4.2 bookkeeping also run on 7.1 behind
an environment switch (`V3DV_EZ71`), loaded with `VK_ICD_FILENAMES` from `/tmp` (no system
change), gave this 2 × 2 (168 places each, same session):

| driver | mask compare | median | worst | > 11 ms | `FEP-clipped-quads` median |
|---|---|---:|---:|---:|---:|
| stock (system) | `Always` | 9.46 | 12.61 | 25 | 0 |
| self-built, switch off | `Always` | 9.38 | 12.59 | 25 | 0 |
| self-built, switch on | `Always` | 9.47 | 12.60 | 27 | 0 |
| stock (system) | `GreaterEqual` | 9.48 | 12.62 | 26 | 0 |
| **self-built, switch on** | **`GreaterEqual`** | **8.59** | **11.80** | **4** | 1 673 473 |

The self-built driver with the switch off reproduces the system driver exactly (identical counter
values, render engine within 0.01 ms), so the gain is the early-Z alone. Per sea, the median moves
by −0.72 (arctic) to −1.13 ms (coral); the render engine median 7.33 → 6.43 ms; the bin engine is
unchanged (0.86 ms). With early-Z the slowest places are city s0 0.45 (11.79), arctic s750000
0.15 (11.43), cave s250000 0.15 (11.20), vents s250000 0.15 (11.03) and coral s0 0.15 (10.91).

What early-Z changes, per frame (medians): `FEP-valid-quads` 2.28 M → 0.55 M,
`TLB-quads-not-passing-z…` 1.70 M → 0.02 M, while `QPU-total-active-clk-cycles-fragment-shading`
(16.05 M → 16.05 M), `QPU-active` and `TMU-total-text-quads-access` do not move. So V3D 7.1 already
tests depth in the TLB *before* it shades a quad: occluded quads were never shaded. What early-Z
saves is the front end's and the TLB's throughput on the ~1.7 M quads a frame that it now drops at
the rasteriser, about half a GPU cycle each. (This also corrects the old reading of the counters
as "heavy overdraw being shaded": the overdraw costs rasteriser/TLB time, not shader time.)

**Fix.** (a) Driver: compile `cmd_buffer_update_ez_state` for every version and call it on 7.1 for
its bookkeeping only (the `CFG_BITS` fields stay 4.2-only); see brief T2. Ship it to the kiosk as
a user-built `libvulkan_broadcom.so` with its own ICD file, selected by `VK_ICD_FILENAMES` in the
kiosk unit's environment only while that file exists (a wrapper, not a second ICD in the list:
two listed ICDs expose two V3D adapters), and send the same patch upstream to Mesa (and to
Raspberry Pi's `mesa` package) so a later Pi OS update makes the local build unnecessary. (b) App:
the mask's depth compare `Always → GreaterEqual` (brief T1) — free, pixel-identical, and needed
for (a) to do anything.

**Expected gain.** Measured: **−0.8…−0.9 ms on the median place** (−0.79 serial, −0.90 render
engine), −0.80 ms on the worst; the places over 11 ms go from 25 to 4.

**Risk.** Visual: none. The `previews` test (28 frames, `GreaterEqual` mask build) rendered under
the stock driver and under the patched driver with early-Z on gives **28 of 28 byte-identical
PNGs**, and the render engine over that run drops 440.5 → 373.6 ms (−15 %). Operational: a locally
built driver must be rebuilt when Pi OS upgrades Mesa; the wrapper falls back to the system driver
whenever the local file is missing, so the kiosk never loses its GPU, only the gain.

### 3.2 The cave's tunnel: one 2-thread shader over the whole window

**Evidence.** Leaving the tunnel out (`TUNNEL`) saves 2.20 ms on the cave's median place and
5.30 / 5.49 / 5.43 ms at cave 0.15 (seeds 250000 / 0 / 750000), 4.45 / 4.84 ms at brine 0.85 and
4.3 / 3.8 ms at forest s250000 0.85 and shallows s500000 0.85: the legs whose end looks into the
cave's mouth. A throwaway build in which `fs_model` returns the bare albedo when
`MATERIAL == VAULT` (same geometry, same draws) saves **5.48 / 5.06 / 5.31 ms at cave 0.15 and
4.72 / 4.47 ms at brine 0.85**, the cave's median 2.53 ms: **the tunnel's whole cost is its
fragment shader.** Counters at cave s250000 0.15 with and without the tunnel: TLB-passing quads
0.67 M → 0.38 M (the tunnel is ~0.3 M quads, the whole window), fragment-shading QPU cycles
26.7 M → 10.4 M, texture accesses 18.7 M → 8.4 M, render engine 11.08 → 6.00 ms, binning only
−0.2 ms.

**Root cause.** `Dive3d models material5 *`: `fs_model` specialised for the vault is 187
instructions at **2 threads** (36 temps; shaderdb's `V3D_DEBUG=perf` shows register allocation
failing at 4 threads), and it covers the window. Its one texture tap's address waits on a square
root and a `log2` (the explicit LOD from the distance), and every term after it needs the tap
(the slopes bend the normal that the lamp, the floor's bounce and the shine all use). ~16 M QPU
cycles are ~1 ms of issue per QPU, yet the pass takes 5 ms: with two threads a QPU has one other
thread to run while a fragment waits on its varyings and its texture, so the tunnel waits rather
than computes. (The vertex shader, 574 instructions at 2 threads, is not the cost: the tunnel has
at most 16 k vertices.)

**Fix (T3).** A fragment shader of the tunnel's own, at 4 threads, that issues its fetch first:
an `fs_vault` entry point (or the vault branch of `fs_model` pared down) reading only the
varyings it uses; the tap's uv (`(p.x + p.z, p.y) × surface.y × 1.35`) and LOD computed per
vertex and interpolated, so the fetch can go out with the first instructions; the terms that vary
smoothly over a wall (the lamp's falloff and cone, the floor bounce's `low` and its breathing,
`near`, the cracks' pulse) per vertex in one or two packed vec4s; per pixel only what needs the
tap: the bumped normal, the two N·L terms, the tone and the shine. Iterate offline with
`deploy/v3d-shaderdb/shaderdb.sh` until the pipeline reads 4 threads.

**Expected gain.** Bound: 5.1–5.5 ms at cave 0.15, 4.5–4.7 ms at brine 0.85 (the shader made
trivial). The reef's shaders lost roughly half their per-pixel cost going from 2 to 4 threads
(`reef-perf`), so **~2–3 ms at those seven places and ~1 ms on the cave's median** is the
realistic target. With early-Z (T1/T2) the cave 0.15 places are 10.6–11.2 ms; after T3 they
would be ~8–9 ms.

**Risk.** Medium. Per-vertex lamp falloff on a coarse tunnel mesh near the glass can show facets;
the LOD from the vertices can sharpen or soften the grain. Compare the cave legs' previews and
the cave places of `screenshots` (`DIVE3D_SHOTS=cave`).

### 3.3 The city: every pixel pays for the windows

**Evidence.** Leaving the models out saves 2.86 ms on the city's median place and 5.15 ms at
city s0 0.45. A throwaway build in which `fs_model` returns the bare albedo when
`MATERIAL == CITY_LIGHTS` saves **5.08 ms at city s0 0.45**, 0.98 ms on the city's median, and
also 2.30 ms at arctic s750000 0.15 and 1.58 ms at vents s250000 0.15 (legs that look back at, or
on to, the city): **the city's cost is its fragment shader.**

**Root cause.** `Dive3d models material11 *`: 650–667 instructions at **2 threads** (50–51
temps). `city_light` serves every part of every building from one shader: the part is read per
pixel from the length of the model-space normal (neon tubes, lit glass, brass, plain stone, and
the windowed walls with their grid of rooms, lifts and failing lamps). A V3D shader's thread
count is set by its largest path, so a neon tube or a plain roof costs a pixel as much as a lit
window with a room behind it.

**Fix (T5).** Split the city by part. When the library builds a city model, sort its triangles by
part kind (the mark `mesh::city` already writes) into separate index ranges, and draw each range
with a pipeline specialised by a `CITY_PART` override (neon; lit glass; brass and plain stone;
windowed wall). The first three become short shaders at 4 threads. For the windowed walls, move
what a window keeps for its life (its hash, lit or not, warmth, brightness, room style) out of the
per-pixel path: per vertex where the window grid allows it, or from one tap into a small baked
table indexed by the window's cell; and paint the room from a small baked atlas of rooms instead
of `city_room`'s arithmetic, aiming at 4 threads. The draws grow by a few dozen a frame (~0.7 µs
each, `docs/pipeline-optimization.md` §65), which is noise.

**Expected gain.** Bound 5.1 ms at city s0 0.45 and 2.3 ms at arctic s750000 0.15;
realistically **2–3 ms at the worst city view** and ~0.5 ms on the city's median. With early-Z
city s0 0.45 is 11.79 ms, the slowest place of all; after T4 it would be ~9–10 ms.

**Risk.** Medium-high: the city is the most detailed look in the dive. A baked room atlas has
less variety than the per-pixel rooms; check the city's previews and `screenshots`
(`DIVE3D_SHOTS=city`) at near and far distances.

### 3.4 Open water: the caustic ceiling above the horizon

**Evidence.** Leaving the water out saves 1.74 ms median (up to 3.03 ms: abyss, atlantis,
arctic and coral at 0.45, where most of the window is open water). The existing
`CHEAP_BACKGROUND` switch (`DIVE3D_SKIP=16384`, a uniform `debug` bit in the same pipeline: the
depth gradient only, no beam, glint, ceiling or sky) saves **0.84–1.01 ms median** (three sessions;
1.00 ms of render engine), 1.94 ms max, and takes the places over 11 ms from 26 to 14 (25 to 13 in
another session). So of the water's 1.74 ms, ~0.9 ms is
its shader's work and ~0.8 ms is the pass itself (rasterising, depth-testing and writing the
uncovered pixels, which a 3-instruction shader would still pay).

Where the shader's 0.9 ms goes, from three throwaway builds, each measured in one session
against its own baseline (168 places each):

| build | median saved | per sea (median saved) |
|---|---:|---|
| below the horizon, return `water_at` at once (what a horizon split would do) | **0.02** (render engine) | nothing anywhere |
| `water_at` (gradient, sun, searchlight beam, city glow) replaced by the bare gradient | 0.05 | within ±0.25 everywhere, as many seas slower as faster |
| the open-water glint removed (`SKY` 0 and 2 only) | 0.10 | abyss 0.37, wrecks 0.34, arctic 0.33, bloom 0.33, vents 0.15, the rest 0 |
| everything but the gradient (`CHEAP_BACKGROUND`) | 0.84 | atlantis 1.43, brine 1.41, wrecks 1.41, trench 1.33, coral 1.08, arctic 1.06, vents 0.92, shallows 0.83, sargasso 0.82, abyss 0.81, city 0.70, bloom 0.53, cave 0.48, forest 0.35 |

The pixels below the horizon already skip the expensive terms (they sit behind `up > 0` and
`dir.y > 0` branches that whole 16-pixel groups take together), and `water_at`, which every water
pixel runs, is cheap. So **the cost is above the horizon**, and what is left once the glint and `water_at` are
accounted for is **`water_ceiling`**, the underside of the surface drawn while the boat is within
six units of it (`SKY` 1): ~0.7 ms median and 1.0–1.4 ms in atlantis, brine, wrecks, trench, coral
and arctic (the trench's 1.33 ms with no glint at all is the ceiling alone). Per pixel it is one
caustic fetch, seven `sin`s (swell ×3, chop, sun, fine ×2), a `pow(…, 18)` and the wash toward the
open water; the glint (`SKY` 0) is four `sin`s and a `pow(…, 14)`.

**Root cause.** One fragment shader per sky (`SKY` override, `sky_mode` on the CPU):

| pipeline | when | instructions | threads | temps |
|---|---|---:|---:|---:|
| `Dive3d water #0` | open water | 229 | 4 | 19 |
| `Dive3d water #1` | under the ceiling | 318 | 4 | 22 |
| `Dive3d water #2` | polar sky through the leads | 321 | **2** | 19 |
| `Dive3d water #3` | Snell's window | 398 | **2** | 28 |

#0 and #1, the ones nearly every frame uses, already run at 4 threads: the ceiling costs because
it is a lot of transcendental arithmetic on every pixel above the horizon, not because of a
thread cliff.

**Why #2 and #3 run at 2 threads with 19 and 28 temps.** Not register pressure: the compiler logs
no failed allocation for them. In Mesa's `v3d_tex.c`, a texture lookup may write at most
16 / threads TMU registers (the input FIFO's 16 slots are shared by the threads), and
`while (tmu_writes > 16 / c->threads) c->threads /= 2;` lowers the whole shader's thread count
otherwise. `sky_at` samples a **cube array with an explicit LOD**
(`textureSampleLevel(sky_cubes, …, dir, layer, 0.0)`): s, t, r, the layer and the LOD are five
writes, one more than a 4-thread shader may use per lookup. The sky cubes have one mip level
(`sky_cube_texture`) and the sampler's min and mag filters are both linear, so the LOD is not
needed: `textureSample` (implicit LOD, four writes) returns the same texels.

**Measured: the obvious way to fix it is a net loss.** WGSL only allows implicit-LOD sampling in
uniform control flow, so a throwaway build moved the sky lookup out of the `dir.y > 0` branches
(under the `SKY` checks, which are pipeline constants and so uniform). Offline that gives water #2
316 instructions at **4 threads** and #3 399 at **4 threads**. On the Pi (168 places, same
session, against a baseline build): no place got faster; the 11 places that use #2 or #3 got
**0.02–0.15 ms slower** (texture accesses +0.05…+0.76 M quads a frame, fragment cycles
+0.02…+0.31 M), because every water pixel below the horizon now fetches the sky too. Overall the
median is unchanged (9.44 → 9.54 ms is session drift: the fragment cycles' median moved by 41 of
16 M). None of the 11 places is among the slow ones (6.9–10.3 ms, and arctic s750000 0.15, the
one slow place that draws the polar sky, changed by +0.02 ms), because the sky covers a small part
of their window. A 4-thread #2/#3 that keeps the lookup inside the branch (an implicit-LOD lookup
under `@diagnostic(off, derivative_uniformity)`, exact here because the cubes have one level; or
two plain `Cube` views of the array, one per leg, bound from the CPU, which take four writes even
with the LOD) would avoid the extra fetches, but the most it can win is the thread step on a small
part of a few mid-cost frames. **Low value; not briefed.**

**Fix (T6).** Make the ceiling cheaper per pixel without changing how it reads. Its terms are a
fixed pattern on the surface plane (`at`, the ray's hit on y = 0) moving with time, so they can
come from textures instead of `sin`: (a) bake the swell, chop and fine cells into the unused
channels of `caustic_map` (or a second small tileable texture) and scroll them, so the ceiling
becomes two or three fetches of the same `at` and a few multiply-adds; (b) or drop the terms that
the final wash leaves invisible, measured term by term against the previews (the survey in
`docs/pipeline-optimization.md` §14 found the old glint's `sin`s and `pow` drew nothing in the
previews; the ceiling has not been checked that way). Not recommended: a
horizon split (measured above: 0.02 ms), #2/#3 at 4 threads (measured above), a per-vertex grid
(the plane recedes to the horizon, where any grid undersamples).

**Expected gain.** Bounded by `CHEAP_BACKGROUND`: 0.84 ms median, 1.0–1.4 ms in six seas. A
ceiling at a third of its arithmetic would take ~0.4–0.5 ms off the median and 0.6–0.9 ms in those
seas.

**Risk.** Medium: the ceiling is the look of every shallow sea. A baked swell repeats where the
`sin` sum did not (pick a texture period well beyond the 0.3-unit caustic tile); compare the
shallow seas' previews and `screenshots`.

### 3.5 Seabed: mostly hidden quads, which early-Z removes

**Evidence.** Leaving the seabed out saves 1.58 ms median, bin 0.20 of it, and up to 3.4 ms at
cave 0.15 and brine/forest 0.85, exactly the places where the tunnel hides most of the seabed.
Its shaders are not the problem: `fs_terrain` is 109 instructions at 4 threads; `vs_terrain`
(404 instructions, 2 threads) runs for every tile vertex, but replacing its point-light loop by
the sun alone (`CHEAP_TERRAIN`) saves only 0.10 ms median, 0.36 max. With early-Z on (T1/T2)
the cave 0.15 places drop by 1.4–1.6 ms, the largest drops of all: the seabed behind the tunnel
walls is what early-Z throws away.

**Fix.** T1/T2 first. Then measure what the seabed still costs with early-Z (`DIVE3D_SKIP=2` on
the patched driver) before doing more; the candidates are not drawing the seabed tiles the cave's
massif encloses (the tunnel's section is known on the CPU) and a lighter `vs_terrain` (the tiles
are built on the CPU already; static light could be baked into them).

**Expected gain.** Beyond early-Z, unmeasured; at most the 1.4 ms median that remains of the
seabed's cost, and the CHEAP_TERRAIN bound says the vertex lighting is only ~0.1 ms of it.

### 3.6 Sprites: the vents' plumes

**Evidence.** Leaving all sprites out saves 0.43 ms median but up to 4.10 ms. By kind
(`DIVE3D_SKIP_EFFECTS`, 48 places of the forest, vents, abyss and wrecks legs):

| sprite kind | median saved | max | where |
|---|---:|---:|---|
| `GLOW` | 0.04 (vents 1.13) | **3.75** | vents 0.15/0.45: 2.5–3.4 ms; abyss s500000 0.85: 3.2 ms |
| `MOTE` (marine snow) | 0.26 | 0.56 | everywhere |
| `BUBBLE`, `EMBER` | 0.00 | 0.05 | — |

**Root cause.** Each vent chimney emits a plume of 8 or 12 additive `GLOW` discs
(`world::vents`, radius 0.05 → 0.21/0.28 as they rise) that overlap one another. A chimney near
the window puts 8–12 blended full-size layers over a large part of the screen; the cost is fill
(blending is cheap per layer on V3D, ~0.4 ms per full-screen layer in `gpu_cost`, but the plumes
stack). `vs_sprite` (388 instructions, 2 threads) is not the issue at ~1 k sprites.

**Fix (T4).** Draw each plume as one or two stretched quads with the rising puffs computed in the
fragment shader (the lava falls already do this through `f.shafts`), or fade a plume disc out as
its projected radius grows past ~60 px, so a near chimney is one translucent column rather than a
stack of discs. Identify the abyss's large glows the same way (`DIVE3D_SKIP_EFFECTS` on that
place).

**Expected gain.** Up to ~2.5–3 ms at the vents' 0.15/0.45 places (bounded by the 3.75 ms of
all glows), ~1 ms on the vents' median. Visual risk: medium (the plume's look changes); review
the vents' previews.

### 3.7 Glass: jellyfish drawn with the largest shaders in the dive

**Evidence.** Leaving the glass out saves 1.51 ms on the bloom's median place and up to 3.71 ms
(arctic s500000 0.85, bloom 0.15, cave s0 0.85 for the crystals, atlantis s250000 0.85). A
throwaway build in which `fs_glass` returns at once saves 0.77 ms on the bloom's median and up to
2.14 ms: **about half of the glass's cost is its fragment shader**, the other half its geometry and
its blending.

**Root cause.** One glass pipeline for every glass material and every animation: `fs_glass` is
506 instructions at 2 threads (49 temps), and its vertex shader is `vs_model` compiled for any
animation (`ANIM = -1`): **1078 instructions for the render pass and 640 for binning, both at 2
threads**, the largest vertex shaders in the dive, run for every jellyfish vertex in both passes.

**Fix (T7).** Specialise glass as the models already are: a pipeline per animation the glass uses
(still crystals; the jellies' PULSE, TRAIL and GLIDE, or one "jelly" variant if three cost too
much start-up), so a crystal's vertex shader drops to the still model's ~400 / 66 instructions and
a jelly's loses the animations it never runs; and `fs_glass` per material (CRYSTAL, JELLY,
JELLY_SOFT, COMB) through a `MATERIAL` override, each path alone, aiming at 4 threads.

**Expected gain.** Bound 0.77 ms (bloom median) to 2.1 ms (worst) for the fragment shader; part
of the remaining ~0.7 ms for the vertex shaders. Realistically 0.5–1 ms in the bloom and
1–1.5 ms at the worst glass places. Pixel-identical if the specialisation only removes dead
paths.

### 3.8 The UI's present path: ~2 ms a frame that `hotspots` never sees

> **Update 2026-11-10:** settled in `docs/kiosk-ui-cost.md`. Cage's direct scan-out is now
> confirmed (`cage -D`, and the client's `WAYLAND_DEBUG`: `XR24`, linear). The 1.4 ms TFU job is
> the Vulkan WSI's linear copy, which every client pays, `vkcube` included. The UI costs
> 1.3–2.5 ms of render engine a frame. T9 is not worth doing.

**Evidence.** §2's live sample: with the kiosk running normally the render engine does 4 jobs a
frame at 60 fps and is busy 733 ms a second; the visual's own frame was one job of ~10.2 ms (the
globe was on screen; the dive's frame is likewise one render job). The other three render jobs
cost **≈2.0 ms a frame**. Of the two TFU jobs a frame, one is the visual's own upload (~40 µs);
the other takes **≈1.4 ms** (84 ms/s in all), about what the TFU needs to convert a whole 1080²
RGBA8 image between the linear and the tiled layouts.

**Cage is very likely not the cost: it appears to scan the app's buffers out.** Read-only, without touching
the kiosk: `kmsprint` on the Pi shows one active plane on the HDMI output (no cursor plane), and
`DRM_IOCTL_MODE_GETFB2` on the framebuffers it shows (no DRM master needed; a 40-line Python
`ioctl`, run with the system `python3`) gives four framebuffers in rotation, each
**1080×1080 `ARGB8888`, linear, pitch 4352**. wlroots renders its own output buffers as
`XRGB8888` unless told otherwise, and cage does not; four ARGB buffers in rotation are therefore
the client's swapchain, and cage is scanning them out directly, with no GPU work of its own a frame.
(Inferred from the format; wlroots' debug log, `cage -D`, or the client's `WAYLAND_DEBUG` would
confirm it, both of which need the kiosk restarted.)
(The panel's mode is 1080×1080 at **60.07 Hz**, which matters for §3.9's pacing note.)

What is left is the app's own present path, after the visual's frame: Slint's frame (Skia drawing
the sea's texture full-screen plus the UI; a clear, one full-screen textured quad and the store is
0.79 ms in `gpu_cost`), something else of about one full-screen pass (a load-and-store pass is
0.77 ms in `gpu_cost`; a copy of the swapchain image into the linear buffer the display scans out
would look like this), a smaller job, and the one 1.4 ms TFU conversion. Which caller issues which
job is not established: `gpu_stats` is global, and the per-client counters in
`/proc/<pid>/fdinfo` are not readable for the kiosk's processes without root.

**Consequence.** The kiosk's budget for the dive's render engine is ~14.5 ms at 60 Hz, not
16.7 ms; at the worst place today (11.08 ms render) that leaves ~3.5 ms, and the adaptive render
scale (`SPOTIFY_ROUND_ADAPTIVE`, which steers from the render engine's busy time, UI included)
has headroom. A frame drawn at a time unrelated to the display's vsync (§3.9) still lands on the
next vblank only if all of this fits.

**Fix (T9).** Attribute the three render jobs and the TFU job to their callers (Mesa's
`V3D_DEBUG=cl` names every job and its targets; the app's surface configuration — format,
alpha mode, present mode, image count — is the first thing to log), then remove the full-screen
copy and conversion if they are what they look like: render the UI straight into the scannable
image, or keep the swapchain tiled if the display path does not need linear. The KMS overlay for
the dive itself stays blocked (`docs/pipeline-optimization.md` §71).

**Expected gain.** Unmeasured; bounded by ≈1.2 ms of render engine a frame (everything after
Slint's own 0.8 ms) plus the 1.4 ms TFU job, on every frame of every scene, the globe and the
player included.

### 3.9 CPU: not a hidden limiter

**Evidence.** `DIVE3D_PHASES=1` over the 42 places of one visit (median of six frames a place):

| phase | median ms | max ms |
|---|---:|---:|
| `world::build` | 0.616 | 1.241 |
| seabed tiles | 0.031 | 0.043 |
| order | 0.029 | 0.074 |
| sort | 0.034 | 0.139 |
| stage | 0.014 | 0.071 |
| upload (`write_buffer`s) | 0.017 | 0.034 |
| encode | 0.011 | 0.024 |
| **total** | **0.763** | **1.577** |

With the sea pass left out the whole frame (CPU, an empty submission and the wait) is 0.75 ms
median. In the kiosk the visualizer thread builds frame N+1 while the GPU renders frame N (two in
flight, `visuals::spawn`), so 0.8–1.6 ms of CPU against 7–11 ms of render engine never limits the
rate. The live kiosk's threads (globe on screen) use 86 ms/s on the UI thread and 30 ms/s on the
visualizer: a fraction of one of the four cores. The CPU governor is `ondemand` (1.7 of 2.4 GHz
when sampled); `performance` would shave the CPU part of the serial `hotspots` number but not the
kiosk's frame.

**Frame pacing (not a cost, a smoothness note; T10).** The visualizer paces itself with a CPU
timer (`visuals.rs`: `deadline += budget` at exactly `1 / TARGET_FPS`, then `pace`), not the
display's vblank, and its submission and Slint's frame both run on the UI thread. The panel's
mode is **60.07 Hz** (85.5 MHz over 1280 × 1112, `kmsprint`), so a 60.00 Hz producer falls one
frame behind the display every **~14 s**: one refresh shows the previous dive frame again, and
while the frame's completion drifts across the vblank, jitter can show a frame twice and skip the
next. Pacing from the display instead (Slint's rendering notifier, or the swapchain's present
timing) would lock the two; measured by counting repeated and skipped frames over a minute, not by
`hotspots`.

### 3.10 Start-up

**Evidence.** `Nautilus::new` plus four frames, in the test binary (`hotspots` with no legs):
**15.43 s cold** (`MESA_SHADER_CACHE_DISABLE=true`), **5.88 s warm** (Mesa's disk cache, twice).
The ~9.5 s difference is the V3D compiler: `compile()` creates 161 pipelines eagerly
(48 models + 48 sealed + 48 fading + 4 water + 2 cheap water + seabed, corners, glass, sprites,
shafts, beams and 5 impostor forms), each with a vertex shader compiled twice (render and binning)
and a fragment shader, up to six compiler strategies each.

**The warm start is the mesh library.** `mesh::Library::build()` alone (the
`dive3d::mesh::tests::closed_shapes_are_sealed_and_sheets_are_not` unit test, which does little
else) takes **4.65 s on the Pi**, single-threaded: generating the 64 models and simplifying their
LODs with meshopt, on every start. So the warm 5.9 s is ~4.6 s of mesh generation and ~1.3 s of
everything else (Vulkan, textures, naga and Mesa's cache), and the cold 15.4 s adds ~9.5 s of
shader compilation.

**Root cause.** (1) The library is regenerated from scratch on every start, on one core of four.
(2) 18 of the 161 pipelines are the `MATERIAL == -1` ("any") model pipelines, which no placer
reaches (every solid material is in `MATERIALS`; the glass-only materials go through `fs_glass`),
and the any-material fragment shaders are the largest (393–420 instructions). More of the
material × grounded × animation combinations are never drawn.

**Fix (T8).** (a) Build the models in parallel (`std::thread::scope` over `Model::ALL`, results
gathered in model order so the library is bit-identical; a unit test compares it with the serial
build), or cache the built library on disk keyed by a hash of the generator's inputs. (b) Drop the
"any" material slot (an unknown material becomes a test failure, not a fallback pipeline).
(c) Compile only the pipeline slots a voyage can use (a test that runs `world::build` over every
leg of a few visits and collects `pipeline_slot`), or compile on first use. Start-up does not
change the frame rate; it decides how soon after boot the dive can be shown, and on a cold start
the build competes for the CPU with the UI for ~15 s.

**Expected gain.** (a) ~3 s off every start on four cores (4.65 s → ~1.2–1.6 s), all of it with a
disk cache. (b) ~1 s off a cold start (18 of 161 pipelines, the largest shaders among them).
(c) more on a cold start, depending on how many slots are unused.

## 4. Task briefs for implementation agents

Ordered by value for the effort. Hand an agent §4.0 and its own brief; each brief names the part
of §3 that holds its evidence. Every number to beat is from this document's sessions on the stock
driver unless the brief says otherwise.

| brief | what | expected gain | effort | tier | parallel |
|---|---|---|---|---|---|
| T1 | corner mask `GreaterEqual` | none alone; needed by T2 | minutes | Haiku | yes |
| T2 | early-Z on V3D 7.1 in v3dv, shipped as a local driver + upstream | −0.8…−0.9 ms median, every frame | 1–2 days | Sonnet | yes (driver only) |
| T3 | the tunnel's own 4-thread fragment shader | −2…−3 ms at 7 places, −1 ms cave median | ~1 day | Sonnet | yes |
| T4 | one column per vent plume | −1.5…−3 ms at the vents' 0.15/0.45 places | ~half a day | Sonnet | yes |
| T5 | the city split by building part | −2…−3 ms at city s0 0.45, −1 ms arctic s750000 | 2–3 days | Sonnet | yes, merges with T3/T7 |
| T6 | a cheaper caustic ceiling | −0.3…−0.5 ms median, −0.6…−1 ms in six seas | ~1 day | Sonnet | yes |
| T7 | glass specialised by material and animation | −0.5…−1 ms bloom, −1…−1.5 ms worst glass | ~1 day | Sonnet | yes, merges with T3/T5 |
| T8 | start-up: parallel mesh library, no unused pipelines | warm start 5.9 → ≤ 3 s | ~half a day | Sonnet; (b) Haiku | yes |
| T9 | the UI's present path: attribute, then drop the copy | up to ~1.2 ms render + 1.4 ms TFU a frame | investigation | Sonnet | yes; needs the user |
| T10 | pace the dive from the display | no repeated frame every ~14 s | ~1 day | Sonnet | yes; needs the user to deploy |

### 4.0 Rules for every brief

1. **Branch.** Work in your own git worktree on a branch named for the brief (`perf-t3-vault`, …).
   Do not push, merge to main or deploy: the user does. The repository is public: never write the
   Pi's address or user name into tracked files, only `<user>@<pi>`.
2. **Build on the Mac only; never run cargo on the Pi.** For the Pi:
   `find src -type f -exec touch {} +` then `deploy/build-for-pi.sh --tests`, and check that
   `md5 -q target/pi/spotify-round-tests` changed since your previous build (the build can hand
   back a stale binary from another worktree otherwise).
3. **The kiosk on the Pi is the user's live display.** Benchmark only through
   `deploy/bench-pi.sh run` (it freezes the kiosk for the run and resumes it however the run ends)
   or a script that does `kill -STOP` with a `trap … kill -CONT` on every exit path. Never leave
   it stopped, never replace the deployed binary, never restart it. Agents have no `sudo`.
4. **One Pi run at a time.** The GPU and its counters are shared by everything on the Pi.
   `bench-pi.sh` holds `/tmp/nautilus-bench.lock` on the Pi for the length of a run, so runs from
   parallel agents queue; a hand-written Pi script takes the same lock first
   (`exec 9>/tmp/nautilus-bench.lock; flock 9`).
5. **Measure before and after in one session**, the default 168 places:
   `PI=<user>@<pi> deploy/bench-pi.sh run <brief>-before` (base commit's test binary), then
   `… run <brief>-after`, then `deploy/bench-pi.sh compare <brief>-before <brief>-after`. Report
   the median, the worst, the places over 11 ms and the per-sea medians (the CSVs in
   `target/bench/`). The median drifts by up to ±0.1 ms between runs: a gain under 0.1 ms needs a
   second pair. `DIVE3D_HOTSPOT_SCENES=<sea,…>` narrows a run for iteration; the acceptance run is
   the full one.
6. **Gates, all on the Mac:** `deploy/v3d-shaderdb/check.sh` (about a minute; fails if any shader
   lost threads or started spilling; a brief that changes thread counts or adds pipelines runs
   `deploy/v3d-shaderdb/check.sh . --update` once the table is what it wants, and commits the new
   `baseline.txt` with the change); `cargo test --release`; and CI's
   `cargo clippy --all-targets -- -D warnings` and `cargo fmt --all -- --check`.
7. **Pictures.** Before and after:
   `DIVE3D_PREVIEW_DIR=<dir> cargo test --release dive3d::tests::previews -- --ignored --nocapture`
   (28 frames from every leg), then `/usr/bin/python3 target/compare_frames.py <before> <after>`
   (the script is untracked in the main checkout's `target/`; copy it). A brief that claims
   pixel-identical output shows mean 0.000; a brief that changes the look also writes the
   `screenshots` of its sea (`DIVE3D_SHOTS_DIR=<dir> DIVE3D_SHOTS=<sea> cargo test --release
   dive3d::tests::screenshots -- --ignored --nocapture`) and puts the most-changed frames side by
   side for the user, who decides. Where the driver is the change (T2), render the previews on the
   Pi with the test binary instead (kiosk frozen, rules 3–4).
8. Do not re-propose what §5 rules out (f16 among them) without new evidence.

### T1. Corner mask: `GreaterEqual` instead of `Always` (Haiku)

- **Goal.** Stop the sea pass's first draw from switching early-Z off for the whole pass. Alone it
  changes nothing on the stock driver (measured 9.46 → 9.48 ms median, noise); it is what lets T2
  work (§3.1).
- **Where.** `src/dive3d/mod.rs`, the `Dive3d corners` pipeline:
  `depth_compare: Some(wgpu::CompareFunction::Always)`. Leave the `Always` in `src/gpu_cost.rs`
  alone (a measurement probe).
- **Approach.** `CompareFunction::GreaterEqual`, with a comment saying why: the depth target is
  cleared to 0.0 and the mask writes 1.0, so the test passes wherever the mask draws, exactly as
  `Always` did; but v3dv treats `ALWAYS` with depth writes as an early-Z-incompatible test and
  disables early-Z for the rest of the render pass, while `GREATER_EQUAL` matches every other draw
  in the pass.
- **Constraints.** §4.0. No shader changes: the shader gate passes unchanged. `cargo test
  --release`, clippy, fmt.
- **Acceptance.** Previews identical (mean 0.000, max 0). Pi, full run before/after: median within
  ±0.1 ms. With T2's driver: `DIVE3D_COUNTERS=FEP-clipped-quads` reads > 0 at every place.
- **Parallel:** yes. **Tier:** Haiku.

### T2. Early-Z on V3D 7.1 in v3dv, shipped locally and upstream (Sonnet)

- **Goal.** Turn on the Pi 5's early-Z for the dive's render pass. Measured with a throwaway
  driver plus T1: **median 9.46 → 8.59 ms, worst 12.61 → 11.80 ms, places over 11 ms 25 → 4**,
  render engine median 7.33 → 6.43 ms, previews byte-identical (§3.1).
- **Where.** Mesa 26.2.2, `src/broadcom/vulkan/v3dvx_cmd_buffer.c`: `cmd_buffer_update_ez_state()`
  and its only call, in `v3dX(cmd_buffer_emit_configuration_bits)()`, are compiled only
  `#if V3D_VERSION == 42`. That function is the only place that sets `job->has_ez_draws` and
  decides `job->first_ez_state`; `set_rcl_early_z_config()` reads them when it writes the render
  pass's `TILE_RENDERING_MODE_CFG_COMMON`, so on 7.1 every job gets `early_z_disable = true`.
- **Approach.**
  1. Check Mesa's main branch first. If the 7.1 bookkeeping is already fixed there, backport that
     commit unchanged and cite it.
  2. Otherwise patch (this is the measured change, minus the throwaway's environment switch):
     ```diff
     -#if V3D_VERSION == 42
      /* Updates cmd_buffer, and their job, early z state tracking. Returns false if
       * EZ must be disabled for the current draw call.
     + *
     + * V3D 7.x has no per-draw early-Z bits in CFG_BITS, but the job's direction
     + * and has_ez_draws, which set_rcl_early_z_config() needs, come from here.
       */
      static bool
      cmd_buffer_update_ez_state(struct v3dv_cmd_buffer *cmd_buffer,
     …
         return !disable_ez;
      }
     -#endif
     …
      #if V3D_VERSION == 42
            bool enable_ez = cmd_buffer_update_ez_state(cmd_buffer, pipeline);
            config.early_z_enable = enable_ez;
            config.early_z_updates_enable = config.early_z_enable &&
               cmd_buffer->state.z_updates_enable;
     +#else
     +      (void) cmd_buffer_update_ez_state(cmd_buffer, pipeline);
      #endif
     ```
     Per-shader cases (discard, depth writes) stay safe on 7.1: the shader state record's
     `turn_off_early_z_test` bit is set from the fragment shader's `disable_ez` on every version.
  3. Build on the Mac in an arm64 Debian trixie container (the `mesa` stage of
     `deploy/v3d-shaderdb/Dockerfile` is a working recipe, without its `patch-mesa.sh`, which only
     renames shaders for the shader table), from the source of the installed
     package (Raspberry Pi's `mesa 26.2.2-1~bpo13+0~rpt1`; upstream `mesa-26.2.2` reproduced the
     system driver exactly in the experiment, counter for counter), with
     `-Dvulkan-drivers=broadcom -Dgallium-drivers= -Dplatforms=wayland` and the other options of
     that recipe. The throwaway had `-Dplatforms=` (no window-system support), which is enough for
     the offscreen benchmark but not for the kiosk.
  4. Install without `sudo` and with a fallback: the `.so` and an ICD JSON with its absolute
     `library_path` under the kiosk user's home (say `~/.local/lib/nautilus-mesa/`), **not** in a
     Vulkan ICD directory (the loader would then list the system driver and this one, and wgpu
     would see two V3D adapters). Select it for the app only: at the very top of `main()`, before
     any thread or wgpu instance exists, if that JSON exists and neither `VK_DRIVER_FILES` nor
     `VK_ICD_FILENAMES` is set, set `VK_DRIVER_FILES` to it (an `unsafe` `set_var` in Rust 2024,
     sound there because nothing else runs yet), and log which driver the adapter reports.
     Removing the file returns the kiosk to the system driver at its next start. (The
     alternative, `Environment=VK_DRIVER_FILES=…` in the kiosk unit, needs the user's `sudo` and has
     no fallback if the file goes missing.) The first start on the new driver recompiles every
     shader (~15 s: Mesa's cache is keyed on the driver build).
  5. Upstream: prepare a Mesa merge request (patch, the §3.1 counters and the 2 × 2 table) and a
     note for Raspberry Pi's mesa package, for the user to send, so a later Pi OS update makes the
     local driver unnecessary. Record in `docs/` how to rebuild the local driver when Pi OS
     upgrades Mesa.
  6. Follow-up once it runs: the draw-order probes of `docs/pipeline-optimization.md` §7.4/§12
     (`DEPTH_FIRST_DRAWS`, `TERRAIN_FIRST`, nearest-first) were measured without early-Z; re-run
     them with it (`DIVE3D_SKIP=<bit>`) before ruling them out for good.
- **Constraints.** §4.0 rules 2–5 and 7; the shader gate is unaffected (no app shader changes);
  `cargo test --release`, clippy, fmt for the `main()` change.
- **Acceptance.**
  - Counters: `PI=<user>@<pi> deploy/bench-pi.sh run t2-stock DIVE3D_COUNTERS=FEP-clipped-quads,TLB-quads-not-passing-z-and-stencil-test`
    and `… run t2-ez VK_DRIVER_FILES=<abs path>/icd.json DIVE3D_COUNTERS=…` (T1 in the binary):
    `FEP-clipped-quads` 0 → ≥ 1 M median; `TLB-quads-not-passing…` 1.7 M → ≤ 0.1 M median.
  - Time, same pair: median −0.7 ms or better, worst −0.7 ms or better, places over 11 ms ≤ 6.
  - Pictures: the `previews` test on the Pi under both drivers gives byte-identical PNGs
    (`md5sum` of each; 28 of 28 in the experiment).
  - The app logs the local driver when the file exists and the system one when it is removed.
- **Parallel:** yes (driver and `main()` only). **Tier:** Sonnet.

### T3. The tunnel's own fragment shader at 4 threads (Sonnet)

- **Goal.** Cut the vault's per-pixel cost (§3.2): 4.4–5.5 ms at the seven slowest tunnel views
  (cave 0.15 at seeds 0/250000/750000, brine 0.85 at 0/750000, forest s250000 0.85, shallows
  s500000 0.85), 2.2 ms on the cave's median place. All of it is `fs_model`'s vault path,
  187 instructions at 2 threads (36 temps).
- **Where.** `src/shaders/scene.wesl`: the full-HD `fs_model` (its `MATERIAL == VAULT` uv, LOD and
  tap) and `vault_hd`; `vs_model` and the `Lit` varyings. `src/dive3d/mod.rs`: the model pipelines
  for the vault's material slot (labels `Dive3d models material5 …`), where the fragment entry
  point is chosen.
- **Approach.**
  1. A full-HD-only `fs_vault` entry point for the vault's pipelines, reading only the varyings
     it needs.
  2. Per vertex, interpolated, what varies smoothly over a wall: the tap's uv
     (`(p.x + p.z, p.y) × surface.y × 1.35`) and its LOD (`clamp(log2(distance), 0, 8)`); the lamp's
     falloff and cone (`1 / (1 + reach² × 0.12)`, `cone`); the floor bounce's `low` and breathing;
     `near`; the cracks' `pulse`. Pack them into one or two `vec4` varyings (a vault-only vertex
     output, or `Lit` fields the vault does not use).
  3. Per pixel only what needs the tap: the bumped normal, the lamp's N·L, the bounce's
     `bumped.y`, the tone and the shine (`pow(…, 16)`). Issue the `textureSampleLevel` first, with
     nothing before it but varyings, so its latency overlaps the arithmetic.
  4. Iterate offline with `deploy/v3d-shaderdb/shaderdb.sh .` until the `material5` FS rows read
     4 threads (≤ 32 temps). The vertex shader (574 / 66 instructions, ≤ 16 k vertices) is not the
     cost; leave it unless the extra outputs push it into spilling.
- **Constraints.** §4.0; gate: the vault FS goes to 4 threads, nothing else loses threads (update
  the baseline); `cargo test --release`, clippy, fmt; pictures: the cave legs.
- **Acceptance.** Pi, `bench-pi.sh run t3-before DIVE3D_HOTSPOT_SCENES=cave,brine,forest,shallows`
  and `t3-after` with the same scenes for iterating, then the full pair: each cave 0.15 place
  −2 ms or better, the brine 0.85 places −1.5 ms or better, the cave's median −0.8 ms or better, no
  sea's median worse by more than 0.1 ms. Pictures: the cave legs' previews mean ≤ 1/255; no
  facets of the per-vertex lamp near the glass (look at the closest frames); side by side for the
  user.
- **Parallel:** yes; T5 and T7 also edit `fs_model`/the pipeline table: merge one at a time.
  **Tier:** Sonnet.

### T4. One column per vent plume (Sonnet)

- **Goal.** The `GLOW` sprites cost up to 3.75 ms: 2.5–3.4 ms at the vents' 0.15 and 0.45 places
  and 3.2 ms at abyss s500000 0.85 (§3.6). Take ≥ 1.5 ms off each without losing the plume.
- **Where.** `src/dive3d/world.rs`: the chimneys' plumes, `for k in 0..8` (radius
  `0.05 + rise × 0.16`) and the vents field's `for k in 0..12` (`0.06 + rise × 0.22`);
  `vs_sprite`/`fs_sprite` in `src/shaders/scene.wesl` if a sprite mode is added. First confirm
  that abyss s500000 0.85's glows are plumes too: `DIVE3D_HOTSPOT_SCENES=abyss
  DIVE3D_HOTSPOT_SEEDS=500000` with and without `DIVE3D_SKIP_EFFECTS=8589934592` (glows off).
- **Approach,** cheapest first:
  (a) Fade a disc out by its size on screen: in `vs_sprite`, scale a `GLOW` disc's tint by a
  smooth step of its projected radius (full below ~60 px, gone above ~150 px) and collapse it
  to a zero-area quad when gone, so a chimney close to the window keeps its small, far discs and
  drops the big overlapping ones.
  (b) Replace each plume by one or two camera-facing stretched quads (a column from the mouth to
  the plume's top) whose rising puffs are a scrolling pattern in the fragment shader, the way the
  lava falls already do in `fs_shaft`: one blended layer instead of 8–12.
- **Constraints.** §4.0; gate; tests; pictures: the vents' previews and `screenshots`
  (`DIVE3D_SHOTS=vents`), side by side for the user (the plume's look changes).
- **Acceptance.** Pi, `bench-pi.sh run t4-before DIVE3D_HOTSPOT_SCENES=vents,abyss` / `t4-after`:
  vents 0.15 and 0.45 places −1.5 ms or better each, abyss s500000 0.85 −1.5 ms or better; with
  `DIVE3D_SKIP_EFFECTS=8589934592` at those places the remaining glow cost is ≤ 1 ms. Full pair: no
  sea worse.
- **Parallel:** yes. **Tier:** Sonnet.

### T5. The city split by building part (Sonnet)

- **Goal.** `fs_model` for `CITY_LIGHTS` (650–667 instructions, 2 threads, 50–51 temps) costs
  5.1 ms at city s0 0.45, the slowest place once T1/T2 are in (11.79 ms), 2.3 ms at arctic s750000
  0.15 and 1.0 ms on the city's median (§3.3). Take 2–3 ms off city s0 0.45 and ≥ 1 ms off the
  arctic place.
- **Where.** `src/dive3d/mesh.rs` (the city's generator, which marks each triangle's part as the
  length of its model-space normal: `kind = length(local_normal) − 1` in `city_light`);
  `src/dive3d/world.rs` `city()`; `src/dive3d/mod.rs` (`material_slot`, `pipeline_slot`, the
  model draw loop and its index ranges); `src/shaders/scene.wesl` `city_light`, `city_room` and the
  `CITY_LIGHTS` branch of the full-HD `fs_model`.
- **Approach.**
  1. When the library builds a city model (every LOD), sort its triangles by part into contiguous
     index ranges and keep the ranges.
  2. Draw each range with a pipeline specialised by a new `CITY_PART` override: neon
     (`kind > 2.3`), lit glass, brass and plain stone, windowed walls (read `city_light`'s branches
     for the exact split). The first three become short shaders at 4 threads.
  3. Windowed walls: take what a window keeps for its life (its hash, lit or not, warmth,
     brightness, room style) out of the per-pixel path, per vertex where the window grid allows or
     from one fetch of a small baked table indexed by the window's cell; replace `city_room`'s
     arithmetic by a small baked atlas of rooms if it is what holds the registers. Aim at 4
     threads; if out of reach, measure fewer instructions at 2.
  4. A few dozen more draws a frame cost ~0.7 µs each: noise.
  Consider two branches: steps 1–2 first (pixel-identical), then step 3.
- **Constraints.** §4.0; gate (update the baseline); tests; pictures: identical for neon, glass and
  stone; the walls side by side at near and far distances (`DIVE3D_SHOTS=city`) for the user.
- **Acceptance.** shaderdb: neon/glass/stone city FS at 4 threads; walls at 4 threads or ≥ 30 %
  fewer instructions. Pi, `bench-pi.sh run t5-before DIVE3D_HOTSPOT_SCENES=city,arctic,vents` /
  after, then the full pair: city s0 0.45 −2 ms or better, arctic s750000 0.15 −1 ms or better,
  city median −0.4 ms or better, no sea worse by more than 0.1 ms.
- **Parallel:** yes; shares `fs_model` and the pipeline table with T3 and T7. **Tier:** Sonnet
  (the largest brief).

### T6. A cheaper caustic ceiling (Sonnet)

- **Goal.** The water's shader work above the horizon costs 0.84–1.01 ms median and 1.0–1.4 ms
  in atlantis, brine, wrecks, trench, coral and arctic, nearly all of it `water_ceiling` (§3.4).
  Take ≥ 0.3 ms off the median and ≥ 0.5 ms off those six seas without a visible change.
- **Where.** `src/shaders/scene.wesl`: `water_ceiling`, the glint block of the full-HD
  `fs_background`; the `caustic_map` texture's creation (`src/dive3d/mod.rs`, `textures.rs`).
- **Approach.**
  1. Weigh each term: throwaway builds dropping, one at a time, the swell (three `sin`s), the chop
     and its `pow(…, 18)` sheen, the `sun` term, the fine cells (two `sin`s); compare previews. A
     term whose removal gives mean < 0.05/255 and max < 4 on every frame that shows the ceiling
     draws nothing: drop it.
  2. For what remains, replace `sin` sums with fetches: the swell, chop and fine cells are
     periodic patterns of `at` (the ray's hit on the surface) with time as a phase, so bake them
     into a small tileable texture (or the spare channels of `caustic_map`, which uses `.r` only)
     and scroll it with `t`: the ceiling becomes two or three fetches of the same `at` and a few
     multiply-adds. The same for the glint's `wave` if step 1 keeps it.
  3. Keep water #0 and #1 at 4 threads (2D fetches use three TMU writes, within the limit).
  Not this: a horizon split (measured 0.02 ms), #2/#3 at 4 threads (measured slower), a
  per-vertex grid (undersamples toward the horizon).
- **Constraints.** §4.0; gate; tests; pictures: the shallow seas.
- **Acceptance.** Bound: `DIVE3D_SKIP=16384` (`CHEAP_BACKGROUND`). Pi full pair: median −0.3 ms or
  better; atlantis, brine, wrecks, trench, coral and arctic medians −0.5 ms or better; no sea worse
  by more than 0.1 ms. Pictures: previews mean ≤ 0.5/255 on every frame, max ≤ 16; the
  shallows and atlantis frames side by side for the user.
- **Parallel:** yes (the water only). **Tier:** Sonnet.

### T7. Glass specialised by material and animation (Sonnet)

- **Goal.** Glass costs 1.5 ms on the bloom's median place and up to 3.7 ms (arctic s500000 0.85,
  bloom 0.15, cave s0 0.85, atlantis s250000 0.85); about half is `fs_glass` (§3.7). Take
  ≥ 0.5 ms off the bloom's median and ≥ 1 ms off the worst glass place.
- **Where.** `src/dive3d/mod.rs`: the `Dive3d glass` pipeline (its `vs_model` built with
  `ANIM = -1`) and the glass draw list; `src/shaders/scene.wesl`: the full-HD `fs_glass`,
  `vs_model`'s `ANIM` switch; `src/dive3d/world.rs` for which glass models animate how.
- **Approach.**
  1. Vertex: a glass pipeline per animation the glass uses, as the solid models have since
     `675c234` (crystals and comb jellies still or swaying; the jellies' own animations), or one
     "jelly" variant if three cost too much start-up. The any-animation `vs_model` is 1078
     instructions (640 binning) at 2 threads; a still one is ~400 / 66.
  2. Fragment: `fs_glass` per glass material (`CRYSTAL` 3, `JELLY` 7, `JELLY_SOFT` 9, `COMB` 10)
     through a `MATERIAL` override, each path alone, aiming at 4 threads (49 temps today).
  3. Glass is blended: keep its back-to-front order and switch pipelines between draws as needed
     (~0.7 µs a draw).
- **Constraints.** §4.0; gate (update the baseline); tests; pictures: identical if only dead paths
  go (mean 0.000).
- **Acceptance.** Pi, `bench-pi.sh run t7-before DIVE3D_HOTSPOT_SCENES=bloom,abyss,wrecks,arctic,atlantis,cave`
  / after, then the full pair: bloom median −0.5 ms or better, the worst glass place −1 ms or
  better, no sea worse by more than 0.1 ms.
- **Parallel:** yes; shares files with T3 and T5. **Tier:** Sonnet.

### T8. Start-up: the mesh library in parallel, no unused pipelines (Sonnet; (b) Haiku)

- **Goal.** Warm start 5.9 s → ≤ 3 s (4.65 s of it is `mesh::Library::build` on one core); cold
  start −1 s or more (§3.10).
- **Where.** `src/dive3d/mesh.rs` `Library::build` (and its caller in `Nautilus::new`);
  `src/dive3d/mod.rs` `compile()`, `MATERIALS`, `material_slot`, `pipeline_slot`.
- **Approach.**
  (a) Build each model's mesh and LODs on its own thread (`std::thread::scope`, at most three at
  once so the UI thread keeps a core), collecting results in model order; a unit test asserts the
  parallel library equals the serial one byte for byte. Optional: a disk cache of the built
  library keyed by a hash of the generator's inputs.
  (b) Drop the "any" material slot (`MATERIAL == -1`, 18 pipelines that no placer reaches): an
  unknown material becomes a test failure (a test that builds every leg of a few visits and
  asserts each model's material has its own slot), not a fallback pipeline.
  (c) Optional: collect the `pipeline_slot`s a voyage actually uses (the same test) and compile
  only those, or compile on first use.
- **Constraints.** §4.0; gate (removed pipelines show as "gone"; nothing loses threads); tests;
  pictures identical (mean 0.000).
- **Acceptance.** On the Pi, with the test binary that `bench-pi.sh run <tag>` copied to
  `/tmp/srt-<tag>`, the kiosk frozen and the lock held:
  `ssh <user>@<pi> 'exec 9>/tmp/nautilus-bench.lock; flock 9; mkdir -p /tmp/hot-start; P=$(pgrep -x spotify-round); trap "kill -CONT $P" EXIT; kill -STOP $P; cd /tmp; time env DIVE3D_HOTSPOT_SEEDS=0 DIVE3D_HOTSPOT_SCENES=none DIVE3D_HOTSPOT_DIR=/tmp/hot-start /tmp/srt-<tag> dive3d::tests::hotspots --ignored'`
  times the start-up (`Nautilus::new` and four frames; add `MESA_SHADER_CACHE_DISABLE=true` after
  `env` for a cold start), and the same wrapper around
  `/tmp/srt-<tag> dive3d::mesh::tests::closed_shapes_are_sealed_and_sheets_are_not` times the
  library alone. Targets: library 4.65 s → ≤ 1.8 s; warm start ≤ 3 s; cold start −1 s or more with
  (b).
- **Parallel:** yes. **Tier:** Sonnet for (a) and (c), Haiku for (b).

### T9. The UI's present path: attribute ~2 ms a frame, then drop the copy (Sonnet; needs the user)

- **Goal.** After the visual's own frame, three render jobs (≈2.0 ms) and one TFU job (≈1.4 ms)
  a frame go to the UI and its presentation, in every scene (§3.8). Find out which is what, and
  remove any full-screen copy or layout conversion among them: up to ≈1.2 ms of render engine and
  the 1.4 ms TFU job a frame.
- **Known.** The display scans out the app's own swapchain buffers (four 1080² `ARGB8888` linear
  framebuffers in rotation, read with `kmsprint` and `DRM_IOCTL_MODE_GETFB2`; §3.8), so cage very
  likely does no GPU work a frame (confirm with `cage -D` when the kiosk is restarted anyway). Slint draws with Skia on the shared wgpu device, and the visual's texture reaches
  it as a `wgpu::Texture` (`ui.rs`, `set_idle_frames`). A rendering notifier makes Slint add a
  clear pass of its own (`ui.rs`, `watch_rendering_if_asked`), so do not add one to measure.
  `SPOTIFY_ROUND_SCENE=nautilus` fixes the dive on screen; `SPOTIFY_ROUND_CONFIG_DIR` points a
  second copy of the app at its own settings and tokens.
- **Approach.**
  1. Log once at start the surface configuration Slint chose: format, alpha mode, present mode,
     swapchain image count, adapter and driver.
  2. Attribute the jobs with Mesa's `V3D_DEBUG=cl` (every job with its targets: a full-screen
     render job that loads a tiled image and stores raster is a copy; a 1080² TFU job is a layout
     conversion, and the dump says from what). This needs the app on the Pi's display with that
     environment: ask the user for a window in which they stop the kiosk (`sudo`) and start the
     app by hand under cage, or run a copy under a headless cage (`WLR_BACKENDS=headless`, its own
     `SPOTIFY_ROUND_CONFIG_DIR`, the kiosk frozen only while measuring), knowing the headless
     output may negotiate a different buffer layout from the panel's.
  3. Fix what it finds; the likely shapes: Skia rendering into an intermediate that is then
     copied into the swapchain image; a per-frame upload of a full-screen image; the WSI rendering
     tiled and converting to linear for scan-out.
- **Constraints.** §4.0; no change to the kiosk without the user.
- **Acceptance.** In the live kiosk (after the user deploys), read-only:
  `ssh <user>@<pi> 'F=$(ls /sys/class/drm/card*/device/gpu_stats | head -1); cat $F; sleep 10; cat $F'`
  with the dive on screen: render jobs per displayed frame 4 → ≤ 3 and the render engine's busy
  time −1 ms a frame or better; the ~84 ms/s of TFU gone or explained.
- **Parallel:** yes, but it needs the user's time. **Tier:** Sonnet.

### T10. Pace the dive from the display (Sonnet; optional)

- **Goal.** No dive frame shown twice or skipped because of the clock (§3.9): the visualizer's
  60.00 Hz timer falls a frame behind the 60.07 Hz panel every ~14 s, ~4 times a minute.
- **Where.** `src/visuals.rs` (the visualizer loop: `deadline += budget`, `pace`), `src/ui.rs`
  (`show_idle_frame`).
- **Approach.** Let the display set the rate: e.g. render the next dive frame when the UI thread
  takes the current one (it already runs `show_idle_frame` there), keeping two frames in flight,
  or measure the presented rate and pace at its period instead of `1 / TARGET_FPS`. Not with a
  rendering notifier (it costs a clear pass a frame, T9). The animation clock already follows
  real time, so a different rate needs nothing else.
- **Constraints.** §4.0; `hotspots` must not change (rendering is untouched).
- **Acceptance.** A debug counter over 60 s of the dive (`SPOTIFY_ROUND_SCENE=nautilus`): dive
  frames produced vs. frames the UI presented with a new dive frame. Before: ~4 repeats a minute
  expected from the beat; after: 0–1. Needs the user to deploy the build to count.
- **Parallel:** yes. **Tier:** Sonnet.

## 5. Things tried and ruled out

Measured in this analysis unless a section of `docs/pipeline-optimization.md` (PO) or
`docs/nautilus.md` is named.

| idea | result | where |
|---|---|---|
| Half precision (f16) in the shaders | No shader gains a thread: Mesa 26.2 keeps each f16 in a 32-bit register and never packs two into `vfmul`/`vfadd`; `light_at` in f16 cut the model vertex shaders ~1 % and made the glass spill. Retry only if Mesa learns to vectorise 16-bit ALU work. | `docs/nautilus.md` |
| Splitting the water at the horizon (a cheap pipeline below it) | **0.02 ms** (render engine median): pixels below the horizon already branch past the expensive terms. | §3.4 |
| Water #2/#3 at 4 threads by an implicit-LOD sky lookup hoisted out of the branch | 4 threads offline, but **0.02–0.15 ms slower** at the 11 places that use them (every water pixel then fetches the sky); none of them slow. | §3.4 |
| More early-Z by the app alone (mask `GreaterEqual` on the stock driver) | No change (9.46 → 9.48 ms): the stock v3dv never enables early-Z on 7.1; with T2 it is the whole gain. | §3.1 |
| Overdraw as shader cost | Not the case on V3D 7.1: the TLB depth-tests before shading, so occluded quads were never shaded (early-Z leaves fragment cycles at 16.05 M → 16.05 M). The overdraw costs rasteriser/TLB throughput, which T2 removes. | §3.1 |
| The GPU as ALU-, bandwidth- or TMU-bound | ~20 M instructions a frame are ~1.4 ms of issue per QPU of 8.2 ms busy; L2T misses ~24 k and memory reads ~40 k a frame. Thread count and quad throughput set the time. | §2 |
| Draw calls and batching | Issuing every model draw with no instances (`DRAW_CALLS`) saves the same as leaving the models out; encoding is 0.011 ms a frame; draws cost ~0.7 µs. | §2, PO §65 |
| Merging uploads | 14 `write_buffer` TFU jobs a frame, 0.12 ms in all. | §2 |
| CPU work (static caches, placement caches, pooling) | The CPU is 0.76 ms median, 1.58 ms max a frame and overlapped with the GPU in the kiosk; the cache experiments of PO §21–55 cannot pay. The CPU governor (`ondemand`) only touches the serial number. | §3.9, PO §21–55 |
| Point lights in the vertex shaders | Sun-only `vs_terrain` (`CHEAP_TERRAIN`) saves 0.10 ms median (0.36 max); sun-only models (`CHEAP_MODELS`) 0.19 ms (0.76 max). | §3.5 |
| A flat/static model vertex shader, the cofactor hoist | Still 2 threads (46 temps) and no measured gain. | PO §5, §20 |
| Depth-first draws across materials | +1.46 ms (260 → ~2100 draws). Measured without early-Z; T2 step 6 re-checks the order probes with it. | PO §7.4 |
| Opaque draws nearest-first; the seabed before the models | +0.08 ms; +0.14 ms (without early-Z; re-checked by T2 step 6). | PO §12 |
| Keeping (loading) the colour target instead of clearing | +1.58 ms: a load reads the whole panel (a load-and-store pass is 0.77 ms against 0.40 ms for clear-and-store). | PO §7.4, `gpu_cost` |
| A clipmap terrain; a baked clipmap | 2.2× slower (the vertex shader recomputes the procedural ground); the baked one pays a fetch per vertex. | PO §13, §14 |
| A normal map on the seabed | +1.6 ms. | PO §15 |
| Coarser terrain mesh | −0.33 ms for a visibly softer seabed; render scale is the better trade. | PO §15 |
| The cheap and shimmer water pipelines as the default | They drop the ceiling and the glint, which today's shallow seas show (§3.4); T6 keeps them and makes them cheaper. | PO §13, §14 |
| MSAA; anisotropic filtering | The dive renders one sample (`SAMPLES = 1`) and samples without anisotropy (`anisotropy_clamp: 1`); either would multiply the per-pixel cost this document is about. | — |
| A KMS overlay for the dive; the LinuxKMS backend | wgpu 30 cannot export or render into an imported image; the LinuxKMS backend does not carry the dive's texture. And cage very likely scans the app's buffers out already (§3.8): what an overlay could still save is Slint's own full-screen draw of the sea, which T9 looks at inside the app. | PO §68, §71, §3.8 |
| A lower render scale as the fix | Deployed as the adaptive controller (`SPOTIFY_ROUND_ADAPTIVE`, 0.6–1.0); it trades sharpness for time and stays a safety net, not the plan. | PO §70, §72 |
| A depth pre-pass | Would add a second geometry pass (~1.3 ms of geometry today) to save shading that the TLB already skips for occluded quads. | §2, §3.1 |
