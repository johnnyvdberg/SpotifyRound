# Mesa's Raspberry Pi 5 Vulkan driver (v3dv, V3D 7.1) plus the v3d noop
# drm-shim, so V3D shaders compile without Pi hardware, and the Rust toolchain
# of deploy/pi-build/Dockerfile to build the app against it.
FROM debian:trixie AS mesa

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates git pkg-config meson ninja-build \
        python3-mako python3-yaml python3-packaging python3-pycparser \
        bison flex glslang-tools libdrm-dev libexpat1-dev zlib1g-dev libzstd-dev \
    && rm -rf /var/lib/apt/lists/*

# Raspberry Pi OS trixie ships mesa 26.2.2-1~bpo13+0~rpt1 (checked 2026-10-28).
ARG MESA_REF=mesa-26.2.2
RUN git clone --depth 1 --branch ${MESA_REF} https://gitlab.freedesktop.org/mesa/mesa.git /mesa
WORKDIR /mesa
COPY patch-mesa.sh /patch-mesa.sh
RUN sh /patch-mesa.sh
RUN meson setup build --prefix=/opt/mesa --buildtype=release \
        -Dvulkan-drivers=broadcom -Dgallium-drivers=v3d -Dtools=drm-shim \
        -Dplatforms= -Dglx=disabled -Degl=disabled -Dgbm=disabled \
        -Dllvm=disabled -Dvideo-codecs= -Dvalgrind=disabled -Dlibunwind=disabled \
        -Dbuild-tests=false \
    && ninja -C build \
    && ninja -C build install \
    && find build -name '*drm_shim*.so' -exec cp {} /opt/mesa/lib/ \;

FROM debian:trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl pkg-config \
        libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libwayland-dev \
        libegl1-mesa-dev libgl1-mesa-dev \
        libseat-dev libinput-dev libudev-dev \
        clang libclang-dev \
        libvulkan1 vulkan-tools libdrm2 libexpat1 zlib1g libzstd1 \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain stable

COPY --from=mesa /opt/mesa /opt/mesa
WORKDIR /src
