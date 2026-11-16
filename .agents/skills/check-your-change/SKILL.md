---
name: check-your-change
description: Use before you commit a change, to choose the cheapest check that shows it works, and to know whether it needs one of the repo's six kept tests.
---

Check your work with a throwaway test and don't commit it. Pick the cheapest thing that shows the change works:
call the tool on a local network (the `local-network` skill), open the screen (`see-it-running`), or run the one
function in `iex -S mix` in `core/server/`. Run it once. Commit when it passes.

A failing check is chased until you know why, and reported. Never skip it, loosen it, or commit "a small fix" on a
guess you haven't reproduced.

## The kept tests

Each starts its own network on this machine. Run one only when your change touches what it guards; their headers
say what they do and how to run them.

| Test | Guards | Run it when you change |
| --- | --- | --- |
| `core/server/test/who.sh` | who may do what: relation tuples, groups, sharing links, a plugin's reach | permissions, relations, calls between plugins, the clock |
| `plugins/vault/test/vault.sh` | secrets: encryption, the Vault, letting a device in, the recovery key | encryption, the Vault, sign-in to outside services, anything that holds a secret |
| `plugins/backups/test/backups.sh` | data kept and restored | backups, the bucket's layout, stored names |
| `core/server/test/failover.sh` | a database outlives the server writing it | the database plugin, leases, servers taking over |
| `core/server/test/install.sh` | how releases are built, installed and updated | `tooling/release`, `one serve`, `one update`, the server's release (about 15 min) |
| `clients/app/test/isolation.sh` | what a plugin's app may reach in the real app | the app's bridge, its web view, `clients/app/` |

Once `cargo build --release` has run, `failover.sh` takes about 30 s and `isolation.sh` about 10 s here. `who.sh`,
`vault.sh` and `backups.sh` each start a whole network with `tooling/network`, so minutes; `install.sh` about 15. Run `install.sh` on its own: it builds into the repo's shared `target/`, which makes a test running beside it rebuild everything and time out.

## Mistakes already made

- Reading the codebase from scratch. Start from the files your task names; `rg` for the rest.
- Running every test, or the 15-minute install test, for a change nowhere near what it guards.
- Re-running tests after rebasing onto other people's unrelated commits.
- A test that read a log before the process had created it, and failed for that. Follow a log with `tail -F`, as
  `tooling/network` does, never by reading it once.
- Merging a fix whose one test was failing, on the guess that the test was wrong.
