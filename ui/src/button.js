import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { Element, delegatesFocus, look } from "./element.js"
import css from "./button.css"

/// `<ui-button>Save</ui-button>`: something to press. `variant` says how much it matters: one `default` to a view,
/// then `secondary`, `outline` and `ghost`, and `destructive` for what can't be undone. `size="icon"` is square,
/// for an icon alone, named by `aria-label`. `type="submit"` sends the form it's in. `loading` shows it's working
/// and can't be pressed again.
class Button extends Element {
  static styles = look(css)
  static shadowRootOptions = delegatesFocus
  static formAssociated = true
  static properties = {
    variant: { reflect: true },
    size: { reflect: true },
    type: {},
    disabled: { type: Boolean, reflect: true },
    loading: { type: Boolean, reflect: true },
    ariaLabel: { attribute: "aria-label" },
  }

  internals = this.attachInternals()

  constructor() {
    super()
    this.variant = "default"
    this.type = "button"
    // A button that can't be pressed says nothing to the page, whatever inside it was pressed.
    this.addEventListener("click", (event) => (this.disabled || this.loading) && event.stopImmediatePropagation(), true)
  }

  render() {
    return html`
      <button class="press ring" ?disabled=${this.disabled || this.loading} aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-busy=${this.loading ? "true" : nothing} @click=${this.pressed}>
        ${this.loading ? html`<ui-spinner></ui-spinner>` : nothing}
        <slot></slot>
      </button>
    `
  }

  /// The button inside can't send the form itself: the form is outside this element.
  pressed() {
    if (this.type === "submit") this.internals.form?.requestSubmit()
    if (this.type === "reset") this.internals.form?.reset()
  }
}
customElements.define("ui-button", Button)
