import { createEffect } from "solid-js"
import { covers } from "./bridge.ts"

/// `ref={popover(() => api().open, () => api().setOpen(false))}` on a part Zag places (a menu's, a select's list, a
/// calendar): shows it as a popover while it's open, over everything else on the page and inside a sheet too, where
/// nothing it sits inside can cut it off. With `close`, it's one of the things a device's own back closes.
export function popover(open: () => boolean, close?: () => void): (placed: HTMLElement) => void {
  let element: HTMLElement | undefined
  let shut: (() => void) | undefined
  createEffect(open, (now) => {
    if (!element?.isConnected || now === element.matches(":popover-open")) return
    element.togglePopover(now)
    shut?.()
    shut = now && close ? covers(close) : undefined
  })
  return (placed: HTMLElement) => {
    element = placed
    element.popover = "manual"
  }
}
