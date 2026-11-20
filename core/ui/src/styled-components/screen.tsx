// A whole view of an app, a part of one, and a list beside what it opens: written once on the drawing primitives
// (#primitives), the same on the DOM and on a native host, which each say what's theirs (#bridge): their history,
// their device's back and how they move between screens.
import { children, createContext, createEffect, createSignal, onSettled, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Pressable, ScrollView, Text, View } from "#primitives"
import { back as goBack, covers, go, keydown, step, titled, touch, viewport } from "#bridge"
import { BarEnd, Button, Icon } from "./button.tsx"
import { messages } from "../words.ts"
import { ActionBar, type Action } from "./actions.tsx"
import { ListStyle } from "./list.tsx"
import { Choose, Collect } from "./collect.ts"
import { negative, styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

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
  // On a computer the bar is the window's title row, its unified toolbar, as macOS's System Settings and Raycast have
  // it: at the window's top, after the window's controls and title where it's at the window's start, and before
  // the controls at its end where it's at its end.
  titleRow: { paddingTop: 0 },
  atStart: { paddingStart: tokens.length.titleBarStart },
  atEnd: { paddingEnd: tokens.length.titleBarEnd },
  // The arrow, not the button's edge, lines up with what's under it.
  back: { marginStart: negative(tokens.length.iconInset) },
  // On a computer, Back and Forward in one pill of glass before the title, as macOS's System Settings and Raycast's
  // Settings have them; under a finger, Back alone, round.
  pill: {
    flexDirection: "row",
    alignItems: "center",
    flexShrink: 0,
    height: tokens.length.control,
    paddingInline: tokens.grid[1],
    borderRadius: tokens.radius.full,
    backgroundColor: tokens.colour.glass,
    boxShadow: tokens.shadow.pill,
  },
  pillButton: { width: tokens.length.control, height: tokens.length.control, alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.full, cursor: "pointer", color: tokens.colour.secondaryLabel },
  pillOff: { color: tokens.colour.quaternaryLabel, cursor: "default" },
  pillLine: { width: tokens.length.hairline, height: tokens.grid[4], backgroundColor: tokens.colour.quinaryLabel },
  round: { paddingInline: 0 },
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
  // Above a card in grouped cards: the group's name, as Raycast's Settings heads one, in line with the card's words.
  groupedHeading: { font: tokens.type.medium, fontWeight: tokens.weight.medium, color: tokens.colour.label, paddingInline: tokens.grid[3], paddingTop: tokens.grid[2], paddingBottom: tokens.grid[1] },
  // What's in a screen of grouped cards keeps a readable width in the middle of the window, as Raycast's Settings' does,
  // so a note, a field or a button is in line with the cards.
  column: { flexGrow: 1, gap: tokens.length.gutter },
  groupedColumn: { width: "100%", maxWidth: tokens.length.readable, alignSelf: "center" },
  split: { flexGrow: 1, flexShrink: 1, minHeight: 0, minWidth: 0, position: "relative" },
  beside: { flexDirection: "row" },
  pane: { minHeight: 0, minWidth: 0, overflow: "hidden", backgroundColor: tokens.colour.background },
  // Where there's no room for both, one shows at a time, over the other, which keeps its place and where it's
  // scrolled to: it's hidden, not gone.
  over: { position: "absolute", inset: 0 },
  hidden: { opacity: 0, pointerEvents: "none" },
  // Where there is, the list keeps a readable width and what it opens has the rest.
  list: { flexBasis: "38%", minWidth: tokens.length.columnMin, maxWidth: tokens.length.column, flexShrink: 0, borderEndWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  // A window's sidebar is narrower, as macOS's and Raycast's Settings' are.
  sidebar: { flexBasis: tokens.length.sidebar, minWidth: tokens.length.sidebar, maxWidth: tokens.length.sidebar },
  detail: { flexGrow: 1, flexShrink: 1 },
})

/** How far a screen with a large title is scrolled when that's gone under its bar, in CSS pixels. */
const LARGE = 56

/** How wide a split's list is beside what it opens (tokens.length.columnMin, 38%, tokens.length.column), and how
 * wide it has to be for both, in CSS pixels. */
const columnMin = 18 * 16
const column = 24 * 16
const wide = 48 * 16
const sidebarWidth = 15 * 16

/** How wide what's being drawn has room for: the window, or a split's side. */
const Room = createContext<() => number>(() => viewport().width)
/** What closes what a split has open beside its list, for the list's Back button, while something is. */
const Opened = createContext<() => (() => void) | undefined>(() => undefined)
/** What a screen's Back does when it says nothing of its own: back through the history, or closing a split's detail. */
const Leave = createContext<() => void>(goBack)
/** Whether it's in a split's detail beside its list, where a screen has nowhere to go back to. A split in that detail
 * whose own detail is pushed over its list has a Back again, to that list. */
const InDetail = createContext<() => boolean>(() => false)
type Went = { back?: () => void; forward?: () => void } | null
/** Where the window's Back and Forward go, read as it changes. */
const Went = createContext<() => Went>(() => null)
/// `<History value={{ back, forward }}>`: where a window's Back and Forward go, as macOS's System Settings keeps them:
/// the screens in it draw them in their bar, each greyed where it goes nowhere, as `value` changes. A split pushing a
/// screen over its list starts again, its Back going back to the list. ⌘[ and ⌘] on a Mac, and Alt+← and Alt+→
/// elsewhere, go Back and Forward too, as a browser's and Finder's do.
export function History(props: { value: Went; children?: JSX.Element }) {
  onSettled(() =>
    keydown((event) => {
      const only = (held: boolean) => held && [event.metaKey, event.altKey, event.ctrlKey, event.shiftKey].filter(Boolean).length === 1
      const way = only(event.metaKey) ? { "[": "back", "]": "forward" }[event.key] : only(event.altKey) ? { ArrowLeft: "back", ArrowRight: "forward" }[event.key] : undefined
      const went = way && props.value?.[way as "back" | "forward"]
      if (!went) return
      event.preventDefault()
      went()
    }),
  )
  return <Went value={() => props.value}>{props.children}</Went>
}
/** Which of the window's edges a screen's bar reaches, where its title row's controls and title are: a split's
 * list beside what it opens reaches the start, and what it opens the end. */
const Edge = createContext<() => { start: boolean; end: boolean }>(() => ({ start: true, end: true }))
/** Whether a state is all there is of a split's detail beside its list, which it fills, in its middle. */
export const Middle = createContext<() => boolean>(() => false)

/// `<Screen heading="Notes" action={<Button>Add</Button>}>...</Screen>`: a whole view of an app: a bar across its
/// top that stays as what's under it scrolls, with its `heading` and what it does (`action`: its one main button, or a
/// menu), then what's in it, one part under another. With `back` the bar starts with a Back button, which calls
/// `onBack`, as the device's own back gesture does: a screen with `back` says where it goes, but in a split's detail,
/// which it closes. A split's list closes what's open beside it first. An
/// app's first screen has no Back: the window's rail and the device's back leave the app. Where the window's title
/// names the app, its first screen's heading gives way to it. With `largeTitle`
/// the screen draws its name large itself, as iOS's App Store's pages do, and the bar's shows once that has scrolled away.
export function Screen(props: { heading?: JSX.Element; action?: JSX.Element; back?: boolean; onBack?: () => void; largeTitle?: boolean; children?: JSX.Element }) {
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
  createEffect(() => !!props.back && !!props.onBack, (now) => (now ? covers(back, { layer: false }) : undefined))
  const backed = () => !!props.back && !inDetail()
  const history = useContext(Went)
  const style = useContext(ListStyle)
  const edge = useContext(Edge)
  // Only an app's first screen, which has no Back, gives way to the window's title.
  const [scrolled, setScrolled] = createSignal(0)
  const named = () => (!!props.back || !titled()) && (!props.largeTitle || scrolled() > LARGE)
  // On a computer, as Raycast's: the bar's buttons and menus' items are the screen's actions, in the footer's pills and
  // its action panel (⌘K), after what the row chosen can do; under a finger they stay in the bar.
  const computer = !touch()
  // Its action panel is where a list's rows and the bar's buttons go on a computer; a window of grouped cards has none,
  // as Raycast's Settings window has none, so they stay in the bar and on their rows.
  const panelled = computer && style === "plain"
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
  const footed = () => panelled && actions().length > 0
  return (
    <View style={look.screen}>
      <View role="banner" style={[look.bar, computer && look.titleRow]}>
        <View style={[look.row, computer && edge().start && look.atStart, computer && edge().end && look.atEnd]}>
          <Show when={backed() || (computer && history())}>
            <View role="group" style={[look.pill, !(computer && history()) && look.round]}>
              <Pressable
                role="button"
                aria-label={messages.back}
                aria-disabled={backed() || history()?.back ? undefined : "true"}
                onPress={() => (backed() ? back() : history()?.back?.())}
                style={[look.pillButton, !(backed() || history()?.back) && look.pillOff]}
              >
                <Icon name="caret-left" along />
              </Pressable>
              <Show when={computer && history()}>
                <View style={look.pillLine} />
                <Pressable
                  role="button"
                  aria-label={messages.forward}
                  aria-disabled={!backed() && history()?.forward ? undefined : "true"}
                  onPress={() => !backed() && history()?.forward?.()}
                  style={[look.pillButton, (backed() || !history()?.forward) && look.pillOff]}
                >
                  <Icon name="caret-right" along />
                </Pressable>
              </Show>
            </View>
          </Show>
          <Heading level={1} aria-hidden={named() ? undefined : "true"} style={[look.title, computer && look.computerTitle, !named() && look.unnamed]}>
            {props.heading}
          </Heading>
          <Show when={props.action}>
            <BarEnd value={true}>
              <Collect value={panelled ? collect : null}>
                <View style={look.action}>{props.action}</View>
              </Collect>
            </BarEnd>
          </Show>
        </View>
      </View>
      <ScrollView role="main" style={look.main} onScroll={props.largeTitle ? (event) => setScrolled(event.currentTarget.scrollTop) : undefined}>
        <View style={look.safe}>
          <View style={[look.parts, footed() && look.footed]}>
            <View style={[look.column, (style === "insetGrouped" || (style === "sidebar" && touch())) && look.groupedColumn]}>
              <Choose value={panelled ? choose : null}>
                <Middle value={() => false}>{props.children}</Middle>
              </Choose>
            </View>
          </View>
        </View>
      </ScrollView>
      <Show when={footed()}>
        <ActionBar actions={actions()} />
      </Show>
    </View>
  )
}

/** A heading: its words a `Text`, or, where it's more than words (a button at its end, an icon before it), a box
 * that's the heading, as a `Text` holds only words. */
function Heading(props: { level: number; "aria-hidden"?: "true"; style: Parameters<typeof Text>[0]["style"]; children?: JSX.Element }) {
  const inside = children(() => props.children)
  const words = () => inside.toArray().every((part) => typeof part === "string" || typeof part === "number")
  return (
    <Show when={words()} fallback={<View role="heading" aria-level={props.level} aria-hidden={props["aria-hidden"]} style={props.style}>{inside()}</View>}>
      <Text role="heading" aria-level={props.level} aria-hidden={props["aria-hidden"]} style={props.style}>
        {inside()}
      </Text>
    </Show>
  )
}

/// `<Section heading="This week">...</Section>`: a part of a screen under a small grey heading.
export function Section(props: { heading?: JSX.Element; children?: JSX.Element }) {
  // Grouped cards head their groups, and so does a sidebar under a finger, which is drawn as them.
  const style = useContext(ListStyle)
  const grouped = () => style === "insetGrouped" || (style === "sidebar" && touch())
  return (
    <View style={look.section}>
      <Show when={props.heading}>
        <Heading level={2} style={[look.heading, touch() && look.touchHeading, grouped() && look.groupedHeading]}>
          {props.heading}
        </Heading>
      </Show>
      {props.children}
    </View>
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
  const history = useContext(Went)
  const edge = useContext(Edge)
  // What a split in another's detail opens is pushed over its list, as a window's page of settings opens what's in it.
  const inDetail = useContext(InDetail)
  const beside = () => room() >= wide && !inDetail()
  // A sidebar's list is a sidebar's width (`ListStyle`), and the detail has the rest.
  const sidebar = useContext(ListStyle) === "sidebar"
  const listWidth = () => (sidebar ? sidebarWidth : Math.min(column, Math.max(columnMin, room() * 0.38)))
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
  // back through the history, which a device's gesture shows as the whole page going back. A sidebar's choice beside it
  // is no more closed than macOS's is by Escape.
  createEffect(() => (props.open ? (beside() ? (sidebar ? "" : "beside") : "over") : ""), (how) => {
    if (!how) return
    open.push(close)
    const done = how === "beside" ? covers(close, { layer: false }) : step(close)
    return () => (open.splice(open.indexOf(close), 1), done())
  })

  return (
    <View style={[look.split, beside() && look.beside]}>
      <View aria-hidden={!beside() && shown() ? "true" : undefined} style={[look.pane, beside() ? look.list : look.over, beside() && sidebar && look.sidebar, !beside() && shown() && look.hidden]}>
        <Room value={() => (beside() ? listWidth() : room())}>
          <Edge value={() => ({ start: edge().start, end: edge().end && !beside() })}>
          <Opened value={() => (props.open ? close : undefined)}>
            <Middle value={() => false}>{props.children}</Middle>
          </Opened>
          </Edge>
        </Room>
      </View>
      <View aria-hidden={!beside() && !shown() ? "true" : undefined} style={[look.pane, beside() ? look.detail : look.over, !beside() && !shown() && look.hidden]}>
        <Room value={() => (beside() ? room() - listWidth() - 1 : room())}>
          <Edge value={() => ({ start: edge().start && !beside(), end: edge().end })}>
          <Leave value={close}>
            <History value={beside() ? history() : null}>
            <InDetail value={beside}>
              <Middle value={beside}>{props.detail}</Middle>
            </InDetail>
            </History>
          </Leave>
          </Edge>
        </Room>
      </View>
    </View>
  )
}
