import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { Field } from "./field.js"
import { icon } from "./icon.js"

/// `<ui-input label="Name" name="name">`: one line of text. `type` is an `<input>`'s (`email`, `tel`, `url`,
/// `password`, `search`), and so are `autocomplete`, `inputmode`, `enterkeyhint`, `maxlength`, `pattern` and
/// `readonly`, so the device shows the right keyboard and offers what it knows. Enter sends the form.
class Input extends Field {
  static properties = {
    type: {},
    autocomplete: {},
    inputmode: {},
    enterkeyhint: {},
    maxlength: {},
    pattern: {},
    readonly: { type: Boolean },
  }

  control(message) {
    return html`
      <input id="control" class="native box ring"
        type=${this.type ?? "text"} .value=${this.value} placeholder=${ifDefined(this.placeholder)}
        autocomplete=${ifDefined(this.autocomplete)} inputmode=${ifDefined(this.inputmode)} enterkeyhint=${ifDefined(this.enterkeyhint)}
        maxlength=${ifDefined(this.maxlength)} pattern=${ifDefined(this.pattern)}
        ?required=${this.required} ?disabled=${this.disabled} ?readonly=${this.readonly}
        aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-invalid=${message ? "true" : "false"} aria-describedby="message"
        @input=${(event) => (this.value = event.target.value)}
        @change=${() => this.emit("change")}
        @keydown=${this.enter}>
    `
  }
}
customElements.define("ui-input", Input)

/// `<ui-search placeholder="Search expenses" aria-label="Search expenses">`: what's typed to find something in a
/// list: a text field with a magnifier and, once there's text, a button that clears it. Escape clears it too. It
/// tells the page `input` as it's typed; the list it narrows is the app's.
class Search extends Input {
  constructor() {
    super()
    this.type = "search"
    this.enterkeyhint = "search"
  }

  control(message) {
    return html`
      <div class="search" @keydown=${this.escape}>
        ${glass}
        ${super.control(message)}
        ${this.value && !this.disabled ? html`<button class="press ring" type="button" aria-label="Clear" @click=${this.clear}>${x}</button>` : nothing}
      </div>
    `
  }

  /// Escape empties a field with text in it, and only then does what it does elsewhere (closes the sheet).
  escape(event) {
    if (event.key !== "Escape" || !this.value) return
    event.preventDefault()
    this.clear()
  }

  clear() {
    this.changed("")
    this.focus()
  }
}
customElements.define("ui-search", Search)

const glass = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("magnifying-glass")}</svg>`
const x = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("x")}</svg>`

/// `<ui-textarea label="Note" name="note">`: several lines of text, growing with what's typed. `rows` is how tall
/// it starts.
class Textarea extends Field {
  static properties = {
    rows: { type: Number },
    maxlength: {},
    readonly: { type: Boolean },
  }

  control(message) {
    return html`
      <textarea id="control" class="native box ring"
        rows=${this.rows ?? 3} .value=${this.value} placeholder=${ifDefined(this.placeholder)} maxlength=${ifDefined(this.maxlength)}
        ?required=${this.required} ?disabled=${this.disabled} ?readonly=${this.readonly}
        aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-invalid=${message ? "true" : "false"} aria-describedby="message"
        @input=${(event) => (this.value = event.target.value)}
        @change=${() => this.emit("change")}></textarea>
    `
  }
}
customElements.define("ui-textarea", Textarea)
