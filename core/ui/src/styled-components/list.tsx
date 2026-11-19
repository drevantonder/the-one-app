// A list and its rows: written once on the drawing primitives, the same on the DOM and on a native host.
import { children, createContext, createEffect, createMemo, createSignal, flush, For, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Text } from "#primitives"
import { direction, reducedMotion, touch, viewport } from "#bridge"
import { Icon } from "./button.tsx"
import { RowMenu, type RowMenu as Menu } from "./menu.tsx"
import { Choose } from "./collect.ts"
import { messages } from "../words.ts"
import type { Action } from "./actions.tsx"
import { negative, styles } from "../styles.ts"
import { duration, tokens } from "../tokens.ts"
import type { Keyed, Located, Pressed } from "../types/ui.d.ts"

const look = styles({
  list: {
    maxWidth: tokens.length.readable,
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.card,
    color: tokens.colour.label,
    boxShadow: tokens.shadow.edge,
    overflow: "hidden",
  },
  // On a computer, as Raycast's: rows on the window itself, each a rounded highlight under the pointer or where it's
  // chosen, their words lined up with the heading above them.
  flat: { backgroundColor: tokens.colour.transparent, boxShadow: "none", overflow: "visible", marginInline: negative(tokens.grid[2]) },
  // Every row draws the line above it, and the list hides the first's.
  rows: { marginTop: negative(tokens.length.hairline) },
  flatRows: { marginTop: 0 },
  // In grouped cards on a computer, as Raycast's Settings: the card is inset, and its lines between rows run from its
  // inset to its inset.
  groupedList: { width: "100%", maxWidth: tokens.length.readable, alignSelf: "center", paddingInline: tokens.grid[3] },
  row: { position: "relative", flexDirection: "row", alignItems: "stretch", overflow: "hidden", borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  flatRow: { borderTopWidth: 0, borderRadius: tokens.radius.medium },
  groupedInside: { minHeight: tokens.length.groupedRow, paddingInline: 0, paddingBlock: tokens.grid[1.5], gap: tokens.grid[2.5], backgroundColor: tokens.colour.transparent },
  sidebarInside: { minHeight: tokens.length.sidebarRow },
  wraps: { flexWrap: "wrap" },
  current: { backgroundColor: tokens.colour.selection },
  // A listbox's option, the one chosen highlighted as a row open beside its list is.
  option: { borderRadius: tokens.radius.medium },
  chosen: { backgroundColor: tokens.colour.selection },
  // Its row has no highlight of its own under the pointer: the pointer chooses it, which highlights the option.
  quiet: { backgroundColor: tokens.colour.transparent },
  held: { userSelect: "none", touchAction: "pan-y" },
  inside: {
    flexGrow: 1,
    flexShrink: 1,
    flexBasis: tokens.length.menu,
    minWidth: 0,
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[3],
    minHeight: tokens.length.row,
    paddingBlock: tokens.grid[2],
    paddingInline: tokens.grid[3],
    backgroundColor: { hover: tokens.colour.quinaryLabel, pressed: tokens.colour.quaternaryLabel, focused: tokens.colour.quinaryLabel },
    transitionProperty: ["backgroundColor"],
    transition: tokens.motion.quick,
  },
  flatInside: { paddingBlock: 0, paddingInline: tokens.grid[2], borderRadius: tokens.radius.medium, backgroundColor: { hover: tokens.colour.selection, pressed: tokens.colour.quaternaryLabel, focused: tokens.colour.selection } },
  pressable: { cursor: "pointer" },
  after: { paddingStart: 0 },
  settling: { transitionProperty: ["translateX", "width"], transition: tokens.motion.settle },
  words: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  // On a computer the grey words follow the heading on its line, as Raycast's subtitle does.
  flatWords: { flexDirection: "row", alignItems: "baseline", gap: tokens.grid[2] },
  heading: { font: tokens.type.medium, fontWeight: tokens.weight.medium, lineClamp: 1 },
  flatHeading: { flexShrink: 0, maxWidth: "70%", fontWeight: tokens.weight.regular },
  text: { font: tokens.type.medium, color: tokens.colour.secondaryLabel, lineClamp: 2 },
  // Under its heading in grouped cards and a sidebar, as Raycast's Settings draws a row's subtitle.
  groupedHeading: { fontWeight: tokens.weight.regular },
  groupedSize: { font: tokens.type.groupedRow },
  sidebarSize: { font: tokens.type.sidebarRow },
  groupedText: { font: tokens.type.caption },
  destructive: { color: tokens.colour.red },
  flatText: { flexShrink: 1, minWidth: 0, lineClamp: 1 },
  start: { flexShrink: 0 },
  end: { flexShrink: 0, maxWidth: "50%", color: tokens.colour.secondaryLabel, textAlign: "end" },
  control: { flexShrink: 0, minWidth: tokens.length.control, justifyContent: "center", alignItems: "center", paddingStart: tokens.grid[1] },
  action: { flexShrink: 0, flexDirection: "row", alignItems: "center", marginStart: "auto", paddingBottom: tokens.grid[2], paddingEnd: tokens.grid[3] },
  swipe: { position: "absolute", top: 0, bottom: 0, end: 0, alignItems: "center", justifyContent: "center", overflow: "hidden", backgroundColor: tokens.colour.red, color: tokens.colour.onAccent, cursor: "pointer" },
  chevron: { flexShrink: 0, color: tokens.colour.tertiaryLabel },
  // A row coming or going: it opens up where it goes, or closes up, fading as it does.
  // How tall a part that isn't drawn is, held as it was.
  tall: (height: number) => ({ height }),
  moved: (translateX: number) => ({ translateX }),
  swiped: (width: number) => ({ width }),
  slot: { overflow: "hidden", transitionProperty: ["height", "opacity"], transition: tokens.motion.settle },
  entering: { opacity: { default: 1, entering: 0 } },
  leaving: { opacity: 0, transitionProperty: ["height", "opacity"], transition: tokens.motion.settle },
})

/** How long a row takes to open up or close up, in milliseconds: the settle pace's (tokens.ts). */
const settle = duration(tokens.motion.settle)

/** A row as the list draws it: coming (its height opening up from none), there, or going (closing up), and the
 * height it's held at while it does. */
type Slot = {
  node: JSX.Element
  state: () => State
  setState: (state: State) => void
  to: () => number | undefined
  setTo: (to: number | undefined) => void
  height?: number
  /** Whether its row is drawn, and whether it's waiting to be (`List`). */
  drawn: () => boolean
  setDrawn: (drawn: boolean) => void
  waiting?: boolean
}
type State = "coming" | "there" | "going"

const slot = (node: JSX.Element, state: State, to?: number): Slot => {
  const [now, setState] = createSignal(state)
  const [held, setTo] = createSignal(to)
  const [drawn, setDrawn] = createSignal(false)
  return { node, state: now, setState, to: held, setTo, drawn, setDrawn }
}

/// `<ListStyle value="insetGrouped">…</ListStyle>`: how the lists in it are drawn, as SwiftUI's `listStyle` is. `plain`,
/// the usual, is Raycast's rows on a computer and a card of rows under a finger. `insetGrouped` is cards of rows on
/// every device, each row its heading over its grey line, and one that opens with a chevron, as Raycast's Settings and
/// iOS's Settings draw them. `sidebar` is a window's sidebar: rows on the window with their icons, the one open beside
/// it filled, as macOS's and Raycast's Settings' sidebars; under a finger, as `insetGrouped`. A screen whose lists
/// aren't plain has no action bar, as Raycast's Settings window has none.
export const ListStyle = createContext<"plain" | "insetGrouped" | "sidebar">("plain")

/** Whether a row is an option of a listbox, which its list's slot is for it. */
const InListbox = createContext(false)

/** What a list tells each row in it: whether it's drawn, by the node the row is, as the part of the list it's in comes
 * near the window and goes from it. */
type Drawing = Map<unknown, (drawn: boolean) => void>
const Drawing = createContext<Drawing>()

/** How many rows a list draws or doesn't together, how many of those coming near the window it draws in a frame and
 * for how long at most (at least one), and how tall a row not drawn yet is held, until one is: a row's on a computer. */
const CHUNK = 8
const PER_FRAME = 3
const BUDGET = 4
const ROW = 38

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
const insides = new WeakMap<Listbox, Inside>()

let listboxes = 0

/// `const results = createListbox()`, then `<Search {...results.field()} onKeyDown={results.keys} … />` and
/// `<List listbox={results} label="Results">…rows…</List>`: a list of rows to choose from while what's typed in keeps
/// the keyboard, as a search's results are (WAI-ARIA's combobox and listbox): ↑ ↓, Home and End in the field choose a
/// row, Return calls `onOpen` with the one chosen, the pointer over a row chooses it (a press opens it, as its
/// `onClick` says), and a screen reader hears which. `results.chosen()` is the
/// one chosen, first unless it's moved; `results.choose(0)` goes back to the first, as a search does once it's typed in.
export function createListbox({ onOpen }: { onOpen?: (index: number) => void } = {}): Listbox {
  const id = `listbox-${++listboxes}`
  const [chosen, setChosen] = createSignal(0)
  const [count, setCount] = createSignal(0)
  const option = (index: number) => `${id}-${index}`
  const choose = (index: number) => setChosen(Math.max(0, Math.min(count() - 1, index)))
  const keys = (event: Keyed) => {
    if (event.altKey || event.metaKey || event.ctrlKey || !count()) return false
    const to = { ArrowDown: chosen() + 1, ArrowUp: chosen() - 1, Home: 0, End: count() - 1 }[event.key]
    if (to !== undefined) choose(to)
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

/// `<List><For each={notes()}>{(note) => <Row heading={note.text} />}</For></List>`: things of one kind, one under
/// another, in one group that stops at a readable width however wide the room is. With no rows it shows nothing. A
/// row added to a list that has some opens up where it goes, and one taken out closes up, the rest making room or
/// closing the gap, at the settle pace; for whoever asked for less motion a new row fades in and one taken out just
/// goes. A list that changes in any other way (filled, a row changed for another) just shows what it's given. A long
/// list draws only its rows near the window, as React Native's FlatList and UIKit's table views do: 16 at a time, as
/// they come near it, the rest held at the height they were, or a row's, until they do.
export function List(props: { listbox?: Listbox; label?: string; children?: JSX.Element }) {
  const style = useContext(ListStyle)
  // On a computer a plain list and a sidebar are rows on the window; grouped cards are cards there too.
  const flat = () => !touch() && style !== "insetGrouped"
  const grouped = () => !touch() && style === "insetGrouped"
  const drawing: Drawing = new Map()
  const rows = children(() => (
    <Drawing value={drawing}>
      <InListbox value={!!props.listbox}>{props.children}</InListbox>
    </Drawing>
  ))
  const [slots, setSlots] = createSignal<Slot[]>([])
  createEffect(() => slots().filter((one) => one.state() !== "going").length, (count) => void (props.listbox && insides.get(props.listbox)!.rows(count)))
  createEffect(
    () => rows.toArray(),
    (next) => {
      const before = slots().filter((one) => one.state() !== "going")
      const kept = new Set(next)
      const gone = before.filter((one) => !kept.has(one.node))
      const had = new Set(before.map((one) => one.node))
      const come = next.filter((node) => !had.has(node))
      // First filled, or some gone and some come (a row changed, another list's rows): a plain swap.
      const plain = !before.length || (gone.length > 0 && come.length > 0)
      const still = reducedMotion()
      const was = new Map(before.map((one) => [one.node, one]))
      const made: Slot[] = next.map((node) => was.get(node) ?? slot(node, plain ? "there" : "coming", plain || still ? undefined : 0))
      // What's going keeps its place, before the row that followed it, until it's closed up.
      if (!plain && !still)
        for (const one of gone) {
          const after = before.slice(before.indexOf(one) + 1).find((other) => kept.has(other.node))
          const at = after ? made.findIndex((other) => other.node === after.node) : made.length
          one.setState("going")
          one.setTo(one.height)
          made.splice(at, 0, one)
          requestAnimationFrame(() => requestAnimationFrame(() => one.setTo(0)))
          setTimeout(() => setSlots((now) => now.filter((other) => other !== one)), settle)
        }
      setSlots(made)
    },
  )
  /// A row's height as it's laid out: a coming one opens up to it, then is as tall as it is.
  const laid = (one: Slot, height: number) => {
    if (!one.drawn()) return
    if (one.state() === "coming" && one.height === undefined) {
      requestAnimationFrame(() => requestAnimationFrame(() => one.setTo(height)))
      setTimeout(() => (one.setState("there"), one.setTo(undefined)), settle)
    }
    if (one.state() !== "going") one.height = height
    tall = height
  }
  let tall = ROW
  /// Draws a row, or puts it away. What comes near the window as it's scrolled is drawn for a few milliseconds a
  /// frame, those nearest it first, as React Native's FlatList draws in batches and React's scheduler in slices, so a
  /// frame stays within its time; it's near the window half a window ahead, so it's drawn before it shows.
  const draw = (one: Slot, now: boolean) => {
    one.waiting = false
    one.setDrawn(now)
    drawing.get(one.node)?.(now)
  }
  let waiting: Slot[] = []
  const next = () => {
    const until = performance.now() + BUDGET
    waiting = waiting.filter((one) => one.waiting)
    // Each drawn as it's set (`flush`), so the time it took is what's counted.
    for (let drawn = 0; waiting.length && drawn < PER_FRAME && (!drawn || performance.now() < until); drawn++) {
      draw(waiting.shift()!, true)
      flush()
    }
    if (waiting.length) requestAnimationFrame(next)
  }
  const wait = (ones: Slot[]) => {
    if (!ones.length) return
    if (!waiting.length) requestAnimationFrame(next)
    for (const one of ones) one.waiting = true
    waiting.push(...ones)
  }
  /// Where each part of the list drawn starts, so one coming near above them is drawn from its foot up.
  const shown = new Set<number>()
  /// Each part of the list, by where it starts: drawn as it comes near the window (those that fill it at once), and
  /// held at its height while it's far from it; the one chosen in a listbox stays drawn.
  const chunks = createMemo(() => Array.from({ length: Math.ceil(slots().length / CHUNK) }, (_, at) => at * CHUNK))
  const Chunk = (chunk: { start: number }) => {
    const mine = () => slots().slice(chunk.start, chunk.start + CHUNK)
    const [near, setNear] = createSignal(chunk.start < Math.ceil(viewport().height / 32))
    const chosen = () => !!props.listbox && Math.floor(props.listbox.chosen() / CHUNK) * CHUNK === chunk.start
    const drawn = () => near() || chosen()
    // What fills the window as the list fills, and a row added to a part that's drawn, is drawn at once; a part
    // that comes near as it's scrolled, a few rows a frame. What isn't a Row of the UI library's is drawn as it comes.
    let was: boolean | undefined
    createEffect(
      () => [drawn(), mine()] as const,
      ([now, ones]) => {
        const scrolled = now && was === false && !chosen()
        was = now
        if (!now) {
          shown.delete(chunk.start)
          return ones.forEach((one) => draw(one, false))
        }
        const coming = ones.filter((one) => !one.drawn() && !one.waiting)
        const later = coming.filter((one) => scrolled && drawing.has(one.node))
        coming.filter((one) => !later.includes(one)).forEach((one) => draw(one, true))
        wait([...shown].some((start) => start > chunk.start) ? later.reverse() : later)
        shown.add(chunk.start)
      },
    )
    return (
      <Box onIntersection={(event) => setNear(event.isIntersecting)} style={[!drawn() && look.tall(mine().reduce((sum, one) => sum + (one.height ?? tall), 0))]}>
        <Show when={drawn()}>
          <For each={mine()}>{(one, at) => slotted(one, () => chunk.start + at())}</For>
        </Show>
      </Box>
    )
  }
  const slotted = (one: Slot, at: () => number) => (
              <Box style={[look.slot, one.state() === "coming" && look.entering, one.state() === "going" && look.leaving, one.to() !== undefined && look.tall(one.to()!), !one.drawn() && look.tall(one.height ?? tall)]}>
                <Show
                  when={props.listbox}
                  fallback={<Box onLayout={(event) => laid(one, event.currentTarget.getBoundingClientRect().height)}>{one.node}</Box>}
                >
                  {(listbox) => (
                    <Box
                      role="option"
                      id={insides.get(listbox())!.option(at())}
                      aria-selected={listbox().chosen() === at() ? "true" : "false"}
                      onPointerEnter={() => listbox().choose(at())}
                      onLayout={(event) => laid(one, event.currentTarget.getBoundingClientRect().height)}
                      style={[look.option, listbox().chosen() === at() && look.chosen]}
                    >
                      {one.node}
                    </Box>
                  )}
                </Show>
              </Box>
  )
  return (
    <Show when={slots().length}>
      <Box role={props.listbox ? "listbox" : "list"} id={props.listbox?.id} aria-label={props.label} style={[look.list, flat() && look.flat, grouped() && look.groupedList]}>
        <Box style={[look.rows, flat() && look.flatRows]}>
          <For each={chunks()}>{(start) => <Chunk start={start} />}</For>
        </Box>
      </Box>
    </Show>
  )
}

/// How far a finger moves before it's a swipe or a scroll, not a press.
const SLOP = 10
/// How long a press is held before it opens a row's menu.
const HOLD = 500
/// How far aside a row stays to show its destructive item.
const SHOWN = 88

/// `<Row heading="Remove Device…" destructive onClick={remove} />` is a row that does something destructive, its
/// heading red, as iOS's Settings has one at the foot of a page.
/// `<Row heading="Coffee with Sam" text="Meals · 12 Nov" />`: one thing in a list: its `heading`, up to two grey lines
/// of `text`, what it starts with (`start`, an icon) and what it ends with (`end`: an amount, a switch). With
/// `onClick` the whole row is pressed and opens in place. `current` marks the one that's open beside the list, in a
/// split. `control` is a control before all of it, pressed apart from the row, as a checkbox that picks rows is, and
/// `action` a button after all of it, pressed apart from the row, as Sign In is for an account that needs it; an action
/// that would leave the words narrower than a menu goes under them. A ⋯ `Menu` in its `end` opens on a long press or a
/// right-click on the row too, and its `destructive` item (Delete, Remove) shows as a finger swipes the row toward
/// its start, as a phone's lists do: pressed, or swiped most of the way, it's done.
export function Row(props: RowProps): JSX.Element {
  // In a list, its part of the list draws it as it comes near the window (`List`); until then, and once it's far from
  // it again, it's an empty box, made in a moment however long the list. The one open beside the list is always drawn.
  const drawing = useContext(Drawing)
  if (!drawing) return <Drawn {...props} />
  const [drawn, setDrawn] = createSignal(false)
  // The node it is, as its list has it, by which the list draws it.
  const [row] = children(() => (
    <Box>
      <Show when={drawn() || props.current}>
        <Drawn {...props} />
      </Show>
    </Box>
  )).toArray()
  drawing.set(row, setDrawn)
  onCleanup(() => drawing.delete(row))
  return row
}

type RowProps = {
  heading?: JSX.Element
  text?: JSX.Element
  start?: JSX.Element
  end?: JSX.Element
  onClick?: (event?: Located) => void
  current?: boolean
  control?: JSX.Element
  action?: JSX.Element
  destructive?: boolean
}

/// A row, drawn.
function Drawn(props: RowProps) {
  const [menu, setMenu] = createSignal<Menu>()
  // How far aside the row is, and whether it's on its way somewhere by itself.
  const [swiped, setSwiped] = createSignal(0)
  const [settling, setSettling] = createSignal(false)
  let width = 0
  let start: { x: number; y: number; at: number } | undefined
  let swiping = false
  let moves: { time: number; value: number }[] = []
  // When a press last swiped the row, held it or put it back: the click its lifting makes isn't a press on the row.
  let moved = -Infinity
  let held: ReturnType<typeof setTimeout> | undefined
  onCleanup(() => clearTimeout(held))
  const aside = (px: number, settle = false) => (setSettling(settle), setSwiped(Math.max(0, Math.min(px, width || px))))
  const at = (event: Located) => ({ x: event.clientX, y: event.clientY, width: 0, height: 0 })

  const down = (event: Pressed) => {
    if (!menu() || event.button !== 0) return
    width = event.currentTarget.getBoundingClientRect().width
    if (swiped()) return void ((moved = event.timeStamp), aside(0, true))
    start = { x: event.clientX, y: event.clientY, at: swiped() }
    swiping = false
    moves = []
    held = setTimeout(() => ((moved = Infinity), (start = undefined), menu()?.open(at(event))), HOLD)
  }
  const move = (event: Pressed) => {
    if (!start) return
    const dx = (start.x - event.clientX) * (direction() === "rtl" ? -1 : 1)
    const dy = event.clientY - start.y
    if (!swiping && Math.hypot(dx, dy) < SLOP) return
    clearTimeout(held)
    if (!swiping) {
      swiping = event.pointerType === "touch" && !!menu()?.destructive() && Math.abs(dx) > Math.abs(dy)
      if (!swiping) return void (start = undefined)
    }
    aside(start.at + dx)
    moves = [...moves.slice(-31), { time: event.timeStamp, value: swiped() }]
  }
  const up = (event: Pressed) => {
    clearTimeout(held)
    if (moved === Infinity) moved = event.timeStamp
    if (!start) return
    start = undefined
    if (!swiping) return
    moved = event.timeStamp
    const item = menu()?.destructive()
    if (item && swiped() > width * 0.6) {
      aside(width, true)
      menu()?.select(item.value)
      setTimeout(() => aside(0, true), 400)
      return
    }
    // Let go, it goes where it was heading: its speed as it was let go, carried on as a phone's scrolling slows.
    const last = moves.at(-1)
    const from = moves.findLast((move) => last!.time - move.time >= 80) ?? moves[0]
    const speed = !last || event.timeStamp - last.time > 80 ? 0 : (last.value - from.value) / Math.max(1, last.time - from.time)
    aside(swiped() + (speed * 0.998) / (1 - 0.998) > SHOWN / 2 ? SHOWN : 0, true)
  }
  const cancel = () => {
    clearTimeout(held)
    if (start && swiping) aside(0, true)
    start = undefined
  }
  // What a press that was a swipe, a held press, or put the row back would otherwise do as it lifts: open the row.
  const pressed = (event: Located) => event.timeStamp - moved >= 600 && props.onClick?.(event)

  // On a computer, what it can do is the screen's action panel's while it's chosen: under the pointer, as Raycast's
  // rows are, or open beside the list. Open, then its menu's items.
  // In a listbox its list's slot is the option, which says so.
  const inListbox = useContext(InListbox)
  const choose = useContext(Choose)
  const actions = (): Action[] => [
    ...(props.onClick ? [{ label: messages.open, onAction: () => props.onClick!() }] : []),
    ...(menu()?.items() ?? []).map((item) => ({ label: item.label, destructive: item.destructive, disabled: item.disabled, onAction: () => menu()!.select(item.value) })),
  ]
  const chosen = () => choose?.set(actions, actions().length ? actions : undefined)
  if (choose) {
    createEffect(() => !!props.current, (now) => void (now && chosen()))
    onCleanup(() => choose.clear(actions))
  }

  const style = useContext(ListStyle)
  // A plain list's rows on a computer are Raycast's, the grey words beside the heading; elsewhere they're under it.
  const flat = () => !touch() && style === "plain"
  const grouped = () => !touch() && style === "insetGrouped"
  const sidebar = () => !touch() && style === "sidebar"
  // A row that opens something says so with a chevron, as iOS's Settings does, but not in a computer's sidebar, whose
  // rows open beside it, or a plain list's.
  const opens = () => !!props.onClick && style !== "plain" && (touch() || style === "insetGrouped") && !props.destructive
  const inside = () => (
    <>
      <Show when={props.start}>
        <Box style={look.start}>{props.start}</Box>
      </Show>
      <Box style={[look.words, flat() && look.flatWords]}>
        <Text style={[look.heading, flat() && look.flatHeading, style !== "plain" && look.groupedHeading, grouped() && look.groupedSize, sidebar() && look.sidebarSize, props.destructive && look.destructive]}>{props.heading}</Text>
        <Show when={props.text}>
          <Text style={[look.text, flat() && look.flatText, style !== "plain" && !touch() && look.groupedText]}>{props.text}</Text>
        </Show>
      </Box>
      <Show when={props.end}>
        <Box style={look.end}>{props.end}</Box>
      </Show>
      <Show when={opens()}>
        <Box style={look.chevron}>
          <Icon name="caret-right" />
        </Box>
      </Show>
    </>
  )
  const moving = () => [settling() && look.settling, swiped() > 0 && look.moved(direction() === "rtl" ? swiped() : -swiped())]
  return (
    <RowMenu value={setMenu}>
      <Box
        role={inListbox ? undefined : "listitem"}
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={cancel}
        onContextMenu={(event) => menu()?.open(at(event))}
        onPointerEnter={chosen}
        style={[look.row, !touch() && !grouped() && look.flatRow, !!props.action && look.wraps, props.current && look.current, !!menu() && look.held]}
      >
        <Show when={props.control}>
          <Box style={[look.control, moving()]}>{props.control}</Box>
        </Show>
        <Show
          when={props.onClick}
          fallback={<Box style={[look.inside, !touch() && look.flatInside, grouped() && look.groupedInside, sidebar() && look.sidebarInside, !!props.control && look.after, moving()]}>{inside()}</Box>}
        >
          <Box role={inListbox ? undefined : "button"} aria-current={props.current ? "true" : undefined} onClick={pressed} style={[look.inside, !touch() && look.flatInside, grouped() && look.groupedInside, sidebar() && look.sidebarInside, look.pressable, inListbox && look.quiet, !!props.control && look.after, moving()]}>
            {inside()}
          </Box>
        </Show>
        <Show when={props.action}>
          <Box style={[look.action, moving()]}>{props.action}</Box>
        </Show>
        <Show when={menu()?.destructive()}>
          {(item) => (
            <Box aria-hidden="true" onClick={() => (menu()?.select(item().value), aside(0, true))} style={[look.swipe, settling() && look.settling, look.swiped(swiped())]}>
              <Text>{item().label}</Text>
            </Box>
          )}
        </Show>
      </Box>
    </RowMenu>
  )
}
