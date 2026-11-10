#!/bin/bash
# Verify that every firmware submodule is checked out at the tag pinned in
# versions.env (and that the superproject records the same commit).
set -euo pipefail

cd "$(dirname "$0")/.."
# shellcheck disable=SC1091
source versions.env

fail=0
check() {
    local path=$1 tag=$2
    if [ ! -e "$path/.git" ]; then
        echo "MISSING  $path  (run: make submodules)"
        fail=1
        return
    fi
    local recorded head want
    # The index holds the gitlink commit (works before the first commit, too).
    recorded=$(git ls-files -s "$path" | awk '{print $2}')
    head=$(git -C "$path" rev-parse HEAD)
    if ! want=$(git -C "$path" rev-parse -q --verify "refs/tags/${tag}^{commit}"); then
        echo "NO TAG   $path  ($tag not found; try: git -C $path fetch --tags)"
        fail=1
        return
    fi
    if [ "$head" != "$want" ] || [ "$recorded" != "$want" ]; then
        echo "MISMATCH $path  tag $tag=$want, checked out=$head, recorded=$recorded"
        fail=1
    else
        echo "ok       $path  $tag"
    fi
}

check firmware/lib/pico-sdk         "$PICO_SDK_TAG"
check firmware/lib/tinyusb          "$TINYUSB_TAG"
check firmware/lib/ioLibrary_Driver "$IOLIBRARY_TAG"
check firmware/lib/monocypher       "$MONOCYPHER_TAG"

exit $fail
