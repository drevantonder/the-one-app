# Architecture

The node runs on every server: laptops, home boxes, work machines. Clients (the Mac and iPhone apps, the CLI, MCP, a browser) ask a server and keep nothing but a cache. Each instance of the product (The One App, The PTC App) is its own install, with its own CLI, port, key and data, so someone in several runs several side by side; instances never talk to each other.

Everything a user or agent does is a signed entry. An entry goes to its user's user servers and to every cloud server; every view is rebuilt by replaying entries in one shared order. The node knows nothing about any app: apps and drivers are repos inside the instance, released and loaded while it runs.

```
 Mac app, iPhone app, widget   CLI   MCP   agent hooks   git-remote-one
            \                   |     |       /             /
   one/client/1 over iroh     local HTTP API (src/api.rs)
                 \                    |
                  node (src/node.rs) ─── iroh: one/sync/1, one/join/1, one/call/1, one/blob/1
                   │          │                     │
     store (src/store.rs)   apps (src/apps.rs)     drivers (src/drivers.rs)
     the entries this       workerd: each release   processes the node starts;
     server should hold,    loaded while running,   each says whom it serves here
     and blobs              a store per user        and answers calls
                   │
     entry (src/entry.rs): p2panda-core operations, signed, hash-linked per server
     members (src/members.rs): who's in the instance, replayed from its entries
     sync (src/sync.rs): the protocol, and which entries go where; no clock or I/O, so tests/sim.rs can drive it
     repos (src/repos.rs): git packs as blobs, branches as entries; releases built and named here
```

## Where to look

- `SPEC.md` is the contract between machines. If two must agree on it, it's there.
- `src/entry.rs`: what an entry is, and the replay order `(time, author, seq)`.
- `src/members.rs`: users, their servers and clients, roles, cloud servers. Pure: entries in, members out.
- `src/sync.rs`: what two servers say to each other, and who should hold which entry. It reads and writes the store but has no clock or I/O, so the simulator can drive it.
- `src/node.rs`: the only place with state and I/O: the store, sessions, pairing codes, where calls run, hooks.
- `src/apps.rs`: workerd, which the node downloads (pinned, hash-checked), configures and supervises. `src/runtime/host.js` is the one Worker the node configures; it loads each release's bundle with workerd's dynamic worker loader, so a release needs no restart. The host holds the Durable Objects, one per app per user and one shared per app, and hands each request stubs for exactly the caller's store and the shared one, so app code can't name another user's. `on` handlers see no clock or randomness.
- `src/drivers.rs`: starts the drivers released for this server's target, takes their registrations over the local API, writes `driver` entries, and passes them calls and their entries.
- `src/repos.rs`: the catalog: imports from git sources, pushes, and releases. Releasing bundles the app with a pinned Bun (npm packages and all) and asks a server of each target to build its drivers.
- `src/git.rs`: `git-remote-one`, the same binary under another name; git runs it for `one://` URLs and it speaks git's remote-helper protocol to the local API.
- `src/update.rs`, `src/release.rs`: `one update` and `one release` for the node itself: build main's tip on your own servers and publish it signed with the brand's release key; `install.sh` installs GitHub's latest, if signed with it.
- `client/`: the library phones link (Rust, with a C interface in `one_client.h`).
- `apple/`: the Mac app (a client, with a user server's node inside) and the iPhone app and widget. They draw apps' screens and widgets and name no app.
- `brand.toml`: the product's name, CLI name, port, accent, release key, and the git sources of the apps a new instance starts with. Nothing else names the product.
- `vendor/`: two iroh dependencies patched so the client builds for iPhone; drop them when upstream builds there.

Apps and drivers live in their own repos, never here: the board, shell, agents and models, Gmail, WhatsApp, desktops, and the Apple ones (iMessage, Reminders, Calendar, Contacts). An app and its driver share a repo when they always go together.

## Invariants

- Once servers can reach each other again, each holds exactly the entries it should, and the same entries give the same views on every server (`tests/sim.rs`).
- An entry, once appended, is never lost: any server that holds it passes it on to every server that should hold it.
- A server holds only its user's entries unless it's a cloud server.
- App code running for one user never reads another user's store.
- A key that isn't a current server or client can't sync, call or ask.
- The node never names an app or driver.
