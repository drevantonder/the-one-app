# The One App

The One App designed again from the ground up, starting with what an artifact is. The app as it ran before is at the
tag `before-redesign` and on the branch `v1`.

## Running it

`mise install` installs Erlang, Elixir and Bun, and `rustup` installs Rust. On Linux the app is built on GTK and
WebKitGTK (`libwebkit2gtk-4.1-dev`). Then, with mise active in your shell:

    cargo build --release
    (cd ui && bun install && bun build.js)
    cd server && mix deps.get && mix network /tmp/one-network

That starts a whole network on this machine in a folder: its bucket, its one server and a client's home. Starting it
again in the same folder carries on with what's there. In another terminal, use its client:

    export ONE_HOME=/tmp/one-network/client
    target/release/one deploy --name ui --files-from ui/dist
    target/release/one deploy --name tracker --files-from plugins/tracker
    target/release/one tracker add --tracker water --amount 2
    target/release/one tracker list
    target/release/one --help
    target/release/one

`--help` lists every tool, each of which is a command, and `target/release/one mcp` is the same tools as MCP over
stdio. The last, asked nothing, is the app: a home screen with the tracker's app on it. Deploying a plugin again
updates it while the server runs, and its app loads again where it's open. Without `ONE_HOME`, a client is kept where
the system keeps an app's data.

`mix network /tmp/one-network spare` starts a second server on the same bucket, as a spare: stop the first, and the
tracker carries on with nothing lost. `cd server && mix test` runs the tests, and `server/test/takeover.sh` stops
real servers in the middle of writes and checks what's left.

A network with an address has its bucket, its relay and its way in for outsiders there. `address/proof/proof.sh` runs
one whole on this machine, with nothing deployed, and shows each of them working. To put one on Cloudflare, make an R2
bucket and an S3 API token for it, then:

    cd address && bun install && bun run deploy
    for secret in BUCKET ACCESS_KEY_ID SECRET_ACCESS_KEY; do bunx wrangler secret put $secret -c signer/wrangler.jsonc; done
    cd ../server && mix network /tmp/one-network --address https://<where the gateway was deployed>

`BUCKET` is the bucket's S3 address with its name on the end: `https://<account>.r2.cloudflarestorage.com/<bucket>`.

- `server`: One Core, in Elixir, with its Rust add-ons in `server/native`.
- `client`: the client's Rust half, which is the CLI and MCP.
- `client/shell`: the app, a window on that half, and the one program both are.
- `ui`: the kit an app's UI is written with.
- `address`: what sits at a network's address, each a Cloudflare Worker: the gateway, the relay, the signer, the web
  client's files and the way in for outsiders.
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
