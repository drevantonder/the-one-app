# TheOneApp

The one app to rule them all: a personal backend for your AI agents, running on
the devices you already own.

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

## Try it

```sh
go install github.com/drevantonder/the-one-app/cmd/one@latest
one serve                       # node on 127.0.0.1:4747, MCP at /mcp
one instructions set "Keep answers short."
one memory note "Prefers Go for backend work"
one memory wake
```

Connect your agents with the command `one mcp` (MCP over stdio), the same on
every device. For example, `claude mcp add --scope user the-one-app -- one mcp`.
It relays to the node, which also serves MCP over HTTP at
`http://127.0.0.1:4747/mcp`. Agents receive the MCP instructions plugins build
together: your instructions, and the memory plugin's rule to call
`memory_wake` first.

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
an API endpoint. See [`examples/habits`](examples/habits), and
[`examples/jobs`](examples/jobs) for jobs and consistent entries.
