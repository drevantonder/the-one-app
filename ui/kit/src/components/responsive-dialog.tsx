import * as React from "react"
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog"
import { cn } from "@/lib/utils"

/// shadcn's responsive dialog: a dialog in the middle of the window where it has room, else a sheet up from its bottom.
/// Its head is its `title` and `action`, its buttons (the one that keeps what's in it, say Save, or Cancel where nothing
/// is); it closes by Escape or a click outside it too, telling `onOpenChange(false)`.
function ResponsiveDialog({ open, onOpenChange, title, action, className, children }: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  action?: React.ReactNode
  className?: string
  children: React.ReactNode
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent showCloseButton={false} className={cn(shape, sheetBelow, className)}>
        <div className="flex shrink-0 items-center gap-2 border-b px-4 py-3">
          <DialogTitle className="min-w-0 flex-1 truncate text-base">{title}</DialogTitle>
          <DialogDescription className="sr-only">{title}</DialogDescription>
          {action}
        </div>
        {children}
      </DialogContent>
    </Dialog>
  )
}

const shape = "flex flex-col gap-0 overflow-hidden p-0 sm:max-w-[480px] max-h-[88vh]"
// By the window's width alone, as one element, so a phone turned or a window resized keeps what's typed in it.
const sheetBelow = "max-sm:top-auto max-sm:bottom-0 max-sm:left-0 max-sm:translate-x-0 max-sm:translate-y-0 max-sm:max-w-none max-sm:max-h-[92svh] max-sm:rounded-none max-sm:rounded-t-xl max-sm:border-x-0 max-sm:border-b-0 max-sm:duration-(--move-travel) max-sm:ease-(--ease-travel) max-sm:data-[state=closed]:duration-(--move-settle) max-sm:data-[state=open]:zoom-in-100 max-sm:data-[state=closed]:zoom-out-100 max-sm:data-[state=open]:slide-in-from-bottom max-sm:data-[state=closed]:slide-out-to-bottom"

/// What a screen keeps to open a dialog afresh each time: `[dialog, show, setOpen]`. `show(target)` opens it for
/// `target` (what it's about, or nothing), with a new `dialog.n` each time; the dialog is drawn once `dialog.n > 0`,
/// `key={dialog.n}` so what was typed in the last one is gone, `open={dialog.open}` and `onOpenChange={setOpen}`. It
/// stays drawn after it closes, so it animates out, and `dialog.for` is still what it was for.
function useDialog<T = null>() {
  const [d, set] = React.useState<{ n: number; open: boolean; for: T | null }>({ n: 0, open: false, for: null })
  return [
    d,
    (target?: T) => set((x) => ({ n: x.n + 1, open: true, for: target ?? null })),
    (open: boolean) => set((x) => ({ ...x, open })),
  ] as const
}

export { ResponsiveDialog, useDialog }
