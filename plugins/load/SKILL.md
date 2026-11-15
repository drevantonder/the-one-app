---
name: load
description: Read before giving a machine heavy work, to pick the one with most to standby, or when asked how busy a machine is.
---

Load says how busy each machine is: how many processors it has, its load over the last 1, 5 and 15 minutes, its
memory and disk in use (`used` of `total` bytes), how busy its CPU and GPU were every 5 seconds over the last two
minutes (percent, oldest first), and the containers it hosts. A load as high as the number of processors is a
machine with no time to standby, so use `now` before giving a machine heavy work, to pick the one with most to standby.
A machine that couldn't be asked, such as one that's off, has a `problem` instead.

Each machine is measured by Load's service `Load.Meter`, which brings a small program written in Rust and
builds it on the machine, so the machine needs `rustc`. `services` says which servers run it, and
`install_service` runs it on one.
