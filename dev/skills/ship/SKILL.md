---
name: ship
description: Getting a change you made to The One App out to every device. Use once it works.
---

# Ship a change

`one-dev <command> --help` says what each takes. This says when.

You own a change until it's seen running. `one-dev unfinished` lists what isn't, and your prompts say so for what your checkout made.

1. Test and push as AGENTS.md says, then `one-dev signoff tested --ran "<the tests you chose>"` in that checkout. Only a commit signed off this way goes out; a newer one without it stays behind, and sending names it.
2. On personal, which has the toolchain and the release key: `one-dev ship`. It sends each part that changed, in order (apps, mac, services, node, workers, web, dev), and returns when every device online runs it, or says which doesn't and why. Each part sends the newest tested commit and keeps the commit it sent, so a part with nothing new does nothing, and a push that lands meanwhile goes out next time. `one-dev changes` says what's due, part by part. `one-dev update` is the same send without the wait.
3. Look at it working where it went, then `one-dev signoff seen <commit> --checked "<what you checked>"`.
4. A part on its own: `one-dev update workers|web|apps|services|mac|node|dev`. Renamed or removed apps are retired by `update apps`, which says where to `one add` a renamed one afterwards.
5. A change to Workers or the web client alone needs no node build, and isn't waited for by any device. Any change to Rust, or to anything not known to be outside the node, sends everything: `update node` sends the node, then its Workers and web client. A part is marked sent only once it has gone out, so what an update says didn't is what `one-dev changes` lists, and running `update` again sends it.
6. A Worker change that nodes need first: `one-dev update workers` before `one-dev update node`. If Rust is in the same change, deploy it first with your build (`target/debug/one cloudflare deploy` on the Workers' home).
7. PTC builds its fork, which the ptc-sync skill brings main into each day: run that skill by hand to get a change to PTC sooner.
8. `one-dev status --until <commit>`, to watch one commit or named devices. A Mac that's behind: the mac-behind skill.
