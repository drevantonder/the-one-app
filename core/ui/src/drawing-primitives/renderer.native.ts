// Solid's universal renderer for a native host: what a page draws, as mutations of its tree, each a direct call
// of the host's (`globalThis.mutations`, ../renderer.ts), and what the person does, from the host, to the handler it's
// for. A native host loads it as `/ui/client/web.js`, in place of Solid's DOM renderer: a page's bundle, built
// for the DOM, imports only what both have (`createComponent`, `memo`).
import { createRenderer } from "@solidjs/universal"
import { dynamicComponent, omit, type Component } from "solid-js"
import type { Event, Mutations, Type } from "./renderer.ts"
import { flat, type Styles } from "../styles.ts"

/** A node as the sandbox keeps it: where it is in the tree, and the handlers the host's events go to, by the event's
 * name (`keydown` for `onKeyDown`). */
export interface Node {
  id: number
  type: Type
  parent?: Node
  children: Node[]
  handlers: Record<string, (event: object) => void>
}

const mutations = (globalThis as unknown as { mutations: Mutations }).mutations
let next = 1
/** Every node JS still holds, by its id, for the events the host sends to it. */
const nodes = new Map<number, WeakRef<Node>>()
/** A node JS no longer holds is dropped by the host too. */
const held = new FinalizationRegistry<number>((id) => {
  nodes.delete(id)
  mutations.drop(id)
})

/** The root the host gives: the app's window, or its frame. */
export const root: Node = { id: 0, type: "box", children: [], handlers: {} }

function node(type: Type, value?: string): Node {
  const made: Node = { id: next++, type, children: [], handlers: {} }
  nodes.set(made.id, new WeakRef(made))
  held.register(made, made.id)
  mutations.create(made.id, type, value)
  return made
}

const types = new Set<Type>(["box", "scroll", "text", "image", "path", "input", "picker", "video", "frame", "list"])

export const { render: mount, effect, memo, createComponent, createElement, createTextNode, insertNode, insert, spread, setProp, mergeProps, applyRef, ref } = createRenderer<Node>({
  createElement(tag) {
    if (!types.has(tag as Type)) throw new Error(`A page draws a box, a scroll, a text, an image, a path, an input, a picker, a video, a frame or a list, not a ${tag}`)
    return node(tag as Type)
  },
  // Solid hands over what the page wrote, a number too (`{count()}`), which the DOM makes text of itself.
  createTextNode: (value) => node("raw-text", String(value)),
  replaceText(text, value) {
    mutations.set(text.id, "value", String(value))
  },
  isTextNode: (node) => node.type === "raw-text",
  setProperty(node, name, value) {
    if (/^on[A-Z]/.test(name)) {
      const event = name.slice(2).toLowerCase()
      if (typeof value === "function") node.handlers[event] = value as (event: object) => void
      else delete node.handlers[event]
      mutations.set(node.id, name as "onClick", typeof value === "function" ? true : null)
    } else if (name === "style") {
      // Styles go by their ids, from the tables the host has (../renderer.ts's `Styles`), the last winning, each dynamic one's
      // values after it. A falsy one is none, as React Native's is, so a page's `cond && look.x` says nothing.
      mutations.set(node.id, "style", flat(value as Styles).flatMap((style) => (style.$dynamic ? [style.$id, style.$dynamic] : [style.$id])))
    } else mutations.set(node.id, name as "role", (value ?? null) as never)
  },
  insertNode(parent, node, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const at = anchor ? parent.children.indexOf(anchor) : -1
    parent.children.splice(at < 0 ? parent.children.length : at, 0, node)
    mutations.insert(parent.id, node.id, anchor?.id)
  },
  removeNode(parent, node) {
    parent.children.splice(parent.children.indexOf(node), 1)
    node.parent = undefined
    mutations.remove(parent.id, node.id)
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
 * `preventDefault()` does nothing here. What a handler throws is reported (`reportError`), as a browser's listener's is. */
export function event({ id, name, editing, action, key, shiftKey, altKey, ctrlKey, metaKey, scrollTop, scrollLeft, rect, ...told }: Event) {
  const handler = nodes.get(id)?.deref()?.handlers[name]
  const target = { value: editing?.text ?? "", scrollTop: scrollTop ?? 0, scrollLeft: scrollLeft ?? 0, getBoundingClientRect: () => rect ?? { x: 0, y: 0, width: 0, height: 0 } }
  try {
    handler?.({ ...told, currentTarget: target, target, editing, action, key: key ?? "", shiftKey: !!shiftKey, altKey: !!altKey, ctrlKey: !!ctrlKey, metaKey: !!metaKey, preventDefault() {} })
  } catch (error) {
    reportError(error)
  }
}

/** Takes `node` out of what it's in, as Solid takes a node out. */
export function detach(node: Node) {
  if (!node.parent) return
  const parent = node.parent
  parent.children.splice(parent.children.indexOf(node), 1)
  node.parent = undefined
  mutations.remove(parent.id, node.id)
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
