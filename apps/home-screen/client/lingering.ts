import { createEffect, createSignal, untrack } from "solid-js"

/// What's shown while `when` gives it, kept as it was for `ms` after, as it goes: what to show, and whether it's going.
export function lingering<T>(when: () => T | false | undefined | null, ms: number): [() => T | undefined, () => boolean] {
  const [shown, setShown] = createSignal<T>()
  const [going, setGoing] = createSignal(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  createEffect(when, (now) => {
    clearTimeout(timer)
    if (now) return void (setGoing(false), setShown(() => now))
    if (untrack(shown) === undefined) return
    setGoing(true)
    timer = setTimeout(() => (setShown(undefined), setGoing(false)), ms)
  })
  return [shown, going]
}
