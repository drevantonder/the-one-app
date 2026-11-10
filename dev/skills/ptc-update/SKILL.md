---
name: ptc-update
description: Sending main out to The PTC App's network, a custom build of The One App.
---

# PTC's update

A timer on ptc runs `ptc-dev update` every 15 minutes, with its own markers and its own release key: each run takes main's newest commit signed off as tested and does nothing if nothing changed. To send it sooner, or one part alone, the ship skill's commands, on ptc, with `ptc-dev` for `one-dev`, in this environment:

    ssh drevan@100.76.107.116
    . ~/.profile; export PTC_HOME=~/.local/share/the-ptc-app; alias ptc-dev="nice ~/.local/lib/the-ptc-app/ptc-dev"

Its build folders are under `$PTC_HOME/cache/builds`, its clean checkout `$PTC_HOME/cache/src`, and the device to wait for is `ptc`. The config comes from its own repo, its latest each time: a change to it sends everything. Builds are named `<commit>-<config>`, which `status --until` also takes. `ptc-dev` itself is `~/.local/lib/the-ptc-app/ptc-dev`. It has no GitHub releases. What the timer did: `journalctl --user -u ptc-update`.
