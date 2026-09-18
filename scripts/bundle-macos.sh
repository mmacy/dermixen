#!/bin/sh
# Builds dermixen-app in release mode and assembles target/Dermixen.app, a
# macOS application bundle that declares the Dermixen mix document type
# and carries the app icon from packaging/icon/Dermixen.icns.
# Run this from the repository root.
set -eu

cd "$(dirname "$0")/.."

VERSION=$(grep -m1 '^version' Cargo.toml | sed -E 's/^version *= *"([^"]+)".*/\1/')

cargo build --release -p dermixen-app

APP="target/Dermixen.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"

cp "target/release/dermixen-app" "$APP/Contents/MacOS/dermixen-app"
cp "packaging/icon/Dermixen.icns" "$APP/Contents/Resources/Dermixen.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>CFBundleName</key>
	<string>Dermixen</string>
	<key>CFBundleDisplayName</key>
	<string>Dermixen</string>
	<key>CFBundleIdentifier</key>
	<string>com.github.mmacy.dermixen</string>
	<key>CFBundleExecutable</key>
	<string>dermixen-app</string>
	<key>CFBundleIconFile</key>
	<string>Dermixen</string>
	<key>CFBundlePackageType</key>
	<string>APPL</string>
	<key>CFBundleShortVersionString</key>
	<string>${VERSION}</string>
	<key>CFBundleVersion</key>
	<string>${VERSION}</string>
	<key>CFBundleInfoDictionaryVersion</key>
	<string>6.0</string>
	<key>LSMinimumSystemVersion</key>
	<string>11.0</string>
	<key>NSHighResolutionCapable</key>
	<true/>
	<key>CFBundleDocumentTypes</key>
	<array>
		<dict>
			<key>CFBundleTypeName</key>
			<string>Dermixen mix</string>
			<key>LSItemContentTypes</key>
			<array>
				<string>com.github.mmacy.dermixen.mix</string>
			</array>
			<key>CFBundleTypeRole</key>
			<string>Editor</string>
			<key>LSHandlerRank</key>
			<string>Owner</string>
		</dict>
	</array>
	<key>UTExportedTypeDeclarations</key>
	<array>
		<dict>
			<key>UTTypeIdentifier</key>
			<string>com.github.mmacy.dermixen.mix</string>
			<key>UTTypeConformsTo</key>
			<array>
				<string>public.json</string>
			</array>
			<key>UTTypeDescription</key>
			<string>Dermixen mix</string>
			<key>UTTypeTagSpecification</key>
			<dict>
				<key>public.filename-extension</key>
				<array>
					<string>dmx</string>
				</array>
			</dict>
		</dict>
	</array>
</dict>
</plist>
PLIST

plutil -lint "$APP/Contents/Info.plist"

echo "$APP"
