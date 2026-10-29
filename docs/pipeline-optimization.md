# Pipeline optimisation log

Working notes for the throughput pass on the Nautilus scene. Measurements are
from `DIVE3D_STATS=1`, `dive3d::tests::coverage`, `previews` frame captures and
`deploy/v3d-shaderdb/shaderdb.sh`; the reference comparison lives in
[pipeline-comparison.md](pipeline-comparison.md).

## 1. LOD selection (done)

**Problem.** `level_of` accepted a coarser model only when `error × px < 0.5`
*and* (`stray < 0.1` or `edge × px < 1`). Simplification makes vertex colours
stray by ~0.98 on most models, so the second clause reduced to "changed triangles
are sub-pixel". Measured on the forest leg: **95% of model triangles were drawn
at full detail and LOD 2 was never selected at all.**

**Change.** `LOD_ERROR 0.5 → 2.0`, `LOD_EDGE 1.0 → 6.0`.

**Measured, forest leg** (the heaviest):

| | model triangles | draws |
|---|---|---|
| before | 321 k – 339 k | 242–256 |
| after | 202 k – 226 k | 242–252 |

~36% fewer model triangles, no change in draw count.

**Visual check.** Eight leg frames, before and after, compared per pixel
(`target/compare_frames.py`): mean 0.015/255 over all eight, 0.08% of pixels
changed by more than 2/255, worst single pixel 237 (one edge). LOD popping in
motion is not measured — the selection has no hysteresis.

## 2. Seabed shading baked into the tiles (done)

**Problem.** `vs_terrain` recomputed, per vertex per frame, what only depends on
where the vertex is: the biome, the floor's albedo, five `vnoise` fields, sand
ripples, Atlantis' paving, and the cave's veins. The CPU already builds and
caches every tile in `terrain.rs`, so that work was being done twice — once on
the CPU for the height, then again on the GPU for the colour, every frame.

**Change.** The tile builder now computes the floor's colour, texture layer,
detail strengths and cave glow, and writes them into the vertex. The shader only
applies the distance-dependent fade and the cave's flicker:

```
scale = (1 + noise'·fade) · (1 − ripple·fade) · (1 − pave·fade)
albedo = baked.rgb · scale + glow.rgb · (flicker · 0.35)
```

`Point` grows from 24 to 72 bytes (1.7 MB per frame of vertex data, about
100 MB/s at 60 Hz — under 1% of the Pi's memory bandwidth).

**Measured** (`shaderdb.sh`, V3D compiler):

| `vs_terrain` | before | after |
|---|---|---|
| instructions | 1059 | **345** |
| uniforms | 361 | 166 |
| temps | 64 | 46 |
| nops | 70 | 23 |

Still 2 threads: the five vertex attributes (18 floats) keep it above the
32-register cliff. A variant with the attributes removed compiled to 32 temps
and 4 threads, so the attributes are what hold it back, not the maths.

**Visual check.** Pixel-identical to the pre-bake frames: mean 0.000, max 1/255
on a handful of pixels from rounding in the mix order.

**CPU cost.** The work moved to the tile builder, which now does ~5 `vnoise`
fields and the biome lookups per vertex. `cpu_cost` gained a line for it:

```
cpu seabed tiles: 7.779 ms a visit (79611 vertices)   [Apple M1 Max]
```

That is a whole visit's tiles from empty (~223 tiles, 80 k vertices), built
incrementally as the boat sails — a few tiles, well under 0.1 ms, on the frames
that build any at all. The per-frame scene build is 0.414 ms for comparison.

## 3. The foot of grounded models (done)

**Problem.** Every grounded model's fragment shader did, per pixel,
`leg(voyage_x(p.x, p.z))` and then blended against `floor_albedo(l)` — a dynamic
lookup into an eight-entry table — to tint the few centimetres of model bedded
in the sand. That is why the grounded variants cost two to three times the free
ones (rock: 53 → 224 instructions).

**Change.** `world::build` already found the ground height under each grounded
instance; it now also finds the sand's colour there and stores it on the
instance. The shader blends that colour in with the same per-pixel burial
factor. The burial has to stay per pixel: computing it per vertex and
interpolating spread the sand blend up whole triangles — measured at 12.6% of
the forest frame changed — so only the lookup moved.

**Measured** (`shaderdb.sh`, fragment):

| pipeline | before | after |
|---|---|---|
| rock, grounded | 224 | **77** |
| lava, grounded | 338 | **164** |
| copper, grounded | 308 | **108** |
| vault, grounded | 291 | **140** |
| gold, grounded | 263 | **90** |

The free variants were already lean and are unchanged. The cost lands on the
vertex side: `vs_model` gained 35 instructions (508 → 543) to write the extra
varying, and copper crossed from 4 threads to 2 at 34 temps — still far cheaper
at 108 instructions and one nop than 308 at four threads.

**Visual check.** Mean 0.002/255 over the eight legs, 0.01% of pixels changed:
the per-instance sand colour differs from the per-pixel one only where a biome
border runs under a model's footprint.

## 4. Where the remaining cost is

Coverage, measured on the forest leg by drawing with each part left out
(`dive3d::tests::coverage`):

| part | pixels | share |
|---|---|---|
| models | 664 k | **56.9%** |
| seabed | 352 k | 30.2% |
| sun shafts | 193 k | 16.5% |
| water (the rest) | ~150 k | ~13% |
| sprites | 8 k | 0.7% |

| stage | instructions | invocations | notes |
|---|---|---|---|
| `fs_model` (used) | 53–165 | ~664 k px | 2 threads on vault and copper |
| `vs_model` | 503–543 | ~120 k verts | 2 threads on most, 51–55 temps |
| `vs_model` binning | 215 | ~120 k verts | a second vertex pass, as the GPU tiles |
| `fs_background` | 266 | ~150 k px | drawn last, only uncovered pixels |
| `vs_terrain` | 345 | ~23 k verts | was 1059 |
| `fs_terrain` | 40 | 352 k px | already cheap |

Leftovers:

- The two largest `fs_model` variants (`#14` 610, `#15` 610 instructions) are
  the `MATERIAL == -1` fallback pipelines. Every material the scene assigns is
  in `MATERIALS`, so nothing is drawn through them; they exist as startup
  compile cost and a trap.
- `vs_model` still runs the four-point-light loop per vertex. The `Instance`
  attribute that was meant to carry a per-instance light is now the sand colour,
  so that path has to be built from scratch if it is wanted.

**On desktop timings.** The M1 Max runs the dive in ~1.5 ms and is not
throughput-bound; repeat runs drifted from 1.5 to 2.5 ms as the machine warmed,
so those numbers cannot resolve a 5% change. The V3D instruction counts, the
triangle counts and the per-pixel diffs are the evidence here; pricing the work
needs the Pi.

## 5. Next

1. `vs_model` is now the largest single item: the cofactor normal matrix is
   rebuilt per vertex (three crosses) and could be hoisted to the instance.
2. `fs_background` (266 instructions, ~13% of pixels) — worth ~0.2 ms if cut.
3. Delete the unused fallback pipelines, or give the remaining materials their
   own slot.

## 5. Recovered tree, and one experiment that failed

`world.rs` was lost to a scripting mistake and rebuilt from the harness session
transcripts: 49 `edit` calls replayed, two test functions extracted from
payloads whose anchors no longer matched, two duplicate blocks removed. 4770
lines, 108 tests, all passing once the diver test's missed `rem_euclid(BIOMES)`
was restored.

Visual check against the pre-loss frames:

| leg | mean Δ |
|---|---|
| trench, wrecks, atlantis, abyss | 0.000 |
| arctic | 0.090 |
| forest | 0.438 |
| shallows | 5.454 |
| cave | 2.844 |

The three that differ are the kelp-heavy legs and the cave: the tuning payloads
that rewrote `kelp_here`/`kelp` and the lava glow did not replay.

**The cofactor hoist failed its measurement.** Replacing three crosses per
vertex with three prefetched instance attributes took `vs_model` from 503 to 502
instructions but from **51 to 60 temps** and 222 to 230 uniforms. It was
reverted. Verified afterwards that the whole table matches the last good state:

| shader | instructions | temps | threads |
|---|---|---|---|
| `vs_terrain` | 345 | 46 | 2 |
| `vs_model` | 503 / 543 | 51 / 55 | 2 |
| `vs_model` binning | 215 | 32 | 4 |
| `fs_model`, rock grounded | 77 | 23 | 4 |
| `fs_model`, lava grounded | 164 | 28 | 4 |
| `fs_background` | 266 | 20 | 4 |

Three of the four throughput changes survived measurement; the fourth did not
and is gone.

## 6. Raspberry Pi 5: the frame is not fill-bound

Measured on the device itself: `ssh john@192.168.0.33`, adapter `V3D 7.1.7.0`,
V3D at 960 MHz, `throttled=0x0`, 72 °C.

**How to measure.** `deploy/build-for-pi.sh --tests` builds both binaries in the
arm64 Debian container (46 s incremental), `scp target/pi/spotify-round-tests`
to the Pi, then freeze the kiosk or it contends for the GPU:

```
kill -STOP <spotify-round pid>          # 15.55 ms mean while it runs
/tmp/spotify-round-tests dive3d::tests::frame_cost --ignored --nocapture
kill -CONT <spotify-round pid>          # 12.997 ms mean while it is frozen
```

**`frame_cost`, kiosk frozen, baseline 13.44 ms:**

| variant | Δ | reading |
|---|---|---|
| no solid models | **−4.35 ms** | models are 32% of the frame |
| empty sea | −9.81 ms | everything the pass draws |
| quarter pixels | **−2.62 ms** | *all* fragment shading in the frame |
| no terrain | −1.48 ms | |
| plain textures | −1.20 ms | texture sampling |
| full detail (LOD off) | **+1.85 ms** | the LOD work is worth 1.85 ms |
| cheap models | +0.33 ms | the cheap-light path saves nothing |

(Sprites and "no noise" came out *slower*; treat ±1.5 ms as noise.)

**What this means — and it reverses the assumption this pass was built on:**

1. **Fragment shading is 2.62 ms of 13.44.** All the `fs_model` work
   (224 → 77 instructions) addressed a bucket smaller than the models' cost.
2. **Models cost 4.35 ms and are not pixel-bound** — quartering the pixels
   removes less than the models do. They pay in ~250 draws, ~150k vertices and
   the binning pass. Batching, fewer draws and coarser geometry pay here;
   better shading does not.
3. **An empty sea still costs 3.63 ms** — 27% of the frame is fixed overhead
   outside all drawing (pass setup, submission, or the texture handoff to
   Slint). Worth finding before anything else.
4. Shader instruction counts went down by 67% and the frame did not move:
   12.3 ms (README, pre-loss) → 13.0 ms now. The LOD change is the one measured
   win, at 1.85 ms.

Absolute numbers are not directly comparable to the README's 12.3 ms: the
recovered scene still differs from the lost one on shallows, forest and cave
(kelp placement and the cave glow), and those are the heavier legs.

**Corrected in §7.** The "not fill-bound" reading does not survive a closer
measurement: the draw calls are free, and the models' pixels cost as much as
their vertices. The 3.63 ms is CPU scene construction, which the app overlaps
with the GPU and which does not enter the frame.

## 7. The Pi, second pass: the fixed cost is the CPU, and the dive makes 60 fps

Measured at 1b22e1e, kiosk frozen, V3D 7.1.7.0 at 960 MHz, ARM at 1.7 GHz,
`throttled=0x0`, 60–67 °C. Within one session `frame_cost` repeats to ±0.1 ms;
between sessions absolute numbers move (the 13.44 in §6 is not reproducible
now: the same tree reads 11.2, and the kiosk binary §6 measured was built at
16:09, before `world.rs` was recovered at 17:10). Compare deltas measured in one
session, or alternate binaries.

**`frame_cost`** (legs 1, 3, 4; 20 frames each; two runs, 11.21 and 11.19 ms):

| variant | Δ |
|---|---|
| no solid models | −3.66 ms |
| quarter pixels | −3.50 ms |
| no terrain | −1.43 ms |
| empty sea | −8.98 ms |
| full detail | +1.88 ms |

**`frame_time`** (serial submit and wait, 60 frames a leg): shallows **13.49**,
forest **12.96**, atlantis 10.49, cave 10.43, wrecks 9.01, abyss 8.96, arctic
9.86, trench 8.25; mean **10.43 ms**. The dive is under the 16.7 ms budget on
every leg with 3.2 ms to spare on the heaviest.

**The kiosk**, `SPOTIFY_ROUND_FULL_SPEED=1`, Nautilus forced: window **61 fps
for a full 60-second sample**, scene 60–61 fps, gpu 11.6–16.7 ms. The gpu
reading is quantised by the 60 Hz present (a frame that makes the vblank reads
16.7, one that misses it reads ~12 or less), so it is not a frame time.

The kiosk had been running a binary from 16:09 — before `world.rs` was
recovered at 17:10. At the same point after a restart (the shallows leg) its
scene thread took **4.0–4.2 ms** a frame; the current tree takes **1.7–1.8**.
Later in the voyage (the forest) the current tree's scene thread reads
3.1–3.8 ms, so the whole leg range is about 1.7–3.8 ms; the old binary was two
milliseconds behind it where they can be compared.

### 7.1 The "fixed overhead" is the CPU, not the submission

The new `fixed_cost` test splits the empty frame, and the new `DIVE3D_PHASES`
prints the draw's own phases:

| part | ms |
|---|---|
| `world::build` | **1.35** |
| `terrain::update` (boat still) | 0.02 |
| submit (empty encoder) | 0.05 |
| empty sea | 1.94 |
| order + sort + stage + upload | 0.03 + 0.07 + 0.03 + 0.02 |
| encode | 0.015 |
| clear pass (GPU) | 0.46 |

With the boat moving, `terrain::update` builds the new tiles as it goes and
rises to 0.17 ms. So roughly 70% of the time outside drawing is **building the
scene**: the 22 placers, then the per-instance ground and sand lookup. Staging
is 0.15 ms, submission 0.05 ms, and the texture handoff to Slint costs nothing
because it is an import: Slint samples the same texture, nothing is copied or
re-uploaded. The 0.46 ms clear is inside the pass, not in "empty sea".

In the app `world::build` runs on the visualizer thread while the GPU draws the
previous frame (two in flight), so it does not add its 1.35 ms to the frame.
It is why the old kiosk binary's scene thread sat at 4 ms; that is a CPU
problem, not the 3.63 ms mystery.

Per placer (`DIVE3D_BUILD_STEPS`, means over eight legs): kelp 0.24, cave 0.23,
particles 0.11, shoals 0.10 (0.17–0.24 in the rich legs), rocks 0.08, trench
0.07 (0.29 in the trench), ruins 0.06, forest 0.06, gardens 0.03; the rest
under 0.05. The per-instance grounding is 0.03–0.15. There is no single
dominant placer to rewrite.

### 7.2 Models: the draws are free, the pixels are not

Three new probes:

- `DRAW_CALLS` issues every model draw with an empty instance range: all 260
  draws and their pipeline binds stay, the geometry goes. It saves 2.7–3.6 of
  the models' 3.67 ms, so **draw calls and batching are worth about 0.1 ms**,
  not 3.7. There is nothing to win by merging draws.
- `COARSE_DETAIL` draws every instance at its coarsest level: forest falls from
  150 k to 65 k vertices and 203 k to 56 k triangles, and the leg loses 1.87 ms
  (shallows 1.21, mean 0.4–1.4 depending on the run). Geometry is worth
  **1–2 ms**.
- `QUARTER_MODELS` draws only the models into a quarter of the pixels (the
  water and seabed refill the other three quarters, so this is net): forest
  loses 1.54 ms, shallows 2.93, cave 3.99, abyss 2.36. Model **pixels** are
  worth at least as much as model vertices.

That makes §6.2 wrong on both counts. The model cost is roughly half geometry
and half pixels, and the geometry half is only small because the LOD chain is
shallow: a fish is 272 → 102 → 76 triangles, and LOD 1/2 only qualify below
about 14/11 px on screen because `LOD_STRAY` is above the simplification stray
on most models, so the `edge × px < 6` clause is the one that binds.

### 7.3 Loosening LOD is a poor trade from here

`DIVE3D_LOD=error,edge` overrides the selection in test builds. Mean over the
eight legs and forest leg, against the preview frames (mean Δ/255, and pixels
changed by more than 2/255):

| error,edge | mean | forest | worst leg Δ | pixels > 2 |
|---|---|---|---|---|
| 2,6 (current) | 10.35 | 12.78 | — | — |
| 2,18 | 10.21 | 12.55 | 0.13 (shallows) | 1.3% |
| 4,12 | 10.04 | 12.16 | 0.12 (shallows) | 2.2% |
| 6,18 | 9.97 | 11.94 | 0.23 (forest) | 4.0% |

Even the most aggressive is worth 0.4–0.8 ms of the 13, for a visible change
on model edges, and the selection has no hysteresis, so looser thresholds pop
more. The current 2, 6 was kept.

### 7.4 Tried and reverted

- **Depth-first across materials** (so early-Z can reject more): +1.46 ms. It
  turns 260 draws into ~2100 and V3D charges ~0.7 µs a draw.
- **Keeping the colour target instead of clearing it**: +1.58 ms. On a
  tile-based GPU a load is a full read of the panel; a clear is not.
- **Biome early-outs in the placers** (skip the cell grid when the cave or the
  kelp is nowhere near): removed. The window looks out of the hull's *side*, so
  the visible x span is ±0.72 × depth, two to three legs wide; a corridor
  sample that missed the negative-x side skipped a visible garden on the arctic
  leg (one instance, twelve sprites). Sampling the whole span leaves nothing to
  skip. Biome gating does not work for this camera.
- **`gardens`**: the seabed lookup and visibility test moved after the
  per-cell biome rejection. Exact (previews pixel-identical) and saves ~0.03 ms
  of the build. Kept.

### 7.5 Where the frame goes, and what is left

Forest, serial: 12.96 = ~1.5 build (overlapped in the app) + 1.43 terrain +
3.67 models (half pixels, half geometry) + 0.46 clear + the rest water,
sprites, shafts, glass and the corner mask. The only remaining levers are look
changes: texture sampling in the model fragments (plain textures save 1.57 ms),
model geometry, and terrain triangles. Submission, pass setup and the handoff
have nothing left to give.

The 16.7 ms target is met: 10.43 ms mean, 13.49 ms on the worst leg, and 61 fps
on the kiosk. The useful next step is not more micro-optimisation but keeping
the deployed binary current — the kiosk had been running a pre-recovery build
with a 4 ms scene thread.


## 8. What 90 fps would take

**The panel first.** The kiosk's EDID detailed timing is 85.5 MHz over
1280 × 1112 totals for 1080 × 1080 active: **60.07 Hz**. `app::TARGET_FPS` is
60 for that reason, and the tick, the globe and the progress ring all follow
it. This display cannot present 90 fps; that needs a 90 Hz panel or mode, and
the pacing constant has to follow it. Everything below is what the renderer
would have to cost to be 90 Hz-capable.

**Budget.** 90 Hz is 11.1 ms a frame, including Slint's composite of the sea
texture. The V3D has no timer queries, so the composite is not measured
separately; keep about 1–1.5 ms for it and aim the dive at **9.5–10 ms** of GPU
work.

**Resolution is the lever that reaches it.** `DIVE3D_SCALE` renders the pass
into a fraction of the panel (`frame_time`, serial, `DIVE3D_SCALE=n`). The
target and the clear stay 1080² in this probe, so a real smaller target saves a
little more than these numbers show, and the ~1.4 ms CPU build is included but
overlaps the GPU in the app:

| scale | target | mean | forest | shallows |
|---|---|---|---|---|
| 1.0 | 1080² | 10.48 | 12.99 | 13.52 |
| 0.9 | 972² | 9.38 | 12.02 | 12.03 |
| 0.833 | 900² | 8.70 | 11.31 | 11.18 |
| 0.75 | 810² | 7.80 | 10.46 | 9.99 |
| 0.667 | 720² | 7.09 | 9.70 | 9.03 |

At 900² the worst leg is 11.3 serial, or about 9.9 ms of GPU after the build
comes out — borderline once the UI is added. At 810² the worst is 10.5 serial,
about 9.1 GPU, which leaves the ~1.5 ms the UI may need. 720² is comfortable.
The cost is softness: 900² is a 17% linear drop from the panel, 810² is 25%,
720² is 33%; only the sea is scaled, the UI stays native. The design had no
smaller profile on purpose, so this is a product decision. It is also the
standard mobile lever: Unity's URP exposes it as **Render Scale** with a
bilinear or FSR upscaler, scaling the 3D render but not the UI
([URP Asset](https://docs.unity3d.com/Packages/com.unity.render-pipelines.universal@15.0/manual/universalrp-asset.html)).

**The other levers, for margin or to avoid scaling.** None is free; the
measured ones are small:
- Model fragments re-light per pixel, four point lights included, although
  `vs_model` already applies the same lights per vertex. A pipeline variant
  that drops the point-light loop from the fragment `light_at` is the largest
  fragment cut left; it changes how the four point lights fall across a model
  (per-vertex instead of per-pixel), so measure it with a frame diff.
- LOD `2,18` saves 0.14 mean / 0.25 forest, `4,12` 0.3 / 0.6, both with edge
  changes and more popping.
- The whole water pass is 0.35 ms, so cutting its 266 instructions saves at
  most ~0.2.
- Terrain is 1.43 ms. `vs_terrain` is 46 temps at 2 threads; fewer attributes
  is the only way to 4 threads, worth perhaps 0.2–0.4. Coarser far tiles save
  0.2–0.3 and show.
- Sprites and sun shafts are ~0.5 ms together; dropping the shafts at 90 Hz is
  ~0.15.
- The 0.46 ms clear is not reducible: a load was measured slower.

**What will not help:** batching the draws (~0.1 ms for all 260), submission
(0.05 ms), the texture handoff (an import), and the CPU build (1.4 ms,
overlapped; the app's 3.1–3.8 ms is contention, and at 90 Hz two frames in
flight give it 22 ms).

**At 1080²** the safe levers total well under a millisecond, so 90 fps on the
full panel is out of reach without giving up per-pixel point lighting or
terrain detail. The practical path is **810², `TARGET_FPS` from the panel, a
90 Hz mode**, and then 720² if the composite eats the margin.

## 9. Every effect, measured in one build

`effect_cost` (new, ignored test) turns every draw-level effect off one at a time
from **one binary** and prints one table. Two masks:

- `DIVE3D_SKIP` (pass bits, now a `u64`): the old bits keep their values, and
  `MASK`, `FADING`, `SUN_SHAFTS`, `BEAMS` and `TUNNEL` are new.
- `DIVE3D_SKIP_EFFECTS` (test only): one bit per model group and per sprite
  kind. Models are filtered before staging, so their sorting and upload go too;
  sprite kinds are filtered before the upload; passes are skipped at draw time.
  The model groups are by mesh — `Cone` is forest trunks, stalactites, crinoid
  arms and the volcano together; `Box` is wreck castles, ruin blocks and penguin
  wings; `Sphere` is coral, pearls, eyes, ports and lanterns — so a shared mesh
  cannot be split per feature.

The test measures 63 variants over all eight legs, round-robin on paired
frames with a settling frame, and reports ms, delta, share, and how many
instances or sprites it saw. 92 s on the Pi; 24 s on the desktop, where the
1.5 ms frame makes most rows noise.

**Pi 5, kiosk frozen, everything 10.83 ms** (top of the table; rows below
±0.05 ms are noise):

| effect left out | Δ ms | share | what it was |
|---|---|---|---|
| the whole sea pass | −8.81 | 81% | every draw |
| quarter pixels | −4.36 | 40% | all fragments and depth |
| opaque models (with fading) | −3.94 | 36% | `MODELS` |
| model geometry | −3.39 | 31% | `DRAW_CALLS`: every model draw, no instances |
| **fading models** | **−2.62** | **24%** | `FADING`: blended after the water |
| quarter model pixels | −1.85 | 17% | models into a quarter of the pixels, rest full |
| planes | −1.38 | 13% | sea surface + ice sheet |
| terrain | −1.21 | 11% | |
| plain textures | −1.06 | 10% | |
| coarsest detail | −0.90 | 8% | |
| no noise | −0.60 | 6% | |
| trilinear only | −0.49 | 5% | |
| all cheap lights | −0.36 | 3% | |
| fish | −0.35 | 3% | 7 750 instances |
| sprites | −0.33 | 3% | |
| open water | −0.32 | 3% | |
| motes | −0.21 | 2% | 36 265 sprites |
| shafts + beams | −0.20 | 2% | |
| rocks 0–2 | 0.15–0.19 | each | ~2 100 each |
| point lights | −0.16 | 1% | |
| cave tunnel | −0.08 | 1% | |
| glass | −0.06 | 1% | |

**The fading models are the surprise.** They cost 2.62 ms — twice the opaque
models' 1.32 ms — and the old part bits could not see them: `MODELS` removed
both, so the blend pass hid inside the 3.7 ms "models". They are the ice and
the props inside a biome's fade band, drawn after the water, back to front,
with a `discard`; the window looks across several legs, so some props are
always mid-fade. That is the largest single effect after the pass, and the
first place to look after resolution for 90 fps.

Everything else is small and overlaps: the planes row (1.38) covers both the
solid sea surface and the fading ice sheet, so it straddles the opaque and the
fading rows; fish, sprites, water, motes and shafts are a few tenths each; the
per-model rows outside the fish, the three rocks and the planes are ≤0.05 ms on
the Pi. The shared-mesh groups
cannot be split, and the per-model deltas are net of the water and terrain
uncovered behind them, so they are not additive.

## 10. Bubbles: no pops, and a wake that follows the submarine

Four emitters shed `BUBBLE` sprites: the diver's helmet exhaust, the penguins'
flippers, the sister submarine's screw, and the Nautilus' hull. A frame-matching
diagnostic (`debug_bubble_jumps`, ignored) found three real pops and one
velocity flaw:

- **Diver helmet**: `rise = fract(t*0.35 + b/4 + feet[0])` with constant alpha.
  Each of four bubbles teleported 0.35 units from the top of its column back to
  the exhaust every 2.86 s, one every 0.71 s: 39 bright jumps in six seconds,
  worst 0.364 units at full brightness.
- **Penguin trail**: `back = (b + fract(t*4.8))*0.05 + 0.04` shared one phase,
  so the whole five-bubble trail snapped 0.25 units forward at 4.8 Hz.
- **Hull and wake**: aged and faded, but hard-culled by `visible()` at the view
  cone: 15 bright edge appearances in six seconds in the 200 s encounter.
- **Wake**: `screw(t) + aft(t)*water_speed*age`, a straight line locked to the
  current heading and rescaled around the current screw every frame — no lag
  under acceleration, no curvature in a turn.

The fix adds one lifecycle helper (`world.rs`): `cycle = t/life + k/N`,
`age = frac(cycle)*life`, `cohort = floor(cycle)`, and
`fade = smooth(0, 0.08 life, age) · smooth(life, 0.86 life, age)`; the cohort's
hash changes only where the fade is zero. Diver and penguin bubbles are now
left where they were shed and rise there. The hull bubbles keep
`start(t) − v·age`.

The wake is read from her own track, sampled from the existing analytic
`at(τ)` path — no stored history, so previews and seeks are deterministic:

```
q = screw(t) + S(t−age) − S(t) + spread·hash + (0.045 + 0.02·churn)·age·ŷ
churn = smooth(0.02, 0.25, water_speed)
spacing along the track = |Ṡ| · life / 28
```

Spacing and length grow with speed, the trail curves through a turn, and with
no way on the bubbles collapse to the screw and rise. A `churn` term widens and
spreads them with speed. All four emitters now fade over the last 10% of the
view cone (`window_fade`) instead of being culled, and the swipe rush uses a
fixed 220-bubble stream with the count only fading its tail in and out, so a
change in speed no longer re-phases the field.

`bubbles_never_pop` runs six seconds of the diver, the penguin and the 200 s
encounter at 60 fps and asserts no bubble above 0.15 brightness jumps more than
0.03 units: **0** now, where the diver had 0.364-unit pops. All eight leg
previews are byte-identical between two renders, and the before/after mean
pixel difference is ≤0.01/255. The frame is unchanged: 10.49 ms mean, 12.96 ms
forest.

## 11. Lighting: already the low-spec reference

The references are in
[pipeline-comparison.md §6](pipeline-comparison.md#6-low-spec-lighting-the-references-already-do-what-this-renderer-does):
URP keeps its main light per pixel and its additional lights per vertex; Godot
4.4 forces vertex shading on mobile; Filament has no per-vertex path and spends
precision instead; PowerVR's rule is to move work to where there are fewer
instances. The Pi build (`full-hd`) already computes the whole light at the
vertex — sun, sky, caustics, searchlight and four point lights — and its
fragment shader has no `light_at` at all (53–165 instructions). So there is no
lighting solution to import: the remaining fragment cost is texture sampling,
and the remaining vertex cost is the 503-instruction `vs_model`, which a
runtime switch cannot reduce below the register cliff. The `!full_hd` build
still lights twice (vertex and fragment); it is not the kiosk.
