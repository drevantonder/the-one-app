// The strict primitives on a native host: each a node of its type, with its props, which Solid's universal renderer
// tells the host as mutations (renderer.ts). The UI library's components import them as `#primitives`, which is these in
// the native build and the DOM's (../primitives.tsx) in the DOM's, so each component is the same code on both.
import { merge, omit, onCleanup, type Element } from "solid-js"
import { createElement, detach, insert, insertNode, render as draw, root, spread, unsupported } from "./renderer.ts"
import { layer } from "../layer.ts"
import type { Mutations, Type } from "../renderer.ts"
import type * as Strict from "../types/ui.d.ts"
import type { PickerProps } from "../primitives.tsx"

/** A primitive: a node of its type, with its props. */
const primitive = (type: Exclude<Type, "raw-text">) => (props: object) => {
  const node = createElement(type)
  spread(node, props)
  return node as unknown as Element
}

export const Box: typeof Strict.Box = primitive("box")
export const Scroll: typeof Strict.Scroll = primitive("scroll")
export const Path: typeof Strict.Path = primitive("path")
export const Text: typeof Strict.Text = primitive("text")
export const Image: typeof Strict.Image = primitive("image")
/** An editable text: its `value` is its editing state as the app sets it, the caret at its end. */
export const EditableText: typeof Strict.EditableText = (props) =>
  primitive("input")(
    merge(omit(props, "value"), {
      get editing() {
        const text = props.value
        return text === undefined ? undefined : { text, selection: { base: text.length, extent: text.length }, composing: null }
      },
    }),
  )

/** The device's own picker (../renderer.ts's `picker`), which the UI library's Select draws under a finger. */
export const Picker = primitive("picker") as (props: PickerProps) => Element

/** A screen's live video, which the host connects, plays and sends what's done on it to (../renderer.ts's `video`),
 * where it draws one (`Mutations.video`). Where it doesn't, it's told so, which opens the page another way, and it throws,
 * as a part of the UI library a native host doesn't draw does (./index.tsx). */
export const Video: typeof Strict.Video = (props) => {
  if ((globalThis as { mutations?: Mutations }).mutations?.video) return primitive("video")(props)
  unsupported("Video")
  throw new Error("Video isn't drawn by this native host (core/ui/src/strict/renderer.ts's `video`)")
}

/** Another app's page, in a sandbox of its own (../renderer.ts's `frame`), where the host draws one (`Mutations.frame`).
 * Where it doesn't, it's told so, which opens the page another way, and it throws, as `Video` does. */
export const Frame: typeof Strict.Frame = (props) => {
  if ((globalThis as { mutations?: Mutations }).mutations?.frame) return primitive("frame")(props)
  unsupported("Frame")
  throw new Error("Frame isn't drawn by this native host (core/ui/src/strict/renderer.ts's `frame`)")
}

export const render: typeof Strict.render = (code) => draw(code as never)

/** A layer: a box over the whole window, at the root, after the page, so it's drawn over it. A modal one says so
 * (`aria-modal`): the host holds the keyboard and the screen reader in it, and what's behind it is inert, as the DOM's
 * is. */
export const Layer: typeof Strict.Layer = (props) => {
  const box = createElement("box")
  spread(box, { style: layer, get "aria-modal"() { return props.modal ? "true" : undefined } })
  insertNode(root, box)
  insert(box, () => props.children)
  onCleanup(() => detach(box))
  return undefined as unknown as Element
}
