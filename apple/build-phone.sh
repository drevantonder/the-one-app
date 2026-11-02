#!/bin/bash
# Builds the iPhone app and installs it: on the booted simulator (or $SIMULATOR), or with `device`, signed with
# your team, on the connected iPhone (or $DEVICE). CONFIG=path/to/config.toml builds another white label. SCREENS=1
# builds Screens instead, beside it: the iPhone app with every client's screens as its home.
set -euo pipefail
cd "$(dirname "$0")"
. ./project.sh
scheme=Phone id=app.$CONFIG_CLI.phone name=$CONFIG_NAME
if [ "${SCREENS:-}" = 1 ]; then scheme=Screens id=$id.screens name=Screens; fi

if [ "${1:-}" = device ]; then
	# Signing needs the keychain, so over SSH the build runs in the logged-in session. Xcode 27's xcodebuild doesn't
	# see the Apple Account in Xcode's settings ("No Accounts"), so it can't register a new white label's App IDs and app
	# group; it signs once Xcode has: build the Phone (or Screens) scheme for the iPhone in Xcode once (scripted, that's
	# `tell application "Xcode"` to open Shells.xcodeproj, set its active scheme and run destination, and `build`).
	build=(xcodebuild -project Shells.xcodeproj -scheme $scheme -configuration Release -destination generic/platform=iOS
		-derivedDataPath build/derived -allowProvisioningUpdates build -quiet)
	if [ -n "${SSH_CONNECTION:-}" ]; then session "${build[@]}"; else "${build[@]}"; fi
	device=${DEVICE:-$(xcrun devicectl list devices | grep physical | grep iPhone | grep -v unavailable | grep -Eo '[0-9A-F]{8}-[0-9A-F]{16}' | head -1)}
	xcrun devicectl device install app --device "$device" "build/derived/Build/Products/Release-iphoneos/$name.app"
	xcrun devicectl device process launch --device "$device" "$id" || true
else
	xcodebuild -project Shells.xcodeproj -scheme $scheme -configuration Debug -destination "generic/platform=iOS Simulator" \
		-derivedDataPath build/derived ARCHS=arm64 build -quiet
	xcrun simctl bootstatus "${SIMULATOR:-booted}" -b >/dev/null
	xcrun simctl install "${SIMULATOR:-booted}" "build/derived/Build/Products/Debug-iphonesimulator/$name.app"
	xcrun simctl launch "${SIMULATOR:-booted}" "$id"
fi
