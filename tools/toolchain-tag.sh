#!/bin/bash
# Print a short content hash of everything that goes into the toolchain
# images (versions.env and docker/). CI tags the registry images with it and
# rebuilds them only when it changes.
#
# Usage: toolchain-tag.sh [COMMIT]
#   without COMMIT: the working tree; with COMMIT: that commit's files (the
#   same hash for the same content, see tools/image-inputs.sh).
set -euo pipefail
cd "$(dirname "$0")/.."
export LC_ALL=C
commit=${1:-}
if [ -z "$commit" ]; then
    {
        sha256sum versions.env
        find docker -type f -print0 | sort -z | xargs -0 sha256sum
    } | sha256sum | cut -c1-16
    exit 0
fi
hash_of() { git show "$commit:$1" | sha256sum | cut -d' ' -f1; }
{
    printf '%s  %s\n' "$(hash_of versions.env)" versions.env
    git ls-tree -r --name-only "$commit" -- docker | sort | while IFS= read -r f; do
        printf '%s  %s\n' "$(hash_of "$f")" "$f"
    done
} | sha256sum | cut -c1-16
