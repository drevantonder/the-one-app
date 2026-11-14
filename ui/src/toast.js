import { html, nothing } from "lit"
import { repeat } from "lit/directives/repeat.js"
import { Element, look } from "./element.js"
import { Machine, late, over, part } from "./zag.js"
import css from "./toast.css"

/// Zag's toasts, the toasts there are, and the one element of the page that shows them: all there from the first
/// toast on.
let zag
let store
let toaster
/// Where on the page the toaster is: the page itself, or the open sheet or alert that's on top (sheet.js).
let home = document.body

/// Moves toasts to `where`, or back to the page. They go on showing as they were.
export function hold(where = document.body) {
  home = where
  if (toaster?.isConnected && toaster.parentNode !== home) home.append(toaster)
}

/// `toast("Deleted", { action: { label: "Undo", run: restore } })`: one line about what just happened, at the foot
/// of the window for a few seconds, with one thing to do about it. What went well needs none: fine is silent.
export async function toast(text, { action } = {}) {
  zag ??= (await late).toast
  store ??= zag.createStore({ placement: "bottom", gap: 8, max: 3, offsets: "1rem" })
  toaster ??= document.createElement("ui-toaster")
  if (toaster.parentNode !== home) home.append(toaster)
  store.create({ title: text, duration: action ? 8000 : 4000, action: action && { label: action.label, onClick: action.run } })
}

/// Where toasts show: one for the page, over everything else on it.
class Toaster extends Element {
  static styles = look(css)

  machine = new Machine(this, zag.group, () => ({ store }))

  render() {
    const { api } = this.machine
    this.service ??= this.machine.vanilla.service
    return html`
      <div popover="manual" ${part(api.getGroupProps())} ${over(api.getCount() > 0)}>
        ${repeat(api.getToasts(), (toast) => toast.id, (toast, index) => html`
          <ui-toast .toast=${toast} .index=${index} .parent=${this.service}></ui-toast>
        `)}
      </div>
    `
  }
}

/// One toast. It draws into the toaster's shadow root, where the toaster's machine looks for it.
class Toast extends Element {
  static properties = { toast: {}, index: {}, parent: {} }

  machine = new Machine(this, zag, () => ({ ...this.toast, index: this.index, parent: this.parent }))

  createRenderRoot() {
    return this
  }

  render() {
    const { api } = this.machine
    return html`
      <div class="toast" ${part(api.getRootProps())}>
        <span ${part(api.getGhostBeforeProps())}></span>
        <p ${part(api.getTitleProps())}>${api.title}</p>
        ${this.toast.action ? html`<button class="press ring" ${part(api.getActionTriggerProps())}>${this.toast.action.label}</button>` : nothing}
        <span ${part(api.getGhostAfterProps())}></span>
      </div>
    `
  }
}
customElements.define("ui-toast", Toast)
customElements.define("ui-toaster", Toaster)
