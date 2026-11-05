// The gallery: every component and layout part of the kit, to see at any width. Built as an app's screen would be: it
// imports React and the kit by the names the page's import map gives, and its own Tailwind utilities (gallery.css).
import { useState } from "react"
import { createRoot } from "react-dom/client"
import {
  AlertDialog, AlertDialogAction, AlertDialogCancel, AlertDialogContent, AlertDialogDescription, AlertDialogFooter,
  AlertDialogHeader, AlertDialogTitle, AlertDialogTrigger, AppIcon, Badge, Button, Card, CardContent, CardDescription,
  CardFooter, CardHeader, CardTitle, Dialog, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
  DialogTrigger, DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuLabel, DropdownMenuSeparator,
  DropdownMenuTrigger, Empty, Grid, Icon, Input, List, ListRow, Problem, Separator, Sheet, SheetContent,
  SheetDescription, SheetHeader, SheetTitle, SheetTrigger, Sidebar, SidebarContent, SidebarGroup, SidebarGroupLabel,
  SidebarMenu, SidebarMenuBadge, SidebarMenuButton, SidebarMenuItem, SidebarProvider, Skeleton, Split, Switch, Tabs,
  TabsContent, TabsList, TabsTrigger,
} from "@one/ui"
import "./gallery.css"
import { DotsThreeIcon, MagnifyingGlassIcon, PlusIcon } from "@phosphor-icons/react"

const apps = [
  { name: "agent-devices", title: "Agent Devices", icon: "browsers", color: "#a2845e" },
  { name: "board", title: "Agent Board", icon: "chat-text", color: "#ff9500" },
  { name: "device-activity", title: "Device Activity", icon: "monitor", color: "#30b0c7" },
  { name: "repos", title: "Repos", icon: "git-branch", color: "#5856d6" },
  { name: "computer", title: "Computer Use", icon: "cursor-click", color: "#5e5ce6" },
  { name: "groups", title: "Groups", icon: "users-three", color: "#34c759" },
  { name: "connect-ai", title: "Connect Your AI", icon: "plugs-connected", color: "#ff2d55" },
  { name: "backups", title: "Backups", icon: "hard-drives", color: "#ff9500" },
  { name: "labs", title: "Labs", icon: "flask", color: "#af52de" },
  { name: "devices", title: "Devices", icon: "devices", color: "#8e8e93" },
]

const items = [
  { id: "anthropic", title: "Anthropic", sub: "Key · Used by AI and Models", app: { name: "ai", icon: "sparkle", color: "#d97757" } },
  { id: "github", title: "GitHub", sub: "Account · Used by Repos", app: { name: "github", icon: "github-logo", color: "#24292f" } },
  { id: "work", title: "andre@work", sub: "Google account · Used by Gmail and Drive", app: { name: "google", icon: "google-logo", color: "#4285f4" } },
  { id: "wifi", title: "Office Wi-Fi", sub: "network: Office-5G", app: { name: "wifi", icon: "wifi-high", color: "#8e8e93" } },
]

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="flex flex-col gap-3">
      <h2 className="text-sm font-medium text-muted-foreground">{title}</h2>
      {children}
    </section>
  )
}

function Row({ children }: { children: React.ReactNode }) {
  return <div className="flex flex-wrap items-center gap-2">{children}</div>
}

function Vault() {
  const [open, setOpen] = useState<string | null>(null)
  const item = items.find((i) => i.id === open)
  return (
    <Split
      title={item?.title}
      onClose={() => setOpen(null)}
      list={
        <List>
          {items.map((i) => (
            <ListRow key={i.id} leading={<AppIcon app={i.app} size={32} />} title={i.title} sub={i.sub} selected={i.id === open} onClick={() => setOpen(i.id)} />
          ))}
        </List>
      }
      detail={item && (
        <Card>
          <CardHeader className="flex flex-row items-center gap-4">
            <AppIcon app={item.app} size={56} />
            <div className="min-w-0 flex-1">
              <CardTitle className="text-lg">{item.title}</CardTitle>
              <CardDescription>You</CardDescription>
            </div>
            <Button variant="outline" size="sm">Rename</Button>
          </CardHeader>
          <CardContent className="flex flex-col gap-3">
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm">AI</span>
              <Switch defaultChecked />
            </div>
            <div className="flex items-center justify-between gap-3">
              <span className="text-sm">AGENTS.md</span>
              <Switch />
            </div>
          </CardContent>
          <CardFooter>
            <Button variant="destructive" size="sm">Delete Item</Button>
          </CardFooter>
        </Card>
      )}
    />
  )
}

function Gallery() {
  return (
    <main className="mx-auto flex flex-col gap-10">
      <h1 className="text-2xl font-semibold tracking-tight">UI kit</h1>

      <Section title="Button: one default per view, then secondary, outline, ghost; destructive only for what can't be undone">
        <Row>
          <Button>Default</Button>
          <Button variant="secondary">Secondary</Button>
          <Button variant="outline">Outline</Button>
          <Button variant="ghost">Ghost</Button>
          <Button variant="destructive">Destructive</Button>
          <Button variant="link">Link</Button>
        </Row>
        <Row>
          <Button size="sm" variant="outline"><PlusIcon /> New Item</Button>
          <Button size="lg">Large</Button>
          <Button size="icon" variant="ghost" aria-label="More"><DotsThreeIcon weight="bold" /></Button>
          <Button disabled>Disabled</Button>
        </Row>
      </Section>

      <Section title="Badge">
        <Row>
          <Badge>Default</Badge>
          <Badge variant="secondary">Secondary</Badge>
          <Badge variant="outline">Outline</Badge>
          <Badge variant="destructive" className="rounded-full">3</Badge>
        </Row>
      </Section>

      <Section title="Input">
        <div className="relative max-w-sm">
          <MagnifyingGlassIcon className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input placeholder="Search" className="pl-8" />
        </div>
      </Section>

      <Section title="Tabs">
        <Tabs defaultValue="you" className="max-w-md">
          <TabsList>
            <TabsTrigger value="you">You</TabsTrigger>
            <TabsTrigger value="team">Team</TabsTrigger>
            <TabsTrigger value="everyone">Everyone</TabsTrigger>
          </TabsList>
          <TabsContent value="you" className="text-sm text-muted-foreground">Yours alone.</TabsContent>
          <TabsContent value="team" className="text-sm text-muted-foreground">Your team's.</TabsContent>
          <TabsContent value="everyone" className="text-sm text-muted-foreground">Everyone's.</TabsContent>
        </Tabs>
      </Section>

      <Section title="Switch">
        <Row>
          <Switch defaultChecked aria-label="On" />
          <Switch aria-label="Off" />
          <Switch disabled aria-label="Disabled" />
        </Row>
      </Section>

      <Section title="Card">
        <Card className="max-w-sm">
          <CardHeader>
            <CardTitle>Backups</CardTitle>
            <CardDescription>Encrypted copies of everything.</CardDescription>
          </CardHeader>
          <CardContent className="text-sm">Last backed up 5 minutes ago.</CardContent>
          <CardFooter><Button variant="outline" size="sm">Back Up Now</Button></CardFooter>
        </Card>
      </Section>

      <Section title="Dropdown menu, dialog, alert dialog, sheet">
        <Row>
          <DropdownMenu>
            <DropdownMenuTrigger asChild><Button variant="outline">Menu</Button></DropdownMenuTrigger>
            <DropdownMenuContent>
              <DropdownMenuLabel>Agent Devices</DropdownMenuLabel>
              <DropdownMenuSeparator />
              <DropdownMenuItem>Open</DropdownMenuItem>
              <DropdownMenuItem>Rename</DropdownMenuItem>
              <DropdownMenuItem variant="destructive">Delete</DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
          <Dialog>
            <DialogTrigger asChild><Button variant="outline">Dialog</Button></DialogTrigger>
            <DialogContent>
              <DialogHeader>
                <DialogTitle>Rename</DialogTitle>
                <DialogDescription>What it's called on every device.</DialogDescription>
              </DialogHeader>
              <Input defaultValue="Anthropic" />
              <DialogFooter><Button>Save</Button></DialogFooter>
            </DialogContent>
          </Dialog>
          <AlertDialog>
            <AlertDialogTrigger asChild><Button variant="outline">Alert dialog</Button></AlertDialogTrigger>
            <AlertDialogContent>
              <AlertDialogHeader>
                <AlertDialogTitle>Delete Anthropic?</AlertDialogTitle>
                <AlertDialogDescription>Apps using it stop working until you add another.</AlertDialogDescription>
              </AlertDialogHeader>
              <AlertDialogFooter>
                <AlertDialogCancel>Cancel</AlertDialogCancel>
                <AlertDialogAction className="bg-destructive text-white hover:bg-destructive/90">Delete</AlertDialogAction>
              </AlertDialogFooter>
            </AlertDialogContent>
          </AlertDialog>
          <Sheet>
            <SheetTrigger asChild><Button variant="outline">Sheet</Button></SheetTrigger>
            <SheetContent>
              <SheetHeader>
                <SheetTitle>Anthropic</SheetTitle>
                <SheetDescription>Key · Used by AI and Models</SheetDescription>
              </SheetHeader>
            </SheetContent>
          </Sheet>
        </Row>
      </Section>

      <Section title="Separator and skeleton">
        <div className="flex max-w-sm flex-col gap-3">
          <Skeleton className="h-4 w-3/4" />
          <Skeleton className="h-4 w-1/2" />
          <Separator />
          <Skeleton className="aspect-[16/10] w-full rounded-xl" />
        </div>
      </Section>

      <Section title="Problem: on the thing it's about, with the action that fixes it">
        <Problem title="Couldn't reach your devices." text="Check your connection." action={{ label: "Try Again", run: () => {} }} />
        <Problem warn title="None of your devices runs Agent Devices." text="It needs a Mac, or a Linux device with Podman." action={{ label: "Open App Store", run: () => {} }} />
      </Section>

      <Section title="Empty: one grey line">
        <Card><Empty>None yet. Agents make one when they need a browser.</Empty></Card>
      </Section>

      <Section title="App icons, in each app's colour, and icons by name">
        <Row>
          {apps.map((a) => <AppIcon key={a.name} app={a} />)}
          <AppIcon app={{ name: "x", title: "Xylophone" }} />
        </Row>
        <Row>
          {["magnifying-glass", "gear", "warning-circle-fill", "check-circle-fill", "browsers"].map((n) => <Icon key={n} name={n} className="size-5" />)}
        </Row>
      </Section>

      <Section title="Grid: fills the room, at most 4 across here">
        <Grid min="12rem" max={4}>
          {apps.slice(0, 6).map((a) => (
            <div key={a.name} className="min-w-0">
              <div className="grid aspect-[16/10] place-items-center rounded-xl bg-muted"><AppIcon app={a} size={48} /></div>
              <div className="mt-2 truncate text-sm font-medium">{a.title}</div>
              <div className="truncate text-sm text-muted-foreground">personal</div>
            </div>
          ))}
        </Grid>
      </Section>

      <Section title="List: rows that stop at a readable width">
        <List>
          {apps.slice(0, 4).map((a) => <ListRow key={a.name} leading={<AppIcon app={a} size={28} />} title={a.title} sub="On every device" trailing={a.name === "devices" ? <Badge variant="destructive" className="rounded-full">1</Badge> : undefined} />)}
        </List>
      </Section>

      <Section title="Split: what a list opens shows beside it where there's room, else in a sheet">
        <Vault />
      </Section>

      <Section title="Sidebar pieces">
        <div className="h-[28rem] w-64 overflow-hidden rounded-xl border">
          <SidebarProvider className="min-h-0">
            <Sidebar collapsible="none" className="w-full">
              <SidebarContent>
                <SidebarGroup>
                  <SidebarGroupLabel>You</SidebarGroupLabel>
                  <SidebarMenu>
                    {apps.slice(0, 2).map((a, i) => (
                      <SidebarMenuItem key={a.name}>
                        <SidebarMenuButton isActive={i === 0}><AppIcon app={a} size={20} />{a.title}</SidebarMenuButton>
                      </SidebarMenuItem>
                    ))}
                  </SidebarMenu>
                </SidebarGroup>
                <SidebarGroup>
                  <SidebarGroupLabel>Settings</SidebarGroupLabel>
                  <SidebarMenu>
                    {apps.slice(5).map((a) => (
                      <SidebarMenuItem key={a.name}>
                        <SidebarMenuButton><AppIcon app={a} size={20} />{a.title}</SidebarMenuButton>
                        {a.name === "devices" && <SidebarMenuBadge className="rounded-full bg-destructive text-white">1</SidebarMenuBadge>}
                      </SidebarMenuItem>
                    ))}
                  </SidebarMenu>
                </SidebarGroup>
              </SidebarContent>
            </Sidebar>
          </SidebarProvider>
        </div>
      </Section>
    </main>
  )
}

createRoot(document.getElementById("root")!).render(<Gallery />)
