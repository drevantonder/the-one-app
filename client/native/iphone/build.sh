#!/bin/bash
# Builds the iPhone app (client/native: Tauri's window, in the Xcode project iphone/project.yml makes) with its
# home-screen widget, and in a build with push its notification extension, and installs it:
#   build.sh               a debug build, on the booted simulator (or $SIMULATOR)
#   build.sh device [dir]  signed with the team, on the paired iPhone (or $DEVICE); with dir, kept there for an update
#                          to install
#   build.sh testflight    archived and uploaded to TestFlight, once Apple has everything it needs for that
#   build.sh preflight     only asks Apple that, in seconds, and installs the App Store profiles
# Those two read the config's testflight_key (a command printing App Store Connect's .p8 key, kept only for the run),
# testflight_key_id and testflight_issuer. The App Store profiles are named after the app ("<name> App Store", "<name>
# Widgets App Store", "<name> Notifications App Store"), on an Apple Distribution certificate in a keychain this user
# searches: $BUILD_KEYCHAIN and $BUILD_KEYCHAIN_PASSWORD_FILE, if that keychain isn't the login one.
# CONFIG=path/to/config.toml builds another custom build; push is on with its `push = true`, or PUSH=1.
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
[ "${1:-}" = preflight ] || alone "$@"
cd "$native"
mode=${1:-simulator}
out=$native/target/iphone
mkdir -p "$out"
if [ "$mode" = simulator ]; then rust=aarch64-apple-ios-sim profile=debug configuration=Debug; else rust=aarch64-apple-ios profile=release configuration=Release; fi
# A custom build's config repo's commits after main's, so a change to either alone makes a higher build, which
# TestFlight wants of each upload.
config_count=$(git -C "$(dirname "$config")" rev-list --count HEAD 2>/dev/null || echo 0)
export BUILD_NUMBER=$BUILD_NUMBER.$config_count
# Push needs the Push Notifications capability on the App IDs and an APNs key for the push Worker, which only the team's
# account holder makes: so builds leave it out until the config says `push = true`, or PUSH=1.
if [ "${PUSH:-}" = 1 ] || [ "$(field push)" = true ]; then export PUSH=YES; else export PUSH=; fi

if [ "$mode" = testflight ] || [ "$mode" = preflight ]; then
	export ASC_KEY_ID=$(field testflight_key_id) ASC_ISSUER=$(field testflight_issuer)
	[ -n "$(field testflight_key)" ] && [ -n "$ASC_KEY_ID" ] && [ -n "$ASC_ISSUER" ] || { echo "$config needs testflight_key (a command printing App Store Connect's .p8 key), testflight_key_id and testflight_issuer"; exit 1; }
	private=$(mktemp -d)
	trap 'rm -rf "$private"' EXIT
	export ASC_KEY=$private/key.p8
	eval "$(field testflight_key)" > "$ASC_KEY"
	export CONFIG_NAME CONFIG_IPHONE APPLE_TEAM PUSH
	swift "$native/iphone/preflight.swift"
	[ "$mode" = testflight ] || exit 0
fi

# The app's Rust, with its Swift (iphone/Sources) and Tauri's in it, which the project links.
release= && [ $profile = debug ] || release=--release
CONFIG=$config ONE_BUILD=$BUILD_COMMIT cargo build -q --lib --target $rust $release
mkdir -p "$out/lib" && cp "target/$rust/$profile/libone_native.a" "$out/lib/"

# The icon is its tile, full bleed: the icons are drawn on Apple's Mac grid, an 824-pixel tile in 1024. iPhone icons
# can't be transparent, so what's left of the tile's corners takes the colour of its edge.
contents() { printf '{"images":[%s],"info":{"author":"xcode","version":1}%s}\n' "$1" "${2:-}"; }
icons=$out/Phone.xcassets/AppIcon.appiconset
rm -rf "$out/Phone.xcassets" && mkdir -p "$icons"
sips -c 824 824 "$(art icon)" --out "$out/tile.png" >/dev/null
swift - "$out/tile.png" "$icons/icon.png" <<'SWIFT'
import AppKit
let a = CommandLine.arguments, tile = NSBitmapImageRep(data: try! Data(contentsOf: URL(filePath: a[1])))!
let cg = CGContext(data: nil, width: 1024, height: 1024, bitsPerComponent: 8, bytesPerRow: 0,
                   space: CGColorSpace(name: CGColorSpace.sRGB)!, bitmapInfo: CGImageAlphaInfo.noneSkipLast.rawValue)!
cg.setFillColor((tile.colorAt(x: tile.pixelsWide / 2, y: 4) ?? .black).cgColor)
cg.fill(CGRect(x: 0, y: 0, width: 1024, height: 1024))
cg.draw(tile.cgImage!, in: CGRect(x: 0, y: 0, width: 1024, height: 1024))
try! NSBitmapImageRep(cgImage: cg.makeImage()!).representation(using: .png, properties: [:])!.write(to: URL(filePath: a[2]))
SWIFT
contents '{"filename":"icon.png","idiom":"universal","platform":"ios","size":"1024x1024"}' > "$icons/Contents.json"
contents "" > "$out/Phone.xcassets/Contents.json"
# The logo the widget shows.
rm -rf "$out/Artwork.xcassets" && mkdir -p "$out/Artwork.xcassets/Logo.imageset"
contents "" > "$out/Artwork.xcassets/Contents.json"
cp "$(art logo)" "$out/Artwork.xcassets/Logo.imageset/logo.png"
contents '{"filename":"logo.png","idiom":"universal"}' > "$out/Artwork.xcassets/Logo.imageset/Contents.json"

# With push, the client library (client/ffi) and its Swift, for the notification extension to open notes with.
if [ -n "$PUSH" ]; then
	client=$out/Client
	"$root/client/ffi/bindings.sh" swift "$client/generated"
	mkdir -p "$client/headers"
	cp "$client/generated/one_client_ffi.swift" "$client/"
	cp "$client/generated/one_client_ffiFFI.h" "$client/headers/"
	cp "$client/generated/one_client_ffiFFI.modulemap" "$client/headers/module.modulemap"
	(cd "$root" && CONFIG=$config cargo build -q -p one-client-ffi --release --target $rust)
	rm -rf "$client/Client.xcframework"
	xcodebuild -create-xcframework -library "$root/target/$rust/release/libone_client_ffi.a" -headers "$client/headers" -output "$client/Client.xcframework" >/dev/null
fi
xcodegen generate --quiet --spec iphone/project.yml

name=$CONFIG_NAME id=$CONFIG_IPHONE
# Over SSH the keychain, which signing needs, is the logged-in session's.
signed() { if [ -n "${SSH_CONNECTION:-}" ]; then session "$@"; else "$@"; fi; }
# Xcode registers the App IDs, App Group and profiles it signs with; with App Store Connect's key, without an Apple
# Account in Xcode (which xcodebuild doesn't see over SSH).
auth=(-allowProvisioningUpdates)
[ -z "${ASC_KEY:-}" ] || auth+=(-authenticationKeyPath "$ASC_KEY" -authenticationKeyID "$ASC_KEY_ID" -authenticationKeyIssuerID "$ASC_ISSUER")
build=(xcodebuild -project iphone/Phone.xcodeproj -scheme Phone -configuration $configuration -derivedDataPath "$out/derived" -quiet)

case $mode in
simulator)
	"${build[@]}" -destination "generic/platform=iOS Simulator" build
	xcrun simctl bootstatus "${SIMULATOR:-booted}" -b >/dev/null
	xcrun simctl install "${SIMULATOR:-booted}" "$out/derived/Build/Products/Debug-iphonesimulator/$name.app"
	# Started anew (what it prints is in the simulator's log: xcrun simctl spawn <it> log stream).
	xcrun simctl launch --terminate-running-process "${SIMULATOR:-booted}" "$id"
	;;
device)
	signed "${build[@]}" -destination generic/platform=iOS "${auth[@]}" build
	app=$out/derived/Build/Products/Release-iphoneos/$name.app
	if [ -n "${2:-}" ]; then rm -rf "$2" && mkdir -p "$2" && ditto "$app" "$2/$name.app"; exit; fi
	# The paired iPhone, as an update's job finds it (src/build.rs), over USB or the network.
	devices=$(mktemp) && xcrun devicectl list devices --json-output "$devices" >/dev/null
	device=${DEVICE:-$(jq -r '[.result.devices[] | select(.hardwareProperties.reality == "physical" and .hardwareProperties.deviceType == "iPhone" and .connectionProperties.pairingState == "paired")][0].identifier // empty' "$devices")}
	xcrun devicectl device install app --device "$device" "$app"
	xcrun devicectl device process launch --device "$device" "$id" || true
	;;
testflight)
	rm -rf "$out/Phone.xcarchive" "$out/export"
	signed "${build[@]}" -destination generic/platform=iOS "${auth[@]}" -archivePath "$out/Phone.xcarchive" archive
	cat > "$out/export.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>method</key><string>app-store-connect</string>
  <key>destination</key><string>upload</string>
  <key>teamID</key><string>$APPLE_TEAM</string>
  <key>signingStyle</key><string>manual</string>
  <key>signingCertificate</key><string>Apple Distribution</string>
  <key>provisioningProfiles</key><dict>
    <key>$id</key><string>$name App Store</string>
    <key>$id.widgets</key><string>$name Widgets App Store</string>
$([ -z "$PUSH" ] || printf '    <key>%s.notifications</key><string>%s Notifications App Store</string>\n' "$id" "$name")  </dict>
  <key>manageAppVersionAndBuildNumber</key><false/>
</dict></plist>
PLIST
	# In this shell, not the logged-in session's: there codesign can't reach a keychain other than the login one
	# (errSecInternalComponent), and unlocking one is refused.
	[ -z "${BUILD_KEYCHAIN:-}" ] || security unlock-keychain -p "$(cat "$BUILD_KEYCHAIN_PASSWORD_FILE")" "$BUILD_KEYCHAIN"
	xcodebuild -exportArchive -archivePath "$out/Phone.xcarchive" -exportOptionsPlist "$out/export.plist" -exportPath "$out/export" "${auth[@]}"
	echo "Uploaded $name $BUILD_NUMBER ($BUILD_COMMIT) to TestFlight"
	;;
*) echo "build.sh [simulator | device [dir] | testflight | preflight]"; exit 1 ;;
esac
