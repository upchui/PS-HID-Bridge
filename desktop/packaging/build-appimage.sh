#!/bin/bash
# Build the desktop app and package it as an AppImage.
# Runs inside the pshb-desktop image (see Makefile target `desktop`).
set -euo pipefail

ROOT=/work
APP_ID=io.github.thealhu.ps5hidbridge
APPDIR=$ROOT/.cache/build/AppDir
OUTPUT=$ROOT/out/PS5-HID-Bridge-x86_64.AppImage
TARGET_DIR=${CARGO_TARGET_DIR:-$ROOT/desktop/target}

cd "$ROOT/desktop"
cargo build --release --locked -p ps5-hid-bridge

rm -rf "$APPDIR"
mkdir -p "$ROOT/out"

export QMAKE=/opt/qt6/bin/qmake
export QML_SOURCES_PATHS=$ROOT/desktop/ui/qml
# Qt >= 6.10 ships the Wayland client platform plugin in qtbase.
export EXTRA_PLATFORM_PLUGINS="libqwayland.so"
export EXTRA_QT_MODULES="svg;waylandclient"
export LDAI_OUTPUT=$OUTPUT
export LDAI_NO_APPSTREAM=1

linuxdeploy \
    --appdir "$APPDIR" \
    --executable "$TARGET_DIR/release/ps5-hid-bridge" \
    --desktop-file "$ROOT/desktop/packaging/$APP_ID.desktop" \
    --icon-file "$ROOT/desktop/packaging/icons/$APP_ID.svg" \
    --plugin qt \
    --output appimage

ls -l "$OUTPUT"
