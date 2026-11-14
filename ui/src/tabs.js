import { html } from "lit"
import * as tabs from "@zag-js/tabs"
import { Element, look } from "./element.js"
import { Machine, part } from "./zag.js"
import css from "./tabs.css"

/// `<ui-tabs><ui-tab value="all" label="All">…</ui-tab><ui-tab value="month" label="This month">…</ui-tab></ui-tabs>`:
/// a few views of one thing, one showing at a time. `value` is the one showing, the first unless set, and it
/// tells the page `change`.
class Tabs extends Element {
  static styles = look(css)
  static properties = { value: { reflect: true } }

  machine = new Machine(this, tabs, () => ({
    value: this.value,
    onValueChange: ({ value }) => {
      if (value === this.value) return
      this.value = value
      this.emit("change")
    },
  }))

  constructor() {
    super()
    this.watchChildren()
  }

  get tabs() {
    return [...this.children].filter((child) => child.localName === "ui-tab")
  }

  willUpdate() {
    this.value ??= this.tabs[0]?.getAttribute("value")
  }

  render() {
    const { api } = this.machine
    return html`
      <div class="tabs" ${part(api.getListProps())}>
        ${this.tabs.map((tab) => html`
          <button class="press ring" ${part(api.getTriggerProps({ value: tab.getAttribute("value") }))}>${tab.getAttribute("label")}</button>
        `)}
        <div class="indicator" ${part(api.getIndicatorProps())}></div>
      </div>
      <slot></slot>
    `
  }

  updated() {
    // Only what changes is set: setting it again would read as the app changing its tabs.
    for (const tab of this.tabs) tab.toggleAttribute("hidden", tab.getAttribute("value") !== this.value)
  }
}

/// One view of a `<ui-tabs>`: `label` is what its tab says, `value` what it's known by, and what's inside is what
/// shows while it's chosen.
class Tab extends Element {
  static styles = look(":host { display: grid; gap: var(--gap); }")
  static properties = { value: { reflect: true }, label: { reflect: true } }

  internals = Object.assign(this.attachInternals(), { role: "tabpanel" })

  render() {
    this.internals.ariaLabel = this.label
    return html`<slot></slot>`
  }
}
customElements.define("ui-tab", Tab)
customElements.define("ui-tabs", Tabs)
