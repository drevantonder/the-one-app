#!/bin/bash
# Builds what a Mac takes from the record, signed, in native-apps/target/mac/built (`<cli> publish-mac` publishes it):
# the window, zipped (app.zip), which carries no background program; the program (<cli>, which runs as oned and onet);
# its app runtime (<cli>-apps, published as the service `apps`);
# each service in Rust (<cli>-<name>); and each in Swift, an app of its own holding its own permission, zipped
# (<cli>-<name>.zip, services/sdk/mac-helpers.sh). With --release, the window is notarized too. CONFIG=path/to/config.toml
# builds another custom build. Nothing is installed: a Mac takes what's published with `<cli> update`.
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
alone "$@"
BUNDLE_ID=app.$CONFIG_CLI.mac
cd "$native"

# Built unstripped, so each program's symbols can be kept before it's stripped below: a stall's stacks (logs/stall.sample)
# are read against them by one-dev symbols, as the releases themselves carry none. The strip is here, not in Cargo's profile.
(cd "$root" && CONFIG=$config ONE_BUILD=$BUILD_COMMIT CARGO_PROFILE_RELEASE_STRIP=none cargo build --release)
CONFIG=$config ONE_BUILD=$BUILD_COMMIT cargo build --release

out=$native/target/mac/built
app=$out/$CONFIG_NAME.app
rm -rf "$out"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
# The window, by the app's name, as the Mac app's was (what quits and finds it, `<cli> update`, names it so).
cp target/release/one-native "$app/Contents/MacOS/$CONFIG_NAME"
# Cargo names the program after the package, whatever the custom build.
cp "$root/target/release/one" "$out/$CONFIG_CLI"
# Its app runtime (app-runtime/), which oned starts beside it, published as the service `apps`.
cp "$root/target/release/one-apps" "$out/$CONFIG_CLI-apps"
# Its apps' services in Rust (apps/<name>/service/), which Cargo built with it.
for m in "$root"/apps/*/service/Cargo.toml; do
	[ -e "$m" ] || continue
	s=$(basename "$(dirname "$(dirname "$m")")")
	cp "$root/target/release/one-$s" "$out/$CONFIG_CLI-$s"
done

symbols=$HOME/Library/Caches/$CONFIG_CLI/symbols
mkdir -p "$symbols"
for program in "$out/$CONFIG_CLI" "$out/$CONFIG_CLI"-*; do
	[ -e "$program" ] || continue
	uuid=$(dwarfdump --uuid "$program" | awk '{print $2; exit}')
	nm -n "$program" | awk '$2 ~ /^[Tt]$/ {print $1, $3}' | gzip -9 >"$symbols/$uuid.gz"
	strip -x "$program"
done
# A symbols file is a few MB, and a build makes one per program: the newest 200 are the last 25 or so.
ls -1t "$symbols" | tail -n +201 | while read -r old; do rm -f "$symbols/$old"; done
# What the record names this build: the program's word for it, with a custom build's config commit.
build=$("$out/$CONFIG_CLI" version --json | sed -n 's/.*"commit":"\([^"]*\)".*/\1/p')
[ -n "$build" ] || { echo "the program built doesn't say what it is"; exit 1; }

# The icon is the config's as it is.
icon=$(art icon)
rm -rf target/mac/AppIcon.iconset && mkdir -p target/mac/AppIcon.iconset
for s in 16 32 128 256 512; do
	sips -z $s $s "$icon" --out target/mac/AppIcon.iconset/icon_${s}x${s}.png >/dev/null
	sips -z $((s * 2)) $((s * 2)) "$icon" --out target/mac/AppIcon.iconset/icon_${s}x${s}@2x.png >/dev/null
done
iconutil -c icns target/mac/AppIcon.iconset -o "$app/Contents/Resources/AppIcon.icns"

# What macOS and `<cli> update` read of it: Build is what the record names this build.
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
  <key>CFBundleExecutable</key><string>$CONFIG_NAME</string>
  <key>CFBundleName</key><string>$CONFIG_NAME</string>
  <key>CFBundleDisplayName</key><string>$CONFIG_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>CFBundleShortVersionString</key><string>0.1.0</string>
  <key>CFBundleVersion</key><string>$BUILD_NUMBER</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>LSMinimumSystemVersion</key><string>26.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.productivity</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string></string>
  <key>CLI</key><string>$CONFIG_CLI</string>
  <key>Yours</key><string>$CONFIG_YOURS</string>
  <key>NodePort</key><string>$CONFIG_PORT</string>
  <key>BuildCommit</key><string>$BUILD_COMMIT</string>
  <key>Build</key><string>$build</string>
  <key>CFBundleURLTypes</key>
  <array><dict>
    <key>CFBundleURLName</key><string>$BUNDLE_ID</string>
    <key>CFBundleURLSchemes</key><array><string>$CONFIG_CLI</string></array>
  </dict></array>
</dict>
</plist>
PLIST

# Sign with this Mac's Developer ID, with the hardened runtime, so the app opens on any Mac once notarized; and always
# the same way, since macOS keeps what it grants (the Keychain's items, permissions) only for code signed alike. Without
# a Developer ID it falls back to Apple Development, then ad hoc.
identities=$(security find-identity -v -p codesigning)
identity=$(echo "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
identity=${identity:-$(echo "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}
sign=(codesign --force --options runtime --sign "${identity:--}")
[ -z "$identity" ] || sign+=(--timestamp)
for s in "$out/$CONFIG_CLI"-* "$out/$CONFIG_CLI"; do
	[ -e "$s" ] || continue
	keychain "${sign[@]}" "$s" || { echo "couldn't sign $s as ${identity:-ad hoc}"; exit 1; }
done
# A Mac on a build from before the window carried no background program takes its update only if the window carries the
# program, saying what it is (that build's src/update/mac.rs): a copy, which nothing runs, so such a Mac still updates
# to this. It goes once no Mac runs one.
cp "$out/$CONFIG_CLI" "$app/Contents/MacOS/$CONFIG_CLI"
keychain "${sign[@]}" "$app" || { echo "couldn't sign the app as ${identity:-ad hoc}"; exit 1; }
# Its Mac services in Swift, each a helper app of its own holding its own permission (services/sdk/mac-helpers.sh), which
# oned unpacks and runs where it keeps them (oned::helper).
keychain env CONFIG="$config" SIGN="$identity" ICON="$app/Contents/Resources/AppIcon.icns" "$root/services/sdk/mac-helpers.sh" || { echo "couldn't build the Mac's helper apps"; exit 1; }
for helper in "$root"/target/mac-helpers/*.app; do
	[ -e "$helper" ] || continue
	program=$(basename "$(ls -d "$helper/Contents/MacOS/$CONFIG_CLI"-*)")
	ditto -c -k --keepParent "$helper" "$out/$program.zip"
done

# A release is notarized, with the ticket stapled to the app, so Gatekeeper lets it open wherever it's downloaded.
# notarytool's credentials are a keychain profile named after the CLI: xcrun notarytool store-credentials <cli>-notary
if [ "${1:-}" = --release ]; then
	case $identity in "Developer ID Application: "*) ;; *) echo "a release needs a Developer ID Application certificate"; exit 1 ;; esac
	profile=$CONFIG_CLI-notary zip=$native/target/mac/notarize.zip
	if keychain xcrun notarytool history --keychain-profile "$profile" >/dev/null; then
		rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
		keychain xcrun notarytool submit "$zip" --keychain-profile "$profile" --wait
		keychain xcrun stapler staple "$app"
	else
		echo "warning: not notarized, so it opens only where it isn't downloaded with a browser: this Mac's keychain has no notarytool profile $profile (xcrun notarytool store-credentials $profile)"
	fi
fi
ditto -c -k --keepParent "$app" "$out/app.zip"
echo "Built $out"
