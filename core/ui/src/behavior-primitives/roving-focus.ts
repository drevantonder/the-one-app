// The roving tab stop: one tab stop for a list's items, and the arrow keys between them, as WAI-ARIA's practices ask
// of tabs, radios, menus and toolbars. From Vercel's UI SDK (Apache-2.0),
// packages/core/src/foundation/focus/roving-focus.ts at ab55044caee66cc8228fa7ed3525088a1b7b5af7: its rules (the stop
// on the item focused last where the group remembers it, else the preferred, else the first enabled; the arrows along
// its orientation, swapped right to left; Home and End; wrapping with `loop`; Alt and the shortcut modifiers left
// alone), on our host's focus: an item has the keyboard by its `autoFocus`, which a move sets, and says it has it by
// `onFocus`, in place of an element's `focus()` and its focus events.
import { createMemo, createSignal } from "solid-js"
import { direction } from "#bridge"
import { moveIndex, rovingStop, stepFor, type Orientation } from "./collection.ts"
import type { Keyed } from "../types/ui.d.ts"

/** An item of a roving group: its key, whether it can't be activated (it can still take focus), and whether it takes
 * the stop before focus has been in the group (the selected tab, the checked radio). */
export type RovingItem = { key: string; disabled?: boolean; preferred?: boolean }

/** What a roving group's item spreads on its `Pressable`: its tab stop, the keyboard where the group moves it, and
 * telling the group it has it. */
export type RovingItemProps = { tabIndex: 0 | -1; autoFocus: boolean; onFocus: () => void; onBlur: () => void }

/** `const roving = createRovingFocus({ items: () => tabs().map((tab) => ({ key: tab.value, preferred: tab.value === value() })) })`,
 * then each item `{...roving.item(key)}` and the group `onKeyDown={(event) => roving.keydown(event)}`: one tab stop for
 * the items, which the arrows move along `orientation` (`vertical` unless said; `both` for radios), with Home and End,
 * wrapping with `loop`. → `keydown`'s the key it moved to, so a group that selects as it moves (tabs, radios) selects
 * it, or undefined where the key isn't the group's. */
export function createRovingFocus(options: { items: () => RovingItem[]; orientation?: Orientation; loop?: boolean; remember?: boolean }) {
  const remember = options.remember ?? true
  // The item focus was on last, which a group that remembers keeps the stop on; the one that has the keyboard now;
  // and the one a move sends it to.
  const [remembered, setRemembered] = createSignal<string | null>(null)
  const [holder, setHolder] = createSignal<string>()
  const [moved, setMoved] = createSignal<string>()
  // Worked out once as the items change, not by each item's tab stop: each item reads it, not every item.
  const stop = createMemo(() => rovingStop(options.items().map((item) => ({ key: item.key, disabled: !!item.disabled, focusable: true, preferred: !!item.preferred })), remember ? remembered() : null))
  return {
    stop,
    /** The item that has the keyboard, if one of the group's does. */
    focused: holder,
    item: (key: string): RovingItemProps => ({
      get tabIndex() {
        return stop() === key ? 0 : -1
      },
      get autoFocus() {
        return moved() === key
      },
      onFocus: () => (setHolder(key), remember && setRemembered(key)),
      onBlur: () => (holder() === key && setHolder(undefined), moved() === key && setMoved(undefined)),
    }),
    /** Gives an item the keyboard. */
    focus: (key: string) => setMoved(key),
    keydown(event: Keyed, from = holder() ?? stop() ?? undefined) {
      if (event.altKey || event.ctrlKey || event.metaKey) return
      const list = options.items()
      const at = list.findIndex((item) => item.key === from)
      const step = stepFor(event.key, options.orientation ?? "vertical", direction() === "rtl")
      if (at < 0 || step === undefined) return
      const to = list[moveIndex(at, step, list.length, options.loop ?? false)]?.key
      if (to === undefined) return
      event.preventDefault()
      setMoved(to)
      if (remember) setRemembered(to)
      return to
    },
  }
}
