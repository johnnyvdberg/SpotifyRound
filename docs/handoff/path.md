# The boat path in three dimensions: handoff

Branch `path-handoff` (copy of `worktree-agent-a252fa726026deff4`), based on
`dive-visuals` (5fed499). The agent's work is `517cbf4` and `774983f`; this
branch adds the uncommitted last state it left behind and the fixes and
summary below.

The original agent hit its session limit with two tests red. It had already
decided on the way out to revert the corridor experiment and keep the abeam
cull: "Revert the corridor and the divers-test tweak; keep the abeam cull."
The uncommitted tree had that revert, a smaller `SWING` (0.28) and a smoothed
Atlantis bound. Those are all in this branch.

## What changed

- **The helmsman swings the boat in open water (`helm.rs`).** A slow weave of
  the voyage's own, ±16°, a quarter of a degree a second at most, turns the
  nose off the track so the window looks a little ahead or astern and things
  come at the glass on the diagonal instead of only sliding past it. He dips
  a little as he looks ahead and lifts as he looks back (`DIP`), so the path
  moves in y as well as across.
  - The swing waits on the course spring: the two together never turn the
    nose faster than the hull may turn, and it is held straight in the cave's
    tunnel, where the crab looks up it.
  - `room()` closes the swing over the Atlantis ruins, whose columns stand
    close to the lane. It is averaged over x − 1 … x + 5 so a swipe's passage
    does not snap it straight; the target, not the spring's state, is
    bounded, which keeps the rate continuous. That last change is what makes
    the survey's jitter test pass (a hard clamp caused a 4.7°/s jolt when a
    swipe crossed the ruins).
- **The world is culled abeam, not along the window (`world.rs`).** `cells()`
  now keeps a cell if it is near the line square to the boat's way as well as
  inside the window's footprint, and the footprint is taken from half as near
  so swinging towards something does not make it vanish. `Pose` carries
  `abeam` for this.
- **The survey measures the effect (`helm/survey.rs`).** `Summary` gains
  `toward` (the fastest the window closes on what it looks at, m/s), `crab`
  (the most the way is off the nose) and `sweep` (how far a hull point comes
  from the corridor, per biome). The swipe window is widened to ±6 s, because
  a passage's turn now settles over more frames. Over the survey voyages:
  crab up to **15.5°**, toward up to **0.17 m/s**, jitter **0.18°/s**.
- **A regression test is re-aimed (`world.rs`).** The swing can take a
  meeting that used to sit on the glass out of view. `bubbles_never_pop` now
  samples the diver helmets at 630 s and a shallows encounter at 840 s, both
  chosen by scanning visit 0 for trails that still cross the window; its
  popping guard is unchanged. (The three original scenes were 633, 3015 and
  3407 s; the last two no longer show bubbles under the swing.)

## What did not survive

The WIP had briefly added a third, high-frequency bend to `corridor()` (and
the shader's copy) and moved the divers test's pose to the middle of the
party. The agent reverted both before it died; this branch keeps them
reverted. The extra bend made the course too tight for the hull, and the
divers test was compensating rather than measuring the path.

## Checks

`cargo fmt`, `cargo clippy --release --all-targets` clean, `cargo test
--release`: **118 passed, 22 ignored**. The helm survey's `the_hull_moves_
like_a_heavy_boat` and the bubble guards both pass.

## Risks and follow-ups

- ±16° is a visible crab; the window's near edge may lose a close meeting
  sooner than before, which is why a test scene moved. Watch a full leg for
  scenery popping at the window's edge.
- The abeam cull assumes `Pose::abeam` is the hull's outward direction; any
  other pose source (`Pose::level` in tests and previews) gets `[0, 1]` and
  behaves as before.
- Nothing has been measured on the Pi, and no motion screenshots were
  captured before the agent ran out. The survey numbers are the evidence.
- The swing composes with the new city's avenue: the city keeps buildings
  2.3 units off the corridor, so the bound is not applied to it.
