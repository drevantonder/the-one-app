#!/bin/bash
# Installs the app at $1 (built by client/native/mac/build.sh or downloaded and checked by the Mac's job beside it) to
# ~/Applications and starts it. CONFIG_NAME and CONFIG_CLI name it.
set -euo pipefail
app=$1
BUNDLE_ID=app.$CONFIG_CLI.mac
label=$BUNDLE_ID.oned
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
