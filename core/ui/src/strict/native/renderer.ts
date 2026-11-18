// Solid's universal renderer for a native host: what a strict page draws, as the strict protocol's ops
// (../protocol.ts), sent to the host (`globalThis.host`) once a turn of the event loop, and what the person does,
// from the host, to the handler it's for. A native host loads it as `/ui/client/web.js`, in place of Solid's DOM
// renderer: a strict page's bundle, built for the DOM, imports only what both have (`createComponent`, `memo`).
import { createRenderer } from "@solidjs/universal"
import { dynamicComponent, omit, type Component } from "solid-js"
import { VERSION, type Event, type Host, type Op, type Type } from "../protocol.ts"
import { flat, type Styles } from "../../styles.ts"

/** A node as the sandbox keeps it: where it is in the tree, and the handlers the host's events go to, by the event's
 * name (`keydown` for `onKeyDown`). */
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

const types = new Set<Type>(["box", "scroll", "text", "image", "path", "input"])

export const { render: mount, effect, memo, createComponent, createElement, createTextNode, insertNode, insert, spread, setProp, mergeProps, applyRef, ref } = createRenderer<Node>({
  createElement(tag) {
    if (!types.has(tag as Type)) throw new Error(`A strict page draws a box, a scroll, a text, an image, a path or an input, not a ${tag}`)
    return node(tag as Type)
  },
  // Solid hands over what the page wrote, a number too (`{count()}`), which the DOM makes text of itself.
  createTextNode: (value) => node("raw-text", String(value)),
  replaceText(text, value) {
    queue({ op: "set", id: text.id, name: "value", value: String(value) })
  },
  isTextNode: (node) => node.type === "raw-text",
  setProperty(node, name, value) {
    if (/^on[A-Z]/.test(name)) {
      const event = name.slice(2).toLowerCase()
      if (typeof value === "function") node.handlers[event] = value as (event: object) => void
      else delete node.handlers[event]
      queue({ op: "set", id: node.id, name: name as "onClick", value: typeof value === "function" ? true : null })
    } else if (name === "style" && value && (Array.isArray(value) || "$id" in (value as object))) {
      // Styles go by their ids, from the tables the host has (../protocol.ts's `Styles`), the last winning.
      queue({ op: "set", id: node.id, name: "style", value: flat(value as Styles).map((style) => ("$id" in style ? style.$id : style)) })
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

/** What the person did, from the host, to the handler it's for: `click` to `onClick`, `keydown` to `onKeyDown`, with
 * what the DOM's event would say of it: an `input`'s editing state as `editing`, and its text as
 * `currentTarget.value`; its action key's `action`; a key as `key` with its modifiers; how far a `scroll` has
 * scrolled as `currentTarget.scrollTop`; and a pointer where it is and where the node is drawn
 * (`currentTarget.getBoundingClientRect()`). What a host does with a key by itself isn't the page's, so
 * `preventDefault()` does nothing here. */
export function event({ id, name, editing, action, key, shiftKey, altKey, ctrlKey, metaKey, scrollTop, scrollLeft, rect, ...told }: Event) {
  const handler = nodes.get(id)?.deref()?.handlers[name]
  const target = { value: editing?.text ?? "", scrollTop: scrollTop ?? 0, scrollLeft: scrollLeft ?? 0, getBoundingClientRect: () => rect ?? { x: 0, y: 0, width: 0, height: 0 } }
  handler?.({ ...told, currentTarget: target, target, editing, action, key: key ?? "", shiftKey: !!shiftKey, altKey: !!altKey, ctrlKey: !!ctrlKey, metaKey: !!metaKey, preventDefault() {} })
}

/** Takes `node` out of what it's in, as Solid takes a node out. */
export function detach(node: Node) {
  if (!node.parent) return
  const parent = node.parent
  parent.children.splice(parent.children.indexOf(node), 1)
  node.parent = undefined
  queue({ op: "remove", parent: parent.id, id: node.id })
}

/** Draws a page at the root. */
export const render = (code: () => Node) => mount(code, root)

/** Solid's own `dynamic`, whose third argument, at runtime, draws a source that's a type rather than a component. */
const core = dynamicComponent as unknown as (source: () => unknown, options: object | undefined, type: (type: string, props: object) => Node) => Component<any>

/** A component chosen as it runs, as `@solidjs/web`'s `dynamic`: `source` gives a component, or a primitive's type. */
export const dynamic = (source: () => Component<any> | string | undefined, options?: object) =>
  core(source, options, (type: string, props: object) => {
    const made = createElement(type)
    spread(made, props)
    return made
  })

/** `<Dynamic component={…} />`, as `@solidjs/web`'s. */
export const Dynamic = (props: { component: Component<any> | string | undefined } & Record<string, unknown>) => createComponent(dynamic(() => props.component) as never, omit(props, "component") as never)
