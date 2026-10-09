#!/bin/sh
# Assemble the Linux release directory: the binary plus the desktop entry,
# icons and installer that give it a launcher icon.
# Usage: stage.sh BINARY STAGE_DIR
set -eu

BINARY=$1
STAGE=$2
ROOT=$(cd "$(dirname "$0")/../.." && pwd)

mkdir -p "$STAGE"
cp "$BINARY" "$STAGE/cocovm"
cp "$ROOT/packaging/linux/cocovm.desktop" "$ROOT/packaging/linux/install.sh" \
    "$STAGE/"
cp -R "$ROOT/packaging/icons" "$STAGE/icons"
