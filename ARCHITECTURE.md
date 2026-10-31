# Architecture

A node runs on every device that can keep a log: laptops, the home box, servers. Phones and browsers are clients of a node. Everything a user or agent does is a signed entry appended to their device's log; every view anyone sees is rebuilt by replaying every log in one shared order.

```
 shells (apple/)  CLI  MCP  agent hooks          phones (client/)
        \          |    |     /                       |
         local HTTP API (src/api.rs)       one/client/1 over iroh
                    \                             /
                     node (src/node.rs) ── iroh: one/sync/1, one/join/1, one/call/1
                      │         │
          store (src/store.rs)  apps (src/apps.rs): workerd; a Durable Object per app and space
          entries, per space    on handlers, operations, widgets, hooks
                      │
          entry (src/entry.rs): p2panda-core operations, signed, hash-linked per device
          space (src/space.rs): membership, replayed from `space` entries
          sync (src/sync.rs): the protocol, with no clock or network; tests/sim.rs runs it on a hostile simulated network
```

## Where to look

- `SPEC.md` is the contract between devices. If two devices must agree on it, it's there.
- `src/entry.rs`: what an entry is, and the replay order `(time, author, seq)`.
- `src/space.rs`: who's in a space. Pure: entries in, members out.
- `src/sync.rs`: what two devices say to each other. It reads and writes the store but has no clock or network, so the simulator can drive it.
- `src/node.rs`: the only place with state and I/O: stores, sessions, pairing codes, calls, hooks.
- `src/apps.rs`: apps run in workerd, which the node downloads (pinned, hash-checked), configures and supervises. `src/runtime/shim.js` makes each app a Worker with one Durable Object per space holding its views in SQLite; its `on` handlers see no clock or randomness, and operations can only append entries.
- `src/builtin.rs`: the apps this build ships with, handed to the node by the binary; the core names none.
- `src/native.rs` (the type), `src/shell.rs`, `src/agents.rs`, `src/models.rs`: device features in Rust. `src/providers.rs`: the same for features written outside the node (the Mac app's Swift, Go), over the local API.
- `src/update.rs`: `one update` and `one release`, which build main's tip on your own devices (the Macs build their app through the shell app) and publish it as a release; `install.sh` installs GitHub's latest.
- `apps/`: the apps every build ships with. They use nothing a user's app couldn't.
- `client/`: the library phones link (Rust, with a C interface in `one_client.h`).
- `apple/`: the Mac app now, and the iPhone app's shared views. The Mac app provides Reminders, Calendar, Contacts and iMessage (`apple/Mac/Providers`), since macOS grants their permissions to it.
- `brand.toml`: the product's name, CLI name, port and accent. Nothing else names the product.
- `vendor/`: two iroh dependencies patched so the client builds for iPhone; drop them when upstream builds there.

## Invariants

- The same entries in the same order give the same views on every device (`tests/sim.rs`).
- An entry, once appended, is never lost: any device that held it passes it on.
- A device that isn't a current member can't sync, call or ask.
- The core never names an app.
