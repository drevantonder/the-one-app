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
/// `onClick` it's a button, with a chevron, and `selected` marks the one open beside it.
function ListRow({ leading, title, sub, trailing, selected, onClick, className }: {
  leading?: React.ReactNode
  title: React.ReactNode
  sub?: React.ReactNode
  trailing?: React.ReactNode
  selected?: boolean
  onClick?: () => void
  className?: string
}) {
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
        <span className="block truncate font-medium">{title}</span>
        {sub && <span className="block truncate text-muted-foreground">{sub}</span>}
      </span>
      {trailing}
      {onClick && <CaretRightIcon className="size-4 shrink-0 text-muted-foreground" />}
    </Row>
  )
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

/// A list and what's open from it: beside it where there's room (`wide`, its own width, not the window's), else in a
/// sheet that slides up over it. `detail` is what's open, or nothing; `onClose` is told when the sheet is closed.
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
      style={{ gridTemplateColumns: beside ? "minmax(0, 26rem) minmax(0, 1fr)" : "minmax(0, 1fr)" }}
    >
      <div className="min-w-0">{list}</div>
      {beside ? (
        detail && <div className="min-w-0 max-w-2xl">{detail}</div>
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

export { Grid, List, ListRow, Split, useWidth }
