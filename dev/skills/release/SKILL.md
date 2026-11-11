---
name: release
description: Publishing a numbered release of The One App on GitHub, for someone starting their first network.
---

# Release

A GitHub release is only for someone starting their first network, who has no address yet: every network's other computers install from its own address (`<address>/install`). It's laid out as that address's `/updates/` is, a signed record and each file by its hash, with the same installers.

Everything goes out, whatever `changes` says, and `cargo test` passes on the commit first.

1. On the device with the release key, forget what was sent, so every part is due: `rm -r ~/.cache/one/sent`.
2. `one-dev update --release`, so a Mac notarizes its app.
3. `one-dev release <tag>`. It builds main's tip if `update` hasn't, and waits for a Mac's app, so a Mac with the release key must be online.
