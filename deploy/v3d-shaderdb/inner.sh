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

# The image is built for whatever the host is: an arm64 Mac or Pi, or an
# x86_64 machine. The driver compiles for the V3D either way.
arch=$(uname -m)
multiarch=$(gcc -print-multiarch 2>/dev/null || echo "$arch-linux-gnu")
export LD_PRELOAD=/opt/mesa/lib/$multiarch/libv3d_noop_drm_shim.so
export VK_ICD_FILENAMES=/opt/mesa/share/vulkan/icd.d/broadcom_icd.$arch.json
export V3D_DEBUG=shaderdb
export V3DV_ENABLE_PIPELINE_CACHE=off
export MESA_SHADER_CACHE_DISABLE=true
export MESA_LOG_PIPELINE_NAMES=1
export WGPU_BACKEND=vulkan
export WGPU_DEBUG=1
export DIVE3D_PREVIEW_DIR=/tmp/previews
export DIVE3D_PREVIEW_OFFSETS=0
mkdir -p "$DIVE3D_PREVIEW_DIR"

# Each of these builds every pipeline of its scene: the Nautilus' previews,
# the planet's and the solar tour's benchmarks (which also stream a whole
# planet, its moons and a sky through every generator pipeline).
export PLANET_SEED=7
tests=${SHADERDB_TESTS:-dive3d::tests::previews planet::tests::frame_time solar::tests::frame_time}
for test in $tests; do
    echo "== running $test" >&2
    timeout "${SHADERDB_TIMEOUT:-300}" \
        /work/test-binary "$test" --ignored --exact --nocapture 2>&1
    echo "== test $test exit status $?"
done
