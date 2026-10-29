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
