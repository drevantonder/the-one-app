import { html } from "lit"
import { Element, look } from "./element.js"
import { move } from "./screen.js"
import css from "./split.css"

/// `<ui-split><ui-screen heading="Expenses">…</ui-screen><ui-screen slot="detail" heading="Coffee with Sam" back>…</ui-screen></ui-split>`:
/// a list and what it opens (`slot="detail"`): beside it where there's room, pushed over it where there isn't.
/// Each side scrolls by itself, and the split is as tall as the window unless the page says. Set `open`, or call
/// `show()` and `close()`, as a sheet's. While it's open the device's own back, and the detail's Back button, close
/// it, and it tells the page `close`. Beside its list a detail has no Back button.
class Split extends Element {
  static styles = look(css)
  static properties = { open: { type: Boolean, reflect: true } }

  /// The device's back, or forward: what's open is what the history says.
  travelled = (event) => {
    this.byDevice = event.hasUAVisualTransition
    this.open = !!history.state?.uiSplit
  }

  connectedCallback() {
    super.connectedCallback()
    addEventListener("popstate", this.travelled)
    // The page was loaded again with the detail open: what was in it is gone, so it starts shut.
    if (history.state?.uiSplit && !this.open) history.replaceState({ ...history.state, uiSplit: false }, "")
  }

  disconnectedCallback() {
    super.disconnectedCallback()
    removeEventListener("popstate", this.travelled)
  }

  show() {
    this.open = true
  }

  close() {
    this.open = false
  }

  render() {
    return html`
      <div class="panes">
        <div class="list"><slot></slot></div>
        <div class="detail"><slot name="detail"></slot></div>
      </div>
    `
  }

  updated(changed) {
    if (!changed.has("open")) return
    const panes = this.renderRoot.querySelector(".panes")
    const flip = () => panes.toggleAttribute("data-open", this.open)
    // It starts as it's written, and the device has already shown its own back gesture's move.
    const still = changed.get("open") === undefined || this.byDevice || getComputedStyle(panes).getPropertyValue("--beside")
    this.byDevice = false
    if (still) flip()
    else move(this, flip, { back: !this.open })
    // Opening it is one step in the history, so that the device's back is what closes it.
    if (this.open && !history.state?.uiSplit) history.pushState({ ...history.state, uiSplit: true }, "")
    if (!this.open && history.state?.uiSplit) history.back()
    if (!this.open && changed.get("open")) this.emit("close")
  }
}
customElements.define("ui-split", Split)
