---
name: release
description: Use once your change is on main, to put it live on the network by the smallest path it needs, or to see what is live. Whoever makes a change releases it.
---

Release from `main` as it is now, never an older commit, and only what your change touched.

A release is part of your change: it isn't done until it's live.

## Versions

The platform (One Core's server, the UI library, the edge, the clients and what they say to each other) has one
version, Semantic Versioning's, which `tooling/version` reads from the commits since 0.15.0 by their Conventional
Commit types, as semantic-release does: a breaking change (`type!:` or a `BREAKING CHANGE:` footer) takes the next
minor, a `feat` or a `fix` the next patch. Before 1.0 a minor is breaking, as Cargo, npm and Hex read 0.x. A build
is that version with its commit's count and hash as build metadata, `0.15.2+2974.febd37350abc`. Nothing decides a
bump but how commits are marked: only a change a part, plugin or agent using the platform must change for (a
renamed field, tool or method with no alias, a new required param, a removed function plugins call) is marked `!`,
never one inside the repo (tests, skills, tooling). One that wasn't is marked by a commit after it,
`fix!: <what broke>`, before it's released.

Exactly two rules hold at runtime:

- Parts talk only on the same 0.minor (`OneCore.Builds.talk?/2`). A server refuses a request of another
  (`OneCore.Mcp`), which says why; a server asks one behind it nothing, as if it were off; the relay takes a server
  behind the edge for restarting and passes the edge's calls only to servers on its own (`core/edge/relay/relay.ts`).
  What updates a part that's behind, `tools/list`, `builds`, `download` and `update`, is answered whatever it's on. `one builds` says each
  server's `build`, the `edge`'s, and `why` one doesn't talk.
- A plugin runs only on a server its `requires`' `"core" => "~> 0.16.0"` allows (`OneCore.Plugins.Version`):
  deploying it to an older one is refused, saying so, and a server that loads it runs the plugin nowhere until it's
  updated.

## Order

One release at a time. Before publishing a server or a client, look for another under way: a build in `one builds`'
`latest` that not every server or device runs yet, or the release lock held (`${XDG_RUNTIME_DIR:-/tmp}/one-release.lock`
names its build). If there's one, don't publish: say whose it is, from its commit's author and Agent Memory, and stop.

A patch release mixes with what's live, in any order. A breaking one goes in this order, back to back, so the network
is split only for minutes:

1. The servers, one at a time (`tooling/release server`, `one update --server <key>` for each). A server that goes
   back stops the release: read its log, and update no other.
2. The plugins that require the new platform, once every server is on it.
3. The edge, from the commit the servers run.
4. The clients (`tooling/release client`, `one update` on each device).

| Changed | Release |
| --- | --- |
| A plugin (`plugins/<name>/`, here or in a network's repo) | `ONE_SUBJECT=network one deploy --name <name> --files-from plugins/<name>`, from that repo (about 12 s). Refused, someone else's newer change is live: rebase and deploy again, never `--force` |
| A plugin with a NIF (`server/nif/`) | `tooling/nifs`, then the same deploy. A Mac's NIF whose inputs haven't changed is the one last published (or cached here); one that changed is built on the Mac, which must be awake |
| The UI library (`core/ui/`) | The server, which ships it and builds every app again against it, then the edge, which serves it to browsers |
| The edge (`core/edge/`, a plugin's `edge/`, `clients/web/`) | `bun install && bun edge.js deploy <the network's address>` in `core/edge/`, with the Cloudflare token in `CLOUDFLARE_API_TOKEN` |
| The server (`core/server/`, or Rust it builds with) | `tooling/release server`, then `one update --server <key>` for each server, one at a time |
| The desktop app or CLI (`core/client/`, `clients/`, a plugin's `client/native/`) | `tooling/release client`, then `one update` on each device. `client` includes the Mac's build, made on the Mac over ssh; Linux's alone is `tooling/release one/linux-x86_64 one-cli/linux-x86_64` (about 4 min) |
| A phone's app | Not from here: its network's owner builds it from the network's repo (`app/`) and puts it on the phone (`iphone`). Say one is needed |
| Docs, `tooling/`, `.agents/` | Nothing |

Windows' builds, `server/windows-x86_64` and `one-cli/windows-x86_64`, are made on Linux like the others, and are
built only for a network with a Windows server or device. They haven't yet run on a Windows PC: until
one has been installed and updated as a server on one, a Windows PC isn't made a server of the live network.

A plugin change that uses something new in `core/server/` says so in its `requires`, `"core" => "~> 0.16.0"`, and is
deployed once every server is on that version. A plugin someone deployed by hand isn't redeployed by the server.

On a device that is also a server, `one update` asks the program keeping that server running (`one serve run`) to
take the latest builds, the program's among them, and answers before it has. What it did is in that program's log,
`journalctl --user -u one-server-<key's first 12>` on Linux: `one --version` then says the new build.

A server leaves a plugin deployed by hand as it is, so a plugin deploy is always safe beside a server release: deploy
it even when the server is released too.

One server or edge release on a machine at a time. `tooling/release` waits for another itself; run an edge deploy
under `flock "${XDG_RUNTIME_DIR:-/tmp}/one-release.lock" bun edge.js deploy <address>`. The network's address and
where its Cloudflare token is kept are in Agent Memory, not the repo.

## What is live

- `one builds` gives each server's `build` as `<version>+<count>.<commit>`, and the `edge`'s;
  `one builds | jq -r '.servers[] | .server + " " + .name + " " + .build'` lists the keys `one update --server` takes.
- `git diff --stat <commit> origin/main` is what isn't live on the server.
- `ONE_SUBJECT=network one code --name <plugin>` is a plugin's files as deployed.

## Mistakes already made

- Releasing everything when one plugin changed. A server release is minutes and restarts every server; a plugin is
  seconds.
- Releasing a server build onto storage whose stored names it had renamed. A rename of anything kept (a blob prefix,
  a database, a table) needs a one-time rename of what's stored first; say so in the commit and do it before
  `one update`.
- Releasing from a worktree behind `origin/main`, which `tooling/release` refuses once a newer build is published.
- Releasing a server from a tree with changes not committed: the build is `.dirty`, and every server refuses it
  ("says it's ….dirty, not what was published").
- Changing what servers ask each other (`hooks/call`, `plugins/call`, what a call passed on says in `_meta`)
  without marking it `!`: the servers on the version before read it wrongly while the release reaches them (`This
  device isn't signed in here`, 240 times in three minutes). Marked, they refuse it instead, and are asked nothing
  until they're updated. One that can't take its update for that (`permissions couldn't decide it`) is taken by hand,
  as its program would: download the build `one download --target <kind>` links, unpack it in its
  `server/builds/<build>` past `bin/one_core eval "OneCore.Release.check()"` saying that build, write the build to
  `server/next`, and restart its unit (`systemctl --user restart one-server-<key's first 12>`, or on a Mac
  `launchctl kickstart -k gui/$(id -u)/app.theone.server.<key's first 12>`); it goes back by itself if it doesn't
  answer.
- Deploying the edge before the servers, from a commit they don't run: its relay took them all for gone and server
  updates went back (17 Nov). It now takes those behind it for restarting, which still pauses the network until
  they're updated: the edge goes after the servers.

## Report

What you released, by which command, and the minutes from your commit reaching `main` to `one builds` (or
`one code`) showing it live.
