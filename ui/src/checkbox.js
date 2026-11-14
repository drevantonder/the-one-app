import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { more } from "./element.js"
import { Field } from "./field.js"
import css from "./checkbox.css"

/// `<ui-checkbox label="I have the receipt" name="receipt">`: one thing that's so or not, ticked. `checked` is
/// whether it is. In a form it's sent as `value` ("on" unless set) when checked, and not at all otherwise.
class Checkbox extends Field {
  static styles = [Field.styles, more(css)]
  static properties = { checked: { type: Boolean } }

  get formValue() {
    return this.checked ? this.value || "on" : null
  }

  render() {
    const message = this.error || this.problem
    const set = (event) => (this.checked = event.target.checked)
    return html`
      <label class="choice press">
        <input id="control" class="native ring" type="checkbox" role=${ifDefined(this.constructor.role)} .checked=${!!this.checked}
          ?required=${this.required} ?disabled=${this.disabled}
          aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-invalid=${message ? "true" : "false"} aria-describedby="message"
          @input=${set} @change=${(event) => (set(event), this.emit("change"))}>
        ${this.label ? html`<span>${this.label}${this.hint ? html`<small>${this.hint}</small>` : nothing}</span>` : nothing}
      </label>
      ${message ? html`<p id="message" class="error">${message}</p>` : nothing}
    `
  }

  formResetCallback() {
    super.formResetCallback()
    this.checked = this.hasAttribute("checked")
  }
}
customElements.define("ui-checkbox", Checkbox)

/// `<ui-switch label="Reimbursable" name="reimbursable">`: something that's on or off, and takes effect at once.
/// A checkbox in every other way.
class Switch extends Checkbox {
  static role = "switch"
}
customElements.define("ui-switch", Switch)
