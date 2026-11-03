# The globe's edge at four threads, ETC2 maps, and GL's extra CPU

From `main` 792a2ca, 2026-11-03. Pi 5, kiosk frozen for every run.

## The edge shader: 2 → 4 threads

`fs_main` drew the globe's 2-pixel anti-aliased edge (D2). It ran at 2 threads with 57 temps
because it held three things live at once: the planet's surface (the 31 temps `fs_disc` needs on
its own), the limb's glow, and space behind it. The edge is now drawn in two passes over the same
annulus (`PASSES` in `globe.rs`): `fs_halo` paints the limb over space, then `fs_edge` paints the
surface over it with hardware blending, its alpha the pixel's coverage. Each fits in 32 registers.

| shader (offline shaderdb) | inst | threads | temps |
|---|---|---|---|
| `fs_main`, before | 696 | 2 | 57 |
| `fs_edge` | 444 | **4** | 31 |
| `fs_halo` (also under the edge) | 154 | 4 | 15 |

The frame time does not change measurably (6.44 ms back to back with the maps uncompressed, against
6.41 on `main`): the edge is about 6 k pixels, as D2 expected. The edge is also cleaner. The old
shader returned early in some pixels of a 2×2 quad, so the texture derivatives at the limb were
undefined, and the limb showed dark specks and gaps at 4× zoom. Now every pixel of the quad runs the
same code. The blend differs from the old composite in about 60 pixels a frame, all on the limb.

## ETC2 and EAC maps

`src/etc.rs` is an ETC2/EAC encoder and decoder: the ETC1 individual and differential modes,
ETC2 planar (for smooth gradients like the oceans) and EAC. It has no T or H modes. `build.rs`
decodes `assets/earth`, builds each mip chain on the CPU (the day map in linear light) and
compresses every level. A stamp of the inputs skips this when nothing changed. Encoding all four
maps takes about 2 s on the Mac (`build-override` is at opt-level 3).

| map | was | now | level-0 PSNR against the source |
|---|---|---|---|
| day | RGBA8 sRGB | ETC2 RGBA8 sRGB, the water mask in alpha | 38.7 dB (water 47.8) |
| night lights | R8 | EAC R11 | 46.9 dB |
| clouds | R8 | EAC R11 | 41.1 dB |
| relief | RGBA8 (slopes + water) | EAC RG11 | 61.3 dB |

That is 25 MB of GPU memory instead of 80, with the same number of texture fetches: the water now
comes with the day map's fetch instead of the relief's. Start-up decodes nothing: `Globe::new`
dropped from about 270 ms to 70 ms on the Pi. The binary is 19 MB larger, because the compressed
maps are bigger than the JPEGs they replace. The decoder agrees bit for bit with `texture2ddecoder`
on ETC2 colour. That crate's EAC is wrong (it reads the index bits as little-endian), so the EAC
decoder follows Mesa's `texcompress_etc.c`. Without `TEXTURE_COMPRESSION_ETC2` the maps are
decompressed on upload. `SPOTIFY_ROUND_ETC2=0` forces that path, to compare.

**Pi, headless GL** (`globe::tests::frame_time`, back to back, steady over 2–3 runs):

| | ms/frame |
|---|---|
| `main` | 6.41 |
| this, `SPOTIFY_ROUND_ETC2=0` (edge split only) | 6.44 |
| this | **6.01** |

**Pi, live on the display, GL** (`live-pi.sh`, 45 s, two runs each): 61 fps and no long gaps
either way, GPU **14.14 → 13.53 ms** a frame. That leaves 2.9 ms of headroom under the refresh,
where there was 2.3.

The gain is smaller than the 3.3× drop in traffic suggests: as D2 found, the globe is limited by
the texture unit's filtering rate (the 4x anisotropic taps), and compression does not change it.

Pi GL previews against the Mac's: 47–51 dB, the usual filtering differences.

### The dive: tried, not kept

The dive's 19 surface layers (256², RGBA8 with mips) were compressed to ETC2 RGBA8 at start-up and
benchmarked against `SPOTIFY_ROUND_ETC2=0` with the same binary (`bench-pi.sh`, 204 places): median
7.29 → 7.26 ms, worst 10.69 → 10.47. A few reef and meadow places gained 0.15–0.45 ms, but places
swing by ±0.25 ms between runs. The tiles are small enough to stay in the texture cache. The
tunnel's `Vault` layer packs tone, two slopes and the molten cracks into its four channels, and ETC
cannot hold three independent colour channels (21–24 dB, with errors over the whole range). Shipping
it would need that layer in a texture of its own. Not worth it for this gain, so the code was
reverted.

## Why GL takes more CPU than Vulkan

Per-thread CPU, steady state (`/proc` over 20 s after 14 s of warm-up), `--mock`, 61 fps,
% of one core, user + sys:

| scene | GL | Vulkan |
|---|---|---|
| globe | 12.3 | 13.7 |
| dive (seed 22), three runs | 25.9 / 26.4 / 25.5 | 23.6 / 23.7 / 23.6 |
| dive: UI thread | 14.6 | 8.5 (+2.5 in a second, nearly all-sys thread) |
| dive: visualizer thread | 11.2 | 12.6 |

So GL costs more only in the dive, by 2–3 % of one core. The globe is the other way round. The
difference is on the UI thread. Profiling with `perf` (user space, with Mesa's
`mesa-libgallium-dbgsym` for symbols) puts about half of that thread's GL samples in `libgallium`,
Mesa's GL driver:

- `v3d_write_uniforms` and `v3d_draw_vbo`: the uniform stream and draw packets, rebuilt on every
  draw.
- Shader-variant lookup on every program switch: `blake3_*` hashing, `set_search`,
  `hash_table_search`, `nir_find_variable_with_location`, `v3d_update_compiled_fs/shaders`,
  `v3d_get_compiled_shader`. GL resolves state at draw time; Vulkan bakes it into the pipeline.
- Rebinding textures, UBOs and programs: `_mesa_update_texture_state`, `st_bind_ubos`,
  `update_program`.

The dive makes about 80–100 draws a frame (32 model draws, 33 terrain tiles, sprites and
backgrounds) across many pipelines, which are specialised to keep the V3D at 4 threads. That comes
to about 9 µs of driver time a draw on the A76, about 5.6 % of a core. On Vulkan the driver takes
about 1.8 % across both threads. All of the GL work lands on the UI thread, because wgpu's GL
backend runs the GL calls at `queue.submit`, which this app makes on the UI thread
(`Graphics::submit`). `MESA_NO_ERROR=1` changed nothing.

Ways to reduce it, none done: fewer draws (the 33 terrain tiles are one pipeline and could be one
draw), fewer pipeline switches between draws, or going back to Vulkan for the display, which
costs GPU time on the globe (D2).

## Tools

- `perf` runs on the Pi without installing anything: Debian's `linux-perf` and its libraries
  unpacked into `/tmp/perf` (`LD_LIBRARY_PATH=/tmp/perf/usr/lib/aarch64-linux-gnu`). Use
  `-e cpu-clock:u` (perf_event_paranoid is 2), launch the process under `perf` (attaching to the
  kiosk is refused), add `--user-callchains` for call graphs, and use
  `--symfs /tmp/dbg/root` for Mesa's symbols (`apt-get download mesa-libgallium-dbgsym`, then
  `dpkg-deb -x`). Mesa's frames do not unwind, so only flat profiles are useful.
