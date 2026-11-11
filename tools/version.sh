#!/bin/bash
# Print the version: MAJOR.MINOR from the file VERSION (the only place it is
# edited by hand) plus the number of commits as the last part, so every
# commit on main has a higher version than the one before. The firmware also
# uses that number as its binary version, which the boot ROM compares to pick
# the newer of the two firmware slots.
set -euo pipefail
cd "$(dirname "$0")/.."

base=$(tr -d '[:space:]' < VERSION)
if ! [[ $base =~ ^[0-9]+\.[0-9]+$ ]]; then
    echo "VERSION must be MAJOR.MINOR (e.g. 0.9), found '$base'" >&2
    exit 1
fi
# Without git history (e.g. a source tarball) the last part is 0.
build=$(git rev-list --count HEAD 2>/dev/null || echo 0)
echo "$base.$build"
