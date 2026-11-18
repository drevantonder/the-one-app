// A list and its rows: written once in strict, on the primitives, the same on the DOM and on a native host.
import { children, createEffect, createSignal, For, onCleanup, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Text } from "#primitives"
import { direction, reducedMotion } from "#bridge"
import { RowMenu, type RowMenu as Menu } from "./menu.tsx"
import { negative, styles } from "./styles.ts"
import { duration, tokens } from "./tokens.ts"
import type { Located, Pressed } from "./strict/types/ui.d.ts"

const look = styles({
  list: {
    maxWidth: tokens.length.readable,
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.background,
    color: tokens.colour.label,
    boxShadow: tokens.shadow.edge,
    overflow: "hidden",
  },
  // Every row draws the line above it, and the list hides the first's.
  rows: { marginTop: negative(tokens.length.hairline) },
  row: { position: "relative", flexDirection: "row", alignItems: "stretch", overflow: "hidden", borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  wraps: { flexWrap: "wrap" },
  current: { backgroundColor: tokens.colour.quinaryLabel },
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
  pressable: { cursor: "pointer" },
  after: { paddingStart: 0 },
  settling: { transitionProperty: ["translateX", "width"], transition: tokens.motion.settle },
  words: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  heading: { font: tokens.type.medium, fontWeight: tokens.weight.medium, lineClamp: 1 },
  text: { font: tokens.type.medium, color: tokens.colour.secondaryLabel, lineClamp: 2 },
  start: { flexShrink: 0 },
  end: { flexShrink: 0, maxWidth: "50%", color: tokens.colour.secondaryLabel, textAlign: "end" },
  control: { flexShrink: 0, minWidth: tokens.length.control, justifyContent: "center", alignItems: "center", paddingStart: tokens.grid[1] },
  action: { flexShrink: 0, flexDirection: "row", alignItems: "center", marginStart: "auto", paddingBottom: tokens.grid[2], paddingEnd: tokens.grid[3] },
  swipe: { position: "absolute", top: 0, bottom: 0, end: 0, alignItems: "center", justifyContent: "center", overflow: "hidden", backgroundColor: tokens.colour.red, color: tokens.colour.onAccent, cursor: "pointer" },
  chevron: { color: tokens.colour.secondaryLabel },
  // A row coming or going: it opens up where it goes, or closes up, fading as it does.
  slot: { overflow: "hidden", transitionProperty: ["height", "opacity"], transition: tokens.motion.settle },
  entering: { opacity: { default: 1, entering: 0 } },
  leaving: { opacity: 0, transitionProperty: ["height", "opacity"], transition: tokens.motion.settle },
})

/** How long a row takes to open up or close up, in milliseconds: the settle pace's (tokens.ts). */
const settle = duration(tokens.motion.settle)

/** A row as the list draws it: coming (its height opening up from none), there, or going (closing up), and the
 * height it's held at while it does. */
type Slot = { node: JSX.Element; state: () => State; setState: (state: State) => void; to: () => number | undefined; setTo: (to: number | undefined) => void; height?: number }
type State = "coming" | "there" | "going"

const slot = (node: JSX.Element, state: State, to?: number): Slot => {
  const [now, setState] = createSignal(state)
  const [held, setTo] = createSignal(to)
  return { node, state: now, setState, to: held, setTo }
}

/// `<List><For each={notes()}>{(note) => <Row heading={note.text} />}</For></List>`: things of one kind, one under
/// another, in one group that stops at a readable width however wide the room is. With no rows it shows nothing. A
/// row added to a list that has some opens up where it goes, and one taken out closes up, the rest making room or
/// closing the gap, at the settle pace; for whoever asked for less motion a new row fades in and one taken out just
/// goes. A list that changes in any other way (filled, a row changed for another) just shows what it's given.
export function List(props: { children?: JSX.Element }) {
  const rows = children(() => props.children)
  const [slots, setSlots] = createSignal<Slot[]>([])
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
      const made: Slot[] = next.map((node) => before.find((one) => one.node === node) ?? slot(node, plain ? "there" : "coming", plain || still ? undefined : 0))
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
    if (one.state() === "coming" && one.height === undefined) {
      requestAnimationFrame(() => requestAnimationFrame(() => one.setTo(height)))
      setTimeout(() => (one.setState("there"), one.setTo(undefined)), settle)
    }
    if (one.state() !== "going") one.height = height
  }
  return (
    <Show when={slots().length}>
      <Box role="list" style={look.list}>
        <Box style={look.rows}>
          <For each={slots()}>
            {(one) => (
              <Box style={[look.slot, one.state() === "coming" && look.entering, one.state() === "going" && look.leaving, one.to() !== undefined && { height: one.to()! }]}>
                <Box onLayout={(event) => laid(one, event.currentTarget.getBoundingClientRect().height)}>{one.node}</Box>
              </Box>
            )}
          </For>
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

/// `<Row heading="Coffee with Sam" text="Meals · 12 Nov" />`: one thing in a list: its `heading`, up to two grey lines
/// of `text`, what it starts with (`start`, an icon) and what it ends with (`end`: an amount, a switch). With
/// `onClick` the whole row is pressed and opens in place. `current` marks the one that's open beside the list, in a
/// split. `control` is a control before all of it, pressed apart from the row, as a checkbox that picks rows is, and
/// `action` a button after all of it, pressed apart from the row, as Sign In is for an account that needs it; an action
/// that would leave the words narrower than a menu goes under them. A ⋯ `Menu` in its `end` opens on a long press or a
/// right-click on the row too, and its `destructive` item (Delete, Remove) shows as a finger swipes the row toward
/// its start, as a phone's lists do: pressed, or swiped most of the way, it's done.
export function Row(props: {
  heading?: JSX.Element
  text?: JSX.Element
  start?: JSX.Element
  end?: JSX.Element
  onClick?: () => void
  current?: boolean
  control?: JSX.Element
  action?: JSX.Element
}) {
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
  const pressed = (event: Located) => event.timeStamp - moved >= 600 && props.onClick?.()

  const inside = () => (
    <>
      <Show when={props.start}>
        <Box style={look.start}>{props.start}</Box>
      </Show>
      <Box style={look.words}>
        <Text style={look.heading}>{props.heading}</Text>
        <Show when={props.text}>
          <Text style={look.text}>{props.text}</Text>
        </Show>
      </Box>
      <Show when={props.end}>
        <Box style={look.end}>{props.end}</Box>
      </Show>
    </>
  )
  const moving = () => [settling() && look.settling, swiped() > 0 && { translateX: direction() === "rtl" ? swiped() : -swiped() }]
  return (
    <RowMenu value={setMenu}>
      <Box
        role="listitem"
        onPointerDown={down}
        onPointerMove={move}
        onPointerUp={up}
        onPointerCancel={cancel}
        onContextMenu={(event) => menu()?.open(at(event))}
        style={[look.row, !!props.action && look.wraps, props.current && look.current, !!menu() && look.held]}
      >
        <Show when={props.control}>
          <Box style={[look.control, moving()]}>{props.control}</Box>
        </Show>
        <Show
          when={props.onClick}
          fallback={<Box style={[look.inside, !!props.control && look.after, moving()]}>{inside()}</Box>}
        >
          <Box role="button" aria-current={props.current ? "true" : undefined} onClick={pressed} style={[look.inside, look.pressable, !!props.control && look.after, moving()]}>
            {inside()}
          </Box>
        </Show>
        <Show when={props.action}>
          <Box style={[look.action, moving()]}>{props.action}</Box>
        </Show>
        <Show when={menu()?.destructive()}>
          {(item) => (
            <Box aria-hidden="true" onClick={() => (menu()?.select(item().value), aside(0, true))} style={[look.swipe, settling() && look.settling, { width: swiped() }]}>
              <Text>{item().label}</Text>
            </Box>
          )}
        </Show>
      </Box>
    </RowMenu>
  )
}
