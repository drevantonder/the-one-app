// A menu, with its state and no look: open or not and where, its items, the keyboard among them, and what was chosen.
// From Vercel's UI SDK (Apache-2.0), packages/core/src/components/menu/menu-api.ts at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7: opened, the keyboard is on its first item; the arrows move through the
// items, wrapping (`loop`), Home and End to the first and the last, a letter to the next that starts with it
// (typeahead); Enter and Space choose; Tab, Escape, a press outside and the device's back close it, and the keyboard
// goes back to its trigger; it opens under its trigger, its end at the trigger's, flipped above where there isn't
// room. On our host's focus (`autoFocus`) and layout (`onLayout`, `viewport()`).
import { createEffect, createSignal, merge } from "solid-js"
import { direction, viewport } from "#bridge"
import { createControllableSignal } from "../behavior-primitives/controllable.ts"
import { createDismissable } from "../behavior-primitives/dismissable.ts"
import { computePlacement } from "../behavior-primitives/placement.ts"
import { createRovingFocus } from "../behavior-primitives/roving-focus.ts"
import { createTypeahead, isTypeaheadKey } from "../behavior-primitives/typeahead.ts"
import { pixels, tokens } from "../tokens.ts"
import type { Keyed, Laid, Located } from "../types/ui.d.ts"

/** Where something is drawn: what opens a menu, or where a row was held. */
export type At = { x: number; y: number; width: number; height: number }

/** An item, as the keyboard and the action panel see it. */
export type MenuEntry = { value: string; label: string; disabled: boolean; destructive: boolean }

const rectOf = (event: Laid | Located): At => {
  const { x, y, width, height } = event.currentTarget.getBoundingClientRect()
  return { x, y, width, height }
}

/** A `Menu.Root`'s props. */
export interface MenuProps {
  open?: boolean
  onOpenChange?: (open: boolean) => void
  onSelect?: (value: string) => void
}

/** `const menu = createMenu(props)`: one menu's state, and the props of each of its parts. */
export function createMenu(props: MenuProps) {
  const [shown, setShown] = createControllableSignal<boolean>({ value: () => props.open, defaultValue: () => false, onChange: (open) => props.onOpenChange?.(open) })
  // Where it was opened (a row held), else under its trigger, wherever that's drawn now.
  const [where, setWhere] = createSignal<At>()
  const [trigger, setTrigger] = createSignal<At>()
  const [size, setSize] = createSignal<{ width: number; height: number }>()
  const [items, setItems] = createSignal<MenuEntry[]>([])
  const usable = () => items().filter((item) => !item.disabled)
  const roving = createRovingFocus({ items: () => usable().map((item) => ({ key: item.value })), loop: true, remember: false })
  const typeahead = createTypeahead()
  const at = () => (shown() ? (where() ?? trigger()) : undefined)
  const open = (to: At | undefined) => {
    setWhere(to)
    setShown(!!to)
  }
  const close = () => open(undefined)
  const select = (value: string) => (close(), props.onSelect?.(value))
  const layer = createDismissable({ open: () => !!at(), onDismiss: close })
  // Opened, the keyboard is on its first item. Its size stays what it was last laid out as, which its host says again
  // whenever it changes: a host tells a node's layout only as it changes, so one moved into the layer at the size it had
  // says nothing, and waiting to be told again would leave it unplaced, unseen.
  createEffect(
    () => !!at(),
    (opened) => {
      typeahead.reset()
      const first = usable()[0]
      if (opened && first) roving.focus(first.value)
    },
  )
  /** Where its content goes, once it's measured: under what opened it, its end at that's end, flipped above where
   * there isn't room, inside the window's edges. */
  const placed = () => {
    const anchor = at()
    const measured = size()
    if (!anchor || !measured) return
    return computePlacement(anchor, measured, viewport(), { side: "bottom", align: "end", sideOffset: pixels(tokens.grid[1]), alignOffset: 0, collisionPadding: pixels(tokens.grid[2]), rtl: direction() === "rtl" })
  }
  return {
    at,
    placed,
    open,
    close,
    select,
    items,
    /** Adds an item, in order. → what takes it out. */
    add: (entry: MenuEntry) => (setItems((now) => [...now, entry]), () => setItems((now) => now.filter((item) => item !== entry))),
    /** Opens it under what was pressed, or closes it if it's open. */
    toggle: (event: Located | undefined) => open(at() || !event ? undefined : rectOf(event)),
    trigger: () => ({
      "aria-haspopup": "menu" as const,
      get "aria-expanded"() {
        return at() ? ("true" as const) : ("false" as const)
      },
      get autoFocus() {
        return layer.returning()
      },
      onLayout: (event: Laid) => setTrigger(rectOf(event)),
    }),
    /** What's around it, under it in its layer: a press there closes it. */
    outside: () => layer.outside,
    content: () => ({
      role: "menu" as const,
      tabIndex: -1 as const,
      onLayout: (event: Laid) => {
        const { width, height } = event.currentTarget.getBoundingClientRect()
        setSize({ width, height })
      },
      onKeyDown: (event: Keyed) => {
        if (layer.keydown(event) || roving.keydown(event) !== undefined) return
        if (event.key === "Tab") return close()
        if (!isTypeaheadKey(event)) return
        const list = usable()
        const found = typeahead(event.key, list.map((item) => item.label), list.findIndex((item) => item.value === roving.focused()))
        if (found >= 0) (event.preventDefault(), roving.focus(list[found].value))
      },
    }),
    item: (entry: () => MenuEntry) =>
      merge(roving.item(entry().value), {
        role: "menuitem" as const,
        tabIndex: -1 as const,
        get "aria-disabled"() {
          return entry().disabled ? ("true" as const) : undefined
        },
        onPointerEnter: () => !entry().disabled && roving.focus(entry().value),
        onPress: () => !entry().disabled && select(entry().value),
        onKeyDown: (event: Keyed) => (event.key === "Enter" || event.key === " ") && !typeahead.active() && (event.preventDefault(), !entry().disabled && select(entry().value)),
      }),
  }
}
