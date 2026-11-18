// Lynx's main thread: applies protocol v1 updates to Lynx elements with Element PAPI, and sends taps and typing back
// to the app in the background thread. A node's element is made when it's first inserted, once its static props have
// come, so a box that scrolls can be a <scroll-view>.
import { css } from "./style.js"
import { TOKENS } from "./tokens.gen.js"

const colour = (name) => TOKENS[name] ?? name
const BASE = "display:flex;flex-direction:column;flex-shrink:0"

const nodes = new Map() // id -> { type, props, el, parent }
let page

function ensure(id) {
  const n = nodes.get(id)
  if (n.el) return n.el
  const scroll = n.type === "box" && n.props.style?.overflow === "scroll"
  switch (n.type) {
    case "box": n.el = scroll ? __CreateScrollView(0) : __CreateView(0); break
    case "text": n.el = __CreateText(0); break
    case "image": n.el = __CreateImage(0); break
    case "input": n.el = __CreateElement("input", 0); break
    case "raw-text": n.el = __CreateRawText(n.props.value ?? ""); return n.el
  }
  if (scroll) __SetAttribute(n.el, "scroll-orientation", "vertical")
  for (const [name, value] of Object.entries(n.props)) apply(n, name, value)
  return n.el
}

const TRAIT = { button: "button", img: "image", heading: "header", link: "link", listitem: "none", searchbox: "none" }

function apply(n, name, value) {
  const el = n.el
  switch (name) {
    case "style": {
      const base = n.type === "box" ? BASE + ";" : n.type === "input" ? "width:100%;flex-shrink:0;" : n.type === "image" ? "flex-shrink:0;" : ""
      __SetInlineStyles(el, base + css(value, colour, true))
      if (n.type === "text") __SetAttribute(el, "text-maxline", value?.lineClamp ? String(value.lineClamp) : undefined)
      break
    }
    case "value":
      if (n.type === "raw-text") __SetAttribute(el, "text", value ?? "")
      else __SetAttribute(el, "value", value ?? "")
      break
    case "label":
      __SetAttribute(el, "accessibility-element", value != null)
      __SetAttribute(el, "accessibility-label", value ?? undefined)
      break
    case "role":
      if (TRAIT[value]) __SetAttribute(el, "accessibility-traits", TRAIT[value])
      if (value === "heading") __SetAttribute(el, "accessibility-heading", true)
      break
    case "src":
      __SetAttribute(el, "src", value == null ? undefined : value.startsWith("data:") ? value : `asset:///web/${value}`)
      break
    case "placeholder": case "type": case "disabled":
      __SetAttribute(el, name, value ?? undefined)
      break
    case "onPress": __AddEvent(el, "bindEvent", "tap", value ? `${n.id}:press` : undefined); break
    case "onInput": __AddEvent(el, "bindEvent", "input", value ? `${n.id}:input` : undefined); break
    case "onSubmit": __AddEvent(el, "bindEvent", "confirm", value ? `${n.id}:submit` : undefined); break
    case "onFocus": __AddEvent(el, "bindEvent", "focus", value ? `${n.id}:focus` : undefined); break
    case "onBlur": __AddEvent(el, "bindEvent", "blur", value ? `${n.id}:blur` : undefined); break
    default:
      // The rest of the semantic tree (level, description, states) has no Lynx attribute yet: kept for 6b.
      break
  }
}

let updates = 0
function applyOps(data) {
  const update = typeof data === "string" ? JSON.parse(data) : data.data ? JSON.parse(data.data) : data
  for (const op of update.ops) {
    switch (op.op) {
      case "create":
        nodes.set(op.id, { id: op.id, type: op.type, props: op.type === "raw-text" ? { value: op.value } : {}, el: undefined })
        break
      case "set": {
        const n = nodes.get(op.id)
        n.props[op.name] = op.value
        if (n.el) apply(n, op.name, op.value)
        break
      }
      case "insert": {
        const parent = op.parent === 0 ? page : ensure(op.parent)
        const child = nodes.get(op.id)
        const pType = op.parent === 0 ? "box" : nodes.get(op.parent).type
        // Solid inserts an empty text as a placeholder anywhere; outside a text it's an empty box.
        if (child.type === "raw-text" && pType !== "text" && !child.el) child.type = "box"
        const el = ensure(op.id)
        if (op.before !== undefined) __InsertElementBefore(parent, el, ensure(op.before))
        else __AppendElement(parent, el)
        break
      }
      case "remove":
        __RemoveElement(op.parent === 0 ? page : nodes.get(op.parent).el, nodes.get(op.id).el)
        break
      case "drop":
        nodes.delete(op.id)
        break
    }
  }
  updates++
  __FlushElementTree(page, { __lynx_timing_flag: updates === 1 ? "__lynx_timing_actual_fmp" : `update-${updates}` })
}

globalThis.renderPage = function () {
  page = __CreatePage("0", 0)
  __SetInlineStyles(page, "display:flex;flex-direction:column;height:100%")
  nodes.set(0, { id: 0, type: "box", props: {}, el: page })
}
globalThis.updatePage = function () {}
globalThis.updateGlobalProps = function () {}
globalThis.getPageData = function () { return null }
globalThis.removeComponents = function () {}
globalThis.processData = function (d) { return d }
globalThis.applyOps = applyOps
