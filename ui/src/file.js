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

/// An image drawn no bigger than `side` pixels either way. One this browser can't draw (an iPhone's HEIC, anywhere
/// else) throws.
async function drawn(file, side) {
  const image = await createImageBitmap(file)
  const scale = Math.min(1, side / Math.max(image.width, image.height))
  const canvas = Object.assign(document.createElement("canvas"), { width: Math.round(image.width * scale), height: Math.round(image.height * scale) })
  canvas.getContext("2d").drawImage(image, 0, 0, canvas.width, canvas.height)
  image.close()
  return canvas
}

/// A photo as a JPEG no bigger than `side` pixels either way. Anything that isn't a picture, or can't be drawn,
/// is as it was.
async function fitted(file, side) {
  if (!file.type.startsWith("image/")) return file
  try {
    const canvas = await drawn(file, side)
    const photo = await new Promise((made) => canvas.toBlob(made, "image/jpeg", 0.8))
    return new File([photo], file.name.replace(/\.[^.]*$/, "") + ".jpg", { type: photo.type, lastModified: file.lastModified })
  } catch {
    return file
  }
}

/// `<ui-file label="Receipt" name="receipt" accept="image/*" multiple>`: files the user picks: a photo taken now,
/// an image, anything else. Its button opens the device's own chooser, which offers what the device has (under a
/// finger, the camera and the photo library); `accept` and `capture` are an `<input type="file">`'s. What's chosen
/// shows above the button, each with a button that removes it. With `multiple` choosing again adds; without it one
/// file is all there is, and it's removed to choose another. `files` is what's chosen, as an array of Files, and
/// a form is sent them under `name`. `fit="2000"` makes each photo chosen a JPEG of at most that many pixels
/// either way first: a phone's photo is many times what reading a receipt needs.
class FileField extends Field {
  static styles = [Field.styles, more(css)]
  static properties = {
    accept: {},
    capture: {},
    multiple: { type: Boolean },
    fit: { type: Number },
    files: { attribute: false },
  }

  /// Each image's small picture, once it's drawn, kept while the file is chosen. It's a `data:` address: an MCP
  /// App's page may show no `blob:` one.
  previews = new Map()

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
    for (const file of this.previews.keys()) if (!this.files.includes(file)) this.previews.delete(file)
    for (const file of this.files) {
      if (this.previews.has(file) || !file.type.startsWith("image/")) continue
      this.previews.set(file, undefined)
      // One that can't be drawn keeps the look of any other file.
      drawn(file, 96).then((canvas) => {
        if (!this.previews.has(file)) return
        this.previews.set(file, canvas.toDataURL("image/jpeg", 0.8))
        this.requestUpdate()
      }, () => {})
    }
  }

  control(message) {
    return html`
      ${this.files.length ? html`
        <ul>
          ${this.files.map((file) => html`
            <li>
              ${this.previews.get(file) ? html`<img src=${this.previews.get(file)} alt="">` : html`<span class="thumb">${paper}</span>`}
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

  async chosen(event) {
    let chosen = [...event.target.files]
    // The device's control only chooses: emptied, it can be given the same file again.
    event.target.value = ""
    if (this.fit) chosen = await Promise.all(chosen.map((file) => fitted(file, this.fit)))
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
