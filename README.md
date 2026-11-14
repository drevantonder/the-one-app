# The One App

The One App designed again from the ground up, starting with what an artifact is. The app as it ran before is at the
tag `before-redesign` and on the branch `v1`.

## Running it

`mise install` installs Erlang, Elixir, Bun and a local S3 store, and `rustup` installs Rust. On Linux the app is
built on GTK and WebKitGTK (`libwebkit2gtk-4.1-dev`). Then:

    ./network /tmp/one-network

That builds what isn't built and starts a whole network on this machine, kept in that folder, with the plugins The
One App ships with. It says how to open the app and call a tool. `--spare` starts a second server too, which takes
over when the first stops, and `--address` runs the network's address here as well.

Without `ONE_HOME`, a client is kept where the system keeps an app's data. Where there's no desktop, as under
`xvfb-run`, start the app inside `dbus-run-session`: GTK otherwise waits half a minute for a desktop that isn't
there.

`cd server && mix test` runs the tests. The proofs run real servers and the real app and say what happened:
`server/test/takeover.sh` stops servers in the middle of writes, `server/test/services.sh` runs services on one of
two servers, `server/test/who.sh` shows who may reach what as several users and a scheduled job across a takeover, `address/proof/proof.sh` runs a whole address with nothing deployed, and
`client/shell/test/isolation.sh` has an app try everything it mustn't reach.

To put a network's address on Cloudflare, make an R2 bucket and an S3 API token for it, and write the signer's
secrets to `address/signer/secrets`, a line each: `BUCKET`, the bucket's S3 address with its name on the end
(`https://<account>.r2.cloudflarestorage.com/<bucket>`), `ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, and `FIRST_SERVER`,
which you make up (`openssl rand -hex 16`). Then:

    cd address && bun install && bun run deploy
    cd ../server && mix network /tmp/one-network --address https://<where the gateway was deployed> --first <FIRST_SERVER>

The network's first server shows `FIRST_SERVER` that once, and nobody without it can become the first. It opens
nothing after.

The same app builds for Android with Tauri's CLI (`cargo install tauri-cli`), an Android SDK and its NDK:

    cd client/shell
    ANDROID_HOME=~/Android/Sdk NDK_HOME=~/Android/Sdk/ndk/<version> cargo tauri android build --debug --apk

Its project is `client/shell/gen/android`. A phone's client is kept in the app's own folder, and until a client
can join a network from its screen, it's told its network by a `network` file put there, as `mix network` writes
one for a network with an address.

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
