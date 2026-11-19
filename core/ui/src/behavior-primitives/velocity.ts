// How fast a finger or a pointer was going as it let go, from its moves. Copied from Vercel's UI SDK (Apache-2.0),
// packages/core/src/foundation/motion/velocity.ts at ab55044caee66cc8228fa7ed3525088a1b7b5af7, unchanged:
//
// Release velocity from input samples: least squares over recent event timestamps, degree 2 (Android's
// `VelocityTracker` and Flutter default to it), degree 1 with fewer samples.
// Times are in ms and velocities in px/ms.
//
// Degree 2 needs four samples here, not three: three samples fit exactly, and
// a hard deceleration between them (a coarse mouse drag, a dropped frame)
// can flip the end slope's sign and turn a fling up into a dismissal.
//
// The horizon is 70ms, not 100ms: a slow drag that ends in a
// 50ms flick to 2px/ms reads 1.43px/ms over 100ms at 120Hz, because a quadratic
// can't follow the kink, and 1.9px/ms over 70ms.

const defaults = {
  /** How far back samples count, in ms before the newest. */
  horizon: 70,
  /** How old the newest sample can be, in ms, before the pointer counts as stopped and reads 0. */
  staleAfter: 40,
  /** The fastest the tracker reads, in px/ms either way. */
  maxSpeed: 8,
  /** How many samples it keeps: enough for the horizon at 240 Hz. */
  capacity: 20,
}

// Fits the samples and returns the slope at the newest one. Times are taken
// relative to the newest sample, so the fitted derivative at 0 is the linear
// coefficient. `times` must be ascending.
function leastSquaresVelocity(times: ArrayLike<number>, positions: ArrayLike<number>, start = 0, end = times.length) {
  const count = end - start
  if (count < 2) return 0
  const newest = times[end - 1]
  if (count < 4) return linearSlope(times, positions, start, end)
  // Normal equations for x = a + b·t + c·t², with sums of powers of t.
  let [s0, s1, s2, s3, s4, x0, x1, x2] = [0, 0, 0, 0, 0, 0, 0, 0]
  for (let index = start; index < end; index++) {
    const t = times[index] - newest
    const x = positions[index]
    const t2 = t * t
    s0 += 1
    s1 += t
    s2 += t2
    s3 += t2 * t
    s4 += t2 * t2
    x0 += x
    x1 += x * t
    x2 += x * t2
  }
  // Cramer's rule for the b coefficient of the 3×3 system.
  const determinant = s0 * (s2 * s4 - s3 * s3) - s1 * (s1 * s4 - s3 * s2) + s2 * (s1 * s3 - s2 * s2)
  if (Math.abs(determinant) < 1e-9) return linearSlope(times, positions, start, end)
  const numerator = s0 * (x1 * s4 - s3 * x2) - x0 * (s1 * s4 - s3 * s2) + s2 * (s1 * x2 - x1 * s2)
  return numerator / determinant
}

function linearSlope(times: ArrayLike<number>, positions: ArrayLike<number>, start: number, end: number) {
  let meanTime = 0
  let meanPosition = 0
  const count = end - start
  for (let index = start; index < end; index++) {
    meanTime += times[index] / count
    meanPosition += positions[index] / count
  }
  let covariance = 0
  let variance = 0
  for (let index = start; index < end; index++) {
    const dt = times[index] - meanTime
    covariance += dt * (positions[index] - meanPosition)
    variance += dt * dt
  }
  return variance > 0 ? covariance / variance : 0
}

/// `const tracker = createVelocityTracker()`, then `tracker.add(event.timeStamp, event.clientX)` as a finger moves and
/// `tracker.velocity(event.timeStamp)` as it lifts: how fast it was going, in pixels a millisecond, for what a page
/// moves by hand (the Home Screen's pages) to carry on or settle as a phone's do. A finger that stopped before it
/// lifted reads 0; `reset()` starts the next gesture afresh. One tracker an axis.
export function createVelocityTracker(options: Partial<typeof defaults> = {}) {
  const { horizon, staleAfter, maxSpeed, capacity } = { ...defaults, ...options }
  const times = new Float64Array(capacity)
  const positions = new Float64Array(capacity)
  const windowTimes = new Float64Array(capacity)
  const windowPositions = new Float64Array(capacity)
  let count = 0
  let newestIndex = -1

  /** Adds a sample: a pointer's position along the axis at `time`, like a move event's `timeStamp`. */
  function add(time: number, position: number) {
    if (count > 0 && time <= times[newestIndex]) return
    newestIndex = (newestIndex + 1) % capacity
    times[newestIndex] = time
    positions[newestIndex] = position
    count = Math.min(count + 1, capacity)
  }

  /** The speed at `now` from the recent samples, in px/ms, or 0 when the newest is older than `staleAfter`. */
  function velocity(now: number) {
    if (count < 2) return 0
    const newest = times[newestIndex]
    if (now - newest > staleAfter) return 0
    let windowCount = 0
    for (let age = count - 1; age >= 0; age--) {
      const index = (newestIndex - age + capacity) % capacity
      if (newest - times[index] > horizon) continue
      windowTimes[windowCount] = times[index]
      windowPositions[windowCount] = positions[index]
      windowCount++
    }
    return Math.max(-maxSpeed, Math.min(maxSpeed, leastSquaresVelocity(windowTimes, windowPositions, 0, windowCount)))
  }

  /** Forgets every sample, so the next gesture starts fresh. */
  function reset() {
    count = 0
    newestIndex = -1
  }

  return { add, velocity, reset }
}

/** How far something let go at `velocity` (px/ms) goes on by itself, slowing as a phone's scrolling does: UIKit's
 * normal deceleration rate, which the UI SDK's sheet coasts at (foundation/popups/sheet-pointer.ts). */
export const projected = (velocity: number, rate = 0.998) => (velocity * rate) / (1 - rate)
