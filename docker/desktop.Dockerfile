# syntax=docker/dockerfile:1
#
# Desktop build environment (Rust core, Qt 6 + Kirigami UI, AppImage packaging).
#
# Stages:
#   base       AlmaLinux 9 (glibc 2.34) + gcc-toolset-14 + CMake
#   rust-base  base + Rust            -> used for the core crate, CLI and tests
#   qt         base + Qt from source  -> long first build, cached afterwards
#   kf6        qt + ECM + Kirigami
#   desktop    kf6 + Rust + linuxdeploy -> builds the AppImage
#
# Rust lives in its own branch (rust-base) and is copied into the final stage,
# so bumping the Rust version does not invalidate the Qt layers.
# All version pins come from versions.env via --build-arg.

# Default only silences a docker lint; the Makefile passes versions.env.
ARG ALMA_TAG=9.8

############################################################################
FROM almalinux:${ALMA_TAG} AS base

ARG CMAKE_VERSION
ARG CMAKE_SHA256

RUN dnf -y install epel-release dnf-plugins-core \
 && dnf config-manager --set-enabled crb \
 && dnf -y install \
        gcc gcc-c++ \
        gcc-toolset-14-gcc gcc-toolset-14-gcc-c++ gcc-toolset-14-binutils \
        git-core perl python3 pkgconf-pkg-config patchelf file ninja-build \
        xz tar which findutils diffutils make ccache \
 && dnf clean all

# gcc-toolset-14 first in PATH. Its libstdc++.so is a linker script that links
# newer symbols statically, so binaries only need the system libstdc++.
ENV PATH=/opt/rh/gcc-toolset-14/root/usr/bin:/usr/local/bin:/usr/bin:/bin \
    LD_LIBRARY_PATH=/opt/rh/gcc-toolset-14/root/usr/lib64:/opt/rh/gcc-toolset-14/root/usr/lib \
    CC=/opt/rh/gcc-toolset-14/root/usr/bin/gcc \
    CXX=/opt/rh/gcc-toolset-14/root/usr/bin/g++ \
    CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=/opt/rh/gcc-toolset-14/root/usr/bin/gcc

COPY scripts/fetch-verify.sh /usr/local/bin/fetch-verify.sh

RUN fetch-verify.sh \
        "https://github.com/Kitware/CMake/releases/download/v${CMAKE_VERSION}/cmake-${CMAKE_VERSION}-linux-x86_64.tar.gz" \
        "${CMAKE_SHA256}" /tmp/cmake.tar.gz \
 && tar -xzf /tmp/cmake.tar.gz -C /usr/local --strip-components=1 \
 && rm /tmp/cmake.tar.gz \
 && cmake --version

############################################################################
FROM base AS rust-base

ARG RUSTUP_VERSION
ARG RUSTUP_SHA256
ARG RUST_VERSION

ENV RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:/opt/rh/gcc-toolset-14/root/usr/bin:/usr/local/bin:/usr/bin:/bin

RUN fetch-verify.sh \
        "https://static.rust-lang.org/rustup/archive/${RUSTUP_VERSION}/x86_64-unknown-linux-gnu/rustup-init" \
        "${RUSTUP_SHA256}" /tmp/rustup-init \
 && chmod +x /tmp/rustup-init \
 && CARGO_HOME=/opt/cargo /tmp/rustup-init -y --no-modify-path --profile minimal \
        --default-toolchain "${RUST_VERSION}" -c clippy -c rustfmt \
 && rm /tmp/rustup-init \
 && chmod -R a+rX /opt/rustup /opt/cargo \
 && rustc --version

############################################################################
FROM base AS qt

ARG QT_VERSION
ARG QTBASE_SHA256
ARG QTSHADERTOOLS_SHA256
ARG QTDECLARATIVE_SHA256
ARG QTSVG_SHA256
ARG JOBS=6

RUN dnf -y install \
        fontconfig-devel freetype-devel zlib-devel \
        libxkbcommon-devel libxkbcommon-x11-devel \
        libX11-devel libxcb-devel libXrender-devel libSM-devel libICE-devel \
        xcb-util-devel xcb-util-cursor-devel xcb-util-image-devel \
        xcb-util-keysyms-devel xcb-util-renderutil-devel xcb-util-wm-devel \
        mesa-libGL-devel mesa-libEGL-devel mesa-libgbm-devel libdrm-devel \
        wayland-devel wayland-protocols-devel \
        dbus-devel at-spi2-core-devel \
 && dnf clean all

COPY scripts/build-qt-module.sh /usr/local/bin/build-qt-module.sh

ENV CCACHE_DIR=/ccache \
    QT_VERSION=${QT_VERSION}

# One RUN per module so a failure late in the chain keeps the earlier layers.
RUN --mount=type=cache,target=/ccache JOBS=${JOBS} build-qt-module.sh qtbase        "${QTBASE_SHA256}"
RUN --mount=type=cache,target=/ccache JOBS=${JOBS} build-qt-module.sh qtshadertools "${QTSHADERTOOLS_SHA256}"
RUN --mount=type=cache,target=/ccache JOBS=${JOBS} build-qt-module.sh qtsvg         "${QTSVG_SHA256}"
RUN --mount=type=cache,target=/ccache JOBS=${JOBS} build-qt-module.sh qtdeclarative "${QTDECLARATIVE_SHA256}"

ENV PATH=/opt/qt6/bin:/opt/rh/gcc-toolset-14/root/usr/bin:/usr/local/bin:/usr/bin:/bin \
    QMAKE=/opt/qt6/bin/qmake \
    CMAKE_PREFIX_PATH=/opt/qt6

############################################################################
FROM qt AS kf6

ARG KF_VERSION
ARG ECM_SHA256
ARG KIRIGAMI_SHA256
ARG JOBS=6

RUN set -eux; \
    minor=${KF_VERSION%.*}; \
    cd /tmp; \
    fetch-verify.sh "https://download.kde.org/stable/frameworks/${minor}/extra-cmake-modules-${KF_VERSION}.tar.xz" \
        "${ECM_SHA256}" ecm.tar.xz; \
    mkdir ecm && tar -xJf ecm.tar.xz -C ecm --strip-components=1; \
    cmake -S ecm -B ecm/build -G Ninja \
        -DCMAKE_INSTALL_PREFIX=/opt/qt6 \
        -DBUILD_TESTING=OFF \
        -DBUILD_HTML_DOCS=OFF -DBUILD_MAN_DOCS=OFF -DBUILD_QTHELP_DOCS=OFF; \
    cmake --build ecm/build; \
    cmake --install ecm/build; \
    rm -rf /tmp/ecm /tmp/ecm.tar.xz

# Kirigami's own translations (poqm) need Qt LinguistTools (qttools), which we
# do not build; the bridge UI does not depend on them.
RUN --mount=type=cache,target=/ccache set -eux; \
    minor=${KF_VERSION%.*}; \
    cd /tmp; \
    fetch-verify.sh "https://download.kde.org/stable/frameworks/${minor}/kirigami-${KF_VERSION}.tar.xz" \
        "${KIRIGAMI_SHA256}" kirigami.tar.xz; \
    mkdir kirigami && tar -xJf kirigami.tar.xz -C kirigami --strip-components=1; \
    rm -rf kirigami/poqm; \
    cmake -S kirigami -B kirigami/build -G Ninja \
        -DCMAKE_BUILD_TYPE=Release \
        -DCMAKE_INSTALL_PREFIX=/opt/qt6 \
        -DCMAKE_PREFIX_PATH=/opt/qt6 \
        -DKDE_INSTALL_USE_QT_SYS_PATHS=ON \
        -DBUILD_TESTING=OFF \
        -DBUILD_EXAMPLES=OFF \
        -DCMAKE_C_COMPILER_LAUNCHER=ccache -DCMAKE_CXX_COMPILER_LAUNCHER=ccache; \
    cmake --build kirigami/build --parallel "${JOBS}"; \
    cmake --install kirigami/build; \
    rm -rf /tmp/kirigami /tmp/kirigami.tar.xz

############################################################################
FROM kf6 AS desktop

ARG LINUXDEPLOY_TAG
ARG LINUXDEPLOY_SHA256
ARG LINUXDEPLOY_QT_TAG
ARG LINUXDEPLOY_QT_SHA256
ARG LINUXDEPLOY_APPIMAGE_TAG
ARG LINUXDEPLOY_APPIMAGE_SHA256

COPY --from=rust-base /opt/rustup /opt/rustup
COPY --from=rust-base /opt/cargo /opt/cargo

ENV RUSTUP_HOME=/opt/rustup \
    PATH=/opt/cargo/bin:/opt/qt6/bin:/opt/rh/gcc-toolset-14/root/usr/bin:/usr/local/bin:/usr/bin:/bin

# The tools are AppImages themselves; extract them once (no FUSE in containers).
RUN set -eux; \
    get() { \
        name=$1; repo=$2; tag=$3; sha=$4; \
        fetch-verify.sh "https://github.com/linuxdeploy/${repo}/releases/download/${tag}/${name}-x86_64.AppImage" \
            "$sha" "/tmp/${name}.AppImage"; \
        chmod +x "/tmp/${name}.AppImage"; \
        cd /opt && "/tmp/${name}.AppImage" --appimage-extract >/dev/null; \
        mv /opt/squashfs-root "/opt/${name}"; \
        ln -s "/opt/${name}/AppRun" "/usr/local/bin/${name}"; \
        rm "/tmp/${name}.AppImage"; \
    }; \
    get linuxdeploy linuxdeploy "${LINUXDEPLOY_TAG}" "${LINUXDEPLOY_SHA256}"; \
    get linuxdeploy-plugin-qt linuxdeploy-plugin-qt "${LINUXDEPLOY_QT_TAG}" "${LINUXDEPLOY_QT_SHA256}"; \
    get linuxdeploy-plugin-appimage linuxdeploy-plugin-appimage "${LINUXDEPLOY_APPIMAGE_TAG}" "${LINUXDEPLOY_APPIMAGE_SHA256}"; \
    chmod -R a+rX /opt/linuxdeploy*
