// Records a page as a native host would get it: `bun record.js <the page's built folder> <answers.json>` runs the
// page's own bundle, as app.js built it, with the UI library (dist/client), as a host's sandbox would, answers each tool it calls from answers.json (`{ "<tool>": <its answer> }`), and anything else it asks its client by
// its method (`{ "ui/initialize": { "hostContext": { "titled": true } } }`), and prints
// the mutations it made, one a line, then its semantic tree (src/drawing-primitives/outline.ts). Build the UI library
// first.
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { fileURLToPath } from "node:url"
import { outline } from "./src/drawing-primitives/outline.ts"

const dist = fileURLToPath(new URL("dist/client/", import.meta.url))
// What a page imports of the UI library, from where it's built, as a host serves it at `/ui/client/`.
Bun.plugin({
  name: "ui",
  setup(build) {
    build.onResolve({ filter: /^\/ui\/client\// }, ({ path }) => ({ path: join(dist, path.slice("/ui/client/".length)) }))
  },
})

/** Runs the page that `index.html` in `folder` loads, as a native host would, with `answers` for its tools.
 * → each mutation it made, as `[name, ...arguments]`, and the tree they drew. */
export async function record(folder, answers) {
  const made = []
  const drawn = new Map([[0, { id: 0, type: "box", props: {}, children: [] }]])
  const tell = (message) => queueMicrotask(() => globalThis.ui.message({ jsonrpc: "2.0", ...message }))
  const mutation = (name, apply) => (...args) => (made.push([name, ...args]), apply(...args))
  globalThis.mutations = {
    create: mutation("create", (id, type, value) => drawn.set(id, { id, type, props: value === undefined ? {} : { value }, children: [] })),
    set: mutation("set", (id, name, value) => void (drawn.get(id).props[name] = value)),
    insert: mutation("insert", (parent, id, before) => {
      const [into, node] = [drawn.get(parent), drawn.get(id)]
      node.parent?.children.splice(node.parent.children.indexOf(node), 1)
      node.parent = into
      const at = before === undefined ? -1 : into.children.indexOf(drawn.get(before))
      into.children.splice(at < 0 ? into.children.length : at, 0, node)
    }),
    remove: mutation("remove", (parent, id) => {
      const node = drawn.get(id)
      drawn.get(parent).children.splice(drawn.get(parent).children.indexOf(node), 1)
      node.parent = undefined
    }),
    drop: mutation("drop", (id) => void drawn.delete(id)),
  }
  globalThis.host = {
    post({ id, method, params }) {
      if (id === undefined) return
      if (method !== "tools/call") return method in answers ? tell({ id, result: answers[method] }) : tell({ id, error: { message: `${method} isn't answered here` } })
      // `{ "$error": … }` is its refusal and `{ "$image": "data:…" }` a picture, as scenes.js answers them.
      const answer = answers[params.name]
      if (answer === undefined || answer?.$error) return tell({ id, result: { isError: true, content: [{ type: "text", text: answer?.$error ?? `No answer for ${params.name}` }] } })
      const [, mimeType, data] = answer?.$image?.match(/^data:(.*?);base64,(.*)$/) ?? []
      tell({ id, result: { content: [mimeType ? { type: "image", mimeType, data } : { type: "text", text: JSON.stringify(answer) }] } })
    },
  }
  const page = readFileSync(join(folder, "index.html"), "utf8").match(/<script type="module"[^>]*src="\.?\/?([^"]+)"/)[1]
  await import(join(folder, page))
  // What it asked for answered and drawn: until a turn of the event loop changes nothing more.
  for (let seen = -1; seen !== made.length; ) {
    seen = made.length
    await new Promise((turn) => setTimeout(turn))
  }
  return { made, root: drawn.get(0) }
}

/** The semantic tree of what a native host drew. */
export function semantic(node) {
  const { type, props } = node
  if (type === "raw-text") return { text: props.value ?? "", states: {}, children: [] }
  const states = {}
  for (const [name, value] of Object.entries(props)) if (name.startsWith("aria-") && value !== null && name !== "aria-label") states[name.slice(5)] = String(value)
  if (props.disabled) states.disabled = "true"
  const role = props.role ?? (type === "input" ? (props.type === "search" ? "searchbox" : "textbox") : type === "image" && props.alt ? "img" : undefined)
  const hidden = props["aria-hidden"] === "true" || (type === "image" && !props.alt) || (type === "path" && !props.role)
  return hidden ? { states: {}, children: [] } : { role, name: props["aria-label"] ?? (type === "image" ? props.alt : undefined), states, children: node.children.map(semantic) }
}

if (import.meta.main) {
  const { made, root } = await record(process.argv[2], JSON.parse(readFileSync(process.argv[3], "utf8")))
  for (const mutation of made) console.log(JSON.stringify(mutation))
  console.log(`\n${outline(semantic(root))}`)
}
