---
name: ship
description: Getting a change you made to The One App out to every device. Use once it works.
---

# Ship a change

`one-dev <command> --help` says what each takes. This says when.

1. Test and push as AGENTS.md says.
2. On personal, which has the toolchain and the release key: `one-dev update`. It runs each part that changed, in order: apps, mac, services, node, workers, web, dev. Each part sends main's tip and keeps the commit it sent, so a part with nothing new does nothing, and a push that lands meanwhile goes out next time. `one-dev changes` says what's due, part by part.
3. A part on its own: `one-dev update workers|web|apps|services|mac|node|dev`. Renamed or removed apps are retired by `update apps`, which says where to `one add` a renamed one afterwards.
4. A change to Workers or the web client alone needs no node build, and isn't waited for by any device. Any change to Rust, or to anything not known to be outside the node, sends everything.
5. A Worker change that nodes need first: `one-dev update workers` before `one-dev update node`. If Rust is in the same change, deploy it first with your build (`target/debug/one cloudflare deploy` on the Workers' home).
6. PTC follows main by itself on ptc's timer: the ptc-update skill, only to send it sooner.
7. `one-dev status --until <commit>`, only when a change must be seen live. A Mac that's behind: the mac-behind skill.
