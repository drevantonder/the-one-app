---
name: insights
description: Read when someone asks what's failing, slow or broken in their network, how long something took, what AI cost, or to read a trace; and before looking into a bug or a slow tool.
---

Insights keeps what the network's servers, clients and plugins did: a span for each tool call, numbers such as how
long each tool took and what AI used, and warnings, errors and crashes. It's kept in the network for 14 days, up
to 256 MB, and nothing about it leaves. It holds no one's data: names, sizes, durations and outcomes, never what
anyone wrote, was given or answered. What a server recorded arrives here about a minute later.

- `overview` is what the Insights screen in Settings shows: what's failing, what's slowest and what AI cost, over
  the last `hours` (24 if left out, up to 336).
- `errors` lists each kind of error once: where (a span's name, as `<plugin> <tool>`, or what logged it), its
  message, how often, when it was last, and the trace of the newest. Start a bug there.
- `trace` reads one trace whole by its `id`, across every server it went through: each span's depth, server,
  start and duration in milliseconds, outcome, message and events. A call that one server passed to another is
  one trace, the other server's span under the first's.
- `slowest` lists tools by their 95th percentile, with their calls, failures and 50th and 99th percentiles in
  milliseconds, then how long each app took to open, from a tap to its first drawing.
- `metric` gives one metric over time, per step, as its count, sum and quantiles, of the data with the
  `attributes` given: `one.tool.duration` with `{"one.plugin": "backups", "one.tool": "now"}`, say. With no
  `name` it lists every metric there is, with its attributes.
- `spending` gives the tokens AI models took in and gave out, as plugins record them, per model per day, and what they cost where the endpoint says
  (OpenRouter does). It has no prices of its own.
- Whose a call was is shown to the network's owners, and to whoever it was: to anyone else, how the network is
  doing is open, and who did what isn't.

- `record` is how a plugin's tool records more of its call:
  `OneCore.Call.tool(call, "insights", "record", %{"records" => records})`, where Insights is added
  (`OneCore.Call.plugins/1`). Each record has a `name`, in OpenTelemetry's dotted lowercase and following its
  conventions where it has some (`gen_ai.client.token.usage`), and `attributes`: names, sizes and outcomes, never
  what anyone wrote, was given or answered, and never a secret. With a `value` it's a number, summed and spread
  over each minute by its name and attributes and read back with `metric`; with `took`, in milliseconds, it's
  something that ended now, a span under the call, with an `error` if it failed; and otherwise a log line of the
  call, read in its trace.
