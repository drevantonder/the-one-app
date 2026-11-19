// A listbox, with its state and no look: rows to choose from while what's typed in keeps the keyboard, as a search's
// results are. From Vercel's UI SDK (Apache-2.0), packages/core/src/components/listbox/listbox-root-keyboard.ts at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7: the field that has the keyboard is a combobox that says which option it's
// on (`aria-activedescendant`), the arrows move to the option before and after, stopping at the ends, Home and End to
// the first and last (foundation/focus/collection.ts), and Enter opens the one it's on.
import { createSignal } from "solid-js"
import { moveIndex, stepFor } from "../behavior-primitives/collection.ts"
import type { Keyed } from "../types/ui.d.ts"

/** A listbox: which of its rows is chosen, and what's typed in that drives it. */
export interface Listbox {
  /** The list's id, which the field's `aria-controls` names. */
  readonly id: string
  chosen: () => number
  choose: (index: number) => void
  /** What the field that has the keyboard says of it, as WAI-ARIA's combobox: spread on it. */
  field: () => { role: "combobox"; "aria-controls": string; "aria-expanded": "true" | "false"; "aria-activedescendant"?: string }
  /** The field's keys: ↑ ↓ choose the row before and after, Home and End the first and last, Return opens the one
   * chosen (`onOpen`). → whether it took the key. */
  keys: (event: Keyed) => boolean
}

/** What its list tells it, how many rows there are, and an option's id by where it is. */
type Inside = { option: (index: number) => string; rows: (count: number) => void }
export const insides = new WeakMap<Listbox, Inside>()

let listboxes = 0

/// `const results = createListbox()`, then `<Search {...results.field()} onKeyDown={results.keys} … />` and
/// `<List listbox={results} label="Results">…rows…</List>`: a list of rows to choose from while what's typed in keeps
/// the keyboard, as a search's results are (WAI-ARIA's combobox and listbox): ↑ ↓, Home and End in the field choose a
/// row, Return calls `onOpen` with the one chosen, the pointer over a row chooses it (a press opens it, as its
/// `onPress` says), and a screen reader hears which. `results.chosen()` is the
/// one chosen, first unless it's moved; `results.choose(0)` goes back to the first, as a search does once it's typed in.
export function createListbox({ onOpen }: { onOpen?: (index: number) => void } = {}): Listbox {
  const id = `listbox-${++listboxes}`
  const [chosen, setChosen] = createSignal(0)
  const [count, setCount] = createSignal(0)
  const option = (index: number) => `${id}-${index}`
  const choose = (index: number) => setChosen(Math.max(0, Math.min(count() - 1, index)))
  const keys = (event: Keyed) => {
    if (event.altKey || event.metaKey || event.ctrlKey || !count()) return false
    const step = stepFor(event.key, "vertical", false)
    if (step !== undefined) choose(moveIndex(chosen(), step, count(), false))
    else if (event.key === "Enter" && onOpen) onOpen(chosen())
    else return false
    event.preventDefault()
    return true
  }
  const listbox: Listbox = {
    id,
    chosen,
    choose,
    keys,
    field: () => ({ role: "combobox", "aria-controls": id, "aria-expanded": count() ? "true" : "false", "aria-activedescendant": count() ? option(chosen()) : undefined }),
  }
  insides.set(listbox, {
    option,
    rows: (rows) => {
      setCount(rows)
      if (chosen() >= rows) setChosen(Math.max(0, rows - 1))
    },
  })
  return listbox
}

