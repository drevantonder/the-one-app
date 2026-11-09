import * as React from "react"
import { ArrowUpRightIcon, CaretLeftIcon, CodeIcon } from "@phosphor-icons/react"
import { AppIcon, type App } from "@/components/app-icon"
import { Kept } from "@/components/kept"
import { Button } from "@/components/ui/button"
import {
  Sidebar, SidebarContent, SidebarGroup, SidebarGroupLabel, SidebarInset, SidebarMenu, SidebarMenuBadge,
  SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarTrigger, useSidebar,
} from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"

/// A row of the sidebar: an app, a page's Accounts (`folder`, the icons of its apps that keep accounts) or a Settings item. `href` is where
/// it goes, a real link for a new tab; `external` opens another site.
type ShellItem = {
  key: string
  title: string
  href: string
  app?: App
  folder?: App[]
  badge?: number
  selected?: boolean
  external?: boolean
}

/// A group of rows under its page's name (You, a group's, Everyone, Settings), which opens that page.
type ShellSection = { key: string; title: string; href: string; selected?: boolean; items: ShellItem[] }

/// The window every client shows screens in: the sidebar of pages and their apps, then Settings', and beside it a bar
/// with the title over what's open (`children`, or `screen`, an element the client keeps its screens' frames in, put
/// here as it is). Where its room is narrow, the sidebar is a sheet behind the bar's button. `onGo` is told of a plain
/// click on a row or a page's name (a click with a modifier opens the link as the browser does); `back`, a Back button
/// before the title; `end`, what the bar ends with. A `phone`'s has no sidebar: its home is the pages side by side
/// (their own titles, no bar), and what's opened from them has a bar with Back, as the native phone apps had. `banner`,
/// what's wrong with the whole window (being offline), once, at the top of what it shows.
function Shell({ sections, title, back, onBack, onGo, developer, onDeveloper, open, onOpenChange, screen, children, className, phone, banner }: {
  sections: ShellSection[]
  title?: string
  back?: boolean
  onBack?: () => void
  onGo?: (href: string) => void
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
      {/* One button, where it stays as the sidebar slides under it, clear of a title bar lying over the page (`--titlebar-area-x`). */}
      <div className="pointer-events-none absolute top-0 left-[calc(var(--titlebar-area-x,0px)+0.75rem)] z-20 flex h-(--shell-bar) items-center">
        <SidebarTrigger className="pointer-events-auto" />
      </div>
      <Sidebar>
        <div className="h-(--shell-bar) shrink-0" />
        <SidebarContent className="relative">
          <Selected />
          {sections.map((s) => <ShellGroup key={s.key} section={s} onGo={onGo} />)}
        </SidebarContent>
      </Sidebar>
      {/* Where the bar starts: past the sidebar's button while the sidebar's away, as the sidebar goes. */}
      <SidebarInset className="min-w-0 overflow-hidden [--shell-bar-start:calc(var(--titlebar-area-x,0px)+3.25rem)] peer-data-[state=expanded]:[--shell-bar-start:0.75rem]">
        <header data-slot="shell-bar" className={cn("flex h-(--shell-bar) shrink-0 items-center gap-2 pr-3 pl-(--shell-bar-start) transition-[padding] duration-200 ease-linear", title && "border-b wco:border-b-0")}>
          {back && (
            <Button variant="ghost" size="icon" className="relative z-20 size-7" onClick={onBack} aria-label="Back">
              <CaretLeftIcon />
            </Button>
          )}
          <h1 className="min-w-0 truncate text-sm font-semibold">{title}</h1>
          {developer && (
            <Button variant="ghost" size="icon" className="relative z-20 ml-auto size-7" onClick={onDeveloper} aria-label="Developer">
              <CodeIcon />
            </Button>
          )}
        </header>
        {banner}
        {body}
      </SidebarInset>
    </SidebarProvider>
  )
}

/// The highlight behind the selected row, one element that slides from row to row.
function Selected() {
  const ref = React.useRef<HTMLSpanElement>(null)
  React.useLayoutEffect(() => {
    const el = ref.current!
    const row = el.parentElement!.querySelector<HTMLElement>('[data-sidebar="menu-button"][data-active="true"]')
    el.style.visibility = row ? "" : "hidden"
    if (!row) return
    const box = el.parentElement!.getBoundingClientRect(), at = row.getBoundingClientRect()
    el.style.width = `${at.width}px`
    el.style.height = `${at.height}px`
    el.style.translate = `${at.left - box.left}px ${at.top - box.top + el.parentElement!.scrollTop}px`
    if (!el.dataset.placed) requestAnimationFrame(() => (el.dataset.placed = ""))
  })
  return <span ref={ref} aria-hidden="true" className="absolute top-0 left-0 rounded-md bg-sidebar-accent data-[placed]:transition-[translate] data-[placed]:duration-200 data-[placed]:ease-out motion-reduce:transition-none" />
}

function ShellGroup({ section, onGo }: { section: ShellSection; onGo?: (href: string) => void }) {
  const { isMobile, setOpenMobile } = useSidebar()
  // A plain click goes in place, and closes the sidebar's sheet; any other is the browser's (a new tab).
  const go = (href: string) => (e: React.MouseEvent) => {
    if (!onGo || e.metaKey || e.ctrlKey || e.shiftKey || e.button) return
    e.preventDefault()
    onGo(href)
    if (isMobile) setOpenMobile(false)
  }
  return (
    <SidebarGroup>
      <SidebarGroupLabel asChild>
        <a href={section.href} onClick={go(section.href)} data-selected={section.selected || undefined} className="hover:text-sidebar-foreground data-[selected]:text-sidebar-foreground">
          {section.title}
        </a>
      </SidebarGroupLabel>
      <SidebarMenu>
        {section.items.map((item) => (
          <SidebarMenuItem key={item.key}>
            <SidebarMenuButton asChild isActive={item.selected} className="data-[active=true]:bg-transparent">
              <a
                href={item.href}
                {...(item.external ? { target: "_blank", rel: "noopener" } : { onClick: go(item.href) })}
              >
                {item.folder ? <Folder apps={item.folder} /> : item.app && <AppIcon app={item.app} size={20} />}
                <span className="truncate">{item.title}</span>
                {item.external && <ArrowUpRightIcon className="ml-auto text-muted-foreground" />}
              </a>
            </SidebarMenuButton>
            {!!item.badge && <SidebarMenuBadge className="rounded-full bg-destructive text-white peer-data-[active=true]/menu-button:text-white">{item.badge}</SidebarMenuBadge>}
          </SidebarMenuItem>
        ))}
      </SidebarMenu>
    </SidebarGroup>
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

export { Shell, Folder, type ShellItem, type ShellSection }
