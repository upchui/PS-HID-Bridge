#!/bin/bash
# Print a hash of what one toolchain image is built from: its Dockerfile,
# the scripts it copies, the versions.env values passed to it and the
# processor. Two commits with the same hash build the same image, so CI can
# give an existing image the new tag instead of building it again (all
# images keep one tag, tools/toolchain-tag.sh).
#
# Usage: image-inputs.sh IMAGE ARCH [COMMIT]
#   IMAGE: firmware, rust, desktop or apptest; ARCH: x86_64 or aarch64;
#   without COMMIT: the working tree.
set -euo pipefail
cd "$(dirname "$0")/.."
export LC_ALL=C
image=$1
arch=$2
commit=${3:-}

case $image in
    firmware) files=(docker/firmware.Dockerfile docker/scripts/fetch-verify.sh) ;;
    rust | desktop | apptest)
        files=(docker/desktop.Dockerfile docker/scripts/fetch-verify.sh docker/scripts/arch-sha.sh
               docker/scripts/build-qt-module.sh) ;;
    *) echo "image-inputs.sh: unknown image $image" >&2; exit 2 ;;
esac
read -r -a keys <<<"$(make -s print-image-args IMAGE="$image")"

# A file as it is in the working tree or in COMMIT; missing: empty.
show() {
    if [ -n "$commit" ]; then
        git show "$commit:$1" 2>/dev/null || true
    else
        cat "$1" 2>/dev/null || true
    fi
}

{
    echo "image=$image arch=$arch"
    for f in "${files[@]}"; do
        echo "== $f"
        show "$f"
    done
    echo "== build arguments"
    versions=$(show versions.env)
    for k in "${keys[@]}"; do
        grep -E "^$k=" <<<"$versions" || echo "$k (missing)"
    done
} | sha256sum | cut -c1-16
