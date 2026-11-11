#!/bin/bash
# Build the desktop app and package it as an AppImage.
# Runs inside the pshb-desktop image (see Makefile target `desktop`).
set -euo pipefail

ROOT=/work
APP_ID=io.github.thealhu.ps5hidbridge
APPDIR=$ROOT/.cache/build/AppDir
OUTPUT=$ROOT/out/PS5-HID-Bridge-x86_64.AppImage
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/desktop/target}
# Set by the Makefile: version (tools/version.sh) and commit date.
: "${PSHB_VERSION:?PSHB_VERSION not set (build with make desktop)}"
PSHB_DATE=${PSHB_DATE:-$(date -u +%F)}

cd "$ROOT/desktop"
cargo build --release --locked -p ps5-hid-bridge

rm -rf "$APPDIR"
mkdir -p "$ROOT/out" "$APPDIR/usr/share/metainfo"
sed -e "s/@VERSION@/$PSHB_VERSION/" -e "s/@DATE@/$PSHB_DATE/" \
    "$ROOT/desktop/packaging/$APP_ID.metainfo.xml" > "$APPDIR/usr/share/metainfo/$APP_ID.metainfo.xml"

export QMAKE=/opt/qt6/bin/qmake
export QML_SOURCES_PATHS=$ROOT/desktop/ui/qml
# Qt >= 6.10 ships the Wayland client platform plugin in qtbase; offscreen
# is for the self-test.
export EXTRA_PLATFORM_PLUGINS="libqwayland.so;libqoffscreen.so"
export EXTRA_QT_MODULES="svg"
export LDAI_OUTPUT=$OUTPUT
export LDAI_NO_APPSTREAM=1

# Libraries the host's own libraries (freetype, fontconfig, ...) also load:
# bundling the older build-system copies makes the host versions fail with
# missing symbols (seen with Manjaro's libbrotlidec vs. AlmaLinux 9's
# libbrotlicommon). Every desktop system ships them.
HOST_LIBS=(libbrotlicommon.so.1 libbrotlidec.so.1 libbrotlienc.so.1 libpng16.so.16 libbz2.so.1
           libglib-2.0.so.0 libpcre.so.1 libgraphite2.so.3 libxml2.so.2 libzstd.so.1)
EXCLUDES=()
for l in "${HOST_LIBS[@]}"; do EXCLUDES+=(--exclude-library "$l"); done

# 1. Executable, Qt libraries, QML modules (Kirigami, Controls) and plugins.
linuxdeploy \
    "${EXCLUDES[@]}" \
    --appdir "$APPDIR" \
    --executable "$TARGET_DIR/release/ps5-hid-bridge" \
    --desktop-file "$ROOT/desktop/packaging/$APP_ID.desktop" \
    --icon-file "$ROOT/desktop/packaging/icons/$APP_ID.svg" \
    --plugin qt

# 2. The Wayland platform plugin needs its shell/decoration/graphics plugins,
#    which linuxdeploy-plugin-qt does not know about yet.
for dir in wayland-shell-integration wayland-decoration-client wayland-graphics-integration-client; do
    mkdir -p "$APPDIR/usr/plugins/$dir"
    for f in /opt/qt6/plugins/$dir/*.so; do
        case "$f" in *-server.so) continue ;; esac  # compositor side only
        cp "$f" "$APPDIR/usr/plugins/$dir/"
    done
    linuxdeploy "${EXCLUDES[@]}" --appdir "$APPDIR" --deploy-deps-only "$APPDIR/usr/plugins/$dir"
done

# Leftovers from earlier runs or other dependency paths.
for l in "${HOST_LIBS[@]}"; do rm -f "$APPDIR/usr/lib/$l"; done

# 3. Package.
linuxdeploy "${EXCLUDES[@]}" --appdir "$APPDIR" --output appimage

ls -l "$OUTPUT"
