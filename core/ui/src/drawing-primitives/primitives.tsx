// The drawing primitives: each a node of its type, with its props as the host names them (renderer.ts's `Props`),
// which Solid's universal renderer tells the host as mutations (renderer.native.ts). The UI library's components
// import them as `#primitives`.
import { createContext, createEffect, createMemo, createRoot, createSignal, getOwner, merge, omit, onCleanup, onSettled, Repeat, runWithOwner, Show, untrack, useContext, type Element } from "solid-js"
import { tellDrawn } from "../bridge.ts"
import { createElement, detach, insert, insertNode, render as draw, root, spread, type Node } from "./renderer.native.ts"
import { layer } from "./layer.ts"
import type { Mutations, Type } from "./renderer.ts"
import type * as Page from "../types/ui.d.ts"
import { focusable, literal, options, renamed, type PickerProps } from "./props.ts"
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

/// `<Pressable role="button" aria-label="Close" onPress={close}>...</Pressable>`: a view that's pressed, by a pointer,
/// a finger, or Enter or Space where the keyboard is, as React Native's Pressable, its one way to press (none of the
/// `Touchable…` family). With a role of what's pressed (`button`, `menuitem`, `option`, …) the keyboard reaches it.
/// `onPress` is told where it was (`clientX`, `currentTarget.getBoundingClientRect()`), and a `disabled` one isn't
/// pressed. How it looks hovered or pressed is its style's (`backgroundColor: { default: …, pressed: … }`), not a
/// function of its state, and a touch held is `onContextMenu`, not `onLongPress`. A press is the innermost one's under
/// it, as React Native's responder, UIKit's hit-testing and Flutter's gesture arena give it: one in another (an app's
/// icon in its category's square) is pressed alone, as a native host presses it. Where Tab reaches it, it draws the
/// focus ring as the keyboard comes to it (`tokens.shadow.focus`), unless its style says how it looks `focused`.
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
/// `<View style={look.row}>...</View>`: an app's own layout, around anything else: a flex column unless its `style`
/// says otherwise, as React Native's View, styled from the design tokens only. `role` and `aria-…` say what it is to
/// a screen reader and to an agent, as ARIA has them, which React Native takes too in place of its older
/// `accessibility…` props; `id` and `tabIndex` are the web's, and there's no `testID` or `nativeID`. `pointerEvents`
/// is in its style, not a prop. What's pressed is a `Pressable`. A page moved by hand listens to its pointers too
/// (`onPointerDown`…, as the web and React Native both name them).
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
/// `<ScrollView style={look.list}>...</ScrollView>`: a view whose content scrolls along its `flexDirection`, down in
/// a column and sideways in a row, with no scrollbar beside it, as on a phone. It has one style, where React Native's
/// takes `horizontal` and a `contentContainerStyle` too. `onScroll` is told how far it's scrolled, as the web's
/// (`currentTarget.scrollTop`).
export const ScrollView: typeof Page.ScrollView = (props) => scroll(renamed(props, ["tabIndex", "autoFocus", "onDrop"], { tabindex: (it) => it.tabIndex, autofocus: (it) => it.autoFocus, onDrop: dropped }))

const text = primitive("text")
/// `<Text style={look.title}>Recently Deleted</Text>`: words, in one of the type styles (`font: tokens.type.title`)
/// and a token's `color`, or as what it's in has them. It runs on in a text it's in, and is a block in a view, as React
/// Native's Text. `role="heading"` with `aria-level` makes it a heading; one inside another with `role="link"` and
/// `onPress` is a link in its words. An app's words are always in a `Text` or in a UI library component that takes them.
export const Text: typeof Page.Text = (props) => text(renamed(props, ["onPress", "tabIndex", "style"], { style: (it) => focusable(it, true), onClick: (it) => it.onPress, tabindex: (it) => it.tabIndex }))

/// `<Image src="icon.svg" alt="" style={look.icon} />`: a picture, from a `data:` address or the app's
/// own files, named by `alt`, scaled to fit its box.
export const Image: typeof Page.Image = primitive("image")

/** The `viewBox` of the `Svg` a path is in. */
const ViewBox = createContext<() => string>(() => "0 0 24 24")
/** An `Svg`'s box is what its paths are placed in, each filling it, unless its style places it itself. */
const { drawing, whole } = styles({ drawing: { position: "relative" }, whole: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0 } })

/// `<Svg viewBox="0 0 100 20" style={look.chart}><Path d="M0 20 L50 4 L100 12" /></Svg>`: a vector drawing, as
/// react-native-svg's `Svg`, its `viewBox` stretched to its box (`preserveAspectRatio="none"`, the one way it's
/// drawn): an icon, a chart's line or area. Each `Path` in it is filled with the text's colour (`color` in the Svg's
/// style), as `fill="currentColor"` is. `role="img"` with `aria-label` names one that says something; one with no role
/// is only how it looks.
export const Svg: typeof Page.Svg = (props) => {
  const node = createElement("box")
  spread(node, merge(omit(props, "viewBox", "children", "style"), { get style() { return [drawing, props.style] }, get "aria-hidden"() { return props["aria-hidden"] ?? (props.role ? undefined : "true") } }))
  insert(node, () => <ViewBox value={() => props.viewBox}>{props.children}</ViewBox>)
  return node as unknown as Element
}

const path = primitive("path")
/// `<Path d="M0 20 L50 4 L100 12" />`, in an `Svg`: a shape, as react-native-svg's `Path`, its stroke as wide however
/// it's stretched.
export const Path: typeof Page.Path = (props) => {
  const viewBox = useContext(ViewBox)
  return path(merge(omit(props, "style"), { "aria-hidden": "true", get viewBox() { return viewBox() }, get style() { return [whole, props.style] } }))
}

/// `<TextInput value={text()} onChangeText={setText} onSubmitEditing={save} />`: text to type, bare, as React
/// Native's TextInput, which `Input` draws in a field's box, in one line or with `multiline` in
/// several, where selection, an input method's composing, paste and autofill are the device's own. `enterKeyHint`
/// names its action key (a phone's Done, Go, Search, Send), and `onSubmitEditing` is told it was pressed: Return in one
/// line, as an input method that's composing doesn't. In several lines Return is a new line unless `enterKeyHint`
/// says otherwise. A `readOnly` one, or one with `selectTextOnFocus`, is all selected as it's focused, to copy or to type over. Its text runs the way its first strong character does
/// (`dir="auto"`), so Hebrew or Arabic starts at the right.
export const TextInput: typeof Page.TextInput = (props) =>
  primitive("input")(
    renamed(props, ["value", "inputMode", "secureTextEntry", "enterKeyHint", "onSubmitEditing", "onChangeText", "readOnly", "selectTextOnFocus", "autoFocus", "autoCapitalize", "autoCorrect", "spellCheck", "autoComplete"], {
      editing: (it) => (it.value === undefined ? undefined : { text: it.value, selection: { base: it.value.length, extent: it.value.length }, composing: null }),
      type: (it) => (it.multiline ? undefined : it.secureTextEntry ? "password" : it.inputMode && ["email", "url", "tel", "search"].includes(it.inputMode) ? it.inputMode : undefined),
      inputmode: (it) => it.inputMode,
      action: (it) => (it.enterKeyHint === "enter" ? "newline" : it.enterKeyHint),
      onAction: (it) => it.onSubmitEditing && (() => it.onSubmitEditing!()),
      onInput: (it) => it.onChangeText && ((event: { currentTarget: { value: string } }) => it.onChangeText!(event.currentTarget.value)),
      readonly: (it) => it.readOnly,
      selecttextonfocus: (it) => it.selectTextOnFocus,
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

/// `<Frame src="chrome-profiles/client/profile.html#work" title="Work" style={look.fill} />`: another app's page, in
/// a sandbox of its own at its own app's address, which the client answers as that app's, as tall as its page says its content
/// is where its style says no height. Only a trusted app's page shows one.
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

/// `<VirtualizedList data={notes()} estimatedItemSize={38} renderItem={({ item }) => <Row heading={item.text} />} />`:
/// a long list, of which only the rows near the window are drawn, as React Native's VirtualizedList (FlatList's) and
/// FlashList draw them: those a window either side of what shows, each as it comes into that and gone as it leaves it,
/// the room of the rest held as tall as the drawn ones are on average (`estimatedItemSize` until one is), so a list of
/// 10,000 is as quick as one of 20. A native host owns where it is (renderer.ts's `list`); here the page does, from
/// where it's drawn as anything scrolls. A row is its item's, by its index: what changes in `data` changes the row.
export const VirtualizedList: typeof Page.VirtualizedList = (props) => {
  const rest = () => omit(props, "data", "renderItem", "estimatedItemSize", "keep")
  /** What `renderItem` is told of row `index`, as it changes. A row whose index the data no longer reaches, as it shrinks,
   * still shows the item it had until the list drops it: what it draws runs once more as the data changes, before it's
   * dropped, and must not find it gone. */
  const info = (index: () => number) => {
    let kept: unknown
    return {
      get item() {
        return (kept = props.data[index()] ?? kept) as never
      },
      get index() {
        return index()
      },
    }
  }
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
    // A row kept (`keep`) is drawn wherever the host places it, as a page reusing a row for another does.
    const kept = props.keep !== undefined && props.keep < count ? props.keep : undefined
    const inRange = (index: number) => index >= first && index < last
    const needed = (index: number) => inRange(index) || index === kept
    const missing = [...Array.from({ length: last - first }, (_, at) => first + at), ...(kept === undefined || inRange(kept) ? [] : [kept])].filter((index) => !held.has(index)).sort((a, b) => away(a) - away(b))
    const free = all.filter((one) => !needed(one.at()))
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
  createEffect(() => props.keep, (keep) => void (keep !== undefined && soon()))
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

/// Draws the page, and tells its client once what it asked for as it opened has settled.
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

/// `<Modal visible={open()}>…</Modal>`: what's drawn over the whole window while it's `visible` (unless it says, it
/// is), over the page and everything over it before, with everything else inert: nothing else is pressed, focused or
/// read by a screen reader. It's a box the window's size, see-through, that lets presses through where nothing of its
/// own is (what's in it says `pointerEvents: "auto"`), so it draws its own scrim and sheet: React Native's
/// `transparent`, always, and no `animationType`, as what's in it moves itself. Escape and Back are what's in it to
/// listen for (`onKeyDown`), not `onRequestClose`.
export const Modal: typeof Page.Modal = (props) => <Show when={props.visible ?? true}>{over(true, () => props.children)}</Show>
/// `<Portal>…</Portal>`: what's drawn over the whole window, over the page and everything over it before, without
/// taking the page from anyone, as a toast or a select's list is: a box the window's size, see-through, that lets
/// presses through where nothing of its own is (what's in it says `pointerEvents: "auto"`). React Native has no
/// such component of its own; this is Solid's `Portal` and @gorhom/portal's.
export const Portal: typeof Page.Portal = (props) => over(false, () => props.children)
