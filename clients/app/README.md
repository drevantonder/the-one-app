# The One App

This folder builds the One App, plain: with its own name and identifier and no network in it. A network's own app is
this build, finished for it afterwards, as white-label apps are made: its name, identifier, the schemes and hosts of
its links and its address are written in, and it's signed for it. The network's settings are in its repo's `app/`, as
a Tauri config (`tauri.conf.json`: `productName`, `identifier`, `plugins.deep-link.mobile`) and its address
(`address`), and never in this repo or its GitHub runs, whose logs anyone can read. An iPhone's is finished and sent to
TestFlight by `tooling/testflight <the network's repo>`. Its link hosts are where its web view uses the network's
passkeys (`webcredentials:`) and which of its links open in it (`applinks:`), as the network's edge says it may
(`apps/sign-in/edge/wrangler.jsonc`). `tooling/network` starts a network on this machine and opens an app built from
here on it.

Android needs an Android SDK and its NDK (`ANDROID_HOME`, `NDK_HOME`), and iPhone a Mac with Xcode, `xcodegen` and
Rust's iOS targets, signed by an Apple team (`APPLE_DEVELOPMENT_TEAM`). Their projects are `gen/android` and
`gen/apple`. The iPhone's is `gen/apple/project.yml`: `./generate` there makes `app.xcodeproj` from it with
`xcodegen`, which the build uses as it's committed. GitHub's macOS runner builds it (`.github/workflows/iphone.yml`).
`xcrun devicectl device install app` puts what's built in `gen/apple/build/arm64` on a paired iPhone, and `--target
aarch64-sim --debug` builds it for the simulator.

A phone's client is kept in the app's own folder, on an iPhone the one it shares with its notification extension, and
is told its network's address by a `network` file put there (`{"address": "https://..."}`), as `mix network` writes one
for a network with an address. An app with none there starts on the network its own resources name, in a `network`
file of the same kind: beside its program, in a Mac bundle's `Contents/Resources`, or in an APK's `assets`. It starts
on that network's sign-in page.

## On a Mac

A Mac's app is its bundle, `macos/` with the program as `Contents/MacOS/one`, which `tooling/release` signs in it. It
goes in `/Applications`, and `~/.local/bin/one` is a link to the program in it, so the CLI, a server and `one update`
all run and replace that one program.

## On Windows

Windows' app is installed by its installer, `installer.nsi`, for its user alone, with no administrator: `one.exe` in
`%LOCALAPPDATA%\Programs\the One App`, on the user's PATH, in the Start menu and in Settings > Apps. A person
downloads it from their network (`one download --target windows-x86_64`'s `one-setup`), and the app updates itself
from then on.

## Its page from the network

A phone's window's page (`page/`) is built into its app (a computer's windows are all native), and its network's edge serves it too, at `<address>/page/`, from
the commit the edge was deployed from (`core/edge/edge.js`). As the app opens, it asks for that, keeps a newer one it
runs on, and loads it from its next start, as Expo Updates do by default (`src/page.rs`); without one, or with no
network, it loads its own. So a change to the page goes live with an edge deploy, with no build of the app. It takes one only signed by the platform's key, whose public half is built in, checked again each start: the
deploy signs it with `PAGE_SIGNING_KEY` (read from 1Password's "The One App page signing key" when unset), and refuses
without a key. `page/update.json` says the lowest build of the app the page runs on (`native`): a change to the page that
needs something new of the app's Rust half (a command, an event) raises it to the build that adds that, so an app
older than that keeps its own page until it's updated.
