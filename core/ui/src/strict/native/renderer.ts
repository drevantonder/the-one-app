// Solid's universal renderer for a native host: what a strict page draws, as the strict protocol's ops
// (../protocol.ts), sent to the host (`globalThis.host`) once a turn of the event loop, and what the person does,
// from the host, to the handler it's for. A native host loads it as `/ui/client/web.js`, in place of Solid's DOM
// renderer: a strict page's bundle, built for the DOM, imports only what both have (`createComponent`, `memo`).
import { createRenderer } from "@solidjs/universal"
import { VERSION, type Event, type Host, type Op, type Type } from "../protocol.ts"

/** A node as the sandbox keeps it: where it is in the tree, and the handlers the host's events go to. */
export interface Node {
  id: number
  type: Type
  parent?: Node
  children: Node[]
  handlers: Record<string, (event: object) => void>
}

const host = (globalThis as { host?: Host }).host
let ops: Op[] = []
let next = 1
/** Every node JS still holds, by its id, for the events the host sends to it. */
const nodes = new Map<number, WeakRef<Node>>()
/** A node JS no longer holds is dropped by the host too. */
const held = new FinalizationRegistry<number>((id) => {
  nodes.delete(id)
  queue({ op: "drop", id })
})

/** The root the host gives: the app's window, or its frame. */
export const root: Node = { id: 0, type: "box", children: [], handlers: {} }

/** What's to send, sent once what's running now has run, as one batch. */
function queue(op: Op) {
  if (!ops.length) queueMicrotask(send)
  ops.push(op)
}
function send() {
  const batch = ops
  ops = []
  host?.update({ v: VERSION, ops: batch })
}

function node(type: Type, value?: string): Node {
  const made: Node = { id: next++, type, children: [], handlers: {} }
  nodes.set(made.id, new WeakRef(made))
  held.register(made, made.id)
  queue(value === undefined ? { op: "create", id: made.id, type } : { op: "create", id: made.id, type, value })
  return made
}

const types = new Set<Type>(["box", "text", "image", "input"])

export const { render: mount, effect, memo, createComponent, createElement, createTextNode, insertNode, insert, spread, setProp, mergeProps, applyRef, ref } = createRenderer<Node>({
  createElement(tag) {
    if (!types.has(tag as Type)) throw new Error(`A strict page draws a box, a text, an image or an input, not a ${tag}`)
    return node(tag as Type)
  },
  createTextNode: (value) => node("raw-text", value),
  replaceText(text, value) {
    queue({ op: "set", id: text.id, name: "value", value })
  },
  isTextNode: (node) => node.type === "raw-text",
  setProperty(node, name, value) {
    if (/^on[A-Z]/.test(name)) {
      if (typeof value === "function") node.handlers[name] = value as (event: object) => void
      else delete node.handlers[name]
      queue({ op: "set", id: node.id, name: name as "onClick", value: typeof value === "function" ? true : null })
    } else queue({ op: "set", id: node.id, name: name as "role", value: value ?? null })
  },
  insertNode(parent, node, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const at = anchor ? parent.children.indexOf(anchor) : -1
    parent.children.splice(at < 0 ? parent.children.length : at, 0, node)
    queue(anchor ? { op: "insert", parent: parent.id, id: node.id, before: anchor.id } : { op: "insert", parent: parent.id, id: node.id })
  },
  removeNode(parent, node) {
    parent.children.splice(parent.children.indexOf(node), 1)
    node.parent = undefined
    queue({ op: "remove", parent: parent.id, id: node.id })
  },
  getParentNode: (node) => node.parent,
  getFirstChild: (node) => node.children[0],
  getNextSibling: (node) => node.parent?.children[node.parent.children.indexOf(node) + 1],
})

/** What the person did, from the host, to the handler it's for: `click` to `onClick`. An `input` is told what's in
 * it now, as the DOM's `event.currentTarget.value`. */
export function event({ id, name, value }: Event) {
  const handler = nodes.get(id)?.deref()?.handlers[`on${name[0].toUpperCase()}${name.slice(1)}`]
  const target = { value: value ?? "" }
  handler?.({ currentTarget: target, target })
}

/** Draws a page at the root. */
export const render = (code: () => Node) => mount(code, root)
