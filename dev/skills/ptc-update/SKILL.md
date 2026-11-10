---
name: ptc-update
description: Sending main out to The PTC App's network, a custom build of The One App.
---

# PTC's update

Nothing sends PTC's build by itself: the agent with a reason to runs `ptc-dev update` on ptc, with its own markers and its own release key. It takes the newest commit on `ptc/the-one-app`'s main signed off as tested, checks it with the `check` in PTC's config, and does nothing if nothing changed. A failed check says what failed, and nothing of that build goes out. Run it, or the ship skill's commands for one part alone, on ptc, with `ptc-dev` for `one-dev`, in this environment:

    ssh drevan@100.76.107.116
    . ~/.profile; export PTC_HOME=~/.local/share/the-ptc-app; alias ptc-dev="nice ~/.local/lib/the-ptc-app/ptc-dev"

Its build folders are under `$PTC_HOME/cache/builds`, its clean checkout `$PTC_HOME/cache/src`, and the device to wait for is `ptc`. The config comes from its own repo, its latest each time: a change to it sends everything. Builds are named `<commit>-<config>`, which `status --until` also takes. `ptc-dev` itself is `~/.local/lib/the-ptc-app/ptc-dev`. It has no GitHub releases.
