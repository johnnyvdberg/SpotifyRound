#!/usr/bin/env bash
# Pictures of every place the Nautilus can be, and what a change did to them.
#
#   deploy/shots.sh run NAME [NAME=VALUE ...]
#       renders every place of dive3d::places::all (same seeds and times
#       every run; new legs appear by themselves) on this machine into
#       target/shots/NAME/, with target/shots/NAME/index.html, a contact
#       sheet per sea. NAME=VALUE pairs go to the test: DIVE3D_SHOTS=cave
#       keeps the places whose names contain "cave", DIVE3D_SKIP_EFFECTS=...
#   PI=<user>@<pi> deploy/shots.sh pi NAME [NAME=VALUE ...]
#       the same on the Pi through target/pi/spotify-round-tests
#       (deploy/build-for-pi.sh --tests), so the driver's own rendering
#       shows. Freezes the kiosk for the run and resumes it, under the
#       benchmark lock.
#   deploy/shots.sh compare A B
#       writes target/shots/compare-A-B.html: for every place before | after
#       | difference x16 with a change score, most-changed first, and the same
#       table on stdout.
set -euo pipefail
cd "$(dirname "$0")/.."

usage() {
    sed -n '2,17p' "$0" | sed 's/^# \{0,1\}//' >&2
    exit 2
}

# The system python has numpy and Pillow on some Macs, Homebrew's on others.
python() {
    local py
    for py in python3 /usr/bin/python3 /opt/homebrew/bin/python3; do
        if "$py" -c 'import numpy, PIL' 2>/dev/null; then
            "$py" deploy/shots.py "$@"
            return
        fi
    done
    echo "need a python3 with numpy and Pillow" >&2
    exit 1
}

pairs() {
    local pair
    for pair in "$@"; do
        [[ $pair == *=* ]] || usage
        printf '%s\0' "$pair"
    done
}

run() {
    local name=$1
    shift
    local dir=target/shots/$name
    rm -rf "$dir"
    mkdir -p "$dir"
    local -a env=()
    local pair
    for pair in "$@"; do
        [[ $pair == *=* ]] || usage
        env+=("$pair")
    done
    env "DIVE3D_SHOTS_DIR=$dir" ${env[@]+"${env[@]}"} \
        cargo test --release dive3d::tests::screenshots -- --ignored --nocapture
    python index "$dir"
}

pi() {
    : "${PI:?set PI=<user>@<pi>}"
    local name=$1
    shift
    local bin=target/pi/spotify-round-tests remote=/tmp/srt-shots-$name
    [[ -f $bin ]] || { echo "no $bin: run deploy/build-for-pi.sh --tests" >&2; exit 1; }
    scp -q "$bin" "$PI:$remote"
    local here there
    here=$(md5 -q "$bin" 2>/dev/null || md5sum "$bin" | cut -d' ' -f1)
    there=$(ssh "$PI" "md5sum $remote" | cut -d' ' -f1)
    [[ $here == "$there" ]] || { echo "the Pi's copy differs from $bin" >&2; exit 1; }
    local env="DIVE3D_SHOTS_DIR=/tmp/shots-$name" pair
    for pair in "$@"; do
        [[ $pair == *=* ]] || usage
        env+=" $(printf %q "$pair")"
    done
    ssh "$PI" "set -e; exec 9>/tmp/nautilus-bench.lock
        flock -n 9 || { echo 'waiting for another benchmark on the Pi'; flock 9; }
        P=\$(pgrep -x spotify-round || true)
        [ -n \"\$P\" ] && kill -STOP \$P; trap '[ -n \"\$P\" ] && kill -CONT \$P; rm -f $remote' EXIT
        mkdir -p /tmp/shots-$name; cd /tmp
        env $env $remote dive3d::tests::screenshots --ignored --nocapture 2>&1 | grep -v '^MESA'"
    local dir=target/shots/$name
    rm -rf "$dir"
    mkdir -p "$dir"
    scp -q -r "$PI:/tmp/shots-$name/." "$dir/"
    ssh "$PI" "rm -rf /tmp/shots-$name"
    python index "$dir"
}

compare() {
    [[ -d target/shots/$1 && -d target/shots/$2 ]] || { echo "no target/shots/$1 or $2" >&2; exit 1; }
    python compare "target/shots/$1" "target/shots/$2" "target/shots/compare-$1-$2.html"
}

case "${1:-}" in
    run) [[ $# -ge 2 ]] || usage; shift; run "$@" ;;
    pi) [[ $# -ge 2 ]] || usage; shift; pi "$@" ;;
    compare) [[ $# -eq 3 ]] || usage; compare "$2" "$3" ;;
    *) usage ;;
esac
