#!/bin/bash
# Compiles the prototypes in this directory with the Pi 5's shader compiler, inside
# the v3d-shaderdb image, using the test binary that image last built (see
# docs/solar-perf-plan.md for the command). Prints Mesa's raw shader-db log.
arch=$(uname -m)
multiarch=$(gcc -print-multiarch 2>/dev/null || echo "$arch-linux-gnu")
export LD_PRELOAD=/opt/mesa/lib/$multiarch/libv3d_noop_drm_shim.so
export VK_ICD_FILENAMES=/opt/mesa/share/vulkan/icd.d/broadcom_icd.$arch.json
export V3D_DEBUG=shaderdb V3DV_ENABLE_PIPELINE_CACHE=off MESA_SHADER_CACHE_DISABLE=true MESA_LOG_PIPELINE_NAMES=1 WGPU_BACKEND=vulkan WGPU_DEBUG=1
bin=$(ls -t /target/release/deps/spotify_round-* | grep -v '\.d$' | head -1)
echo "== running shader_lab::tests::compile" >&2
timeout 300 "$bin" shader_lab::tests::compile --ignored --exact --nocapture 2>&1
echo "== test shader_lab::tests::compile exit status $?"
