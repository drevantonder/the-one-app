// The Home Screen: yours, everyone's, then each of your groups', one owner above another, each a row of pages and then
// their App Library. Left and right moves between an owner's pages, as a phone's do, and up and down between owners,
// unless a page scrolls itself, as the App Library does. The owner's name at the top opens a list of every owner. A
// page never scrolls: it's a fixed grid, and what doesn't fit goes on to the next. A wide window shows as many whole
// pages of an owner's side by side as fit, one to three. Where it is and what's open over it are one state machine
// (machine.ts), whose snapshot is one signal here.
import { createEffect, createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js"
import { render, type JSX } from "@solidjs/web"
import { createActor } from "xstate"
import { call, changed, covers, Empty, Icon, Menu, open, Problem, Widget } from "ui"
import "./home.css"
import { type At, kinds, type Kind, machine, type Opened } from "./machine.ts"
import type { Account, App, Category, Item, Pages, PluginWithAccounts, Widget as WidgetItem } from "./types.ts"

const HOME = "home-screen/client/index.html"
const ACCOUNTS = "home-screen/client/accounts.html"
const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

/** An owner: whose Home Screen it is, by the name a person reads, and what `home-screen_pages` answers for them. */
interface Owner extends Pages {
  key: string
  subject?: string
  name: string
}

/// You, everyone, then each of your groups, in the order they're listed. A network with nothing that keeps groups
/// has none.
const owners = createMemo(async () => {
  const groups = await json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => [])
  const whose: { subject?: string; name: string }[] = [{ name: "You" }, { subject: "network", name: "Everyone" }, ...groups.map(({ group, called }) => ({ subject: group, name: called }))]
  return Promise.all(whose.map((who) => json<Pages>("home-screen_pages", {}, who.subject).then((pages): Owner => ({ ...pages, ...who, key: who.subject ?? "you" }))))
})
changed(() => refresh(owners))

/// The one machine, and its snapshot as one signal.
const home = createActor(machine)
const [state, setState] = createSignal(home.getSnapshot())
home.subscribe(setState)
home.start()

/// The window says what it shows (`notifications/one/showing`): an app over the Home Screen, or the Home Screen.
addEventListener("message", ({ source, data }) => {
  if (source === parent && data?.method === "notifications/one/showing") home.send({ type: data.params?.app === HOME ? "home" : "launch" })
})

const still = matchMedia("(prefers-reduced-motion: reduce)")
const rtl = () => (document.dir === "rtl" ? -1 : 1)

/// Opens an app for whoever its owner is, or for its person where it's about whoever opens it. `behind` only starts
/// it, as a finger lands on it.
const launch = (app: App, owner: Owner, behind?: boolean) => open(app.app, { subject: app.subject === "user" ? undefined : owner.subject, behind })

// A page's grid: four columns, and as many rows of 5.5rem as its height has room for, up to six, as a phone's.
const COLUMNS = 4
const SPANS: Record<string, [number, number]> = { "2x2": [2, 2], "4x2": [4, 2], "4x4": [4, 4] }

/** Something on a Home page where it sits: its column and row, from 1, and how many of each it spans. */
interface Placed {
  item: Item | "accounts"
  column: number
  row: number
  width: number
  height: number
}

/// An owner's items as pages: each where it first fits on the last page, row by row, a widget from an even column,
/// and what doesn't fit starting the next page, as a phone fills its Home Screen. There's always one.
function pack(items: (Item | "accounts")[], rows: number): Placed[][] {
  const pages: Placed[][] = [[]]
  let taken: boolean[][] = []
  const free = (width: number, height: number) => {
    for (let row = 0; row + height <= rows; row++)
      for (let column = 0; column + width <= COLUMNS; column += width > 1 ? 2 : 1)
        if (Array.from({ length: height }, (_, r) => Array.from({ length: width }, (_, c) => taken[row + r]?.[column + c])).flat().every((cell) => !cell)) return { row, column }
  }
  for (const item of items) {
    const [width, tall] = typeof item === "object" && item.widget ? (SPANS[item.size!] ?? [2, 2]) : [1, 1]
    const height = Math.min(tall, rows)
    let spot = free(width, height)
    if (!spot) {
      pages.push([])
      taken = []
      spot = free(width, height)!
    }
    for (let row = spot.row; row < spot.row + height; row++) for (let column = spot.column; column < spot.column + width; column++) (taken[row] ??= [])[column] = true
    pages.at(-1)!.push({ item, column: spot.column + 1, row: spot.row + 1, width, height })
  }
  return pages
}

/// A widget on a page: what its plugin's tool answers, drawn, and asked again each minute it's in view. Pressing it
/// opens its app.
function Card(props: { item: WidgetItem; owner: Owner }) {
  const tool = () => props.item.widget.replace("/", "_")
  const layout = createMemo(() => json<NonNullable<Parameters<typeof Widget>[0]["layout"]>>(tool(), { size: props.item.size }, props.owner.subject))
  onSettled(() => {
    const stop = changed(() => refresh(layout))
    const every = setInterval(() => document.visibilityState === "visible" && refresh(layout), 60_000)
    return () => (stop(), clearInterval(every))
  })
  const act = async (name: string | undefined, inputs: Record<string, unknown>) => {
    await call(`${props.item.widget.split("/")[0]}_${name}`, inputs, { subject: props.owner.subject }).catch(() => {})
    refresh(layout)
  }
  const opens = () => props.item.opens && launch(props.item.opens, props.owner)
  return (
    <Errored fallback={(error) => <Widget name={props.item.name} size={props.item.size} problem={String((error() as Error).message ?? error())} />}>
      <Loading fallback={<Widget name={props.item.name} size={props.item.size} />}>
        <Widget name={props.item.name} size={props.item.size} layout={layout()} onCall={act} onOpen={props.item.opens ? opens : undefined} />
      </Loading>
    </Errored>
  )
}

/// An icon with its name under it, an app's or the Accounts folder's. The app starts as a finger lands on it, and shows
/// as the finger lifts.
function Tile(props: { name: string; onPress: (behind?: boolean) => void; children: JSX.Element }) {
  return (
    <button type="button" class="tile" onPointerDown={() => props.onPress(true)} onClick={() => props.onPress()}>
      <span class="icon">{props.children}</span>
      <span>{props.name}</span>
    </button>
  )
}

const AppTile = (props: { app: App; owner: Owner }) => (
  <Tile name={props.app.name} onPress={(behind) => launch(props.app, props.owner, behind)}>
    <img src={props.app.icon} alt="" />
  </Tile>
)

/// The Accounts folder, last on your pages: the icons of the plugins with accounts, which aren't apps, small in a 3×3
/// grid, and a dot while an account needs its person.
function AccountsTile(props: { plugins: PluginWithAccounts[] }) {
  return (
    <Tile name="Accounts" onPress={(behind) => open(ACCOUNTS, { behind })}>
      <span class="folder">
        <For each={props.plugins.slice(0, 9)} keyed={(plugin) => plugin.plugin}>
          {(plugin) => <img src={plugin().icon} alt="" />}
        </For>
      </span>
      <Loading>
        <Attention plugins={props.plugins} />
      </Loading>
    </Tile>
  )
}

/// A dot while any account of the folder's plugins is not ok, as each plugin's `accounts` answers.
function Attention(props: { plugins: PluginWithAccounts[] }) {
  const needed = createMemo(async () => {
    const lists = await Promise.all(props.plugins.map((plugin) => json<{ accounts: Account[] }>(`${plugin.name}_accounts`, {}, plugin.subject).then((listed) => listed.accounts, (): Account[] => [])))
    return lists.flat().some((account) => !account.ok)
  })
  onSettled(() => changed(() => refresh(needed)))
  return (
    <Show when={needed()}>
      <i class="attention" aria-label="An account needs you" />
    </Show>
  )
}

/// A Home page: its grid, each item where it was placed. An owner with nothing on their pages has one grey line.
function HomePage(props: { owner: Owner; placed: Placed[]; rows: number; withAccounts: PluginWithAccounts[] }) {
  return (
    <section class="page" style={{ "--rows": props.rows }}>
      <Show when={props.placed.length} fallback={<Empty>No apps on this page.</Empty>}>
        <div class="grid">
          <For each={props.placed} keyed={({ item }) => (item === "accounts" ? item : (item.widget ?? item.app) + (item.size ?? ""))}>
            {(placed) => (
              <div class="cell" data-size={placed().item !== "accounts" ? (placed().item as Item).size : undefined} style={{ "--column": placed().column, "--row": placed().row, "--width": placed().width, "--height": placed().height }}>
                <Show when={placed().item !== "accounts" && (placed().item as Item)} fallback={<AccountsTile plugins={props.withAccounts} />}>
                  {(item) => (
                    <Show when={item().widget} fallback={<AppTile app={item() as App} owner={props.owner} />}>
                      <Card item={item() as WidgetItem} owner={props.owner} />
                    </Show>
                  )}
                </Show>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  )
}

/// An owner's App Library: a category for each kind of app they have, two across, each its first four icons small,
/// or its first three and the rest smaller in the fourth's place. Tapping an icon opens its app; tapping anywhere
/// else in a category opens it.
function Library(props: { owner: Owner; index: number; places: Map<string, HTMLElement> }) {
  const opening = (category: Category) => () => state().matches("nothing") && home.send({ type: "open", category: { owner: props.index, name: category.name } })
  return (
    <section class="page library scrolls" aria-label={`${props.owner.name}'s App Library`}>
      <Show when={props.owner.library.length} fallback={<Empty>No apps yet.</Empty>}>
        <div class="categories">
          <For each={props.owner.library} keyed={(category) => category.name}>
            {(category) => (
              <div class="category">
                <div class="place" ref={(place) => props.places.set(`${props.index}:${category().name}`, place)} onClick={opening(category())}>
                  <For each={category().apps.length > 4 ? category().apps.slice(0, 3) : category().apps} keyed={(app) => app.app}>
                    {(app) => (
                      <button type="button" class="app" aria-label={app().name} onPointerDown={() => launch(app(), props.owner, true)} onClick={(event) => (event.stopPropagation(), launch(app(), props.owner))}>
                        <img src={app().icon} alt="" />
                      </button>
                    )}
                  </For>
                  <Show when={category().apps.length > 4}>
                    <button type="button" class="more" aria-label={`All of ${category().name}`}>
                      <For each={category().apps.slice(3, 7)} keyed={(app) => app.app}>
                        {(app) => <img src={app().icon} alt="" />}
                      </For>
                    </button>
                  </Show>
                </div>
                <button type="button" class="name" onClick={opening(category())}>{category().name}</button>
              </div>
            )}
          </For>
        </div>
      </Show>
    </section>
  )
}

/// A category opened over its App Library: its name, and its apps in a panel that grows out of its place there and
/// shrinks back into it, as an iPhone's folder does, while what's behind it blurs. It's open only once it has grown:
/// until then a tap does nothing. Tapping beside the panel, Escape or the device's own back closes it.
function OpenCategory(props: { owner: Owner; opened: Opened; places: Map<string, HTMLElement> }) {
  let veil!: HTMLDivElement
  let panel!: HTMLDivElement
  let title!: HTMLHeadingElement
  let grid!: HTMLDivElement
  const category = () => props.owner.library.find((category) => category.name === props.opened.name)
  const phase = () => (state().matches({ category: "opening" }) ? "opening" : state().matches({ category: "open" }) ? "open" : "closing")
  const close = () => state().matches({ category: "open" }) && home.send({ type: "close" })
  // From its place to the middle, or back, in the time the machine gives it.
  const zoom = (opening: boolean) => {
    const place = props.places.get(`${props.opened.owner}:${props.opened.name}`)
    if (!place || !panel.isConnected) return
    const [from, to] = [place.getBoundingClientRect(), panel.getBoundingClientRect()]
    const moved = `translate(${from.x + from.width / 2 - (to.x + to.width / 2)}px, ${from.y + from.height / 2 - (to.y + to.height / 2)}px) scale(${from.width / to.width}, ${from.height / to.height})`
    const timing: KeyframeAnimationOptions = { duration: still.matches ? 0 : 320, easing: "cubic-bezier(0.32, 0.72, 0, 1)", fill: "both" }
    const way = <T,>(frames: T[]) => (opening ? frames : frames.toReversed())
    veil.animate(way([{ opacity: 0 }, { opacity: 1 }]), timing)
    title.animate(way([{ opacity: 0, transform: "translateY(12px)" }, { opacity: 1, transform: "none" }]), timing)
    grid.animate(way([{ opacity: 0 }, { opacity: 1 }]), { ...timing, duration: Number(timing.duration) * 0.7, delay: opening ? Number(timing.duration) * 0.3 : 0 })
    panel.animate(way([{ transform: moved, borderRadius: "1.5rem" }, { transform: "none", borderRadius: "2.25rem" }]), timing)
  }
  createEffect(phase, (now) => {
    if (now !== "open") zoom(now === "opening")
  })
  onSettled(() => {
    const shut = covers(close)
    const escape = (event: KeyboardEvent) => event.key === "Escape" && close()
    addEventListener("keydown", escape)
    return () => (shut(), removeEventListener("keydown", escape))
  })
  return (
    <div class="opened" ref={veil} data-phase={phase()} onClick={(event) => event.target === veil && close()}>
      <h2 ref={title}>{props.opened.name}</h2>
      <div class="panel" ref={panel} onClick={(event) => event.target === panel && close()}>
        <div class="grid" ref={grid}>
          <For each={category()?.apps ?? []} keyed={(app) => app.app}>
            {(app) => (
              <div class="cell">
                <Tile name={app().name} onPress={(behind) => phase() === "open" && launch(app(), props.owner, behind)}>
                  <img src={app().icon} alt="" />
                </Tile>
              </div>
            )}
          </For>
        </div>
      </div>
    </div>
  )
}

/// The page dots of an owner's row, which belong to it and move with it: one for each Home page and the App Library's
/// mark last, the one shown current. Tapping one goes there, and so does sliding a finger along them, a page for every
/// 28 points, as a phone's do.
function Dots(props: { owner: Owner; index: number; kinds: Kind[]; hidden: boolean }) {
  const current = () => (state().context.at.owner === props.index ? state().context.at.page : (state().context.last[props.index] ?? 0))
  let quiet = 0
  const slide = (down: PointerEvent & { currentTarget: HTMLElement }) => {
    const [dots, from] = [down.currentTarget, current()]
    let sliding = false
    const move = (event: PointerEvent) => {
      const across = (event.clientX - down.clientX) * rtl()
      if (!sliding && Math.abs(across) < 10) return
      sliding = true
      dots.setPointerCapture(down.pointerId)
      const page = Math.max(0, Math.min(props.kinds.length - 1, from + Math.trunc(across / 28)))
      if (page !== current()) home.send({ type: "go", page })
    }
    const up = () => {
      dots.removeEventListener("pointermove", move)
      if (sliding) quiet = Date.now() + 300
    }
    dots.addEventListener("pointermove", move)
    dots.addEventListener("pointerup", up, { once: true })
    dots.addEventListener("pointercancel", up, { once: true })
  }
  return (
    <nav class="dots" aria-label={`${props.owner.name}'s pages`} data-hidden={props.hidden ? "" : undefined} onPointerDown={slide}>
      <For each={props.kinds}>
        {(kind, page) => (
          <button type="button" class={kind === "library" ? "library" : "dot"} aria-label={kind === "library" ? "App Library" : `Page ${page() + 1}`} aria-current={current() === page() ? "page" : undefined} onClick={() => Date.now() > quiet && home.send({ type: "go", page: page() })}>
            <Show when={kind === "library"} fallback={<i />}>
              <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true">
                <path d="M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z" />
              </svg>
            </Show>
          </button>
        )}
      </For>
    </nav>
  )
}

/// The owner switcher, fixed at the top in the middle: the owner's name, which rolls as the owners move, and a tap
/// opens the list of every owner, the current one ticked. Picking one slides there.
function Switcher(props: { owners: Owner[]; hidden: boolean }) {
  const at = () => state().context.at.owner
  return (
    <div class="top" data-hidden={props.hidden ? "" : undefined}>
      <Menu.Root open={state().matches("listing")} onOpenChange={({ open }) => home.send({ type: open ? "list" : "dismiss" })} onSelect={({ value }) => home.send({ type: "pick", owner: Number(value) })} positioning={{ placement: "bottom" }}>
        <Menu.Trigger class="switch" variant="ghost" aria-label={props.owners[at()]?.name}>
          <span class="roll" aria-hidden="true">
            <span class="names">
              <For each={props.owners}>{(owner) => <span>{owner.name}</span>}</For>
            </span>
          </span>
          <Icon name="caret-up-down" />
        </Menu.Trigger>
        <Menu.Positioner>
          <Menu.Content>
            <For each={props.owners}>
              {(owner, index) => (
                <Menu.Item value={String(index())} data-current={index() === at() ? "" : undefined}>
                  <Icon name="check" />
                  <span>{owner.name}</span>
                </Menu.Item>
              )}
            </For>
          </Menu.Content>
        </Menu.Positioner>
      </Menu.Root>
    </div>
  )
}

/// Room for a page's rows, its pages side by side, and whether the window is wide: the window's, as it changes.
function measure() {
  const rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16
  const wide = innerWidth >= 34 * rem
  // A page starts under the owner switcher, and keeps the dots' room at its foot.
  const room = innerHeight - (wide ? 5 : 3.5) * rem - 4 * rem
  const rows = Math.max(1, Math.min(6, Math.floor((room + rem) / (6.5 * rem))))
  const fit = wide ? Math.max(1, Math.min(3, Math.floor((innerWidth - 4 * rem + 3 * rem) / (26 * rem)))) : 1
  return { wide, rows, fit }
}

function HomeScreen(props: { owners: Owner[] }) {
  let root!: HTMLDivElement
  let main!: HTMLElement
  const strips: HTMLElement[] = []
  const places = new Map<string, HTMLElement>()
  const [room, setRoom] = createSignal(measure())

  // Every plugin with accounts on any owner's pages, each once, is in the one Accounts folder, last on yours.
  const withAccounts = createMemo(() => [...new Map(props.owners.flatMap((owner) => owner.plugins_with_accounts.map((plugin): [string, PluginWithAccounts] => [plugin.plugin, { ...plugin, subject: owner.subject }]))).values()])
  const pages = createMemo(() => props.owners.map((owner, index) => pack([...owner.items, ...(index === 0 && withAccounts().length ? ["accounts" as const] : [])], room().rows)))
  const laid = createMemo(() => pages().map((homes) => [...homes.map((): Kind => "home"), "library" as Kind]), { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) })
  createEffect(laid, (pages) => home.send({ type: "laid", pages }))

  /// The kind of the page an owner is on: the one shown, for whoever's shown.
  const kindOf = (owner: number) => {
    const { at, last, pages } = state().context
    return pages[owner]?.[owner === at.owner ? at.page : (last[owner] ?? 0)] ?? "home"
  }
  const step = (strip: HTMLElement) => (strip.firstElementChild as HTMLElement | null)?.offsetWidth ?? strip.clientWidth
  const gap = (strip: HTMLElement) => parseFloat(getComputedStyle(strip).columnGap) || 0

  /// Where the pages rest now, told to the machine when it's somewhere new; and the owner's name rolls as they move.
  const moved = () => {
    const height = Math.max(1, main.clientHeight)
    root.style.setProperty("--f", String(main.scrollTop / height))
    const owner = Math.max(0, Math.min(props.owners.length - 1, Math.round(main.scrollTop / height)))
    const strip = strips[owner]
    const page = strip ? Math.round(Math.abs(strip.scrollLeft) / (step(strip) + gap(strip))) : 0
    const { at } = home.getSnapshot().context
    if (at.owner !== owner || at.page !== page) home.send({ type: "settled", at: { owner, page } })
  }

  /// Moves the pages to `at`, as the machine says.
  const slide = (at: At, smooth: boolean) => {
    const behavior = smooth && !still.matches ? "smooth" : "instant"
    main.scrollTo({ top: at.owner * main.clientHeight, behavior })
    const strip = strips[at.owner]
    strip?.scrollTo({ left: at.page * (step(strip) + gap(strip)) * rtl(), behavior })
  }

  onSettled(() => {
    const sliding = home.on("slide", ({ at, smooth }) => slide(at, smooth))
    const resized = () => setRoom(measure())
    addEventListener("resize", resized)

    // A trackpad or a wheel: one gesture is one page or one owner, read here, since a browser snaps a trackpad's small
    // scroll back where it started. Up and down on a page that scrolls itself is that page's.
    let total = 0
    let axis: "x" | "y" | undefined
    let done = false
    let last = 0
    const wheel = (event: WheelEvent) => {
      if (!home.getSnapshot().matches("nothing")) return
      const now = performance.now()
      if (now - last > 250) [total, axis, done] = [0, undefined, false]
      last = now
      axis ??= Math.abs(event.deltaX) > Math.abs(event.deltaY) ? "x" : "y"
      if (axis === "y" && (event.target as Element).closest?.(".page.scrolls")) return
      event.preventDefault()
      if (done) return
      total += axis === "x" ? event.deltaX : event.deltaY
      if (Math.abs(total) < 30) return
      done = true
      const by = Math.sign(total) as 1 | -1
      home.send(axis === "y" ? { type: "step", owner: by } : { type: "step", page: (by * rtl()) as 1 | -1 })
    }
    root.addEventListener("wheel", wheel, { passive: false })

    // The arrow keys move as a swipe does: up and down on a page that scrolls itself scroll it.
    const keys = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || event.metaKey || event.ctrlKey || !home.getSnapshot().matches("nothing")) return
      const to = ({ ArrowRight: { page: rtl() }, ArrowLeft: { page: -rtl() }, ArrowDown: { owner: 1 }, ArrowUp: { owner: -1 } } as Record<string, { page: 1 | -1 } | { owner: 1 | -1 }>)[event.key]
      if (!to || ("owner" in to && kinds[kindOf(home.getSnapshot().context.at.owner)].scrolls)) return
      event.preventDefault()
      home.send({ type: "step", ...to })
    }
    addEventListener("keydown", keys)
    return () => (sliding.unsubscribe(), removeEventListener("resize", resized), root.removeEventListener("wheel", wheel), removeEventListener("keydown", keys))
  })

  const opened = () => state().context.category
  return (
    <div class={["home", { wide: room().wide }]} ref={root}>
      <main class="owners" ref={main} onScroll={moved}>
        <For each={props.owners} keyed={(owner) => owner.key}>
          {(owner, index) => (
            <section class="owner" aria-label={owner().name}>
              <div class="strip" ref={(strip) => (strips[index()] = strip)} style={{ "--pages": Math.min(room().fit, laid()[index()]?.length ?? 1) }} onScroll={moved}>
                <For each={pages()[index()] ?? [[]]} keyed={false}>
                  {(placed) => <HomePage owner={owner()} placed={placed()} rows={room().rows} withAccounts={withAccounts()} />}
                </For>
                <Library owner={owner()} index={index()} places={places} />
              </div>
              <Dots owner={owner()} index={index()} kinds={laid()[index()] ?? []} hidden={!kinds[kindOf(index())].chrome || (laid()[index()]?.length ?? 0) <= room().fit} />
            </section>
          )}
        </For>
      </main>
      <Switcher owners={props.owners} hidden={!kinds[kindOf(state().context.at.owner)].chrome} />
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[opened().owner]} opened={opened()} places={places} />}</Show>
    </div>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Problem heading="Can't show your Home Screen" text={error() as Error} />}>
      <Loading>
        <HomeScreen owners={owners()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
