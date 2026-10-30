#!/bin/bash
# Builds the Mac app with this checkout's node inside it, signs it ad hoc and installs it to ~/Applications.
# BRAND=path/to/brand.toml builds another brand.
set -euo pipefail
cd "$(dirname "$0")"
. ./project.sh
BUNDLE_ID=app.$BRAND_CLI.mac
label=$BUNDLE_ID.node

(cd "$root" && BRAND=$brand cargo build --release)

xcodebuild -project Shells.xcodeproj -scheme Mac -configuration Release -destination generic/platform=macOS -derivedDataPath build/derived build -quiet

app="build/derived/Build/Products/Release/$BRAND_NAME.app"
cp "$root/target/release/$BRAND_CLI" "$app/Contents/MacOS/"
mkdir -p "$app/Contents/Resources" && cp build/AppIcon.icns "$app/Contents/Resources/"
plutil -replace CFBundleIconFile -string AppIcon "$app/Contents/Info.plist"
mkdir -p "$app/Contents/Library/LaunchAgents"
cat > "$app/Contents/Library/LaunchAgents/$label.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>BundleProgram</key><string>Contents/MacOS/$BRAND_CLI</string>
  <key>ProgramArguments</key><array><string>$BRAND_CLI</string><string>serve</string></array>
  <key>AssociatedBundleIdentifiers</key><array><string>$BUNDLE_ID</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
</dict>
</plist>
PLIST
codesign --force --sign - "$app/Contents/MacOS/$BRAND_CLI"
codesign --force --sign - "$app"

osascript -e "quit app id \"$BUNDLE_ID\"" 2>/dev/null || true
sleep 1
dest="$HOME/Applications/$BRAND_NAME.app"
rm -rf "$dest"
ditto "$app" "$dest"
mkdir -p "$HOME/.local/bin"
ln -sf "$dest/Contents/MacOS/$BRAND_CLI" "$HOME/.local/bin/$BRAND_CLI"
# A running node keeps the old binary until it restarts.
launchctl kickstart -k "gui/$(id -u)/$label" 2>/dev/null || true
open "$dest"
echo "Installed $dest"
