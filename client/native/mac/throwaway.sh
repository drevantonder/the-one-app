#!/bin/bash
# `one-dev throwaway client mac`'s half on the Mac: builds this checkout's app as a throwaway's (THROWAWAY: its own id,
# folder and Keychain items, beside the user's own app), signs it as the Mac app is signed, so the Keychain knows it
# from one build to the next, and starts it in the logged-in session, signed in to ADDRESS, with what it says in
# target/throwaway.log here. Started again each time.
#   throwaway.sh <address> <control port>
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
cd "$native"
address=$1 control=$2
THROWAWAY=1 nice cargo build -q
identities=$(security find-identity -v -p codesigning)
identity=$(echo "$identities" | sed -n 's/.*"\(Developer ID Application: [^"]*\)".*/\1/p' | head -1)
identity=${identity:-$(echo "$identities" | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)}
keychain codesign --force --identifier "app.$CONFIG_CLI.mac.throwaway" --sign "${identity:--}" "$native/target/debug/one-native" >/dev/null

job=app.$CONFIG_CLI.mac.throwaway
launchctl bootout "gui/$(id -u)/$job" 2>/dev/null || true
upper=$(echo "$CONFIG_CLI" | tr '[:lower:]' '[:upper:]')
log=$native/target/throwaway.log
rm -f "$log"
mkdir -p "$native/target/throwaway-home"
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
