---
name: qa
description: Use before you commit a change, to check it works and doesn't break what it could touch, and whenever you change a screen, which is shown with real pictures before it's called done. There are no kept tests: you check your own change.
---

Check your work with a throwaway check and never commit it. Use the cheapest thing that shows it works: call the
tool on a local network (`local-network`), open the screen (Screens, below), or run the function in `iex -S mix`
in `core/server/`.

Work out what your change touches, or could touch, including things that look unrelated, and check those too. If
that includes secrets, who may do what, a plugin's app reaching another's, or data kept and restored, try to break
it yourself on a local network. You decide the attacks; there is no fixed list.

A failing check is chased until you know why, and reported. Never skip it, loosen it, or commit "a small fix" on a
guess you haven't reproduced.

## Screens

UI work is shown with real pictures of the real app, never described. A phone-sized desktop window (390×844) shows
most screen changes in a minute; build for a phone or simulator only when the change is about that platform. Save pictures in your harness's scratch
folder, never in the repo, and embed them in your reply. Screen wording and look follow `DESIGN.md`: read the
section of the screen you change first.

### Desktop (Linux)

Start a network (`local-network`), then the app as its owner, on a screen of its own:

    ONE_HOME=<folder>/client ONE_AS=owner dbus-run-session -- xvfb-run -a -s "-screen 0 1024x720x24" bash -c '
      target/debug/one >"$0/app.log" 2>&1 & app=$!
      grep -q -m1 "^home-screen opened" <(tail -F --pid=$app -n +1 "$0/app.log")
      window=$(xdotool search --sync --onlyvisible --name "The One App" | head -1)
      # xdotool mousemove --window "$window" <x> <y> click 1, xdotool type ...
      import -window root "$0/<name>.png"
      kill $app' <scratch folder>

Without `dbus-run-session` GTK waits half a minute for a desktop that isn't there. The app logs `the client opened`, then `home-screen opened` once
the Home Screen is up, though its opening mark is still fading then: take the picture after the press that shows your change.
`xdotool` presses and types; `import` (ImageMagick) takes the picture. Look at every picture before you show it.

### Android

An emulator with the AVD `one-phone` (Android 36, x86_64, KVM) is set up here:

    ~/Android/Sdk/emulator/emulator -avd one-phone -no-window -no-audio &
    adb install -r <the APK `cargo tauri android build` says it made>
    adb exec-out screencap -p > <scratch>/<name>.png

How to build the APK is in `clients/app/README.md`. A phone's app needs a network with an address
(`tooling/network <folder> --edge`, or the README's `TAURI_ONE_ADDRESS`); `adb reverse tcp:<port> tcp:<port>` makes
the emulator's `localhost:<port>` this machine's. Stop the emulator when you're done (`adb emu kill`).

### iPhone simulator

On the Mac (the `iphone` skill for ssh), build with `--target aarch64-sim --debug`, then:

    xcrun simctl boot "one throwaway iPhone"
    xcrun simctl install booted <the .app>
    xcrun simctl launch booted app.theone.client
    xcrun simctl io booted screenshot <file>.png

`scp mac:<file>.png` brings the picture back. Shut the simulator down after (`xcrun simctl shutdown booted`).

The app keeps its client in its App Group, which the simulator gives it, so `main.mm` sets `ONE_HOME` to that and
`SIMCTL_CHILD_ONE_HOME` changes nothing. To open it on a network of your own, set aside what's in
`xcrun simctl get_app_container booted app.theone.client group.app.theone.client`, put your client's `key` and
`network` there, make the network's address the Mac's too (`ssh -N -R <port>:localhost:<port> mac`), launch with
`SIMCTL_CHILD_ONE_AS=owner`, and put back what was there after. `axe tap -x <x> -y <y> --udid <udid>` taps, in points. `axe swipe` and drags need `--delta 4` to move like a finger: its default
steps of 50 px look like a velocity bug in any gesture.

## Mistakes already made

- Checking again after rebasing onto other people's unrelated commits.
- A check that read a log before the process had created it, and failed for that. Follow a log with `tail -F`, as
  `tooling/network` does, never by reading it once.
- Merging a fix whose check was failing, on the guess that the check was wrong.
- Reproducing a reported bug in your own setup when it only happens in the reporter's: half an hour at the default
  text size for a bug that showed only at the owner's. Start with their exact device, size, text size and build.
