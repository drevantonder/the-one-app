---
name: ship
description: Getting a change you made to The One App out to every device. Use once it works.
---

# Ship a change

`one-dev <command> --help` says what each takes. This says when.

1. Test and push as AGENTS.md says.
2. On personal, which has the toolchain and the release key: `one-dev changes`. Send only what it lists. Other threads ship too, so it may be nothing: then stop.
3. In this order, skipping what it doesn't list:
   - Renamed or removed apps: `one-dev retire`, before the build without them goes out. It says where to `one add` a renamed one afterwards.
   - The Mac app: `one-dev publish source`.
   - Services: `one-dev publish services`.
   - Apps, with no node to send: `one-dev deploy apps ~/.cache/one/src`, which `changes` left at main's tip.
   - The node or `one-dev`: `one-dev build`, which prints a folder named by the commit. Then, for the node:
     1. `one-dev publish <folder>/one`
     2. `one-dev status --until <commit> --device personal`: the next two need it running here.
     3. `one-dev deploy apps`
     4. `one-dev deploy workers <commit>`
     5. `one-dev publish web <folder>`
   - `one-dev` itself, which no device takes: `install <folder>/one-dev "$(command -v one-dev)"`, then `scp` it to `.local/bin/` on `drevan@100.76.107.116` (ptc) and `linux`, for their throwaways.
4. `one-dev changes --sent`, only once all of it went out: what you leave unmarked goes out next time.
5. The ptc-update skill, at the same time if you like: it's built on its own machine.
6. `one-dev status --until <commit>`, for the devices online. A Mac that's behind: the mac-behind skill.

A Worker change that nodes need first: deploy it before publishing the node, with your build (`target/debug/one cloudflare deploy` on the Workers' home).
