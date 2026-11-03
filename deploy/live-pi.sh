#!/bin/bash
# Live measurement of a build on the kiosk's display, run ON THE PI (copy it to
# /tmp/d1/ first; nothing here runs cargo or needs sudo):
#
#   scp deploy/live-pi.sh target/pi/spotify-round <user>@<pi>:/tmp/d1/
#   ssh <user>@<pi> /tmp/d1/live-pi.sh gl /tmp/d1/spotify-round 45 \
#       SPOTIFY_ROUND_SCENE=nautilus SPOTIFY_ROUND_SEED=22 SPOTIFY_ROUND_DISPLAY=gl
#
# usage: live-pi.sh TAG BINARY SECONDS [VAR=VALUE ...]
# Runs BINARY as a second client under the kiosk's cage (the kiosk frozen with
# SIGSTOP and resumed on every exit), --mock, holding the bench lock. Prints
# the visualizer/full-speed log lines' medians, the GPU's render-queue time per
# second, and takes a screenshot at the middle of the run.
set -u
TAG=$1; BIN=$2; SECS=$3; shift 3
# Logs and screenshots go next to the script.
D=$(cd "$(dirname "$0")" && pwd)
mkdir -p $D
exec 9>/tmp/nautilus-bench.lock
flock 9
K=$(pgrep -x spotify-round | head -1)
[ -n "$K" ] && kill -STOP $K
P=
cleanup() { [ -n "$P" ] && kill $P 2>/dev/null; sleep 0.5; [ -n "$K" ] && kill -CONT $K; }
trap cleanup EXIT INT TERM
stats() { awk '$1=="render"{r=$4;j=$3} $1=="tfu"{t=$4;tj=$3} $1=="bin"{b=$4} END{print r, b, t, j, tj}' /sys/class/drm/card0/device/gpu_stats; }
env XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 \
    SPOTIFY_ROUND_LOG=info,spotify_round::visuals=debug "$@" \
    timeout $SECS $BIN --mock > $D/$TAG.log 2>&1 &
P=$!
W=14
sleep $W
read r0 b0 t0 j0 tj0 < <(stats); s0=$(date +%s.%N)
sleep $(( (SECS - W - 3) / 2 ))
XDG_RUNTIME_DIR=/run/user/1000 WAYLAND_DISPLAY=wayland-0 grim -l 1 $D/$TAG.png 2>/dev/null
sleep $(( (SECS - W - 3) / 2 ))
read r1 b1 t1 j1 tj1 < <(stats); s1=$(date +%s.%N)
wait $P 2>/dev/null; P=
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
