// Direct manipulation: a finger or a pointer moving something with it along one way, and how fast it was going as it
// let go, as a sheet is pulled down, a row swiped aside or a page of the Home Screen swiped to the next; and a press
// held. From Vercel's UI SDK (Apache-2.0) at ab55044caee66cc8228fa7ed3525088a1b7b5af7:
// packages/core/src/foundation/direct-manipulation/swipe-actions.ts's start of a swipe (it moves past a slop first,
// and one that moves across first is a scroll, not this), foundation/drag-and-drop/pointer.ts's swallowing the press
// a drag ends on, foundation/direct-manipulation/hold.ts's press held (its delay and how far it may move), and the
// release velocity of foundation/motion/velocity.ts. On our host the pointer's events are a box's `onPointerDown`…
// (renderer.ts's `Moved`), whose moves and end come to it wherever the pointer goes once it's down on it.
import { onCleanup } from "solid-js"
import { createVelocityTracker } from "./velocity.ts"
import type { Located, Pressed } from "../types/ui.d.ts"

/** How far a press moves before it's a drag, in pixels. */
export const SLOP = 8
/** How fast a release is a flick, in pixels a millisecond. */
export const FLICK = 0.3
/** How long after a drag or a hold its pointer's press is still the gesture's, not a press of what it ended on. */
const SWALLOW = 600

/** What a drag tells as it ends: how far it went along its way, how fast it was going, and whether it was cancelled. */
export type Dragged = { delta: number; velocity: number; canceled: boolean; event: Pressed }

/** The pointer's events a gesture listens to, spread on the box it's on. */
export type Gesture = {
  onPointerDown: (event: Pressed) => void
  onPointerMove: (event: Pressed) => void
  onPointerUp: (event: Pressed) => void
  onPointerCancel: (event: Pressed) => void
}

/** `const pull = createDrag({ axis: "y", onMove: (delta) => …, onEnd: ({ delta, velocity }) => … })`, then
 * `{...pull.gesture}` on the box: a finger or a pointer moving it along `axis`. It starts once the pointer has moved
 * `slop` that way (`accepts` decides which presses may start one); one that moves the other way first is left to
 * what scrolls. `swallows(event)` says whether a press is the end of one, which isn't a press of what it ended on. */
export function createDrag(options: {
  axis: "x" | "y"
  slop?: number
  accepts?: (event: Pressed) => boolean
  onStart?: (event: Pressed) => void
  onMove: (delta: number, event: Pressed) => void
  onEnd: (dragged: Dragged) => void
}) {
  const along = (event: Pressed) => (options.axis === "x" ? event.clientX : event.clientY)
  const across = (event: Pressed) => (options.axis === "x" ? event.clientY : event.clientX)
  const velocity = createVelocityTracker()
  let start: { along: number; across: number; pointerId: number } | undefined
  let dragging = false
  let ended = -Infinity
  const end = (event: Pressed, canceled: boolean) => {
    if (!start || event.pointerId !== start.pointerId) return
    const was = dragging
    const delta = along(event) - start.along
    start = undefined
    dragging = false
    if (!was) return
    ended = event.timeStamp
    options.onEnd({ delta, velocity: canceled ? 0 : velocity.velocity(event.timeStamp), canceled, event })
  }
  const gesture: Gesture = {
    onPointerDown: (event) => {
      if (start || event.button !== 0 || (options.accepts && !options.accepts(event))) return
      start = { along: along(event), across: across(event), pointerId: event.pointerId }
      velocity.reset()
      velocity.add(event.timeStamp, along(event))
    },
    onPointerMove: (event) => {
      if (!start || event.pointerId !== start.pointerId) return
      const delta = along(event) - start.along
      if (!dragging) {
        const sideways = across(event) - start.across
        if (Math.hypot(delta, sideways) < (options.slop ?? SLOP)) return
        // A press that moves the other way first is a scroll, not this.
        if (Math.abs(sideways) >= Math.abs(delta)) return void (start = undefined)
        dragging = true
        options.onStart?.(event)
      }
      velocity.add(event.timeStamp, along(event))
      options.onMove(delta, event)
    },
    onPointerUp: (event) => end(event, false),
    onPointerCancel: (event) => end(event, true),
  }
  return { gesture, dragging: () => dragging, swallows: (event: Located) => event.timeStamp - ended < SWALLOW }
}

/** `const hold = createHold({ onHold: (event) => menu.open(at(event)) })`, then `{...hold.gesture}`: a press held
 * `delay` milliseconds without moving `tolerance` pixels, as a long press opens a phone's context menu. A held press
 * isn't a press of what it was on (`swallows`). */
export function createHold(options: { delay?: number; tolerance?: number; accepts?: (event: Pressed) => boolean; onHold: (event: Pressed) => void }) {
  const { delay = 500, tolerance = 10 } = options
  let press: { x: number; y: number; pointerId: number; timer: ReturnType<typeof setTimeout> } | undefined
  let held = false
  let ended = -Infinity
  const stop = () => (clearTimeout(press?.timer), (press = undefined))
  onCleanup(stop)
  const gesture: Gesture = {
    onPointerDown: (event) => {
      if (press || event.button !== 0 || (options.accepts && !options.accepts(event))) return
      held = false
      press = { x: event.clientX, y: event.clientY, pointerId: event.pointerId, timer: setTimeout(() => ((held = true), (press = undefined), options.onHold(event)), delay) }
    },
    onPointerMove: (event) => {
      if (press?.pointerId === event.pointerId && Math.hypot(event.clientX - press.x, event.clientY - press.y) > tolerance) stop()
    },
    onPointerUp: (event) => {
      if (held) ended = event.timeStamp
      held = false
      stop()
    },
    onPointerCancel: () => ((held = false), stop()),
  }
  return { gesture, swallows: (event: Located) => event.timeStamp - ended < SWALLOW }
}

/** Gestures on one box, each told every pointer event, in order. */
export const together = (...gestures: Gesture[]): Gesture => ({
  onPointerDown: (event) => gestures.forEach((one) => one.onPointerDown(event)),
  onPointerMove: (event) => gestures.forEach((one) => one.onPointerMove(event)),
  onPointerUp: (event) => gestures.forEach((one) => one.onPointerUp(event)),
  onPointerCancel: (event) => gestures.forEach((one) => one.onPointerCancel(event)),
})
