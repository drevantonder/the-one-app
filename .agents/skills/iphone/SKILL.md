---
name: iphone
description: Use to build the app for iPhone or the iPhone simulator and put it on the owner's phone, from Linux over ssh to the Mac, without the owner doing anything.
---

Everything iPhone happens on the Mac: `ssh mac 'zsh -lc "<command>"'` (a login shell, so Homebrew, Rust and Xcode's
tools are on its PATH). Work in a checkout of your own there under `~/scratch/`, at your commit, with the network's
repo beside it (only its `app/` is read; copying that folder over is enough). Clone it whole, not `--depth 1`: the
app's version and build number are `tooling/version`'s count of commits, so a shallow clone builds an app numbered 1.
The Mac is the owner's own and shared by every agent: tonight several agents' simulators and builds took its load past
700, so builds timed out and a server update on it went back. Before booting a simulator or building, run `uptime`
there; above about 20, don't add to it: tell your orchestrator. Use one simulator, reuse one that's booted rather than
booting another, and shut down and delete yours and remove your `~/scratch/` checkout when you're done.

An app is a network's: the commands that build it are in that repo's `app/README.md`, and what they need in
`clients/app/README.md`. This is what neither says.

## What exists

- No Apple account is signed in to Xcode, so `xcodebuild -allowProvisioningUpdates` and `cargo tauri ios build`'s
  own signing and export fail (`errSecInternalComponent`).
- Signing uses the Apple Development identity in a keychain of its own, not the login keychain, and a development
  profile of the app's own App ID, in `~/Library/Developer/Xcode/UserData/Provisioning Profiles/` (named by UUID:
  `security cms -D -i <file>` shows each one's `Name`, App ID and entitlements). The App IDs (the app's identifier,
  and its `.notifications`) have Push Notifications and App Groups on. The wildcard profile ("iOS Team Provisioning
  Profile: *") carries neither, so a build signed with it isn't woken by notifications.
- The team's App Store Connect API key (Admin) makes App IDs and profiles with the API itself: `POST /v1/bundleIds`,
  `/v1/bundleIdCapabilities` and `/v1/profiles` (`IOS_APP_DEVELOPMENT`, the team's `DEVELOPMENT` certificate and the
  phone from `/v1/devices`), whose `profileContent` is the profile. A profile is made again when its App ID changes,
  which leaves it invalid. The key's item is in Agent Memory.
- That key can't make an App Group or put one on an App ID: the API has nothing for it, and Xcode's own developer
  services, which `xcodebuild -allowProvisioningUpdates -authenticationKeyPath ...` uses, refuse it (401). The group
  (`group.` and the app's identifier) is made and ticked on both App IDs on the developer website; until it is, a
  profile's `com.apple.security.application-groups` is empty, the app keeps its client in its own folder, and the
  notification extension can't be signed.
- A development profile's `aps-environment` is `development`, so the phone's APNs token is the sandbox's, which the
  network's edge reaches only with `APNS_SANDBOX=1` (`plugins/notifications/edge/README.md`).
- The phone is paired with the Mac: `xcrun devicectl list devices` names it.

## Building and signing

1. Turn signing off in your checkout: `CODE_SIGNING_ALLOWED: NO` in `gen/apple/project.yml`'s settings for both
   targets, `app_iOS` and `Notifications`, then `gen/apple/generate` (with the network's config if it has one).
   An `.xcconfig` doesn't do it: `cargo tauri ios build` takes none. Then the network's iPhone build command: its
   archive succeeds and its export fails (`errSecInternalComponent`), and the app is
   `gen/apple/build/app_iOS.xcarchive/Products/Applications/<name>.app`. Or build for the simulator with
   `--target aarch64-sim --debug`, which needs no signing.
2. Put each target's profile in it as `embedded.mobileprovision`, and sign it with that profile's own entitlements
   less `keychain-access-groups` (a wildcard, which a signature can't hold) and an empty
   `com.apple.security.application-groups`: `security cms -D -i <profile> > p.plist`,
   `plutil -extract Entitlements xml1 -o e.plist p.plist`, then `/usr/libexec/PlistBuddy -c "Delete :<key>" e.plist`
   (`plutil` reads the dots in `com.apple...` as a path). Sign
   `PlugIns/Notifications.appex` first, then the app; with no profile for the extension, remove `PlugIns/` instead.
3. Unlock the keychain and `codesign` in the same ssh connection: `security -i` reads `unlock-keychain -p ...` on
   stdin, then `codesign --force --keychain <it> --sign "Apple Development" --entitlements <file> <app>`. The
   password goes on stdin, never in a command line, where `ps` and logs see it. The key's partition list must
   already allow `codesign` (`security set-key-partition-list -S apple-tool:,apple:`); if it doesn't, a dialog
   appears on the owner's screen. `security lock-keychain <it>` at the end of that same connection.
4. `xcrun devicectl device install app --device <id> <app>` installs over the app there, keeping its data, locked
   phone or not; `xcrun devicectl device info apps --device <id> --bundle-id <the app's identifier>` shows its build.
   Then `xcrun devicectl device process launch --device <id> <the app's identifier>`, which a locked phone refuses
   (`Locked`). `one builds` shows the phone's new build only once the app has opened.

The keychain's password is in the owner's password manager; ask the orchestrator or Agent Memory for the item.

## Mistakes already made

- Signing through anything running in the owner's logged-in session (Xcode, a GUI `osascript`, a Terminal tab):
  each signature put a password dialog on his screen. Only ssh, and only a keychain unlocked in that connection.
- Leaving signing to `cargo tauri ios build` or Xcode's export, which fail with no account signed in.
- Signing with the wildcard profile: the phone got no APNs token and said it isn't woken by notifications.
