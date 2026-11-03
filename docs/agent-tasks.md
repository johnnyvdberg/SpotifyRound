# Agent tasks: the Nautilus roadmap

The work queue for agents improving the Nautilus dive. Read this, the rules below, and the docs
they point to before starting a task. Update the **Status** column of your task when you finish
(and add a line under *Notes for the user* if something needs the user).

Background: `docs/nautilus.md` (requirements per scene, how to measure),
`docs/perf-bottlenecks.md` (the 2026-10-31 analysis: where the frame goes, briefs T1–T10, what is
ruled out in §5), `docs/pipeline-optimization.md` (history; check it before re-trying an idea),
`docs/handoff/*.md` (earlier agents' notes per scene).

## State on 2026-11-01

- The Pi **measures on OpenGL ES, but the kiosk shows Vulkan** (`src/gpu.rs::backend_order`).
  Mesa's GL driver gives the V3D 7.1 early-Z, its Vulkan driver does not: with the corner mask's
  `GreaterEqual` test the headless dive went from a 9.89 to an 8.43 ms median and 12.43 to 10.91
  ms worst (30 places). But wgpu 30's GL backend presents to a Wayland window only when its
  instance is made on the compositor's `wl_display`, which Slint does not hand out before the
  window exists: the GL kiosk deployed 2026-11-01 00:14 rendered every frame and showed none (a
  black screen with the cursor on it). The display runs Vulkan again; tests, benches and previews
  stay on GL. So `bench-pi.sh` numbers are the GL ones, and the kiosk is 0.9-2.6 ms a frame slower
  until D1 lands.
  T1 and T2 of `perf-bottlenecks.md` are done/obsolete for GL only.
  **D1 (branch `d1-display`)**: the display can now run GL (`SPOTIFY_ROUND_DISPLAY=gl`, femtovg
  draws the UI). It is faster on the dive and slower on the globe; the default is still Vulkan.
- The offline shader tool (`deploy/v3d-shaderdb`) compiles through **Vulkan** (v3dv). The GL
  driver uses the same V3D compiler backend, so threads/temps/spills carry over, but confirm a
  borderline shader on the Pi with `V3D_DEBUG=shaderdb MESA_SHADER_CACHE_DISABLE=true
  WGPU_BACKEND=gl` on the test binary (kiosk frozen).

## Rules for every task

1. **Branch and worktree.** Work in your own git worktree on a branch named after the task
   (`p1-tunnel`, `v6-atlantis`, …), branched from the current `main`. Commit there. Do not push,
   merge into `main`, deploy or restart the kiosk: the coordinator does. Author email must be
   `johnnyvdberg@gmail.com` (check `git config user.email`). The repository is **public**: never
   write the Pi's address or user name into tracked files; write `<user>@<pi>`.
2. **Build on the Mac; never run cargo on the Pi.** `deploy/build-for-pi.sh --tests` builds in a
   Docker volume shared by every checkout (the Docker VM's disk is small): always
   `find src -type f -exec touch {} +` before a build and check the binary's md5 changed.
   `deploy/v3d-shaderdb/shaderdb.sh` serialises its own builds with a lock.
3. **The kiosk is the user's live display.** Benchmark only through `deploy/bench-pi.sh run`
   (freezes the kiosk for the run, resumes it on every exit, holds `/tmp/nautilus-bench.lock`
   so parallel agents queue). A hand-written Pi script takes the same lock
   (`exec 9>/tmp/nautilus-bench.lock; flock 9`) and the same `kill -STOP`/`trap … kill -CONT`.
   Do not use `sudo` on the Pi. Clean up what you put in the Pi's `/tmp`.
4. **Measure before and after in one session** with `bench-pi.sh run <task>-before` (the base
   commit's binary) and `run <task>-after`, then `bench-pi.sh compare`. The acceptance run is the
   full default (four visits, every sea); narrow with `DIVE3D_HOTSPOT_SCENES=` while iterating.
   Report median, worst, places over 11 ms and per-sea medians.
5. **Gates on the Mac:** `deploy/v3d-shaderdb/check.sh .` (update the baseline with `--update`
   when the change is intended, and commit it), `cargo test --release`,
   `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all -- --check`.
6. **Pictures.** Before/after previews and screenshots of every sea you touched (`deploy/shots.sh run/compare`, see
   `docs/nautilus.md`). Put the comparison images in your handoff note.
7. **Budget.** The frame must stay under **11 ms at every place** on the Pi (the display's 16.7 ms
   minus the UI and headroom). A visual task may spend headroom a perf task freed, but only with
   its cost measured and switchable (rule 8).
8. **Everything new is measurable and switchable.** Each new element, creature, effect or
   environment gets an entry in the dive's skip lists (`dive3d::part` / `dive3d::effect`, read
   through `DIVE3D_SKIP` / `DIVE3D_SKIP_EFFECTS`) so its cost can be priced and it can be dropped
   without a code change, and its measured cost goes in `docs/feature-costs.md` (task I2).
9. **Maintainable.** Follow the code's existing structure (placers in `world.rs`, meshes in
   `mesh.rs`, shading in `scene.wesl`, tables over special cases). New biomes and environments are
   data plus small functions, not new rendering paths, unless the task says otherwise. Comments
   explain why, as the surrounding code does.
10. **Handoff.** Write `docs/handoff/<task>.md`: what changed, measurements, pictures, what is left.

## Queue

Tasks in one row group edit the same files; one agent does a group in order. At most **two agents
run at once**, from different groups. Model tier is a suggestion.

| id | group | task | tier | status |
|---|---|---|---|---|
| I1 | tooling | Screenshot generation that shows changes at a glance | Sonnet | done (`deploy/shots.sh`, `docs/nautilus.md`) |
| I2 | tooling | Per-feature cost benchmark and `docs/feature-costs.md` | Sonnet | done (`bench-pi.sh features`, `docs/feature-costs.md`) |
| G1 | tooling | What the GL backend gives us that we build by hand | Sonnet | done: nothing to land (`docs/handoff/g1.md`) |
| P1 | perf-a | Tunnel's own 4-thread fragment shader (perf-bottlenecks T3) | Sonnet | done on `perf-a` (cave 0.15 -1.9 ms; `docs/handoff/perf-a.md`) |
| P2 | perf-a | One column per vent plume (T4) | Sonnet | done on `perf-a` (vents -1.4 ms median; the look changes slightly, user to review) |
| P3 | perf-a | Cheaper caustic ceiling (T6) | Sonnet | partly done on `perf-a` (median -0.3 ms, six seas -0.1...-0.5; the rest needs a visible change) |
| P4 | perf-a | Draw order for early-Z: opaque front to back, re-check §5's draw-order items | Sonnet | done on `perf-a` (tunnel after the seabed along its axis, cave -0.55 ms; seabed-first and nearest-first re-measured, not adopted) |
| P5 | perf-b | City split by building part (T5) | Sonnet | done on branch `perf-b` (walls still two threads; `docs/handoff/perf-b.md`) |
| P6 | perf-b | Glass specialised by material and animation (T7) | Sonnet | done on branch `perf-b` |
| P7 | perf-b | Start-up: parallel mesh library, drop unused pipelines (T8) | Sonnet | done on branch `perf-b` (warm 6.7 to 2.7 s, cold 12.6 to 6.6 s) |
| P8 | perf-b | Pace the dive from the display (T10) | Sonnet | implemented behind `SPOTIFY_ROUND_PACE=display`, default unchanged; needs the user to evaluate on the kiosk |
| V1 | water | Realistic visibility: shorter draw distance in water haze, detail reinvested near | Opus | done on branch `water` (median 7.63 -> 6.76 ms with the reinvestment; `docs/handoff/water.md`) |
| V2 | water | Water surface seen from below | Sonnet | done on branch `water` (mirror, facets, Snell window; +0.1 ms median, worst 10.25 ms coral) |
| V3 | ground | Ice less blocky | Sonnet | done, merged into `main` (rounded keels lit through, bergs, floes; `docs/handoff/ground.md`) |
| V4 | ground | Terrain integration and per-biome placement realism | Opus | done, merged into `main` (`world::bed` rules; `docs/handoff/ground.md`) |
| V5 | life | More wildlife, specific to each biome | Opus | done, merged into `main` (`docs/handoff/v5.md`) |
| V6 | atlantis | At least ten new Atlantis elements | Opus | done, merged into `main` (fourteen elements, rolling seabed; `docs/handoff/v6.md`) |
| V7 | atlantis | Underwater Greek temple environment | Opus | done, merged into `main` (octastyle temple the boat sails through; `docs/handoff/v7.md`) |
| V8 | lostcity | Lost city: travel through large corridors with detailed interiors | Opus | done, merged into `main` (the drowned city's galleria; `docs/handoff/v8.md`) |
| V9 | biomes | Three new biomes unlike the current ones | Opus | done, merged into `main` (meadow, glass sponges, blue hole; `docs/handoff/v9.md`) |
| V10 | detail | More detail in every biome with headroom | Sonnet | done, merged: near detail, faceted rocks, kelp, seagrass, glass sponges, coral, the volcano, the abyss's glow and the forest's frond crowns, three mangrove stands (`docs/handoff/v10.md`) |
| M1 | models | Imported models: Quaternius (CC0) as the house source for animals and props, SMK scans for statues | Opus | done on branch `m1-gltf`: glTF reader baking textures to colours, LODs for imports, Poly Haven bronzes (Atlantis), chests and barrels (wrecks); Quaternius's Drive still over quota 2026-11-02; rocks left for after T1 (`docs/handoff/m1.md`) |
| M2 | models | Quaternius ships as worn wrecks | Opus | done on branch `m2-wrecks` with Poly Haven's Dutch ships (Quaternius's Drive over quota): holed, silted, listing, masts snapped; wrecks +0.10 ms median, worst 8.91 (`docs/handoff/m2.md`) |
| M3 | models | A drowned colonial fort from Poly Haven's modular fort, with its cannon | Opus | done on branch `m3-fort`: half way through the wrecks, tower, gate, ramparts sunk and toppled, eight cannon down the bank; +0.35 ms median where in view, worst 0.65 (`docs/handoff/m3.md`) |
| D1 | display | OpenGL ES on the kiosk's display, for its early-Z | Opus | done on branch `d1-display`: femtovg vendored and specialised per pipeline; live GL beats Vulkan + Skia on the dive (61 fps both, 1.0-1.6 ms less GPU a frame) but the globe drops to 53 fps, so the default stays Vulkan; `SPOTIFY_ROUND_DISPLAY=gl` switches (`docs/handoff/d1.md`) |
| T1 | terrain | Terrain noise from FastNoiseLite | Opus | done on branch `t1-terrain`: warped OpenSimplex2 height, dunes (shallows, wrecks, sargasso), ridged rock (abyss, vents), Voronoi brine terraces and pools; Pi median 7.14 -> 7.23 ms, worst 10.48, none over 11; seabed build 96 -> 95 ms (`docs/handoff/t1.md`) |
| D2 | display | The globe at full speed on GL, so GL can be the display's default | Opus | todo |

Order: tooling and perf first (they make room and make change visible), then V1 (sets the visible
range everything else is built for), then V3/V4, V5, V6/V7, V8, V9, V10.

## Task descriptions

### I1. Screenshot generation that shows changes at a glance
Make it one command to render a fixed, deterministic set of views of every sea (same seeds and
camera times every run), write a contact sheet per run, and compare two runs: side-by-side pairs,
an amplified difference image, a per-view change score, sorted most-changed first, in one HTML page
(`target/shots/<run>/index.html`, `target/shots/compare-<a>-<b>.html`). Commit the comparison
script (the untracked `target/compare_frames.py` in the main checkout is a start). Cover every
leg including new ones automatically (driven by the voyage's leg list, not a hand list). Optionally
render on the Pi through the test binary (GL), so driver differences show. Document in
`docs/nautilus.md`.

### I2. Per-feature cost benchmark
A `hotspots`-style test (or a mode of `effect_cost`) that, for every entry in `dive3d::part` and
`dive3d::effect`, measures the frame with and without it at the places where it is visible, and
writes a table: feature, seas where it appears, median and worst ms it costs. A wrapper
`deploy/bench-pi.sh features TAG` runs it and saves the table; `docs/feature-costs.md` holds the
latest table and how to regenerate it. Every later task adds its features here.

### G1. What the GL backend gives us that we build by hand
Now that the Pi runs wgpu's GLES backend: list the GPU features it exposes on the V3D
(`adapter.features()`, limits, texture formats) and find where the dive hand-builds something the
hardware or driver does better. Candidates: ETC2/EAC compressed textures for the baked atlases
(less TMU bandwidth; the TMU is busy), hardware mipmap generation, texture arrays vs atlases,
sampler states, `textureGather`, instancing limits. Prototype the most promising one, measure on
the Pi, and either land it or document the result. Keep Vulkan working (it is the fallback and the
shaderdb path).

### P1–P8. Performance
P1 = `perf-bottlenecks.md` T3, P2 = T4, P3 = T6, P5 = T5, P6 = T7, P7 = T8, P8 = T10: follow those
briefs, but re-measure the starting point on GL first (rule 4), since early-Z changed what is
expensive. **P4**: with early-Z working, order the opaque draws so the nearest, largest occluders
go first (the seabed and big models front to back; the full-screen water last among opaques, as
it already is if so), and re-check the draw-order experiments §5 lists as tried without early-Z.

### V1. Realistic visibility under water
Real water limits sight to tens of metres, less in murky seas. Give every sea a visibility
distance that fits it (clear tropical shallows far, the bloom and the brine murky, the abyss short
and dark) and make the haze reach full colour at that distance, so nothing beyond it is drawn:
cull placement, models and terrain tiles there. Spend the time this frees on detail within range
(denser near placement, higher LOD nearer, more seabed resolution near the window). Measure the
saving first, then what the reinvestment costs. Check every sea's look with I1.

### V2. The water surface from below
Improve how the surface reads from below in the shallow seas: the bright Snell window, the
mirror of the sea outside it, moving wave facets and the caustic light under it. Within budget.

### V3. Ice less blocky
The polar sea's ice reads blocky: smoother, rounded floe and berg shapes with more vertices near
the window (LOD), softer normals, a translucent blue-white sub-surface tint and frost texture.

### V4. Terrain integration and placement realism
Objects should look bedded into the ground: sinking into sand, aligned with slope where they
would be, sediment drifts against their up-current side, rock outcrops that rise out of the
terrain instead of standing on it, scour and debris round wrecks and ruins. Then go through every
sea with I1 screenshots and fix placements that look wrong for that biome (floating, intersecting,
implausible density, wrong depth or slope, unrealistic grouping), writing the rules down as data.

### V5. More wildlife
Add creatures that belong to each sea (for example: reef fish shoals, rays, turtles and reef
sharks in the shallows and reef; garibaldi and sea lions in the kelp; anglerfish, gulper eels and
siphonophores in the abyss; tube-worm shrimp and crabs at the vents; seals, narwhals and
krill in the polar sea; eels and lobsters in wrecks), with believable movement (schools, hovering,
bottom-walking) and LODs. Keep species as data so adding one is a table entry plus a mesh.

### V6. Atlantis: at least ten new elements
Add ten or more new things to Atlantis that fit its look (for example: a fallen colossus, an
orichalcum-clad wall, concentric canal rings, a harbour with sunken triremes, a bronze gate,
statues on plinths, mosaic floors, an aqueduct, a lighthouse ruin, amphora fields, a great stair,
inscribed stelae), placed as a coherent city plan, not scattered.

### V7. Underwater Greek temple
A new environment (its own leg or a place within Atlantis' leg, whichever fits the voyage): a
Greek temple on the seabed, with stylobate and steps, Doric columns (some fallen, drums scattered),
entablature and pediment fragments, a cella with a statue, sea life colonising it, light shafts
through the colonnade. The boat passes along and through it.

### V8. Lost city traversal
A sequence where the boat travels through a lost city's large corridors and halls: long
colonnaded avenues and covered galleries wide enough to pass, with detailed interiors seen through
openings (rooms, stairs, furniture, collapsed floors, light from openings above). It needs the
path (`docs/handoff/path.md`, `helm.rs`) to thread the corridors and the camera's view never to
clip through walls (`docs/nautilus.md`, "What the window must never show").

### V9. Three new biomes
Three seas unlike the current fourteen. Suggestions (the agent may choose others, documented why):
a seagrass meadow with mangrove roots (bright, green, shallow), a glass-sponge reef (cold, deep,
ghostly lattices), a blue hole (a vertical sinkhole with stalactites and a halocline). Each with its
own terrain, placement, palette, light and wildlife, added to the voyage, `docs/nautilus.md`
requirements, previews and benchmarks.

### V10. Detail where there is headroom
With the feature costs (I2) and the per-sea timings, add detail to every sea that has room under
the 11 ms budget: more variety in placement, small props, textures, motion. Stop where the budget
ends, and record what each addition costs.

### M1. Imported models

The user wants open models in place of hand-built ones wherever a better one
exists, from one consistent source: Quaternius (CC0, coloured low-poly,
animated; e.g. https://quaternius.com/packs/cutefish.html). Statues are scans
of the SMK's public-domain casts (`assets/models/CREDITS.md`). List each
original in `assets/models/models.tsv`; `deploy/models.sh` fetches it into
`target/models/` and reduces it with `tools/decimate`, so a higher quality
level later is a bigger triangle count and a rerun. Keep the procedural model
as the template the import is fitted to (`cast` in `mesh.rs`), and keep its
animation working (`aux`). Each import gets a skip switch and a measured row.

Second source for scans, from https://github.com/madjin/awesome-cc0: Poly Haven
(CC0, photo-scanned and textured, open API at `api.polyhaven.com`, no login).
Useful: `coast_rocks_*`, cliffs and boulders (for the rocks task), ceramic and
antique vases (amphorae), barrels, `cannon_01`, lanterns, chairs and tables (the
galleria's rooms). The 1k glTF shares the full-resolution mesh (about 1.3 M
triangles for a rock), so `tools/decimate` needs a glTF reader that bakes the
diffuse texture into vertex colours. Kenney (direct CC0 zips) is the fallback. Poly Haven also has `bronze_ray_statue`,
`bronze_shark_statue` and `bronze_whale_statue` (Atlantis) and `dutch_ship_large_01/02` and
`dutch_ship_medium` (M2's wrecks); it has no coral or sponges.
Meshy has no credits left.

### M2. Ships as wrecks

Quaternius's ships pack (https://quaternius.com/packs/ships.html) as wrecks:
worn down first, with broken masts, holed and listing hulls, growth and silt,
bedded by `world::bed` rules like the procedural wrecks.

### M3. A drowned fort

The user likes Poly Haven's Modular Fort 01 (https://polyhaven.com/a/modular_fort_01,
CC0, Rico Cilliers, 28 k triangles, textured stone, plaster and trim; part of the
"smugglers cove" collection with `cannon_01` and barrels). Build a drowned
colonial fort from its modules: ramparts, a gate and a corner bastion, half
buried and fallen, with cannon spilled down the slope. It belongs in the wrecks
(a fort above the anchorage the wrecks were lost in), or on the blue hole's bank
(V9). It needs M1's glTF reader with the textures baked to vertex colours, and
`world::bed` seating like the ruins. Skip switch and measured row as usual.

### D1. OpenGL ES on the display

The user wants GL's speed on the kiosk, not only in the benchmarks. Two things stand in the way.
Presenting: wgpu 30's EGL backend shows frames in a Wayland window only from an instance made on
that window's `wl_display`; branch `display-gl` (0fe8015) vendors `i-slint-backend-winit` 1.18.1
with `request_graphics_api_for_display` and opens the device there, and that works. Drawing the
UI: Slint's Skia renderer runs on a wgpu device only on Vulkan, Metal or DX12, so a GL display
needs femtovg, and femtovg costs more than early-Z saves. Live under cage at full speed on the
dive: Vulkan + Skia 61 fps (vsync), GL + femtovg 47, Vulkan + femtovg 28-39. Ways on: Skia's own
GL surface sharing the dive's texture through an EGL image (`opengl_surface.rs` in
`i-slint-renderer-skia`), or finding what femtovg spends its time on (most likely the 1080 x 1080
dive image drawn through its one large shader every frame) and drawing that image with a plain
textured quad. Measure live, not headless: the window's own drawing is the question.

### D2. The globe on GL

D1 (`docs/handoff/d1.md`) made GL + femtovg faster than Vulkan + Skia live on the dive (1.0-1.6 ms
less GPU a frame, 61 fps), but the globe drops to 53 fps on GL (17.6 ms GPU against 15.1 on
Vulkan, about 7 long gaps a second): its frame shader costs more on GL, gains nothing from early-Z
and has no adaptive scale. Make the globe hold 61 fps with headroom on GL: get its fragment shader
to 4 threads (`deploy/v3d-shaderdb`, confirm on the Pi through GL), split or specialise it as the
dive's shaders were, give it the adaptive scale, or whatever the measurements point to. Keep the
globe's look (before/after pictures). Measure live with `deploy/live-pi.sh` on both displays, the
globe and the dive. When GL is better on every scene, set `DISPLAY_GL_DEFAULT = true` in
`src/gpu.rs` and say so in the handoff; Vulkan stays reachable with `SPOTIFY_ROUND_DISPLAY=vulkan`.

### T1. Terrain noise from FastNoiseLite

The user asks for FastNoiseLite (https://github.com/Auburn/FastNoiseLite, MIT; Rust crate
`fastnoise-lite`) for better terrain. Use it for the seabed's heightmap and shape noise in
`world.rs`/`terrain.rs`: OpenSimplex2 with domain warp for the dunes, ridged fractal for the
ridges and rocky seas, cellular (Voronoi) for cracked mud, brine pools and polygonal terraces.
The heightmap is built on the CPU, so the GPU cost changes only through the shape; keep each
sea's character, compare pictures before and after for every sea, and keep the seeds
deterministic (`places.rs` screenshots must still find their places).

## Notes for the user

(Things only the user can do; agents append here.)

- **D1, GL on the display (branch `d1-display`).** Live under cage, GL + the specialised femtovg
  matches Vulkan + Skia's 61 fps on the dive with 1.0-1.6 ms less GPU a frame, but the globe falls
  to 53 fps (Vulkan 61). The default stays Vulkan. To prefer the dive, add
  `Environment=SPOTIFY_ROUND_DISPLAY=gl` to the kiosk unit's drop-in and restart; remove it if the
  screen ever shows only the cursor. Numbers in `docs/handoff/d1.md`.

- **Deploy (coordinator, 2026-11-01).** The kiosk still runs the first GL build (dba5471). `main`
  at the wave-1 merge measured on the Pi, 168 places: median 8.30 -> 7.65 ms, worst 10.88 ->
  10.55 ms, every sea faster; warm start 6.7 -> 2.7 s. Deploying was refused by the coordinator's
  permission settings, so: `deploy/build-for-pi.sh`, copy `target/pi/spotify-round` over the
  kiosk's binary (keep the old one, e.g. `spotify-round.before-wave1`) and
  `sudo systemctl restart cage-kiosk`. Then the P8 check below.
- **Docker disk.** The Docker VM's disk (59 GB) was full; the coordinator removed only this
  work's own build volumes. About 14 GB of unused anonymous volumes and 33 GB of unused images
  remain, some from other projects: review with `docker system df -v` and prune what you don't
  need, or grow the VM disk in Docker Desktop.

- **P8, pacing from the display (branch `perf-b`).** Not evaluated live. To try it: deploy the
  build, run the kiosk twice for a minute each with `SPOTIFY_ROUND_SCENE=nautilus` and
  `--log spotify_round::visuals=debug` (the second time also `SPOTIFY_ROUND_PACE=display`). The
  once-a-second `visualizer` line then carries `taken` (frames the UI showed), `short_gaps` (two
  takes under 10 ms apart: a dive frame that was never shown) and `long_gaps` (over 25 ms: a frame
  shown twice). The timer should show about 4 long gaps a minute and the display pacing 0 to 1;
  also watch a slow pan for stutter. If the display mode ever looks worse (the UI thread's takes
  not following the refresh), unset the variable: nothing else changed.
- **P5/P6/P7** change pipelines and the mesh library only; the Pi numbers are in
  `docs/handoff/perf-b.md`. The new pipelines are not new features, so nothing was added to the
  skip lists.
- perf-a (2026-11-01): review the vent plumes' look (`docs/handoff/img/perf-a-vents.jpg`). The Docker
  VM's disk filled up twice during the work (reclaim with `docker system prune`), and the Pi's `/tmp`
  is crowded with other runs' 47 MB test binaries, which `bench-pi.sh` never removes.
- water (V1/V2, 2026-11-01): look at the seas' new sight (`docs/handoff/img/water-murky.jpg`,
  `water-clear.jpg`): the far distance is hazier everywhere and gone beyond each sea's sight
  (`world::SIGHT`, a table in `docs/nautilus.md`), the abyss's far wall and Atlantis' farthest
  temples among it; tune a sea by its one number. The surface from below
  (`water-surface.jpg`) is best judged live, since its facets move.
- models (M1-M3, 2026-11-02): three branches stacked `m1-gltf` -> `m2-wrecks` -> `m3-fort` (merge
  `m3-fort` to take all three). Look at the bronzes (`docs/handoff/img/m1-bronzes.jpg`), the
  Dutch wrecks (`m2-wrecks.jpg`) and the fort (`m3-fort.jpg`). Quaternius's Google Drive still
  answers "Quota exceeded" for every file, so nothing came from the house source: when it works
  again, its ships and fish are `models.tsv` rows. `deploy/models.sh` now fetches Poly Haven
  assets (`polyhaven:ASSET#NODES`).
