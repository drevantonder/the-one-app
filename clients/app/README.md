# Building the app for phones

The same app builds for Android with Tauri's CLI (`cargo install tauri-cli`), an Android SDK and its NDK:

    ANDROID_HOME=~/Android/Sdk NDK_HOME=~/Android/Sdk/ndk/<version> cargo tauri android build --debug --apk

And for iPhone on a Mac, with Xcode, `xcodegen` and Rust's iOS targets, signed by an Apple team:

    APPLE_DEVELOPMENT_TEAM=<team> cargo tauri ios build --export-method debugging

Their projects are `gen/android` and `gen/apple`. The iPhone's is `gen/apple/project.yml`: `./generate` there makes
`app.xcodeproj` from it with `xcodegen`, which the build uses as it's committed. `xcrun devicectl device install app`
puts what's built in `gen/apple/build/arm64` on a paired iPhone, and `--target aarch64-sim --debug` builds it for the
simulator.

A phone's client is kept in the app's own folder, on an iPhone the one it shares with its notification extension, and
is told its network's address by a `network` file put there (`{"address": "https://..."}`), as `mix network` writes one
for a network with an address. An app built with `TAURI_ONE_ADDRESS=https://...` set, on any system, starts on that
network with no file there. It starts on that network's sign-in page.

## On a Mac

A Mac's app is its bundle, which `cargo tauri build --bundles app` makes in `target/release/bundle/macos`. It goes in
`/Applications`, and `~/.local/bin/one` is a link to the program in it, so the CLI, a server and `one update` all run
and replace that one program.

## A workplace's own app

A workplace's own app, with its own name, identifier and icon, is a Tauri config of its own merged over
`tauri.conf.json`, as Tauri merges one:

    {"productName": "The PTC App", "identifier": "app.ptcau.client", "bundle": {"icon": ["/path/to/icon.png"]}}

`cargo tauri build --config <that file>` builds it, and `TAURI_CONFIG="$(cat <that file>)" cargo build --release` the
program alone. Its identifier names the folder its client is kept in too, so it sits beside The One App on one device.
On iPhone, Tauri's CLI puts the name and identifier in the Xcode project as it builds, and `cargo tauri icon <png>`
makes its icons. `gen/apple/generate <that file>` first makes its Xcode project, so its notification extension and App
Group are named after its identifier too. Android's project names The One App's identifier and name itself
(`app/build.gradle.kts`, `strings.xml`, the folder of `MainActivity`), so a workplace's Android app is made from its
own `cargo tauri android init` for now.
