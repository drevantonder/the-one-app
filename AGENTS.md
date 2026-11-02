# The One App

Your devices, and your team's, as one place for apps and agents. Read `ARCHITECTURE.md` before changing code. What machines say to each other is in the code that says it and its tests (`tests/contracts.rs` where two languages must agree): read those before changing anything that crosses a machine boundary.

## Principles

- Six primitives: key, log, view, blob, call, app. Scopes, servers, deploys, sidecars, the Vault and backups are made of them. A feature that needs a seventh is designed wrong until proven otherwise.
- The core never names an app. The starter apps use exactly what anyone's app can.
- The kernel, nodes and clients. The kernel is a few small services, locked down and trusted with everything; nodes run calls and hold nothing open; clients are what people and their agents use. A hacked node gets only the calls that ran on it while it was hacked; a hacked kernel service is everyone's loss, which is why it stays small. ARCHITECTURE.md's "Being built" is the design, and it wins over the rules above it until they're rewritten.
- Each app's database for a scope is the truth, kept sealed in the install's bucket; who can do what is a signed log kept forever.
- Dumb beats clever. Delete before adding. No dependency without a reason written in its commit.
- One name per thing. A rename reaches everything in the same change: code, files, docs, screens, CLI and MCP text, every caller and the data, rolled out to every device together. No aliases or fallbacks for old names, and no internal words (module names, "provider") where a person reads.
- The old One App and the PTC App are feature lists, not code to copy.

## Work

- `one try` (`target/release/one try` for your build) starts a new install beside the real one, with only what every new install starts with: what a new person sees. Review defaults and test changes there with `one try <command>` or `one try mcp`; `one try stop` deletes it.
- `cargo test` before every commit, and `gh signoff` once it's pushed: tests run on our own machines, never in CI. `tests/sim.rs` runs the simulator; a failing seed is a bug, never a flake.
- A change machines see must be one an older device can safely ignore (phones and Macs can run behind the servers). If it can't be, make it so first. Until the rebuild in ARCHITECTURE.md lands this doesn't hold: there is nothing in production, installs start fresh, and only the board is brought across.
- Push to `main` as you go (pull with `--rebase` first: several threads on several machines work on it at once). After each push to main, run `one update` on a Linux server of yours: it tests and builds main's tip, every Linux node updates itself, and each Mac rebuilds its app over SSH. `one release vX.Y.Z` does the same and publishes it on GitHub, where the installer gets it.
- PTC's install runs The PTC App on linux and ptc: main's code built with the `config.toml` in PTC's own repo, PTC-AU-Tech/ptc-apps (this repo never names PTC). After `one update`, update it too: `ssh drevan@100.76.107.116 'PTC_HOME=~/.local/share/the-ptc-app nice ~/.local/lib/the-ptc-app/ptc update'` fetches main's tip and ptc-apps' latest config, tests and builds them as that white label, signs it with The PTC App's release key, and both PTC nodes take it. Build on ptc, not linux: linux's newer glibc makes binaries ptc can't run. On ptc the old PTC App owns `~/.local/bin/ptc` and `~/.local/share/ptc`, hence those paths. PTC has no GitHub releases: `install.sh` installs The One App.
- Apps reach servers only by a deploy: `one deploy`, or an admin adding a built-in app (the folders in `apps/`, which every release ships; see ARCHITECTURE.md). Server sidecars, only by an owner of a server installing them there. Install-specific apps (PTC's) never go in `apps/`.
- One commit per change, once it works: `area: what changes, for whoever uses it`. No co-author or "Generated with" lines.
- Document only what the code can't say.
- Nothing describes the code but the code: no hand-written references that can drift; generate them or point at the source.
