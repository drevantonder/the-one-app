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

Your machines run a small node and become **servers**. Each server has owners and users, and holds only what they may see: your laptop holds your data and your groups'; a team box holds its team's, and the data of each member who chooses to trust it. A server the install owns holds everything and serves everyone: a **privileged server**, because admins trust it, not because the code says so. Servers find each other by key over [iroh](https://iroh.computer), with no account and no VPN, and keep working when some are off. On a machine with a node, the CLI, MCP, a browser and the Mac app use it directly; your phone is a **client** that asks your servers.

Everything lives in one of three scopes: yours, a group's (a team, a family), or everyone's. Anyone can start a group and invite people; being in one shares the group's data, never your own. Everything anyone does is a signed entry, which goes only to the servers that hold its scope; every view is rebuilt by replaying entries in one shared order, so every server shows the same once it has caught up. Nobody, admins included, sees your data unless you trust their server, and an admin opening it is something you see.

Inside run **apps**: JavaScript with tools for agents, screens and widgets. `one deploy <dir>` puts an app's code into the install; every app runs its latest deploy everywhere, and everyone can read its code as written. You install apps for yourself, group admins for their group, and admins for everyone. Apps reach what only some machines can do (iMessage on a Mac, a local model, a shell, WhatsApp's linked session) through **drivers**, native code a server's owners install on it. Apps keep tokens as **secrets**, which they use but never see. Nothing is built in.

## What's in it

A new install starts with the apps `brand.toml` lists:

- **Board**: one-line posts agents and people leave each other in topics: shared memory for every agent.
- **Agents**: one AGENTS.md and one set of skills, written into every agent harness on every one of your servers.
- **Repos**: git hosting. `git clone one://board` gives you an app to start from; you push your own branches (`main@you`) to anyone's repo as suggestions, and its owner moves `main`.
- **Shell** and **Models**: run a command, or ask a local model, on whichever of your servers can.
- **iMessage**, **Reminders**, **Calendar**, **Contacts**, from your Mac; **Gmail**, **WhatsApp** and **Desktops**.

Fork any app under a new name, or write your own, and `one deploy` it. Every operation is also an MCP tool and a CLI command.

Not all of this is built yet: "Built so far" in `ARCHITECTURE.md` says what is.

## Start

```sh
cargo build --release
sh install.sh target/release/one         # runs the node as a service
one create Home --as Andre               # a new install, on your first server; keep its recovery key in 1Password
one pair                                 # a pairing code for your next server or phone
one join <code>                          # on it
one board post --text "hello from here"
one group create Family                  # share data with others: one group invite Family <user>
```

For agents: `claude mcp add --scope user one -- one mcp`.

On a Mac, `apple/build.sh` builds the app with the node and the Apple drivers inside it.

## Make it yours

The app is whitelabelled: `brand.toml` sets its name, CLI name, port, accent colour, starter apps and what it calls its privileged servers, and nothing else in the code names the product. `brands/` has two more: `BRAND=brands/ptc.toml cargo build --release` builds The PTC App; install it under its CLI name (`ptc`).

Each context is its own app: The One App is yours, The PTC App is PTC's. Each has its own CLI, port, data, keys and MCP server, so someone in several runs several side by side. A brand may give its privileged servers a name of its own ("the PTC servers").

## How it works

`ARCHITECTURE.md` is the map, and `SPEC.md` is the protocol every machine speaks. `tests/sim.rs` runs hundreds of servers over simulated links that drop, duplicate and reorder messages, split apart and skew clocks, and checks each ends up with exactly what it should hold.
