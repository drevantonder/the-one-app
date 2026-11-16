---
name: iphone
description: Use to build the app for iPhone or the iPhone simulator and put it on the owner's phone, from Linux over ssh to the Mac, without the owner doing anything.
---

Everything iPhone happens on the Mac: `ssh mac 'zsh -lc "<command>"'` (a login shell, so Homebrew, Rust and Xcode's
tools are on its PATH). Work in a checkout of your own there under `~/scratch/`, at your commit, with the network's
repo beside it. An app is a network's: the commands that build it are in that repo's `app/README.md`, and what they
need in `clients/app/README.md`. This is what neither says.

## What exists

- No Apple account is signed in to Xcode, so `xcodebuild -allowProvisioningUpdates` and `cargo tauri ios build`'s
  own signing and export fail (`errSecInternalComponent`).
- Signing uses the Apple Development identity in a keychain of its own, not the login keychain, and the team's
  wildcard profile ("iOS Team Provisioning Profile: *", in
  `~/Library/Developer/Xcode/UserData/Provisioning Profiles/`). A wildcard profile carries no push and no App Group,
  so such a build has no notifications, and the app keeps its client in its own folder instead of the group's.
- The phone is paired with the Mac: `xcrun devicectl list devices` names it.

## Building and signing

1. Turn signing off in your checkout: `CODE_SIGNING_ALLOWED: NO` in `gen/apple/project.yml`'s settings for both
   targets, `app_iOS` and `Notifications`, then `gen/apple/generate` (with the network's config if it has one).
   An `.xcconfig` doesn't do it: `cargo tauri ios build` takes none. Then the network's iPhone build command: its
   archive succeeds and its export fails (`errSecInternalComponent`), and the app is
   `gen/apple/build/app_iOS.xcarchive/Products/Applications/<name>.app`. Or build for the simulator with
   `--target aarch64-sim --debug`, which needs no signing. This is how the last build on the phone was made, on
   16 November 2026, from `clients/shell` before it was `clients/app` (whose archive was `shell_iOS.xcarchive`); not
   yet run since.
2. Put the wildcard profile in the app as `embedded.mobileprovision`, and sign with entitlements that hold only
   `application-identifier` (`<team>.<the app's identifier>`), `com.apple.developer.team-identifier` and `get-task-allow`.
   Remove the notification extension (`PlugIns/`) first: it can't be signed without the App Group.
3. Unlock the keychain and `codesign` in the same ssh connection: `security -i` reads `unlock-keychain -p ...` on
   stdin, then `codesign --force --keychain <it> --sign "Apple Development" --entitlements <file> <app>`. The
   password goes on stdin, never in a command line, where `ps` and logs see it. The key's partition list must
   already allow `codesign` (`security set-key-partition-list -S apple-tool:,apple:`); if it doesn't, a dialog
   appears on the owner's screen.
4. `xcrun devicectl device install app --device <id> <app>`, then
   `xcrun devicectl device process launch --device <id> <the app's identifier>`.

The keychain's password is in the owner's password manager; ask the orchestrator or Agent Memory for the item.

## Mistakes already made

- Signing through anything running in the owner's logged-in session (Xcode, a GUI `osascript`, a Terminal tab):
  each signature put a password dialog on his screen. Only ssh, and only a keychain unlocked in that connection.
- Leaving signing to `cargo tauri ios build` or Xcode's export, which fail with no account signed in.
