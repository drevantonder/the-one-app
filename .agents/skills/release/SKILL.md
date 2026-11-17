---
name: release
description: Use once your change is on main, to put it live on the network by the smallest path it needs, or to see what is live. Whoever makes a change releases it.
---

Release from `main` as it is now, never an older commit, and only what your change touched.

A release is part of your change: it isn't done until it's live.

## Versions

The platform (One Core's server, the UI library, the edge, the clients and what they say to each other) has one
version, and each app one of its own, Semantic Versioning's. `tooling/version` says the platform's, and
`tooling/version apps/<name>` an app's, from the Conventional Commits that touch that part's paths, as
release-please's manifest mode does: a breaking change (`type!:` or a `BREAKING CHANGE:` footer) takes the next
minor, a `feat` or a `fix` the next patch, and a commit touching two parts moves each. Before 1.0 a minor is
breaking, as Cargo, npm and Hex read 0.x. A build is that version with its commit's count and hash as build
metadata, `0.18.2+2974.febd37350abc`. Nothing decides a bump but how commits are marked and what they touch: `!`
marks what breaks whoever uses the part the commit changes. One that wasn't marked is marked by a commit after it
that touches the same part, `fix!: <what broke>`, before it's released.

Exactly two rules hold at runtime:

- Parts talk only on the same 0.minor (`OneCore.Builds.talk?/2`). A server refuses a request of another
  (`OneCore.Mcp`), which says why; a server asks one behind it nothing, as if it were off; the relay passes the edge's
  calls only to servers on its own (`core/edge/relay/relay.ts`), apart from whether they're ready. What updates a
  part that's behind, `tools/list`, `builds`, `download` and `update`, is answered whatever it's on. `one builds` says each
  server's `build`, the `edge`'s, and `why` one doesn't talk.
- An app runs only on a server its `requires`' `"core" => "~> 0.16.0"` allows (`OneCore.Apps.Version.platform/1`,
  worked out from the two versions, never kept): `runs`, `waiting_for_server` (deploying it to an older one is
  refused, saying so) or `waiting_for_app` (a newer server: "gmail needs an update for 0.17"). `one apps`
  and `one builds` say which, and a server that can't run one says why in telemetry.

## Order

One release at a time. Before publishing a server or a client, look for another under way: a build in `one builds`'
`latest` that not every server or device runs yet, or the release lock held (`${XDG_RUNTIME_DIR:-/tmp}/one-release.lock`
names its build). If there's one, don't publish: say whose it is, from its commit's author and Agent Memory, and stop.

A release is under way while it publishes (`tooling/release` holds the lock) and rolls out (`one update` until every
server and device runs it), and no longer. Nothing is watched after it: each server verifies a build answers before it
keeps it, and a bug found later is fixed forward.

A patch release mixes with what's live, in any order. A breaking one goes in this order, back to back, so the network
is split only for minutes:

1. The servers (`tooling/release server`, then one server first and the rest at once, as below). A first server that
   goes back stops the release: read its log, and update no other.
2. The apps that require the new platform, once every server is on it.
3. The edge, from the commit the servers run.
4. The clients (`tooling/release client`, `one update` on each device).

| Changed | Release |
| --- | --- |
| An app (`apps/<name>/`, here or in a network's repo) | `ONE_SUBJECT=network one deploy --name <name> --files-from apps/<name>`, from that repo, into each network it's released to (about 12 s). Refused, someone else's newer change is live: rebase and deploy again, never `--force` |
| An app with a NIF (`server/nif/`) | `tooling/nifs`, then the same deploy. A Mac's NIF whose inputs haven't changed is the one last published (or cached here); one that changed is built on the Mac, which must be awake |
| An app's native code (`server/nif/`, `server/bin/`) | Also `tooling/publish-nifs` once it's on `main`: it publishes the new builds to a GitHub Release and pushes their `server/checksums.json`, from which every network updating the app from git gets them. A Mac's are built on GitHub's macOS runner when the Mac is busy |
| The UI library (`core/ui/`) | The server, which ships it and builds every app again against it, then the edge, which serves it to browsers |
| The edge (`core/edge/`, an app's `edge/`, `clients/web/`) | `bun install && bun edge.js deploy <the network's address>` in `core/edge/`, with the Cloudflare token in `CLOUDFLARE_API_TOKEN` |
| The server (`core/server/`, or Rust it builds with) | `tooling/release server`, then `one update --server <first>` for one server, and once it answers `up_to_date` on the new build, `one update --server <key>` for each of the rest at once. A build that only changed One Core's code is loaded into each server while it runs (`update_method` `hot_load`, seconds, answered once done, except where `OneCore.Server`, which holds its connections, changed: that answers that it went away, and `one builds` says how it went); one with a new runtime, `one`, 0.minor, supervisor, struct, macro or `-on_load` module restarts each (`restart: <why>`, as before) |
| The desktop app or CLI (`core/client/`, `clients/`, an app's `client/native/`) | `tooling/release client`, then `one update` on each device. `client` includes the Mac's build, made on the Mac over ssh; Linux's alone is `tooling/release one/linux-x86_64 one-cli/linux-x86_64` (about 4 min) |
| A phone's app | Not from here: its network's owner builds it from the network's repo (`app/`) and puts it on the phone (`iphone`). Say one is needed |
| Docs, `tooling/`, `.agents/` | Nothing |

Windows' builds, `server/windows-x86_64` and `one-cli/windows-x86_64`, are made on Linux like the others, and are
built only for a network with a Windows server or device. They haven't yet run on a Windows PC: until
one has been installed and updated as a server on one, a Windows PC isn't made a server of the live network.

An app change that uses something new in `core/server/` says so in its `requires`, `"core" => "~> 0.16.0"`, and is
deployed once every server is on that version.

A server release deploys no apps. This repo's apps are published as any author's are: deployed, each that
changed, into each network the release serves, the owner's (its address is in Agent Memory), where they're the
network's own trusted apps, so every install of them takes the new version at once. A new network gets them from
whoever makes it (`tooling/network`).

On a device that is also a server, `one update` asks the program keeping that server running (`one serve run`) to
take the latest builds, the program's among them, and answers before it has. What it did is in that program's log,
`journalctl --user -u one-server-<key's first 12>` on Linux: `one --version` then says the new build.

One server or edge release on a machine at a time. `tooling/release` waits for another itself; run an edge deploy
under `flock "${XDG_RUNTIME_DIR:-/tmp}/one-release.lock" bun edge.js deploy <address>`. The network's address and
where its Cloudflare token is kept are in Agent Memory, not the repo.

## What is live

- `one builds` gives each server's `build` as `<version>+<count>.<commit>`, and the `edge`'s;
  `one builds | jq -r '.servers[] | .server + " " + .name + " " + .build'` lists the keys `one update --server` takes.
- `git diff --stat <commit> origin/main` is what isn't live on the server.
- `ONE_SUBJECT=network one code --name <app>` is an app's files as deployed.

## Mistakes already made

- Releasing everything when one app changed. A server release is minutes and restarts every server; an app is
  seconds.
- Releasing a server build onto storage whose stored names it had renamed. A rename of anything kept (a blob prefix,
  a database, a table) needs a one-time rename of what's stored first; say so in the commit and do it before
  `one update`.
- Releasing from a worktree behind `origin/main`, which `tooling/release` refuses once a newer build is published.
- Releasing a server from a tree with changes not committed: the build is `.dirty`, and every server refuses it
  ("says it's ….dirty, not what was published").
- Changing what servers ask each other (`hooks/call`, `apps/call`, what a call passed on says in `_meta`)
  without marking it `!`: the servers on the version before read it wrongly while the release reaches them (`This
  device isn't signed in here`, 240 times in three minutes). Marked, they refuse it instead, and are asked nothing
  until they're updated. One that can't take its update for that (`permissions couldn't decide it`) is taken by hand,
  as its program would: download the build `one download --target <kind>` links, unpack it in its
  `server/builds/<build>` past `bin/one_core eval "OneCore.Release.check()"` saying that build, set `next` to
  `{"build": <it>, "program": <its one --version>}` and `server_update_status` to `verifying` in `server/state.json`,
  and restart its unit (`systemctl --user restart one-server-<key's first 12>`, or on a Mac
  `launchctl kickstart -k gui/$(id -u)/app.theone.server.<key's first 12>`); it goes back by itself if it doesn't
  answer.
- Deploying the edge before the servers, from a commit they don't run: its relay took them all for gone and server
  updates went back (17 Nov). It now sends those behind it nothing, which still pauses the network until they're
  updated: the edge goes after the servers.

## Report

What you released, by which command, and the minutes from your commit reaching `main` to `one builds` (or
`one code`) showing it live.
