# The One App

Your devices, and your team's, as one place for apps and agents. Read `ARCHITECTURE.md` before changing code and `SPEC.md` before changing anything that crosses a device boundary.

## Principles

- Six primitives: key, log, replay, blob, call, app. A feature that needs a seventh is designed wrong until proven otherwise.
- The core never names an app. Built-in apps use exactly what a user's app can.
- No peer is privileged. Anything a cloud peer does, a home box could do.
- Replay is deterministic: the same entries in the same order give the same views, on every device, every time.
- Dumb beats clever. Delete before adding. No dependency without a reason written in its commit.
- The old One App and the PTC App are feature lists, not code to copy.

## Work

- `cargo test` before every commit, and `gh signoff` once it's pushed: tests run on our own devices, never in CI. `tests/sim.rs` runs the network simulator; a failing seed is a bug, never a flake.
- Changes to `SPEC.md` are breaking for every device: bump `PROTOCOL` in `src/entry.rs`.
- Push to `main` as you go (pull with `--rebase` first: several threads on several machines work on it at once). After each push to main, run `one update` on a Linux device of yours: it tests and builds main's tip, every Linux node updates itself, and the Macs rebuild their app. `one release vX.Y.Z` does the same and publishes it on GitHub, where the installer gets it.
- One commit per change, once it works: `area: what changes, for whoever uses it`. No co-author or "Generated with" lines.
- Document only what the code can't say.
