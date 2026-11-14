import { html, nothing } from "lit"
import { Element, look } from "./element.js"
import { icon } from "./icon.js"
import css from "./screen.css"

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
