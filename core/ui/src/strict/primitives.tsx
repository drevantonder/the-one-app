// The strict primitives, as the DOM draws them: a box, a scroll, a text, an image and a path, each a plain element with
// the semantic props the strict protocol has (protocol.ts) and its `class`, in the strict style vocabulary (style.ts),
// which Tailwind draws from the tokens on what's `ui-strict` (tailwind.js). The input is the UI library's `Input`. A native host draws the
// same props from the ops Solid's universal renderer sends it. Version 1's `style` is drawn as inline CSS.
import { omit } from "solid-js"
import { Dynamic, render as mount, type JSX } from "@solidjs/web"
import type { Props, Style } from "./protocol.ts"

/// `render(() => <App />)`: draws an app's first screen, where its page is: the page itself on the web, the app's
/// window on a native host. → what takes it away again.
export const render = (code: () => JSX.Element): (() => void) => mount(code, document.body)

/** What a primitive takes of the protocol's props: its role and states, with what it listens for. */
type Semantics = Pick<Props, Extract<keyof Props, "role" | `aria-${string}`>>

/** What a key pressed is told, as the DOM's `KeyboardEvent`. */
export interface Keyed {
  key: string
  shiftKey: boolean
  altKey: boolean
  ctrlKey: boolean
  metaKey: boolean
}

/** What a `Scroll` scrolling is told: how far it's scrolled. */
export interface Scrolled {
  currentTarget: { scrollTop: number; scrollLeft: number }
}

/** What every primitive but an image takes: how it looks, and what it listens for. */
interface Listens extends Semantics {
  class?: string
  /** Version 1's. */
  style?: Style
  tabindex?: 0 | -1
  onClick?: () => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
  onPointerEnter?: () => void
  onPointerLeave?: () => void
}

/** A box's props. */
export interface BoxProps extends Listens {
  children?: JSX.Element
}

/// `<Box class="flex-row items-center gap-2 p-3 bg-muted rounded-lg">...</Box>`: a strict app's own layout, around
/// anything else, as GPUI's `div`: a flex column unless its `class` says otherwise, as React Native's View, styled by
/// the strict style vocabulary, Tailwind's classes by the design tokens only (`core/ui/src/strict/style.ts`). With
/// `role="button"` and `onClick` it's something to press, which the keyboard reaches too; any other `role` and
/// `aria-…` say what it is to a screen reader and to an agent, as ARIA has them.
export function Box(props: BoxProps) {
  const own = omit(props, "style", "class", "children")
  return (
    <Dynamic component={props.role === "button" ? "button" : "div"} type={props.role === "button" ? "button" : undefined} {...own} class={["ui-strict ui-strict-box", props.class]} data-font={props.style?.font} style={css(props.style)}>
      {props.children}
    </Dynamic>
  )
}

/** A scroll's props. */
export interface ScrollProps extends BoxProps {
  onScroll?: (event: Scrolled) => void
}

/// `<Scroll class="flex-1 gap-gap px-gutter">...</Scroll>`: a box whose content scrolls, down unless its class says
/// `overflow-x-auto`, with no scrollbar beside it, as on a phone. `onScroll` is told how far it's scrolled.
export function Scroll(props: ScrollProps) {
  const own = omit(props, "style", "class", "children")
  return (
    <div {...own} class={["ui-strict ui-strict-box ui-strict-scroll", props.class]} data-font={props.style?.font} style={css(props.style)}>
      {props.children}
    </div>
  )
}

/** A text's props. */
export interface TextProps extends Semantics {
  class?: string
  /** Version 1's. */
  style?: Style
  children?: JSX.Element
}

/// `<Text class="text-title">Recently Deleted</Text>`: words, in one of the type styles (`text-body`, `text-label`,
/// `text-headline`, `text-title`, `text-figure`, `text-subheadline`, `text-footnote`, `text-caption-1`,
/// `text-caption-2`), a font (`font-sans`, `font-mono`) and a colour by its token (`text-muted-foreground`), or as
/// what it's in has them. It runs on in a text it's in, and is a block in a box. `role="heading"` with `aria-level`
/// makes it a heading. A strict app's words are always in a `Text` or in a UI library component that takes them.
export function Text(props: TextProps) {
  const own = omit(props, "style", "class", "children")
  const tag = () => (props.role === "heading" ? `h${props["aria-level"] ?? 2}` : "span")
  return (
    <Dynamic component={tag()} {...own} role={props.role === "heading" ? undefined : props.role} class={["ui-strict ui-strict-text", props.class]} data-font={props.style?.font} style={css(props.style)}>
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
  class?: string
  /** Version 1's. */
  style?: Style
}

/// `<Image src="icon.svg" alt="" class="size-6 rounded-icon" />`: a picture, from a `data:` address or the app's own
/// files, named by `alt`, scaled to fit its box.
export function Image(props: ImageProps) {
  return <img src={props.src} alt={props.alt} aria-hidden={props["aria-hidden"]} class={["ui-strict ui-strict-image", props.class]} style={css(props.style)} />
}

/** A path's props. */
export interface PathProps extends Pick<Semantics, "role" | "aria-label" | "aria-hidden"> {
  /** Its shape, as SVG's path data. */
  d: string
  /** The box `d` is drawn in, as SVG's: `"0 0 256 256"`. */
  viewBox: string
  class?: string
}

/// `<Path viewBox="0 0 100 20" d="M0 20 L50 4 L100 12" class="h-8 w-full fill-none stroke-foreground stroke-2" />`: a
/// vector shape, as SVG's `<path>`, filled with the text's colour unless its class says otherwise (`fill-none
/// stroke-foreground stroke-2`), its `viewBox` stretched to its box, its stroke as wide whatever the stretch: an icon,
/// a chart's line or area. `role="img"` with `aria-label` names one that says something.
export function Path(props: PathProps) {
  return (
    <svg viewBox={props.viewBox} preserveAspectRatio="none" role={props.role} aria-label={props["aria-label"]} aria-hidden={props["aria-hidden"] ?? (props.role ? undefined : "true")} class={["ui-strict ui-strict-path", props.class]}>
      <path d={props.d} vector-effect="non-scaling-stroke" />
    </svg>
  )
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
