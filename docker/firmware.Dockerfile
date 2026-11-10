# syntax=docker/dockerfile:1
#
# Firmware build environment: Arm GNU toolchain, CMake, Ninja, picotool.
# The pico-sdk itself comes from the firmware/lib submodule; a second,
# identical checkout is only used here to build picotool.
# Also used for the host-side unit tests and the board simulator.

# Default only silences a docker lint; the Makefile passes versions.env.
ARG UBUNTU_TAG=24.04
FROM ubuntu:${UBUNTU_TAG}

ARG PICO_SDK_TAG
ARG PICOTOOL_TAG
ARG ARM_TOOLCHAIN_VERSION
ARG ARM_TOOLCHAIN_SHA256

RUN apt-get update \
 && DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
        build-essential cmake ninja-build git ca-certificates curl xz-utils \
        python3 python3-cryptography \
 && rm -rf /var/lib/apt/lists/*

COPY scripts/fetch-verify.sh /usr/local/bin/fetch-verify.sh

RUN fetch-verify.sh \
        "https://developer.arm.com/-/media/Files/downloads/gnu/${ARM_TOOLCHAIN_VERSION}/binrel/arm-gnu-toolchain-${ARM_TOOLCHAIN_VERSION}-x86_64-arm-none-eabi.tar.xz" \
        "${ARM_TOOLCHAIN_SHA256}" /tmp/arm.tar.xz \
 && mkdir -p /opt/arm \
 && tar -xJf /tmp/arm.tar.xz -C /opt/arm --strip-components=1 \
 && rm /tmp/arm.tar.xz \
 && rm -rf /opt/arm/share/doc /opt/arm/share/info /opt/arm/share/man

ENV PATH=/opt/arm/bin:$PATH

# picotool: needed by the SDK for UF2 generation, hashing and partition tables
# on RP2350. Built without libusb (no device access needed in the container);
# mbedtls from the SDK enables hashing/signing support.
RUN git clone -q --depth 1 -b "${PICO_SDK_TAG}" https://github.com/raspberrypi/pico-sdk.git /tmp/pico-sdk \
 && git -C /tmp/pico-sdk submodule update -q --init --depth 1 lib/mbedtls \
 && git clone -q --depth 1 -b "${PICOTOOL_TAG}" https://github.com/raspberrypi/picotool.git /tmp/picotool \
 && cmake -S /tmp/picotool -B /tmp/picotool/build -G Ninja \
        -DPICO_SDK_PATH=/tmp/pico-sdk \
        -DPICOTOOL_NO_LIBUSB=1 \
        -DPICOTOOL_FLAT_INSTALL=1 \
        -DCMAKE_INSTALL_PREFIX=/opt/picotool \
 && cmake --build /tmp/picotool/build \
 && cmake --install /tmp/picotool/build \
 && rm -rf /tmp/pico-sdk /tmp/picotool \
 && /opt/picotool/picotool/picotool version

ENV picotool_DIR=/opt/picotool/picotool \
    PATH=/opt/picotool/picotool:/opt/arm/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin
