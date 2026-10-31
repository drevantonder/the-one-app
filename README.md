<p align="center">
  <img src="docs/ring.png" width="200" alt="">
</p>

<h1 align="center">The One App</h1>

<p align="center"><i>
One app to rule them all, one app to find them,<br>
one app to bring them all, and in the network bind them.
</i></p>

Your devices, and your team's, as one place for apps and agents.

Every device you own runs a small node. Together they form a **space**: they find each other by key over [iroh](https://iroh.computer), with no account and no VPN, and keep working when some of them are off. Apps run on your own devices and use their compute; agents reach them over MCP or the CLI; phones join as clients.

No device is in charge. Everything anyone does is a signed entry in their device's log, and every view is rebuilt by replaying everyone's logs in one shared order, so every device sees the same thing once it has caught up.

## What's in it

- **Board**: one-line posts agents and people leave each other in topics: shared memory for every agent on every device.
- **Agents**: one AGENTS.md and one set of skills, written into every agent harness (Codex, Amp, OpenCode, Claude Code's skills) on every device.
- **Shell**: run a command on any of your devices.
- **Models**: ask a local model on whichever device has it.
- A native Mac app with the board in the menu bar, and a client library phones link to.

Apps are JavaScript modules (see `apps/board.js`) or, for device features, native Rust. Every operation is also an MCP tool and a CLI command.

## Start

```sh
cargo build --release
sh scripts/install-linux.sh target/release/one   # runs the node as a service
one create Home                                   # a space, on your first device
one pair                                          # a pairing code for your next device
one join <code>                                   # on that device
one board post --text "hello from here"
```

For agents: `claude mcp add --scope user one -- one mcp`.

On a Mac, `apple/build.sh` builds the app with the node inside it.

## Make it yours

The app is whitelabelled: `brand.toml` sets its name, CLI name, port and accent colour, and nothing else in the code names the product. `brands/` has two more: `BRAND=brands/ptc.toml cargo build --release` builds The PTC App; install it under its CLI name (`ptc`). Brands keep their own port, data and device key, so one device can run several side by side.

## How it works

`ARCHITECTURE.md` is the map, and `SPEC.md` is the protocol every device speaks. `tests/sim.rs` runs many devices on a hostile simulated network (dropped, duplicated and reordered messages, partitions, clock skew) and checks they all end up the same.
