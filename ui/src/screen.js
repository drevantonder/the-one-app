import { html, nothing } from "lit"
import { Element, look, reduced } from "./element.js"
import { icon } from "./icon.js"
import css from "./screen.css"

/// Changes which screen shows, as a move: `change` puts the new one in place, and the browser slides from what
/// was there to what is (its View Transitions), back the way it came for `back`. `screen` is what moves: a part of
/// the page, or all of it. Where the browser can't, or the user asked for no motion, it's a plain swap.
export function move(screen, change, { back = false } = {}) {
  if (!document.startViewTransition || reduced.matches) return void change()
  const page = document.documentElement
  screen.style.viewTransitionName = "ui-screen"
  page.toggleAttribute("data-ui-back", back)
  document.startViewTransition(change).finished.finally(() => {
    screen.style.viewTransitionName = ""
    page.removeAttribute("data-ui-back")
  })
}

// Between one page and the next the browser moves by itself (page.css). Going back, it moves the other way.
addEventListener("pagereveal", ({ viewTransition }) => {
  const { entry, from, navigationType } = globalThis.navigation?.activation ?? {}
  if (!viewTransition || navigationType !== "traverse" || entry.index > from.index) return
  document.documentElement.setAttribute("data-ui-back", "")
  viewTransition.finished.finally(() => document.documentElement.removeAttribute("data-ui-back"))
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
