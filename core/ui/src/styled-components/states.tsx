// An empty screen, what's coming, a problem, something under way and something starting: written once on the
// drawing primitives (#primitives), the same on every host.
import { createEffect, createSignal, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Pressable, ScrollView, Text, View } from "#primitives"
import { copy, touch } from "#bridge"
import { ListStyle } from "./list.tsx"
import { Button, Icon, Spinner } from "./button.tsx"
import { Blank, Middle } from "./screen.tsx"
import { messages, readable, whole } from "../words.ts"
import { styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

const look = styles({
  state: { gap: tokens.grid[4], maxWidth: tokens.length.readable, alignItems: "flex-start", font: tokens.type.medium },
  // In the middle of its place: what's under way, or what fills a split's detail beside its list.
  middle: { maxWidth: "none", alignItems: "center", justifyContent: "center", padding: tokens.length.gutter, textAlign: "center" },
  fill: { flexGrow: 1 },
  grey: { color: tokens.colour.secondaryLabel },
  // All there is, its line is body text, as iOS's ContentUnavailableView's description is.
  body: { font: tokens.type.large, fontWeight: tokens.weight.regular },
  // A list's card, and each line a row of it, a heading's line over a grey line's, as tall as such a row, so what
  // replaces it takes the room it took.
  skeleton: {
    alignSelf: "stretch",
    boxShadow: tokens.shadow.edge,
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.card,
    overflow: "hidden",
    opacity: 0,
    transitionProperty: ["opacity"],
    transition: tokens.motion.settle,
  },
  late: { opacity: 1 },
  // As a row's insides are, in each list style (list.tsx's `Row`).
  line: { justifyContent: "center", minHeight: tokens.length.row, paddingBlock: tokens.grid[2], paddingInline: tokens.grid[3] },
  flatLine: { flexDirection: "row", justifyContent: "flex-start", alignItems: "center", gap: tokens.grid[2], paddingBlock: 0, paddingInline: tokens.grid[2] },
  groupedLine: { minHeight: tokens.length.groupedRow, paddingBlock: tokens.grid[1.5] },
  sidebarLine: { minHeight: tokens.length.sidebarRow },
  next: { borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  // A line of words, as tall as one, its words a bar across the middle of it, as iOS's placeholder draws text.
  bar: { position: "relative", width: "40%", font: tokens.type.medium, color: tokens.colour.transparent },
  shade: { position: "absolute", top: "20%", bottom: "20%", start: 0, end: 0, borderRadius: tokens.radius.small, backgroundColor: tokens.colour.quinaryLabel, animationName: "pulse", animation: tokens.motion.pulse },
  longer: { width: "70%" },
  flatLonger: { width: "30%" },
  groupedBar: { font: tokens.type.groupedRow },
  sidebarBar: { font: tokens.type.sidebarRow },
  captionBar: { font: tokens.type.caption },
  // As the mockup's: a box tinted by how much it needs the user, its symbol in that colour, its words plain, and the
  // action that fixes it at its end.
  problem: {
    flexDirection: "row",
    // Its actions go under its words where they'd leave the words narrower than a menu.
    flexWrap: "wrap",
    alignItems: "center",
    alignSelf: "stretch",
    columnGap: tokens.grid[3],
    rowGap: tokens.grid[3],
    paddingBlock: tokens.grid[3],
    paddingInline: tokens.grid[3],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.redTint,
    color: tokens.colour.label,
    textAlign: "start",
  },
  centred: { alignSelf: "center", width: "100%", maxWidth: tokens.length.readable },
  warn: { backgroundColor: tokens.colour.orangeTint },
  symbol: { alignSelf: "flex-start", paddingTop: tokens.grid[0.5], color: tokens.colour.red },
  warnSymbol: { color: tokens.colour.orange },
  words: { flexGrow: 1, flexShrink: 1, flexBasis: tokens.length.menu, minWidth: 0, gap: tokens.grid[0.5], alignItems: "flex-start" },
  heading: { fontWeight: tokens.weight.medium },
  said: { color: tokens.colour.secondaryLabel },
  // Show Details: quiet, under the sentence.
  toggle: {
    flexDirection: "row",
    alignItems: "center",
    gap: tokens.grid[1],
    marginTop: tokens.grid[1],
    borderRadius: tokens.radius.small,
    color: { default: tokens.colour.secondaryLabel, hover: tokens.colour.label },
    font: tokens.type.small,
    cursor: "pointer",
    userSelect: "none",
  },
  caret: { transitionProperty: ["rotate"], transition: tokens.motion.quick },
  turned: { rotate: 90 },
  actions: { flexDirection: "row", flexWrap: "wrap", flexShrink: 0, maxWidth: "100%", gap: tokens.grid[2], color: tokens.colour.label },
  // The rest of a tool's error, as it was written, with Copy in its corner as a code block has.
  details: { position: "relative", alignSelf: "stretch", marginTop: tokens.grid[1] },
  code: {
    minHeight: tokens.length.control,
    maxHeight: tokens.length.excerpt,
    paddingBlock: tokens.grid[2],
    paddingStart: tokens.grid[3],
    paddingEnd: tokens.length.control,
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.quinaryLabel,
    color: tokens.colour.label,
  },
  mono: { font: tokens.type.small, fontFamily: tokens.font.mono, whiteSpace: "pre-wrap", textAlign: "start" },
  copy: { position: "absolute", top: 0, end: 0, color: tokens.colour.secondaryLabel },
  status: { alignItems: "center", gap: tokens.grid[2], color: tokens.colour.secondaryLabel, font: tokens.type.small },
  label: { font: tokens.type.medium, fontWeight: tokens.weight.medium, color: tokens.colour.label },
  // On what's there now, which it dims, as a card.
  over: { position: "absolute", inset: 0, backgroundColor: tokens.colour.veil },
  card: { paddingBlock: tokens.grid[4], paddingInline: tokens.grid[6], borderWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel, borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.card },
  steps: { gap: tokens.grid[3], alignItems: "flex-start", textAlign: "start" },
  step: { flexDirection: "row", gap: tokens.grid[2.5], color: tokens.colour.secondaryLabel },
  now: { color: tokens.colour.label },
  mark: { width: tokens.length.icon, alignItems: "center", paddingTop: tokens.grid[0.5] },
  dot: { width: tokens.grid[2.5], height: tokens.grid[2.5], marginTop: tokens.grid[1], borderWidth: tokens.length.hairline, borderColor: tokens.colour.quaternaryLabel, borderRadius: tokens.radius.full },
  note: { color: tokens.colour.secondaryLabel, font: tokens.type.small },
})

/// `<Empty>No notes yet.</Empty>`: an empty screen or list: one grey line, and a button (`action`) only when
/// there's an obvious first step.
export function Empty(props: { children?: JSX.Element; action?: JSX.Element }) {
  const middle = useContext(Middle)
  const blank = useContext(Blank)
  createEffect(() => true, () => (blank(1), () => blank(-1)))
  return (
    <View style={[look.state, middle() && look.middle, middle() && look.fill]}>
      <Text style={[look.grey, middle() && look.body]}>{props.children}</Text>
      {props.action}
    </View>
  )
}

/// `<Skeleton rows={3} />`: grey shapes the shape of the list that's coming, once it's taken longer than a blink,
/// so a quick load shows nothing in between: each row as tall as a list's row with a heading and a line of text, so
/// the list takes its place without moving what's around it. It's what `<Loading fallback>` shows.
export function Skeleton(props: { rows?: number }) {
  const middle = useContext(Middle)
  const style = useContext(ListStyle)
  const flat = () => !touch() && style === "plain"
  const grouped = () => !touch() && style === "insetGrouped"
  const sidebar = () => !touch() && style === "sidebar"
  const [late, setLate] = createSignal(false)
  setTimeout(() => setLate(true), 100)
  return (
    <View role="status" aria-busy="true" aria-label={messages.loading} style={[look.state, middle() && look.centred]}>
      <View style={[look.skeleton, late() && look.late]}>
        {Array.from({ length: props.rows ?? 3 }, (_, i) => (
          <View style={[look.line, flat() && look.flatLine, grouped() && look.groupedLine, sidebar() && look.sidebarLine, i > 0 && look.next]}>
            <View style={[look.bar, grouped() && look.groupedBar, sidebar() && look.sidebarBar]}>
              <Text>{"\u00a0"}</Text>
              <View style={look.shade} />
            </View>
            <View style={[look.bar, look.longer, flat() && look.flatLonger, style !== "plain" && !touch() && look.captionBar]}>
              <Text>{"\u00a0"}</Text>
              <View style={look.shade} />
            </View>
          </View>
        ))}
      </View>
    </View>
  )
}

/// `<Problem heading="Couldn't load notes" text="The server didn't answer." action={<Button>Try again</Button>} />`:
/// a problem, shown on the thing it's about: a symbol, a short `heading`, one sentence of `text`, and the action
/// that fixes it. Red, for what needs the user now; `warn` for what can wait. It's what `<Errored fallback>` shows.
/// A `text` that's an error, as `call` throws it (`<Errored fallback={(error) => <Problem text={error()} />}>`), or a
/// tool's error as text, shows what a person reads of it (`readable`), and the rest under Show Details, to copy for an
/// agent or for support.
export function Problem(props: { heading: JSX.Element; text?: JSX.Element | Error; action?: JSX.Element; warn?: boolean }) {
  const middle = useContext(Middle)
  const all = () => (typeof props.text === "string" || props.text instanceof Error ? whole(props.text) : undefined)
  const [open, setOpen] = createSignal(false)
  return (
    <View role="alert" style={[look.state, middle() && look.middle, middle() && look.fill]}>
      <View style={[look.problem, middle() && look.centred, props.warn && look.warn]}>
        <View style={[look.symbol, props.warn && look.warnSymbol]}>
          <Icon name="warning-circle" />
        </View>
        <View style={look.words}>
          <Text style={look.heading}>{props.heading}</Text>
          <Show when={props.text}>
            <Text style={look.said}>{all() === undefined ? (props.text as JSX.Element) : readable(all()!)}</Text>
          </Show>
          <Show when={all() && all() !== readable(all()!)}>
            <Pressable role="button" aria-expanded={open() ? "true" : "false"} onPress={() => setOpen(!open())} style={look.toggle}>
              <View style={[look.caret, open() && look.turned]}>
                <Icon name="caret-right" along />
              </View>
              <Text>{open() ? messages.hideDetails : messages.showDetails}</Text>
            </Pressable>
            <Show when={open()}>
              <Details text={all()!} />
            </Show>
          </Show>
        </View>
        <Show when={props.action}>
          <View style={look.actions}>{props.action}</View>
        </Show>
      </View>
    </View>
  )
}

/// What follows the first paragraph of a tool's error, which is for an agent, and Copy, which takes all of it to
/// paste to an agent or to support. Under a problem's Show Details, and in what a toast's Details opens.
export function Details(props: { text: string }) {
  const [copied, setCopied] = createSignal(false)
  return (
    <View style={look.details}>
      <ScrollView style={look.code}>
        <Text style={look.mono}>{props.text.slice(readable(props.text).length).trim()}</Text>
      </ScrollView>
      <View style={look.copy}>
        <Button variant="ghost" size="icon" aria-label={copied() ? messages.copied : messages.copy} onPress={() => void copy(props.text).then(() => setCopied(true))}>
          <Icon name={copied() ? "check" : "copy"} />
        </Button>
      </View>
    </View>
  )
}

/// `<Status heading="Connecting…" />`: something under way where what it's about will be: a spinner, a short
/// `heading` and, only if it helps, one sentence of `text`, in the middle of its place. With `over`, it's a card on
/// what's there now, which it dims: a live view reconnecting, something saving. Its place is `position: "relative"`.
export function Status(props: { heading: JSX.Element; text?: JSX.Element; over?: boolean }) {
  const middle = useContext(Middle)
  return (
    <View role="status" style={[look.state, look.middle, middle() && look.fill, props.over && look.over]}>
      <View style={[look.status, props.over && look.card]}>
        <Spinner />
        <Text style={look.label}>{props.heading}</Text>
        <Show when={props.text}>
          <Text>{props.text}</Text>
        </Show>
      </View>
    </View>
  )
}

/// `<Steps steps={["Getting a browser ready", "Opening Personal"]} at={0} text="The first time takes a few minutes." />`:
/// something starting, as the steps it takes: those before `at` done, `at` under way with, only if it helps, one
/// sentence of `text`, and the rest to come. In the middle of its place, as `Status` is.
export function Steps(props: { steps: JSX.Element[]; at: number; text?: JSX.Element }) {
  const middle = useContext(Middle)
  return (
    <View role="status" style={[look.state, look.middle, middle() && look.fill]}>
      <View role="list" style={look.steps}>
        {props.steps.map((step, i) => (
          <View role="listitem" style={[look.step, i === props.at && look.now]}>
            <View style={look.mark}>{i < props.at ? <Icon name="check" /> : i === props.at ? <Spinner /> : <View style={look.dot} />}</View>
            <View>
              <Text>{step}</Text>
              <Show when={i === props.at && props.text}>
                <Text style={look.note}>{props.text}</Text>
              </Show>
            </View>
          </View>
        ))}
      </View>
    </View>
  )
}
