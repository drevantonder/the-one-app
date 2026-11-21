// A list and its rows: written once on the drawing primitives, the same on every host.
import { children, createContext, createEffect, createSignal, For, onCleanup, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Pressable, Text, textOf, View, VirtualizedList } from "#primitives"
import { direction, reducedMotion, touch } from "#bridge"
import { Icon } from "./button.tsx"
import { RowMenu, type RowMenu as Menu } from "./menu.tsx"
import { Choose } from "./collect.ts"
import { Room } from "./screen.tsx"
import { messages } from "../words.ts"
import type { Action } from "./actions.tsx"
import { negative, styles } from "../styles.ts"
import { duration, tokens } from "../tokens.ts"
import type { Located } from "../types/ui.d.ts"
import { createHold, together } from "../behavior-primitives/drag.ts"
import { insides, type Listbox } from "../headless-components/listbox.ts"
import { createList } from "../headless-components/list.ts"
import { createSwipeActions } from "../behavior-primitives/swipe-actions.ts"

/** How wide a list's room is for a row's grey words to fit beside its heading, in CSS pixels: a sheet's middle. */
const ROOMY = 640

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
  },
  flatInside: { paddingBlock: 0, paddingInline: tokens.grid[2], borderRadius: tokens.radius.medium },
  // Its grey words under its heading, a row has room above and below, as iOS's subtitle cells do.
  stackedInside: { paddingBlock: tokens.grid[2] },
  // Only a row that does something when pressed lights up under the pointer, as it's pressed and as it's focused.
  pressable: {
    cursor: "pointer",
    backgroundColor: { hover: tokens.colour.quinaryLabel, pressed: tokens.colour.quaternaryLabel, focused: tokens.colour.quinaryLabel },
    transitionProperty: ["backgroundColor"],
    transition: tokens.motion.quick,
  },
  flatPressable: { backgroundColor: { hover: tokens.colour.selection, pressed: tokens.colour.quaternaryLabel, focused: tokens.colour.selection } },
  // The row open beside its list is filled already, so the keyboard on it is the focus ring.
  openFocused: { boxShadow: { focused: tokens.shadow.focus } },
  after: { paddingStart: 0 },
  // The words have the row's whole width, which an action then wraps under, as a phone's list gives them it.
  full: { flexBasis: "100%" },
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
  // With its grey words under it, the heading has the row's width and two lines.
  underHeading: { flexShrink: 1, maxWidth: "100%", lineClamp: 2 },
  start: { flexShrink: 0 },
  end: { flexShrink: 0, maxWidth: "50%", color: tokens.colour.secondaryLabel, textAlign: "end" },
  control: { flexShrink: 0, minWidth: tokens.length.control, justifyContent: "center", alignItems: "center", paddingStart: tokens.grid[1] },
  action: { flexShrink: 0, flexDirection: "row", alignItems: "center", marginStart: "auto", paddingEnd: tokens.grid[3] },
  // In a narrow room it may wrap under the words, where the row has room under it as under any line.
  wrapped: { paddingBottom: tokens.grid[2] },
  swipe: { position: "absolute", top: 0, bottom: 0, end: 0, alignItems: "center", justifyContent: "center", overflow: "hidden", backgroundColor: tokens.colour.red, color: tokens.colour.onAccent, cursor: "pointer" },
  chevron: { flexShrink: 0, color: tokens.colour.tertiaryLabel },
  // A row coming or going: it opens up where it goes, or closes up, fading as it does.
  // How tall a row coming or going is held, as it opens up or closes up.
  tall: (height: number) => ({ height }),
  moved: (translateX: number) => ({ translateX }),
  swiped: (width: number) => ({ width }),
  slot: { overflow: "hidden", transitionProperty: ["height", "opacity"], transition: tokens.motion.settle },
  entering: { opacity: { default: 1, entering: 0 } },
  leaving: { opacity: 0, transitionProperty: ["height", "opacity"], transition: tokens.motion.settle },
})

/** How long a row takes to open up or close up, in milliseconds: the settle pace's (tokens.ts). */
const settle = duration(tokens.motion.settle)

/** A row as the list draws it: what it shows (a row's node, or an item), coming (its height opening up from none),
 * there, or going (closing up), and the height it's held at while it does. */
type Slot<X> = {
  value: () => X
  setValue: (value: X) => void
  key: unknown
  state: () => State
  setState: (state: State) => void
  to: () => number | undefined
  setTo: (to: number | undefined) => void
  height?: number
}
type State = "coming" | "there" | "going"

const slot = <X,>(value: X, key: unknown, state: State, to?: number): Slot<X> => {
  const [now, setState] = createSignal(state)
  const [held, setTo] = createSignal(to)
  const [shown, setShown] = createSignal({ value })
  return { value: () => shown().value, setValue: (value) => void setShown({ value }), key, state: now, setState, to: held, setTo }
}

/** The rows of `values`, each by its `key`, as they come and go: a row added to a list that has some opens up where it
 * goes, and one taken out closes up where it was, at the settle pace (one that's taken out keeps its place, before the
 * row that followed it, until it's closed up); a list first filled, or changed in any other way, is a plain swap.
 * Whoever asked for less motion sees a new row fade in, and one taken out just go. */
function createSlots<X>(values: () => readonly X[], key: (value: X) => unknown) {
  const [slots, setSlots] = createSignal<Slot<X>[]>([])
  createEffect(
    () => [...values()],
    (next) => {
      const before = slots().filter((one) => one.state() !== "going")
      const keys = next.map(key)
      const kept = new Set(keys)
      const gone = before.filter((one) => !kept.has(one.key))
      const had = new Set(before.map((one) => one.key))
      const come = keys.filter((one) => !had.has(one))
      // First filled, or some gone and some come (a row changed, another list's rows): a plain swap.
      const plain = !before.length || (gone.length > 0 && come.length > 0)
      const still = reducedMotion()
      const was = new Map(before.map((one) => [one.key, one]))
      const made: Slot<X>[] = next.map((value, at) => {
        const kept = was.get(keys[at])
        if (!kept) return slot(value, keys[at], plain ? "there" : "coming", plain || still ? undefined : 0)
        kept.setValue(value)
        return kept
      })
      if (!plain && !still)
        for (const one of gone) {
          const after = before.slice(before.indexOf(one) + 1).find((other) => kept.has(other.key))
          const at = after ? made.findIndex((other) => other.key === after.key) : made.length
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
  const laid = (one: Slot<X>, height: number) => {
    if (one.state() === "coming" && one.height === undefined) {
      requestAnimationFrame(() => requestAnimationFrame(() => one.setTo(height)))
      setTimeout(() => (one.setState("there"), one.setTo(undefined)), settle)
    }
    if (one.state() !== "going") one.height = height
  }
  return { slots, laid }
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

/** The keyboard of the lists in it, one tab stop for all their rows (`ListKeys`). */
const Scope = createContext<ReturnType<typeof createList> | null>(null)
/** The keyboard of the list a row is in, and the list. */
const Keys = createContext<{ keys: ReturnType<typeof createList>; list: object } | null>(null)
/** Where a long list's row is in it, which a native host places by that rather than its order in the tree. */
const At = createContext<(() => number) | null>(null)

/** `<ListKeys>…</ListKeys>`: the lists in it are one to the keyboard, their rows one tab stop, Up and Down going on
 * from one list to the next, as a Mac's sidebar's sections are one list. A `Split`'s list side is. */
export function ListKeys(props: { children?: JSX.Element }) {
  const style = useContext(ListStyle)
  return <Scope value={createList({ opens: () => !touch() && style === "sidebar" })}>{props.children}</Scope>
}

/// `<List><Row heading="Notes" /><Row heading="Reminders" /></List>`: things of one kind, one under another, in one
/// group that stops at a readable width however wide the room is. With no rows it shows nothing.
/// `<List each={notes()} keyed={(note) => note.id}>{(note) => <Row heading={note().text} />}</List>`: a row for each
/// item, as `For` draws them, drawn only as it comes near the window, as UIKit's and Android's lists and FlashList
/// draw them (`VirtualizedList`), so 10,000 are as quick as 20. A row added to a list that has some opens up where it
/// goes, and one taken out closes up, the rest making room or closing the gap, at the settle pace; for whoever asked
/// for less motion a new row fades in and one taken out just goes. A list that changes in any other way (filled, a
/// row changed for another) just shows what it's given. Its rows are its items, by `keyed` (as `For`'s), by
/// themselves where it says none, and each is given its item as an accessor, `(note) => <Row heading={note().text} />`,
/// as a row going out of sight is reused for one coming into it, as RecyclerView's are. On a computer its rows are one
/// tab stop: Up and Down move between them, Home and End to its ends, and letters typed to the next that starts with
/// them, as a Mac's and Windows' lists move, and in a `sidebar` the row the keyboard comes to opens beside it.
export function List<T>(props: { listbox?: Listbox; label?: string; each?: readonly T[]; keyed?: (item: T) => unknown; children?: JSX.Element | ((item: () => T, index: () => number) => JSX.Element) }) {
  const style = useContext(ListStyle)
  // On a computer a plain list and a sidebar are rows on the window; grouped cards are cards there too.
  const flat = () => !touch() && style !== "insetGrouped"
  const grouped = () => !touch() && style === "insetGrouped"
  const each = props.each !== undefined
  // A listbox's keyboard is its own (the field it's for); every other list's is its rows', with the lists beside it.
  const scope = useContext(Scope)
  const keys = props.listbox ? null : { keys: scope ?? createList({ opens: () => !touch() && style === "sidebar" }), list: {} }
  const rows = each ? undefined : children(() => <Keys value={keys}><InListbox value={!!props.listbox}>{props.children as JSX.Element}</InListbox></Keys>)
  const values = (): readonly unknown[] => (each ? (props.each ?? []) : rows!.toArray())
  const { slots, laid } = createSlots(values, (value) => (each && props.keyed ? props.keyed(value as T) : value))
  // Home and End reach a long list's real ends, which its rows are placed by their index in.
  if (each && keys) keys.keys.sized(keys.list, () => slots().length)
  createEffect(() => slots().filter((one) => one.state() !== "going").length, (count) => void (props.listbox && insides.get(props.listbox)!.rows(count)))
  /// A row, held as it opens up or closes up, and as a listbox's option, by where it is in the list. In a long list a
  /// row is reused for another as it goes out of sight (`VirtualizedList`), so what it shows is its slot as that changes
  /// (`one`), and `each`'s rows are given their item as an accessor, as `For`'s are by a key.
  const slotted = (one: () => Slot<unknown>, at: () => number) => {
    const node = each ? <At value={at}>{(props.children as (item: () => T, index: () => number) => JSX.Element)(() => one().value() as T, at)}</At> : (one().value() as JSX.Element)
    // How tall it was laid out, which a slot it's given as it's reused is told too: its height didn't change, so no
    // layout says so.
    let height: number | undefined
    const measured = (event: { currentTarget: { getBoundingClientRect(): { height: number } } }) => laid(one(), (height = event.currentTarget.getBoundingClientRect().height))
    createEffect(one, (slot) => void (height !== undefined && laid(slot, height)))
    return (
      <View style={[look.slot, one().state() === "coming" && look.entering, one().state() === "going" && look.leaving, one().to() !== undefined && look.tall(one().to()!)]}>
        <Show when={props.listbox} fallback={<View onLayout={measured}>{node}</View>}>
          {(listbox) => (
            <View
              role="option"
              id={insides.get(listbox())!.option(at())}
              aria-selected={listbox().chosen() === at() ? "true" : "false"}
              onPointerEnter={() => listbox().choose(at())}
              onLayout={measured}
              style={[look.option, listbox().chosen() === at() && look.chosen]}
            >
              {node}
            </View>
          )}
        </Show>
      </View>
    )
  }
  return (
    <Show when={slots().length}>
      <View role={props.listbox ? "listbox" : "list"} id={props.listbox?.id} aria-label={props.label} onKeyDown={keys?.keys.keydown} style={[look.list, flat() && look.flat, grouped() && look.groupedList]}>
        <Show
          when={each}
          fallback={
            <View style={[look.rows, flat() && look.flatRows]}>
              <For each={slots()}>{(one, at) => slotted(() => one, at)}</For>
            </View>
          }
        >
          <Keys value={keys}>
            <InListbox value={!!props.listbox}>
              <VirtualizedList style={[look.rows, flat() && look.flatRows]} data={slots()} keep={keys?.keys.keeping(keys.list)} renderItem={(row) => slotted(() => row.item, () => row.index)} />
            </InListbox>
          </Keys>
        </Show>
      </View>
    </Show>
  )
}

/// How far aside a row stays to show its destructive item.
const SHOWN = 88

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

/// `<Row heading="Remove Device…" destructive onPress={remove} />` is a row that does something destructive, its
/// heading red, as iOS's Settings has one at the foot of a page.
/// `<Row heading="Coffee with Sam" text="Meals · 12 Nov" />`: one thing in a list: its `heading`, up to two grey lines
/// of `text`, what it starts with (`start`, an icon) and what it ends with (`end`: an amount, a switch). With
/// `onPress` the whole row is pressed and opens in place. `current` marks the one that's open beside the list, in a
/// split. `control` is a control before all of it, pressed apart from the row, as a checkbox that picks rows is, and
/// `action` a button after all of it, pressed apart from the row, as Sign In is for an account that needs it; an action
/// goes under the words under a finger in a narrow room, as a phone's rows keep their words wide, and elsewhere where it
/// would leave them narrower than a menu. A ⋯ `Menu` in its `end` opens on a long press or a
/// right-click on the row too, and its `destructive` item (Delete, Remove) shows as a finger swipes the row toward
/// its start, as a phone's lists do: pressed, or swiped most of the way, it's done.
export function Row(props: RowProps): JSX.Element {
  const [menu, setMenu] = createSignal<Menu>()
  // The heading is drawn once, and its words are the row's name, which letters typed find it by.
  const heading = children(() => props.heading)
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
  const room = useContext(Room)
  // A plain list's rows on a computer are Raycast's, and where there's room, the grey words beside the heading, as
  // Raycast's subtitle is; elsewhere they're under it, wrapping to a second line before they're cut, as iOS's
  // subtitle cells are, so what they say (a state, who, why) isn't what's cut.
  const flat = () => !touch() && style === "plain"
  const beside = () => flat() && room() >= ROOMY
  const grouped = () => !touch() && style === "insetGrouped"
  const sidebar = () => !touch() && style === "sidebar"
  // Under a finger in a narrow room an action goes under the words, which keep the row's width, never beside them.
  const full = () => !!props.action && touch() && room() < ROOMY
  // A row that opens something says so with a chevron, as iOS's Settings does, but not in a computer's sidebar, whose
  // rows open beside it, or a plain list's.
  const opens = () => !!props.onPress && style !== "plain" && (touch() || style === "insetGrouped") && !props.destructive
  const inside = () => (
    <>
      <Show when={props.start}>{(start) => <View style={look.start}>{start()}</View>}</Show>
      <View style={[look.words, beside() && look.flatWords]}>
        <Text style={[look.heading, flat() && look.flatHeading, !beside() && look.underHeading, style !== "plain" && look.groupedHeading, grouped() && look.groupedSize, sidebar() && look.sidebarSize, props.destructive && look.destructive]}>{heading()}</Text>
        <Show when={props.text}>{(text) => <Text style={[look.text, beside() && look.flatText, style !== "plain" && !touch() && look.groupedText]}>{text()}</Text>}</Show>
      </View>
      <Show when={props.end}>{(end) => <View style={look.end}>{end()}</View>}</Show>
      <Show when={opens()}>
        <View style={look.chevron}>
          <Icon name="caret-right" />
        </View>
      </Show>
    </>
  )
  const moving = () => [swipe.settling() && look.settling, swipe.offset() > 0 && look.moved(direction() === "rtl" ? swipe.offset() : -swipe.offset())]
  // Its place in its list's keyboard: where it's drawn, or in a long list, where it is in it.
  const list = useContext(Keys)

  const keyboard = list?.keys.row({
    node: () => drawn(),
    list: list.list,
    at: useContext(At) ?? undefined,
    label: () => heading.toArray().map(textOf).join(""),
    pressable: () => !!props.onPress,
    current: () => !!props.current,
    open: () => props.onPress?.(),
  })
  const drawn = children(() => (
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
        <Show when={props.control}>{(control) => <View style={[look.control, moving()]}>{control()}</View>}</Show>
        <Show
          when={props.onPress}
          fallback={<View style={[look.inside, !touch() && look.flatInside, flat() && !beside() && look.stackedInside, grouped() && look.groupedInside, sidebar() && look.sidebarInside, !!props.control && look.after, full() && look.full, moving()]}>{inside()}</View>}
        >
          <Pressable
            role={inListbox ? undefined : "button"}
            aria-current={props.current ? "true" : undefined}
            tabIndex={keyboard?.tabIndex}
            autoFocus={keyboard?.autoFocus}
            onFocus={() => (keyboard?.onFocus(), chosen())}
            onBlur={keyboard?.onBlur}
            onPress={pressed}
            style={[look.inside, look.pressable, !touch() && look.flatInside, flat() && !beside() && look.stackedInside, !touch() && look.flatPressable, grouped() && look.groupedInside, sidebar() && look.sidebarInside, inListbox && look.quiet, !!props.control && look.after, props.current && look.openFocused, full() && look.full, moving()]}>
            {inside()}
          </Pressable>
        </Show>
        <Show when={props.action}>{(action) => <View style={[look.action, room() < ROOMY && look.wrapped, moving()]}>{action()}</View>}</Show>
        <Show when={menu()?.destructive()}>
          {(item) => (
            <Pressable aria-hidden="true" onPress={() => (menu()?.select(item().value), swipe.close())} style={[look.swipe, swipe.settling() && look.settling, look.swiped(swipe.offset())]}>
              <Text>{item().label}</Text>
            </Pressable>
          )}
        </Show>
      </View>
    </RowMenu>
  ))

  return drawn as unknown as JSX.Element
}
