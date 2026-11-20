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

A Mac's app is its bundle, `macos/` with the program as `Contents/MacOS/one`, which `tooling/release` signs in it. It
goes in `/Applications`, and `~/.local/bin/one` is a link to the program in it, so the CLI, a server and `one update`
all run and replace that one program.

## On Windows

Windows' app is installed by its installer, `installer.nsi`, for its user alone, with no administrator: `one.exe` in
`%LOCALAPPDATA%\Programs\the One App`, on the user's PATH, in the Start menu and in Settings > Apps. A person
downloads it from their network (`one download --target windows-x86_64`'s `one-setup`), and the app updates itself
from then on.

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

## Its page from the network

A phone's window's page (`page/`) is built into its app (a computer's windows are all native), and its network's edge serves it too, at `<address>/page/`, from
the commit the edge was deployed from (`core/edge/edge.js`). As the app opens, it asks for that, keeps a newer one it
runs on, and loads it from its next start, as Expo Updates do by default (`src/page.rs`); without one, or with no
network, it loads its own. So a change to the page goes live with an edge deploy, with no build of the app. It takes one only signed by the platform's key, whose public half is built in, checked again each start: the
deploy signs it with `PAGE_SIGNING_KEY` (read from 1Password's "The One App page signing key" when unset), and refuses
without a key. `page/update.json` says the lowest build of the app the page runs on (`native`): a change to the page that
needs something new of the app's Rust half (a command, an event) raises it to the build that adds that, so an app
older than that keeps its own page until it's updated.
