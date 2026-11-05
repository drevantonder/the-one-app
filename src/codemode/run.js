// Runs one code mode script for `one run` and MCP's execute (src/code.rs), with OpenCode's interpreter: a JavaScript
// subset with no eval, network, files or timers, that reaches the world only through `tools`. Each tool call goes back
// to the command that started this, which runs it as the person, as any MCP call; this side only asks.
//
// stdin: first { code, tools: [{ app, tool, description, input, confirm }], limits }, then { reply, ok | error } for
// each call. stdout: { call, app, tool, input } for each call, then { done: result }.
import { CodeMode, Namespace, Tool, toolError } from "@opencode/codemode"
import { Effect } from "effect"

const waiting = new Map()
let started
const lines = (async function* () {
  let rest = ""
  for await (const chunk of Bun.stdin.stream()) {
    rest += new TextDecoder().decode(chunk)
    let at
    while ((at = rest.indexOf("\n")) >= 0) {
      yield rest.slice(0, at)
      rest = rest.slice(at + 1)
    }
  }
})()

const first = await lines.next()
const { code, tools: list, limits, descriptions } = JSON.parse(first.value)
;(async () => {
  for await (const line of lines) {
    if (!line.trim()) continue
    const { reply, ok, error } = JSON.parse(line)
    const w = waiting.get(reply)
    waiting.delete(reply)
    if (w) error === undefined ? w.resolve(ok) : w.reject(error)
  }
})()

let next = 0
const say = (message) => process.stdout.write(JSON.stringify(message) + "\n")
const call = (t, input) =>
  new Promise((resolve, reject) => {
    const id = ++next
    waiting.set(id, { resolve, reject })
    say({ call: id, app: t.app, tool: t.tool, input })
  })

const byPath = new Map()
const apps = {}
for (const t of list) {
  byPath.set(`${t.app}.${t.tool}`, t)
  ;(apps[t.app] ??= {})[t.tool] = Tool.make({
    description: t.description ?? "",
    input: t.input ?? { type: "object" },
    output: {},
    execute: (input) => Effect.tryPromise({ try: () => call(t, input), catch: (e) => toolError(String(e)) }),
  })
}
const tools = Object.fromEntries(Object.entries(apps).map(([app, tools]) => [app, Namespace.make({ description: descriptions?.[app] ?? "", tools })]))

const runtime = CodeMode.make({
  tools,
  limits,
  hooks: {
    // What sends or can't be undone runs only with confirm: true, given once the person agreed, as everywhere else.
    "tool.before": ({ name, input }) => {
      const t = byPath.get(name)
      if (t?.confirm && input?.confirm !== true)
        return Effect.fail(toolError(`${t.app}_${t.tool} sends or can't be undone: ask the person first, then call it again with confirm: true`))
      return Effect.void
    },
  },
})

say({ done: await Effect.runPromise(runtime.execute(code)) })
process.exit(0)
