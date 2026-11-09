// How things move inside a screen, with Motion's small animate. Each only starts from where a thing already is, so none
// holds anything back; with no motion wanted they're nothing.
import { animate } from "motion/mini"

const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches
const SETTLE = [0.22, 1, 0.36, 1] as const

/// What has just arrived fades in; from `from` where it replaces something that was showing, so there's never a blank.
export function appear(el: Element, from = 0) {
  if (!still()) animate(el, { opacity: [from, 1] }, { duration: 0.16, ease: SETTLE })
}

/// What's been pushed `by` pixels down glides from where it was to where it is now.
export function glide(el: Element, by: number) {
  if (!still()) animate(el, { translate: [`0 ${by}px`, "0 0"] }, { duration: 0.2, ease: SETTLE })
}

/// A grid opening a second column (`opening`) or closing it: its tracks slide from `from` to `to`, and its gap with them.
/// The browser's own animation, with no fill, so the grid is left as its style says.
export function slideColumns(el: Element, from: string, to: string, opening: boolean) {
  if (!still()) el.animate({ gridTemplateColumns: [from, to], columnGap: opening ? ["0px", "1.5rem"] : ["1.5rem", "0px"] }, { duration: 200, easing: `cubic-bezier(${SETTLE})` })
}
