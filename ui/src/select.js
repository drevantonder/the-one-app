import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import * as select from "@zag-js/select"
import { touch } from "./element.js"
import { Field } from "./field.js"
import { icon } from "./icon.js"
import { Machine, over, part } from "./zag.js"

const ids = { trigger: "trigger", label: "label" }
const positioning = { strategy: "fixed", placement: "bottom-start", sameWidth: true, gutter: 4 }

/// `<ui-select label="Category" name="category"><option value="meals">Meals</option>…</ui-select>`: one of a few,
/// written as a `<select>` is. With no `placeholder` the first option is chosen, as a `<select>` does; an
/// `<option value="">` is how one is cleared. Under a finger it opens the device's own picker; with a mouse, ours.
class Select extends Field {
  selected = []
  collection = select.collection({ items: [] })
  listed = "[]"

  machine = new Machine(this, select, () => ({
    ids,
    positioning,
    collection: this.collection,
    value: this.selected,
    disabled: this.disabled,
    invalid: !!(this.error || this.problem),
    onValueChange: ({ value: [value = ""] }) => value !== this.value && this.changed(value),
  }))

  constructor() {
    super()
    this.watchChildren()
  }

  get labelled() {
    return touch.matches ? "control" : "trigger"
  }

  willUpdate(changed) {
    const options = [...this.querySelectorAll("option")].map(({ value, label, disabled }) => ({ value, label, disabled }))
    const listed = JSON.stringify(options)
    if (listed !== this.listed) {
      this.listed = listed
      this.collection = select.collection({ items: options, isItemDisabled: (item) => item.disabled })
    }
    if (changed.has("value")) this.selected = this.value ? [this.value] : []
  }

  control(message) {
    const options = this.collection.items
    const chosen = options.find((option) => option.value === this.value)
    const face = html`<span class=${chosen ? "" : "placeholder"}>${chosen?.label ?? this.placeholder}</span>${caret}`
    const { api } = this.machine
    return html`
      <div class="picker">
        ${touch.matches
          ? html`<div class="box" aria-hidden="true" ?data-disabled=${this.disabled} ?data-invalid=${!!message}>${face}</div>`
          : html`
            <button class="box ring" ${part(api.getTriggerProps())} aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-describedby="message">${face}</button>
            <div popover="manual" ${part(api.getPositionerProps())} ${over(api.open)}>
              <div class="popup list" ${part(api.getContentProps())}>
                ${options.map((item) => html`
                  <div class="item" ${part(api.getItemProps({ item }))}>
                    <span ${part(api.getItemTextProps({ item }))}>${item.label}</span>
                    <span ${part(api.getItemIndicatorProps({ item }))}>${tick}</span>
                  </div>
                `)}
              </div>
            </div>
          `}
        <select id="control" class="native" ?hidden=${!touch.matches} ?required=${this.required} ?disabled=${this.disabled}
          aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-invalid=${message ? "true" : "false"} aria-describedby="message"
          @input=${(event) => (this.value = event.target.value)} @change=${() => this.emit("change")}>
          ${this.placeholder && !options.some((option) => option.value === "") ? html`<option value="" disabled hidden .selected=${!this.value}>${this.placeholder}</option>` : nothing}
          ${options.map((option) => html`<option value=${option.value} ?disabled=${option.disabled} .selected=${option.value === this.value}>${option.label}</option>`)}
        </select>
      </div>
    `
  }

  updated() {
    // Nothing chosen and no placeholder: the first option is, as the device's own control has it.
    if (!this.value) this.value = this.renderRoot.querySelector("#control").value
    super.updated()
  }
}
customElements.define("ui-select", Select)

const caret = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("caret-up-down")}</svg>`
const tick = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("check")}</svg>`
