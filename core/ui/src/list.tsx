import { createContext, createSignal, onSettled, Show } from "solid-js"
import type { Accessor } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Icon } from "./button.tsx"
import { pace, reduced } from "./shared.ts"
import type { Api } from "./zag.tsx"

/// `<List><For each={notes()}>{(note) => <Row heading={note.text} />}</For></List>`: things of one kind, one under
/// another, in one group that stops at a readable width however wide the room is. With no rows it shows nothing. A
/// row added to a list that has some opens up where it goes, and one taken out closes up, the rest making room or
/// closing the gap. A list that changes in any other way (filled, a row edited, another list's rows) just shows
/// what it's given.
export function List(props: { children?: JSX.Element; class?: JSX.ClassValue }) {
  return (
    <div class={["ui-list", props.class]} role="list" ref={moves}>
      {props.children}
    </div>
  )
}

/// What a row says. An app often draws its whole list again with new rows: one that says the same is the same row.
const says = (row: Element) => row.textContent

/// Watches a list's rows come and go, and moves them as they do.
function moves(list: HTMLElement) {
  let rows: HTMLElement[] = []
  const seen = new MutationObserver(() => {
    const before = rows
    const after = (rows = ([...list.children] as HTMLElement[]).filter((row) => !row.inert))
    if (reduced.matches || !before.length) return
    const left = Map.groupBy(after, says)
    const stayed = new Map(before.map((row) => [row, left.get(says(row))?.shift()]))
    const gone = before.filter((row) => !stayed.get(row))
    const come = [...left.values()].flat()
    // Some gone and some new is a row that changed, or another list's rows: a plain swap.
    if (gone.length && come.length) return
    for (const row of come) row.animate({ height: ["0px", `${row.offsetHeight}px`], opacity: [0, 1] }, pace(list, "settle"))
    for (const row of gone) leave(list, row, before.slice(before.indexOf(row) + 1).map((later) => stayed.get(later)).find(Boolean))
    seen.takeRecords()
  })
  queueMicrotask(() => {
    rows = [...list.children] as HTMLElement[]
    seen.observe(list, { childList: true })
  })
}

/// A row that was taken out leaves a copy of itself where it was, `before` the row that followed it, for as long as
/// it takes to close up: the row itself is the app's, and gone.
function leave(list: HTMLElement, row: HTMLElement, before: HTMLElement | null = null) {
  const height = `${row.offsetHeight || (list.firstElementChild as HTMLElement | null)?.offsetHeight || 0}px`
  const copy = row.cloneNode(true) as HTMLElement
  copy.inert = true
  list.insertBefore(copy, before?.parentNode === list ? before : null)
  copy.animate({ opacity: [1, 0] }, { ...pace(list, "quick"), fill: "forwards" })
  copy.animate({ height: [height, "0px"] }, pace(list, "settle")).finished.then(() => copy.remove())
}

/// `<Row heading="Coffee with Sam" text="Meals · 12 Nov" />`: one thing in a list: its `heading`, one grey line of
/// `text`, what it starts with (`start`, an icon) and what it ends with (`end`: an amount, a switch). With `href`
/// it's a link to what it opens, with a chevron; with `onClick` the whole row is pressed and opens in place.
/// `current` marks the one that's open beside the list, in a split. `control` is a control before all of it,
/// pressed apart from the row, as a checkbox that picks rows is: `control={<input type="checkbox" aria-label="On
/// this claim" checked={...} onChange={...} />}`. It's drawn as the UI library's, and the row's whole height in front of
/// the row is its to press. A ⋯ `Menu` in its `end` opens on a long press or a right-click on the row too, and a
/// `destructive` item in it (Delete, Remove) shows as a swipe toward the row's start under a finger, as a phone's
/// lists do.
export function Row(props: {
  heading?: JSX.Element
  text?: JSX.Element
  start?: JSX.Element
  end?: JSX.Element
  href?: string
  onClick?: (event: MouseEvent) => void
  current?: boolean
  control?: JSX.Element
}) {
  const inside = () => (
    <>
      <Show when={props.start}>
        <span class="start">{props.start}</span>
      </Show>
      <span class="words">
        <span class="heading">{props.heading}</span>
        <Show when={props.text}>
          <span class="text">{props.text}</span>
        </Show>
      </span>
      <Show when={props.end}>
        <span class="end">{props.end}</span>
      </Show>
      <Show when={props.href}>
        <Icon name="caret-right" along />
      </Show>
    </>
  )
  const [menu, setMenu] = createSignal<RowMenu>()
  let row!: HTMLDivElement
  onSettled(() => presses(row, menu))
  return (
    <RowMenu value={setMenu}>
    <div class="ui-row" role="listitem" ref={row} data-current={props.current ? "" : undefined} data-menu={menu() ? "" : undefined} data-swipe={menu()?.destructive() ? "" : undefined}>
      <Show when={props.control}>
        <label class="ui-choice control">{props.control}</label>
      </Show>
      <Show when={props.href} fallback={
        <Show when={props.onClick} fallback={<div class="row">{inside()}</div>}>
          <button type="button" class="row press" aria-current={props.current ? "true" : undefined} onClick={(event) => props.onClick?.(event)}>{inside()}</button>
        </Show>
      }>
        <a class="row press" href={props.href} aria-current={props.current ? "true" : undefined}>{inside()}</a>
      </Show>
      <Show when={menu()?.destructive()}>
        {(item) => (
          <button type="button" class="swipe" tabindex="-1" aria-hidden="true" onClick={() => menu()?.select(item().dataset.value!)}>
            <span>{item().textContent}</span>
          </button>
        )}
      </Show>
    </div>
    </RowMenu>
  )
}

/** What a ⋯ menu in a row gives the row: its machine, to open it where the row was pressed, its `destructive` item,
 * and what selects an item, as pressing it does. */
export interface RowMenu {
  api: Accessor<Api>
  destructive: Accessor<HTMLElement | undefined>
  select: (value: string) => void
}

/// How a menu in a row's `end` tells the row it's there, and that it's gone. A menu anywhere else, as in a screen's bar,
/// finds none.
export const RowMenu = createContext<((menu: RowMenu | undefined) => void) | null>(null)

/// How far a finger moves before it's a swipe or a scroll, not a press.
const SLOP = 10

/// A press on a row with a menu: held, or a right-click, it opens the menu where it was, as Zag's context menu does
/// (`getContextTriggerProps`). With a `destructive` item, a finger swiping the row toward its start moves it aside
/// to show that item, which stays shown past half its width and is done when swiped most of the way. A press on the
/// row while it's aside, anywhere but that item, puts it back. Presses on the menu's own button, or the row's
/// `control`, are theirs. → what stops it.
function presses(row: HTMLElement, menu: Accessor<RowMenu | undefined>) {
  let start: { x: number; y: number; at: number } | undefined
  let swiping = false
  let swiped = 0
  // When a press last swiped the row, or put it back: the click its lifting makes isn't a press on the row.
  let moved = 0
  const toward = () => (getComputedStyle(row).direction === "rtl" ? -1 : 1)
  // How far aside the row stays to show its item: the item's words, with room either side.
  const shown = () => (row.querySelector<HTMLElement>(".swipe > span")?.offsetWidth ?? 0) + 40
  const aside = (px: number, settle = false) => {
    swiped = Math.max(0, Math.min(px, row.offsetWidth))
    row.toggleAttribute("data-settling", settle)
    row.style.setProperty("--swiped", `${swiped}px`)
  }
  const theirs = (event: Event) => event.target instanceof Element && !!event.target.closest("[data-menu-trigger], .control, .swipe")
  const context = () => menu()?.api().getContextTriggerProps()

  const down = (event: PointerEvent) => {
    if (!menu() || theirs(event) || event.button !== 0) return
    if (swiped) return void ((moved = event.timeStamp), aside(0, true))
    start = { x: event.clientX, y: event.clientY, at: swiped }
    swiping = false
    context()?.onPointerDown(event)
  }
  const move = (event: PointerEvent) => {
    if (!start) return
    const dx = (start.x - event.clientX) * toward()
    const dy = event.clientY - start.y
    if (!swiping && Math.hypot(dx, dy) < SLOP) return
    if (!swiping) {
      context()?.onPointerCancel(event)
      swiping = event.pointerType === "touch" && !!menu()?.destructive() && Math.abs(dx) > Math.abs(dy)
      if (!swiping) return void (start = undefined)
      row.setPointerCapture(event.pointerId)
    }
    aside(start.at + dx)
  }
  const up = (event: PointerEvent) => {
    if (!start) return
    start = undefined
    context()?.onPointerUp(event)
    if (!swiping) return
    moved = event.timeStamp
    const item = menu()?.destructive()
    if (item && swiped > row.offsetWidth * 0.6) {
      aside(row.offsetWidth, true)
      menu()?.select(item.dataset.value!)
      setTimeout(() => aside(0, true), 400)
    } else aside(swiped > shown() / 2 ? shown() : 0, true)
  }
  const cancel = (event: PointerEvent) => {
    context()?.onPointerCancel(event)
    if (start && swiping) aside(0, true)
    start = undefined
  }
  // What a press that was a swipe, a held press, or put the row back would otherwise do as it lifts: open the row.
  const click = (event: MouseEvent) => {
    // The item it showed was pressed: done, and the row goes back.
    if (event.target instanceof Element && event.target.closest(".swipe")) return aside(0, true)
    if (theirs(event) || !(event.timeStamp - moved < 600 || menu()?.api().open)) return
    event.preventDefault()
    event.stopPropagation()
  }
  const right = (event: MouseEvent) => {
    if (!theirs(event)) context()?.onContextMenu(event)
  }
  // Pressed anywhere else, a row that's aside goes back.
  const elsewhere = (event: PointerEvent) => {
    if (swiped && !row.contains(event.target as Node)) aside(0, true)
  }
  row.addEventListener("pointerdown", down)
  row.addEventListener("pointermove", move)
  row.addEventListener("pointerup", up)
  row.addEventListener("pointercancel", cancel)
  row.addEventListener("click", click, true)
  row.addEventListener("contextmenu", right)
  document.addEventListener("pointerdown", elsewhere, true)
  return () => document.removeEventListener("pointerdown", elsewhere, true)
}
