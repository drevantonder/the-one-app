# The One App

Your devices, and your team's, as one place for apps and agents. Read `ARCHITECTURE.md` before changing code and `SPEC.md` before changing anything that crosses a device boundary.

## Principles

- Six primitives: key, log, fold, blob, call, app. A feature that needs a seventh is designed wrong until proven otherwise.
- The core never names an app. Built-in apps use exactly what a user's app can.
- No peer is privileged. Anything a cloud peer does, a home box could do.
- Folds are deterministic: the same entries in the same order give the same views, on every device, every time.
- Dumb beats clever. Delete before adding. No dependency without a reason written in its commit.
- The old One App and the PTC App are feature lists, not code to copy.

## Work

- `cargo test` before every commit. `tests/sim.rs` runs the network simulator; a failing seed is a bug, never a flake.
- Changes to `SPEC.md` are breaking for every device: bump `PROTOCOL` in `src/entry.rs`.
- One commit per change, once it works: `area: what changes, for whoever uses it`. No co-author or "Generated with" lines.
- Document only what the code can't say.
