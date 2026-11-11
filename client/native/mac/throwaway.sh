#!/bin/bash
# `one-dev throwaway client mac`'s half on the Mac: builds this checkout's app as a throwaway's (THROWAWAY: its own id,
# folder and Keychain items, beside the user's own app), signs it as the Mac app is signed, so the Keychain knows it
# from one build to the next, and starts it in the logged-in session, signed in to ADDRESS, with what it says in
# target/throwaway.log here; with this checkout's program beside it as its oned, which, being no app bundle with a
# launch agent, the window starts itself, for a computer of its own (target/throwaway-home, its onet at <onet address>,
# taking what's signed with RELEASE_KEY, the throwaway network's). Started again each time.
#   RELEASE_KEY=<key> throwaway.sh <address> <control port> <onet address>
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
. "$native/throwaway-build.sh"
cd "$native"
address=$1 control=$2 onet=$3
THROWAWAY=1 nice cargo build -q
(cd "$root" && nice cargo build -q -p one-cli)
ln -sf "$root/target/debug/$CONFIG_CLI" "$native/target/debug/${CONFIG_CLI}d"
identities=$(security find-identity -v -p codesigning)
identity=$(echo "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
identity=${identity:-$(echo "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}
keychain codesign --force --identifier "app.$CONFIG_CLI.mac.throwaway" --sign "${identity:--}" "$native/target/debug/one-native" >/dev/null

job=app.$CONFIG_CLI.mac.throwaway
launchctl bootout "gui/$(id -u)/$job" 2>/dev/null || true
upper=$(echo "$CONFIG_CLI" | tr '[:lower:]' '[:upper:]')
log=$native/target/throwaway.log
rm -f "$log"
# Its computer is the throwaway network's: a new network's starts again.
home=$native/target/throwaway-home
[ "$(cat "$home/.network" 2>/dev/null)" = "$address" ] || rm -rf "$home"
mkdir -p "$home" && echo "$address" > "$home/.network"
cat > "$native/target/throwaway.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>$job</string>
  <key>ProgramArguments</key><array><string>$native/target/debug/one-native</string></array>
  <key>EnvironmentVariables</key><dict>
    <key>${upper}_ADDRESS</key><string>$address</string>
    <key>${upper}_SIGN_IN_PRINT</key><string>1</string>
    <key>${upper}_CONTROL</key><string>$control</string>
    <key>${upper}_HOME</key><string>$native/target/throwaway-home</string>
    <key>${upper}_THROWAWAY</key><string>$native/target/throwaway-home</string>
    <key>${upper}_ADDR</key><string>$onet</string>
    <key>${upper}_RELEASE_KEY</key><string>$RELEASE_KEY</string>
  </dict>
  <key>StandardOutPath</key><string>$log</string>
  <key>StandardErrorPath</key><string>$log</string>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Interactive</string>
</dict>
</plist>
PLIST
launchctl bootstrap "gui/$(id -u)" "$native/target/throwaway.plist"
echo "$log"
