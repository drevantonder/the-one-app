// A dialog over the page, with its state and no look: a sheet or an alert. From Vercel's UI SDK (Apache-2.0) at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7, packages/core/src/components/dialog/dialog.ts: modal while open, with
// what's behind it inert and the keyboard held in it (our host's `Modal` and `aria-modal`), drawn until its exit has
// run; Escape, a press on what's around it and the device's back close it, each asking its `onClose`. A sheet is
// pulled down to close too (foundation/popups/sheet-pointer.ts): let go a third of the way down, or flicked so it
// would carry on past that, it closes; otherwise it goes back.
import { createSignal } from "solid-js"
import { createDismissable } from "../behavior-primitives/dismissable.ts"
import { createDrag } from "../behavior-primitives/drag.ts"
import { createPresence } from "../behavior-primitives/presence.ts"
import { projected } from "../behavior-primitives/velocity.ts"
import type { Pressed } from "../types/ui.d.ts"

/** A dialog's props. */
export interface DialogProps {
  open?: boolean
  onClose?: () => void
}

/** `const dialog = createDialog(props, duration(tokens.motion.travel))`: whether it's drawn and going, and the props
 * of what's around it and of itself. */
export function createDialog(props: DialogProps, takes: number) {
  const presence = createPresence(() => !!props.open, takes)
  const close = () => props.onClose?.()
  const layer = createDismissable({ open: () => !!props.open, onDismiss: close })
  return {
    ...presence,
    outside: () => layer.outside,
    content: (role: "dialog" | "alertdialog") => ({ role, "aria-modal": "true" as const, tabIndex: -1 as const, onKeyDown: (event: Parameters<typeof layer.keydown>[0]) => void layer.keydown(event) }),
  }
}

/** `const pull = createPull({ enabled: () => !wide(), onClose })`, then `{...pull.gesture}` on the sheet: a finger
 * pulling it down, never a mouse, which `down()` says how far, and which closes it as it's let go far enough. */
export function createPull(options: { enabled: () => boolean; onClose: () => void }) {
  const [down, setDown] = createSignal<number>()
  let height = 0
  const drag = createDrag({
    axis: "y",
    accepts: (event: Pressed) => options.enabled() && event.pointerType !== "mouse" && ((height = event.currentTarget.getBoundingClientRect().height), true),
    onMove: (delta) => setDown(Math.max(0, delta)),
    onEnd: ({ delta, velocity, canceled }) => {
      setDown(undefined)
      if (!canceled && Math.max(0, delta) + Math.max(0, projected(velocity)) > height / 3) options.onClose()
    },
  })
  return { gesture: drag.gesture, down }
}
