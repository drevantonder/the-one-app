# The One App

Your devices, and your team's, as one place for apps and agents. Read `ARCHITECTURE.md` before changing code and `SPEC.md` before changing anything that crosses a machine boundary.

## Principles

- Six primitives: key, log, view, blob, call, app. Scopes, repos, releases, drivers and backups are made of them. A feature that needs a seventh is designed wrong until proven otherwise.
- Nothing is built in. The node never names an app or driver, and this repo contains none; the starter apps use exactly what anyone's app can.
- No server is privileged. A cloud server is only a server that holds every scope: anything it does, a user server could do for its own user.
- Entries are the truth; views are caches. The same entries replayed in the same order give the same views, on every server, every time.
- Dumb beats clever. Delete before adding. No dependency without a reason written in its commit.
- The old One App and the PTC App are feature lists, not code to copy.

## Work

- `cargo test` before every commit, and `gh signoff` once it's pushed: tests run on our own machines, never in CI. `tests/sim.rs` runs the simulator; a failing seed is a bug, never a flake.
- Changes to `SPEC.md` are breaking for every machine: bump `PROTOCOL` in `src/entry.rs`.
- Push to `main` as you go (pull with `--rebase` first: several threads on several machines work on it at once). After each push to main, run `one update` on a Linux server of yours: it tests and builds main's tip, every Linux node updates itself, and each Mac rebuilds its app over SSH. `one release vX.Y.Z` does the same and publishes it on GitHub, where the installer gets it.
- Apps and drivers change in their own repos, and reach servers only by an explicit release.
- One commit per change, once it works: `area: what changes, for whoever uses it`. No co-author or "Generated with" lines.
- Document only what the code can't say.
