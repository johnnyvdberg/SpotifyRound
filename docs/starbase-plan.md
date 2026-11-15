# The starbase: a plan

A plan for implementation agents, not a record of work done. Nothing here is wired into the app.

In some systems of the [solar tour](solar.md), the tour visits a starbase the size of a moon. The
ship flies into a hangar mouth on its surface and down a long tunnel. Windows in the tunnel walls look
out on a neon city that fills the base's hollow interior. The ship docks in a spaceport bay, where
robot arms, welders and fuel lines service it while a HUD panel ticks through the work. Then it undocks,
turns about, flies back out through the tunnel and leaves on its next transfer. It has to run within
the Pi 5's frame budget, so the expensive parts are baked once or faked with parallax, and everything
moving is a handful of instanced meshes and sprites.

## 0. What the code already gives us

| Need | Already there | Where |
|---|---|---|
| A body you can fly to, label and put on the minimap | body `Kind`s, stops (`Stop { planet, moon }`), transfers with capture burns, labels, minimap | `system.rs`, `tour.rs`, `labels.rs` |
| A scripted sequence that depends only on the time | encounter scripts that return a `Shot` (the ship, where it looks, its roll, alert, shake, objects) | `voyage.rs` |
| Procedural meshes, seeded, cached on disk | surface-nets distance-field models with parts (`HULL`, `WINDOWS`, `ENGINE`, …), built on a worker thread | `models.rs` (`ring_station` is the nearest relative) |
| Instanced models with per-part lighting | `vs_model` / `fs_model`, families 1–5 | `encounter.wesl` |
| Sparks, glints, bursts, shields, shake | `vs_fx_spot`, `fs_fx_glint`, `fs_fx_burst`, `fs_fx_jolt` | `encounter.wesl` |
| Lit windows and neon, on a budget | the drowned city's `city_light` and `mesh::city` parts | `dive3d/mesh.rs`, `scene.wesl` |
| Maps painted on the GPU a few rows a frame | moon and planet map generator | `stream.rs`, `generate.wesl` |
| Baking once per system | starfield, sky, craters, gas | `bake.rs` |
| A progress panel with results | the orbital scanner | `scan.rs` |
| Importing CC0 models as vertex-coloured OBJ | `tools/decimate`, `assets/models/models.tsv`, `CREDITS.md` | `tools/`, `assets/models/` |
| Pi limits | 16 textures a stage, 16 KiB uniforms, **no early-Z on Vulkan**, many small triangles are expensive | `architecture.md`, `perf/solar-perf-plan.md` §0 |

The plan reuses all of these. It adds one body kind, one phase family, one tunnel mesh, one window
shader, one bake and one model family.

## 1. The sequence (about 150 s)

| # | Stage | s | What the camera sees | How it is drawn |
|---|---|---|---|---|
| 1 | **Approach** | (transfer) | The base grows from a dot into a moon: a faceted grey sphere crossed by panel seams and lit traffic lanes, with grids of city lights on its night side. A lit hangar mouth turns into view. | An ordinary transfer to a stop. The surface is a new map kind in `generate.wesl` (§3.1), drawn by the moon path. |
| 2 | **Entry** | 12 | Guidance chevrons run toward the mouth. The ship slows, rolls level and passes through a shimmering atmosphere field into the tunnel. | Mouth model (family 6), chevron sprites (`fx_spot`), field as one `fs_gate_field`-style disc. The body fades to the interior set as the camera crosses the mouth. |
| 3 | **Tunnel** | 25 | A ribbed tunnel with strips of windows on both sides and overhead. Behind the glass is a deep neon city of towers, sky bridges and flying traffic, with true parallax. It curves gently, so the end is never in view. | Instanced tunnel segments (§3.2). Window panes ray-trace 3 nested parallax cylinders, each sampling a baked city layer (§3.3). Traffic is glow sprites. |
| 4 | **Dock** | 15 | The tunnel opens into a spaceport bay: other ships in berths, gantries and beacon lights. The ship is guided into a cradle, and clamps close with a thud. | Bay model, berths are reused battle ship meshes, clamp shake through `Shot::shake`. |
| 5 | **Service** | 40 | Robot arms swing over the nose. Welding arcs throw sparks and briefly light the canopy. A fuel line couples, and a scan plane sweeps the hull. A **SERVICE** panel ticks: hull plating, coolant, reaction mass, drive coil, nav sync, plus any incident from this system (§4). | Arms are jointed instances, posed in Rust each frame. Sparks are `fx_spot` with gravity. The arc light adds to the bay's ambient. The panel uses the scanner's layout. |
| 6 | **Undock** | 12 | Clamps release. The ship backs out on thrusters, yaws 180° with the bay sweeping past, and lines up on the tunnel. | Pure camera script. |
| 7 | **Exit** | 20 | Back down the tunnel faster, the city streaking past. It bursts out of the mouth into space, with the star's glare as the eye adjusts, then starts the next transfer. | Same as stages 2–3 in reverse. A white `flash` and the existing glare carry the cut. |

Stages 2–7 make one scripted phase, as a stopover is scripted (§2). Stage 1 is a normal transfer,
so the existing departure, capture and labelling machinery all work unchanged.

**Design note.** Keep the base clearly our own design, not a famous film station: a faceted or
banded sphere with several small ports and lit lanes. Avoid a single concave weapon dish and an
equatorial trench.

## 2. Timeline and data (B1)

- `system.rs`: a `starbase` flag on a moon made a world (done in B1, painted as `Kind::Barren` until B2 gives it a map kind of its own). It is placed as a moon of a giant, or as a stop of its own
  round the star when the system has no giant. Seed it in about one system in four (`STARBASE_CHANCE`).
  Give it a name from a short list ("Halcyon Yard", "Meridian Deep", …), a radius of 900–1600 km, and
  a seed for the city palette and layout.
- `tour.rs`: after the transfer to the base, push `Phase::Base(BaseStage)` segments in place of
  `High`/`Depart`, with `BaseStage = Entry | Tunnel | Dock | Service | Undock | Exit`. The next
  transfer leaves from the mouth's position, outbound.
- New `src/solar/starbase.rs`: `fn shot(base: &Starbase, stage, local) -> BaseShot`. It works like
  `voyage::shot`: a pure function of time in the **base frame** (the mouth at the origin, the tunnel
  along +z, then the bay). It returns the camera, roll, shake, flash, the arm poses, the clamp state,
  spark emitters and the panel's progress.
- The tunnel's centreline is a fixed Catmull–Rom spline (`tour.rs` already has `catmull_rom`) with a
  gentle S-bend. The camera's speed along it follows `smoothstep` ease-ins.
- `View` gets `base: Option<BaseScene>`, like `scene: Option<Scene>` for stopovers. While it is
  `Some` and the camera is inside, the system's bodies, star and sky are **not drawn** (`deep`-style
  culling), so the tunnel never pays for them.
- Test hooks: `SOLAR_STARBASE=1` forces a base into the first system and the itinerary, as
  `SOLAR_ENCOUNTER` does, and `SOLAR_STARBASE_AT=<stage>` jumps to a stage.

## 3. Rendering

### 3.1 The exterior (B2)

- A new map kind in `generate.wesl`: a lattice of panel plates of three sizes (cellular noise
  quantised to a grid on the cube-sphere), recessed seams, a few broad bands and lane lines. An
  emissive channel holds city-light grids and lane lights, lit only on the night side as Earth's
  night lights are in `globe.rs`.
- The mouth is a mesh (§3.4) placed on the surface, so it is sharp close up while the map handles
  the far view.
- Cost: the same as a mapped moon, which the S1–S3 mesh plan in `perf/solar-perf-plan.md` already
  made cheap. No new full-screen pass.

### 3.2 The tunnel (B3)

- One segment mesh, 40 m long, built by surface nets like `ring_station`: an octagonal or hexagonal
  bore with a rib, deck plating, two window strips and an overhead strip, and light bars. Window
  panes are their **own flat quads**, not the surface-net surface, so they have exact planes and UVs
  for the window shader.
- Drawn instanced along the spline: about 14 segments in view, with fog that hides the far end and
  pop-in. Keep it under 3k triangles a segment, and large ones (the Pi's binner charges for many
  small triangles, §0).
- **Draw order matters on the Pi**: Vulkan on V3D runs no early-Z, so every fragment that is drawn is
  paid for. Draw the walls first (opaque, cheap, per-vertex light, light bars emissive), then the
  panes. The panes cover only their own pixels, so the city costs nothing outside the windows. The
  existing depth annulus that masks the round frame's corners stays first.

### 3.3 The city behind the glass (B4). The main trick.

Use **interior mapping on cylinders** instead of geometry. Each pane pixel casts the view ray
outward from the tunnel axis and intersects it with three virtual cylinders round that axis at radii
`R1 < R2 < R3` (near facades, mid skyline, far haze and mega-towers). On each hit it samples a baked
layer texture at `(angle, z)`, front to back with alpha, and stops when the colour is opaque. This
is van Dongen's interior mapping, with cylinders where he used room boxes. The tunnel runs along the
cylinders' axis, so each intersection is a 2D quadratic: about 10 instructions each.

- **Bake** (`bake.rs`, once when the base comes into the itinerary): three RGBA8 layers,
  1024 × 256 each (3 MB), in one array texture (1 binding, which matters with 16 a stage). They are
  painted procedurally in a bake shader: building columns with hashed heights and setbacks, window
  grids with hashed lit/unlit cells in warm and cool whites, neon bands and vertical signs in the
  base's palette (magenta, cyan, amber), sky bridges, and haze increasing with the layer. The layers
  wrap in angle and repeat in `z` with a per-layer offset, so the repeat never lines up.
- **Motion**: a few dozen traffic glows on lanes between the cylinders (`fx_spot`, depth-tested
  against the panes, so they show only through glass). A hashed per-cell flicker on the near
  layer's signs comes from one time uniform.
- **Glass**: a faint reflection of the tunnel's light bars (a scrolling 1D gradient, not a probe),
  a Fresnel tint and grime in the corners.
- **Budget**: aim for `fs_pane` ≤ 80 instructions at 4 threads (check with `deploy/v3d-shaderdb`).
  Panes cover roughly 20–30 % of the circle.
- **Fallback for lower quality**: drop to two layers, or one layer with parallax only.
- Better-looking skylines, optional: render CC0 building kits (§5) into the layers **offline** with
  `tools/` and ship the three layers as PNGs (about 1 MB). The runtime cost is the same, and the
  silhouettes look better than hashed boxes.

### 3.4 The spaceport bay and the service (B5–B6)

- **Model family 6, "base"**, in `models.rs` (bump `GENERATOR`): the mouth, the bay shell (a
  flattened dome with berths and gantries), the cradle with 4 clamps, and a robot arm link (a
  capsule-and-box segment, 3 sizes) with a tool head (welder, fuel coupler, scanner). New parts:
  `CLAMP`, `HAZARD` (painted stripes), `SCREEN`. Lit by the existing `fs_model` part logic, plus
  arc-light ambient from a uniform.
- **Arms**: 2–3 arms of 3–4 links each, posed by forward kinematics in `starbase.rs` from keyframed
  joint angles, written as `models::Instance` (it carries a quaternion). That is about 12 instances.
  Each tool tip feeds a spark emitter.
- **Sparks**: up to about 150 `fx_spot` particles with a ballistic path computed in the vertex
  shader from (birth, velocity), so nothing is simulated on the CPU. Each weld pulse also raises an
  arc flash in the bay ambient and puts a glint on the canopy.
- **Other ships in berths**: reuse 2–4 of the battle fleets' meshes (`DESIGNS × VARIANTS`), parked
  and unlit but for their windows.
- **The SERVICE panel**: `scan.rs`'s layout and timing with new rows, for example
  `HULL PLATING 71% → 100%`, `COOLANT FLUSH`, `REACTION MASS 18 t`, `DRIVE COIL CALIBRATED`,
  `NAV SYNC`, and amber or green states on the rim alarm as each finishes.

## 4. Integration (B7)

- **Ship's computer** (`console.rs`): NAVIGATION lists the base when the system has one ("dock at
  Halcyon Yard"). Choosing it routes the ship there next.
- **Incidents tie-in**: a hull breach, coolant leak or loose ship's cat earlier in the visit adds a
  service row that resolves it ("HULL BREACH, DECK 3: SEALED", "SHIP'S CAT: RECOVERED FROM GANTRY").
  A small change that makes the visit feel connected.
- **Minimap and labels**: the base is a stop, so it is labelled and drawn like a moon. Inside, the
  minimap shows the tunnel and bay as a schematic (optional).
- **Without models**: until the model library is built, a system rolls no base, as
  `Passage::without_models` handles encounters.

## 5. Open-source resources

Kenney's packs are left out on purpose: their art style does not fit the tour.
Everything in the shipped build should be CC0 or public domain, as `assets/models/CREDITS.md`
already requires. Bring imports in through `tools/decimate` (glTF → decimated, vertex-coloured OBJ)
and list each in `models.tsv` and `CREDITS.md`.

**Models: spaceport, props, arms** (for props and offline reference. Keep the base itself
procedural, so it matches the seeded variants and the scene's scale.)

| Resource | Licence | Use |
|---|---|---|
| [Quaternius Modular Sci-Fi MegaKit](https://quaternius.com/packs/modularscifimegakit.html) ([OGA](https://opengameart.org/content/modular-sci-fi-megakit), [80.lv](https://80.lv/articles/a-free-modular-asset-pack-with-over-270-game-ready-models/)) | CC0; 270+ models, glTF | Corridor and wall reference for the tunnel and bay, props |
| [Quaternius Sci-Fi Essentials Kit](https://quaternius.com/packs/scifiessentialskit.html) ([itch](https://quaternius.itch.io/sci-fi-essentials-kit), [OGA](https://opengameart.org/content/sci-fi-essentials-kit)) | CC0; glTF | Robots (service drones), animated screens, crates on the bay floor |
| [Quaternius LowPoly Modular Sci-Fi Environments](https://opengameart.org/content/lowpoly-modular-sci-fi-environments) | CC0; FBX/OBJ/Blend | More interior props |
| [Quaternius LowPoly Spaceships](https://opengameart.org/content/lowpoly-spaceships-pack) | CC0; glTF | Ships parked in berths (or reuse ours) |
| [Quaternius Ultimate Space Kit](https://quaternius.com/packs/ultimatespacekit.html) ([OGA](https://opengameart.org/content/ultimate-space-kit-by-quaternius), [Sketchfab](https://sketchfab.com/3d-models/ultimate-space-kit-84c108ff2bcf4d4cbf2adff74a942822)) | Stated as CC0, but Sketchfab's licence field says CC-BY. **Check before use** | Mechs, service robots |
| [Mega modular hangar kit (jamzuz)](https://jamzuz.itch.io/modular-lowpoly-interior-assets) | Paid ($4); textures CC0, model licence unstated | **Skip**: not clearly open |

**City: building silhouettes for the baked layers** (rendered offline into the three layers, so
their polygon count never reaches the Pi)

| Resource | Licence | Use |
|---|---|---|
| [Quaternius Downtown City MegaKit](https://quaternius.com/packs/downtowncitymegakit.html) | CC0; 300+ modular pieces, most of them free (some behind Patreon tiers) | Tower and block silhouettes, then restyled with neon in the bake |
| [Quaternius LowPoly Buildings](https://opengameart.org/content/lowpoly-buildings-pack) | CC0; palette swaps through atlas textures | More variety |
| [Frosty Cyberpunk Interior Pack (rubberduck)](https://rbduck.itch.io/cold-cyberpunk-asset-pack) | CC0 | Pipes, vents and signage for the bay and the near city layer |

No CC0 cyberpunk *city exterior* kit turned up. The cyberpunk look comes from the bake: neon bands,
vertical signs, haze and palette. Paid kits on CGTrader are royalty-free but not open, so leave them
out.

**Textures** (optional. The bay can stay vertex-coloured like the rest of the tour. One 512² detail
atlas at most.)

| Resource | Licence | Use |
|---|---|---|
| [ambientCG](https://ambientcg.com) (search "sci-fi panel", "metal plates", "diamond plate") | CC0 | Panel and plate detail for the bay floor and clamps |
| [3dtextures.me sci-fi tag](https://3dtextures.me/tag/sci-fi/) ([Metal Plate 049](https://3dtextures.me/2026/04/20/metal-plate-049-free-seamless-pbr-texture/)) | CC0 (per listing) | Alternatives |

**Technique references** (read for the method. Write our own WGSL.)

| Reference | What to take |
|---|---|
| Joost van Dongen, *Interior Mapping* (CGI 2008): [blog](https://joostdevblog.blogspot.com/2018/09/interior-mapping-real-rooms-without.html), [Game Developer](https://gamedeveloper.com/programming/interior-mapping-rendering-real-rooms-without-geometry), [80.lv](https://80.lv/articles/interior-mapping-rendering-real-rooms-without-geometry) | The ray-against-virtual-surfaces idea behind §3.3 |
| [DagorEngine interior mapping docs](https://gaijinentertainment.github.io/DagorEngine/assets/shaders/rendinst_interior_mapping.html) (open-source engine) | Cubemap variant, several rooms a window, random rotation per room |
| [Godot fake-interior shaders](https://godotshaders.com/shader/fake-window-interior-shader/) ([tuned variant](https://godotshaders.com/shader/fake-3d-interior-projection-shader-fixed-tuned/)) | Compact working code to compare against. Check each shader's licence |
| *Visualizing building interiors using virtual windows*, SIGGRAPH Asia 2014 ([paper](https://unpaywall.org/10.1145%2F2669024.2669029)) | Baked light inside windows |

**Avoid** copying Shadertoy code: its default licence is CC BY-NC-SA, which is not compatible with a
CC0 asset policy.

## 6. Budget on the Pi 5

The tour now runs 5–8 ms in orbit and peaks near 12–15 ms in the spool-up (`perf/solar-perf-plan.md`
§0). The base's target is **≤ 8 ms at its worst frame**, inside the bay with sparks.

| Draw | Estimate | Why it is cheap |
|---|---|---|
| Tunnel walls | 1.0–1.5 ms | Per-vertex light, emissive bars, big triangles, about 40k triangles in view at most |
| Panes and city | 1.5–2.5 ms | Only pane pixels; 3 cylinder hits and 3 fetches from one array texture |
| Traffic and sparks | ≤ 0.5 ms | About 200 small additive sprites |
| Bay, arms, berths | 1.5–2.5 ms | Surface-net meshes with LODs, already the pattern for fleets |
| HUD panel | as the scanner | Slint, unchanged path |
| System bodies, sky | 0 inside | Culled while `View::base` is inside |

Risks: no early-Z on Vulkan (handled by draw order and panes as separate quads); too many small
triangles (keep segments coarse, LOD the bay); a texture-binding squeeze (one array texture for the
city); bake time at system arrival (spread it across frames, as `stream.rs` does).

## 7. Stages for implementation

| | Stage | Done when |
|---|---|---|
| B1 ✓ | Body kind, `Phase::Base` stages, `starbase.rs` camera script, `SOLAR_STARBASE`; the way in staged on a docking panel, with a hold when other traffic has priority (one time in five) | The HUD names each stage, the camera follows the spline (flat-coloured placeholders), and the tour continues after it |
| B2 | Exterior map kind and the mouth model | Approach screenshots read as a built moon by day and night |
| B3 | Tunnel segments, instanced, with fog and draw order | Tunnel at 60 fps on the Pi with flat panes |
| B4 | City bake and `fs_pane` parallax, traffic | Screenshot set through the tunnel; `fs_pane` ≤ 80 instructions on shader-db |
| B5 | Bay, cradle, clamps, berths; dock and undock scripts | Dock and undock screenshots, clamp shake |
| B6 | Arms, sparks, arc light, SERVICE panel | Service screenshots at 5 moments; worst frame ≤ 8 ms on the Pi |
| B7 | Console entry, incident rows, without-models fallback | Console test like `tests.rs:1045`; a base is never rolled before the models are ready |
| B8 | Pi sweep and docs | `solar::tests::frame_time` sweep through a base on seeds 5 and 23; a section in `solar.md` with screenshots |

Tests to add: `solar::tests::starbase` (ignored: screenshots of every stage, like `encounters`),
a unit test that the base's segments sum to their stated duration and that the next transfer leaves
from the mouth, and a shader check for the new uniforms (`shader_check.rs` covers this already once
the structs are registered).
