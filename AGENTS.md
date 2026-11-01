# The One App

Your devices, and your team's, as one place for apps and agents. Read `ARCHITECTURE.md` before changing code and `SPEC.md` before changing anything that crosses a machine boundary.

## Principles

- Six primitives: key, log, view, blob, call, app. Scopes, servers, deploys, drivers, the Vault and backups are made of them. A feature that needs a seventh is designed wrong until proven otherwise.
- The core never names an app. The starter apps use exactly what anyone's app can.
- Privileged servers are trusted by admins, not built in: any server can be made one, and the install keeps working without them. If something only a privileged server can do is in code rather than in trust, the design is wrong.
- Entries are the truth; views are caches. The same entries replayed in the same order give the same views, on every server, every time.
- Dumb beats clever. Delete before adding. No dependency without a reason written in its commit.
- One name per thing. A rename reaches everything in the same change: code, files, docs, screens, CLI and MCP text, keeping live paths working. No old names left behind, and no internal words (module names, "provider") where a person reads.
- The old One App and the PTC App are feature lists, not code to copy.

## Work

- `cargo test` before every commit, and `gh signoff` once it's pushed: tests run on our own machines, never in CI. `tests/sim.rs` runs the simulator; a failing seed is a bug, never a flake.
- Changes to `SPEC.md` are breaking for every machine: bump `PROTOCOL` in `src/entry.rs`.
- Push to `main` as you go (pull with `--rebase` first: several threads on several machines work on it at once). After each push to main, run `one update` on a Linux server of yours: it tests and builds main's tip, every Linux node updates itself, and each Mac rebuilds its app over SSH. `one release vX.Y.Z` does the same and publishes it on GitHub, where the installer gets it.
- PTC's install runs The PTC App on linux and ptc: main's code built with the `config.toml` in PTC's own repo, PTC-AU-Tech/ptc-apps (this repo never names PTC). After `one update`, update it too: `ssh drevan@100.76.107.116 'PTC_HOME=~/.local/share/the-ptc-app nice ~/.local/lib/the-ptc-app/ptc update'` fetches main's tip and ptc-apps' latest config, tests and builds them as that edition, signs it with The PTC App's release key, and both PTC nodes take it. Build on ptc, not linux: linux's newer glibc makes binaries ptc can't run. On ptc the old PTC App owns `~/.local/bin/ptc` and `~/.local/share/ptc`, hence those paths. PTC has no GitHub releases: `install.sh` installs The One App.
- Apps reach servers only by a deploy: `one deploy`, or an admin adding a built-in app (the folders in `apps/`, which every release ships; see ARCHITECTURE.md). Drivers, only by an owner of a server installing them there. Install-specific apps (PTC's) never go in `apps/`.
- One commit per change, once it works: `area: what changes, for whoever uses it`. No co-author or "Generated with" lines.
- Document only what the code can't say.
