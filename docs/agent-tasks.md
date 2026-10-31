# Agent tasks: the Nautilus roadmap

The work queue for agents improving the Nautilus dive. Read this, the rules below, and the docs
they point to before starting a task. Update the **Status** column of your task when you finish
(and add a line under *Notes for the user* if something needs the user).

Background: `docs/nautilus.md` (requirements per scene, how to measure),
`docs/perf-bottlenecks.md` (the 2026-10-31 analysis: where the frame goes, briefs T1–T10, what is
ruled out in §5), `docs/pipeline-optimization.md` (history; check it before re-trying an idea),
`docs/handoff/*.md` (earlier agents' notes per scene).

## State on 2026-11-01

- The Pi runs **OpenGL ES** (`src/gpu.rs::backend_order`): Mesa's GL driver gives the V3D 7.1
  early-Z, its Vulkan driver does not. With the corner mask's `GreaterEqual` test this took the
  dive from a 9.89 to an 8.43 ms median and 12.43 to 10.91 ms worst (30 places). Live, the kiosk
  draws one render job a frame and no TFU uploads (Vulkan: four render jobs and two TFU jobs).
  T1 and T2 of `perf-bottlenecks.md` are therefore done/obsolete; every other number in that
  document was measured on Vulkan without early-Z and must be re-measured before it is trusted.
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
   Docker volume of the checkout's own (named after its directory), so worktrees do not hand each
   other binaries; still `find src -type f -exec touch {} +` before a build and check the
   binary's md5 changed.
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
| I2 | tooling | Per-feature cost benchmark and `docs/feature-costs.md` | Sonnet | todo |
| G1 | tooling | What the GL backend gives us that we build by hand | Sonnet | todo |
| P1 | perf-a | Tunnel's own 4-thread fragment shader (perf-bottlenecks T3) | Sonnet | todo |
| P2 | perf-a | One column per vent plume (T4) | Sonnet | todo |
| P3 | perf-a | Cheaper caustic ceiling (T6) | Sonnet | todo |
| P4 | perf-a | Draw order for early-Z: opaque front to back, re-check §5's draw-order items | Sonnet | todo |
| P5 | perf-b | City split by building part (T5) | Sonnet | todo |
| P6 | perf-b | Glass specialised by material and animation (T7) | Sonnet | todo |
| P7 | perf-b | Start-up: parallel mesh library, drop unused pipelines (T8) | Sonnet | todo |
| P8 | perf-b | Pace the dive from the display (T10) | Sonnet | todo |
| V1 | water | Realistic visibility: shorter draw distance in water haze, detail reinvested near | Opus | todo |
| V2 | water | Water surface seen from below | Sonnet | todo |
| V3 | ground | Ice less blocky | Sonnet | todo |
| V4 | ground | Terrain integration and per-biome placement realism | Opus | todo |
| V5 | life | More wildlife, specific to each biome | Opus | todo |
| V6 | atlantis | At least ten new Atlantis elements | Opus | todo |
| V7 | atlantis | Underwater Greek temple environment | Opus | todo |
| V8 | lostcity | Lost city: travel through large corridors with detailed interiors | Opus | todo |
| V9 | biomes | Three new biomes unlike the current ones | Opus | todo |
| V10 | detail | More detail in every biome with headroom | Sonnet | todo |

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

## Notes for the user

(Things only the user can do; agents append here.)
