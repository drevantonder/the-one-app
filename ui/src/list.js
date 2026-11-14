import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { Element, delegatesFocus, look } from "./element.js"
import { icon } from "./icon.js"
import css from "./list.css"

/// `<ui-list>` of `<ui-row>`s: things of one kind, one under another, in one group that stops at a readable width
/// however wide the room is. With no rows it shows nothing.
class List extends Element {
  static styles = look(":host { display: block; max-width: var(--readable); border: 1px solid var(--border); border-radius: var(--radius-xl); background: var(--card); color: var(--card-foreground); overflow: hidden; } :host(:empty) { display: none; } ::slotted(:not(:first-child)) { border-top: 1px solid var(--border); }")

  internals = Object.assign(this.attachInternals(), { role: "list" })

  render() {
    return html`<slot></slot>`
  }
}
customElements.define("ui-list", List)

/// `<ui-row heading="Coffee with Sam" text="Meals · 12 Nov">`: one thing in a list: its `heading`, one grey line
/// of `text`, what it starts with (`slot="start"`, an icon) and what it ends with (`slot="end"`: an amount, a
/// switch). With `href` it's a link to what it opens, with a chevron; with `button` the whole row is pressed
/// (listen for `click`) and opens in place.
class Row extends Element {
  static styles = look(css)
  static shadowRootOptions = delegatesFocus
  static properties = { heading: {}, text: {}, href: {}, button: { type: Boolean } }

  internals = Object.assign(this.attachInternals(), { role: "listitem" })

  render() {
    const inside = html`
      <slot name="start"></slot>
      <span class="words">
        <span class="heading">${this.heading}</span>
        ${this.text ? html`<span class="text">${this.text}</span>` : nothing}
      </span>
      <slot name="end"></slot>
      ${this.href ? html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("caret-right")}</svg>` : nothing}
    `
    if (this.href) return html`<a class="row press" href=${this.href}>${inside}</a>`
    if (this.button) return html`<button class="row press">${inside}</button>`
    return html`<div class="row">${inside}</div>`
  }
}
customElements.define("ui-row", Row)
