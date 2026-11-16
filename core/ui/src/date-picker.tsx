import { children, createContext, For, omit, Show, useContext } from "solid-js"
import { parseDate } from "./day.ts"
import { Icon } from "./button.tsx"
import { useField } from "./field.tsx"
import { said, touch } from "./shared.ts"
import { popover } from "./popover.ts"
import { machine, type Api, type Part, type Root } from "./zag.tsx"
import type { JSX } from "@solidjs/web"
import type * as datePicker from "@zag-js/date-picker"

const { Ark, use, draw } = machine("date-picker", () => import("./zag/date-picker.ts"), ["div", "getRootProps"], {
  Label: ["label", "getLabelProps"],
  Control: ["div", "getControlProps"],
  Input: ["input", "getInputProps", (zag) => zag.inputProps],
  Trigger: ["button", "getTriggerProps"],
  ClearTrigger: ["button", "getClearTriggerProps"],
  Positioner: ["div", "getPositionerProps"],
  Content: ["div", "getContentProps"],
}, {
  defaults: () => {
    const field = useField()
    return () => ({
      ids: { trigger: touch() ? undefined : field?.id, label: field?.label },
      invalid: field?.invalid(),
      positioning: { strategy: "fixed", placement: "bottom-start", gutter: 4 },
      locale: navigator.language,
      fixedWeeks: true,
    })
  },
  // Until its machine is here, its face says the day, and nothing opens.
  still: (root: Partial<datePicker.Props>) => ({ value: root.value ?? root.defaultValue ?? [], min: root.min, max: root.max, disabled: root.disabled }),
})

// The view a part is in (`day`, `month` or `year`), and the cell a cell's trigger is in.
const View = createContext<datePicker.DateView>("day")
const Cell = createContext<() => object>()

/// A part of a view, whose attributes Zag gives for the view it's in.
const inView = <Of extends HTMLElement>(element: string, get: string): Part<{}, Of> => (props) => {
  const api = use()
  const view = useContext(View)
  return draw(element, () => api()[get]({ view }), props)
}

// What each view's cells are called by Zag.
const cells: Record<datePicker.DateView, string> = { day: "getDayTableCell", month: "getMonthTableCell", year: "getYearTableCell" }


const shown = new Intl.DateTimeFormat(navigator.language, { dateStyle: "medium", timeZone: "UTC" })
/// A day of the month or a year, in the digits the user's language counts in.
const digits = new Intl.NumberFormat(navigator.language, { useGrouping: false })

/// `<Field label="Date"><DatePicker.Root name="date" /></Field>`: a day. It's Ark UI's date picker
/// (ark-ui.com/docs/components/date-picker), with its parts and props: `value` and `onValueChange` hold days as Zag
/// does (`parseDate("2026-11-14")`, and `value[0].toString()` back), `min` and `max` too. Given no parts, it draws
/// them all: a face that shows the day as the user's language writes it, which opens a calendar, or under a finger
/// the device's own picker. In a form it's sent as `name`, written as `<input type="date">` writes it.
export const DatePicker: {
  Root: Root<datePicker.Props & { placeholder?: string; name?: string; required?: boolean }>
  Label: Part<{}, HTMLLabelElement>
  Control: Part
  Input: Part<datePicker.InputProps, HTMLInputElement>
  Trigger: Part<{}, HTMLButtonElement>
  ClearTrigger: Part<{}, HTMLButtonElement>
  Positioner: Part
  Content: Part
  View: Part<{ view: datePicker.DateView }>
  ViewControl: Part
  PrevTrigger: Part<{}, HTMLButtonElement>
  ViewTrigger: Part<{}, HTMLButtonElement>
  NextTrigger: Part<{}, HTMLButtonElement>
  RangeText: Part
  Table: Part<{ columns?: number }, HTMLTableElement>
  TableHead: Part<{}, HTMLTableSectionElement>
  TableHeader: Part<{ scope?: string }, HTMLTableCellElement>
  TableBody: Part<{}, HTMLTableSectionElement>
  TableRow: Part<{}, HTMLTableRowElement>
  TableCell: Part<{ value: datePicker.DateValue | number; disabled?: boolean; columns?: number; visibleMonth?: datePicker.DateValue }, HTMLTableCellElement>
  TableCellTrigger: Part
  Context: (props: { children: (api: () => Api) => JSX.Element }) => JSX.Element
} = {
  ...Ark,
  Root: (props) => {
    const inside = children(() => props.children)
    return (
      <Ark.Root {...omit(props, "children", "placeholder", "name", "required")} class={["ui-date-picker", props.class]}>
        {inside.toArray().length ? inside() : <Whole placeholder={props.placeholder} name={props.name} required={props.required} />}
      </Ark.Root>
    )
  },
  Positioner: (props) => {
    const api = use()
    return <Ark.Positioner {...props} ref={popover(() => api().open, () => api().setOpen(false))} />
  },
  View: (props) => {
    const api = use()
    return (
      <View value={props.view}>{draw("div", () => api().getViewProps({ view: props.view }), omit(props, "view"))}</View>
    )
  },
  ViewControl: inView("div", "getViewControlProps"),
  PrevTrigger: inView("button", "getPreviousTriggerProps"),
  ViewTrigger: inView("button", "getViewTriggerProps"),
  NextTrigger: inView("button", "getNextTriggerProps"),
  RangeText: (props) => {
    const api = use()
    return draw("span", () => api().getRangeTextProps(), props)
  },
  Table: (props) => {
    const api = use()
    const view = useContext(View)
    return draw("table", () => api().getTableProps({ view, columns: props.columns }), omit(props, "columns"))
  },
  TableHead: inView("thead", "getTableHeadProps"),
  TableHeader: inView("th", "getTableHeaderProps"),
  TableBody: inView("tbody", "getTableBodyProps"),
  TableRow: inView("tr", "getTableRowProps"),
  TableCell: (props) => {
    const api = use()
    const view = useContext(View)
    const cell = () => ({ value: props.value, disabled: props.disabled, columns: props.columns, visibleMonth: props.visibleMonth })
    return (
      <Cell value={cell}>{draw("td", () => api()[`${cells[view]}Props`](cell()), omit(props, "value", "disabled", "columns", "visibleMonth"))}</Cell>
    )
  },
  TableCellTrigger: (props) => {
    const api = use()
    const view = useContext(View)
    const cell = useContext(Cell)
    return draw("div", () => api()[`${cells[view]}TriggerProps`](cell!()), props)
  },
  /// `<DatePicker.Context>{(api) => ...}</DatePicker.Context>`: what's in the calendar now, to draw from.
  Context: (props) => props.children(use()),
}

/// Every part of a date picker, as the UI library draws one. Under a finger the device's own `<input type="date">` lies
/// over the face, unseen, so a tap opens the device's own picker; with a mouse the face opens our calendar, and the
/// device's control only holds the day, for the form.
function Whole(props: { placeholder?: string; name?: string; required?: boolean }) {
  const api = use()
  const field = useField()
  const day = () => api().value[0]?.toString() ?? ""
  const bound = (value?: datePicker.DateValue) => value?.toString()
  return (
    <>
      <DatePicker.Control class="ui-picker">
        <Show when={!touch()}>
          <DatePicker.Trigger class="ui-box ring" aria-labelledby={field ? `${field.label} ${field.id}` : undefined} aria-describedby={field?.message} data-invalid={field?.invalid() ? "" : undefined}>
            <Face day={day()} placeholder={props.placeholder} />
          </DatePicker.Trigger>
        </Show>
        <Show when={touch()}>
          <div class="ui-box" aria-hidden="true" data-disabled={api().disabled ? "" : undefined} data-invalid={field?.invalid() ? "" : undefined}>
            <Face day={day()} placeholder={props.placeholder} />
          </div>
        </Show>
        <input
          class="native"
          type="date"
          id={touch() ? field?.id : undefined}
          name={props.name}
          value={day()}
          min={bound(api().min)}
          max={bound(api().max)}
          hidden={!touch()}
          required={props.required}
          disabled={api().disabled}
          aria-describedby={field?.message}
          aria-invalid={field?.invalid() ? "true" : undefined}
          onChange={(event) => (event.target.value ? api().setValue([parseDate(event.target.value)]) : api().clearValue())}
        />
      </DatePicker.Control>
      <Show when={!touch()}>
        <DatePicker.Positioner>
          <DatePicker.Content class="ui-popup ui-calendar">
            <Calendar />
            <Show when={day() && !props.required}>
              <DatePicker.ClearTrigger class="clear press ring">{said.clear}</DatePicker.ClearTrigger>
            </Show>
          </DatePicker.Content>
        </DatePicker.Positioner>
      </Show>
    </>
  )
}

function Face(props: { day: string; placeholder?: string }) {
  return (
    <>
      <span class={{ placeholder: !props.day }}>{props.day ? shown.format(new Date(props.day)) : props.placeholder}</span>
      <Icon name="calendar-blank" />
    </>
  )
}

/// The calendar: a month of days, and from its heading the months of a year, and from theirs the years.
function Calendar() {
  const api = use()
  const bar = (text: () => string) => (
    <DatePicker.ViewControl class="bar">
      <DatePicker.PrevTrigger class="press ring"><Icon name="caret-left" along /></DatePicker.PrevTrigger>
      <DatePicker.ViewTrigger class="press ring">{text()}</DatePicker.ViewTrigger>
      <DatePicker.NextTrigger class="press ring"><Icon name="caret-right" along /></DatePicker.NextTrigger>
    </DatePicker.ViewControl>
  )
  const grid = (rows: () => { value: number; label: string }[][]) => (
    <DatePicker.Table columns={4}>
      <DatePicker.TableBody>
        <For each={rows()}>
          {(row) => (
            <DatePicker.TableRow>
              <For each={row}>
                {(cell) => (
                  <DatePicker.TableCell value={cell.value} columns={4}>
                    <DatePicker.TableCellTrigger class="ring">{cell.label}</DatePicker.TableCellTrigger>
                  </DatePicker.TableCell>
                )}
              </For>
            </DatePicker.TableRow>
          )}
        </For>
      </DatePicker.TableBody>
    </DatePicker.Table>
  )
  return (
    <>
      <DatePicker.View view="day">
        {bar(() => api().visibleRangeText.start)}
        <DatePicker.Table>
          <DatePicker.TableHead>
            <DatePicker.TableRow>
              <For each={api().weekDays}>{(day) => <DatePicker.TableHeader scope="col" aria-label={day.long}>{day.narrow}</DatePicker.TableHeader>}</For>
            </DatePicker.TableRow>
          </DatePicker.TableHead>
          <DatePicker.TableBody>
            <For each={api().weeks}>
              {(week) => (
                <DatePicker.TableRow>
                  <For each={week}>
                    {(value) => (
                      <DatePicker.TableCell value={value}>
                        <DatePicker.TableCellTrigger class="ring">{digits.format(value.day)}</DatePicker.TableCellTrigger>
                      </DatePicker.TableCell>
                    )}
                  </For>
                </DatePicker.TableRow>
              )}
            </For>
          </DatePicker.TableBody>
        </DatePicker.Table>
      </DatePicker.View>
      <DatePicker.View view="month">
        {bar(() => digits.format(api().visibleRange.start.year))}
        {grid(() => api().getMonthsGrid({ columns: 4, format: "short" }))}
      </DatePicker.View>
      <DatePicker.View view="year">
        {bar(() => `${digits.format(api().getDecade().start)} – ${digits.format(api().getDecade().end)}`)}
        {grid(() => api().getYearsGrid({ columns: 4 }))}
      </DatePicker.View>
    </>
  )
}
