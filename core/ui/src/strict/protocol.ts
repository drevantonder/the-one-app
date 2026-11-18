// How an app written in strict mode draws on a native host (GPUI on a computer, Lynx on Android): its JS runs in a
// sandbox of its own, and Solid's universal renderer (@solidjs/universal) sends the host what changes in its tree as
// ops, which the host draws. What the person does comes back as events. On the web, iOS and in MCP Apps the same app
// draws to the DOM instead, and none of this is used.
//
// This file is the protocol. A host speaks one `VERSION`; an app's entrypoint says the version it needs (`host`), and
// a host that speaks an older one, or none, opens it in a web view. A change a host of this version couldn't draw
// (a new type, prop, class or op) is a new version. A host draws every version up to its own.
//
// Version 2 is the strict contract: a host draws only the primitives (`Type`), styled by the strict style vocabulary
// (`class`, style.ts), and sends back what the person does to them (`EventName`). Every component of the UI library
// is written once on them, so a host draws no component of its own. Version 1's `style` is still drawn, for pages
// written for 1, until they're written for 2.

/** The protocol's version, which a host says it speaks and an entrypoint's `host` names. */
export const VERSION = 2

/** A node, as the host knows it. `0` is the root, which the host makes: an app's window, or its frame. */
export type Id = number

/** What a node is: one of the primitives, or a run of text inside a `text` (Lynx's `raw-text`).
 * - `box`, GPUI's `div`: a flex column unless its class says otherwise, as React Native's View, which clips nothing.
 * - `scroll`: a box whose content scrolls, down unless its class says `overflow-x-auto`, as GPUI's `div` with
 *   `overflow_y_scroll`, with no scrollbar beside it, as on a phone; one the device draws over it as it moves.
 * - `text`: words. One in a `text` runs on in its line, and one in a box is a block, as React Native's Text. A text
 *   takes its parent's font, type style and colour where it has none of its own, as CSS does; the root's are
 *   `font-sans`, `text-body` and `text-foreground`.
 * - `image`: a picture (`src`), as GPUI's `img`, scaled to fit its box, keeping its shape.
 * - `path`: a vector shape (`d`, in its `viewBox`), as SVG's `<path>` and GPUI's `svg`: filled with the text's
 *   colour unless its class says otherwise (`fill-none stroke-muted-foreground stroke-2`), its `viewBox` stretched to
 *   its box, its stroke as wide however it's stretched. An icon, a chart's line or area.
 * - `input`: a text field, as an `<input>`, or a `<textarea>` with `multiline`. Its look is its class, as a box's.
 * A `raw-text` outside a `text` is drawn as if in one. */
export type Type = "box" | "scroll" | "text" | "image" | "path" | "input" | "raw-text"

/** JS → host: what changed since the last update, in order, as one batch. The host draws once it has the batch. */
export interface Update {
  v: typeof VERSION
  ops: Op[]
}

export type Op =
  /** A new node, not yet in the tree. A `raw-text` has its `value`. */
  | { op: "create"; id: Id; type: Type; value?: string }
  /** One of a node's props (`Props`), or `null` for none. A `raw-text`'s only prop is its `value`. */
  | { op: "set"; id: Id; name: keyof Props; value: unknown }
  /** Puts `id` into `parent` before `before`, or at its end: a node already in the tree moves. */
  | { op: "insert"; parent: Id; id: Id; before?: Id }
  /** Takes `id`, with what's in it, out of `parent`. It may be inserted again. */
  | { op: "remove"; parent: Id; id: Id }
  /** JS no longer holds `id` (it was garbage collected), so it's never inserted again and the host frees it. */
  | { op: "drop"; id: Id }

/** Host → JS: what the person did to a node that listens for it (its `on…` prop), one at a time. */
export interface Event {
  v: typeof VERSION
  op: "event"
  id: Id
  name: EventName
  /** An `input`'s text, with `input`. */
  value?: string
  /** The key, with `keydown`, as the DOM's `KeyboardEvent.key` names it (`Enter`, `Escape`, `ArrowDown`, `a`), and
   * which modifiers were held. */
  key?: string
  shiftKey?: boolean
  altKey?: boolean
  ctrlKey?: boolean
  metaKey?: boolean
  /** How far a `scroll` has scrolled, in CSS pixels, with `scroll`. */
  scrollTop?: number
  scrollLeft?: number
}

/** What a native host gives an app's sandbox, as `globalThis.host`, before it loads the app's page: the module its
 * entrypoint's page names, whose imports of the UI library (`/ui/client/web.js`, `/ui/client/ui.js`) the host loads
 * from `/ui/client/native/`, the same names, built for it. */
export interface Host {
  /** Takes a batch of ops, to draw. */
  update(update: Update): void
  /** Takes a JSON-RPC message for the app's client, as a page on the DOM posts one to its frame's parent (MCP Apps):
   * `ui/initialize`, `tools/call` and the rest, which the client answers as it answers a page. */
  post(message: object): void
}

/** What the UI library gives the host, as `globalThis.ui`, once it's loaded. */
export interface Sandbox {
  /** Something the person did. */
  event(event: Event): void
  /** A JSON-RPC message from the client, as a page on the DOM gets one in its `message` event: an answer to what it
   * posted, or a notification (`notifications/one/changed`, `ui/notifications/host-context-changed`). */
  message(message: object): void
}

/** What a node can listen for, as its `on…` prop: `click` is `onClick`, `pointerenter` `onPointerEnter`, as the DOM
 * names them. `click` is a press, by a pointer, a finger, or Enter or Space where the keyboard is; `pointerenter` and
 * `pointerleave` are a mouse or a trackpad's pointer coming over it and going (GPUI's `on_hover`), never a finger;
 * `keydown` a key pressed where the keyboard is, in it or in what's in it; `scroll` a `scroll` scrolling. A host offers
 * them as accessibility actions too: `click` is AccessKit's `Click`, `input` its `SetValue`, `focus` and `blur` its
 * `Focus` and `Blur`, `scroll` its `ScrollIntoView` and `ScrollDown`. */
export type EventName = "click" | "input" | "focus" | "blur" | "keydown" | "pointerenter" | "pointerleave" | "scroll"

/** Every node's props, as `set` sends them, named as a strict app's JSX names them, which are the DOM's. One semantic
 * tree, which a screen reader and an agent both read: a node's role, its name (`aria-label`, or what a `text` says),
 * its value, its states (`aria-…`, `disabled`) and its actions (which `on…` it has), as ARIA has them and a
 * host maps them to AccessKit's. A listener is sent as `true` while there is one, `null` once there isn't. */
export interface Props {
  /** What it is, by ARIA's name, which a host maps to AccessKit's `Role`. None is a plain group. */
  role?: Role
  /** Its accessible name: a button's words where it has no text, a region's name. */
  "aria-label"?: string
  /** A longer description, read after its name. */
  "aria-description"?: string
  /** A heading's level, 1 to 6. */
  "aria-level"?: number
  /** A progress bar's or a slider's number, between `aria-valuemin` and `aria-valuemax`. */
  "aria-valuenow"?: number
  "aria-valuemin"?: number
  "aria-valuemax"?: number
  "aria-disabled"?: Bool
  "aria-selected"?: Bool
  "aria-checked"?: Bool | "mixed"
  "aria-expanded"?: Bool
  "aria-pressed"?: Bool | "mixed"
  "aria-busy"?: Bool
  "aria-current"?: Bool
  "aria-invalid"?: Bool
  /** A dialog that holds the keyboard and the screen reader while it's open, with what's behind it inert. */
  "aria-modal"?: Bool
  /** What a press opens, as ARIA's. */
  "aria-haspopup"?: Bool | "menu" | "listbox" | "dialog"
  /** Left out of the semantic tree: it's only how it looks. */
  "aria-hidden"?: Bool
  /** How it's laid out and looks, in the strict style vocabulary (style.ts): `"flex-row items-center gap-2 px-4
   * bg-muted rounded-full"`, classes split by spaces, each with any variants (`hover:bg-accent`). */
  class?: string
  /** Version 1's: how it's laid out and looks, as the strict style subset. A host draws it for a page written for 1. */
  style?: Style
  /** `image`: a `data:` address, or a file of the app's own by its path. */
  src?: string
  /** `image`: what it shows, its accessible name. Empty for one that's only how it looks. */
  alt?: string
  /** `path`: its shape, as SVG's path data. */
  d?: string
  /** `path`: the box its `d` is drawn in, as SVG's `viewBox`: `"0 0 256 256"`. */
  viewBox?: string
  /** What the keyboard reaches by Tab, as the DOM's: `0` for something it reaches that it otherwise wouldn't, `-1`
   * for something focused only by a press. A press, an input and what has `onClick` are reached already. */
  tabindex?: 0 | -1
  /** `input`: its text. */
  value?: string
  /** `input`: what kind of text it takes, as an `<input type>`. */
  type?: "text" | "search" | "email" | "url" | "tel" | "number" | "password"
  /** `input`: more than one line, as a `<textarea>`. */
  multiline?: boolean
  /** `input`: the grey words in it while it's empty. */
  placeholder?: string
  /** `input`: what a phone's keyboard capitalises, and whether it checks spelling and fills it in, as the DOM's. */
  autocapitalize?: "off" | "none" | "on" | "sentences" | "words" | "characters"
  spellcheck?: boolean
  autocomplete?: string
  /** `input`: what a phone's keyboard's Enter says it does, as the DOM's `enterkeyhint`. */
  enterkeyhint?: "enter" | "done" | "go" | "next" | "previous" | "search" | "send"
  /** `input`: has the keyboard as it's drawn. */
  autofocus?: boolean
  disabled?: boolean
  required?: boolean
  onClick?: true
  onInput?: true
  onFocus?: true
  onBlur?: true
  onKeyDown?: true
  onPointerEnter?: true
  onPointerLeave?: true
  onScroll?: true
}

/** A state that's so or not, as ARIA writes it. */
export type Bool = "true" | "false"

/** ARIA's roles a strict node may have. */
export type Role =
  | "alert" | "button" | "checkbox" | "dialog" | "group" | "heading" | "img" | "link" | "list" | "listitem" | "main"
  | "menu" | "menuitem" | "navigation" | "progressbar" | "radio" | "radiogroup" | "region" | "searchbox" | "separator"
  | "slider" | "status" | "switch" | "tab" | "tablist" | "tabpanel" | "textbox" | "toolbar" | "banner" | "alertdialog"
  | "listbox" | "option" | "combobox" | "form" | "tooltip"

// Version 1's strict style subset, which a page written for 1 has as its `style`: CSS's own property names and values,
// as React Native and Lynx take them, limited to what every host draws, with colours, radii, sizes and type by the design tokens (core/ui/src/tokens.css), never a value
// of their own. Lengths are CSS pixels.

/** A colour, by its token: `muted-foreground` is `var(--muted-foreground)`. */
export type Colour =
  | "background" | "foreground" | "card" | "card-foreground" | "popover" | "popover-foreground" | "primary"
  | "primary-foreground" | "secondary" | "secondary-foreground" | "muted" | "muted-foreground" | "accent"
  | "accent-foreground" | "pressed" | "destructive" | "warning" | "border" | "input" | "ring" | "overlay"

/** A type style (DESIGN.md: text is one of a few type styles), each a size, a weight and a tracking from the tokens. */
export type Font = "body" | "label" | "title" | "figure" | "subheadline" | "footnote" | "caption-1" | "caption-2"

/** A space: pixels, or a screen's `gap` or `gutter`. */
export type Space = number | "gap" | "gutter"

/** A size: pixels, a percentage of the room, `auto`, or a token: what's pressed (`control`), an icon (`icon`), and
 * where lists stop growing (`readable`). */
export type Size = number | `${number}%` | "auto" | "control" | "icon" | "readable"

/** A radius, by its token, or `full` for a pill or a circle. */
export type Radius = 0 | "sm" | "md" | "lg" | "xl" | "full"

export interface Style {
  display?: "flex" | "grid" | "none"
  flexDirection?: "row" | "column" | "row-reverse" | "column-reverse"
  flexWrap?: "nowrap" | "wrap"
  flexGrow?: number
  flexShrink?: number
  flexBasis?: Size
  alignItems?: "flex-start" | "center" | "flex-end" | "stretch" | "baseline"
  alignSelf?: "auto" | "flex-start" | "center" | "flex-end" | "stretch" | "baseline"
  justifyContent?: "flex-start" | "center" | "flex-end" | "space-between" | "space-around" | "space-evenly"
  /** Equal columns, or as many as fit at least so wide, filling the room. */
  gridTemplateColumns?: `repeat(${number}, 1fr)` | `repeat(auto-fill, minmax(${number}px, 1fr))`
  gridColumn?: `span ${number}`
  gap?: Space
  rowGap?: Space
  columnGap?: Space
  padding?: Space
  paddingInline?: Space
  paddingBlock?: Space
  paddingTop?: Space
  paddingRight?: Space
  paddingBottom?: Space
  paddingLeft?: Space
  margin?: Space
  marginInline?: Space | "auto"
  marginBlock?: Space
  width?: Size
  height?: Size
  minWidth?: Size
  minHeight?: Size
  maxWidth?: Size
  maxHeight?: Size
  aspectRatio?: number
  overflow?: "visible" | "hidden" | "scroll"
  opacity?: number
  backgroundColor?: Colour
  color?: Colour
  borderColor?: Colour
  /** Borders are hairlines, or none. */
  borderWidth?: 0 | 1
  borderTopWidth?: 0 | 1
  borderRightWidth?: 0 | 1
  borderBottomWidth?: 0 | 1
  borderLeftWidth?: 0 | 1
  borderRadius?: Radius
  font?: Font
  textAlign?: "start" | "center" | "end"
  fontVariantNumeric?: "normal" | "tabular-nums"
  /** At most so many lines, the last cut with "…". */
  lineClamp?: number
}
