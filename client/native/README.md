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

Until each platform's old app (`apple/`, `windows/`, `android/`) is gone, this is the list of what it does that this
must too, ticked as it does and checked in `one throwaway`. Then this section goes, down to what the code can't say.

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

Windows (`windows/`, replaced by this and `windows/` here: the package, MSIX, cross-built on Linux and packaged with
a C# helper in the Windows VM by `windows/build.sh`; `one throwaway client windows`):

- [x] The same package as the old app's, which installs over it in place: `TheOneApp`, `CN=The One App`, family
      `TheOneApp_ah4xyz62b3km4` (`windows_package()`), protocol, app-link host, widgets' COM class and its four
      definitions, the toast activator, names; its program is this one (checked, Windows: installed over 1.305, the
      board's three pinned widgets came back drawn by the new helper)
- [x] Runs as the package (started by AUMID or by its path), so its folder is the package's own
      (`LocalCache\Local\app.<cli>.windows`) and the badge, startup task and widgets have its identity (checked)
- [x] Signs in, is let in by code, pages and screens; a new build keeps the sign-in; signing out takes the device out
      (checked, Windows, `one throwaway client windows`)
- [x] Named "<Name> on <COMPUTERNAME>"; `on` is the MachineGuid hashed, as the old app's (named: checked; replacing
      an earlier sign-in: not checked)
- [x] Screens reach none of the app's commands. WebView2 puts Tauri's internals and `chrome.webview` in every frame,
      but a sandboxed frame's invoke is refused ("Origin header is not a valid URL"), its raw postMessage of `copy` runs
      nothing (the clipboard kept its text, where the page's own `copy` lands), and the parent is unreachable (checked)
- [x] `one://open?…` from outside goes to the running app (one process), a second start shows the hidden window
      (checked)
- [x] Closing leaves it in the tray; the tray item opens the panel; Open at Login is the package's startup task, on
      from the first start (checked)
- [x] Title bar (OS decorations) and page follow Windows' light and dark (checked)
- [x] Taskbar count badge (BadgeUpdateManager): every badge the sidebar draws, totalled by web.js, said on the
      window's first draw and on each change; 0 clears it, as sign-out does (checked: a badge set by hand shows, and
      the next draw clears it)
- [x] Widgets board: the helper draws each pinned widget from the app's widget cache with the old cards and words,
      Customize (Choose a Widget, "<Title> · <Page>", Done, Cancel), a toggle and a button run the widget's tool
      (`--call`), drawn at once then as it left them, a failing tool says "Couldn't do that: <why>", it reads again
      with `--widgets` when shown and every 5 minutes, and redraws when the app writes the cache; signed out, "Sign in
      to <Name> to see your pages." (checked, a test app's toggle and button). While the app runs the helper's calls
      are answered by it, over its loopback port (`headless.rs`): a button's tool answered in 60-80 ms, where a client
      started each time took ~18 s; a fresh client draws its widgets at once (checked, Windows, `one throwaway client
      windows`: Repos' "No repos yet." on the first start, and four `--widgets` at once leave no error)
- [ ] Toasts from pushes (WNS through the helper, `--open-note`, `--register-push`), Approve / Not Me with no window
      (`--answer-join`): built as the old app's, not checked: The One App has no WNS app (`wns_app`/`wns_object` empty)
- [x] Links to the address (`https://<address>/…`, the AppUriHandler): the package names the config's host, and the app
      makes the address it signs in to its one app-link host (`claim`, at start and when an address is taken), so
      Windows hands it those links, which open the screen they name (checked, Windows: a start of the program with
      `<address>/app/repos/main` from the desktop's session opened Repos; an https address got Windows' own record of
      the host for the package; the throwaway's address is http, which Windows takes no app links for)

Mac (`apple/Mac`, `apple/Shared`), iPhone (`apple/Phone`, `Notifications`, `Widget`), Android (`android/`): each old
app's full inventory lands here before that platform is written.
