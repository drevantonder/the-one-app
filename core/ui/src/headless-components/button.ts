// A button, with its state and no look. From Vercel's UI SDK (Apache-2.0), packages/core/src/components/button/button.ts
// at ab55044caee66cc8228fa7ed3525088a1b7b5af7: a disabled one stays where the keyboard reaches it (`aria-disabled`,
// its `focusableWhenDisabled`) so it can be found and read, and does nothing pressed; a loading one is busy, can't be
// pressed again, and once it has been for a second a screen reader hears so (`pendingAnnouncement`). Pressed by a
// pointer, a finger, Enter or Space, as our host's `Pressable` is.
import { createEffect } from "solid-js"
import { announce } from "../behavior-primitives/announce.tsx"
import { messages } from "../words.ts"
import type { Located } from "../types/ui.d.ts"

/** A button's props: whether it can't be pressed, or is busy with what it did, and what pressing it does. */
export interface ButtonState {
  disabled?: boolean
  loading?: boolean
  onPress?: (event: Located) => void
}

/** `const button = createButton(props)`, then `<Pressable {...button.root()}>`. */
export function createButton(props: ButtonState) {
  const off = () => !!props.disabled || !!props.loading
  createEffect(
    () => !!props.loading,
    (loading) => {
      if (!loading) return
      const timer = setTimeout(() => announce(messages.loading), 1000)
      return () => clearTimeout(timer)
    },
  )
  return {
    off,
    root: () => ({
      role: "button" as const,
      get "aria-disabled"() {
        return off() ? ("true" as const) : undefined
      },
      get "aria-busy"() {
        return props.loading ? ("true" as const) : undefined
      },
      onPress: (event: Located) => !off() && props.onPress?.(event),
    }),
  }
}
