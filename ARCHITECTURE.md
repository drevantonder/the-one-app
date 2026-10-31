# Architecture

The node runs on every machine that isn't a phone: laptops, home boxes, work machines. Each is a **server**: a user server holds what its user may see, and a cloud server holds everything. Phones are **clients**: they ask a server and keep nothing but a cache. On a server's machine, the CLI, MCP, a browser and the Mac app's window use that server's local API as its user. Each context (The One App, The PTC App) is its own install, with its own CLI, port, keys and data, so someone in several runs several side by side; installs never talk to each other.

Everything lives in a scope: one user's, a group's, or the install's (`global`). Everything anyone does is a signed entry in the author server's log for that scope, and goes only to the servers that should hold that scope. Every view is a cache, made by replaying a scope's entries in one shared order. The node knows nothing about any app or driver: they are git repos inside the install, released and loaded while it runs.

```
 Mac app's window, CLI, MCP, browser, agent hooks, git-remote-one, drivers     iPhone app, widget
                         \                                                       |
                local HTTP API (src/api.rs)                          one/client/1 over iroh
                                     \                                  /
                       node (src/node.rs) ─── iroh: one/sync/1, one/join/1, one/call/1, one/blob/1
                        │             │                        │                    │
   store (src/store.rs)       apps (src/apps.rs)       drivers (src/drivers.rs)   backup (src/backup.rs)
   entries per author and     workerd: each release    processes it starts, and   entries and blobs to
   scope, blobs, views'       loaded while running;    any that register; which   an S3 bucket, encrypted;
   positions and checkpoints  a store per app per      serve here, advertised     restore from one
                              scope, kept on disk      in sync
                        │
   entry (src/entry.rs): p2panda-core operations, signed, hash-linked per author and scope
   members (src/members.rs): users, groups, servers, clients, the recovery key, replayed from global entries
   sync (src/sync.rs): the protocol, and which scope goes where; no clock or I/O, so tests/sim.rs can drive it
   repos (src/repos.rs): pushes and packs, branches replayed from them, releases built and named here
```

## Where to look

- `SPEC.md` is the contract between machines. If two must agree on it, it's there.
- `src/entry.rs`: what an entry is, its scope, and the replay order `(time, author, seq)`.
- `src/members.rs`: users, groups, roles, servers, clients, cloud servers and the recovery key, and so which servers hold which scope. Pure: entries in, members out.
- `src/sync.rs`: what two servers say to each other. It reads and writes the store but has no clock or I/O, so the simulator can drive it.
- `src/store.rs`: every log of every scope this server should hold, the blobs they name, and for each view the last entry it replayed and its checkpoints.
- `src/node.rs`: the only place with state and I/O: sessions, pairing codes, where calls run, hooks.
- `src/apps.rs`: workerd, which the node downloads (pinned, hash-checked), configures and supervises. `src/runtime/host.js` is the one Worker the node configures; it loads each release's bundle with workerd's dynamic worker loader, so a release needs no restart. The host holds the Durable Objects, one per app per scope, kept on disk across restarts, and hands each call stubs for exactly the scopes its user may use. `on` handlers see no clock or randomness.
- `src/drivers.rs`: starts the drivers released for this server's target and allowed here, takes their registrations and any other's over the local API, tells sync whom each serves, and passes them calls and their entries.
- `src/repos.rs`: imports from git URLs, pushes, compactions, the branches they replay to, and releases. Releasing bundles the app with a pinned Bun (npm packages and all) and builds its drivers for this server's target.
- `src/git.rs`: `git-remote-one`, the same binary under another name; git runs it for `one://` URLs, and it speaks git's smart protocol to the local API, which serves each repo from a bare repo rebuilt from its entries.
- `src/backup.rs`: streams entries and blobs to a bucket, and restores from one.
- `src/update.rs`, `src/release.rs`: `one update` and `one release` for the node itself: build main's tip on your own servers (and the Mac app on each Mac, over SSH) and publish it signed with the brand's release key; `install.sh` installs GitHub's latest, if signed with it.
- `src/screen/`: the design kit and the bridge every app screen is served in.
- `client/`: the library phones link (Rust, with a C interface in `one_client.h`).
- `apple/`: the Mac app (a user server's node inside, its window and menu bar, and the host of the Apple drivers, built in from their own repo so they keep the permissions macOS grants the app) and the iPhone app and widget (a client). They draw apps' screens and widgets and name no app.
- `brand.toml`: the product's name, CLI name, port, accent, release key, and the git sources of the apps a new install starts with. Nothing else names the product. `brands/` has the others.
- `vendor/`: two iroh dependencies patched so the client builds for iPhone; drop them when upstream builds there.

This repo contains no apps and no drivers. The board, agents, shell, models, Gmail, WhatsApp, desktops, and the Apple apps (iMessage, Reminders, Calendar, Contacts) each live in their own repo. An app and its driver share a repo, and a release, when they always go together; a driver many apps use has its own.

## Invariants

- Once servers can reach each other again, each holds exactly the scopes it should, and the same entries give the same views on every server (`tests/sim.rs`).
- An entry, once appended, is never lost: any server that holds it passes it on to every server that should hold it.
- A user server holds only what its user may see.
- A call never runs twice, and its code reaches only the scopes its user may use.
- A key that isn't a current server or client can't sync, call or ask.
- What runs is always an explicit release, never a branch head.
- The node never names an app or driver.
