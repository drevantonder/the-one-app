// How a strict page and the UI library's components look: styles, static and typed, beside what uses them, whose
// values are design tokens (tokens.ts). The build resolves each `styles({ … })` ahead of time (core/ui/tables.ts): to
// atomic CSS classes on the web and to a style table for a native host, each style an id. Nothing about a style is
// worked out as a page runs but which apply: a node's `style` is a list of them, and the last to say a property is
// all it is, with no cascade and no specificity.
import type { Inline, Properties, State } from "./strict/renderer.ts"
import type { Token } from "./tokens.ts"

/** A token of one kind, or its negative (`negative(tokens.space.lg)`, a margin pulling outward). */
type Of<Kind extends string> = Token<Kind>
type Pull = { token: Token<"length">; negative: true }
type Length = Of<"length"> | 0
type Size = Length | `${number}%` | "auto"

/** What a style may say, as React Native names it, with its values by kind of token: a colour is a colour token, a
 * space or a size a length token (or a percentage, `auto` or 0), a corner a radius token, and so on. No raw colour,
 * length or font: the types refuse one. `start` and `end` are the line's, left and right where text runs left to
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
  backgroundColor: Of<"colour">
  color: Of<"colour">
  borderColor: Of<"colour">
  borderWidth: Length
  borderTopWidth: Length
  borderBottomWidth: Length
  borderStartWidth: Length
  borderEndWidth: Length
  borderStyle: Properties["borderStyle"]
  borderRadius: Of<"radius"> | 0
  borderTopStartRadius: Of<"radius"> | 0
  borderTopEndRadius: Of<"radius"> | 0
  borderBottomStartRadius: Of<"radius"> | 0
  borderBottomEndRadius: Of<"radius"> | 0
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
  fill: Of<"colour"> | "currentColor" | "none"
  stroke: Of<"colour"> | "currentColor" | "none"
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
 * take only styles that say what it lets a caller change (`Styled<"marginTop" | "alignSelf">`). */
export type Styled<P extends keyof Look = keyof Look> = {
  readonly $id: string
  readonly properties?: P
}

export type { Inline }

/** What a primitive's `style` takes: styles, inline ones (`{ translateY: 40 }`, worked out as the page runs), and a
 * condition's `false` or `undefined` for one that doesn't apply. */
export type Styles<P extends keyof Look = keyof Look> = Styled<P> | Inline | false | null | undefined | readonly Styles<P>[]

/// `const look = styles({ row: { flexDirection: "row", gap: tokens.space.md, backgroundColor: { default:
/// tokens.colour.card, hover: tokens.colour.muted } } })`, then `<Box style={[look.row, open() && look.open]} />`:
/// styles, beside what they style, whose values are tokens, never raw colours, sizes or fonts. The build makes them
/// ahead of time, so its argument is written out: an object of literals, `tokens.…` and `negative(…)`, nothing
/// else. The last style in a list to say a property is all it is. A component takes a caller's with a typed prop
/// that names what it may change: `style?: Styles<"marginTop" | "alignSelf">`.
export function styles<const T extends Record<string, Style>>(_sheet: T): { [K in keyof T]: Styled<keyof T[K] & keyof Look> } {
  throw new Error("styles() is made by the build (core/ui/tables.ts), as a page is built: it doesn't run")
}

/** A length's negative: a margin, an inset or a move pulling the other way. */
export const negative = (token: Token<"length">): Pull => ({ token, negative: true })

/** The styles that apply, in order, as one list: what a list of `Styles` says, its falsy ones left out. */
export function flat(style: Styles | undefined, into: (Styled | Inline)[] = []): (Styled | Inline)[] {
  if (Array.isArray(style)) for (const one of style) flat(one, into)
  else if (style) into.push(style as Styled | Inline)
  return into
}

/** `styles` on the web: their classes, the last to say a property winning, and their inline styles as CSS. */
export function web(style: Styles | undefined): { class: string; style?: Record<string, string> } {
  const by: Record<string, string> = {}
  const inline: Record<string, string> = {}
  for (const one of flat(style)) {
    if ("$id" in one) for (const [property, names] of Object.entries(one)) property !== "$id" && property !== "properties" && (by[property] = names as string)
    else for (const [property, value] of Object.entries(one)) {
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
