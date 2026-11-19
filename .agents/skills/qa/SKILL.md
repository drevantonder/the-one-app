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

### Scenes: every UI change

A screen's states are scenes, with what they show beside them, so no network is needed to see one: a component's in
`core/ui/gallery/*.scenes.tsx`, an app's in `apps/<app>/client/scenes.json` (the format is `core/ui/scenes.js`'s
header). A change that makes a screen, or a state of one, adds its scene. Every change to how something looks runs,
in core/ui, after `bun build.js`:

    bun scenes.js --out <scratch>/before          # on main, before the change (or an earlier run)
    bun scenes.js --out <scratch>/after --against <scratch>/before [<folder>…]

It draws every scene, or those of the folders named, on the DOM in headless Chromium (Playwright's, which
`bunx playwright-core install chromium` fetches where it's missing), by our renderer on the web there (its WebGPU on
SwiftShader: `web.png`, compared with `native.png`; a scene with taps isn't yet) and on a native host by its headless
harness (`cargo build -p renderer --example screenshot`). What counts is **what changed since `--against`** on our renderer
(`changed.png`): a tree that changed, or pictures more than 2% apart, is looked at. A difference that's meant is said
in the commit; one that isn't is fixed. Our renderer is the reference, judged against DESIGN.md (Raycast-inspired), not
the browser: it will draw the web too. Until it does, browsers, MCP hosts and older phones still show the DOM, so a
large DOM-vs-native difference it reports (`dom.png`, `native.png`, `diff.png`) is worth a look, but matching the DOM
isn't the goal. A native host that hasn't the runtime version the UI library needs draws nothing, which it says.

### Which to use

On a computer every app's page opens as a native window: drive it with the inspector (Native windows, below). A
screen's states with no network → `bun scenes.js` (above): before against after.

### Native windows

An app's window is drawn natively by the launcher's process, so WebDriver doesn't reach it: drive it with the
inspector, which every native window answers (`clients/app/renderer/src/inspect.rs` is its
protocol). Find nodes by role and text, tap, type, `settle`, then `snap`, never a sleep:

- Offscreen, the same on every run, with no network: `bun dist/build/app.js apps/<app>/client <out>` in `core/ui`
  (after `bun build.js`), then `cargo run -p renderer --example screenshot -- <out> <png> <json>
  --ui core/ui/dist/client --answers <answers.json> --requests <requests.jsonl>`, its header says how. Its clock is
  paused: `clock` `step` moves it.
- On Windows (the Surface, `ssh surface`): build with `cargo xwin build --target x86_64-pc-windows-msvc` (the
  harness as `--example screenshot`, the app as `-p app`) and copy it to a folder of your own in `%TEMP%`; the harness
  runs over ssh as it is (Direct3D 12 draws offscreen without a desktop). The app's windows need the logged-in
  session: start it there with a scheduled task (`schtasks /Create … /IT`, `conhost.exe --headless <a .cmd setting
  ONE_HOME>`), and a second `one.exe one:<app>` with the same `ONE_HOME`, even over ssh, hands it the link. A local
  network's edge is on this machine's localhost: `ssh -N -R <port>:localhost:<port> surface` puts it at the same
  address there. Its screen is often locked: `computer-use screenshot` with the window's `pid` and `window_id`
  still pictures it. Delete the folder, the tasks and the debug build's `%LOCALAPPDATA%\app.theone.client.dev` after.
- In the app on a local network (about 9 s to a picture), with your worktree's build, `target/debug/one` (the
  `one` on your PATH is the live release, which hasn't your change; the app and the CLI are that one program):
  install the Inspector once (`target/debug/one install --app network/apps/inspector`), start `openbox` and
  `target/debug/one one:<app>` in a desktop of your own, so nothing is shared with other agents and nothing is left
  running (`ONE_HOME=<folder>/client ONE_AS=owner dbus-run-session -- xvfb-run -a -s "-screen 0 1280x900x24" sh -c
  'openbox & target/debug/one one:<app>'`: without a window manager the launcher loses its focus as it resizes and
  puts itself away, and without `dbus-run-session` GTK waits half a minute), then call its tools as that user:
  `target/debug/one inspector windows`, `… inspector tap --text …`, `… inspector settle`, `… inspector snap >
  <scratch>/<name>.png` (stdout is the PNG, the window's own size,
  1024×720 at first). They reach the window through the launcher, as an agent on another device does: it draws every
  native window itself.

### Android

An emulator with the AVD `one-phone` (Android 36, x86_64, KVM) is set up here:

    ~/Android/Sdk/emulator/emulator -avd one-phone -no-window -no-audio &
    adb install -r <the APK `cargo tauri android build` says it made>
    adb exec-out screencap -p > <scratch>/<name>.png

How to build the APK is in `clients/app/README.md`. Rust alone checks for Android with the NDK's compilers on `PATH`:
`PATH=~/Android/Sdk/ndk/<version>/toolchains/llvm/prebuilt/linux-x86_64/bin:$PATH cargo check -p renderer --target
x86_64-linux-android`. In a worktree that hasn't built yet, run `tooling/copy-build --android` first
(Cargo's Android target, Gradle's build folders and the generated project come from the worktree that built an APK last, so the
build is about a minute, not minutes); sccache and `~/.gradle` are already shared by every worktree. A new AVD's Chrome asks to
sign in and about notifications the first time it opens, and ignores its command-line file (`--disable-fre`) on a phone that
isn't debuggable: tap "Use without an account" and "No thanks" once (find them with `adb shell uiautomator dump`), and the
AVD keeps that. A phone's app needs a network's address
(the one `tooling/network` prints, or the README's `TAURI_ONE_ADDRESS`); `adb reverse tcp:<port> tcp:<port>` makes
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

- Pictures in the web client (`clients/web`) in Chrome: a browser signs in with a passkey and holds nobody's key, so
  the Vault and what's encrypted don't open there as they do in the app.

- Checking again after rebasing onto other people's unrelated commits.
- A check that read a log before the process had created it, and failed for that. Follow a log with `tail -F`, as
  `tooling/network` does, never by reading it once.
- Merging a fix whose check was failing, on the guess that the check was wrong.
- Reproducing a reported bug in your own setup when it only happens in the reporter's: half an hour at the default
  text size for a bug that showed only at the owner's. Start with their exact device, size, text size and build.
