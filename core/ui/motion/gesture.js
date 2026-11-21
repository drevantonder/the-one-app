// How a gesture moves, and how far what it moves goes frame by frame, beside what iOS would draw.


/// How a gesture moves, as the inspector's `replay` takes it: each stretch `{dx, dy, ms}`, and whether it lifts.
export function moves(gesture) {
  const ms = ([dx, dy], speed) => (Math.hypot(dx, dy) / speed) * 1000
  if (gesture.moves) return { moves: gesture.moves, lift: gesture.lift !== false }
  // Input as a device delivered it: its events, which move nothing by a finger's moves.
  if (gesture.trace) return { moves: [], lift: false, trace: gesture.trace }
  if (gesture.swipe) return { moves: [{ dx: gesture.swipe[0], dy: gesture.swipe[1], ms: ms(gesture.swipe, gesture.speed ?? 800) }], lift: true }
  // A flick: a short way, quickly, as a thumb flicks a list.
  if (gesture.fling) return { moves: [{ dx: gesture.fling[0], dy: gesture.fling[1], ms: 80 }], lift: true }
  if (gesture.drag) {
    const parts = (gesture.pauses ?? 0) + 1
    const [dx, dy] = gesture.drag.map((by) => by / parts)
    const each = ms([dx, dy], gesture.speed ?? 400)
    const stretches = Array.from({ length: parts }, (_, at) => [...(at ? [{ ms: 200 }] : []), { dx, dy, ms: each }]).flat()
    // Held still before it's let go, so it's let go still.
    return { moves: [...stretches, { ms: 150 }], lift: true }
  }
  // A sheet pulled down, slowing, then flicked on as it's let go.
  if (gesture.pull) return { moves: [{ dy: gesture.pull * 0.75, ms: 400 }, { dy: gesture.pull * 0.25, ms: 70 }], lift: true }
  throw new Error(`A gesture is a swipe, a fling, a drag, a pull or its moves: ${JSON.stringify(gesture)}`)
}

/// What the inspector selects for a tap's name, as `scenes.js` presses it.
export const selector = (tap) => (typeof tap === "string" ? { name: tap } : tap)

/// How far what moved had gone at each frame: what `follow` names where it's said and moved, else what scrolls.
export function travel(frames) {
  const series = (pick) => frames.map((frame) => pick(frame)).map((at) => (at === undefined ? undefined : at - pick(frames[0])))
  const candidates = [
    ["along", series((frame) => frame.x)],
    ["down", series((frame) => frame.y)],
    ["scrolled along", series((frame) => frame.scroll?.x)],
    ["scrolled down", series((frame) => frame.scroll?.y)],
  ].filter(([, values]) => values.every((value) => value !== undefined))
  const range = (values) => Math.max(...values) - Math.min(...values)
  return candidates.sort(([, a], [, b]) => range(b) - range(a))[0] ?? ["still", frames.map(() => 0)]
}

/// Where the finger was `t` ms in, along its moves, the way `travel` measures: on what it moves, or against what scrolls.
export function finger(stretches, t, way) {
  let [x, y, since] = [0, 0, 0]
  for (const { dx = 0, dy = 0, ms } of stretches) {
    const part = ms <= 0 ? 1 : Math.min(Math.max((t - since) / ms, 0), 1)
    ;[x, y] = [x + dx * part, y + dy * part]
    if (t < since + ms) break
    since += ms
  }
  return { along: x, down: y, "scrolled along": -x, "scrolled down": -y }[way] ?? 0
}

/// iOS's own motion, as Apple gives it: SwiftUI's springs (`.spring(duration:bounce:)`: a mass of 1, stiffness
/// (2π/duration)², damping 4π(1 − bounce)/duration), its default `.spring` 0.5 s with no bounce, which a node let go
/// springs back by, and UIScrollView's normal deceleration, 0.998 of its speed kept each millisecond.
export const swiftui = { spring: [500, 0], smooth: [500, 0], snappy: [500, 0.15], bouncy: [500, 0.3] }
const DECELERATION = 0.998

/// Where a spring of `duration` ms and `bounce`, `from` its rest and moving `velocity` a second, is `t` ms later.
export function spring([duration, bounce], from, velocity, t) {
  const response = Math.max(duration / 1000, 0.01)
  const fraction = bounce >= 0 ? 1 - bounce : 1 / (1 + bounce)
  const [k, c, s] = [(2 * Math.PI / response) ** 2, (4 * Math.PI * fraction) / response, t / 1000]
  const natural = Math.sqrt(k)
  const ratio = c / (2 * natural)
  if (Math.abs(ratio - 1) < 1e-4) return (from + (velocity + natural * from) * s) * Math.exp(-natural * s)
  if (ratio < 1) {
    const damped = natural * Math.sqrt(1 - ratio * ratio)
    return Math.exp(-ratio * natural * s) * (from * Math.cos(damped * s) + ((velocity + ratio * natural * from) / damped) * Math.sin(damped * s))
  }
  const root = natural * Math.sqrt(ratio * ratio - 1)
  const [r1, r2] = [-ratio * natural + root, -ratio * natural - root]
  const a = (velocity - r2 * from) / (r1 - r2)
  return a * Math.exp(r1 * s) + (from - a) * Math.exp(r2 * s)
}

/// What iOS would draw after the press lifts, frame by frame from the first after it, where it says: a node let go
/// springs back by SwiftUI's `.spring` from where it was drawn at the speed the finger had; what scrolls, flung at
/// the finger's speed, slows as UIScrollView does. A pause before the lift leaves no speed. → each frame's value,
/// or none before the lift, or none where iOS gives nothing to compare (a stretch past an end springing back, or a
/// tap, which moves nothing).
export function ios(frames, way, values, stretches) {
  const lift = stretches.reduce((sum, { ms }) => sum + ms, 0)
  const last = stretches.at(-1)
  if (!last) return undefined
  const speed = last.dx || last.dy ? ((way.endsWith("along") ? last.dx ?? 0 : last.dy ?? 0) / last.ms) * 1000 : 0
  const at = frames.findIndex((frame) => frame.at >= lift)
  if (at < 0) return undefined
  const [start, rest] = [values[at], values.at(-1)]
  if (way.startsWith("scrolled")) {
    // A scroll still stretched past its end, or that comes to one, springs back as iOS's does, which Apple doesn't give.
    if (frames.slice(at).some((frame) => (frame.scroll?.y ?? 0) < 0 || (frame.scroll?.x ?? 0) < 0) || values.slice(at).some((value) => Math.abs(value - start) > Math.abs(rest - start) + 0.5)) return undefined
    const velocity = -speed
    return frames.map((frame, n) => (n < at ? undefined : start + (velocity * (DECELERATION ** (frame.at - frames[at].at) - 1)) / Math.log(DECELERATION) / 1000))
  }
  return frames.map((frame, n) => (n < at ? undefined : rest + spring(swiftui.spring, start - rest, speed, frame.at - frames[at].at)))
}


/// A tap as the harness's request, as scenes.js presses it.
export const pressed = (tap) => (tap.key ? { op: "key", ...tap } : tap.type ? { op: "type", text: tap.type } : { op: "tap", ...selector(tap) })
