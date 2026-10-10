#!/bin/sh
# Regenerate every platform icon from crates/coco-egui/assets/cocovm-icon.png:
# the macOS PNG, the hicolor PNG set (Linux launchers), and the Windows .ico.
# Run from anywhere after changing the source artwork; needs ImageMagick 7.
# Pass --macos-only to update just the macOS export.
set -eu

ROOT=$(cd "$(dirname "$0")/.." && pwd)
SOURCE="$ROOT/crates/coco-egui/assets/cocovm-icon.png"
HICOLOR="$ROOT/packaging/icons/hicolor"
ICO="$ROOT/crates/coco-egui/assets/cocovm.ico"
MACOS_PNG="$ROOT/crates/coco-egui/assets/cocovm-icon-macos.png"
MACOS_CANVAS_SIZE=1024
MACOS_TILE_SIZE=824
MACOS_CORNER_RADIUS=185
MASK_SUPERSAMPLING=4
PNG_SIZES="16 32 48 64 128 256 512"
# Windows stops at 256; Explorer picks the nearest from this set.
ICO_SIZES="256,128,64,48,32,16"

case "${1:-}" in
    ""|--macos-only) ;;
    *) echo "Usage: $0 [--macos-only]" >&2; exit 2 ;;
esac

# Resize the full illustration before masking its empty corners. Render the
# rounded-square mask at higher resolution for smooth edges, then center the
# tile on a transparent canvas (100 pixels of padding on each side).
mask_size=$((MACOS_TILE_SIZE * MASK_SUPERSAMPLING))
mask_edge=$((mask_size - 1))
mask_radius=$((MACOS_CORNER_RADIUS * MASK_SUPERSAMPLING))
magick "$SOURCE" -filter Lanczos -resize "${MACOS_TILE_SIZE}x${MACOS_TILE_SIZE}" \
    \( -size "${mask_size}x${mask_size}" xc:black -fill white \
        -draw "roundrectangle 0,0 $mask_edge,$mask_edge $mask_radius,$mask_radius" \
        -filter Lanczos -resize "${MACOS_TILE_SIZE}x${MACOS_TILE_SIZE}" \) \
    -alpha off -compose CopyOpacity -composite \
    -compose Over -background none -gravity center \
    -extent "${MACOS_CANVAS_SIZE}x${MACOS_CANVAS_SIZE}" -strip "PNG32:$MACOS_PNG"

[ "${1:-}" != --macos-only ] || exit 0

for size in $PNG_SIZES; do
    dir="$HICOLOR/${size}x${size}/apps"
    mkdir -p "$dir"
    magick "$SOURCE" -filter Lanczos -resize "${size}x${size}" -strip \
        "$dir/cocovm.png"
done
magick "$SOURCE" -define "icon:auto-resize=$ICO_SIZES" "$ICO"
