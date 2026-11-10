# Storage and permissions: what's left

Decided by Andre on 2026-11-05 and done on 2026-11-06: every scope's data is SQLite databases kept as signed steps in the
bucket, and who can do what is the scope database's rows, asked with `may` (`src/permissions/`). The code and
ARCHITECTURE.md say how. This file keeps what they can't: why, what stays on purpose, and what was tried and not shipped.

## Why

The network kept the log beside its databases: signed entries kept forever and replayed by every device in causal order.
It grew forever, every device replayed all of it, its order was never final so it couldn't be compacted, and who may do
what was 85 hand-written checks, plus 35 admin checks elsewhere and each app's own.

## Left, and why

- **Readers of stored data under old shapes stay**, since the data is signed or sealed and can't be written again: `database::migrate` (steps' old kinds and names), steps with no signature beside them (`stored`), `_grants` (`keys::app_schedules_key_name_before`, sealed today), a bucket whose root is under `install` (`node::bucket_root`, which writes `network` the first time it's read), and a scope database copy whose services name no scope.
- **`src/key/`, `src/blob/` and `src/call/`** are each one file or the node's today, so they wait until there's more than one file to put there.
- **The frozen logs** (`<scope>/node/log/`) stay in the bucket untouched, as history: nothing reads them, and backups copy them as any database ("history").

## Not shipped: clients syncing with TanStack DB

Spiked on 2026-11-05, since today's API gives it nothing to do; it waits for the node to give write positions, which wait until a screen is seen needing them (ARCHITECTURE.md, Decided).

- What was tried: Settings › Users (it writes, and has a hand-written optimistic layer) read and wrote through a TanStack DB collection, in `one-dev throwaway client web` and `android`: optimistic at once, rolled back with the problem box on a failed write, kept on a good one.
- Size: the collection alone is 121 KB minified, 36 KB gzipped (`@tanstack/db` 1.0.0-rc.1.3, still a release candidate, whose transaction API changed under the spike: `isPersisted` became `when("settled")`). Settings' whole screen kit is 86 KB, 29 KB gzipped, so it'd be more than the kit, on every screen that used it. `@tanstack/offline-transactions` adds 32 KB (8 KB gzipped).
- Speed: no difference you can measure. The click on Users to its list showing took 60–80 ms either way, also at 4× slower CPU; running the bundle takes about 2 ms.
- Code: Users' own optimistic layer is about a dozen lines. The collection saved 3 lines of the screen and cost an adapter of about 35.
- Why: with no position from a write, the optimistic layer can only be dropped by fetching all of `/api/state` again, which is what Users does now. `changed` names an app, not rows, so a collection is a whole snapshot with an optimistic layer on top.
- Offline: `offline-transactions` queues writes in IndexedDB, which a screen's frame can't open (`SecurityError`, as `localStorage`); it needs a storage adapter over the bridge's `one.cache`, and a way to run a queued write again that the node says is safe to repeat.
- What it needs from the node first: a write's answer says the database and step position it wrote; a read that waits until the node's (or the client's) copy reaches a position; and a change notice that names rows or a position range, not only an app.

## Open

- **The default roles:** the core's are `permissions::DEFAULTS`. The network's admins keep adding users to any group
  (their `manager` role gives a group's `member`), but make no one its admin: decide who makes a new admin of a group whose
  admins have all gone.
- **Busy app databases** that collide often: measure before changing anything (batching, then a lease).
