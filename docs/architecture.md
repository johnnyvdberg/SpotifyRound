# Architecture

## Source layout

```
src/
├── main.rs            CLI entry point, logging, rustls provider
├── app.rs             controller: polling, commands, view models, animation tick
├── ui.rs              every Slint property write + offscreen preview renderer
├── motion.rs          Animato: accent tween, reveal, pulse
├── visuals.rs         idle scene registry and the render thread; picks a scene at random
├── globe.rs           idle scene: the Earth, a wgpu fragment shader over the day, night, cloud and relief maps, lit through a model of the air
├── lens.rs            what the two eyepiece scenes share: one full-screen shader over one uniform block, the frame ring, a seeded die
├── observatory.rs     idle scene: a night at the telescope, through its eyepiece
├── microscope.rs      idle scene: slides of pond life under a microscope
├── noise3d.rs         a 64³ volume of smooth noise and its gradient, which the planet and solar shaders sample
├── shader_lab.rs      tests that compile WGSL experiments and dump the shaders for shader-db
├── shader_check.rs    tests: every shader validated with naga, every uniform struct checked against it
├── gpu_cost.rs        tests: what the GPU at hand charges for instructions, passes and uploads
├── v3d_perf.rs        tests: the Pi's V3D performance counters
├── solar/             idle scene: the grand tour of star systems
│   ├── mod.rs         the scene: its state, the frame loop's entry points
│   ├── setup.rs       pipelines, bind groups and render targets, built once
│   ├── pipe.rs        the pipeline keys
│   ├── layout.rs      sizes and `ev` rows shared with the shaders (injected by build.rs)
│   ├── frame.rs       one frame: uniforms, instance buffers and the passes
│   ├── orders.rs      draw order of the bodies and models
│   ├── bodies.rs      the bodies in view and how each is drawn
│   ├── encounters.rs  what the stopovers between stars place each frame: fleets, panels, effects
│   ├── bake.rs        the starfield, sky, crater and gas maps baked once a system
│   ├── system.rs      seeded stars, planets, rings and moons, with real masses and orbital speeds
│   ├── tour.rs        the timeline: transfers, orbits, departure, hyperjump; camera and HUD at any moment
│   ├── voyage.rs      the encounters between the stars and when they happen
│   ├── models.rs      the starship and station meshes, built on a worker thread
│   ├── scan.rs        the orbital survey: what each scan finds about a planet, and when
│   ├── labels.rs      the labels on the worlds in view
│   ├── console.rs     the ship's computer
│   ├── stream.rs      paints planet, moon and sky maps on the GPU a band of rows per frame, into two slots
│   └── tests.rs       unit tests, screenshots and benchmarks
├── shaders/           WESL package, linked to WGSL by `build.rs`
│   ├── common.wesl    shared smoothing, distance and noise functions
│   ├── noise3d.wesl   sampling the 64³ noise volume
│   ├── scene.wesl     the dive: water, seabed, models, glass and sprites
│   ├── solar/         the tour: frame.wesl (background, glare, warp, bodies), encounter.wesl
│                      (the stopovers), generate.wesl (the map generator), noise.wesl
│   └── lens/          plain WGSL, included by the eyepiece scenes: prelude.wgsl (shared),
│                      observatory.wgsl, microscope.wgsl
├── dive3d/            idle scene: the Nautilus' voyage, rasterised in 3D
│   ├── mod.rs         renderer: passes, pipelines, lighting; draws the sea full-panel
│   ├── helm.rs        how the boat moves: a 20 m hull's track, height, pitch, roll and clearances
│   ├── heightmap.rs   the random seabed of each visit: rolling ground, ridges and canyons
│   ├── terrain.rs     the seabed in tiles, built as they come into view and dropped behind
│   ├── textures.rs    the procedural surfaces: rock, planks, marble, scales, coral, basalt…
│   ├── world.rs       what is in the water each frame: features, swimmers, lights, sprites
│   ├── places.rs      named moments of the voyage, for screenshots and reference sheets
│   └── mesh.rs        every model, generated at start-up, reordered by meshoptimizer
├── voyage.rs          the voyage itself: distance travelled, swipes, and the dive profile behind the depth gauge
├── ring.rs            the player's backdrop: glow, vignette and progress rim in one wgpu fragment shader
├── gpu.rs             the single wgpu/Vulkan device, handed to Slint before the window exists
├── artwork.rs         download → decode → rounded mask → accent colour → cache
├── browser.rs         xdg-open / start / open
├── config.rs          config.toml, token paths, permissions
├── format.rs          time and pluralisation
├── service/
│   ├── mod.rs         `Source` = live Spotify API or demo backend
│   ├── api.rs         typed Spotify Web API client
│   ├── auth.rs        PKCE OAuth, callback listener and phone login page, token refresh
│   └── mock.rs        in-memory demo library
ui/
├── main.slint         window, ambience, screen router
├── theme.slint        design tokens
├── state.slint        Player / Devices / Library / App / Idle globals
├── components/        icons, buttons, artwork, rim ring, rows, toast…
├── images/scenes/     the screensaver picker's round windows, baked from each scene by `visuals::tests::picker_thumbnails`
└── screens/           now playing, device picker, playlists, queue, login, idle, screensaver picker
assets/earth/          Blue Marble day, night and clouds (4K), relief and water maps (2K)
build.rs               Slint + WESL build steps; the WGSL artifacts go to OUT_DIR
```

**Shader and mesh tooling.** The shaders are a small [WESL](https://wesl-lang.dev/) package under
`src/shaders/`. `build.rs` links each root (the dive's `scene.wesl`, the tour's
`solar/encounter.wesl` and `solar/generate.wesl`) to plain WGSL in `OUT_DIR` before wgpu sees it,
and injects the layout constants of `src/solar/layout.rs` so Rust and the shaders cannot disagree. The models are reordered with
[meshoptimizer](https://github.com/zeux/meshoptimizer) (`meshopt`) for vertex fetch and post-transform
cache locality, and the CPU mesh math uses [`glam`](https://github.com/bitshifter/glam-rs).
`wgpu-profiler` wraps the Nautilus encoder when `SPOTIFY_ROUND_PROFILE=1` and the adapter exposes
timestamp queries. The Pi 5's V3D does not, so there it is compiled in but left off and the
`V3D` sysfs accounting is used instead.

**Threading.** Slint runs on the main thread. A Tokio runtime polls `/me/player` once a second and
performs commands; results are marshalled back with `upgrade_in_event_loop`. `slint::Image` is not
`Send`, so artwork is decoded to a `SharedPixelBuffer` on a worker thread and turned into an image on
the UI thread. A timer on the UI thread ticks once per frame at the panel's refresh rate (16⅔ ms for
60 Hz, not the 16 ms that would make it 62.5 Hz) and advances the progress clock between polls. The
rim is not a Slint path: `ring.rs` draws the track and the played sector with one
fragment shader into one of three textures, and Slint only composites the latest one. That removes
per-frame SVG parsing and stroke tessellation, and a paused track repeats the same input so the rim
is not redrawn at all.

**Hardware acceleration.** Before any window exists, `src/gpu.rs` opens a wgpu device (Vulkan first)
and hands it to Slint with `BackendSelector::require_wgpu_30(WGPUConfiguration::Manual { .. })`, so
Skia composites the whole UI through Vulkan (`-v` logs the adapter and the graphics API in use).
That API sits behind Slint's `unstable-wgpu-30` flag, so the feature and `slint::wgpu_30` names move
with wgpu releases; the idle scenes and the progress rim render on the same device and their frames are
handed over as textures, so nothing round-trips through the CPU. On a Pi without a compositor build
with `--features kms` to draw straight through DRM/KMS instead of winit:

```bash
cargo build --release --features kms
```

that feature pulls in libseat and libinput, so `libseat-dev`, `libinput-dev` and `libudev-dev` have to
be installed for the build. Playback position is interpolated from a monotonic clock and a poll only
re-anchors it when it disagrees by more than a second, which keeps the rim moving forwards at a
constant rate instead of twitching as the answers arrive.

## Designing for the Pi's GPU

How the costs are measured is in [benchmarking.md](benchmarking.md). What the measurements led to:

- The display opens its device on **Vulkan** and Slint draws the window with **Skia**, which compiles
  a small shader for each kind of draw. `SPOTIFY_ROUND_DISPLAY=gl` opens it on OpenGL ES instead,
  where Skia cannot draw, so the window is drawn by **femtovg**. Stock femtovg runs one
  1090-instruction shader for every paint type (a full-screen layer cost about 5 ms on the Pi), so
  the copy in `vendor/femtovg` specialises its shader per pipeline. GL measured as fast or faster in
  the benchmarks, but on the kiosk in daily use Vulkan was faster and GL's driver costs more CPU on
  the UI thread, so Vulkan is the default (`DISPLAY_GL_DEFAULT` in `gpu.rs`).
- Headless, the tests and benchmarks open OpenGL ES first on the Pi: Mesa's GL driver runs the V3D's
  early-Z and its Vulkan driver does not, which made every frame of the dive 0.9–2.6 ms slower.
- The device is opened with explicit limits, the Pi's baseline (16 textures a stage, 16 KiB uniform
  blocks) lowered further wherever an adapter offers less, so a desktop run fails on what the Pi
  could not draw. On Vulkan, compiled pipelines are kept in a disk cache in the user's cache
  directory (`SPOTIFY_ROUND_PIPELINE_CACHE=0` turns it off).
- The Nautilus' sea is the panel's full **1080 × 1080** with **one** sample, handed to Slint as one
  texture. Its budget and what it spends it on are in [docs/nautilus.md](nautilus.md) and
  [docs/perf/pipeline-optimization.md](perf/pipeline-optimization.md).
- What can be drawn together is: the player's glow, vignette and rim are one opaque texture from
  `ring.rs`, and the globe paints the space behind itself, so those screens are one full-screen layer
  for Slint. The Nautilus is one full-panel sea texture.
- Shaders are specialised where they branch on something fixed for a draw: the dive's model
  shader gets a pipeline per material and per grounded flag, so a swimming fish does not carry the
  seabed bedding its rocks need, and the globe works out once what never changes for a pixel (where
  it lies on the maps, the air towards the viewer) instead of every frame.
- No rendering notifier is installed unless the frame-rate readout asks for one: while one is set,
  Slint submits the window clear in a pass of its own.
- Every submission is made on the UI thread (`Graphics::submit`). wgpu drains the queue before it
  reconfigures the window's surface and fails if another thread submits meanwhile, which Slint
  does on the UI thread whenever the swapchain goes out of date.
- The visualizer keeps two frames in flight, no more. A scene thread that submits on its own clock
  queues work ahead of the window's frames faster than the GPU finishes it, until every frame on
  screen is late.

## Idle scenes

**Idle visualisers.** `screens/idle.slint` fades a full-screen overlay in when there is nothing left to
show: three seconds after Spotify reports nothing playing, or thirty seconds after a loaded track was
paused. Resuming playback closes it again, and a tap on the visuals returns to the player and keeps
them away until playback resumes.

Each visit picks a scene at random from the renderers that came up. `visuals.rs` owns that registry and
the render thread: a scene implements `Visual`, is added to `Scene`, and gets a block of three render
targets; the UI only needs a scene index and a frame, so adding another visualisation does not touch
the Slint screens. Both scenes are drawn with [wgpu](https://wgpu.rs) on the Vulkan device the UI itself
uses, straight into textures that Slint samples from the same device. Nothing is read back or
re-uploaded, so a frame never leaves the GPU. Frames are drawn at 60 fps and only while the overlay is
visible; at rest the thread sleeps and the display draws no power at all. `-vv` logs the achieved rate.

### Idle rendering, Slint and screensavers

The idle scene is an app-supplied wgpu texture, not a Slint-drawn image. The render thread keeps
three slots per scene (`SLOTS = 3`), hands Slint one `Image::try_from(texture)` per slot once, and
switches the visible slot by index. That is the path Slint's
[wgpu texture example](https://github.com/slint-ui/slint/tree/master/examples/wgpu_texture) uses: no
CPU readback and no per-frame image copy.

A few practical constraints found while checking this against Slint's renderer:

- Keep the handed texture **opaque and full-frame**. Slint's Skia/WGPU path can treat an
  app-supplied texture as unpremultiplied or only partially redrawn
  ([slint#13203](https://github.com/slint-ui/slint/issues/13203)); the sea is `Rgba8Unorm` with
  alpha 1, and the static overlay was removed so there is exactly one image layer.
- Keep the UI's image model stable and swap the index, rather than replacing the model every frame.
  `set_idle_frames` runs when a scene is built; `show_idle_frame` runs per frame.
- The render thread is already visibility-gated by `running`; when the visualizer is hidden it
  sleeps and does not submit GPU work. That is the right place to save power, rather than making
  Slint redraw a hidden layer.
- Actual screen blanking/DPMS belongs to the Wayland compositor or the panel (`cage`, `wlopm`,
  DPMS), not to Slint. The app should keep rendering only while its visualizer is visible and let
  the compositor own idle blanking. This is also why the renderer does not install a continuous
  Slint timer.
- The two-frame GPU fence (`in_flight`) is deliberate: one frame in flight leaves the V3D idle
  while Slint is compositing; more than two queues work ahead of the display.
