// A list's keyboard, with no look: one tab stop for its rows, Up and Down between them, Home and End to its ends, and
// letters typed to the next row that starts with them, as macOS's NSTableView and Windows' ListView move and as
// WAI-ARIA's practices ask of a list's items. On the roving tab stop (behavior-primitives/roving-focus.ts, as radios,
// menus and tabs have it) and typeahead (behavior-primitives/typeahead.ts).
import { createMemo, createSignal, onCleanup, onSettled } from "solid-js"
import { documentOrder } from "#primitives"
import { createRovingFocus, type RovingItemProps } from "../behavior-primitives/roving-focus.ts"
import { createTypeahead, isTypeaheadKey } from "../behavior-primitives/typeahead.ts"
import type { Keyed } from "../types/ui.d.ts"

/** A row, as the keyboard sees it: what it draws, the list it's in and where it is there where the list knows (a long
 * list's, whose rows a native host places by their index, not by their order in its tree), what it says, whether it's
 * pressed (only those take the keyboard), whether it's the one open beside the list (the stop before the keyboard has
 * been in it), and what opening it does. */
export type ListRow = { node: () => unknown; list: object; at?: () => number; label: () => string; pressable: () => boolean; current: () => boolean; open: () => void }

/** Which of two rows comes first: by their place in a long list, else by where they're drawn in the page's tree. */
const order = (a: ListRow, b: ListRow) => (a.list === b.list && a.at && b.at ? a.at() - b.at() : documentOrder(a.node(), b.node()))

/** `const keys = createList({ opens: () => sidebar })`, then each list `onKeyDown={keys.keydown}` and each row's
 * pressable `{...keys.row({ node, list, label, pressable, current, open })}`: the rows of every list it's given to
 * one tab stop, the last one the keyboard was on, else the one open beside the lists, else the first; Up and Down move
 * between them, from one list on to the next, Home and End to the ends, and letters typed to the next that starts
 * with them. With `opens`, the row moved to is opened, as a Mac's sidebar opens the section the arrows come to. */
export function createList(props: { opens: () => boolean }) {
  let made = 0
  const [rows, setRows] = createSignal<(ListRow & { key: string })[]>([])
  // Rows are put in order once they're drawn, where they are in the page.
  const [drawn, setDrawn] = createSignal(0)
  const ordered = createMemo(() => (drawn(), rows().filter((row) => row.pressable()).sort(order)))
  const roving = createRovingFocus({ items: () => ordered().map((row) => ({ key: row.key, preferred: row.current() })) })
  const typeahead = createTypeahead()
  const moved = (key: string) => props.opens() && ordered().find((row) => row.key === key)?.open()
  // How many rows each long list has, whose rows are mostly not drawn, and the row of one the keyboard is being sent
  // to, which the list keeps drawn (`keep`) and which takes the keyboard when it's there.
  const lengths = new Map<object, () => number>()
  const [wanted, setWanted] = createSignal<{ list: object; index: number }>()
  return {
    /** A long list's row count, so Home and End reach its real ends. */
    sized(list: object, count: () => number) {
      lengths.set(list, count)
      onCleanup(() => lengths.delete(list))
    },
    /** The row of `list` the keyboard is being sent to, which isn't drawn yet. */
    keeping: (list: object) => (wanted()?.list === list ? wanted()!.index : undefined),
    keydown(event: Keyed) {
      // Only where a row has it: a field or a switch in one keeps its own keys.
      if (roving.focused() === undefined) return
      const sent = wanted()
      setWanted(undefined)
      // Home and End go to the first and last row of the lists, which a long list may not have drawn: its list keeps
      // that row, and it takes the keyboard where it's drawn.
      if ((event.key === "Home" || event.key === "End") && !event.altKey && !event.ctrlKey && !event.metaKey) {
        const rows = ordered()
        const edge = event.key === "Home" ? rows[0] : rows.at(-1)
        const count = edge && lengths.get(edge.list)
        if (edge && count) {
          const index = event.key === "Home" ? 0 : count() - 1
          const drawn = rows.find((row) => row.list === edge.list && row.at?.() === index)
          event.preventDefault()
          typeahead.reset()
          if (drawn) return (roving.focus(drawn.key), moved(drawn.key))
          return setWanted({ list: edge.list, index })
        }
      }
      // The next row of a long list is the one after it by its index, drawn or not, where the rows drawn have a gap;
      // after a row the keyboard is still being sent to, the one after that, so no key is lost while the page is busy.
      const step = event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0
      const here = step ? ordered().find((row) => row.key === roving.focused()) : undefined
      const from = sent ?? (here?.at && { list: here.list, index: here.at() })
      const length = from && lengths.get(from.list)
      if (step && from && length && !event.altKey && !event.ctrlKey && !event.metaKey) {
        const index = Math.min(Math.max(from.index + step, 0), length() - 1)
        const drawn = ordered().find((row) => row.list === from.list && row.at?.() === index)
        if (sent && drawn) return (event.preventDefault(), typeahead.reset(), roving.focus(drawn.key), moved(drawn.key))
        if (sent || !drawn) return (event.preventDefault(), typeahead.reset(), setWanted({ list: from.list, index }))
      }
      const to = roving.keydown(event)
      if (to !== undefined) return (typeahead.reset(), moved(to))
      if (!isTypeaheadKey(event)) return
      const list = ordered()
      const found = typeahead(event.key, list.map((row) => row.label()), list.findIndex((row) => row.key === roving.focused()))
      if (found < 0) return
      event.preventDefault()
      roving.focus(list[found].key)
      moved(list[found].key)
    },
    /** A row: its pressable's tab stop and keyboard, for as long as it's there. */
    row(row: ListRow): RovingItemProps {
      const entry = { ...row, key: String(made++) }
      setRows((now) => [...now, entry])
      onSettled(() => void setDrawn((times) => times + 1))
      onCleanup(() => setRows((now) => now.filter((one) => one !== entry)))
      const item = roving.item(entry.key)
      return {
        get tabIndex() {
          return item.tabIndex
        },
        get autoFocus() {
          return item.autoFocus || (wanted()?.list === row.list && row.at?.() === wanted()?.index)
        },
        onFocus: () => (setWanted(undefined), item.onFocus()),
        onBlur: item.onBlur,
      }
    },
  }
}
