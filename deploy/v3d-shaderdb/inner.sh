#!/bin/bash
# Runs inside the v3d-shaderdb image: builds the dive3d previews test and runs it
# on Mesa's v3dv driver with the v3d noop drm-shim, printing compile stats.
set -uo pipefail
export CARGO_TARGET_DIR=/target
cd /src

echo "== building" >&2
cargo test --release --locked --no-run 'dive3d::tests::previews' >&2 || exit 1

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
    cargo test --release --locked 'dive3d::tests::previews' -- --ignored --exact --nocapture 2>&1
echo "== test exit status $?"
