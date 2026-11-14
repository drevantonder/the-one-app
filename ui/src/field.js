import { html, nothing } from "lit"
import { Element, delegatesFocus, look } from "./element.js"
import css from "./field.css"

/// What the browser says of a field left empty, in the user's language.
export const missing = Object.assign(document.createElement("input"), { required: true }).validationMessage

/// `<ui-input label="Name" hint="As your card has it" name="name" required>` is what every form field is: a
/// `label` above its control, a grey `hint` below, and its problem in the hint's place: `error`, which the app
/// sets, or what's wrong with what was typed, once the form was sent. It takes part in a `<form>` as an `<input>`
/// does: `name`, `value`, `required`, `disabled`, in FormData, and in the form's reset. It tells the page `input`
/// as its value changes and `change` once it has. `autofocus` on the field a sheet opens for brings up a phone's
/// keyboard.
export class Field extends Element {
  static styles = look(css)
  static shadowRootOptions = delegatesFocus
  static formAssociated = true
  static properties = {
    label: {},
    hint: {},
    error: {},
    name: { reflect: true },
    value: {},
    placeholder: {},
    required: { type: Boolean, reflect: true },
    disabled: { type: Boolean, reflect: true },
    ariaLabel: { attribute: "aria-label" },
    problem: { state: true },
  }

  internals = this.attachInternals()

  constructor() {
    super()
    this.value = ""
    // The browser's own bubble isn't ours, and a phone's web view shows none: the problem shows on the field.
    this.addEventListener("invalid", (event) => {
      event.preventDefault()
      this.problem = this.internals.validationMessage
      if (this.internals.form.querySelector(":invalid:not(fieldset)") === this) this.focus()
    })
  }

  /// Which part the label is for: the one a press on the label goes to.
  get labelled() {
    return "control"
  }

  /// What the form is sent for this field.
  get formValue() {
    return this.value
  }

  /// What's wrong with the value, as `[flags, message]` for ElementInternals, or nothing. A field around a native
  /// control asks the control.
  get invalid() {
    const control = this.renderRoot.querySelector(".native")
    if (control) return control.validity.valid ? null : [control.validity, control.validationMessage]
    return this.required && !this.value ? [{ valueMissing: true }, missing] : null
  }

  render() {
    const message = this.error || this.problem
    return html`
      ${this.label ? html`<label id="label" for=${this.labelled}>${this.label}</label>` : nothing}
      ${this.control(message)}
      ${message ? html`<p id="message" class="error">${message}</p>` : this.hint ? html`<p id="message">${this.hint}</p>` : nothing}
    `
  }

  updated() {
    this.internals.setFormValue(this.formValue)
    const [flags, text] = this.invalid ?? [{}]
    this.internals.setValidity(flags, text, this.renderRoot.querySelector("#control") ?? undefined)
    // Once it's been called wrong, it says what's wrong as it's put right.
    if (this.problem) this.problem = text
  }

  /// A new value from the user.
  changed(value, done = true) {
    this.value = value
    this.emit("input")
    if (done) this.emit("change")
  }

  /// Enter in a one-line field sends the form, as it does in a form's own `<input>`. The key press ends here: the
  /// form may move focus (closing a sheet puts it back on what opened it), and the same press mustn't press that.
  enter(event) {
    if (event.key !== "Enter" || event.isComposing) return
    event.preventDefault()
    this.internals.form?.requestSubmit()
  }

  formResetCallback() {
    this.value = this.getAttribute("value") ?? ""
    this.problem = ""
  }

  formDisabledCallback(disabled) {
    this.disabled = disabled
  }
}
