# Protocol

Version 1. Everything here is what two devices must agree on.

## Keys

Every device has one Ed25519 key. It is the device's iroh endpoint id and signs everything the device writes.

A **space** is identified by the public half of a space key, made when the space is created. Its secret stays on the device that created it.

## Entries

An entry is a p2panda-core operation:

- **Header extensions:** `{ space: [u8; 32], log: String, time: u64 }`.
  - `log` names the audience. Version 1 has one audience, `"space"`.
  - `time` is a hybrid clock in milliseconds: never less than the author's previous entry's, and never less than any entry the author had seen.
- **Body:** UTF-8 JSON, `{ "app": string, "kind": string, "data": any }`.

Each device writes one log per space and audience. Its entries are numbered from 0, and each one links to the hash of the previous. An entry is valid when its signature verifies, its backlink matches, and its `space` is the space it arrived for.

## Order

The **fold order** is `(time, author, seq)`, compared as integers, then key bytes, then integers. Every view is a fold of all entries in that order, so it is the same on every device.

## Membership

Membership is the fold of entries whose app is `space`:

- `genesis`: `{ name, root_sig }`. The first entry of a space. `root_sig` is the space key's signature over `author ‖ name`. The author becomes an admin.
- `admit`: `{ device, person, name, role }`. Valid when written by an admin, or by a member admitting a device for their own `person`. `role` is `admin` or `member`.
- `remove`: `{ device }`. Valid when written by an admin. Entries the removed device writes after the `remove` (in fold order) are ignored.
- `rename`: `{ device, name }`. Valid when written by that device or an admin.

Folds of other apps only see entries whose author was a member when they were written.

## Sync (ALPN `one/sync/1`)

One bidirectional stream of length-prefixed CBOR messages:

- `Hello { space, frontier: [(author, log, next_seq)] }`
- `Entries { space, entries: [(header, body)] }`

Each side sends `Hello`, then sends every entry the other's frontier lacks, then sends new entries as they are written. `Hello` repeats every 30 seconds to repair gaps. Only members' devices may open the stream. Receiving an entry twice is harmless.

## Join (ALPN `one/join/1`)

An invite is a ticket: `{ space, inviter: EndpointAddr, secret: [u8; 16] }` in CBOR, encoded as lowercase base32. The inviter remembers, for each secret, whose device it is for: one of its own person's, or a new person's. The joiner opens the stream and sends `{ secret, name }`; the inviter appends an `admit` and answers `{ space_name, person }`. The joiner then syncs as a member. A secret works once and only until the inviter restarts.

## Calls (ALPN `one/call/1`)

One stream per call: a JSON request `{ space, app, op, input }`, then a JSON reply `{ ok }` or `{ error }`. Only members may call.

## Clients (ALPN `one/client/1`)

A device that keeps no log (a phone) is still a member: it joins with a ticket like any device. It then asks any member device that does keep logs to act for it. Each request is one stream: JSON `{ method, path, body }`, answered with the same JSON the local API gives (below), or `{ error }`. `GET /v1/events` instead streams events as JSON lines until the stream closes. Operations run for the asking device: `ctx.me` is the client, and the entries they append are written by the device that ran them. Only members may ask.

## Local API

Each node serves HTTP on `127.0.0.1:<brand port>` for its own device's shells, CLI and MCP:

- `GET /v1/state`: the brand, this device, and each space with its devices and whether they're online.
- `GET /v1/apps`: every app's name, title, icon, instructions, operations and widgets.
- `POST /v1/call { space?, device?, app, op, input }` → `{ ok }`.
- `GET /v1/widgets?space=`: every widget, rendered.
- `POST /v1/spaces { name }`, `POST /v1/invite { space?, mine }` → `{ ticket }`, `POST /v1/join { ticket }`, `POST /v1/device { name }`, `POST /v1/remove { space?, device }`.
- `GET /v1/events`: server-sent events `changed { space, apps }`, `peers { space }` and `spaces`.
- `POST /hooks/<app>/<name>`: for agent harnesses' hooks. The body is the hook's JSON; the reply is plain text for the agent. The node remembers per `session_id` when the hook last ran and passes it to the app as `since` (null on a session's first call).

## Apps

An app is JavaScript: one module whose default export is

```js
{
  name, title, icon, instructions,
  schema,                         // SQL run on its empty view database
  ops: { name: { description, input, run(ctx, input) } },
  fold: { kind(db, entry) },      // no clock, no randomness, no I/O
  widgets: { name: { title, render(ctx) } },
  hooks: { name(ctx, { event, since }) },  // returns text for the agent
}
```

`ctx` has `append(kind, data)`, `query(sql, params)`, `me` and `now()`. A fold handler gets `db.exec(sql, params)` and `db.query(sql, params)`, and the entry `{ author, person, name, time, data }`.

## Widgets

A widget renders to a tree of nodes, drawn natively by each shell and as HTML in chats:

- `{ t: "col" | "row", gap?, c: [nodes] }`
- `{ t: "text", text, size?: "xl" | "lg" | "md" | "sm", weight?: "medium" | "bold", tone?: "muted", lines? }`
- `{ t: "time", at, size?, tone? }`: a time in milliseconds, shown relative to now.

Renderers skip node types and fields they don't know.
