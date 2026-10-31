# Protocol

Version 10. Everything here is what two machines must agree on. What's marked *(not built yet)* is designed but not in the code; what's marked *(for now)* is how the code does it until then.

An **install** is the product as one context runs it (The One App, The PTC App, The Van Tonders App), with its own CLI, port, API and data on each machine. Installs never talk to each other. Inside an install:

- **Users** are the people in it. Each is an admin or a member. **Groups** are sets of users that share data; each member of a group is its admin or a member.
- **Servers** run the node. Each has owners and users (see "Servers"), and holds what they may read.
- **Clients** are machines without a node (phones). They hold only a cache and ask a server.
- **Apps** run on every server: JavaScript, in workerd. Only apps have tools, screens and widgets.
- **Drivers** are native code apps wrap: OS frameworks, hardware, shell and files, live sessions.

Each server was admitted for one user, its **local user**. Everything on its machine (CLI, MCP, agents, a browser, the Mac app's window) uses its local API and acts as that user.

## Keys

Every server and client has one Ed25519 key. It is its iroh endpoint id and signs everything it writes. A key belongs to exactly one install, forever, so no entry, message or pairing code names the install. A machine that leaves joins again with a new key.

The **recovery key** is one more key, made with the install and kept only in the admins' 1Password. It never writes an entry and has no log: it only countersigns a restoring server's `admit` (see "Backups").

## Scopes

Everything lives in a scope:

- `user:<id>`: one user's own data.
- `group:<id>`: a group's data.
- `global`: the install's.

User and group ids are 16 random hex characters, chosen by the server that adds them. A user **may use** their own scope, the scopes of the groups they're in, and `global`. Being in a group never exposes a user's own data.

Each server writes one log per scope it writes to, keyed by its key and the scope. A server holds every entry of every log of every scope it holds, and no other.

**Acting for** an owner: a user acts for themselves; a group's admins act for the group; the install's admins act for `admins` and for `install`.

## Servers

Every server has an ACL, a `global` entry:

- `acl { server, owners, users }`: `owners` and `users` are lists of `user:<id>`, `group:<id>` or `install`. Written by anyone acting for one of its current owners. Only admins may make `install` an owner. A server without one has its local user as its only owner and user. An owner is always a user too.

A user **trusts** a server they are a user of when they are directly one of its owners, or they wrote `trust { server }` (members of an owning group, and listed users, opt in this way), or it is privileged: everyone trusts every privileged server, and can't untrust one. `trust` and `untrust` are `global` entries by that user.

A server **holds**:

- `global`;
- every scope its owners can read: a user owner reads their own scope and their groups'; a group owner reads its own; `install` reads every scope;
- the scope of each user who trusts it.

A server owned by `install` holds everything and serves everyone in its users: it is **privileged**. All others are **unprivileged**. A brand may name its privileged servers ("the PTC servers"). A group's data goes only to servers whose owners can read it: its members' servers, servers it owns, and privileged servers. It never follows a user's trust.

Examples: a laptop has owners `[user:you]`, users `[user:you]`. A team box: owners `[group:finance]`, users `[group:finance]`, holding the group's data and that of each member who trusts it. A shared box: owners `[user:carol]`, users `[user:carol, user:dan, user:eve]`.

A server that should no longer hold a scope (an owner left a group, a user untrusted it, the ACL changed) deletes that scope's logs, its blobs no other entry names *(not built yet: blobs stay)*, its views and its secrets. It keeps only the end of its own log there (seq, hash, time), and if it holds the scope again it writes there only once it has its own log back, so no seq is ever written twice. Removing a user or a server deletes nothing anywhere else.

**Opening a user's data.** Nothing gives anyone access to another user's data but this: someone acting for an owner of a server that holds `user:<id>` writes `open { server }` in that scope. For an hour from its time, the opener's calls on that server may run as that user, reading only (appends fail). The user sees it ("Alice opened your data on 3 Nov"). On a privileged server, that is any admin. The opener asks that server over `one/call/1`, as `{ app: "node", op: "open", input: { user } }`, since only a server that holds the scope can write in it.

## Entries

An entry is a p2panda-core operation:

- **Header extensions:** `{ log: String, time: u64, user: String, by?: [u8; 32] }`.
  - `log` is the scope the entry is in.
  - `time` is a hybrid clock in milliseconds: greater than its author's previous entry's, in any log, and never less than any entry the author had seen.
  - `user` is the user the entry was written as.
  - `by` is the client or server an operation ran for, when that isn't the author (a phone's post, written by the server that ran it).
- **Body:** UTF-8 JSON, `{ app: string, kind: string, data: any, blobs?: [hash] }`. `blobs` lists the blobs it refers to. Large things never go inside an entry, only in a blob it names.

A log's entries are numbered from 0, and each links to the hash of the previous. Sync accepts any entry whose signature and backlink verify. Whether it counts is decided when replaying, by the members as replayed up to it. An entry counts when:

- its author is a current server that holds its scope (a restoring server's own `admit` makes it current);
- `user` trusts the author, and may use its scope (an `open` is written as the opener); `by`, if given, is a client of `user` or a server `user` trusts;
- if its app is `node`, its kind allows it (below).

Replay skips entries that don't count. An entry that arrives before the `admit` of its author is kept, and counts once the `admit` arrives.

## Order and views

The **replay order** is `(time, author, log, seq)`, compared as integers, key bytes, string bytes, integers. Replaying any scope also replays `global`'s member entries (see "Members") in the same order, so who counts, roles and names are decided the same way in every scope. Every view (the members, each app's stores) is made by replaying a scope's entries in that order, so it is the same on every server that holds the same entries.

Views are caches: a server may drop them and replay, and how it keeps them is its own. A new server replays every scope it holds from the start. An entry that sorts before what a view has replayed (a member entry of `global` included) means that view replays again from before it.

## Node entries

Entries whose app is `node` are the node's own: apps never see them. "Admins only" means the entry's `user` was an admin at that point in replay order.

### Members

All in `global`. These are the entries every scope replays with its own.

- `genesis { name, user, user_name, server_name, recovery }`: the install's first entry, written by its first server: "this install is called `name`, I am a server of `user`, its first admin, and `recovery` is the recovery key". Its hash is the install's **root**. A server learns it by creating the install or by joining, and ignores every other `genesis`.
- `admit { key, user, name, as, user_name?, role?, recovery? }`: adds a server (`as: "server"`) or client (`as: "client"`) named `name`, for `user`. A user may admit their own servers and clients. Only an admin may admit for a new user, and that `admit` carries the new user's `user_name` and `role` (`admin` or `member`). A restoring server writes its own `admit` for an admin, with `recovery`, the recovery key's signature over `key` and `user` *(not built yet)*. A removed key is never admitted again.
- `acl { server, owners, users }`, `trust { server }`, `untrust { server }`: see "Servers".
- `role { user, role }`: admins only.
- `remove { key }` or `remove { user }`: admins only, or someone acting for an owner of that server, or a user removing their own client. Removing a user removes all their keys.
- `group { group, name }`: by anyone, who becomes its first admin.
- `invite { group, user, role }`: that group's admins. `accept { group }`: by the invited user, who then is a member, as `admin` or `member`.
- `member { group, user, role }`: that group's admins change a member's role, or with `role: null` remove them. A user may remove themselves.
- `rename { key | user | group, name }`: by that key's user or that user, a group's admin, or an admin.

The install's admins manage every group as its admins do (`invite`, `member`, `rename`), so a group whose last admin left isn't stuck; they don't act for it otherwise (as a server's or an app's owner).
- `relays { urls }`: admins only. The iroh relays every machine of the install uses, before iroh's own.
- `recovery { key }`: admins only. Replaces the recovery key.

A user's **role** in a scope is their role in the install for `global`, in the group for `group:<id>`, and `admin` in their own scope. Apps see only entries that count, under the names and roles that were current when they were written.

### Apps

`app` and `deploy` are in `global`: every app and its code is visible to everyone in the install.

- `app { name, owner, from? }`: makes an app called `name`, unique in the install (the first in replay order wins), owned by `owner` (`user:<id>`, `group:<id>` or `admins`), by anyone acting for it. Written again by someone acting for the current owner, it gives the app a new owner. A **fork** is a new app with its own name and data, and `from`, the app it came from.
- `deploy { app, bundle }`: by someone acting for the app's owner, with `bundle` among its `blobs`. `bundle` is a blob: a tar (ustar) holding `app.js`, the built ES module, and `src/`, the files it was built from, as written. Every server runs each app's latest deploy, and nothing else: rolling back is deploying an older bundle again.
- `install { app }` and `uninstall { app }`, in the scope they install in: a user's by that user, a group's by its admins, `global` by admins. A scope's installs are what a shell shows as that scope's page. An app is **installed for** a user when it is installed in their scope, a group of theirs, or `global`. The same app may be installed in several scopes: it is still one app at one version, with data per scope. Installing an app also installs, in the same scope, each app it uses that isn't installed there *(not built yet)*.

### Drivers

- *(not built yet)* `driver { server, name, blob }`, in `global`: someone acting for an owner of `server` installs a driver there. `blob` holds its executable for that server's target, or is null to remove it.

### Secrets

*(Not built yet.)*

- `secret { app, name, hosts, value }`, in its scope: sets the secret `name` of `app`, sent only to `hosts` (host names). A user's by that user, a group's by its admins, `global`'s by admins. `value: null` deletes it.

A secret is set, never read: nothing (CLI, MCP, API, query, view, app or agent) can read its value back; only that it exists, its hosts, and replacing or deleting it. The node keeps it out of every view.

### Node releases

- `node-release { commit, target, blob, size, committed, signature }`, in `global`: `target` is a Rust target triple, `blob` holds the binary, `committed` is when the commit was made (seconds since 1970), and `signature` is the publisher's signature over the binary.
- `running { commit, target }`: each server writes one per build, so everyone can see what each runs.

Each brand has a **release key**, Ed25519, whose public half every build carries and whose secret only the publisher holds. A signature is OpenSSH's (`ssh-keygen -Y sign -n release`: the armored "sshsig" format, SHA-512). Any server may publish or pass on a release; who wrote the entry doesn't matter. A node takes the newest release for its target whose signature is by its release key. If its own build differs and isn't newer, it fetches the blob and checks the signature over it; then, since the entry's other fields aren't signed, it checks that the binary itself says (`version --json`) it's that commit, for its target, and no older than its own build. Only then does it replace its binary and exit; its service manager starts the new one.

## Sync (ALPN `one/sync/1`)

Between two servers. One bidirectional stream of length-prefixed CBOR messages:

- `Hello { frontier: [(author, log, next_seq)], drivers: [(name, [user], description)] }`
- `Entries { entries: [(header, body)] }`

A server accepts a connection only from a key it knows as a current server, or from the machine that just joined with a pairing code it made. It checks again on every message.

Each side sends `Hello`, then every entry the other lacks of the logs of the scopes the other holds (in batches of at most 200), then new ones as they're written. `drivers` lists the drivers serving on the sender, the users each serves, and *(for now, until apps wrap drivers)* each one's description: title, instructions and operations, which servers list as tools. `Hello` repeats every 30 seconds, and at once when the sender's drivers change or it receives an entry before the one that entry follows, so the gap is filled. Entries timed more than an hour ahead of the receiver's clock are refused. Receiving an entry twice is harmless. When a server's members change, it says `Hello` to every peer if it now holds a scope it didn't, and sends each peer that now holds a scope it didn't what that peer lacks there.

A server is **ready** once it has replayed what it holds and, since it last woke, received every entry each connected server's `Hello` offered it.

## Join (ALPN `one/join/1`)

A **pairing code** adds a server or client. It is `{ from: EndpointAddr, secret: [u8; 16] }` in CBOR, encoded as lowercase base32; `from` carries the install's relay. The server that made it remembers, for each secret, whose it is: the asker's own user's, or a new user's (admins only, with the new user's name and role). The joining machine opens the stream and sends `{ secret, name, as }`. The server appends an `admit` and answers `{ root, user }`, or `{ error }`. A client is refused when no server would serve its user ("this install has no server for you yet"): a phone only asks servers, so a phone-only user needs a privileged server whose users include `install`, or for an existing user, one of their servers. A server then syncs; a client asks. A pairing code works once, for seven days. A key that already belongs to an install can't join another.

## Calls (ALPN `one/call/1`)

A call runs an operation of an app, or of a driver for an app, as a user. One stream per call: JSON `{ app | driver, op, input, me, scopes, as? }`, where `me` is `{ key, user }` of the server or client that asked, `scopes` the scopes the call touches, and `as` a user whose data was opened for the caller (see "Servers"). Then a JSON reply: `{ ok }`, `{ error }`, or `{ elsewhere: why }` when it can't run here. *(For now, `app` also names a driver, and there is no `driver` field.)*

**Scopes first.** The asker names the scopes a call touches before it runs; a client may leave them out, and each server then takes every scope the call may reach (see "Calls" under "Apps"). A server answers `{ elsewhere }` before running anything when it doesn't serve that user, doesn't hold every scope named, isn't ready, or lacks the app's latest deploy or the driver. So failing over to the next server never repeats a side effect. A call that names a scope it may not reach is an error.

A server accepts a call as a user only if that user trusts it, and only from one of that user's clients or a server that user trusts. A call that takes longer than 30 seconds is stopped.

**Where calls run.** An app's call as a user runs on a server that user trusts and that holds every scope the call touches; a driver's, on a server whose `Hello` lists that driver for that user. The asker tries them nearest first (its own machine, then the fastest to answer lately), one at a time, never two at once, and tries the next only on a connection error or `elsewhere`. Clients do the same; a server asked by a client for a driver it doesn't run passes the call on to one that does.

## Blobs (ALPN `one/blob/1`)

A blob is immutable bytes named by their BLAKE3 hash: attachments, bundles, git packs, builds. A server fetches the blobs named by the entries it holds, as they arrive, except node builds for other targets and drivers installed on other servers. To ask, a machine sends a hash (64 hex characters) on one stream and gets the bytes back; it checks them against the hash. A server answers only if an entry in a scope the asker holds names the blob. *(For now, a server fetches only deploys' bundles as they arrive, and releases' binaries when it takes them; other blobs when asked.)*

## Apps

An app is one ES module, run in workerd (Cloudflare's open-source Workers runtime), whose default export is

```js
{
  name, title, icon, instructions,
  uses: { apps: [name], drivers: [name] },
  schema: { user, group, global },  // SQL run on each empty store of that scope
  ops: { name: { description, input, run(ctx, input) } },
  on: { kind(db, entry) },          // no clock, no randomness, no I/O
  widgets: { name: { title, render(ctx) } },
  hooks: { name(ctx, { event, since }) },  // returns text for the agent
  screens: { name: { title, html } },      // `main` is the one a shell opens first
}
```

**Stores.** An app has a private store per scope, SQLite in a Durable Object of its own, for each scope that has a `schema`. A store replays the app's entries in its scope with the app's latest deploy. For each, in replay order, `on[entry.kind]` gets the store, as `db.exec(sql, params)` and `db.query(sql, params)`, and the entry `{ id, scope, kind, user, name, role, groups, time, data, blobs }`. `id` is `<first 8 hex of author>:<seq>`; `name` is the name of `by`, else of the author; `role` is the writer's role in the scope, and `groups` their role in each of their groups, at that point. A new deploy replays its stores from empty, beside the old ones; calls use the old until it's done *(for now: the runtime restarts with the new deploy and replays its stores before running calls)*. There are no migrations.

**Calls.** A call runs as one user. `ctx` gives it:

- `me`: `{ key, user, name, role }`, the client or server that asked, and its user's role in the install.
- `now()`: the time.
- `user`, `group(id)` and `global`: the scopes the call named, each `{ id, name, role, query(sql, params), append(kind, data, blobs?), secret(name, value, hosts) }`, or null; `role` is the caller's there. `groups` lists the groups it named. `query` returns a promise. *(`secret` not built yet.)*
- `install(app, scope, installed = true)`: installs (or uninstalls) an app in a scope (`user`, `group:<id>` or `global`), with the caller's own rights there, as the caller could themselves. So an App Store is an ordinary app (`brand.toml`'s `store`).
- `blob.put(bytes)` → hash, and `blob.get(hash)` → bytes.
- `apps.<name>.<op>(input)` and `drivers.<name>.<op>(input)`: what `uses` names, as the same user. Nothing else is wired to it. *(Not built yet.)*
- `fetch`, as in any Worker: the internet is open to every app.

A call may reach the caller's own scope, each of their groups the app is installed for (in that group or in `global`; a group whose installs the server can't see counts), and `global` if it is installed there. It reaches the ones it named, every one it may reach if it named none. `append` buffers: when `run` returns, the entries are written by the server that ran it, as the caller, with `by` if the caller isn't that server, and replayed there before the result returns. Queries in the same call don't see them. A call that fails or is stopped writes nothing.

**Secrets in calls.** *(Not built yet.)* An app puts a placeholder where a secret goes, `Authorization: "Bearer {{gmail_token}}"`, in the headers or body of a `fetch`. Every outbound fetch goes through the node, which fills it in on the way out, and only if the request goes to one of the secret's hosts; otherwise it refuses the request. `{{name}}` is the app's secret in the caller's scope, else in `global`; `{{<group id>/name}}` is a group's. `secret(name, value, hosts)` lets a call set (never read) one of its app's secrets, if its caller may set secrets in that scope, so an app can keep a token it was given.

## Drivers

*(For now, a driver is a native app built into the node (shell, agents, models), or a provider: a program that registers over the local API with its description, `POST /v1/providers/<name>`, and long-polls `GET /v1/providers/<name>/calls`, answering `POST /v1/providers/<name>/calls/<id>` with `{ ok, entries? }` or `{ error }`. Both serve their server's local user, write their entries in that user's scope, and are advertised in `Hello`. The rest of this section is not built yet.)*

A driver serves its users from its own machine: a call to the `whatsapp` driver from anyone's app reaches the caller's own WhatsApp. It runs unsandboxed, so a server runs a driver only if someone acting for one of its owners installed it there; a privileged server, only drivers an admin installed. A driver is installed either by a `driver` entry, and the node starts and supervises it, or by registering over the local API, which counts as installed by the local user (the Mac app does, for the Apple drivers, so they keep the permissions macOS grants it).

A driver decides in its own code whether it can serve there (is chat.db readable, is Incus present, is a session linked here) and for which of the users who trust the server, and tells its node. Its node advertises it in `Hello`; it writes no entries. Its state is its own, on its machine. A call checks again that it's ready.

Local API, for drivers:

- `POST /v1/drivers/<name> { users }` registers it, serving those users here; `{ why }` says it can't serve here.
- `GET /v1/drivers/<name>/calls` waits up to 25 seconds for a call `{ id, op, input, me }` (204 if none). `POST /v1/drivers/<name>/calls/<id>` answers `{ ok }` or `{ error }`.

A driver that stops asking for calls for a minute is gone.

## Backups

*(Not built yet.)*

Any server may be given an S3-compatible bucket (R2, B2, AWS, MinIO), with its credentials and a 32-byte backup key, both kept in 1Password. The setting is the server's own, never an entry. The server writes every entry and blob it holds that the bucket lacks, then each as it arrives:

- `entries/<name>`: the entry's header and body, in CBOR. `name` is the BLAKE3 hash, keyed by the backup key, of `(author, log, seq)`.
- `blobs/<name>`: the blob. `name` is the keyed hash of its hash.

Each object is encrypted with XChaCha20-Poly1305, its random 24-byte nonce first, and written with `If-None-Match: *`, so it's written once. The naming and encryption keys are derived from the backup key (BLAKE3 `derive_key`, contexts `one backup names` and `one backup data`). Several servers may write to one bucket without coordinating: the same name always holds the same plaintext. The bucket decides nothing, and sees only how many objects there are and their sizes.

**Restore** is import. A fresh server, in no install, downloads the bucket, decrypts it, checks every signature and backlink, and takes the `genesis`. When no server is left to pair with, it writes its own `admit` for an admin, countersigned with the recovery key (read once from 1Password, then forgotten). It replays its views, fetching blobs from the bucket as they're needed, and other servers can pair with it as usual.

## Clients (ALPN `one/client/1`)

A client joins with a pairing code like a server, then asks the servers its user trusts, nearest first, one at a time, as calls do. Each request is one stream: JSON `{ method, path, body }`, answered with the same JSON the local API gives (below), or `{ error }`, or `{ elsewhere: why }`. Clients may ask `GET /v1/state`, `GET /v1/apps`, `GET /v1/widgets`, `GET /v1/screen { app, screen, params, scope }`, `POST /v1/call`, `POST /v1/name`, `POST /v1/pair`, `POST /v1/trust` and `GET /v1/blobs/<hash>` (answered `{ blob }`, base64). A server that doesn't serve the client's user answers everything but `GET /v1/state` with `{ elsewhere }`. `GET /v1/events` instead streams events as JSON lines until the stream closes. Calls run for the client: `ctx.me` is the client. Only current clients may ask.

## Local API

Each server serves HTTP on `127.0.0.1:<brand port>` for its machine's shells, CLI, MCP, git helpers and drivers, acting as its local user. It refuses requests with an `Origin` header or a `Host` other than this machine, so no web page can use it.

- `GET /v1/state`: the brand (with its `store` app), `me` (who's asking: key, name, user), this `server` (and whether it's ready), and the `install` (null until it creates or joins one): its users, groups, servers (online, owners, users, privileged, who trusts each, whether each serves me, build, target, drivers) and clients (`asked`: when each last asked this server, milliseconds, or null); and every `open` of this user's data.
- `GET /v1/apps`: every app in the install (owner, `from`, deploys, where installed), with title, icon, instructions, operations, widgets and screens for each installed for this user.
- `POST /v1/call { app, op, input, scopes?, server?, as? }` → `{ ok }`. `server` (a name or key prefix) runs it there; otherwise it runs on the nearest server that can.
- `GET /v1/widgets`: every widget, rendered.
- `GET /v1/screens/<app>/<screen>?params=&scope=`: an app screen as a page (see "Screens").
- `POST /v1/create { name, user_name }` → `{ recovery }`, the recovery key's secret, given once, for 1Password. `POST /v1/pair { new_user?: { name, role } }` → `{ code }`. `POST /v1/join { code }`. Creating, joining or restoring is refused once the server belongs to an install.
- `POST /v1/name { name }`, `POST /v1/remove { key | user }`, `POST /v1/role { user, role }`, `POST /v1/relays { urls }`, `POST /v1/recovery` → `{ recovery }` *(not built yet)*. Users, groups and servers may be given by name.
- `POST /v1/acl { server, owners, users }`, `POST /v1/trust { server, trust }`, `POST /v1/open { user, server }`.
- `POST /v1/group { name }` → `{ group }`, `POST /v1/invite { group, user, role }`, `POST /v1/accept { group }`, `POST /v1/member { group, user, role }`.
- `POST /v1/app { name, owner, from? }`, `POST /v1/deploy { app, bundle }`, `POST /v1/install { app, scope, installed }`.
- *(Not built yet.)* `POST /v1/driver { server, name, blob }`. `GET /v1/secrets?scope=`: the secrets this user may set there: app, name, hosts, never values. `POST /v1/secrets { scope, app, name, hosts, value }`, with `value: null` to delete.
- `POST /v1/node-release { path, commit, target, committed, signature }`: publishes the binary at `path`, if `signature` is the release key's over it.
- `POST /v1/blobs` with the bytes as the body → `{ blob, size }`. `GET /v1/blobs/<hash>`: a blob, from this server or another.
- *(Not built yet.)* `POST /v1/backup { url, access_key, secret_key, key }` or `{ url: null }`. `POST /v1/restore { url, access_key, secret_key, key, recovery?, user?, name }`.
- `GET /v1/events`: server-sent events `changed { apps }`, `servers` and `install` (this server created, joined or restored one).
- `POST /hooks/<app>/<name>`: for agent harnesses' hooks. The body is the hook's JSON; the reply is plain text for the agent. The node remembers per `session_id` when the hook last ran and passes it to the app as `since` (null on a session's first call).
- Drivers: see "Drivers" (for now, `/v1/providers/<name>`).

## Screens

An app's screens are HTML. A server serves each as a whole page: the app's HTML inside the design kit (CSS that follows light and dark and the brand's accent) with a bridge, `one`:

- `one.call(op, input)`: runs one of this app's operations as the user looking, and resolves to its result.
- `one.open(screen, params)`: opens another of this app's screens; the shell pushes it with its own navigation.
- `one.title(text)`: the screen's title, shown by the shell.
- `one.on("changed", fn)`: called when this app's data changes.
- `one.params`, `one.me`, and helpers `one.esc`, `one.ago`, `one.avatar`.
- `one.scope`: the page the shell opened the screen from (`user`, `group:<id>` or `global`), or null. Only a hint: the app decides what to show, and draws its own scope picker if it has one.

A shell answers the bridge itself: in WKWebView, the page posts `{ type: "call" | "open" | "title", … }` to the `one` message handler and awaits the reply; the shell passes server events to `one.__event(event)`. In a plain browser on a server's machine, the bridge talks to the node directly. Shells get pages from `GET /v1/screens/<app>/<screen>?params=&scope=` (local API) or `GET /v1/screen { app, screen, params, scope }` (clients). A page a server served may call the server it came from.

## Widgets

A widget renders to a tree of nodes, drawn natively by each shell and as HTML in chats:

- `{ t: "col" | "row", gap?, c: [nodes] }`
- `{ t: "text", text, size?: "xl" | "lg" | "md" | "sm", weight?: "medium" | "bold", tone?: "muted", lines? }`
- `{ t: "time", at, size?, tone? }`: a time in milliseconds, shown relative to now.

Renderers skip node types and fields they don't know.
