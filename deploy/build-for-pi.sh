#!/usr/bin/env bash
# Build the Raspberry Pi binary in an arm64 Debian 13 container, which matches
# Raspberry Pi OS, and leave it in target/pi/.
#
#   deploy/build-for-pi.sh            the app
#   deploy/build-for-pi.sh --tests    also the test binary, for the benchmarks
#   deploy/build-for-pi.sh -- --features kms   extra arguments for cargo
#
# Needs Docker. On an arm64 host (a Mac with Apple silicon) the container runs
# natively; elsewhere Docker emulates arm64, which works but is slow.
set -euo pipefail
cd "$(dirname "$0")/.."

tests=false
extra=()
while [[ $# -gt 0 ]]; do
    case "$1" in
        --tests) tests=true ;;
        --) shift; extra=("$@"); break ;;
        *) echo "unknown option: $1" >&2; exit 2 ;;
    esac
    shift
done

image=spotify-round-pi-build
docker build --platform linux/arm64 -t "$image" deploy/pi-build

# The registry and the build directory live in volumes, so rebuilds are
# incremental and the host's own target/ is left alone. Checkouts share one
# build volume (one per checkout filled the Docker VM's disk), so touch the
# sources before building and check the binary changed; or give a checkout
# its own volume with SPOTIFY_ROUND_PI_VOLUME=<name>.
target_volume=${SPOTIFY_ROUND_PI_VOLUME:-spotify-round-pi-target}
run() {
    docker run --rm --platform linux/arm64 \
        -v "$PWD":/src \
        -v spotify-round-pi-cargo:/usr/local/cargo/registry \
        -v "$target_volume":/target \
        -e CARGO_TARGET_DIR=/target \
        "$image" "$@"
}

mkdir -p target/pi
run sh -c "cargo build --release --locked ${extra[*]:-} && cp /target/release/spotify-round /src/target/pi/"
echo "built target/pi/spotify-round"
if $tests; then
    # The test binary's name carries a hash: ask cargo for it.
    run sh -c "cargo test --release --locked --no-run --message-format=json ${extra[*]:-} 2>/dev/null \
        | grep -o '\"executable\":\"[^\"]*spotify_round-[^\"]*\"' | tail -1 | cut -d'\"' -f4 \
        | xargs -I{} cp {} /src/target/pi/spotify-round-tests"
    echo "built target/pi/spotify-round-tests"
fi
