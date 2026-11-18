# Building a network's app

There's no app of the One App's own: each network has its own, built from this folder with the settings in its
repo's `app/`, whose `README.md` gives the commands. They're Tauri's CLI's (`cargo tauri`), run here, with the
network's repo checked out beside this one. A network's app is a Tauri config of its own (`app/tauri.conf.json`)
merged over `tauri.conf.json` with `--config`, and its network's address (`app/address`) given to the build as
`TAURI_ONE_ADDRESS`. The config names its network's domain, as the host of its app links (`plugins.deep-link.mobile`:
Tauri's deep-link plugin), so the app opens those links and its web view uses that network's passkeys, as the
network's edge says it may (`apps/sign-in/edge/wrangler.jsonc`), and it may give the app a name, identifier and icon
of its own. `tooling/network` starts a network on this machine and opens an app built from here on it.

Android needs an Android SDK and its NDK (`ANDROID_HOME`, `NDK_HOME`), and iPhone a Mac with Xcode, `xcodegen` and
Rust's iOS targets, signed by an Apple team (`APPLE_DEVELOPMENT_TEAM`). Their projects are `gen/android` and
`gen/apple`. The iPhone's is `gen/apple/project.yml`: `./generate` there makes `app.xcodeproj` from it with
`xcodegen`, which the build uses as it's committed. `xcrun devicectl device install app` puts what's built in
`gen/apple/build/arm64` on a paired iPhone, and `--target aarch64-sim --debug` builds it for the simulator.

A phone's client is kept in the app's own folder, on an iPhone the one it shares with its notification extension, and
is told its network's address by a `network` file put there (`{"address": "https://..."}`), as `mix network` writes one
for a network with an address. An app built with `TAURI_ONE_ADDRESS=https://...` set, on any system, starts on that
network with no file there. It starts on that network's sign-in page.

## On a Mac

A Mac's app is its bundle, which `cargo tauri build --bundles app` makes in `target/release/bundle/macos`. It goes in
`/Applications`, and `~/.local/bin/one` is a link to the program in it, so the CLI, a server and `one update` all run
and replace that one program.

## A name, identifier and icon of its own

Tauri merges a network's config over `tauri.conf.json` as it builds:

    {"productName": "<its name>", "identifier": "<its identifier>", "bundle": {"icon": ["../../../<its repo>/app/icons/icon.png"]}}

`TAURI_CONFIG="$(cat <that file>)" cargo build --release` builds the program alone. Its identifier names the folder its
client is kept in too, so it sits beside the One App on one device. On iPhone, Tauri's CLI puts the name and
identifier in the Xcode project as it builds, and `cargo tauri icon <png>` makes its icons. `gen/apple/generate <that
file>` first makes its Xcode project, so its notification extension and App Group are named after its identifier too.
Android's project names the One App's identifier and name itself (`app/build.gradle.kts`, `strings.xml`, the folder
of `MainActivity`), so a network's Android app with an identifier of its own is made from its own
`cargo tauri android init --config <that file>` for now.
