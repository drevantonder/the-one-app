// Paging as a phone's Home Screen pages, by hand rather than by the browser's scroll snapping. On an iPhone, WebKit's
// snapping creeps for most of a second after a flick, turns a page on any slow release short of half way, and has no
// give at the first and last owner. Here, while a finger is down the pages are the finger, one to one, and past the
// ends they give with resistance; once it lifts, a spring seeded with the finger's speed takes them to a page: the
// next if it was flicked or dragged past half way, else back where they were. A finger that lands while they move
// catches them where they are.

/** One axis of the pages: where they are, in points, where they're going, and how fast. `draw` puts them there. */
export interface Spring {
  value: number
  target: number
  velocity: number
  draw: (value: number) => void
}

export const spring = (draw: (value: number) => void): Spring => ({ value: 0, target: 0, velocity: 0, draw })

// A critically damped spring that settles in about a third of a second, as a phone's pages do, with no bounce: a page
// lands, it doesn't wobble.
const STIFFNESS = 322
const DAMPING = 2 * Math.sqrt(STIFFNESS)
/// A finger moving faster than this, in points a second, as it lifts, has flicked.
const FLICK = 300

const moving = new Set<Spring>()
let last = 0
let frame = 0

function step(now: number) {
  // A frame late, as when the page was hidden, is taken as one, so nothing jumps.
  const dt = Math.min(1 / 30, (now - last) / 1000)
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

/// Takes `one` to `target`, leaving at `velocity`, or there at once for whoever asked for less motion.
export function glide(one: Spring, target: number, velocity = one.velocity) {
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return jump(one, target)
  one.target = target
  one.velocity = velocity
  moving.add(one)
  if (!frame) {
    last = performance.now()
    frame = requestAnimationFrame(step)
  }
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
/// pages, moving at `velocity` points a second. A flick goes on the way it was going; otherwise the nearest. One page
/// at most, as a phone's.
export function settle(from: number, at: number, velocity: number, pages: number) {
  const to = Math.abs(velocity) > FLICK ? (velocity > 0 ? Math.ceil(at) : Math.floor(at)) : Math.round(at)
  return Math.max(0, Math.min(pages - 1, from - 1 > to ? from - 1 : Math.min(from + 1, to)))
}

/// How fast a finger moved over the last few moves, in points a second: none if it had stopped before it lifted.
export function speed(moves: { time: number; value: number }[], now: number) {
  const recent = moves.filter((move) => now - move.time < 80)
  if (recent.length < 2) return 0
  const [first, end] = [recent[0], recent.at(-1)!]
  return ((end.value - first.value) / Math.max(1, end.time - first.time)) * 1000
}
