// The WebView baseline: the same app and op stream, drawn to the DOM in the same page, as today's web view draws.
import { start as notifications } from "./screen.js"
import { start as form } from "./form.js"
import { css } from "./style.js"

const TAG = { box: "div", text: "span", image: "img", input: "input" }
const nodes = new Map([[0, document.getElementById("root")]])
const meta = new Map([[0, { type: "box" }]])
const colour = (name) => `var(--${name})`
const EVENTS = { onClick: "click", onInput: "input", onFocus: "focus", onBlur: "blur", onKeyDown: "keydown" }
let event

function apply(id, name, value) {
  const el = nodes.get(id)
  const m = meta.get(id)
  if (m.type === "raw-text") { el.data = value ?? ""; return }
  if (name === "style") {
    el.style.cssText = (m.type === "box" ? "display:flex;flex-direction:column;flex-shrink:0;" : "") + css(value, colour, false)
  } else if (name === "src") {
    el.src = value
  } else if (name === "value") {
    el.value = value ?? ""
  } else if (EVENTS[name]) {
    const type = EVENTS[name]
    el[`on${type}`] = value ? (e) => event({ v: 1, op: "event", id, name: type, value: e.target.value, key: e.key }) : null
  } else if (value == null || value === false) {
    el.removeAttribute(name)
  } else {
    el.setAttribute(name, value === true ? "" : String(value))
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

event = (location.hash === "#form" ? form : notifications)(applyUpdate)
