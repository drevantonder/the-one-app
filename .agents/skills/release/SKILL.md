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

- Parts one 0.minor apart talk, and two or more apart don't (`OneCore.Builds.talk?/2`). A server refuses a request
  of one that far off (`OneCore.Mcp`), which says why; a server asks one that far behind it nothing, as if it were
  off; the relay passes the edge's calls only to servers within one of its own (`core/edge/relay/relay.ts`), apart
  from whether they're ready. What updates a
  part that's behind, `tools/list`, `builds`, `download` and `update`, is answered whatever it's on. `one builds` says each
  server's `build`, the `edge`'s, and `why` one doesn't talk.
- An app runs on a server on its `requires`' `"core" => ">= 0.16.0"` or newer (`OneCore.Apps.Version.platform/1`,
  worked out from the two versions, never kept): `runs`, or `waiting_for_server` on an older one (deploying it there
  is refused, saying so). `one apps` and `one builds` say which, and a server that can't run one says why in
  telemetry.

## Order

One release at a time: `tooling/release` refuses while another is under way, and says which; then say whose it is
and stop. A server the network hasn't seen for 10 minutes (`asleep` in it), or one not ready with no update
under way, holds up none: it's named, and takes the latest by itself as it starts or reconnects. A build it went
back from it tries once more, 10 minutes on, and after that only when asked (`one update --server`). `tooling/release` updates each server itself once it has
published, in turn, saying what each took; the first that goes back stops the rest, which stay on the old build, safe.
Run it again to take on the servers still behind. Run no `one update --server` by hand for a release. Nothing is watched after a release: each server verifies a build answers before it keeps it, and a bug
found later is fixed forward.

Any release mixes with what's live, a breaking one too, since parts one 0.minor apart talk. Release the servers
first, then the edge from the commit they run, then the clients; an app that requires the new platform is deployed
once every server is on it. Release the next breaking change only once every part is on this one: two apart don't
talk.

| Changed | Release |
| --- | --- |
| An app (`apps/<name>/`, here or in a network's repo) | `ONE_SUBJECT=network one deploy --name <name> --files-from apps/<name>`, from that repo, into each network it's released to (about 12 s). Refused, someone else's newer change is live: rebase and deploy again, never `--force` |
| An app with a NIF (`server/nif/`) | `tooling/nifs`, then the same deploy. A Mac's NIF whose inputs haven't changed is the one last published (or cached here); one that changed is built on the Mac, which must be awake |
| An app's native code (`server/nif/`, `server/bin/`) | Also `tooling/publish-nifs` once it's on `main`: it publishes the new builds to a GitHub Release and pushes their `server/checksums.json`, from which every network updating the app from git gets them. A Mac's are built on GitHub's macOS runner when the Mac is busy |
| The UI library (`core/ui/`) | The server, which ships it and builds every app again against it, then the edge, which serves it to browsers |
| The edge (`core/edge/`, an app's `edge/`, `clients/web/`) | `bun install && bun edge.js deploy <the network's address>` in `core/edge/`, with the Cloudflare token in `CLOUDFLARE_API_TOKEN` |
| The server (`core/server/`, or Rust it builds with) | `tooling/release server`, which updates each server after publishing and fails saying why one went back (within the 180 s a start has, and a minute). A build that only changed One Core's code is loaded into each server while it runs (`update_method` `hot_load`, seconds); a changed NIF of One Core's is loaded beside the one before, and what calls it started again on it; one with a runtime built from anything else (`runtime-inputs`), messages `one` doesn't speak (`client::serve`), 0.minor, supervisor, struct, macro or `-on_load` module restarts each (`restart: <why>`, as before). A new `one` alone restarts nothing: it's put in place as its build is loaded, and runs the server from its next start |
| The desktop app or CLI (`core/client/`, `clients/`, an app's `client/native/`) | `tooling/release client`, then `one update` on each device. `client` includes the Mac's build, made on the Mac over ssh; Linux's alone is `tooling/release one/linux-x86_64 one-cli/linux-x86_64` (about 4 min) |
| A phone's app | Not from here: its network's owner builds it from the network's repo (`app/`) and puts it on the phone (`iphone`). Say one is needed |
| Docs, `tooling/`, `.agents/` | Nothing |

Windows' builds, `server/windows-x86_64` and `one-cli/windows-x86_64`, are made on Linux like the others, and are
built only for a network with a Windows server or device. They haven't yet run on a Windows PC: until
one has been installed and updated as a server on one, a Windows PC isn't made a server of the live network.

An app change that uses something new in `core/server/` says so in its `requires`, `"core" => ">= 0.16.0"`, and is
deployed once every server is on that version.

A server release deploys no apps. This repo's apps are published as any author's are: deployed, each that
changed, into each network the release serves, the owner's (its address is in Agent Memory), where they're the
network's own trusted apps, so every install of them takes the new version at once. A new network gets them from
whoever makes it (`tooling/network`).

On a device that is also a server, `one update` asks the program keeping that server running (`one serve run`) to
take the latest builds, the program's among them, and answers as `one update --server` does. What it did is in that
program's log, `journalctl --user -u one-server-<key's first 12>` on Linux.

Run an edge deploy only while no release publishes on the machine, which `tooling/release` keeps as a pid file:
`! { read -r pid _ <"${XDG_RUNTIME_DIR:-/tmp}/one-release.pid" && kill -0 "$pid"; } 2>/dev/null && bun edge.js deploy <address>`. The network's address and
where its Cloudflare token is kept are in Agent Memory, not the repo.

## What is live

- `one builds` gives each server's `build` as `<version>+<count>.<commit>`, and the `edge`'s;
  `one builds | jq -r '.servers[] | .server + " " + .name + " " + .build'` lists the keys `one update --server` takes.
- `git diff --stat <commit> origin/main` is what isn't live on the server.
- `ONE_SUBJECT=network one code --name <app>` is an app's files as deployed.

## Mistakes already made

- A breaking release that apps' code can't run on (the plugin-to-app rename, 17 Nov) leaves sign-in not running on
  each server it reaches, so nobody is signed in there and nothing can be deployed to it: keep sign-in's new version
  first with the new build's own `OneCore.Apps.Store.keep/6` on a server's machine (as the bucket is reached without
  the node) and restart that server.
- Releasing everything when one app changed. A server release is minutes and restarts every server; an app is
  seconds.
- Releasing a server build onto storage whose stored names it had renamed. A rename of anything kept (a blob prefix,
  a database, a table) needs a one-time rename of what's stored first; say so in the commit and do it before
  `one update`.
- Releasing from a worktree behind `origin/main`, which `tooling/release` refuses once a newer build is published.
- Releasing a server from a tree with changes not committed: the build is `.dirty`, and every server refuses it
  ("says it's ….dirty, not what was published").
- Changing what servers ask each other (`hooks/call`, `apps/call`, what a call passed on says in `_meta`) so the
  version before reads it wrongly while the release reaches them (`This device isn't signed in here`, 240 times in
  three minutes): keep it compatible for one release, the new beside the old. A server that can't take its update
  (`permissions couldn't decide it`) is taken by hand, as its program would: download the build `one download --target <kind>` links, unpack it in its
  `server/builds/<build>` past `bin/one_core eval "OneCore.Release.check()"` saying that build, set `next` to
  `{"build": <it>, "program": <its one --version>}` and `server_update_status` to `verifying` in `server/state.json`,
  and restart its unit (`systemctl --user restart one-server-<key's first 12>`, or on a Mac
  `launchctl kickstart -k gui/$(id -u)/app.theone.server.<key's first 12>`); it goes back by itself if it doesn't
  answer.
- Deploying the edge before the servers, from a commit they don't run: its relay took them all for gone and server
  updates went back (17 Nov). It now sends nothing only to those more than one 0.minor behind it; the edge still
  goes after the servers.
- A breaking release that removed two app categories (17 Nov), when parts talked only on one 0.minor and apps
  pinned one with `~>`: the network split in two and six apps waited for updates. Now parts talk across one, apps
  say a minimum, and a removal is a deprecation.

## Report

What you released, by which command, and the minutes from your commit reaching `main` to `one builds` (or
`one code`) showing it live.
