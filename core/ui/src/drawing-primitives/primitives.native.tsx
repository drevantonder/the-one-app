// The drawing primitives on a native host: each a node of its type, with its props as the host names them
// (renderer.ts's `Props`), which Solid's universal renderer tells the host as mutations (renderer.native.ts). The UI
// library's components import them as `#primitives`, which is these in the native build and the DOM's (primitives.tsx)
// in the DOM's, so each component is the same code on both.
import { createContext, createSignal, merge, omit, onCleanup, Repeat, Show, useContext, type Element } from "solid-js"
import { createElement, detach, insert, insertNode, render as draw, root, spread, unsupported } from "./renderer.native.ts"
import { layer } from "./layer.ts"
import type { Mutations, Type } from "./renderer.ts"
import type * as Page from "../types/ui.d.ts"
import type { PickerProps } from "./primitives.tsx"
import { options, renamed } from "./props.ts"
import { styles } from "../styles.ts"

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
    renamed(props, ["tabIndex", "autoFocus", "onPress", "disabled", "onDrop"], {
      tabindex: (it) => it.tabIndex,
      autofocus: (it) => it.autoFocus,
      onClick: (it) => (it.disabled ? undefined : it.onPress),
      onDrop: dropped,
      "aria-disabled": (it) => (it.disabled ? "true" : it["aria-disabled"]),
    }),
  )
export const View: typeof Page.View = (props) => Pressable(props)

const scroll = primitive("scroll")
export const ScrollView: typeof Page.ScrollView = (props) => scroll(renamed(props, ["tabIndex", "autoFocus", "onDrop"], { tabindex: (it) => it.tabIndex, autofocus: (it) => it.autoFocus, onDrop: dropped }))

const text = primitive("text")
export const Text: typeof Page.Text = (props) => text(renamed(props, ["onPress", "tabIndex"], { onClick: (it) => it.onPress, tabindex: (it) => it.tabIndex }))

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
      autocapitalize: (it) => it.autoCapitalize,
      autocorrect: (it) => it.autoCorrect,
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

/** A screen's live video, which the host connects, plays and sends what's done on it to (renderer.ts's `video`),
 * where it draws one (`Mutations.video`). Where it doesn't, it's told so, which opens the page another way, and it throws,
 * as a part of the UI library a native host doesn't draw does (../index.native.tsx). */
export const Video: typeof Page.Video = (props) => {
  if (!(globalThis as { mutations?: Mutations }).mutations?.video) {
    unsupported("Video")
    throw new Error("Video isn't drawn by this native host (core/ui/src/drawing-primitives/renderer.ts's `video`)")
  }
  return primitive("video")(renamed(props, ["aria-label", "autoFocus"], { label: (it) => it["aria-label"], autofocus: (it) => it.autoFocus }))
}

/** Another app's page, in a sandbox of its own (renderer.ts's `frame`), where the host draws one (`Mutations.frame`).
 * Where it doesn't, it's told so, which opens the page another way, and it throws, as `Video` does. */
export const Frame: typeof Page.Frame = (props) => {
  if ((globalThis as { mutations?: Mutations }).mutations?.frame) return primitive("frame")(props)
  unsupported("Frame")
  throw new Error("Frame isn't drawn by this native host (core/ui/src/drawing-primitives/renderer.ts's `frame`)")
}

/** How many rows a `list` has before its host first asks for some: a window's. */
const FIRST_ROWS = 24

/** A list of rows of which only those near the window are drawn, as the host asks for them (renderer.ts's `list`): the
 * host lays them out and owns where it is, and the page draws only the rows it's asked for, each as its index comes
 * into the range and gone as it leaves it. A host that doesn't draw one draws every row in a box. */
export const VirtualizedList: typeof Page.VirtualizedList = (props) => {
  const rest = () => omit(props, "data", "renderItem", "estimatedItemSize")
  const row = (index: number) => props.renderItem({ get item() { return props.data[index] }, index })
  if (!(globalThis as { mutations?: Mutations }).mutations?.list) {
    const node = createElement("box")
    spread(node, rest())
    insert(node, () => <Repeat count={props.data.length}>{row}</Repeat>)
    return node as unknown as Element
  }
  const [range, setRange] = createSignal({ first: 0, last: FIRST_ROWS })
  const last = () => Math.min(range().last, props.data.length)
  const first = () => Math.min(range().first, last())
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
      onRange: (event: { first: number; last: number }) => setRange({ first: event.first, last: event.last }),
    }),
  )
  insert(node, () => (
    <Repeat count={last() - first()} from={first()}>
      {(index) => {
        const box = createElement("box")
        spread(box, { index })
        insert(box, () => row(index))
        return box as unknown as Element
      }}
    </Repeat>
  ))
  return node as unknown as Element
}

export const render: typeof Page.render = (code) => draw(code as never)

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
