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
if [ -e .git ]; then
    # A shallow clone or a git error would give a wrong (smaller) number:
    # fail instead of producing a version that is not higher.
    shallow=$(git rev-parse --is-shallow-repository)
    if [ "$shallow" != false ]; then
        echo "Shallow clone: the commit count is unknown (git fetch --unshallow)" >&2
        exit 1
    fi
    build=$(git rev-list --count HEAD)
else
    build=0 # not a git checkout (e.g. a source tarball)
fi
echo "$base.$build"
