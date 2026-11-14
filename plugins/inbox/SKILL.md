---
name: inbox
description: Read when someone asks which files arrived on a machine, or wants a folder there watched for scans, downloads or exports.
---

Inbox watches a folder on a machine and keeps a list of the files that arrive in it: scans, downloads, exports,
whatever lands there. `list` gives what arrived, from any machine, whether or not the one watching is up.

The watching is done on the machine itself, by Inbox's service `Inbox.Watcher`. `services` says which servers
run it, and `install_service` runs it on one. Then `watch` tells that machine which folder to watch, and
`watching` says what each machine is watching now. A file that was already there when watching began isn't
an arrival.
