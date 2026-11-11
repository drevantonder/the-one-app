#!/bin/bash
# `one-dev throwaway client mac`'s half on the Mac: builds this checkout's window as a throwaway's (THROWAWAY: its own id,
# folder and Keychain items, beside the user's own app), signed as the Mac app is signed, so the Keychain knows it from
# one build to the next, in the home of a computer of its own (target/throwaway-home, its onet at <onet address>, taking
# what's signed with RELEASE_KEY, the throwaway network's), where `<cli> update` puts its window. Beside it, in
# target/throwaway-publish, what the throwaway network publishes for its computer to set up from, as
# client/native/mac/build.sh makes it for a real one: the window zipped, the program, and each helper app zipped. Its
# window's launch job, which starts it in the logged-in session signed in to ADDRESS, with what it says in
# target/throwaway.log, is written, not started: one-dev starts it once it's published those. Built again each time.
#   RELEASE_KEY=<key> throwaway.sh <address> <control port> <onet address>
# → its log, the folder to publish and the window's launch job, on its last line.
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
. "$native/throwaway-build.sh"
cd "$native"
address=$1 control=$2 onet=$3
THROWAWAY=1 nice cargo build -q
(cd "$root" && nice cargo build -q -p one-cli)
identities=$(security find-identity -v -p codesigning)
identity=$(echo "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
identity=${identity:-$(echo "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}

job=app.$CONFIG_CLI.mac.throwaway
launchctl bootout "gui/$(id -u)/$job" 2>/dev/null || true
upper=$(echo "$CONFIG_CLI" | tr '[:lower:]' '[:upper:]')
log=$native/target/throwaway.log
rm -f "$log"
# Its computer is the throwaway network's: a new network's starts again, its oned's launch agent with it.
home=$native/target/throwaway-home
if [ "$(cat "$home/.network" 2>/dev/null)" != "$address" ]; then
	for agent in "$home"/app.$CONFIG_CLI.${CONFIG_CLI}d.throwaway-*.plist; do
		[ -e "$agent" ] && launchctl bootout "gui/$(id -u)" "$agent" 2>/dev/null || true
	done
	rm -rf "$home"
fi
mkdir -p "$home" && echo "$address" > "$home/.network"

publish=$native/target/throwaway-publish
rm -rf "$publish" && mkdir -p "$publish"
cp "$root/target/debug/$CONFIG_CLI" "$publish/$CONFIG_CLI"
build=$("$publish/$CONFIG_CLI" version --json | sed -n 's/.*"commit":"\([^"]*\)".*/\1/p')
env=(HOME "$home" THROWAWAY "$home" ADDR "$onet" RELEASE_KEY "$RELEASE_KEY" ADDRESS "$address")
dict() { local k v out=; while [ $# -gt 0 ]; do k=$1 v=$2; shift 2; out+="<key>${upper}_$k</key><string>$v</string>"; done; echo "$out"; }
# The window, as a Mac's is: where `<cli> update` puts it, opened by launchd or, after an update, by `open`, which gives
# it what's in its LSEnvironment.
app=$home/Applications/$CONFIG_NAME.app
rm -rf "$app" && mkdir -p "$app/Contents/MacOS"
cp target/debug/one-native "$app/Contents/MacOS/$CONFIG_NAME"
cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleIdentifier</key><string>$job</string>
  <key>CFBundleExecutable</key><string>$CONFIG_NAME</string>
  <key>CFBundleName</key><string>$CONFIG_NAME</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>Build</key><string>$build</string>
  <key>LSEnvironment</key><dict>$(dict "${env[@]}" SIGN_IN_PRINT 1 CONTROL "$control")</dict>
</dict>
</plist>
PLIST
keychain codesign --force --sign "${identity:--}" "$app" >/dev/null
ditto -c -k --keepParent "$app" "$publish/app.zip"
# Its helper apps, as a real Mac's are, which its oned unpacks and runs as it does a real one's.
keychain env SIGN="$identity" "$root/services/sdk/mac-helpers.sh" >/dev/null
for helper in "$root"/target/mac-helpers/*.app; do
	[ -e "$helper" ] || continue
	program=$(basename "$(ls -d "$helper/Contents/MacOS/$CONFIG_CLI"-*)")
	ditto -c -k --keepParent "$helper" "$publish/$program.zip"
done

cat > "$native/target/throwaway.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$job</string>
  <key>ProgramArguments</key><array><string>$app/Contents/MacOS/$CONFIG_NAME</string></array>
  <key>EnvironmentVariables</key><dict>$(dict "${env[@]}" SIGN_IN_PRINT 1 CONTROL "$control")</dict>
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
PLIST
echo "$log $publish $native/target/throwaway.plist"
