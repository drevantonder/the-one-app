// How a page and the UI library's components look: styles, static and typed, beside what uses them, whose
// values are design tokens (tokens.ts). The build resolves each `styles({ … })` ahead of time (core/ui/tables.ts): to
// atomic CSS classes on the web and to a style table for a native host, each style an id. Nothing about a style is
// worked out as a page runs but which apply: a node's `style` is a list of them, and the last to say a property is
// all it is, with no cascade and no specificity.
import type { Dynamic, Properties, State, Varies } from "./drawing-primitives/renderer.ts"
import type { Token } from "./tokens.ts"

/** A token of one kind, or its negative (`negative(tokens.space.lg)`, a margin pulling outward). */
type Of<Kind extends string> = Token<Kind>
type Pull = { token: Token<"length">; negative: true }
/** A colour as data has it: a hex string (`#ff8800`, `#f80`, `#ff880080`). */
type Hex = `#${string}`
/** A length: a token, or a number of pixels, which in a static style is a warning ("make it a token") and in a
 * dynamic one is the point. */
type Length = Of<"length"> | number
type Size = Length | `${number}%` | "auto"
type Colour = Of<"colour"> | Hex
type Radius = Of<"radius"> | number

/** What a style may say, as React Native names it, with its values by kind of token: a colour is a colour token, a
 * space or a size a length token (or a percentage or `auto`), a corner a radius token, and so on. A raw colour or
 * length is a value of its own, which the build warns about in a static style, as it's one that should be a token, and
 * takes as it is in a dynamic one (`styles()`). `start` and `end` are the line's, left and right where text runs left to
 * right. Shorthands (`padding`, `paddingInline`, `gap`, `borderWidth`, `borderRadius`, `inset`) are written out by the
 * build, so each side is a property of its own. */
export interface Look {
  display: Properties["display"]
  flexDirection: Properties["flexDirection"]
  flexWrap: Properties["flexWrap"]
  flexGrow: number
  flexShrink: number
  flexBasis: Size
  alignItems: Properties["alignItems"]
  alignSelf: Properties["alignSelf"]
  alignContent: Properties["alignContent"]
  justifyContent: Properties["justifyContent"]
  gridTemplateColumns: { repeat: number } | { fill: Of<"length"> }
  gridColumn: { span: number }
  gap: Length
  rowGap: Length
  columnGap: Length
  padding: Length
  paddingInline: Length
  paddingBlock: Length
  paddingTop: Length
  paddingBottom: Length
  paddingStart: Length
  paddingEnd: Length
  margin: Size | Pull
  marginInline: Size | Pull
  marginBlock: Size | Pull
  marginTop: Size | Pull
  marginBottom: Size | Pull
  marginStart: Size | Pull
  marginEnd: Size | Pull
  width: Size
  height: Size
  minWidth: Size
  minHeight: Size
  maxWidth: Size | "none"
  maxHeight: Size | "none"
  aspectRatio: number
  position: Properties["position"]
  inset: Size | Pull
  top: Size | Pull
  bottom: Size | Pull
  start: Size | Pull
  end: Size | Pull
  zIndex: number
  overflow: Properties["overflow"]
  translateX: Size | Pull
  translateY: Size | Pull
  scale: number
  rotate: number
  opacity: number
  backgroundColor: Colour
  color: Colour
  borderColor: Colour
  borderWidth: Length
  borderTopWidth: Length
  borderBottomWidth: Length
  borderStartWidth: Length
  borderEndWidth: Length
  borderStyle: Properties["borderStyle"]
  borderRadius: Radius
  borderTopStartRadius: Radius
  borderTopEndRadius: Radius
  borderBottomStartRadius: Radius
  borderBottomEndRadius: Radius
  boxShadow: Of<"shadow"> | "none"
  backdropFilter: Of<"material"> | "none"
  /** A type style: size, line height, weight, tracking and figures together. */
  font: Of<"type">
  fontFamily: Of<"font">
  fontWeight: Of<"weight">
  textAlign: Properties["textAlign"]
  lineClamp: number
  whiteSpace: Properties["whiteSpace"]
  textDecorationLine: Properties["textDecorationLine"]
  fill: Colour | "currentColor" | "none"
  stroke: Colour | "currentColor" | "none"
  strokeWidth: number
  cursor: Properties["cursor"]
  pointerEvents: Properties["pointerEvents"]
  /** Where a press moves the window rather than reaching the page, as CSS's `app-region` (Chromium's, Electron's):
   * `drag` for a window with no title bar of its own, a launcher's panel, and `no-drag` for what's in it that isn't. */
  appRegion: "drag" | "no-drag"
  touchAction: Properties["touchAction"]
  userSelect: Properties["userSelect"]
  transitionProperty: (keyof Properties)[]
  transition: Of<"motion">
  /** How fast what a spring `transition` moves sets off, as a fraction of the way it goes a second, as UIKit's
   * `initialSpringVelocity` is: a fling's speed, which the spring then keeps, where one says it. A change it moves
   * while this is said sets off at it; leave it out for one that sets off from rest, or at the speed it has. */
  transitionVelocity: number
  /** `spin`, `pulse`, or keyframes, as CSS's `@keyframes`: a style by where each is (`from`, `50%`, `to`). */
  animationName: "spin" | "pulse" | "none" | { [at in "from" | "to" | `${number}%`]?: { [P in keyof Look]?: Look[P] } }
  animation: Of<"motion">
  /** What of it is about to move, grow or fade, as CSS's `will-change`: a host draws it ahead, as a layer of its
   * own, so it moves from the first frame without being drawn then. Only for what's about to, as it holds memory. */
  willChange: (keyof Properties)[]
}

/** A style as it's written: each property a value, or a value by the node's state (`{ default, hover, pressed }`). */
export type Style = { [P in keyof Look]?: Look[P] | ({ default?: Look[P] } & { [S in State]?: Look[P] }) }

/** A style as the build made it: its id, and its CSS classes by property. `P` is what it says, so a component can
 * take only styles that say what it lets a caller change (`Styled<"marginTop" | "alignSelf">`). A dynamic style's call
 * has the values it was given too (`$dynamic`), which are CSS custom properties on the web and what a host adds to its
 * style, by property. */
export type Styled<P extends keyof Look = keyof Look> = {
  readonly $id: string
  readonly $dynamic?: Dynamic
  /** A style whose `transitionVelocity` is a value: its spring's duration and bounce, for the web's easing. */
  readonly $spring?: readonly [duration: number, bounce: number]
  readonly properties?: P
}

/** What a primitive's `style` takes: styles, and a condition's `false` or `undefined` for one that doesn't apply. */
export type Styles<P extends keyof Look = keyof Look> = Styled<P> | false | null | undefined | readonly Styles<P>[]

/** What `styles()` takes: each style an object, or a function of its values that returns one (`(height: number) => ({
 * height })`), whose values are its parameters, as they are. */
type Sheet = Record<string, Style | ((...values: never[]) => Style)>

/// `const look = styles({ row: { flexDirection: "row", gap: tokens.space.md, backgroundColor: { default:
/// tokens.colour.card, hover: tokens.colour.muted } }, bar: (height: number, colour: `#${string}`) => ({ height,
/// backgroundColor: colour }) })`, then `<View style={[look.row, open() && look.open, look.bar(share() * 80,
/// item.colour)]} />`: styles, beside what they style, whose values are tokens, never raw colours, sizes or fonts (a raw
/// one is a warning, "make it a token"). A function is a dynamic style, as StyleX's are: a style of its values, which
/// are what it's called with as it's called (a height as a number moves, a colour from data), and nothing else about it
/// changes. Each value in what it returns is a parameter as it is, or a token or a literal; a value worked out of one
/// is worked out where it's called. The build makes them ahead of time, so its argument is written out: an object of
/// literals, `tokens.…`, `negative(…)`, an app's own tokens (`defineVars`) and those functions, nothing else. The last
/// style in a list to say a property is all it is. A component takes a caller's with a typed prop that names what it
/// may change: `style?: Styles<"marginTop" | "alignSelf">`.
export function styles<const T extends Sheet>(_sheet: T): { [K in keyof T]: T[K] extends (...values: infer V) => infer R ? (...values: V) => Styled<keyof R & keyof Look> : Styled<keyof T[K] & keyof Look> } {
  throw new Error("styles() is made by the build (core/ui/tables.ts), as a page is built: it doesn't run")
}

/** A token's value by condition, as `defineVars` and `createTheme` take it: one for every condition, or `{ default,
 * dark }` (a colour's light and dark, and a length's `touch` and `phone` too). */
type Value<T> = Varies<T>
type Defined<T> = { [K in keyof T]: T[K] extends Value<number> ? Token<"length"> : Token<"colour"> }

/// `export const brand = defineVars({ accent: { default: "#ff5500", dark: "#ff8844" }, tile: 72 })`, then `tokens` of
/// an app's own, which `styles()` takes as it does ours (`backgroundColor: brand.accent`): a colour is a hex colour, the
/// same in light and dark or `{ default, dark }`, and a length a number of pixels (`{ default, touch, phone }` where it
/// varies). As StyleX's `defineVars`: written out in a module of the app's, as named exports, which its other modules
/// import. Both hosts draw them the same.
export function defineVars<const T extends Record<string, Value<Hex> | Value<number>>>(_vars: T): Defined<T> {
  throw new Error("defineVars() is made by the build (core/ui/tables.ts), as a page is built: it doesn't run")
}

/// `createTheme(tokens.colour, { primary: { default: "#0a7a3f", dark: "#3ddc84" } })`: our tokens (`tokens.colour`,
/// `tokens.length`) or an app's own (`defineVars`) as its page has them, each named one by its values: StyleX's
/// `createTheme`, for the app's whole page, not a part of it. What's drawn at a part of one's opacity follows (the label's
/// 60% is the new label at 60%). Written out, at the top of a module of the app's.
export function createTheme<T extends Record<string, Token<"colour"> | Token<"length">>>(_vars: T, _values: { [K in keyof T]?: T[K] extends Token<"colour"> ? Value<Hex> : Value<number> }): void {
  throw new Error("createTheme() is made by the build (core/ui/tables.ts), as a page is built: it doesn't run")
}

/** A length's negative: a margin, an inset or a move pulling the other way. */
export const negative = (token: Token<"length">): Pull => ({ token, negative: true })

/** The styles that apply, in order, as one list: what a list of `Styles` says, its falsy ones left out. */
export function flat(style: Styles | undefined, into: Styled[] = []): Styled[] {
  if (Array.isArray(style)) for (const one of style) flat(one, into)
  else if (style) into.push(style as Styled)
  return into
}

/** Properties whose numbers aren't lengths. */
const unitless = new Set(["flexGrow", "flexShrink", "opacity", "zIndex", "aspectRatio", "scale", "strokeWidth", "lineClamp"])

/** A dynamic value as CSS has it: pixels for a length, degrees for `rotate`, a number as it is where the property
 * takes one, and a string (a hex colour, `50%`, `auto`) as it is. */
export const cssValue = (property: string, value: number | string): string =>
  typeof value === "string" ? value : property === "rotate" ? `${value}deg` : unitless.has(property) ? String(value) : `${value}px`

/** `styles` on the web: their classes, the last to say a property winning, and their inline styles as CSS. A spring
 * set off at a speed (`transitionVelocity`) is its own easing, as it's given. */
export function web(style: Styles | undefined): { class: string; style?: Record<string, string> } {
  const by: Record<string, string> = {}
  const inline: Record<string, string> = {}
  for (const one of flat(style)) {
    for (const [property, names] of Object.entries(one)) !property.startsWith("$") && property !== "properties" && (by[property] = names as string)
    for (const [property, value] of Object.entries(one.$dynamic ?? {})) {
      if (property === "transitionVelocity" && one.$spring && typeof value === "number") {
        const { settles, easing } = sprung(one.$spring[0], one.$spring[1], value)
        Object.assign(inline, { "transition-duration": `${settles}ms`, "transition-timing-function": easing })
      } else inline[`--${one.$id}-${property}`] = cssValue(property, value)
    }
  }
  return { class: Object.values(by).join(" "), style: Object.keys(inline).length ? inline : undefined }
}

/** A spring from 0 to 1, as SwiftUI's `.spring(duration:bounce:)` moves (its stiffness and damping, a mass of 1),
 * setting off at `velocity` (of the way, a second), as CSS draws it: how long it takes to settle, in milliseconds, and
 * its path then as a `linear()` easing. CSS starts each change from rest, so one turned back midway loses the speed it
 * had, which a native host keeps. */
export function sprung(duration: number, bounce: number, velocity = 0) {
  const response = duration / 1000
  const stiffness = (2 * Math.PI / response) ** 2
  const damping = (4 * Math.PI * (bounce >= 0 ? 1 - bounce : 1 / (1 + bounce))) / response
  const path = [0]
  let [x, v, t] = [0, velocity, 0]
  const step = 1 / 1000
  while (t < 5 && (Math.abs(1 - x) > 0.001 || Math.abs(v) > 0.01)) {
    v += (stiffness * (1 - x) - damping * v) * step
    x += v * step
    t += step
    path.push(x)
  }
  const points = 40
  const easing = Array.from({ length: points + 1 }, (_, at) => +(at === points ? 1 : path[Math.round((at / points) * (path.length - 1))]).toFixed(4))
  return { settles: Math.round(t * 1000), easing: `linear(${easing.join(", ")})` }
}
