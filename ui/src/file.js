import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { more, said } from "./element.js"
import { Field, missing } from "./field.js"
import { icon } from "./icon.js"
import css from "./file.css"

const units = ["byte", "kilobyte", "megabyte", "gigabyte"]

/// How big a file is, as the user's language writes it: "248 kB".
function size(bytes) {
  const unit = Math.min(units.length - 1, Math.floor(Math.log10(Math.max(bytes, 1)) / 3))
  return new Intl.NumberFormat(navigator.language, { style: "unit", unit: units[unit], maximumFractionDigits: unit > 1 ? 1 : 0 }).format(bytes / 1000 ** unit)
}

/// `<ui-file label="Receipt" name="receipt" accept="image/*" multiple>`: files the user picks: a photo taken now,
/// an image, anything else. Its button opens the device's own chooser, which offers what the device has (under a
/// finger, the camera and the photo library); `accept` and `capture` are an `<input type="file">`'s. What's chosen
/// shows above the button, each with a button that removes it. With `multiple` choosing again adds; without it one
/// file is all there is, and it's removed to choose another. `files` is what's chosen, as an array of Files, and
/// a form is sent them under `name`.
class FileField extends Field {
  static styles = [Field.styles, more(css)]
  static properties = {
    accept: {},
    capture: {},
    multiple: { type: Boolean },
    files: { attribute: false },
  }

  /// The address of each image's picture, kept while the file is chosen.
  previews = new Map()
  /// Images this browser can't draw (an iPhone's HEIC, anywhere else).
  undrawn = new WeakSet()

  constructor() {
    super()
    this.files = []
  }

  /// As an `<input type="file">` has it: the first file's name, and only ever set to nothing.
  get value() {
    return this.files?.[0]?.name ?? ""
  }

  set value(text) {
    if (text === "" && this.files?.length) this.files = []
  }

  get formValue() {
    const sent = new FormData()
    for (const file of this.files) sent.append(this.name, file)
    return sent
  }

  get invalid() {
    return this.required && !this.files.length ? [{ valueMissing: true }, missing] : null
  }

  willUpdate() {
    for (const [file, address] of this.previews) {
      if (this.files.includes(file)) continue
      URL.revokeObjectURL(address)
      this.previews.delete(file)
    }
  }

  disconnectedCallback() {
    super.disconnectedCallback()
    for (const address of this.previews.values()) URL.revokeObjectURL(address)
    this.previews.clear()
  }

  preview(file) {
    if (!this.previews.has(file)) this.previews.set(file, URL.createObjectURL(file))
    return this.previews.get(file)
  }

  control(message) {
    return html`
      ${this.files.length ? html`
        <ul>
          ${this.files.map((file) => html`
            <li>
              ${file.type.startsWith("image/") && !this.undrawn.has(file)
                ? html`<img src=${this.preview(file)} alt="" @error=${() => (this.undrawn.add(file), this.requestUpdate())}>`
                : html`<span class="thumb">${paper}</span>`}
              <span class="words"><span>${file.name}</span><small>${size(file.size)}</small></span>
              <button class="press ring" type="button" aria-label="${said.remove} ${file.name}" ?disabled=${this.disabled} @click=${() => this.remove(file)}>${x}</button>
            </li>
          `)}
        </ul>
      ` : nothing}
      <div class="picker" ?hidden=${!this.multiple && this.files.length > 0}>
        <div class="box" aria-hidden="true" ?data-disabled=${this.disabled} ?data-invalid=${!!message}>${clip}<span>${this.placeholder ?? said.file}</span></div>
        <input id="control" class="native" type="file" accept=${ifDefined(this.accept)} capture=${ifDefined(this.capture)}
          ?multiple=${this.multiple} ?disabled=${this.disabled}
          aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-invalid=${message ? "true" : "false"} aria-describedby="message"
          @input=${(event) => event.stopPropagation()} @change=${this.chosen}>
      </div>
    `
  }

  chosen(event) {
    const chosen = [...event.target.files]
    // The device's control only chooses: emptied, it can be given the same file again.
    event.target.value = ""
    this.set(this.multiple ? [...this.files, ...chosen] : chosen.slice(0, 1))
  }

  async remove(file) {
    this.set(this.files.filter((kept) => kept !== file))
    // Its button went with it: the keyboard carries on from what's left.
    await this.updateComplete
    this.focus()
  }

  set(files) {
    this.files = files
    this.emit("input")
    this.emit("change")
  }

  formResetCallback() {
    super.formResetCallback()
    this.files = []
  }
}
customElements.define("ui-file", FileField)

const clip = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("paperclip")}</svg>`
const paper = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("file")}</svg>`
const x = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("x")}</svg>`
