import { html } from "lit"
import { Element, look, said } from "./element.js"
import css from "./spinner.css"

/// `<ui-spinner>`: something's under way. The size of the text around it.
class Spinner extends Element {
  static styles = look(css)

  internals = Object.assign(this.attachInternals(), { role: "status" })

  render() {
    this.internals.ariaLabel = said.loading
    return html`<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /></svg>`
  }
}
customElements.define("ui-spinner", Spinner)
