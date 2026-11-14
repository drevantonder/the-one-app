import { html } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import * as numberInput from "@zag-js/number-input"
import { Field } from "./field.js"
import { Machine, part } from "./zag.js"

const ids = { input: "control", label: "label" }

/// `<ui-number label="Amount" name="amount" min="0">`: a number, typed on the device's number keyboard, as the
/// user's language writes numbers. The arrow keys step it by `step`, and it's kept within `min` and `max`.
/// `currency="AUD"` shows it as money; `decimals` is how many places it's shown to. `value` is the plain number
/// as text ("8.5"), and `valueAsNumber` the number.
class NumberField extends Field {
  static properties = {
    min: { type: Number },
    max: { type: Number },
    step: { type: Number },
    currency: {},
    decimals: { type: Number },
  }

  machine = new Machine(this, numberInput, () => ({
    ids,
    defaultValue: this.value,
    min: this.min,
    max: this.max,
    step: this.step,
    disabled: this.disabled,
    required: this.required,
    invalid: !!(this.error || this.problem),
    formatOptions: this.format,
    locale: navigator.language,
    onValueChange: ({ valueAsNumber }) => this.typed(valueAsNumber),
    onValueCommit: () => this.emit("change"),
  }))

  get valueAsNumber() {
    return this.value === "" ? NaN : Number(this.value)
  }

  willUpdate(changed) {
    if (changed.has("currency") || changed.has("decimals")) {
      this.format = {
        ...(this.currency && { style: "currency", currency: this.currency }),
        ...(this.decimals != null && { minimumFractionDigits: this.decimals, maximumFractionDigits: this.decimals }),
      }
    }
    // The machine holds what's in the box, from the value it's made with. A value set from outside goes to it;
    // one it told us is its own, and sending that back would rewrite the box under the user's typing.
    this.held ??= this.value
    if (this.value === this.held) return
    this.held = this.value
    if (this.value === "") this.machine.api.clearValue()
    else this.machine.api.setValue(Number(this.value))
  }

  typed(number) {
    if (text(number) === this.value) return
    this.held = this.value = text(number)
    this.emit("input")
  }

  control() {
    // Zag gives the box its text as a starting value and from then on writes the box itself. Its plain-JavaScript
    // wrapper would write it on every draw, over what's being typed, so it's given only as where the box starts.
    const { value, ...input } = this.machine.api.getInputProps()
    this.starts ??= value
    return html`
      <input class="box ring" ${part(input)} value=${this.starts}
        placeholder=${ifDefined(this.placeholder)} aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-describedby="message"
        @keydown=${this.enter}>
    `
  }
}
customElements.define("ui-number", NumberField)

const text = (number) => (Number.isNaN(number) ? "" : String(number))
