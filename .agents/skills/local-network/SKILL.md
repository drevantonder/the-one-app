---
name: local-network
description: Use to run a whole network of your own on this machine, to try a change, call its tools as different users, or open the app against it, without touching the owner's live network.
---

    tooling/network <folder> [--standby] [--edge] [--owner <name>]

Its header says what each flag does. It builds what isn't built (`cargo build --release`, the UI library,
`mix deps.get && mix compile`), starts the server, and prints how to use it once it's running. From a fresh
worktree that's about two minutes here; after that, seconds. Run it in the background and let your harness tell
you when it ends; put the folder in your scratch folder.

## Acting as someone

    export ONE_HOME=<folder>/client ONE_AS=owner
    target/release/one relations group --name family
    ONE_AS=alice target/release/one relations groups
    ONE_FOR=network target/release/one deploy --name <plugin> --files-from plugins/<plugin>

Always set `ONE_HOME`: without it, `one` (yours or the installed one) is the owner's live client on the live
network. `ONE_AS` picks the user, since nobody signs in on a network being built; `ONE_FOR=groups/<name>` or
`ONE_FOR=network` runs a call for a group or the network.

## Logs and stopping

Everything is in the folder: `server.log` (and `standby.log`, `edge.log`, `bucket.log` with those flags), the
bucket, and the client's home. Starting again in the same folder carries on with what's there.

Stop it by its PID: `kill <pid of tooling/network>` stops every process it started, since each is its own process
group. Never `pkill` or `killall`: other agents run networks on this machine too.

## Mistakes already made

- Running a test or `iex -S mix` in a fresh worktree, which has no deps: `mix deps.get` in `core/server/` and
  `cargo build --release` first (`tooling/network` does both itself).
- Rebuilding from scratch in a new worktree each time. Keep one worktree built and rebase it, rather than making a
  new one for every check.
- Starting the app where there's no desktop without `dbus-run-session` (see `see-it-running`).
- Setting `CARGO_TARGET_DIR`: `tooling/network` and the tests run `target/release/one` from the repo.
