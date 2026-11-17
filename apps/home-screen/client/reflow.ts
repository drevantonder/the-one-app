// While a Home page is edited, its icons and widgets glide to where they now go, as an iPhone's do when an app is
// dragged among them or taken off, rather than jumping there: each is drawn where it was, then springs to its new place
// (pager.ts's spring, which keeps its speed if it moves again on the way). For whoever asked for less motion, one that
// moved fades in where it is instead, as iOS cross-fades.
import { pace } from "ui"
import { glide, spring, type Spring } from "./pager.ts"

const still = matchMedia("(prefers-reduced-motion: reduce)")

/** Where a cell was laid out last, by its place in its page, which a glide doesn't change, and how far from it it's drawn. */
interface Placed {
  left: number
  top: number
  x: Spring
  y: Spring
}

const placed = new WeakMap<HTMLElement, Placed>()

/// Notes where each of `cells` is laid out now, and with `glides`, takes each that moved there from where it was.
export function reflow(cells: Iterable<HTMLElement>, glides: boolean) {
  for (const cell of cells) {
    const [left, top] = [cell.offsetLeft, cell.offsetTop]
    const was = placed.get(cell)
    if (!was) {
      const draw = () => (cell.style.translate = one.x.value || one.y.value ? `${one.x.value}px ${one.y.value}px` : "")
      const one: Placed = { left, top, x: spring(draw), y: spring(draw) }
      placed.set(cell, one)
      continue
    }
    const [x, y] = [was.left - left, was.top - top]
    Object.assign(was, { left, top })
    if (!glides || (!x && !y)) continue
    if (still.matches) {
      cell.animate([{ opacity: 0 }, { opacity: 1 }], pace(cell, "settle"))
      continue
    }
    was.x.value += x
    was.y.value += y
    glide(was.x, 0)
    glide(was.y, 0)
  }
}

/// How far from its place `cell` is drawn, as it glides there, in the window's points where its page is drawn `scale`
/// times larger.
export function offset(cell: HTMLElement, scale = 1) {
  const one = placed.get(cell)
  return { x: (one?.x.value ?? 0) * scale, y: (one?.y.value ?? 0) * scale }
}
