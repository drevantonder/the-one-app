import { html } from "lit"
import { Element, look } from "./element.js"
import css from "./spinner.css"

/// `<ui-spinner>`: something's under way. The size of the text around it.
class Spinner extends Element {
  static styles = look(css)

  connectedCallback() {
    super.connectedCallback()
    this.role = "status"
    this.ariaLabel ??= "Loading"
  }

  render() {
    return html`<svg viewBox="0 0 24 24" aria-hidden="true"><circle cx="12" cy="12" r="9" /></svg>`
  }
}
customElements.define("ui-spinner", Spinner)
