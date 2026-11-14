import { html, nothing } from "lit"
import { Element, delegatesFocus, look, more, said } from "./element.js"
import { icon } from "./icon.js"
import { behaviour, over, part } from "./zag.js"
import button from "./button.css"
import css from "./menu.css"

// Under an icon the menu opens back along the line from it, where there's room: an icon sits at the end of things.
// Under a button that says what it is, it opens along the line, as a select does.
const fromIcon = { strategy: "fixed", placement: "bottom-end", gutter: 4 }
const fromWords = { strategy: "fixed", placement: "bottom-start", gutter: 4 }

/// `<ui-menu label="More"><ui-menu-item>Rename</ui-menu-item><hr><ui-menu-item destructive>Delete</ui-menu-item></ui-menu>`:
/// a button that opens a few things to do. The button is an icon (`icon`, three dots unless said), named by
/// `label`. With `variant`, a `<ui-button>`'s, it's a button of that weight that says `label`, after its `icon`
/// if it has one; `size="icon"` keeps that one to its icon. Choosing an item clicks it: listen for `click` on the
/// item. An `<hr>` is a line between groups.
class Menu extends Element {
  static styles = [...look(button), more(css)]
  static shadowRootOptions = delegatesFocus
  static properties = { label: {}, icon: {}, variant: { reflect: true }, size: { reflect: true } }

  constructor() {
    super()
    this.watchChildren()
    behaviour(this, "menu", () => ({
      positioning: this.says ? fromWords : fromIcon,
      onSelect: ({ value }) => this.children[value].click(),
    }))
  }

  /// Whether its button says its label, not only shows an icon.
  get says() {
    return !!this.variant && this.size !== "icon"
  }

  render() {
    const api = this.machine?.api
    const { says } = this
    const shown = says ? this.icon : (this.icon ?? "dots-three")
    return html`
      <button class="trigger press ring" data-variant=${this.variant ?? "ghost"} data-size=${says ? nothing : "icon"}
        ${part(api?.getTriggerProps())} aria-label=${says ? nothing : (this.label ?? said.more)}>
        ${shown ? html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon(shown)}</svg>` : nothing}
        ${says ? this.label : nothing}
      </button>
      ${api ? this.popup(api) : nothing}
    `
  }

  popup(api) {
    return html`
      <div popover="manual" ${part(api.getPositionerProps())} ${over(api.open, () => api.setOpen(false))}>
        <div class="popup" ${part(api.getContentProps())}>
          ${[...this.children].map((item, i) => item.localName === "hr"
            ? html`<hr ${part(api.getSeparatorProps())}>`
            : html`
              <div class="item ${item.hasAttribute("destructive") ? "destructive" : ""}" ${part(api.getItemProps({ value: String(i), disabled: item.hasAttribute("disabled") }))}>
                ${item.hasAttribute("icon") ? html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon(item.getAttribute("icon"))}</svg>` : nothing}
                ${item.textContent.trim()}
              </div>
            `)}
        </div>
      </div>
    `
  }
}

/// One thing to do in a `<ui-menu>`: its text, an `icon` before it, `destructive` if it can't be undone.
class MenuItem extends Element {
  static properties = {
    icon: { reflect: true },
    destructive: { type: Boolean, reflect: true },
    disabled: { type: Boolean, reflect: true },
  }
}
customElements.define("ui-menu-item", MenuItem)
customElements.define("ui-menu", Menu)
