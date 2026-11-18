// The strict primitives, as the DOM draws them: a box, a scroll, a text, an image and a path, each a plain element with
// the semantic props the strict protocol has (protocol.ts). The input is the UI library's `Input`. A native host draws
// the same props from the ops Solid's universal renderer sends it. A primitive's `style` is styles (../styles.ts),
// drawn by their classes, or, on a page written for version 1, one style object, drawn as inline CSS.
import { omit, onSettled } from "solid-js"
import { Dynamic, Portal, render as mount, type JSX } from "@solidjs/web"
import { layer } from "./layer.ts"
import type { Props, Version1Style } from "./protocol.ts"
import type { Located, Pressed, Wheeled } from "./types/ui.d.ts"
import { web, type Styles } from "../styles.ts"

/// `<Layer modal>…</Layer>`: what's drawn over the whole window, over the page and every layer before it, as a
/// sheet or a menu is: a box the window's size that lets presses through where nothing of its own is (what's in it
/// says `pointerEvents: "auto"`). While a `modal` one is shown, everything else is inert: nothing else is pressed,
/// focused or read by a screen reader.
export function Layer(props: { modal?: boolean; children?: JSX.Element }) {
  let box!: HTMLDivElement
  onSettled(() => {
    if (!props.modal) return
    const others = [...document.body.children].filter((element) => element !== box.parentElement && element !== box && !(element as HTMLElement).inert) as HTMLElement[]
    for (const other of others) other.inert = true
    // The keyboard goes into it: to what asks for it (`autofocus`, a phone's keyboard with it), or to its dialog.
    box.querySelector<HTMLElement>("[autofocus], [aria-modal='true']")?.focus()
    return () => others.forEach((other) => (other.inert = false))
  })
  return (
    <Portal mount={document.body}>
      <div ref={box} {...drawn("ui-strict-box", layer)}>
        {props.children}
      </div>
    </Portal>
  )
}

/// `render(() => <App />)`: draws an app's first screen, where its page is: the page itself on the web, the app's
/// window on a native host. → what takes it away again.
export const render = (code: () => JSX.Element): (() => void) => mount(code, document.body)

/** A primitive's `style`: styles, or version 1's one object. */
type Style = Styles | Version1Style
/** Whether `style` is version 1's. */
const v1 = (style: Style | undefined): style is Version1Style => !!style && !Array.isArray(style) && typeof style === "object" && !("$id" in style)
/** What a primitive's element has of its `style`: its classes after the primitive's own, its inline CSS, and a version
 * 1 type style's `data-font`. */
const drawn = (own: string, style: Style | undefined) => (v1(style) ? { class: own, style: css(style), "data-font": style.font } : (({ class: names, style: inline }) => ({ class: `${own} ${names}`, style: inline }))(web(style as Styles)))

/** What a primitive takes of the protocol's props: its role and states, with what it listens for. */
type Semantics = Pick<Props, Extract<keyof Props, "role" | `aria-${string}`>>

/** What a key pressed is told, as the DOM's `KeyboardEvent`: `preventDefault()` keeps what the key does by itself
 * in a field (a caret moved, the focus moved on by Tab) from happening, on the DOM. */
export interface Keyed {
  key: string
  shiftKey: boolean
  altKey: boolean
  ctrlKey: boolean
  metaKey: boolean
  preventDefault(): void
}

/** What a `Scroll` scrolling is told: how far it's scrolled. */
export interface Scrolled {
  currentTarget: { scrollTop: number; scrollLeft: number }
}

/** What every primitive but an image takes: how it looks, and what it listens for. */
interface Listens extends Semantics {
  style?: Style
  tabindex?: 0 | -1
  onClick?: (event: Located) => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
  onPointerEnter?: () => void
  onPointerLeave?: () => void
  onPointerDown?: (event: Pressed) => void
  onPointerMove?: (event: Pressed) => void
  onPointerUp?: (event: Pressed) => void
  onPointerCancel?: (event: Pressed) => void
  onContextMenu?: (event: Located) => void
  onWheel?: (event: Wheeled) => void
}

/** A box's props. */
export interface BoxProps extends Listens {
  children?: JSX.Element
}

/** The roles of what's pressed, which the DOM draws as a button, so the keyboard reaches it too. */
const pressed = new Set(["button", "menuitem", "tab", "checkbox", "radio", "switch", "option", "link"])

/// `<Box style={{ flexDirection: "row", gap: "gutter", padding: 12, backgroundColor: "muted", borderRadius: "lg" }}>...</Box>`:
/// a strict app's own layout, around anything else: a flex column unless its `style` says otherwise, as React Native's
/// View, styled from the design tokens only. With `onClick` and a role of what's pressed (`button`, `menuitem`,
/// `option`, …) it's something to press, which the keyboard reaches too; any other `role` and `aria-…` say what it is
/// to a screen reader and to an agent, as ARIA has them. A page moved by hand listens to its pointers too (`follows`).
export function Box(props: BoxProps) {
  const own = omit(props, "style", "children", ...moved)
  const moves = movable(props)
  const button = () => !!props.role && pressed.has(props.role) && !!props.onClick
  return (
    <Dynamic
      component={button() ? "button" : "div"}
      type={button() ? "button" : undefined}
      {...own}
      {...moves}
      data-scrolls={v1(props.style) && props.style.overflow === "scroll" ? "" : undefined}
      {...drawn("ui-strict-box", props.style)}
    >
      {props.children}
    </Dynamic>
  )
}

/** A scroll's props. */
export interface ScrollProps extends BoxProps {
  onScroll?: (event: Scrolled) => void
}

/// `<Scroll style={{ flexGrow: 1, gap: "gutter", paddingInline: "gutter" }}>...</Scroll>`: a box whose content scrolls
/// along its `flexDirection`, down in a column and sideways in a row, with no scrollbar beside it, as on a phone.
/// `onScroll` is told how far it's scrolled.
export function Scroll(props: ScrollProps) {
  const own = omit(props, "style", "children", ...moved)
  const moves = movable(props)
  return (
    <div {...own} {...moves} data-scrolls="" {...drawn("ui-strict-box ui-strict-scroll", props.style)}>
      {props.children}
    </div>
  )
}

/** What a page moved by hand listens for (protocol.ts's `Moved`), which `movable` draws. */
const moved = ["onPointerDown", "onPointerMove", "onPointerUp", "onPointerCancel", "onContextMenu", "onWheel"] as const

/// A box's or a scroll's `ref` and `onContextMenu` for what it moves by hand: its pointers (`follows`), and a
/// right-click or a touch held, in place of the device's own menu.
const movable = (props: BoxProps) => ({
  ref: (box: HTMLElement) => (props.onPointerDown || props.onPointerMove || props.onPointerUp || props.onPointerCancel || props.onWheel) && follows(box, props),
  onContextMenu: (event: MouseEvent) => props.onContextMenu && (event.preventDefault(), event.stopPropagation(), props.onContextMenu(event as unknown as Located)),
})

/// A box's pointers, as the protocol has them: a pointer's moves over it, a mouse's too, and once one is down on it,
/// its moves and its end wherever it goes, as a touch's are, each told where the box is (`currentTarget`). While a
/// finger that's down on it moves, the browser scrolls nothing for it, unless it's in something in the box that
/// scrolls (an iPhone's web view would bounce the page). A wheel over it is its own, unless it's up and down over
/// something in it that scrolls, whose that is.
function follows(box: HTMLElement, props: BoxProps) {
  const down = new Set<number>()
  const told = (event: PointerEvent): Pressed => ({ clientX: event.clientX, clientY: event.clientY, timeStamp: event.timeStamp, pointerId: event.pointerId, pointerType: event.pointerType as Pressed["pointerType"], button: event.button, currentTarget: box })
  const move = (event: PointerEvent) => down.has(event.pointerId) && props.onPointerMove?.(told(event))
  const end = (event: PointerEvent) => {
    if (!down.delete(event.pointerId)) return
    if (event.type === "pointerup") props.onPointerUp?.(told(event))
    else props.onPointerCancel?.(told(event))
    if (down.size) return
    removeEventListener("pointermove", move, true)
    removeEventListener("pointerup", end, true)
    removeEventListener("pointercancel", end, true)
  }
  box.addEventListener("pointerdown", (event) => {
    if (!down.size) {
      addEventListener("pointermove", move, true)
      addEventListener("pointerup", end, true)
      addEventListener("pointercancel", end, true)
    }
    down.add(event.pointerId)
    props.onPointerDown?.(told(event))
  })
  box.addEventListener("pointermove", (event) => !down.has(event.pointerId) && props.onPointerMove?.(told(event)))
  box.addEventListener("dragstart", (event) => event.preventDefault())
  box.dataset.follows = ""
  box.addEventListener("touchmove", (event) => down.size && !scrolls(event.target, box) && event.cancelable && event.preventDefault(), { passive: false })
  box.addEventListener(
    "wheel",
    (event) => {
      if (Math.abs(event.deltaY) > Math.abs(event.deltaX) && scrolls(event.target, box)) return
      event.preventDefault()
      const lines = event.deltaMode === WheelEvent.DOM_DELTA_LINE ? 16 : event.deltaMode === WheelEvent.DOM_DELTA_PAGE ? box.clientHeight : 1
      props.onWheel?.({ deltaX: event.deltaX * lines, deltaY: event.deltaY * lines, timeStamp: event.timeStamp })
    },
    { passive: false },
  )
}

/** Whether `target` is in something in `box` that scrolls. */
const scrolls = (target: EventTarget | null, box: HTMLElement) => {
  const scroller = (target as Element | null)?.closest?.("[data-scrolls]")
  return !!scroller && scroller !== box && box.contains(scroller)
}

/** A text's props. */
export interface TextProps extends Semantics {
  style?: Style
  children?: JSX.Element
}

/// `<Text style={{ font: "title" }}>Recently Deleted</Text>`: words, in one of the type styles (`font`: `body`,
/// `label`, `title`, `figure`, `subheadline`, `footnote`, `caption-1`, `caption-2`) and a token's `color`, or as
/// what it's in has them. It runs on in a text it's in, and is a block in a box. `role="heading"` with `aria-level`
/// makes it a heading. A strict app's words are always in a `Text` or in a UI library component that takes them.
export function Text(props: TextProps) {
  const own = omit(props, "style", "children")
  const tag = () => (props.role === "heading" ? `h${props["aria-level"] ?? 2}` : "span")
  return (
    <Dynamic component={tag()} {...own} role={props.role === "heading" ? undefined : props.role} {...drawn("ui-strict-text", props.style)}>
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
/// own files, named by `alt`, scaled to fit its box.
export function Image(props: ImageProps) {
  return <img src={props.src} alt={props.alt} aria-hidden={props["aria-hidden"]} {...drawn("ui-strict-image", props.style)} />
}

/** A path's props. */
export interface PathProps extends Pick<Semantics, "role" | "aria-label" | "aria-hidden"> {
  /** Its shape, as SVG's path data. */
  d: string
  /** The box `d` is drawn in, as SVG's: `"0 0 256 256"`. */
  viewBox: string
  style?: Style
}

/// `<Path viewBox="0 0 100 20" d="M0 20 L50 4 L100 12" style={{ width: "100%", height: 32 }} />`: a vector shape, as
/// SVG's `<path>`, filled with the text's colour, its `viewBox` stretched to its box: an icon, a chart's line or area.
/// `role="img"` with `aria-label` names one that says something.
export function Path(props: PathProps) {
  return (
    <svg viewBox={props.viewBox} preserveAspectRatio="none" role={props.role} aria-label={props["aria-label"]} aria-hidden={props["aria-hidden"] ?? (props.role ? undefined : "true")} {...drawn("ui-strict-path", props.style)}>
      <path d={props.d} vector-effect="non-scaling-stroke" />
    </svg>
  )
}

const colours = new Set(["backgroundColor", "color", "borderColor"])
const offsets = new Set(["top", "right", "bottom", "left"])
const spaces = new Set(["gap", "rowGap", "columnGap", "padding", "paddingInline", "paddingBlock", "paddingTop", "paddingRight", "paddingBottom", "paddingLeft", "margin", "marginInline", "marginBlock"])
const sizes = new Set(["flexBasis", "width", "height", "minWidth", "minHeight", "maxWidth", "maxHeight"])
const tokens: Record<string, string> = { gutter: "var(--gutter)", control: "var(--control)", icon: "var(--icon)", readable: "var(--readable)" }

/** A strict style as CSS: its tokens as the custom properties they name, its pixels as `px`. Its `font` is the
 * element's `data-font` (ui.css). */
export function css(style: Version1Style | undefined): JSX.CSSProperties | undefined {
  if (!style) return
  const out: Record<string, string | number> = {}
  for (const [name, value] of Object.entries(style) as [keyof Version1Style, string | number][]) {
    const property = name.replace(/[A-Z]/g, (letter) => `-${letter.toLowerCase()}`)
    if (name === "font" || value === undefined) continue
    else if (name === "translateX" || name === "translateY") out.translate = `${style.translateX ?? 0}px ${style.translateY ?? 0}px`
    else if (name === "rotate") out.rotate = `${value}deg`
    else if (offsets.has(name)) out[property] = `${value}px`
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
