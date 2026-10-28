#!/bin/sh
# Builds TheOneApp.app: the one binary plus the Swift helpers the Apple
# plugins use, signed with a Developer ID so macOS keeps its permissions
# across rebuilds. Usage: macos/build.sh [install]
#   SIGN_IDENTITY  signing identity (default: the first "Developer ID Application")
set -eu
cd "$(dirname "$0")/.."
identity=${SIGN_IDENTITY:-$(security find-identity -v -p codesigning | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)}
[ -n "$identity" ] || { echo "no Developer ID Application identity; set SIGN_IDENTITY" >&2; exit 1; }

app=build/TheOneApp.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp macos/Info.plist "$app/Contents/Info.plist"
go build -trimpath -o "$app/Contents/MacOS/one" ./cmd/one
for helper in eventkit contacts messages; do
	swiftc -O -suppress-warnings "plugins/apple/$helper.swift" -o "$app/Contents/MacOS/one-$helper"
	codesign --force --options runtime --timestamp=none --entitlements macos/TheOneApp.entitlements \
		--identifier "dev.theoneapp.$helper" --sign "$identity" "$app/Contents/MacOS/one-$helper"
done
codesign --force --options runtime --timestamp=none --entitlements macos/TheOneApp.entitlements --sign "$identity" "$app"
codesign --verify --strict "$app"
echo "built $app"

if [ "${1:-}" = install ]; then
	dest="$HOME/Applications/TheOneApp.app"
	mkdir -p "$HOME/Applications" "$HOME/.local/bin"
	rm -rf "$dest.new" && cp -R "$app" "$dest.new" && rm -rf "$dest" && mv "$dest.new" "$dest"
	ln -sf "$dest/Contents/MacOS/one" "$HOME/.local/bin/one"
	echo "installed $dest (one is linked into ~/.local/bin)"
fi
