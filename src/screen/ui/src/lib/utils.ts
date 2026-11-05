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

/// A file's size as people read it: "512 Bytes", "1.5 KB", "3.2 MB".
export function fileSize(bytes: number) {
  return bytes < 1024 ? `${bytes} Bytes` : bytes < 1048576 ? `${(bytes / 1024).toFixed(1)} KB` : `${(bytes / 1048576).toFixed(1)} MB`
}
