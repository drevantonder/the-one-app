import { html, nothing } from "lit"
import { ifDefined } from "lit/directives/if-defined.js"
import { more, said, touch } from "./element.js"
import { Field } from "./field.js"
import { icon } from "./icon.js"
import { behaviour, over, part } from "./zag.js"
import css from "./date-picker.css"

const ids = { trigger: "trigger" }
const positioning = { strategy: "fixed", placement: "bottom-start", gutter: 4 }
const shown = new Intl.DateTimeFormat(navigator.language, { dateStyle: "medium", timeZone: "UTC" })

/// `<ui-date-picker label="Date" name="date">`: a day. `value`, `min` and `max` are written as an
/// `<input type="date">` writes them (2026-11-14), and it's shown as the user's language writes dates. Under a
/// finger it opens the device's own picker; with a mouse, our calendar.
class DatePicker extends Field {
  static styles = [Field.styles, more(css)]
  static properties = { min: {}, max: {} }

  /// Our own calendar, once Zag is here to open it, and only with a mouse.
  get ours() {
    return touch.matches ? undefined : this.machine?.api
  }

  get labelled() {
    return this.ours ? "trigger" : "control"
  }

  willUpdate() {
    if (!touch.matches) this.asked ??= behaviour(this, "datePicker", (zag) => ({
      ids,
      positioning,
      locale: navigator.language,
      ...this.days(zag),
      disabled: this.disabled,
      fixedWeeks: true,
      onValueChange: ({ value: [day] }) => (day?.toString() ?? "") !== this.value && this.changed(day?.toString() ?? ""),
    }))
  }

  /// The day, and the first and last it can be, as Zag holds days: made again only when they change.
  days(zag) {
    const said = `${this.value} ${this.min} ${this.max}`
    if (this.held?.said !== said) this.held = { said, value: this.value ? [zag.parse(this.value)] : [], min: this.min ? zag.parse(this.min) : undefined, max: this.max ? zag.parse(this.max) : undefined }
    const { value, min, max } = this.held
    return { value, min, max }
  }

  control(message) {
    const face = html`
      <span class=${this.value ? "" : "placeholder"}>${this.value ? shown.format(new Date(this.value)) : this.placeholder}</span>
      <svg viewBox="0 0 256 256" aria-hidden="true">${icon("calendar-blank")}</svg>
    `
    const api = this.ours
    return html`
      <div class="picker" ${part(api?.getControlProps())}>
        ${!api
          ? html`<div class="box" aria-hidden="true" ?data-disabled=${this.disabled} ?data-invalid=${!!message}>${face}</div>`
          : html`
            <button class="box ring" ${part(api.getTriggerProps())} aria-labelledby="label trigger" ?data-invalid=${!!message}
              aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-describedby="message">${face}</button>
            <div popover="manual" ${part(api.getPositionerProps())} ${over(api.open)}>${this.calendar(api)}</div>
          `}
        <input id="control" class="native" type="date" .value=${this.value} min=${ifDefined(this.min)} max=${ifDefined(this.max)}
          ?hidden=${!!api} ?required=${this.required} ?disabled=${this.disabled}
          aria-label=${ifDefined(this.ariaLabel ?? undefined)} aria-invalid=${message ? "true" : "false"} aria-describedby="message"
          @input=${(event) => (this.value = event.target.value)} @change=${() => this.emit("change")}>
      </div>
    `
  }

  /// The calendar: a month of days, and from its heading the months of a year, and from theirs the years.
  calendar(api) {
    const decade = api.getDecade()
    const bar = (view, text) => html`
      <div class="bar" ${part(api.getViewControlProps({ view }))}>
        <button class="press ring" ${part(api.getPrevTriggerProps({ view }))}>${previous}</button>
        <button class="press ring" ${part(api.getViewTriggerProps({ view }))}>${text}</button>
        <button class="press ring" ${part(api.getNextTriggerProps({ view }))}>${next}</button>
      </div>
    `
    const grid = (view, rows, cell, trigger) => html`
      <table ${part(api.getTableProps({ view, columns: 4 }))}>
        <tbody ${part(api.getTableBodyProps({ view }))}>
          ${rows.map((row) => html`
            <tr ${part(api.getTableRowProps({ view }))}>
              ${row.map((value) => html`
                <td ${part(cell({ ...value, columns: 4 }))}><div class="ring" ${part(trigger({ ...value, columns: 4 }))}>${value.label}</div></td>
              `)}
            </tr>
          `)}
        </tbody>
      </table>
    `
    return html`
      <div class="popup" ${part(api.getContentProps())}>
        <div ${part(api.getViewProps({ view: "day" }))}>
          ${bar("day", api.visibleRangeText.start)}
          <table ${part(api.getTableProps({ view: "day" }))}>
            <thead ${part(api.getTableHeadProps({ view: "day" }))}>
              <tr ${part(api.getTableRowProps({ view: "day" }))}>
                ${api.weekDays.map((day) => html`<th scope="col" aria-label=${day.long}>${day.narrow}</th>`)}
              </tr>
            </thead>
            <tbody ${part(api.getTableBodyProps({ view: "day" }))}>
              ${api.weeks.map((week) => html`
                <tr ${part(api.getTableRowProps({ view: "day" }))}>
                  ${week.map((value) => html`
                    <td ${part(api.getDayTableCellProps({ value }))}><div class="ring" ${part(api.getDayTableCellTriggerProps({ value }))}>${value.day}</div></td>
                  `)}
                </tr>
              `)}
            </tbody>
          </table>
        </div>
        <div ${part(api.getViewProps({ view: "month" }))}>
          ${bar("month", api.visibleRange.start.year)}
          ${grid("month", api.getMonthsGrid({ columns: 4, format: "short" }), api.getMonthTableCellProps, api.getMonthTableCellTriggerProps)}
        </div>
        <div ${part(api.getViewProps({ view: "year" }))}>
          ${bar("year", `${decade.start} – ${decade.end}`)}
          ${grid("year", api.getYearsGrid({ columns: 4 }), api.getYearTableCellProps, api.getYearTableCellTriggerProps)}
        </div>
        ${this.value && !this.required ? html`<button class="clear press ring" ${part(api.getClearTriggerProps())}>${said.clear}</button>` : nothing}
      </div>
    `
  }
}
customElements.define("ui-date-picker", DatePicker)

const previous = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("caret-left")}</svg>`
const next = html`<svg viewBox="0 0 256 256" aria-hidden="true">${icon("caret-right")}</svg>`
