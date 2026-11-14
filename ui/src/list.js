import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { Element, delegatesFocus, look, pace, reduced } from "./element.js"
import { icon } from "./icon.js"
import css from "./list.css"

const listCss = `
  :host { display: block; max-width: var(--readable); border: 1px solid var(--border); border-radius: var(--radius-xl); background: var(--card); color: var(--card-foreground); overflow: hidden; --row-line: 1px solid var(--border); }
  :host(:empty) { display: none; }
  /* Every row draws the line above it, and the first row's is out of sight above the list. */
  slot { display: block; margin-top: -1px; }
`

/// What a row says. An app often draws its whole list again with new rows: one that says the same is the same row.
const says = (row) => `${row.heading}\n${row.text}\n${row.textContent}`

/// `<ui-list>` of `<ui-row>`s: things of one kind, one under another, in one group that stops at a readable width
/// however wide the room is. With no rows it shows nothing. A row added to a list that has some opens up where
/// it goes, and one taken out closes up, the rest making room or closing the gap. A list that changes in any
/// other way (filled, a row edited, another list's rows) just shows what it's given.
class List extends Element {
  static styles = look(listCss)

  internals = Object.assign(this.attachInternals(), { role: "list" })
  /// The rows it had when it last looked, and those on their way out: each still in the list, with what's
  /// closing it up.
  rows = []
  going = new Map()
  seen = new MutationObserver((changes) => this.changed(changes))

  connectedCallback() {
    super.connectedCallback()
    this.rows = [...this.children]
    this.seen.observe(this, { childList: true })
  }

  disconnectedCallback() {
    super.disconnectedCallback()
    this.seen.disconnect()
  }

  render() {
    return html`<slot></slot>`
  }

  changed(changes) {
    // A row the app put back while it was on its way out stays.
    for (const row of changes.flatMap((change) => [...change.addedNodes])) {
      this.going.get(row)?.cancel()
      if (this.going.delete(row)) row.inert = false
    }
    const before = this.rows
    const after = (this.rows = [...this.children].filter((row) => !this.going.has(row)))
    if (reduced.matches || !before.length) return
    const left = Map.groupBy(after, says)
    const stayed = new Map(before.map((row) => [row, left.get(says(row))?.shift()]))
    const gone = before.filter((row) => !stayed.get(row))
    const come = [...left.values()].flat()
    // Some gone and some new is a row that changed, or another list's rows: a plain swap.
    if (gone.length && come.length) return
    for (const row of come) this.arrive(row)
    for (const row of gone) this.leave(row, before.slice(before.indexOf(row) + 1).map((later) => stayed.get(later)).find(Boolean))
    // What this did to its own rows isn't news to it.
    this.seen.takeRecords()
  }

  /// A new row opens up from nothing, once it's drawn and its height is known.
  async arrive(row) {
    await row.updateComplete
    row.animate({ height: ["0px", `${row.offsetHeight}px`], opacity: [0, 1] }, pace(this, "settle"))
  }

  /// A row that was taken out goes back where it was, `before` the row that followed it, for as long as it takes
  /// to close up.
  leave(row, before = null) {
    const height = `${row.offsetHeight || this.rows[0]?.offsetHeight || 0}px`
    this.insertBefore(row, before)
    row.inert = true
    row.animate({ opacity: [1, 0] }, { ...pace(this, "quick"), fill: "forwards" })
    const closing = row.animate({ height: [height, "0px"] }, pace(this, "settle"))
    this.going.set(row, closing)
    closing.finished.then(() => {
      this.going.delete(row)
      row.inert = false
      row.remove()
      this.seen.takeRecords()
    }, () => {})
  }
}
customElements.define("ui-list", List)

/// `<ui-row heading="Coffee with Sam" text="Meals · 12 Nov">`: one thing in a list: its `heading`, one grey line
/// of `text`, what it starts with (`slot="start"`, an icon) and what it ends with (`slot="end"`: an amount, a
/// switch). With `href` it's a link to what it opens, with a chevron; with `button` the whole row is pressed
/// (listen for `click`) and opens in place. `current` marks the one that's open beside the list, in a split.
class Row extends Element {
  static styles = look(css)
  static shadowRootOptions = delegatesFocus
  static properties = { heading: {}, text: {}, href: {}, button: { type: Boolean }, current: { type: Boolean, reflect: true } }

  internals = Object.assign(this.attachInternals(), { role: "listitem" })

  render() {
    const inside = html`
      <slot name="start"></slot>
      <span class="words">
        <span class="heading">${this.heading}</span>
        ${this.text ? html`<span class="text">${this.text}</span>` : nothing}
      </span>
      <slot name="end"></slot>
      ${this.href ? html`<svg class="along" viewBox="0 0 256 256" aria-hidden="true">${icon("caret-right")}</svg>` : nothing}
    `
    if (this.href) return html`<a class="row press" href=${this.href} aria-current=${this.current ? "true" : nothing}>${inside}</a>`
    if (this.button) return html`<button class="row press" aria-current=${this.current ? "true" : nothing}>${inside}</button>`
    return html`<div class="row">${inside}</div>`
  }
}
customElements.define("ui-row", Row)
