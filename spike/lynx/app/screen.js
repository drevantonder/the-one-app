// The spike's app: the Notifications inbox, written as a strict app would be (box, text, image, input and the
// protocol's props), rendered by Solid 2.0's universal renderer into protocol v1 ops (core/ui/src/strict/protocol.ts).
// The same code runs in Lynx's background thread (ops go to its main thread, which draws with Element PAPI) and in a
// WebView (ops go to a DOM host in the same page), so the two hosts get the same op stream.
import { createRenderer } from "@solidjs/universal"
import { createSignal, createMemo, For, flush } from "solid-js"

let nextId = 1
let batch = []
let send = () => {}
let queued = false
const handlers = new Map() // id -> { press, input, ... }

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

class Node {
  constructor(type) {
    this.id = nextId++
    this.type = type
    this.parent = undefined
    this.children = []
  }
}

const EVENT = { onPress: "press", onInput: "input", onSubmit: "submit", onFocus: "focus", onBlur: "blur" }

const renderer = createRenderer({
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
function h(type, props, ...children) {
  if (typeof type === "function") return renderer.createComponent(type, props)
  const node = renderer.createElement(type)
  for (const [name, value] of Object.entries(props || {})) {
    if (typeof value === "function" && !EVENT[name]) renderer.effect(value, (v, prev) => void renderer.setProp(node, name, v, prev))
    else renderer.setProp(node, name, value)
  }
  for (const child of children) renderer.insert(node, child, null)
  return node
}
const text = (props, ...children) => h("text", props, ...children)

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

export function start(onUpdate) {
  send = onUpdate
  const [query, setQuery] = createSignal("")
  const [read, setRead] = createSignal(new Set())
  const all = Array.from({ length: 30 }, (_, i) => ({
    id: i,
    app: APPS[i % APPS.length],
    title: TITLES[i % TITLES.length],
    from: FROM[i % FROM.length],
    text: TEXTS[(i * 5) % TEXTS.length],
    ago: i < 5 ? `${i * 7 + 2}m` : `${i - 3}h`,
  }))
  const shown = createMemo(() => {
    const q = query().toLowerCase()
    return q ? all.filter((n) => (n.title + n.text + n.from).toLowerCase().includes(q)) : all
  })

  const Row = (n) =>
    h(
      "box",
      {
        role: "listitem",
        label: () => `${read().has(n.id) ? "" : "Unread. "}${n.title}, ${n.from}, ${n.ago} ago`,
        onPress: () => setRead((s) => new Set(s).add(n.id)),
        style: {
          flexDirection: "row",
          alignItems: "center",
          gap: 12,
          paddingBlock: 12,
          borderBottomWidth: 1,
          borderColor: "border",
        },
      },
      h("image", { src: `icons/${n.app}.png`, label: n.from, style: { width: 40, height: 40, borderRadius: "md" } }),
      h(
        "box",
        { style: { flexGrow: 1, flexShrink: 1, gap: 2 } },
        h(
          "box",
          { style: { flexDirection: "row", justifyContent: "space-between", alignItems: "baseline", gap: 8 } },
          text({ style: () => ({ font: read().has(n.id) ? "body" : "label", color: "foreground", lineClamp: 1, flexShrink: 1 }) }, n.title),
          text({ style: { font: "caption-1", color: "muted-foreground", fontVariantNumeric: "tabular-nums" } }, n.ago),
        ),
        text({ style: { font: "footnote", color: "muted-foreground", lineClamp: 2 } }, `${n.from} · ${n.text}`),
      ),
    )

  const root = new Node("box")
  root.id = 0
  nextId = 1
  renderer.render(
    () =>
      h(
        "box",
        { role: "main", style: { flexDirection: "column", padding: "gutter", gap: "gap", backgroundColor: "background", height: "100%" } },
        text({ role: "heading", level: 1, style: { font: "title", color: "foreground" } }, "Notifications"),
        h("input", {
          type: "search",
          placeholder: "Search",
          label: "Search notifications",
          onInput: (value) => setQuery(value),
          style: { height: "control", paddingInline: 12, borderRadius: "md", backgroundColor: "muted", color: "foreground", font: "body" },
        }),
        h(
          "box",
          { role: "list", label: "Inbox", style: { flexDirection: "column", flexGrow: 1, overflow: "scroll" } },
          h(For, { get each() { return shown() }, children: (n) => Row(n) }),
          () => (shown().length ? null : text({ style: { font: "body", color: "muted-foreground" } }, "No notifications.")),
        ),
      ),
    root,
  )
  flush()
  return function event(e) {
    const fn = handlers.get(e.id)?.[e.name]
    if (fn) {
      fn(e.value)
      flush()
    }
  }
}

