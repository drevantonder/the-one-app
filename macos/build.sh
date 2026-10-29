#!/bin/sh
# Builds TheOneApp.app: the menu bar app, which runs the node, plus the one
# binary and the Swift helpers the Apple plugins use, signed with a Developer
# ID so macOS keeps its permissions across rebuilds.
# Usage: macos/build.sh [install|zip|notarize]
#   zip       also zip the app into build/TheOneApp-macos.zip, as `one update` sends it
#   notarize  zip it notarized and stapled, for a release
#   SIGN_IDENTITY   signing identity (default: the first "Developer ID Application")
#   VERSION         the version the node reports (default: dev)
#   NOTARY_PROFILE  notarytool keychain profile (default: one)
set -eu
cd "$(dirname "$0")/.."
identity=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)}
[ -n "$identity" ] || { echo "no Developer ID Application identity; set SIGN_IDENTITY" >&2; exit 1; }

app=build/TheOneApp.app
ts=--timestamp=none
[ "${1:-}" = notarize ] && ts=--timestamp # notarization requires a secure timestamp
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp macos/Info.plist "$app/Contents/Info.plist"
go build -trimpath -ldflags "-X main.version=${VERSION:-dev}" -o "$app/Contents/MacOS/one" ./cmd/one
codesign --force --options runtime $ts --entitlements macos/TheOneApp.entitlements \
	--identifier dev.theoneapp.one --sign "$identity" "$app/Contents/MacOS/one"
swiftc -O -suppress-warnings macos/menubar.swift -o "$app/Contents/MacOS/TheOneApp"
for helper in eventkit contacts messages; do
	swiftc -O -suppress-warnings "plugins/apple/$helper.swift" -o "$app/Contents/MacOS/one-$helper"
	codesign --force --options runtime $ts --entitlements macos/TheOneApp.entitlements \
		--identifier "dev.theoneapp.$helper" --sign "$identity" "$app/Contents/MacOS/one-$helper"
done
codesign --force --options runtime $ts --entitlements macos/TheOneApp.entitlements --sign "$identity" "$app"
codesign --verify --strict "$app"
echo "built $app"

if [ "${1:-}" = install ]; then
	dest="$HOME/Applications/TheOneApp.app"
	mkdir -p "$HOME/Applications" "$HOME/.local/bin"
	rm -rf "$dest.new" && cp -R "$app" "$dest.new" && rm -rf "$dest" && mv "$dest.new" "$dest"
	ln -sf "$dest/Contents/MacOS/one" "$HOME/.local/bin/one"
	echo "installed $dest (one is linked into ~/.local/bin)"
fi

zip=build/TheOneApp-macos.zip
case "${1:-}" in
zip)
	rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
	echo "zipped $zip"
	;;
notarize)
	rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
	xcrun notarytool submit "$zip" --keychain-profile "${NOTARY_PROFILE:-one}" --wait
	xcrun stapler staple "$app"
	rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
	echo "notarized $zip"
	;;
esac
