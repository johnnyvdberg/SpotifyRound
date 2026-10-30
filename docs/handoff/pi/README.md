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
