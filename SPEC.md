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

An invite is a ticket: `{ space, inviter: EndpointAddr, secret: [u8; 16], person: Option<[u8; 16]> }`, encoded as base32. The joiner opens the stream, sends `{ secret, name }`, and the inviter appends an `admit` and answers `{ space_name }`. The joiner then syncs as a member. A secret works once.

## Calls (ALPN `one/call/1`)

One stream per call: a JSON request `{ space, app, op, input }`, then a JSON reply `{ ok }` or `{ error }`. Only members may call.

## Apps

An app is JavaScript: one module whose default export is

```js
{
  name, title, icon, instructions,
  schema,                         // SQL run on its empty view database
  ops: { name: { description, input, run(ctx, input) } },
  fold: { kind(db, entry) },      // no clock, no randomness, no I/O
  widgets: { name: { title, render(ctx) } },
}
```

`ctx` has `append(kind, data)`, `query(sql, params)`, `me` and `now()`. A fold handler gets `db.exec(sql, params)` and `db.query(sql, params)`, and the entry `{ author, person, name, time, data }`.
