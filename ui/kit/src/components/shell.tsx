import * as React from "react"
import { ArrowUpRightIcon, CaretLeftIcon, CaretRightIcon } from "@phosphor-icons/react"
import { Collapsible } from "radix-ui"
import { AppIcon, type App } from "@/components/app-icon"
import { Kept } from "@/components/kept"
import { Button } from "@/components/ui/button"
import {
  Sidebar, SidebarContent, SidebarGroup, SidebarGroupLabel, SidebarInset, SidebarMenu, SidebarMenuBadge,
  SidebarMenuButton, SidebarMenuItem, SidebarProvider, SidebarTrigger, useSidebar,
} from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"

/// A row of the sidebar: an app, a Settings item, or the Accounts folder (`folder`, the icons of the apps that keep accounts). `href` is
/// where it goes, a real link for a new tab; `external` opens another site.
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

/// A count as a badge says it.
export const badgeCount = (n: number) => (n > 99 ? "99+" : String(n))

/// A part of the sidebar: a page (You, a group's, Everyone) or Settings, its name going there and `items` under it unless
/// it's collapsed (`open`), when `badge`, what they count, shows on its name.
type ShellSection = { key: string; title: string; href: string; selected?: boolean; open: boolean; badge?: number; items: ShellItem[] }

/// The window every client shows screens in: the sidebar of sections, and beside it a bar with the title over what's open
/// (`children`, or `screen`, an element the client keeps its screens' frames in, put here as it is). Where its room is
/// narrow, the sidebar is a sheet behind the bar's button. `onGo` is told of a plain click on a row or a section's name (a
/// click with a modifier opens the link as the browser does), `onPress` as a plain press comes down on one, before it lets
/// go, and `onSection` as a section is collapsed or opened; `back`, a Back button
/// before the title; `onBarStart`, how far along the bar its own buttons reach. A `phone`'s has no sidebar: its home is the
/// pages side by side (their own titles, no bar), and what's opened from them has a bar with Back, as the native phone apps
/// had. `banner`, what's wrong with the whole window (being offline), once, at the top of what it shows.
function Shell({ sections = [], title, back, onBack, onGo, onPress, onSection, onBarStart, open, onOpenChange, screen, children, className, phone, banner }: {
  sections?: ShellSection[]
  title?: string
  back?: boolean
  onBack?: () => void
  onGo?: (href: string) => void
  onPress?: (href: string) => void
  onSection?: (key: string, open: boolean) => void
  onBarStart?: (px: number) => void
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
              <Button variant="ghost" className="justify-self-start px-2 pointer-coarse:px-3" onClick={onBack} aria-label="Back">
                <CaretLeftIcon />
              </Button>
              {title && <h1 className="min-w-0 truncate text-[15px] font-semibold">{title}</h1>}
            </header>
          )}
          {body}
        </div>
      </div>
    )
  }
  return (
    <SidebarProvider open={open} onOpenChange={onOpenChange} className={cn("relative h-full", className)}>
      {/* The first Tab, for a keyboard past the sidebar's rows: on to the screen, whose frame takes the next. */}
      <Button
        variant="secondary"
        size="sm"
        className="absolute top-1.5 left-[calc(var(--titlebar-area-x,0px)+5.5rem)] z-30 -translate-y-14 focus-visible:translate-y-0"
        onClick={() => document.querySelector<HTMLElement>('[data-slot="shell-screen"] iframe:not([aria-hidden])')?.focus()}
      >
        Skip to Content
      </Button>
      {/* The window’s buttons, where they stay as the sidebar slides under them, clear of a title bar lying over the page (`--titlebar-area-x`). */}
      <div className="pointer-events-none absolute top-0 left-[calc(var(--titlebar-area-x,0px)+0.75rem)] z-20 flex h-(--shell-bar) items-center gap-1">
        <SidebarTrigger className="pointer-events-auto" />
      </div>
      <Sidebar>
        <div className="h-(--shell-bar) shrink-0" />
        <SidebarContent role="navigation" aria-label="Pages" className="[scrollbar-width:none]">
          <div className="relative pb-2">
            <Selected sections={sections} />
            {sections.map((section) => <ShellGroup key={section.key} section={section} onGo={onGo} onPress={onPress} onSection={onSection} />)}
          </div>
        </SidebarContent>
      </Sidebar>
      {/* Where the bar starts: past the window’s buttons while the sidebar’s away, as the sidebar goes. */}
      <SidebarInset data-banner={banner ? "" : undefined} className="min-w-0 overflow-hidden [--shell-bar-start:calc(var(--titlebar-area-x,0px)+3.25rem)] peer-data-[state=expanded]:[--shell-bar-start:0.75rem]">
        <BarStart back={back} onChange={onBarStart} />
        {/* Over the screen, whose large title lies in this row until it scrolls away and the title here takes over. Above the
            title bar’s drag region only for its buttons: the rest lets the drag through. */}
        <header data-slot="shell-bar" className={cn("pointer-events-none relative z-12 flex h-(--shell-bar) shrink-0 items-center gap-2 pr-3 pl-(--shell-bar-start) transition-[padding] duration-(--move-settle) ease-(--ease-settle) [&>button]:pointer-events-auto", title && "pointer-events-auto border-b bg-background wco:pointer-events-none wco:border-b-0")}>
          {back && (
            <Button variant="ghost" size="icon" className="size-7" onClick={onBack} aria-label="Back">
              <CaretLeftIcon />
            </Button>
          )}
          {title && <h1 className="min-w-0 truncate text-sm font-semibold">{title}</h1>}
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

/// The highlight behind what's selected, a row or a section's name: one element that slides between them as `sections`
/// change, and keeps to what it's behind as a section above opens or collapses.
function Selected({ sections }: { sections: ShellSection[] }) {
  const ref = React.useRef<HTMLSpanElement>(null)
  const scrolledTo = React.useRef<Element | null>(null)
  const place = React.useCallback(() => {
    const el = ref.current!, list = el.parentElement!
    const row = list.querySelector<HTMLElement>('[data-active="true"]')
    el.style.visibility = row ? "" : "hidden"
    if (!row) return
    const box = list.getBoundingClientRect(), at = row.getBoundingClientRect()
    el.style.width = `${at.width}px`
    el.style.height = `${at.height}px`
    el.style.translate = `${at.left - box.left}px ${at.top - box.top}px`
    if (!el.dataset.placed) requestAnimationFrame(() => (el.dataset.placed = ""))
    if (row !== scrolledTo.current) row.scrollIntoView({ block: "nearest" })
    scrolledTo.current = row
  }, [])
  React.useLayoutEffect(place, [sections, place])
  React.useLayoutEffect(() => {
    const el = ref.current!
    const follow = new ResizeObserver(() => {
      const sliding = el.getAnimations().length > 0
      if (!sliding) el.style.transition = "none"
      place()
      if (!sliding) {
        el.getBoundingClientRect()
        el.style.transition = ""
      }
    })
    follow.observe(el.parentElement!)
    return () => follow.disconnect()
  }, [place])
  return <span ref={ref} aria-hidden="true" className="absolute top-0 left-0 rounded-md bg-sidebar-accent data-[placed]:transition-[translate] data-[placed]:duration-(--move-settle) data-[placed]:ease-(--ease-settle) motion-reduce:transition-none" />
}

/// What a row or a section's name does when pressed: a plain click goes in place and closes the sidebar's sheet, any other
/// is the browser's (a new tab), as is all of a link to another site.
function useRowLink(row: Pick<ShellItem, "href" | "external">, onGo?: (href: string) => void, onPress?: (href: string) => void) {
  const { isMobile, setOpenMobile } = useSidebar()
  if (row.external) return { href: row.href, target: "_blank", rel: "noopener" }
  const plain = (e: React.MouseEvent) => !e.button && !e.metaKey && !e.ctrlKey && !e.shiftKey
  return {
    href: row.href,
    onClick: (e: React.MouseEvent) => {
      if (!onGo || !plain(e)) return
      e.preventDefault()
      onGo(row.href)
      if (isMobile) setOpenMobile(false)
    },
    onPointerDown: (e: React.PointerEvent) => plain(e) && onPress?.(row.href),
  }
}

const redBadge = "rounded-full bg-destructive text-white peer-data-[active=true]/menu-button:text-white"

/// A section: its name in quiet type, going to its page all across the row; beside the name the caret that collapses what's
/// under it, there on a hover, where nothing can hover, or while it's collapsed.
function ShellGroup({ section, onGo, onPress, onSection }: { section: ShellSection; onGo?: (href: string) => void; onPress?: (href: string) => void; onSection?: (key: string, open: boolean) => void }) {
  const link = useRowLink(section, onGo, onPress)
  return (
    <SidebarGroup className="py-1.5">
      <Collapsible.Root open={section.open} onOpenChange={(open) => onSection?.(section.key, open)}>
        <SidebarGroupLabel data-active={section.selected} className={cn("group/section relative gap-0.5 hover:text-sidebar-foreground data-[active=true]:text-sidebar-foreground", !section.open && !!section.badge && "pr-7")}>
          <a {...link} className="min-w-0 truncate rounded-sm ring-sidebar-ring outline-hidden after:absolute after:inset-0 focus-visible:ring-2">{section.title}</a>
          {section.items.length > 0 && (
            <Collapsible.Trigger aria-label={section.title} className="group/caret relative grid size-5 shrink-0 place-items-center rounded-sm opacity-0 ring-sidebar-ring outline-hidden group-hover/section:opacity-100 focus-visible:opacity-100 focus-visible:ring-2 data-[state=closed]:opacity-100 [@media(hover:none)]:opacity-100">
              <CaretRightIcon className="size-3 transition-transform duration-(--move-quick) ease-(--ease-quick) group-data-[state=open]/caret:rotate-90" />
            </Collapsible.Trigger>
          )}
          {!section.open && !!section.badge && <SidebarMenuBadge className={cn(redBadge, "top-1.5")}>{badgeCount(section.badge)}</SidebarMenuBadge>}
        </SidebarGroupLabel>
        {/* Its padding, taken back by its margin, is room for a row's focus ring inside what clips it as it slides. */}
        <Collapsible.Content className="-m-1 overflow-hidden p-1 duration-(--move-quick) ease-(--ease-quick) data-[state=closed]:animate-collapsible-up data-[state=open]:animate-collapsible-down">
          <SidebarMenu>
            {section.items.map((item) => <ShellRow key={item.key} row={item} onGo={onGo} onPress={onPress} />)}
          </SidebarMenu>
        </Collapsible.Content>
      </Collapsible.Root>
    </SidebarGroup>
  )
}

function ShellRow({ row, onGo, onPress }: { row: ShellItem; onGo?: (href: string) => void; onPress?: (href: string) => void }) {
  const link = useRowLink(row, onGo, onPress)
  return (
    <SidebarMenuItem>
      <SidebarMenuButton asChild isActive={row.selected} className="data-[active=true]:bg-transparent">
        <a {...link}>
          {row.folder ? <Folder apps={row.folder} /> : row.app && <AppIcon app={row.app} size={20} />}
          <span className="truncate">{row.title}</span>
          {row.external && <ArrowUpRightIcon className="ml-auto text-muted-foreground" />}
        </a>
      </SidebarMenuButton>
      {!!row.badge && <SidebarMenuBadge className={redBadge}>{badgeCount(row.badge)}</SidebarMenuBadge>}
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

export { Shell, Folder, type ShellItem, type ShellSection }
