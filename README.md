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

Your machines run a small node and become **servers**. Your laptop is a **user server**: it holds only what you may see. A **cloud server** (a home box, a work server) holds everyone's, and admins choose which servers those are. They find each other by key over [iroh](https://iroh.computer), with no account and no VPN, and keep working when some are off. On a machine with a node, the CLI, MCP, a browser and the Mac app use it directly; your phone is a **client** that asks your servers.

Everything lives in one of three scopes: yours, a group's (a team, a family), or everyone's. Everything anyone does is a signed entry, which goes only to the servers that should hold its scope; every view is rebuilt by replaying entries in one shared order, so every server shows the same once it has caught up. No machine is in charge.

Inside run **apps**, each its own git repo: JavaScript with tools for agents, screens and widgets. Anyone installs apps for themselves, group admins for their group, and admins for everyone. Apps reach what only some machines can do (iMessage on a Mac, a local model, a shell, WhatsApp's linked session, a Gmail sign-in) through **drivers**, which decide for themselves where they can serve. Nothing is built in.

## What's in it

A new install starts with the apps `brand.toml` lists, imported from their repos:

- **Board**: one-line posts agents and people leave each other in topics: shared memory for every agent.
- **Agents**: one AGENTS.md and one set of skills, written into every agent harness on every one of your servers.
- **Shell** and **Models**: run a command, or ask a local model, on whichever of your servers can.
- **iMessage**, **Reminders**, **Calendar**, **Contacts**, from your Mac; **Gmail**, **WhatsApp** and **Desktops**, from whichever server has them.

Import any other app from its git URL, or write your own: `git clone one://board` gives you one to start from, and `git push` releases nothing until you say so. Every operation is also an MCP tool and a CLI command.

## Start

```sh
cargo build --release
sh install.sh target/release/one         # runs the node as a service
one create Home                          # a new install, on your first server; keep its recovery key in 1Password
one pair                                 # a pairing code for your next server or phone
one join <code>                          # on it
one board post --text "hello from here"
```

For agents: `claude mcp add --scope user one -- one mcp`.

On a Mac, `apple/build.sh` builds the app with the node inside it.

Give any server an S3-compatible bucket with `one backup <url>`, and it keeps an encrypted copy of everything it holds; `one restore <url>` on a fresh server brings an install back.

## Make it yours

The app is whitelabelled: `brand.toml` sets its name, CLI name, port, accent colour and starter apps, and nothing else in the code names the product. `brands/` has two more: `BRAND=brands/ptc.toml cargo build --release` builds The PTC App; install it under its CLI name (`ptc`).

Each context is its own app: The One App is yours, The PTC App is PTC's. Each has its own CLI, port, data, keys and MCP server, so someone in several runs several side by side, and agents see each as its own server.

## How it works

`ARCHITECTURE.md` is the map, and `SPEC.md` is the protocol every machine speaks. `tests/sim.rs` runs hundreds of servers over simulated links that drop, duplicate and reorder messages, split apart and skew clocks, and checks each ends up with exactly what it should hold.
