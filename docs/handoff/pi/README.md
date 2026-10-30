# Pi hotspot measurements

Two tables from `dive3d::tests::hotspots` on the Pi, kiosk frozen with
`kill -STOP $(pgrep -x spotify-round)`, V3D 7.1.7.0, seeds 0 and 500000,
8 frames. Each pair was run twice, back to back:

- `hotspots-5fed499.csv`: the branch point, built from a detached worktree at
  5fed499.
- `hotspots-merged.csv`: `dive-visuals-finish` (coral + jellyfish + city +
  cave + path merged).

Median place: 9.81–9.83 ms at 5fed499, 10.57–10.61 ms merged (+0.78 ms).
Slowest place: 14.7 ms at 5fed499, 14.2 ms merged. Budget 16.6 ms.

Reproduce (binaries still on the Pi):

```sh
scp target/pi/spotify-round-tests john@192.168.0.33:/tmp/srt-finish
ssh john@192.168.0.33 '
  P=$(pgrep -x spotify-round); kill -STOP $P; trap "kill -CONT $P" EXIT
  mkdir -p /tmp/hot-fin; cd /tmp
  DIVE3D_HOTSPOT_SEEDS=0,500000 DIVE3D_HOTSPOT_FRAMES=8 \
  DIVE3D_HOTSPOT_DIR=/tmp/hot-fin \
    ./srt-finish dive3d::tests::hotspots --ignored --nocapture'
```

The 5fed499 binary is `/tmp/srt-5fed499` on the Pi; rebuild it with a clean
worktree at 5fed499 and `deploy/build-for-pi.sh --tests` if it has gone.

## Second pass (5b49068)

Measured by Claude on 2026-10-30, kiosk frozen, seeds 0 and 500000, 8
frames, two back-to-back runs of each binary (`hotspots2-*.csv`), averaged:

- Median place: **9.82 ms** at 5fed499, **9.63 ms** at 5b49068.
- Slowest place: 14.71 ms (forest) at 5fed499, 12.74 ms (coral) at 5b49068.
- The city leg: 8.89 ms mean, 10.85 ms worst place. Cave 9.94 → 9.78,
  coral 11.42 → 10.83, bloom 9.36 → 8.03; brine (+0.66), trench (+0.48) and
  arctic (+0.45) are a little dearer. Every place is inside the 16.6 ms budget.

The path changes where each place's camera stands, so per-place numbers
compare the same moments of the voyage rather than the same views.
