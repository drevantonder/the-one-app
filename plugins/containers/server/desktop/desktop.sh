#!/bin/sh
# Starts the display, its window manager and Chromium (`browser`), and keeps the container running for as long as the
# display. `desktop --no-browser` leaves Chromium to be started later with `browser`, as Chrome does once the user data
# dir it starts from is in place.
# The display is Xvnc's, an X server like Xvfb that RandR resizes while it runs, taking no VNC connections (-rfbport
# -1). `SCREEN` is the size it starts at, `1280x800x24`. It writes its display's number to `-displayfd` once it's
# ready. A container that's stopped and started again keeps its files, so what the last run left behind is removed
# first: an X server refuses to start over its display's lock and socket.
rm -f "/tmp/.X${DISPLAY#:}-lock" "/tmp/.X11-unix/X${DISPLAY#:}" /tmp/display-ready
mkfifo /tmp/display-ready
Xvnc "$DISPLAY" -geometry "${SCREEN%x*}" -depth "${SCREEN##*x}" -rfbport -1 -SecurityTypes None -nolisten tcp \
  -Log "*:stderr:0" -displayfd 3 3>/tmp/display-ready &
display=$!
read -r _ </tmp/display-ready
openbox &
[ "$1" = --no-browser ] || browser about:blank &
wait "$display"
