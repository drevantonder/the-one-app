---
name: local-network
description: Use to run a whole network of your own on this machine, to try a change, call its tools as different users, or open the app against it, without touching the owner's live network.
---

    tooling/network <folder> [--standby] [--edge] [--owner <name>]

Its header says what each flag does. It builds what isn't built (the UI library, then the server's `mix compile` as it
starts, which builds the Rust workspace's debug build with `cargo build`), starts the server, and prints how to use it once it's
running. A fresh worktree's first build takes a few minutes; after that, a new network starts in about 15 s. Run it in the background and let your
harness tell you when it ends; put the folder in your scratch folder.

## The fastest loop

- An app or One Core's Elixir (`core/server/lib/`) changed: just call a tool. Before a client's request, the
  network's code reloader (`tooling/code_reloader.exs`) loads what changed into every server, in under a second: One
  Core's modules while it runs, and an app deployed as `one deploy --files-from` deploys it, its app built again
  too. A compile error, or what a running server can't load (a supervisor, a struct, `-on_load`: the rule a hot load
  follows), is that request's error, saying so. A changed NIF is built again by `mix compile` in `core/server/`, and
  then loads beside the build before.
- Anything else changed (One Core's NIFs, `core/ui`, `clients/`, the deps), or a request says to: stop it and start it
  again in the same folder. Only what changed is built again, so that's seconds.
- A commit, or a rebase that moves it, builds `one` again once (about 5 s): a build is named by its commit.

## Acting as someone

    export ONE_HOME=<folder>/client ONE_AS=owner
    target/debug/one permissions group --name family
    ONE_AS=alice target/debug/one permissions groups
    ONE_SUBJECT=network target/debug/one deploy --name <app> --files-from apps/<app>

Always set `ONE_HOME`: without it, `one` (yours or the installed one) is the owner's live client on the live
network. `ONE_AS` picks the user, since nobody signs in on a network being built; `ONE_SUBJECT=groups/<name>` or
`ONE_SUBJECT=network` runs a call for a group or the network.

## Looking inside the server

Each server is an Erlang node: `iex --sname dbg-$$ --cookie $(cat <folder>/cookie) --remsh <node>` attaches, with the
node's name as `tooling/network` prints it. `:recon`, `:recon_trace` and `:observer_cli.start()` are there.

## Logs and stopping

Everything is in the folder: `server.log`, `reloader.log` (and `standby.log`, `edge.log`, `bucket.log` with those flags), the
bucket, and the client's home. Starting again in the same folder carries on with what's there.

Start it in the background, however you like (`setsid`, `mise exec`); its PID is then in `<folder>/pid`.
Stop it with `kill $(cat <folder>/pid)`, which stops all of it. Started again in the same folder it keeps its ports,
so a server installed with `one serve on` there reconnects by itself.

## Mistakes already made

- Running `iex -S mix` in a fresh worktree, which has no deps: `mix deps.get` in `core/server/` first
  (`tooling/network` does it itself).
- Restarting the network to try a change to an app, rather than deploying it into the running one.
- Rebuilding from scratch in a new worktree each time. Keep one worktree built and rebase it, rather than making a
  new one for every check.
- Starting the app where there's no desktop without `dbus-run-session` (see `qa`'s Screens).
- Setting `CARGO_TARGET_DIR`: `tooling/network` runs `target/debug/one` from the repo.
