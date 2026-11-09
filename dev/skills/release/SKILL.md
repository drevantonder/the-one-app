---
name: release
description: Publishing a numbered release of The One App on GitHub, for the installer.
---

# Release

Everything goes out, whatever `changes` says, and `cargo test` passes on the commit first.

1. Ship as the ship skill says, every step, with `one-dev publish source --release` so a Mac notarizes its app, and `one-dev publish services` for every service.
2. `one-dev release <tag> <folder>`, with the folder `one-dev build` printed. It waits for a Mac's app, so a Mac with the release key must be online.
