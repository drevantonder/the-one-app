// The UI library as a strict app sees it (`tsconfig.strict.json` names it `ui`): the strict primitives, the
// components that are strict, and what an app asks for in passing, each with only the props a native host draws
// too. Nothing here is the DOM's. The UI library's own typecheck proves its components take these (../check.ts),
// so a strict app runs unchanged on the DOM. A component an app needs that isn't here yet is added here, with only
// the props every host can draw, and the check says whether the DOM's takes them.
import type { Element } from "solid-js"
import type { ListCollection } from "@zag-js/collection"
import type { Props, Style } from "../protocol.ts"
import type { Class, Classes } from "../style.ts"

export type { Class, Classes, Element, Style }

/** What a node says it is, and its states, as ARIA has them. */
export type Semantics = Pick<Props, Extract<keyof Props, "role" | `aria-${string}`>>

// The primitives. Each takes its `class` in the strict style vocabulary (../style.ts), written out, which the
// typecheck reads class by class: `class="flex-row gap-2 hover:bg-accent"`, or one of a few such as a condition
// chooses (`class={open() ? "… bg-accent" : "…"}`). `style` is version 1's, for a page written for 1.

/** What a key pressed is told, as the DOM's `KeyboardEvent`. */
export type Keyed = {
  key: string
  shiftKey: boolean
  altKey: boolean
  ctrlKey: boolean
  metaKey: boolean
}

/** What a `Scroll` scrolling is told: how far it's scrolled. */
export type Scrolled = {
  currentTarget: { scrollTop: number; scrollLeft: number }
}

export type BoxProps<C extends string = ""> = Semantics & {
  class?: Classes<C>
  style?: Style
  tabindex?: 0 | -1
  onClick?: () => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
  onPointerEnter?: () => void
  onPointerLeave?: () => void
  children?: Element
}
export declare function Box<C extends string = "">(props: BoxProps<C>): Element

export type ScrollProps<C extends string = ""> = BoxProps<C> & {
  onScroll?: (event: Scrolled) => void
}
export declare function Scroll<C extends string = "">(props: ScrollProps<C>): Element

export type TextProps<C extends string = ""> = Semantics & {
  class?: Classes<C>
  style?: Style
  children?: Element
}
export declare function Text<C extends string = "">(props: TextProps<C>): Element

export type ImageProps<C extends string = ""> = Pick<Semantics, "aria-hidden"> & {
  src: string
  alt: string
  class?: Classes<C>
  style?: Style
}
export declare function Image<C extends string = "">(props: ImageProps<C>): Element

export type PathProps<C extends string = ""> = Pick<Semantics, "role" | "aria-label" | "aria-hidden"> & {
  d: string
  viewBox: string
  class?: Classes<C>
}
export declare function Path<C extends string = "">(props: PathProps<C>): Element

/** What an `Input`'s or a `Search`'s `onInput` is told: what's in it now. */
export type Typed = {
  currentTarget: { value: string }
}

export type InputProps = Pick<Semantics, "aria-label" | "aria-description" | "aria-invalid"> & {
  name?: string
  value?: string
  type?: Props["type"]
  placeholder?: string
  disabled?: boolean
  required?: boolean
  autofocus?: boolean
  autocapitalize?: Props["autocapitalize"]
  spellcheck?: boolean
  autocomplete?: string
  enterkeyhint?: Props["enterkeyhint"]
  onInput?: (event: Typed) => void
  onFocus?: () => void
  onBlur?: () => void
  onKeyDown?: (event: Keyed) => void
}
export declare function Input(props: InputProps): Element
export declare function Textarea(props: Omit<InputProps, "type">): Element
export declare function Search(props: Omit<InputProps, "type">): Element

// The components.

// Props are types, not interfaces, so the DOM's components, which take any `data-` attribute too, take them.

export declare function render(code: () => Element): () => void

export declare function Screen(props: { heading?: Element; action?: Element; back?: boolean; onBack?: () => void; children?: Element }): Element
export declare function Section(props: { heading?: Element; children?: Element }): Element
export declare function Split(props: { open?: boolean; onClose?: () => void; detail?: Element; children?: Element }): Element
export declare function List(props: { children?: Element }): Element
export declare function Row(props: {
  heading?: Element
  text?: Element
  start?: Element
  end?: Element
  onClick?: () => void
  current?: boolean
  control?: Element
  action?: Element
}): Element

export type IconName =
  | "calendar-blank" | "caret-left" | "caret-right" | "caret-up-down" | "check" | "copy" | "corners-in" | "corners-out"
  | "dots-three" | "file" | "gear-six" | "magnifying-glass" | "paperclip" | "pencil-simple" | "plus" | "trash"
  | "warning-circle" | "x"
export declare function Icon(props: { name: IconName; along?: boolean }): Element
export declare function Spinner(): Element

export type ButtonProps = {
  variant?: "default" | "secondary" | "outline" | "ghost" | "destructive"
  size?: "icon"
  loading?: boolean
  disabled?: boolean
  "aria-label"?: string
  onClick?: () => void
  children?: Element
}
export declare function Button(props: ButtonProps): Element
export declare function Badge(props: { variant?: "default" | "secondary" | "destructive" | "outline"; "aria-label"?: string; children?: Element }): Element

export declare function Empty(props: { action?: Element; children?: Element }): Element
export declare function Skeleton(props: { rows?: number }): Element
export declare function Problem(props: { heading: Element; text?: Element | Error; action?: Element; warn?: boolean }): Element
export declare function Status(props: { heading: Element; text?: Element; over?: boolean }): Element
export declare function Steps(props: { steps: Element[]; at: number; text?: Element }): Element

export declare function Field(props: { label?: Element; hint?: Element; error?: string; disabled?: boolean; children?: Element }): Element
/** What a `Checkbox`'s or a `Switch`'s `onChange` is told: whether it's ticked now. */
export type Ticked = {
  currentTarget: { checked: boolean }
}
export type ChoiceProps = {
  name?: string
  label?: Element
  hint?: Element
  checked?: boolean
  disabled?: boolean
  "aria-label"?: string
  onChange?: (event: Ticked) => void
}
export declare function Checkbox(props: ChoiceProps): Element
export declare function Switch(props: ChoiceProps): Element
export declare function RadioGroup(props: {
  name?: string
  value?: string
  options: { value: string; label: Element; disabled?: boolean }[]
  required?: boolean
  disabled?: boolean
  onChange?: (value: string) => void
}): Element

/** What a select chooses from, and what it's told of what's chosen, as Ark UI's select has them. */
export type Choice = { value: string; label: string; disabled?: boolean }
export type Choices = ListCollection<Choice>
export declare function createListCollection(options: { items: Choice[] }): Choices
export declare const Select: {
  Root: (props: {
    name?: string
    collection: Choices
    value?: string[]
    defaultValue?: string[]
    placeholder?: string
    disabled?: boolean
    onValueChange?: (details: { value: string[] }) => void
  }) => Element
}

/** What a `LiveScreen` is told: its `action`, and what that action takes, as Computer Use's screens have them. */
export type ScreenInputs = { action: string } & Record<string, unknown>
export declare function LiveScreen(props: { label: string; screen?: (inputs: ScreenInputs) => Promise<unknown>; watch?: boolean; children?: Element }): Element

export declare function Alert(props: { heading: Element; text?: Element; open?: boolean; onClose?: () => void; action?: Element; children?: Element }): Element

export declare function Sheet(props: { heading?: Element; open?: boolean; onClose?: () => void; children?: Element }): Element

export declare const Menu: {
  Root: (props: { onSelect?: (details: { value: string }) => void; children?: Element }) => Element
  Trigger: (props: { variant?: ButtonProps["variant"]; size?: "icon"; "aria-label"?: string; children?: Element }) => Element
  Positioner: (props: { children?: Element }) => Element
  Content: (props: { children?: Element }) => Element
  Item: (props: { value: string; destructive?: boolean; disabled?: boolean; children?: Element }) => Element
  /** Items under a small heading, as iOS's menus group them. */
  Group: (props: { label: Element; children?: Element }) => Element
  Separator: (props: {}) => Element
}

export declare const Tabs: {
  Root: (props: { value?: string; defaultValue?: string; onValueChange?: (details: { value: string | null }) => void; children?: Element }) => Element
  List: (props: { children?: Element }) => Element
  Trigger: (props: { value: string; children?: Element }) => Element
  Content: (props: { value: string; children?: Element }) => Element
  Indicator: (props: {}) => Element
}

// What an app asks for in passing.

export type Subject = {
  subject?: string
}
export declare function call(tool: string, inputs?: object, options?: Subject): Promise<string>
export declare function changed(listener: () => void): () => void
export declare function toast(text: string | Error, options?: { action?: { label: string; run: () => void } }): Promise<void>
export declare function confirm(question: { heading: string; text?: string; action?: string; destructive?: boolean }): Promise<boolean>
export declare function open(app: string, options?: Subject): Promise<unknown>
export declare function leave(): Promise<void>
export declare function link(): string | undefined
export declare function component(app: string, name: string): Promise<Record<string, (props: Record<string, unknown>) => Element>>
export declare function showRecoveryKey(options?: Subject): Promise<boolean>
export declare function useRecoveryKey(options?: Subject): Promise<boolean>
export declare function go(change: () => void, options?: { back?: boolean }): void
export declare function share(text: string): Promise<"shared" | "copied" | undefined>
export declare function seen(at: string | number | Date): string
export declare function haptic(kind: "light" | "medium" | "heavy" | "rigid" | "soft" | "selection" | "success" | "warning" | "error"): void
export declare function connection(): "connecting" | "online" | "offline" | "reconnecting"
/** Whether this page is shown, as it changes: not while its person is in another app, as their device's Settings. */
export declare function visible(): boolean

/** What `client()` answers: what this device is, and what it can do that an app asks of it. */
export type Client = {
  device: string
  network?: string
  build: string
  server?: string
  push?: { state: "notAsked" | "denied" | "registering" | "registered" | "homeScreen" } | { state: "failed"; why: string }
  computerUse?: { on: boolean }
  theme: "light" | "dark" | null
  pinsShortcuts?: boolean
  settings?: boolean
  launcher?: boolean
}
/** What `client(change)` changes. */
export type ClientChange = {
  theme?: "light" | "dark" | null
  statusBar?: boolean
  leave?: boolean
  serve?: boolean
  update?: boolean
  push?: boolean
  settings?: "accessibility" | "app"
}
export declare function client(): Promise<Client>
export declare function client(change: ClientChange): Promise<{ message: string }>
