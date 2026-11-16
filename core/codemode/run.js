// Runs one code mode script, for MCP's execute and `one run`, with OpenCode's interpreter: a JavaScript subset with no
// eval, network, files or timers, that reaches the world only through `tools`. Each tool call goes back to whatever
// started this, the CLI (`clients/cli/src/codemode.rs`) or a server (`OneCore.CodeMode`), which runs it as the person,
// as any MCP call; this side only asks. Its limits and how its result reads are here, so both say the same.
//
// stdin: first { code, tools: [{ name, plugin, description, input, output }], namespaces: { <plugin>: what it's for } }, then
// { reply, ok | error } for each call. stdout: { call, name, input } for each call, then { done: { ok, text } }.
import { CodeMode, Namespace, Tool, toolError } from "@opencode/codemode"
import { Effect } from "effect"

// Ten minutes, a hundred calls and 60 KB back.
const limits = { timeoutMs: 600_000, maxToolCalls: 100, maxOutputBytes: 60_000 }

const lines = (async function* () {
  let rest = ""
  const decoder = new TextDecoder()
  for await (const chunk of Bun.stdin.stream()) {
    rest += decoder.decode(chunk, { stream: true })
    let at
    while ((at = rest.indexOf("\n")) >= 0) {
      yield rest.slice(0, at)
      rest = rest.slice(at + 1)
    }
  }
})()

const { code, tools: list, namespaces = {} } = JSON.parse((await lines.next()).value)
const waiting = new Map()
;(async () => {
  for await (const line of lines) {
    if (!line.trim()) continue
    const { reply, ok, error } = JSON.parse(line)
    const call = waiting.get(reply)
    waiting.delete(reply)
    if (call) error === undefined ? call.resolve(ok) : call.reject(error)
  }
})()

let next = 0
const say = (message) => process.stdout.write(JSON.stringify(message) + "\n")
const call = (name, input) =>
  new Promise((resolve, reject) => {
    const id = ++next
    waiting.set(id, { resolve, reject })
    say({ call: id, name, input })
  })

// A plugin's tool is tools.<plugin>.<tool>, in a namespace that says what the plugin is for, which the script's
// `search()` ranks by as One Core's `search` does (`OneCore.Search`), and One Core's own is tools.<name>.
const tools = {}
const plugins = {}
for (const t of list) {
  const tool = Tool.make({
    description: t.description ?? "",
    input: t.input ?? { type: "object" },
    // What it answers, so a signature says its type. A tool from before tools said it answers unknown.
    output: t.output ?? {},
    execute: (input) => Effect.tryPromise({ try: () => call(t.name, input), catch: (e) => toolError(String(e)) }),
  })
  if (t.plugin) (plugins[t.plugin] ??= {})[t.name.slice(t.plugin.length + 1)] = tool
  else tools[t.name] = tool
}
for (const [plugin, its] of Object.entries(plugins))
  tools[plugin] = Namespace.make({ description: namespaces[plugin] ?? "", tools: its })

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

say({ done: { ok: result.ok, text } })
process.exit(0)
