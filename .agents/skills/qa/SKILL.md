---
name: qa
description: Use before you commit a change, to check it works and doesn't break what it could touch, and whenever you change a screen, which is shown with real pictures before it's called done. There are no kept tests: you check your own change.
---

Check your work with a throwaway check and never commit it. Use the cheapest thing that shows it works: call the
tool on a local network (`local-network`), open the screen (Screens, below), or run the function in `iex -S mix`
in `core/server/`.

Work out what your change touches, or could touch, including things that look unrelated, and check those too. The skill
`blast-radius` finds what it could break beyond the diff. If
that includes secrets, who may do what, an app's entrypoint reaching another's, or data kept and restored, try to break
it yourself on a local network. You decide the attacks; there is no fixed list.

A failing check is chased until you know why, and reported. Never skip it, loosen it, or commit "a small fix" on a
guess you haven't reproduced.

## Screens

UI work is shown with real pictures of the real app, never described. Pictures are for checking your own change:
take the few that check it, not every size and theme, a before only where judging it needs one, and a recording
only where motion is what changed. A change that goes live in seconds, as an app deploy does, is seen live.
A phone-sized desktop window (390×844) shows most screen changes; build for a phone or simulator only when the change
is about that platform. Save pictures in your harness's scratch folder, never in the repo, and look at each before
you show it. Screen wording and look follow `DESIGN.md`: read the section of the screen you change first.

### Desktop (Linux), by WebDriver

Tauri's WebDriver drives the app: `tauri-driver` (`cargo install tauri-driver --locked`) in front of WebKitGTK's
`WebKitWebDriver` (`webkit2gtk-driver`), with any WebDriver client. Start a network (`local-network`), then:

    ONE_HOME=<folder>/client ONE_AS=owner dbus-run-session -- xvfb-run -a -s "-screen 0 1024x900x24" bash -c '
      free() { python3 -c "import socket; s = socket.socket(); s.bind((\"\", 0)); print(s.getsockname()[1])"; }
      export WEBDRIVER=http://127.0.0.1:$(free)
      tauri-driver --port ${WEBDRIVER##*:} --native-port $(free) >"$0/driver.log" 2>&1 & driver=$!
      uv run -q --with selenium python3 <your script, which connects to $WEBDRIVER>
      kill $driver' <scratch folder>

Without `dbus-run-session` GTK waits half a minute for a desktop that isn't there. Its ports are free ones, never
`tauri-driver`'s own 4444 and 4445: other agents run WebDriver on this machine too. The session's capabilities are
`{"browserName": "wry", "tauri:options": {"application": "<repo>/target/debug/one", "args": ["<link>"]}}`, then
`set_window_size(390, 844)`; the app's log is `driver.log`. One app runs per desktop session: a second start,
`target/debug/one <link>`, hands its link to it, as a desktop's link handler does.

- Open the screen by its link, the same one a person's link or notification opens: `one:<app>` (its first page),
  `one:<app>/<page>` (a page by its file, `one:sign-in/client/security.html`), and a fragment the app follows,
  `one:home-screen#library`. Click only for what the change is about.
- Wait for state, never a time: the window is ready once `body:not([aria-busy]) iframe:not([inert]):not([aria-busy])`
  is there (the app showing has drawn), and still once what moves has ended, in the window and in the app's frame
  (`switch_to.frame`): `Promise.all(document.getAnimations().filter(a => a.effect?.getComputedTiming().endTime !==
  Infinity).map(a => a.finished.catch(() => {})))`, as a cancelled one rejects.
- An app's elements are in its frame: switch to it to find, click or wait, and back to the window for the picture
  (`save_screenshot`). An element's own screenshot inside a frame comes out black.
- A hold is `click_and_hold(…).pause(1).release()`; WebKit's driver's right-click opens no menu.
- Touch isn't WebKitGTK's: its driver sends a touch pointer as a mouse, and one finger at most. Touch and a second
  finger are Chromium's, so Android's (below) or Chrome's: CDP's `Input.dispatchTouchEvent` with a `touchPoints`
  entry per finger, or in Chrome W3C Actions with one `pointerType: "touch"` pointer per finger in one action.
- What every page says to its console, Solid's dev warnings too, is in `driver.log` as `CONSOLE WARN …` (a debug
  build only). WebKitGTK here has no WebDriver BiDi, so `log.entryAdded` isn't there.

### Android

An emulator with the AVD `one-phone` (Android 36, x86_64, KVM) is set up here:

    ~/Android/Sdk/emulator/emulator -avd one-phone -no-window -no-audio &
    adb install -r <the APK `cargo tauri android build` says it made>
    adb exec-out screencap -p > <scratch>/<name>.png

How to build the APK is in `clients/app/README.md`. In a worktree that hasn't built yet, run `tooling/copy-build --android` first
(Cargo's Android target, Gradle's build folders and the generated project come from the worktree that built an APK last, so the
build is about a minute, not minutes); sccache and `~/.gradle` are already shared by every worktree. A new AVD's Chrome asks to
sign in and about notifications the first time it opens, and ignores its command-line file (`--disable-fre`) on a phone that
isn't debuggable: tap "Use without an account" and "No thanks" once (find them with `adb shell uiautomator dump`), and the
AVD keeps that. A phone's app needs a network with an address
(`tooling/network <folder> --edge`, or the README's `TAURI_ONE_ADDRESS`); `adb reverse tcp:<port> tcp:<port>` makes
the emulator's `localhost:<port>` this machine's. Stop the emulator when you're done (`adb emu kill`).

A debug build's app is a WebView that takes CDP: `adb forward tcp:0 localabstract:webview_devtools_remote_$(adb
shell pidof app.theone.client)` gives the port, `/json` there its page, and an app's frame is in `Page.getFrameTree`.
That's how touch and a second finger reach it (`Input.dispatchTouchEvent`); `adb shell input` has one finger.

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
