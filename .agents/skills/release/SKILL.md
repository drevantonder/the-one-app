---
name: release
description: Use once your change is on main, to put it live on the network by the smallest path it needs, or to see what is live. Whoever makes a change releases it.
---

Release from `main` as it is now, never an older commit, and only what your change touched.

| Changed | Release |
| --- | --- |
| A plugin (`plugins/<name>/`) | `ONE_FOR=network one deploy --name <name> --files-from plugins/<name>` (about 12 s) |
| A plugin with a NIF (`server/nif/`) | `tooling/nifs`, then the same deploy |
| The UI library (`ui/`) | `bun build.js` in `ui/`, then `ONE_FOR=network one deploy --name ui --files-from ui/dist`, then the edge |
| The edge (`core/edge/`, a plugin's `edge/`, `clients/web/`) | `bun install && bun edge.js deploy <the network's address>` in `core/edge/`, with the Cloudflare token in `CLOUDFLARE_API_TOKEN` |
| The server (`core/server/`, or Rust it builds with) | `tooling/release server`, then `one update --server <key>` for each server, one at a time |
| The desktop app or CLI (`core/client/`, `clients/`, a plugin's `client/native/`) | `tooling/release client`, then `one update` on each device. `client` includes the Mac's build, made on the Mac over ssh; Linux's alone is `tooling/release one/linux-x86_64 one-cli/linux-x86_64` (about 4 min) |
| A phone's app | Not from here: a phone takes a build from its store. Say one is needed |
| Docs, tests, `tooling/`, `.agents/` | Nothing |

A plugin change that uses something new in `core/server/` (often in the same commit) waits for the server release:
deploy it once every server is on that build, since a server on the old build refuses or breaks it.

On a device that is also a server, `one update` asks the program keeping that server running (`one serve run`) to
take the latest builds, the program's among them, and answers before it has. What it did is in that program's log,
`journalctl --user -u one-server-<key's first 12>` on Linux: `one --version` then says the new build.

A server leaves a plugin deployed by hand as it is, so a plugin deploy is always safe beside a server release: deploy
it even when the server is released too.

One server or edge release on a machine at a time. `tooling/release` waits for another itself; run an edge deploy
under `flock "${XDG_RUNTIME_DIR:-/tmp}/one-release.lock" bun edge.js deploy <address>`. The network's address and
where its Cloudflare token is kept are in Agent Memory, not the repo.

## What is live

- `one builds` gives each server's `build` as `<count>-<commit>`; `one builds | jq -r '.servers[] | .server + " " + .name + " " + .build'`
  lists the keys `one update --server` takes.
- `git diff --stat <commit> origin/main` is what isn't live on the server.
- `ONE_FOR=network one code --name <plugin>` is a plugin's files as deployed.

## Mistakes already made

- Releasing everything when one plugin changed. A server release is minutes and restarts every server; a plugin is
  seconds.
- Releasing a server build onto storage whose stored names it had renamed. A rename of anything kept (a blob prefix,
  a database, a table) needs a one-time rename of what's stored first; say so in the commit and do it before
  `one update`.
- Releasing from a worktree behind `origin/main`, which `tooling/release` refuses once a newer build is published.

## Report

What you released, by which command, and the minutes from your commit reaching `main` to `one builds` (or
`one code`) showing it live.
