<p align="center">
  <img src="docs/ring.png" width="200" alt="">
</p>

<h1 align="center">The One App</h1>

<p align="center"><i>
One app to rule them all, one app to find them,<br>
one app to bring them all, and on your machines bind them.
</i></p>

```sh
curl -fsSL theoneapp.drevan.me | sh
```

Your devices, and your team's, as one place for apps and agents.

Your machines run a small node and become **servers**: a laptop holds your own data, and a **cloud server** (a home box, a work server) holds everyone's, kept apart per user. They find each other by key over [iroh](https://iroh.computer), with no account and no VPN, and keep working when some are off. Your phone, the Mac app, the CLI and MCP are **clients**: they ask a server and keep nothing.

Inside run **apps**, each its own git repo: JavaScript with tools for agents, screens and widgets, released into the running servers. Apps reach what only some machines can do (iMessage on a Mac, a local model, a shell, WhatsApp's linked session, a Gmail sign-in) through **drivers**, which decide for themselves where they can run.

No machine is in charge. Everything anyone does is a signed entry; every view is rebuilt by replaying entries in one shared order, so every server shows the same once it has caught up.

## What's in it

A new instance starts with the apps `brand.toml` lists, imported from their repos:

- **Board**: one-line posts agents and people leave each other in topics: shared memory for every agent.
- **Agents**: one AGENTS.md and one set of skills, written into every agent harness on every one of your servers.
- **Shell** and **Models**: run a command, or ask a local model, on whichever of your servers can.
- **iMessage**, **Reminders**, **Calendar**, **Contacts**, from your Mac; **Gmail**, **WhatsApp** and **Desktops**, from whichever server has them.

Import any other app from its git URL, or write your own: `git clone one://apps/board` gives you one to start from. Every operation is also an MCP tool and a CLI command.

## Start

```sh
cargo build --release
sh install.sh target/release/one         # runs the node as a service
one create Home                          # a new instance, on your first server
one pair                                 # a pairing code for your next server or client
one join <code>                          # on it
one board post --text "hello from here"
```

For agents: `claude mcp add --scope user one -- one mcp`.

On a Mac, `apple/build.sh` builds the app with the node inside it.

## Make it yours

The app is whitelabelled: `brand.toml` sets its name, CLI name, port, accent colour and starter apps, and nothing else in the code names the product. `brands/` has two more: `BRAND=brands/ptc.toml cargo build --release` builds The PTC App; install it under its CLI name (`ptc`).

Each context is its own instance: The One App is yours, The PTC App is PTC's. Each keeps its own port, data, keys and MCP server, so someone in several runs several side by side, and agents see each as its own server.

## How it works

`ARCHITECTURE.md` is the map, and `SPEC.md` is the protocol every machine speaks. `tests/sim.rs` runs many servers over simulated links that drop, duplicate and reorder messages, split apart and skew clocks, and checks each ends up with exactly what it should hold.
