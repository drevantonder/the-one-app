// The WebView baseline: the same app and op stream, drawn to the DOM in the same page, as today's webview draws.
import { start } from "./screen.js"
import { css } from "./style.js"

const TAG = { box: "div", text: "span", image: "img", input: "input" }
const ROLE_TAG = { heading: "h1" }
const nodes = new Map([[0, document.getElementById("root")]])
const meta = new Map([[0, { type: "box" }]])
const colour = (name) => `var(--${name})`
const EVENTS = { onPress: "click", onInput: "input", onFocus: "focus", onBlur: "blur" }
let event

function apply(id, name, value) {
  const el = nodes.get(id)
  const m = meta.get(id)
  if (m.type === "raw-text") { el.data = value ?? ""; return }
  switch (name) {
    case "style": el.style.cssText = (m.type === "box" ? "display:flex;flex-direction:column;flex-shrink:0;" : "") + css(value, colour, false); break
    case "label": value == null ? el.removeAttribute("aria-label") : el.setAttribute(m.type === "image" ? "alt" : "aria-label", value); break
    case "role": if (value === "heading") el.setAttribute("aria-level", "1"); value == null ? el.removeAttribute("role") : el.setAttribute("role", value); break
    case "level": el.setAttribute("aria-level", value); break
    case "src": el.src = value; break
    case "placeholder": case "type": el[name] = value ?? ""; break
    case "value": el.value = value ?? ""; break
    default:
      if (EVENTS[name]) {
        const type = EVENTS[name]
        const ev = name.slice(2).toLowerCase()
        el[`on${type}`] = value ? (e) => event({ v: 1, op: "event", id, name: ev, value: e.target.value }) : null
      }
  }
}

let updates = 0
function applyUpdate(update) {
  for (const op of update.ops) {
    switch (op.op) {
      case "create":
        meta.set(op.id, { type: op.type })
        nodes.set(op.id, op.type === "raw-text" ? document.createTextNode(op.value ?? "") : document.createElement(TAG[op.type]))
        break
      case "set": apply(op.id, op.name, op.value); break
      case "insert": nodes.get(op.parent).insertBefore(nodes.get(op.id), op.before === undefined ? null : nodes.get(op.before)); break
      case "remove": nodes.get(op.parent).removeChild(nodes.get(op.id)); break
      case "drop": nodes.delete(op.id); meta.delete(op.id); break
    }
  }
  if (++updates === 1) globalThis.Host?.drawn()
  else globalThis.Host?.updated()
}

event = start(applyUpdate)
