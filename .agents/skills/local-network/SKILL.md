---
name: local-network
description: Use to run a whole network of your own on this machine, to try a change, call its tools as different users, or open the app against it, without touching the owner's live network.
---

    tooling/network <folder> [--standby] [--edge] [--owner <name>]

Its header says what each flag does. It builds what isn't built (the UI library, then the server's `mix compile` as it
starts, which builds the Rust workspace's debug build with `cargo build`), starts the server, and prints how to use it once it's
running. A worktree's first build starts from a copy of the build of a worktree beside it (`tooling/copy-build`, which
`tooling/network` and `mix compile` run; run it yourself before a first `cargo`), so its first network builds only what
differs, and starts in about 20 s; after that, in about 10 s.
Run it in the background and let your harness tell you when it ends; put the folder in your scratch folder.

## The fastest loop

- An app, One Core's Elixir, the UI library (`core/ui`) or any Rust changed: just call a tool, or reload the app's
  page. Before a client's request, the network's code reloader (`tooling/code_reloader.exs`) loads what changed into
  every server, as a server's update hot-loads: One Core's modules while it runs, the Rust workspace built again
  (`one` and every NIF, each loaded beside the build before), the UI library built again and served, and an app
  deployed as `one deploy --files-from` deploys it. A build error, or what a running server can't load (a supervisor,
  a struct, `-on_load`: the rule a hot load follows), is that request's error, saying so.
- The deps or the config changed, or a request says to: stop it and start it again in the same folder.
- A commit, or a rebase that moves it, builds `one` again once (about 5 s): a build is named by its commit.

## Acting as someone

    export ONE_HOME=<folder>/client ONE_AS=owner
    target/debug/one permissions group --name family
    ONE_AS=alice target/debug/one permissions groups
    ONE_SUBJECT=network target/debug/one deploy --name <app> --files-from apps/<app>

Always set `ONE_HOME`: without it, `one` (yours or the installed one) is the owner's live client on the live
network. `ONE_AS` picks the user, since nobody signs in on a network being built; `ONE_SUBJECT=groups/<name>` or
`ONE_SUBJECT=network` runs a call for a group or the network.

`owner`, `alice` and `bob` start with their encryption set up, their keys on `<folder>/client`'s device, so the
Vault opens at once in `one` and in the app run with that home. Anyone else is set up with `one encryption approve`
then `one vault grant --servers <key>` (`one vault security` lists them).

## Signing a device in

A device that isn't `<folder>/client` (a phone, the simulator, another client home) needs a network with an
address, `--edge`. Put `{"address": "<the address>"}` in its home's `network` file (or build it with
`TAURI_ONE_ADDRESS`), open it, and it shows a code. Then, as whoever it's to be, one step:

    ONE_AS=alice one sign-in approve --code WDJB-MJHT

On a network being built anyone may be named, the owner too, with no `--owner` and no passkey. For an emulator,
`adb reverse tcp:<port> tcp:<port>` first, `adb shell run-as app.theone.client sh -c 'cat > network'` writes its
`network`, and `adb shell pm grant app.theone.client android.permission.POST_NOTIFICATIONS` skips Android's question.
The app says its code in `adb logcat -s RustStdoutStderr`, as `this device's session: SignIn { … code: "…" }`. To
open the Vault there too, let it hold the key from here: `ONE_AS=alice one encryption approve --device <its id in
one sign-in devices>`.

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

- Copying `<folder>/client`'s `key` and `network` onto a phone: that client acts as whoever `ONE_AS` says and is
  nobody's at the address, so the phone says it was removed once its token ends. Approve the phone by its code.
- Starting a network with `--owner` and joining it with a virtual passkey just to approve a phone.
- Running `iex -S mix` in a fresh worktree, which has no deps: `mix deps.get` in `core/server/` first
  (`tooling/network` does it itself).
- Restarting the network to try a change to an app, rather than deploying it into the running one.
- Starting the app where there's no desktop without `dbus-run-session` (see `qa`'s Screens).
- Setting `CARGO_TARGET_DIR`: `tooling/network` runs `target/debug/one` from the repo, and worktrees sharing one run
  each other's builds.
