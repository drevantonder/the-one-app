# What exists already

Read this before designing anything here, so we don't build what exists, look again at what was set aside, or decide
again what's decided. Add to it whenever something turns up that nobody in the conversation knew or remembered.
Everything is as of November 2026, and from reading, not running, unless it says otherwise.

## Exists, and close to what's designed here

None of these is chosen. Each is named so it's weighed before anything like it is built.

- **celld** (celld.dev, Deno, Apache-2.0, v0.6.2, beta): runs a Cloudflare Workers application, Durable Objects
  included, on your own machines. Each cell is a SQLite database shipped to your bucket in Litestream's format. One node
  serves a cell at a time, claimed by a conditional write of a small record to the bucket. Each turn as writer ships to
  its own numbered folder, and a write is confirmed only while the record still names that node. That is nearly this
  redesign's database and lease. Not chosen as the server. Its limits: one application per fleet, unencrypted traffic between
  nodes, no Windows, clients only query a node, and upgrades that need every node stopped.
- **Litestream**: ships a SQLite database's log to a bucket. One writer, no takeover, and no check before an upload, so
  by itself it doesn't stop a server that lost its turn from writing.
- **Zanzibar**: where relation tuples and the relations' names come from. SpiceDB and OpenFGA are services that
  implement it; we keep the tuples ourselves and run neither.
- **TanStack DB**: a store inside a client that screens query, fed by plain requests to a backend. Live queries,
  optimistic changes at the screen, kept on disk in SQLite (browsers, Tauri), and a separate package that queues
  changes made offline. A 1.0 release candidate is reported for 29 October 2026, by secondary sources only. Not named in
  the rules.
- **Better Auth**: a candidate for signing in, not chosen. The old PTC app runs on it.
- **Conditional writes**: Amazon S3, Cloudflare R2, Tigris, Google Cloud Storage and Azure Blob have them. Backblaze B2,
  Hetzner and DigitalOcean Spaces don't, by celld's own tests.

## Not checked yet

- That a Worker can pass requests, and a live connection, to a home server through a tunnel without keeping state.
- That celld can load many plugins inside its one application.

## Looked at and set aside

- **Durable Objects on Cloudflare as where the truth is kept**: their data can't be moved out to your own machine.
- **D1 as a standby**: it has no feed of its changes, and a database per user or group fits it badly.
- **Turso and sqld**: one primary that copies pass writes to. Its newer engine lets many connections write at once on
  one machine, in preview. Neither is several servers writing one database.
- **CouchDB**: two equal servers both writing, but documents and kept conflicts, not SQL. **rqlite**: SQL and no
  conflicts, but three machines.
- **Willow, Earthstar, iroh-willow**: no "write only if absent", last write wins by a clock the writer sets, and
  unreleased or archived. **iroh-blobs and iroh-docs**: a way to copy blobs between devices, not a store.
- **Jazz**: its encrypted design is frozen and its makers wrote up why they left it; Jazz 2 is rows a server is
  trusted with, in alpha.
- **Expo**: no desktop apps, so the shell stays Tauri.
- **iroh for how devices link**: its strength is direct links across the internet, which isn't required here.

## Decided earlier, and easy to miss

- **How devices link.** Five options were compared in a table earlier in November 2026: A, everything through the
  network's address; B, A plus direct on the same local network; C, B plus one simple attempt across the internet;
  D, C plus WebRTC for browsers; E, iroh. The choice was B, with C held back until streaming a screen across the internet
  is common. Most traffic is small requests, and the case to make fastest is an AI on its user's own computer.
- **"Without needing the internet"** in the vision means two devices on one local network can reach each other. It
  never meant a network works with its bucket unreachable.
