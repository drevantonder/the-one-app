import { html } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { Field } from "./field.js"

/// `<ui-input label="Name" name="name">`: one line of text. `type` is an `<input>`'s (`email`, `tel`, `url`,
/// `password`, `search`), and so are `autocomplete`, `inputmode`, `maxlength`, `pattern` and `readonly`, so the
/// device shows the right keyboard and offers what it knows. Enter sends the form.
class Input extends Field {
  static properties = {
    type: {},
    autocomplete: {},
    inputmode: {},
    maxlength: {},
    pattern: {},
    readonly: { type: Boolean },
  }

  control(message) {
    return html`
      <input id="control" class="native box ring"
        type=${this.type ?? "text"} .value=${this.value} placeholder=${ifDefined(this.placeholder)}
        autocomplete=${ifDefined(this.autocomplete)} inputmode=${ifDefined(this.inputmode)}
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
