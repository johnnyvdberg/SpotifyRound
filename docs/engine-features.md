# Engine features: what is next

The renderer's next features, ranked by what they add for what they cost on the Pi 5's V3D
(estimates until measured). The finished task queue and its handoff notes were removed on
2026-11-03; they are in git history at `da1fb0d` (`docs/agent-tasks.md`, `docs/handoff/`).

| # | Feature | What it adds | Where it lives | Expected cost |
|---|---|---|---|---|
| E1 | Contact shadows | **Done 2026-11-03.** A soft dark rectangle on the floor under the rocks and the museum's stands, cases and benches, lying on the plane through the ground round each (`world::rim_plane`); coral heads and the rest can follow with one `Frame::shadow` call each | `vs_shadow`/`fs_shadow`, multiplied after the seabed, `part::SHADOWS` | Measured +0.10 ms a place on the Pi (+0.25 worst) |
| E2 | Baked vertex occlusion on built architecture | Corners where walls meet floors, under friezes and in arches darken, so boxy halls read as built | Worked out once when the meshes or instances are built; the shader only multiplies | None per frame |
| E3 | Museum lamp pools | The lamps over the paintings and mineral cases light the walls per pixel: real pools of light | A museum-only pipeline taking each slab's nearest two or three lamps | Moderate, museum only; must keep four threads |
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
