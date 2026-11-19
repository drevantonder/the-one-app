// A row swiped aside to show what's behind it, as a phone's lists show Delete. From Vercel's UI SDK (Apache-2.0),
// packages/core/src/foundation/direct-manipulation/swipe-actions.ts at ab55044caee66cc8228fa7ed3525088a1b7b5af7: a
// finger's press that moves sideways first swipes the row (one that moves up or down first is a scroll); let go past
// `fullSwipe` of the row's width it runs its action at once; otherwise a flick decides whichever way it went, and
// past half of what's behind it, it stays open; a press on an open row closes it. On our host's pointer events
// (drag.ts), toward the row's start, which is right to left too.
import { createSignal } from "solid-js"
import { direction } from "#bridge"
import { createDrag, FLICK } from "./drag.ts"
import type { Located, Pressed } from "../types/ui.d.ts"

/** `const swipe = createSwipeActions({ room: 88, enabled, onFullSwipe })`, then `{...swipe.gesture}` on the row and
 * `swipe.offset()` how far aside it's drawn, `settling()` whether it's on its way somewhere by itself. */
export function createSwipeActions(options: { room: number; fullSwipe?: number; enabled: () => boolean; onFullSwipe: () => void }) {
  const [offset, setOffset] = createSignal(0)
  const [settling, setSettling] = createSignal(false)
  let width = 0
  let from = 0
  let closed = -Infinity
  const aside = (px: number, settle = false) => (setSettling(settle), setOffset(Math.max(0, Math.min(px, width || px))))
  // Toward the row's start is aside: left, or right right to left.
  const toward = (dx: number) => (direction() === "rtl" ? dx : -dx)
  const drag = createDrag({
    axis: "x",
    accepts: (event: Pressed) => {
      width = event.currentTarget.getBoundingClientRect().width
      // A press on an open row closes it, and is that, not a press of the row.
      if (offset()) return (aside(0, true), (closed = event.timeStamp), false)
      from = 0
      return event.pointerType === "touch" && options.enabled()
    },
    onMove: (dx) => aside(from + toward(dx)),
    onEnd: ({ velocity, canceled }) => {
      if (canceled) return aside(0, true)
      if (offset() >= width * (options.fullSwipe ?? 0.6)) {
        aside(width, true)
        options.onFullSwipe()
        return void setTimeout(() => aside(0, true), 400)
      }
      const speed = toward(velocity)
      // A flick decides whichever way it went; otherwise past half of what's behind it stays open.
      aside((Math.abs(speed) >= FLICK ? speed > 0 : offset() >= options.room / 2) ? options.room : 0, true)
    },
  })
  return {
    gesture: drag.gesture,
    offset,
    settling,
    close: () => aside(0, true),
    /** Whether a press is the end of a swipe, or the press that closed the row. */
    swallows: (event: Located) => drag.swallows(event) || event.timeStamp - closed < 600,
  }
}
