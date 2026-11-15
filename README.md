# The One App

The One App designed again from the ground up, starting with what an artifact is. The app as it ran before is at the
tag `before-redesign` and on the branch `v1`.

## Running it

`mise install` installs Erlang, Elixir, Bun and a local S3 store, and `rustup` installs Rust. On Linux the app is
built on GTK and WebKitGTK (`libwebkit2gtk-4.1-dev`). Then:

    ./network /tmp/one-network

That builds what isn't built and starts a whole network on this machine, kept in that folder, with the plugins The
One App ships with. It says how to open the app and call a tool. `--spare` starts a second server too, which takes
over when the first stops, and `--address` runs the network's address here as well. That's a network being built,
where nobody signs in and a command says who it acts as. With `--address --owner <name>` it's one people sign in
to, with a passkey, and it says where its first owner joins.

Without `ONE_HOME`, a client is kept where the system keeps an app's data. Where there's no desktop, as under
`xvfb-run`, start the app inside `dbus-run-session`: GTK otherwise waits half a minute for a desktop that isn't
there.

`cd server && mix test` runs the tests. The proofs run real servers and the real app and say what happened:
`server/test/takeover.sh` stops servers in the middle of writes, `server/test/services.sh` runs services on one of
two servers, with one that picks up what it wasn't told and a network nobody uses reading nothing,
`server/test/who.sh` shows who may do what as several users, each with their own plugins and servers, and a
scheduled job across a takeover, on a folder or with `--address` through the signer,
`server/test/vault.sh` shows that what's encrypted is read only by the servers its owner chose, and never by a
tool's answer, a log or the bucket, through a sign-in to an outside service and the loss of every device,
`server/test/backups.sh` backs a network up to a disk and to S3-compatible storage, destroys its bucket and
restores it from each with the recovery key, `address/proof/proof.sh` runs a whole address with nothing deployed,
`address/proof/sign-in.sh` has people sign in
with passkeys in real browsers and let in devices and an assistant, and `client/shell/test/isolation.sh` has an app
try everything it mustn't reach, and `server/test/install.sh` has a machine with nothing on it but `one` install a
server, update it, go back from a build that can't start, and remove it.

To put a network's address on Cloudflare, make an R2 bucket and an S3 API token for it, and write the signer's
secrets to `address/signer/secrets`, a line each: `BUCKET`, the bucket's S3 address with its name on the end
(`https://<account>.r2.cloudflarestorage.com/<bucket>`), `ACCESS_KEY_ID`, `SECRET_ACCESS_KEY`, and `FIRST_SERVER`,
which you make up (`openssl rand -hex 16`). The network's address is a name on a zone of that Cloudflare account,
which deploying attaches to the gateway. Then:

    cd ui && bun install && bun build.js
    cd ../address && bun install && bun run deploy https://<the network's address>
    cd ../server && mix network /tmp/one-network --address https://<the network's address> --first <FIRST_SERVER> --owner <name>

The network's first server shows `FIRST_SERVER` that once, and nobody without it can become the first. It opens
nothing after. `--owner` names the network's first owner, and the server says where they join: a link they open to
make their passkey.

A machine that's let in as someone's becomes a server of theirs with `one serve on` (and the network's with
`ONE_FOR=network`), which needs nothing else installed: it downloads the network's newest build of the server for
that kind of machine, and the system keeps it running, a systemd user unit on Linux and a LaunchAgent on a Mac.
`one serve off` stops it and removes it. `./release` builds the server and `one` from the commit checked out and
publishes them to the network of the `one` running it, whose owner it has to be. Nothing updates by itself: the
`update` tool has a server's machine take the newest builds, `one update` does it for this device, and the `builds`
tool says which build each server and device is on.

The same app builds for Android with Tauri's CLI (`cargo install tauri-cli`), an Android SDK and its NDK:

    cd client/shell
    ANDROID_HOME=~/Android/Sdk NDK_HOME=~/Android/Sdk/ndk/<version> cargo tauri android build --debug --apk

And for iPhone on a Mac, with Xcode, `xcodegen` and Rust's iOS targets, signed by an Apple team:

    cd client/shell
    APPLE_DEVELOPMENT_TEAM=<team> cargo tauri ios build --export-method debugging

Their projects are `client/shell/gen/android` and `client/shell/gen/apple`. `xcrun devicectl device install app` puts
what's built in `gen/apple/build/arm64` on a paired iPhone, and `--target aarch64-sim --debug` builds it for the
simulator. A phone's client is kept in the app's own folder, and is told its network's address by a `network` file put
there (`{"address": "https://..."}`), as `mix network` writes one for a network with an address. An app built with
`TAURI_ONE_ADDRESS=https://...` set, on any system, starts on that network with no file there. It starts on that
network's sign-in page.

A workplace's own app, with its own name, identifier and icon, is a Tauri config of its own merged over
`client/shell/tauri.conf.json`, as Tauri merges one:

    {"productName": "The PTC App", "identifier": "app.ptcau.client", "bundle": {"icon": ["/path/to/icon.png"]}}

`cargo tauri build --config <that file>` builds it, and `TAURI_CONFIG="$(cat <that file>)" cargo build --release`
the program alone. Its identifier names the folder its client is kept in too, so it sits beside The One App on one
device. On iPhone, Tauri's CLI puts the name and identifier in the Xcode project as it builds, and `cargo tauri icon
<png>` makes its icons. Android's project names The One App's identifier and name itself (`app/build.gradle.kts`,
`strings.xml`, the folder of `MainActivity`), so a workplace's Android app is made from its own `cargo tauri android
init` for now.

- `server`: One Core, in Elixir, with its Rust add-ons in `server/native`.
- `client`: the client's Rust half, which is the CLI and MCP.
- `client/shell`: the app, a window on that half, and the one program both are.
- `ui`: the kit an app's UI is written with.
- `address`: what sits at a network's address, each a Cloudflare Worker: the gateway, the relay, the signer, the web
  client's files and the way in for a sharing link's holder. `address.js` runs or deploys them, with each plugin's
  part at the address.
- `plugins`: the plugins The One App ships with, each a folder of its own.

## Reading it

- `VISION.md`: the vision, unchanged.
- `GLOSSARY.md`: the redesign's words.
- `ARCHITECTURE.md`: the redesign's rules.
- `AGENTS.md`: how to build it.
- `STACK.md`: what it's built with, what isn't chosen yet, and what to test first.
- `PRIOR-ART.md`: what exists already, what was set aside and why, and decisions that are easy to miss. Read it
  before designing anything.

Not designed yet: how a client's offline writes reach a server.

The glossary and the rules name no technology on purpose, to avoid overfitting or getting distracted. What's chosen is
in `STACK.md`, and what else exists is in `PRIOR-ART.md`.

Claims is the pressing need, but a bad platform would mean another rebuild, so the platform comes first.

Later, and not critical: bringing data across from the app as it runs today and from the old PTC app.

Parked, to keep in mind: the public as a who, anyone at all and not signed in, as a `viewer` of what an owner makes
public. Filling in something like a form, and visitors without an account, come after that.

Left open until the rest lands: what starts a network's cloud server when no other server answers for a user or group.
It's a big choice, and fewer other things should be moving first.
