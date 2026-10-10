#!/bin/sh
# Wrap a release binary in a cocovm.app bundle with its Finder icon.
# Usage: bundle.sh BINARY VERSION APP_DIR   (VERSION like 0.7.8)
# Signing and notarization stay in the release workflow.
set -eu

BINARY=$1
VERSION=$2
APP=$3
ROOT=$(cd "$(dirname "$0")/../.." && pwd)
SOURCE_ICON="$ROOT/crates/coco-egui/assets/cocovm-icon-macos.png"
ICONSET_SIZES="16 32 128 256 512"

mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>cocovm</string>
  <key>CFBundleDisplayName</key><string>CoCoVM</string>
  <key>CFBundleIdentifier</key><string>com.sperano.cocovm</string>
  <key>CFBundleExecutable</key><string>cocovm</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>LSApplicationCategoryType</key><string>public.app-category.games</string>
  <key>NSHumanReadableCopyright</key><string>GPL-3.0-or-later</string>
  <key>CFBundleIconFile</key><string>cocovm</string>
</dict>
</plist>
PLIST
plutil -lint "$APP/Contents/Info.plist"
cp "$BINARY" "$APP/Contents/MacOS/cocovm"

WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT
trap 'exit 1' HUP INT TERM
ICONSET="$WORK/cocovm.iconset"
mkdir -p "$ICONSET"
for size in $ICONSET_SIZES; do
    sips -z "$size" "$size" "$SOURCE_ICON" \
        --out "$ICONSET/icon_${size}x${size}.png" > /dev/null
    double=$((size * 2))
    sips -z "$double" "$double" "$SOURCE_ICON" \
        --out "$ICONSET/icon_${size}x${size}@2x.png" > /dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/cocovm.icns"
