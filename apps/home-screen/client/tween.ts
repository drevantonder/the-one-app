import { createSignal, untrack } from "solid-js"
import { reducedMotion } from "ui"

/// A value that moves to where it's told, `0` to `1`, over `duration` milliseconds, eased as iOS's are, a frame at a
/// time, and from wherever it got to when it's told to turn back: `done` is called once it's there. For whoever asked
/// for less motion, it's there at once. It starts at `at` (`0` unless it's drawn already where it's going: what mounts
/// as a category already open mustn't start at the square it grew out of).
export function tween(duration: number, done: (at: number) => void, at = 0) {
  const [value, setValue] = createSignal(at)
  let frame = 0
  let from = at
  let to = at
  let begun = -1
  const step = (time: number) => {
    if (begun < 0) begun = time
    const t = Math.min(1, (time - begun) / (duration * Math.abs(to - from) || 1))
    setValue(from + (to - from) * (1 - (1 - t) ** 3))
    if (t < 1) frame = requestAnimationFrame(step)
    else ((frame = 0), done(to))
  }
  return {
    value,
    to(target: number) {
      cancelAnimationFrame(frame)
      from = untrack(value)
      to = target
      begun = -1
      if (untrack(reducedMotion) || from === to) return void (setValue(to), done(to))
      frame = requestAnimationFrame(step)
    },
    stop: () => cancelAnimationFrame(frame),
  }
}
