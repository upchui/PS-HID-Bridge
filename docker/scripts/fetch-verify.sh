#!/bin/sh
# Download a file and verify its SHA-256 checksum.
# Usage: fetch-verify.sh <url> <sha256> <destination>
set -eu

url=$1
sha=$2
dest=$3

curl -fsSL --retry 5 --retry-delay 3 -o "$dest" "$url"
echo "$sha  $dest" | sha256sum -c -
