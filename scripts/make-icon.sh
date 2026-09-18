#!/bin/sh
# Renders the app icon from packaging/icon/dermixen.svg into the files the
# bundles and the window use: PNGs at 256, 512, and 1024 pixels beside the
# SVG, Dermixen.icns for the macOS bundle, and the copy of the SVG the
# documentation site shows as its logo. Run it from the repository
# root after changing the SVG, and commit what it writes. It needs
# rsvg-convert (the librsvg package in Homebrew) and, for the .icns, the
# iconutil command that ships with macOS.
set -eu

cd "$(dirname "$0")/.."

ICON_DIR="packaging/icon"
SVG="$ICON_DIR/dermixen.svg"

cp "$SVG" docs/images/dermixen-icon.svg
echo docs/images/dermixen-icon.svg

for size in 256 512 1024; do
	rsvg-convert -w "$size" -h "$size" "$SVG" -o "$ICON_DIR/dermixen-$size.png"
	echo "$ICON_DIR/dermixen-$size.png"
done

if command -v iconutil >/dev/null 2>&1; then
	ICONSET=$(mktemp -d)/Dermixen.iconset
	mkdir -p "$ICONSET"
	for size in 16 32 128 256 512; do
		double=$((size * 2))
		rsvg-convert -w "$size" -h "$size" "$SVG" -o "$ICONSET/icon_${size}x${size}.png"
		rsvg-convert -w "$double" -h "$double" "$SVG" -o "$ICONSET/icon_${size}x${size}@2x.png"
	done
	iconutil -c icns "$ICONSET" -o "$ICON_DIR/Dermixen.icns"
	rm -r "$(dirname "$ICONSET")"
	echo "$ICON_DIR/Dermixen.icns"
else
	echo "iconutil is not on the path, so Dermixen.icns was not written. Run this on macOS to write it."
fi
