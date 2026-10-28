#!/bin/bash
# Print Raspberry Pi 5 (V3D 7.1, Mesa v3dv) shader compile stats for every wgpu
# pipeline the dive3d previews test builds, without Pi hardware.
#
#   shaderdb.sh <path to a SpotifyRound checkout> [raw log file]
#
# Builds (once) a Docker image with Mesa 26.2.2 (what Raspberry Pi OS trixie
# ships) compiled with the broadcom Vulkan driver and the v3d noop drm-shim,
# builds the test in a Docker volume (not the checkout's target/), runs it with
# V3D_DEBUG=shaderdb and prints one line per compiled shader, labelled with the
# wgpu pipeline label and the entry point. The checkout is mounted read-only.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "${1:?usage: shaderdb.sh <repo checkout> [raw log]}" && pwd)
raw=${2:-$(mktemp -t v3d-shaderdb.XXXXXX)}
# One build directory per checkout: cargo goes by modification times, and a
# `git archive` export carries old ones, so a shared one can go stale.
volume=v3d-shaderdb-target-$(printf %s "$repo" | shasum | cut -c1-12)

docker build -q -t v3d-shaderdb "$here" >/dev/null

docker run --rm \
    -v "$repo":/src:ro \
    -v "$here":/w:ro \
    -v "$volume":/target \
    -v v3d-shaderdb-cargo:/usr/local/cargo/registry \
    -e SHADERDB_TIMEOUT="${SHADERDB_TIMEOUT:-300}" \
    v3d-shaderdb bash /w/inner.sh >"$raw" 2>&1 || true

python3 "$here/label.py" "$raw"
echo "(raw log: $raw)" >&2
