---
name: load
description: Read before giving a machine heavy work, to pick the one with most to spare, or when asked how busy a machine is.
---

Load says how busy each machine is: how many processors it has, and its load over the last 1, 5 and 15
minutes. A load as high as the number of processors is a machine with no time to spare, so use `now` before
giving a machine heavy work, to pick the one with most to spare.

Each machine is measured by Load's service `Load.Meter`, which brings a small program written in Rust and
builds it on the machine, so the machine needs `rustc`. `services` says which servers run it, and
`install_service` runs it on one.
