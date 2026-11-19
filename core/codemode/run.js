// Runs one code mode script, for MCP's execute and `one run`, with OpenCode's interpreter: a JavaScript subset with no
// eval, network, files or timers, that reaches the world only through `tools`. It runs bundled (`bun run build`) in a
// server's QuickJS sandbox (`core/server/nif/codemode`), which gives `__say` and calls `run` once with the script, then
// `reply` with each call's answer. Each tool call goes back to the server (`OneCore.CodeMode`), which runs it as the
// person, as any MCP call; this side only asks. Its limits and how its result reads are here.
//
// run({ code, tools: [{ name, app, description, input, output }], namespaces: { <app>: what it's for } }) gives
// { ok, text }. __say({ call, name, input }) for each call; reply({ reply, ok | error }) answers it.
import { CodeMode, Namespace, Tool, toolError } from "@opencode/codemode"
import { Effect } from "effect"

// No limit on its time or its calls, as OpenCode's Core has it: it runs until it's done, or whatever started it stops
// it. What it gives back is kept to 60 KB, which an agent's client takes whole (Claude Code keeps 25,000 tokens of an
// MCP tool's answer).
const limits = { maxOutputBytes: 60_000 }

const waiting = new Map()
let next = 0
const call = (name, input) =>
  new Promise((resolve, reject) => {
    const id = ++next
    waiting.set(id, { resolve, reject })
    __say(JSON.stringify({ call: id, name, input }))
  })

globalThis.reply = (line) => {
  const { reply, ok, error } = JSON.parse(line)
  const call = waiting.get(reply)
  waiting.delete(reply)
  if (call) error === undefined ? call.resolve(ok) : call.reject(error)
}

globalThis.run = async ({ code, tools: list, namespaces = {} }) => {
  // An app's tool is tools.<app>.<tool>, in a namespace that says what the app is for, which the script's
  // `search()` ranks by as One Core's `search` does (`OneCore.Search`), and One Core's own is tools.<name>.
  const tools = {}
  const apps = {}
  for (const t of list) {
    const tool = Tool.make({
      description: t.description ?? "",
      input: t.input ?? { type: "object" },
      // What it answers, so a signature says its type. A tool that says nothing of it answers unknown.
      output: t.output ?? {},
      execute: (input) => Effect.tryPromise({ try: () => call(t.name, input), catch: (e) => toolError(String(e)) }),
    })
    if (t.app) (apps[t.app] ??= {})[t.name.slice(t.app.length + 1)] = tool
    else tools[t.name] = tool
  }
  for (const [app, its] of Object.entries(apps))
    tools[app] = Namespace.make({ description: namespaces[app] ?? "", tools: its })

  const result = await Effect.runPromise(CodeMode.make({ tools, limits }).execute(code))

  // What it returned, or what went wrong, with what it logged.
  let text = result.ok
    ? typeof result.value === "string"
      ? result.value
      : JSON.stringify(result.value)
    : `${result.error?.kind ?? "Error"}: ${result.error?.message ?? ""}`
  if (result.logs?.length) text += `\n\nLogged:\n${result.logs.join("\n")}`
  for (const warning of result.warnings ?? []) text += `\n\nWarning: ${warning.message}`
  if (result.truncated) text += `\n\n(cut at ${limits.maxOutputBytes / 1000} KB: return less)`
  return { ok: result.ok, text }
}
