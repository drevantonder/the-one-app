import { html, nothing } from "lit"
import { unsafeSVG } from "lit/directives/unsafe-svg.js"
import { Element, look } from "./element.js"
import css from "./icon.css"
import calendarBlank from "@phosphor-icons/core/assets/regular/calendar-blank.svg"
import caretLeft from "@phosphor-icons/core/assets/regular/caret-left.svg"
import caretRight from "@phosphor-icons/core/assets/regular/caret-right.svg"
import caretUpDown from "@phosphor-icons/core/assets/regular/caret-up-down.svg"
import check from "@phosphor-icons/core/assets/regular/check.svg"
import dotsThree from "@phosphor-icons/core/assets/regular/dots-three.svg"
import magnifyingGlass from "@phosphor-icons/core/assets/regular/magnifying-glass.svg"
import pencilSimple from "@phosphor-icons/core/assets/regular/pencil-simple.svg"
import plus from "@phosphor-icons/core/assets/regular/plus.svg"
import trash from "@phosphor-icons/core/assets/regular/trash.svg"
import warningCircle from "@phosphor-icons/core/assets/fill/warning-circle-fill.svg"
import x from "@phosphor-icons/core/assets/regular/x.svg"

// The Phosphor icons the kit draws with, by Phosphor's own names. An app that wants another puts Phosphor's SVG
// where the icon goes.
const icons = {
  "calendar-blank": calendarBlank,
  "caret-left": caretLeft,
  "caret-right": caretRight,
  "caret-up-down": caretUpDown,
  check,
  "dots-three": dotsThree,
  "magnifying-glass": magnifyingGlass,
  "pencil-simple": pencilSimple,
  plus,
  trash,
  "warning-circle": warningCircle,
  x,
}

/// An icon, for a template: `${icon("check")}`.
export const icon = (name) => {
  if (!icons[name]) throw new Error(`No icon named "${name}". There are: ${Object.keys(icons).join(", ")}`)
  return unsafeSVG(icons[name].replace(/^<svg[^>]*>|<\/svg>$/g, ""))
}

/// `<ui-icon name="plus">`: one of the kit's icons, the size of the text around it and its colour.
class Icon extends Element {
  static styles = look(css)
  static properties = { name: {} }

  render() {
    return this.name ? html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon(this.name)}</svg>` : nothing
  }
}
customElements.define("ui-icon", Icon)
