#!/bin/bash
# Builds the Mac app with this checkout's node inside it, signs it and installs it to ~/Applications; with --release, it
# also notarizes it. BRAND=path/to/brand.toml builds another brand.
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
# Sign with this Mac's Developer ID, with the hardened runtime, so the app opens on any Mac once notarized; and always
# the same way, since macOS keeps the permissions it grants (Reminders, Full Disk Access…) only for code signed alike.
# Without a Developer ID it falls back to Apple Development, then ad hoc. Over SSH the keychain is out of reach, so
# codesign and notarytool run as one-shot jobs in the logged-in session instead.
keychain() { "$@" 2>/dev/null || session "$@"; }
identities=$(security find-identity -v -p codesigning)
identity=$(echo "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
identity=${identity:-$(echo "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}
sign=(codesign --force --options runtime --sign "${identity:--}")
[ -z "$identity" ] || sign+=(--timestamp)
keychain "${sign[@]}" "$PWD/$app/Contents/MacOS/$BRAND_CLI" &&
	keychain "${sign[@]}" --entitlements "$PWD/Mac.entitlements" "$PWD/$app" || { echo "couldn't sign the app as ${identity:-ad hoc}"; exit 1; }

# A release is notarized, with the ticket stapled to the app, so Gatekeeper lets it open wherever it's downloaded.
# notarytool's credentials are a keychain profile named after the CLI: xcrun notarytool store-credentials <cli>-notary
if [ "${1:-}" = --release ]; then
	case $identity in "Developer ID Application: "*) ;; *) echo "a release needs a Developer ID Application certificate"; exit 1 ;; esac
	profile=$BRAND_CLI-notary zip=$PWD/build/notarize.zip
	if keychain xcrun notarytool history --keychain-profile "$profile" >/dev/null; then
		rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
		keychain xcrun notarytool submit "$zip" --keychain-profile "$profile" --wait
		keychain xcrun stapler staple "$PWD/$app"
	else
		echo "warning: not notarized, so it opens only where it isn't downloaded with a browser: this Mac's keychain has no notarytool profile $profile (xcrun notarytool store-credentials $profile)"
	fi
fi

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
