// The rail's line, over an app, which a finger pulls in to go home.
import { launcher } from "./apps.js"
import { here, moving, on, showing, shown, windows } from "./client.js"
import { HELD } from "./move-machine.js"
import { byId } from "./util.js"

/// The rail: every app's way back to the Home Screen, above every app on a phone or a tablet: a home line on the
/// window's right edge, iOS's home indicator on its side, which a finger pulls in to go home (`pull`). It keeps out of
/// the way while something of the app's is open over it (a sheet, a menu), as the device's back does. A computer has
/// none: each app is a window of its own there, which the computer's own Dock or taskbar goes between.
const line = byId("line")

/// Whether the rail's line shows: while the window shows its apps and has a Home Screen to go back to, over an app,
/// at any of its screens, unless something is drawn over it, a sheet or a menu (`data-away`). Android is told where
/// the line is, which its back gesture leaves to it.
/** @type {string | undefined} */
let excluded
/// The fingers on the line, by their pointers, where each came down, and the one pulling it (`pull`, below).
/** @type {Map<number, { x: number, y: number }>} */
const touching = new Map()
/** @type {{ id: number, x: number, recent: [number, number][] } | undefined} */
let pull
export function rail() {
  const now = shown.getSnapshot()
  // A computer's windows have none: its own Dock or taskbar goes between them.
  const homed = Boolean(launcher) && !windows && now.hasTag("pages") && !now.matches("app_store")
  const lined = homed && here() !== null && !showing(here())?.layer
  line.toggleAttribute("data-away", !lined)
  // A line that went home under a finger comes back with its app.
  if (lined && !pull) line.toggleAttribute("data-pulled", false)
  const at = lined ? line.getBoundingClientRect() : undefined
  const edge = at ? [at.left, at.top, at.right, at.bottom].map(Math.round).join(",") : ""
  if (edge !== excluded) globalThis.androidEdge?.postMessage(edge)
  excluded = edge
}
addEventListener("resize", rail)

/// How far in, as a share of the window's width, past which a finger let go of the line goes home.
const PULLED = 0.18
/// A finger on the line, pulling it in: once it has come in far enough to say so, the window's move to the Home
/// Screen follows it (`grab` with `home`, and `drag`), the app as far shrunk as `p`, and let go, it goes home if it
/// came in past `PULLED` or was flicked in, and otherwise springs back. A tap does nothing. The first finger to come
/// in takes it, and only its lifting lets go: another on the line meanwhile, as the hand holding the phone touches
/// its edge, is ignored, as iOS's swipe home ignores a second finger.
line.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return
  line.setPointerCapture(event.pointerId)
  touching.set(event.pointerId, { x: event.clientX, y: event.clientY })
})
line.addEventListener("pointermove", (event) => {
  const down = touching.get(event.pointerId)
  if (!pull && down) {
    // Too little yet to say which way it goes; up or down isn't a pull.
    if (Math.hypot(event.clientX - down.x, event.clientY - down.y) < 8) return
    touching.delete(event.pointerId)
    if (!(down.x > event.clientX && Math.abs(event.clientX - down.x) > Math.abs(event.clientY - down.y))) return
    on({ type: "grab", home: true })
    if (moving()?.matches("tracking")) pull = { id: event.pointerId, x: down.x, recent: [] }
  }
  if (event.pointerId !== pull?.id) return
  const inward = (pull.x - event.clientX) / innerWidth
  // The app follows the finger, and past the card (`HELD`) slows to a stop, as a rubber band does.
  const p = HELD * Math.tanh(Math.max(0, inward) / HELD)
  pull.recent = [...pull.recent.filter(([then]) => event.timeStamp - then <= 100), [event.timeStamp, inward]]
  line.toggleAttribute("data-pulled", true)
  on({ type: "drag", p })
})
/** @param {PointerEvent} event */
function letGo(event) {
  touching.delete(event.pointerId)
  if (event.pointerId !== pull?.id) return
  const moves = pull.recent
  // How fast it was coming in over its last tenth of a second, in widths a second.
  const [[since, was] = [0, 0]] = moves
  const [at, inward] = moves.at(-1) ?? [0, 0]
  const velocity = at - since >= 20 ? ((inward - was) / (at - since)) * 1000 : 0
  const home = event.type !== "pointercancel" && (inward >= PULLED || velocity > 0.6)
  if (!home) line.toggleAttribute("data-pulled", false)
  on({ type: "release", velocity: Math.max(0, velocity) * (home ? 1 : 0), end: home ? 1 : 0 })
  // Going home, it stays hidden as the window's history follows.
  pull = undefined
}
line.addEventListener("pointerup", letGo)
line.addEventListener("pointercancel", letGo)
