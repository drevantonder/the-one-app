import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

/// A dialog's `onOpenAutoFocus`: Radix focuses its first control when it opens, and a browser may draw that control's
/// keyboard ring though it was a click that opened it. This focuses the dialog itself, where nothing is drawn, and
/// the first Tab goes to the first control, ringed. A control the dialog gave focus to itself (autoFocus) keeps it.
export function focusDialog(event: Event) {
  event.preventDefault()
  const dialog = event.target as HTMLElement
  if (!dialog.contains(document.activeElement)) dialog.focus({ preventScroll: true })
}

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
