// Records a strict page as a native host would get it: `bun record.js <the page's built folder> <answers.json>` runs the
// page's own bundle, as app.js built it for the DOM, with the UI library built for a native host (dist/client/native),
// as a host's sandbox would, answers each tool it calls from answers.json (`{ "<tool>": <its answer> }`), and anything else it asks its client by
// its method (`{ "ui/initialize": { "hostContext": { "breadcrumb": true } } }`), and prints
// the strict protocol's ops it sent, one batch a line, then its semantic tree (src/strict/outline.ts), which the same
// page's tree on the DOM is to match (`outlineOf` in a page, from `bun record.js --dom`). Build the UI library first.
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { outline } from "./src/strict/outline.ts"

const dist = new URL("dist/client/", import.meta.url).pathname
// What a page imports of the UI library, as a native host loads it.
const native = { "/ui/client/solid.js": "solid.js", "/ui/client/web.js": "native/web.js", "/ui/client/ui.js": "native/ui.js" }
Bun.plugin({
  name: "native",
  setup(build) {
    build.onResolve({ filter: /^\/ui\/client\// }, ({ path }) => ({ path: join(dist, native[path] ?? path.slice("/ui/client/".length)) }))
  },
})

/** Runs the page that `index.html` in `folder` loads, as a native host would, with `answers` for its tools.
 * → each batch of ops it sent, and the tree they drew. */
export async function record(folder, answers) {
  const updates = []
  const drawn = new Map([[0, { id: 0, type: "box", props: {}, children: [] }]])
  const tell = (message) => queueMicrotask(() => globalThis.ui.message({ jsonrpc: "2.0", ...message }))
  globalThis.host = {
    update(update) {
      updates.push(update)
      for (const op of update.ops) apply(drawn, op)
    },
    post({ id, method, params }) {
      if (id === undefined) return
      if (method !== "tools/call") return method in answers ? tell({ id, result: answers[method] }) : tell({ id, error: { message: `${method} isn't answered here` } })
      if (!(params.name in answers)) return tell({ id, result: { isError: true, content: [{ type: "text", text: `No answer for ${params.name}` }] } })
      tell({ id, result: { content: [{ type: "text", text: JSON.stringify(answers[params.name]) }] } })
    },
  }
  const page = readFileSync(join(folder, "index.html"), "utf8").match(/<script type="module"[^>]*src="\.?\/?([^"]+)"/)[1]
  await import(join(folder, page))
  // What it asked for answered and drawn: until a turn of the event loop sends nothing more.
  for (let sent = -1; sent !== updates.length; ) {
    sent = updates.length
    await new Promise((turn) => setTimeout(turn))
  }
  return { updates, root: drawn.get(0) }
}

function apply(drawn, op) {
  const node = drawn.get(op.id)
  if (op.op === "create") drawn.set(op.id, { id: op.id, type: op.type, props: op.value === undefined ? {} : { value: op.value }, children: [] })
  else if (op.op === "set") node.props[op.name] = op.value
  else if (op.op === "insert") {
    const parent = drawn.get(op.parent)
    node.parent?.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const at = op.before === undefined ? -1 : parent.children.indexOf(drawn.get(op.before))
    parent.children.splice(at < 0 ? parent.children.length : at, 0, node)
  } else if (op.op === "remove") {
    drawn.get(op.parent).children.splice(drawn.get(op.parent).children.indexOf(node), 1)
    node.parent = undefined
  } else if (op.op === "drop") drawn.delete(op.id)
}

/** The semantic tree of what a native host drew, as `fromDom` makes the DOM's. */
export function semantic(node) {
  const { type, props } = node
  if (type === "raw-text") return { text: props.value ?? "", states: {}, children: [] }
  const states = {}
  for (const [name, value] of Object.entries(props)) if (name.startsWith("aria-") && value !== null && name !== "aria-label") states[name.slice(5)] = String(value)
  if (props.disabled) states.disabled = "true"
  const role = props.role ?? (type === "input" ? (props.type === "search" ? "searchbox" : "textbox") : type === "image" && props.alt ? "img" : undefined)
  const hidden = props["aria-hidden"] === "true" || (type === "image" && !props.alt)
  return hidden ? { states: {}, children: [] } : { role, name: props["aria-label"] ?? (type === "image" ? props.alt : undefined), states, children: node.children.map(semantic) }
}

if (import.meta.main) {
  if (process.argv[2] === "--dom") {
    // What a page runs to have `outlineOf(document.body)`, its tree as text.
    const [built] = (await Bun.build({ entrypoints: [new URL("src/strict/outline.ts", import.meta.url).pathname], minify: true })).outputs
    const code = await built.text()
    const named = Object.fromEntries([...code.match(/export\s*\{([^}]*)\}/)[1].matchAll(/(\w+) as (\w+)/g)].map(([, inner, outer]) => [outer, inner]))
    console.log(`(()=>{${code.replace(/export\s*\{[^}]*\};?/, "")};window.outlineOf=(element)=>${named.outline}(${named.fromDom}(element))})()`)
  } else {
    const { updates, root } = await record(process.argv[2], JSON.parse(readFileSync(process.argv[3], "utf8")))
    for (const update of updates) console.log(JSON.stringify(update))
    console.log(`\n${outline(semantic(root))}`)
  }
}
