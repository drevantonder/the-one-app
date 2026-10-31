# Protocol

Version 9. Everything here is what two machines must agree on.

An **instance** is one install of the product (The One App, The PTC App, The Van Tonders App): its own CLI, port, data and key on each machine. Separate instances never talk to each other. Inside an instance:

- **Users** are the people in it. Each is an admin or a member.
- **Servers** run the node and hold entries. A **user server** holds one user's data (the node on someone's Mac). A **cloud server** holds everyone's, kept apart per user (a home tower, a work server). Admins choose which servers are cloud servers; whoever runs one can in principle read everything on it.
- **Clients** hold nothing but a cache and ask a server: the Mac app's window and menu bar, the iPhone app and widget, the CLI, MCP, a browser.
- **Apps** run inside it: JavaScript, in workerd, with tools, screens and widgets.
- **Drivers** are what apps use that can't run everywhere: OS frameworks, hardware, machine access, live sessions, sign-ins. They have no tools of their own.

## Keys

Every server and client has one Ed25519 key. It is its iroh endpoint id and signs everything it writes. A key belongs to exactly one instance, forever: a machine that leaves joins again with a new key.

## Entries

An entry is a p2panda-core operation:

- **Header extensions:** `{ time: u64, user?: String, by?: [u8; 32] }`.
  - `time` is a hybrid clock in milliseconds: never less than the author's previous entry's, and never less than any entry the author had seen.
  - `user` is the user the entry belongs to. Without it, the entry is the instance's own (see "Instance entries").
  - `by` is the client or server an operation ran for, when that isn't the author (a phone's post, written by the server that ran it).
- **Body:** UTF-8 JSON, `{ "app": string, "kind": string, "data": any, "shared"?: true, "blobs"?: [hash] }`. `shared` puts the entry in its app's shared store instead of its user's (see "Apps"); `blobs` lists the blobs it refers to.

Each server writes one log. Its entries are numbered from 0, and each links to the hash of the previous. An entry is valid when its signature verifies, its backlink matches, its author was a server of the instance when it wrote it (see "Members"), and:

- with `user`: the author is one of that user's servers, or a cloud server; `by`, if given, is one of that user's servers or clients;
- without `user`: it is one of the instance entries below, allowed for its author.

Large things never go inside an entry: they go in a blob, which the entry names.

## Where entries go

An entry with a `user` goes to that user's user servers and to every cloud server, nowhere else. Instance entries go to every server. A server holds exactly the entries that should reach it, and passes each on to every other server that should hold it. A cloud server made a user server again deletes the other users' entries, their blobs and their stores; removing a user deletes nothing.

## Order

The **replay order** is `(time, author, seq)`, compared as integers, then key bytes, then integers. Every view is rebuilt by replaying entries in that order, so it is the same on every server that holds the same entries. Views are caches: a server may drop them and replay.

## Instance entries

Instance entries have app `node`; apps never see them. They say who is in the instance, and what runs in it.

### Members

- `genesis { name, user, user_name, server_name }`: the instance's first entry, written by its first server: "this instance is called `name`, and I am a server of `user`, its first admin". Its hash is the instance's **root**; a server knows it from creating the instance or from joining, and ignores every other `genesis`.
- `admit { key, user, name, as, user_name?, role? }`: adds a server (`as: "server"`) or client (`as: "client"`) named `name` for `user`. A member may admit their own servers and clients; only an admin may admit for a new user, which then carries the user's `user_name` and `role` (`admin` or `member`). A removed key is never admitted again.
- `cloud { key, cloud: bool }`: admins only. Makes a server a cloud server, or a user server again.
- `role { user, role }`: admins only.
- `remove { key }` or `remove { user }`: admins only, or a member removing their own server or client. Removing a user removes all their keys. Entries a removed key writes after the `remove` (in replay order) are ignored.
- `rename { key, name }` or `rename { user, name }`: by that key or that user's, or an admin.

A server belongs to the user it was admitted for; a cloud server still does, but holds everyone's entries. A user's **user servers** are their servers that aren't cloud servers.

Apps see only entries whose author was a current server when they were written, and see the names that were current then.

### What runs

- `import { repo, source }`: adds a repo to the instance's catalog, with the git URL it came from. Admins only.
- `push { repo, ref, old, new, pack }`: moves a branch of a repo from commit `old` to `new`; `pack` is a blob holding a git packfile with the objects `new` needs. A push whose `old` isn't the branch's head at that point in replay order is ignored. Any member may push.
- `release { repo, commit, bundle?, drivers?, uses }`: what the instance runs of `repo`, from now on. `bundle` is a blob of the app's bundled JavaScript; `drivers` is `[{ name, target, blob }]`, a build of each driver for each target; `uses` is `{ apps: [name], drivers: [name] }`. A later `release` of an older commit rolls back; `commit: null` uninstalls. Admins only.
- `driver { name, users, why? }`: written by a server when a driver there starts, stops, or changes whom it serves: the users it serves on that server, or, with none, why it can't serve there.
- `node-release` and `running`: see "Releases of the node".

## Sync (ALPN `one/sync/1`)

Between two servers. One bidirectional stream of length-prefixed CBOR messages:

- `Hello { frontier: [(author, next_seq)] }`
- `Entries { entries: [(header, body)] }`

Each side sends `Hello`, then every entry the other lacks and should hold (in batches of at most 200), then new ones as they are written. `Hello` repeats every 30 seconds; a server that receives an entry before the one it follows, or one whose author it doesn't yet know as a server, answers with `Hello` at once, so the gap is filled. Entries timed more than an hour ahead of the receiver's clock are refused. Only current servers may sync, checked on every message; a joining server may sync with the server whose pairing code it used. Receiving an entry twice is harmless.

## Join (ALPN `one/join/1`)

A **pairing code** adds a server or client. It is `{ from: EndpointAddr, secret: [u8; 16] }` in CBOR, encoded as lowercase base32. The server that made it remembers, for each secret, whose it is: the asker's own user's, or a new user's (admins only, with the new user's name and role). The joining machine opens the stream and sends `{ secret, name, as }`; the server appends an `admit` and answers `{ instance, root, user }`. A server then syncs; a client asks. A pairing code works once, for seven days. A key that already belongs to an instance can't join another.

## Calls (ALPN `one/call/1`)

A call runs an operation of an app, or of a driver for an app, for a user. One stream per call: JSON `{ app | driver, op, input, me }`, where `me` is `{ key, user }` of whoever asked, then a JSON reply `{ ok }` or `{ error }`. Only current servers may call each other, and a user server only for its own user's keys; a cloud server may call for anyone.

**Where calls run.** An app's call runs on whichever allowed server answers first: the caller's user servers and every cloud server. If the app has a shared store and the instance has more than one user, only cloud servers are allowed, since only they hold every user's shared entries. A driver's call is allowed on the servers whose newest `driver` entry for it lists the caller's user. To call, a server that is itself allowed runs the call; otherwise it opens a stream to every allowed server it reaches at once, sends `{ ready: { app | driver, op } }`, sends the call to the first to answer `ready`, and closes the rest. A user with no servers of their own (a phone) is served by cloud servers.

## Blobs (ALPN `one/blob/1`)

A blob is immutable bytes named by their BLAKE3 hash: photos, attachments, app bundles, git packs, node binaries. A server fetches the blobs every entry it holds lists, as the entries arrive, and keeps them. To ask, a machine sends a hash (64 hex characters) on one stream and gets the bytes back; it checks them against the hash. A server answers only if an entry it holds lists the blob and the asker may hold that entry or see it (its user's, shared, or the instance's).

## Apps

An app is one ES module, run in workerd (Cloudflare's open-source Workers runtime), whose default export is

```js
{
  name, title, icon, instructions,
  uses: { apps: [name], drivers: [name] },
  schema,                         // SQL run on each user's empty store
  shared,                         // SQL run on the empty shared store, if it has one
  ops: { name: { description, input, run(ctx, input) } },
  on: { kind(db, entry) },        // no clock, no randomness, no I/O
  widgets: { name: { title, render(ctx) } },
  hooks: { name(ctx, { event, since }) },  // returns text for the agent
  screens: { name: { title, html } },      // `main` is the one a shell opens first
}
```

Each app has a private **store** per user, SQLite in its own Durable Object, and, if it declares `shared`, one shared store. A user's store replays that user's entries for the app; the shared store replays the app's `shared` entries from every user. For each entry, in replay order, `on[entry.kind]` gets the store it replays into, as `db.exec(sql, params)` and `db.query(sql, params)`, and the entry `{ id, app, kind, user, name, time, data, blobs }`, where `id` is `<first 8 hex of author>:<seq>` and `name` is the name of `by`, else of the author.

`ctx` is for the user who asked:

- `me`: `{ key, user, name }`, the client or server that asked.
- `now()`: the time.
- `query(sql, params)`: reads that user's store. `shared.query(sql, params)` reads the shared store.
- `append(kind, data, blobs?)`: writes an entry for that user. `share(kind, data, blobs?)` writes one into the shared store.
- `blob.put(bytes)` → hash, and `blob.get(hash)` → bytes.
- `apps.<name>.<op>(input)` and `drivers.<name>.<op>(input)`: calls an app or driver the app declared in `uses`, for the same user. Nothing else is wired to it.
- `fetch`, as in any Worker: the internet is open to every app.

App code never reaches another user's store. Entries an operation appends are written by the server that ran it, with the caller's `user` and, if the caller isn't that server, `by`. A call that takes longer than 30 seconds is stopped.

## Repos and releases

Every app and driver is a git repo. In an instance, a repo's objects are blobs (git packs) and its branches are `push` entries, so every server has it. Git reaches it as `one://apps/<name>` or `one://drivers/<name>` through `git-remote-one`, which talks to the local API.

A repo has `one.toml` at its root, naming what it holds and how to build it:

```toml
app = "imessage"                 # an app, its module at app.js
[drivers.imessage]               # a driver, built on a server of each target
targets = ["aarch64-apple-darwin"]
build = "swift build -c release && cp .build/release/imessage $OUT"
```

An app and the driver it wraps live in one repo, and ship in one release, when they always go together (the iMessage app and its `imessage` driver). A driver many apps use has a repo of its own.

To release a commit, a server installs the app's npm packages and bundles it (with a pinned Bun, downloaded and hash-checked like workerd), and builds each driver on a server of each of its targets, which run its `build` with `$OUT` set; the blobs it makes are named in the `release` entry, signed like every entry by the server that wrote it. Every server loads a new release into workerd without restarting, and starts or stops drivers for its target.

Installing an app releases it and every app and driver it `uses`, by name, from the catalog; it fails, naming what's missing, when the catalog has no repo of that name. `brand.toml` lists the git sources a new instance imports and releases when it's created.

## Drivers

A driver belongs to a user: a call to the `whatsapp` driver from anyone's app reaches the caller's own WhatsApp. It runs on every server it has a build for, as a process its node starts and supervises, and decides in its own code whether it can serve there (is chat.db readable, is Incus present, is a session linked here). It tells its node over the local API, which writes a `driver` entry.

A driver's state is either **portable**, in its user's entries (a Gmail token), so any of the user's servers that runs the driver can serve, or **local**, on the machine where it lives (WhatsApp's linked session), so it serves only there. Like an app, a driver's entries are its own: every copy of it sees the entries of the users its server holds.

Local API, for drivers:

- `POST /v1/drivers/<name> { users }` registers it, serving those users here; `{ why }` says it can't serve here.
- `GET /v1/drivers/<name>/calls` waits up to 25 seconds for a call `{ id, op, input, me }` (204 if none). `POST /v1/drivers/<name>/calls/<id>` answers `{ ok, entries?: [{ kind, data, blobs? }] }` or `{ error }`; the entries are written for the caller's user.
- `GET /v1/drivers/<name>/entries?after=` lists the driver's entries this server holds, for every user, in replay order.

A driver that stops asking for calls for a minute is gone, and its node writes `driver { name, users: [], why }`.

## Releases of the node

A release of the node is an instance entry `node-release { commit, target, blob, size, committed, signature }`, where `target` is a Rust target triple, `blob` holds the binary, `committed` is when the commit was made (seconds since 1970), and `signature` is the publisher's signature over the binary.

What runs is the publisher's to decide, not the instance's: each brand has a **release key**, Ed25519, whose public half every build carries and whose secret only the publisher holds (the publisher's password manager). A signature is OpenSSH's (`ssh-keygen -Y sign -n release`: the armored "sshsig" format, namespace `release`, SHA-512). Any server may publish or pass on a release; who wrote the entry doesn't matter. A node takes the newest release for its target whose signature is by its release key, and ignores newer ones that aren't. If its own build differs and isn't newer, it fetches the blob and checks the signature over it; then, since the entry's other fields aren't signed, it checks that the binary itself says (`version --json`) it's that commit, for its target, and no older than its own build. Only then does it replace its binary and exit; its service manager starts the new one. Nodes inside an app bundle (the Mac app) update with the app instead. Each server also writes `running { commit, target }` once per build, so everyone can see what each runs.

`one update` tests the tip of main and has every server run it; `one release vX.Y.Z` also signs it off (`gh signoff`; there is no CI) and publishes it on GitHub, where each file comes with `<file>.sig`, its signature. The installer checks it with the key it carries, and installs nothing that fails.

## Clients (ALPN `one/client/1`)

A client joins with a pairing code like a server, then asks its user's servers or any cloud server, whichever answers first. Each request is one stream: JSON `{ method, path, body }`, answered with the same JSON the local API gives (below), or `{ error }`. Clients may ask `GET /v1/state`, `GET /v1/apps`, `GET /v1/widgets`, `GET /v1/screen`, `POST /v1/call`, `POST /v1/name`, `POST /v1/pair` and `GET /v1/blobs/<hash>`. `GET /v1/events` instead streams events as JSON lines until the stream closes. Calls run for the client: `ctx.me` is the client. Only current clients may ask.

## Local API

Each server serves HTTP on `127.0.0.1:<brand port>` for its machine's shells, CLI, MCP and drivers, acting for the server's own user. It refuses requests with an `Origin` header or a `Host` other than this machine, so no web page can use it.

- `GET /v1/state`: the brand, this server, and the instance (null until it creates or joins one): its users, and their servers and clients: whether each server is online, whether it's a cloud server, and when each client last asked this server (`asked`, milliseconds, or null).
- `GET /v1/apps`: every installed app's name, title, icon, instructions, operations, widgets and screens.
- `POST /v1/call { app, op, input }` → `{ ok }`.
- `GET /v1/widgets`: every widget, rendered.
- `GET /v1/screens/<app>/<screen>?params=`: an app screen as a page (see "Screens").
- `POST /v1/create { name, user_name }`, `POST /v1/pair { new_user?: { name, role } }` → `{ code }`, `POST /v1/join { code }`, `POST /v1/name { name }`, `POST /v1/remove { key | user }`, `POST /v1/cloud { key, cloud }`, `POST /v1/role { user, role }`. Creating or joining is refused once the server belongs to an instance.
- `GET /v1/catalog`: every repo, its source, branches and release. `POST /v1/import { source }`, `POST /v1/install { repo }`, `POST /v1/release { repo, commit }`.
- `GET /v1/git/<repo>`: the branches and their packs. `POST /v1/git/<repo> { ref, old, new, pack }`: a push, after the pack is a blob.
- `POST /v1/node-release { path, commit, target, committed, signature }`: publishes the binary at `path`, if `signature` is the release key's over it.
- `POST /v1/blobs` with the bytes as the body → `{ blob, size }`. `GET /v1/blobs/<hash>`: a blob, from this server or another.
- `GET /v1/events`: server-sent events `changed { apps }`, `servers` and `instance` (this server created or joined one).
- `POST /hooks/<app>/<name>`: for agent harnesses' hooks. The body is the hook's JSON; the reply is plain text for the agent. The node remembers per `session_id` when the hook last ran and passes it to the app as `since` (null on a session's first call).
- Drivers: see "Drivers".

## Screens

An app's screens are HTML. A server serves each as a whole page: the app's HTML inside the design kit (CSS that follows light and dark and the brand's accent) with a bridge, `one`:

- `one.call(op, input)`: runs one of this app's operations for the client looking, and resolves to its result.
- `one.open(screen, params)`: opens another of this app's screens; the shell pushes it with its own navigation.
- `one.title(text)`: the screen's title, shown by the shell.
- `one.on("changed", fn)`: called when this app's data changes.
- `one.params`, `one.me`, and helpers `one.esc`, `one.ago`, `one.avatar`.

A shell answers the bridge itself: in WKWebView, the page posts `{ type: "call" | "open" | "title", … }` to the `one` message handler and awaits the reply; the shell passes server events to `one.__event(event)`. In a plain browser on a server, the bridge talks to the node directly. Shells get pages from `GET /v1/screens/<app>/<screen>?params=` (local API) or `GET /v1/screen { app, screen, params }` (clients). A page a server served may call the server it came from.

## Widgets

A widget renders to a tree of nodes, drawn natively by each shell and as HTML in chats:

- `{ t: "col" | "row", gap?, c: [nodes] }`
- `{ t: "text", text, size?: "xl" | "lg" | "md" | "sm", weight?: "medium" | "bold", tone?: "muted", lines? }`
- `{ t: "time", at, size?, tone? }`: a time in milliseconds, shown relative to now.

Renderers skip node types and fields they don't know.
