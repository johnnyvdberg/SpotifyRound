# Dive visuals: finished separately, ready for review

Written 2026-10-30 by the DeepSeek session that picked up Claude's memory
handoff (`~/.claude/projects/-Users-john-projects-SpotifyRound/memory/dive-visuals-handoff.md`)
after the Claude session hit its limit.

**Claude's own branches and worktrees were not modified.** Everything here is
on a separate branch, so nothing is overwritten and the original worktrees can
still be inspected exactly as they were left.

## Where it is

- Branch **`dive-visuals-finish`**, head at the commit that adds this file.
- Worktree **`.worktrees/finish`** inside this repository.
- Base: `dive-visuals` at **5fed499**, which is still where that branch is.
- Nothing has been pushed or deployed.

Useful commands:

```sh
cd .worktrees/finish
git log --oneline --first-parent dive-visuals..HEAD
git diff --stat dive-visuals...HEAD
git diff dive-visuals...HEAD -- src/dive3d/mesh.rs src/dive3d/mod.rs \
    src/dive3d/textures.rs src/shaders/scene.wesl
```

## What was merged

| Work | Original branch and commits | Kept as | Merge commit |
| --- | --- | --- | --- |
| Branching coral, brain coral, anemones, sea whips | `worktree-agent-a4eb1e492f5d74a97` @ 488f589 | merged unchanged | `ec43fbe` |
| Swimming jellyfish, five species, comb jellies | `worktree-agent-afcc99977990fb724` @ 6487b58, 4b2ef66 | merged | `e57864b` |
| Drowned city, fourteenth sea | `worktree-agent-a5ea2066e52e6a8dd` @ f1f9cb1 | merged | `f6c2af3` |
| Lava cave rock, relief, glowing seams | `worktree-agent-ae5bd40c7e85ad16b` @ f846c1d | finished on `cave-handoff` @ 97030f2 | `0813c82` |
| Boat path in all three axes | `worktree-agent-a252fa726026deff4` @ 517cbf4, 774983f | finished on `path-handoff` @ a4e8f89 | `b40aff0` |

The last two agents ran out of credit mid-task. Their uncommitted work was
saved in new commits on the two handoff branches, not overwritten:

- **`cave-handoff`**: the final point-light/bounce tuning the agent had not
  saved, plus `docs/handoff/cave.md`.
- **`path-handoff`**: the agent's last uncommitted state (corridor and
  divers-test revert, 16° swing, smoothed Atlantis bound), plus the jitter fix,
  re-aimed bubble test and `docs/handoff/path.md`.

The original worktrees still contain their original uncommitted changes as
well, so there are two copies of each piece of WIP.

## What had to be reconciled

Review these in particular; they are the places where two agents' work met.

- **`Model` enum and `ALL` are now 60 entries.** Coral's four, the jellyfish's
  two and the city's six are appended after `Tree`, in that order. The enum
  order is a GPU index, so do not reorder it.
- **Material ids.** The jellyfish's comb jelly already used
  `material::COMB = 10`. The city's material moved from 10 to **11**, and
  `CITY_LIGHTS` in `scene.wesl` is 11. `MATERIALS` in `mod.rs` carries 11.
  `docs/handoff/city.md` was updated to match.
- **Texture layers.** Coral's `Surface::Brain` is layer 15; the cave's
  `Surface::Vault` is layer 16 (`VAULT_LAYER = 16` in the shader). Both have
  tests guarding the number.
- **Static placers.** `static_steps` is 12 entries: the jellyfish moved
  `drifters` into the cache, the city appended `city`. `steps` is 13: it
  gained `city_beams` and lost `drifters`. `STATIC_NAMES`/`STEP_NAMES` follow.
- **Bubble regression.** The path's swing moves the old bubble scenes
  (633/3015/3407 s) out of view, and the city's extra leg had moved the later
  ones to 3556/3810 s. The test now uses the early diver helmets (630 s) and a
  shallows encounter (840 s), both of which still cross the swung window. Its
  popping guard is unchanged.
- **The swing clamp.** The path agent's hard clamp snapped the heading when a
  swipe crossed the Atlantis ruins, producing a 4.7°/s jolt. The spring's
  *target* is now bounded instead of its state, which keeps the rate
  continuous. This is the only logic fix added beyond the agents' own work.

## Verification

- `cargo fmt`: clean.
- `cargo clippy --release --all-targets`: clean, no warnings.
- `cargo test --release`: **121 passed, 0 failed, 22 ignored**.
- `docs/dive3d-scenes.png` and `.html` regenerated: 81 places, 1960 × 3360.
- High-resolution renders were checked for coral, the drowned city, the bloom
  and the cave; all render and read as intended.
- **Pi, kiosk frozen, V3D, seeds 0 and 500000, 8 frames, two runs each:**

  | | 5fed499 | merged |
  | --- | --- | --- |
  | hotspots | 78 | 84 |
  | median place | 9.81–9.83 ms | 10.57–10.61 ms |
  | mean place | 9.85–9.86 ms | 10.62–10.65 ms |
  | slowest place | 14.7 ms | 14.2 ms |

  The merged branch costs about **+0.78 ms** on the median place (+8%),
  consistent across both runs. Every measured place is below 14.3 ms, inside
  the 16.6 ms 60 fps budget. The new city leg is 10.7 ms. Raw tables are in
  `docs/handoff/pi/`. The binaries are `/tmp/srt-finish` and
  `/tmp/srt-5fed499` on the Pi.

## Claude: please check this when you are back

1. Read this file in `.worktrees/finish` and confirm the branch is based on
   the `dive-visuals` you left (5fed499).
2. Review the five merges and, above all, the conflict resolutions listed
   under "What had to be reconciled".
3. Run `cargo fmt --check`, `cargo clippy --release --all-targets` and
   `cargo test --release` in `.worktrees/finish`.
4. Look at the regenerated sheet and the high-resolution shots; then run a
   leg in motion to judge the path swing. The survey reports up to 15.5° crab
   and 0.17 m/s toward the window, with jitter 0.18°/s.
5. Re-run the Pi hotspots if you want a second opinion; the exact commands are
   in `docs/handoff/pi/` and the binaries are still on the Pi.
6. If it is good, fast-forward `dive-visuals` to `dive-visuals-finish` (or
   merge it normally). If not, commit fixes on `dive-visuals-finish`. Do not
   push or deploy without asking.

## Remaining risks

- The Pi is only **+0.8 ms** worse on the median place, but the slowest is
  14.2 ms: a hot Pi or a slower SD card could eat the remaining 2.4 ms.
- Coral draws 8–9 more calls per coral frame; the Pi numbers include that and
  stay under budget.
- The cave shader is longer, though it still takes one texture tap. Its
  RGBA texture change gives every non-vault layer `a = 1`; only the vault and
  cave cliffs read alpha, but check the other biomes for a change in haze.
- The path's swing changes which meetings are visible at the window edge; one
  regression test was re-aimed for that reason. Watch a full leg for pop-in.
- The city is a new sea with its own light material; its Pi cost is fine here,
  but it has had the least visual play-testing of the five.
- No motion capture of the new path was made. The survey numbers and the
  scene sheet are the evidence.

## Second pass: cave nooks and city types (2026-10-30 evening)

Commit **`5b49068`** adds a second visual pass on top of the merge above.

### Cave

- Three shallow nooks per cave, between the five secret passages:
  - **camp**: two crates, a bedroll, a lost diver's helmet, a lantern still
    burning and a pick against the wall;
  - **crystal**: seven crystal points growing out of the wall, with a violet
    light and a twinkle;
  - **ember**: a still lava pool, a rim of cooled rock and embers rising.
- `Seabed::nooks()`/`NookKind`/`Nook` define them; `tunnel_radius_with_nooks()`
  stands the wall back over each, so the hollow and its props always agree.
  The bump is outward only, so the hull keeps its clearance.
- Stalagmites and rubble now gather along the walls between the nooks, and
  three screenshot places (`10-cave-nook-camp`, `-crystal`, `-ember`) were
  added so the nooks can be inspected.
- **These are not finished to the same standard as the rest.** The nook
  frames at the exact centres are mostly wall (the window looks sideways into
  the hollow), and the camp frame is a close jumble of crates. The camp reads
  best in `10-cave-passages-2`; the crystal and ember nooks have not yet been
  seen clearly in a shot. Scale, placement and camera moments need another
  pass.

### City

- Four new building models, appended after `Sign` (`Model::ALL` is now 64):
  - **Ziggurat**: a stepped art-deco pyramid with neon string courses;
  - **Arcade**: a low two-storey hall with a colonnade, lit canopy and neon
    marquee;
  - **Factory**: a riveted works hall with sawtooth skylights, two banded
    stacks, pipes and a glowing furnace;
  - **Theatre**: a round picture palace with brass fins, a glass marquee and
    a lit cupola (toned down from an over-bright first version).
- Denser placement: near blocks have an 8% empty chance (was 20%), far 18%
  (was 30%); the avenue frontage is biased towards the low types and a row of
  small street shops fills the pavement between blocks.
- New `effect::CITY_BLOCKS` group and `MATERIALS`/model registrations.
- **The density and the four types are in, but the city's Pi cost is not
  measured.** More instances and draw calls are expected.

### Verification for this pass

- `cargo fmt` and `cargo clippy --release --all-targets`: clean.
- `cargo test --release`: **121 passed, 22 ignored**.
- Scene sheet regenerated for **84 places** (the three nook places added).

### Benchmark pending — Claude, please run it

No Pi measurement has been made for `5b49068`. The table earlier in this file
is for the merge at `653f3f1`, not this pass. The user explicitly asked for a
benchmark and for you to repeat it in case. Suggested:

```sh
cd .worktrees/finish
deploy/build-for-pi.sh --tests
scp target/pi/spotify-round-tests john@192.168.0.33:/tmp/srt-finish2
ssh john@192.168.0.33 '
  P=$(pgrep -x spotify-round); kill -STOP $P; trap "kill -CONT $P" EXIT
  mkdir -p /tmp/hot-fin2; cd /tmp
  DIVE3D_HOTSPOT_SEEDS=0,500000 DIVE3D_HOTSPOT_FRAMES=8 \
  DIVE3D_HOTSPOT_DIR=/tmp/hot-fin2 \
    ./srt-finish2 dive3d::tests::hotspots --ignored --nocapture'
```

Compare against `/tmp/srt-5fed499` (still on the Pi) with the same settings.
The important numbers are the city leg, the cave leg, coral and the overall
median. Put the fresh table in `docs/handoff/pi/` and report whether the new
city density keeps every place under the 16.6 ms frame budget.

### Other rough edges for the next pass

- The city's new foreground types need a look in motion; the theatre's neon
  is now calmer, but the factory stacks and the shop row have not been judged
  from a moving window.
- The nook camera places were added at the nook centres and are not yet good
  screenshots; tune their offsets or add a nook-specific view.
- The scene sheet is otherwise current.
