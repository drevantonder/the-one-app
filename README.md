<p align="center">
  <img src="docs/ring.png" width="200" alt="">
</p>

<h1 align="center">The One App</h1>

<p align="center"><i>
One app to rule them all, one app to find them,<br>
one app to bring them all, and in the network bind them.
</i></p>

```sh
curl -fsSL theoneapp.drevan.me | sh
```

Your devices, and your team's, as one place for apps and agents.

Each of your machines runs a small node and becomes one of your **devices**. Each has owners and users, and holds only what they may see: your laptop holds your data and your groups'; a team box holds its team's, and the data of each member who chooses to trust it. A device the install owns holds everything and serves everyone: a **privileged device**, because admins trust it, not because the code says so. Devices reach each other by key through your One App's own address, with no VPN and nothing to open, and keep working when some are off. On a machine with a node, the CLI, MCP, a browser and the Mac app use it directly; your phone is a **client** that asks your devices.

Everything lives in one of three scopes: yours, a group's (a team, a family), or everyone's. Anyone can start a group and invite people; being in one shares the group's data, never your own. Each app's data in a scope is one database, sealed in the install's storage, which only the devices that hold the scope can open; who can do what is a signed log every device reads alike. Nobody, admins included, sees your data unless you trust their device, and an admin opening it is something you see.

Inside run **apps**: JavaScript with tools for agents, screens and widgets. `one deploy <dir>` puts an app's code into the install; every app runs its latest deploy everywhere, and everyone can read its code as written. You install apps for yourself, group admins for their group, and admins for everyone. Apps reach what only some machines can do (iMessage on a Mac, a local model, a shell, WhatsApp's linked session) through **device sidecars**, native code a device's owners install on it. Keys, sign-ins and accounts live in the **Vault**, as items apps are switched on for: they use secret fields but never see them. Nothing is built in.

## What's in it

A new install starts with the apps `config.toml` lists, and the App Store adds the others, among them:

- **Board**: one-line posts agents and people leave each other in topics: shared memory for every agent.
- **AGENTS.md**: one AGENTS.md and one set of skills, written into every agent harness on every one of your devices.
- **Repos**: git hosting for stock git, on every computer in the install: `git clone http://localhost:4747/repos/board.git` gives you an app to start from. A repo's owner pushes to it; anyone else forks it, pushes to their fork, and its owner fetches from there.
- **Shell** and **Models**: run a command, or ask a local model, on whichever of your devices can.
- **iMessage**, **Reminders**, **Calendar**, **Contacts**, from your Mac; **Gmail**, **WhatsApp** and **Agent Devices**, where your agents also use your Mac once you let them.

Fork any app under a new name, or write your own, and `one deploy` it. Every operation is also an MCP tool and a CLI command.

## Start

```sh
cargo build --release
sh install.sh target/release/one         # runs the node as a service
one try                                  # look around first: a new install of its own, beside yours; one try stop deletes it
one create Home --as Andre               # a new install, on your first device; keep its recovery key in 1Password
one pair                                 # a pairing code for your next device
one join <code>                          # on it
one board post --text "hello from here"
one group create Family                  # share data with others: one group invite Family <user>
```

For agents: `claude mcp add --scope user one -- one mcp`.

On a Mac, `apple/build.sh` builds the app with the node and the Apple sidecars inside it. On Linux, `android/build.sh` builds the Android app (the Android SDK in `~/Android/Sdk`, or `$ANDROID_HOME`), and `windows/build.sh` the Windows app, in a Windows VM over SSH (WinUI builds only on Windows), installing and starting it there.

## Make it yours

The app is whitelabelled: `config.toml` sets its name, CLI name, port, icons, release key and starter apps, and nothing else in the code names the product. Another white label is this code built with its own `config.toml`, kept in its own repo, its paths relative to it: `CONFIG=../acme/config.toml cargo build --release`, then install it under its CLI name. Its `update` fetches that repo's latest config beside main. What an install does (what its App Store calls the apps made there, push) is in its settings, which admins change in Settings.

Each context is its own app: The One App is yours, a workplace's white label is theirs. Each has its own CLI, port, data, keys and MCP server, so someone in several runs several side by side.

## How it works

`ARCHITECTURE.md` is the map and its few rules; the code, and its tests, are the rest. `tests/sim.rs` has many devices write one database in one bucket over requests that are slow, fail or land late, and checks every device ends up reading the same.
