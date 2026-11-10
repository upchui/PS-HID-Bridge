#!/bin/bash
# Print a short content hash of everything that goes into the toolchain
# images (versions.env and docker/). CI tags the registry images with it and
# rebuilds them only when it changes.
set -euo pipefail
cd "$(dirname "$0")/.."
export LC_ALL=C
{
    sha256sum versions.env
    find docker -type f -print0 | sort -z | xargs -0 sha256sum
} | sha256sum | cut -c1-16
