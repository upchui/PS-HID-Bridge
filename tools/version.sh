#!/bin/bash
# Print the project version after checking that the firmware, the desktop
# workspace and the AppStream metainfo all carry the same one.
set -euo pipefail
cd "$(dirname "$0")/.."

firmware=$(sed -nE 's/^project\(ps5_hid_bridge VERSION ([0-9.]+).*/\1/p' firmware/CMakeLists.txt)
desktop=$(sed -nE '/^\[workspace\.package\]/,/^\[/ s/^version = "([^"]+)"/\1/p' desktop/Cargo.toml)
metainfo=$(sed -nE 's/.*<release version="([^"]+)".*/\1/p' \
    desktop/packaging/io.github.thealhu.ps5hidbridge.metainfo.xml | head -n 1)

if [ -z "$firmware" ] || [ "$firmware" != "$desktop" ] || [ "$firmware" != "$metainfo" ]; then
    echo "Version mismatch: firmware/CMakeLists.txt '$firmware'," \
        "desktop/Cargo.toml '$desktop', metainfo '$metainfo'" >&2
    exit 1
fi
echo "$firmware"
