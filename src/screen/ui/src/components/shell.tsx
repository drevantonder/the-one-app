import * as React from "react"
import { ArrowUpRightIcon, CaretLeftIcon, CodeIcon } from "@phosphor-icons/react"
import { AppIcon, type App } from "@/components/app-icon"
import { Button } from "@/components/ui/button"
import { Separator } from "@/components/ui/separator"
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
/// (their own titles, no bar), and what's opened from them has a bar with Back, as the native phone apps had.
function Shell({ sections, title, back, onBack, onGo, developer, onDeveloper, open, onOpenChange, screen, children, className, phone }: {
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
}) {
  const body = (
    <div data-slot="shell-screen" className="relative flex min-h-0 flex-1 flex-col">
      {screen ? <Kept element={screen} /> : children}
    </div>
  )
  if (phone) {
    return (
      <div className={cn("flex h-full flex-col pt-[env(safe-area-inset-top)]", className)}>
        {back && (
          <header data-slot="shell-bar" className="grid h-11 shrink-0 grid-cols-[1fr_auto_1fr] items-center px-1">
            <Button variant="ghost" className="justify-self-start px-2" onClick={onBack} aria-label="Back">
              <CaretLeftIcon />
            </Button>
            <h1 className="min-w-0 truncate text-[15px] font-semibold">{title}</h1>
          </header>
        )}
        {body}
      </div>
    )
  }
  return (
    <SidebarProvider open={open} onOpenChange={onOpenChange} className={cn("h-full", className)}>
      <Sidebar>
        <SidebarContent>
          {sections.map((s) => <ShellGroup key={s.key} section={s} onGo={onGo} />)}
        </SidebarContent>
      </Sidebar>
      <SidebarInset className="min-w-0 overflow-hidden">
        <header data-slot="shell-bar" className="flex h-12 shrink-0 items-center gap-2 border-b px-3">
          <SidebarTrigger />
          <Separator orientation="vertical" className="mr-1 data-[orientation=vertical]:h-4" />
          {back && (
            <Button variant="ghost" size="icon" className="size-7" onClick={onBack} aria-label="Back">
              <CaretLeftIcon />
            </Button>
          )}
          <h1 className="min-w-0 truncate text-sm font-semibold">{title}</h1>
          {developer && (
            <Button variant="ghost" size="icon" className="ml-auto size-7" onClick={onDeveloper} aria-label="Developer" title="Developer">
              <CodeIcon />
            </Button>
          )}
        </header>
        {body}
      </SidebarInset>
    </SidebarProvider>
  )
}

/// An element the client made and keeps, put here as it is: its frames go on running through every draw.
function Kept({ element }: { element: HTMLElement }) {
  const ref = React.useRef<HTMLDivElement>(null)
  React.useLayoutEffect(() => {
    if (element.parentElement !== ref.current) ref.current?.append(element)
  }, [element])
  return <div ref={ref} className="contents" />
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
            <SidebarMenuButton asChild isActive={item.selected}>
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
