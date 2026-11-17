// The Home Screen: yours, everyone's, then each of your groups', one owner above another, each a row of pages and then
// their App Library. Left and right moves between an owner's pages, as a phone's do, and up and down between owners,
// unless a page scrolls itself, as the App Library does. The owner's name at the top opens a list of every owner. A
// page never scrolls: it's a fixed grid, and what doesn't fit goes on to the next. A wide window shows as many whole
// pages of an owner's side by side as fit, one to three. Whoever arranges an owner's pages edits them as a phone's
// Home Screen is edited: touch and hold, or right-click, then Edit Home Screen. Where it is, what's open over it and
// the editing are one state machine (machine.ts), whose snapshot is one signal here.
import { createEffect, createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show } from "solid-js"
import { render, type JSX } from "@solidjs/web"
import { createActor } from "xstate"
import { Button, call, changed, confirm, covers, Empty, Icon, List, Menu, open, Problem, Row, Section, Sheet, toast, Widget } from "ui"
import "./home.css"
import { type At, type Editing, fitted, type Held, kinds, type Kind, machine, type Opened, pack, type Page, type Placed, shown, spans } from "./machine.ts"
import type { App, Category, Item, Pages, Size, Widget as WidgetItem } from "./types.ts"

const HOME = "home-screen/client/index.html"
const SIZES: Record<Size, string> = { "2x2": "Small", "4x2": "Medium", "4x4": "Large" }
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
const editing = () => state().context.editing
const editingOwner = (owner: number) => state().matches("editing") && editing()?.owner === owner

/// Opens an app for whoever its owner is, for the group whose install it is, or for its person where it's about
/// whoever opens it. `behind` only starts it, as a finger lands on it.
const launch = (app: App, owner: Owner, behind?: boolean) => open(app.app, { subject: app.subject === "user" ? undefined : (app.subject ?? owner.subject), behind })

/// An owner's pages as they're kept: each with what's on it and whether it's hidden, to edit.
const draft = (owner: Owner, index: number): Editing => ({ owner: index, pages: fitted(owner.pages.map((page) => ({ items: [...page.items], hidden: page.hidden }))) })

/// An owner's pages as they were just edited, shown until the Home Screen is asked again.
const [kept, setKept] = createSignal<Editing>()

// A finger that stays this long holds what it's on, as a phone's does.
const HOLD = 450

/// A widget on a page: what its plugin's tool answers, drawn, and asked again each minute it's in view. Pressing it
/// opens its app.
function Card(props: { item: WidgetItem; owner: Owner; still?: boolean }) {
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
  const opens = () => !props.still && state().matches("nothing") && props.item.opens && launch(props.item.opens, props.owner)
  return (
    <Errored fallback={(error) => <Widget name={props.item.name} size={props.item.size} problem={String((error() as Error).message ?? error())} />}>
      <Loading fallback={<Widget name={props.item.name} size={props.item.size} />}>
        <Widget name={props.item.name} size={props.item.size} layout={layout()} onCall={props.still ? undefined : act} onOpen={props.item.opens ? opens : undefined} />
      </Loading>
    </Errored>
  )
}

/// An icon with its name under it. The app starts as a finger lands on it, and shows as the finger lifts, unless the
/// finger held it.
function Tile(props: { name: string; onPress?: (behind?: boolean) => void; children: JSX.Element }) {
  return (
    <button type="button" class="tile" onPointerDown={() => props.onPress?.(true)} onClick={() => props.onPress?.()}>
      <span class="icon">{props.children}</span>
      <span>{props.name}</span>
    </button>
  )
}

/// What a Home page holds as it's drawn: an item of the owner's, and where it is among the page's.
type Cell = { item: Item; index: number }

/// Watches a finger or a mouse on what it's given, for the Home Screen's gestures: a touch held still is a hold, as is
/// a right-click; a press that moves while editing, or once held, lifts it. `held` says what it is, as it is then.
function holds(held: () => Held | undefined, arranges: () => boolean) {
  return (element: HTMLElement) => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let start: PointerEvent | undefined
    const clear = () => (clearTimeout(timer), (start = undefined))
    const rect = () => {
      const { x, y, width, height } = element.getBoundingClientRect()
      return { x, y, width, height }
    }
    const lift = (event: PointerEvent) => {
      const what = held()
      if (!what?.item) return
      home.send({ type: "lift", held: { ...what, rect: rect() }, editing: state().matches("editing") ? undefined : pending?.() })
      // The Home Screen keeps the finger from here on, whatever's drawn again under it.
      document.querySelector<HTMLElement>(".home")?.setPointerCapture(event.pointerId)
    }
    element.addEventListener("contextmenu", (event) => {
      event.preventDefault()
      event.stopPropagation()
      const what = held()
      if (arranges() && what && state().matches("nothing")) home.send({ type: "hold", held: { ...what, rect: rect() } })
    })
    element.addEventListener("pointerdown", (event) => {
      if (event.button || !arranges()) return
      start = event
      // Editing, a press that moves lifts at once, wherever the pointer goes first; otherwise a finger held still is a
      // hold.
      if (state().matches({ editing: "still" }) && held()?.item) element.setPointerCapture(event.pointerId)
      else if (!state().matches("editing") && event.pointerType !== "mouse")
        timer = setTimeout(() => {
          const what = held()
          if (what && state().matches("nothing")) home.send({ type: "hold", held: { ...what, rect: rect() } })
        }, HOLD)
    })
    element.addEventListener("pointermove", (event) => {
      if (!start) return
      const moved = Math.hypot(event.clientX - start.clientX, event.clientY - start.clientY)
      if (moved < 8) return
      // Held, or editing, a finger that moves carries what it's on; before that, it's a swipe.
      if (state().matches("holding") || (state().matches({ editing: "still" }) && held()?.item)) {
        clear()
        lift(event)
      } else clear()
    })
    element.addEventListener("pointerup", clear)
    element.addEventListener("pointercancel", clear)
    // A web view drags an icon as a picture of itself unless it's told not to, and a finger carrying something moves
    // it, not the page.
    element.addEventListener("dragstart", (event) => event.preventDefault())
    element.addEventListener("touchmove", (event) => (state().matches("holding") || state().matches({ editing: "dragging" })) && event.preventDefault(), { passive: false })
  }
}

/// The shown owner's pages to edit, as they're kept, for what's lifted before editing began: set by the Home Screen.
let pending: (() => Editing | undefined) | undefined

/// Until when a tap isn't one: just after something carried was put down.
let quiet = 0
/// Where the pointer was last, so what's carried starts under it.
let pointer = { x: 0, y: 0 }
addEventListener("pointermove", (event) => (pointer = { x: event.clientX, y: event.clientY }), { capture: true, passive: true })

/// A Home page: its grid, each item where it was placed. While its owner's pages are edited, everything jiggles with
/// its ⊖, and what's lifted leaves its place empty. An owner with nothing on their pages has one grey line.
function HomePage(props: { owner: Owner; index: number; page: number; placed: Placed<Cell>[]; small?: boolean }) {
  const editingHere = () => !props.small && editingOwner(props.index)
  const lifted = (cell: Cell) => {
    const held = state().context.held
    return editingHere() && state().matches({ editing: "dragging" }) && held?.page === props.page && held.index === cell.index
  }
  const arranges = () => props.owner.arranges && !props.small
  return (
    <section class={["page", { small: props.small }]} data-page={props.page} ref={props.small ? undefined : holds(() => (editingHere() ? undefined : { page: props.page, index: -1, from: "page" }), arranges)}>
      <Show when={props.placed.length || editingHere()} fallback={<Empty>No apps on this page.</Empty>}>
        <div class="grid">
          <For each={props.placed} keyed={({ item }) => (item.item.widget ?? item.item.app) + (item.item.size ?? "")}>
            {(placed) => (
              <div
                class={["cell", { jiggles: editingHere(), lifted: lifted(placed().item) }]}
                data-index={placed().item.index}
                data-size={placed().item.item.size}
                style={{ "--column": placed().column, "--row": placed().row, "--width": placed().width, "--height": placed().height, "--wobble": `${(placed().item.index * 97) % 240}ms` }}
                ref={props.small ? undefined : holds(() => ({ item: placed().item.item, page: props.page, index: placed().item.index, from: "page" }), arranges)}
              >
                <Show when={placed().item.item}>
                  {(item) => (
                    <>
                      <Show when={item().widget} fallback={<Tile name={(item() as App).name} onPress={(behind) => state().matches("nothing") && launch(item() as App, props.owner, behind)}><img src={(item() as App).icon} alt="" /></Tile>}>
                        <Card item={item() as WidgetItem} owner={props.owner} still={props.small || editingHere()} />
                      </Show>
                      <Show when={editingHere()}>
                        <button type="button" class="minus" aria-label={`Remove ${item().name}`} onPointerDown={(event) => event.stopPropagation()} onClick={() => home.send({ type: "remove", page: props.page, index: placed().item.index })}>
                          <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 12h10" /></svg>
                        </button>
                      </Show>
                    </>
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

/// An owner's App Library: a category for each kind of app they have, two across, each its first four apps' icons
/// with their names, or its first three and the rest smaller in the fourth's place. Tapping an icon opens its app; tapping anywhere
/// else in a category opens it. Touching and holding an app lifts it, to drag onto a Home page.
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
                    {(app) => <LibraryApp app={app()} owner={props.owner} index={props.index} />}
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

/// An app's icon in an App Library: a tap opens it, and a finger that holds it lifts it onto the owner's Home pages.
function LibraryApp(props: { app: App; owner: Owner; index: number }) {
  return (
    <button
      type="button"
      class="app"
      ref={holds(() => ({ item: props.app, page: 0, index: -1, from: "library" }), () => props.owner.arranges)}
      onPointerDown={() => state().matches("nothing") && launch(props.app, props.owner, true)}
      onClick={(event) => (event.stopPropagation(), state().matches("nothing") && launch(props.app, props.owner))}
    >
      <img src={props.app.icon} alt="" />
      <span>{props.app.name}</span>
    </button>
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

/// What a finger holds, lifted over the blurred Home Screen, as a phone's is, with its menu under it: Edit Home Screen,
/// and for an item, taking it off. Moving the finger carries it into editing; anywhere else, Escape or the device's
/// own back puts it down.
function Holding(props: { owner: Owner; index: number; held: Held }) {
  const dismiss = () => state().matches("holding") && home.send({ type: "dismiss" })
  const edit = () => home.send({ type: "edit", editing: draft(props.owner, props.index) })
  const removing = () => {
    const editing = draft(props.owner, props.index)
    const at = shown(editing.pages)[props.held.page]
    const pages = editing.pages.map((page, index) => (index === at ? { ...page, items: page.items.toSpliced(props.held.index, 1) } : page))
    home.send({ type: "dismiss" })
    keep({ ...editing, pages: pages.filter((page) => page.items.length) }, props.owner)
  }
  // From the App Library, it goes last on the owner's last page, as a phone's does.
  const adding = () => {
    const editing = draft(props.owner, props.index)
    const at = shown(editing.pages).at(-1)
    const pages = at === undefined ? [...editing.pages, { items: [props.held.item!] }] : editing.pages.map((page, index) => (index === at ? { ...page, items: [...page.items, props.held.item!] } : page))
    home.send({ type: "dismiss" })
    keep({ ...editing, pages: fitted(pages) }, props.owner)
  }
  onSettled(() => {
    const shut = covers(dismiss)
    const escape = (event: KeyboardEvent) => event.key === "Escape" && dismiss()
    addEventListener("keydown", escape)
    return () => (shut(), removeEventListener("keydown", escape))
  })
  const rect = props.held.rect ?? { x: innerWidth / 2, y: innerHeight / 2, width: 0, height: 0 }
  const below = rect.y + rect.height < innerHeight * 0.6
  const item = props.held.item
  // An app of the App Library that's on a page already has no Add to Home Screen.
  const added = !!item && props.owner.pages.some((page) => page.items.some((other) => other.app === item.app))
  const library = props.held.from === "library"
  return (
    <div class="holding" onClick={(event) => event.target === event.currentTarget && dismiss()}>
      <Show when={item}>
        {(item) => (
          <div class="lifted-copy" style={{ left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` }}>
            <Show when={item().widget} fallback={library ? <img class="alone" src={(item() as App).icon} alt="" /> : <Tile name={(item() as App).name}><img src={(item() as App).icon} alt="" /></Tile>}>
              <Card item={item() as WidgetItem} owner={props.owner} still />
            </Show>
          </div>
        )}
      </Show>
      <div class="hold-menu" role="menu" hidden={library && added} style={{ left: `${Math.max(12, Math.min(innerWidth - 252, rect.x))}px`, ...(below ? { top: `${rect.y + rect.height + 12}px` } : { bottom: `${innerHeight - rect.y + 12}px` }) }}>
        <button type="button" role="menuitem" hidden={props.held.from === "library"} onClick={edit}>
          <span>Edit Home Screen</span>
          <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true"><path d="M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z" /></svg>
        </button>
        <Show when={library && !added}>
          <button type="button" role="menuitem" onClick={adding}>
            <span>Add to Home Screen</span>
            <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true"><path d="M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm0,192a88,88,0,1,1,88-88A88.1,88.1,0,0,1,128,216Zm48-88a8,8,0,0,1-8,8H136v32a8,8,0,0,1-16,0V136H88a8,8,0,0,1,0-16h32V88a8,8,0,0,1,16,0v32h32A8,8,0,0,1,176,128Z" /></svg>
          </button>
        </Show>
        <Show when={item && props.held.from === "page"}>
          <button type="button" role="menuitem" class="destructive" onClick={removing}>
            <span>{item?.widget ? "Remove Widget" : "Remove from Home Screen"}</span>
            <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true"><path d="M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm0,192a88,88,0,1,1,88-88A88.1,88.1,0,0,1,128,216Zm48-88a8,8,0,0,1-8,8H88a8,8,0,0,1,0-16h80A8,8,0,0,1,176,128Z" /></svg>
          </button>
        </Show>
      </div>
    </div>
  )
}

/// Keeps an owner's pages as they were edited, for everyone who sees them, and says why if they couldn't be.
async function keep(editing: Editing, owner: Owner) {
  setKept(editing)
  const pages = editing.pages.map((page) => ({ items: page.items.map((item) => (item.widget ? { widget: item.widget, size: item.size } : { app: item.app })), ...(page.hidden ? { hidden: true } : {}) }))
  try {
    await call("home-screen_arrange", { pages }, { subject: owner.subject })
  } catch (error) {
    toast(error as Error)
  }
  refresh(owners)
}

/// The widget gallery: each widget the owner's apps have, and once one's picked, its sizes, each drawn as it would be,
/// with Add Widget, which places it first on the page shown.
function Gallery(props: { owner: Owner }) {
  const [picked, setPicked] = createSignal<WidgetItem>()
  const [size, setSize] = createSignal<Size>()
  const close = () => (setPicked(), setSize(), home.send({ type: "dismiss" }))
  const add = () => {
    const widget = picked()!
    home.send({ type: "place", item: { ...widget, size: size() ?? widget.sizes[0] } })
    setPicked()
    setSize()
  }
  return (
    <Sheet heading={picked()?.name ?? "Widgets"} open={state().matches({ editing: "gallery" })} onClose={close}>
      <Show
        when={picked()}
        fallback={
          <Show when={props.owner.widgets.length} fallback={<Empty>No widgets yet.</Empty>}>
            <List>
              <For each={props.owner.widgets} keyed={(widget) => widget.widget}>
                {(widget) => <Row heading={widget().name} start={widget().opens ? <img class="logo" src={widget().opens!.icon} alt="" /> : undefined} onClick={() => (setPicked(widget()), setSize(widget().sizes[0]))} />}
              </For>
            </List>
          </Show>
        }
      >
        {(widget) => (
          <div class="gallery">
            <div class="sizes" role="radiogroup" aria-label="Size">
              <For each={widget().sizes}>
                {(one) => (
                  <button type="button" role="radio" aria-checked={String(size() === one) as "true" | "false"} class="size" data-size={one} onClick={() => setSize(one)}>
                    <span class="preview">
                      <Card item={{ ...widget(), size: one }} owner={props.owner} still />
                    </span>
                    <span>{SIZES[one]}</span>
                  </button>
                )}
              </For>
            </div>
            <Button onClick={add}>Add Widget</Button>
          </div>
        )}
      </Show>
    </Sheet>
  )
}

/// The page overview: every Home page small, in order, each with a tick for whether it's shown. Unticked, a page is
/// hidden and can be deleted; dragging one puts it elsewhere. Done goes back to editing.
function Overview(props: { owner: Owner; index: number }) {
  const pages = () => editing()?.pages ?? []
  const [dragging, setDragging] = createSignal<number>()
  const remove = async (page: number) => {
    if (await confirm({ heading: "Delete Page?", text: "Its apps stay in the App Library.", action: "Delete", destructive: true })) home.send({ type: "delete", page })
  }
  const drag = (page: number) => (down: PointerEvent & { currentTarget: HTMLElement }) => {
    const from = down.currentTarget
    from.setPointerCapture(down.pointerId)
    setDragging(page)
    const up = (event: PointerEvent) => {
      from.removeEventListener("pointerup", up)
      setDragging()
      const over = (document.elementsFromPoint(event.clientX, event.clientY) as HTMLElement[]).find((under) => under.dataset?.overview !== undefined)
      if (over && Number(over.dataset.overview) !== page) home.send({ type: "reorder", from: page, to: Number(over.dataset.overview) })
    }
    from.addEventListener("pointerup", up)
  }
  onSettled(() => covers(() => home.send({ type: "dismiss" })))
  return (
    <div class="overview">
      <div class="bar">
        <Button onClick={() => home.send({ type: "dismiss" })}>Done</Button>
      </div>
      <div class="thumbnails">
        <For each={pages()}>
          {(page, index) => (
            <div class={["thumbnail", { hidden: page.hidden, dragging: dragging() === index() }]} data-overview={index()}>
              <div class="miniature" onPointerDown={drag(index())}>
                <HomePage owner={props.owner} index={props.index} page={index()} placed={(pack(page.items.map((item, at): Cell => ({ item, index: at })), (cell) => spans(cell.item))[0] ?? [])} small />
                <Show when={page.hidden}>
                  <button type="button" class="minus" aria-label={`Delete page ${index() + 1}`} onPointerDown={(event) => event.stopPropagation()} onClick={() => remove(index())}>
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="M7 12h10" /></svg>
                  </button>
                </Show>
              </div>
              <button type="button" class="tick" role="checkbox" aria-checked={String(!page.hidden) as "true" | "false"} aria-label={`Show page ${index() + 1}`} onClick={() => home.send({ type: "hide", page: index(), hidden: !page.hidden })}>
                <Icon name="check" />
              </button>
            </div>
          )}
        </For>
      </div>
    </div>
  )
}

/// The page dots of an owner's row, which belong to it and move with it: one for each Home page and the App Library's
/// mark last, the one shown current. Tapping one goes there, and so does sliding a finger along them, a page for every
/// 28 points, as a phone's do. While the owner's pages are edited, they're a capsule that opens the page overview.
function Dots(props: { owner: Owner; index: number; kinds: Kind[]; hidden: boolean }) {
  const current = () => (state().context.at.owner === props.index ? state().context.at.page : (state().context.last[props.index] ?? 0))
  let quiet = 0
  const slide = (down: PointerEvent & { currentTarget: HTMLElement }) => {
    if (editingOwner(props.index)) return
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
  const press = (page: number) => () => {
    if (editingOwner(props.index)) return home.send({ type: "overview" })
    if (Date.now() > quiet) home.send({ type: "go", page })
  }
  return (
    <nav class={["dots", { editing: editingOwner(props.index) }]} aria-label={`${props.owner.name}'s pages`} data-hidden={props.hidden && !editingOwner(props.index) ? "" : undefined} onPointerDown={slide}>
      <For each={props.kinds}>
        {(kind, page) => (
          <Show when={!(editingOwner(props.index) && kind === "library")}>
            <button type="button" class={kind === "library" ? "library" : "dot"} aria-label={kind === "library" ? "App Library" : `Page ${page() + 1}`} aria-current={current() === page() ? "page" : undefined} onClick={press(page())}>
              <Show when={kind === "library"} fallback={<i />}>
                <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true">
                  <path d="M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z" />
                </svg>
              </Show>
            </button>
          </Show>
        )}
      </For>
    </nav>
  )
}

/// The owner switcher, fixed at the top in the middle: the owner's name, which rolls as the owners move, and a tap
/// opens the list of every owner, the current one ticked. Picking one slides there. While pages are edited, + and Done
/// are there instead, as on a phone.
function Switcher(props: { owners: Owner[] }) {
  const at = () => state().context.at.owner
  let button!: HTMLButtonElement
  return (
    <div class="top">
      <Show
        when={!state().matches("editing")}
        fallback={
          <div class="edit-bar">
            <Button variant="secondary" size="icon" aria-label="Add Widget" onClick={() => state().matches({ editing: "still" }) && home.send({ type: "gallery" })}>
              <Icon name="plus" />
            </Button>
            <Button variant="secondary" onClick={() => home.send({ type: "done" })}>Done</Button>
          </div>
        }
      >
        <button type="button" class="switch ui-button press ring" data-variant="ghost" ref={button} aria-label={props.owners[at()]?.name} aria-haspopup="menu" aria-expanded={state().matches("listing") ? "true" : "false"} onClick={() => home.send({ type: state().matches("listing") ? "dismiss" : "list" })}>
          <span class="roll" aria-hidden="true">
            <span class="names">
              <For each={props.owners}>{(owner) => <span>{owner.name}</span>}</For>
            </span>
          </span>
          <Icon name="caret-up-down" />
        </button>
        <Menu.Root open={state().matches("listing")} onOpenChange={({ open }) => !open && home.send({ type: "dismiss" })} onSelect={({ value }) => home.send({ type: "pick", owner: Number(value) })} positioning={{ placement: "bottom", getAnchorRect: () => button.getBoundingClientRect() }}>
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
      </Show>
    </div>
  )
}

/// Room for its pages side by side, and whether the window is wide: the window's, as it changes.
function measure() {
  const rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16
  const wide = innerWidth >= 34 * rem
  const fit = wide ? Math.max(1, Math.min(3, Math.floor((innerWidth - 4 * rem + 3 * rem) / (26 * rem)))) : 1
  return { wide, fit }
}

function HomeScreen(props: { owners: Owner[] }) {
  let root!: HTMLDivElement
  let main!: HTMLElement
  const strips: HTMLElement[] = []
  const places = new Map<string, HTMLElement>()
  const [room, setRoom] = createSignal(measure())


  /// Each owner's Home pages as they're drawn: the ones they keep shown, or while they're edited, as they're edited,
  /// each placed on its grid. A page kept with more than fits goes on over the next ones.
  const pages = createMemo(() =>
    props.owners.map((owner, index) => {
      const edited = editingOwner(index) ? editing()!.pages : kept()?.owner === index ? kept()!.pages : owner.pages
      const showing: Page[] = shown(edited).map((at) => edited[at])
      const cells = showing.map((page): Cell[] => page.items.map((item, at) => ({ item, index: at })))
      if (!cells.length) cells.push([])
      return cells.flatMap((page) => (editingOwner(index) ? [pack(page, (cell) => spans(cell.item)).flat()] : pack(page, (cell) => spans(cell.item))))
    }),
  )
  const laid = createMemo(() => pages().map((homes) => [...homes.map((): Kind => "home"), "library" as Kind]), { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) })
  // What was just kept shows until the Home Screen has been asked again.
  createEffect(
    () => props.owners,
    () => setKept(),
    { defer: true },
  )
  createEffect(laid, (pages) => home.send({ type: "laid", pages }))
  pending = () => (props.owners[state().context.at.owner]?.arranges ? draft(props.owners[state().context.at.owner], state().context.at.owner) : undefined)

  /// The kind of the page an owner is on: the one shown, for whoever's shown.
  const kindOf = (owner: number) => {
    const { at, last, pages } = state().context
    return pages[owner]?.[owner === at.owner ? at.page : (last[owner] ?? 0)] ?? "home"
  }
  const step = (strip: HTMLElement) => (strip.firstElementChild as HTMLElement | null)?.offsetWidth ?? strip.clientWidth
  const gap = (strip: HTMLElement) => parseFloat(getComputedStyle(strip).columnGap) || 0

  /// The owner's name rolls as the owners move.
  const moved = () => root.style.setProperty("--f", String(main.scrollTop / Math.max(1, main.getBoundingClientRect().height)))
  /// Where the pages came to rest, told to the machine when it's somewhere new, and the name exactly that owner's,
  /// whatever fraction of a pixel the scroll stopped short by. Nothing waits for it: a swipe while they move is the
  /// browser's, and a step goes on from where the machine last sent them.
  const rested = () => {
    const owner = Math.max(0, Math.min(props.owners.length - 1, Math.round(main.scrollTop / Math.max(1, main.getBoundingClientRect().height))))
    const strip = strips[owner]
    const page = strip ? Math.round(Math.abs(strip.scrollLeft) / (step(strip) + gap(strip))) : 0
    const { at } = home.getSnapshot().context
    if (at.owner !== owner || at.page !== page) home.send({ type: "settled", at: { owner, page } })
    root.style.setProperty("--f", String(owner))
  }

  /// Moves the pages to `at`, as the machine says.
  const slide = (at: At, smooth: boolean) => {
    const behavior = smooth && !still.matches ? "smooth" : "instant"
    main.scrollTo({ top: at.owner * main.clientHeight, behavior })
    const strip = strips[at.owner]
    strip?.scrollTo({ left: at.page * (step(strip) + gap(strip)) * rtl(), behavior })
  }

  /// What's carried, while it's dragged: a copy that follows the finger, and over what it would go before. Held at a
  /// page's edge, it goes on to the next page, or the one before.
  createEffect(
    () => state().matches({ editing: "dragging" }),
    (dragging) => {
      if (!dragging) return
      const held = home.getSnapshot().context.held!
      const rect = held.rect ?? { x: 0, y: 0, width: 60, height: 80 }
      const copy = document.createElement("div")
      copy.className = "carried"
      Object.assign(copy.style, { left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` })
      const item = held.item!
      copy.style.translate = `${pointer.x - rect.x - rect.width / 2}px ${pointer.y - rect.y - rect.height / 2}px`
      copy.innerHTML = item.widget ? `<div class="ui-widget" data-size="${item.size}"></div>` : `<span class="tile"><span class="icon"><img src="${(item as App).icon}" alt=""></span><span>${(item as App).name}</span></span>`
      root.append(copy)
      let edge: ReturnType<typeof setTimeout> | undefined
      let side = 0
      const move = (event: PointerEvent) => {
        copy.style.translate = `${event.clientX - rect.x - rect.width / 2}px ${event.clientY - rect.y - rect.height / 2}px`
        const strip = strips[home.getSnapshot().context.at.owner]
        const bounds = strip.getBoundingClientRect()
        const now = event.clientX < bounds.left + 28 ? -1 : event.clientX > bounds.right - 28 ? 1 : 0
        if (now !== side) {
          clearTimeout(edge)
          side = now
          if (side) edge = setTimeout(function flip() {
            home.send({ type: "flip", by: side as 1 | -1 })
            edge = setTimeout(flip, 900)
          }, 600)
        }
        if (side) return
        const under = document.elementsFromPoint(event.clientX, event.clientY) as HTMLElement[]
        const page = under.find((element) => element.dataset?.page !== undefined)
        if (!page) return
        const cell = under.find((element) => element.classList.contains("cell") && element.dataset.index !== undefined && !element.classList.contains("lifted"))
        const { held } = home.getSnapshot().context
        const index = cell ? Number(cell.dataset.index) : Number.MAX_SAFE_INTEGER
        const at = Number(page.dataset.page)
        if (held && (at !== held.page || (cell && index !== held.index))) home.send({ type: "over", page: at, index })
      }
      const drop = () => home.send({ type: "drop" })
      addEventListener("pointermove", move)
      addEventListener("pointerup", drop)
      addEventListener("pointercancel", drop)
      return () => {
        clearTimeout(edge)
        removeEventListener("pointermove", move)
        removeEventListener("pointerup", drop)
        removeEventListener("pointercancel", drop)
        copy.remove()
        quiet = Date.now() + 400
      }
    },
  )

  onSettled(() => {
    const sliding = home.on("slide", ({ at, smooth }) => slide(at, smooth))
    const arranging = home.on("arrange", ({ editing }) => keep(editing, props.owners[editing.owner]))
    // The window may have changed since this was first drawn.
    const resized = () => setRoom(measure())
    resized()
    addEventListener("resize", resized)

    // A trackpad or a wheel: one gesture is one page or one owner, read here, since a browser snaps a trackpad's small
    // scroll back where it started. Up and down on a page that scrolls itself is that page's. A gesture's momentum
    // dies away, so once it has stepped, what follows is its tail and is let be, until a pause, or a push that grows
    // again once the tail has fallen to half its peak: a new gesture, which steps at once, wherever the pages are.
    let total = 0
    let axis: "x" | "y" | undefined
    let done = false
    let last = 0
    let size = 0
    let peak = 0
    const wheel = (event: WheelEvent) => {
      if (!home.getSnapshot().matches("nothing")) return
      const now = performance.now()
      const push = Math.max(Math.abs(event.deltaX), Math.abs(event.deltaY))
      if (now - last > 250 || (done && size < peak / 2 && push > 8 && push > size * 1.5)) [total, axis, done, peak] = [0, undefined, false, 0]
      last = now
      size = push
      peak = Math.max(peak, push)
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

    // The arrow keys move as a swipe does: up and down on a page that scrolls itself scroll it. Escape ends editing.
    const keys = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || event.metaKey || event.ctrlKey) return
      if (event.key === "Escape" && home.getSnapshot().matches({ editing: "still" })) return home.send({ type: "done" })
      if (event.key === "Escape" && home.getSnapshot().matches({ editing: "overview" })) return home.send({ type: "dismiss" })
      if (!home.getSnapshot().matches("nothing")) return
      const to = ({ ArrowRight: { page: rtl() }, ArrowLeft: { page: -rtl() }, ArrowDown: { owner: 1 }, ArrowUp: { owner: -1 } } as Record<string, { page: 1 | -1 } | { owner: 1 | -1 }>)[event.key]
      if (!to || ("owner" in to && kinds[kindOf(home.getSnapshot().context.at.owner)].scrolls)) return
      event.preventDefault()
      home.send({ type: "step", ...to })
    }
    addEventListener("keydown", keys)
    return () => (sliding.unsubscribe(), arranging.unsubscribe(), removeEventListener("resize", resized), root.removeEventListener("wheel", wheel), removeEventListener("keydown", keys))
  })

  // While pages are edited, the device's own back ends editing, as Done does.
  createEffect(
    () => state().matches({ editing: "still" }),
    (now) => (now ? covers(() => home.send({ type: "done" })) : undefined),
  )

  const opened = () => state().context.category
  const held = () => (state().matches("holding") ? state().context.held : undefined)
  const owner = () => props.owners[state().context.at.owner]
  return (
    <div class={["home", { wide: room().wide, editing: state().matches("editing"), carrying: state().matches({ editing: "dragging" }) }]} ref={root} onClick={(event) => Date.now() > quiet && state().matches({ editing: "still" }) && !(event.target as Element).closest(".cell, .top, .dots, .overview, .ui-dialog") && home.send({ type: "done" })}>
      <main class="owners" ref={main} onScroll={moved} onScrollEnd={rested}>
        <For each={props.owners} keyed={(owner) => owner.key}>
          {(owner, index) => (
            <section class="owner" aria-label={owner().name}>
              <div class="strip" ref={(strip) => (strips[index()] = strip)} style={{ "--pages": Math.min(room().fit, laid()[index()]?.length ?? 1) }} onScrollEnd={rested}>
                <For each={pages()[index()] ?? [[]]} keyed={false}>
                  {(placed, page) => <HomePage owner={owner()} index={index()} page={page} placed={placed()} />}
                </For>
                <Library owner={owner()} index={index()} places={places} />
              </div>
              <Dots owner={owner()} index={index()} kinds={laid()[index()] ?? []} hidden={!kinds[kindOf(index())].dots || (laid()[index()]?.length ?? 0) <= room().fit} />
            </section>
          )}
        </For>
      </main>
      <Switcher owners={props.owners} />
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[opened().owner]} opened={opened()} places={places} />}</Show>
      <Show when={held()}>{(held) => <Holding owner={owner()} index={state().context.at.owner} held={held()} />}</Show>
      <Show when={state().matches({ editing: "overview" }) && editing()}>
        {(edited) => <Overview owner={props.owners[edited().owner]} index={edited().owner} />}
      </Show>
      <Show when={editing()}>{(edited) => <Gallery owner={props.owners[edited().owner]} />}</Show>
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
