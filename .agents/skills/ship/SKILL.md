---
name: ship
description: Getting a change you made out to every device, of The One App or a custom build. Use once it works.
---

# Ship a change

`<cli>-dev <command> --help` says what each takes. This says when. A custom build's tool is `<cli>-dev`, built with its config (`one-dev` for The One App itself), and its builds are named `<commit>-<config commit>`; its own repo's AGENTS.md says which device ships it.

You own a change until it's seen running. `<cli>-dev unfinished` lists what isn't, and your prompts say so for what your checkout made.

0. If the build's `repo` (in its config) is a fork, bring upstream in first: the `keep-a-fork-current` skill. A fork shipped without it ships whatever it last took.
1. Test and push as AGENTS.md says, then `<cli>-dev signoff tested --ran "<the tests you chose>"` in that checkout. Only a commit signed off this way goes out; a newer one without it stays behind, and sending names it.
2. On the device with the toolchain and the release key: `<cli>-dev ship`. It sends each part that changed, in order (apps, mac, services, program, workers, web, dev), asks every device online that's behind but the Macs to take it, as Update does (nothing updates by itself), and returns when every device online runs it, or says which doesn't and why. Each part sends the newest tested commit and keeps the commit it sent, so a part with nothing new does nothing, and a push that lands meanwhile goes out next time. `<cli>-dev changes` says what's due, part by part. `<cli>-dev update` is the same send with only this device updated, and no wait.
3. Look at it working where it went, then `<cli>-dev signoff seen <commit> --checked "<what you checked>"`.
4. A part on its own: `<cli>-dev update workers|web|apps|services|mac|program|dev`. Renamed or removed apps are retired by `update apps`, which says where to `<cli> add` a renamed one afterwards.
5. A change to Workers or the web client alone needs no program build, and isn't waited for by any device. Any change to Rust, or to anything not known to be outside the program, sends everything: `update program` sends the program (`one`, with oned, onet and the app runtime) and oned on Cloudflare, then its Workers and web client. A part is marked sent only once it has gone out, so what an update says didn't is what `<cli>-dev changes` lists, and running `update` again sends it.
6. A Worker change that devices need first: `<cli>-dev update workers` before `<cli>-dev update program`. If Rust is in the same change, deploy it first with your build (`target/debug/one cloudflare deploy`, on any device signed in to the network's 1Password).
7. `<cli>-dev status --until <commit>`, to watch one commit or named devices. A device that was offline: Update it once it's back. A Mac: once its app is published, Update it. A send whose Mac app is due builds it on the Mac ssh reaches as `MAC` (`mac` if unset), in its agent's checkout, while the rest builds here, and publishes it from here; with no Mac reachable, or a build of its agent's already running there, its agent publishes it (the keep-mac-current skill), or `<cli>-dev update mac` once it's done.
