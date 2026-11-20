// How an app's page draws natively, on our renderer on a computer, an iPhone and Android: its JS runs in a
// sandbox of its own (rquickjs), and Solid's universal renderer (@solidjs/universal) tells the host what changes in its
// tree as mutations, which the host applies and draws. What the person does comes back as events. In a browser and in
// MCP Apps the same app draws to the DOM instead, and none of this is used.
//
// This file is the renderer's bridge: what crosses between the sandbox and the host to draw. Its other channel, what a
// page asks its client (calls.ts), is MCP Apps' JSON-RPC, the same on every host. This one is ours and made to be fast,
// as React Native's JSI is: each mutation is a direct call of a host function (`Mutations`), with numbers, strings and
// plain values as they are, never JSON, and the host applies what a turn of the sandbox's event loop changed once it
// has run, as one commit. Each event is a direct call the other way (`Events`).
//
// A host has one runtime version (`RUNTIME_VERSION`), as Expo's native runtime has, which an app's entrypoint names
// (`runtime_version`); a computer whose version is older, or a page that names none, doesn't open it, saying why, and
// a phone's opens it in its web view. A change a host of this
// version couldn't draw (a new type, prop, mutation, token kind, style property or condition, or a change to how they
// cross) is a new version. A host draws every version up to its own.
//
// A host draws only the primitives (`Type`), each styled by styles (`Styles`) whose values are design tokens
// (`Tokens`), and sends back what the person does to them (`EventName`). Every component of the UI library is written
// once on them, so a host draws no component of its own. Both tables come with the UI library and with each app, over
// the air, and a host knows only the kinds of token, the conditions and the style properties, never a token's or a
// style's name or value.

/** The runtime version: what the host's side of the renderer is, which a host says it has, an entrypoint names
 * (`runtime_version`), and the tables say they need. 3 made each mutation and event a direct call, 4 added
 * `intersection`, 5 an app's own tokens (`Styles.tokens`) and the values of a dynamic style (`Dynamic`). */
export const RUNTIME_VERSION = 5

/** A node, as the host knows it. `0` is the root, which the host makes: an app's window, or its frame. */
export type Id = number

/** What a node is: one of the primitives, or a run of text inside a `text` (`raw-text`).
 * - `box`: a flex column unless its style says otherwise, as React Native's View, laid out as Taffy lays out flex and
 *   grid; it clips nothing unless its style says `overflow: "hidden"`.
 * - `scroll`: a box whose content scrolls along its `flexDirection`: down in a column, sideways in a row, with no
 *   scrollbar beside it, as on a phone; one the device draws over it as it moves.
 * - `text`: words. One in a `text` runs on in its line, and one in a box is a block, as React Native's Text. A text
 *   takes its parent's `font`, `fontWeight` and `color` where it has none of its own, as CSS does; the root's are the
 *   page's `body`'s: `label` in `type-body`, on `background`.
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
 *   is (its video's pixels), as core/ui/src/drawing-primitives/input.ts has it (`Input`): the pointer as it moves, its buttons
 *   down and up (1 to 3) and a wheel's steps (4 to 7) with the modifiers held, the keys that aren't text (as X names
 *   them), and what's typed or pasted, which takes a field of its own so a phone's keyboard comes up as it's pressed,
 *   or as its `autofocus` is set. The device's full screen keys (F11, ⌃⌘F) aren't sent: they go on as `keydown` to the node it's in. A host that
 *   doesn't draw it says so (`Mutations.video`), and the page's video shows a problem in its place.
 * - `frame`: another app's page (`src`, `<app>/<page>[#<fragment>]`, for `subject`), drawn in its box as an iframe's
 *   is: its own JS in a sandbox of its own, asking its client as its own app, as tall as it says its content is
 *   (`ui/notifications/size-changed`) where its style says no height, and taking the pointer, the keyboard and Tab
 *   as any box does. Only a trusted app's page shows one. A host that doesn't draw it says so (`Mutations.frame`).
 * - `list`: a column of `count` rows of which the page gives only those the host asks for, each a node with its
 *   `index`, as UIKit's collection view and Android's RecyclerView hold only those near the window: it's as tall as all
 *   of them, the host laying each it has at its own place, those it hasn't as tall as those it has drawn are on
 *   average (`estimate` until it's drawn one), so a page may reuse a row going out of sight for one coming into it,
 *   changing its `index`. As it's scrolled it says `range` with the rows it needs (`first`, and `last`, which isn't
 *   one), half a window behind what shows to two ahead the way it's going, so a fling finds them drawn, and which of
 *   them show (`shownFirst`, `shownLast`). A host that doesn't draw it says so (`Mutations.list`), and the page draws
 *   every row in a `box`.
 * A `raw-text` outside a `text` is drawn as if in one. */
export type Type = "box" | "scroll" | "text" | "image" | "path" | "input" | "picker" | "video" | "frame" | "list" | "raw-text"

/** What a native host gives an app's sandbox, as `globalThis.mutations`, before it loads the app's page: a function for
 * each mutation of its tree, called as Solid's renderer makes it, in order. The host applies what a turn of the
 * sandbox's event loop changed once it has run, and draws it. The page's module, as its entrypoint names it, imports
 * the UI library (`/ui/client/web.js`, `/ui/client/ui.js`), which the host loads from `/ui/client/native/`, the same
 * names, built for it. */
export interface Mutations {
  /** A new node, not yet in the tree. A `raw-text` has its `value`. */
  create(id: Id, type: Type, value?: string): void
  /** One of a node's props (`Props`), or `null` for none. A `raw-text`'s only prop is its `value`. */
  set<Name extends keyof Props>(id: Id, name: Name, value: Props[Name] | null): void
  /** Puts `id` into `parent` before `before`, or at its end: a node already in the tree moves. */
  insert(parent: Id, id: Id, before?: Id): void
  /** Takes `id`, with what's in it, out of `parent`. It may be inserted again. */
  remove(parent: Id, id: Id): void
  /** JS no longer holds `id` (it was garbage collected), so it's never inserted again and the host frees it. */
  drop(id: Id): void
  /** Whether it draws `video`. Where it doesn't, a page's video shows a problem in its place (primitives.native.tsx's
   * `undrawn`). */
  video?: boolean
  /** Whether it draws a `frame`, as `video`. */
  frame?: boolean
  /** Which `list` it draws: 2 lays each row at its own place, so a page may reuse rows; one before (`true`) laid them
   * out as a run, and is drawn every row of in a `box`. */
  list?: boolean | number
}

/** What the UI library gives the host, as `globalThis.ui` (with calls.ts's `Sandbox`), once it's loaded. */
export interface Events {
  /** Something the person did, called directly by the host with the event as a plain object. */
  event(event: Event): void
}

/** Host → JS: what the person did to a node that listens for it (its `on…` prop), one at a time. */
export interface Event {
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
  /** `intersection`'s: whether it's near the window now. */
  isIntersecting?: boolean
  /** A `list`'s `range`: the rows it needs, from `first` to `last`, which isn't one, and which of them show, from
   * `shownFirst` to `shownLast`. */
  first?: number
  last?: number
  shownFirst?: number
  shownLast?: number
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
  /** `wheel`'s, where the system says them (a Mac's trackpad): its fingers' phase, by AppKit's names, and whether it's
   * the momentum after they've lifted. */
  phase?: Phase
  momentum?: boolean
  /** A `video`'s `offer`: its SDP offer, with its candidates. */
  sdp?: string
  /** A `video`'s `state`: its data channel open (`connected`), or its connection lost. */
  state?: "connected" | "lost"
  /** A `video`'s `size`: its video's own, in its pixels. */
  width?: number
  height?: number
}

/** A trackpad's fingers' phase, by AppKit's names. */
export type Phase = "began" | "changed" | "ended" | "cancelled" | "mayBegin"

/** Where a node is drawn in the window, in CSS pixels. */
export interface Rect {
  x: number
  y: number
  width: number
  height: number
}

/** What a node can listen for, as its `on…` prop: `click` is `onClick`, `pointerenter` `onPointerEnter`, as the DOM
 * names them. `click` is a press, by a pointer, a finger, or Enter or Space where the keyboard is; `pointerenter` and
 * `pointerleave` are a mouse or a trackpad's pointer coming over it and going, never a finger; `keydown` a key pressed
 * where the keyboard is, in it or in what's in it; `scroll` a `scroll` scrolling. An `input` says `input` with its
 * editing state whenever it changes (what's typed, a selection, an input method's composing), and `action` when its
 * action key is pressed (a keyboard's Enter, or a phone's Done, Go, Next, Search or Send). A host offers them as
 * accessibility actions too: `click` is AccessKit's `Click`, `input` its `SetValue`, `focus` and `blur` its `Focus`
 * and `Blur`, `scroll` its `ScrollIntoView` and `ScrollDown`. `drop` is files dropped on it from the system (a
 * Finder's, an Explorer's), each its `name` and its bytes as base64 (`body`), under `files`. */
export type EventName = "click" | "input" | "action" | "focus" | "blur" | "keydown" | "pointerenter" | "pointerleave" | "scroll" | "layout" | "intersection" | "range" | "drop" | Moved | Streamed

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

/** Every node's props, as `set` takes them, named as a page's JSX names them, which are the DOM's. One semantic
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
  /** A live region, whose words a screen reader reads as they change, without its focus moving: `polite` once it's
   * idle, `assertive` at once. A `status` is a polite one and an `alert` an assertive one unless it says otherwise. */
  "aria-live"?: "polite" | "assertive" | "off"
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
  /** How it's laid out and looks: its styles (`Styles`) by id, each dynamic one's values (`Dynamic`) after it, the last
   * winning where two say the same property. */
  style?: (StyleId | Dynamic)[]
  /** `image`: a `data:` address, or a file of the app's own by its path. `frame`: the page it shows, as a `one:` link
   * names it after its scheme. */
  src?: string
  /** `frame`: who its page is for, as a `one:` link's is, or whoever this page is for. */
  subject?: string
  /** `image`: what it shows, its accessible name. Empty for one that's only how it looks. */
  alt?: string
  /** `list`: how many rows it has, and how tall one is taken to be until it has drawn one, in CSS pixels. */
  count?: number
  estimate?: number
  /** A row of a `list`: which it is, from 0. */
  index?: number
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
  /** Told whether it's near the window (within half the window's size of what shows of it, as an
   * `IntersectionObserver` with a `rootMargin` of 50% says) as it's first laid out and whenever that changes. */
  onIntersection?: true
  onPointerDown?: true
  onPointerMove?: true
  onPointerUp?: true
  onPointerCancel?: true
  onContextMenu?: true
  onWheel?: true
  /** `box`, `scroll`: follows a pointer or a finger dragging it, as Motion's `drag`, moved by the host where the
   * input arrives: past `dragConstraints` (pixels from where its style puts it) it gives as a rubber band, as loose as
   * `dragElastic`; let go, it's told `dragend` (`offset`, `velocity` in pixels a second) and springs from there, at
   * that speed, to where its style puts it. While it's dragged, it's told `drag` as it moves (`offset`, as far as it's
   * held from where its style puts it), which a host before this one doesn't say. */
  drag?: "x" | "y" | boolean
  dragConstraints?: { top?: number; left?: number; right?: number; bottom?: number }
  dragElastic?: number
  onDragStart?: true
  onDrag?: true
  onDragEnd?: true
  /** `box`, `scroll`: moves from where it was laid out to where it is as that changes, by a spring the host draws
   * (Motion's `layout`), and from where what has the same `layoutId` was a moment ago, where it's new. */
  layout?: boolean
  layoutId?: string
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

/** ARIA's roles a node may have. */
export type Role =
  | "alert" | "button" | "checkbox" | "dialog" | "group" | "heading" | "img" | "link" | "list" | "listitem" | "main"
  | "menu" | "menuitem" | "navigation" | "progressbar" | "radio" | "radiogroup" | "region" | "searchbox" | "separator"
  | "slider" | "status" | "switch" | "tab" | "tablist" | "tabpanel" | "textbox" | "toolbar" | "banner" | "alertdialog"
  | "listbox" | "option" | "combobox" | "form" | "tooltip"

// The tables. The UI library's build makes both from its own TypeScript (core/ui/src/tokens.ts, and each component's
// styles), as it makes the web's CSS from them; each app's build makes its styles' the same way. A host fetches them
// as it fetches a page's modules: the tokens and the UI library's styles at `/ui/client/tokens.json` and
// `/ui/client/styles.json`, an app's styles at `styles.json` beside its page's `index.html`. Each says the runtime
// version it needs (`v`); a host whose version is older opens the page in a web view, as for an entrypoint's
// `runtime_version`.

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
  | { kind: "material"; value: Varies<{ blur: Length; saturate: number; system?: boolean }> }
  /** Font families, the first a device has. */
  | { kind: "font"; value: string[] }
  /** A font weight, 1 to 1000, lighter in `dark` where light text on dark looks heavier. */
  | { kind: "weight"; value: Varies<number> }
  /** A type style (DESIGN.md: text is one of a few): its size, line height (a multiple of its size), weight,
   * tracking (in ems) and whether its figures line up (tabular). */
  | { kind: "type"; value: { font: TokenId; size: Varies<Length>; lineHeight: number; weight: TokenId; tracking: number; tabular: boolean } }
  /** How a change moves: how long it takes, in milliseconds, and its easing, a cubic Bézier's two points; or a spring,
   * by its `bounce` (SwiftUI's `.spring(duration:bounce:)`), which a host that draws only Béziers draws by `easing`. */
  | { kind: "motion"; value: { duration: Varies<number>; easing: Varies<[x1: number, y1: number, x2: number, y2: number]>; bounce?: number } }

/** What a dynamic style (`styles()` with a function) was called with: its values by property, a number of pixels for
 * a length (unitless for `opacity`, `scale`, `flexGrow`, and degrees for `rotate`), a hex colour, or a percentage or
 * `auto`, which a host adds to the style it has by id, as its last, so they win. The properties are longhands. */
export type Dynamic = Record<string, number | string>

/** A style's id. The same style is the same id in every table, so a host may keep them all in one. */
export type StyleId = string

/** The styles of the UI library or an app, by id. */
export interface Styles {
  v: number
  styles: Record<StyleId, Style>
  /** An app's own tokens, and the UI library's it names new values for, for its page alone: by id as `Tokens` has
   * them, those it names winning over `/ui/client/tokens.json`'s. */
  tokens?: Record<TokenId, Token>
}

/** Keyframes, as CSS's `@keyframes`: a style by where each is in a time through, `from`, `to` or a percentage. */
export type Keyframes = { [at in "from" | "to" | `${number}%`]?: { [P in keyof Properties]?: Properties[P] } }

/** A node's state, which a style's value may depend on, as it may on a condition: `hover`, a mouse's pointer over it
 * (never a finger); `pressed`, a pointer or a finger down on it, or Enter or Space held where the keyboard is;
 * `focused`, where the keyboard is, as CSS's `:focus-visible`; `entering`, what it starts from as it's added, to
 * transition from (CSS's `@starting-style`); `exiting`, what it goes to as it's taken out, which a native host draws
 * it moving to before it's gone, as Motion's `exit` (the DOM takes it out at once). A later one wins, as conditions
 * do. */
export type State = "hover" | "pressed" | "focused" | "entering" | "exiting"

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
  /** A motion that repeats while it's drawn, at a motion token's pace and easing, which a native host runs by itself,
   * never waking the page: `spin`, a turn; `pulse`, fading to half and back; or keyframes, as CSS's `@keyframes`, each
   * a style by where it is (`from`, `50%`, `to`), what one leaves out the node's own (a shimmer:
   * `{ from: { translateX: "-100%" }, to: { translateX: "100%" } }`). */
  animationName: "spin" | "pulse" | "none" | Keyframes
  animation: Ref
  /** What of it is about to move, grow or fade (`translateX`, `translateY`, `scale`, `opacity`), as CSS's
   * `will-change`: drawn ahead as a layer of its own, which then only moves (`view::Layer`). */
  willChange: (keyof Properties)[]
}
