#!/bin/sh
# Installs the Dermixen desktop entry and the application/x-dermixen-mix MIME
# type into the current user's XDG data folders, so a .dmx file opens in the
# window, and the app icon into the user's icon theme. This script is for
# Linux.
set -eu

if [ "$(uname -s)" = "Darwin" ]; then
	echo "install-linux-desktop.sh is for Linux. On macOS, run scripts/bundle-macos.sh instead."
	exit 0
fi

cd "$(dirname "$0")/.."

APPS_DIR="$HOME/.local/share/applications"
MIME_DIR="$HOME/.local/share/mime/packages"
ICONS_DIR="$HOME/.local/share/icons/hicolor"

mkdir -p "$APPS_DIR"
mkdir -p "$MIME_DIR"
mkdir -p "$ICONS_DIR/scalable/apps"
mkdir -p "$ICONS_DIR/512x512/apps"

cp packaging/linux/dermixen.desktop "$APPS_DIR/dermixen.desktop"
cp packaging/linux/dermixen-mix.xml "$MIME_DIR/dermixen-mix.xml"
cp packaging/icon/dermixen.svg "$ICONS_DIR/scalable/apps/dermixen.svg"
cp packaging/icon/dermixen-512.png "$ICONS_DIR/512x512/apps/dermixen.png"

if command -v update-mime-database >/dev/null 2>&1; then
	update-mime-database "$HOME/.local/share/mime"
fi

if command -v update-desktop-database >/dev/null 2>&1; then
	update-desktop-database "$APPS_DIR"
fi

echo "dermixen-app has to be on the path for the entry to work."
