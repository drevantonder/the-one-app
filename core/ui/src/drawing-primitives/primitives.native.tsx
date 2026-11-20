// The drawing primitives on a native host: each a node of its type, with its props as the host names them
// (renderer.ts's `Props`), which Solid's universal renderer tells the host as mutations (renderer.native.ts). The UI
// library's components import them as `#primitives`, which is these in the native build and the DOM's (primitives.tsx)
// in the DOM's, so each component is the same code on both.
import { createContext, createMemo, createRoot, createSignal, getOwner, merge, omit, onCleanup, onSettled, Repeat, runWithOwner, Show, untrack, useContext, type Element } from "solid-js"
import { tellDrawn } from "../bridge.native.ts"
import { createElement, detach, insert, insertNode, render as draw, root, spread, type Node } from "./renderer.native.ts"
import { layer } from "./layer.ts"
import type { Mutations, Type } from "./renderer.ts"
import type * as Page from "../types/ui.d.ts"
import type { PickerProps } from "./primitives.tsx"
import { focusable, literal, options, renamed } from "./props.ts"
import { styles } from "../styles.ts"
import { Problem } from "../styled-components/states.tsx"
import { messages } from "../words.ts"

/** A primitive: a node of its type, with its props. */
const primitive = (type: Exclude<Type, "raw-text">) => (props: object) => {
  const node = createElement(type)
  spread(node, props)
  return node as unknown as Element
}

const view = primitive("box")
/** A view's `onDrop`, told the files dropped on it, which come as base64, as an upload goes. */
const dropped = (it: Page.ViewProps) => it.onDrop && ((event: { files: { name: string; body: string }[] }) => it.onDrop!(event.files.map((file) => new File([Uint8Array.from(atob(file.body), (character) => character.charCodeAt(0))], file.name))))

export const Pressable: typeof Page.Pressable = (props) =>
  view(
    renamed(props, ["tabIndex", "autoFocus", "onPress", "disabled", "onDrop", "style"], {
      style: focusable,
      tabindex: (it) => it.tabIndex,
      autofocus: (it) => it.autoFocus,
      onClick: (it) => (it.disabled ? undefined : it.onPress),
      onDrop: dropped,
      "aria-disabled": (it) => (it.disabled ? "true" : it["aria-disabled"]),
    }),
  )
export const View: typeof Page.View = (props) => Pressable(props)

/** The words in what a page drew (a node, or the string or number it was given): what a screen reader says of it. */
export const textOf = (drawn: unknown): string => (typeof drawn === "string" || typeof drawn === "number" ? String(drawn) : drawn && typeof drawn === "object" ? ((drawn as Node).text ?? (drawn as Node).children.map(textOf).join("")) : "")

/** Which of two nodes comes first in the page's tree, as a sort compares them: as the DOM's `compareDocumentPosition`
 * says. Anything not in it is as early as anything. */
export const documentOrder = (a: unknown, b: unknown) => {
  const path = (node: unknown) => {
    const up: Node[] = []
    for (let at = node as Node | undefined; at; at = at.parent) up.unshift(at)
    return up
  }
  const [one, other] = [path(a), path(b)]
  const split = one.findIndex((node, at) => node !== other[at])
  if (!one.length || !other.length || one[0] !== other[0] || split < 0) return 0
  const parent = one[split - 1]
  return parent.children.indexOf(one[split]) - parent.children.indexOf(other[split])
}

const scroll = primitive("scroll")
export const ScrollView: typeof Page.ScrollView = (props) => scroll(renamed(props, ["tabIndex", "autoFocus", "onDrop"], { tabindex: (it) => it.tabIndex, autofocus: (it) => it.autoFocus, onDrop: dropped }))

const text = primitive("text")
export const Text: typeof Page.Text = (props) => text(renamed(props, ["onPress", "tabIndex", "style"], { style: (it) => focusable(it, true), onClick: (it) => it.onPress, tabindex: (it) => it.tabIndex }))

export const Image: typeof Page.Image = primitive("image")

/** The `viewBox` of the `Svg` a path is in. */
const ViewBox = createContext<() => string>(() => "0 0 24 24")
/** An `Svg`'s box is what its paths are placed in, each filling it, unless its style places it itself. */
const { drawing, whole } = styles({ drawing: { position: "relative" }, whole: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0 } })

/** A vector drawing: a box, its paths over it, each in its `viewBox`, stretched to it. */
export const Svg: typeof Page.Svg = (props) => {
  const node = createElement("box")
  spread(node, merge(omit(props, "viewBox", "children", "style"), { get style() { return [drawing, props.style] }, get "aria-hidden"() { return props["aria-hidden"] ?? (props.role ? undefined : "true") } }))
  insert(node, () => <ViewBox value={() => props.viewBox}>{props.children}</ViewBox>)
  return node as unknown as Element
}

const path = primitive("path")
export const Path: typeof Page.Path = (props) => {
  const viewBox = useContext(ViewBox)
  return path(merge(omit(props, "style"), { "aria-hidden": "true", get viewBox() { return viewBox() }, get style() { return [whole, props.style] } }))
}

/** A text input: its `value` is its editing state as the app sets it, the caret at its end, and the rest its props
 * as the host names them. */
export const TextInput: typeof Page.TextInput = (props) =>
  primitive("input")(
    renamed(props, ["value", "inputMode", "secureTextEntry", "enterKeyHint", "onSubmitEditing", "onChangeText", "readOnly", "autoFocus", "autoCapitalize", "autoCorrect", "spellCheck", "autoComplete"], {
      editing: (it) => (it.value === undefined ? undefined : { text: it.value, selection: { base: it.value.length, extent: it.value.length }, composing: null }),
      type: (it) => (it.multiline ? undefined : it.secureTextEntry ? "password" : it.inputMode && ["email", "url", "tel", "search"].includes(it.inputMode) ? it.inputMode : undefined),
      inputmode: (it) => it.inputMode,
      action: (it) => (it.enterKeyHint === "enter" ? "newline" : it.enterKeyHint),
      onAction: (it) => it.onSubmitEditing && (() => it.onSubmitEditing!()),
      onInput: (it) => it.onChangeText && ((event: { currentTarget: { value: string } }) => it.onChangeText!(event.currentTarget.value)),
      readonly: (it) => it.readOnly,
      autofocus: (it) => it.autoFocus,
      autocapitalize: (it) => it.autoCapitalize ?? (literal(it) ? "none" : undefined),
      autocorrect: (it) => it.autoCorrect ?? (literal(it) ? false : undefined),
      spellcheck: (it) => it.spellCheck,
      autocomplete: (it) => it.autoComplete,
    }),
  )

/** The device's own picker (renderer.ts's `picker`), which the UI library's Select draws under a finger, as
 * @react-native-picker/picker's: its options from its items. */
const picker = primitive("picker")
export const Picker = Object.assign(
  (props: PickerProps) => {
    const all = options(() => props.children)
    return picker(
      renamed(props, ["selectedValue", "onValueChange", "enabled", "children"], {
        value: (it) => it.selectedValue,
        options: () => all(),
        disabled: (it) => (it.enabled === false ? true : undefined),
        onInput: (it) => it.onValueChange && ((event: { currentTarget: { value: string } }) => it.onValueChange!(event.currentTarget.value, all().findIndex((option) => option.value === event.currentTarget.value))),
      }),
    )
  },
  { Item: (props: { label: string; value: string; enabled?: boolean }) => props as unknown as Element },
)

/** A part of the UI library this host doesn't draw yet (`what`, renderer.ts's `Mutations`), in its place: a problem
 * saying so, which Telemetry's told too (`reportError`), so only that part of its page fails. */
const undrawn = (what: string) => {
  reportError(new Error(`${what} isn't drawn by this native host yet (core/ui/src/drawing-primitives/renderer.ts's \`${what.toLowerCase()}\`)`))
  return <Problem heading={messages.cantShowHere(what)} text={messages.notDrawnHere} />
}

/** A screen's live video, which the host connects, plays and sends what's done on it to (renderer.ts's `video`),
 * where it draws one (`Mutations.video`); where it doesn't, it's `undrawn`. */
export const Video: typeof Page.Video = (props) => {
  if (!(globalThis as { mutations?: Mutations }).mutations?.video) return undrawn("Video")
  return primitive("video")(renamed(props, ["aria-label", "autoFocus"], { label: (it) => it["aria-label"], autofocus: (it) => it.autoFocus }))
}

/** Another app's page, in a sandbox of its own (renderer.ts's `frame`), where the host draws one (`Mutations.frame`);
 * where it doesn't, it's `undrawn`. */
export const Frame: typeof Page.Frame = (props) => {
  if ((globalThis as { mutations?: Mutations }).mutations?.frame) return primitive("frame")(props)
  return undrawn("Frame")
}

/** How many rows a `list` has before its host first asks for some: a window's. */
const FIRST_ROWS = 24

/** How many rows a `list` fills a frame at most as it's scrolled, those that show first, as RecyclerView's prefetch
 * spreads its work over frames: a fling's frames keep their time, and a row is filled frames before it shows. A row
 * reused costs what it says changing; one made, its whole making, so one a frame at most is made. */
const PER_FRAME = 4

/** A row of a `list`, kept as it goes out of the range and reused for one coming into it (`VirtualizedList`): its
 * index, and what it's drawn as. */
type Cell = { at: () => number; place: (index: number) => void; box: Element; dispose: () => void }

/** A list of rows of which only those near the window are drawn, as the host asks for them (renderer.ts's `list`): the
 * host lays them out and owns where it is, and the page fills only the rows it's asked for, those that show first, a
 * few a frame, each from a row that's gone out of the range where there's one, as RecyclerView and FlashList recycle
 * theirs: its item changes, so a row isn't made again, only what it says. A host that doesn't draw one draws every
 * row in a box. */
export const VirtualizedList: typeof Page.VirtualizedList = (props) => {
  const rest = () => omit(props, "data", "renderItem", "estimatedItemSize")
  /** What `renderItem` is told of row `index`, as it changes. */
  const info = (index: () => number) => ({
    get item() {
      return props.data[index()]
    },
    get index() {
      return index()
    },
  })
  if (Number((globalThis as { mutations?: Mutations }).mutations?.list ?? 0) < 2) {
    const node = createElement("box")
    spread(node, rest())
    insert(node, () => <Repeat count={props.data.length}>{(index) => props.renderItem(info(() => index))}</Repeat>)
    return node as unknown as Element
  }
  const owner = getOwner()
  const [cells, setCells] = createSignal<Cell[]>([])
  /** A row, made under the list, so it has what the list's in (its style, a listbox), and drawn only while its index
   * is one of the list's. */
  const cell = (index: number): Cell =>
    runWithOwner(owner, () =>
      createRoot((dispose) => {
        const [at, place] = createSignal(index)
        const box = createElement("box")
        spread(box, {
          get index() {
            return at()
          },
        })
        const there = createMemo(() => at() < props.data.length)
        insert(box, () => (there() ? untrack(() => props.renderItem(info(at))) : null))
        return { at, place: (index: number) => void place(index), box: box as unknown as Element, dispose }
      }),
    )
  let wanted = { first: 0, last: FIRST_ROWS, shownFirst: 0, shownLast: FIRST_ROWS }
  let waiting = false
  /// The rows it's asked for that it hasn't, filled the next frame, as its display draws it.
  const soon = () => {
    if (waiting) return
    waiting = true
    requestAnimationFrame(() => ((waiting = false), fill(PER_FRAME, 1)))
  }
  /// Fills the rows it's asked for that it hasn't (`most` of them, those that show first, then those nearest them),
  /// each from a row out of the range where there's one, making `making` at most; the rest the next frame.
  const fill = (most: number, making: number) => {
    const count = props.data.length
    const [first, last] = [Math.min(wanted.first, count), Math.min(wanted.last, count)]
    const all = cells()
    const held = new Set(all.map((one) => one.at()))
    const away = (index: number) => Math.max(0, wanted.shownFirst - index, index - wanted.shownLast + 1)
    const missing = Array.from({ length: last - first }, (_, at) => first + at).filter((index) => !held.has(index)).sort((a, b) => away(a) - away(b))
    const free = all.filter((one) => one.at() < first || one.at() >= last)
    const made: Cell[] = []
    let filled = 0
    for (const index of missing) {
      const reused = free.pop()
      if (reused) reused.place(index)
      else if (made.length < making) made.push(cell(index))
      else break
      if (++filled === most) break
    }
    // What the range no longer needs, as it shrinks, goes, but a frame's worth kept for the next.
    const extra = free.slice(PER_FRAME)
    if (made.length || extra.length) {
      extra.forEach((one) => one.dispose())
      setCells([...all.filter((one) => !extra.includes(one)), ...made])
    }
    if (missing.length > filled) soon()
  }
  fill(FIRST_ROWS, FIRST_ROWS)
  const node = createElement("list")
  spread(
    node,
    merge(rest(), {
      get count() {
        return props.data.length
      },
      get estimate() {
        return props.estimatedItemSize
      },
      onRange: (event: { first: number; last: number; shownFirst?: number; shownLast?: number }) => {
        wanted = { first: event.first, last: event.last, shownFirst: event.shownFirst ?? event.first, shownLast: event.shownLast ?? event.last }
        soon()
      },
    }),
  )
  insert(node, () => cells().map((one) => one.box))
  return node as unknown as Element
}

/// Draws the page, and tells its client once what it asked for as it opened has settled, as the DOM's `render` does.
export const render: typeof Page.render = (code) => draw((() => (onSettled(() => void tellDrawn()), code())) as never)

/** A box over the whole window, at the root, after the page, so it's drawn over it. A modal one says so
 * (`aria-modal`): the host holds the keyboard and the screen reader in it, and what's behind it is inert, as the DOM's
 * is. */
function over(modal: boolean, content: () => Element) {
  const node = createElement("box")
  spread(node, { style: layer, "aria-modal": modal ? "true" : undefined })
  insertNode(root, node)
  insert(node, content)
  onCleanup(() => detach(node))
  return undefined as unknown as Element
}

export const Modal: typeof Page.Modal = (props) => <Show when={props.visible ?? true}>{over(true, () => props.children)}</Show>
export const Portal: typeof Page.Portal = (props) => over(false, () => props.children)
