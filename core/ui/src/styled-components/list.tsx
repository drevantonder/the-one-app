// A list and its rows: written once on the drawing primitives, the same on the DOM and on a native host.
import { children, createContext, createEffect, createMemo, createSignal, flush, For, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Pressable, Text, View } from "#primitives"
import { direction, reducedMotion, touch, viewport } from "#bridge"
import { Icon } from "./button.tsx"
import { RowMenu, type RowMenu as Menu } from "./menu.tsx"
import { Choose } from "./collect.ts"
import { messages } from "../words.ts"
import type { Action } from "./actions.tsx"
import { negative, styles } from "../styles.ts"
import { duration, tokens } from "../tokens.ts"
import type { Located } from "../types/ui.d.ts"
import { createHold, together } from "../behavior-primitives/drag.ts"
import { insides, type Listbox } from "../headless-components/listbox.ts"
import { createSwipeActions } from "../behavior-primitives/swipe-actions.ts"

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
  // In grouped cards on a computer, as Raycast's Settings: the row open is a rounded highlight inside the card, as macOS's
  // lists draw one, and the line between rows runs from the words' inset to the card's other.
  groupedList: { width: "100%" },
  rule: { position: "absolute", top: 0, start: tokens.grid[3], end: tokens.grid[3], height: tokens.length.hairline, backgroundColor: tokens.colour.quinaryLabel },
  groupedRow: { borderTopWidth: 0 },
  highlight: { position: "absolute", top: tokens.grid[0.5], bottom: tokens.grid[0.5], start: tokens.grid[1.5], end: tokens.grid[1.5], borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.selection },
  // A finger moving up or down on a row scrolls its list, as the device does it; only sideways is the row's, to swipe.
  row: { position: "relative", flexDirection: "row", alignItems: "stretch", overflow: "hidden", borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, touchAction: "pan-y" },
  flatRow: { borderTopWidth: 0, borderRadius: tokens.radius.medium },
  groupedInside: { minHeight: tokens.length.groupedRow, paddingInline: tokens.grid[3], paddingBlock: tokens.grid[1.5], gap: tokens.grid[2.5], backgroundColor: tokens.colour.transparent },
  sidebarInside: { minHeight: tokens.length.sidebarRow },
  wraps: { flexWrap: "wrap" },
  current: { backgroundColor: tokens.colour.selection },
  // A listbox's option, the one chosen highlighted as a row open beside its list is.
  option: { borderRadius: tokens.radius.medium },
  chosen: { backgroundColor: tokens.colour.selection },
  // Its row has no highlight of its own under the pointer: the pointer chooses it, which highlights the option.
  quiet: { backgroundColor: tokens.colour.transparent },
  held: { userSelect: "none" },
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
  heading: { font: tokens.type.medium, fontWeight: tokens.weight.medium, lineClamp: 2 },
  flatHeading: { flexShrink: 0, maxWidth: "70%", fontWeight: tokens.weight.regular, lineClamp: 1 },
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
      <View onIntersection={(event) => setNear(event.isIntersecting)} style={[!drawn() && look.tall(mine().reduce((sum, one) => sum + (one.height ?? tall), 0))]}>
        <Show when={drawn()}>
          <For each={mine()}>{(one, at) => slotted(one, () => chunk.start + at())}</For>
        </Show>
      </View>
    )
  }
  const slotted = (one: Slot, at: () => number) => (
              <View style={[look.slot, one.state() === "coming" && look.entering, one.state() === "going" && look.leaving, one.to() !== undefined && look.tall(one.to()!), !one.drawn() && look.tall(one.height ?? tall)]}>
                <Show
                  when={props.listbox}
                  fallback={<View onLayout={(event) => laid(one, event.currentTarget.getBoundingClientRect().height)}>{one.node}</View>}
                >
                  {(listbox) => (
                    <View
                      role="option"
                      id={insides.get(listbox())!.option(at())}
                      aria-selected={listbox().chosen() === at() ? "true" : "false"}
                      onPointerEnter={() => listbox().choose(at())}
                      onLayout={(event) => laid(one, event.currentTarget.getBoundingClientRect().height)}
                      style={[look.option, listbox().chosen() === at() && look.chosen]}
                    >
                      {one.node}
                    </View>
                  )}
                </Show>
              </View>
  )
  return (
    <Show when={slots().length}>
      <View role={props.listbox ? "listbox" : "list"} id={props.listbox?.id} aria-label={props.label} style={[look.list, flat() && look.flat, grouped() && look.groupedList]}>
        <View style={[look.rows, flat() && look.flatRows]}>
          <For each={chunks()}>{(start) => <Chunk start={start} />}</For>
        </View>
      </View>
    </Show>
  )
}

/// How far aside a row stays to show its destructive item.
const SHOWN = 88

/// `<Row heading="Remove Device…" destructive onPress={remove} />` is a row that does something destructive, its
/// heading red, as iOS's Settings has one at the foot of a page.
/// `<Row heading="Coffee with Sam" text="Meals · 12 Nov" />`: one thing in a list: its `heading`, up to two grey lines
/// of `text`, what it starts with (`start`, an icon) and what it ends with (`end`: an amount, a switch). With
/// `onPress` the whole row is pressed and opens in place. `current` marks the one that's open beside the list, in a
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
    <View>
      <Show when={drawn() || props.current}>
        <Drawn {...props} />
      </Show>
    </View>
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
  /** What pressing it does, as React Native's `onPress`. */
  onPress?: (event?: Located) => void
  current?: boolean
  control?: JSX.Element
  action?: JSX.Element
  destructive?: boolean
}

/// A row, drawn.
function Drawn(props: RowProps) {
  const [menu, setMenu] = createSignal<Menu>()
  const at = (event: Located) => ({ x: event.clientX, y: event.clientY, width: 0, height: 0 })
  // Held, its menu opens where it was held; swiped toward its start by a finger, its destructive item shows.
  const hold = createHold({ accepts: () => !!menu() && !swipe.offset(), onHold: (event) => menu()?.open(at(event)) })
  const swipe = createSwipeActions({
    room: SHOWN,
    enabled: () => !!menu()?.destructive(),
    onFullSwipe: () => {
      const item = menu()?.destructive()
      if (item) menu()?.select(item.value)
    },
  })
  // What a press that was a swipe, a held press, or put the row back would otherwise do as it lifts: open the row.
  const pressed = (event: Located) => !hold.swallows(event) && !swipe.swallows(event) && props.onPress?.(event)

  // On a computer, what it can do is the screen's action panel's while it's chosen: under the pointer, as Raycast's
  // rows are, or open beside the list. Open, then its menu's items.
  // In a listbox its list's slot is the option, which says so.
  const inListbox = useContext(InListbox)
  const choose = useContext(Choose)
  const actions = (): Action[] => [
    ...(props.onPress ? [{ label: messages.open, onAction: () => props.onPress!() }] : []),
    ...(menu()?.items() ?? []).map((item) => ({ label: item.label, destructive: item.destructive, disabled: item.disabled, menu: true, onAction: () => menu()!.select(item.value) })),
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
  const opens = () => !!props.onPress && style !== "plain" && (touch() || style === "insetGrouped") && !props.destructive
  const inside = () => (
    <>
      <Show when={props.start}>
        <View style={look.start}>{props.start}</View>
      </Show>
      <View style={[look.words, flat() && look.flatWords]}>
        <Text style={[look.heading, flat() && look.flatHeading, style !== "plain" && look.groupedHeading, grouped() && look.groupedSize, sidebar() && look.sidebarSize, props.destructive && look.destructive]}>{props.heading}</Text>
        <Show when={props.text}>
          <Text style={[look.text, flat() && look.flatText, style !== "plain" && !touch() && look.groupedText]}>{props.text}</Text>
        </Show>
      </View>
      <Show when={props.end}>
        <View style={look.end}>{props.end}</View>
      </Show>
      <Show when={opens()}>
        <View style={look.chevron}>
          <Icon name="caret-right" />
        </View>
      </Show>
    </>
  )
  const moving = () => [swipe.settling() && look.settling, swipe.offset() > 0 && look.moved(direction() === "rtl" ? swipe.offset() : -swipe.offset())]
  return (
    <RowMenu value={setMenu}>
      <View
        role={inListbox ? undefined : "listitem"}
        {...together(hold.gesture, swipe.gesture)}
        onContextMenu={(event) => menu()?.open(at(event))}
        onPointerEnter={chosen}
        style={[look.row, !touch() && !grouped() && look.flatRow, grouped() && look.groupedRow, !!props.action && look.wraps, props.current && !grouped() && look.current, !!menu() && look.held]}
      >
        <Show when={grouped()}>
          <View style={look.rule} />
          <Show when={props.current}>
            <View style={look.highlight} />
          </Show>
        </Show>
        <Show when={props.control}>
          <View style={[look.control, moving()]}>{props.control}</View>
        </Show>
        <Show
          when={props.onPress}
          fallback={<View style={[look.inside, !touch() && look.flatInside, grouped() && look.groupedInside, sidebar() && look.sidebarInside, !!props.control && look.after, moving()]}>{inside()}</View>}
        >
          <Pressable role={inListbox ? undefined : "button"} aria-current={props.current ? "true" : undefined} onPress={pressed} style={[look.inside, !touch() && look.flatInside, grouped() && look.groupedInside, sidebar() && look.sidebarInside, look.pressable, inListbox && look.quiet, !!props.control && look.after, moving()]}>
            {inside()}
          </Pressable>
        </Show>
        <Show when={props.action}>
          <View style={[look.action, moving()]}>{props.action}</View>
        </Show>
        <Show when={menu()?.destructive()}>
          {(item) => (
            <Pressable aria-hidden="true" onPress={() => (menu()?.select(item().value), swipe.close())} style={[look.swipe, swipe.settling() && look.settling, look.swiped(swipe.offset())]}>
              <Text>{item().label}</Text>
            </Pressable>
          )}
        </Show>
      </View>
    </RowMenu>
  )
}
