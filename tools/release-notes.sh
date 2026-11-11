#!/bin/bash
# Release notes for an automatic pre-release: the commits since the previous
# build and a short guide to the files.
# Usage: tools/release-notes.sh TAG
set -euo pipefail
cd "$(dirname "$0")/.."

tag=${1:?usage: tools/release-notes.sh TAG}
repo=${GITHUB_REPOSITORY:-upchui/PS-HID-Bridge}
prev=$(git describe --tags --abbrev=0 --match 'v*' HEAD~1 2>/dev/null || true)

echo "Automatic build of commit $(git rev-parse --short HEAD) on \`main\`."
echo "Pre-releases are built from every commit and have only been tested automatically."
echo
echo "## Changes"
echo
if [ -n "$prev" ]; then
    git log --no-merges --format='- %s (%h)' "$prev..HEAD"
    echo
    echo "Full diff: https://github.com/$repo/compare/$prev...$tag"
else
    git log --no-merges --max-count=20 --format='- %s (%h)' HEAD
fi

cat <<'EOF'

## Files

| File | Purpose |
|---|---|
| `PS-HID-Bridge-x86_64.AppImage` | desktop app: make it executable (`chmod +x`) and start it |
| `pshb-cli-x86_64-linux` | command line tool |
| `ps-hid-bridge.uf2` | board firmware: update via the web interface (System), or copy it to the board in BOOTSEL mode |
| `ps-hid-bridge-pt.uf2` | partition table: copy it once via BOOTSEL before the first firmware |
| other `.uf2` files | test and recovery images, see "Other images" in the README |
| `SHA256SUMS` | checksums (`sha256sum -c SHA256SUMS`) |
EOF
