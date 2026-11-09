import * as React from "react"
import { flushSync } from "react-dom"
import { createRoot } from "react-dom/client"
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter,
  AlertDialogHeader, AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Button } from "@/components/ui/button"
import {
  DropdownMenu, DropdownMenuCheckboxItem, DropdownMenuContent, DropdownMenuItem, DropdownMenuSeparator, DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Icon } from "@/components/icon"
import { Kept } from "@/components/kept"
import { ResponsiveDialog } from "@/components/responsive-dialog"
import { cn, focusBack } from "@/lib/utils"

// What a page that holds screens shows over them when a screen asks (bridge.js's one.menu, one.alert, and a screen
// opened in a sheet): each drawn in a root of its own on the page, gone once it's closed. The web client answers with
// these; a native shell, until it shows its screens in the web's window, with its own.

/// Draws `draw(close)` over the page until it calls `close`, then takes it away once it has animated out. Focus goes back
/// to what had it (a screen's frame, and in it its button) as it closes, before `close`'s value reaches whoever asked,
/// who may open the next thing from it.
function over<T>(draw: (close: (value: T) => void, open: boolean) => React.ReactNode): Promise<T> {
  return new Promise((done) => {
    const el = document.createElement("div")
    document.body.append(el)
    const root = createRoot(el)
    const opener = document.activeElement
    let closed = false
    const close = (value: T) => {
      if (closed) return
      closed = true
      flushSync(() => root.render(draw(close, false)))
      focusBack(opener)
      done(value)
      setTimeout(() => (root.unmount(), el.remove()), 300)
    }
    root.render(draw(close, true))
  })
}

/// An item of a menu, as bridge.js sends it: `row` items lead, side by side as their icons (a widget's sizes);
/// `section` starts a group after a line; `checked` marks the one in use; `danger`, what can't be undone.
type MenuItem = { title: string; icon?: string; danger?: boolean; checked?: boolean; row?: boolean; section?: boolean }

/// A menu from `rect` (`[x, y, width, height]` in the page, the element it's for), opening away from the edge it's near;
/// or at a point, `[x, y]`. → the index of the item chosen, or null once it's closed by a click elsewhere or Escape.
function showMenu(items: MenuItem[], rect: number[]): Promise<number | null> {
  if (!items.length) return Promise.resolve(null)
  const [x, y, w = 0, h = 0] = rect
  const row = items.map((item, i) => [item, i] as const).filter(([item]) => item.row)
  const checks = items.some((item) => item.checked && !item.row)
  return over<number | null>((close, open) => (
    <DropdownMenu open={open} onOpenChange={(o) => !o && close(null)}>
      <DropdownMenuTrigger asChild>
        <span aria-hidden className="pointer-events-none fixed" style={{ left: x, top: y, width: w, height: h }} />
      </DropdownMenuTrigger>
      <DropdownMenuContent align={w ? "center" : "start"} collisionPadding={8} className="max-w-[min(20rem,calc(100vw-1rem))] min-w-48" onCloseAutoFocus={(e) => e.preventDefault()}>
        {!!row.length && (
          <>
            <div className="flex gap-1">
              {row.map(([item, i]) => (
                <DropdownMenuItem
                  key={i}
                  title={item.title}
                  aria-label={item.title}
                  onSelect={() => close(i)}
                  className={cn("flex-1 justify-center", item.checked && "bg-accent text-accent-foreground")}
                >
                  {item.icon ? <Icon name={item.icon} className="size-[18px]" /> : item.title}
                </DropdownMenuItem>
              ))}
            </div>
            <DropdownMenuSeparator />
          </>
        )}
        {items.map((item, i) => {
          if (item.row) return null
          const line = item.section && i > row.length && <DropdownMenuSeparator />
          const face = <><span className="min-w-0 flex-1 truncate">{item.title}</span>{item.icon && <Icon name={item.icon} />}</>
          return (
            <React.Fragment key={i}>
              {line}
              {checks ? (
                <DropdownMenuCheckboxItem checked={!!item.checked} onSelect={() => close(i)} className={cn(item.danger && "text-destructive")}>{face}</DropdownMenuCheckboxItem>
              ) : (
                <DropdownMenuItem variant={item.danger ? "destructive" : "default"} onSelect={() => close(i)}>{face}</DropdownMenuItem>
              )}
            </React.Fragment>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  ))
}

/// An alert: its title and text, then OK; with `action` (its button's name; `danger` if it can't be undone), Cancel
/// and that. → whether the action was chosen.
function showAlert({ title = "", text = "", action, danger = false }: { title?: string; text?: string; action?: string; danger?: boolean }): Promise<boolean> {
  return over<boolean>((close, open) => (
    <AlertDialog open={open} onOpenChange={(o) => !o && close(false)}>
      <AlertDialogContent size="sm">
        <AlertDialogHeader>
          <AlertDialogTitle>{title}</AlertDialogTitle>
          <AlertDialogDescription className={cn(!text && "sr-only")}>{text || title}</AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          {action ? (
            <>
              <AlertDialogCancel onClick={() => close(false)}>Cancel</AlertDialogCancel>
              <AlertDialogAction variant={danger ? "destructive" : "default"} onClick={() => close(true)}>{action}</AlertDialogAction>
            </>
          ) : (
            <AlertDialogAction className="col-span-2" onClick={() => close(false)}>OK</AlertDialogAction>
          )}
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  ))
}

/// A sheet over the page, for a screen opened in one: `body`, where its caller puts the screen, as tall as `fit` was
/// last told (the screen's own height, as its bridge reports it) up to what the window has, else as its content;
/// `title`, its title, which `setTitle` changes; `done`, its one button's name (Done, or Cancel where nothing's kept
/// unless the screen says). Centred where the window has room, else up from its bottom. `closed` settles once it's
/// gone, by `close`, its button, Escape or a click outside it.
function showSheet({ title = "", done = "Done" }: { title?: string; done?: string } = {}) {
  const body = document.createElement("div")
  body.className = "relative min-h-0 shrink"
  let setTitle: (title: string) => void = () => {}
  let close = () => {}
  const closed = over<void>((closeWith, open) => {
    close = () => closeWith()
    return <ScreenSheet open={open} body={body} title={title} done={done} close={close} titled={(f) => (setTitle = f)} />
  })
  return { body, closed, close: () => close(), setTitle: (t: string) => setTitle(t), fit: (px: number) => (body.style.height = `${px}px`) }
}

function ScreenSheet({ open, body, title: first, done, close, titled }: {
  open: boolean
  body: HTMLElement
  title: string
  done: string
  close: () => void
  titled: (set: (title: string) => void) => void
}) {
  const [title, setTitle] = React.useState(first)
  React.useLayoutEffect(() => titled(setTitle), [titled])
  return (
    <ResponsiveDialog open={open} onOpenChange={(o) => !o && close()} title={title} action={<Button variant="secondary" size="sm" onClick={close}>{done}</Button>}>
      <Kept element={body} className="flex min-h-0 flex-1 flex-col" />
    </ResponsiveDialog>
  )
}

export { showMenu, showAlert, showSheet, type MenuItem }
