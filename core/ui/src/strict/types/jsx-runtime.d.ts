// JSX in a strict app: components only. There are no elements of the DOM's (`<div>`, `<span>`), so writing one is a
// type error: a strict app draws with the UI library's primitives and components (ui.d.ts).
import type { Element as SolidElement } from "solid-js"

export namespace JSX {
  type Element = SolidElement
  interface ArrayElement extends Array<Element> {}
  interface ElementClass {}
  interface ElementAttributesProperty {}
  interface ElementChildrenAttribute {
    children: {}
  }
  interface IntrinsicElements {}
  interface IntrinsicAttributes {}
}
