// The Home Screen: yours, each of your groups' and the network's, side by side, then Settings. Each is a grid of
// the widgets put on it and its apps' icons. Swiping moves between them, and so does sliding along the dots.
// Touch and hold, or right-click, for what an app or a widget offers; hold and move to put it somewhere else.
// Where the window is wide, a sidebar lists every page's apps and Settings' beside them, and an app opens beside it.
// There it is how you pick a page, so the page's name and the dots, which are for where there is none, aren't shown.
import { createEffect, createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js"
import { render, type JSX } from "@solidjs/web"
import { call, changed, confirm, Empty, Icon, List, open, Problem, Row, Section, Sheet, toast, Widget } from "ui"
import "./home.css"
import type { Account, App, Item, Page, PluginWithAccounts, Size, Widget as WidgetItem } from "./types.ts"

const SIZES: Record<Size, string> = { "2x2": "Small", "4x2": "Wide", "4x4": "Large" }
const HOME = "home-screen/client/index.html"
const ACCOUNTS = "home-screen/client/accounts.html"
const json = <T,>(tool: string, inputs?: object, who?: string): Promise<T> => call(tool, inputs, { subject: who }).then(JSON.parse)

/// Every page this user sees, and the parts of Settings. A network with nothing that keeps groups has none.
const home = createMemo(async () => {
  const groups = await json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => [])
  // Each page is named for whose it is: a group by what it's called.
  const whos: { subject?: string; name: string }[] = [{ name: "You" }, ...groups.map(({ group, called }) => ({ subject: group, name: called })), { subject: "network", name: "Everyone" }]
  const [settings, pages] = await Promise.all([json<{ parts: App[] }>("home-screen_settings").then(({ parts }) => parts), Promise.all(whos.map((who) => json<Page>("home-screen_page", {}, who.subject).then((page): Page => ({ ...page, ...who }))))])
  // Every page's plugins with accounts, each once, are in the one Accounts folder.
  const withAccounts = [...new Map(pages.flatMap((page) => page.plugins_with_accounts.map((plugin): [string, PluginWithAccounts] => [plugin.plugin, { ...plugin, subject: page.subject }]))).values()]
  return { pages, settings, withAccounts }
})
changed(() => refresh(home))

/// An item of a page as the page is arranged with it.
const arrangement = (item: Item) => (item.widget ? { widget: item.widget, size: item.size } : { app: item.app })
const same = (a: Pick<Item, "widget" | "app" | "size">, b: Pick<Item, "widget" | "app" | "size">) => (a.widget ?? a.app) === (b.widget ?? b.app) && a.size === b.size

/// Arranges a page as `items`, and says why if it couldn't be.
async function arrange(page: Page, items: Item[]) {
  try {
    await call("home-screen_arrange", { items: items.map(arrangement) }, { subject: page.subject })
  } catch (error) {
    toast((error as Error).message)
  }
  refresh(home)
}

/// What's being held: `{ page, item }`, once a finger has stayed on it, and whether it has moved since.
const [menu, setMenu] = createSignal<{ page: Page; item?: Item }>()
const [adding, setAdding] = createSignal<Page>()
let quiet = 0

/// Touch and hold, or right-click, opens what `item` offers (with none, what its page does), and holding and
/// moving puts it where it's let go. A mouse drags at once.
function held(page: () => Page, item?: () => Item) {
  return (element: HTMLElement) => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let holding: boolean | undefined
    let start: PointerEvent | undefined
    let moved: boolean | undefined
    const over = (event: PointerEvent) => (document.elementsFromPoint(event.clientX, event.clientY) as HTMLElement[]).find((under) => under !== element && under.dataset?.item && under.parentElement === element.parentElement)
    const drop = () => {
      clearTimeout(timer)
      element.style.translate = ""
      element.classList.remove("lifted")
      for (const target of element.parentElement?.querySelectorAll(".target") ?? []) target.classList.remove("target")
      holding = start = undefined
    }
    element.addEventListener("contextmenu", (event) => {
      event.preventDefault()
      event.stopPropagation()
      if (!moved && page().arranges) setMenu({ page: page(), item: item?.() })
    })
    if (!item) return
    element.addEventListener("pointerdown", (event) => {
      if (event.button || !page().arranges) return
      start = event
      moved = false
      holding = event.pointerType === "mouse"
      if (!holding) timer = setTimeout(() => ((holding = true), element.classList.add("lifted")), 400)
    })
    element.addEventListener("pointermove", (event) => {
      if (!start) return
      const [x, y] = [event.clientX - start.clientX, event.clientY - start.clientY]
      if (!holding) return void (Math.hypot(x, y) > 10 && drop())
      if (!moved && Math.hypot(x, y) < 8) return
      moved = true
      element.setPointerCapture(event.pointerId)
      element.classList.add("lifted")
      element.style.translate = `${x}px ${y}px`
      for (const target of element.parentElement!.querySelectorAll(".target")) target.classList.remove("target")
      over(event)?.classList.add("target")
    })
    element.addEventListener("pointerup", (event) => {
      const [was, target] = [holding && !moved && start?.pointerType !== "mouse", moved && over(event)]
      if (moved || was) quiet = Date.now() + 400
      drop()
      if (was) return void setMenu({ page: page(), item: item() })
      if (!target) return
      const items = page().items.filter((other) => other !== item())
      items.splice(items.findIndex((other) => (other.widget ?? other.app) + (other.size ?? "") === target.dataset.item), 0, item())
      arrange(page(), items)
    })
    element.addEventListener("pointercancel", drop)
    // A web view drags an icon as a picture of itself, and cancels the pointer, unless it's told not to.
    element.addEventListener("dragstart", (event) => event.preventDefault())
    // A finger that's holding something moves it, not the page.
    element.addEventListener("touchmove", (event) => holding && event.preventDefault(), { passive: false })
  }
}

/// A widget on a page: what its plugin's tool answers, drawn, and asked again each minute it's in view.
function Card(props: { item: WidgetItem; page: Page }) {
  const tool = () => props.item.widget.replace("/", "_")
  const layout = createMemo(() => json<NonNullable<Parameters<typeof Widget>[0]["layout"]>>(tool(), { size: props.item.size }, props.page.subject))
  onSettled(() => {
    const stop = changed(() => refresh(layout))
    const every = setInterval(() => document.visibilityState === "visible" && refresh(layout), 60_000)
    return () => (stop(), clearInterval(every))
  })
  const act = async (name: string | undefined, inputs: Record<string, unknown>) => {
    try {
      await call(`${props.item.widget.split("/")[0]}_${name}`, inputs, { subject: props.page.subject })
    } catch (error) {
      toast((error as Error).message)
    }
    refresh(layout)
  }
  const opens = () => Date.now() > quiet && props.item.opens && open(props.item.opens, { subject: props.page.subject })
  return (
    <Errored fallback={(error) => <Widget name={props.item.name} size={props.item.size} problem={String((error() as Error).message ?? error())} />}>
      <Loading fallback={<Widget name={props.item.name} size={props.item.size} />}>
        <Widget name={props.item.name} size={props.item.size} layout={layout()} onCall={act} onOpen={props.item.opens ? opens : undefined} />
      </Loading>
    </Errored>
  )
}

/// What the window shows, as it says (`notifications/one/showing`): an app and who it's for, or this.
const [showing, setShowing] = createSignal<{ app: string; subject?: string | null }>({ app: HOME })
addEventListener("message", ({ source, data }) => source === parent && data?.method === "notifications/one/showing" && setShowing(data.params))

/// Whether the window is wide and tall enough for the sidebar. The client keeps an app beside it, as wide as this
/// says (`notifications/one/beside`) whenever that changes.
const beside = matchMedia("(min-width: 48rem) and (min-height: 30rem)")
const [sidebar, setSidebar] = createSignal(beside.matches)
const room = () => parent.postMessage({ jsonrpc: "2.0", method: "notifications/one/beside", params: beside.matches ? { width: getComputedStyle(document.documentElement).getPropertyValue("--sidebar-width").trim() } : {} }, "*")
beside.addEventListener("change", () => (setSidebar(beside.matches), room()))

/// The sidebar's sections its user collapsed, kept on this device.
const [collapsed, setCollapsed] = createSignal(new Set<string>(stored("collapsed") ?? []))
function stored(name: string): string[] | undefined {
  try {
    return JSON.parse(String(localStorage.getItem(name)))
  } catch {
    return undefined
  }
}
function collapse(section: string, shut: boolean) {
  const now = new Set(collapsed())
  if (shut) now.add(section)
  else now.delete(section)
  setCollapsed(now)
  try {
    localStorage.setItem("collapsed", JSON.stringify([...now]))
  } catch {}
}

/// Every page's apps, then Settings', each a section that collapses. A section's name shows its page; an app opens
/// beside the sidebar. Arrow keys move through it, and left and right on a section's name collapse and open it.
/** A section of the sidebar: a page's apps, then the first page's Accounts folder, or Settings'. */
interface Section {
  key: string
  name: string
  apps: App[]
  withAccounts?: PluginWithAccounts[]
}

function Sidebar(props: { sections: Section[]; at: () => number; go: (index: number) => void }) {
  let nav!: HTMLElement
  const home = () => showing().app === HOME
  const inAccounts = () => showing().app === ACCOUNTS
  const current = (app: App) => !home() && (showing().subject ?? null) === (app.subject ?? null) && showing().app.split("#")[0] === app.app
  // A section's name is selected while its page shows, or while it's collapsed over what's open.
  const selected = (section: Section, index: number) => (home() ? props.at() === index : collapsed().has(section.key) && (section.apps.some(current) || (!!section.withAccounts && inAccounts())))
  const pick = (index: number) => {
    props.go(index)
    if (!home()) open(HOME)
  }
  const keys = (event: KeyboardEvent & { target: Element }) => {
    const buttons = [...nav.querySelectorAll<HTMLElement>("button:not([tabindex='-1'])")]
    const at = buttons.indexOf(document.activeElement as HTMLElement)
    const to = { ArrowDown: at + 1, ArrowUp: at - 1, Home: 0, End: buttons.length - 1 }[event.key]
    if (to !== undefined) {
      event.preventDefault()
      buttons[Math.max(0, Math.min(buttons.length - 1, to))]?.focus()
    }
    if (event.target.matches(".name") && ["ArrowLeft", "ArrowRight"].includes(event.key)) {
      event.preventDefault()
      collapse(event.target.closest<HTMLElement>("[data-section]")!.dataset.section!, event.key === (document.dir === "rtl" ? "ArrowRight" : "ArrowLeft"))
    }
  }
  // What's selected stays in view. What an effect returns is its cleanup, and Chrome's `scrollIntoView` returns a
  // promise, so this returns nothing.
  createEffect(() => [showing(), props.at(), collapsed()], () => {
    nav.querySelector("[aria-current]")?.scrollIntoView({ block: "nearest" })
  })
  return (
    <nav class="sidebar" ref={nav} aria-label="Apps" onKeyDown={keys}>
      <For each={props.sections} keyed={(section) => section.key}>
        {(section, index) => (
          <div class="section" data-section={section().key}>
            <div class="label">
              <button type="button" class="name" aria-current={selected(section(), index()) ? "page" : undefined} onClick={() => pick(index())}>
                {section().name}
              </button>
              <button type="button" class="caret" tabindex="-1" aria-label={section().name} aria-expanded={collapsed().has(section().key) ? "false" : "true"} onClick={() => collapse(section().key, !collapsed().has(section().key))}>
                <Icon name="caret-right" along />
              </button>
            </div>
            <Show when={!collapsed().has(section().key)}>
              <List>
                <For each={section().apps} keyed={(app) => app.app}>
                  {(app) => <Row start={<img src={app().icon} alt="" />} heading={app().name} current={current(app())} onClick={() => open(app().app, { subject: app().subject })} />}
                </For>
                <Show when={section().withAccounts}>
                  {(plugins) => <Row start={<Folder plugins={plugins()} />} heading="Accounts" current={inAccounts()} onClick={() => open(ACCOUNTS)} />}
                </Show>
              </List>
            </Show>
          </div>
        )}
      </For>
    </nav>
  )
}

/// An icon with its name under it, an app's or the Accounts folder's, and the page it opens. It starts as a finger
/// lands on it, and shows as the finger lifts.
function Tile(props: { opens: string; name: string; subject?: string; children: JSX.Element }) {
  return (
    <button type="button" class="tile" onPointerDown={() => open(props.opens, { subject: props.subject, behind: true })} onClick={() => Date.now() > quiet && open(props.opens, { subject: props.subject })}>
      <span class="icon">{props.children}</span>
      <span>{props.name}</span>
    </button>
  )
}

/// An app's tile.
const AppTile = (props: { app: App; subject?: string }) => (
  <Tile opens={props.app.app} name={props.app.name} subject={props.subject}>
    <img src={props.app.icon} alt="" />
  </Tile>
)

/// A folder, as a phone draws one: a frosted rounded square holding the icons of up to nine of its plugins, small,
/// in a 3×3 grid from the top left.
function Folder(props: { plugins: PluginWithAccounts[] }) {
  return (
    <span class="folder">
      <For each={props.plugins.slice(0, 9)} keyed={(plugin) => plugin.plugin}>
        {(plugin) => <img src={plugin().icon} alt="" />}
      </For>
    </span>
  )
}

/// The Accounts folder, last on the first page and not moved: the icons of the plugins with accounts, which aren't
/// apps, and a dot while an account needs its person.
function AccountsTile(props: { plugins: PluginWithAccounts[] }) {
  return (
    <Tile opens={ACCOUNTS} name="Accounts">
      <Folder plugins={props.plugins} />
      <Loading>
        <Attention plugins={props.plugins} />
      </Loading>
    </Tile>
  )
}

/// A dot while any account of the folder's plugins is not ok, as each plugin's `accounts` answers.
function Attention(props: { plugins: PluginWithAccounts[] }) {
  const needed = createMemo(async () => {
    const lists = await Promise.all(props.plugins.map((plugin) => json<Account[]>(`${plugin.name}_accounts`, {}, plugin.subject).catch((): Account[] => [])))
    return lists.flat().some((account) => !account.ok)
  })
  onSettled(() => changed(() => refresh(needed)))
  return (
    <Show when={needed()}>
      <i class="attention" aria-label="An account needs you" />
    </Show>
  )
}

function PageGrid(props: { page: Page; withAccounts?: PluginWithAccounts[] }) {
  return (
    <section class="page" ref={held(() => props.page)}>
      <Show when={props.page.items.length || props.withAccounts} fallback={<Empty>No apps yet.</Empty>}>
        <div class="grid">
          <For each={props.page.items} keyed={(item) => (item.widget ?? item.app) + (item.size ?? "")}>
            {(item) => (
              <div class="cell" data-item={(item().widget ?? item().app) + (item().size ?? "")} data-size={item().size} ref={held(() => props.page, item)}>
                <Show when={item().widget} fallback={<AppTile app={item() as App} subject={props.page.subject} />}>
                  <Card item={item() as WidgetItem} page={props.page} />
                </Show>
              </div>
            )}
          </For>
          <Show when={props.withAccounts}>
            <div class="cell">
              <AccountsTile plugins={props.withAccounts!} />
            </div>
          </Show>
        </div>
      </Show>
    </section>
  )
}

/// What a held app or widget offers, or with nothing held, its page: a widget's sizes, taking it off, and the
/// widgets that could be put on.
function Offers() {
  const page = () => menu()!.page
  const item = () => menu()?.item
  const others = () => page().items.filter((other) => other !== item())
  const done = (does: () => unknown) => () => (setMenu(), does())
  const off = done(async () => {
    const plugin = item()!.app!.split("/")[0]
    if (!(await confirm({ heading: `Remove ${item()!.name} from ${page().name}?`, text: "What it kept stays, and comes back with it.", action: "Remove", destructive: true }))) return
    await call("uninstall", { name: plugin }, { subject: page().subject }).catch((error) => toast(error.message))
    refresh(home)
  })
  return (
    <Sheet heading={item()?.name ?? menu()?.page.name} open={!!menu()} onClose={() => setMenu()}>
      <Show when={menu()}>
        <List>
          <Show when={item()?.widget}>
            <For each={(item() as WidgetItem).sizes}>
              {(size) => <Row heading={SIZES[size]} end={size === item()!.size ? <Icon name="check" /> : undefined} onClick={done(() => arrange(page(), page().items.map((other): Item => (other === item() ? ({ ...other, size } as Item) : other))))} />}
            </For>
            <Row heading="Remove Widget" onClick={done(() => arrange(page(), others()))} />
          </Show>
          <Show when={item()?.app}>
            <Row heading={`Remove from ${page().name}`} onClick={off} />
          </Show>
          <Show when={page().widgets.length}>
            <Row heading="Add Widget" onClick={done(() => setAdding(page()))} />
          </Show>
        </List>
      </Show>
    </Sheet>
  )
}

/// The widgets a page's plugins have, each in the sizes it comes in.
function Adding() {
  const add = (widget: WidgetItem, size: Size) => {
    const page = adding()!
    setAdding()
    if (!page.items.some((item) => same(item, { ...widget, size }))) arrange(page, [{ ...widget, size }, ...page.items])
  }
  return (
    <Sheet heading="Add Widget" open={!!adding()} onClose={() => setAdding()}>
      <Show when={adding()}>
        <For each={adding()!.widgets}>
          {(widget) => (
            <Section heading={widget.name}>
              <List>
                <For each={widget.sizes}>{(size) => <Row heading={SIZES[size]} onClick={() => add(widget, size)} />}</For>
              </List>
            </Section>
          )}
        </For>
      </Show>
    </Sheet>
  )
}

function Home(props: { home: { pages: Page[]; settings: App[]; withAccounts: PluginWithAccounts[] } }) {
  let strip!: HTMLDivElement
  const [at, setAt] = createSignal(0)
  const names = () => [...props.home.pages.map((page) => page.name), "Settings"]
  const go = (to: number, smooth = true) => strip.scrollTo({ left: Math.max(0, Math.min(names().length - 1, to)) * strip.clientWidth * (document.dir === "rtl" ? -1 : 1), behavior: smooth ? "smooth" : "instant" })

  onSettled(() => {
    room()
    const keys = (event: KeyboardEvent) => {
      if (sidebar() || (event.target as Element).closest?.("input, textarea, dialog")) return
      if (event.key === "ArrowRight") go(at() + 1)
      else if (event.key === "ArrowLeft") go(at() - 1)
    }
    addEventListener("keydown", keys)
    return () => removeEventListener("keydown", keys)
  })

  /// A finger sliding along the dots changes page, one for every 28 points, as a phone's page dots do.
  const slide = (down: PointerEvent & { currentTarget: HTMLElement }) => {
    const [capsule, from] = [down.currentTarget, at()]
    let sliding = false
    const move = (event: PointerEvent) => {
      const across = event.clientX - down.clientX
      if (!sliding && Math.abs(across) < 10) return
      sliding = true
      capsule.setPointerCapture(down.pointerId)
      go(from + Math.trunc(across / 28), false)
    }
    const up = () => {
      capsule.removeEventListener("pointermove", move)
      if (sliding) quiet = Date.now() + 300
    }
    capsule.addEventListener("pointermove", move)
    capsule.addEventListener("pointerup", up, { once: true })
    capsule.addEventListener("pointercancel", up, { once: true })
  }
  const press = (does: () => void) => () => Date.now() > quiet && does()
  const sections = (): Section[] => [
    ...props.home.pages.map((page, index) => ({
      key: page.subject ?? "you",
      name: page.name,
      apps: page.items.filter((item): item is Extract<Item, App> => !!item.app).map((item) => ({ ...item, subject: page.subject })),
      withAccounts: index ? undefined : props.home.withAccounts,
    })),
    { key: "settings", name: "Settings", apps: props.home.settings },
  ]

  return (
    <div class={["home", { beside: sidebar() }]}>
      <Show when={sidebar()}>
        <Sidebar sections={sections()} at={at} go={(index) => go(index, false)} />
      </Show>
      <main class="pages">
        <div class="strip" ref={strip} onScroll={() => setAt(Math.round(Math.abs(strip.scrollLeft) / strip.clientWidth))}>
          <For each={props.home.pages} keyed={(page) => page.who}>
            {(page, index) => <PageGrid page={page()} withAccounts={index() ? undefined : props.home.withAccounts} />}
          </For>
          <section class="page">
            <div class="grid">
              <For each={props.home.settings} keyed={(part) => part.app}>
                {(part) => <div class="cell"><AppTile app={part()} /></div>}
              </For>
            </div>
          </section>
        </div>
        <Show when={!sidebar()}>
          <footer>
            <p>{names()[at()]}</p>
            <nav class="dots" onPointerDown={slide} aria-label="Pages">
              <For each={props.home.pages} keyed={(page) => page.who}>
                {(page, index) => <button type="button" class="dot" aria-label={page().name} aria-current={at() === index() ? "page" : undefined} onClick={press(() => go(index()))}><i /></button>}
              </For>
              <button type="button" aria-label="Settings" aria-current={at() === names().length - 1 ? "page" : undefined} onClick={press(() => go(names().length - 1))}><Icon name="gear-six" /></button>
            </nav>
          </footer>
        </Show>
      </main>
      <Offers />
      <Adding />
    </div>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your Home Screen pages" text={String((error() as Error).message ?? error())} />}>
      <Loading>
        <Home home={home()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
