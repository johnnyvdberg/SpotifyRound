# The solar tour on the Pi 5: the next round of frame time, planned

A plan for implementation agents, not a record of work done. Nothing here is wired into the app.
Base: `main` at `30cab38` ("Solar: dodge a moon as it is over a second round now"). It follows the
form of [perf-bottlenecks.md](perf-bottlenecks.md): the evidence first, then a brief per change.

What was measured for this plan, and what was not:

- **Measured**: every shader's instructions and threads from the Pi 5's own compiler
  (`deploy/v3d-shaderdb`, Mesa 26.2.2 v3dv on the noop shim), for the app as it is and for the
  prototypes in [solar-lab/](solar-lab/); every draw's pixels at each key moment
  (`deploy/pi-estimate.sh`, seeds 5 and 23).
- **Not measured**: nothing was run on a Pi. The prototypes were compiled, never drawn: their
  pictures, their triangle winding and their cost in milliseconds are unproven. The millisecond
  figures below are the estimator's (`--ms-per-inst 0.025`, two threads counted as twice the cost),
  which reads high: it puts an orbit of a sea world at 15–19 ms where commit `1fd9399` reports under
  10 ms on the Pi. Read the gains as ratios; S1's first job is to measure one on the Pi.

## 0. Measured on the Pi (2026-11-11)

`solar::tests::frame_sweep` on the Pi 5, kiosk stopped, a frame every 8 s through seeds 5 and 23
(medians; run to run within 0.01 ms). Headless runs need `WGPU_BACKEND=vulkan` on the Pi: GL lacks
the frame texture's `VIEW_FORMATS`. Base `5e6a964`, before S1–S5:

| | high orbit | departure, worst | transfer | jump |
|---|---|---|---|---|
| seed 5 before | 7.27 | 10.44 | 2.36 | 4.95 |
| seed 5 after | 5.49 | 7.64 | 2.51 | 4.85 |
| seed 23 before | 8.89 | 12.42 | 2.33 | 4.95 |
| seed 23 after | 5.24 | 7.34 | 2.33 | 4.85 |

- **S1–S4 hold up**: the traced paths (`SOLAR_TRACED`) measured the same as before on every frame, so
  they and the shaders only they used are gone.
- **S5 step 2, the polar fan, did not**: some 42,000 small triangles carrying three varyings cost
  the binner and the varyings more than `atan2` and `log` a pixel. Every drive's jump was 1.1–3.1 ms
  slower than before, so the fan is gone and `fs_warp` draws the whole frame again. The estimator
  cannot see this: it counts pixels and instructions, not triangles.
- **Four threads are not always faster**: Lumen Drift with its streaks before its noise fits four
  threads (376 instructions) but ran 6.60 ms; the other order, two threads (344), runs 5.96.
- **Jump per drive** (seed 5, t 505–560), before → now: Lumen Drift 6.12 → 5.96, Tachyon Tunnel
  4.09 → 4.11, Fold Drive 8.09 → 8.04, Ion Slipstream 4.91 → 4.63, Bubble Drive 4.94 → 4.82.
- `SOLAR_ANISO=4` costs 1.3–1.5 ms in orbit; it stays off.
- **The drop-out** (the jump's light torn open over the next system, no white): the arriving
  visit's first 2.4 s peak at 6.9–8.7 ms, Fold Drive 11.6 (a frame every 0.3 s, seed 5). Drawn as
  one overlay over the new system it peaked at 8.1–13.3 ms; the part ahead of the front is now an
  opaque annulus drawn first and writing the depth, so nothing it hides is shaded. Jumps are
  0.22–0.26 ms faster than before it, the white flash's mix gone and the flare a pipeline of its
  own for the last second.
- **The jump point and spool-up** (8d8bb5f, seed 5, `SWEEP_DRIVE` 1–5): the cruise out runs 3.7 ms,
  the stages of the spool-up 6.8–9.3 ms (`fs_spool`, two threads but for Tachyon Tunnel). Its last
  ~14 s, from the engagement into the jump's first 9 s, mix the drive's jump light into
  `fs_spool` too, and those frames peak at Lumen Drift 13.7, Tachyon Tunnel 15.3, Fold Drive 15.2,
  Ion Slipstream 10.1, Bubble Drive 11.9 ms: the heaviest frames in the tour, close to the budget
  before the UI and the compositor. The jump after is unchanged (3.9–7.8 ms) but for Bubble Drive,
  4.61 → 5.34 ms with its churning stars (265 instructions, was 227). Orbits, transfers and the
  drop-out are unchanged.
- **The frame's corners masked**: the round display never shows the fifth of the frame outside its
  circle, so the pass now starts with a depth-only annulus there (a 32-gon round the circle out
  past the corners) and nothing after it is shaded there. The fade peaks drop to Lumen Drift 11.3,
  Tachyon Tunnel 12.5, Fold Drive 12.6, Ion Slipstream 8.4, Bubble Drive 9.9 ms; the spool-up's
  stages by 1.0–1.5 ms, jumps by 0.4–1.2 ms, orbits by about 0.25 ms.
- Tried and dropped for the fade: a spool shader without the drive's light for the stages before
  the engagement (half the instructions, but Lumen Drift went to four threads and 0.45 ms slower,
  the others 0–0.4 ms faster), and the fade as the jump's light with that shader blended over it
  (Tachyon Tunnel 12.5 → 10.9 ms, every other drive 0.5–0.9 ms slower). Neither lowers the worst
  frame, Fold Drive's 12.6 ms.

## 1. Summary (ranked)

| | what | where the time is now | prototype | expected |
|---|---|---|---|---|
| S1 | Mapped worlds as a sphere mesh: per-vertex light, per-pixel texels only | `fs_surface`, 416–678 inst, half its variants at 2 threads; 80–90 % of every orbit frame | 49–134 inst at 4 threads; sea worlds 217–246 inst in two passes at 4 threads | orbit frames 3–5× cheaper on the dominant draw |
| S2 | The air past the limb as an annulus lit at its vertices | `fs_limb` 343 inst, or +75–100 inst inside every `…l` surface variant | 46 inst at 4 threads | −0.3…−0.6 ms in orbit; halves the surface pipelines (18 → 9 a slot) |
| S3 | Worlds without maps, moons and the star on the same mesh | `fs_flat` 298, `fs_moon` 422, `fs_star` 243 | not prototyped; `fs_globe` without air is 49 | `flat` 3.4 → ~0.6 ms at the start of a transfer; star flybys −0.5 ms |
| S4 | The star's glare off the full-screen background | `fs_space_baked` 91 inst over every pixel no body hides | not prototyped | −0.5…−1.3 ms on every frame outside a jump |
| S5 | Jump drives: no trigonometry per pixel | `fs_warp` 226–464 inst; Lumen Drift at 2 threads (43 temps) | not prototyped | Lumen Drift to 4 threads; −1…−2 ms a drive |
| S6 | Eclipses on the mesh | `fs_eclipse` 277 inst, scissored | not prototyped | small and rare; do last or not at all |

S1 is the plan; the rest are what is left once it lands. S2 depends on nothing and can be done
first or in parallel. S3 and S6 build on S1's vertex shader.

## 2. Evidence

### Where an orbit frame goes today

`deploy/pi-estimate.sh` at `30cab38` (est ms; the heaviest draws):

| moment | est | heaviest |
|---|---:|---|
| seed 5 p1 orbit (sea, clouds) | 16.7 | `surface0sc0a--l` 14.7, space 1.4 |
| seed 5 p3m1 orbit (sea, clouds, cities) | 17.8 | `surface1sc1a--l` 15.8, space 1.4 |
| seed 5 p5 orbit (clouds) | 14.9 | `surface1-c0a--l` 12.9, space 1.4 |
| seed 23 p5 orbit (ringed giant) | 12.0 | `surface0--0agr-` 7.6, ring 1.7, space 1.6 |
| seed 5 p3 orbit (giant) | 6.5 | `surface0--0ag-l` 4.2, space 1.6 |
| seed 23 p3 orbit (airless) | 5.3 | `surface1--0----` 3.3, space 1.4 |
| transfer, a world without maps yet | 6.0 | flat 3.4, space 1.4, limb 0.6 |
| transfer, nothing near | 2.8 | space 2.3 |
| jump (Bubble Drive) | 6.5 | warp 6.0 |

In orbit the world's disc and air cover 545 k of the frame's 1166 k pixels (the solid disc 463 k, a
radius of 384 px), so one shader over one disc is the frame.

### Why `fs_surface` costs what it does

Every pixel of the disc traces its own ray against the sphere, turns the normal into the body's
frame, takes `atan2` and `asin` for the map's coordinates, differences them for a mip level, and
works out the sun's and the viewer's angles, three or four exponentials for the air, the clouds'
shadow offset and the tangent frame, before it reads a texel. None of that depends on a texel, and
all of it varies smoothly over the sphere. With five maps' fetches beside it, the sea and cloud
variants need 37–62 registers and run at two threads.

### The prototype: the same shading with the smooth part at the vertices

[solar-lab/globe.wgsl](solar-lab/globe.wgsl) draws the world as a patch of its own
latitude-longitude grid. The vertex shader does everything above once per vertex; the fragment
shader receives the map coordinates and seven other interpolated values, samples with the GPU's own
mip selection (`textureSampleBias`: no seam, because the mesh never crosses the longitude wrap
inside a triangle), and combines. Compiled by the Pi's compiler:

| world (flags as in `SurfaceLook`) | today `fs_surface` (with limb) | `fs_globe` one pass | two passes |
|---|---|---|---|
| sea, clouds, cities, air (`sc1a`) | 578 @2 (678 @2) | 173 **@2** (38 temps) | 134 + 112, both @4 |
| sea, clouds, air (`sc0a`) | 540 @2 (631 @2) | 146 **@2** (36 temps) | 134 + 83, both @4 |
| clouds, air (`-c0a`) | 480 @2 (554 @2) | 134 @4 | |
| glowing rock, air (`--2a`) | 450 @4 (541 @4) | 95 @4 | |
| air (`--0a`) | 416 @4 (514 @4) | 78 @4 | |
| giant (`--0ag`) | 422 @4 (522 @4) | 83 @4 | |
| ringed giant (`--0agr`) | 469 @2 (546 @2) | 102 @4 | |
| glowing giant (`--2ag`) | 452 @4 (538 @4) | 98 @4 | |
| airless (`--0-`) | 324 @4 | 49 @4 | |
| airless, glowing (`--2-`) | 338 @4 | 63 @4 | |

`vs_globe` is 204–369 instructions at four threads (133 in the binner). At 97 × 97 vertices that
is about 4 M instructions a pass, some 0.1 ms.

The two sea variants do not fit four threads in one pass. Two attempts to make them fit by sending
fewer values across (the cloud deck's light and the air's sheen made per pixel from one colour; the
cloud shadow's offset made per pixel) cost 20–28 instructions and freed at most two registers
(39/34 and 36/34 temps): the pressure is the working set while five texels are live, not the count
of inputs. Splitting does work: `fs_globe` built with `CLOUDS` alone (134 @4), then `fs_glow`
(sea glint and sheen, city lights, the air's sheen; 112 or 83 @4) added over it. That is the
recommended shape for worlds with seas; a later agent may still find a one-pass form under 32.

[solar-lab/limb.wgsl](solar-lab/limb.wgsl): the air past the limb as an annulus, lit (eclipses
included) at its vertices: 46 instructions at four threads against 343.

To recompile the prototypes after a change (about a minute once the image has a build):

```bash
deploy/v3d-shaderdb/shaderdb.sh . >/dev/null   # once: builds the image's test binary
docker run --rm -v v3d-shaderdb-target:/target -v "$PWD/docs/solar-lab":/lab:ro \
    -e SHADER_LAB_DIR=/lab v3d-shaderdb bash /lab/run-lab.sh > /tmp/lab.raw 2>&1
python3 deploy/v3d-shaderdb/label.py /tmp/lab.raw | grep ' FS \| VS '
```

## 3. Briefs

### 3.0 Rules for every brief

The rules of [perf-bottlenecks.md §4.0](perf-bottlenecks.md) apply (own branch and worktree, never
cargo on the Pi, the kiosk is the user's display, one Pi run at a time, the shader-db gate, clippy
and fmt). For the solar tour in particular:

1. **Pictures before and after**, same seeds:
   `SOLAR_SHOTS_DIR=<dir> SOLAR_SHOTS_SEEDS=5,11,23 cargo test --release solar::tests::screenshots -- --ignored`
   and `solar::tests::warps` for S5. Between them seeds 5 and 23 visit a sea world with and without
   cities, a cloud world, a giant, a ringed giant, an airless world and a world-sized moon. Put the
   most-changed frames side by side for the user. None of these briefs is pixel-identical; each says
   what may differ and by how much.
2. **Estimate**: `deploy/pi-estimate.sh` before and after, after
   `deploy/v3d-shaderdb/check.sh . --update` so new pipelines have rows.
3. **On the Pi**: `solar::tests::frame_sweep` (`SWEEP_SEED=5` and `23`) through the test binary, in
   the same session before and after, kiosk frozen (rule 3 of §4.0). Report orbit, transfer and jump
   medians and the worst frame.
4. **No `discard`** in any shader of the frame's pass: one shader that can discard turns the V3D's
   early depth test off for every draw of the frame (see `fs_ring_annulus`).
5. Keep the traced pipelines until the user has seen the Pi's numbers: build them only when
   `SOLAR_TRACED` is set, select them in `surface_passes`, and delete them in a follow-up.

### S1. Mapped worlds as a sphere mesh

**Goal.** Replace `vs_body` + `fs_surface` for every mapped planet and world-sized moon seen from
afar (the only way the tour sees them since `9b5f266`) with `vs_globe` + `fs_globe` (+ `fs_glow`
for worlds with seas), from [solar-lab/globe.wgsl](solar-lab/globe.wgsl).

**Geometry.**

- A grid of N × N cells over a window of the body's own longitude and latitude, generated from
  `vertex_index` (no vertex buffer): `index = row * 129 + column`, so one stride serves every N.
  One static `u16` index buffer holds the triangle lists for N = 8, 16, 32, 64, 96, 128 at known
  offsets; `draw_indexed` picks the range. Two new `vec4` fields on `Body` (shader and Rust):
  `window` (longitude and latitude of the corner, then their spans) and `mesh` (N, and how far the
  mesh stands proud of the surface).
- The window, on the CPU in `params()`: the cap the camera can see. With `a` the direction from the
  centre to the camera in the body's frame (`rot` rows · −place / d), `lat_c = asin(a.y)`,
  `lon_c = atan2(a.x, a.z)` and `alpha = acos(R / d)` plus two cells: latitudes
  `lat_c ± alpha` clamped to the poles; if either pole is inside, all 360° of longitude starting at
  `lon_c − π`; else `lon_c ± asin(sin(alpha) / cos(lat_c))`. Longitude is left unwrapped (it may
  pass ±π); the sampler already repeats in u.
- N from the disc's size: with `R_px` the limb's radius in pixels, cells no wider than
  `sqrt(0.4 / R_px)` radians (a silhouette within 0.2 px of the circle), so 96 or 128 in orbit and
  8–16 for a body a few pixels across. Bodies far away must not draw 128² cells each: sixteen
  bodies at full resolution is half a million triangles through the binner.
- Clip position `(x, y, rank * w, w)` with `w` the depth along the view, as `vs_ring` does: the
  values are interpolated with the perspective and the GPU clips what is behind the camera. Cull
  back faces (the default pipeline has `cull_mode: None`; this one needs `Back`, and the winding
  checked: with the grid's column running east and its row north, triangles are counter-clockwise
  seen from outside if `right = forward × up` as in `params()`; if nothing draws, it is this).
- The soft edge. The mesh is inflated by `mesh.y` = 1.5 px at the limb (`1.5 * frame.z *
  sqrt(d² − R²) / R`), and each vertex carries the old `edge` value of the ray through it against
  the true sphere. The fragment shader's `cover = smoothstep01(-0.75, 0.75, -edge)` is then the
  traced one to within the mesh's sag. The inscribed depth mask (`vs_mask_inscribed`) stays as it
  is and still sits 1.5 px inside.

**Shading.** As the prototype: per vertex the map coordinates (and the clouds' turned longitude),
`edge`, the sun's and viewer's heights, the glint's `n·h`, the sun along east and north, the
forward-scattering lobe, the cloud shadow's offset, the rings' shadow coordinate and strength, and
four colours (sunlight on the ground through the air both ways; the air toward the viewer; sunlight
on the cloud deck; the air's sheen). Per pixel only what a texel changes, plus the terminator's
smoothsteps of the interpolated sun height, which are too sharp to interpolate (a 3° band against
1.9° cells).

**Passes.** One `fs_globe` for every look without a sea. For a look with a sea, `fs_globe` built
with `SEA` off and its `sheen` left out, then `fs_glow` added (`add` blend). Feed `fs_glow`'s
`CLOUDS`/`NIGHT` from the same `SurfaceLook`. The `limb` flag goes (S2), which halves the looks.

**What may differ from today.**

- The limb's last 1.5 px: the prototype multiplies the finished colour by `cover` and writes
  `cover` as alpha; `fs_surface` folds `cover` into the cloud deck's opacity first, which leaves the
  edge a little more opaque than it is covered. The new edge is the more correct one.
- Mip selection is the GPU's own instead of `level()`: the same formula in principle (the larger
  of the two gradients), but expect a slightly different blur toward the limb and the poles.
- Texture magnification near the limb grows by the inflation: under 2 px anywhere.
- With no longitude seam in the derivatives, anisotropic filtering becomes usable again
  (`032c336` turned it off for that seam). Leave it off for S1; it is a separate, measured choice.

**Steps.**

1. `Body.window`/`mesh`, the window and N on the CPU, the index buffer, `counted_draw_indexed` and
   an indexed probe in `gpu.rs` (the estimator needs `covered` for the new draws; the new entry
   points do not go in `estimate.py`'s `BRANCHED` set, since every pixel they reach is shaded).
2. `vs_globe`/`fs_globe` for the airless look first (49 instructions): it exercises the geometry,
   the winding, the edge and the mip selection with almost no shading. Compare pictures of seed 23's
   p3 against the traced one before going on.
3. The other looks, then `fs_glow`. Compare pictures per look.
4. `check.sh . --update`, `pi-estimate.sh`, then the Pi (rule 3).

**Expected.** By instruction counts, at four threads throughout: a sea world's disc from 540–678
at two threads to 217–246; the others 4–6× fewer. Estimator: seed 5 p1 orbit 16.7 → about 4.5 ms.
If two threads cost less than double on the Pi (the estimator's assumption), the ratio shrinks
toward 2.5–3× for sea worlds; it does not vanish.

**Risks.**

- *Interpolation across the terminator and the glint.* The colours through the air change fastest
  where the sun is on the horizon, and the glint is a lobe about 8° across. At N = 96–128 over a
  hemisphere a cell is 1.4–1.9°. If facets show, raise N before moving terms back to the pixel.
- *The rings' shadow at coarse N.* Its coordinate is not linear across a cell; keep N ≥ 32 for a
  ringed world of any size.
- *Triangle throughput.* [perf-bottlenecks.md](perf-bottlenecks.md) finds the V3D's time set by
  thread count and quad throughput. 18 k triangles a pass (half of them culled) is small beside the
  dive's models, but check the bin engine's time on the Pi, not only the render engine's.
- *The close-up passes* (`fs_ground`, `fs_detail`, `fs_clouds`, `fs_cloud_layer`, `fs_glow`,
  `fs_ring_shadow`) are reached only when a body is nearer than 1.6 radii, which the tour's
  keep-clear distances (2.0–2.8 radii) no longer allow for planets. Leave them alone here; say in
  the report whether any frame of a sweep still draws them.

**Effort.** 2–3 days. **Tier.** The strongest model available: geometry, shading and tooling
together.

### S2. The air past the limb as an annulus

**Goal.** Replace `fs_limb` (343 instructions over a fan round the whole disc, which the mask then
rejects inside) and the `LIMB` branch of `fs_surface` with `vs_limb_ring` + `fs_limb_ring` from
[solar-lab/limb.wgsl](solar-lab/limb.wgsl): 128 sides, two bands (from 2.5 px inside the limb to
1.02 radii, then to the top of the air), the light at the ray's closest point worked out per
vertex, eclipses included; per pixel the air's depth along the ray and the soft edge.

**Details.** `add` blend, strict depth test (as `limb` today), 128 × 2 × 6 vertices. Only for a
body wholly ahead of the camera (`inscribable`); otherwise today's `limb` pass. Because eclipses
are in the vertex shader, the split between "limb in the surface pass" and "limb as its own pass
when eclipsed" disappears: drop `SurfaceLook.limb` and its pipelines (9 of the 18 a slot builds).

**May differ.** The glow's falloff is the same function of the same radius; the light along the
limb is interpolated over 2.8° sides, and an eclipse's edge across the air is as soft as a side is
long.

**Expected.** 46 instructions over about 72 k pixels in orbit against 343 (or 75–100 more over the
whole 545 k inside `fs_surface`): −0.3…−0.6 ms, and half the surface pipelines to compile at
start-up. **Effort.** Half a day. **Tier.** Sonnet. Independent of S1.

### S3. Worlds without maps, moons and the star on the mesh

After S1, `vs_globe` serves three more shaders that trace per pixel today:

- `fs_flat` (298): a world whose maps are still streaming in. Per vertex its lit colour through the
  air; per pixel the bands of a giant (two sines of the latitude, which comes in as `uv.y`) and the
  terminator. It is 3.4 ms of the estimate at the start of each transfer.
- `fs_moon` (422): the same as an airless `fs_globe` reading a layer of the moon arrays, plus the
  molten glow in `relief.b`. Moons are small on screen; this is for the pipeline count and the
  occasional moon flyby.
- `fs_star` (243): limb darkening per vertex, the granules' one noise fetch per pixel from an
  interpolated body-frame normal. About 0.6–1 ms in a star flyby.

**Effort.** 1 day. **Tier.** Sonnet, with S1 as the pattern.

### S4. The star's glare off the background

**Now.** `fs_space_baked` is 91 instructions over every pixel no body hides: one cube fetch along
an interpolated ray, then `glare_near` (a normalise, a length, a reciprocal and the colour) and the
jump's flash, for every pixel of the sky whether the star is near it or not.

**Change.** The background reads the cube and nothing else (measure it in the lab; a fetch along an
interpolated ray should be near 20 instructions), with the flash only in a second pipeline used
while `warp.y > 0`. The glare becomes an added polygon round the star, out to where it falls under
half a step of the 8-bit frame (`g * 0.012 * max(colour) < 1.5e-4` in linear light, about 80 disc
radii), with that floor subtracted so its rim does not show. Far from the star that polygon is a
small part of the frame; in a star flyby it is the whole frame and nothing is saved.

**Expected.** −0.5…−1.3 ms on every frame outside a jump (the background is 1.4 ms in orbit and
2.3 ms on a transfer). **May differ.** The faint outer glare, below one step of the display.
**Effort.** Half a day. **Tier.** Sonnet. Independent.

### S5. Jump drives without trigonometry per pixel

**Now.** `fs_warp` takes `atan2` of the pixel, then the cosine and sine of that angle, and a
logarithm of the radius, for every pixel; Lumen Drift needs 43 registers and runs at two threads.

**Change, in two steps; measure after the first.**

1. `around = s / r`: the cosine and sine of the angle are the pixel's own direction. Two sines and
   their registers gone from every drive, for nothing.
2. Draw the jump's background as a polar fan round the heading (256 sectors, rings spaced by a
   constant ratio, the seam's vertices doubled) carrying the angle and the logarithm of the radius
   as interpolated values. Linear interpolation of both is exact to 1e-4 at that density. That
   removes `atan2`, `log` and `length` per pixel.

**Expected.** Lumen Drift under 32 registers (four threads), every drive 60–100 instructions
shorter: −1…−2 ms of 4–8 ms. Jumps are a few seconds a visit and already inside the frame, so this
ranks low. **Effort.** Half a day for step 1, a day for step 2. **Tier.** Sonnet.

### S6. Eclipses on the mesh

`fs_eclipse` (277) traces per pixel under a scissor, only while a shadow falls. On the mesh it
needs the point as an interpolated value and keeps its loop over the shadowing bodies per pixel
(a moon's shadow is too small to light per vertex). Worth doing only if a sweep on the Pi shows an
eclipse frame over budget after S1. **Tier.** Sonnet.

## 4. Considered and not planned

| idea | why not |
|---|---|
| A cube map for the planet maps (no `atan2`, no seam) | The generator works in latitude and longitude throughout (erosion, climate, the mip chain); the mesh gets the same saving without touching it. |
| A lower render scale for the tour | Trades sharpness for time, as for the dive; S1 should make it unnecessary. |
| A G-buffer pass and a shading pass | Two full passes over the disc and a load of the first: the V3D pays 0.4–0.8 ms for a pass and more for a load ([perf-bottlenecks.md §5](perf-bottlenecks.md)). |
| One pass for sea worlds by sending fewer values across | Tried twice in the lab (above): more instructions, the same thread count. |
| Lighting the terminator per vertex | The band is 3° wide; at 1.9° cells it would facet. The prototype keeps it per pixel. |
