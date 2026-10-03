#!/usr/bin/env bash
# Build, sign, notarize, and package cocovm for macOS — the local twin of
# the macOS steps in .github/workflows/release.yml (keep the two in sync;
# the workflow embeds its own copy of the Info.plist so it can service
# tags that predate any given repo file).
#
# One-time setup (stores the app-specific password in your login keychain):
#
#   xcrun notarytool store-credentials cocovm-notary \
#     --apple-id you@example.com --team-id TEAMID
#
# Requires a "Developer ID Application" certificate in the login keychain.
#
# Usage:
#   tools/release-macos.sh [--target <triple>] [--profile <name>] [--skip-build]
#
#   --target      rust target triple (default: host)
#   --profile     notarytool keychain profile (default: $COCOVM_NOTARY_PROFILE
#                 or "cocovm-notary")
#   --skip-build  reuse an existing target/<triple>/release/cocovm
#
# Artifacts land in dist/: a stapled, signed DMG and a tarball whose binary
# carries the same notarization ticket.
set -euo pipefail

TARGET=""
PROFILE="${COCOVM_NOTARY_PROFILE:-cocovm-notary}"
SKIP_BUILD=0
while [ $# -gt 0 ]; do
  case "$1" in
    --target) TARGET=$2; shift 2 ;;
    --profile) PROFILE=$2; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

cd "$(dirname "$0")/.."

[ "$(uname)" = "Darwin" ] || { echo "macOS only" >&2; exit 1; }
[ -n "$TARGET" ] || TARGET=$(rustc -vV | awk '/^host:/ {print $2}')
VERSION=$(awk -F'"' '/^version = /{print $2; exit}' Cargo.toml)
BIN=target/$TARGET/release/cocovm
STAGE=cocovm-v$VERSION-$TARGET
WORK=$(mktemp -d)
trap 'rm -rf "$WORK"' EXIT

if [ "$SKIP_BUILD" -eq 0 ]; then
  cargo build --release -p coco-egui --target "$TARGET"
fi
[ -f "$BIN" ] || { echo "missing $BIN (build failed or wrong --target?)" >&2; exit 1; }

IDENTITY=$(security find-identity -v -p codesigning \
  | awk -F'"' '/Developer ID Application/ {print $2; exit}')
[ -n "$IDENTITY" ] || {
  echo "no Developer ID Application identity in the keychain" >&2; exit 1
}
echo "signing as: $IDENTITY"
echo "version:    $VERSION"
echo "target:     $TARGET"

# --- cocovm.app bundle -----------------------------------------------------
APP="$WORK/cocovm.app"
mkdir -p "$APP/Contents/MacOS"
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
cp "$BIN" "$APP/Contents/MacOS/cocovm"

# Finder icon: .icns generated from the same 1024px art the running app
# embeds for its Dock icon, so the two remain consistent.
ICON_SRC=crates/coco-egui/assets/coco3-console-8bit.png
ICONSET="$WORK/cocovm.iconset"
mkdir "$ICONSET"
for s in 16 32 128 256 512; do
  sips -z "$s" "$s" "$ICON_SRC" --out "$ICONSET/icon_${s}x${s}.png" > /dev/null
  sips -z "$((s * 2))" "$((s * 2))" "$ICON_SRC" \
    --out "$ICONSET/icon_${s}x${s}@2x.png" > /dev/null
done
mkdir -p "$APP/Contents/Resources"
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/cocovm.icns"

# Hardened runtime + secure timestamp are notarization requirements.
codesign --force --options runtime --timestamp --sign "$IDENTITY" "$APP"

# --- notarize + staple -----------------------------------------------------
ditto -c -k --keepParent "$APP" "$WORK/notarize.zip"
xcrun notarytool submit "$WORK/notarize.zip" \
  --keychain-profile "$PROFILE" \
  --wait --timeout 2h --output-format json \
  | tee "$WORK/notary.json"
echo
if [ "$(jq -r .status "$WORK/notary.json")" != "Accepted" ]; then
  # Older notarytool exits 0 on an Invalid verdict. Show the rejection log.
  xcrun notarytool log "$(jq -r .id "$WORK/notary.json")" \
    --keychain-profile "$PROFILE" || true
  exit 1
fi
xcrun stapler staple "$APP"

# --- package ---------------------------------------------------------------
mkdir -p dist
DMGROOT="$WORK/dmgroot"
mkdir "$DMGROOT"
cp -R "$APP" "$DMGROOT/"
cp LICENSE NOTICE.md "$DMGROOT/"
ln -s /Applications "$DMGROOT/Applications"
hdiutil create -volname "CoCoVM" -srcfolder "$DMGROOT" \
  -ov -format UDZO "dist/$STAGE.dmg"
codesign --sign "$IDENTITY" --timestamp "dist/$STAGE.dmg"

mkdir "$WORK/$STAGE"
cp "$APP/Contents/MacOS/cocovm" LICENSE NOTICE.md "$WORK/$STAGE/"
tar -czf "dist/$STAGE.tar.gz" -C "$WORK" "$STAGE"

# --- verify what we're about to ship ---------------------------------------
codesign --verify --strict --deep "dist/$STAGE.dmg"
spctl --assess --type execute -vv "$APP"
xcrun stapler validate "$APP"

echo
echo "dist/$STAGE.dmg"
echo "dist/$STAGE.tar.gz"
