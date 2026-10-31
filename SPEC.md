# Protocol

Version 8. Everything here is what two devices must agree on.

## Keys

Every device has one Ed25519 key. It is the device's iroh endpoint id and signs everything the device writes.

A **space** is identified by the public half of a space key, made when the space is created. Its secret stays on the device that created it.

A node holds one space, under one brand (its name, CLI, port and data). A device in several contexts runs each context's app, each with its own key, data and node; so does a phone.

## Entries

An entry is a p2panda-core operation:

- **Header extensions:** `{ space: [u8; 32], log: String, time: u64 }`.
  - `log` names the audience. Version 1 has one audience, `"space"`.
  - `time` is a hybrid clock in milliseconds: never less than the author's previous entry's, and never less than any entry the author had seen.
- **Body:** UTF-8 JSON, `{ "app": string, "kind": string, "data": any }`.

Each device writes one log per audience. Its entries are numbered from 0, and each one links to the hash of the previous. An entry is valid when its signature verifies, its backlink matches, and its `space` is the space it arrived for.

## Order

The **replay order** is `(time, author, seq)`, compared as integers, then key bytes, then integers. Every view is rebuilt by replaying every device's entries in that order, so it is the same on every device.

## Membership

Membership is rebuilt by replaying the entries whose app is `space`:

- `genesis`: `{ name, root_sig, user, device_name }`. The first entry of a space. `root_sig` is the space key's signature over `author ‖ name`. The author becomes an admin, with that user and device name.
- `admit`: `{ device, user, name, role, client? }`. `client` marks a device that keeps no log (a phone); nobody syncs with it. Valid when written by an admin, or by a member admitting a device for their own `user` as a `member`. `role` is `admin` or `member`. A removed device is never admitted again; it joins with a new key.
- `remove`: `{ device }`. Valid when written by an admin. Entries the removed device writes after the `remove` (in replay order) are ignored.
- `rename`: `{ device, name }`. Valid when written by that device or an admin.

Apps only see entries whose author was a member when they were written, under the name it had then.

## Sync (ALPN `one/sync/1`)

One bidirectional stream of length-prefixed CBOR messages:

- `Hello { space, frontier: [(author, log, next_seq)] }`
- `Entries { space, entries: [(header, body)] }`

Each side sends `Hello`, then sends every entry the other's frontier lacks (in batches of at most 200), then sends new entries as they are written. `Hello` repeats every 30 seconds; a device that receives an entry before the one it follows answers with `Hello` at once, so the gap is filled. Entries timed more than an hour ahead of the receiver's clock, or in a log other than `space`, are refused. Only members' devices may sync, checked on every message; a joining device may sync with the device whose pairing code it used. Receiving an entry twice is harmless.

## Join (ALPN `one/join/1`)

A pairing code is `{ space, from: EndpointAddr, secret: [u8; 16] }` in CBOR, encoded as lowercase base32. The device that made it remembers, for each secret, whose device it is for: another of its own user's, or a new user's (admins only). The joining device opens the stream and sends `{ secret, name, client? }`; the device that made the code appends an `admit` and answers `{ space_name, user }`. The joining device then syncs as a member. A pairing code works once, for seven days.

## Calls (ALPN `one/call/1`)

One stream per call: a JSON request `{ space, app, op, input }`, then a JSON reply `{ ok }` or `{ error }`. Only members may call.

## Blobs (ALPN `one/blob/1`)

A blob is bytes named by their BLAKE3 hash. A member sends a hash (64 hex characters) on one stream and gets the bytes back; it checks them against the hash. Only members may ask.

## Releases

The app `node` is the node's own, and other apps never see it. A release of the node is an entry with kind `release`: `{ commit, target, blob, size, committed, signature }`, where `target` is a Rust target triple, `blob` holds the binary, `committed` is when the commit was made (seconds since 1970), and `signature` is the publisher's signature over the binary.

What runs is the publisher's to decide, not the space's: each brand has a **release key**, Ed25519, whose public half every build carries and whose secret only the publisher holds. A signature is OpenSSH's (`ssh-keygen -Y sign -n release`: the armored "sshsig" format, namespace `release`, SHA-512). Any member's device may publish or pass on a release; who wrote the entry doesn't matter. A node takes the newest release for its target whose signature is by its release key, and ignores newer ones that aren't. If its own build differs and isn't newer, it fetches the blob (from the author, else any member) and checks the signature over it; then, since the entry's other fields aren't signed, it checks that the binary itself says (`version --json`) it's that commit, for its target, and no older than its own build. Only then does it replace its binary and exit; its service manager starts the new one. Nodes inside an app bundle update with the app instead. Each node also writes `running { commit, target }` once per build, so everyone can see what each device runs.

On GitHub, each file of a release comes with `<file>.sig`, its signature; the installer checks it with the key it carries, and installs nothing that fails.

## Providers

A device announces each provider registered with it (see "Local API") as an entry with app `node` and kind `provides`: the app's description, as `GET /v1/apps` gives it. It writes one only when the description differs from the last it announced. Every device lists the newest description each of its own user's devices announced, and calls the app on a device that announced it.

## Clients (ALPN `one/client/1`)

A device that keeps no log (a phone) is still a member: it joins with a pairing code like any device. It then asks any member device that does keep logs to act for it. Each request is one stream: JSON `{ method, path, body }`, answered with the same JSON the local API gives (below), or `{ error }`. Clients may ask `GET /v1/state`, `GET /v1/apps`, `GET /v1/widgets`, `GET /v1/screen`, `POST /v1/call`, `POST /v1/device` and `POST /v1/pair`; the space is the node's. `GET /v1/events` instead streams events as JSON lines until the stream closes. Operations run for the asking device: `ctx.me` is the client, and the entries they append are written by the device that ran them. Only members may ask.

## Local API

Each node serves HTTP on `127.0.0.1:<brand port>` for its own device's shells, CLI and MCP. It refuses requests with an `Origin` header or a `Host` other than this machine, so no web page can use it.

- `GET /v1/state`: the brand, this device, and its `space` (null until it creates or joins one) with its devices: whether each is online, or, for a phone (`client`), when it last asked this device (`asked`, milliseconds, or null).
- `GET /v1/apps`: every app's name, title, icon, instructions, operations and widgets.
- `POST /v1/call { device?, app, op, input }` → `{ ok }`.
- `GET /v1/widgets`: every widget, rendered.
- `GET /v1/screens/<app>/<screen>?params=`: an app screen as a page (see "Screens").
- `POST /v1/spaces { name }`, `POST /v1/pair { new_user? }` → `{ code }`, `POST /v1/join { code }`, `POST /v1/device { name }`, `POST /v1/remove { device }`. Creating or joining a space is refused once the node holds one.
- `POST /v1/release { path, commit, target, committed, signature }`: publishes the binary at `path`, if `signature` is the release key's over it.
- `POST /v1/blobs` with the bytes as the body → `{ blob, size }`: keeps them as a blob. `GET /v1/blobs/<hash>`: a blob, from this device or any member online.
- `GET /v1/events`: server-sent events `changed { apps }`, `peers` and `space` (this device created or joined its space).
- Providers, for device features written outside the node (Swift, Go): `POST /v1/providers/<app>` with the app's description registers it; `GET /v1/providers/<app>/calls` waits up to 25 seconds for a call `{ id, op, input, caller }` (204 if none); `POST /v1/providers/<app>/calls/<id>` answers `{ ok, entries?: [{ kind, data }] }` or `{ error }`. A provider that stops asking for calls for a minute is gone. Providers run only for their device's own user's devices.
- `POST /hooks/<app>/<name>`: for agent harnesses' hooks. The body is the hook's JSON; the reply is plain text for the agent. The node remembers per `session_id` when the hook last ran and passes it to the app as `since` (null on a session's first call).

## Apps

An app is JavaScript: one ES module, run in workerd (Cloudflare's open-source Workers runtime), whose default export is

```js
{
  name, title, icon, instructions,
  schema,                         // SQL run on its empty view database
  ops: { name: { description, input, run(ctx, input) } },
  on: { kind(db, entry) },        // no clock, no randomness, no I/O
  widgets: { name: { title, render(ctx) } },
  hooks: { name(ctx, { event, since }) },  // returns text for the agent
  screens: { name: { title, html } },      // `main` is the one a shell opens first
}
```

`ctx` has `append(kind, data)`, `query(sql, params)`, `me` (who asked: `{ device, user, name }`) and `now()`. For each of the app's entries, in replay order, `on[entry.kind]` gets `db.exec(sql, params)` and `db.query(sql, params)`, and the entry `{ id, app, kind, author, user, name, time, data }`, where `id` is `<first 8 hex of author>:<seq>`. Entries an operation appends are written by the device that ran it. Each app keeps its views in a Durable Object, in SQLite. A call that takes longer than 30 seconds stops the runtime, which starts again.

## Screens

An app's screens are HTML. The node serves each as a whole page: the app's HTML inside the design kit (CSS that follows light and dark and the brand's accent) with a bridge, `one`:

- `one.call(op, input)`: runs one of this app's operations for the device looking, and resolves to its result.
- `one.open(screen, params)`: opens another of this app's screens; the shell pushes it with its own navigation.
- `one.title(text)`: the screen's title, shown by the shell.
- `one.on("changed", fn)`: called when this app's data changes.
- `one.params`, `one.me`, and helpers `one.esc`, `one.ago`, `one.avatar`.

A shell answers the bridge itself: in WKWebView, the page posts `{ type: "call" | "open" | "title", … }` to the `one` message handler and awaits the reply; the shell passes node events to `one.__event(event)`. In a plain browser on the device, the bridge talks to the node directly. Shells get pages from `GET /v1/screens/<app>/<screen>?params=` (local API) or `GET /v1/screen { app, screen, params }` (clients). A page the node served may call the node it came from.

## Widgets

A widget renders to a tree of nodes, drawn natively by each shell and as HTML in chats:

- `{ t: "col" | "row", gap?, c: [nodes] }`
- `{ t: "text", text, size?: "xl" | "lg" | "md" | "sm", weight?: "medium" | "bold", tone?: "muted", lines? }`
- `{ t: "time", at, size?, tone? }`: a time in milliseconds, shown relative to now.

Renderers skip node types and fields they don't know.
