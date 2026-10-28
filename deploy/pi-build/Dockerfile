# A build environment matching Raspberry Pi OS (Debian 13 "trixie", arm64), so a
# Mac with Apple silicon or any arm64 machine builds the Pi's binary natively in
# a container instead of on the Pi. Used by deploy/build-for-pi.sh.
FROM debian:trixie

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential ca-certificates curl pkg-config \
        libfontconfig1-dev libfreetype6-dev libxkbcommon-dev libwayland-dev \
        libegl1-mesa-dev libgl1-mesa-dev \
        libseat-dev libinput-dev libudev-dev \
    && rm -rf /var/lib/apt/lists/*

ENV RUSTUP_HOME=/usr/local/rustup CARGO_HOME=/usr/local/cargo PATH=/usr/local/cargo/bin:$PATH
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain stable

WORKDIR /src
