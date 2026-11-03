# Engine features: what is next

The renderer's next features, ranked by what they add for what they cost on the Pi 5's V3D
(estimates until measured). The finished task queue and its handoff notes were removed on
2026-11-03; they are in git history at `da1fb0d` (`docs/agent-tasks.md`, `docs/handoff/`).

| # | Feature | What it adds | Where it lives | Expected cost |
|---|---|---|---|---|
| E1 | Contact shadows | **Done 2026-11-03.** Soft rectangles on the floor under the museum's stands, cases and benches; under stones a faint ellipse with no core (`Frame::blot`), on the plane through the ground round it (`world::rim_plane`). Coral heads and the rest can follow with one call each | `vs_shadow`/`fs_shadow`, multiplied after the seabed, `part::SHADOWS` | +0.10 ms a place on the Pi |
| E2 | Corner occlusion on built architecture | **Done 2026-11-03, for the floor:** the museum's floor darkens into the foot of every wall, one soft strip a wall through E1's pass. Not by vertex colour: the walls are instances of one unit box, so baked vertex occlusion would stretch with each. Under friezes and in arches is still open | `museum::building`, `end_wall` | Within E1's pass |
| E3 | Museum lamp pools | **Done 2026-11-03.** Each hanging lamp lights a pool on the floor under it and a wash down the nearer wall over the paintings, per pixel, by a quad that scales up what is under it (`dst * (1 + gain)`, drawn twice for up to four times) | `vs_pool`/`fs_pool`, `Frame::pool`, `part::POOLS` | E2, E3 and the softer shadows together: +0.08 ms a place, +0.36 at worst (a 6 ms museum view) |
| E4 | Searchlight highlights | Wet stone, marble, brass and glass glint where the beam lands, from the normal maps | Materials that want it, with roughness in the normal texture's spare channels (no extra read) | Moderate, only on those materials |
| E5 | Bloom | Lamps, lava, vents and the city's windows glow into the water | One pass at 270², added in the composite | About 0.5-1 ms: a full-screen pass is what a tile-based GPU pays for |
| E6 | Close-range detail texture | No blur when the boat brushes a wall | A third texture read in the free relief pipeline | Low-medium; likely costs a thread, so only if E1-E4 leave room |
| E7 | Caustic ceiling, the rest of P3 | The rest of the cheaper caustic ceiling from the perf queue | `scene.wesl` | A perf gain still on the table |
| E8 | Pace the dive from the display (P8) | Frames timed to the panel's refresh | Behind `SPOTIFY_ROUND_PACE=display`; the default is unchanged until the kiosk check settles | Smoothness, not quality |

Not on this GPU: screen-space ambient occlusion and reflections, a deferred G-buffer, and real-time
shadow maps for everything. Each reads and writes full-screen buffers, which is where the V3D is
weakest.

## How a feature lands

- **A pipeline of its own, or baked data.** Specialise with the shader's `override` constants
  (`MATERIAL`, `GROUNDED`, `ANIM`, …) so a shader carries only its own code; bake what does not
  change per frame. A new material is an entry in `world::material` and `MATERIALS`.
- **Gates on the Mac:** `deploy/v3d-shaderdb/check.sh .` (no shader may lose a thread or spill;
  `--update` and commit the baseline when the change is intended), `cargo test --release`,
  `cargo clippy --all-targets -- -D warnings`, `cargo fmt --all -- --check`.
- **Build on the Mac** with `deploy/build-for-pi.sh [--tests]`, never on the Pi; touch `src`
  first and check the binary's md5 changed.
- **Measure on the Pi** before and after in one session, with the kiosk frozen
  (`deploy/bench-pi.sh run`/`compare`, or the `hotspots` test). The median must stay under 11 ms.
  Report the median, the worst views and the places over 11 ms.
- **Switchable and priced:** a new part or effect gets an entry in the skip lists
  (`DIVE3D_SKIP` / `DIVE3D_SKIP_EFFECTS`) and a row in `docs/feature-costs.md`.
- **Pictures:** before and after shots of what changed (`deploy/shots.sh`, `docs/nautilus.md`).
- **Public repository:** commit as `johnnyvdberg@gmail.com`; never write the Pi's address or user
  name into tracked files (`<user>@<pi>`).
