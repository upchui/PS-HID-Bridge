#!/bin/sh
# The checksum for this machine's processor.
# Usage: arch-sha.sh <x86_64 sha256> <aarch64 sha256>
set -eu

case "$(uname -m)" in
    x86_64) echo "$1" ;;
    aarch64) echo "$2" ;;
    *) echo "arch-sha.sh: unsupported processor $(uname -m)" >&2; exit 1 ;;
esac
