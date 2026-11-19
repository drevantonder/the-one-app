// A select, with its state and no look: what's chosen, whether its list is open and where, and the one its keys are
// on. From Vercel's UI SDK (Apache-2.0) at ab55044caee66cc8228fa7ed3525088a1b7b5af7,
// packages/core/src/components/select/select-keyboard.ts and select-api.ts: the keyboard stays on its face (a
// combobox) and says which option it's on (`aria-activedescendant`); closed, the arrows choose the next one, as a
// Mac's pop-up button's do, and a letter the next that starts with it, as a `<select>`'s; open, the arrows, Home and
// End move through the options, skipping those that can't be chosen, a letter to the next that starts with it, Enter
// and Space choose, and Escape, Tab, the keyboard leaving, a press outside and the device's back close it. Its list
// opens under its face, as wide, flipped above where there isn't room. On our host's layout (`onLayout`,
// `viewport()`).
import { createEffect, createSignal } from "solid-js"
import type { CollectionItem, ListCollection } from "@zag-js/collection"
import { direction, viewport } from "#bridge"
import { createControllableSignal } from "../behavior-primitives/controllable.ts"
import { createDismissable } from "../behavior-primitives/dismissable.ts"
import { computePlacement } from "../behavior-primitives/placement.ts"
import { createTypeahead, isTypeaheadKey } from "../behavior-primitives/typeahead.ts"
import { pixels, tokens } from "../tokens.ts"
import type { Keyed, Laid, Located, Rect } from "../types/ui.d.ts"

/** A `Select.Root`'s props, as Ark UI's select names them. */
export interface SelectProps<T extends CollectionItem = CollectionItem> {
  collection: ListCollection<T>
  value?: string[]
  defaultValue?: string[]
  placeholder?: string
  disabled?: boolean
  "aria-label"?: string
  onValueChange?: (details: { value: string[] }) => void
}

let selects = 0

/** `const select = createSelect(props, { disabled })`: one select's state, and the props of each of its parts. */
export function createSelect<T extends CollectionItem>(props: SelectProps<T>, field: { disabled: () => boolean }) {
  const id = `select-${++selects}`
  const items = () => props.collection.items
  const value = (item: T) => props.collection.getItemValue(item) ?? ""
  const label = (item: T) => props.collection.stringifyItem(item) ?? ""
  const itemOff = (item: T) => props.collection.getItemDisabled(item)
  const [own, setOwn] = createControllableSignal<string[]>({ value: () => props.value, defaultValue: () => props.defaultValue, onChange: (value) => props.onValueChange?.({ value }) })
  // With nothing chosen and nothing to say so, the first is, as a `<select>`'s is.
  const chosen = () => own() ?? (props.placeholder ? [] : [props.collection.firstValue].filter((one): one is string => one != null))
  const off = () => !!props.disabled || field.disabled()
  // Where its face is drawn while its list is open, which the list opens from, and the list's size once measured.
  const [at, setAt] = createSignal<Rect>()
  const [size, setSize] = createSignal<{ width: number; height: number }>()
  const [highlighted, setHighlighted] = createSignal(0)
  const typeahead = createTypeahead()
  const close = () => setAt(undefined)
  const layer = createDismissable({ open: () => !!at(), onDismiss: close })
  createEffect(() => !!at(), () => void (setSize(undefined), typeahead.reset()))
  const index = () => items().findIndex((item) => value(item) === chosen()[0])
  const choose = (item: T) => {
    if (itemOff(item)) return
    close()
    setOwn([value(item)])
  }
  const open = (face: Rect) => {
    if (off()) return
    setHighlighted(Math.max(0, index()))
    setAt({ x: face.x, y: face.y, width: face.width, height: face.height })
  }
  // The next that can be chosen, `by` one way or the other from `from`, or the first or the last.
  const step = (from: number, by: number) => {
    const all = items()
    for (let next = from + by; next >= 0 && next < all.length; next += by) if (!itemOff(all[next])) return next
  }
  const placed = () => {
    const face = at()
    const measured = size()
    if (!face || !measured) return
    return computePlacement(face, measured, viewport(), { side: "bottom", align: "start", sideOffset: pixels(tokens.grid[1]), alignOffset: 0, collisionPadding: pixels(tokens.grid[2]), rtl: direction() === "rtl" })
  }
  const keyed = (event: Keyed) => {
    if (off()) return
    const by = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0
    if (by) {
      event.preventDefault()
      if (!at()) {
        const next = step(index(), by)
        if (next !== undefined) choose(items()[next])
      } else setHighlighted((now) => step(now, by) ?? now)
    } else if (at() && (event.key === "Home" || event.key === "End")) {
      event.preventDefault()
      const next = event.key === "Home" ? step(-1, 1) : step(items().length, -1)
      if (next !== undefined) setHighlighted(next)
    } else if (at() && (event.key === "Enter" || (event.key === " " && !typeahead.active()))) {
      event.preventDefault()
      const item = items()[highlighted()]
      if (item) choose(item)
    } else if (at() && event.key === "Tab") close()
    else if (at() && layer.keydown(event)) return
    else if (isTypeaheadKey(event)) {
      const all = items()
      const found = typeahead(event.key, all.map((item) => (itemOff(item) ? "" : label(item))), at() ? highlighted() : index())
      if (found < 0) return
      event.preventDefault()
      if (at()) setHighlighted(found)
      else choose(all[found])
    }
  }
  const option = (index: number) => `${id}-${index}`
  return {
    items,
    value,
    label,
    itemOff,
    chosen,
    off,
    at,
    placed,
    highlighted,
    /** What its face says: the label of what's chosen. */
    text: () => chosen().map((one) => props.collection.stringifyItem(props.collection.find(one))).filter(Boolean).join(", "),
    choose,
    face: () => ({
      role: "combobox" as const,
      "aria-haspopup": "listbox" as const,
      "aria-controls": id,
      get "aria-expanded"() {
        return at() ? ("true" as const) : ("false" as const)
      },
      get "aria-activedescendant"() {
        return at() ? option(highlighted()) : undefined
      },
      get "aria-disabled"() {
        return off() ? ("true" as const) : undefined
      },
      onKeyDown: keyed,
      onBlur: close,
    }),
    /** Opens its list under its face, or closes it. */
    toggle: (event: Located) => (at() ? close() : open(event.currentTarget.getBoundingClientRect())),
    outside: () => layer.outside,
    list: () => ({
      role: "listbox" as const,
      id,
      onLayout: (event: Laid) => {
        const { width, height } = event.currentTarget.getBoundingClientRect()
        setSize({ width, height })
      },
    }),
    option: (item: T, index: () => number) => ({
      role: "option" as const,
      get id() {
        return option(index())
      },
      get "aria-selected"() {
        return chosen().includes(value(item)) ? ("true" as const) : ("false" as const)
      },
      get "aria-disabled"() {
        return itemOff(item) ? ("true" as const) : undefined
      },
      tabIndex: -1 as const,
      onPress: () => choose(item),
      onPointerEnter: () => setHighlighted(index()),
    }),
  }
}
