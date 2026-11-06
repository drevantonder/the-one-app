# The native app

One app for every platform, Tauri 2 on the client itself (`one-client`, `../`): its window is the web client's own page
(`cloudflare/web/web.js` and @one/ui, from `one::web`), the same sidebar, sheets, menus and alerts as in a browser, run
on this device's client and key rather than a browser's. `src/native.js` is what the page finds of the app; everything
else a screen asks is the page's, as in a browser. Only what a web view can't do is the app's (`src/lib.rs`): the key in
the OS's secret store, signing in through the system's browser, files, the clipboard, links outside, notifications;
and per platform, what's beside it (below).

Its own workspace, so the repo's `cargo build` and `cargo test` never build Tauri. `one throwaway client linux` builds
and runs it signed in to your throwaway network; `cargo build` here, then `ONE_ADDRESS=<address> target/debug/one-native`
on a desktop.

## Replacing the old apps

Each platform's old app is gone: this is the list of what they did that this must too, ticked as it does and checked in
`one throwaway`. Then this section goes, down to what the code can't say.

Shared, every platform:

- [x] Window is the web client's: sidebar, pages, screens in sandboxed frames, sheets, menus, alerts (checked, Linux)
- [x] Signs in through the system's browser (loopback redirect), names the device it's for; refresh token in the
      secret store (checked, Linux)
- [x] Let in by code (Waiting for Approval, QR to the address's /approve) (checked, Linux)
- [x] Node events: the window and screens keep up (a CLI post shows at once) (checked, Linux)
- [x] Screens reach none of the app's commands: no Tauri internals in frames, IPC fetch refused, a frame's message to
      WebKit's ipc handler runs nothing (copy probe), parent page unreachable (checked, Linux)
- [x] Address or invite link typed on the welcome, with the old apps' words; an invite opens in the browser (for the
      passkey), then "Made your passkey? Sign in with it."; its secret goes with the request to be let in (checked,
      Linux, but for the browser itself, which the headless test has none of)
- [ ] Save, preview, copy, links outside from screens
- [ ] Notifications permission (Settings › Devices), and notes shown
- [ ] Taken out (removed) → signed out, says why
- [x] `<cli>://open?app=&screen=&params=&scope=`; one instance, a second start (with a link) going to the first
      (checked, Linux)
- [x] The old apps' other links: `<cli>://vault?id=&scope=`, `<cli>://page?scope=[&folder=accounts]`,
      `<cli>://settings[/<item>]`, `<cli>://open?scope=`, links to the address's own pages (web.js `opened`)
- [x] Closing the window leaves it running in the tray (menu bar), with Open and Quit (built; no tray host to see it
      under Xvfb)
- [x] The menu bar panel, the Mac app's, for every computer: a click on the tray item opens it under the item (over it,
      on a taskbar at the bottom), gone when it loses focus: the network's name, the client screen `panel` (your
      devices, which are on, this one; your pages' widgets each once, running and opening as on the pages), then Open,
      Open at Login (✓), Quit (checked, Linux, opened by its command: Linux's tray items show only their menu)
- [ ] Per-widget menu bar items (Mac), "Let Agents Use This Mac" in the panel (Mac)
- [ ] Window size and place kept (tauri-plugin-window-state, built, unchecked); sidebar state (web's, kept)
- [ ] Keyboard shortcuts of the old apps
- [x] Offline: last window shown at once, "Can't Reach Your Devices", trying again; while the person's devices don't
      answer, one quiet banner for the whole window (Andre): "Offline", "Showing what this device kept from <time>.",
      Try Now; across the top on a computer, the content's top on a phone; gone by itself once they answer (checked,
      Linux, the throwaway's node stopped and started; the app's pages are now kept at all, which tauri:// had refused)
- [ ] Speed: usable within 1.5 s of a cold open (ARCHITECTURE.md). Linux, a debug build under Xvfb (software
      rendering) with its keyring daemon starting first: window 1.0 s, the sidebar as it was 1.3 s, the home screen
      1.65 s. Still to measure: release builds, on each platform's real device
- [x] Phones' home is the pages side by side with dots, as the old phone apps' (Andre's decision): no sidebar; what's
      opened from them has a bar with Back (web.js `phone`, @one/ui Shell `phone`; checked on Linux as an iPhone with
      `ONE_PLATFORM=iPhone`, a debug build's)
- [x] A custom build serving one network (config.toml `address`) never asks where
- [x] Icons and names from config.toml (custom builds): product name, identifier `app.<cli>.<platform>`, app icon
- [x] Every page's widgets kept for the platform's own widgets (`src/widgets.rs`: `widgets.json` in the iPhone
      widget's format, `icons.json`, `images/`), again on each change and every five minutes; signed out, they say
      to sign in (checked, Linux: Everyone's three widgets with their sizes and icons)

Linux (new):

- [x] In the launcher with its icon, what opens `<cli>://` links, and open at login (on from the first start, a tray
      switch), each written where XDG says (checked in the throwaway's own folders)
- [x] `one release` builds it (`<cli>-app-<target>`, signed with the release key) and install.sh puts it on a desktop
      and starts it; no `one update` step, as no device of ours is a Linux desktop
- [ ] Notifications: Linux has no push service, so notes show only while the app runs (from the node's events)

Windows (the old app is gone; this is its package, MSIX, cross-built on Linux and packaged with a C# helper in the
Windows VM by `windows/build.sh`; `one throwaway client windows`). The package keeps the old app's identity
(`TheOneApp_ah4xyz62b3km4`, `windows_package()`), so it installs over it in place and keeps the sign-in:

- [ ] Toasts from pushes (WNS through the helper, `--open-note`, `--register-push`), Approve / Not Me with no window
      (`--answer-join`): built as the old app's, not checked: The One App has no WNS app (`wns_app`/`wns_object` empty)
- [x] A link to the address while signed out (`<address>/invite#…`, which the address's web-links file hands the app)
      is taken as if typed on the welcome, as the old app took it: opened in the browser for its passkey, then "Made
      your passkey? Sign in with it." (web.js `opened`, e768aea5; not checked on Windows: the throwaway's address is http)

Mac (`mac/build.sh`, the old Swift app's bundle, in the same identity: `app.<cli>.mac`, the Developer ID team,
the same designated requirement, so it reads the old app's Keychain items and defaults; `one throwaway client mac`, a
build with its own id, folder and Keychain names beside the person's app, on the Mac `MAC` names to ssh):

- [x] The bundle as before: `Contents/MacOS/<NAME>` (the window), `<cli>`, `<cli>d`, `<cli>-<service>`, the launch agent
      `app.<cli>.mac.oned`, `Contents/Helpers` (service/mac-helpers.sh), Info.plist's `CLI`, `Yours`, `NodePort`,
      `BuildCommit`, `CFBundleVersion` (commits), signed with the Developer ID and the hardened runtime, notarized by
      `<cli>-notary` with `--release`; `one update` and `one release` build it (update.rs, `client/native/disk.sh`),
      install.sh as before (checked: built on the Mac, `codesign -dr` the same as the old app's, verified)
- [x] Signed in, in the throwaway, as linux() is; a new build stays signed in (its Keychain items, by its signature)
      (checked, the Mac)
- [x] Screens reach none of the app's commands in WKWebView: a deployed screen has no `__TAURI_INTERNALS__` or
      `__oneNative`, its posts to `webkit.messageHandlers.ipc` run nothing (the clipboard stayed), its IPC fetch is
      refused (no invoke key), the parent page is out of reach; the page's own copy works (checked, the Mac)
- [x] The panel's rows: Open, Open at Login (SMAppService's main app, on from the first start), Let Agents Use This
      Mac (the app's defaults' `computerUse`, which Computer Use's helper reads), Quit; the same in the item's right-click
      menu (checked, the Mac: the rows, and the switch writing `computerUse`)
- [x] Per-widget menu bar items: one per widget kind, its headline as title (≤40), hidden when empty; a click shows
      it under the item (the panel's page with `widget`), a right click Open <app> / Hide from Menu Bar, which hides it
      (the old app's Hide never did); the main item's right-click menu shows a hidden one again (checked, the Mac: Repos'
      item titled "No repos yet."). Each keeps its place in the menu bar (`widget.<kind>`, the old app's autosaveName),
      and the panel's widget cards have a pin that shows a hidden one again or hides it (built, the Mac's build checked;
      not seen: the Mac was locked)
- [ ] The dock's count (Tauri's badge), the window, panel and widget popovers drawn: not yet seen (the Mac was locked)
- [x] The old app's sign-in taken over (`mac::take_over`, the Keychain's same items): its address, sign-in client, Open
      at Login and hidden widgets from its defaults; no new sign-in, the same device key (checked on the real Mac at the
      cut-over, a0950322: the key's Keychain item made by the Swift app on 2026-11-04, the widgets kept, Repos' item
      still hidden)
- [x] This Mac's node kept running (its launch agent registered and kickstarted, or a child) (checked on the real Mac:
      `app.<cli>.mac.oned` running from the new bundle); it approves the window's code at once (`approve_here`): not
      seen, as the Mac's window was let in already
- [ ] Join with a Pairing Code for a Mac whose node is in no network (the welcome's second button, `pair`): built, not
      checked (no Mac without a network to try it on)
- [ ] Keyboard shortcuts, as the app menu's items, so a key pressed in a screen's frame works too: View › Fold Sidebar
      (⌘\), File › Pair a Computer… (⌘⇧I, signed in: Settings › Devices with Add Device open), File › Join with a Pairing
      Code… (⌘⇧J, while this Mac's node is in no network); the Edit menu's, ⌘W and ⌘Q are macOS's own (built, the Mac's
      build checked; not seen: the Mac was locked)
- Not carried: a Mac build with push (its notification extension): no custom build has one; notes show on a Mac only
  as they did, with none

iPhone (`iphone/`: its Xcode project from `project.yml`, built by `iphone/build.sh` for the simulator, the paired
iPhone (`one update`) or TestFlight; the Swift in `Sources/` a Tauri plugin of the app's own, linked by build.rs; the
home-screen widget in `Widget/`, the notification extension in `Notifications/`; `one throwaway client iphone`, a
simulator of its own on the Mac). The old app's identity: `app.<cli>.phone`, `.widgets`, `.notifications`, the App
Group `group.app.<cli>.phone`, which is also its Keychain group:

- [x] Signed in and let in, in the throwaway: the welcome, then the pages side by side with dots, an app's screen with
      a Back bar (checked, the simulator: Repos opened by `<cli>://open?app=repos`)
- [x] Screens reach none of the app's commands: no `__TAURI_INTERNALS__` or `__oneNative` in a frame, no
      `oneHeld` (touch and hold's handler, in a world of its own), the page out of reach, its IPC fetch refused, a post
      to WebKit's `ipc` handler runs nothing (checked, the simulator, a frame of `frame.html`)
- [x] The widgets kept in the App Group (`widgets.json`, `icons.json`, `images/`) for the home-screen widget, which
      reads them as before (checked, the simulator: the files; the widget itself not added, which needs a touch)
- [ ] Sign-in in the system's sheet (ASWebAuthenticationSession, Safari's passkeys, back at `<cli>://signed-in/`):
      built; the throwaway signs in by the loopback instead, so not seen
- [x] The old app's sign-in taken over (`apple::take_over`, the same Keychain items in the group): no new sign-in
      (checked on the paired iPhone, installed over the Swift app: its address, sign-in client and client folder taken
      over, its refresh token refreshed, back in as the same device with no request to approve)
- [x] `one update` builds it (`iphone/build.sh device`) and installs it on the paired iPhone, as the old app's (built
      and installed by hand on the Mac the first time, the Mac's node being away; Release, signed by the team)
- [ ] Touch and hold lifts an icon or widget on the pages with its menu, and moving the finger carries it (`Lift`,
      `__oneLifts`, `__oneDrag`); haptics; the document camera (`one.scan`); the share sheet and Quick Look for
      `one.save` / `one.preview`: built; not tried (the simulator takes no touches from here, and has no camera)
- [ ] Notification taps open their screen, Approve / Not Me answer a device asking to join (as the device), the
      icon's count, a banner while it's open: built; not checked, as no build has push yet (below)
- [ ] Push (`push = true` in config.toml, or PUSH=1): the notification extension opens each note with a device key from
      the Keychain group; the token registered with the push Worker, topic `app.<cli>.phone`: built, not checked. It
      waits on the App IDs' Push capability and an APNs key for the network's push Worker (Andre, top #87)
- [ ] TestFlight (`iphone/build.sh testflight`, App Store Connect's API key; build number `<main's commits>.<config
      repo's commits>`, rising with either): built, not run. It waits on an App Store Connect app for the custom
      build's bundle id (PTC's, `app.ptc.phone`: Andre, top #87); then Google sign-in, receipt scan and photo upload in
      Claims to check there
- Not carried: the iPhone's own menus for
  `one.menu` (the web's, as Andre decided), but for what touch and hold lifts

Android (`android/`: its own Gradle project, built by `android/build.sh` with the app's Rust for each processor and the
Kotlin of the old app that has to be Kotlin, in `android/app/src/main/kotlin/app/client`: `OnePlugin` is the Tauri
plugin of the app's own, `Host` and `Native` what the app's library and the system call each other by, through JNI, with
no window; `one throwaway client android`, a copy of the person's Android Agent Device, with `--as sam`). The old app's
identity: `app.<cli>.android`, signed with the debug key it was signed with, so it installs over it. No `one update` or
release builds it (nothing did): only a throwaway, as before. Checked on an emulator (Android 16, Play Store image):

- [x] Signed in and let in, as the person typed the address: the welcome, Sign In opens a Chrome Custom Tab, which comes
      back to `<cli>://signed-in/`, then Waiting for Approval and the pages side by side with dots (checked: typed
      address, the Custom Tab, the code approved with `one throwaway devices approve`); and as the throwaway client signs
      it in, printing the link rather than opening the tab (`SIGN_IN_PRINT`, `CONTROL` and the address come as the start
      intent's `env.<NAME>` extras, kept for later starts, in a debug build only). Not as Sam: `client android --as sam`
      and `client web --as sam` both stopped at the web client's step ("the web client wasn't let in": Sam's browser,
      not the app)
- [x] The Kotlin app's sign-in taken over (`Host.takeOver`): its address, sign-in client, who's signed in and refresh token
      move to this app's names; its device keys stay where they were (`sign-in.xml`, sealed by the Keystore alias
      `sign-in`), so no new sign-in and no new device (checked on a copy of the person's phone that had the Kotlin app
      signed in to an earlier throwaway; not on a real phone: none of ours has it)
- [x] Pages, screens, menus: an app's screen opened from the pages, its bar with Back, an app's menu (checked); alerts
      and sheets are the web's, as everywhere (not opened here)
- [x] Screens reach none of the app's commands: Android gives a sandboxed frame Tauri's script and `ipc` (a probe screen
      launched Chrome through `ipc.postMessage`), so every command carries a token only the page's own frame is given by
      a web message listener (`oneBridge`, `MainActivity.kt`), which Rust checks (`android::ours`): the same probe
      launches nothing, and an invoke without it is refused (checked, a deployed probe screen)
- [x] Links: `<cli>://open?app=&screen=&params=&scope=` opens the screen (checked); the mobile deep-link config names the
      scheme and https hosts (the address, `LINKS`); App Links on a build with an https address: built, not checked (the
      throwaway's address is http), the address's `assetlinks.json` lists the debug key's fingerprint as before
- [x] Hardware Back: closes what's over the page (a menu, a sheet, an alert), then goes back, at the home leaves the app
      (checked: from a screen, and at the home; a menu open, by the page's `hardwareBack`)
- [x] The scanner (`one.scan()`): Google's document scanner, each page a JPEG as the iPhone's (checked: a page scanned,
      one file returned); `one.can.scan` follows Play services and 1.7 GB of memory
- [x] Save (the share sheet) and preview (ACTION_VIEW; Photos opened a PNG), copy, links outside (a Custom Tab for the
      network's own pages) (checked: save and preview; the Custom Tab by signing in)
- [x] Home-screen widgets: the old app's two, placed in the copy, draw from the app's files (`widgets/`), neutral, app
      tiles as gradient tiles; a button and a toggle (a probe app's) run their tool in the app's library and redraw;
      signed out they say to sign in; touched and held they choose from the pages' widgets (`WidgetChooser`: built;
      not opened); the 15-minute refresh (WorkManager) through the same library: built, not waited for
- [x] Taken out (removed): signed out, "This device was removed. Sign in again.", the widgets say to sign in (checked)
- [x] Text size: Settings' font size zooms the window, screens' frames with it, never narrower than 320 pixels, also while
      the app runs (`about().textScale`, `native.onScale`; web.js `zoomed`); the iPhone's Dynamic Type through the same
      (`iphone::text_scale`: built, not run: the Mac wasn't reachable)
- [x] Edge to edge, the system bars' icons follow light or dark, the keyboard resizes the page (checked: the keyboard
      over the welcome's field)
- [ ] Push: FCM through the custom build's Firebase project (`firebase` in config.toml, empty for The One App): the token
      registered with the push Worker, the note opened with the device's key by the app's library (`open-note`), its
      Approve and Not Me answered with no window (`join`), the permission asked once: built, not checked (no Firebase
      project)
- [ ] Haptics (`one.tap`): called, no error; not felt (an emulator)
- [x] Release build (`android/build.sh release`: Rust optimized, the APK shrunk, signed with the debug key): installed over the
      debug build, starts and shows the welcome, its Keystore and JNI calls through the shrinking (checked)
- Not carried: the phone's own accent colour on what's tapped (the web's look has none, as Andre decided); the old app's
  a sign-in that comes back after the app was killed while the Custom Tab was open (it starts again, as the old app's kept
  its half-made one)
