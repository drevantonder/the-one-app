---
name: server-debug
description: Read when a plugin you deploy misbehaves on a live server and Telemetry's warnings and errors don't say why, to read its debug logs, see its processes or trace a function as it runs.
metadata:
  subtitle: "Look inside a running app"
  about: "For developers: read an app's debug logs, see what it's running and trace it live on your servers, when it goes wrong and the error doesn't say why."
---

Erlang's own ways of looking into a running plugin, for a server nothing attaches to as a node: Logger, observer_cli's
process list and `:recon_trace`. Start with Telemetry (`errors`, `trace`): it keeps every call's span and every
warning and error. Reach for these when that isn't enough.

- `logs` follows what the plugin says to `Logger`, every level from debug up, which Telemetry doesn't keep. The first
  call starts following (`minutes`, `lines`) and answers what's said within `wait` seconds; call the plugin, then
  `logs` again for what it said. `stop` ends it early. Each call waits 25 seconds at most, so any MCP client's
  timeout holds.
- `processes` lists its processes as observer_cli does: memory, reductions, message queue and current function,
  the top `top` by `sort`, and the ETS tables they own. A queue that grows or memory that climbs is where to look.
- `trace` runs `:recon_trace.calls` on one `function` of a `module` (as the plugin wrote it), or all of the
  module's, for `calls` calls or `seconds`, showing each call's arguments and what it returned or raised. Call the
  plugin while it runs, from another shell or agent. Recon's limit counts everyone's calls, so a trace of a function
  that's called a lot stops early, and the server carries on. One trace runs on a server at a time.

Who may: an `editor` of the plugin, as for deploying it. You see only calls running for you, or for `subject`, a group or
the network you own, since what a call was given is its person's. Its processes kept between calls are shown where
everyone it's installed for is `subject`. A plugin that says it's `sensitive` (Vault, Encryption, Sign-in) is never looked
into. In sandbox mode the same tools look inside the plugin's own VM on that server, which a call starts.

Each server runs its own calls: `server` names which to look on, by its machine's name; left out, it's one that
runs the plugin. The answer says which. A follow is on one server, and a client may ask another next time, so pass
the `server` the first answer names to each `logs` call after it, and call the plugin through that one too where
you can.

On a network on your own machine (`tooling/network`), attach to the server with `iex --remsh` instead.
