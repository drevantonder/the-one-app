// The strict primitives, as the DOM draws them: a box, a text and an image, each with the semantic props and the style
// subset the strict protocol has (protocol.ts). The fourth, an input, is the UI library's `Input`. A native host draws
// the same props from the ops Solid's universal renderer sends it.
import { omit } from "solid-js"
import { Dynamic, render as mount, type JSX } from "@solidjs/web"
import type { Props, Style } from "./protocol.ts"

/// `render(() => <App />)`: draws an app's first screen, where its page is: the page itself on the web, the app's
/// window on a native host. → what takes it away again.
export const render = (code: () => JSX.Element): (() => void) => mount(code, document.body)

/** What a primitive takes of the protocol's props: its role and states, with what it listens for. */
type Semantics = Pick<Props, Extract<keyof Props, "role" | `aria-${string}`>>

/** A box's props. */
export interface BoxProps extends Semantics {
  style?: Style
  onClick?: () => void
  onFocus?: () => void
  onBlur?: () => void
  children?: JSX.Element
}

/// `<Box style={{ flexDirection: "row", gap: "gap", padding: 12, backgroundColor: "muted", borderRadius: "lg" }}>...</Box>`:
/// a strict app's own layout, around anything else: a flex column unless its `style` says otherwise, as React Native's
/// View, styled from the design tokens only (DESIGN.md's strict subset). With `role="button"` and `onClick` it's
/// something to press, which the keyboard reaches too; any other `role` and `aria-…` say what it is to a screen
/// reader and to an agent, as ARIA has them.
export function Box(props: BoxProps) {
  const own = omit(props, "style", "children")
  return (
    <Dynamic component={props.role === "button" ? "button" : "div"} type={props.role === "button" ? "button" : undefined} {...own} class="ui-strict-box" data-font={props.style?.font} style={css(props.style)}>
      {props.children}
    </Dynamic>
  )
}

/** A text's props. */
export interface TextProps extends Semantics {
  style?: Style
  children?: JSX.Element
}

/// `<Text style={{ font: "title" }}>Recently Deleted</Text>`: words, in one of the type styles (`font`: `body`,
/// `label`, `title`, `figure`, `subheadline`, `footnote`, `caption-1`, `caption-2`) and a token's `color`, or as
/// what it's in has them. It runs on in a text it's in, and is a block in a box.
/// `role="heading"` with `aria-level` makes it a heading. A strict app's words are always in a `Text` or in a UI
/// library component that takes them.
export function Text(props: TextProps) {
  const own = omit(props, "style", "children")
  const tag = () => (props.role === "heading" ? `h${props["aria-level"] ?? 2}` : "span")
  return (
    <Dynamic component={tag()} {...own} role={props.role === "heading" ? undefined : props.role} class="ui-strict-text" data-font={props.style?.font} style={css(props.style)}>
      {props.children}
    </Dynamic>
  )
}

/** An image's props. */
export interface ImageProps extends Pick<Semantics, "aria-hidden"> {
  /** A `data:` address, or a file of the app's own by its path. */
  src: string
  /** What it shows, which names it; `""` for one that's only how it looks. */
  alt: string
  style?: Style
}

/// `<Image src="icon.svg" alt="" style={{ width: 24, height: 24 }} />`: a picture, from a `data:` address or the app's
/// own files, named by `alt`.
export function Image(props: ImageProps) {
  return <img src={props.src} alt={props.alt} aria-hidden={props["aria-hidden"]} class="ui-strict-image" style={css(props.style)} />
}

const colours = new Set(["backgroundColor", "color", "borderColor"])
const spaces = new Set(["gap", "rowGap", "columnGap", "padding", "paddingInline", "paddingBlock", "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "margin", "marginInline", "marginBlock"])
const sizes = new Set(["flexBasis", "width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight"])
const tokens: Record<string, string> = { gap: "var(--gap)", gutter: "var(--gutter)", control: "var(--control)", icon: "var(--icon)", readable: "var(--readable)" }

/** A strict style as CSS: its tokens as the custom properties they name, its pixels as `px`. Its `font` is the
 * element's `data-font` (ui.css). */
export function css(style: Style | undefined): JSX.CSSProperties | undefined {
  if (!style) return
  const out: Record<string, string | number> = {}
  for (const [name, value] of Object.entries(style) as [keyof Style, string | number][]) {
    const property = name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)
    if (name === "font" || value === undefined) continue
    else if (colours.has(name)) out[property] = `var(--${value})`
    else if (name === "borderRadius") out[property] = value === "full" ? "9999px" : value ? `var(--radius-${value})` : 0
    else if (name.startsWith("border")) out[property] = `${value}px`
    // What scrolls has no scrollbar drawn beside it, as on a phone and on a native host, which draw one over it as it moves.
    else if (name === "overflow") Object.assign(out, value === "scroll" ? { overflow: "auto", "scrollbar-width": "none" } : { overflow: value })
    else if (name === "lineClamp") Object.assign(out, { display: "-webkit-box", "-webkit-box-orient": "vertical", "-webkit-line-clamp": value, overflow: "hidden" })
    else if (spaces.has(name) || sizes.has(name)) out[property] = typeof value === "number" ? `${value}px` : (tokens[value] ?? value)
    else out[property] = value
  }
  return out
}
