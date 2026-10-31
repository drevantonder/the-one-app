# Protocol

Version 9. Everything here is what two machines must agree on.

An **install** is the product as one context runs it (The One App, The PTC App, The Van Tonders App), with its own CLI, port, API and data on each machine. Installs never talk to each other. Inside an install:

- **Users** are the people in it. Each is an admin or a member. **Groups** are sets of users; each member of a group is its admin or a member.
- **Servers** run the node. A **user server** holds only what its user may see; a **cloud server** holds everything. There may be any number of each. Admins choose the cloud servers, and that is the trust decision: whoever runs a cloud server can in principle read what it holds.
- **Clients** are machines without a node (phones). They hold only a cache and ask a server.
- **Apps** run on every server: JavaScript, in workerd. Only apps have tools, screens and widgets.
- **Drivers** are native capabilities apps use: OS frameworks, hardware, shell and files, live sessions, sign-ins.

Every server belongs to one user, the one it was added for. Everything on its machine (CLI, MCP, agents, a browser, the Mac app's window) uses its local API and acts as that user, on a cloud server too.

## Keys

Every server and client has one Ed25519 key. It is its iroh endpoint id and signs everything it writes. A key belongs to exactly one install, forever, so no entry, message or pairing code names the install. A machine that leaves joins again with a new key.

The **recovery key** is one more admin key, made with the install and kept only in the admins' 1Password (see "Recovery").

## Scopes

Everything lives in a scope: entries, views, repos and installs of apps.

- `user:<id>`: one user's own data.
- `group:<id>`: a group's data.
- `global`: the install's.

User and group ids are 16 random hex characters, chosen by the server that adds them. A user **may use** their own scope, the scopes of the groups they're in, and `global`.

Each scope goes to its servers, and nowhere else: a user's scope to that user's user servers, a group's to its members' user servers, and `global` to every server; every cloud server gets every scope. So each of a user's servers holds every scope that user may use.

## Entries

An entry is a p2panda-core operation:

- **Header extensions:** `{ log: String, time: u64, user?: String, by?: [u8; 32] }`.
  - `log` is the scope the entry is in.
  - `time` is a hybrid clock in milliseconds: greater than its author's previous entry's, in any log, and never less than any entry the author had seen.
  - `user` is the user the entry was written as. Without it, it's the author's own user.
  - `by` is the client or server an operation ran for, when that isn't the author (a phone's post, written by the server that ran it).
- **Body:** UTF-8 JSON, `{ "app" | "driver": string, "kind": string, "data": any, "blobs"?: [hash] }`. `blobs` lists the blobs it refers to. Large things never go inside an entry, only in a blob it names.

Each server writes one log per scope it writes to, keyed by its key and the scope. A log's entries are numbered from 0, and each links to the hash of the previous. A server holds every entry of every log it should hold.

Sync accepts any entry whose signature and backlink verify. Whether it counts is decided when replaying (see "Order"), by the members as replayed up to it. An entry counts when:

- its author is a server (or the recovery key, see "Recovery");
- its author is one of `user`'s servers, or a cloud server; `by`, if given, is one of `user`'s servers or clients;
- `user` may use its scope;
- if its app is `node`, its kind allows it (below).

Replay skips entries that don't count. An entry that arrives before the `admit` of its author is kept, and counts once the `admit` arrives.

A server that should no longer hold a scope (its user left the group, or it isn't a cloud server any more) deletes that scope's logs, blobs and views. Removing a user or a server deletes nothing anywhere else.

## Order and views

The **replay order** is `(time, author, seq)`, compared as integers, then key bytes, then integers. Every view (the members, a repo's branches, each app's stores) is made by replaying a scope's entries in that order, so it is the same on every server that holds the same entries. Views are caches: a server may drop them and replay.

A server keeps its views across restarts, each with the last entry it replayed, and at start replays only newer ones. An entry that arrives sorting before that point rewinds the view to the newest checkpoint before the entry, or to empty, and it replays forward from there.

A **checkpoint** is every view of one scope at one point: a blob, with `at: [(author, log, seq, hash)]`, the last entry of each log it includes. A server makes one after every 10,000 entries a scope replays, and keeps the last three. A server that starts holding a scope asks one of its own user's servers or a cloud server for its newest checkpoint (see "Sync"), loads it, and replays what comes after it. It fetches the older entries afterwards, and drops the checkpoint's views if replaying them gives different ones.

## Node entries

Entries whose app is `node` are the node's own: apps never see them. "Admins only" means the entry's `user` was an admin at that point in replay order.

### Members

All in `global`.

- `genesis { name, user, user_name, server_name, recovery }`: the install's first entry, written by its first server: "this install is called `name`, I am a server of `user`, its first admin, and `recovery` is the recovery key". Its hash is the install's **root**. A server learns it by creating the install or by joining, and ignores every other `genesis`.
- `admit { key, user, name, as, user_name?, role? }`: adds a server (`as: "server"`) or client (`as: "client"`) named `name`, for `user`. A user may admit their own servers and clients. Only an admin may admit for a new user, and that `admit` carries the new user's `user_name` and `role` (`admin` or `member`). A removed key is never admitted again.
- `cloud { key, cloud: bool }`: admins only. Makes a server a cloud server, or a user server again.
- `role { user, role }`: admins only.
- `remove { key }` or `remove { user }`: admins only, or a user removing their own server or client. Removing a user removes all their keys.
- `group { group, name }`: admins only. Creates a group.
- `member { group, user, role }`: admins or that group's admins. Adds `user` to the group as `admin` or `member`, or with `role: null` removes them.
- `rename { key | user | group, name }`: by that key or user, a group's admin, or an admin.
- `relays { urls }`: admins only. The iroh relays every machine of the install uses, before iroh's own.

A user's **user servers** are their servers that aren't cloud servers. Apps see only entries that count, under the names that were current when they were written.

### Recovery

Only the recovery key's public half is in the install (in `genesis`). Its entries count only in its own `global` log, and only `admit`, `remove`, `cloud` and `role`, written as an admin. It never syncs or holds anything. It is used on a fresh server when no server can admit one (see "Backups"): the server writes its own `admit` with it and forgets the secret.

## Sync (ALPN `one/sync/1`)

Between two servers. One bidirectional stream of length-prefixed CBOR messages:

- `Hello { frontier: [(author, log, next_seq)], drivers: [(name, [user])] }`
- `Entries { entries: [(header, body)] }`
- `Checkpoint { scope }`, answered by `Checkpoint { scope, at, blob }`, or `{ scope }` if it has none.

A server accepts a connection only from a key it knows as a current server, or from the machine that just joined with a pairing code it made. It checks again on every message.

Each side sends `Hello`, then every entry the other lacks of the logs the other should hold (in batches of at most 200), then new ones as they're written. `drivers` lists the drivers serving on the sender and the users each serves (see "Drivers"). `Hello` repeats every 30 seconds, and at once when the sender's drivers change or it receives an entry before the one that entry follows, so the gap is filled. Entries timed more than an hour ahead of the receiver's clock are refused. Receiving an entry twice is harmless. A server answers `Checkpoint` only for a scope the asker should hold, and only to its own user's servers and cloud servers.

## Join (ALPN `one/join/1`)

A **pairing code** adds a server or client. It is `{ from: EndpointAddr, secret: [u8; 16] }` in CBOR, encoded as lowercase base32. The server that made it remembers, for each secret, whose it is: the asker's own user's, or a new user's (admins only, with the new user's name and role). The joining machine opens the stream and sends `{ secret, name, as }`. The server appends an `admit` and answers `{ root, user }`. A server then syncs; a client asks. A pairing code works once, for seven days. A key that already belongs to an install can't join another.

## Calls (ALPN `one/call/1`)

A call runs an operation of an app, or of a driver for an app, as a user. One stream per call: JSON `{ app | driver, op, input, me }`, where `me` is `{ key, user }` of the server or client that asked, then a JSON reply `{ ok }` or `{ error }`. A server accepts a call as a user only if it is one of that user's servers or a cloud server, and only from one of that user's servers or a cloud server. A call that takes longer than 30 seconds is stopped.

**Where calls run.** An app's call as a user may run on that user's user servers and on every cloud server: each holds everything the call can reach. A driver's call may run only on the servers whose `Hello` lists that driver for that user. A server allowed to run a call runs it itself, if it's ready (it has the app's release loaded and has replayed every entry it holds). Otherwise it opens a stream to every allowed server it reaches at once, sends `{ ready: { app | driver, op } }`, sends the call only to the first to answer `ready`, and closes the rest, so a call never runs twice.

## Blobs (ALPN `one/blob/1`)

A blob is immutable bytes named by their BLAKE3 hash: attachments, app bundles, git packs, checkpoints, builds. A server fetches the blobs named by the entries it holds, as they arrive, except builds of drivers and the node for other targets. To ask, a machine sends a hash (64 hex characters) on one stream and gets the bytes back; it checks them against the hash. A server answers only if an entry the asker's server should hold names the blob, or it is a checkpoint the asker may have.

## Apps

An app is one ES module, run in workerd (Cloudflare's open-source Workers runtime), whose default export is

```js
{
  name, title, icon, instructions,
  schema: { user, group, global },  // SQL run on each empty store of that scope
  ops: { name: { description, input, run(ctx, input) } },
  on: { kind(db, entry) },          // no clock, no randomness, no I/O
  widgets: { name: { title, render(ctx) } },
  hooks: { name(ctx, { event, since }) },  // returns text for the agent
  screens: { name: { title, html } },      // `main` is the one a shell opens first
}
```

**Stores.** An app has a private store per scope, SQLite in a Durable Object of its own, for each scope that has a `schema`. A store replays the app's entries in its scope. For each, in replay order, `on[entry.kind]` gets the store, as `db.exec(sql, params)` and `db.query(sql, params)`, and the entry `{ id, scope, kind, user, name, time, data, blobs }`. `id` is `<first 8 hex of author>:<seq>`; `name` is the name of `by`, else of the author.

**Calls.** A call runs as one user. `ctx` gives it:

- `me`: `{ key, user, name }`, the client or server that asked.
- `now()`: the time.
- `user`, `group(id)` and `global`: the scopes the call may use, each `{ id, name, query(sql, params), append(kind, data, blobs?) }`. `groups` lists the user's groups.
- `blob.put(bytes)` → hash, and `blob.get(hash)` → bytes.
- `apps.<name>.<op>(input)` and `drivers.<name>.<op>(input)`: the apps and drivers the app declared it uses (see "Releases"), as the same user. Nothing else is wired to it.
- `fetch`, as in any Worker: the internet is open to every app.

An app installed for a user reaches only that user's scope; installed for a group, also that group's; installed for everyone, all three. Code never reaches a scope its user may not use. Entries a call appends are written by the server that ran it, with the caller's `user`, and `by` if the caller isn't that server.

**Installs.** Where a user sees an app installed in several scopes, the nearest wins: their own, then a group's (the lowest group id), then global. A call runs the release nearest its user; each store replays with the release nearest its scope. A new release replays its stores from empty, beside the old ones; calls use the old release until it's ready.

## Repos

Every app and driver is a git repo living in a scope, named uniquely in it. Read access follows the scope. Push access follows it too: a user pushes to their own repos, a group's members to the group's, admins to global ones.

A repo is made of entries in its scope (app `node`):

- `repo { name, source? }`: makes a repo, with the git URL it was imported from, if any. By anyone who may push there.
- `push { repo, branch, old, new, pack }`: moves the writer's own branch `<user>/<branch>` from commit `old` to `new`. `pack` is a blob holding a git packfile with the objects `new` needs beyond `old`.
- `compact { repo, packs, pack }`: `pack` is a blob holding every object of the blobs in `packs`, so servers may drop those.

**Branches** come from replaying a repo's pushes in order, once each push's pack (or a compaction holding it) is present. A push moves `<user>/<branch>` if `old` is its head. Otherwise (two of that user's servers pushed before syncing), it makes `<user>/<branch>-<first 8 hex of its author>`. The shared branch `<branch>` moves to each push whose `new` descends from its head (the first push of a branch makes it). A push that doesn't stays only on its user's branch. Every pushed commit stays on some branch.

**Git.** `git clone one://<repo>` finds the repo the caller may read nearest them (their own, a group's, then global); `one://<scope>/<repo>` names the scope. Git runs `git-remote-one`, which speaks git's smart protocol to the local node. A server's bare repo of each repo is a cache, rebuilt from the entries like a view. A push is checked at once with git's fast-forward check against the pusher's own branch (forcing is allowed there), written as a `push` entry and its pack in the pusher's own log of the repo's scope, and accepted. The helper tells git whether the shared branch moved too or the commit stayed on `<user>/<branch>`; that answer is provisional only when servers that haven't synced pushed at once. Importing copies a repo from any git URL into a scope as a `repo` and a `push` per branch.

## Releases

An install of an app or driver is a release in a scope. Releasing in a user's scope installs for that user, which only they may do; in a group's, for its members, by its admins; in `global`, for everyone, by admins.

- `release { repo, commit, bundle?, drivers?, uses }`: from now on, this scope runs `repo` (`{ scope, name }`) at `commit`. `bundle` is a blob of the app's bundled JavaScript. `drivers` is `[{ name, target, blob }]`, builds of its drivers. `uses` is `{ apps: [repo], drivers: [repo] }`, its dependencies as resolved. `commit: null` uninstalls. Releasing an older commit rolls back.

What runs is always the newest release, never a branch head. A server runs a release only if its writer could release in its scope. A cloud server runs only drivers released in `global`; a user server runs those and its own user's. Drivers aren't released in groups.

A repo has `one.toml` at its root:

```toml
app = "whatsapp"                  # an app, its module at app.js
uses = ["whatsapp", "contacts"]   # apps and drivers it calls, by name
[sources]                         # where to get one the catalog lacks
contacts = "https://github.com/drevantonder/one-contacts"
[drivers.whatsapp]                # a driver: its targets, and how to build it
targets = ["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"]
build = "go build -o $OUT ./driver"
```

The **catalog** is the repos a user may read. Releasing resolves each name in `uses` in the catalog, nearest scope first, or imports it from `[sources]`, and releases each one not yet installed for the same users, in the same scope; it fails, naming what's missing, if neither has it. Workerd wires an app only to what its `uses` names.

To release a commit, a server installs the app's npm packages and bundles it with a pinned Bun (downloaded and hash-checked, like workerd), and builds the drivers for its own target, running each `build` with `$OUT` set. Every server loads a new release into workerd without restarting. A server whose target has no build in the release builds the driver itself from the commit; a later release of the same commit by someone allowed may add it, so others fetch it.

`brand.toml` lists the git sources a new install imports and releases in `global` when it's created.

## Drivers

A driver belongs to a user: a call to the `whatsapp` driver from anyone's app reaches the caller's own WhatsApp. A released driver runs on every server that runs its release and has its build, as a process the node starts and supervises; anything else on a server's machine may also register one over the local API (the Mac app does, for the Apple drivers, so they keep the permissions macOS grants it). A driver decides in its own code whether it can serve there (is chat.db readable, is Incus present, is a session linked here) and for whom, and tells its node. Its node advertises it in `Hello` (see "Sync"); it is never written as an entry. A call checks again that it's ready.

A driver's state is either **portable**, in its user's entries (a Gmail token), so any of the user's servers that runs the driver can serve, or **pinned** on the machine where it lives (WhatsApp's linked session), so it serves only there. Its entries are its own (`driver` in the body), in its user's scope.

Local API, for drivers:

- `POST /v1/drivers/<name> { users }` registers it, serving those users here; `{ why }` says it can't serve here.
- `GET /v1/drivers/<name>/calls` waits up to 25 seconds for a call `{ id, op, input, me }` (204 if none). `POST /v1/drivers/<name>/calls/<id>` answers `{ ok, entries?: [{ kind, data, blobs? }] }` or `{ error }`; the entries are written for the caller's user.
- `GET /v1/drivers/<name>/entries?after=` lists the driver's entries this server holds, for every user, in replay order.

A driver that stops asking for calls for a minute is gone.

## Backups

Any server may be given an S3-compatible bucket (R2, B2, AWS, MinIO), with its credentials and a 32-byte backup key, both kept in 1Password. The setting is the server's own, never an entry. The server writes every entry and blob it holds that the bucket lacks, then each as it arrives:

- `entries/<author>/<log>/<seq>`: the entry's header and body, in CBOR.
- `blobs/<hash>`: the blob.

Each object is encrypted with XChaCha20-Poly1305 under the backup key, its random 24-byte nonce first, and written with `If-None-Match: *`, so it's written once. Several servers may write to one bucket without coordinating: the same name always holds the same plaintext. The bucket decides nothing; object names show authors, scopes and counts, never contents.

**Restore** is import. A fresh server, in no install, downloads the bucket, decrypts it, checks every signature and backlink, and takes the `genesis`. When no server is left to pair with, the recovery key (read once from 1Password) then writes its `admit`, continuing the recovery key's log. The server replays its views, fetching blobs from the bucket as they're needed, and other servers can pair with it as usual.

## Releases of the node

A release of the node is a `global` entry `node-release { commit, target, blob, size, committed, signature }`, where `target` is a Rust target triple, `blob` holds the binary, `committed` is when the commit was made (seconds since 1970), and `signature` is the publisher's signature over the binary.

What runs is the publisher's to decide, not the install's: each brand has a **release key**, Ed25519, whose public half every build carries and whose secret only the publisher holds (in 1Password). A signature is OpenSSH's (`ssh-keygen -Y sign -n release`: the armored "sshsig" format, SHA-512). Any server may publish or pass on a release; who wrote the entry doesn't matter. A node takes the newest release for its target whose signature is by its release key, and ignores newer ones that aren't. If its own build differs and isn't newer, it fetches the blob and checks the signature over it; then, since the entry's other fields aren't signed, it checks that the binary itself says (`version --json`) it's that commit, for its target, and no older than its own build. Only then does it replace its binary and exit; its service manager starts the new one. Each server also writes `running { commit, target }` once per build, so everyone can see what each runs.

The Mac app's node updates with the app. `one update` tests the tip of main, has every Linux server run it, and rebuilds the Mac app on each Mac over SSH (by the server's name). `one release vX.Y.Z` also signs it off (`gh signoff`; there is no CI) and publishes it on GitHub, where each file comes with `<file>.sig`, its signature. The installer checks it with the key it carries, and installs nothing that fails.

## Clients (ALPN `one/client/1`)

A client joins with a pairing code like a server, then asks its user's servers or any cloud server, whichever answers first. Each request is one stream: JSON `{ method, path, body }`, answered with the same JSON the local API gives (below), or `{ error }`. Clients may ask `GET /v1/state`, `GET /v1/apps`, `GET /v1/widgets`, `GET /v1/screen`, `POST /v1/call`, `POST /v1/name`, `POST /v1/pair` and `GET /v1/blobs/<hash>`. `GET /v1/events` instead streams events as JSON lines until the stream closes. Calls run for the client: `ctx.me` is the client. Only current clients may ask.

## Local API

Each server serves HTTP on `127.0.0.1:<brand port>` for its machine's shells, CLI, MCP, git and drivers, acting as the server's own user. It refuses requests with an `Origin` header or a `Host` other than this machine, so no web page can use it.

- `GET /v1/state`: the brand, this server, and the install (null until it creates or joins one): its users, groups, servers and clients; whether each server is online, a cloud server, its target and its drivers; and when each client last asked this server (`asked`, milliseconds, or null).
- `GET /v1/apps`: every app installed for this user: name, title, icon, instructions, operations, widgets and screens.
- `POST /v1/call { app, op, input }` → `{ ok }`.
- `GET /v1/widgets`: every widget, rendered.
- `GET /v1/screens/<app>/<screen>?params=`: an app screen as a page (see "Screens").
- `POST /v1/create { name, user_name }` → `{ recovery }`, the recovery key's secret, given once, for 1Password. `POST /v1/pair { new_user?: { name, role } }` → `{ code }`. `POST /v1/join { code }`. Creating, joining or restoring is refused once the server belongs to an install.
- `POST /v1/name { name }`, `POST /v1/remove { key | user }`, `POST /v1/cloud { key, cloud }`, `POST /v1/role { user, role }`, `POST /v1/group { name }` → `{ group }`, `POST /v1/member { group, user, role }`, `POST /v1/relays { urls }`.
- `GET /v1/catalog`: every repo this user may read, its scope, source, branches and releases. `POST /v1/import { source, scope }`, `POST /v1/install { repo, scope }`, `POST /v1/release { repo, commit, scope }`.
- `/v1/git/<scope>/<repo>/…`: git's smart HTTP protocol, for `git-remote-one`.
- `POST /v1/node-release { path, commit, target, committed, signature }`: publishes the binary at `path`, if `signature` is the release key's over it.
- `POST /v1/blobs` with the bytes as the body → `{ blob, size }`. `GET /v1/blobs/<hash>`: a blob, from this server or another.
- `POST /v1/backup { url, access_key, secret_key, key }` or `{ url: null }`. `POST /v1/restore { url, access_key, secret_key, key, recovery?, user?, name }`.
- `GET /v1/events`: server-sent events `changed { apps }`, `servers` and `install` (this server created, joined or restored one).
- `POST /hooks/<app>/<name>`: for agent harnesses' hooks. The body is the hook's JSON; the reply is plain text for the agent. The node remembers per `session_id` when the hook last ran and passes it to the app as `since` (null on a session's first call).
- Drivers: see "Drivers".

## Screens

An app's screens are HTML. A server serves each as a whole page: the app's HTML inside the design kit (CSS that follows light and dark and the brand's accent) with a bridge, `one`:

- `one.call(op, input)`: runs one of this app's operations as the user looking, and resolves to its result.
- `one.open(screen, params)`: opens another of this app's screens; the shell pushes it with its own navigation.
- `one.title(text)`: the screen's title, shown by the shell.
- `one.on("changed", fn)`: called when this app's data changes.
- `one.params`, `one.me`, and helpers `one.esc`, `one.ago`, `one.avatar`.

A shell answers the bridge itself: in WKWebView, the page posts `{ type: "call" | "open" | "title", … }` to the `one` message handler and awaits the reply; the shell passes server events to `one.__event(event)`. In a plain browser on a server's machine, the bridge talks to the node directly. Shells get pages from `GET /v1/screens/<app>/<screen>?params=` (local API) or `GET /v1/screen { app, screen, params }` (clients). A page a server served may call the server it came from.

## Widgets

A widget renders to a tree of nodes, drawn natively by each shell and as HTML in chats:

- `{ t: "col" | "row", gap?, c: [nodes] }`
- `{ t: "text", text, size?: "xl" | "lg" | "md" | "sm", weight?: "medium" | "bold", tone?: "muted", lines? }`
- `{ t: "time", at, size?, tone? }`: a time in milliseconds, shown relative to now.

Renderers skip node types and fields they don't know.
