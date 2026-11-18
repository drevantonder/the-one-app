// Solid 2.0's universal renderer sending protocol v1 ops (core/ui/src/strict/protocol.ts), with the DOM's prop and
// event names as a strict app's JSX has them (onClick → `click`, aria-label). The same code runs in the native host's
// QuickJS (ops go to Rust, which draws them) and in a WebView (ops go to a DOM host in the same page), so both hosts
// get the same op stream.
import { createRenderer } from "@solidjs/universal"
import { flush } from "solid-js"

export let nextId = 1
export const resetIds = () => { nextId = 1 }
let batch = []
export let send = () => {}
export const setSend = (fn) => { send = fn }
let queued = false
export const handlers = new Map() // id -> { press, input, ... }

function emit(op) {
  batch.push(op)
  if (!queued) {
    queued = true
    Promise.resolve().then(() => {
      queued = false
      const ops = batch
      batch = []
      send({ v: 1, ops })
    })
  }
}

export class Node {
  constructor(type) {
    this.id = nextId++
    this.type = type
    this.parent = undefined
    this.children = []
  }
}

const EVENT = { onClick: "click", onInput: "input", onFocus: "focus", onBlur: "blur", onKeyDown: "keydown" }

export const renderer = createRenderer({
  createElement(type) {
    const node = new Node(type)
    emit({ op: "create", id: node.id, type })
    return node
  },
  createTextNode(value) {
    const node = new Node("raw-text")
    node.value = value
    emit({ op: "create", id: node.id, type: "raw-text", value })
    return node
  },
  createSentinel() {
    const node = new Node("box")
    emit({ op: "create", id: node.id, type: "box" })
    emit({ op: "set", id: node.id, name: "style", value: { display: "none" } })
    return node
  },
  replaceText(node, value) {
    node.value = value
    emit({ op: "set", id: node.id, name: "value", value })
  },
  isTextNode: (node) => node.type === "raw-text",
  setProperty(node, name, value) {
    const event = EVENT[name]
    if (event) {
      let own = handlers.get(node.id)
      if (!own) handlers.set(node.id, (own = {}))
      own[event] = value
      emit({ op: "set", id: node.id, name, value: value ? true : null })
      return
    }
    emit({ op: "set", id: node.id, name, value: value ?? null })
  },
  insertNode(parent, node, anchor) {
    if (node.parent) node.parent.children.splice(node.parent.children.indexOf(node), 1)
    node.parent = parent
    const at = anchor ? parent.children.indexOf(anchor) : -1
    if (at < 0) parent.children.push(node)
    else parent.children.splice(at, 0, node)
    emit(anchor ? { op: "insert", parent: parent.id, id: node.id, before: anchor.id } : { op: "insert", parent: parent.id, id: node.id })
  },
  removeNode(parent, node) {
    parent.children.splice(parent.children.indexOf(node), 1)
    node.parent = undefined
    emit({ op: "remove", parent: parent.id, id: node.id })
  },
  getParentNode: (node) => node.parent,
  getFirstChild: (node) => node.children[0],
  getNextSibling(node) {
    const siblings = node.parent?.children
    return siblings && siblings[siblings.indexOf(node) + 1]
  },
})

// What compiled JSX does: static props set once, a function prop (not a listener) as an effect.
export function h(type, props, ...children) {
  if (typeof type === "function") return renderer.createComponent(type, props)
  const node = renderer.createElement(type)
  for (const [name, value] of Object.entries(props || {})) {
    if (typeof value === "function" && !EVENT[name]) renderer.effect(value, (v, prev) => void renderer.setProp(node, name, v, prev))
    else renderer.setProp(node, name, value)
  }
  for (const child of children) renderer.insert(node, child, null)
  return node
}
export const text = (props, ...children) => h("text", props, ...children)

const APPS = ["vault", "backups", "telemetry", "notifications", "soft-delete"]
const TITLES = ["Backup finished", "New sign-in", "Request waiting", "Weekly report", "Deleted for good", "Item shared with you"]
const FROM = ["Vault", "Backups", "Telemetry", "Notifications", "Recently Deleted"]
const TEXTS = [
  "Your network was backed up to the family bucket.",
  "A Pixel 9 signed in as you in Brisbane.",
  "Andre asks for the Google client.",
  "3 errors, 12 slow calls, AI cost $0.42 this week.",
  "18 items were deleted for good after 30 days.",
  "Anna shared Wi-Fi with the family.",
]


// Renders `view` into the host's root (node 0) and returns what the host calls with each event.
export function mount(view, onUpdate) {
  setSend(onUpdate)
  const root = new Node("box")
  root.id = 0
  resetIds()
  renderer.render(view, root)
  flush()
  return function event(e) {
    const fn = handlers.get(e.id)?.[e.name]
    if (fn) {
      fn(e.name === "keydown" ? e.key : e.value)
      flush()
    }
  }
}
