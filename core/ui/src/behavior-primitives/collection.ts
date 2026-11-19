// Which item of a list the arrow keys move to, and which one holds a list's one tab stop. Copied from Vercel's UI SDK
// (Apache-2.0), packages/core/src/foundation/focus/collection.ts at ab55044caee66cc8228fa7ed3525088a1b7b5af7: only
// what our components use, unchanged. Pure, so it runs on every host.

/** Which arrows move along a list: Up and Down, Left and Right, or all four, as radios. */
export type Orientation = "horizontal" | "vertical" | "both"

// How many items a key moves focus by, or "first" and "last". Undefined for
// keys the list doesn't handle, including arrows along the other axis. With
// "both", every arrow works, like radios: Down and the inline end go forward.
export function stepFor(key: string, orientation: Orientation, rtl: boolean) {
  if (orientation === "both") return radioStep(key, rtl) ?? (key === "Home" ? "first" : key === "End" ? "last" : undefined)
  const forward = orientation === "vertical" ? "ArrowDown" : rtl ? "ArrowLeft" : "ArrowRight"
  const backward = orientation === "vertical" ? "ArrowUp" : rtl ? "ArrowRight" : "ArrowLeft"
  if (key === forward) return 1
  if (key === backward) return -1
  if (key === "Home") return "first" as const
  if (key === "End") return "last" as const
}

// The item a radio key moves to: Down and Right go forward, Up and Left back,
// with Left and Right swapped right to left. Like native radios, both axes work.
function radioStep(key: string, rtl: boolean) {
  if (key === "ArrowDown" || key === (rtl ? "ArrowLeft" : "ArrowRight")) return 1
  if (key === "ArrowUp" || key === (rtl ? "ArrowRight" : "ArrowLeft")) return -1
}

/** An item of a roving group, as the tab stop decision sees it. */
export type RovingCandidate = {
  key: string
  /** Whether it can't be activated. */
  disabled: boolean
  /** Whether it can take focus at all. */
  focusable: boolean
  /** Whether it's selected, checked or pressed, so Tab lands on it first. */
  preferred: boolean
}

// The item that holds a group's one tab stop: the item focused last, when the
// group remembers one, else the first preferred item that's enabled, else the
// first enabled one, else the first that can take focus.
export function rovingStop(items: readonly RovingCandidate[], remembered: string | null = null) {
  const focusable = items.filter((item) => item.focusable)
  if (remembered !== null && focusable.some((item) => item.key === remembered)) return remembered
  const enabled = focusable.filter((item) => !item.disabled)
  return (enabled.find((item) => item.preferred) ?? enabled[0] ?? focusable[0])?.key ?? null
}

// The index a step lands on, wrapping or stopping at the ends.
export function moveIndex(index: number, step: number | "first" | "last", count: number, loop: boolean) {
  if (step === "first") return 0
  if (step === "last") return count - 1
  const next = index + step
  if (loop) return ((next % count) + count) % count
  return Math.max(0, Math.min(count - 1, next))
}
