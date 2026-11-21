// The UI library as an app's page sees it (`tsconfig.app.json` names it `ui`): the drawing primitives, the
// components drawn with them, and what an app asks for in passing, each with only the props a native host draws
// too. Nothing here is the DOM's. The UI library's own typecheck proves its components take these (../check.ts). A
// component an app needs that isn't here yet is added here, with only the props every host can draw.
import type { Element } from "solid-js"
import type { Ice, Props, Rect } from "../drawing-primitives/renderer.ts"
import type { Look, Styled, Styles } from "../styles.ts"
import type { tokens as Tokens } from "../tokens.ts"

export type { Element, Ice, Look, Rect, Styled, Styles }

// Styles and tokens (../../styles.ts, ../../tokens.ts).
export { createTheme, defineVars, styles, negative } from "../styles.ts"
export declare const tokens: typeof Tokens

/** What a node says it is, and its states, as ARIA has them. */
export type Semantics = Pick<Props, Extract<keyof Props, "role" | "id" | `aria-${string}`>>

// The primitives.

// What a page moved by hand is told (renderer.ts's `Moved`), each a part of the DOM's event of that name, which is
// what it's given on the DOM.

/** Where it happened, in the window, and where the node is drawn there (`getBoundingClientRect()`): a click's, a
 * `contextmenu`'s. */
export type Located = {
  clientX: number
  clientY: number
  timeStamp: number
  currentTarget: { getBoundingClientRect(): Rect }
}
/** A pointer's, a finger's or a mouse's: which one, what it is, and which button, `0` the main one. */
export type Pressed = Located & {
  pointerId: number
  pointerType: "mouse" | "touch" | "pen"
  button: number
}
/** What a node dragged is told as it's let go, as Motion's `PanInfo`: how far it went from where it was pressed, and
 * how fast it was going, in pixels a second, with where it's drawn. */
export type Dragged = Laid & {
  offset: { x: number; y: number }
  velocity: { x: number; y: number }
  canceled: boolean
}
/** Where a node is drawn, as it's laid out. */
export type Laid = {
  currentTarget: { getBoundingClientRect(): Rect }
}
/** A trackpad's or a wheel's scroll, in CSS pixels. Where the system says them, which a native host on a Mac does and
 * the DOM doesn't: the trackpad's fingers' `phase`, by AppKit's names, and whether it's the `momentum` after they've
 * lifted. Where a wheel turned by clicks, as a mouse's does, how many it turned along and down (`notchesX`,
 * `notchesY`, the deltas' way), which a native host says on every system and the DOM where it says lines (Firefox):
 * what tells it from a trackpad's scroll, or a wheel's that turns smoothly, which has none. */
export type Wheeled = {
  deltaX: number
  deltaY: number
  timeStamp: number
  phase?: "began" | "changed" | "ended" | "cancelled" | "mayBegin"
  momentum?: boolean
  notchesX?: number
  notchesY?: number
}

/** What a key pressed is told, as the DOM's `KeyboardEvent`: `preventDefault()` keeps what the key does by itself
 * in a field (a caret moved, the focus moved on by Tab) from happening, on the DOM. */
export type Keyed = {
  key: string
  shiftKey: boolean
  altKey: boolean
  ctrlKey: boolean
  metaKey: boolean
  preventDefault(): void
}

/** Whether a box is near the window: within half the window's size of what shows of it. */
export type Intersected = {
  isIntersecting: boolean
}
/** What a scroll view scrolling is told: how far it's scrolled. */
export type Scrolled = {
  currentTarget: { scrollTop: number; scrollLeft: number }
}

/** What every primitive but an image and a text input takes: how it looks, and what it listens for. */
type Listens = Semantics & {
  style?: Styles
  /** What Tab reaches, as the web's `tabIndex`: `0` for what it otherwise wouldn't, `-1` for what's focused only by a
   * press. */
  tabIndex?: 0 | -1
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
  onPointerEnter?: () => void
  onPointerLeave?: () => void
  /** Once a pointer is down on it, its moves and its end come here, wherever it goes, as a touch's do. */
  onPointerDown?: (event: Pressed) => void
  onPointerMove?: (event: Pressed) => void
  onPointerUp?: (event: Pressed) => void
  onPointerCancel?: (event: Pressed) => void
  /** A right-click, or a touch held, as the device has it, in place of the device's own menu. */
  onContextMenu?: (event: Located) => void
  onWheel?: (event: Wheeled) => void
  /** Told where it's drawn as it's laid out, and each time that changes, as React Native's `onLayout`. */
  onLayout?: (event: Laid) => void
  /** Told whether it's near the window, as it's first laid out and each time that changes, as an
   * `IntersectionObserver` with a `rootMargin` of 50% tells: what draws only what's near what shows does it by this. */
  onIntersection?: (event: Intersected) => void
  /** Has the keyboard as it's drawn, and again each time it's set. */
  autoFocus?: boolean
  /** Out of reach, with all that's in it, as the DOM's `inert` is: never pressed, never reached by Tab, never read,
   * as what's faded out is. `undefined`, not `false`, for reachable. */
  inert?: true
  children?: Element
}

export type ViewProps = Listens & {
  /** Files dropped on it from the system (a Finder's, an Explorer's). */
  onDrop?: (files: File[]) => void
  /** Follows a pointer or a finger dragging it, as Motion's `drag`: along (`"x"`), down (`"y"`) or both (`true`),
   * moved by the host where the input arrives, never by a turn of the page's JS. */
  drag?: "x" | "y" | boolean
  /** How far it may be dragged up, left, right and down from where its style puts it, in pixels; past that it gives
   * as a rubber band, as loose as `dragElastic` (0 none, 1 loose, 0.5 if unsaid). */
  dragConstraints?: { top?: number; left?: number; right?: number; bottom?: number }
  dragElastic?: number
  /** Moves from where it was laid out to where it is as that changes, as Motion's `layout`: a list reordered, a card
   * grown. The host draws it on its way, by a spring, with no layout again each frame. */
  layout?: boolean
  /** What it shares its place with, as Motion's `layoutId` and SwiftUI's `matchedGeometryEffect`: one drawn where
   * another with the same id was a moment ago goes there from it (a tab's pill sliding to the tab chosen, an icon
   * growing into its window). */
  layoutId?: string
  onDragStart?: () => void
  /** Dragged on, as Motion's `onDrag`: how far it's held from where its style puts it, as it moves. A host before
   * this one doesn't say it. */
  onDrag?: (event: Pick<Dragged, "offset">) => void
  /** Let go: how far it went and how fast it was going, in pixels a second. It springs from where it is, at that
   * speed, to where its style puts it, which this may move. */
  onDragEnd?: (event: Dragged) => void
}
export declare function View(props: ViewProps): Element

export type PressableProps = ViewProps & {
  /** Pressed, by a pointer, a finger, or Enter or Space where the keyboard is. */
  onPress?: (event: Located) => void
  /** Not pressed, and says so. */
  disabled?: boolean
}
export declare function Pressable(props: PressableProps): Element

export type ScrollViewProps = ViewProps & {
  onScroll?: (event: Scrolled) => void
}
export declare function ScrollView(props: ScrollViewProps): Element

export type VirtualizedListProps<T> = Semantics & {
  style?: Styles
  data: readonly T[]
  renderItem: (info: { item: T; index: number }) => Element
  estimatedItemSize?: number
  /** A row to keep drawn wherever the list is scrolled, one the keyboard is being sent to (Home, End), which scrolls
   * into view as it takes it. */
  keep?: number
}
export declare function VirtualizedList<T>(props: VirtualizedListProps<T>): Element

export type TextProps = Semantics & {
  style?: Styles
  /** Pressed where its words are, as an inline link (`role="link"`), reached by Tab with a `tabIndex`. */
  tabIndex?: 0 | -1
  onPress?: (event: Located) => void
  children?: Element
}
export declare function Text(props: TextProps): Element

export type ImageProps = Pick<Semantics, "aria-hidden"> & {
  src: string
  alt: string
  style?: Styles
}
export declare function Image(props: ImageProps): Element

export type SvgProps = Pick<Semantics, "role" | "aria-label" | "aria-hidden"> & {
  viewBox: string
  style?: Styles
  children?: Element
}
export declare function Svg(props: SvgProps): Element

export type PathProps = {
  d: string
  /** Its `fill`, `stroke` and `strokeWidth`. */
  style?: Styles
}
export declare function Path(props: PathProps): Element

/** A screen's live video over WebRTC, which sends what's done on it to the screen itself (renderer.ts's `video`): the
 * page does its signalling, a new `ice` connecting, its `offer` answered with the screen's `answer`. */
export type VideoProps = {
  ice?: Ice
  answer?: string
  /** Looked at only: nothing done on it is sent. */
  watch?: boolean
  "aria-label": string
  /** Has the keyboard, what's typed going to the screen, each time it's set. */
  autoFocus?: boolean
  style?: Styles
  onOffer?: (event: { sdp: string }) => void
  onState?: (event: { state: "connected" | "lost" }) => void
  /** The video's own size, in its pixels, as it comes and changes. */
  onSize?: (event: { width: number; height: number }) => void
}
export declare function Video(props: VideoProps): Element

/** Another app's page, in a box of this one's, as an iframe is (renderer.ts's `frame`): `src` as a `one:` link names
 * it after its scheme, for `subject`, or whoever this page is for. Only a trusted app's page shows one. */
export type FrameProps = {
  src: string
  subject?: string
  /** What it is, its accessible name. */
  title?: string
  style?: Styles
}
export declare function Frame(props: FrameProps): Element

/** What's drawn over the whole window, over the page, while it's `visible` (unless it says, it is), with everything
 * else inert: a sheet, a menu. */
export declare function Modal(props: { visible?: boolean; children?: Element }): Element
/** What's drawn over the whole window, over the page, without taking it from anyone: a toast, a select's list. */
export declare function Portal(props: { children?: Element }): Element

/** What an input's action key says, a phone keyboard's Done, Go, Next, Search or Send, or a new line (`enter`). */
export type EnterKeyHint = "enter" | "done" | "go" | "next" | "previous" | "search" | "send"

export type TextInputProps = Pick<Semantics, "role" | "aria-label" | "aria-description" | "aria-invalid" | "aria-controls" | "aria-activedescendant" | "aria-expanded"> & {
  name?: string
  value?: string
  /** Which keyboard a touch screen shows: `numeric` is digits alone; `email`, `url`, `tel` and `search` are those kinds
   * of field too. */
  inputMode?: Props["inputmode"]
  /** What's typed is hidden, as a password's. */
  secureTextEntry?: boolean
  multiline?: boolean
  placeholder?: string
  /** What its action key says and does: `done` in one line unless it says, a new line in several. */
  enterKeyHint?: EnterKeyHint
  disabled?: boolean
  readOnly?: boolean
  /** All its text is selected as it takes the keyboard, so what's typed replaces it, as React Native's `selectTextOnFocus` does. */
  selectTextOnFocus?: boolean
  required?: boolean
  autoFocus?: boolean
  autoCapitalize?: Props["autocapitalize"]
  autoCorrect?: boolean
  spellCheck?: boolean
  autoComplete?: string
  style?: Styles
  /** What's in it now, each time that changes. */
  onChangeText?: (text: string) => void
  /** Its action key was pressed: Return in one line, or a phone's Done, Go, Search or Send. */
  onSubmitEditing?: () => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
}
export declare function TextInput(props: TextInputProps): Element

// The components.

// Props are types, not interfaces, so the DOM's components, which take any `data-` attribute too, take them.

export declare function render(code: () => Element): () => void

export declare function Screen(props: { heading?: Element; action?: Element; back?: boolean; onBack?: () => void; largeTitle?: boolean; children?: Element }): Element
export declare function Section(props: { heading?: Element; "aria-label"?: string; children?: Element }): Element
/** A row that scrolls sideways from the window's edge to its edge, its first item where the screen's column starts (`style` may say its `gap`). */
export declare function Shelf(props: { role?: "group" | "toolbar" | "list"; "aria-label"?: string; style?: Styles<"gap">; children?: Element }): Element
export declare function Split(props: { open?: boolean; onClose?: () => void; detail?: Element; children?: Element }): Element
/** Where a window's Back and Forward go (`History`), which its screens draw in their bar. */
export declare function History(props: { value: { back?: () => void; forward?: () => void } | null; children: Element }): Element
/** A listbox: which of its rows is chosen, and what's typed in that drives it (`createListbox`). */
export interface Listbox {
  readonly id: string
  chosen: () => number
  choose: (index: number) => void
  field: () => { role: "combobox"; "aria-controls": string; "aria-expanded": "true" | "false"; "aria-activedescendant"?: string }
  keys: (event: Keyed) => boolean
}
export declare function createListbox(): Listbox
/** How fast a finger was going as it lifted, from its moves (`createVelocityTracker`): `add` each move, then
 * `velocity` as it lifts, in pixels a millisecond. */
export declare function createVelocityTracker(options?: { horizon?: number; staleAfter?: number; maxSpeed?: number; capacity?: number }): {
  add(time: number, position: number): void
  velocity(now: number): number
  reset(): void
}
/** How far something let go at `velocity` (px/ms) goes on by itself, slowing as a phone's scrolling does: UIKit's
 * normal deceleration rate unless `rate` says. Where it would carry on to is where a page lands, as an iPhone's do. */
export declare function projected(velocity: number, rate?: number): number
export declare function List(props: { listbox?: Listbox; label?: string; children?: Element }): Element
export declare function List<T>(props: { listbox?: Listbox; label?: string; each: readonly T[]; keyed?: (item: T) => unknown; children: (item: () => T, index: () => number) => Element }): Element
/** How the lists in it are drawn (`ListStyle`): plain, inset grouped cards, or a window's sidebar. */
export declare function ListStyle(props: { value: "plain" | "insetGrouped" | "sidebar"; children: Element }): Element
export declare function Row(props: {
  heading?: Element
  text?: Element
  start?: Element
  end?: Element
  /** Pressed, with where it's drawn; Return in a listbox opens it with none. As React Native's `onPress`. */
  onPress?: (event?: Located) => void
  current?: boolean
  control?: Element
  action?: Element
  /** Something destructive it does, its heading red. */
  destructive?: boolean
}): Element

export type IconName =
  | "arrow-elbow-down-left" | "calendar-blank" | "caret-down" | "caret-left" | "caret-right" | "caret-up-down" | "check" | "copy" | "corners-in"
  | "corners-out" | "device-mobile" | "devices" | "dots-three" | "file" | "gear-six" | "globe" | "hard-drives" | "magnifying-glass" | "minus" | "minus-circle"
  | "paperclip" | "pencil-simple" | "plus" | "plus-circle" | "robot" | "squares-four" | "trash" | "warning-circle" | "x"
export declare function Icon(props: { name: IconName; along?: boolean }): Element
export declare function Spinner(): Element

export type ButtonProps = {
  variant?: "default" | "secondary" | "outline" | "ghost" | "destructive"
  size?: "icon"
  /** Its ends rounded fully, as SwiftUI's capsule: a button that is a pill of its own. */
  shape?: "capsule"
  loading?: boolean
  disabled?: boolean
  "aria-label"?: string
  "aria-haspopup"?: "menu" | "dialog" | "listbox"
  "aria-expanded"?: "true" | "false"
  autoFocus?: boolean
  /** What pressing it does, as React Native's `onPress`. */
  onPress?: (event: Located) => void
  onLayout?: (event: Laid) => void
  /** Where it sits in what it's in, the only part of its look a caller changes. */
  style?: Styles<"marginTop" | "marginBottom" | "marginStart" | "marginEnd" | "alignSelf" | "flexGrow">
  children?: Element
}
export declare function Button(props: ButtonProps): Element
export declare function Badge(props: { variant?: "default" | "secondary" | "destructive" | "outline"; "aria-label"?: string; children?: Element }): Element

export declare function Empty(props: { action?: Element; children?: Element }): Element
export declare function Skeleton(props: { rows?: number }): Element
export declare function Problem(props: { heading: Element; text?: Element | Error; action?: Element; warn?: boolean }): Element
export declare function Status(props: { heading: Element; text?: Element; over?: boolean }): Element
export declare function Steps(props: { steps: Element[]; at: number; text?: Element }): Element
/** The one screen someone signs in on: whose it is, a heading, what they compare or are warned of, and one button. */
export declare function SignIn(props: {
  mark?: string | Element
  heading?: Element
  button?: Element
  quiet?: boolean
  onSignIn?: () => unknown
  problem?: Element
  code?: Element
  note?: Element
  warning?: Element
  stop?: Element
  onStop?: () => unknown
  beside?: boolean
  done?: Element
  status?: Element
  children?: Element
}): Element

export declare function Field(props: { label?: Element; hint?: Element; error?: string; disabled?: boolean; children?: Element }): Element
/** An `Input`'s props: a text input's, in a field's box, with styles of its own over it. */
export type InputProps = Omit<TextInputProps, "style" | "multiline"> & {
  style?: Styles
}
export declare function Input(props: InputProps): Element
export declare function Textarea(props: Omit<InputProps, "style">): Element
export declare function Search(props: Omit<InputProps, "inputMode" | "style">): Element
/** A `Checkbox`'s or a `Switch`'s props: whether it's on (`value`), and what's told when it changes, as React
 * Native's `Switch` has them. */
export type ChoiceProps = {
  label?: Element
  hint?: Element
  error?: string
  value?: boolean
  disabled?: boolean
  "aria-label"?: string
  onValueChange?: (value: boolean) => void
}
export declare function Checkbox(props: ChoiceProps): Element
export declare function Switch(props: ChoiceProps): Element
export declare function RadioGroup(props: {
  value?: string
  options: { value: string; label: Element; disabled?: boolean }[]
  disabled?: boolean
  onValueChange?: (value: string) => void
}): Element

/** What a select chooses from, and what it's told of what's chosen, as React Native's `Picker` has them. */
export type Choice = { value: string; label: string; disabled?: boolean }
export declare const Select: {
  Root: (props: {
    items: Choice[]
    value?: string
    defaultValue?: string
    placeholder?: string
    disabled?: boolean
    "aria-label"?: string
    onValueChange?: (value: string) => void
  }) => Element
}

/** What a `LiveScreen` is told: its `action`, and what that action takes, as Computer Use's screens have them. */
export type ScreenInputs = { action: string } & Record<string, unknown>
export declare function LiveScreen(props: { label: string; screen?: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean; children?: Element }): Element

export declare function Alert(props: { heading: Element; text?: Element; open?: boolean; onClose?: () => void; action?: Element; children?: Element }): Element

export declare function Sheet(props: { heading?: Element; open?: boolean; onClose?: () => void; children?: Element }): Element

/** Something that can be done, as the action panel lists it: what it says, what does it, the keys that do it too
 * (`["mod", "enter"]`), an icon, whether it can't be undone, whether it can't be done now, and its section. */
export type Action = { label: string; onAction: () => void; keys?: string[]; icon?: IconName; destructive?: boolean; disabled?: boolean; section?: string; menu?: boolean }
export declare function Keys(props: { keys: string[] }): Element
export declare function ActionPanel(props: { open: boolean; onOpenChange: (open: boolean) => void; actions: Action[] }): Element
export declare function ActionBar(props: { start?: Element; actions?: Action[] }): Element

export declare const Metadata: ((props: { children?: Element }) => Element) & { Label: (props: { title: Element; text?: Element }) => Element }

export declare const Menu: {
  Root: (props: { open?: boolean; onOpenChange?: (open: boolean) => void; onSelect?: (value: string) => void; children?: Element }) => Element
  Trigger: (props: { variant?: ButtonProps["variant"]; size?: "icon"; shape?: ButtonProps["shape"]; "aria-label"?: string; children?: Element }) => Element
  Positioner: (props: { children?: Element }) => Element
  Content: (props: { children?: Element }) => Element
  Item: (props: { value: string; destructive?: boolean; disabled?: boolean; children?: Element }) => Element
  /** Items under a small heading, as iOS's menus group them. */
  Group: (props: { label: Element; children?: Element }) => Element
  Separator: (props: {}) => Element
}

export declare const Tabs: {
  Root: (props: { value?: string; defaultValue?: string; onValueChange?: (value: string) => void; children?: Element }) => Element
  List: (props: { children?: Element }) => Element
  Trigger: (props: { value: string; children?: Element }) => Element
  Content: (props: { value: string; children?: Element }) => Element
  Indicator: (props: {}) => Element
}

/** A widget's layout, as its tool answers it (the skill `building-apps` lists its parts). */
export type Layout = { [part: string]: unknown }
export declare function Widget(props: {
  name?: string
  size?: "2x2" | "4x2" | "4x4"
  layout?: Layout
  problem?: string
  onCall?: (tool: string | undefined, inputs: Record<string, unknown>) => unknown
  /** Pressed, with where the card is drawn, which the app it opens grows out of. */
  onOpen?: (card: { getBoundingClientRect(): Rect }) => void
}): Element

// Scenes: a component's or a screen's states, named, each drawn as it is with what it shows, for the gallery
// (core/ui/gallery) and for drawing each natively and on the web (core/ui/scenes.js). A scene is what to
// draw, or that with what to press first (`taps`, by what each is named or says), whether it's drawn only by itself
// (`alone`: something over the page), drawn edge to edge in its window, as a whole screen is, rather than inset in it
// (`layout: "fullscreen"`, as Storybook's), how big its window is (`size`, a phone's 390 × 844 unless said) and whether a
// finger points at it (`touch`), what its window keeps at each edge (`safeArea`, top, right, bottom and left: a phone's
// status bar and home indicator), the gestures played on it, frame by frame (`motions`, core/ui/motion.js), and what
// people do with it and what must hold as they do (`checks`, `bun motion.js --checks`), with the seeds a check once
// failed on (`seeds`). A page that logs an error fails the scene, as a browser's console does a test, but for the
// words a scene that shows an error says it logs (`errors`: "Couldn't reach the network"). Its name is its family's and its own: "Buttons/Variants".
export type Scene =
  | (() => Element)
  | { render: () => Element; taps?: Tap[]; alone?: boolean; layout?: "fullscreen"; size?: [number, number]; touch?: boolean; errors?: string[]; safeArea?: [number, number, number, number]; motions?: Record<string, Motion>; checks?: Record<string, CheckStep[]>; seeds?: number[] }
/** A step of a check, as a test's (Playwright's, the UI SDK's): what's done, as a tap is (pressed, a key, typed), a
 * press held `hold` ms or tapped `clicks` times (an `agent`'s comes as an agent's request to a device's window does, by the inspector, though
 * nobody could press it, as on a panel put away), the pointer moved onto something (`hover`), a button held `down` on something and let go (`up`), input played as
 * a device delivered it (`trace`, a JSON file of `{ ms, kind, … }` beside the scenes, as the harness's `trace` takes
 * it: a trackpad's swipe with its phases and momentum, the pointer's nudges, a finger's `touch`es, each with its
 * `phase`), time passing (`wait`, ms), the app in the background that long and back (`away`, ms), which fails where it
 * draws meanwhile, as a phone's GPU refuses that, a panel put away or summoned (`putAway`, `summon`), or the device's own
 * Back (`back`), as Android's key is; the client's answers changed to these tools' and the page told that something
 * changed (`answers`, as an update going away is), or what must
 * hold then (`expect`), waited for a second, or `within` ms. */
export type CheckStep = Tap | { press: Tap; hold?: number; clicks?: number; agent?: true } | { hover: Tap } | { down: Tap } | { up: true } | { trace: string } | { wait: number } | { away: number } | { putAway: true } | { summon: true } | { back: true } | { answers: Record<string, unknown> } | { expect: Expect; within?: number }
/** What must hold: what's drawn whole and in the window (`shown`), what isn't there (`gone`), what has the keyboard
 * (`focused`), words somewhere on it (`text`), a node's states (`states`: `{ of, is: { expanded: true } }`), how opaque
 * it's drawn (`drawn`: `{ of, opacity }`, a pressed look), the glass's blur drawn or not (`blur`), what's over the page dimming it (`dimmed`, as a sheet's or an alert's scrim does), and what the input with the
 * keyboard asks the platform's keyboard for (`keyboard`, as Flutter's `TextInputConfiguration` says it to iOS and
 * Android: `{ textCapitalization: "TextCapitalization.none", autocorrect: false }`), and every text drawn in the window
 * at least `margins` points in from its sides, as a page's margins keep it, and what's drawn as tall as it should be (`tall`:
 * a node of that name, `height` points, within one), as a type size is; every text in sight drawn whole, none cut
 * short with "…", or the one named (`whole`); these texts each as wide a character as the others, as a monospace face's
 * are (`monospace`: the texts, each on one line); no line of a text ending inside a run a pattern finds, as a date is kept
 * on one line (`unbroken`: a regular expression's source); what's named drawn to both of the window's edges (`bleeds`), as a
 * shelf is; every node named the same distance along its row from the window's end, or its start with `at: "start"`
 * (`aligned`: `{ of, at? }`), as controls at the end of rows form a column; nothing of what's named or inside it clipped away (`unclipped`), as a badge
 * overhanging its icon is drawn whole; what's named in the middle of the window across, within a point
 * (`centred`); nothing named so wider than `most` points (`widest`: `{ of, most }`); what's named standing off what's
 * around it by at least `least`:1, WCAG's contrast between the colour just outside its start and its first 4 points in,
 * where a control's ring or a card's edge is (`contrast`: `{ of, least }`); and what a phone's system is asked to show of
 * a finger's selection: its menu for text with these items, inside the safe area, or none (`textMenu`:
 * `{ items: ["cut", "copy", "paste", "selectAll"] }`, or false), so many handles drawn (`handles`), and its magnifier
 * or not (`magnifier`), what's over the page dimming it (`dimmed`, as a sheet's or an alert's scrim does), and at most `js`
 * MB held by its page's JS, collected first (the inspector's `js_bytes`), as a page that keeps what it no longer shows
 * grows past it, whether `settle` says it's `settled`, and where something is drawn (`at`: `{ of, x, y, width }`, each
 * given in points, within one), so what comes in above it is seen not to push it, and a bar's glass is seen to reach the
 * window's edges. */
export type Expect = { shown?: Tap; gone?: Tap; focused?: Tap; text?: string; states?: { of: Tap; is: Record<string, boolean | string> }; drawn?: { of: Tap; opacity: number }; blur?: boolean; dimmed?: boolean; keyboard?: Record<string, unknown>; margins?: number; tall?: { of: Tap; height: number }; whole?: true | Tap; unbroken?: string; monospace?: string[]; unclipped?: Tap; bleeds?: Tap; aligned?: { of: Tap; at?: "start" | "end" }; centred?: Tap; widest?: { of: Tap; most: number }; contrast?: { of: Tap; least: number }; textMenu?: { items?: string[] } | false; handles?: number; magnifier?: boolean; js?: number; settled?: boolean; at?: { of: Tap; x?: number; y?: number; width?: number } }
/** A press that goes down on what `from` names (as a tap does) and moves as `gesture` says, or input played as a device
 * delivered it (a `trace` file of events, as a check's `trace`), with what's watched move (`follow`, named as a tap is;
 * what scrolls under it is always watched). `steady`: what moves never goes back against its way before its input
 * ends, as a swipe that bounces does. `frameMs`: its frames drawn only that often,
 * as a device others load draws them (Telemetry's frame interval), which presses it as a window's late frames do.
 * `blurred`: glass shows blurred in every frame after its first, as a veil it opens does. `keeps`: the tier of
 * core/ui/devices.json whose budget each of its frames keeps, at that tier's rate, the frames a press lands and lifts
 * on too, their JS, painting and GPU as that device would take them. `opensFrom`: what `follow` names is first drawn from
 * where this is, its top and its middle there (within a point), as an iPhone's folder grows out of its square and its
 * apps out of the square's. `once`: what it names is never seen
 * twice in a frame, whether a screen reader reads it or not, as a large title and its bar's small one are cross-faded.
 * `eases`: what each turn of a mouse's wheel moves goes over several frames, never as far as a turn in one, and never
 * holds still while the wheel turns, as Windows' apps (WinUI) and Chrome smooth-scroll a notch (a Mac's moves at once).
 * However it's played, what it takes out is gone once it settles. */
export type Motion = { from?: Tap; gesture: Gesture; follow?: Tap; frameMs?: number; blurred?: boolean; steady?: boolean; keeps?: string; opensFrom?: Tap; once?: Tap; eases?: boolean }
/** How a press moves, in points and points a second: swiped at a steady `speed` (800 unless said) and let go; flung,
 * a short quick flick; dragged with `pauses` of a fifth of a second along the way; a sheet pulled down and let go; or
 * each stretch it moves (`moves`, a stretch with neither `dx` nor `dy` holding still), held at the end unless `lift`. */
export type Gesture =
  | { swipe: [number, number]; speed?: number }
  | { fling: [number, number] }
  | { drag: [number, number]; pauses?: number; speed?: number }
  | { pull: number }
  | { moves: { dx?: number; dy?: number; ms: number }[]; lift?: boolean }
  | { trace: string }
/** What's done, in order (core/ui/scenes.js runs them): pressed by what it's named or says, or that and its role
 * where a name alone is two things (a field's label and its select), or by its role alone (the menu open); a point pressed; a key as the DOM names it, with
 * any of `ctrl`, `shift`, `alt` or `meta` held; or text typed where the keyboard is. */
export type Tap =
  | string
  | { role: string; name?: string }
  | { x: number; y: number }
  | { key: string; ctrl?: boolean; shift?: boolean; alt?: boolean; meta?: boolean }
  | { type: string }
export type Scenes = Record<string, Scene>

// What an app asks for in passing.

export type Subject = {
  subject?: string
}
export declare function call(tool: string, inputs?: object, options?: Subject): Promise<string>
/** Sends a file the person chose where the network keeps it, for a tool of this app's: the name that tool takes for it,
 * in the input its app says is an upload. */
export declare function upload(file: File | Blob): Promise<string>
/** The device's own picker of files to open, from a press: of `accept`'s kinds (`.png`, `image/*`), one or
 * `multiple`. → the files chosen, none where it was cancelled. */
export declare function pickFiles(options?: { accept?: string[]; multiple?: boolean }): Promise<File[]>
/** `changed(() => refresh(notes))`: runs `listener` whenever what this page was answered has changed, to ask again and
 * draw it, but not while the page isn't shown (`visible`): a change then is told once, as it's shown again, as a
 * browser's background tab does nothing until it's looked at. → the function that stops it. */
export declare function changed(listener: () => void): () => void
/** `memoryWarning(() => setKept([]))`: runs `listener` as the page holds nearly all the memory it may have, for it to
 * let go of what it can make again, as iOS's `didReceiveMemoryWarning` tells an app. A browser never tells it. → the
 * function that stops it. */
export declare function memoryWarning(listener: () => void): () => void
export declare function toast(text: string | Error, options?: { action?: { label: string; run: () => void } }): Promise<void>
export declare function confirm(question: { heading: string; text?: string; action?: string; destructive?: boolean }): Promise<boolean>
/** `behind` only starts it, as a finger lands on its icon; `from` is where its icon or widget is drawn, with its corners'
 * `radius`, which it grows out of. */
export declare function open(app: string, options?: Subject & { behind?: boolean; from?: Rect & { radius?: number } }): Promise<unknown>
export declare function leave(): Promise<void>
export declare function link(): string | undefined
export declare function component(app: string, name: string): Promise<Record<string, (props: Record<string, unknown>) => Element>>
export declare function showRecoveryKey(options?: Subject): Promise<boolean>
export declare function useRecoveryKey(options?: Subject): Promise<boolean>
export declare function go(change: () => void, options?: { back?: boolean }): void
export declare function share(text: string): Promise<"shared" | "copied" | undefined>
export declare function date(at: string | number | Date): string
export declare function seen(at: string | number | Date): string
export declare function since(at: string | number | Date, style?: "long" | "narrow"): string
export declare function size(bytes: number): string
export declare function haptic(kind: "light" | "medium" | "heavy" | "rigid" | "soft" | "selection" | "success" | "warning" | "error"): void
export declare function connection(): "connecting" | "online" | "offline" | "reconnecting"
/** Whether this page is shown, as it changes: not while its person is in another app, as their device's Settings. */
export declare function visible(): boolean

/** What `client()` answers: what this device is, and what it can do that an app asks of it. */
export type Client = {
  device: string
  network?: string
  build: string
  /** This device as a server, while it is one or is becoming one: its key, and where becoming one or stopping being
   * one is. */
  server?: {
    key: string
    server_membership: "joining" | "active" | "leaving"
    server_membership_stage?: "registering" | "downloading" | "installing" | "starting" | "draining" | "deregistering" | "uninstalling" | null
  }
  push?: { state: "notAsked" | "denied" | "registering" | "registered" | "homeScreen" } | { state: "failed"; why: string }
  computerUse?: { on: boolean }
  /** Whether a computer's app opens when its person logs in, as the system says; null where this device or build has none. */
  login?: boolean | null
  theme: "light" | "dark" | null
  pinsShortcuts?: boolean
  settings?: boolean
  launcher?: boolean
  /** Whether the launcher's panel is see-through where its page draws nothing, so what it draws floats over the
   * desktop, on the system's glass where it asks for it (`tokens.material.window`). */
  transparent?: boolean
  /** Whether it's a Mac (or an iPhone or iPad), whose keys are ⌘ where a PC's are Ctrl. */
  mac?: boolean
}
/** What `client(change)` changes. */
export type ClientChange = {
  theme?: "light" | "dark" | null
  statusBar?: boolean
  leave?: boolean
  serve?: boolean
  /** A computer's app opens when its person logs in, or not. */
  login?: boolean
  update?: boolean
  push?: boolean
  settings?: "accessibility" | "app"
}
export declare function client(): Promise<Client>
export declare function client(change: ClientChange): Promise<{ message: string }>
/** Something of the page's own is open over it, which `shut` shuts: the device's own back calls it first. With
 * `layer: false`, it's what the device's back does on a screen, and nothing is drawn over the page (a screen's own
 * Back, a split's detail beside its list). → what to call once it has shut. */
export declare function covers(shut: () => void, options?: { layer?: boolean }): () => void
/** How large the page's content is, for a window sized by what it shows (a computer's launcher). */
export declare function sizeChanged(size: { width?: number; height?: number }): Promise<void>
/** Puts an app, by its entrypoint, on the device's own home screen, where `client()` says it can (`pinsShortcuts`). */
export declare function pinShortcut(entrypoint: string): Promise<unknown>
/** The window's size, in CSS pixels, as it changes. */
export declare function viewport(): { width: number; height: number }
/** Which way the page's words run, as its language has them. */
export declare function direction(): "ltr" | "rtl"
/** What the device keeps for itself at each edge of the window (a status bar, a notch, a home indicator), as it
 * changes: the CSS's `env(safe-area-inset-*)`. */
export declare function safeArea(): { top: number; right: number; bottom: number; left: number }
/** Whether whoever's looking asked for less motion, as it changes. */
export declare function reducedMotion(): boolean
/** Calls `listener` with each key pressed that isn't something's own: not in a field, a menu or a dialog, which take
 * their keys themselves. → what stops it. */
export declare function keydown(listener: (event: Keyed) => void): () => void
