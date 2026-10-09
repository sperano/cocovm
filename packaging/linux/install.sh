#!/bin/sh
# Install cocovm, its desktop entry and icons from an unpacked release
# archive. Usage: ./install.sh [PREFIX]; PREFIX defaults to ~/.local (the
# current user only). Use /usr/local with sudo for every user.
set -eu

# Absolute, so a relative prefix still yields a launchable Exec path.
mkdir -p "${1:-"$HOME/.local"}"
PREFIX=$(cd "${1:-"$HOME/.local"}" && pwd)
HERE=$(cd "$(dirname "$0")" && pwd)
BIN_DIR="$PREFIX/bin"
APPLICATIONS_DIR="$PREFIX/share/applications"
ICON_THEME_DIR="$PREFIX/share/icons/hicolor"

install -D -m 755 "$HERE/cocovm" "$BIN_DIR/cocovm"

# Launchers do not search PATH the way a shell does, so point Exec at the
# installed binary.
mkdir -p "$APPLICATIONS_DIR"
sed "s|^Exec=cocovm\$|Exec=\"$BIN_DIR/cocovm\"|" "$HERE/cocovm.desktop" \
    > "$APPLICATIONS_DIR/cocovm.desktop"

for png in "$HERE"/icons/hicolor/*/apps/cocovm.png; do
    size=$(basename "$(dirname "$(dirname "$png")")")
    install -D -m 644 "$png" "$ICON_THEME_DIR/$size/apps/cocovm.png"
done

# Refresh the desktop caches when the tools exist; a stale cache only delays
# the new entry, so failures here are not fatal.
if command -v update-desktop-database > /dev/null 2>&1; then
    update-desktop-database "$APPLICATIONS_DIR" || true
fi
if command -v gtk-update-icon-cache > /dev/null 2>&1; then
    gtk-update-icon-cache -q -t "$ICON_THEME_DIR" || true
fi

echo "Installed cocovm to $BIN_DIR and its launcher to $APPLICATIONS_DIR."
