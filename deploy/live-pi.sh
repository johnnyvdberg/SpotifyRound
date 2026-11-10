#!/bin/bash
# Live measurement of a build on the kiosk's display, run ON THE PI (copy it to
# /tmp/d1/ first; nothing here runs cargo):
#
#   scp deploy/live-pi.sh target/pi/spotify-round <user>@<pi>:/tmp/d1/
#   ssh <user>@<pi> /tmp/d1/live-pi.sh gl /tmp/d1/spotify-round 45 \
#       SPOTIFY_ROUND_SCENE=nautilus SPOTIFY_ROUND_SEED=22 SPOTIFY_ROUND_DISPLAY=gl
#
# usage: live-pi.sh TAG BINARY SECONDS [VAR=VALUE ...]
# Runs BINARY --mock AS the kiosk: a runtime drop-in (/run, gone at reboot)
# points the kiosk service's cage at it for the run, and the stock kiosk comes
# back on every exit. Holds the bench lock; needs `sudo -n systemctl`. Prints
# the visualizer/full-speed log lines' medians, the GPU's render-queue time per
# second, and takes a screenshot at the middle of the run.
#
# Not as a second client on top of the running kiosk: with another window
# under it cage stops scanning the client out and composites every frame,
# which costs ~2.4-3.4 ms of render and ~3.5-5 ms of TFU a frame that the
# kiosk never pays (docs/perf-bottlenecks.md, section 3.8). The solar tour's
# system comes from SOLAR_SEED, not SPOTIFY_ROUND_SEED.
set -u
TAG=$1; BIN=$2; SECS=$3; shift 3
# Logs and screenshots go next to the script.
D=$(cd "$(dirname "$0")" && pwd)
mkdir -p $D
exec 9>/tmp/nautilus-bench.lock
flock 9
DROPIN=/run/systemd/system/cage-kiosk.service.d
BIN=$(realpath "$BIN")
cleanup() {
    sudo -n rm -f $DROPIN/live.conf
    sudo -n systemctl daemon-reload
    sudo -n systemctl restart cage-kiosk
}
trap cleanup EXIT INT TERM
stats() { awk '$1=="render"{r=$4;j=$3} $1=="tfu"{t=$4;tj=$3} $1=="bin"{b=$4} END{print r, b, t, j, tj}' /sys/class/drm/card0/device/gpu_stats; }
sudo -n mkdir -p $DROPIN
printf '[Service]\nExecStart=\nExecStart=/bin/sh -c "exec /usr/bin/cage -- env SPOTIFY_ROUND_LOG=info,spotify_round::visuals=debug %s timeout %s %s --mock > %s 2>&1"\n' \
    "$*" "$SECS" "$BIN" "$D/$TAG.log" | sudo -n tee $DROPIN/live.conf > /dev/null
sudo -n systemctl daemon-reload
sudo -n systemctl restart cage-kiosk
W=14
sleep $W
read r0 b0 t0 j0 tj0 < <(stats); s0=$(date +%s.%N)
sleep $(( (SECS - W - 3) / 2 ))
XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 grim -l 1 $D/$TAG.png 2>/dev/null
sleep $(( (SECS - W - 3) / 2 ))
read r1 b1 t1 j1 tj1 < <(stats); s1=$(date +%s.%N)
# The run ends with the timeout; then the service's restart puts it back.
while pgrep -f "timeout $SECS $BIN" > /dev/null; do sleep 0.5; done
awk -v tag=$TAG -v r0=$r0 -v r1=$r1 -v b0=$b0 -v b1=$b1 -v t0=$t0 -v t1=$t1 -v j0=$j0 -v j1=$j1 -v tj0=$tj0 -v tj1=$tj1 -v s0=$s0 -v s1=$s1 'BEGIN{d=s1-s0; printf "%s render_ms/s=%.1f bin_ms/s=%.1f tfu_ms/s=%.1f render_jobs/s=%.1f tfu_jobs/s=%.1f\n", tag, (r1-r0)/1e6/d, (b1-b0)/1e6/d, (t1-t0)/1e6/d, (j1-j0)/d, (tj1-tj0)/d}'
sed 's/\x1b\[[0-9;]*m//g' $D/$TAG.log | grep -E "graphics |visualizer|full speed|renderer" | tail -n +1 > $D/$TAG.lines
grep -m3 -E "graphics |graphics context" $D/$TAG.lines
# medians of the steady state (skip the first 12 lines of each kind)
for k in fps ms gpu_ms taken short_gaps long_gaps; do
  v=$(grep visualizer $D/$TAG.lines | tail -n +12 | grep -oE " $k=[0-9.-]+" | cut -d= -f2 | sort -g | awk '{a[NR]=$1} END{if(NR) printf "%s(n=%d,min=%s,max=%s)", a[int((NR+1)/2)], NR, a[1], a[NR]}')
  echo -n "$k=$v "
done; echo
grep "full speed" $D/$TAG.lines | tail -n +12 | grep -oE "fps=[0-9]+" | cut -d= -f2 | sort -g | awk '{a[NR]=$1} END{if(NR) printf "ui_fps median=%s min=%s max=%s n=%d\n", a[int((NR+1)/2)], a[1], a[NR], NR}'
grep -ciE "error|panic" $D/$TAG.log | sed 's/^/errors=/'
