import { Show } from "solid-js"
import { Icon } from "./button.jsx"
import { pace, reduced } from "./shared.js"

/// `<List><For each={notes()}>{(note) => <Row heading={note.text} />}</For></List>`: things of one kind, one under
/// another, in one group that stops at a readable width however wide the room is. With no rows it shows nothing. A
/// row added to a list that has some opens up where it goes, and one taken out closes up, the rest making room or
/// closing the gap. A list that changes in any other way (filled, a row edited, another list's rows) just shows
/// what it's given.
export function List(props) {
  return (
    <div class={["ui-list", props.class]} role="list" ref={moves}>
      {props.children}
    </div>
  )
}

/// What a row says. An app often draws its whole list again with new rows: one that says the same is the same row.
const says = (row) => row.textContent

/// Watches a list's rows come and go, and moves them as they do.
function moves(list) {
  let rows = []
  const seen = new MutationObserver(() => {
    const before = rows
    const after = (rows = [...list.children].filter((row) => !row.inert))
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
    rows = [...list.children]
    seen.observe(list, { childList: true })
  })
}

/// A row that was taken out leaves a copy of itself where it was, `before` the row that followed it, for as long as
/// it takes to close up: the row itself is the app's, and gone.
function leave(list, row, before = null) {
  const height = `${row.offsetHeight || list.firstElementChild?.offsetHeight || 0}px`
  const copy = row.cloneNode(true)
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
/// the row is its to press.
export function Row(props) {
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
  return (
    <div class="ui-row" role="listitem" data-current={props.current ? "" : undefined}>
      <Show when={props.control}>
        <label class="ui-choice control">{props.control}</label>
      </Show>
      <Show when={props.href} fallback={
        <Show when={props.onClick} fallback={<div class="row">{inside()}</div>}>
          <button type="button" class="row press" aria-current={props.current ? "true" : undefined} onClick={(event) => props.onClick(event)}>{inside()}</button>
        </Show>
      }>
        <a class="row press" href={props.href} aria-current={props.current ? "true" : undefined}>{inside()}</a>
      </Show>
    </div>
  )
}
