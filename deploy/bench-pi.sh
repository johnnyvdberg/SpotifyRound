#!/usr/bin/env bash
# Time the Nautilus on the Pi from the Mac: copy the test binary over, freeze
# the kiosk while `dive3d::tests::hotspots` runs, and bring its table home.
#
#   PI=<user>@<pi> deploy/bench-pi.sh run TAG [NAME=VALUE ...]
#       times target/pi/spotify-round-tests (deploy/build-for-pi.sh --tests)
#       and leaves the places in target/bench/TAG.csv. NAME=VALUE pairs go to
#       the benchmark: DIVE3D_HOTSPOT_SCENES=coral, DIVE3D_SKIP=..., and so on.
#   deploy/bench-pi.sh compare BEFORE AFTER
#       sets two runs side by side, place by place, slowest first.
#
# The defaults are four visits to every sea, six frames a place. The binary
# is checked against the Pi's copy by checksum, since a stale one is the
# easiest way to measure the wrong build.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
    sed -n '4,10p' "$0" >&2
    exit 2
}

run() {
    local tag=$1
    shift
    : "${PI:?set PI=<user>@<pi>}"
    local bin=target/pi/spotify-round-tests remote=/tmp/srt-$tag
    [[ -f $bin ]] || { echo "no $bin: run deploy/build-for-pi.sh --tests" >&2; exit 1; }
    scp -q "$bin" "$PI:$remote"
    local here there
    here=$(md5 -q "$bin" 2>/dev/null || md5sum "$bin" | cut -d' ' -f1)
    there=$(ssh "$PI" "md5sum $remote" | cut -d' ' -f1)
    [[ $here == "$there" ]] || { echo "the Pi's copy differs from $bin" >&2; exit 1; }

    local env="DIVE3D_HOTSPOT_SEEDS=0,250000,500000,750000 DIVE3D_HOTSPOT_FRAMES=6"
    env+=" DIVE3D_HOTSPOT_DIR=/tmp/hot-$tag"
    for pair in "$@"; do
        [[ $pair == *=* ]] || usage
        env+=" $(printf %q "$pair")"
    done
    # The kiosk shares the GPU (and a global perfmon would count it), so it
    # stops for the run and carries on however the run ends.
    ssh "$PI" "set -e; P=\$(pgrep -x spotify-round || true)
        [ -n \"\$P\" ] && kill -STOP \$P; trap '[ -n \"\$P\" ] && kill -CONT \$P' EXIT
        mkdir -p /tmp/hot-$tag; cd /tmp
        env $env $remote dive3d::tests::hotspots --ignored --nocapture 2>&1 | grep -v '^MESA'"
    mkdir -p target/bench
    scp -q "$PI:/tmp/hot-$tag/hotspots.csv" "target/bench/$tag.csv"
    echo "saved target/bench/$tag.csv"
}

compare() {
    local before=target/bench/$1.csv after=target/bench/$2.csv
    python3 - "$before" "$after" <<'EOF'
import csv, statistics, sys

def load(path):
    with open(path) as f:
        return {(r["seed"], r["scene"], r["place"]): float(r["mean_ms"]) for r in csv.DictReader(f)}

before, after = load(sys.argv[1]), load(sys.argv[2])
both = sorted(set(before) & set(after), key=lambda k: -after[k])
print(f"{'seed':>8} {'scene':<10} {'place':>5} {'before':>7} {'after':>7} {'change':>7}")
for k in both:
    print(f"{k[0]:>8} {k[1]:<10} {k[2]:>5} {before[k]:7.2f} {after[k]:7.2f} {after[k] - before[k]:+7.2f}")
if both:
    b, a = [before[k] for k in both], [after[k] for k in both]
    print(f"{len(both)} places: median {statistics.median(b):.2f} -> {statistics.median(a):.2f} ms, "
          f"worst {max(b):.2f} -> {max(a):.2f} ms, over 14 ms {sum(x > 14 for x in b)} -> {sum(x > 14 for x in a)}")
EOF
}

case "${1:-}" in
    run) [[ $# -ge 2 ]] || usage; shift; run "$@" ;;
    compare) [[ $# -eq 3 ]] || usage; compare "$2" "$3" ;;
    *) usage ;;
esac
