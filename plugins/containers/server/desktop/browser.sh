#!/bin/sh
# Starts Google Chrome on the display, headed, on the user data dir /root/.chromium, with its DevTools Protocol on port
# 9222, opening what it's given. Chrome's own sandbox is off: the container is its sandbox, and a rootless one can't
# make Chrome's. Its cookies are encrypted with a key of Chrome's own rather than the desktop's keyring, which a
# container hasn't got, so the folder opens wherever it's copied. A folder copied from a stopped container keeps
# Chrome's Singleton files, which it refuses to start over.
rm -f /root/.chromium/SingletonLock /root/.chromium/SingletonSocket /root/.chromium/SingletonCookie
exec google-chrome-stable --no-sandbox --no-first-run --no-default-browser-check --disable-dev-shm-usage --start-maximized \
  --password-store=basic --user-data-dir=/root/.chromium --remote-debugging-port=9222 "$@" >>/tmp/chromium.log 2>&1
