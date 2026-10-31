# Architecture

The node runs on every machine that isn't a phone: laptops, home boxes, work machines. Each is a **server**, with owners and users: it holds `global`, what its owners can read, and the data of each user who trusts it. A server the install owns holds everything and serves everyone: it is **privileged**, because admins trust it, not because of any code. Phones are **clients**: they ask a server and keep nothing but a cache. On a server's machine, the CLI, MCP, a browser and the Mac app's window use that server's local API as its local user. Each context (The One App, The PTC App) is its own install, with its own CLI, port, keys and data, so someone in several runs several side by side; installs never talk to each other.

Everything lives in a scope: one user's, a group's, or the install's (`global`). Everything anyone does is a signed entry in the author server's log for that scope, and goes only to the servers that hold that scope. Every view is a cache, made by replaying a scope's entries (and `global`'s member entries) in one shared order. The node knows nothing about any app or driver: apps are deployed into the install while it runs, and drivers are installed on the servers whose owners want them.

```
 Mac app's window, CLI, MCP, browser, agent hooks, git-remote-one, drivers     iPhone app, widget
                         \                                                       |
                local HTTP API (src/api.rs)                          one/client/1 over iroh
                                     \                                  /
                       node (src/node.rs) ─── iroh: one/sync/1, one/join/1, one/call/1, one/blob/1
                        │             │                        │                    │
   store (src/store.rs)       apps (src/apps.rs)       drivers (src/drivers.rs)   backup (src/backup.rs)
   entries per author and     workerd: each deploy     processes it starts, and   entries and blobs to
   scope, blobs, views'       loaded while running;    any that register; which   an S3 bucket, encrypted;
   positions, snapshots       a store per app per      serve here, advertised     restore from one
                              scope; outbound fetch    in sync
                              through secrets.rs
                        │
   entry (src/entry.rs): p2panda-core operations, signed, hash-linked per author and scope
   members (src/members.rs): users, groups, servers' ACLs and trust, clients, apps and installs, from global entries
   sync (src/sync.rs): the protocol, and which scope goes where; no clock or I/O, so tests/sim.rs can drive it
```

## Where to look

- `SPEC.md` is the contract between machines. If two must agree on it, it's there.
- `src/entry.rs`: what an entry is, its scope, and the replay order `(time, author, log, seq)`.
- `src/members.rs`: users, groups, roles, servers, ACLs, trust, clients, `open`s, apps, deploys and installs, and so which servers hold which scope and which entries count. Pure: entries in, members out.
- `src/sync.rs`: what two servers say to each other. It reads and writes the store but has no clock or I/O, so the simulator can drive it.
- `src/store.rs`: every log of every scope this server holds, the blobs they name, and for each view the last entry it replayed. It snapshots each scope's views on disk now and then and keeps the last few; an entry that sorts before a view's position rewinds it to the newest snapshot before the entry, or to empty. Snapshots never leave the machine.
- `src/node.rs`: the only place with state and I/O: sessions, pairing codes, where calls run (nearest first, one at a time), readiness, hooks.
- `src/apps.rs`: workerd, which the node downloads (pinned, hash-checked), configures and supervises. `src/runtime/` is the one Worker the node configures; it loads each deploy's bundle with workerd's dynamic worker loader, so a deploy needs no restart. It holds the Durable Objects, one per app per scope, kept on disk across restarts, and hands each call stubs for exactly the scopes it may reach. `on` handlers see no clock or randomness. Appends are buffered until the call returns.
- `src/secrets.rs`: secret entries, kept apart from every view. Workerd's outbound fetch goes to the node, which fills in placeholders for the request's host and makes the request itself.
- `src/deploy.rs`: `one deploy <dir>`: builds the app with a pinned Bun (downloaded and hash-checked, like workerd), npm packages and all, tars the bundle with its source, uploads it and writes the `deploy`.
- `src/drivers.rs`: starts the drivers installed on this server, takes registrations over the local API, tells sync whom each serves, and passes them calls.
- `src/backup.rs`: streams entries and blobs to a bucket, and restores from one.
- `src/update.rs`, `src/release.rs`: tooling, not protocol. `one update` builds main's tip on your own Linux servers and publishes it as a `node-release`, and rebuilds the Mac app on each Mac over SSH; `one release vX.Y.Z` also signs it off (`gh signoff`; there is no CI) and publishes it on GitHub, each file with `<file>.sig`. `install.sh` installs GitHub's latest, if signed with the brand's release key.
- `src/screen/`: the design kit and the bridge every app screen is served in.
- `client/`: the library phones link (Rust, with a C interface in `one_client.h`).
- `apple/`: the Mac app (a server's node inside, its window and menu bar, and the host of the Apple drivers, built in from the `apple-drivers` repo so they keep the permissions macOS grants the app) and the iPhone app and widget (a client). They draw apps' screens and widgets and name no app.
- `brand.toml`: the product's name, CLI name, port, accent, release key, what it calls its privileged servers, and the bundles a new install deploys and installs in `global`. Nothing else names the product. `brands/` has the others.
- `vendor/`: two iroh dependencies patched so the client builds for iPhone; drop them when upstream builds there.

## Apps it starts with

This repo contains no apps and no drivers. Each app is deployed from wherever its code lives: a repo in the install, on GitHub, or nowhere. CI, if any, is an app or agent that deploys.

- **Board**, **Agents**, **Shell**, **Models**, **Desktops**, **WhatsApp**, and the Apple apps (iMessage, Reminders, Calendar, Contacts): apps, with drivers where they need native code (a shell, a local model, Incus, WhatsApp's linked session, macOS frameworks). The Apple drivers live in the Mac app.
- **Gmail**: a pure app: the internet and a secret, no driver.
- **Repos**: git hosting as an ordinary app, installed in `global`, so every repo is visible to everyone. Its entries are `repo { name, owner }` and `push { repo, branch, base, head }` with a pack blob. A branch moves iff `base` is its current head; no git runs in the app. Only the repo's owner (a user, a group, or admins) moves `main` and other bare branches; anyone pushes their own (`main@bob`), as suggestions. Compaction, if a repo ever needs it, is app code. `git-remote-one`, a small helper shipped with the app, runs on a machine with a node: local git builds the packs, and the helper uploads them as blobs and calls the app through the local API; fetching downloads the packs and indexes them locally. Repo pages are the app's screens.

## Invariants

- Once servers can reach each other again, each holds exactly the scopes it should, and the same entries give the same views on every server (`tests/sim.rs`).
- An entry, once appended, is never lost: any server that holds it passes it on to every server that should hold it.
- A server holds a user's own data only if that user trusts it, and a group's only if its owners can read it.
- No one reads another user's data except by an `open`, which that user sees.
- A call never runs on two servers, reaches only the scopes its user may use, and writes nothing if it fails.
- A key that isn't a current server or client can't sync, call or ask.
- Every app runs its latest deploy, everywhere. A driver runs on a server only if one of its owners installed it.
- A secret's value leaves the node only inside a request to one of its hosts.
- Privileged servers are trusted by admins, not built in: any server can be made one, and the install keeps working without them.
- The node never names an app or driver.
