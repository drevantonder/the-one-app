---
name: release
description: Publishing a numbered release of The One App on GitHub, for the installer.
---

# Release

Everything goes out, whatever `changes` says, and `cargo test` passes on the commit first.

1. On the device with the release key, forget what was sent, so every part is due: `rm -r ~/.cache/one/sent`.
2. `one-dev update --release`, so a Mac notarizes its app.
3. `one-dev release <tag>`. It builds main's tip if `update` hasn't, and waits for a Mac's app, so a Mac with the release key must be online.
