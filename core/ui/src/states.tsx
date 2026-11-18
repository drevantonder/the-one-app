// An empty screen, what's coming, a problem, something under way and something starting: written once in strict, on
// the primitives (#primitives), the same on the DOM and on a native host.
import { createSignal, Show, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Box, Scroll, Text } from "#primitives"
import { copy } from "#bridge"
import { Button, Icon, Spinner } from "./button.tsx"
import { Middle } from "./screen.tsx"
import { messages, readable, whole } from "./words.ts"
import { styles } from "./styles.ts"
import { tokens } from "./tokens.ts"

const look = styles({
  state: { gap: tokens.grid[4], maxWidth: tokens.length.readable, alignItems: "flex-start", font: tokens.type.medium },
  // In the middle of its place: what's under way, or what fills a split's detail beside its list.
  middle: { maxWidth: "none", alignItems: "center", justifyContent: "center", padding: tokens.length.gutter, textAlign: "center" },
  fill: { flexGrow: 1 },
  grey: { color: tokens.colour.secondaryLabel },
  skeleton: {
    alignSelf: "stretch",
    borderWidth: tokens.length.hairline,
    borderColor: tokens.colour.quinaryLabel,
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.card,
    overflow: "hidden",
    opacity: 0,
    transitionProperty: ["opacity"],
    transition: tokens.motion.settle,
  },
  late: { opacity: 1 },
  line: { justifyContent: "center", gap: tokens.grid[2], padding: tokens.grid[3] },
  next: { borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  bar: { width: "40%", height: tokens.grid[3], borderRadius: tokens.radius.small, backgroundColor: tokens.colour.quinaryLabel, animationName: "pulse", animation: tokens.motion.pulse },
  longer: { width: "70%" },
  // As the mockup's: a box tinted by how much it needs the user, its symbol in that colour, its words plain, and the
  // action that fixes it at its end.
  problem: {
    flexDirection: "row",
    alignItems: "center",
    alignSelf: "stretch",
    columnGap: tokens.grid[3],
    paddingBlock: tokens.grid[3],
    paddingInline: tokens.grid[3],
    borderRadius: tokens.radius.medium,
    backgroundColor: tokens.colour.redTint,
    color: tokens.colour.label,
    textAlign: "start",
  },
  centred: { alignSelf: "center", maxWidth: tokens.length.readable },
  warn: { backgroundColor: tokens.colour.orangeTint },
  symbol: { alignSelf: "flex-start", paddingTop: tokens.grid[0.5], color: tokens.colour.red },
  warnSymbol: { color: tokens.colour.orange },
  words: { flexGrow: 1, flexShrink: 1, minWidth: 0, gap: tokens.grid[0.5], alignItems: "flex-start" },
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
  actions: { flexDirection: "row", flexShrink: 0, gap: tokens.grid[2], color: tokens.colour.label },
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
  return (
    <Box style={[look.state, middle() && look.middle, middle() && look.fill]}>
      <Text style={look.grey}>{props.children}</Text>
      {props.action}
    </Box>
  )
}

/// `<Skeleton rows={3} />`: grey shapes the shape of the list that's coming, once it's taken longer than a blink,
/// so a quick load shows nothing in between. It's what `<Loading fallback>` shows.
export function Skeleton(props: { rows?: number }) {
  const [late, setLate] = createSignal(false)
  setTimeout(() => setLate(true), 100)
  return (
    <Box role="status" aria-busy="true" aria-label={messages.loading} style={look.state}>
      <Box style={[look.skeleton, late() && look.late]}>
        {Array.from({ length: props.rows ?? 3 }, (_, i) => (
          <Box style={[look.line, i > 0 && look.next]}>
            <Box style={look.bar} />
            <Box style={[look.bar, look.longer]} />
          </Box>
        ))}
      </Box>
    </Box>
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
    <Box role="alert" style={[look.state, middle() && look.middle, middle() && look.fill]}>
      <Box style={[look.problem, middle() && look.centred, props.warn && look.warn]}>
        <Box style={[look.symbol, props.warn && look.warnSymbol]}>
          <Icon name="warning-circle" />
        </Box>
        <Box style={look.words}>
          <Text style={look.heading}>{props.heading}</Text>
          <Show when={props.text}>
            <Text style={look.said}>{all() === undefined ? (props.text as JSX.Element) : readable(all()!)}</Text>
          </Show>
          <Show when={all() && all() !== readable(all()!)}>
            <Box role="button" aria-expanded={open() ? "true" : "false"} onClick={() => setOpen(!open())} style={look.toggle}>
              <Box style={[look.caret, open() && look.turned]}>
                <Icon name="caret-right" along />
              </Box>
              <Text>{open() ? messages.hideDetails : messages.showDetails}</Text>
            </Box>
            <Show when={open()}>
              <Details text={all()!} />
            </Show>
          </Show>
        </Box>
        <Show when={props.action}>
          <Box style={look.actions}>{props.action}</Box>
        </Show>
      </Box>
    </Box>
  )
}

/// What follows the first paragraph of a tool's error, which is for an agent, and Copy, which takes all of it to
/// paste to an agent or to support. Under a problem's Show Details, and in what a toast's Details opens.
export function Details(props: { text: string }) {
  const [copied, setCopied] = createSignal(false)
  return (
    <Box style={look.details}>
      <Scroll style={look.code}>
        <Text style={look.mono}>{props.text.slice(readable(props.text).length).trim()}</Text>
      </Scroll>
      <Box style={look.copy}>
        <Button variant="ghost" size="icon" aria-label={copied() ? messages.copied : messages.copy} onClick={() => void copy(props.text).then(() => setCopied(true))}>
          <Icon name={copied() ? "check" : "copy"} />
        </Button>
      </Box>
    </Box>
  )
}

/// `<Status heading="Connecting…" />`: something under way where what it's about will be: a spinner, a short
/// `heading` and, only if it helps, one sentence of `text`, in the middle of its place. With `over`, it's a card on
/// what's there now, which it dims: a live view reconnecting, something saving. Its place is `position: "relative"`.
export function Status(props: { heading: JSX.Element; text?: JSX.Element; over?: boolean }) {
  const middle = useContext(Middle)
  return (
    <Box role="status" style={[look.state, look.middle, middle() && look.fill, props.over && look.over]}>
      <Box style={[look.status, props.over && look.card]}>
        <Spinner />
        <Text style={look.label}>{props.heading}</Text>
        <Show when={props.text}>
          <Text>{props.text}</Text>
        </Show>
      </Box>
    </Box>
  )
}

/// `<Steps steps={["Getting a browser ready", "Opening Personal"]} at={0} text="The first time takes a few minutes." />`:
/// something starting, as the steps it takes: those before `at` done, `at` under way with, only if it helps, one
/// sentence of `text`, and the rest to come. In the middle of its place, as `Status` is.
export function Steps(props: { steps: JSX.Element[]; at: number; text?: JSX.Element }) {
  const middle = useContext(Middle)
  return (
    <Box role="status" style={[look.state, look.middle, middle() && look.fill]}>
      <Box role="list" style={look.steps}>
        {props.steps.map((step, i) => (
          <Box role="listitem" style={[look.step, i === props.at && look.now]}>
            <Box style={look.mark}>{i < props.at ? <Icon name="check" /> : i === props.at ? <Spinner /> : <Box style={look.dot} />}</Box>
            <Box>
              <Text>{step}</Text>
              <Show when={i === props.at && props.text}>
                <Text style={look.note}>{props.text}</Text>
              </Show>
            </Box>
          </Box>
        ))}
      </Box>
    </Box>
  )
}
