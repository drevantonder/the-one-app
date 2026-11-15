---
name: see-it-running
description: Use to get a real picture of a change in the app, on the desktop, Android or the iPhone simulator, and whenever you change a screen, which is shown with real pictures before it's called done.
---

UI work is shown with real pictures of the real app, never described. Save pictures in your harness's scratch
folder, never in the repo, and embed them in your reply. Screen wording and look follow `DESIGN.md`: read the
section of the screen you change first.

## Desktop (Linux)

Start a network (`local-network`), then the app as its owner, on a screen of its own, as
`clients/app/test/isolation.sh` does:

    ONE_HOME=<folder>/client ONE_AS=owner dbus-run-session -- xvfb-run -a -s "-screen 0 1024x720x24" bash -c '
      target/release/one >"$0/app.log" 2>&1 & app=$!
      grep -q -m1 "home opened" <(tail -F --pid=$app "$0/app.log")
      window=$(xdotool search --sync --onlyvisible --name "The One App" | head -1)
      # xdotool mousemove --window "$window" <x> <y> click 1, xdotool type ...
      import -window root "$0/<name>.png"
      kill $app' <scratch folder>

Without `dbus-run-session` GTK waits half a minute for a desktop that isn't there. The app logs `home opened` once
Home is up, though its opening mark is still fading then: take the picture after the press that shows your change.
`xdotool` presses and types; `import` (ImageMagick) takes the picture. Look at every picture before you show it.

## Android

An emulator with the AVD `one-phone` (Android 36, x86_64, KVM) is set up here:

    ~/Android/Sdk/emulator/emulator -avd one-phone -no-window -no-audio &
    adb install -r <the APK `cargo tauri android build` says it made>
    adb exec-out screencap -p > <scratch>/<name>.png

How to build the APK is in `clients/app/README.md`. A phone's app needs a network with an address
(`tooling/network <folder> --edge`, or the README's `TAURI_ONE_ADDRESS`); `adb reverse tcp:<port> tcp:<port>` makes
the emulator's `localhost:<port>` this machine's. Stop the emulator when you're done (`adb emu kill`).

## iPhone simulator

On the Mac (the `iphone` skill for ssh), build with `--target aarch64-sim --debug`, then:

    xcrun simctl boot "one throwaway iPhone"
    xcrun simctl install booted <the .app>
    xcrun simctl launch booted app.theone.client
    xcrun simctl io booted screenshot <file>.png

`scp mac:<file>.png` brings the picture back. Shut the simulator down after (`xcrun simctl shutdown booted`).
