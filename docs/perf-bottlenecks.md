# The Nautilus on the Pi 5: where the frame goes, and what to do about it

Status: **in progress** (2026-10-31). Sections are filled in as measurements land; each finding
below is measured on the Pi unless it says otherwise.

Measured at `e5af5ea` (main) on the Pi 5 (V3D 7.1.7 at 960 MHz, 4 slices × 4 QPUs, Mesa
26.2.2-1~bpo13+0~rpt1, kiosk frozen by the bench scripts), `hotspots` over four visits × fourteen
legs × three places (168 places), six frames a place. In this session the baseline reads
**median 9.46 ms, worst 12.61 ms, 0 places over 14 ms, 25 over 11 ms**; render engine median
7.33 ms, bin engine median 0.87 ms. (Absolute numbers drift between sessions by ±0.5 ms; every
comparison below is within one session.)

`hotspots` times a frame serially: CPU build and staging, submission, binning, rendering and the
wait. In the kiosk the CPU and the binning of one frame overlap the rendering of the one before,
and Slint's and cage's own frames share the render engine (§3.8), so the number that limits the
kiosk is **the render engine's time per frame plus ~2 ms of UI and compositor work**.

## 1. Summary (ranked)

| # | Bottleneck | Measured cost | Where | Fix (brief) |
|---|---|---|---|---|
| 1 | **No early-Z at all on the Pi 5**: Mesa 26.2.2's v3dv never enables the V3D 7.1 early-Z unit, and the corner mask's `Always` depth test would disable it anyway | **−0.79 ms median, −0.80 ms worst**, −0.7…−1.1 ms in every sea; 25 → 4 places over 11 ms | every frame, render engine (FEP/TLB) | driver patch + mask `GreaterEqual` (§3.1; T1, T2) |
| 2 | **The cave's tunnel**: a full-screen vault shader at 2 threads, drawn first and overdrawn by everything after it | **2.2 ms median over the cave's places, 4.4–5.5 ms at the seven slowest tunnel views** (cave 0.15, the legs that look into the cave) | cave; brine/forest/shallows 0.85 | (§3.2) |
| 3 | **The city's buildings**: `fs_model` for `CITY_LIGHTS` is 650–667 instructions at 2 threads | 2.9 ms median over the city's places, 5.2 ms at city s0 0.45 | city | (§3.3) |
| 4 | **Open water**: the full-screen background shader | 1.74 ms median, up to 3.0 ms | every sea, most at 0.45 | (§3.4) |
| 5 | **Seabed** | 1.58 ms median, up to 3.4 ms | every sea | (§3.5) |
| 6 | **Sprites** (marine snow, bubbles, glows, embers) | 0.43 ms median, up to 4.1 ms | forest, vents, abyss, wrecks | (§3.6) |
| 7 | **Glass** (jellyfish, crystals) | 1.5 ms median in the bloom, up to 3.7 ms | bloom, abyss, wrecks, arctic | (§3.7) |
| 8 | **UI and compositor**, outside `hotspots` | ≈2.0 ms render engine + ≈1.4 ms TFU per displayed frame | every frame in the kiosk | (§3.8) |
| … | *(CPU, start-up and the ruled-out list being written)* | | | |

## 2. Method and raw evidence

### Tools

- Pi timing: `deploy/build-for-pi.sh --tests` on the Mac (touch `src` first), then
  `PI=<user>@<pi> deploy/bench-pi.sh run TAG [NAME=VALUE…]` and `deploy/bench-pi.sh compare A B`.
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
≈2.0 ms** of render engine, and **two TFU jobs a frame of ≈0.7 ms each** come from outside the
app's own uploads: Slint's frame and cage's composite of it (§3.8). For reference
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
its bookkeeping only (the `CFG_BITS` fields stay 4.2-only); see brief T1. Ship it to the kiosk as
a user-built `libvulkan_broadcom.so` with its own ICD file, selected by `VK_ICD_FILENAMES` in the
kiosk unit's environment only while that file exists (a wrapper, not a second ICD in the list:
two listed ICDs expose two V3D adapters), and send the same patch upstream to Mesa (and to
Raspberry Pi's `mesa` package) so a later Pi OS update makes the local build unnecessary. (b) App:
the mask's depth compare `Always → GreaterEqual` (brief T2) — free, pixel-identical, and needed
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

**Fix (T4).** Split the city by part. When the library builds a city model, sort its triangles by
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

### 3.4 Open water: a full-screen pass whose shader is half its cost

**Evidence.** Leaving the water out saves 1.74 ms median (up to 3.03 ms: abyss, atlantis,
arctic and coral at 0.45, where most of the window is open water). Compiling the same pass with
the existing `CHEAP_BACKGROUND` switch (`DIVE3D_SKIP=16384`: the depth gradient only, no glint,
ceiling or sky) saves **1.01 ms median, 1.94 ms max**. So of the water's 1.74 ms, ~1.0 ms is its
shader's work and ~0.7 ms is the pass itself (rasterising, depth-testing and writing the
uncovered pixels, which a 3-instruction shader would still pay).

**Root cause.** One fragment shader per sky (`SKY` override, `sky_mode` on the CPU):

| pipeline | when | instructions | threads | temps |
|---|---|---:|---:|---:|
| `Dive3d water #0` | open water | 229 | 4 | 19 |
| `Dive3d water #1` | under the ceiling | 318 | 4 | 22 |
| `Dive3d water #2` | polar sky through the leads | 321 | **2** | 19 |
| `Dive3d water #3` | Snell's window | 398 | **2** | 28 |

Every pixel below the horizon pays for the whole shader's register budget even though only the
part above the horizon (`dir.y > 0`, and the glint band `dir.y > 0.05`) uses the waves, the
caustic ceiling and the sky.

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

**Fix.** (a) Split the pass at the horizon: the CPU already knows the camera basis (`Globals`), so
it can emit two polygons, the part of the window below the horizon drawn with a gradient-only
pipeline (`water_at` only, pixel-identical there because the glint, ceiling and sky terms are all
multiplied by zero below `dir.y = 0`; check the `up` band's 0.05 start) and the part above with
the current `SKY` variant (brief T6). (b) Not recommended: #2/#3 at 4 threads (measured above);
a per-vertex grid for the whole water, which would blur the glint and the caustic web.

**Expected gain.** (a) is bounded by the shader's 1.0 ms median share times the part of the water
below the horizon, less what `water_at` alone still costs there: estimated ~0.3–0.6 ms median, up
to ~1 ms at the 0.45 places that look level or down (abyss, atlantis, arctic, coral). Not measured;
the brief's first step is to measure it. Pixel-identical by construction below the horizon.

**Risk.** Low. The seam is the horizon line itself, where every term the split removes is zero; a
mistake shows as a hard line across the water, easy to see in the previews.

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

**Fix.** Draw each plume as one or two stretched quads with the rising puffs computed in the
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

### 3.8 The UI and the compositor: ~2 ms a frame that `hotspots` never sees

**Evidence.** §2's live sample: with the kiosk running normally the render engine does 4 jobs a
frame at 60 fps and is busy 733 ms a second; the visual's own frame was one job of ~10.2 ms (the
globe was on screen; the dive's frame is likewise one render job). The other three render jobs
cost ≈2.0 ms a frame, and two TFU jobs a frame (≈0.7 ms each, 84 ms/s) are not the app's uploads
(those are ~8 µs each). A clear plus one full-screen textured quad plus the store is 0.79 ms on its
own (`gpu_cost`), which is what Slint's frame of the sea is; the rest is cage compositing the
app's buffer rather than scanning it out (a TFU job of ~0.7 ms is what converting a 1080² linear
RGBA8 buffer to the tiled layout the GPU samples costs).

**Consequence.** The kiosk's budget for the dive's render engine is ~14.5 ms at 60 Hz, not
16.7 ms; at the worst place today (11.08 ms render) that leaves ~3.5 ms, and the adaptive render
scale (`SPOTIFY_ROUND_ADAPTIVE`, which steers from the render engine's busy time, UI included)
has headroom. A frame drawn at a time unrelated to the display's vsync (§3.9) still lands on the
next vblank only if all of this fits.

**Fix.** (a) Find out why cage composites instead of scanning out (`cage -d` logs wlroots' direct
scan-out decision; needs a change to the kiosk unit, i.e. the user with sudo): a 1080² opaque
`XRGB8888`/`ARGB8888` client that covers the output is a scan-out candidate unless the buffer's
modifier or an output transform or a software cursor rules it out. Scan-out would return ≈1.2 ms
of render engine and ≈1.4 ms of TFU a frame, for the globe and the player too. (b) The KMS
overlay for the dive itself stays blocked (`docs/pipeline-optimization.md` §71).

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

**Frame pacing (not a cost, a smoothness note).** The visualizer paces itself with a 60 Hz CPU
timer (`visuals::pace`), not the display's vblank, and its submission and Slint's frame both run
on the UI thread. A dive frame that finishes just after a vblank waits a whole refresh, and the
timer's 60 Hz beats against the panel's, so a frame is occasionally shown twice or skipped even
when the GPU has room. Driving the dive from Slint's `BeforeRendering` notifier (render the next
dive frame as the UI starts presenting this one) would lock the two; a design change, measured by
counting repeated frames, not by `hotspots`.

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

**Fix (T9).** (a) Build the models in parallel (`std::thread::scope` over `Model::ALL`, results
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
