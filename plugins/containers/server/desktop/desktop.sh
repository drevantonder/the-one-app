#!/bin/sh
# Starts the display, its window manager and Chromium, and keeps the container running for as long as the display.
# Xvfb tells whoever started it that it's ready with SIGUSR1 when it's started with that signal ignored, as xvfb-run
# has it. Chromium's own sandbox is off: the container is its sandbox, and a rootless one can't make Chromium's.
# A container that's stopped and started again keeps its files, so what the last run left behind is removed first: Xvfb
# refuses to start over its display's lock and socket, and Chromium over its profile's Singleton files.
trap : USR1
rm -f "/tmp/.X${DISPLAY#:}-lock" "/tmp/.X11-unix/X${DISPLAY#:}"
rm -f /root/.chromium/SingletonLock /root/.chromium/SingletonSocket /root/.chromium/SingletonCookie
(trap '' USR1; exec Xvfb "$DISPLAY" -screen 0 "$SCREEN" -nolisten tcp) &
display=$!
wait "$display"
openbox &
chromium --no-sandbox --no-first-run --test-type --disable-dev-shm-usage --start-maximized --user-data-dir=/root/.chromium \
  --remote-debugging-port=9222 about:blank >/tmp/chromium.log 2>&1 &
wait "$display"
