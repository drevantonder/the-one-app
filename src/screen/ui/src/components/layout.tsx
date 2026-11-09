import * as React from "react"
import { CaretRightIcon } from "@phosphor-icons/react"
import { Sheet, SheetContent, SheetDescription, SheetHeader, SheetTitle } from "@/components/ui/sheet"
import { cn } from "@/lib/utils"

/// A grid that fills the room it has: as many columns as fit at `min` wide each, at most `max` of them, so a big window
/// gets bigger tiles rather than more of them past that.
function Grid({ min = "14rem", max = 6, gap = "1rem", className, style, ...props }: React.ComponentProps<"div"> & {
  min?: string
  max?: number
  gap?: string
}) {
  const column = `minmax(min(100%, max(${min}, (100% - ${max - 1} * ${gap}) / ${max})), 1fr)`
  return (
    <div
      data-slot="grid"
      className={cn("grid", className)}
      style={{ gridTemplateColumns: `repeat(auto-fill, ${column})`, gap, ...style }}
      {...props}
    />
  )
}

/// Rows in one group, which stops at a readable width however wide the room is.
function List({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="list"
      className={cn("w-full max-w-2xl divide-y overflow-hidden rounded-xl border bg-card text-card-foreground", className)}
      {...props}
    />
  )
}

/// A row of a List: what it starts with (an app's icon, say), its title and one grey line, and what it ends with. With
/// `onClick` it's a button, with a chevron, and `selected` marks the one open beside it. Its title and line are cut
/// to one line each; with `wrap` they run on to as many as they need.
function ListRow({ leading, title, sub, trailing, selected, wrap, onClick, className }: {
  leading?: React.ReactNode
  title: React.ReactNode
  sub?: React.ReactNode
  trailing?: React.ReactNode
  selected?: boolean
  wrap?: boolean
  onClick?: () => void
  className?: string
}) {
  const cut = wrap ? "break-words" : "truncate"
  const Row = onClick ? "button" : "div"
  return (
    <Row
      data-slot="list-row"
      data-selected={selected || undefined}
      onClick={onClick}
      className={cn(
        "flex w-full min-w-0 items-center gap-3 px-3 py-2.5 text-left text-sm",
        onClick && "cursor-pointer outline-none hover:bg-accent/50 focus-visible:bg-accent/50 data-[selected]:bg-accent",
        className
      )}
    >
      {leading}
      <span className="min-w-0 flex-1">
        <span className={cn("block font-medium", cut)}>{title}</span>
        {sub && <span className={cn("block text-muted-foreground", cut)}>{sub}</span>}
      </span>
      {trailing}
      {onClick && <CaretRightIcon className="size-4 shrink-0 text-muted-foreground" />}
    </Row>
  )
}

/// A grey line under what it's about.
function Hint({ children, className }: { children: React.ReactNode; className?: string }) {
  return <p data-slot="hint" className={cn("text-sm text-muted-foreground", className)}>{children}</p>
}

/// A line of what to do, as a row of a List: it wraps, which a ListRow's title doesn't.
function ListNote({ children }: { children: React.ReactNode }) {
  return <div data-slot="list-note" className="px-3 py-2.5 text-sm">{children}</div>
}

/// How wide an element is, as it changes: what the layout parts size themselves by, rather than the window.
function useWidth<T extends HTMLElement>() {
  const ref = React.useRef<T>(null)
  const [width, setWidth] = React.useState(0)
  React.useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    // Measured at once, before the first paint, so nothing is drawn for a width it doesn't have.
    setWidth(el.getBoundingClientRect().width)
    const seen = new ResizeObserver(([e]) => setWidth(e.contentRect.width))
    seen.observe(el)
    return () => seen.disconnect()
  }, [])
  return [ref, width] as const
}

/// A list and what's open from it: the list has the room until something's open, then that's beside it where there's
/// room (`wide`, its own width, not the window's), else in a sheet that slides up over it. `detail` is what's open, or nothing; `onClose` is told when the sheet is closed.
function Split({ list, detail, title, onClose, wide = 720, className }: {
  list: React.ReactNode
  detail?: React.ReactNode
  /// What's open, for the sheet's title (and screen readers).
  title?: string
  onClose?: () => void
  wide?: number
  className?: string
}) {
  const [ref, width] = useWidth<HTMLDivElement>()
  const beside = width >= wide
  return (
    <div
      ref={ref}
      data-slot="split"
      className={cn("grid items-start gap-6", className)}
      style={{ gridTemplateColumns: beside && detail ? "minmax(0, 26rem) minmax(0, 1fr)" : "minmax(0, 1fr)" }}
    >
      <div className="min-w-0">{list}</div>
      {beside ? (
        detail && <div className="sticky top-4 max-h-[calc(100dvh-2rem)] min-w-0 max-w-2xl overflow-y-auto">{detail}</div>
      ) : (
        <Sheet open={!!detail && width > 0} onOpenChange={(open) => !open && onClose?.()}>
          <SheetContent side="bottom" className="max-h-[90svh] overflow-y-auto rounded-t-xl">
            <SheetHeader className="sr-only">
              <SheetTitle>{title}</SheetTitle>
              <SheetDescription>{title}</SheetDescription>
            </SheetHeader>
            <div className="p-4 pt-10">{detail}</div>
          </SheetContent>
        </Sheet>
      )}
    </div>
  )
}

export { Grid, Hint, List, ListNote, ListRow, Split, useWidth }
