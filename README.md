# The One App

The One App designed again from the ground up, starting with what an artifact is. The app as it ran before is at the
tag `before-redesign` and on the branch `v1`.

## Running it

`mise install` installs Erlang and Elixir, and `rustup` installs Rust. Then, with mise active in your shell:

    cargo build --release
    cd server && mix deps.get && mix network /tmp/one-network

That starts a whole network on this machine in a folder: its bucket, its one server and a client's home. Starting it
again in the same folder carries on with what's there. In another terminal, use its client:

    export ONE_HOME=/tmp/one-network/client
    target/release/one deploy --name tracker --files-from plugins/tracker
    target/release/one tracker add --tracker water --amount 2
    target/release/one tracker list
    target/release/one

The last lists every tool, each of which is a command, and `target/release/one mcp` is the same tools as MCP over
stdio. Deploying a plugin again updates it while the server runs. `cd server && mix test` runs the tests.

- `server`: One Core, in Elixir, with its Rust add-on in `server/native`.
- `client`: the client's Rust half, which is the CLI and MCP.
- `plugins`: the plugins The One App ships with, each a folder of its own.

## Reading it

- `VISION.md`: the vision, unchanged.
- `GLOSSARY.md`: the redesign's words.
- `ARCHITECTURE.md`: the redesign's rules.
- `AGENTS.md`: how to build it.
- `STACK.md`: what it's built with, what isn't chosen yet, and what to test first.
- `PRIOR-ART.md`: what exists already, what was set aside and why, and decisions that are easy to miss. Read it
  before designing anything.

Not designed yet: how a client's offline writes reach a server, and what signs users in.

The glossary and the rules name no technology on purpose, to avoid overfitting or getting distracted. What's chosen is
in `STACK.md`, and what else exists is in `PRIOR-ART.md`.

Claims is the pressing need, but a bad platform would mean another rebuild, so the platform comes first.

Later, and not critical: bringing data across from the app as it runs today and from the old PTC app.

Parked, to keep in mind: the public as a who, anyone at all and not signed in, as a `viewer` of what an owner makes
public. Filling in something like a form, and visitors without an account, come after that.

Left open until the rest lands: what starts a network's cloud server when no other server answers for a user or group.
It's a big choice, and fewer other things should be moving first.
