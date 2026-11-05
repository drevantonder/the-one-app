# Plan: one storage model, and permissions as data

Decided by Andre on 2026-11-05. This file goes once phase 3 is done: from then on the code and ARCHITECTURE.md say it.

## Why

The install keeps three kinds of storage:
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
- nodes and the serverless node keep copies in SQLite;
- clients keep copies through the client crate (`client/`: WebAssembly in browsers, `client/ffi` in native apps), and screens read through TanStack DB, which keeps an optimistic layer until the client's copy reaches the write's position;
- a new device starts from the position and hash handed over by the device that approved it (a Welcome);
- a device offline for longer than the retention period comes back in the same way.

**Permissions** (`src/permissions/`, used by the core and by apps):
- **Principals hold roles on objects.** Principals are people, groups and, later, system users. Agents aren't principals: an agent acts as its principal.
- **Each object type defines its permissions, its verbs.** The core's types are the install, a person's scope, a group's scope, devices, apps in a scope, Vault items and services. Apps declare their own, such as `repo`.
- **A role is data:** a name and a set of permissions on a type. Each type ships simple default roles, and an install adds its own as a signed change.
- **Assigning a role is a relation row.** Rules such as "admin implies member" are rows too.
- **One check, `may(principal, permission, object)`**, as a query over those rows.
- **Unknown means no:** a relation or rule a device doesn't know never grants anything.
- **No escalation:** you give only what you hold.
- **The last admin can't be removed**, and the recovery key sits above everything.
- **Where it's checked:** steps to the scope database are checked by every reader, because they decide who gets keys. Rights inside an app are checked by the node running the call.
- **Rights and keys stay tied:** a relation that lets you read a scope means your devices are sealed its keys, and losing it means a new epoch is owed.
- **Grants become "keep"**, a device keeping an app running for a principal. Keep is a property of the device, not a permission.

**The protocol left in code is three rules:**
1. the install's first step and its recovery key;
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
- where they run: node, the Workers on Cloudflare, serverless and clients.

Each folder's module doc is its architecture. The file map is generated from those docs, and a test fails if a folder has none. ARCHITECTURE.md comes down to one page: what the product is, the trust model, the three rules, and what's designed but not built.

## How we get there without half-implementing: expand, switch, contract

Every phase ends with everything working, sent out, and safe to stop at.

### Phase 0: this plan, and ARCHITECTURE.md stripped back

This phase is docs only.

### Phase 1, expand: there's no wire change, and the log is still the truth

1. **The permissions engine.** It's pure Rust: types, relations, roles as data, default roles, `may`, unknown means no, no escalation, and the last admin.
2. **The scope database**, built deterministically from the existing log on every reader (node, serverless and client crate) and kept up to date as entries arrive. Its tables are listed under "The scope database's tables" below.
3. **Readers switch to it.** Every use of `Members` or `is_admin`, the client crate's checks, the browser's checks (through the client crate's WebAssembly) and the MCP worker's reading all query the scope database and `may`. Nothing writes to it yet. **Phase 1 is done when nothing but the builder reads entries.**
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

### Phase 2, switch: the scope database becomes the truth

This phase has one owner. It starts only once **every device listed in the install reports R1 or later**, which the node database's builds show. An admin can override this for a device that's gone for good.

1. **The cut-over.**
   - The first copy of the scope database is the one built from the log at a final position.
   - Every R1 device checks that copy once by building it itself, from the entries it has already checked, in its own release's form, and keeps it. That's the anchor for existing installs.
   - The scope database's first step marks the log frozen: an admin's device signs it, with the copy and the final position (each device's last entry in `global`, and each other scope's log's last step), so exactly one counts.
2. **Changes to who may do what are written as signed steps to the scope database**, from every client through the client crate's code. The log is never written again.
3. **A device that missed the gate and comes back older** still reads the frozen log, so it can't change who's in until it updates. It can't leak anything either:
   - epochs it makes don't count, and steps under them are skipped, which is the rule already;
   - the relay refuses removed devices regardless.
4. **Storage switches on what R1 built**, each one constant in `src/db.rs`, beside each other:
   - `SIGNED_STEPS`: a writer signs a step in it (`db::SIGNED`: the changeset, its signer, its signature and, on a step a copy follows, the state hash, in one create-only write), not beside it in `<n>.sig`. R1 reads both kinds; an older reader passes over a kind it doesn't know, so it waits for the gate.
   - `DELETE_COVERED_STEPS`: the steps a copy covers go after the retention period. An older reader that can't start from the copy would read what's left as the whole database.
   - From the first step a writer signs in it, which records its position (`db::Sig::from`), every reader refuses a step that isn't signed in it, or whose signature or state hash doesn't hold (`db::refused`, `db::copy_holds`); before it, nothing is. The scope database's readers and writers also check its signer (`db::Signers`, `Db::open_checking`).

### Phase 3, contract: delete, rename, move

These are separate pieces of work, in parallel:
- delete the log's code and its JavaScript copies, and trim `tests/contracts.rs`;
- rename grants to keep everywhere: code, CLI, MCP, screens and data, in one change, as AGENTS.md requires;
- move to those folders one at a time, each move a quick commit announced on the board;
- generate the file map, and finish ARCHITECTURE.md;

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
- **The default roles:** the core's are `permissions::DEFAULTS`; a repo's (owner, collaborator) are G's; a viewer role only once something needs it. The install's admins keep adding people to any group (their `manager` role gives a group's `member`), but, unlike replay's `manages`, make no one its admin: decide before phase 2 who makes a new admin of a group whose admins have all gone.
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
| F3 | 1.3 | The MCP Worker and the serverless node | B, C | `cloudflare/mcp/`, `serverless/` |
| G | 1.5 | Apps' `ctx.may` and Repos | F1 | `src/apps.rs`, `apps/repos/` |
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
