# MyGo spike

An experiment (branch `mygo-spike`, never main): whether [MyGo](https://github.com/egoist/mygo) can be the client shell
on Linux, macOS and Windows, with the Rust client (client/ffi) called from Go.

- `client/`: client/ffi from Go. Its API doesn't say how the library is reached: purego on computers
  (`CGO_ENABLED=0`), or a cgo loader for an iPhone (`-tags cgoffi`, which fills the same `lib` table).
- `signin/`: signing in as an OAuth client of the install's sign-in, with a loopback redirect.
- `bridge/`: src/screen/bridge.js's messages answered, given a platform's `Shell`.
- `desktop/`: the MyGo app.

Needs the library built: `cargo build -p one-client-ffi` (it's found in the checkout's target/debug, beside the
program, or at `ONE_CLIENT_LIB`).

## What it showed (2026-11-05, against `one throwaway`)

Running it: `go build -tags e2e ./desktop` (the tag adds MyGo's automation socket, `MYGO_AUTOMATION=<path>`), then
`ONE_ADDRESS=<throwaway address> ONE_SIGN_IN_HERE=1 ./desktop` (a throwaway's sign-in needs no passkey, so it signs in
in a window of its own). `-forget` removes what it keeps in the secret store.

- **The client from Go, no cgo**: purego passes and returns uniffi's 24-byte RustBuffer by value on linux/amd64,
  darwin/arm64 and windows/amd64 (purego 0.11.1); no C shim was needed. The binding is hand-written (~600 lines,
  `client/`), since uniffi has no Go generator without cgo.
- **Linux** (WebKitGTK 4.1, under Xvfb): signs in, keeps its key in the Secret Service, is let in, lists the apps,
  opens Agent Board's screen, and answers call, kept, icons, icon, title, copy, link, menu, alert, open (a sheet, with
  its value back on close), close, notify (with a notification daemon) and node events. Screens are refused the
  plugin and other apps' origins.
- **Windows** (WebView2, a plain .exe): the same, with the native dialog and menu; the DLL cross-built with
  x86_64-pc-windows-gnu as windows/build.sh does.
- **macOS** (WKWebView, an ad-hoc signed .app): signs in, Keychain, let in, apps, Agent Board, call, title, copy,
  icons. A rebuilt ad-hoc app makes macOS ask before it reads its earlier Keychain items.
- **Gaps**: a passkey (ASAuthorizationController reached through purego's objc; macOS refuses: "the calling process
  does not have an application identifier") and APNs (refused, OSStatus 13) both need the real signing (team ID,
  provisioning profile, associated domains, aps-environment), which `mygo build`'s `macos.entitlements` and
  `macos.provisioningProfile` take. Windows: webauthn.dll v9 is callable by syscall; WNS needs package identity
  (MSIX), which a MyGo .exe hasn't. The tray item shows everywhere. MyGo's menus and dialogs draw no red "danger".
- **Widgets**: MyGo packages no app extensions (resources go to Contents/Resources); apple/Widget's .appex would be
  copied into Contents/PlugIns after `mygo build` and the app signed again with the App Group, and reloading
  timelines (WidgetCenter is Swift-only) would need a small Swift helper.

## iPhone (`ios/`)

Its own module, on [MyGo PR #940](https://github.com/egoist/mygo/pull/940) (iOS and Android), with the core above:
`client/` through its cgo loader (`-tags cgoffi`), the library linked in as a `.syso`. `ios/build.sh simulator` on a
Mac (Go, Rust's iOS targets, MyGo's CLI from that PR; `GEN=<folder bindings.sh made>` saves building the client for the
Mac). What MyGo can't do yet, and the spike works around:

- No extension targets: `notifications/` and `widget/` are built with swiftc and signed by their own scripts, then put
  in the app by `mygo.json`'s `resources` (`build/PlugIns` lands in the bundle as `PlugIns/`), before MyGo signs it.
  Upstream would want `ios.extensions` (sources, Info.plist, entitlements, its own profile on devices) built and signed
  inside the bundle before the app.
- A c-archive carries no `#cgo LDFLAGS`: the frameworks the Rust client needs are named by a plugin
  (`shell/mygo-plugin.json`). Upstream: link an `ios.ldflags`, or the packages' cgo flags.
- Simulator builds have no `application-identifier` or Keychain group, so `mygo.Secrets` fails (-34018) until a
  plugin's entitlements give them, as Xcode always does.
- Entitlements a plugin names are a simulator's as given, a device's only if its profile has them: App Groups, push and
  associated domains need an App ID with those (only the wildcard profile exists for `app.one.mygo`).
