#!/bin/sh
# Regenerate every platform icon from crates/coco-egui/assets/cocovm-icon.png:
# the hicolor PNG set (Linux launchers, macOS iconset) and the Windows .ico.
# Run from anywhere after changing the source artwork; needs ImageMagick 7.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SOURCE="$ROOT/crates/coco-egui/assets/cocovm-icon.png"
HICOLOR="$ROOT/packaging/icons/hicolor"
ICO="$ROOT/crates/coco-egui/assets/cocovm.ico"
PNG_SIZES="16 32 48 64 128 256 512"
# Windows stops at 256; Explorer picks the nearest from this set.
ICO_SIZES="256,128,64,48,32,16"

for size in $PNG_SIZES; do
    dir="$HICOLOR/${size}x${size}/apps"
    mkdir -p "$dir"
    magick "$SOURCE" -filter Lanczos -resize "${size}x${size}" -strip \
        "$dir/cocovm.png"
done
magick "$SOURCE" -define "icon:auto-resize=$ICO_SIZES" "$ICO"
