# Architecture

The map: what lives where, and the few rules the code only shows by keeping them. How each piece works is in the piece, and its tests.

```
 Mac app's window, CLI, MCP, browser, agent hooks, git, sidecars              iPhone app, widget
                         \                                                       |
                local HTTP API (src/api.rs)                          one/client/1 over iroh
                                     \                                  /
                       node (src/node.rs) ─── iroh: one/join/1, one/call/1, one/blob/1
                        │             │                        │
   store (src/store.rs)       apps (src/apps.rs,       sidecars (src/sidecars.rs)
                              src/runtime/, workerd)
                        │             │
   entry (src/entry.rs) ·     databases (src/db.rs) in the bucket (src/bucket.rs)
   members (src/members.rs) · replica (src/replica.rs)
```

## Rules

- **Six primitives**: key, log, view, blob, call, app. Scopes, servers, deploys, sidecars, the Vault and backups are made of them.
- **Each app's databases are its data; entries are who can do what.** An app's data in a scope is one SQLite database, the truth, kept sealed in the install's bucket; a call's writes are one transaction, written as one step, so servers writing at once lose nothing. How things went (who used an app, its errors and AI cost, schedules' runs, each server's build, backup runs) is rows of the node's own database of a scope, changed in place. Members, trust, deploys, settings and the rest of the node's state of each scope are signed entries in their author's log for one scope (a user's, a group's, or the install's, `global`), kept sealed in the bucket at numbered positions, with a copy of all of them now and then (`src/log.rs`), and replayed with `global`'s member entries in one order, `(time, author, log, seq)`, so the same entries say the same on every server. An entry, once appended, is never lost.
- **Trust decides who holds what.** A server holds a user's data only if that user trusts it, and a group's only if its owners can read it. No one reads another user's data except by an `open`, which that user sees. Privileged servers are trusted by admins, not built in: any server can be made one, and the install keeps working without them.
- **A key belongs to one install, forever.** Installs (white labels, workplaces) never talk to each other. A key that isn't a current server or client can't call or ask.
- **The bucket is the truth, and each server on a machine keeps a copy.** Servers send each other calls, blobs, a hello (their sidecars and load) and a "something changed" ping, which is only a hint: each reads what changed from the bucket, and reads it anyway now and then. A database's steps are each written once, at a position their seal binds them to, so a server keeps on its disk the steps it reads and writes, and a write's ping carries its step to the servers that hold the scope: a read uses that copy and asks the bucket first only after a ping that didn't carry its step. Every 10 seconds each server fills its copy from the bucket and checks each step another server sent against the bucket's: one that differs is replaced, what read it is read again, and who sent it is shown in Devices, so a server can't show different servers different data for longer than that. Writes go to the bucket, which decides their order.
- **A call runs once.** It names the scopes it touches before it runs, so a server that can't run it says so before doing anything; one that writes takes a lease in the bucket first, so failing over never repeats it. It writes nothing if it fails.
- **Secrets stay put.** A secret field's value leaves the node only inside a request to one of its item's hosts, or a call to a sidecar its hosts name, by an app switched on for that item. Nothing reads one back.
- **The node never names an app or sidecar.** Apps are deployed into the install while it runs, and every app runs its latest deploy, everywhere; sidecars run only where a server's owner installed them.
- **An older device can safely ignore it.** Phones and Macs run behind the servers, so anything machines see changes only in ways an older one can ignore: new fields and kinds are skipped, and old names keep working for as long as something built before sends them.

## Where things are

- `src/entry.rs`: what an entry is, its scope, and the replay order.
- `src/members.rs`: users, groups, servers, clients, trust, apps and installs, from `global`'s entries; which servers hold which scope and which entries count. Pure: entries in, members out.
- `src/replica.rs`: what a server holds and knows: the entries of each scope it holds, as the bucket gives them, and the members they add up to. No clock or I/O.
- `src/store.rs`: every log of every scope a server holds, and how far it has read each from the bucket.
- `src/node.rs`: the only place with state and I/O: reading and writing the bucket, pairing, calls, `open`, hooks, blobs, deploys.
- `src/api.rs`: the local HTTP API every shell, the CLI, MCP and sidecars use, and what phones ask over iroh. A request to `/<app>/…` is the app's `_http` operation, its body a blob: how stock git reaches Repos. A request with a kernel access token (a browser's, through the gateway) is its person's; anything else passed on from elsewhere (by the gateway, a tunnel or a proxy) is refused.
- `src/apps.rs`, `src/runtime/`: workerd, which runs every app; `src/runtime/shim.js` is what an app's `ctx` is.
- `src/sidecars.rs`: server sidecars: the ones built in (`src/shell.rs`, `src/agents.rs`, `src/models.rs`), and those that register over the local API (`sidecars/`, in Go, and the Mac app's, `apple/Mac/Sidecars`).
- `src/vault.rs`, `src/outbound.rs`: the Vault's items, and filling them into apps' and sidecars' requests on the way out.
- `src/db.rs`: an app's database in a scope, kept in the bucket as sealed steps (each write's changeset, or a schema change) at numbered positions and a copy now and then; a write that loses a race catches up and runs again. No clock, disk or network.
- `src/bucket.rs`: objects by name in a place, a folder or S3-compatible storage: created only if absent and replaced only while unchanged, for an install's own bucket; S3's requests go by presigned URL. `src/bucket/copied.rs`: a server's copy of the databases' steps, on its disk.
- `src/node/status.rs`: how things went, as rows of each scope's node database; a schedule's slot runs once, on the server that takes its lease in the bucket.
- `src/node/storage.rs`: where the install keeps its data, and moving it to S3-compatible storage that every server reaches.
- `src/node/tunnel.rs`: the install's Cloudflare Tunnel, which the gateway reaches nodes through with no public address: every server runs a connector for it, so the address reaches whichever is up.
- `src/backups.rs`, `src/node/backups.rs`: backups, each scope's entries and blobs encrypted with its key and a copy of its folder in the bucket, sent to a place, a folder or S3-compatible storage, that's a Vault item; and which server backs up what, when. A scope's key is in the bucket, sealed to each server that holds the scope and to the recovery key.
- `src/schedules.rs`: apps' schedules, and which server runs each.
- `src/deploy.rs`, `src/bundle.rs`: `one deploy`, building an app with a pinned Bun into a bundle that keeps its source; each app's latest is kept in the bucket too, for a node that holds nothing.
- `src/screen/`: the design kit and the bridge every app screen is served in, and widgets drawn as HTML. `settings/` is Settings' screens, every client's own and the same on each, served as app screens are but with no app: they act through the bridge's `one.api`, as the person looking, which a client answers only for them. `src/mcp_apps.rs`, `src/mcp_view.html`: widgets and screens drawn inline in chats.
- `src/skills.rs`, `skills/`: what agents are told, as Agent Skills. `src/code.rs`, `src/codemode/`: search, and code mode.
- `src/ops.rs`, `src/main.rs`: the CLI and MCP, over stdio and at `/mcp`, where MCP clients elsewhere reach a node through the gateway, as the person they signed in as.
- `src/load.rs`, `src/logos.rs`: how loaded a machine is, and the logos of the sites apps connect to.
- `src/update.rs`, `src/release.rs`: building, signing and publishing the node; tooling, not protocol.
- `kernel/`: the kernel's services, JavaScript for workerd, each one Worker with one Durable Object: `auth/` (Better Auth: passkeys only, since only a passkey's PRF output opens keys, joined by invite links; and the OAuth server every client signs in through), `vault/` (every scope's keys and the Vault's items, sealed; keys, filled-in requests and bucket URLs for what a call names), `schedules/` (fires the schedules people let run while they're away, each run once, with only the keys it was let use), and `gateway/`, a Worker with no keys at the install's address: sign-in to auth (passkeys belong to that address), `/v1/` and `/mcp` to a node, stamped so nothing a node says runs at that origin, and the rest to the web client's files, with `src/screen/`'s at `/screen/`. `web/` is the web client, the Mac app's window in a browser: an OAuth client of auth like any other, its code only ever from the kernel since it holds browsers' tokens, each app screen in a sandboxed frame whose bridge it answers. `shared/crypto.js` is the formats nodes read too.
- `serverless/`: the serverless node, a node as a Worker in the kernel's Cloudflare account, which the gateway sends `/v1/` and `/mcp` to by service binding. `worker.js` checks each request's sign-in, asks the vault for the keys and bucket URLs of what a call names (keeping a sign-in, with its keys, until its token expires), runs the app in an isolate of its own (Worker Loader, with `src/runtime/shim.js`), and writes what it wrote; `src/lib.rs` is the node's own Rust (`src/db.rs`, `src/log.rs` and `src/members.rs`, `src/kernel.rs`'s formats, `src/call.rs`, `src/screen.rs`) as WebAssembly. It reads who can do what from the install's log, and each app's latest deploy, from the bucket; an app's requests go out through the vault, which fills in its Vault items. It runs the calls of apps that keep to their own data, and sends the rest on to a node on a machine; when none answers, it answers the install's state, apps, widgets and badges, and runs any app that needs no sidecar, itself. `src/serverless.rs` deploys it; `update` builds its WebAssembly into the release.
- `src/kernel.rs`: the kernel as nodes meet it: the same formats, checking access tokens, asking the vault, and building the services with the pinned Bun and running them in workerd.
- `client/`: the library phones link. `apple/`: the Mac app (a server's node inside, and the Apple sidecars) and the iPhone app and widget (a client).
- `android/`: the Android app, a client that signs in through the kernel's auth service as the web client does (a Custom Tab for the passkey, back at `<cli>://signed-in/`), its refresh token sealed by the phone's keystore, pages and widgets drawn natively, each app screen in a web view of its own whose bridge it answers.
- `windows/`: the Windows app, the Mac app's window on Windows (WinUI 3), a client that signs in through the kernel's auth service as the web client does (in the person's browser, back at a port on the computer, RFC 8252), its refresh token in Windows' Credential Locker, pages and widgets drawn natively, each app screen in a WebView2 of its own whose bridge it answers.
- `apps/`: the built-in apps, which every release ships and any install's admins may add. Everything else is deployed by an install's own people.
- `config.toml`: the white label: its name, CLI, port, accent, icons, release key and starter apps. Nothing else names the product.
- `tests/`: `sim.rs`, many servers writing one database in one bucket over bad links; `e2e.rs`, real nodes; `contracts.rs`, wherever two languages must agree; `agents/`, fresh agents doing everyday tasks.
- `vendor/`: two iroh dependencies patched so the client builds for iPhone; drop them when upstream builds there.

## Being built

The design that replaces privileged servers and replay. As each part lands, the rules and map above are rewritten to match and its line here goes.

- **Kernel services**: auth (sign-in by passkey only, with Better Auth: people join by invite links, and a link carries the keys of someone who lost every passkey, opened with the recovery key; OAuth for MCP clients), vault (every key, sealed; opened only with a client's secret: a passkey's PRF secret, or an MCP client's refresh token) and schedules (fires schedules; holds, sealed to itself, only the keys for work a person let run while they're away). Small JavaScript for workerd, on Cloudflare or any workerd. Not devices. Off the path of a read: the kernel is asked to sign in, to refresh a token, to change who can do what, to sign a write's create-only URL, and by schedules.
- **Nodes**: the Rust node and sidecars, owned and used by a user, a group or everyone (today's acl). On a machine, or stateless on Cloudflare (the node's Rust as WebAssembly, `serverless/`). Any number in one Cloudflare account.
- **Clients**: the iPhone, Mac, Android and Windows apps, browsers and MCP clients like ChatGPT, all signing in through the auth server the same way. What a client has cached stays on it, protected by its OS. A device with a node and its person's sign-in reads offline from its copy; its writes wait for the bucket.
- **Storage**: on Workers, SQLite compiled into the Rust WebAssembly (`src/db.rs` keeps to SQLite and the bucket for it), since the stock JavaScript build generates WebAssembly at runtime, which Workers forbid. Nodes hold no bucket credentials: the kernel signs create-only URLs for what a call names, so nothing they do deletes data. Erasing destroys a key, not the ciphertext. Who can do what (members, trust, deploys, admins) is a small signed log in `global`, in the bucket, kept forever.
- **Keys**: a tree per scope (scope, then app, then database). Nodes on machines hold no scope keys: they ask the vault for a call's, keeping them until its token expires, as the serverless node does. That gives a node nothing more: a node given an access token could open that session at the vault anyway, so a hacked node gets only the sessions that ran on it while it was hacked. The vault records each session's keys where its person sees them. Signing a client out, or removing it, ends its sessions at the vault, so no node gets keys with its tokens again; what a node already has, it keeps until the token expires. Sign-ins like Google's reach a call as access tokens, never refresh tokens.
- **Sidecars** run as their own OS user per scope. WhatsApp's linked device is pinned to one node and moves only by linking again. Shell and Computer Use run only with a grant for that device and session, approved by the person.
- **Goes**: privileged servers, the main server, replay, view snapshots.

## Designed, not built

- Backups' weekly restore check and recovery from nothing: a server restoring with no one left to pair with admits itself with the recovery key's countersignature.
- Sidecars a node starts from a `sidecar` entry's executable, rather than registering themselves.
