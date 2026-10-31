# The Nautilus on the Pi 5: where the frame goes, and what to do about it

Status: **in progress** (2026-10-31). Sections are filled in as measurements land; each finding
below is measured on the Pi unless it says otherwise.

Measured at `e5af5ea` (main) on the Pi 5 (V3D 7.1.7, Mesa 26.2.2-1~bpo13+0~rpt1, kiosk frozen by
the bench scripts), `hotspots` over four visits × fourteen legs × three places (168 places), six
frames a place. In this session the baseline reads **median 9.46 ms, worst 12.61 ms, 0 places
over 14 ms, 25 over 11 ms**; render engine median 7.33 ms, bin engine median 0.87 ms. (Absolute
numbers drift between sessions by ±0.5 ms; every comparison below is within one session.)

## 1. Summary (ranked)

| # | Bottleneck | Measured cost | Where | Fix |
|---|---|---|---|---|
| 1 | **No early-Z at all on the Pi 5**: Mesa 26.2.2's v3dv never enables the V3D 7.1 early-Z unit, and the corner mask's `Always` depth test would disable it anyway | **0.79 ms median, 0.80 ms on the worst place, −0.7…−1.1 ms in every sea**; 25 → 4 places over 11 ms | every frame, render engine (FEP/TLB) | driver patch (one call un-guarded) + mask `GreaterEqual` (§3.1) |
| … | *(further bottlenecks being measured)* | | | |

## 2. Method and raw evidence

### Tools

- Pi timing: `deploy/build-for-pi.sh --tests` on the Mac (touch `src` first), then
  `PI=<user>@<pi> deploy/bench-pi.sh run TAG [NAME=VALUE…]` and `deploy/bench-pi.sh compare A B`.
- Hardware counters: `DIVE3D_COUNTERS=a,b,…` (≤ 32 of the 93) in `hotspots`, read from a global
  kernel perfmon (`src/v3d_perf.rs`).
- Shader statistics without the Pi: `deploy/v3d-shaderdb/shaderdb.sh <checkout>`.
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
| `QPU-active` | 20.09 M | 33.29 M |
| `TMU-total-text-quads-access` | 8.33 M | 18.71 M |

`TLB-memory-writes` is exactly 1080 × 1080 × 4 B / 256 B: the colour target is stored once and the
depth buffer is not stored at all (its `StoreOp::Discard` works), so there is no wasted tile
traffic on the store side.

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
unchanged (0.86 ms).

What early-Z changes, per frame (medians): `FEP-valid-quads` 2.28 M → 0.55 M,
`TLB-quads-not-passing-z…` 1.70 M → 0.02 M, while `QPU-total-active-clk-cycles-fragment-shading`
(16.05 M → 16.05 M), `QPU-active` and `TMU-total-text-quads-access` do not move. So V3D 7.1 already
tests depth in the TLB *before* it shades a quad: occluded quads were never shaded. What early-Z
saves is the front end's and the TLB's throughput on the ~1.7 M quads a frame that it now drops at
the rasteriser. (This also corrects the old reading of the counters as "heavy overdraw being
shaded": the overdraw costs rasteriser/TLB time, not shader time.)

**Fix.** (a) Driver: compile `cmd_buffer_update_ez_state` for every version and call it on 7.1 for
its bookkeeping only (the `CFG_BITS` fields stay 4.2-only); see brief T1. Ship it to the kiosk as
a user-built `libvulkan_broadcom.so` selected with `VK_ICD_FILENAMES` in the kiosk unit's
environment, and send the same patch upstream to Mesa (and Raspberry Pi's `mesa` package) so a
later Pi OS update makes the local build unnecessary. (b) App: the mask's depth compare
`Always → GreaterEqual` (brief T2) — free, pixel-identical, and needed for (a) to do anything.

**Expected gain.** Measured: −0.79 ms median, −0.80 ms worst, −0.9 ms render-engine median.

**Risk.** Visual: none expected (early-Z is conservative); to be confirmed by a pixel diff of
`screenshots` under both drivers (brief T1's acceptance). Operational: a locally built driver must
be rebuilt if Pi OS upgrades Mesa (the ICD falls back to the system driver if the file is
missing; keep the system ICD as the fallback by listing it second).
