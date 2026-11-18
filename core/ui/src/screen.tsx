// A whole view of an app, a part of one, and a list beside what it opens: written once in strict, on the primitives
// (#primitives), the same on the DOM and on a native host, which each say what's theirs (#bridge): their history,
// their device's back and how they move between screens.
import { createContext, createEffect, createSignal, onSettled, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Scroll, Text } from "#primitives"
import { back as goBack, breadcrumb, covers, go, keydown, step, touch, viewport } from "#bridge"
import { BarEnd, Button, Icon } from "./button.tsx"
import { messages } from "./words.ts"
import { ActionBar, type Action } from "./actions.tsx"
import { Choose, Collect } from "./collect.ts"
import { negative, styles } from "./styles.ts"
import { tokens } from "./tokens.ts"

const look = styles({
  screen: { position: "relative", flexGrow: 1, flexShrink: 1, minHeight: 0, minWidth: 0 },
  // The bar keeps clear of what the device keeps for itself at the top.
  bar: { paddingTop: tokens.length.safeTop, paddingStart: tokens.length.safeLeft, paddingEnd: tokens.length.safeRight, userSelect: "none" },
  row: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2], minHeight: tokens.length.bar, paddingInline: tokens.length.gutter },
  // A long name, which may be all the screen says of what it's about, takes two lines, as Android's bars do.
  title: { flexGrow: 1, flexShrink: 1, minWidth: 0, paddingBlock: tokens.grid[1], font: tokens.type.title, lineClamp: 2, textAlign: "start" },
  // On a computer, Raycast's: its title beside Back, 15px, on one line.
  computerTitle: { font: tokens.type.barTitle, lineClamp: 1 },
  // Where the window names the app above it, its first screen doesn't name it again, and keeps its place.
  unnamed: { opacity: 0 },
  // With nothing else in it, the bar keeps only the title's place under the breadcrumb.
  bare: { minHeight: 0 },
  // The arrow, not the button's edge, lines up with what's under it.
  back: { marginStart: negative(tokens.length.iconInset) },
  action: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2], flexShrink: 0 },
  main: { flexGrow: 1, minHeight: 0 },
  // What's in it keeps clear of what the device keeps for itself at its other edges.
  safe: { flexGrow: 1, paddingStart: tokens.length.safeLeft, paddingEnd: tokens.length.safeRight, paddingBottom: tokens.length.safeBottom },
  parts: { flexGrow: 1, gap: tokens.length.gutter, paddingTop: tokens.grid[2], paddingInline: tokens.length.gutter, paddingBottom: tokens.length.gutter },
  // What's last keeps clear of the footer's pills.
  footed: { paddingBottom: tokens.length.footer },
  section: { gap: tokens.grid[1.5], minWidth: 0 },
  // Raycast's on a computer, 11px; iOS's on a phone, its footnote inset to the rows' words.
  heading: { font: tokens.type.small, color: tokens.colour.secondaryLabel, userSelect: "none" },
  touchHeading: { paddingInline: tokens.grid[4] },
  split: { flexGrow: 1, flexShrink: 1, minHeight: 0, minWidth: 0, position: "relative" },
  beside: { flexDirection: "row" },
  pane: { minHeight: 0, minWidth: 0, overflow: "hidden", backgroundColor: tokens.colour.background },
  // Where there's no room for both, one shows at a time, over the other, which keeps its place and where it's
  // scrolled to: it's hidden, not gone.
  over: { position: "absolute", inset: 0 },
  hidden: { opacity: 0, pointerEvents: "none" },
  // Where there is, the list keeps a readable width and what it opens has the rest.
  list: { flexBasis: "38%", minWidth: tokens.length.columnMin, maxWidth: tokens.length.column, flexShrink: 0, borderEndWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  detail: { flexGrow: 1, flexShrink: 1 },
})

/** How wide a split's list is beside what it opens (tokens.length.columnMin, 38%, tokens.length.column), and how
 * wide it has to be for both, in CSS pixels. */
const columnMin = 18 * 16
const column = 24 * 16
const wide = 48 * 16

/** How wide what's being drawn has room for: the window, or a split's side. */
const Room = createContext<() => number>(() => viewport().width)
/** What closes what a split has open beside its list, for the list's Back button, while something is. */
const Opened = createContext<() => (() => void) | undefined>(() => undefined)
/** What a screen's Back does when it says nothing of its own: back through the history, or closing a split's detail. */
const Leave = createContext<() => void>(goBack)
/** Whether it's in a split's detail beside its list, where a screen has nowhere to go back to. */
const InDetail = createContext<() => boolean>(() => false)
/** Whether a state is all there is of a split's detail beside its list, which it fills, in its middle. */
export const Middle = createContext<() => boolean>(() => false)

/// `<Screen heading="Notes" action={<Button>Add</Button>}>...</Screen>`: a whole view of an app: a bar across its
/// top that stays as what's under it scrolls, with its `heading` and what it does (`action`: its one main button, or a
/// menu), then what's in it, one part under another. With `back` the bar starts with a Back button, which calls
/// `onBack`, or goes back as the device's own back gesture does. A split's list closes what's open beside it first. An
/// app's first screen has no Back: the window's rail, its title bar's Home on a computer, and the device's back leave
/// the app. Where the window's breadcrumb names the app, its first screen's heading gives way to it.
export function Screen(props: { heading?: JSX.Element; action?: JSX.Element; back?: boolean; onBack?: () => void; children?: JSX.Element }) {
  const opened = useContext(Opened)
  const leave = useContext(Leave)
  const inDetail = useContext(InDetail)
  const back = () => {
    const close = opened()
    if (close) close()
    else if (props.onBack) props.onBack()
    else leave()
  }
  // The device's own back calls a Back of the screen's own, as a tap on it does.
  createEffect(() => !!props.back && !!props.onBack, (now) => (now ? covers(back) : undefined))
  const backed = () => !!props.back && !inDetail()
  // Only an app's first screen, which has no Back, gives way to the breadcrumb.
  const named = () => !!props.back || !breadcrumb()
  // On a computer, as Raycast's: the bar's buttons and menus' items are the screen's actions, in the footer's pills and
  // its action panel (⌘K), after what the row chosen can do; under a finger they stay in the bar.
  const computer = !touch()
  const [bar, setBar] = createSignal<Action[]>([])
  const [chosen, setChosen] = createSignal<{ row: object; actions: () => Action[] }>()
  const collect = (action: Action) => {
    setBar((now) => [...now, action])
    return () => setBar((now) => now.filter((one) => one !== action))
  }
  const choose = {
    set: (row: object, actions: (() => Action[]) | undefined) => setChosen(actions ? { row, actions } : undefined),
    clear: (row: object) => chosen()?.row === row && setChosen(undefined),
  }
  const actions = () => [...(chosen()?.actions() ?? []).map((action) => ({ ...action, section: action.section ?? "chosen" })), ...bar()]
  const footed = () => computer && actions().length > 0
  return (
    <Box style={look.screen}>
      <Box role="banner" style={look.bar}>
        <Box style={[look.row, !named() && (!props.action || computer) && look.bare]}>
          <Show when={backed()}>
            <Button variant="ghost" size="icon" aria-label={messages.back} onClick={back} style={look.back}>
              <Icon name="caret-left" along />
            </Button>
          </Show>
          <Text role="heading" aria-level={1} aria-hidden={named() ? undefined : "true"} style={[look.title, computer && look.computerTitle, !named() && look.unnamed]}>
            {props.heading}
          </Text>
          <Show when={props.action}>
            <BarEnd value={true}>
              <Collect value={computer ? collect : null}>
                <Box style={look.action}>{props.action}</Box>
              </Collect>
            </BarEnd>
          </Show>
        </Box>
      </Box>
      <Scroll role="main" style={look.main}>
        <Box style={look.safe}>
          <Box style={[look.parts, footed() && look.footed]}>
            <Choose value={computer ? choose : null}>
              <Middle value={() => false}>{props.children}</Middle>
            </Choose>
          </Box>
        </Box>
      </Scroll>
      <Show when={footed()}>
        <ActionBar actions={actions()} />
      </Show>
    </Box>
  )
}

/// `<Section heading="This week">...</Section>`: a part of a screen under a small grey heading.
export function Section(props: { heading?: JSX.Element; children?: JSX.Element }) {
  return (
    <Box style={look.section}>
      <Show when={props.heading}>
        <Text role="heading" aria-level={2} style={[look.heading, touch() && look.touchHeading]}>
          {props.heading}
        </Text>
      </Show>
      {props.children}
    </Box>
  )
}

/** The splits with something open, the innermost last, which Escape closes first. */
const open: (() => void)[] = []

/// `<Split open={!!note()} onClose={() => setNote()} detail={<Screen heading={note().text} back>...</Screen>}><Screen heading="Notes">...</Screen></Split>`:
/// a list and what it opens (`detail`): beside it where there's room, pushed over it where there isn't, while
/// `open`. Each side scrolls by itself, and the split is as tall as what it's in. While it's open the device's own
/// back, Escape, and the detail's Back button close it, or beside its list, where a detail has none, the list's: it
/// calls `onClose`, and the app sets `open` false.
export function Split(props: { open?: boolean; onClose?: () => void; detail?: JSX.Element; children?: JSX.Element }) {
  const room = useContext(Room)
  const inDetail = useContext(InDetail)
  const beside = () => room() >= wide
  const listWidth = () => Math.min(column, Math.max(columnMin, room() * 0.38))
  // What's drawn as open, which follows `open` as a move.
  const [shown, setShown] = createSignal(false)
  const close = () => props.onClose?.()

  // Escape closes it, unless it's for something open over it, a sheet or a menu, or a split open in it, which closes
  // first.
  onSettled(() => keydown((event) => event.key === "Escape" && props.open && open.at(-1) === close && close()))

  // It starts as it's written, and beside its list it shows at once; pushed over it, it moves.
  createEffect(() => !!props.open, (now, was) => {
    if (was === undefined || beside()) setShown(now)
    else go(() => setShown(now), { back: !now })
  })

  // Open, it's what the device's back closes: beside its list as it closes a sheet, pushed over the list as a step
  // back through the history, which a device's gesture shows as the whole page going back.
  createEffect(() => (props.open ? (beside() ? "beside" : "over") : ""), (how) => {
    if (!how) return
    open.push(close)
    const done = how === "beside" ? covers(close) : step(close)
    return () => (open.splice(open.indexOf(close), 1), done())
  })

  return (
    <Box style={[look.split, beside() && look.beside]}>
      <Box aria-hidden={!beside() && shown() ? "true" : undefined} style={[look.pane, beside() ? look.list : look.over, !beside() && shown() && look.hidden]}>
        <Room value={() => (beside() ? listWidth() : room())}>
          <Opened value={() => (props.open ? close : undefined)}>
            <Middle value={() => false}>{props.children}</Middle>
          </Opened>
        </Room>
      </Box>
      <Box aria-hidden={!beside() && !shown() ? "true" : undefined} style={[look.pane, beside() ? look.detail : look.over, !beside() && !shown() && look.hidden]}>
        <Room value={() => (beside() ? room() - listWidth() - 1 : room())}>
          <Leave value={close}>
            <InDetail value={() => inDetail() || beside()}>
              <Middle value={beside}>{props.detail}</Middle>
            </InDetail>
          </Leave>
        </Room>
      </Box>
    </Box>
  )
}
