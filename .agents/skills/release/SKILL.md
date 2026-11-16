---
name: release
description: Use once your change is on main, to put it live on the network by the smallest path it needs, or to see what is live. Whoever makes a change releases it.
---

Release from `main` as it is now, never an older commit, and only what your change touched.

A release is part of your change: it isn't done until it's live. `tooling/release` takes minutes, so run it in the
foreground with a long timeout, or in the background and have your harness tell you when it ends. Ending your turn
while it runs stops you there: nothing updates the servers afterwards.

| Changed | Release |
| --- | --- |
| A plugin (`plugins/<name>/`, here or in a network's repo) | `ONE_FOR=network one deploy --name <name> --files-from plugins/<name>`, from that repo (about 12 s) |
| A plugin with a NIF (`server/nif/`) | `tooling/nifs`, then the same deploy |
| The UI library (`core/ui/`) | `bun build.js` in `core/ui/`, then `ONE_FOR=network one deploy --name ui --files-from core/ui/dist`, then the edge |
| The edge (`core/edge/`, a plugin's `edge/`, `clients/web/`) | `bun install && bun edge.js deploy <the network's address>` in `core/edge/`, with the Cloudflare token in `CLOUDFLARE_API_TOKEN` |
| The server (`core/server/`, or Rust it builds with) | `tooling/release server`, then `one update --server <key>` for each server, one at a time |
| The desktop app or CLI (`core/client/`, `clients/`, a plugin's `client/native/`) | `tooling/release client`, then `one update` on each device. `client` includes the Mac's build, made on the Mac over ssh; Linux's alone is `tooling/release one/linux-x86_64 one-cli/linux-x86_64` (about 4 min) |
| A phone's app | Not from here: a phone takes a build from its store, built from the network's repo (`app/`). Say one is needed |
| Docs, tests, `tooling/`, `.agents/` | Nothing |

A plugin change that uses something new in `core/server/` (often in the same commit) waits for the server release:
deploy it once every server is on that build, since a server on the old build refuses or breaks it.
The other way round too: a server change that a live plugin depends on (a field it reads, a call it makes) ships
with that plugin's new version in the same step, the first server's update and the plugin's deploy back to back, or
the plugin breaks for whoever uses it meanwhile. A plugin someone deployed by hand isn't redeployed by the server.

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
- Releasing a server from a tree with changes not committed: the build is `-dirty`, and every server refuses it
  ("says it's …-dirty, not what was published").
- Releasing a server whose calls to other servers (`hooks/call`, `plugins/call`) take new required params. Once one
  server is on it, the old ones can't decide anything that asks permissions there, their own update among them
  (`permissions couldn't decide it: This server has no method hooks/call`). Take each left behind by hand, as its
  program would: download the build `one download --target <kind>` links, unpack it in its `server/builds/<build>`
  past `bin/one_core eval "OneCore.Release.check()"` saying that build, write the build to `server/next`, and
  restart its unit (`systemctl --user restart one-server-<key's first 12>`, or on a Mac `launchctl kickstart -k
  gui/$(id -u)/app.theone.server.<key's first 12>`); it goes back by itself if it doesn't answer.

## Report

What you released, by which command, and the minutes from your commit reaching `main` to `one builds` (or
`one code`) showing it live.
