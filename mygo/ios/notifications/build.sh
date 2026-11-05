#!/bin/sh
# Builds the notification extension as Xcode would, with swiftc, into ../build/PlugIns/Notifications.appex, which
# mygo.json's resources put in the app (MyGo has no extension targets): build.sh <simulator|device> <rust target>
# <generated swift folder> <identity>. Our Swift, uniffi's Swift for client/ffi, and the library, linked statically.
set -eu
kind=$1 target=$2 gen=$3 identity=${4:--}
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../.." && pwd)
case $kind in simulator) sdk=iphonesimulator triple=arm64-apple-ios17.0-simulator platform=iPhoneSimulator ;; *) sdk=iphoneos triple=arm64-apple-ios17.0 platform=iPhoneOS ;; esac
out="$here/../build/PlugIns/Notifications.appex"
rm -rf "$here/../build" && mkdir -p "$out" "$here/../build/module"
cp "$gen/one_client_ffiFFI.h" "$here/../build/module/"
cp "$gen/one_client_ffiFFI.modulemap" "$here/../build/module/module.modulemap"
xcrun --sdk $sdk swiftc -target $triple -sdk "$(xcrun --sdk $sdk --show-sdk-path)" -module-name Notifications -application-extension -O \
	-I "$here/../build/module" "$here/NotificationService.swift" "$gen/one_client_ffi.swift" \
	"$root/target/$target/release/libone_client_ffi.a" -framework SystemConfiguration -framework Security -framework UserNotifications -framework Network \
	-Xlinker -e -Xlinker _NSExtensionMain -Xlinker -dead_strip \
	-Xlinker -sectcreate -Xlinker __TEXT -Xlinker __entitlements -Xlinker "$here/entitlements.plist" \
	-o "$out/Notifications"
sed "s/PLATFORM/$platform/" "$here/Info.plist" > "$out/Info.plist"
codesign --force --sign "$identity" --entitlements "$here/entitlements.plist" --generate-entitlement-der "$out"
