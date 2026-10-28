#!/bin/bash
# Runs inside the v3d-shaderdb image: builds the dive3d previews test and runs it
# on Mesa's v3dv driver with the v3d noop drm-shim, printing compile stats.
set -uo pipefail
export CARGO_TARGET_DIR=/target
# Built from a copy with fresh modification times: cargo decides what to
# rebuild by them, and checkouts sharing /target (or a `git archive` export,
# which carries the commit's times) would otherwise reuse another's build.
# Copying, building and taking a private copy of the test binary all happen
# under one lock: copied before it, the sources could be older than another
# checkout's build made while this one waited, and that build would be used.
mkdir -p /work
cd /work
echo "== building" >&2
flock /target/.shaderdb.lock bash -c '
    set -o pipefail
    tar -C /src --exclude=./target --exclude=./.git -cf - . | tar -C /work -xmf -
    cargo test --release --locked --no-run --message-format=json 2>/dev/stderr \
        | grep "\"test\":true" \
        | sed -n "s/.*\"executable\":\"\([^\"]*\)\".*/\1/p" | head -1 > /work/.test-binary
    cp "$(cat /work/.test-binary)" /work/test-binary
' >&2 || exit 1

export LD_PRELOAD=/opt/mesa/lib/aarch64-linux-gnu/libv3d_noop_drm_shim.so
export VK_ICD_FILENAMES=/opt/mesa/share/vulkan/icd.d/broadcom_icd.aarch64.json
export V3D_DEBUG=shaderdb
export V3DV_ENABLE_PIPELINE_CACHE=off
export MESA_SHADER_CACHE_DISABLE=true
export MESA_LOG_PIPELINE_NAMES=1
export WGPU_BACKEND=vulkan
export WGPU_DEBUG=1
export DIVE3D_PREVIEW_DIR=/tmp/previews
export DIVE3D_PREVIEW_OFFSETS=0
mkdir -p "$DIVE3D_PREVIEW_DIR"

echo "== running" >&2
timeout "${SHADERDB_TIMEOUT:-300}" \
    /work/test-binary 'dive3d::tests::previews' --ignored --exact --nocapture 2>&1
echo "== test exit status $?"
