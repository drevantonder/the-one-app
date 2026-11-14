import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { more } from "./element.js"
import { Field } from "./field.js"
import css from "./checkbox.css"

/// `<ui-radio-group label="Paid with" name="paid"><option value="card">My card</option>…</ui-radio-group>`: one of
/// a few, all in view, written as a `<select>` is. For two to five things; a select holds more. The arrow keys
/// move between them, as they do in the browser's own.
class RadioGroup extends Field {
  static styles = [Field.styles, more(css)]

  constructor() {
    super()
    this.watchChildren()
  }

  control(message) {
    const options = [...this.querySelectorAll("option")]
    // The label, and the keyboard, go to the one that's chosen, or the first.
    const first = options.find((option) => option.value === this.value) ?? options[0]
    return html`
      <div class="radios" role="radiogroup" aria-labelledby="label" aria-label=${ifDefined(this.ariaLabel ?? undefined)}
        aria-invalid=${message ? "true" : "false"} aria-describedby="message">
        ${options.map((option) => html`
          <label class="choice press">
            <input id=${option === first ? "control" : nothing} class="ring" type="radio" name="radio" .checked=${option.value === this.value}
              ?disabled=${this.disabled || option.disabled} aria-invalid=${message ? "true" : "false"}
              @input=${() => (this.value = option.value)} @change=${() => this.emit("change")}>
            <span>${option.label}</span>
          </label>
        `)}
      </div>
    `
  }
}
customElements.define("ui-radio-group", RadioGroup)
