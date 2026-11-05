import * as React from "react"
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog"
import { Sheet, SheetContent, SheetDescription, SheetTitle } from "@/components/ui/sheet"
import { cn } from "@/lib/utils"

/// shadcn's responsive dialog: a dialog in the middle of the window where it has room, else a sheet up from its bottom.
/// Its head is its `title` and `action`, its buttons (the one that keeps what's in it, say Save, or Cancel where nothing
/// is); it closes by Escape or a click outside it too, telling `onOpenChange(false)`. `tall` is for something that
/// fills it whatever its height, like a screen.
function ResponsiveDialog({ open, onOpenChange, title, action, tall = false, className, children }: {
  open: boolean
  onOpenChange: (open: boolean) => void
  title: string
  action?: React.ReactNode
  tall?: boolean
  className?: string
  children: React.ReactNode
}) {
  // The window's own width: it sits over the whole window.
  const wide = useWindowWidth() >= 640
  const head = (
    <div className="flex shrink-0 items-center gap-2 border-b px-4 py-3">
      {wide ? <DialogTitle className="min-w-0 flex-1 truncate text-base">{title}</DialogTitle> : <SheetTitle className="min-w-0 flex-1 truncate">{title}</SheetTitle>}
      {wide ? <DialogDescription className="sr-only">{title}</DialogDescription> : <SheetDescription className="sr-only">{title}</SheetDescription>}
      {action}
    </div>
  )
  return wide ? (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        showCloseButton={false}
        className={cn("flex flex-col gap-0 overflow-hidden p-0 sm:max-w-[480px]", tall ? "h-[min(620px,88vh)]" : "max-h-[88vh]", className)}
      >
        {head}
        {children}
      </DialogContent>
    </Dialog>
  ) : (
    <Sheet open={open} onOpenChange={onOpenChange}>
      <SheetContent side="bottom" showCloseButton={false} className={cn("gap-0 overflow-hidden rounded-t-xl", tall ? "h-[92svh]" : "max-h-[92svh]", className)}>
        {head}
        {children}
      </SheetContent>
    </Sheet>
  )
}

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

function useWindowWidth() {
  const [width, setWidth] = React.useState(() => window.innerWidth)
  React.useEffect(() => {
    const resized = () => setWidth(window.innerWidth)
    addEventListener("resize", resized)
    return () => removeEventListener("resize", resized)
  }, [])
  return width
}

export { ResponsiveDialog, useDialog }
