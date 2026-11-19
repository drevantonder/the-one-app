// Paging as a phone's Home Screen pages, by hand rather than by the browser's scroll snapping. On an iPhone, WebKit's
// snapping creeps for most of a second after a flick, turns a page on any slow release short of half way, and has no
// give at the first and last owner. Here, while a finger is down the pages are the finger, one to one, and past the
// ends they give with resistance; once it lifts, a spring seeded with the finger's speed takes them to a page: the one
// its motion would have carried them nearest to, as an iPhone's Home Screen decides, so a short quick flick and a slow
// drag past half way both turn it. A finger that lands while they move
// catches them where they are. For whoever asked for less motion, they cross-fade to where they're going instead.
import { createVelocityTracker, projected, reducedMotion } from "ui"

/** One axis of the pages: where they are, in points, where they're going, and how fast. `draw` puts them there, and
 * `fade`, where it's given, how opaque they are as they cross-fade. */
export interface Spring {
  value: number
  target: number
  velocity: number
  draw: (value: number) => void
  fade?: (opacity: number) => void
}

export const spring = (draw: (value: number) => void, fade?: (opacity: number) => void): Spring => ({ value: 0, target: 0, velocity: 0, draw, fade })

// A critically damped spring that settles in about a third of a second, as a phone's pages do, with no bounce: a page
// lands, it doesn't wobble.
const STIFFNESS = 322
const DAMPING = 2 * Math.sqrt(STIFFNESS)

const moving = new Set<Spring>()
let last = 0
let frame = 0

function step(now: number) {
  // A frame late, as when the page was hidden, is taken as one, so nothing jumps; the first is a frame's.
  const dt = last ? Math.min(1 / 30, (now - last) / 1000) : 1 / 60
  last = now
  for (const one of moving) {
    for (let at = 0; at < dt; at += 1 / 240) {
      const slice = Math.min(1 / 240, dt - at)
      one.velocity += (-STIFFNESS * (one.value - one.target) - DAMPING * one.velocity) * slice
      one.value += one.velocity * slice
    }
    if (Math.abs(one.value - one.target) < 0.25 && Math.abs(one.velocity) < 5) {
      one.value = one.target
      one.velocity = 0
      moving.delete(one)
    }
    one.draw(one.value)
  }
  frame = moving.size ? requestAnimationFrame(step) : 0
}

/// Takes `one` to `target`, leaving at `velocity`; for whoever asked for less motion, a quick cross-fade there, or
/// there at once where it doesn't fade.
export function glide(one: Spring, target: number, velocity = one.velocity) {
  if (reducedMotion()) return one.fade && one.value !== target ? crossFade(one, target) : jump(one, target)
  one.target = target
  one.velocity = velocity
  moving.add(one)
  if (!frame) {
    last = 0
    frame = requestAnimationFrame(step)
  }
}

/// How long a cross-fade takes, out and in, in ms: as quick as what settles.
const FADE = 200

/// Fades `one` out where it is, puts it at `target`, and fades it in there.
function crossFade(one: Spring, target: number) {
  let begun = -1
  one.target = target
  const step = (now: number) => {
    if (begun < 0) begun = now
    const t = Math.min(1, (now - begun) / FADE)
    if (t >= 0.5 && one.value !== target) jump(one, target)
    one.fade!(Math.abs(1 - 2 * t))
    if (t < 1 && one.target === target) requestAnimationFrame(step)
    else one.fade!(1)
  }
  requestAnimationFrame(step)
}

/// Puts `one` at `value` at once.
export function jump(one: Spring, value: number) {
  moving.delete(one)
  one.value = one.target = value
  one.velocity = 0
  one.draw(value)
}

/// Stops `one` where it is, as a finger landing on it does.
export function stop(one: Spring) {
  moving.delete(one)
  one.velocity = 0
}

/// Where a finger `over` points past an end puts the pages, given `size`, the page's: less the further it goes, as
/// iOS's own scroll views give.
export const give = (over: number, size: number) => (over * size * 0.55) / (size + 0.55 * Math.abs(over))

/// The page the pages go to as a finger lifts, from `from`, the one they were on when it landed: they're at `at`, in
/// pages of `size` points, moving at `velocity` points a second. The page nearest where its motion would carry them,
/// one at most, as a phone's.
export function settle(from: number, at: number, velocity: number, pages: number, size: number) {
  const to = Math.round(at + projected(velocity / 1000) / size)
  return Math.max(0, Math.min(pages - 1, from - 1 > to ? from - 1 : Math.min(from + 1, to)))
}

/// How fast a finger was going as it lifted, in points a second, along each way: the UI library's velocity tracker,
/// one an axis, `add`ed each move. None if it had stopped before it lifted.
export function thrower() {
  const [x, y] = [createVelocityTracker(), createVelocityTracker()]
  return {
    add: (event: { timeStamp: number; clientX: number; clientY: number }) => (x.add(event.timeStamp, event.clientX), y.add(event.timeStamp, event.clientY)),
    reset: () => (x.reset(), y.reset()),
    thrown: (now: number) => ({ x: x.velocity(now) * 1000, y: y.velocity(now) * 1000 }),
  }
}
