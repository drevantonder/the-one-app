import { html, nothing } from "lit"
import { Element, look, said } from "./element.js"
import { icon } from "./icon.js"
import css from "./states.css"

/// `<ui-empty>No expenses yet.</ui-empty>`: an empty screen or list: one grey line, and a button
/// (`slot="action"`) only when there's an obvious first step.
class Empty extends Element {
  static styles = look(css)

  render() {
    return html`<p class="empty"><slot></slot></p><slot name="action"></slot>`
  }
}
customElements.define("ui-empty", Empty)

/// `<ui-skeleton rows="3">`: grey shapes the shape of the list that's coming, once it's taken longer than a
/// blink, so a quick load shows nothing in between.
class Skeleton extends Element {
  static styles = look(css)
  static properties = { rows: { type: Number } }

  internals = Object.assign(this.attachInternals(), { role: "status", ariaBusy: "true" })

  render() {
    this.internals.ariaLabel = said.loading
    return html`<div class="skeleton">${Array.from({ length: this.rows ?? 3 }, () => html`<div><i></i><i></i></div>`)}</div>`
  }
}
customElements.define("ui-skeleton", Skeleton)

/// `<ui-problem heading="Couldn't load expenses" text="The server didn't answer.">`: a problem, shown on the thing
/// it's about: a symbol, a short `heading`, one sentence of `text`, and the action that fixes it
/// (`slot="action"`). Red, for what needs the user now; `warn` for what can wait.
class Problem extends Element {
  static styles = look(css)
  static properties = { heading: {}, text: {}, warn: { type: Boolean, reflect: true } }

  internals = Object.assign(this.attachInternals(), { role: "alert" })

  render() {
    return html`
      <div class="problem">
        <svg viewBox="0 0 256 256" aria-hidden="true">${icon("warning-circle")}</svg>
        <strong>${this.heading}</strong>
        ${this.text ? html`<p>${this.text}</p>` : nothing}
        <slot name="action"></slot>
      </div>
    `
  }
}
customElements.define("ui-problem", Problem)
