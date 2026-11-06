# Plan: one storage model, and permissions as data

Decided by Andre on 2026-11-05. Phases 0 to 2 are done, and phase 3 mostly (below). This file goes once phase 3 is done: from then on the code and ARCHITECTURE.md say it.

## Why

The network keeps three kinds of storage:
- app databases, which are SQLite changesets at numbered positions in the bucket;
- the node's database of how things went;
- the log, which is signed entries kept forever and replayed by every device in causal order.

The log is the odd one out, and it carries four costs:
- it grows forever;
- every device replays all of it;
- its order is never final, so it can't be compacted;
- who may do what is 85 hand-written checks in `src/members.rs`, plus 35 admin checks elsewhere and each app's own (`apps/repos/app.js` `owns`, `mayPush`).

Only the scope's own members can write a step that opens (AES-GCM, with the step's name bound in), so no Worker can forge anything already. The order the log's causal replay works out buys only one thing: changing who's in without reaching the bucket, which no other write can do anyway.

## The end state

**The domain is GLOSSARY.md's words**, and a new one comes only when it replaces more than it adds. Here, the database replaces the log, permissions are new, and views go.

**One storage model.** Every scope has SQLite databases:
- each app's;
- the node's, for how things went;
- the **scope database**: people, groups, devices, relations, roles, keys, deploys, apps, services, settings and the Vault's bookkeeping.

Each database is stored and read the same way:
- a copy, plus steps at numbered positions in the bucket;
- a step is written only if its position is free, so the bucket gives one order per database, and a write that loses the race runs again;
- **a step is a signed changeset plus the hash of the database after it.** Readers check the signature, and that the result matches the hash;
- steps are merged in levels (SQLite's changegroup keeps each row's final state);
- steps that a copy covers are deleted after the retention period: 7 years for the scope database, shorter for the rest;
- a copy needs no signer. It's good if its hash matches the hash recorded in the step it covers.

Who holds what:
- nodes and the Cloudflare node keep copies in SQLite;
- clients keep copies through the client crate (`client/`: WebAssembly in browsers, `client/ffi` in native apps), and screens read through TanStack DB, which keeps an optimistic layer until the client's copy reaches the write's position;
- a new device starts from the position and hash handed over by the device that approved it (a Welcome);
- a device offline for longer than the retention period comes back in the same way.

**Permissions** (`src/permissions/`, used by the core and by apps):
- **Principals hold roles on objects.** Principals are people, groups and, later, system users. Agents aren't principals: an agent acts as its principal.
- **Each object type defines its permissions, its verbs.** The core's types are the network, a person's scope, a group's scope, devices, apps in a scope, Vault items and services. Apps declare their own, such as `repo`.
- **A role is data:** a name and a set of permissions on a type. Each type ships simple default roles, and a network adds its own as a signed change.
- **Assigning a role is a relation row.** Rules such as "admin implies member" are rows too.
- **One check, `may(principal, permission, object)`**, as a query over those rows.
- **Unknown means no:** a relation or rule a device doesn't know never grants anything.
- **No escalation:** you give only what you hold.
- **The last admin can't be removed**, and the recovery key sits above everything.
- **Where it's checked:** steps to the scope database are checked by every reader, because they decide who gets keys. Rights inside an app are checked by the node running the call.
- **Rights and keys stay tied:** a relation that lets you read a scope means your devices are sealed its keys, and losing it means a new epoch is owed.
- **Grants become app schedules**, a device running an app's schedules for a principal. They're a property of the device, not a permission.

**The protocol left in code is three rules:**
1. the network's first step and its recovery key;
2. epochs never go down;
3. each step's signer holds the permission that change needs.

A removed device's later steps fail rule 3, because they come after its removal in the one order.

**What's deleted:**
- `src/store.rs`, `src/log.rs`, and most of `src/entry.rs` and `src/node/logs.rs`;
- causal order, ties by hash, cuts and forks;
- replay's hand-written rules;
- grants as a right;
- JavaScript that builds or checks entries (`cloudflare/web/members.js`, the checks in `cloudflare/web/device.js`, `cloudflare/mcp/worker.js`'s reading of entries), and their parts of `tests/contracts.rs`;
- ARCHITECTURE.md's hand-written file map.

In the files affected, roughly 4,800 lines become about 3,000.

**Code layout:**
- the domain's parts: `src/key/`, `src/database/`, `src/permissions/`, `src/blob/`, `src/call/`, `src/app/`;
- what's made of them: devices, the Vault, backups, services and schedules;
- where they run: node, the Workers on Cloudflare, the Cloudflare node and clients.

Each folder's module doc is its architecture. The file map is generated from those docs, and a test fails if a folder has none. ARCHITECTURE.md comes down to one page: what the product is, the trust model, the three rules, and what's designed but not built.

## How we get there without half-implementing: expand, switch, contract

Every phase ends with everything working, sent out, and safe to stop at.

### Phase 0: this plan, and ARCHITECTURE.md stripped back

This phase is docs only.

### Phase 1, expand: there's no wire change, and the log is still the truth

1. **The permissions engine.** It's pure Rust: types, relations, roles as data, default roles, `may`, unknown means no, no escalation, and the last admin.
2. **The scope database**, built deterministically from the existing log on every reader (node, Cloudflare node and client crate) and kept up to date as entries arrive. Its tables are listed under "The scope database's tables" below.
3. **Readers switch to it.** Every use of `Members` or `is_admin`, the client crate's checks, the browser's checks (through the client crate's WebAssembly) and the remote MCP server's reading all query the scope database and `may`. Nothing writes to it yet. **Phase 1 is done when nothing but the builder reads entries.**
4. **Storage, for every database:**
   - signatures and state hashes go in an object beside each step (`<prefix>/<n>.sig`), which older readers ignore;
   - merging in levels;
   - deleting steps that a copy covers, after the retention period.
5. **Apps use `may`.** The node gives apps `ctx.may` and their declared types, and Repos moves to it.
6. **Clients sync with TanStack DB**, against today's node API. **Spiked on 2026-11-05 and not shipped**, since today's API gives it nothing to do; it waits for the node to give write positions (see below).
   - What was tried: Settings › Users (it writes, and has a hand-written optimistic layer) read and wrote through a TanStack DB collection, in `one throwaway client web` and `android`: optimistic at once, rolled back with the problem box on a failed write, kept on a good one.
   - Size: the collection alone is 121 KB minified, 36 KB gzipped (`@tanstack/db` 1.0.0-rc.1.3, still a release candidate, whose transaction API changed under the spike: `isPersisted` became `when("settled")`). Settings' whole screen kit is 86 KB, 29 KB gzipped, so it'd be more than the kit, on every screen that used it. `@tanstack/offline-transactions` adds 32 KB (8 KB gzipped).
   - Speed: no difference you can measure. The click on Users to its list showing took 60–80 ms either way, also at 4× slower CPU; running the bundle takes about 2 ms.
   - Code: Users' own optimistic layer is about a dozen lines. The collection saved 3 lines of the screen and cost an adapter of about 35.
   - Why: with no position from a write, the optimistic layer can only be dropped by fetching all of `/v1/state` again, which is what Users does now. `changed` names an app, not rows, so a collection is a whole snapshot with an optimistic layer on top.
   - Offline: `offline-transactions` queues writes in IndexedDB, which a screen's frame can't open (`SecurityError`, as `localStorage`); it needs a storage adapter over the bridge's `one.cache`, and a way to run a queued write again that the node says is safe to repeat.
   - What it needs from the node first: a write's answer says the database and step position it wrote; a read that waits until the node's (or the client's) copy reaches a position; and a change notice that names rows or a position range, not only an app.

This release is **R1**.

### Phase 2, switch: the scope database becomes the truth (done, 2026-11-06)

One cut-over release (`dc8d0351`). Each network's recovery key signed one freeze, in a ceremony Andre ran: The One App's log froze at its step 833, the PTC App's at 615. The freeze is the first step of `global`'s scope database the recovery key signed, carrying the database there; who's who has been the scope database's signed steps since, checked by `may` at each. Storage signs each step in it and refuses those that aren't (`db::SIGNED`, `db::refused`, `db::copy_holds`). Deleting the steps a copy covers (`DELETE_COVERED_STEPS`) stays off until Andre decides.

### Phase 3, contract: delete, rename, move

Done (2026-11-06):
- **The gate**: a device older than this build is told to update by one number, `database::READS`, which nodes, the relay, sign-in and the URL signer each check as it asks. Devices no longer say what they read or when; `one status` no longer lists devices not updated yet.
- **The freeze, read without replay**: every reader (nodes, clients, browsers, the Cloudflare node, services, backups) takes who's who from the first freeze the recovery key the genesis names signed, its copy checked against its hash, or from its Welcome's hash; a new network writes that freeze as it's made (`steps::first_freeze`). The ceremony (`one switch`, `one switch check`, `one throwaway switch`), `FREEZING`, the freeze's mark, `left_out`, the `gate` entry, `/v1/entries/global`, posting entries, and the JavaScript that signed and checked entries are gone. The recovery key lets a device in by a step it signs.
- **Storage always on**: `Db::open_switched`, `MaybeFrozen`, asking whether the log is frozen before a write, and the per-device frozen flag are gone.
- **Repos** asks the node's `may`, with no fallback for nodes from before it; a browser's record of who's who has no `roles` or `runs` for older tabs.

Left, each its own piece of work:
- **The frozen logs, read only.** Every scope but `global` starts, before its first step, from what its frozen log's entries add up to (`steps::scope_anchor`), which needs `global`'s frozen log replayed to the freeze (`Members::replay`, causal order, forks). A node keeps each scope's database once worked out (`scope-kept/`), but a new node and the Cloudflare node, which keeps nothing, work it out again. To delete replay: each scope gets a signed starting point, such as a step by one of its people's devices naming the hash of its database at the freeze, written once by the first device of theirs on this release; once every scope has one, readers start there and `members::replay`, `entry::causal`, the forks and the log reading go. Backups of history from before the freeze read their entries the same way.
- **The store of entries** (`src/store.rs`, `src/node/logs.rs`, `Replica::store`) still mirrors the frozen logs, for that and for what still asks it: blobs an entry names, backups' snapshots, release entries, device seqs. Each moves to the scope database's rows, then the store goes.
- **Step rules as rows.** A step is applied by `members`' hand-written rules on `Members` read from the database and built back (`steps::apply`); replacing them with SQL over the rows and `may` deletes most of `src/members.rs`.
- **`members.js`** stays: it builds the content of the steps a browser signs, the same content entries carried.
- **Moving folders** and **generating the file map**, as planned. (The renames are done: `app-schedules` steps, the `choose-devices` verb, the `network`, `app_schedules` and `app_schedule_keys` tables, `_app_schedules`, the bucket's `network` object; what older releases wrote is read in these names by `src/database/migrate.rs`.)

## The scope database's tables

This is a draft. Workstreams B and C agree it before writing code.

- `principals(id, kind: person|group|system, name, handle, removed)`
- `devices(key, seal, principal, name, kinds, passkey, removed)`
- `types`, `roles` and `relations`: `permissions::SCHEMA` (`src/permissions/mod.rs`), whose doc says what a subject and an object are. Settled by B.
- `epochs(scope, epoch, key_hash)` and `sealed(scope, epoch, device)`
- `keeps(device, principal, scope, app, until)`
- `apps`, `deploys`, `installs`, `services`, `settings(name, value)`, `vault`, `recovery(key, set_at)`

Every row change is in a signed step, so "who did what" for seven years is the steps themselves, not a column.

## Open questions, with defaults

- **The browser's download** grows with SQLite in WebAssembly. Measure it in phase 1. The fallback is the browser building changesets itself in JavaScript.
- **The scope database's step signer:** a person's device, by default. Nodes sign app databases' steps. Under rule 3, their steps to the scope database are those the scope's permissions allow nodes, such as a node's status.
- **The default roles:** the core's are `permissions::DEFAULTS`; a repo's (owner, collaborator) are G's; a viewer role only once something needs it. The network's admins keep adding people to any group (their `manager` role gives a group's `member`), but, unlike replay's `manages`, make no one its admin: decide before phase 2 who makes a new admin of a group whose admins have all gone.
- **Retention:** 7 years for the scope database, by default. Andre decides app data's.
- **Busy app databases** that collide often: measure before changing anything (batching, then a lease).

## Workstreams

| | Phase | Work | Needs first | Owns |
|---|---|---|---|---|
| A | 0 | Strip ARCHITECTURE.md back | none | `ARCHITECTURE.md` |
| B | 1.1 | The permissions engine | the tables agreed | `src/permissions/` |
| C | 1.2 | The scope database and its builder from the log | the tables agreed | `src/database/scope*`, the builder |
| D | 1.4 | Storage: signatures beside steps, levels, deletion | none | `src/db.rs`, `src/bucket*` |
| E | 1.6 | Clients sync with TanStack DB | none | the screens' shared JavaScript |
| F1 | 1.3 | The node reads the scope database and `may` | B, C | `src/node/`, `src/*.rs` callers |
| F2 | 1.3 | The client crate and the browser | B, C | `client/`, `cloudflare/web/` |
| F3 | 1.3 | The remote MCP server and the Cloudflare node | B, C | `cloudflare/mcp/`, `cloudflare/node/` |
| G | 1.5 | Apps' `ctx.may` and Repos | F1 | `src/app/runtime.rs`, `apps/repos/` |
| H | 2 | Switching over, with its gate | all of phase 1, and every device on R1 | one owner |
| I1–I4 | 3 | Delete, rename to keep, move folders, finish the docs | H | split by area |

Each workstream follows AGENTS.md:
- it works in its own worktree;
- it runs `cargo test` before each commit;
- it pushes to main as it goes, then runs `gh signoff` and `one update`;
- it posts to the board when it starts and when it's done;
- it says what it checked in `one throwaway`.

## Risks

- **Phase 1's builder and reader switch must agree with replay exactly.** C's test builds the scope database from every replay test's entries and checks that `may` agrees with replay's answers.
- **Several threads push to main all day.** Moving folders waits for phase 3 and goes one folder at a time.
- **Phase 2 is the one step that can't be undone easily.** It's gated, it has one owner, and it's tried in `one throwaway`, including a device on R0 that comes back afterwards.
