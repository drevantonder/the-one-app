import { html, nothing } from "lit"
import { Element, look, reduced } from "./element.js"
import { icon } from "./icon.js"
import css from "./screen.css"

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let shown = false
addEventListener("popstate", (event) => {
  shown = !!event.hasUAVisualTransition
  setTimeout(() => (shown = false))
}, true)

let moving

/// `go(() => draw(expense))`: changes which screen shows, as a move. `change` puts the new screen in place, and
/// the browser slides to it from what was there (its View Transitions): in over it, or with `back`, back out
/// from over it: `addEventListener("popstate", () => go(draw, { back: true }))`. Ask the server first: the page
/// holds still until `change` is done. `part` is what moves, when it isn't the whole page. Where the browser can't,
/// or the user asked for no motion, it's a plain swap. From one page to another the browser does this by itself
/// (page.css).
export function go(change, { back = false, part } = {}) {
  if (!globalThis.ViewTransition || !("types" in ViewTransition.prototype) || reduced.matches || shown) return void change()
  part?.style.setProperty("view-transition-name", "ui-screen")
  const move = (moving = document.startViewTransition({ update: change, types: [back ? "ui-back" : "ui-forward", part ? "ui-part" : "ui-page"] }))
  move.finished.finally(() => moving === move && part?.style.removeProperty("view-transition-name"))
}

// Back to a page left earlier is the device's to show. The browser would slide it in as if it were new.
addEventListener("pageswap", ({ activation, viewTransition }) => {
  if (activation?.navigationType === "traverse" && activation.entry.index < activation.from.index) viewTransition?.skipTransition()
})

/// `<ui-screen heading="Expenses">`: a whole view of an app: a bar across its top that stays as it scrolls, with
/// its `heading` and what it does (`slot="action"`: its one main button, or a menu), then what's in it, one part
/// under another. With `back` the bar starts with a Back button, which tells the page `back` and then goes back
/// in the browser's history as the device's own back gesture does, unless the page stopped it.
class Screen extends Element {
  static styles = look(css)
  static properties = { heading: {}, back: { type: Boolean } }

  render() {
    return html`
      <header>
        ${this.back ? html`
          <button class="press ring" aria-label="Back" @click=${this.goBack}>
            <svg viewBox="0 0 256 256" aria-hidden="true">${icon("caret-left")}</svg>
          </button>
        ` : nothing}
        <h1>${this.heading}</h1>
        <slot name="action"></slot>
      </header>
      <main><slot></slot></main>
    `
  }

  goBack() {
    if (this.dispatchEvent(new Event("back", { bubbles: true, composed: true, cancelable: true }))) history.back()
  }
}
customElements.define("ui-screen", Screen)

/// `<ui-section heading="This week">`: a part of a screen under a small grey heading.
class Section extends Element {
  static styles = look(":host { display: grid; gap: 0.5rem; min-width: 0; } h2 { margin: 0; color: var(--muted-foreground); font-size: var(--text-sm); font-weight: 500; }")
  static properties = { heading: {} }

  render() {
    return html`${this.heading ? html`<h2>${this.heading}</h2>` : nothing}<slot></slot>`
  }
}
customElements.define("ui-section", Section)
