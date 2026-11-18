// How a strict page and the UI library's components look: styles, static and typed, beside what uses them, whose
// values are design tokens (tokens.ts). The build resolves each `styles({ … })` ahead of time (core/ui/tables.ts): to
// atomic CSS classes on the web and to a style table for a native host, each style an id. Nothing about a style is
// worked out as a page runs but which apply: a node's `style` is a list of them, and the last to say a property is
// all it is, with no cascade and no specificity.
import type { Dynamic, Inline, Properties, State, Varies } from "./strict/renderer.ts"
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
  touchAction: Properties["touchAction"]
  userSelect: Properties["userSelect"]
  transitionProperty: (keyof Properties)[]
  transition: Of<"motion">
  animationName: Properties["animationName"]
  animation: Of<"motion">
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
  readonly properties?: P
}

export type { Inline }

/** What a primitive's `style` takes: styles, inline ones (`{ translateY: 40 }`, worked out as the page runs), and a
 * condition's `false` or `undefined` for one that doesn't apply. */
export type Styles<P extends keyof Look = keyof Look> = Styled<P> | Inline | false | null | undefined | readonly Styles<P>[]

/** What `styles()` takes: each style an object, or a function of its values that returns one (`(height: number) => ({
 * height })`), whose values are its parameters, as they are. */
type Sheet = Record<string, Style | ((...values: never[]) => Style)>

/// `const look = styles({ row: { flexDirection: "row", gap: tokens.space.md, backgroundColor: { default:
/// tokens.colour.card, hover: tokens.colour.muted } }, bar: (height: number, colour: `#${string}`) => ({ height,
/// backgroundColor: colour }) })`, then `<Box style={[look.row, open() && look.open, look.bar(share() * 80,
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
export function flat(style: Styles | undefined, into: (Styled | Inline)[] = []): (Styled | Inline)[] {
  if (Array.isArray(style)) for (const one of style) flat(one, into)
  else if (style) into.push(style as Styled | Inline)
  return into
}

/** Properties whose numbers aren't lengths. */
const unitless = new Set(["flexGrow", "flexShrink", "opacity", "zIndex", "aspectRatio", "scale", "strokeWidth", "lineClamp"])

/** A dynamic value as CSS has it: pixels for a length, degrees for `rotate`, a number as it is where the property
 * takes one, and a string (a hex colour, `50%`, `auto`) as it is. */
export const cssValue = (property: string, value: number | string): string =>
  typeof value === "string" ? value : property === "rotate" ? `${value}deg` : unitless.has(property) ? String(value) : `${value}px`

/** `styles` on the web: their classes, the last to say a property winning, and their inline styles as CSS. */
export function web(style: Styles | undefined): { class: string; style?: Record<string, string> } {
  const by: Record<string, string> = {}
  const inline: Record<string, string> = {}
  for (const one of flat(style)) {
    if ("$id" in one) {
      for (const [property, names] of Object.entries(one)) property !== "$id" && property !== "$dynamic" && property !== "properties" && (by[property] = names as string)
      for (const [property, value] of Object.entries(one.$dynamic ?? {})) inline[`--${one.$id}-${property}`] = cssValue(property, value)
    } else for (const [property, value] of Object.entries(one)) {
      const css = property === "start" ? "inset-inline-start" : property === "end" ? "inset-inline-end" : property
      if (property === "translateX") inline["--ui-x"] = `${value}px`
      else if (property === "translateY") inline["--ui-y"] = `${value}px`
      else if (property === "rotate") inline.rotate = `${value}deg`
      else inline[css] = property === "scale" || property === "opacity" ? String(value) : `${value}px`
      if (property === "translateX" || property === "translateY") inline.translate = "var(--ui-x, 0) var(--ui-y, 0)"
    }
  }
  return { class: Object.values(by).join(" "), style: Object.keys(inline).length ? inline : undefined }
}
