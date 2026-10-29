<p align="center">
  <img src="docs/ring.png" width="200" alt="">
</p>

<h1 align="center">The One App</h1>

<p align="center"><i>
One app to rule them all, one app to find them,<br>
one app to bring them all, and in the network bind them.
</i></p>

Every agent you use (Claude Code, Codex, opencode, anything that speaks MCP)
connects to the same node and gets the same instructions, the same memory and
the same tools, through MCP, a CLI or an HTTP API.

Bring your own devices: a laptop, a Raspberry Pi, an old PC. Each runs a node;
they find each other over [Tailscale](https://tailscale.com) and keep each
other in sync, and an S3-compatible bucket (Cloudflare R2, or one you host)
backs everything up, encrypted on your devices. No server required.

## How it's built

A small core, with everything else as plugins.

- **The core** keeps an append-only log, runs plugins, and exposes their
  operations as MCP tools, CLI commands and an HTTP API (with OpenAPI), all
  from one definition.
- **Plugins** are Go programs the core starts as separate processes. Each one
  rebuilds its state by replaying its own log entries, so it never migrates
  stored data. Built-in features use the same contract:
  - `instructions`: your standing instructions for agents, with history.
  - `memory`: long-term memory, a rewrite of
    [OptMem](https://github.com/VictorTaelin/OptMem).
- **Shared state** (the tool list and the MCP instructions agents receive) is rebuilt from
  empty by running each plugin's transforms in order, as in
  [OpenCode 2](https://anoma.ly/notes/opencode-reloaded/). That makes reloads
  safe: a plugin's new generation simply replaces the old one.

## Devices, sync and backup

- **Every device keeps a full copy** and works offline. Each appends to its own
  log; entries sort the same way everywhere (hybrid logical clock, device,
  sequence), so devices converge without a server.
- **Sync** runs node to node over the tailnet, only between machines of the
  same Tailscale user. Changes arrive within a second; devices that are never
  online together exchange entries through the bucket. A late entry from a
  device that was offline makes affected plugins replay their log in order.
- **Backup**: `one bucket setup-r2 <name>` (or `bucket_configure` for any S3
  provider). Logs are encrypted before upload; your other devices pick up the
  bucket automatically. Keep `one bucket kit` in a password manager: with it,
  `one bucket import <kit>` rebuilds a device from the bucket alone.
- **Two kinds of data**, chosen per entry type. *Available* entries can be
  written on any device, even offline, and merge. *Consistent* entries have one
  writer at a time, chosen by a lease in the bucket (conditional writes, with
  an epoch); other devices forward the operation to it.
- **Jobs** (`c.Every(...)`) run once per interval on exactly one device, even
  as devices come and go.
- **Any operation runs on any device that has its plugin.** Every tool lists
  where it is available and takes a `device` argument, e.g.
  `one shell run --command uptime --device linux`. Without one, it runs on this
  device if it can, otherwise on one that can.

## Try it

```sh
curl -fsSL https://raw.githubusercontent.com/drevantonder/the-one-app/main/install.sh | sh
one serve                       # node on 127.0.0.1:4747, MCP at /mcp
one instructions set "Keep answers short."
one memory note "Prefers Go for backend work"
one memory wake
```

On a Mac, build the signed app instead, which the Apple plugins (Reminders,
Calendar, Contacts, iMessage) need: `macos/build.sh install` puts
`TheOneApp.app` in `~/Applications` and links `one` into `~/.local/bin`. Open
the app: it adds a menu bar item and starts the node, unless one is already
running, logging to `~/Library/Logs/TheOneApp/node.log`. macOS then asks for
each permission once, as The One App, and keeps it across updates; a node
started some other way may be asked again. Reading iMessages also needs Full
Disk Access for The One App.

Or build it yourself: `go install github.com/drevantonder/the-one-app/cmd/one@latest`.

Connect your agents with the command `one mcp` (MCP over stdio), the same on
every device. For example, `claude mcp add --scope user the-one-app -- one mcp`.
It relays to the node, which also serves MCP over HTTP at
`http://127.0.0.1:4747/mcp`. Agents receive the MCP instructions plugins build
together: your instructions, and the memory plugin's rule to call
`memory_wake` first.

## Updating

`one update` updates the node on every device of yours that runs one: it
builds the tip of `main` (in a clean clone, never your working tree), sends
each node its build over the tailnet, and each installs it and restarts in
place. Macs running TheOneApp.app get the app, which a Mac's node builds and
signs (`ONE_MAC` names which). `one update --release` takes the latest GitHub
release instead, and needs no Go.

`one release v1.2.3` publishes one: it tests the tip of `main`, signs it off
with [gh-signoff](https://github.com/basecamp/gh-signoff), builds every
platform locally and the notarized app on the Mac, updates every device, and
creates the GitHub release. The app is notarized once the Mac has a
notarytool profile (`xcrun notarytool store-credentials one`); until then it
is only signed, which `install.sh` installs fine, but a browser download needs
Open Anyway in System Settings.

## Plugins with no code

Point a plugin at a service and its tools appear on every device:

```sh
one plugins add gmail --discovery 'https://gmail.googleapis.com/$discovery/rest?version=v1' \
  --oauth google --scopes https://www.googleapis.com/auth/gmail.modify
one gmail sign_in --clientId … --clientSecret …   # once, on a device with a browser
```

The source can be a remote MCP server (`--mcp URL`), a local one
(`--command "npx …"`), or a Google API Discovery document (`--discovery URL`).
The sign-in is stored in the plugin's own log, so it works on every device.
Signing in to another account adds it; with several, every operation takes an
`account` argument, and `accounts` lists them.

A local server can be limited to the devices that have it; the others reach it
through the mesh. For example, computer use on a Mac with
[Cua Driver](https://cua.ai/docs/cua-driver) installed:

```sh
one plugins add computer-use --command "cua-driver mcp" --platforms darwin
```

## Spaces: computers for agents

The built-in `spaces` plugin gives agents desktops of their own, each a
container running [Cua](https://cua.ai)'s Linux image, with no Cua account.
On Linux it uses rootless [Podman](https://podman.io) (with lingering on,
`loginctl enable-linger`, so Spaces outlive logins); on a Mac, Apple's
[container](https://github.com/apple/container) (`brew install container &&
brew services start container`), a light VM per Space. Every device lists
every Space and passes calls on to the one it is on.

```sh
one spaces create --name messenger
one spaces view --name messenger    # a link to the desktop, from any of your devices
```

Sign in to sites in the viewer once; the Space's home folder is a volume of
its own, so sign-ins survive restarts. The Space's tools, from browser control
to clicks and typing, are `spaces` operations too, such as
`one spaces get_browser_state`, with a `space` argument once there are
several. The viewer is served over HTTPS on the tailnet, with
`tailscale serve`.

The built-in `messenger` plugin reads Facebook Messenger this way, from the
browser in the Space called messenger, where you sign in to messenger.com
once: `one messenger chats`, `read --chat …` (an unread chat is marked unread
again), `search --query …` and `send --chat … --text …` (to a recent chat,
or a person by their exact name).

Spaces aren't backed up unless you ask: `one spaces backup --name messenger
--on true` stores the home folder (without caches) in the bucket, encrypted,
when the Space stops and every 6 hours, keeping the last 3. To move a Space,
`release` it, then `create` it on the other device, which restores it.

## Writing a plugin

```go
package main

import "github.com/drevantonder/the-one-app/sdk"

type checkIn struct {
	Habit string `json:"habit"`
}

func main() {
	sdk.Run(sdk.Plugin{ID: "habits", Version: "0.1.0", Setup: func(c *sdk.Ctx) error {
		counts := map[string]int{}
		c.Entry("checkIn", sdk.Available)
		c.OnReset(func() { counts = map[string]int{} })
		sdk.Handle(c, "checkIn", func(_ sdk.Entry, in checkIn) error { counts[in.Habit]++; return nil })
		sdk.Op(c, "check_in", "Record a habit.", sdk.Write, func(call *sdk.Call, in checkIn) (sdk.Result, error) {
			_, err := call.Append("checkIn", in)
			return sdk.Text("Done."), err
		})
		return nil
	}})
}
```

`one plugins link ./habits` builds and loads it; it's rebuilt and hot-reloaded
whenever its files change. `one plugins publish habits` builds it for every
platform and installs it on all your devices, which swap to each new build as
they sync. `habits_check_in` is then an MCP tool,
`one habits check_in run` a CLI command, and `POST /v1/call/habits/check_in`
an API endpoint. A plugin can also run other plugins' operations
(`call.Invoke`) and keep files in the bucket, encrypted (`call.PutFile`,
`GetFile`, `Files`, `DeleteFile`). See [`examples/habits`](examples/habits), and
[`examples/jobs`](examples/jobs) for jobs and consistent entries.

## Views

A plugin can show its results as an [MCP Apps](https://modelcontextprotocol.io/community/seps/1865-mcp-apps-interactive-user-interfaces-for-mcp)
view, an HTML page that MCP hosts like Claude draw: `c.View(name, html, meta)`
adds one at `ui://<plugin>/<name>`, and `c.ShowIn(op, uri, appOnly)` shows an
operation's result in it. The view gets its data through tool calls; it has
no network access unless its CSP allows it.

The menu bar is built like the MCP instructions: the core's items (Devices,
Plugins) come first, then each plugin's `c.Menubar(...)` transform can add its
own view, made for a narrow menu, or reorder or hide any. `one menubar` shows
the result.
