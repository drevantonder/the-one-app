---
name: ptc-update
description: Sending main out to The PTC App's network, a custom build of The One App.
---

# PTC's update

The ship skill's steps, on ptc, with `ptc-dev` for `one-dev`, in this environment:

    ssh drevan@100.76.107.116
    . ~/.profile; export PTC_HOME=~/.local/share/the-ptc-app; alias ptc-dev="nice ~/.local/lib/the-ptc-app/ptc-dev"

Its build folder is under `$PTC_HOME/cache/builds`, its clean checkout `$PTC_HOME/cache/src`, and the device to wait for is `ptc`. The config comes from its own repo: `ptc-dev build` takes its latest. Its builds are named `<commit>-<config>`, as the folder is: `deploy workers` takes all of that, `status --until` only the commit before the `-`. `ptc-dev` itself is `~/.local/lib/the-ptc-app/ptc-dev`, and it has no other machine to copy to. It has no GitHub releases.
