#!/bin/bash
# `one-dev throwaway client iphone`'s half on the Mac: builds this checkout's iPhone app for the simulator, on a simulator
# of its own (named NAME, which no other throwaway's has: made the first time, booted now), and starts it signed in to ADDRESS, with what it says
# in target/iphone-throwaway.log here. Started again each time; `one-dev throwaway stop` shuts the simulator down and deletes
# it. The simulator's look is a phone's, not its defaults (light, the usual text size): dark, and the text a size smaller
# (Andre's phone), as a page that only draws at the defaults would show nothing there, as one did when Dynamic Type zoomed it.
#   throwaway.sh <address> <control port> <simulator name>
set -euo pipefail
. "$(dirname "$0")/../apple.sh"
cd "$native"
address=$1 control=$2 name=$3
. "$native/throwaway-build.sh"
sim=$(xcrun simctl list devices -j | jq -r --arg n "$name" '[.devices[][] | select(.name == $n)][0].udid // empty')
if [ -z "$sim" ]; then
	# An iPhone like one the Mac already simulates, on a runtime it has.
	type=$(xcrun simctl list devices available -j | jq -r '[.devices[][] | select(.name | startswith("iPhone"))][0].deviceTypeIdentifier')
	sim=$(xcrun simctl create "$name" "$type")
fi
xcrun simctl boot "$sim" 2>/dev/null || true
xcrun simctl bootstatus "$sim" -b >/dev/null
xcrun simctl ui "$sim" appearance dark
xcrun simctl ui "$sim" content_size small
log=$native/target/iphone-throwaway.log
mkdir -p "$(dirname "$log")"
rm -f "$log"
upper=$(echo "$CONFIG_CLI" | tr '[:lower:]' '[:upper:]')
# What the app reads as it starts, passed by simctl, and where it prints.
export "SIMCTL_CHILD_${upper}_ADDRESS=$address" "SIMCTL_CHILD_${upper}_SIGN_IN_PRINT=1" "SIMCTL_CHILD_${upper}_CONTROL=$control"
# What it prints, which the simulator keeps in its log: from now, into the log file, as it comes.
pkill -f "simctl spawn $sim log stream" 2>/dev/null || true
nohup sh -c "xcrun simctl spawn $sim log stream --style compact --predicate 'process == \"$CONFIG_NAME\"' | sed -l -n -E 's/.*\] \[std(out|err)\] //p' > '$log'" >/dev/null 2>&1 &
SIMULATOR=$sim nice "$native/iphone/build.sh" simulator >/dev/null
echo "$sim $log"
