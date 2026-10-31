# The One App

Your devices, and your team's, as one place for apps and agents. Read `ARCHITECTURE.md` before changing code and `SPEC.md` before changing anything that crosses a machine boundary.

## Principles

- Six primitives: key, log, view, blob, call, app. Scopes, servers, deploys, drivers, secrets and backups are made of them. A feature that needs a seventh is designed wrong until proven otherwise.
- The core never names an app. This repo contains no apps or drivers; the starter apps use exactly what anyone's app can.
- Privileged servers are trusted by admins, not built in: any server can be made one, and the install keeps working without them. If something only a privileged server can do is in code rather than in trust, the design is wrong.
- Entries are the truth; views are caches. The same entries replayed in the same order give the same views, on every server, every time.
- Dumb beats clever. Delete before adding. No dependency without a reason written in its commit.
- The old One App and the PTC App are feature lists, not code to copy.

## Work

- `cargo test` before every commit, and `gh signoff` once it's pushed: tests run on our own machines, never in CI. `tests/sim.rs` runs the simulator; a failing seed is a bug, never a flake.
- Changes to `SPEC.md` are breaking for every machine: bump `PROTOCOL` in `src/entry.rs`.
- Push to `main` as you go (pull with `--rebase` first: several threads on several machines work on it at once). After each push to main, run `one update` on a Linux server of yours: it tests and builds main's tip, every Linux node updates itself, and each Mac rebuilds its app over SSH. `one release vX.Y.Z` does the same and publishes it on GitHub, where the installer gets it.
- Apps live outside this repo and reach servers only by `one deploy`; drivers, only by an owner of a server installing them there.
- One commit per change, once it works: `area: what changes, for whoever uses it`. No co-author or "Generated with" lines.
- Document only what the code can't say.
