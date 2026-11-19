// A layer over the page that a person can dismiss: a menu, a select's list, a sheet, an alert. Escape, a press outside
// it and the device's back each close it, and the keyboard goes back to what opened it. From Vercel's UI SDK
// (Apache-2.0) at ab55044caee66cc8228fa7ed3525088a1b7b5af7: the reasons a layer is dismissed
// (packages/core/src/foundation/kernel/details.ts's `DismissReason`) and how each is heard
// (foundation/popups/popup.ts, foundation/popups/dismissal.ts), on our host: Escape is the layer's `onKeyDown`, a
// press outside is what's around it (a see-through `Pressable` under it in its `Modal` or `Portal`), the back is
// `covers`, and the keyboard goes back by the opener's `autoFocus`.
import { createEffect, createSignal } from "solid-js"
import { covers } from "#bridge"
import type { Keyed } from "../types/ui.d.ts"

/** How a layer was dismissed, by the UI SDK's names. */
export type DismissReason = "escape-key" | "outside-press" | "back"

/** `const layer = createDismissable({ open, onDismiss: (reason) => setOpen(false) })`, then the layer's
 * `onKeyDown={layer.keydown}`, what's around it `{...layer.outside}`, and its opener `autoFocus={layer.returning()}`:
 * a layer that's dismissed as people expect, while `open`. */
export function createDismissable(options: { open: () => boolean; onDismiss: (reason: DismissReason) => void }) {
  // Closed after it was open, the keyboard goes back to what opened it, until it opens again.
  const [returning, setReturning] = createSignal(false)
  createEffect(options.open, (open, was) => void setReturning(!open && !!was))
  // The device's back closes it before it goes anywhere else.
  createEffect(options.open, (open) => (open ? covers(() => options.onDismiss("back")) : undefined))
  // A press outside is one that went down outside it: the press that opened it (a row held) ends where it's now
  // drawn over, and isn't one.
  let down = false
  createEffect(options.open, () => void (down = false))
  return {
    returning,
    /** Escape on it. → whether it took the key. */
    keydown: (event: Keyed) => {
      if (event.key !== "Escape") return false
      event.preventDefault()
      options.onDismiss("escape-key")
      return true
    },
    outside: {
      onPointerDown: () => void (down = true),
      onPress: () => down && ((down = false), options.onDismiss("outside-press")),
    },
  }
}
