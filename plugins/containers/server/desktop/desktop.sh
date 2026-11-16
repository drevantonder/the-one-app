#!/bin/sh
# Starts the display, its window manager and Chromium (`browser`), and keeps the container running for as long as the
# display. `desktop --no-browser` leaves Chromium to be started later with `browser`, as Chrome does once the user data
# dir it starts from is in place.
# Xvfb tells whoever started it that it's ready with SIGUSR1 when it's started with that signal ignored, as xvfb-run
# has it. A container that's stopped and started again keeps its files, so what the last run left behind is removed
# first: Xvfb refuses to start over its display's lock and socket.
trap : USR1
rm -f "/tmp/.X${DISPLAY#:}-lock" "/tmp/.X11-unix/X${DISPLAY#:}"
(trap '' USR1; exec Xvfb "$DISPLAY" -screen 0 "$SCREEN" -nolisten tcp) &
display=$!
wait "$display"
openbox &
[ "$1" = --no-browser ] || browser about:blank &
wait "$display"
