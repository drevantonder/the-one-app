# Architecture

The map: what lives where, and the few rules the code only shows by keeping them. How each piece works is in the piece, and its tests.

```
 Mac app's window, CLI, MCP, browser, agent hooks, git-remote-one, sidecars    iPhone app, widget
                         \                                                       |
                local HTTP API (src/api.rs)                          one/client/1 over iroh
                                     \                                  /
                       node (src/node.rs) ─── iroh: one/sync/1, one/join/1, one/call/1, one/blob/1
                        │             │                        │
   store (src/store.rs)       apps (src/apps.rs,       sidecars (src/sidecars.rs)
                              src/runtime/, workerd)
                        │
   entry (src/entry.rs) · members (src/members.rs) · sync (src/sync.rs)
```

## Rules

- **Six primitives**: key, log, view, blob, call, app. Scopes, servers, deploys, sidecars, the Vault and backups are made of them.
- **Entries are the truth; views are caches.** Everything anyone does is a signed entry in its author's log for one scope (a user's, a group's, or the install's, `global`). Every view is made by replaying a scope's entries, with `global`'s member entries, in one order, `(time, author, log, seq)`, so the same entries give the same views on every server. An entry, once appended, is never lost.
- **Trust decides who holds what.** A server holds a user's data only if that user trusts it, and a group's only if its owners can read it. No one reads another user's data except by an `open`, which that user sees. Privileged servers are trusted by admins, not built in: any server can be made one, and the install keeps working without them.
- **A key belongs to one install, forever.** Installs (white labels, workplaces) never talk to each other. A key that isn't a current server or client can't sync, call or ask.
- **A call runs once.** It names the scopes it touches before it runs, so a server that can't run it says so before doing anything, and failing over never repeats a side effect. It writes nothing if it fails.
- **Secrets stay put.** A secret field's value leaves the node only inside a request to one of its item's hosts, or a call to a sidecar its hosts name, by an app switched on for that item. Nothing reads one back.
- **The node never names an app or sidecar.** Apps are deployed into the install while it runs, and every app runs its latest deploy, everywhere; sidecars run only where a server's owner installed them.
- **An older device can safely ignore it.** Phones and Macs run behind the servers, so anything machines see changes only in ways an older one can ignore: new fields and kinds are skipped, and old names keep working for as long as something built before sends them.

## Where things are

- `src/entry.rs`: what an entry is, its scope, and the replay order.
- `src/members.rs`: users, groups, servers, clients, trust, apps and installs, from `global`'s entries; which servers hold which scope and which entries count. Pure: entries in, members out.
- `src/sync.rs`: what two servers say to each other. No clock or I/O, so `tests/sim.rs` can drive it.
- `src/store.rs`: every log of every scope a server holds.
- `src/node.rs`: the only place with state and I/O: sessions, pairing, calls, `open`, hooks, blobs, deploys.
- `src/api.rs`: the local HTTP API every shell, the CLI, MCP and sidecars use, and what phones ask over iroh.
- `src/apps.rs`, `src/runtime/`: workerd, which runs every app; `src/runtime/shim.js` is what an app's `ctx` is.
- `src/sidecars.rs`: server sidecars: the ones built in (`src/shell.rs`, `src/agents.rs`, `src/models.rs`), and those that register over the local API (`sidecars/`, in Go, and the Mac app's, `apple/Mac/Sidecars`).
- `src/vault.rs`, `src/outbound.rs`: the Vault's items, and filling them into apps' and sidecars' requests on the way out.
- `src/backups.rs`, `src/node/backups.rs`: backups, each scope's entries and blobs encrypted with its own key (sealed to the recovery key) and sent to a place, a folder or S3-compatible storage, that's a Vault item; and which server backs up what, when.
- `src/schedules.rs`: apps' schedules, and which server runs each.
- `src/deploy.rs`, `src/bundle.rs`: `one deploy`, building an app with a pinned Bun into a bundle that keeps its source.
- `src/screen/`: the design kit and the bridge every app screen is served in. `src/mcp_apps.rs`, `src/mcp_view.html`: widgets and screens drawn inline in chats.
- `src/skills.rs`, `skills/`: what agents are told, as Agent Skills. `src/code.rs`, `src/codemode/`: search, and code mode.
- `src/ops.rs`, `src/main.rs`: the CLI and MCP.
- `src/git.rs`: `git-remote-one`, for the Repos app.
- `src/load.rs`, `src/logos.rs`: how loaded a machine is, and the logos of the sites apps connect to.
- `src/update.rs`, `src/release.rs`: building, signing and publishing the node; tooling, not protocol.
- `client/`: the library phones link. `apple/`: the Mac app (a server's node inside, and the Apple sidecars) and the iPhone app and widget (a client).
- `apps/`: the built-in apps, which every release ships and any install's admins may add. Everything else is deployed by an install's own people.
- `config.toml`: the white label: its name, CLI, port, accent, icons, release key and starter apps. Nothing else names the product.
- `tests/`: `sim.rs`, hundreds of servers over bad links; `e2e.rs`, real nodes; `contracts.rs`, wherever two languages must agree; `agents/`, fresh agents doing everyday tasks.
- `vendor/`: two iroh dependencies patched so the client builds for iPhone; drop them when upstream builds there.

## Designed, not built

- Backups' pruning, failure pushes, weekly restore check and recovery from nothing: a server restoring with no one left to pair with admits itself with the recovery key's countersignature.
- Sidecars a node starts from a `sidecar` entry's executable, rather than registering themselves.
- View snapshots on disk, so a node needn't replay every scope when it starts.
