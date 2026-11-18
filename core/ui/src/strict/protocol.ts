// How an app written in strict mode draws on a native host (GPUI on a computer, Lynx on Android): its JS runs in a
// sandbox of its own, and Solid's universal renderer (@solidjs/universal) sends the host what changes in its tree as
// ops, which the host draws. What the person does comes back as events. On the web, iOS and in MCP Apps the same app
// draws to the DOM instead, and none of this is used.
//
// This file is the protocol. A host speaks one `VERSION`; an app's entrypoint says the version it needs (`host`), and
// a host that speaks an older one, or none, opens it in a web view. A change a host of this version couldn't draw
// (a new type, prop, style or op) is a new version.

/** The protocol's version, which a host says it speaks and an entrypoint's `host` names. */
export const VERSION = 1

/** A node, as the host knows it. `0` is the root, which the host makes: an app's window, or its frame. */
export type Id = number

/** What a node is: one of the four primitives, or a run of text inside a `text` (Lynx's `raw-text`). A `box` is a flex
 * column unless its style says otherwise, as React Native's View; a `text` in a `text` runs on in its line, and one in
 * a `box` is a block, as React Native's Text; a text takes its parent's `font` and `color` where it has none of its
 * own, as CSS does, and the root's are `body` and `foreground`. A `raw-text` outside a `text` is drawn as if in one. */
export type Type = "box" | "text" | "image" | "input" | "raw-text"

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
}

/** What a node can listen for, as its `on…` prop: `click` is `onClick`, as the DOM names them. A host offers each as
 * an accessibility action too: `click` is AccessKit's `Click`, `input` its `SetValue`, `focus` and `blur` its `Focus`
 * and `Blur`. */
export type EventName = "click" | "input" | "focus" | "blur"

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
  "aria-busy"?: Bool
  "aria-current"?: Bool
  "aria-invalid"?: Bool
  /** Left out of the semantic tree: it's only how it looks. */
  "aria-hidden"?: Bool
  /** How it's laid out and looks: the strict style subset. */
  style?: Style
  /** `image`: a `data:` address, or a file of the app's own by its path. */
  src?: string
  /** `image`: what it shows, its accessible name. Empty for one that's only how it looks. */
  alt?: string
  /** `input`: its text. */
  value?: string
  /** `input`: what kind of text it takes, as an `<input type>`. */
  type?: "text" | "search" | "email" | "url" | "tel" | "number" | "password"
  /** `input`: more than one line, as a `<textarea>`. */
  multiline?: boolean
  /** `input`: the grey words in it while it's empty. */
  placeholder?: string
  disabled?: boolean
  required?: boolean
  onClick?: true
  onInput?: true
  onFocus?: true
  onBlur?: true
}

/** A state that's so or not, as ARIA writes it. */
export type Bool = "true" | "false"

/** ARIA's roles a strict node may have. */
export type Role =
  | "alert" | "button" | "checkbox" | "dialog" | "group" | "heading" | "img" | "link" | "list" | "listitem" | "main"
  | "menu" | "menuitem" | "navigation" | "progressbar" | "radio" | "radiogroup" | "region" | "searchbox" | "separator"
  | "slider" | "status" | "switch" | "tab" | "tablist" | "tabpanel" | "textbox" | "toolbar" | "banner"

// The strict style subset: CSS's own property names and values, as React Native and Lynx take them, limited to what
// every host draws, with colours, radii, sizes and type by the design tokens (core/ui/src/tokens.css), never a value
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
