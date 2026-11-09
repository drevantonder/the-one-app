import * as React from "react"
import { CaretLeftIcon, CodeIcon, GearSixIcon } from "@phosphor-icons/react"
import { AppIcon, type App } from "@/components/app-icon"
import { Kept } from "@/components/kept"
import { Button } from "@/components/ui/button"
import {
  Sidebar, SidebarContent, SidebarGroup, SidebarInset, SidebarMenu, SidebarMenuBadge,
  SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarTrigger, useSidebar,
} from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"

/// A row of the sidebar: an app, or the Accounts folder (`folder`, the icons of the apps that keep accounts). `href` is where it goes, a real
/// link for a new tab.
type ShellItem = {
  key: string
  title: string
  href: string
  app?: App
  folder?: App[]
  badge?: number
  selected?: boolean
}

/// A page's row (You, a group's, Everyone), and under it `items`, the apps of the page the window is on; with none under
/// it, `badge` is what its apps count. Settings is one too, at the sidebar's foot.
type ShellPage = { key: string; title: string; href: string; selected?: boolean; badge?: number; items: ShellItem[] }

/// The window every client shows screens in: the sidebar of pages, the current one's apps under it and Settings at its
/// foot, and beside it a bar with the title over what's open (`children`, or `screen`, an element the client keeps its
/// screens' frames in, put here as it is). Where its room is narrow, the sidebar is a sheet behind the bar's button. `onGo`
/// is told of a plain click on a row (a click with a modifier opens the link as the browser does); `back`, a Back button
/// before the title; `onBarStart`, how far along the bar its own buttons reach. A `phone`'s has no sidebar: its home is the
/// pages side by side (their own titles, no bar), and what's opened from them has a bar with Back, as the native phone apps
/// had. `banner`, what's wrong with the whole window (being offline), once, at the top of what it shows.
function Shell({ pages = [], settings, title, back, onBack, onGo, onBarStart, developer, onDeveloper, open, onOpenChange, screen, children, className, phone, banner }: {
  pages?: ShellPage[]
  settings?: ShellPage
  title?: string
  back?: boolean
  onBack?: () => void
  onGo?: (href: string) => void
  onBarStart?: (px: number) => void
  developer?: boolean
  onDeveloper?: () => void
  open?: boolean
  onOpenChange?: (open: boolean) => void
  screen?: HTMLElement
  children?: React.ReactNode
  className?: string
  phone?: boolean
  banner?: React.ReactNode
}) {
  const body = (
    <div data-slot="shell-screen" className="relative flex min-h-0 flex-1 flex-col">
      {screen ? <Kept element={screen} /> : children}
    </div>
  )
  if (phone) {
    return (
      <div className={cn("flex h-full flex-col pt-[env(safe-area-inset-top)]", className)}>
        {banner}
        {/* The bar lies over where the screen is, which keeps its size as a screen with no bar gives way to one with a bar. */}
        <div className="relative flex min-h-0 flex-1 flex-col">
          {back && (
            <header data-slot="shell-bar" className="absolute inset-x-0 top-0 z-10 grid h-11 grid-cols-[1fr_auto_1fr] items-center bg-background px-1">
              <Button variant="ghost" className="justify-self-start px-2" onClick={onBack} aria-label="Back">
                <CaretLeftIcon />
              </Button>
              <h1 className="min-w-0 truncate text-[15px] font-semibold">{title}</h1>
            </header>
          )}
          {body}
        </div>
      </div>
    )
  }
  return (
    <SidebarProvider open={open} onOpenChange={onOpenChange} className={cn("relative h-full", className)}>
      {/* The window’s buttons, where they stay as the sidebar slides under them, clear of a title bar lying over the page (`--titlebar-area-x`). */}
      <div className="pointer-events-none absolute top-0 left-[calc(var(--titlebar-area-x,0px)+0.75rem)] z-20 flex h-(--shell-bar) items-center gap-1">
        <SidebarTrigger className="pointer-events-auto" />
        {developer && (
          <Button variant="ghost" size="icon" className="pointer-events-auto size-7" onClick={onDeveloper} aria-label="Developer">
            <CodeIcon />
          </Button>
        )}
      </div>
      <Sidebar>
        <div className="h-(--shell-bar) shrink-0" />
        <SidebarContent className="relative">
          <Selected rows={pages} />
          <SidebarGroup>
            <SidebarMenu>
              {pages.map((p) => (
                <React.Fragment key={p.key}>
                  <ShellRow row={p.items.length ? { ...p, badge: 0 } : p} onGo={onGo} className="font-medium" />
                  {p.items.map((item) => (
                    <ShellRow key={item.key} row={item} onGo={onGo}>
                      {item.folder ? <Folder apps={item.folder} /> : item.app && <AppIcon app={item.app} size={20} />}
                    </ShellRow>
                  ))}
                </React.Fragment>
              ))}
            </SidebarMenu>
          </SidebarGroup>
          {settings && (
            <SidebarGroup className="sticky bottom-0 mt-auto bg-sidebar has-data-[active=true]:bg-transparent">
              <SidebarMenu>
                <ShellRow row={settings} onGo={onGo} className="font-medium">
                  <GearSixIcon />
                </ShellRow>
              </SidebarMenu>
            </SidebarGroup>
          )}
        </SidebarContent>
      </Sidebar>
      {/* Where the bar starts: past the window’s buttons while the sidebar’s away, as the sidebar goes. */}
      <SidebarInset data-banner={banner ? "" : undefined} className={cn("min-w-0 overflow-hidden peer-data-[state=expanded]:[--shell-bar-start:0.75rem]", developer ? "[--shell-bar-start:calc(var(--titlebar-area-x,0px)+5.25rem)]" : "[--shell-bar-start:calc(var(--titlebar-area-x,0px)+3.25rem)]")}>
        <BarStart back={back} onChange={onBarStart} />
        {/* Over the screen, whose large title lies in this row until it scrolls away and the title here takes over. Above the
            title bar’s drag region only for its buttons: the rest lets the drag through. */}
        <header data-slot="shell-bar" className={cn("pointer-events-none relative z-12 flex h-(--shell-bar) shrink-0 items-center gap-2 pr-3 pl-(--shell-bar-start) transition-[padding] duration-200 ease-linear [&>button]:pointer-events-auto", title && "pointer-events-auto border-b bg-background wco:pointer-events-none wco:border-b-0")}>
          {back && (
            <Button variant="ghost" size="icon" className="size-7" onClick={onBack} aria-label="Back">
              <CaretLeftIcon />
            </Button>
          )}
          <h1 className="min-w-0 truncate text-sm font-semibold">{title}</h1>
        </header>
        {banner}
        {body}
      </SidebarInset>
    </SidebarProvider>
  )
}

/// Says where in the bar its title starts, past Back and the window's buttons, at once as that changes rather than as the
/// bar's padding slides there.
function BarStart({ back, onChange }: { back?: boolean; onChange?: (px: number) => void }) {
  useSidebar()
  const ref = React.useRef<HTMLSpanElement>(null)
  React.useLayoutEffect(() => onChange?.(ref.current!.offsetWidth))
  return <span ref={ref} aria-hidden="true" className={cn("invisible absolute", back ? "w-[calc(var(--shell-bar-start)+2.25rem)]" : "w-(--shell-bar-start)")} />
}

/// The highlight behind the selected row, one element that slides from row to row, placed again as `rows` change.
function Selected({ rows }: { rows: ShellPage[] }) {
  const ref = React.useRef<HTMLSpanElement>(null)
  React.useLayoutEffect(() => {
    const el = ref.current!, list = el.parentElement!
    const place = () => {
      const row = list.querySelector<HTMLElement>('[data-sidebar="menu-button"][data-active="true"]')
      el.style.visibility = row ? "" : "hidden"
      if (!row) return
      const box = list.getBoundingClientRect(), at = row.getBoundingClientRect()
      el.style.width = `${at.width}px`
      el.style.height = `${at.height}px`
      el.style.translate = `${at.left - box.left}px ${at.top - box.top + list.scrollTop}px`
      if (!el.dataset.placed) requestAnimationFrame(() => (el.dataset.placed = ""))
    }
    place()
    // Settings stays at the foot as the rows scroll under it.
    list.addEventListener("scroll", place, { passive: true })
    return () => list.removeEventListener("scroll", place)
  }, [rows])
  return <span ref={ref} aria-hidden="true" className="absolute top-0 left-0 rounded-md bg-sidebar-accent data-[placed]:transition-[translate] data-[placed]:duration-200 data-[placed]:ease-out motion-reduce:transition-none" />
}

function ShellRow({ row, onGo, className, children }: { row: ShellItem; onGo?: (href: string) => void; className?: string; children?: React.ReactNode }) {
  const { isMobile, setOpenMobile } = useSidebar()
  // A plain click goes in place, and closes the sidebar's sheet; any other is the browser's (a new tab).
  const go = (e: React.MouseEvent) => {
    if (!onGo || e.metaKey || e.ctrlKey || e.shiftKey || e.button) return
    e.preventDefault()
    onGo(row.href)
    if (isMobile) setOpenMobile(false)
  }
  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild isActive={row.selected} className={cn("data-[active=true]:bg-transparent", className)}>
        <a href={row.href} onClick={go}>
          {children}
          <span className="truncate">{row.title}</span>
        </a>
      </SidebarMenuButton>
      {!!row.badge && <SidebarMenuBadge className="rounded-full bg-destructive text-white peer-data-[active=true]/menu-button:text-white">{row.badge}</SidebarMenuBadge>}
    </SidebarMenuItem>
  )
}

/// A page's Accounts: the icons of its apps that keep accounts, four at most, in a tile of its own; with none, four faint places.
function Folder({ apps, size = 20 }: { apps: App[]; size?: number }) {
  const mini = Math.round(size * 0.36)
  return (
    <span
      data-slot="folder"
      className="grid shrink-0 grid-cols-2 content-start bg-muted-foreground/20"
      style={{ width: size, height: size, borderRadius: size * 0.225, padding: size * 0.1, gap: size * 0.08, gridAutoRows: mini }}
    >
      {apps.length
        ? apps.slice(0, 4).map((a) => <AppIcon key={a.name} app={{ ...a, icon: undefined, title: " " }} size={mini} />)
        : Array.from({ length: 4 }, (_, i) => <i key={i} className="rounded-[2px] bg-muted-foreground/20" />)}
    </span>
  )
}

export { Shell, Folder, type ShellItem, type ShellPage }
