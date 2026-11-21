// A widget: a card of an app's, drawn from the parts a layout is made of. Written once on the drawing primitives,
// the same on every host. It fills the cells a Home Screen page gives it, and what doesn't fit is cut
// off, never scrolled. Its name is small and grey, low in its order. Colour is for what needs someone.
import { createContext, createSignal, For, Match, Show, Switch as Which, useContext } from "solid-js"
import type { JSX } from "@solidjs/web"
import { Image, Path, Pressable, Svg, Text, View } from "#primitives"
import type { Laid, Located, Rect } from "../types/ui.d.ts"
import { Spinner } from "./button.tsx"
import { Switch } from "./field.tsx"
import { styles, type Styles } from "../styles.ts"
import { tokens } from "../tokens.ts"

const look = styles({
  card: {
    position: "relative",
    zIndex: 0,
    flexGrow: 1,
    gap: tokens.grid[2],
    minWidth: 0,
    minHeight: 0,
    padding: tokens.grid[3],
    borderRadius: tokens.radius.large,
    backgroundColor: tokens.colour.quinaryLabel,
    color: tokens.colour.label,
    font: tokens.type.medium,
    overflow: "hidden",
    userSelect: "none",
  },
  opens: { cursor: "pointer" },
  // What opens the app: a button of its own, spread under the card's parts, so a keyboard reaches it and a screen
  // reader names it.
  open: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, zIndex: -1, borderRadius: tokens.radius.large, boxShadow: { focused: tokens.shadow.focus } },
  name: { flexShrink: 0, font: tokens.type.small, fontWeight: tokens.weight.medium, color: tokens.colour.secondaryLabel, lineClamp: 1 },
  problem: { font: tokens.type.small, color: tokens.colour.orange },
  // A column or a row.
  column: { gap: tokens.grid[1], minWidth: 0, minHeight: 0 },
  row: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2], minWidth: 0, minHeight: 0 },
  root: { flexGrow: 1 },
  // A part's own size, which it says as it's drawn.
  width: (width: number) => ({ width }),
  height: (height: number) => ({ height }),
  square: (size: number) => ({ width: size, height: size }),
  corner: (top: number, start: number, width: number, height: number) => ({ top, start, width, height }),
  centre: { alignItems: "center" },
  // What every part may say of itself: that it takes the room that's left, that it's a width of its own, that it's
  // faded, and its tone.
  grow: { flexGrow: 1, flexShrink: 1, flexBasis: 0 },
  fixed: { flexGrow: 0, flexShrink: 0 },
  full: { width: "100%" },
  dim: { opacity: 0.5 },
  muted: { color: tokens.colour.secondaryLabel },
  warn: { color: tokens.colour.orange },
  bad: { color: tokens.colour.red },
  // Words: a type style by `size`, a weight, and in a row one line.
  text: { minWidth: 0, font: tokens.type.medium },
  xl: { font: tokens.type.largeTitle },
  lg: { font: tokens.type.title },
  sm: { font: tokens.type.small },
  medium: { fontWeight: tokens.weight.medium },
  bold: { fontWeight: tokens.weight.semibold },
  line: { lineClamp: 1 },
  // A figure is never cut: in a row, a stat keeps its own width and the others give way.
  stat: { flexShrink: 0 },
  figure: { flexDirection: "row", alignItems: "baseline", gap: tokens.grid[1] },
  value: { font: tokens.type.figure },
  unit: { font: tokens.type.large, color: tokens.colour.secondaryLabel },
  small: { font: tokens.type.small, color: tokens.colour.secondaryLabel, lineClamp: 1 },
  // A ring, a bar and a line: their shapes fill a box of their own, in the part's colour.
  gauge: { position: "relative", flexShrink: 0 },
  over: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, width: "100%", height: "100%" },
  // What a ring or a bar fills, in the text at 20% whatever its tone.
  track: { color: tokens.colour.quaternaryLabel },
  area: { opacity: 0.15 },
  middle: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, alignItems: "center", justifyContent: "center" },
  meterLabel: { font: tokens.type.small, fontWeight: tokens.weight.semibold },
  progress: { gap: tokens.grid[1], minWidth: 0 },
  bar: { position: "relative", height: tokens.grid[2] },
  drawn: { position: "relative", flexShrink: 0 },
  chart: { flexShrink: 0, gap: tokens.grid[1] },
  bars: { position: "relative", flexGrow: 1, minHeight: 0 },
  labels: { flexDirection: "row" },
  label: { flexGrow: 1, flexBasis: 0, minWidth: 0, font: tokens.type.small, color: tokens.colour.secondaryLabel, textAlign: "center", lineClamp: 1 },
  // A row of a list, a hairline above it but the first.
  item: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2], minWidth: 0, paddingBlock: tokens.grid[1], borderTopWidth: tokens.length.hairline, borderColor: tokens.colour.quinaryLabel },
  first: { borderTopWidth: 0 },
  words: { flexGrow: 1, flexShrink: 1, minWidth: 0 },
  end: { flexShrink: 0, color: tokens.colour.secondaryLabel },
  image: { flexShrink: 0, borderRadius: tokens.radius.small, backgroundColor: tokens.colour.quinaryLabel },
  round: { borderRadius: tokens.radius.full },
  icon: { position: "relative", flexShrink: 0 },
  divider: { alignSelf: "stretch", height: tokens.length.hairline, marginBlock: tokens.grid[1], backgroundColor: tokens.colour.quaternaryLabel },
  // A widget's own button and switch.
  press: {
    alignItems: "center",
    justifyContent: "center",
    minHeight: tokens.grid[8],
    paddingInline: tokens.grid[3],
    borderRadius: tokens.radius.full,
    backgroundColor: { default: tokens.colour.quinaryLabel, hover: tokens.colour.quaternaryLabel },
    font: tokens.type.medium,
    fontWeight: tokens.weight.medium,
    cursor: "pointer",
    boxShadow: { focused: tokens.shadow.focus },
  },
  flip: { flexGrow: 1, minWidth: 0 },
  // A `gap` a layout says, in points, as the nearest step of the grid.
  gap0: { gap: 0 },
  gap2: { gap: tokens.grid[0.5] },
  gap4: { gap: tokens.grid[1] },
  gap8: { gap: tokens.grid[2] },
  gap10: { gap: tokens.grid[2.5] },
  gap12: { gap: tokens.grid[3] },
  gap16: { gap: tokens.grid[4] },
  gap20: { gap: tokens.grid[5] },
  gap24: { gap: tokens.grid[6] },
  gap32: { gap: tokens.grid[8] },
  // Words cut to so many `lines`.
  lines1: { lineClamp: 1 },
  lines2: { lineClamp: 2 },
  lines3: { lineClamp: 3 },
  lines4: { lineClamp: 4 },
  lines5: { lineClamp: 5 },
  lines6: { lineClamp: 6 },
  lines7: { lineClamp: 7 },
  lines8: { lineClamp: 8 },
})

const gaps = [[0, look.gap0], [2, look.gap2], [4, look.gap4], [8, look.gap8], [10, look.gap10], [12, look.gap12], [16, look.gap16], [20, look.gap20], [24, look.gap24], [32, look.gap32]] as const
const lines = [look.lines1, look.lines2, look.lines3, look.lines4, look.lines5, look.lines6, look.lines7, look.lines8]
const tones = { muted: look.muted, warn: look.warn, bad: look.bad } as Record<string, typeof look.muted | undefined>
const sizes = { xl: look.xl, lg: look.lg, sm: look.sm, xs: look.sm } as Record<string, typeof look.xl | undefined>
const weights = { medium: look.medium, bold: look.bold } as Record<string, typeof look.medium | undefined>

/** A widget's layout, as its tool answers it (the skill `building-apps` lists its parts): one part, which may hold
 * others. It's what an app said, so what's a number is read as one where it's used. */
export interface Layout {
  part?: string
  parts?: Layout[]
  title?: string | false
  tone?: string
  grow?: boolean
  from?: string
  dim?: boolean
  width?: unknown
  height?: unknown
  gap?: unknown
  align?: string
  text?: string
  size?: string
  weight?: string
  lines?: unknown
  at?: unknown
  style?: string
  value?: unknown
  unit?: string
  label?: string
  values?: unknown[]
  labels?: unknown[]
  max?: unknown
  kind?: string
  subtitle?: string
  trailing?: string
  url?: string
  round?: boolean
  path?: unknown
  badge?: unknown
  tool?: string
  inputs?: Record<string, unknown>
  on?: boolean
}

/** What a widget's button or switch calls: its app's tool, with its inputs. */
type OnCall = (tool: string | undefined, inputs: Record<string, unknown>) => unknown

/// A number from a layout, or `or` where it isn't one.
function number(value: unknown): number | undefined
function number(value: unknown, or: number): number
function number(value: unknown, or?: number) {
  return value === null || value === "" || !Number.isFinite(Number(value)) ? or : Number(value)
}
const share = (value: unknown) => Math.max(0, Math.min(1, number(value, 0)))
const figure = (value: unknown) => (typeof value === "number" ? value.toLocaleString(undefined, { maximumFractionDigits: 2 }) : String(value ?? "–"))
/** A coordinate, short. */
const at = (value: number) => Math.round(value * 100) / 100

/// When something was, as a widget says it.
function time(when: unknown, style?: string) {
  const date = new Date(number(when, 0))
  const ago = (Date.now() - date.getTime()) / 1000
  if (style === "date") return date.toLocaleDateString(undefined, { day: "numeric", month: "short" })
  if (style === "clock") return date.toLocaleTimeString(undefined, { hour: "numeric", minute: "2-digit" })
  const [count, unit]: [number, Intl.RelativeTimeFormatUnit] = Math.abs(ago) < 3600 ? [ago / 60, "minute"] : Math.abs(ago) < 86400 ? [ago / 3600, "hour"] : [ago / 86400, "day"]
  return new Intl.RelativeTimeFormat(undefined, { numeric: "auto", style: "short" }).format(-Math.round(count), unit)
}

/** What a widget's parts share: how wide the card is, which a column or row with `from` is dropped under; its tool's
 * caller; and what's told a press its own button or switch took, so the card doesn't open for it too. */
const Card = createContext<{ width: () => number | undefined; onCall?: OnCall; took: (event: Located) => void }>()

/// What every part may say of itself: how much room it takes, whether it's faded, and its tone; and its `width`.
const room = (part: Layout) => [!!part.grow && look.grow, !!part.dim && look.dim, tone(part), number(part.width) !== undefined && look.fixed]
const tone = (part: Layout) => (part.tone ? tones[part.tone] : undefined)
const sized = (part: Layout, height?: number) => [number(part.width) !== undefined && look.width(number(part.width)!), height !== undefined && look.height(height)]

/** Points along a box `width` by `height`, one for each value up to `max`, from the left edge to the right. */
const along = (values: number[], max: number, width: number, height: number) =>
  values.map((value, index) => [values.length > 1 ? (index / (values.length - 1)) * width : width / 2, height - 1 - (Math.min(value, max) / max) * (height - 2)] as const)

/** A line through `points`, `thick` across, as a shape to fill: its two edges, each a half's width out, mitred at
 * each turn. */
function band(points: readonly (readonly [number, number])[], thick: number) {
  if (points.length < 2) return ""
  const normal = (from: readonly [number, number], to: readonly [number, number]) => {
    const [dx, dy] = [to[0] - from[0], to[1] - from[1]]
    const length = Math.hypot(dx, dy) || 1
    return [-dy / length, dx / length] as const
  }
  const out = points.map((point, index) => {
    const before = index > 0 ? normal(points[index - 1], point) : undefined
    const after = index < points.length - 1 ? normal(point, points[index + 1]) : undefined
    const [a, b] = [before ?? after!, after ?? before!]
    const [x, y] = [a[0] + b[0], a[1] + b[1]]
    const length = Math.hypot(x, y) || 1
    // How far out the mitre is, no more than twice the half's width at a sharp turn.
    const mitre = Math.min(2, 1 / Math.max(0.5, (x / length) * a[0] + (y / length) * a[1])) * (thick / 2)
    return [(x / length) * mitre, (y / length) * mitre] as const
  })
  const edge = (side: 1 | -1, order: number[]) => order.map((index) => `${at(points[index][0] + side * out[index][0])} ${at(points[index][1] + side * out[index][1])}`)
  const indices = points.map((_, index) => index)
  return `M${[...edge(1, indices), ...edge(-1, indices.reverse())].join("L")}Z`
}

/** A rectangle with round corners, as a path. */
const rounded = (x: number, y: number, width: number, height: number, radius: number) => {
  const r = Math.max(0, Math.min(radius, width / 2, height / 2))
  return `M${at(x + r)} ${at(y)}H${at(x + width - r)}A${r} ${r} 0 0 1 ${at(x + width)} ${at(y + r)}V${at(y + height - r)}A${r} ${r} 0 0 1 ${at(x + width - r)} ${at(y + height)}H${at(x + r)}A${r} ${r} 0 0 1 ${at(x)} ${at(y + height - r)}V${at(y + r)}A${r} ${r} 0 0 1 ${at(x + r)} ${at(y)}Z`
}

/** A circle, as a path: clockwise, or with `back` the other way, which cuts it out of one around it. */
const circle = (x: number, y: number, radius: number, back = false) =>
  `M${at(x - radius)} ${at(y)}A${at(radius)} ${at(radius)} 0 1 ${back ? 0 : 1} ${at(x + radius)} ${at(y)}A${at(radius)} ${at(radius)} 0 1 ${back ? 0 : 1} ${at(x - radius)} ${at(y)}Z`

/// A box its shapes fill, drawn once it's laid out, in its own pixels, so a line is as thick however wide it is.
function Drawn(props: { label?: string; style?: Styles; draw: (width: number, height: number) => JSX.Element }) {
  const [size, setSize] = createSignal<Rect>()
  const laid = (event: Laid) => {
    const { width, height } = event.currentTarget.getBoundingClientRect()
    if (width !== size()?.width || height !== size()?.height) setSize({ x: 0, y: 0, width, height })
  }
  return (
    <View role={props.label ? "img" : undefined} aria-label={props.label} onLayout={laid} style={props.style}>
      <Show when={size()?.width && size()?.height ? size() : undefined}>{(box) => props.draw(box().width, box().height)}</Show>
    </View>
  )
}

function Spark(props: { part: Layout }) {
  const values = () => (Array.isArray(props.part.values) ? props.part.values.map((value) => number(value, 0)) : [])
  const max = () => number(props.part.max, Math.max(...values(), 1))
  return (
    <Drawn
      label={props.part.label}
      style={[look.drawn, number(props.part.width) === undefined && look.full, ...room(props.part), sized(props.part, number(props.part.height, 28))]}
      draw={(width, height) => (
        <Show
          when={values().length}
          fallback={<Svg viewBox={`0 0 ${at(width)} ${at(height)}`} style={look.over}><Path d={Array.from({ length: Math.ceil(width / 5) }, (_, index) => `M${index * 5} ${at(height / 2 - 0.75)}h2v1.5h-2Z`).join("")} /></Svg>}
        >
          <Svg viewBox={`0 0 ${at(width)} ${at(height)}`} style={[look.over, look.area]}><Path d={`M${along(values(), max(), width, height).map(([x, y]) => `${at(x)} ${at(y)}`).join("L")}L${at(width)} ${at(height)}L0 ${at(height)}Z`} /></Svg>
          <Svg viewBox={`0 0 ${at(width)} ${at(height)}`} style={look.over}><Path d={band(along(values(), max(), width, height), 1.5)} /></Svg>
        </Show>
      )}
    />
  )
}

function Chart(props: { part: Layout }) {
  const values = () => (Array.isArray(props.part.values) ? props.part.values.map((value) => number(value, 0)) : [])
  const max = () => number(props.part.max, Math.max(...values(), 1))
  /// Bars, each in its own slot, the labels' under it, at three fifths of its width.
  const bars = (width: number, height: number) => {
    const slot = width / Math.max(values().length, 1)
    return values()
      .map((value, index) => {
        const tall = (Math.min(value, max()) / max()) * height
        return tall > 0 ? rounded(index * slot + slot * 0.2, height - tall, slot * 0.6, tall, 3) : ""
      })
      .join("")
  }
  return (
    <View style={[look.chart, ...room(props.part), look.height(number(props.part.height, 70))]}>
      <Drawn
        label={props.part.label}
        style={look.bars}
        draw={(width, height) => (
          <Svg viewBox={`0 0 ${at(width)} ${at(height)}`} style={look.over}><Path d={props.part.kind === "line" ? band(along(values(), max(), width, height), 1.5) : bars(width, height)} /></Svg>
        )}
      />
      <Show when={Array.isArray(props.part.labels)}>
        <View style={look.labels}>
          <For each={props.part.labels} keyed={false}>
            {(label) => <Text style={look.label}>{String(label() ?? "")}</Text>}
          </For>
        </View>
      </Show>
    </View>
  )
}

/// A bar filled to a share: a pill, and over it what's filled, from its round start to a straight end.
function Bar(props: { width: number; height: number; value: number }) {
  const round = () => props.height / 2
  const filled = () => {
    const [end, r, width, height] = [props.value * props.width, round(), props.width, props.height]
    if (end <= 0) return ""
    if (end >= width - r) return rounded(0, 0, width, height, r)
    if (end <= r) return rounded(0, 0, end, height, end / 2)
    return `M${at(r)} 0H${at(end)}V${at(height)}H${at(r)}A${at(r)} ${at(r)} 0 0 1 ${at(r)} 0Z`
  }
  return (
    <>
      <Svg viewBox={`0 0 ${at(props.width)} ${at(props.height)}`} style={[look.over, look.track]}><Path d={rounded(0, 0, props.width, props.height, round())} /></Svg>
      <Svg viewBox={`0 0 ${at(props.width)} ${at(props.height)}`} style={look.over}><Path d={filled()} /></Svg>
    </>
  )
}

/// A ring filled to a share, no thicker than a sixth of itself, from the top, clockwise, with round ends.
function Gauge(props: { part: Layout }) {
  const size = () => number(props.part.width, 46)
  const thick = () => Math.min(5, size() / 6)
  const middle = () => size() / 2
  const point = (radius: number, turn: number) => [at(middle() + radius * Math.sin(turn * 2 * Math.PI)), at(middle() - radius * Math.cos(turn * 2 * Math.PI))].join(" ")
  const ring = () => `${circle(middle(), middle(), middle())}${circle(middle(), middle(), middle() - thick(), true)}`
  const filled = () => {
    const [part, outer, inner, mid] = [share(props.part.value), middle(), middle() - thick(), middle() - thick() / 2]
    if (part >= 1) return ring()
    if (part <= 0) return ""
    const large = part > 0.5 ? 1 : 0
    const end = (turn: number) => circle(middle() + mid * Math.sin(turn * 2 * Math.PI), middle() - mid * Math.cos(turn * 2 * Math.PI), thick() / 2)
    return `M${point(outer, 0)}A${outer} ${outer} 0 ${large} 1 ${point(outer, part)}L${point(inner, part)}A${at(inner)} ${at(inner)} 0 ${large} 0 ${point(inner, 0)}Z${end(0)}${end(part)}`
  }
  return (
    <View role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share(props.part.value) * 100)} aria-label={props.part.label} style={[look.gauge, tone(props.part), look.square(size())]}>
      <Svg viewBox={`0 0 ${size()} ${size()}`} style={[look.over, look.track]}><Path d={ring()} /></Svg>
      <Svg viewBox={`0 0 ${size()} ${size()}`} style={look.over}><Path d={filled()} /></Svg>
      <Show when={props.part.label && size() >= 36}>
        <View style={look.middle}>
          <Text style={look.meterLabel}>{props.part.label}</Text>
        </View>
      </Show>
    </View>
  )
}

/// A button or a switch calls its app's tool, and shows that it's under way until the widget is drawn again.
function acts(props: { part: Layout }) {
  const card = useContext(Card)!
  const [busy, setBusy] = createSignal(false)
  const act = async (inputs: Record<string, unknown>) => {
    setBusy(true)
    try {
      await card.onCall?.(props.part.tool, inputs)
    } finally {
      setBusy(false)
    }
  }
  return [busy, act, card.took] as const
}

function Press(props: { part: Layout }) {
  const [busy, act, took] = acts(props)
  return (
    <Pressable
      role="button"
      aria-disabled={busy() ? "true" : undefined}
      aria-busy={busy() ? "true" : undefined}
      onPress={(event: Located) => (took(event), !busy() && act(props.part.inputs ?? {}))}
      style={[look.press, ...room(props.part), sized(props.part)]}
    >
      <Show when={busy()} fallback={<Text>{props.part.label}</Text>}>
        <Spinner />
      </Show>
    </Pressable>
  )
}

function Flip(props: { part: Layout }) {
  const [busy, act, took] = acts(props)
  // Its own box takes the press first on the DOM, where it then goes on to the card, which leaves it be.
  return (
    <Pressable tabIndex={-1} onPress={took} style={[look.flip, ...room(props.part)]}>
      <Switch label={props.part.label} value={!!props.part.on} disabled={busy()} onValueChange={(on) => act({ ...(props.part.inputs ?? {}), on })} />
    </Pressable>
  )
}

/// One part of a layout. A part the UI library doesn't have draws nothing. A widget's tool answers its whole layout
/// anew each time it's asked, so parts are kept by where they are, not by which object answered them: each stays, and
/// only what changed in it is drawn again. `row` says it's in a row, where words keep to one line; `first`, that it's
/// the first in what it's in.
function Part(props: { part: Layout; root?: boolean; row?: boolean; first?: boolean }) {
  const card = useContext(Card)!
  const part = () => props.part
  // A column or row that says `from` is dropped where the widget is narrower than that, by the widget's own width,
  // never the window's: what doesn't fit goes whole, rather than being cut off.
  const fits = () => !part().from || (card.width() ?? 0) >= (part().from === "md" ? 288 : part().from === "sm" ? 240 : 0)
  const gap = () => {
    const points = number(part().gap)
    return points === undefined ? undefined : gaps.reduce((best, step) => (Math.abs(step[0] - points) < Math.abs(best[0] - points) ? step : best))[1]
  }
  const clamp = () => (props.row ? look.line : number(part().lines) ? lines[Math.min(8, Math.max(1, Math.round(number(part().lines)!))) - 1] : undefined)
  const words = () => [look.text, part().size ? sizes[part().size!] : undefined, part().weight ? weights[part().weight!] : undefined, clamp(), ...room(part()), sized(part())]
  return (
    <Which>
      <Match when={(part().part === "column" || part().part === "row") && fits()}>
        <View style={[part().part === "row" ? look.row : look.column, props.root && part().part === "column" && look.root, gap(), part().align === "center" && look.centre, ...room(part()), sized(part())]}>
          <For each={Array.isArray(part().parts) ? part().parts! : []} keyed={false}>
            {(inside, index) => <Part part={inside()} row={part().part === "row"} first={index === 0} />}
          </For>
        </View>
      </Match>
      <Match when={part().part === "text"}>
        <Text style={words()}>{part().text ?? ""}</Text>
      </Match>
      <Match when={part().part === "time"}>
        <Text style={words()}>{time(part().at, part().style)}</Text>
      </Match>
      <Match when={part().part === "stat"}>
        <View style={[look.stat, ...room(part())]}>
          <View style={look.figure}>
            <Text style={look.value}>{figure(part().value)}</Text>
            <Show when={part().unit}>
              <Text style={look.unit}>{part().unit}</Text>
            </Show>
          </View>
          <Show when={part().label}>
            <Text style={look.small}>{part().label}</Text>
          </Show>
        </View>
      </Match>
      <Match when={part().part === "gauge"}>
        <Gauge part={part()} />
      </Match>
      <Match when={part().part === "progress"}>
        <View style={[look.progress, ...room(part()), sized(part())]}>
          <Show when={part().label}>
            <Text style={look.small}>{part().label}</Text>
          </Show>
          <View role="progressbar" aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(share(part().value) * 100)} aria-label={part().label} style={look.bar}>
            <Drawn style={look.over} draw={(width, height) => <Bar width={width} height={height} value={share(part().value)} />} />
          </View>
        </View>
      </Match>
      <Match when={part().part === "spark"}>
        <Spark part={part()} />
      </Match>
      <Match when={part().part === "chart"}>
        <Chart part={part()} />
      </Match>
      <Match when={part().part === "item"}>
        <View style={[look.item, props.first && look.first, ...room(part())]}>
          <View style={look.words}>
            <Text style={look.line}>{part().title || ""}</Text>
            <Show when={part().subtitle}>
              <Text style={look.small}>{part().subtitle}</Text>
            </Show>
          </View>
          <Show when={part().at || part().trailing}>
            <Text style={look.end}>{part().at ? time(part().at) : part().trailing}</Text>
          </Show>
        </View>
      </Match>
      <Match when={part().part === "image"}>
        <Show
          when={/^data:image\//.test(part().url ?? "") && part().url}
          fallback={<View style={[look.image, part().round && look.round, number(part().width) === undefined && look.full, sized(part(), number(part().height))]} />}
        >
          {(url) => <Image src={url()} alt="" style={[look.image, part().round && look.round, number(part().width) === undefined && look.full, sized(part(), number(part().height))]} />}
        </Show>
      </Match>
      <Match when={part().part === "icon"}>
        <View aria-hidden="true" style={[look.icon, tone(part()), look.square(number(part().width, 16))]}>
          <Show when={part().path}>
            <Svg viewBox="0 0 24 24" style={look.over}><Path d={String(part().path)} /></Svg>
          </Show>
          {/* A badge, at the corner of what it's on, or the whole icon. */}
          <Show when={part().badge}>
            <Svg viewBox="0 0 24 24" style={[look.over, part().path ? look.corner((number(part().width, 16) * 13) / 24, (number(part().width, 16) * 13) / 24, (number(part().width, 16) * 11) / 24, (number(part().width, 16) * 11) / 24) : undefined]}><Path d={String(part().badge)} /></Svg>
          </Show>
        </View>
      </Match>
      <Match when={part().part === "divider"}>
        <View role="separator" style={look.divider} />
      </Match>
      <Match when={part().part === "button"}>
        <Press part={part()} />
      </Match>
      <Match when={part().part === "switch"}>
        <Flip part={part()} />
      </Match>
    </Which>
  )
}

/// `<Widget name="Weather" size="2x2" layout={layout()} onCall={(tool, inputs) => call(tool, inputs)} onOpen={open} />`:
/// an app's widget, as a card: the `layout` its tool answered, drawn from the parts a layout is made of (the
/// skill `building-apps` lists them), under its `name`, at a `size` of `2x2`, `4x2` or `4x4`. A button or a
/// switch in it calls `onCall` with its tool and inputs, and pressing the card anywhere else calls `onOpen` with where
/// the card is drawn (`getBoundingClientRect()`), which its app grows out of. With a `problem` it says the widget
/// stopped working, and why. With no layout yet it's a grey card.
export function Widget(props: {
  name?: string
  size?: "2x2" | "4x2" | "4x4"
  layout?: Layout
  problem?: string
  onCall?: OnCall
  onOpen?: (card: { getBoundingClientRect(): Rect }) => void
}) {
  const [width, setWidth] = createSignal<number>()
  // On the DOM a press goes on from what took it to what it's in, as a native host's doesn't: the card leaves be the
  // one its own button, switch or open button took.
  let taken: Located | undefined
  const took = (event: Located) => void (taken = event)
  const open = (event: Located) => {
    if (event === taken) return
    took(event)
    const card = event.currentTarget
    props.onOpen?.(card)
  }
  return (
    <Pressable
      tabIndex={-1}
      onPress={props.onOpen ? open : undefined}
      onLayout={(event: Laid) => setWidth(event.currentTarget.getBoundingClientRect().width)}
      style={[look.card, props.onOpen && look.opens]}
    >
      <Show when={props.onOpen}>
        <Pressable role="button" aria-label={props.name} onPress={open} style={look.open} />
      </Show>
      <Card value={{ width, onCall: props.onCall, took }}>
        <Show when={props.problem || props.layout?.title !== false}>
          <Text style={look.name}>{props.name}</Text>
        </Show>
        <Show when={!props.problem} fallback={<Text style={look.problem}>This widget stopped working. {props.problem}</Text>}>
          <Show when={props.layout}>{(layout) => <Part part={layout()} root />}</Show>
        </Show>
      </Card>
    </Pressable>
  )
}
