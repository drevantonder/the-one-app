import * as React from "react";
import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/// A dialog's focus, spread on its content as `{...dialogFocus()}`. Opening: Radix focuses its first control, and a
/// browser may draw that control's keyboard ring though it was a click that opened it; this focuses the dialog itself,
/// where nothing is drawn, and the first Tab goes to the first control, ringed (a control the dialog gave focus to
/// itself, autoFocus, keeps it). Closing: Radix gives focus back only to a trigger it was given, and these open from a
/// state, so focus would drop to the start of the page, and a keyboard's user would be at the top again; this puts it
/// back on what had it as the dialog opened (with none, Radix does what it does).
export function useDialogFocus() {
  const opener = React.useRef<Element | null>(null)
  return {
    onOpenAutoFocus(event: Event) {
      opener.current = document.activeElement
      event.preventDefault()
      const dialog = event.target as HTMLElement
      if (!dialog.contains(document.activeElement)) dialog.focus({ preventScroll: true })
    },
    onCloseAutoFocus: (event: Event) => focusBack(opener.current, event),
  }
}

/// Focus back on `opener`, where it came from, if it's still there. Where `opener` is the frame of a screen (an
/// overlay of the window's, which a screen asked for), that's the frame, and in it what had focus. Given the `event` of
/// a dialog's `onCloseAutoFocus`, only if focus is lost (on the page, or in what's gone): that is Radix's, late.
export function focusBack(opener: Element | null, event?: Event) {
  if (!(opener instanceof HTMLElement) || opener === document.body || !opener.isConnected) return
  if (event) {
    const now = document.activeElement
    if (now && now !== document.body && !(event.target instanceof Node && event.target.contains(now))) return
    event.preventDefault()
  }
  opener.focus({ preventScroll: true })
  if (opener instanceof HTMLIFrameElement) opener.contentWindow?.postMessage({ oneFocus: true }, "*")
}

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

/// A file's size as users read it: "512 Bytes", "1.5 KB", "3.2 MB".
export function fileSize(bytes: number) {
  return bytes < 1024 ? `${bytes} Bytes` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`
}
