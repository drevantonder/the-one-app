// The strict primitives on a native host: each a node of its type, with its props, which Solid's universal renderer
// sends the host as ops (renderer.ts). The UI library's components import them as `#primitives`, which is these in
// the native build and the DOM's (../primitives.tsx) in the DOM's, so each component is the same code on both.
import { onCleanup, type Element } from "solid-js"
import { createElement, detach, insert, insertNode, render as draw, root, spread } from "./renderer.ts"
import { layer } from "../layer.ts"
import type { Type } from "../protocol.ts"
import type * as Strict from "../types/ui.d.ts"

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
export const Input: typeof Strict.Input = primitive("input")
export const Textarea: typeof Strict.Textarea = (props) => primitive("input")({ ...props, multiline: true })
export const Search: typeof Strict.Search = (props) => primitive("input")({ ...props, type: "search" })

export const render: typeof Strict.render = (code) => draw(code as never)

/** A layer: a box over the whole window, at the root, after the page, so it's drawn over it. A modal one's dialog says
 * so (`aria-modal`), and the host holds the keyboard and the screen reader in it. */
export const Layer: typeof Strict.Layer = (props) => {
  const box = createElement("box")
  spread(box, { style: layer })
  insertNode(root, box)
  insert(box, () => props.children)
  onCleanup(() => detach(box))
  return undefined as unknown as Element
}
