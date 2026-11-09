---
name: ptc-update
description: Sending main out to The PTC App's network, a custom build of The One App.
---

# PTC's update

The ship skill's steps, on ptc, with `ptc-dev` for `one-dev`, in this environment:

    ssh drevan@100.76.107.116
    . ~/.profile; export PTC_HOME=~/.local/share/the-ptc-app; alias ptc-dev="nice ~/.local/lib/the-ptc-app/ptc-dev"

Its build folder is under `$PTC_HOME/cache/builds`, its clean checkout `$PTC_HOME/cache/src`, and the device to wait for is `ptc`. The config comes from its own repo: `ptc-dev build` takes its latest. Commits have its config's after them (`<commit>-<config>`): use what the folder is called. It has no GitHub releases.
