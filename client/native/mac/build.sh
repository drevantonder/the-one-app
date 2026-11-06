#!/bin/bash
# Builds the Mac app (client/native) with this checkout's program inside it, which its launch agent runs as oned, signs
# it and installs it to ~/Applications; with --release, it also notarizes it. CONFIG=path/to/config.toml builds another
# custom build. NO_INSTALL=1 leaves it built, in client/native/target/mac/.
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
BUNDLE_ID=app.$CONFIG_CLI.mac
label=$BUNDLE_ID.oned
cd "$native"

(cd "$root" && CONFIG=$config ONE_BUILD=$BUILD_COMMIT cargo build --release)
CONFIG=$config ONE_BUILD=$BUILD_COMMIT cargo build --release

app=$native/target/mac/$CONFIG_NAME.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources" "$app/Contents/Library/LaunchAgents"
# The window, by the app's name, as the Mac app's was (what quits and finds it, below, names it so).
cp target/release/one-native "$app/Contents/MacOS/$CONFIG_NAME"
# Cargo names the node after the package, whatever the custom build.
cp "$root/target/release/one" "$app/Contents/MacOS/$CONFIG_CLI"
# Run by this name, it's oned, the Mac as a device.
ln -sf "$CONFIG_CLI" "$app/Contents/MacOS/${CONFIG_CLI}d"
# Its apps' services in Rust (apps/<name>/service/), which Cargo built with it: oned runs each from here.
for m in "$root"/apps/*/service/Cargo.toml; do
	[ -e "$m" ] || continue
	s=$(basename "$(dirname "$(dirname "$m")")")
	cp "$root/target/release/one-$s" "$app/Contents/MacOS/$CONFIG_CLI-$s"
done

# The icon is the config's as it is.
icon=$(art icon)
rm -rf target/mac/AppIcon.iconset && mkdir -p target/mac/AppIcon.iconset
for s in 16 32 128 256 512; do
	sips -z $s $s "$icon" --out target/mac/AppIcon.iconset/icon_${s}x${s}.png >/dev/null
	sips -z $((s * 2)) $((s * 2)) "$icon" --out target/mac/AppIcon.iconset/icon_${s}x${s}@2x.png >/dev/null
done
iconutil -c icns target/mac/AppIcon.iconset -o "$app/Contents/Resources/AppIcon.icns"

# What macOS, the node's scripts (install.sh, update) and its helpers read of it.
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
  <key>CFBundleURLTypes</key>
  <array><dict>
    <key>CFBundleURLName</key><string>$BUNDLE_ID</string>
    <key>CFBundleURLSchemes</key><array><string>$CONFIG_CLI</string></array>
  </dict></array>
</dict>
</plist>
PLIST

# Only this build's launch agent: one an earlier build left would still be registered.
cat > "$app/Contents/Library/LaunchAgents/$label.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$label</string>
  <key>BundleProgram</key><string>Contents/MacOS/${CONFIG_CLI}d</string>
  <key>ProgramArguments</key><array><string>${CONFIG_CLI}d</string></array>
  <key>AssociatedBundleIdentifiers</key><array><string>$BUNDLE_ID</string></array>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
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
# Its Mac services in Swift, each a helper app of its own holding its own permission (service/mac-helpers.sh), which
# oned runs a copy of (oned::placed).
keychain env CONFIG="$config" SIGN="$identity" ICON="$app/Contents/Resources/AppIcon.icns" "$root/service/mac-helpers.sh" || { echo "couldn't build the Mac's helper apps"; exit 1; }
[ -z "$(ls "$root/target/mac-helpers")" ] || ditto "$root/target/mac-helpers" "$app/Contents/Helpers"
for s in "$app/Contents/MacOS/$CONFIG_CLI"-* "$app/Contents/MacOS/$CONFIG_CLI"; do
	[ -e "$s" ] || continue
	keychain "${sign[@]}" "$s" || { echo "couldn't sign $s as ${identity:-ad hoc}"; exit 1; }
done
keychain "${sign[@]}" "$app" || { echo "couldn't sign the app as ${identity:-ad hoc}"; exit 1; }

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
[ -z "${NO_INSTALL:-}" ] || { echo "Built $app"; exit 0; }

# Installing isn't stopped part way: an update's job stops a build for a newer one, which waits for this instead.
trap '' TERM INT
# Only one copy may run: ask the running app to quit, wait for it, and stop it if it won't, before the new one opens.
osascript -e "quit app id \"$BUNDLE_ID\"" 2>/dev/null || true
for _ in $(seq 20); do pgrep -f "/Contents/MacOS/$CONFIG_NAME\$" >/dev/null || break; sleep 0.5; done
pkill -f "/Contents/MacOS/$CONFIG_NAME\$" 2>/dev/null && sleep 1 || true
# oned the app started itself, when its launch agent wasn't on, outlives it: stop it too, so the launch agent's, the
# new build, is the one that runs.
pkill -f "/Contents/MacOS/${CONFIG_CLI}d\$" 2>/dev/null || true
dest="$HOME/Applications/$CONFIG_NAME.app"
rm -rf "$dest"
ditto "$app" "$dest"
mkdir -p "$HOME/.local/bin"
# Another program by the CLI's name stays; the app's node runs all the same.
cli=$HOME/.local/bin/$CONFIG_CLI
if [ ! -e "$cli" ] || [ -L "$cli" ]; then ln -sf "$dest/Contents/MacOS/$CONFIG_CLI" "$cli"; else echo "Left $cli alone: it isn't this app's"; fi
# A running node keeps the old binary until it restarts.
launchctl kickstart -k "gui/$(id -u)/$label" 2>/dev/null || true
open "$dest"
echo "Installed $dest"
