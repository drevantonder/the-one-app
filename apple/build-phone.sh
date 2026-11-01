#!/bin/bash
# Builds the iPhone app and installs it: on the booted simulator (or $SIMULATOR), or with `device`, signed with
# your team, on the connected iPhone (or $DEVICE). CONFIG=path/to/config.toml builds another context.
set -euo pipefail
cd "$(dirname "$0")"
. ./project.sh
./build-client.sh >/dev/null
id=app.$CONFIG_CLI.phone

if [ "${1:-}" = device ]; then
	# Signing needs the keychain, so over SSH the build runs in the logged-in session. Xcode 27's xcodebuild doesn't
	# see the Apple Account in Xcode's settings ("No Accounts"), so it can't register a new context's App IDs and app
	# group; it signs once Xcode has: build the Phone scheme for the iPhone in Xcode once (scripted, that's
	# `tell application "Xcode"` to open Shells.xcodeproj, set its active scheme and run destination, and `build`).
	build=(xcodebuild -project Shells.xcodeproj -scheme Phone -configuration Release -destination generic/platform=iOS
		-derivedDataPath build/derived -allowProvisioningUpdates build -quiet)
	if [ -n "${SSH_CONNECTION:-}" ]; then session "${build[@]}"; else "${build[@]}"; fi
	device=${DEVICE:-$(xcrun devicectl list devices | grep physical | grep iPhone | grep -v unavailable | grep -Eo '[0-9A-F]{8}-[0-9A-F]{16}' | head -1)}
	xcrun devicectl device install app --device "$device" "build/derived/Build/Products/Release-iphoneos/$CONFIG_NAME.app"
	xcrun devicectl device process launch --device "$device" "$id" || true
else
	xcodebuild -project Shells.xcodeproj -scheme Phone -configuration Debug -destination "generic/platform=iOS Simulator" \
		-derivedDataPath build/derived ARCHS=arm64 build -quiet
	xcrun simctl bootstatus "${SIMULATOR:-booted}" -b >/dev/null
	xcrun simctl install "${SIMULATOR:-booted}" "build/derived/Build/Products/Debug-iphonesimulator/$CONFIG_NAME.app"
	xcrun simctl launch "${SIMULATOR:-booted}" "$id"
fi
