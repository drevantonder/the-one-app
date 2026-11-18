// A list and its rows: written once in strict, on the primitives, the same on the DOM and on a native host.
import { children, createSignal, onCleanup, Show } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Text } from "#primitives"
import { RowMenu, type RowMenu as Menu } from "./menu.tsx"
import { negative, styles } from "./styles.ts"
import { tokens } from "./tokens.ts"
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
})

/// `<List><For each={notes()}>{(note) => <Row heading={note.text} />}</For></List>`: things of one kind, one under
/// another, in one group that stops at a readable width however wide the room is. With no rows it shows nothing.
export function List(props: { children?: JSX.Element }) {
  const rows = children(() => props.children)
  return (
    <Show when={rows.toArray().length}>
      <Box role="list" style={look.list}>
        <Box style={look.rows}>{rows()}</Box>
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
    const dx = start.x - event.clientX
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
  const moving = () => [settling() && look.settling, swiped() > 0 && { translateX: -swiped() }]
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
