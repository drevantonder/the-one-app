#!/bin/sh
# Builds the widget extension as Xcode would, with swiftc, into ../build/PlugIns/Widget.appex (mygo.json's resources
# put build/PlugIns in the app): build.sh <simulator|device> <identity>.
set -eu
kind=$1 identity=${2:--}
here=$(cd "$(dirname "$0")" && pwd)
case $kind in simulator) sdk=iphonesimulator triple=arm64-apple-ios17.0-simulator platform=iPhoneSimulator ;; *) sdk=iphoneos triple=arm64-apple-ios17.0 platform=iPhoneOS ;; esac
out="$here/../build/PlugIns/Widget.appex"
rm -rf "$out" && mkdir -p "$out"
xcrun --sdk $sdk swiftc -target $triple -sdk "$(xcrun --sdk $sdk --show-sdk-path)" -module-name Widget -application-extension -parse-as-library -O \
	"$here/Widget.swift" -framework SwiftUI -framework WidgetKit \
	-Xlinker -sectcreate -Xlinker __TEXT -Xlinker __entitlements -Xlinker "$here/entitlements.plist" -o "$out/Widget"
sed "s/PLATFORM/$platform/" "$here/Info.plist" > "$out/Info.plist"
codesign --force --sign "$identity" --entitlements "$here/entitlements.plist" --generate-entitlement-der "$out"
