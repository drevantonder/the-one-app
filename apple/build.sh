#!/bin/bash
# Builds the Mac app with this checkout's program inside it, which its launch agent runs as oned, signs it and installs it to ~/Applications; with --release, it
# also notarizes it. CONFIG=path/to/config.toml builds another custom build.
set -euo pipefail
cd "$(dirname "$0")"
CLIENT_TARGETS=aarch64-apple-darwin
. ./project.sh
BUNDLE_ID=app.$CONFIG_CLI.mac
label=$BUNDLE_ID.oned

(cd "$root" && CONFIG=$config ONE_BUILD=$(git rev-parse --short=12 HEAD) cargo build --release)

xcodebuild -project Shells.xcodeproj -scheme Mac -configuration Release -destination generic/platform=macOS -derivedDataPath build/derived build -quiet

app="build/derived/Build/Products/Release/$CONFIG_NAME.app"
# Cargo names the binary after the package, whatever the custom build.
cp "$root/target/release/one" "$app/Contents/MacOS/$CONFIG_CLI"
# Run by this name, it's oned, the Mac as a device.
ln -sf "$CONFIG_CLI" "$app/Contents/MacOS/${CONFIG_CLI}d"
# Its apps' services in Rust (apps/<name>/service/), which Cargo built with it: oned runs each from here.
for m in "$root"/apps/*/service/Cargo.toml; do
	[ -e "$m" ] || continue
	s=$(basename "$(dirname "$(dirname "$m")")")
	cp "$root/target/release/one-$s" "$app/Contents/MacOS/$CONFIG_CLI-$s"
done
mkdir -p "$app/Contents/Resources" && cp build/AppIcon.icns "$app/Contents/Resources/"
plutil -replace CFBundleIconFile -string AppIcon "$app/Contents/Info.plist"
# Only this build's launch agent: one an earlier build left would still be registered.
rm -rf "$app/Contents/Library/LaunchAgents"
mkdir -p "$app/Contents/Library/LaunchAgents"
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
# the same way, since macOS keeps the permissions it grants (Reminders, Full Disk Access…) only for code signed alike.
# Without a Developer ID it falls back to Apple Development, then ad hoc. Over SSH the keychain is out of reach, so
# codesign and notarytool run as one-shot jobs in the logged-in session instead.
keychain() { "$@" 2>/dev/null || session "$@"; }
identities=$(security find-identity -v -p codesigning)
identity=$(echo "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
identity=${identity:-$(echo "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}
sign=(codesign --force --options runtime --sign "${identity:--}")
[ -z "$identity" ] || sign+=(--timestamp)
entitlements=$PWD/Mac.entitlements
# With push (apple/project.sh), the app and its notification extension each carry the Developer ID provisioning profile
# of their App ID, which grants push and the keychain group where the extension reads the device keys it opens notes
# with: made at developer.apple.com, and downloaded to ~/Library/Developer/Xcode/UserData/Provisioning Profiles.
if [ -n "$PUSH" ]; then
	# The profile for App ID $1, and its entitlements, in build/profile.plist.
	profile() {
		for f in "$HOME/Library/Developer/Xcode/UserData/Provisioning Profiles"/* "$HOME/Library/MobileDevice/Provisioning Profiles"/*; do
			[ -f "$f" ] && security cms -D -i "$f" > build/profile.plist 2>/dev/null || continue
			[ "$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.application-identifier" build/profile.plist 2>/dev/null)" = "$APPLE_TEAM.$1" ] &&
				[ "$(/usr/libexec/PlistBuddy -c "Print :ProvisionsAllDevices" build/profile.plist 2>/dev/null)" = true ] && { echo "$f"; return; }
		done
		echo "push needs a Developer ID provisioning profile for $1 (developer.apple.com), downloaded to ~/Library/Developer/Xcode/UserData/Provisioning Profiles" >&2
		return 1
	}
	# Entitlements $1 for App ID $2, from $3's if given, with what its profile grants: its id, the team, and the keychain
	# group.
	granted() {
		if [ -n "${3:-}" ]; then cp "$3" "$1"; else printf '<?xml version="1.0" encoding="UTF-8"?>\n<plist version="1.0"><dict/></plist>\n' > "$1"; fi
		/usr/libexec/PlistBuddy -c "Add :com.apple.application-identifier string $APPLE_TEAM.$2" -c "Add :com.apple.developer.team-identifier string $APPLE_TEAM" \
			-c "Add :keychain-access-groups array" -c "Add :keychain-access-groups:0 string $APPLE_TEAM.$BUNDLE_ID" "$1"
	}
	appex="$app/Contents/PlugIns/Notifications.appex"
	cp "$(profile "$BUNDLE_ID.notifications")" "$appex/Contents/embedded.provisionprofile"
	granted build/MacNotifications.entitlements "$BUNDLE_ID.notifications"
	/usr/libexec/PlistBuddy -c "Add :com.apple.security.app-sandbox bool true" build/MacNotifications.entitlements
	cp "$(profile "$BUNDLE_ID")" "$app/Contents/embedded.provisionprofile"
	aps=$(/usr/libexec/PlistBuddy -c "Print :Entitlements:com.apple.developer.aps-environment" build/profile.plist)
	entitlements=$PWD/build/Mac.entitlements
	granted "$entitlements" "$BUNDLE_ID" Mac.entitlements
	/usr/libexec/PlistBuddy -c "Add :com.apple.developer.aps-environment string $aps" "$entitlements"
	keychain "${sign[@]}" --entitlements "$PWD/build/MacNotifications.entitlements" "$PWD/$appex" || { echo "couldn't sign the notification extension as ${identity:-ad hoc}"; exit 1; }
fi
# Its Mac services in Swift, each a helper app of its own holding its own permission (service/mac-helpers.sh), which
# oned runs from here.
keychain env CONFIG="$config" SIGN="$identity" ICON="$PWD/build/AppIcon.icns" "$root/service/mac-helpers.sh" || { echo "couldn't build the Mac's helper apps"; exit 1; }
rm -rf "$app/Contents/Helpers"
[ -z "$(ls "$root/target/mac-helpers")" ] || ditto "$root/target/mac-helpers" "$app/Contents/Helpers"
for s in "$app/Contents/MacOS/$CONFIG_CLI"-*; do
	[ -e "$s" ] || continue
	keychain "${sign[@]}" "$PWD/$s" || { echo "couldn't sign $s as ${identity:-ad hoc}"; exit 1; }
done
keychain "${sign[@]}" "$PWD/$app/Contents/MacOS/$CONFIG_CLI" &&
	keychain "${sign[@]}" --entitlements "$entitlements" "$PWD/$app" || { echo "couldn't sign the app as ${identity:-ad hoc}"; exit 1; }

# A release is notarized, with the ticket stapled to the app, so Gatekeeper lets it open wherever it's downloaded.
# notarytool's credentials are a keychain profile named after the CLI: xcrun notarytool store-credentials <cli>-notary
if [ "${1:-}" = --release ]; then
	case $identity in "Developer ID Application: "*) ;; *) echo "a release needs a Developer ID Application certificate"; exit 1 ;; esac
	profile=$CONFIG_CLI-notary zip=$PWD/build/notarize.zip
	if keychain xcrun notarytool history --keychain-profile "$profile" >/dev/null; then
		rm -f "$zip" && ditto -c -k --keepParent "$app" "$zip"
		keychain xcrun notarytool submit "$zip" --keychain-profile "$profile" --wait
		keychain xcrun stapler staple "$PWD/$app"
	else
		echo "warning: not notarized, so it opens only where it isn't downloaded with a browser: this Mac's keychain has no notarytool profile $profile (xcrun notarytool store-credentials $profile)"
	fi
fi

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
