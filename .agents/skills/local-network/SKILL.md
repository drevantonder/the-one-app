---
name: local-network
description: Use to run a whole network of your own on this machine, to try a change, call its tools as different users, or open the app against it, without touching the owner's live network.
---

    tooling/network <folder> [--standby] [--edge] [--owner <name>]

Its header says what each flag does. It builds what isn't built (the UI library, then the server's `mix compile` as it
starts, which builds the Rust workspace with `cargo build --release`), starts the server, and prints how to use it once it's
running. From a fresh worktree that's under two minutes here; after that, about 3 s. Run it in the background and let your
harness tell you when it ends; put the folder in your scratch folder.

## The fastest loop

- A plugin changed: deploy it into the running network, which takes under a second, its app built again too:
  `ONE_SUBJECT=network target/release/one deploy --name <plugin> --files-from plugins/<plugin>`. A changed NIF is built
  again by `mix compile` in `core/server/`, and deployed the same way: it loads beside the build before, with no
  restart.
- Anything else changed (`core/`, `clients/`): stop it and start it again in the same folder. Only what changed is
  built again, so that's seconds.
- A commit, or a rebase that moves it, builds `one` again once (about 30 s): a build is named by its commit.

## Acting as someone

    export ONE_HOME=<folder>/client ONE_AS=owner
    target/release/one permissions group --name family
    ONE_AS=alice target/release/one permissions groups
    ONE_SUBJECT=network target/release/one deploy --name <plugin> --files-from plugins/<plugin>

Always set `ONE_HOME`: without it, `one` (yours or the installed one) is the owner's live client on the live
network. `ONE_AS` picks the user, since nobody signs in on a network being built; `ONE_SUBJECT=groups/<name>` or
`ONE_SUBJECT=network` runs a call for a group or the network.

## Looking inside the server

Each server is an Erlang node: `iex --sname dbg-$$ --cookie $(cat <folder>/cookie) --remsh <node>` attaches, with the
node's name as `tooling/network` prints it. `:recon`, `:recon_trace` and `:observer_cli.start()` are there.

## Logs and stopping

Everything is in the folder: `server.log` (and `standby.log`, `edge.log`, `bucket.log` with those flags), the
bucket, and the client's home. Starting again in the same folder carries on with what's there.

Stop it by its PID: `kill <pid of tooling/network>` stops every process it started, since each is its own process
group. Never `pkill` or `killall`: other agents run networks on this machine too.

## Mistakes already made

- Running `iex -S mix` in a fresh worktree, which has no deps: `mix deps.get` in `core/server/` first
  (`tooling/network` does it itself).
- Restarting the network to try a change to a plugin, rather than deploying it into the running one.
- Rebuilding from scratch in a new worktree each time. Keep one worktree built and rebase it, rather than making a
  new one for every check.
- Starting the app where there's no desktop without `dbus-run-session` (see `qa`'s Screens).
- Setting `CARGO_TARGET_DIR`: `tooling/network` runs `target/release/one` from the repo.
