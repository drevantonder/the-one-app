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
- [x] Offline: last window shown at once, "Can't Reach Your Devices", trying again; a screen's kept page shown at once
      with "Offline · from …" and Try Now when its devices don't answer, gone once they do (checked, Linux, the
      throwaway's node stopped and started; the app's pages are now kept at all, which tauri:// had refused)
- [ ] Speed: usable within 1.5 s of a cold open (ARCHITECTURE.md). Linux, a debug build under Xvfb (software
      rendering) with its keyring daemon starting first: window 1.0 s, the sidebar as it was 1.3 s, the home screen
      1.65 s. Still to measure: release builds, on each platform's real device
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

Mac (`apple/Mac`, `apple/Shared`), iPhone (`apple/Phone`, `Notifications`, `Widget`), Windows (`windows/`), Android
(`android/`): each old app's full inventory lands here before that platform is written.
