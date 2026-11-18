// How an app written in strict mode draws natively, on our renderer on a computer and Android: its JS runs in a
// sandbox of its own (rquickjs), and Solid's universal renderer (@solidjs/universal) sends the host what changes in its
// tree as ops, which the host draws. What the person does comes back as events. On the web, iOS and in MCP Apps the
// same app draws to the DOM instead, and none of this is used.
//
// This file is the protocol. A host speaks one `VERSION`; an app's entrypoint says the version it needs (`host`), and
// a host that speaks an older one, or none, opens it in a web view. A change a host of this version couldn't draw (a
// new type, prop, op, token kind, style property or condition) is a new version. A host draws every version up to its
// own.
//
// Version 2 is the strict contract: a host draws only the primitives (`Type`), each styled by styles (`Styles`) whose
// values are design tokens (`Tokens`), and sends back what the person does to them (`EventName`). Every component of
// the UI library is written once on them, so a host draws no component of its own. Both tables come with the UI
// library and with each app, over the air, and a host knows only the kinds of token, the conditions and the style
// properties, never a token's or a style's name or value. Version 1's `style` objects are still drawn, for pages
// written for 1, until they're written for 2.

/** The protocol's version, which a host says it speaks, an entrypoint's `host` names, and the tables say they need. */
export const VERSION = 2

/** A node, as the host knows it. `0` is the root, which the host makes: an app's window, or its frame. */
export type Id = number

/** What a node is: one of the primitives, or a run of text inside a `text` (`raw-text`).
 * - `box`: a flex column unless its style says otherwise, as React Native's View, laid out as Taffy lays out flex and
 *   grid; it clips nothing unless its style says `overflow: "hidden"`.
 * - `scroll`: a box whose content scrolls along its `flexDirection`: down in a column, sideways in a row, with no
 *   scrollbar beside it, as on a phone; one the device draws over it as it moves.
 * - `text`: words. One in a `text` runs on in its line, and one in a box is a block, as React Native's Text. A text
 *   takes its parent's `font`, `fontWeight` and `color` where it has none of its own, as CSS does; the root's are its
 *   page's root style's (`Styles.root`).
 * - `image`: a picture (`src`), scaled to fit its box, keeping its shape.
 * - `path`: a vector shape (`d`, in its `viewBox`), as SVG's `<path>`: filled with its `fill`, the text's colour
 *   unless its style says otherwise, and stroked with its `stroke`; its `viewBox` is stretched to its box, its stroke
 *   as wide however it's stretched. An icon, a chart's line or area.
 * - `input`: a text field, one line, or several with `multiline`, whose text is its editing state (`Editing`), and
 *   runs the way its first strong character does, as the DOM's `dir="auto"`.
 * - `picker`: the device's own way of choosing one of a few (`options`), as a phone's wheel or sheet is: see-through
 *   over its box, so what's drawn under it shows, and pressed, it opens the device's picker, which sends `input` with
 *   the `value` chosen. A page uses it where a finger points; a host with no picker of its own may draw none.
 * - `video`: a screen's live video over WebRTC, scaled to fit its box, keeping its shape, as a remote desktop app shows
 *   one. The node is the stream: it makes the peer connection, with its `ice` (the screen's ICE servers, and the policy
 *   to use them by), receiving video only, and a data channel both ends make (`input`, negotiated, id 0); once it has
 *   gathered its candidates it sends `offer` with its SDP, whole, and the page sets its `answer`, the screen's. A new
 *   `ice` is a new connection, the last one closed. It sends `state`, `connected` as its data channel opens and `lost`
 *   as the channel closes or the connection fails, and `size`, the video's own, as it comes and changes. What the
 *   person does on it, unless it's `watch`ed, goes straight down the data channel as JSON, each where on the screen it
 *   is (its video's pixels), as core/ui/src/strict/input.ts has it (`Input`): the pointer as it moves, its buttons
 *   down and up (1 to 3) and a wheel's steps (4 to 7) with the modifiers held, the keys that aren't text (as X names
 *   them), and what's typed or pasted, which takes a field of its own so a phone's keyboard comes up as it's pressed,
 *   or as its `autofocus` is set. The device's full screen keys (F11, ⌃⌘F) aren't sent: they go on as `keydown` to the node it's in. A host that
 *   doesn't draw it says so (`Host.video`), and the page that uses it is opened another way.
 * A `raw-text` outside a `text` is drawn as if in one. */
export type Type = "box" | "scroll" | "text" | "image" | "path" | "input" | "picker" | "video" | "raw-text"

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
  /** The page uses `what`, a part of the UI library a native host doesn't draw yet, so the host opens it another way:
   * a computer's or a phone's in a web view. */
  | { op: "unsupported"; what: string }

/** Host → JS: what the person did to a node that listens for it (its `on…` prop), one at a time. */
export interface Event {
  v: typeof VERSION
  op: "event"
  id: Id
  name: EventName
  /** An `input`'s editing state, with `input`: what's typed, selected and composed now. */
  editing?: Editing
  /** What an `input`'s action key did, with `action`. */
  action?: TextInputAction
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
  /** When it happened, in milliseconds on the host's own clock, which `requestAnimationFrame` is on too. */
  timeStamp?: number
  /** A pointer's, `contextmenu`'s and `click`'s: where it is in the window, in CSS pixels. */
  clientX?: number
  clientY?: number
  /** A pointer's: which pointer, as each finger is its own, what it is, and which button, `0` the main one, `2` the
   * secondary. */
  pointerId?: number
  pointerType?: "mouse" | "touch" | "pen"
  button?: number
  /** A pointer's, `contextmenu`'s and `click`'s: where the node is in the window, as it's drawn, transforms and all. */
  rect?: Rect
  /** `wheel`'s: how far a trackpad or a wheel moved, in CSS pixels. */
  deltaX?: number
  deltaY?: number
  /** A `video`'s `offer`: its SDP offer, with its candidates. */
  sdp?: string
  /** A `video`'s `state`: its data channel open (`connected`), or its connection lost. */
  state?: "connected" | "lost"
  /** A `video`'s `size`: its video's own, in its pixels. */
  width?: number
  height?: number
}

/** Where a node is drawn in the window, in CSS pixels. */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
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
  /** Whether it draws `video`. A page that uses one where it doesn't tells it `unsupported`, so it's opened another
   * way, as a host of version 2 before it drew video doesn't know the type. */
  video?: boolean
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
 * `pointerleave` are a mouse or a trackpad's pointer coming over it and going, never a finger; `keydown` a key pressed
 * where the keyboard is, in it or in what's in it; `scroll` a `scroll` scrolling. An `input` says `input` with its
 * editing state whenever it changes (what's typed, a selection, an input method's composing), and `action` when its
 * action key is pressed (a keyboard's Enter, or a phone's Done, Go, Next, Search or Send). A host offers them as
 * accessibility actions too: `click` is AccessKit's `Click`, `input` its `SetValue`, `focus` and `blur` its `Focus`
 * and `Blur`, `scroll` its `ScrollIntoView` and `ScrollDown`. */
export type EventName = "click" | "input" | "action" | "focus" | "blur" | "keydown" | "pointerenter" | "pointerleave" | "scroll" | "layout" | Moved | Streamed

/** What a `video` tells its page of its stream: `offer`, `state` and `size`. */
export type Streamed = "offer" | "state" | "size"

/** What a page that's moved by hand listens for, as the Home Screen's pages are, a finger's or a mouse's, as the DOM
 * names them: a pointer's (`pointerdown` to `pointercancel`) go to the node under it and to each node it's in that
 * listens, the innermost first, as the DOM's bubble, and once a pointer is down on a node, its moves and its end go
 * to that node, wherever it goes, as a touch's do; `contextmenu` is a right-click, or a touch held (AccessKit's
 * `ShowContextMenu`); `wheel` a trackpad's or a wheel's scroll over it. */
export type Moved = "pointerdown" | "pointermove" | "pointerup" | "pointercancel" | "contextmenu" | "wheel"

/** An input's editing state, as Flutter's `TextEditingValue`: its text, what's selected, and what an input method is
 * still composing. Every platform's keyboard bridge and the DOM say it in this one shape. Offsets are into `text`, in
 * UTF-16 code units, as JS counts them. */
export interface Editing {
  text: string
  /** What's selected, from where it started (`base`) to where it ends (`extent`): a caret where the two are the same.
   * `extent` before `base` is a selection made backwards. */
  selection: { base: number; extent: number }
  /** What an input method is composing and hasn't committed (a word being written in Japanese, a dictated phrase),
   * from `start` to `end`, or `null`. */
  composing: { start: number; end: number } | null
}

/** What an input's action key does, as Flutter's `TextInputAction`, which a phone's keyboard names on it: `newline`
 * (a multiline input's Enter), `done`, `go`, `next`, `previous`, `search`, `send`. */
export type TextInputAction = "newline" | "done" | "go" | "next" | "previous" | "search" | "send"

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
  /** What it's known by, which `aria-controls` and `aria-activedescendant` name it by. Like them it's only what it
   * is, never how it's drawn, so a host that doesn't read it yet draws the page the same. */
  id?: string
  /** What this drives, by its `id`: the listbox what's typed in narrows (WAI-ARIA's combobox). */
  "aria-controls"?: string
  /** The option the keyboard is on while it stays in what's typed in, by its `id` (WAI-ARIA's combobox). */
  "aria-activedescendant"?: string
  /** How it's laid out and looks: its styles (`Styles`) by id, and inline ones worked out as the page runs, the last
   * winning where two say the same property. A page written for 1 sends one `Version1Style` instead. */
  style?: (StyleId | Inline)[] | Version1Style
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
  /** `raw-text`: its words; `picker`: the option chosen. */
  value?: string
  /** `input`: its editing state, as its app last set it. What the person does to it comes back as `input` events
   * ahead of this, which the host keeps until the app sets it again. */
  editing?: Editing
  /** `input`: what its action key does. One line's is `done` unless it says otherwise, several lines' `newline`. */
  action?: TextInputAction
  /** `picker`: what can be chosen, each its value and what it says, and whether it can be. Its `value` is the one
   * chosen. */
  options?: { value: string; label: string; disabled?: boolean }[]
  /** `video`: the ICE servers to connect with, as WebRTC's `RTCConfiguration` has them, and the policy to use them by.
   * A new one is a new connection. */
  ice?: Ice
  /** `video`: the screen's SDP answer to its `offer`. */
  answer?: string
  /** `video`: looked at only, so nothing done on it is sent. */
  watch?: boolean
  /** `video`: what it shows, its accessible name, as an `image`'s `alt`. */
  label?: string
  /** `input`: what kind of text it takes, as an `<input type>`: which keyboard a phone shows, and `password` hides it. */
  type?: "text" | "search" | "email" | "url" | "tel" | "number" | "password"
  /** `input`: which keyboard a touch screen shows for it, as `inputmode`: `numeric` is digits alone. */
  inputmode?: "text" | "numeric" | "decimal" | "tel" | "email" | "url" | "search"
  /** `input`: shown, chosen and copied, but not typed in. */
  readonly?: boolean
  /** `input`: more than one line, as a `<textarea>`. */
  multiline?: boolean
  /** `input`: the grey words in it while it's empty. */
  placeholder?: string
  /** `input`: what a phone's keyboard capitalises, whether it checks spelling, and what it may fill in (the DOM's
   * `autocomplete` tokens: `email`, `current-password`, `one-time-code`). */
  autocapitalize?: "none" | "sentences" | "words" | "characters"
  spellcheck?: boolean
  autocomplete?: string
  /** Has the keyboard as it's drawn, and again each time it's set: a field a phone's keyboard comes up for, a menu's
   * item its arrow keys move to. */
  autofocus?: boolean
  disabled?: boolean
  required?: boolean
  onClick?: true
  onInput?: true
  onAction?: true
  onFocus?: true
  onBlur?: true
  onKeyDown?: true
  onPointerEnter?: true
  onPointerLeave?: true
  onScroll?: true
  /** Told where it's drawn (`rect`) as it's laid out and each time that changes, as React Native's `onLayout`. */
  onLayout?: true
  onPointerDown?: true
  onPointerMove?: true
  onPointerUp?: true
  onPointerCancel?: true
  onContextMenu?: true
  onWheel?: true
  onOffer?: true
  onState?: true
  onSize?: true
}

/** What a `video` connects with, as WebRTC's `RTCConfiguration` has it. */
export interface Ice {
  iceServers: { urls: string | string[]; username?: string; credential?: string }[]
  iceTransportPolicy?: "all" | "relay"
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
// as React Native takes them, limited to what every host draws, with colours, radii, sizes and type by the design
// tokens' names, never a value of their own. Lengths are CSS pixels.

/** A colour, by its token: `muted-foreground` is `var(--muted-foreground)`. */
export type Colour =
  | "background" | "foreground" | "card" | "primary" | "primary-foreground" | "muted" | "muted-foreground" | "destructive"
  | "warning" | "border" | "overlay"

/** A type style (DESIGN.md: text is one of a few type styles), each a size, a weight and a tracking from the tokens. */
export type Font = "body" | "label" | "title" | "figure" | "subheadline" | "footnote" | "caption-1" | "caption-2"

/** A space: pixels, or a screen's `gutter`. */
export type Space = number | "gutter"

/** A size: pixels, a percentage of the room, `auto`, or a token: what's pressed (`control`), an icon (`icon`), and
 * where lists stop growing (`readable`). */
export type Size = number | `${number}%` | "auto" | "control" | "icon" | "readable"

/** A radius, by its token, or `full` for a pill or a circle. */
export type Radius = 0 | "sm" | "md" | "xl" | "full"

export interface Version1Style {
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
  /** `absolute` takes it out of its parent's flow, placed by `top`, `right`, `bottom` and `left` in its nearest
   * parent that's `relative` or `absolute`, over what's drawn before it, or under what's drawn before it with a
   * higher `zIndex`. */
  position?: "relative" | "absolute"
  top?: number
  right?: number
  bottom?: number
  left?: number
  zIndex?: number
  /** Drawn moved, larger or turned from where it's laid out, around its middle, which moves nothing else: CSS's
   * `translate`, `scale` and `rotate` (degrees), React Native's `transform`. What a page moves by hand, a frame at a
   * time (`requestAnimationFrame`), it moves with these. */
  translateX?: number
  translateY?: number
  scale?: number
  rotate?: number
  /** `none` lets what's under it have the pointer, as a picture of something carried over the screen does. */
  pointerEvents?: "auto" | "none"
}

// The tables. The UI library's build makes both from its own TypeScript (core/ui/src/tokens.ts, and each component's
// styles), as it makes the web's CSS from them; each app's build makes its styles' the same way. A host fetches them
// as it fetches a page's modules: the tokens and the UI library's styles at `/ui/client/tokens.json` and
// `/ui/client/styles.json`, an app's styles at `styles.json` beside its page's `index.html`. Each says the protocol
// `v`ersion it needs; a host that speaks an older one opens the page in a web view, as for an entrypoint's `host`.

/** A token's id, as its table names it. A host never knows one by name: a style says which it uses. */
export type TokenId = string

/** What a token's value may depend on, which a host knows of its device and window as they change, in the order they
 * win: where two that hold give a value, the later's is it.
 * - `dark`: the window is dark, as its person's Colour Mode, or the device's where they chose none, says.
 * - `touch`: what points at it is a finger, not a mouse or a trackpad (CSS's `pointer: coarse`).
 * - `phone`: an iPhone's or Android phone's own text size, which sets `rem` to the size its person chose.
 * - `reducedMotion`: its person asked for less motion. */
export type Condition = "dark" | "touch" | "phone" | "reducedMotion"
export const conditions: Condition[] = ["dark", "touch", "phone", "reducedMotion"]

/** A value, or a value by condition: `default`, and what it is where a condition holds. */
export type Varies<T> = T | ({ default: T } & { [C in Condition]?: T })

/** A colour in sRGB, each channel and alpha from 0 to 1. */
export type Rgba = [r: number, g: number, b: number, a: number]

/** A length: CSS pixels, or rems, which are the device's text size (16px on a computer; on a phone, `phone`'s). */
export type Length = { px: number } | { rem: number }

/** The design tokens, by id. A host knows each `kind` and reads its value; a kind it doesn't know is a `v` it
 * doesn't speak. */
export interface Tokens {
  v: number
  tokens: Record<TokenId, Token>
}

export type Token =
  /** A colour, as it is light, and dark (`dark`). */
  | { kind: "colour"; value: Varies<Rgba> }
  /** A length: a space, a size, a border's width. */
  | { kind: "length"; value: Varies<Length> }
  /** A corner's radius, or a part of its box's side (`{ percent: 22.5 }`); a pill's or a circle's is `{ px: 9999 }`,
   * half its height. */
  | { kind: "radius"; value: Varies<Length | { percent: number }> }
  /** Shadows, drawn under what has it, the first nearest. */
  | { kind: "shadow"; value: Varies<{ x: Length; y: Length; blur: Length; spread: Length; colour: Rgba; inset?: true }[]> }
  /** What shows through what has it (`backdropFilter`): what's behind, blurred so far and saturated so much (1 is as
   * it is), as Apple's materials and Liquid Glass. */
  | { kind: "material"; value: Varies<{ blur: Length; saturate: number }> }
  /** Font families, the first a device has. */
  | { kind: "font"; value: string[] }
  /** A font weight, 1 to 1000, lighter in `dark` where light text on dark looks heavier. */
  | { kind: "weight"; value: Varies<number> }
  /** A type style (DESIGN.md: text is one of a few): its size, line height (a multiple of its size), weight,
   * tracking (in ems) and whether its figures line up (tabular). */
  | { kind: "type"; value: { font: TokenId; size: Varies<Length>; lineHeight: number; weight: TokenId; tracking: number; tabular: boolean } }
  /** How a change moves: how long it takes, in milliseconds, and its easing, a cubic Bézier's two points. */
  | { kind: "motion"; value: { duration: Varies<number>; easing: Varies<[x1: number, y1: number, x2: number, y2: number]> } }

/** What a style can't know ahead: where something is as the page runs, a finger moving it or a menu placed under
 * what opened it, in CSS pixels (`rotate` in degrees, `scale` and `opacity` as numbers), as React Native's inline
 * styles are. */
export type Inline = { [P in "top" | "bottom" | "start" | "end" | "width" | "height" | "translateX" | "translateY" | "scale" | "rotate" | "opacity"]?: number }

/** A style's id. The same style is the same id in every table, so a host may keep them all in one. */
export type StyleId = string

/** The styles of the UI library or an app, by id, and the root's: what the root of its page is styled by (its
 * background, its text's colour and type style), in the UI library's table. */
export interface Styles {
  v: number
  styles: Record<StyleId, Style>
  root?: StyleId
}

/** A node's state, which a style's value may depend on, as it may on a condition: `hover`, a mouse's pointer over it
 * (never a finger); `pressed`, a pointer or a finger down on it, or Enter or Space held where the keyboard is;
 * `focused`, where the keyboard is, as CSS's `:focus-visible`; `entering`, what it starts from as it's added, to
 * transition from (CSS's `@starting-style`). A later one wins, as conditions do. */
export type State = "hover" | "pressed" | "focused" | "entering"

/** A style: properties, as React Native names them, each a value or a value by state (`{ default, hover }`). Every
 * shorthand is written out (`padding` is its four sides), so the last style to say a property is all it is. */
export type Style = { [P in keyof Properties]?: Properties[P] | ({ default?: Properties[P] } & { [S in State]?: Properties[P] }) }

/** A token's value, or its negative (`{ token, negative: true }`, a margin pulling outward). */
export type Ref = { token: TokenId; negative?: true }

/** A length where one may be a part of the room or none: a token, a percentage of the parent's, or `auto`. */
export type Dimension = Ref | `${number}%` | "auto" | 0

/** Every property a style may have, laid out as Taffy lays out flex and grid. `start` and `end` are the line's: left
 * and right where text runs left to right. */
export interface Properties {
  display: "flex" | "grid" | "none"
  flexDirection: "row" | "column" | "row-reverse" | "column-reverse"
  flexWrap: "nowrap" | "wrap" | "wrap-reverse"
  flexGrow: number
  flexShrink: number
  flexBasis: Dimension
  alignItems: "flex-start" | "center" | "flex-end" | "stretch" | "baseline"
  alignSelf: "auto" | "flex-start" | "center" | "flex-end" | "stretch" | "baseline"
  alignContent: "flex-start" | "center" | "flex-end" | "stretch" | "space-between" | "space-around"
  justifyContent: "flex-start" | "center" | "flex-end" | "space-between" | "space-around" | "space-evenly"
  /** Equal columns (`{ repeat: 3 }`), or as many as fit, each at least `fill` wide, sharing what's left. */
  gridTemplateColumns: { repeat: number } | { fill: Ref }
  gridColumn: { span: number }
  rowGap: Ref | 0
  columnGap: Ref | 0
  paddingTop: Ref | 0
  paddingBottom: Ref | 0
  paddingStart: Ref | 0
  paddingEnd: Ref | 0
  marginTop: Dimension
  marginBottom: Dimension
  marginStart: Dimension
  marginEnd: Dimension
  width: Dimension
  height: Dimension
  minWidth: Dimension
  minHeight: Dimension
  maxWidth: Dimension | "none"
  maxHeight: Dimension | "none"
  aspectRatio: number
  /** `fixed` is placed in the window, over what's in it, whatever scrolls, as the DOM's top layer is: a sheet, a
   * menu. */
  position: "relative" | "absolute" | "fixed"
  top: Dimension
  bottom: Dimension
  start: Dimension
  end: Dimension
  zIndex: number
  overflow: "visible" | "hidden"
  /** Moved from where it's laid out, along and down, and grown, without moving what's around it. */
  translateX: Dimension
  translateY: Dimension
  scale: number
  /** Turned, in degrees, around its middle. */
  rotate: number
  opacity: number
  backgroundColor: Ref
  color: Ref
  borderColor: Ref
  borderTopWidth: Ref | 0
  borderBottomWidth: Ref | 0
  borderStartWidth: Ref | 0
  borderEndWidth: Ref | 0
  borderStyle: "solid" | "dashed"
  borderTopStartRadius: Ref | 0
  borderTopEndRadius: Ref | 0
  borderBottomStartRadius: Ref | 0
  borderBottomEndRadius: Ref | 0
  boxShadow: Ref | "none"
  /** A material token: what's behind it shows through, blurred, under its background. */
  backdropFilter: Ref | "none"
  /** A type style token: size, line height, weight, tracking and figures together. */
  font: Ref
  fontFamily: Ref
  fontWeight: Ref
  textAlign: "start" | "center" | "end"
  /** At most so many lines, the last cut with "…". */
  lineClamp: number
  whiteSpace: "normal" | "nowrap" | "pre-wrap"
  textDecorationLine: "none" | "underline"
  /** A path's paint: a colour token, the text's colour, or none. */
  fill: Ref | "currentColor" | "none"
  stroke: Ref | "currentColor" | "none"
  strokeWidth: number
  cursor: "default" | "pointer" | "text" | "grab" | "grabbing" | "not-allowed"
  pointerEvents: "auto" | "none"
  /** Which drags of a finger the device scrolls by itself, telling the node `pointercancel` as it starts, as CSS's
   * `touch-action`: `pan-y` scrolls a list up and down through a row that's swiped sideways. Otherwise a finger down
   * on a node that listens for its pointer moves it alone. */
  touchAction: "auto" | "pan-x" | "pan-y"
  userSelect: "auto" | "none" | "text"
  /** What changes of it move rather than jump, and how (a motion token). */
  transitionProperty: (keyof Properties)[]
  transition: Ref
  /** A motion that repeats while it's drawn, at a motion token's pace: `spin`, a turn; `pulse`, fading to half and
   * back. */
  animationName: "spin" | "pulse" | "none"
  animation: Ref
}
