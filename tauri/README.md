# Tauri spike

An experiment (branch `tauri-spike`, never main): whether [Tauri 2](https://v2.tauri.app) can be the client shell on
Linux, macOS, Windows and iPhone, as the MyGo spike (`mygo/` on branch `mygo-spike`) asked of MyGo. The shell is Rust,
so it uses `client/` (`one_client::Device`) directly: no FFI, no binding.

- `src/`: the shell. `lib.rs` (windows, the `screen` scheme, natives, tray, sign-in flow), `bridge.rs` (bridge.js's
  messages, as mygo/bridge), `node.rs`, `signin.rs` (OAuth with a loopback redirect, as mygo/signin), `secrets.rs`
  (keyring: Secret Service, Keychain, Credential Manager).
- `plugin/`: a Tauri plugin of our own for what only Swift does on an iPhone: menus (an action sheet), a passkey,
  push registration, the App Group.
- `note/`: the notification extension's Rust (`one_open_note`, one C function), and `seal`, which seals a note to a
  device for `xcrun simctl push`.
- `ios/`: `project.yml`, the Xcode project's template (`bundle.iOS.template`), with the notification extension and
  widget targets; their Swift.

Its own workspace, so the repo's `cargo build`/`cargo test` never build Tauri. Running it: `cargo build` here, then
`ONE_ADDRESS=<throwaway address> ONE_SIGN_IN_HERE=1 target/debug/one-tauri`; `ONE_OPEN=board` opens a screen,
`ONE_PROBE=1` logs more, `ONE_CONTROL=<port>` takes `POST /eval/<window>` (JavaScript) from an agent, `--forget` removes
what it keeps. iPhone: `cargo tauri ios init` (renders `ios/project.yml`), `cargo build -p one-tauri-note --lib --target
aarch64-apple-ios-sim`, `cargo tauri ios build -t aarch64-sim`; release needs `rustup component add llvm-tools` (below).

## What it showed (2026-11-05/06, against `one throwaway`)

Screens are untrusted pages on their own origin per app (`screen://<app>/`; `https://screen.<app>/` on Windows and
Android, where WebView2 takes custom schemes only as http(s)), each in a window no capability names. They post
bridge.js's messages to their own origin's `/one`, answered by the shell for that window's app alone; node events are
evaluated in (`one.__event`). Checked on every platform by a probe in Agent Board's screen: `__TAURI_INTERNALS__.invoke`
of the shell's commands, `plugin:event|listen`, clipboard, opener and window commands are all refused by Tauri's ACL;
another app's origin (`fetch`, iframe) is refused by the handler, which matches the request's host to the window's
app; own-only messages (`api`, `signOut`, `cache`) are refused. The shell's own commands are listed in `build.rs`, since
unlisted app commands are allowed to every window.

- **Linux** (WebKitGTK 4.1, Xvfb): signs in (in a window of its own), Secret Service, let in, 23 apps, Agent Board;
  call, kept, icons, icon, title, copy (read back from the clipboard), link (reaches xdg-open), menu (GTK, the chosen
  index back), alert (GTK dialog, true/false back), open as a sheet with its value back on close, notify (seen in
  dunst's history), node events (a CLI post shows at once). Tray made (no tray host under Xvfb to see it).
- **macOS** (WKWebView, an ad-hoc signed .app from `cargo tauri build`): Keychain, let in, apps, Agent Board, the
  probes, node events, the notification banner. Menu/alert not tried there (Andre was using the Mac).
- **Windows** (WebView2 156, a plain .exe cross-built with x86_64-pc-windows-gnu, beside WebView2Loader.dll):
  Credential Manager, let in by its sign-in, apps, Agent Board, every probe; Win32 menu (index back), dialog parented
  to the screen (Cancel → false), sheet with its value, node events, tray icon in the overflow. webauthn.dll answers API
  version 9 from Rust (the `windows` crate). Over http, screens weren't a secure context (no `crypto.subtle`):
  `use_https_scheme` fixed it.
- **iPhone** (iOS 27 Simulator, Xcode 27): Tauri's generated XcodeGen project; signs in (a second window), Keychain,
  let in, apps, Agent Board (a second UIWindow over the home page), every probe, node events. Menus: Tauri has none on
  mobile, so the plugin's action sheet (danger in red as destructive). Alerts: tauri-plugin-dialog. App Group: read
  and written by the app. Passkey from the plugin: error 1004, "Application with identifier X96JC3HCV7.app.one.tauri is
  not associated with domain one.thevantonders.com" (no apple-app-site-association; expected). Push: provisional
  authorization (no prompt) and an APNs token (the plugin adds the delegate's token callbacks to tao's app delegate,
  which has none); the push Worker refused it: "topic is the app's bundle id" (this build's isn't the network's).
  Notification extension and widget: built as targets of the generated project, embedded in PlugIns/, registered
  (pluginkit lists both). A note sealed to the device and sent by `simctl push` arrived (quietly, provisional) but the
  extension didn't run, foreground or background, so opening one end to end is unproven: it needs a device, and the
  App ID below.

### Keeping the Xcode project ours

`tauri ios build` never regenerates the project: it builds the pbxproj, patching the `_iOS` target's team, bundle id
and name. `tauri ios init` renders it again from `bundle.iOS.template`, which here is `ios/project.yml`: deleted
`gen/apple` and re-init'd twice, the extensions, their entitlements (App Group, Keychain group, aps-environment) and
the SystemConfiguration link came back each time. So nothing generated needs committing.

### What Tauri would need upstream

- Menus and a native "danger" style on mobile (muda is desktop-only); destructive buttons in tauri-plugin-dialog (a
  danger alert is only a warning icon) and muda menus.
- A way for plugins to receive app-delegate callbacks (push tokens) without swizzling.
- iOS: closing or stacking windows (a pushed screen has no way back but the shell's own UI); sheets.
- swift-rs on Xcode 27: without `llvm-objcopy` (`rustup component add llvm-tools`) a plugin's `@_cdecl` symbol stays
  internal and release builds fail to link (debug builds don't).
- A `cdylib` crate type (Android's) fails to link on iOS once a Swift plugin is in: the template's
  `["staticlib", "cdylib", "rlib"]` has to drop it.
- Windows: MSIX (package identity, for WNS); Tauri bundles MSI and NSIS only. Linux needs the -dev packages
  (libwebkit2gtk-4.1-dev …) to build.
- macOS notifications report "granted" without asking.

## Numbers

| | Tauri | MyGo | Today |
| --- | --- | --- | --- |
| Linux binary (release, stripped) | 36 MB | 12.4 MB + client .so | — |
| Windows .exe | 41 MB + 160 KB loader (gnu) | — | — |
| macOS .app | 25 MB | — | — |
| iPhone .app (sim, release) | 31 MB (extension 1.2 MB, widget 84 KB) | — | — |
| Linux, signed in: apps listed / board shown | ~330 ms / ~700 ms | 146 ms / +82 ms | — |
| Linux memory (PSS, home + board) | 260 MB | 213 MB | — |
| macOS: apps / board | 637 ms / +68 ms; shell 45 MB + web processes ~105 MB | — | — |
| Windows: shell ready / apps | 295 ms / 19 s* | — | — |
| iPhone sim: apps / board | 2.5–3.6 s / +190–230 ms | — | — |

\* Each request from the Windows VM through `agent-devices reach` takes ~530 ms (4 ms on Linux), so Windows' network
time is the VM's forwarding, not the shell. The Linux runs are Xvfb, software rendering, with
`WEBKIT_DISABLE_DMABUF_RENDERER=1` for both (MyGo sets it; without it Tauri used 487 MB). MyGo's client library there was
a debug build of client/ffi.

Lines (no blanks or comments): Tauri 1113 Rust (shell) + 146 (plugin, Rust and Swift) + 27 (note) + 74 Swift
(extensions) + 36 (home page). Today: apple/ 7284 Swift, windows/ 3707 C#, android/ 3649 Kotlin. MyGo: 2344 Go (desktop,
iPhone, client binding, sign-in, bridge).
