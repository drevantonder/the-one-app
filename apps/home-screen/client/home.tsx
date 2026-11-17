// The Home Screen: yours, everyone's, then each of your groups', one owner above another, each a row of three
// sections: Home, their pages; their App Library; and the App Store. Each section is a screenful or more, never less:
// as many of its pages side by side as fit, each a phone's page drawn larger to suit the window's height on a wide
// one, and centred where there are fewer. Left and right moves a screenful, as a phone's pages do, through Home's
// screenfuls and on to the App Library and the App Store, and up and down between owners, unless a page scrolls
// itself. The pages move by hand, as a phone's do (pager.ts). The owner's name, in the bar at the bottom beside the
// sections' icons, opens a list of every owner. A page never scrolls: it's a fixed grid, and what doesn't fit goes on
// to the next. Whoever arranges an owner's pages edits them as a phone's Home Screen is edited:
// touch and hold an app or a widget (press and hold it, or right-click it, with a mouse), then Edit Home Screen, or
// hold it a while longer, or touch and hold a page's empty space. Where it is, what's open over it and the editing are
// one state machine (machine.ts), whose snapshot is one signal here.
import { createEffect, createMemo, createSignal, Errored, flush, For, Loading, onSettled, refresh, Show, untrack } from "solid-js"
import { Dynamic, render, type JSX } from "@solidjs/web"
import { createActor } from "xstate"
import { Button, call, changed, client, confirm, covers, Empty, haptic, Icon, List, Menu, open, pace, Problem, Row, Section, Sheet, toast, Widget } from "ui"
import "./home.css"
import { give, glide, jump, settle, speed, spring, type Spring, stop } from "./pager.ts"
import { offset, reflow } from "./reflow.ts"
import { type At, type Editing, fitted, type Held, kinds, type Kind, machine, type Opened, pack, type Page, type Placed, shown, spans } from "./machine.ts"
import type { Entrypoint, Category, Item, Listed, Pages, Size, Widget as WidgetItem } from "./types.ts"

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
  if (source === parent && data?.method === "notifications/one/showing") home.send({ type: data.params?.entrypoint === HOME ? "home" : "launch" })
})

/// The owner and screenful nearest where the pages are as they move, which the dots follow during a swipe, as a
/// phone's do, rather than once it ends.
const [seen, setSeen] = createSignal<At>({ owner: 0, screen: 0 }, { equals: (a, b) => a.owner === b.owner && a.screen === b.screen })
/// How much a wide window draws the Home Screen larger than a phone does: a phone's pages, scaled to the window's
/// height, twice at most, so a bigger screen shows more pages rather than larger icons. Set by `measure`.
let scale = 1

/// The owner switcher's names and the owners' sections, which roll together as the owners move: each a window onto a
/// strip with a row for every owner. `at` is where the owners are, in owners. Each window is as wide as the row it
/// shows, or between two rows' widths as the owners pass, so the bar fits whoever's shown, and a row fades as it
/// rolls away, as a picker's do.
const rolls = new Set<HTMLElement>()
const widths = new WeakMap<HTMLElement, number[]>()
let rolled = 0
function roll(at = rolled) {
  rolled = at
  for (const window of rolls) {
    const strip = window.firstElementChild as HTMLElement
    const sizes = widths.get(window) ?? []
    strip.style.translate = `0 ${(-at * 100) / Math.max(1, strip.children.length)}%`
    for (const [index, row] of [...strip.children].entries()) (row as HTMLElement).style.opacity = `${Math.max(0, 1 - Math.abs(index - at) * 1.5)}`
    if (!sizes.length) continue
    const near = Math.max(0, Math.min(sizes.length - 1, at))
    const [low, high] = [Math.floor(near), Math.ceil(near)]
    window.style.width = `${sizes[low] + (sizes[high] - sizes[low]) * (near - low)}px`
  }
}
/// Each row's own width, measured whenever one changes, in the bar's own points: it's drawn larger on a wide window,
/// perhaps still on its way there.
const sized = new ResizeObserver(() => {
  for (const window of rolls) {
    const bar = window.closest<HTMLElement>(".toolbar")!
    const drawn = bar.getBoundingClientRect().height / bar.offsetHeight || 1
    widths.set(window, [...window.firstElementChild!.children].map((row) => row.getBoundingClientRect().width / drawn))
  }
  roll()
})
const rolling = (window: HTMLElement) => rolls.add(window)

const still = matchMedia("(prefers-reduced-motion: reduce)")
const rtl = () => (document.dir === "rtl" ? -1 : 1)
const editing = () => state().context.editing
const editingOwner = (owner: number) => state().matches("editing") && editing()?.owner === owner

/// Opens an app for whoever its owner is, for the group whose install it is, or for its person where it's about
/// whoever opens it. `behind` only starts it, as a finger lands on it. It grows out of its icon, `from`, as an
/// iPhone's does.
const launch = (entrypoint: Entrypoint, owner: Owner, behind?: boolean, from?: Element) => {
  const icon = from?.getBoundingClientRect()
  open(entrypoint.entrypoint, { subject: entrypoint.subject === "user" ? undefined : (entrypoint.subject ?? owner.subject), behind, from: icon && { x: icon.x, y: icon.y, width: icon.width, height: icon.height } })
}
/// The UI library's pace of that name, from its tokens.
const paced = (name: "quick" | "settle" | "travel") => pace(document.documentElement, name)

/// An owner's pages as they're kept: each with what's on it and whether it's hidden, to edit.
const draft = (owner: Owner, index: number): Editing => ({ owner: index, pages: fitted(owner.pages.map((page) => ({ items: [...page.items], hidden: page.hidden }))) })

/// An owner's pages as they were just edited, shown until the Home Screen is asked again.
const [kept, setKept] = createSignal<Editing>()

// A finger that stays this long holds what it's on, as a phone's does, and on a Home page, held this much longer
// still, it's lifted into editing, as an iPhone's menu gives way to the drag. An icon sinks under the finger for as
// long as it takes to hold (home.css's `--hold`).
const HOLD = 450
const LONGER = 800

/// A widget on a page: what its app's tool answers, drawn, and asked again each minute it's in view. Pressing it
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
function Tile(props: { name: string; onPress?: (behind?: boolean, icon?: Element) => void; children: JSX.Element }) {
  return (
    <button type="button" class="tile" onPointerDown={(event) => event.button === 0 && props.onPress?.(true)} onClick={(event) => props.onPress?.(false, event.currentTarget.querySelector("img") ?? undefined)}>
      <span class="icon">{props.children}</span>
      <span>{props.name}</span>
    </button>
  )
}

/// Taking an item off a Home page asks first, as an iPhone's does: an app stays in the App Library, and a widget's app
/// and data stay.
const removal = (item: Item): Parameters<typeof confirm>[0] =>
  item.widget ? { heading: "Remove Widget?", action: "Remove", destructive: true } : { heading: `Remove “${item.name}”?`, text: "It stays in your App Library.", action: "Remove from Home Screen" }

/// What a Home page holds as it's drawn: an item of the owner's, and where it is among the page's.
type Cell = { item: Item; index: number }

/// Watches a finger or a mouse on what it's given, for the Home Screen's gestures: a press held still is a hold, as is
/// a right-click; a press that moves while editing, or once held, lifts it, and so does one held still a while longer
/// on a Home page. A mouse holds as a finger does, as an iPad's pointer does. `held` says what it is, as it is then. A
/// hold on a page's empty space has nothing to lift or offer, so it edits the page at once, as a phone's does. Only
/// whoever `arranges` lifts anything; a menu is held open for whoever it has one for (`menu`), as an App Library's has
/// for everyone.
function holds(held: () => Held | undefined, arranges: () => boolean, menu = arranges) {
  return (element: HTMLElement) => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let start: PointerEvent | undefined
    // Whether this press has held: a mouse's click as it lets go is the hold's end, not a tap.
    let fired = false
    const clear = () => {
      clearTimeout(timer)
      start = undefined
      fired = false
      removeEventListener("pointermove", move)
      removeEventListener("pointerup", up)
      removeEventListener("pointercancel", clear)
    }
    const rect = () => {
      const { x, y, width, height } = element.getBoundingClientRect()
      return { x, y, width, height }
    }
    // `at` is where the pointer is now: a press held still a while longer lifts with no move to say so.
    const lift = (at: PointerEvent, from: PointerEvent) => {
      clear()
      const what = held()
      if (!what?.item || !arranges()) return
      grab = { x: from.clientX, y: from.clientY }
      pointer = { x: at.clientX, y: at.clientY }
      home.send({ type: "lift", held: { ...what, rect: rect() }, editing: state().matches("editing") ? undefined : pending?.() })
      // The Home Screen keeps the pointer from here on, whatever's drawn again under it.
      document.querySelector<HTMLElement>(".home")?.setPointerCapture(at.pointerId)
    }
    const hold = () => {
      const what = held()
      if (!menu() || !what || !state().matches("nothing")) return
      fired = !!start
      if (!what.item && !what.listed) return arranges() && pending?.() && home.send({ type: "edit", editing: pending()! })
      home.send({ type: "hold", held: { ...what, rect: rect() } })
      const from = start
      if (from && what.from === "page") timer = setTimeout(() => state().matches("holding") && lift(from, from), LONGER)
    }
    const move = (event: PointerEvent) => {
      if (!start || event.pointerId !== start.pointerId) return
      const moved = Math.hypot(event.clientX - start.clientX, event.clientY - start.clientY)
      if (moved < 8) return
      // Held, or editing, a press that moves carries what it's on; before that, it's a swipe.
      if (state().matches("holding") || (state().matches({ editing: "still" }) && held()?.item)) lift(event, start)
      else clear()
    }
    const up = (event: PointerEvent) => {
      if (event.pointerId !== start?.pointerId) return
      if (fired) {
        const swallow = (event: MouseEvent) => (event.stopPropagation(), event.preventDefault())
        addEventListener("click", swallow, { capture: true, once: true })
        setTimeout(() => removeEventListener("click", swallow, true))
      }
      clear()
    }
    element.addEventListener("contextmenu", (event) => {
      event.preventDefault()
      event.stopPropagation()
      clear()
      hold()
    })
    element.addEventListener("pointerdown", (event) => {
      // ⊖ is pressed, not held or lifted.
      if (event.button || !menu() || (event.target as Element).closest(".minus")) return
      clear()
      start = event
      // Once held, the menu lies over what's held, and an iPhone's web view sends the press's moves and its end to
      // whatever's under the finger then, whatever has the pointer: so the press is followed wherever it goes.
      addEventListener("pointermove", move)
      addEventListener("pointerup", up)
      addEventListener("pointercancel", clear)
      // Editing, a press that moves lifts at once, wherever the pointer goes first; otherwise a press held still is a
      // hold.
      if (!state().matches("editing")) timer = setTimeout(hold, HOLD)
    })
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
/// Where the pointer was last, and where it pressed what it lifted: what's carried stays where the finger took it.
let pointer = { x: 0, y: 0 }
let grab = { x: 0, y: 0 }
addEventListener("pointermove", (event) => (pointer = { x: event.clientX, y: event.clientY }), { capture: true, passive: true })

/// What an element is kept in, `where`, under `key`, for as long as it's drawn: it moves when its key changes.
function registered<K extends string | number>(where: Record<K, HTMLElement>, key: () => K) {
  let element!: HTMLElement
  createEffect(key, (at) => {
    where[at] = element
    return () => delete where[at]
  })
  return (drawn: HTMLElement) => (element = drawn)
}

/// A Home page: its grid, each item where it was placed. While its owner's pages are edited, everything jiggles with
/// its ⊖, and what's lifted leaves its place empty. An owner with nothing on their pages has one grey line.
function HomePage(props: { owner: Owner; index: number; page: number; placed: Placed<Cell>[]; small?: boolean }) {
  const editingHere = () => !props.small && editingOwner(props.index)
  const lifted = (cell: Cell) => {
    const held = state().context.held
    return editingHere() && state().matches({ editing: "dragging" }) && held?.page === props.page && held.index === cell.index
  }
  const arranges = () => props.owner.arranges && !props.small
  const removing = async (item: Item, index: number) => {
    if (await confirm(removal(item))) home.send({ type: "remove", page: props.page, index })
  }
  return (
    <section class={["page", { small: props.small }]} data-page={props.page} ref={holds(() => (editingHere() ? undefined : { page: props.page, index: -1, from: "page" }), arranges)}>
      <Show when={props.placed.length || editingHere()} fallback={<Empty>No apps on this page.</Empty>}>
        <div class="grid">
          <For each={props.placed} keyed={({ item }) => (item.item.widget ?? item.item.entrypoint) + (item.item.size ?? "")}>
            {(placed) => (
              <div
                class={["cell", { jiggles: editingHere(), lifted: lifted(placed().item) }]}
                data-index={placed().item.index}
                data-size={placed().item.item.size}
                style={{ "--column": placed().column, "--row": placed().row, "--width": placed().width, "--height": placed().height, "--wobble": `${(placed().item.index * 97) % 240}ms` }}
                ref={holds(() => ({ item: placed().item.item, page: props.page, index: placed().item.index, from: "page" }), arranges)}
              >
                <Show when={placed().item.item}>
                  {(item) => (
                    <>
                      <Show when={item().widget} fallback={<Tile name={(item() as Entrypoint).name} onPress={(behind, icon) => state().matches("nothing") && launch(item() as Entrypoint, props.owner, behind, icon)}><img src={(item() as Entrypoint).icon} alt="" /></Tile>}>
                        <Card item={item() as WidgetItem} owner={props.owner} still={props.small || editingHere()} />
                      </Show>
                      <Show when={editingHere()}>
                        <button type="button" class="minus" aria-label={`Remove ${item().name}`} onClick={() => removing(item(), placed().item.index)}>
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

/** A category as an App Library page shows it: in a square of four, or wide, across the page, in one of eight. */
type Folder = { category: Category; wide: boolean }

/// An App Library's pages: its categories in order, six to a page, two across and three down. A category of more than
/// four apps spans the page's width and shows eight, where the library needs more than one page anyway and that adds
/// none: the largest first, as many as fit. A row a wide one leaves half empty takes the next small one.
function shelve(library: Category[]): Folder[][] {
  const paged = (wide: Set<Category>) => {
    const rows: Folder[][] = []
    let half: Folder[] | undefined
    for (const category of library) {
      if (wide.has(category)) rows.push([{ category, wide: true }])
      else if (half) (half.push({ category, wide: false }), (half = undefined))
      else rows.push((half = [{ category, wide: false }]))
    }
    return Array.from({ length: Math.max(1, Math.ceil(rows.length / 3)) }, (_, page) => rows.slice(page * 3, page * 3 + 3).flat())
  }
  const wide = new Set<Category>()
  const fewest = paged(wide).length
  if (fewest > 1)
    for (const category of library.filter((category) => category.entrypoints.length > 4).sort((a, b) => b.entrypoints.length - a.entrypoints.length)) {
      wide.add(category)
      if (paged(wide).length > fewest) wide.delete(category)
    }
  return paged(wide)
}

/// An App Library page: its categories, each its first four apps' icons with their names, or wide, its first eight,
/// or with more than that, all but the last and the rest smaller in the last's place. Tapping an icon opens its app;
/// tapping anywhere else in a category opens it. Touching and holding an app lifts it, to drag onto a Home page.
function Library(props: { owner: Owner; index: number; folders: Folder[]; places: Record<string, HTMLElement> }) {
  // Tapped again as it shrinks back into its place, it turns and grows again.
  const opening = (category: () => Category) => () => {
    const opened = state().context.category
    const back = state().matches({ category: "closing" }) && opened?.owner === props.index && opened.name === category().category
    if (state().matches("nothing") || back) home.send({ type: "open", category: { owner: props.index, name: category().category } })
  }
  return (
    <section class="page library" aria-label={`${props.owner.name}'s App Library`}>
      <Show when={props.folders.length} fallback={<Empty>No apps yet.</Empty>}>
        <div class="categories">
          <For each={props.folders} keyed={({ category }) => category.category}>
            {(folder) => {
              const room = () => (folder().wide ? 8 : 4)
              const category = () => folder().category
              const shown = () => (category().entrypoints.length > room() ? category().entrypoints.slice(0, room() - 1) : category().entrypoints)
              return (
                <div class={["category", { wide: folder().wide }]}>
                  <div class="place" ref={registered(props.places, () => `${props.index}:${category().category}`)} onClick={opening(category)}>
                    <For each={shown()} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                      {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} index={props.index} />}
                    </For>
                    <Show when={category().entrypoints.length > room()}>
                      <button type="button" class="more" aria-label={`All of ${category().category}`}>
                        <For each={category().entrypoints.slice(room() - 1, room() + 3)} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                          {(entrypoint) => <img src={entrypoint().icon} alt="" />}
                        </For>
                      </button>
                    </Show>
                  </div>
                  <button type="button" class="name" onClick={opening(category)}>{category().category}</button>
                </div>
              )
            }}
          </For>
        </div>
      </Show>
    </section>
  )
}

/// An app's icon in an App Library: a tap opens it, or for an app with no entrypoint, its App Store page, and a finger
/// that holds it opens its menu, from which whoever arranges the owner's pages lifts it onto them.
function LibraryEntrypoint(props: { entrypoint: Listed; owner: Owner; index: number }) {
  return (
    <button
      type="button"
      class="app"
      ref={holds(() => ({ item: opens(props.entrypoint), listed: props.entrypoint, page: 0, index: -1, from: "library" }), () => props.owner.arranges, () => true)}
      onPointerDown={() => state().matches("nothing") && opens(props.entrypoint) && launch(opens(props.entrypoint)!, props.owner, true)}
      onClick={(event) => (event.stopPropagation(), state().matches("nothing") && press(props.entrypoint, props.owner, false, event.currentTarget.querySelector("img") ?? undefined))}
    >
      <img src={props.entrypoint.icon} alt="" />
      <span>{props.entrypoint.name}</span>
    </button>
  )
}

/// An app of an App Library as an entrypoint, which it opens, or nothing, for an app with none.
const opens = (listed: Listed): Entrypoint | undefined => (listed.entrypoint ? (listed as Entrypoint) : undefined)
/// Opens an app of an App Library: its entrypoint, or for an app with none, its App Store page over the Home Screen.
const press = (listed: Listed, owner: Owner, behind?: boolean, icon?: Element) => (opens(listed) ? launch(opens(listed)!, owner, behind, icon) : !behind && setAbout(listed.app))

/// Who a call about an install is for: a group or the network, or for a user's own, nobody said.
const subjectOf = (installed: string) => (installed.startsWith("users/") ? undefined : installed)
const and = (words: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(words)

/// The app whose App Store page is open over the Home Screen, by its artifact.
const [about, setAbout] = createSignal<string>()

/// An app's App Store page, over the Home Screen, as the App Store draws it (its `Listing` export), for an app with no
/// entrypoint of its own: whether it's installed, its version, Update, Uninstall, what it requires and what requires
/// it. Back, or the device's own, closes it.
function About(props: { app: string; store: string }) {
  const listing = createMemo(() => import(/* @vite-ignore */ new URL(`../../${props.store}/client/store.js`, document.baseURI).href).then((module) => module.Listing))
  onSettled(() => covers(() => setAbout()))
  return (
    <div class="about">
      <Errored fallback={(error) => <Problem heading="Can't show it" text={error() as Error} />}>
        <Loading>
          <Dynamic component={listing()} app={props.app} onBack={() => setAbout()} />
        </Loading>
      </Errored>
    </div>
  )
}

/// The App Store's screen, the page after the App Library: the App Store app's own (its `store` export), beside this
/// page where a client serves each app's. It's drawn only while its owner is the one shown, as it's the same for all.
function StorePage(props: { name: string; shown: boolean }) {
  const store = createMemo(() => (props.shown ? import(/* @vite-ignore */ new URL(`../../${props.name}/client/store.js`, document.baseURI).href).then((module) => module.default) : undefined))
  return (
    <section class="page store scrolls" aria-label="App Store">
      <Show when={props.shown}>
        <Errored fallback={(error) => <Problem heading="Can't show the App Store" text={error() as Error} />}>
          <Loading>
            <Dynamic component={store()} />
          </Loading>
        </Errored>
      </Show>
    </section>
  )
}

/// A category opened over its App Library: its name, and its apps in a panel that grows out of its place there and
/// shrinks back into it, as an iPhone's folder does, while what's behind it blurs. Its apps open only once it has
/// grown. Tapping beside the panel, Escape or the device's own back closes it, even as it grows: it turns back from
/// where it got to. For whoever asked for less motion, it fades in and out where it is. It's a modal `<dialog>`, so
/// focus stays in it and the rest is inert while it's open; as it shrinks it's a dialog that isn't modal, so the pages
/// under it take a finger.
function OpenCategory(props: { owner: Owner; opened: Opened; places: Record<string, HTMLElement> }) {
  let veil!: HTMLDialogElement
  let panel!: HTMLDivElement
  let title!: HTMLHeadingElement
  let grid!: HTMLDivElement
  const category = () => props.owner.library.find((category) => category.category === props.opened.name)
  const phase = () => (state().matches({ category: "opening" }) ? "opening" : state().matches({ category: "open" }) ? "open" : "closing")
  const close = () => !state().matches({ category: "closing" }) && home.send({ type: "close" })
  // What moves it, while it moves: turned back midway, each runs back from where it is.
  let moving: Animation[] = []
  // From its place to the middle, or back, and once that's done, whichever way it ended up going, the machine's told.
  const zoom = (opening: boolean) => {
    // Modal while it's open or opening, and not as it shrinks: opened again, it's shown the other way.
    if (veil.isConnected) {
      if (veil.open) veil.close()
      if (opening) veil.showModal()
      else veil.show()
    }
    if (moving.some((one) => one.playState === "running")) for (const one of moving) one.reverse()
    else {
      const place = props.places[`${props.opened.owner}:${props.opened.name}`]
      if (!place || !panel.isConnected) return void home.send({ type: "zoomed" })
      for (const one of moving) one.cancel()
      const [from, to] = [place.getBoundingClientRect(), panel.getBoundingClientRect()]
      // It's drawn `scale` times larger, its own points with it, so the way across the window is that many fewer of them.
      const moved = `translate(${(from.x + from.width / 2 - (to.x + to.width / 2)) / scale}px, ${(from.y + from.height / 2 - (to.y + to.height / 2)) / scale}px) scale(${from.width / to.width}, ${from.height / to.height})`
      const timing: KeyframeAnimationOptions = { ...paced("travel"), fill: "both" }
      const way = <T,>(frames: T[]) => (opening ? frames : frames.toReversed())
      const appear: Keyframe[] = [{ opacity: 0 }, { opacity: 1 }]
      moving = [
        veil.animate(way(appear), timing),
        title.animate(way(still.matches ? appear : [{ opacity: 0, transform: "translateY(12px)" }, { opacity: 1, transform: "none" }]), timing),
        grid.animate(way(appear), { ...timing, duration: Number(timing.duration) * 0.7, delay: opening ? Number(timing.duration) * 0.3 : 0 }),
        panel.animate(way(still.matches ? appear : [{ transform: moved, borderRadius: "1.5rem" }, { transform: "none", borderRadius: "2.25rem" }]), timing),
      ]
    }
    const now = moving
    Promise.all(now.map((one) => one.finished)).then(() => moving === now && home.send({ type: "zoomed" }), () => {})
  }
  // Driven by a memo, which changes only when the phase does, not at every event of the machine.
  createEffect(createMemo(phase), (now) => {
    if (now !== "open") zoom(now === "opening")
  })
  onSettled(() => covers(close))
  return (
    // Escape, and the browser's own close requests, shrink it back as a tap beside it does, rather than closing it at once.
    <dialog class="opened" ref={veil} aria-labelledby="opened-category" data-phase={phase()} onCancel={(event) => (event.preventDefault(), close())} onClick={(event) => event.target === veil && close()}>
      <h2 id="opened-category" ref={title}>{props.opened.name}</h2>
      <div class="panel" ref={panel} onClick={(event) => event.target === panel && close()}>
        <div class="grid" ref={grid}>
          <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
            {(entrypoint) => (
              <div class="cell">
                <Tile name={entrypoint().name} onPress={(behind, icon) => phase() === "open" && press(entrypoint(), props.owner, behind, icon)}>
                  <img src={entrypoint().icon} alt="" />
                </Tile>
              </div>
            )}
          </For>
        </div>
      </div>
    </dialog>
  )
}

/// A spring as an easing, for what opens out of something, as iOS's menus do: quick, a little past where it goes and
/// back.
const springy = `linear(${Array.from({ length: 41 }, (_, step) => {
  const [t, damping, natural] = [step / 40, 0.7, 10]
  const damped = natural * Math.sqrt(1 - damping ** 2)
  return (1 - Math.exp(-damping * natural * t) * (Math.cos(damped * t) + ((damping * natural) / damped) * Math.sin(damped * t))).toFixed(4)
}).join(", ")})`

/// What a finger holds, lifted over the blurred Home Screen, as a phone's is, with its menu beside it, which grows out
/// of it with a spring and shrinks back into it: Edit Home Screen and Remove App on a Home page, Add to Home Screen and
/// Remove App in the App Library. Moving the finger, or on a Home page holding it a while longer, carries it into
/// editing; anywhere else, Escape, a tap beside it or the device's own back puts it down. The menu is a popover, in the
/// top layer, which the browser closes on Escape or a tap beside it; the arrow keys move between its items.
function Holding(props: { owner: Owner; index: number; held: Held }) {
  let veil!: HTMLDivElement
  let menu!: HTMLDivElement
  let copy: HTMLDivElement | undefined
  const dismiss = () => state().matches("holding") && home.send({ type: "dismiss" })
  const edit = () => home.send({ type: "edit", editing: draft(props.owner, props.index) })
  // Off its Home page, once that's what's wanted.
  const removing = async () => {
    const [owner, editing, held] = [props.owner, draft(props.owner, props.index), props.held]
    dismiss()
    if (!(await confirm(removal(held.item!)))) return
    const at = shown(editing.pages)[held.page]
    const pages = editing.pages.map((page, index) => (index === at ? { ...page, items: page.items.toSpliced(held.index, 1) } : page))
    keep({ ...editing, pages: pages.filter((page) => page.items.length) }, owner)
  }
  // The App Library holds every app that's installed: it goes from there by being uninstalled, once its alert has
  // said what goes with it, the app's other entrypoints and the apps that need it, into Recently Deleted.
  const uninstalls = async () => {
    const listed = props.held.listed!
    const subject = subjectOf(listed.installed.for)
    dismiss()
    try {
      const { apps } = await json<{ apps: { name: string; for: string; required_by: string[] }[] }>("apps", {}, subject)
      const needs = apps.find((one) => one.for === listed.installed.for && one.name === listed.installed.name)?.required_by ?? []
      // Every app of every App Library shown here, as an app installed for everyone that's about whoever opens it is
      // only in theirs.
      const all = (owners() ?? [props.owner]).flatMap((owner) => owner.library.flatMap((category) => category.entrypoints))
      const parts = [...new Set(all.filter((other) => other.app === listed.app && other.entrypoint && other.entrypoint !== listed.entrypoint).map((other) => other.name))]
      const needing = [...new Set(needs.map((name) => all.find((other) => other.installed.for === listed.installed.for && other.installed.name === name)?.title ?? name))]
      const text =
        [
          parts.length ? `${and(parts)} ${parts.length === 1 ? "is" : "are"} part of it and will be removed too.` : "",
          needing.length ? `${and(needing)} ${needing.length === 1 ? "needs it, so it" : "need it, so they"}'ll be removed too.` : "",
        ]
          .filter(Boolean)
          .join(" ") || "It stays in Recently Deleted for 30 days."
      if (!(await confirm({ heading: `Remove ${listed.title}?`, text, action: "Remove App", destructive: true }))) return
      await call("uninstall", { name: listed.installed.name, confirm: needs.length > 0 }, { subject })
    } catch (error) {
      toast(error as Error)
    }
    refresh(owners)
  }
  // Why whoever holds it can't remove it: it's a group's or everyone's, and they're not one of its owners.
  const cant = () => {
    const listed = props.held.listed
    if (!listed || listed.installed.removes) return undefined
    const name = owners()?.find((owner) => (owner.subject ?? owner.who) === listed.installed.for)?.name ?? listed.installed.for
    return `This is ${name}'s, and you're not one of its owners.`
  }
  // From the App Library, it goes last on the owner's last page, as a phone's does.
  const adding = () => {
    const editing = draft(props.owner, props.index)
    const at = shown(editing.pages).at(-1)
    const pages = at === undefined ? [...editing.pages, { items: [props.held.item!] }] : editing.pages.map((page, index) => (index === at ? { ...page, items: [...page.items, props.held.item!] } : page))
    dismiss()
    keep({ ...editing, pages: fitted(pages) }, props.owner)
  }
  onSettled(() => {
    haptic("medium")
    menu.showPopover()
    // It has the keys, so the arrow keys reach its items, with none of them picked out until they're used.
    menu.focus({ preventScroll: true })
    const fade = paced("quick")
    veil.animate([{ opacity: 0 }, {}], paced("settle"))
    menu.animate([{ opacity: 0 }, {}], fade)
    if (!still.matches) {
      menu.animate([{ scale: "0.25" }, {}], { duration: 450, easing: springy })
      copy?.animate([{ scale: "1" }, {}], { duration: 450, easing: springy })
    }
    const shut = covers(dismiss)
    return () => {
      shut()
      close()
    }
  })
  // As it goes, a copy of it shrinks back into what was held and the blur clears, from wherever it had got to: unless
  // it's being carried, when what follows the finger takes the copy's place.
  const close = () => {
    const now = (element: Element | undefined) => (element?.isConnected ? getComputedStyle(element) : undefined)
    const [menuNow, copyNow] = [now(menu), now(copy)]
    const settling: KeyframeAnimationOptions = { ...paced("travel"), fill: "forwards" }
    const ghost = veil.cloneNode(true) as HTMLDivElement
    ghost.inert = true
    ghost.style.pointerEvents = "none"
    document.querySelector(".home")?.append(ghost)
    const [ghostMenu, ghostCopy] = [ghost.querySelector<HTMLElement>(".hold-menu")!, ghost.querySelector<HTMLElement>(".lifted-copy")]
    // Its copy shrinks back where it was drawn, over the rest, at its own size: not as a popover, which a copy isn't
    // shown as, and not in what's held, which is drawn larger on a wide window.
    ghostMenu.removeAttribute("popover")
    ghost.after(ghostMenu)
    if (home.getSnapshot().matches({ editing: "dragging" })) ghostCopy?.remove()
    ghost.animate([{ opacity: now(veil)?.opacity ?? 1 }, { opacity: 0 }], settling).finished.finally(() => ghost.remove())
    ghostMenu.animate([{ opacity: menuNow?.opacity ?? 1, scale: menuNow?.scale ?? "1" }, { opacity: 0, scale: still.matches ? "1" : "0.25" }], settling).finished.finally(() => ghostMenu.remove())
    if (!still.matches) ghostCopy?.animate([{ scale: copyNow?.scale ?? "1.06" }, { scale: "1" }], settling)
  }
  // Where it is in the window, which its menu is under, or over where there's no room under it; and in the points of
  // the layer it's lifted onto, which is drawn as much larger as the pages are. The menu's in the top layer, at its
  // own size.
  const held = props.held.rect ?? { x: 0, y: 0, width: 0, height: 0 }
  const rect = { x: held.x / scale, y: held.y / scale, width: held.width / scale, height: held.height / scale }
  const [width, height] = [document.documentElement.clientWidth, document.documentElement.clientHeight]
  const below = held.y + held.height < height * 0.6
  const left = Math.max(12, Math.min(width - 252, held.x))
  // Up and down move between what can be done, as a menu's keys do, and Home and End to the first and the last.
  const keys = (event: KeyboardEvent) => {
    const items = [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]
    const at = items.indexOf(document.activeElement as HTMLButtonElement)
    const to = { ArrowDown: at + 1, ArrowUp: at < 0 ? items.length - 1 : at - 1, Home: 0, End: items.length - 1 }[event.key]
    if (to === undefined || !items.length) return
    event.preventDefault()
    items[(to + items.length) % items.length].focus()
  }
  const item = props.held.item
  // An app of the App Library that's on a page already has no Add to Home Screen.
  const added = !!item && props.owner.pages.some((page) => page.items.some((other) => other.entrypoint === item.entrypoint))
  const library = props.held.from === "library"
  return (
    <div class="holding" ref={veil} onClick={(event) => event.target === event.currentTarget && dismiss()}>
      <Show when={item ?? (props.held.listed as Item | undefined)}>
        {(item) => (
          <div class="lifted-copy" ref={copy} style={{ left: `${rect.x}px`, top: `${rect.y}px`, width: `${rect.width}px`, height: `${rect.height}px` }}>
            <Show when={item().widget} fallback={library ? <img class="alone" src={(item() as Entrypoint).icon} alt="" /> : <Tile name={(item() as Entrypoint).name}><img src={(item() as Entrypoint).icon} alt="" /></Tile>}>
              <Card item={item() as WidgetItem} owner={props.owner} still />
            </Show>
          </div>
        )}
      </Show>
      <div
        class="hold-menu"
        role="menu"
        aria-label={(item ?? props.held.listed)?.name}
        popover="auto"
        tabindex="-1"
        ref={menu}
        onToggle={(event) => event.newState === "closed" && dismiss()}
        onKeyDown={keys}
        style={{ left: `${left}px`, "transform-origin": `${held.x + held.width / 2 - left}px ${below ? "0" : "100%"}`, ...(below ? { top: `${held.y + held.height + 12}px` } : { bottom: `${height - held.y + 12}px` }) }}
      >
        <Show when={!library}>
          <button type="button" role="menuitem" onClick={edit}>
            <span>Edit Home Screen</span>
            <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true"><path d="M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z" /></svg>
          </button>
        </Show>
        <Show when={library && item && !added && props.owner.arranges}>
          <button type="button" role="menuitem" onClick={adding}>
            <span>Add to Home Screen</span>
            <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true"><path d="M128,24A104,104,0,1,0,232,128,104.11,104.11,0,0,0,128,24Zm0,192a88,88,0,1,1,88-88A88.1,88.1,0,0,1,128,216Zm48-88a8,8,0,0,1-8,8H136v32a8,8,0,0,1-16,0V136H88a8,8,0,0,1,0-16h32V88a8,8,0,0,1,16,0v32h32A8,8,0,0,1,176,128Z" /></svg>
          </button>
        </Show>
        <Show when={item || props.held.listed}>
          <button type="button" role="menuitem" class={cant() ? undefined : "destructive"} disabled={!!cant()} onClick={library ? uninstalls : removing}>
            <span class="label">
              <span>{item?.widget ? "Remove Widget" : "Remove App"}</span>
              <Show when={cant()}>{(why) => <small>{why()}</small>}</Show>
            </span>
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
  const pages = editing.pages.map((page) => ({ items: page.items.map((item) => (item.widget ? { widget: item.widget, size: item.size } : { entrypoint: item.entrypoint })), ...(page.hidden ? { hidden: true } : {}) }))
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
  const [dragging, setDragging] = createSignal<Page>()
  const remove = async (page: number) => {
    if (await confirm({ heading: "Delete Page?", text: "Its apps stay in the App Library.", action: "Delete", destructive: true })) home.send({ type: "delete", page })
  }
  // A page dragged is lifted and follows the finger one to one, from where it was taken; as it passes over another,
  // it takes that one's place then, and the others glide aside (reflow.ts). Let go, it springs into its place from
  // where it was, at the speed it was going, as an icon dropped on a page does.
  const drag = (page: Page) => (down: PointerEvent & { currentTarget: HTMLElement }) => {
    if (down.button) return
    const from = down.currentTarget
    const thumbnail = from.closest<HTMLElement>(".thumbnail")!
    const others = () => [...thumbnail.parentElement!.children].filter((one): one is HTMLElement => one !== thumbnail && one instanceof HTMLElement)
    from.setPointerCapture(down.pointerId)
    setDragging(page)
    reflow(others(), false)
    const laid = { x: thumbnail.offsetLeft, y: thumbnail.offsetTop }
    const moves: { time: number; x: number; y: number }[] = []
    const follow = (event: PointerEvent) => {
      const [x, y] = [event.clientX - down.clientX - (thumbnail.offsetLeft - laid.x), event.clientY - down.clientY - (thumbnail.offsetTop - laid.y)]
      thumbnail.style.translate = `${x}px ${y}px`
      return { x, y }
    }
    const move = (event: PointerEvent) => {
      follow(event)
      moves.push({ time: event.timeStamp, x: event.clientX, y: event.clientY })
      if (moves.length > 32) moves.shift()
      const over = (document.elementsFromPoint(event.clientX, event.clientY) as HTMLElement[]).find((under) => under !== thumbnail && under.dataset?.overview !== undefined)
      const [at, to] = [pages().indexOf(page), Number(over?.dataset.overview)]
      if (!over || at < 0 || to === at) return
      home.send({ type: "reorder", from: at, to })
      flush()
      haptic("selection")
      follow(event)
      reflow(others(), true)
    }
    const up = (event: PointerEvent) => {
      from.removeEventListener("pointermove", move)
      from.removeEventListener("pointerup", up)
      from.removeEventListener("pointercancel", up)
      setDragging()
      const at = follow(event)
      const thrown = event.type === "pointerup" ? { x: speed(moves.map(({ time, x }) => ({ time, value: x })), event.timeStamp), y: speed(moves.map(({ time, y }) => ({ time, value: y })), event.timeStamp) } : { x: 0, y: 0 }
      const draw = () => (thumbnail.style.translate = x.value || y.value ? `${x.value}px ${y.value}px` : "")
      const [x, y] = [spring(draw), spring(draw)]
      jump(x, at.x)
      jump(y, at.y)
      glide(x, 0, thrown.x)
      glide(y, 0, thrown.y)
    }
    from.addEventListener("pointermove", move)
    from.addEventListener("pointerup", up)
    from.addEventListener("pointercancel", up)
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
            <div class={["thumbnail", { hidden: page.hidden, dragging: dragging() === page }]} data-overview={index()}>
              <div class="miniature" onPointerDown={drag(page)}>
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

/// Each section's mark, Phosphor's house, squares-four and storefront, and the name a person hears for it.
const SECTIONS: Record<Kind, { name: string; path: string }> = {
  home: {
    name: "Home",
    path: "M219.31,108.68l-80-80a16,16,0,0,0-22.62,0l-80,80A15.87,15.87,0,0,0,32,120v96a8,8,0,0,0,8,8h64a8,8,0,0,0,8-8V160h32v56a8,8,0,0,0,8,8h64a8,8,0,0,0,8-8V120A15.87,15.87,0,0,0,219.31,108.68ZM208,208H160V152a8,8,0,0,0-8-8H104a8,8,0,0,0-8,8v56H48V120l80-80,80,80Z",
  },
  library: {
    name: "App Library",
    path: "M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z",
  },
  store: {
    name: "App Store",
    path: "M232,96a7.89,7.89,0,0,0-.3-2.2L217.35,43.6A16.07,16.07,0,0,0,202,32H54A16.07,16.07,0,0,0,38.65,43.6L24.31,93.8A7.89,7.89,0,0,0,24,96h0v16a40,40,0,0,0,16,32v72a8,8,0,0,0,8,8H208a8,8,0,0,0,8-8V144a40,40,0,0,0,16-32V96ZM54,48H202l11.42,40H42.61Zm50,56h48v8a24,24,0,0,1-48,0Zm-16,0v8a24,24,0,0,1-35.12,21.26,7.88,7.88,0,0,0-1.82-1.06A24,24,0,0,1,40,112v-8ZM200,208H56V151.2a40.57,40.57,0,0,0,8,.8,40,40,0,0,0,32-16,40,40,0,0,0,64,0,40,40,0,0,0,32,16,40.57,40.57,0,0,0,8-.8Zm4.93-75.8a8.08,8.08,0,0,0-1.8,1.05A24,24,0,0,1,168,112v-8h48v8A24,24,0,0,1,204.93,132.2Z",
  },
}

/** A section as the bar shows it: its kind, its first screenful, and how many it has. */
type Run = { kind: Kind; from: number; count: number }
const runs = (kinds: Kind[]): Run[] =>
  kinds.reduce<Run[]>((all, kind, screen) => (all.at(-1)?.kind === kind ? (all.at(-1)!.count++, all) : [...all, { kind, from: screen, count: 1 }]), [])

/// An owner's sections in the bar: each its icon, which stands for its first screenful, as iOS's Spotlight glyph stands
/// for a page dot, then a dot for each screenful after it, only while that section is the one shown; its dots open
/// out of the icon as it's reached and fold back into it as it's left. The screenful shown is lit. Tapping an icon
/// goes to the start of its section and a dot to its screenful, and sliding a finger along them moves a screenful for
/// every 28 points with a tick at each, as a phone's dots do. While the owner's pages are edited, it's Home alone,
/// which opens the page overview.
function Dots(props: { owner: Owner; index: number; kinds: Kind[] }) {
  const current = () => (seen().owner === props.index ? seen().screen : (state().context.last[props.index] ?? 0))
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
      const screen = Math.max(0, Math.min(props.kinds.length - 1, from + Math.trunc(across / 28)))
      if (screen === current()) return
      home.send({ type: "go", screen })
      haptic("selection")
    }
    const up = () => {
      dots.removeEventListener("pointermove", move)
      if (sliding) quiet = Date.now() + 300
    }
    dots.addEventListener("pointermove", move)
    dots.addEventListener("pointerup", up, { once: true })
    dots.addEventListener("pointercancel", up, { once: true })
  }
  const press = (screen: number) => () => {
    if (editingOwner(props.index)) return home.send({ type: "overview" })
    if (Date.now() > quiet) home.send({ type: "go", screen })
  }
  const sections = createMemo(() => runs(editingOwner(props.index) ? props.kinds.filter((kind) => kind === "home") : props.kinds), { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) })
  return (
    <nav class="dots" ref={(nav) => sized.observe(nav)} aria-label={`${props.owner.name}'s Home Screen`} inert={seen().owner !== props.index || undefined} onPointerDown={slide}>
      <For each={sections()} keyed={false}>
        {(section) => {
          const focused = () => current() >= section().from && current() < section().from + section().count
          const name = () => SECTIONS[section().kind].name
          return (
            <>
              <button type="button" class="section" aria-label={name()} aria-current={current() === section().from ? "page" : undefined} onClick={press(section().from)}>
                <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true">
                  <path d={SECTIONS[section().kind].path} />
                </svg>
              </button>
              <Show when={section().count > 1}>
                <span class={["more", { open: focused() }]} inert={!focused() || undefined}>
                  <span>
                    <For each={Array.from({ length: section().count - 1 }, (_, at) => section().from + at + 1)}>
                      {(screen) => (
                        <button type="button" class="dot" aria-label={`${name()}, ${screen - section().from + 1} of ${section().count}`} aria-current={current() === screen ? "page" : undefined} onClick={press(screen)}>
                          <i />
                        </button>
                      )}
                    </For>
                  </span>
                </span>
              </Show>
            </>
          )
        }}
      </For>
    </nav>
  )
}

/// The owner switcher and the sections, one capsule floating at the bottom in the middle, as iOS 26's floating bars
/// are: the owner's name on the left, which opens the list of every owner, the current one ticked, and their sections'
/// icons and dots on the right. Both roll as the owners move. While pages are edited it's Home's alone.
function Toolbar(props: { owners: Owner[]; laid: Kind[][] }) {
  const at = () => state().context.at.owner
  let button!: HTMLButtonElement
  return (
    <div class={["toolbar", { editing: state().matches("editing") }]}>
      <button type="button" class="switch ui-button press ring" data-variant="ghost" ref={button} aria-label={props.owners[at()]?.name} aria-haspopup="menu" aria-expanded={state().matches("listing") ? "true" : "false"} onClick={() => home.send({ type: state().matches("listing") ? "dismiss" : "list" })}>
        <span class="roll" aria-hidden="true" ref={rolling}>
          <span class="names">
            <For each={props.owners}>{(owner) => <span ref={(name) => sized.observe(name)}>{owner.name}</span>}</For>
          </span>
        </span>
        <Icon name="caret-up-down" />
      </button>
      <Menu.Root open={state().matches("listing")} onOpenChange={({ open }) => !open && home.send({ type: "dismiss" })} onSelect={({ value }) => home.send({ type: "pick", owner: Number(value) })} positioning={{ placement: "top", getAnchorRect: () => button.getBoundingClientRect() }}>
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
      <div class="roll" ref={rolling}>
        <div class="rows">
          <For each={props.owners} keyed={(owner) => owner.key}>
            {(owner, index) => <Dots owner={owner()} index={index()} kinds={props.laid[index()] ?? []} />}
          </For>
        </div>
      </div>
    </div>
  )
}

/// While pages are edited, + and Done float at the top, over the pages, which stay where they are, as on a phone.
function EditBar() {
  return (
    <div class="top">
      <div class="edit-bar">
        <Button variant="secondary" size="icon" aria-label="Add Widget" onClick={() => state().matches({ editing: "still" }) && home.send({ type: "gallery" })}>
          <Icon name="plus" />
        </Button>
        <Button variant="secondary" onClick={() => home.send({ type: "done" })}>Done</Button>
      </div>
    </div>
  )
}

/// Whether the Home Screen is wide, how much larger its pages are drawn (`scale`), the gaps beside and between them,
/// and how many whole pages fit side by side at a phone's width, in the room it's given, `width` by `height`. Its
/// pages are laid out by CSS from these; the pager needs how many fit, to know where each stop is.
function measure(width: number, height: number) {
  const rem = parseFloat(getComputedStyle(document.documentElement).fontSize) || 16
  const wide = width >= 34 * rem
  scale = wide ? Math.max(1, Math.min(2, height / 900)) : 1
  const [side, gap] = [2 * rem * scale, 3 * rem * Math.min(scale, 1.6)]
  const fit = wide ? Math.max(1, Math.floor((width - 2 * side + gap) / (390 * scale + gap))) : 1
  return { wide, fit, scale, side, gap }
}

function HomeScreen(props: { owners: Owner[] }) {
  let root!: HTMLDivElement
  let main!: HTMLElement
  const strips: Record<number, HTMLElement> = {}
  const places: Record<string, HTMLElement> = {}
  // The owners as they are when a spring, a gesture or what the machine sends asks, outside anything that tracks them.
  const owned = () => untrack(() => props.owners)
  // A phone's until the Home Screen has been measured, which it is before it's first drawn.
  const [room, setRoom] = createSignal({ wide: false, fit: 1, scale: 1, side: 0, gap: 0 })


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
  /// Each owner's App Library, in its pages.
  const shelves = createMemo(() => props.owners.map((owner) => shelve(owner.library)))
  /// Each owner's screenfuls: its sections in order, each as many of its pages side by side as fit, from which page
  /// of it, and how many. The App Store is one screenful, the whole of it.
  const screens = createMemo(() => {
    const fit = room().fit
    const across = (kind: Kind, length: number) => Array.from({ length: Math.ceil(length / fit) }, (_, at) => ({ kind, from: at * fit, count: Math.min(fit, length - at * fit) }))
    return pages().map((homes, owner) => [...across("home", homes.length), ...across("library", shelves()[owner].length), ...(props.owners[owner].store ? [{ kind: "store" as Kind, from: 0, count: 1 }] : [])])
  })
  const laid = createMemo(() => ({ screens: screens().map((owner) => owner.map(({ kind }) => kind)), fit: room().fit }), { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) })
  // What was just kept shows until the Home Screen has been asked again.
  createEffect(
    () => props.owners,
    () => setKept(),
    { defer: true },
  )
  createEffect(laid, ({ screens, fit }) => home.send({ type: "laid", screens, fit }))
  /// A link to the App Library, `one:home-screen#library`, shows the first of it, as its mark in the dots does, once
  /// there is one. It's followed once, and the address forgets it, so the same link is followed again.
  const linked = ({ screens }: { screens: Kind[][] }) => {
    const library = screens[home.getSnapshot().context.at.owner]?.indexOf("library") ?? -1
    if (location.hash !== "#library" || library < 0) return
    history.replaceState(history.state, "", location.pathname + location.search)
    home.send({ type: "link", screen: library })
  }
  createEffect(laid, linked)
  onSettled(() => {
    const follow = () => linked(untrack(laid))
    addEventListener("hashchange", follow)
    return () => removeEventListener("hashchange", follow)
  })
  /// The cells of the Home pages, which glide to their new places while they're edited, and not as editing begins.
  const cells = () => root.querySelectorAll<HTMLElement>(".strip .page .cell")
  let wasEditing = false
  createEffect(pages, () => {
    const now = home.getSnapshot().matches("editing")
    reflow(cells(), now && wasEditing)
    wasEditing = now
  })
  pending = () => (owned()[state().context.at.owner]?.arranges ? draft(owned()[state().context.at.owner], state().context.at.owner) : undefined)

  /// The section of the screenful an owner is on: the one shown, for whoever's shown.
  const kindOf = (owner: number) => {
    const { last, screens } = state().context
    return screens[owner]?.[owner === seen().owner ? seen().screen : (last[owner] ?? 0)] ?? "home"
  }
  /// Each owner's screenfuls, across: `strips` are the windows onto them, each a screen wide, and `tracks` the rows
  /// that move in them.
  const tracks: Record<number, HTMLElement> = {}
  const height = () => main.clientHeight || root.clientHeight
  /// How far apart an owner's screenfuls are, a screen's width, and how many their row stops at.
  const pitch = (owner: number) => strips[owner]?.clientWidth || root.clientWidth
  const stops = (owner: number) => tracks[owner]?.children.length || 1
  const furthest = (owner: number) => (stops(owner) - 1) * pitch(owner)
  /// The owners, up and down, and each owner's pages, across: where they are, as the pager moves them.
  const down = spring((y) => {
    main.style.translate = `0 ${-y}px`
    roll(y / height())
    look()
  })
  const across: Spring[] = []
  const sideways = (owner: number) =>
    (across[owner] ??= spring((x) => {
      if (tracks[owner]) tracks[owner].style.translate = `${-x * rtl()}px 0`
      look()
    }))
  /// The owner and page nearest where they are, which the dots and the owner's name follow.
  const look = () => {
    const owner = Math.max(0, Math.min(owned().length - 1, Math.round(down.value / height())))
    setSeen({ owner, screen: Math.round(sideways(owner).value / pitch(owner)) })
  }

  /// Moves the pages to `at`, as the machine says: gliding, or at once where they're only laid out again.
  const slide = (at: At, smooth: boolean) => {
    const move = smooth ? glide : jump
    move(down, at.owner * height())
    move(sideways(at.owner), at.screen * pitch(at.owner))
  }

  /// A finger on the pages swipes them: across between an owner's pages, and up and down between owners, unless the
  /// page scrolls itself, as the App Library does, or they're being edited. Until it has moved 10 points it may be a
  /// tap or a hold; then it's a swipe, along the way it moved most, and the pages follow it from there. A mouse has
  /// the wheel and the keys.
  let swipe: { id: number; x: number; y: number; axis?: "x" | "y"; owner: number; from: number; base: number; moves: { time: number; value: number }[] } | undefined
  let swiped = 0
  const press = (event: PointerEvent) => {
    const on = event.target as Element
    if (event.pointerType === "mouse" || event.button || !on.closest(".owners")) return
    // A category shrinking back into its place lets the pages under it have the finger.
    if (!state().matches("nothing") && !state().matches({ category: "closing" }) && !(state().matches({ editing: "still" }) && !on.closest(".cell"))) return
    const owner = Math.max(0, Math.min(owned().length - 1, Math.round(down.value / height())))
    // A finger that lands while they move catches them.
    stop(down)
    stop(sideways(owner))
    swipe = { id: event.pointerId, x: event.clientX, y: event.clientY, owner, from: 0, base: 0, moves: [] }
  }
  const drag = (event: PointerEvent) => {
    if (!swipe || event.pointerId !== swipe.id) return
    if (!swipe.axis) {
      const [dx, dy] = [event.clientX - swipe.x, event.clientY - swipe.y]
      if (Math.hypot(dx, dy) < 10) return
      const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y"
      // A hold that began has the finger, and up and down is the page's own where it scrolls, and nobody's while editing.
      const free = state().matches("nothing") || state().matches({ category: "closing" }) || state().matches({ editing: "still" })
      if (!free || (axis === "y" && (state().matches("editing") || kinds[kindOf(swipe.owner)].scrolls))) return void (swipe = undefined)
      const moving = axis === "x" ? sideways(swipe.owner) : down
      const size = axis === "x" ? pitch(swipe.owner) : height()
      Object.assign(swipe, { axis, x: event.clientX, y: event.clientY, base: moving.value, from: Math.round(moving.value / size) })
      root.setPointerCapture(event.pointerId)
    }
    const x = swipe.axis === "x"
    const moved = x ? -(event.clientX - swipe.x) * rtl() : -(event.clientY - swipe.y)
    const end = x ? furthest(swipe.owner) : (owned().length - 1) * height()
    const size = x ? strips[swipe.owner].clientWidth : height()
    const value = swipe.base + moved
    swipe.moves.push({ time: event.timeStamp, value })
    if (swipe.moves.length > 32) swipe.moves.shift()
    jump(x ? sideways(swipe.owner) : down, value < 0 ? give(value, size) : value > end ? end + give(value - end, size) : value)
  }
  const lift = (event: PointerEvent) => {
    if (!swipe || event.pointerId !== swipe.id) return
    // Where the finger lifted is where it last was: moves come once a frame, so the last may be short of it.
    if (event.type === "pointerup" && swipe.axis) drag(event)
    const done = swipe
    swipe = undefined
    if (!done.axis) return
    swiped = performance.now()
    const velocity = event.type === "pointercancel" ? 0 : speed(done.moves, event.timeStamp)
    if (done.axis === "x") {
      const moving = sideways(done.owner)
      const screen = settle(done.from, moving.value / pitch(done.owner), velocity, stops(done.owner))
      glide(moving, screen * pitch(done.owner), velocity)
      home.send({ type: "settled", at: { owner: done.owner, screen } })
    } else {
      const owner = settle(done.from, down.value / height(), velocity, owned().length)
      glide(down, owner * height(), velocity)
      home.send({ type: "settled", at: { owner, screen: Math.round(sideways(owner).value / pitch(owner)) } })
    }
  }
  /// The pages have the finger, not the web view the Home Screen is in: an iPhone's scrolls it up and down, bouncing,
  /// whatever `touch-action` says, unless its touches are cancelled. Up and down on a page that scrolls is that page's.
  const touchMove = (event: TouchEvent) => {
    if (swipe && !(swipe.axis !== "x" && (event.target as Element).closest?.(".page.scrolls"))) event.preventDefault()
  }
  /// A finger that swiped taps nothing as it lifts.
  const tapped = (event: MouseEvent) => {
    if (performance.now() - swiped < 350) (event.stopPropagation(), event.preventDefault())
  }

  // Each effect below is driven by a memo, which changes only when whether it's so does: given the snapshot itself, an
  // effect runs again on every event, and the drag's would put what's carried down and lift it again at each move.
  /// What's carried, while it's dragged: a copy that lifts larger with a shadow and follows the finger one to one, and
  /// over what it would go before. Held at a page's edge a moment, it goes on to the next page, or the one before. Let
  /// go, it springs into its place from where it was, at the speed it was moving.
  createEffect(
    createMemo(() => state().matches({ editing: "dragging" })),
    (dragging) => {
      if (!dragging) return
      const held = home.getSnapshot().context.held!
      const rect = held.rect ?? { x: pointer.x - 30, y: pointer.y - 40, width: 60, height: 80 }
      const copy = document.createElement("div")
      copy.className = "carried"
      // It's laid out at a phone's size and drawn as much larger as the pages are, from its middle, which is where
      // what it was had its middle.
      const [width, height] = [rect.width / scale, rect.height / scale]
      Object.assign(copy.style, { left: `${rect.x + (rect.width - width) / 2}px`, top: `${rect.y + (rect.height - height) / 2}px`, width: `${width}px`, height: `${height}px` })
      const item = held.item!
      const follow = (x: number, y: number) => (copy.style.translate = `${x - grab.x}px ${y - grab.y}px`)
      follow(pointer.x, pointer.y)
      copy.innerHTML = item.widget ? `<div class="ui-widget" data-size="${item.size}"></div>` : `<span class="tile"><span class="icon"><img src="${(item as Entrypoint).icon}" alt=""></span><span>${(item as Entrypoint).name}</span></span>`
      root.append(copy)
      if (!still.matches) copy.animate([{ scale: `${scale}`, filter: "drop-shadow(0 0 0 oklch(0 0 0 / 0))" }, {}], paced("settle"))
      haptic("medium")
      const moves: { time: number; x: number; y: number }[] = []
      let edge: ReturnType<typeof setTimeout> | undefined
      let side = 0
      // A page turns after a moment at its edge, and again while it stays there, as an iPhone's does.
      const turn = () => {
        const screen = home.getSnapshot().context.at.screen
        home.send({ type: "flip", by: side as 1 | -1 })
        if (home.getSnapshot().context.at.screen !== screen) haptic("selection")
        edge = setTimeout(turn, 900)
      }
      const move = (event: PointerEvent) => {
        follow(event.clientX, event.clientY)
        moves.push({ time: event.timeStamp, x: event.clientX, y: event.clientY })
        if (moves.length > 32) moves.shift()
        const strip = strips[home.getSnapshot().context.at.owner]
        const bounds = strip.getBoundingClientRect()
        const now = event.clientX < bounds.left + 28 ? -1 : event.clientX > bounds.right - 28 ? 1 : 0
        if (now !== side) {
          clearTimeout(edge)
          side = now
          if (side) edge = setTimeout(turn, 600)
        }
        if (side) return
        const under = document.elementsFromPoint(event.clientX, event.clientY) as HTMLElement[]
        const page = under.find((element) => element.dataset?.page !== undefined)
        if (!page) return
        // Over where each cell sits, not where it's drawn while it glides there, or one gliding past takes the place.
        const cell = [...page.querySelectorAll<HTMLElement>(".cell[data-index]:not(.lifted)")].find((one) => {
          const [box, by] = [one.getBoundingClientRect(), offset(one, scale)]
          return event.clientX >= box.left - by.x && event.clientX < box.right - by.x && event.clientY >= box.top - by.y && event.clientY < box.bottom - by.y
        })
        const { held } = home.getSnapshot().context
        const index = cell ? Number(cell.dataset.index) : Number.MAX_SAFE_INTEGER
        const at = Number(page.dataset.page)
        if (held && (at !== held.page || (cell && index !== held.index))) home.send({ type: "over", page: at, index })
      }
      // Where it was last, so it can settle there once it's let go.
      let left = held
      // How fast the finger was going as it let go, which the copy goes on at.
      let thrown = { x: 0, y: 0 }
      const drop = (event: PointerEvent) => {
        left = home.getSnapshot().context.held ?? left
        if (event.type === "pointerup") thrown = { x: speed(moves.map(({ time, x }) => ({ time, value: x })), event.timeStamp), y: speed(moves.map(({ time, y }) => ({ time, value: y })), event.timeStamp) }
        home.send({ type: "drop" })
      }
      addEventListener("pointermove", move)
      addEventListener("pointerup", drop)
      addEventListener("pointercancel", drop)
      return () => {
        clearTimeout(edge)
        removeEventListener("pointermove", move)
        removeEventListener("pointerup", drop)
        removeEventListener("pointercancel", drop)
        quiet = Date.now() + 400
        haptic("light")
        // What was carried settles into its place as it's let go, rather than vanishing over it: a spring from where
        // the finger left it, at the speed it was going, to the middle of its place, wherever that is as it goes.
        const place = strips[home.getSnapshot().context.at.owner]?.querySelector<HTMLElement>(`[data-page="${left.page}"] .cell[data-index="${left.index}"]`)
        if (!place || still.matches) {
          copy.animate([{ opacity: 1 }, { opacity: 0 }], { ...paced(place ? "settle" : "quick"), fill: "forwards" }).finished.finally(() => copy.remove())
          place?.animate([{ opacity: 0 }, { opacity: 1 }], paced("settle"))
          return
        }
        place.style.visibility = "hidden"
        const target = () => {
          const to = place.getBoundingClientRect()
          const glided = offset(place, scale)
          return { x: to.x - glided.x + to.width / 2 - (rect.x + rect.width / 2), y: to.y - glided.y + to.height / 2 - (rect.y + rect.height / 2) }
        }
        const draw = () => {
          // Its place may still be on its way, as the pages turn or the others make room: the copy goes after it.
          const to = target()
          if (Math.abs(x.target - to.x) > 0.5) glide(x, to.x)
          if (Math.abs(y.target - to.y) > 0.5) glide(y, to.y)
          copy.style.translate = `${x.value}px ${y.value}px`
          if (x.value === x.target && y.value === y.target && !x.velocity && !y.velocity) (copy.remove(), (place.style.visibility = ""))
        }
        const [x, y] = [spring(draw), spring(draw)]
        const [fromX = 0, fromY = 0] = copy.style.translate.split(" ").map(parseFloat)
        const to = target()
        Object.assign(x, { value: fromX, target: to.x })
        Object.assign(y, { value: fromY, target: to.y })
        glide(x, to.x, thrown.x)
        glide(y, to.y, thrown.y)
        copy.animate([{}, { scale: `${scale}`, filter: "drop-shadow(0 0 0 oklch(0 0 0 / 0))" }], { ...paced("travel"), fill: "forwards" })
      }
    },
  )

  onSettled(() => {
    const sliding = home.on("slide", ({ at, smooth }) => slide(at, smooth))
    const arranging = home.on("arrange", ({ editing }) => keep(editing, owned()[editing.owner]))
    // The room it's given may have changed since it was first drawn, and every owner's pages stay on the page they
    // were. It's laid out for the new size first, so where the pages are is measured from what's drawn.
    const resized = () => {
      setRoom(measure(root.clientWidth, root.clientHeight))
      flush()
      // Once it's drawn again for the new size, where each cell is is where it is.
      requestAnimationFrame(() => reflow(cells(), false))
      const { at, last } = home.getSnapshot().context
      owned().forEach((_, owner) => jump(sideways(owner), (owner === at.owner ? at.screen : (last[owner] ?? 0)) * pitch(owner)))
      jump(down, at.owner * height())
    }
    const watched = new ResizeObserver(resized)
    watched.observe(root)

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
      home.send(axis === "y" ? { type: "step", owner: by } : { type: "step", screen: (by * rtl()) as 1 | -1 })
    }
    root.addEventListener("wheel", wheel, { passive: false })
    root.addEventListener("pointerdown", press)
    root.addEventListener("pointermove", drag)
    root.addEventListener("pointerup", lift)
    root.addEventListener("pointercancel", lift)
    root.addEventListener("click", tapped, true)
    root.addEventListener("touchmove", touchMove, { passive: false })

    // The arrow keys move as a swipe does: up and down on a page that scrolls itself scroll it. Escape ends editing.
    const keys = (event: KeyboardEvent) => {
      if (event.defaultPrevented || event.altKey || event.metaKey || event.ctrlKey) return
      // A key pressed in an alert or a sheet is its own: the browser closes it on Escape, and that's all Escape does.
      if ((event.target as Element).closest?.("dialog")) return
      // Escape closes the page overview, and otherwise ends editing, carrying something or not: nothing's open over it.
      if (event.key === "Escape" && home.getSnapshot().matches({ editing: "overview" })) return home.send({ type: "dismiss" })
      if (event.key === "Escape" && (home.getSnapshot().matches({ editing: "still" }) || home.getSnapshot().matches({ editing: "dragging" }))) return home.send({ type: "done" })
      if (!home.getSnapshot().matches("nothing")) return
      const to = ({ ArrowRight: { screen: rtl() }, ArrowLeft: { screen: -rtl() }, ArrowDown: { owner: 1 }, ArrowUp: { owner: -1 } } as Record<string, { screen: 1 | -1 } | { owner: 1 | -1 }>)[event.key]
      if (!to || ("owner" in to && kinds[kindOf(home.getSnapshot().context.at.owner)].scrolls)) return
      event.preventDefault()
      home.send({ type: "step", ...to })
    }
    addEventListener("keydown", keys)
    return () => (sliding.unsubscribe(), arranging.unsubscribe(), watched.disconnect(), root.removeEventListener("wheel", wheel), removeEventListener("keydown", keys), root.removeEventListener("pointerdown", press), root.removeEventListener("pointermove", drag), root.removeEventListener("pointerup", lift), root.removeEventListener("pointercancel", lift), root.removeEventListener("click", tapped, true), root.removeEventListener("touchmove", touchMove))
  })

  // Editing begins with a tick, as an iPhone's does; lifting something into it plays its own. While it lasts, the
  // device's status bar is hidden, and + and Done are where it was.
  createEffect(
    createMemo(() => state().matches("editing")),
    (now) => {
      if (now && home.getSnapshot().matches({ editing: "still" })) haptic("selection")
      client({ statusBar: !now }).catch(() => {})
    },
    { defer: true },
  )
  // While pages are edited, the device's own back ends editing, as Done does.
  createEffect(
    createMemo(() => state().matches({ editing: "still" })),
    (now) => (now ? covers(() => home.send({ type: "done" })) : undefined),
  )

  const opened = () => state().context.category
  const held = () => (state().matches("holding") ? state().context.held : undefined)
  const owner = () => props.owners[state().context.at.owner]
  return (
    <div class={["home", { wide: room().wide, editing: state().matches("editing"), carrying: state().matches({ editing: "dragging" }) }]} style={{ "--pages-scale": room().scale, "--pages-side": `${room().side}px`, "--pages-gap": `${room().gap}px`, "--pages": room().fit, "--pages-shown": screens()[state().context.at.owner]?.[state().context.at.screen]?.count ?? 1, "--hold": `${HOLD}ms` }} ref={root} onClick={(event) => Date.now() > quiet && state().matches({ editing: "still" }) && !(event.target as Element).closest(".cell, .top, .toolbar, .overview, .ui-dialog") && home.send({ type: "done" })}>
      <main class="owners" ref={main}>
        <For each={props.owners} keyed={(owner) => owner.key}>
          {(owner, index) => (
            <section class="owner" aria-label={owner().name}>
              <div class="strip" ref={registered(strips, index)}>
                <div class="track" ref={registered(tracks, index)}>
                  <For each={screens()[index()] ?? []} keyed={false}>
                    {(screen) => (
                      <div class={["screen", screen().kind]}>
                        <Show when={screen().kind === "home"}>
                          <For each={(pages()[index()] ?? []).slice(screen().from, screen().from + screen().count)} keyed={false}>
                            {(placed, at) => <HomePage owner={owner()} index={index()} page={screen().from + at} placed={placed()} />}
                          </For>
                        </Show>
                        <Show when={screen().kind === "library"}>
                          <For each={shelves()[index()].slice(screen().from, screen().from + screen().count)} keyed={false}>
                            {(folders) => <Library owner={owner()} index={index()} folders={folders()} places={places} />}
                          </For>
                        </Show>
                        <Show when={screen().kind === "store" && owner().store}>{(name) => <StorePage name={name()} shown={seen().owner === index()} />}</Show>
                      </div>
                    )}
                  </For>
                </div>
              </div>
            </section>
          )}
        </For>
      </main>
      <Toolbar owners={props.owners} laid={laid().screens} />
      <Show when={state().matches("editing")}>
        <EditBar />
      </Show>
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[opened().owner]} opened={opened()} places={places} />}</Show>
      <Show when={held()}>{(held) => <Holding owner={owner()} index={state().context.at.owner} held={held()} />}</Show>
      <Show when={state().matches({ editing: "overview" }) && editing()}>
        {(edited) => <Overview owner={props.owners[edited().owner]} index={edited().owner} />}
      </Show>
      <Show when={editing()}>{(edited) => <Gallery owner={props.owners[edited().owner]} />}</Show>
      <Show when={about() && props.owners.find((owner) => owner.store)?.store}>{(store) => <About app={about()!} store={store()} />}</Show>
    </div>
  )
}

/// The Home Screen didn't draw: what DESIGN.md says of a launcher that doesn't, with Try Again, which loads this page
/// again, its newest version among it, and what went wrong under Show Details. Its client loads it again by itself
/// once a newer version is out, as the older one meeting what's new is the likeliest cause.
function Broken(props: { error: Error }) {
  return (
    <div class="broken">
      <Problem heading="Home Screen didn't open." text={new Error(`Try again in a moment.\n\n${typeof props.error?.cause === "string" ? props.error.cause : (props.error?.message ?? props.error)}`)} action={<Button variant="outline" onClick={() => location.reload()}>Try Again</Button>} />
    </div>
  )
}

render(
  () => (
    <Errored fallback={(error) => <Broken error={error() as Error} />}>
      <Loading>
        <HomeScreen owners={owners()} />
      </Loading>
    </Errored>
  ),
  document.body,
)
