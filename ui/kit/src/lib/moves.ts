// How things move: every duration and easing in the app, as Motion transitions, `animate(el, keyframes, moves.settle)`.
// CSS has the same as variables (moves-css.ts): `duration-(--move-settle) ease-(--ease-settle)`. Inside a screen each
// move only starts from where a thing already is, so none holds anything back; with no motion wanted they're nothing.
import { animate } from "motion/mini"

const SETTLE = [0.22, 1, 0.36, 1] as const
const TRAVEL = [0.32, 0.72, 0, 1] as const

export const moves = {
  /// What goes, what's small: a press, a fade, a menu.
  quick: { duration: 0.12, ease: SETTLE },
  /// What arrives or shifts in place.
  settle: { duration: 0.2, ease: SETTLE },
  /// What crosses the screen: a sheet, an app from its icon.
  travel: { duration: 0.3, ease: TRAVEL },
}

export const still = () => matchMedia("(prefers-reduced-motion: reduce)").matches

/// Motion writes what it ends on into the element's style, where it would beat the classes it's left with.
const leavingNoStyle = (el: Element, name: string) => () => (el as HTMLElement).style.removeProperty(name)

/// What has just arrived fades in; from `from` where it replaces something that was showing, so there's never a blank.
export function appear(el: Element, from = 0) {
  if (!still()) animate(el, { opacity: [from, 1] }, moves.quick).finished.then(leavingNoStyle(el, "opacity"))
}

/// What's been pushed `by` pixels down glides from where it was to where it is now.
export function glide(el: Element, by: number) {
  if (!still()) animate(el, { translate: [`0 ${by}px`, "0 0"] }, moves.settle).finished.then(leavingNoStyle(el, "translate"))
}

/// A grid opening a second column (`opening`) or closing it: its tracks slide from `was` to what its style says, and its
/// gap with them, and one still sliding goes on from where it's got to. → the tracks it ends on. The browser's own
/// animation, with no fill, so the grid is left as its style says.
export function slideColumns(el: HTMLElement, was: string, opening: boolean) {
  const style = getComputedStyle(el)
  const sliding = el.getAnimations()
  const from = sliding.length ? style.gridTemplateColumns : opening ? `${was} 0px` : was
  const gap = sliding.length ? style.columnGap : opening ? "0px" : "1.5rem"
  sliding.forEach((a) => a.cancel())
  const tracks = style.gridTemplateColumns
  if (!still()) el.animate({ gridTemplateColumns: [from, opening ? tracks : `${tracks} 0px`], columnGap: [gap, opening ? "1.5rem" : "0px"] }, { duration: moves.settle.duration * 1000, easing: `cubic-bezier(${SETTLE})` })
  return tracks
}
