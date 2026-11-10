#!/bin/bash
# Build and install one Qt module from the official source tarball into /opt/qt6.
# Usage: build-qt-module.sh <module> <sha256>
# Environment: QT_VERSION, JOBS
set -euo pipefail

module=$1
sha=$2
minor=${QT_VERSION%.*}
url="https://download.qt.io/official_releases/qt/${minor}/${QT_VERSION}/submodules/${module}-everywhere-src-${QT_VERSION}.tar.xz"

work=/tmp/qtbuild/${module}
mkdir -p "$work"
cd "$work"
fetch-verify.sh "$url" "$sha" src.tar.xz
tar -xJf src.tar.xz --strip-components=1
rm src.tar.xz

mkdir build
cd build
if [ "$module" = qtbase ]; then
    # Bundle the small 3rd-party libs so the AppImage needs fewer host libraries.
    # Wayland client support lives in qtbase since Qt 6.10 (libqwayland.so).
    ../configure \
        -prefix /opt/qt6 \
        -release \
        -opensource -confirm-license \
        -nomake examples -nomake tests \
        -ccache \
        -no-icu \
        -no-openssl \
        -no-glib \
        -dbus-linked \
        -fontconfig -system-freetype \
        -qt-harfbuzz -qt-pcre -qt-libpng -qt-libjpeg -system-zlib \
        -xcb -bundled-xcb-xinput \
        -feature-wayland \
        -no-feature-vulkan
else
    /opt/qt6/bin/qt-configure-module .. -- -DCMAKE_C_COMPILER_LAUNCHER=ccache -DCMAKE_CXX_COMPILER_LAUNCHER=ccache
fi

cmake --build . --parallel "${JOBS}"
cmake --install .

cd /
rm -rf /tmp/qtbuild
