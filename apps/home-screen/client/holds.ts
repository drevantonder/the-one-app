// How a finger or a mouse holds, lifts and drops what it's on: the gestures of every app and widget.
import { type Located, type Pressed, type PressableProps, type Rect } from "ui"
import { type Held } from "./machine.ts"
import { home, quietFor, finger } from "./store.ts"
import { pending } from "./owners.ts"

// A finger that stays this long holds what it's on, as a phone's does, and on a Home page, held this much longer
// still, it's lifted into editing, as an iPhone's menu gives way to the drag.
const HOLD = 450

const LONGER = 800

/** What the Home Screen's gestures listen for on what they're given. */
export type Holds = Pick<PressableProps, "onPointerDown" | "onPointerMove" | "onPointerUp" | "onPointerCancel" | "onContextMenu">

/// Watches a finger or a mouse on what it's given, for the Home Screen's gestures: a press held still is a hold, as is
/// a right-click; a press that moves while editing, or once held, lifts it, and so does one held still a while longer
/// on a Home page. A mouse holds as a finger does, as an iPad's pointer does. `held` says what it is, as it is then. A
/// hold on a page's empty space has nothing to lift or offer, so it edits the page at once, as a phone's does. Only
/// whoever `arranges` lifts anything; a menu is held open for whoever it has one for (`menu`), as an App Library's has
/// for everyone. What's lifted follows the finger that lifted it (`carry`), wherever it goes.
export function holds(held: () => Held | undefined, arranges: () => boolean, menu = arranges): Holds {
  let timer: ReturnType<typeof setTimeout> | undefined
  let start: Pressed | undefined
  let rect: Rect | undefined
  // Whether this press has held: a mouse's click as it lets go is the hold's end, not a tap.
  let fired = false
  const clear = () => {
    clearTimeout(timer)
    start = undefined
    fired = false
  }
  // `at` is where the pointer is now: a press held still a while longer lifts with no move to say so.
  const lift = (at: Pressed, from: Pressed) => {
    clear()
    const what = held()
    if (!what?.item || !arranges()) return
    finger.grab = { x: from.clientX, y: from.clientY }
    finger.pointer = { x: at.clientX, y: at.clientY }
    finger.carrier = at.pointerId
    home.send({ type: "lift", held: { ...what, rect }, editing: home.getSnapshot().matches("editing") ? undefined : pending() })
  }
  const hold = (event: Located) => {
    const what = held()
    if (!menu() || !what || !(home.getSnapshot().matches("nothing") || home.getSnapshot().matches("category"))) return
    fired = !!start
    if (!what.item && !what.listed) return arranges() && home.send({ type: "edit", editing: pending() })
    home.send({ type: "hold", held: { ...what, rect: event.currentTarget.getBoundingClientRect() } })
    const from = start
    if (from && what.from === "page") timer = setTimeout(() => home.getSnapshot().matches("holding") && lift(from, from), LONGER)
  }
  return {
    onPointerDown: (event) => {
      if (event.button || !menu()) return
      clear()
      start = event
      rect = event.currentTarget.getBoundingClientRect()
      // Editing, a press that moves lifts at once, wherever the pointer goes first; otherwise a press held still is a
      // hold.
      if (!home.getSnapshot().matches("editing")) timer = setTimeout(() => start && hold(start), HOLD)
    },
    onPointerMove: (event) => {
      if (finger.carry && event.pointerId === finger.carrier) return finger.carry.move(event)
      if (!start || event.pointerId !== start.pointerId) return
      if (Math.hypot(event.clientX - start.clientX, event.clientY - start.clientY) < 8) return
      // Held, or editing, a press that moves carries what it's on; before that, it's a swipe.
      if (home.getSnapshot().matches("holding") || (home.getSnapshot().matches({ editing: "still" }) && held()?.item)) lift(event, start)
      else clear()
    },
    onPointerUp: (event) => {
      if (finger.carry && event.pointerId === finger.carrier) return finger.carry.drop(event)
      if (event.pointerId !== start?.pointerId) return
      if (fired) quietFor(300)
      clear()
    },
    onPointerCancel: (event) => {
      if (finger.carry && event.pointerId === finger.carrier) return finger.carry.drop(event)
      if (event.pointerId === start?.pointerId) clear()
    },
    onContextMenu: (event) => {
      clear()
      hold(event)
    },
  }
}
