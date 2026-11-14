import { createEffect } from "solid-js"
import { covers } from "./bridge.js"

/// `ref={popover(() => api().open, () => api().setOpen(false))}` on a part Zag places (a menu's, a select's list, a
/// calendar): shows it as a popover while it's open, over everything else on the page and inside a sheet too, where
/// nothing it sits inside can cut it off. With `close`, it's one of the things a device's own back closes.
export function popover(open, close) {
  let element
  let shut
  createEffect(open, (now) => {
    if (!element?.isConnected || now === element.matches(":popover-open")) return
    element.togglePopover(now)
    shut?.()
    shut = now && close ? covers(close) : undefined
  })
  return (placed) => {
    element = placed
    element.popover = "manual"
  }
}
