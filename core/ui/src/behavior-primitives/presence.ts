// Something that's drawn while it's open and while it goes, so it can move away before it's gone: a sheet, an alert,
// a toast. The rule of Vercel's UI SDK (Apache-2.0), packages/core/src/foundation/motion/presence.ts at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7 (`animateOut`): a part that closes stays until its exit has run, and one
// that opens again before then stays, coming back. Our host runs the exit as a style's transition, so how long it
// takes is the motion token's (`duration`).
import { createEffect, createSignal, onCleanup } from "solid-js"

/** `const presence = createPresence(() => props.open, duration(tokens.motion.travel))`: whether it's drawn
 * (`shown`), and whether it's on its way out (`leaving`), which its style moves it away by. */
export function createPresence(open: () => boolean, takes: number) {
  const [shown, setShown] = createSignal(false)
  const [leaving, setLeaving] = createSignal(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  createEffect(open, (now) => {
    clearTimeout(timer)
    if (now) return void (setLeaving(false), setShown(true))
    if (!shown()) return
    setLeaving(true)
    timer = setTimeout(() => (setShown(false), setLeaving(false)), takes)
  })
  onCleanup(() => clearTimeout(timer))
  return { shown, leaving }
}
