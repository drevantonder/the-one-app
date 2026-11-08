# The native app

One app for every platform, Tauri 2 on the client itself (`one-client`, `../`): its window is the web client's own page
(`cloudflare/web/` and @one/ui, from `one::web`), the same sidebar, sheets, menus and alerts as in a browser, run
on this device's client and key rather than a browser's. `src/native.js` is what the page finds of the app; everything
else a screen asks is the page's, as in a browser. Only what a web view can't do is the app's (`src/lib.rs`): the key in
the OS's secret store, signing in through the system's browser, files, the clipboard, links outside, notifications;
and per platform, what's beside it (below).

Its own workspace, so the repo's `cargo build` and `cargo test` never build Tauri. `one throwaway client linux` builds
and runs it signed in to your throwaway network; `cargo build` here, then `ONE_ADDRESS=<address> target/debug/one-native`
on a desktop.

What the window shows comes from `src/bundle.rs`: the release-key-signed web bundle this device last kept (`<data>/web/`, fetched from
`<address>/updates/`), newest first, or the one built in. It takes a new bundle the next time the window opens. A debug build takes
`<CLI>_WEB_KEY` and `<CLI>_WEB_BASE` to test with other keys and places.

Its log (`<data>/logs/client.log`, capped, `one::logs`) holds the page's uncaught errors and failed loads, page loads, panics
and what the platform code says. `one logs <device>` reads it through a node the app is connected to, which asks the app on
that connection (so only while it's open); the node checks who may, as for its own log. It keeps no keys or page content.

## Each platform

Each keeps the old app's identity, so it installed over it in place and kept its sign-in: the Mac's `app.<cli>.mac`
(Developer ID team, same designated requirement, its Keychain items), the
iPhone's `app.<cli>.phone` with `.widgets`, `.notifications` and the App Group `group.app.<cli>.phone` (also its
Keychain group), Android's `app.<cli>.android` signed with the debug key it was signed with, Windows'
package `TheOneApp_ah4xyz62b3km4` (`windows_package()`). Don't change them: a device would sign in again as a new one.

- Linux: `one release` builds it, install.sh puts it on a desktop. Its tray item shows only its menu (Linux's tray items
  show no other), so it has no panel.
- Mac: `mac/build.sh` (`one update`, `one release`, through `disk.sh`), the same bundle as before: the window, the CLI,
  the node and its launch agent, services, helpers.
- iPhone: `iphone/build.sh` for the simulator, the paired iPhone (`one update`) or TestFlight; the Swift in `Sources/` is a
  Tauri plugin of the app's own, `Widget/` the home-screen widget, `Notifications/` the notification extension.
- Android: `android/build.sh`, Kotlin only where it must be (`android/app/src/main/kotlin/app/client`); only throwaways
  build it, as nothing built the old one. Android gives screens' frames Tauri's `ipc`, so each command carries a token
  only the page's own frame has (`ours` in src/platform/android.rs).
- Windows: MSIX, cross-built on Linux and packaged with the C# helper (Widgets board, toasts) in the Windows VM by
  `windows/build.sh`.

Not carried, by Andre's decision: the iPhone's own menus (the web's, but for what touch and hold lifts), Android's accent
colour on what's tapped, a Mac build with push.

## Still open

- Pushes on every platform: built as the old apps', never received. The One App has no APNs key, Firebase project or WNS
  app (Andre, top #87); then notification taps, Approve / Not Me with no window, the iPhone's count and banner.
- TestFlight (`iphone/build.sh testflight`): waits on an App Store Connect app for `app.ptc.phone` (Andre, top #87);
  then Google sign-in, receipt scan and photo upload in Claims there.
- Seen on a Mac: the window, the panel, the dock's count, the widgets' menu bar popovers and the app menu's keys (⌘\,
  ⌘⇧I, ⌘⇧J). Built and checked by its build; the Mac's screen has been locked each time.
- Felt on a real phone: touch and hold lifting on the iPhone, haptics, the iPhone's document camera and share sheet,
  the iPhone's Safari sign-in sheet.
- The iPhone's window following Dynamic Type: WebKit's CSS zoom left screens' frames at no size (a black window) and its
  page zoom keeps the layout at the screen's width, so the window doesn't zoom there (`platform::iphone::text_scale`).
  Android's font size zooms it by CSS, which Chromium lays frames out in.
- Join with a Pairing Code on a Mac whose node is in no network: built, no such Mac to try.
- Speed on real devices: usable within 1.5 s of a cold open is met on Linux (a debug build under Xvfb: window 0.8 s, the
  home 1.1 to 1.25 s); release builds on the Mac, iPhone, Android and Windows not measured.
