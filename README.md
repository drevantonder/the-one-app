# TheOneApp

The one app to rule them all: a personal backend for your AI agents, running on
the devices you already own.

Every agent you use (Claude Code, Codex, opencode, anything that speaks MCP)
connects to the same node and gets the same instructions, the same memory and
the same tools, through MCP, a CLI or an HTTP API.

> Early days: this is milestone 1, a single device. Syncing between devices
> over Tailscale, and backup to S3-compatible storage, come next.

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
- **Shared state** (the tool list, `wake`, MCP instructions) is rebuilt from
  empty by running each plugin's transforms in order, as in
  [OpenCode 2](https://anoma.ly/notes/opencode-reloaded/). That makes reloads
  safe: a plugin's new generation simply replaces the old one.

## Try it

```sh
go install github.com/drevantonder/the-one-app/cmd/one@latest
one serve                       # node on 127.0.0.1:4747, MCP at /mcp
one instructions set "Keep answers short."
one memory note "Prefers Go for backend work"
one wake
```

Connect an agent to `http://127.0.0.1:4747/mcp`. Its first call should be
`wake`; the server's instructions tell it so.

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
whenever its files change. `habits_check_in` is then an MCP tool,
`one habits check_in run` a CLI command, and `POST /v1/call/habits/check_in`
an API endpoint. See [`examples/habits`](examples/habits).
