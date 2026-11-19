// A dialog over the page, with its state and no look: a sheet or an alert. From Vercel's UI SDK (Apache-2.0) at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7, packages/core/src/components/dialog/dialog.ts: modal while open, with
// what's behind it inert and the keyboard held in it (our host's `Modal` and `aria-modal`), drawn until its exit has
// run; Escape, a press on what's around it and the device's back close it, each asking its `onClose`. A sheet is
// pulled down to close too (foundation/popups/sheet-pointer.ts), which the host moves as the finger does (`drag`): let
// go a third of the way down, or flicked so it would carry on past that, it closes; otherwise it springs back.
import { createDismissable } from "../behavior-primitives/dismissable.ts"
import { createPresence } from "../behavior-primitives/presence.ts"
import { projected } from "../behavior-primitives/velocity.ts"
import type { Dragged } from "../types/ui.d.ts"

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

/** `{...createPull({ enabled: () => !wide(), onClose })}` on the sheet: it's dragged down, which the host moves as the
 * finger goes, held at its top, and closes as it's let go far enough. */
export function createPull(options: { enabled: () => boolean; onClose: () => void }) {
  return {
    get drag() {
      return options.enabled() ? ("y" as const) : undefined
    },
    dragConstraints: { top: 0 },
    // Pulled up past its top, it gives a little, as an iPhone's sheet does.
    dragElastic: 0.15,
    onDragEnd: ({ offset, velocity, canceled, currentTarget }: Dragged) => {
      const height = currentTarget.getBoundingClientRect().height
      if (!canceled && Math.max(0, offset.y) + Math.max(0, projected(velocity.y / 1000)) > height / 3) options.onClose()
    },
  }
}
