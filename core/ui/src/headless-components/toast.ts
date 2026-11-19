// Toasts, with their state and no look: which show, for how long, and what a screen reader hears of them. From
// Vercel's UI SDK (Apache-2.0) at ab55044caee66cc8228fa7ed3525088a1b7b5af7: when each closes by itself,
// packages/core/src/components/toast/toast-timers.ts, copied (one countdown per toast, paused while the pointer rests
// on them or the page isn't seen, and the time paused kept, never reset); at most a few at once, the oldest going for a
// new one (toast-api.ts's `maxVisible`); each read out as it comes (foundation/platform/announce.ts).
import { createEffect, createRoot, createSignal } from "solid-js"
import { visible } from "#bridge"
import { announce } from "../behavior-primitives/announce.tsx"

/** Why the countdowns are paused: the pointer on them, the page not seen. */
type PauseReason = "item-hover" | "page-hide"

type Countdown = { duration: number; remaining: number; startedAt?: number; timer?: ReturnType<typeof setTimeout> }

/** One countdown per toast shown, and one set of pause reasons, each cleared on its own. */
function createToastTimers(expire: (id: number) => void, now = () => Date.now()) {
  const countdowns = new Map<number, Countdown>()
  const reasons = new Set<PauseReason>()
  function stop(countdown: Countdown) {
    if (countdown.startedAt === undefined) return
    clearTimeout(countdown.timer)
    countdown.remaining -= now() - countdown.startedAt
    countdown.startedAt = undefined
    countdown.timer = undefined
  }
  function start(id: number, countdown: Countdown) {
    if (countdown.startedAt !== undefined) return
    countdown.startedAt = now()
    countdown.timer = setTimeout(() => (countdowns.delete(id), expire(id)), Math.max(0, countdown.remaining))
  }
  const apply = () => {
    for (const [id, countdown] of countdowns) reasons.size ? stop(countdown) : start(id, countdown)
  }
  return {
    add: (id: number, duration: number) => (countdowns.set(id, { duration, remaining: duration }), apply()),
    remove: (id: number) => {
      const countdown = countdowns.get(id)
      if (countdown) stop(countdown)
      countdowns.delete(id)
    },
    /** Pauses every countdown for a reason, until that reason is cleared. */
    pause: (reason: PauseReason) => void (!reasons.has(reason) && (reasons.add(reason), apply())),
    /** Clears one reason. Countdowns resume once no reason is left. */
    resume: (reason: PauseReason) => void (reasons.delete(reason) && apply()),
  }
}

/** A toast: what it says, and its one action. */
export type Toast = { id: number; title: string; action?: { label: string; run: () => void } }

/** At most this many show at once; the oldest goes for a new one. */
const MOST = 3

/** `const toasts = createToaster()`: the toasts, which `show` adds and `dismiss` takes away, which the host draws
 * going (its style's `exiting`); spread `region()` on what holds them. */
export function createToaster() {
  const [toasts, setToasts] = createSignal<Toast[]>([])
  let made = 0
  const dismiss = (id: number) => {
    timers.remove(id)
    setToasts((now) => now.filter((one) => one.id !== id))
  }
  const timers = createToastTimers(dismiss)
  // A page that isn't seen keeps its toasts until it is.
  createRoot(() => createEffect(visible, (seen) => (seen ? timers.resume("page-hide") : timers.pause("page-hide"))))
  return {
    toasts,
    dismiss,
    show: (title: string, action?: Toast["action"]) => {
      if (toasts().length >= MOST) dismiss(toasts()[0].id)
      const id = ++made
      setToasts((now) => [...now, { id, title, action }])
      timers.add(id, action ? 8000 : 4000)
      announce(title)
    },
    region: () => ({ onPointerEnter: () => timers.pause("item-hover"), onPointerLeave: () => timers.resume("item-hover") }),
  }
}
