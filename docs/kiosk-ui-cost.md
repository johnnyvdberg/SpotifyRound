# What the UI costs on the kiosk

Status: **settled** (2026-11-10, measured at `032c336` on the Pi 5, Mesa 26.2.2, cage 0.3.1,
wlroots 0.20.2). Read this before spending time on the UI's or the compositor's share of a
frame: it is small, it is known, and most of what looked like UI cost was the measuring setup.

## The numbers

Render-engine time per displayed frame, the kiosk running the build itself under cage (the only
client), the scene frozen so only the UI draws:

| What is on screen | Render engine | TFU |
|---|---:|---:|
| Player's scene image + solar transfer HUD (ticker, narration, minimap) | **1.3 ms** | 1.5 ms |
| the same, HUD hidden | 1.2 ms | 1.5 ms |
| Player's scene image + solar orbit HUD (ticker, scanner panel, minimap) | **2.5 ms** | 1.5 ms |
| cage | **0** | **0** |
| `vkcube` full screen, for scale | 0.9 ms | 1.5 ms |

- **Cage does no GPU work.** It scans the app's buffers out directly. `cage -D` logs
  "Direct scan-out enabled" when the kiosk maps its window. `WAYLAND_DEBUG=client` shows the
  buffers: 1080×1080 `XR24` (opaque), linear modifier, pitch 4352. The unmapped decoration
  subsurfaces winit creates do not stop it.
- **The TFU job (~1.5 ms a frame) is the Vulkan WSI's copy into the linear buffer the display
  reads.** Every client pays it, `vkcube` included. It runs on the TFU, not the render engine,
  so it does not add to the frame's render time.
- **The HUD's own cost is ~0.1 ms in transfer.** In orbit the UI costs ~1.2 ms more than in
  transfer. That extra is unattributed. A likely part is the pulsing SCAN/BURN tag: an
  `opacity` below 1 on an element with children makes Skia open a layer, which splits the
  render pass.

Budget: at the panel's 60.07 Hz a frame has 16.6 ms of render engine. The **scene gets
16.6 − 1.3…2.5 ms**. A 10 ms target for the whole frame leaves the scene 7.5–8.7 ms.

## The scene plus the UI, live vs benchmark

Solar seed 11, the scene alone (`solar::tests::frame_sweep`, kiosk service stopped) against the
kiosk running it live (`gpu_ms` in the visualizer log):

| Stretch | Scene alone | Live | Gap |
|---|---:|---:|---|
| High orbit (t 65–130) | 7.2–7.4 ms | 9.5–9.7 ms | the orbit UI, ~2.3 ms |
| Transfer (t 0–60) | 2.2–3.2 ms | 3.3–7.6 ms | the UI (1.3 ms), plus map generation for the next stop, which the benchmark finishes before it times |

## Measuring it, and the traps that cost a day

- **Run the build as the kiosk, never as a second client on top of it.** With another window
  under it, cage stops scanning the client out. It then composites every frame on the GPU
  (`Updating 1080x1080@4352 shadow for linear texture` in `cage -D`). That adds **~2.4–3.4 ms
  of render and ~3.5–5 ms of TFU a frame** that the kiosk never pays, and it looks exactly like
  an expensive UI. `deploy/live-pi.sh` now swaps the kiosk service's command for the run (a
  runtime drop-in in `/run`), and puts the stock kiosk back on exit.
- **Benchmarks stop the kiosk service** (`deploy/bench-pi.sh`, `deploy/shots.sh`), cage and
  all. They do not pause it. For headless tests the numbers matched the old `SIGSTOP` to within
  0.01 ms, but nothing else is left on the GPU.
- **`gpu_ms` in the visualizer log and the stats overlay is the whole render engine** (scene, UI
  and anything else on the GPU) divided by the scene's frames. It is not the scene's time.
  `frame_sweep` and `hotspots` time the scene alone.
- **Per-process split**: `sudo cat /proc/<pid>/fdinfo/<fd>` for each v3d fd gives
  `drm-engine-render` and `drm-engine-tfu` in ns. Read it twice and take the difference. cage
  and the kiosk it launched share cage's client id.
- **The solar tour's system comes from `SOLAR_SEED`.** `SPOTIFY_ROUND_SEED` seeds the other
  scenes. Without `SOLAR_SEED` a live run flies a random system and cannot be compared with
  `frame_sweep`.
- **`SPOTIFY_ROUND_DISPLAY=gl` cannot show the solar tour.** Its frame texture needs
  `VIEW_FORMATS`, which wgpu's GL backend lacks, and the app falls back to another scene. The
  kiosk runs Vulkan.

## When to look again

Only if one of these changes:

- The window stops being scanned out: check `cage -D` for "Direct scan-out disabled" while the
  kiosk runs alone. A translucent window or swapchain format, a mapped subsurface, or a second
  surface on screen would each do it.
- The UI gains `opacity` below 1 on elements with children, or anything else that makes Skia
  open a layer. Each one splits the render pass, which costs a load and store of the whole
  screen (≈0.8 ms at 1080²; `docs/perf-bottlenecks.md` §2).
