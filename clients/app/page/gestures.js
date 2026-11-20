// The device's gestures that move the window: a trackpad's swipe, an iPhone's swipe in from the edge, Android's back gesture, and the device's own back.
import { invoke } from "./host.js"
import { launcher } from "./apps.js"
import { post } from "./bridge.js"
import { baseKey, depth, front, here, launcherOver, moving, nativeLauncher, on, open, panel, showing } from "./client.js"
import { rail } from "./rail.js"

/// Whether the device's own back and forward gestures are on: an iPhone's swipe in from the edge, a trackpad's swipe.
/// They go through this window's history, so they're off while something is open over the app that's showing, as a
/// phone's are under a sheet, and its Back or Escape shuts that first. At an app's first screen a computer's swipe is
/// the window's own (`wheel`), which takes the app home as its Back does, so the web view's, which would slide a
/// picture of what's under it, is off there too.
let swiping = true
/// Whether the device's swipe takes the app that's showing home, there being one of its own: an iPhone's.
let taking = false
/// Whether back has anything to go back from: an app over the launcher, or something open over the app that's
/// showing. Where it hasn't, Android shows going home as its back gesture is made (`MainActivity`).
/** @type {boolean | undefined} */
let backs
export function gestures() {
  const view = showing(here())
  const takes = home()
  const on = !view?.over && !takes
  if (on !== swiping || takes !== taking) invoke("navigation_gestures", { on, home: takes })
  swiping = on
  taking = takes
  const back = !!view?.over || (!!view && view.entrypoint !== launcher) || (nativeLauncher && launcherOver && (!view || view.key === baseKey()))
  if (back !== backs) globalThis.androidBack?.postMessage(String(back))
  backs = back
  rail()
}
/// The window's own sheet or alert open over every app (its recovery key, Update Required), if one is.
export const covering = () => /** @type {HTMLDialogElement | null} */ (document.querySelector("dialog[open]"))
/// Whether back from what's showing takes an app home: it's at its first screen, with nothing open over it, the
/// app's or the window's.
export const home = () => depth > 0 && front?.first === true && !front.over && !covering()

/// A trackpad's two-finger swipe sideways, as each frame tells of its wheel (`src/initialization-script.js`), the
/// way a browser's swipe between pages goes back: at an app's first screen, swiping back takes it home, the move
/// following the fingers, and as they lift it carries on if it's past halfway or flicked, and otherwise springs
/// back. Once they've lifted, the wheel moves on by itself only ever slower (momentum), which is left alone; a quicker
/// move is the fingers again, which catch the move where it is. Up and down is never taken, nor a swipe that scrolls
/// something of the app's own that way, or one while the app has its own screens to go back to, whose web view's
/// swipe it is.
/** @type {{ x: number, y: number, from: number, recent: [number, number][], velocity: number, speed: number, slowing: number, began: number, own: boolean, taken?: boolean, lifted?: boolean } | undefined} */
let swipe
/// How long the wheel is still before the fingers are taken to have lifted: it says nothing while they're still.
const LIFTED = 150
let lifting = 0
/** @type {unknown} */
let released
/** @param {{ x: number, y: number, at: number, own: boolean }} moved */
function wheel({ x, y, at, own }) {
  clearTimeout(lifting)
  lifting = setTimeout(() => {
    lift()
    swipe = undefined
  }, LIFTED)
  const speed = Math.abs(x) + Math.abs(y)
  // A move quicker than the one before, by more than a wheel's moves wander, is the fingers'.
  const quicker = !!swipe && speed > swipe.speed * 1.15
  if (swipe?.lifted && !quicker) return void (swipe.speed = speed)
  if (!swipe || swipe.lifted) swipe = { x: 0, y: 0, from: 0, recent: [], velocity: 0, speed, slowing: 0, began: speed, own: false }
  // Slowing over several moves, never quicker, from a swipe's speed, is what's left once the fingers lift.
  if (quicker) Object.assign(swipe, { slowing: 0, began: speed })
  else swipe.slowing += 1
  swipe.speed = speed
  swipe.x += x
  swipe.y += y
  swipe.own ||= own
  const end = getComputedStyle(document.documentElement).direction === "rtl" ? -1 : 1
  if (swipe.taken === undefined) {
    // Too little yet to say which way it goes.
    if (Math.abs(swipe.x) + Math.abs(swipe.y) < 10) return
    const move = moving()
    // A move the fingers let go of is caught where it is, as long as it's still going.
    const caught = !!released && move?.matches("playing") === true && move.context.move === released
    swipe.taken = false
    if (Math.abs(swipe.x) <= Math.abs(swipe.y) || swipe.own || !(caught || (swipe.x * end < 0 && home()))) return
    on({ type: "grab" })
    swipe.taken = moving()?.matches("tracking") === true
    swipe.from = move?.context.move?.p ?? 0
  }
  if (!swipe.taken) return
  if (swipe.slowing >= 6 && swipe.began >= 4 && speed <= swipe.began * 0.8) return lift()
  const p = swipe.from - (swipe.x * end) / innerWidth
  // How fast it goes, over its last tenth of a second, which a page too busy to hear each move as it came leaves as
  // it was.
  swipe.recent = [...swipe.recent.filter(([then]) => at - then <= 100), [at, p]]
  const [[since, was]] = swipe.recent
  if (at - since >= 20) swipe.velocity = ((p - was) / (at - since)) * 1000
  on({ type: "drag", p: Math.min(1, Math.max(0, p)) })
}
/// The fingers have lifted: a move they took is let go, as fast as it was going.
function lift() {
  if (!swipe) return
  if (swipe.taken) {
    on({ type: "release", velocity: swipe.velocity })
    released = moving()?.context.move
  }
  swipe.taken = false
  swipe.lifted = true
}
addEventListener("message", ({ data }) => {
  if (data?.method === "notifications/one/wheel") wheel(data.params)
  // Escape on a computer, at an app's first screen with nothing open over it, takes it home as its Back does, and on
  // the launcher puts it away.
  if (data?.method === "notifications/one/escape" && home()) history.back()
  else if (data?.method === "notifications/one/escape" && panel() && !depth && !open.get(baseKey())?.over) invoke("hide")
})
/// An iPhone's swipe in from the left edge (`edge` in src/lib.rs), which is the window's own: WebKit's swipe between
/// pages, which would go forward from the right edge too, where the rail's line is, is off. At an app's first screen,
/// or while a move is still going, the move follows the finger, `p` as far across the window as it's come, as a pop
/// does, and let go at `velocity` (across the window a second), it carries on if it's past halfway or flicked, or
/// springs back. Anywhere else it goes back a screen through the window's history, as the device's back does, once
/// it's let go a third of the way across or flicked. Taken away, as by a call, it does nothing.
/** @type {{ p: number } | undefined} */
let stepping
window.grab = () => {
  on({ type: "grab" })
  stepping = moving()?.matches("tracking") ? undefined : { p: 0 }
}
/** @param {number} p */
window.drag = (p) => {
  if (stepping) stepping.p = p
  else on({ type: "drag", p: Math.min(1, Math.max(0, p)) })
}
/** @param {number} velocity @param {0 | 1} [end] */
window.release = (velocity, end) => {
  if (!stepping) return on({ type: "release", velocity, end })
  const { p } = stepping
  stepping = undefined
  if (end ?? (velocity > 0.5 || p > 1 / 3)) history.back()
}

/// Android's back gesture, as its finger comes in from the edge (`MainActivity`): where back takes an app home, the
/// move follows it in the shape of Android's back to home, the app drawn a little way (`BACK`) toward where it goes as
/// the finger comes all the way in, and once Android says it's done (`back`), carries on home from there, or
/// cancelled, goes back.
let backing = false
/// How far Android's back gesture draws the move, at most, before it's let go: the app as small as 0.9 of itself
/// going into its icon, as Android's own back to home has it.
const BACK = 0.12
/** @type {{ p: number, at: number, velocity: number }} */
const backed = { p: 0, at: 0, velocity: 0 }
window.backStarted = () => {
  if (!home()) return
  on({ type: "grab" })
  backing = moving()?.matches("tracking") === true
  Object.assign(backed, { p: 0, at: performance.now(), velocity: 0 })
}
/** @param {number} progress */
window.backProgressed = (progress) => {
  if (!backing) return
  // Most of the way in the gesture's first part, slowing after, as Android's own does (its decelerate curve).
  const p = BACK * (1 - (1 - Math.min(1, Math.max(0, progress))) ** 3)
  const now = performance.now()
  const dt = (now - backed.at) / 1000
  if (dt > 0) backed.velocity = 0.6 * ((p - backed.p) / dt) + 0.4 * backed.velocity
  Object.assign(backed, { p, at: now })
  on({ type: "drag", p })
}
window.backCancelled = () => {
  if (backing) on({ type: "release", velocity: backed.velocity, end: 0 })
  backing = false
}
/// The device's own back, where a device has one (the app's window there calls this): Android's gesture, done, takes
/// the app it drew going home the rest of the way, and otherwise what's open over the app that's showing shuts first,
/// the window's own sheet as it would to Escape (an alert that can't be shut stays).
/// → whether either was. If neither was, the device goes back through this window's history, which leads back through
/// an app's screens and out of it, and from the launcher out of the app.
window.back = () => {
  if (backing) {
    backing = false
    on({ type: "release", velocity: backed.velocity, end: 1 })
    return true
  }
  const sheet = covering()
  if (sheet) {
    sheet.requestClose()
    return true
  }
  const view = open.get(here() ?? "") ?? open.get(baseKey())
  if (!view?.over || view.frame.inert) return false
  post(view, { method: "notifications/one/back" })
  return true
}

/// A tap on an iPhone's status bar (the app's window there calls this): the app that's showing scrolls to its top.
window.scrollToTop = () => {
  const view = showing(here())
  if (view) post(view, { method: "notifications/one/scroll-to-top" })
}
