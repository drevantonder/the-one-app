// The Home Screen: two sections side by side, Home and the App Library, as an iPhone's. Home is every owner's pages:
// yours, everyone's, then each of your groups', one owner above another, each a row of screenfuls, each as many of
// their pages side by side as fit, a phone's page drawn larger to suit the window's height on a wide one, and centred
// where there are fewer. The App Library, every app you can reach, by category, is one page, for everyone, a screen
// wide, that scrolls up and down. Left and right moves a screenful, as a phone's pages do, through Home's screenfuls
// and on to the App Library; up and down on Home moves between owners. The pages move by hand, as a phone's do
// (pager.ts). On Home, a small pill at the top in the middle says whose pages they are and opens a list of every
// owner. At the bottom, just clear of the home indicator, the dots pill has the owner's page dots, then the App
// Library's glyph, lit there. The App Store is an app like any other, in the App Library. A page never scrolls: it's
// a fixed grid, and what doesn't fit goes on to the next. Whoever arranges an owner's pages edits
// them as a phone's Home Screen is edited: touch and hold an app or a widget (press and hold it, or right-click it,
// with a mouse), then Edit Home Screen, or hold it a while longer, or touch and hold a page's empty space. An app held
// in the App Library is carried onto any owner's pages. Where it is, what's open over it and the editing are one state
// machine (machine.ts), whose snapshot is one signal here.
import { createEffect, createMemo, createSignal, Errored, flush, For, Loading, onSettled, refresh, Show, untrack } from "solid-js"
import { render, type JSX } from "@solidjs/web"
import { createActor } from "xstate"
import { Button, call, changed, confirm, covers, Empty, haptic, Icon, List, Menu, open, pace, Problem, Row, Search, Sheet, Spinner, toast, Widget } from "ui"
import "./home.css"
import { give, glide, jump, settle, speed, spring, type Spring, stop } from "./pager.ts"
import { offset, reflow } from "./reflow.ts"
import { type At, carries, COLUMNS, type Drop, type Editing, fitted, type Held, type Kind, machine, nearest, pack, type Page, type Placed, ROWS, shown, spans } from "./machine.ts"
import type { Entrypoint, Category, Item, Listed, Pages, Size, Widget as WidgetItem } from "./types.ts"

const SIZES: Record<Size, string> = { "2x2": "Small", "4x2": "Medium", "4x4": "Large" }
const json = <T,>(tool: string, inputs?: object, subject?: string): Promise<T> => call(tool, inputs, { subject }).then(JSON.parse)

/** An owner: whose Home Screen it is, by the name a person reads, and what `home-screen_pages` answers for them. */
interface Owner extends Pages {
  key: string
  subject?: string
  name: string
}

/// You, everyone, then each of your groups, in the order they're listed. A network with nothing that keeps groups
/// has none. Yours and everyone's are asked for beside the groups, which they don't wait on.
const owners = createMemo(async () => {
  const pages = (who: { subject?: string; name: string }) => json<Pages>("home-screen_pages", {}, who.subject).then((pages): Owner => ({ ...pages, ...who, key: who.subject ?? "you" }))
  const groups = json<{ groups: { group: string; called: string }[] }>("permissions_groups").then(({ groups }) => groups, (): { group: string; called: string }[] => [])
  const [yours, everyone, theirs] = await Promise.all([pages({ name: "You" }), pages({ subject: "network", name: "Everyone" }), groups.then((groups) => Promise.all(groups.map(({ group, called }) => pages({ subject: group, name: called }))))])
  return [yours, everyone, ...theirs]
})
changed(() => refresh(owners))

/// The one machine, and its snapshot as one signal.
const home = createActor(machine)
const [state, setState] = createSignal(home.getSnapshot())
home.subscribe(setState)
home.start()

/// The owner and screenful nearest where the pages are as they move, which the dots follow during a swipe, as a
/// phone's do, rather than once it ends.
const [seen, setSeen] = createSignal<At>({ owner: 0, screen: 0 }, { equals: (a, b) => a.owner === b.owner && a.screen === b.screen })
/// How much a wide window draws the Home Screen larger than a phone does: a phone's pages, scaled to the window's
/// height, twice at most, so a bigger screen shows more pages rather than larger icons. Set by `measure`.
let scale = 1

/// The owner pill's names and the dots pill's rows, which roll together as the owners move: each a window onto a
/// strip with a row for every owner. `at` is where the owners are, in owners. Each window is as wide as the row it
/// shows, or between two rows' widths as the owners pass, so each pill fits whoever's shown, and a row fades as it
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
/// Each row's own width, measured whenever one changes, in its pill's own points: it's drawn larger on a wide window,
/// perhaps still on its way there.
const sized = new ResizeObserver(() => {
  for (const window of rolls) {
    const bar = window.closest<HTMLElement>(".pill")!
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
/// whoever opens it. `behind` only starts it, as a finger lands on it. It grows out of its icon or widget, `from`, as
/// an iPhone's does.
const launch = (entrypoint: Entrypoint, owner: Owner, behind?: boolean, from?: Element) =>
  open(entrypoint.entrypoint, { subject: entrypoint.subject === "user" ? undefined : (entrypoint.subject ?? owner.subject), behind, from: from && place(from) })
/// Where an icon or a widget is in the window, and how round its corners are there, as drawn: pages drawn larger on a
/// wide window have larger ones.
const place = (from: Element) => {
  const { x, y, width, height } = from.getBoundingClientRect()
  const corner = getComputedStyle(from).borderTopLeftRadius
  const radius = corner.endsWith("%") ? (parseFloat(corner) / 100) * width : parseFloat(corner) * (width / ((from as HTMLElement).offsetWidth || width))
  return { x, y, width, height, radius }
}
/// The UI library's pace of that name, from its tokens.
const paced = (name: "quick" | "settle" | "travel") => pace(document.documentElement, name)
/// As something goes, a copy of it fades out from where it is, over what's under it, taking no taps, as iOS's do.
const fadeOut = (element: HTMLElement) => {
  const from = element.isConnected ? getComputedStyle(element).opacity : "1"
  const ghost = element.cloneNode(true) as HTMLElement
  ghost.inert = true
  ghost.style.pointerEvents = "none"
  document.querySelector(".home")?.append(ghost)
  ghost.animate([{ opacity: from }, { opacity: 0 }], { ...paced("settle"), fill: "forwards" }).finished.finally(() => ghost.remove())
}

/// An owner's pages as they're kept: each with what's on it and whether it's hidden, to edit.
const draft = (owner: Owner, index: number): Editing => ({ owner: index, pages: fitted(owner.pages.map((page) => ({ items: [...page.items], hidden: page.hidden }))) })

/// Owners' pages as they were just edited, shown until the Home Screen is asked again.
const [kept, setKept] = createSignal<Editing[]>([])

// A finger that stays this long holds what it's on, as a phone's does, and on a Home page, held this much longer
// still, it's lifted into editing, as an iPhone's menu gives way to the drag. An icon sinks under the finger for as
// long as it takes to hold (home.css's `--hold`).
const HOLD = 450
const LONGER = 800
// What's carried makes room where it rests this long, as an iPhone's does, not at every place it passes.
const REST = 150

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
  const opens = (card: HTMLElement) => !props.still && state().matches("nothing") && props.item.opens && launch(props.item.opens, props.owner, false, card)
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
    // Where it is in the window, at its own size around its middle: as it jiggles, the box around it is larger.
    const rect = () => {
      const box = element.getBoundingClientRect()
      const [width, height] = [element.offsetWidth * scale, element.offsetHeight * scale]
      return { x: box.x + (box.width - width) / 2, y: box.y + (box.height - height) / 2, width, height }
    }
    // `at` is where the pointer is now: a press held still a while longer lifts with no move to say so.
    const lift = (at: PointerEvent, from: PointerEvent) => {
      clear()
      const what = held()
      if (!what?.item || !arranges()) return
      grab = { x: from.clientX, y: from.clientY }
      pointer = { x: at.clientX, y: at.clientY }
      carrier = at.pointerId
      home.send({ type: "lift", held: { ...what, rect: rect() }, editing: state().matches("editing") ? undefined : pending() })
      // The Home Screen keeps the pointer from here on, whatever's drawn again under it.
      document.querySelector<HTMLElement>(".home")?.setPointerCapture(at.pointerId)
    }
    const hold = () => {
      const what = held()
      if (!menu() || !what || !state().matches("nothing")) return
      fired = !!start
      if (!what.item && !what.listed) return arranges() && home.send({ type: "edit", editing: pending() })
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

/// The pages of the owner Home is on, to edit, as they're kept, for what's lifted before editing began.
const pending = () => {
  const at = state().context.at.owner
  return draft(untrack(owners)![at], at)
}

/// Until when a tap isn't one: just after something carried was put down.
let quiet = 0
/// Where the pointer was last, and where it pressed what it lifted: what's carried stays where the finger took it,
/// `carrier`, whichever other fingers there are.
let pointer = { x: 0, y: 0 }
let grab = { x: 0, y: 0 }
let carrier = -1
addEventListener("pointermove", (event) => event.isPrimary && (pointer = { x: event.clientX, y: event.clientY }), { capture: true, passive: true })

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
/// its ⊖, and what's lifted leaves its place empty. An owner with nothing on their pages has one grey line. An app
/// dropped where it isn't installed waits there, dimmed, while it's asked about, and shows its progress as it installs.
function HomePage(props: { owner: Owner; index: number; page: number; placed: Placed<Cell>[]; small?: boolean }) {
  const editingHere = () => !props.small && editingOwner(props.index)
  // What's carried leaves its place empty, where it'll land, wherever that is as the rest make room.
  const lifted = (cell: Cell) => editingHere() && state().matches({ editing: "dragging" }) && carries(state().context.held, cell.item)
  // Dropped where it isn't installed, what was stacked on it waits with it.
  const waiting = (cell: Cell) => editingHere() && carries(state().context.held, cell.item) && (state().matches({ editing: "asking" }) || state().matches({ editing: "installing" }) || state().matches({ editing: "refused" }))
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
                class={["cell", { jiggles: editingHere(), lifted: lifted(placed().item), waiting: waiting(placed().item) }]}
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
                      <Show when={waiting(placed().item) && state().matches({ editing: "installing" })}>
                        <span class="progress" role="status" aria-label={`Installing ${item().name}`}>
                          <Spinner />
                        </span>
                      </Show>
                      <Show when={editingHere() && arranges() && !waiting(placed().item)}>
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

/// Whether Background Apps is open, which each device remembers. Its storage may be missing or refused, as in a
/// private window; then it starts closed.
const BACKGROUND = "home-screen:background-apps"
const [unfolded, setUnfolded] = createSignal(
  (() => {
    try {
      return localStorage.getItem(BACKGROUND) === "open"
    } catch {
      return false
    }
  })(),
)
const unfold = (open: boolean) => {
  setUnfolded(open)
  try {
    localStorage.setItem(BACKGROUND, open ? "open" : "closed")
  } catch {}
}

/// An App Library's categories, each only its apps with an entrypoint, or under Background Apps (`background`), only
/// its apps with none.
const sorted = (library: Category[], background: boolean) =>
  library.flatMap((category) => {
    const entrypoints = category.entrypoints.filter((listed) => !listed.entrypoint === background)
    return entrypoints.length ? [{ ...category, entrypoints }] : []
  })
/// A category of the App Library as it's opened, and its place there is registered: under Background Apps, apart from
/// the category of the same name above it.
const BEHIND = "background:"
const openedCategory = (library: Category[], key: string) =>
  key.startsWith(BEHIND) ? sorted(library, true).find((category) => category.category === key.slice(BEHIND.length)) : sorted(library, false).find((category) => category.category === key)

/// The App Library: one page, which scrolls, as an iPhone's does, with every app you can reach once, whether it's
/// installed for you, a group or everyone: your own App Library, which has theirs. Its search is at its top; under it,
/// the categories of the apps with a screen of their own, as many across as fit, each its first four apps' icons with
/// their names in a square, or with more than that, three and the rest smaller in the fourth's place, and its name
/// under it. Tapping an icon opens its app; tapping anywhere else in a category opens it. Under them is Background
/// Apps, closed until it's tapped, as iOS's collapsible sections are: a row flush with the background whose chevron
/// turns as it opens the categories of background apps, those with no screen of their own, under it. Typing in the
/// search shows the apps whose names match, A to Z, in place of the categories, and the background apps that match
/// under Background Apps, open. Touching and holding an app gives its menu, and moving one with a screen carries it
/// onto Home's pages.
function Library(props: { owner: Owner; places: Record<string, HTMLElement> }) {
  const [query, setQuery] = createSignal("")
  // Tapped while a search shows it open, Background Apps closes only for that search.
  const [shut, setShut] = createSignal(false)
  const searching = () => !!query().trim()
  // Tapped again as it shrinks back into its place, it turns and grows again.
  const opening = (key: () => string) => () => {
    const back = state().matches({ category: "closing" }) && state().context.category === key()
    if (state().matches("nothing") || back) home.send({ type: "open", category: key() })
  }
  const found = (background: boolean) => {
    const words = query().trim().toLowerCase()
    const all = props.owner.library.flatMap((category) => category.entrypoints).filter((listed) => !listed.entrypoint === background)
    return all.filter((listed) => listed.name.toLowerCase().includes(words)).sort((a, b) => a.name.localeCompare(b.name))
  }
  const apps = () => sorted(props.owner.library, false)
  const background = () => sorted(props.owner.library, true)
  const open = () => (searching() ? !shut() : unfolded())
  const categories = (list: Category[], prefix: string) => (
    <div class="categories">
      <For each={list} keyed={(category) => category.category}>
        {(category) => {
          const key = () => prefix + category().category
          const shown = () => (category().entrypoints.length > 4 ? category().entrypoints.slice(0, 3) : category().entrypoints)
          return (
            <div class="category">
              <div class="place" ref={registered(props.places, key)} onClick={opening(key)}>
                <For each={shown()} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                  {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} />}
                </For>
                <Show when={category().entrypoints.length > 4}>
                  <button type="button" class="more" aria-label={`All of ${category().category}`}>
                    <For each={category().entrypoints.slice(3, 7)} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                      {(entrypoint) => <img src={entrypoint().icon} alt="" />}
                    </For>
                  </button>
                </Show>
              </div>
              <button type="button" class="name" onClick={opening(key)}>{category().category}</button>
            </div>
          )
        }}
      </For>
    </div>
  )
  const results = (list: Listed[]) => (
    <div class="found">
      <For each={list} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
        {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} />}
      </For>
    </div>
  )
  return (
    <section class="page library scrolls" aria-label="App Library">
      <div class="shelf">
        <Search placeholder="App Library" aria-label="Search the App Library" value={query()} onInput={(event) => (setQuery(event.currentTarget.value), setShut(false))} />
        <Show when={searching() ? found(false).length || found(true).length : props.owner.library.length} fallback={<Empty>{searching() ? "No apps match." : "No apps yet."}</Empty>}>
          <Show when={searching()} fallback={<Show when={apps().length}>{categories(apps(), "")}</Show>}>
            <Show when={found(false).length}>{results(found(false))}</Show>
          </Show>
          <Show when={searching() ? found(true).length : background().length}>
            <section class="background" aria-labelledby="background-apps">
              <button type="button" id="background-apps" class="disclosure" aria-expanded={open() ? "true" : "false"} onClick={() => (searching() ? setShut(!shut()) : unfold(!unfolded()))}>
                <span>Background Apps</span>
                <Icon name="caret-right" along />
              </button>
              <Show when={open()}>{searching() ? results(found(true)) : categories(background(), BEHIND)}</Show>
            </section>
          </Show>
        </Show>
      </div>
    </section>
  )
}

/// An app's icon in the App Library: a tap opens it, or for an app with no entrypoint, the App Store at its page, and a finger
/// that holds it opens its menu; moving it carries it onto whichever owner's pages it's dropped on.
function LibraryEntrypoint(props: { entrypoint: Listed; owner: Owner }) {
  return (
    <button
      type="button"
      class="app"
      ref={holds(() => ({ item: opens(props.entrypoint), listed: props.entrypoint, page: 0, index: -1, from: "library" }), () => true)}
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
/// Opens an app of an App Library: its entrypoint, or for an app with none, the App Store at its page, by a link
/// (`one:app-store#app=…`), growing out of its icon as any app does.
const press = (listed: Listed, owner: Owner, behind?: boolean, icon?: Element) => {
  if (opens(listed)) return launch(opens(listed)!, owner, behind, icon)
  const store = untrack(owners)?.[0]?.store
  if (behind || !store) return
  open(`${store}#app=${encodeURIComponent(listed.app)}`, { from: icon && place(icon) })
}

/// Who a call about an install is for: a group or the network, or for a user's own, nobody said.
const subjectOf = (installed: string) => (installed.startsWith("users/") ? undefined : installed)
const and = (words: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(words)

/// A category opened over its App Library: its name, and its apps in a panel that grows out of its place there and
/// shrinks back into it, as an iPhone's folder does, while what's behind it blurs. Its apps open only once it has
/// grown. Tapping beside the panel, Escape or the device's own back closes it, even as it grows: it turns back from
/// where it got to. For whoever asked for less motion, it fades in and out where it is. It's a modal `<dialog>`, so
/// focus stays in it and the rest is inert while it's open; as it shrinks it's a dialog that isn't modal, so the pages
/// under it take a finger.
function OpenCategory(props: { owner: Owner; opened: string; places: Record<string, HTMLElement> }) {
  let veil!: HTMLDialogElement
  let panel!: HTMLDivElement
  let title!: HTMLHeadingElement
  let grid!: HTMLDivElement
  const category = () => openedCategory(props.owner.library, props.opened)
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
      const place = props.places[props.opened]
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
      <h2 id="opened-category" ref={title}>{category()?.category}</h2>
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
/// of it with a spring and shrinks back into it: Edit Home Screen and Remove App on a Home page; in the App Library, who
/// the app is installed for, Add to Home Screen, onto the pages of the owner Home was last on, and Remove App. Moving
/// the finger, or on a Home page holding it a while longer, carries it into editing; anywhere else, Escape, a tap
/// beside it or the device's own back puts it down. The menu is a popover in the top layer, a manual one: the browser
/// closes an automatic one as the finger that held it lifts, which is outside it. The arrow keys move between its items.
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
    keep([{ ...editing, pages: pages.filter((page) => page.items.length) }])
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
      const all = (untrack(owners) ?? [props.owner]).flatMap((owner) => owner.library.flatMap((category) => category.entrypoints))
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
    return `This is ${whose(listed.installed.for)}'s, and you're not one of its owners.`
  }
  // From the App Library, it goes last on the owner's last page, as a phone's does.
  const adding = () => {
    const editing = draft(props.owner, props.index)
    const at = shown(editing.pages).at(-1)
    const pages = at === undefined ? [...editing.pages, { items: [props.held.item!] }] : editing.pages.map((page, index) => (index === at ? { ...page, items: [...page.items, props.held.item!] } : page))
    dismiss()
    keep([{ ...editing, pages: fitted(pages) }])
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
  // Up and down move between what can be done, as a menu's keys do, and Home and End to the first and the last; Escape
  // puts it down.
  const keys = (event: KeyboardEvent) => {
    if (event.key === "Escape") return dismiss()
    const items = [...menu.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")]
    const at = items.indexOf(document.activeElement as HTMLButtonElement)
    const to = { ArrowDown: at + 1, ArrowUp: at < 0 ? items.length - 1 : at - 1, Home: 0, End: items.length - 1 }[event.key]
    if (to === undefined || !items.length) return
    event.preventDefault()
    items[(to + items.length) % items.length].focus()
  }
  const item = props.held.item
  // An app of the App Library that's on a page already has no Add to Home Screen.
  const added = !!item && props.owner.pages.some((page) => page.items.some((other) => !item.widget && other.entrypoint === theirs(item as Entrypoint, props.owner)))
  const library = props.held.from === "library"
  // It's added from here only where it opens, onto the pages of whoever Home was last on; elsewhere, it's carried there.
  const adds = library && !!item && !item.widget && !added && props.owner.arranges && installedFor(item as Entrypoint, props.owner)
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
        popover="manual"
        tabindex="-1"
        ref={menu}
        onToggle={(event) => event.newState === "closed" && dismiss()}
        onKeyDown={keys}
        style={{ left: `${left}px`, "transform-origin": `${held.x + held.width / 2 - left}px ${below ? "0" : "100%"}`, ...(below ? { top: `${held.y + held.height + 12}px` } : { bottom: `${height - held.y + 12}px` }) }}
      >
        <Show when={library && props.held.listed}>
          {(listed) => <p class="hold-for">Installed for {whose(listed().installed.for)}</p>}
        </Show>
        <Show when={!library}>
          <button type="button" role="menuitem" onClick={edit}>
            <span>Edit Home Screen</span>
            <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true"><path d="M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z" /></svg>
          </button>
        </Show>
        <Show when={adds}>
          <button type="button" role="menuitem" onClick={adding}>
            <span>{props.index ? `Add to ${props.owner.name}'s Home Screen` : "Add to Home Screen"}</span>
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

/// Whether an app opens on an owner's pages: it's installed for them, or for everyone.
const installedFor = (item: Entrypoint, owner: Owner) => owner.apps.some((one) => one.app === item.app)

/// An app's entrypoint as an owner's pages name it: by the name its app is installed by for them, which may not be
/// the one it has where it was taken from.
const theirs = (item: Entrypoint, owner: Owner) => {
  const name = owner.apps.find((one) => one.app === item.app)?.name
  return name ? `${name}/${item.entrypoint.slice(item.entrypoint.indexOf("/") + 1)}` : item.entrypoint
}

/// Who an install is for, as a person reads it in a sentence: you, Everyone, or a group by its name.
const whose = (installed: string) => {
  const owner = untrack(owners)?.find((one) => one.who === installed)
  return !owner ? installed : owner.subject ? owner.name : "you"
}

/// Keeps owners' pages as they were edited, each that changed, for everyone who sees them, and says why if they
/// couldn't be.
async function keep(edited: Editing[]) {
  const all = untrack(owners)!
  // Pages as `arrange` takes them, each app by the name it's installed by for the owner, and an empty page gone.
  const asked = (pages: Page[], owner: Owner) =>
    pages.filter((page) => page.items.length).map((page) => ({ items: page.items.map((item) => (item.widget ? { widget: item.widget, size: item.size } : { entrypoint: theirs(item as Entrypoint, owner) })), ...(page.hidden ? { hidden: true } : {}) }))
  const changed = edited.filter((one) => JSON.stringify(asked(one.pages, all[one.owner])) !== JSON.stringify(asked(all[one.owner].pages, all[one.owner])))
  setKept(changed)
  await Promise.all(
    changed.map((one) =>
      call("home-screen_arrange", { pages: asked(one.pages, all[one.owner]) }, { subject: all[one.owner].subject }).catch((error) => toast(error as Error)),
    ),
  )
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
  let overview!: HTMLDivElement
  onSettled(() => {
    const shut = covers(() => home.send({ type: "dismiss" }))
    return () => (shut(), fadeOut(overview))
  })
  return (
    <div class="overview" ref={overview}>
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

/// The App Library's glyph at the end of the dots, a 2×2 grid, Phosphor's squares-four.
const LIBRARY = "M104,40H56A16,16,0,0,0,40,56v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,104,40Zm0,64H56V56h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V56A16,16,0,0,0,200,40Zm0,64H152V56h48v48Zm-96,32H56a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,104,136Zm0,64H56V152h48v48Zm96-64H152a16,16,0,0,0-16,16v48a16,16,0,0,0,16,16h48a16,16,0,0,0,16-16V152A16,16,0,0,0,200,136Zm0,64H152V152h48v48Z"

/// An owner's page dots, a row of the dots pill: one for each of their Home screenfuls, even one alone, the one shown
/// lit. Tapping one goes to its screenful, and sliding a finger along them moves a screenful for every 28 points with
/// a tick at each, as a phone's dots do, on past the last to the App Library. While the owner's pages are edited, they
/// open the page overview.
function Dots(props: { owner: Owner; index: number; count: number }) {
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
      const screen = Math.max(0, Math.min(props.count, from + Math.trunc(across / 28)))
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
  return (
    <nav class="dots" ref={(nav) => sized.observe(nav)} aria-label={`${props.owner.name}'s pages`} inert={seen().owner !== props.index || undefined} onPointerDown={slide}>
      <For each={Array.from({ length: Math.max(1, props.count) }, (_, at) => at)}>
        {(screen) => (
          <button type="button" class="dot" aria-label={`Page ${screen + 1} of ${props.count}`} aria-current={current() === screen ? "page" : undefined} onClick={press(screen)}>
            <i />
          </button>
        )}
      </For>
    </nav>
  )
}

/// The caret under the owner's name, Phosphor's caret-down.
const CARET = "M213.66,101.66l-80,80a8,8,0,0,1-11.32,0l-80-80A8,8,0,0,1,53.66,90.34L128,164.69l74.34-74.35a8,8,0,0,1,11.32,11.32Z"

/// The owner pill, at the top of Home in the middle, in Liquid Glass: whose pages they are, "You ⌄", which opens the
/// list of every owner, the current one ticked. Its name rolls as the owners move. While pages are edited, + and Done
/// are either side of it, and it opens nothing.
function OwnerPill(props: { owners: Owner[] }) {
  const at = () => state().context.at.owner
  let button!: HTMLButtonElement
  return (
    <div class="top-row">
      <button type="button" class="owner-pill pill" ref={button} aria-label={props.owners[at()]?.name} aria-haspopup="menu" aria-expanded={state().matches("listing") ? "true" : "false"} aria-disabled={state().matches("editing") || undefined} onClick={() => home.send({ type: state().matches("listing") ? "dismiss" : "list" })}>
        <span class="roll" aria-hidden="true" ref={rolling}>
          <span class="names">
            <For each={props.owners}>{(owner) => <span ref={(name) => sized.observe(name)}>{owner.name}</span>}</For>
          </span>
        </span>
        <svg class="ui-icon" viewBox="0 0 256 256" aria-hidden="true">
          <path d={CARET} />
        </svg>
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
    </div>
  )
}

/// The dots pill, at the bottom in the middle, just clear of the home indicator, in the owner pill's glass: the dots
/// of the owner shown, then the App Library's glyph, which says there's one more page and goes there, lit while it's
/// shown, as the dots are. It stays where it is as the owners move, and on the App Library too, its dots rolling to
/// the next owner's and taking their number. While pages are edited the glyph stays, dimmed, and goes nowhere.
function DotsPill(props: { owners: Owner[]; laid: Kind[][] }) {
  const count = (owner: number) => (props.laid[owner] ?? []).filter((kind) => kind === "home").length
  const library = () => props.laid[seen().owner]?.[seen().screen] === "library"
  const go = () => {
    if (state().matches("editing")) return
    const screen = props.laid[state().context.at.owner]?.indexOf("library") ?? -1
    if (screen >= 0) home.send({ type: "go", screen })
  }
  return (
    <div class="dots-pill pill">
      <div class="roll" ref={rolling}>
        <div class="rows">
          <For each={props.owners} keyed={(owner) => owner.key}>
            {(owner, index) => <Dots owner={owner()} index={index()} count={count(index())} />}
          </For>
        </div>
      </div>
      <button type="button" class="glyph" aria-label="App Library" aria-current={library() ? "page" : undefined} aria-disabled={state().matches("editing") ? "true" : undefined} onClick={go}>
        <svg viewBox="0 0 256 256" aria-hidden="true">
          <path d={LIBRARY} />
        </svg>
      </button>
    </div>
  )
}

/// Who an owner is in a sentence: you, Everyone, or a group by its name.
const named = (owner: Owner) => (owner.subject ? owner.name : "you")

/// What's carried and stacked on it, each as a person reads its name: an app from the App Library by its app's.
const carriedApps = (held: Held) => [held.item, ...(held.stack ?? [])].flatMap((item) => (item && !item.widget ? [{ item: item as Entrypoint, title: item === held.item ? (held.listed?.title ?? item.name) : item.name }] : []))

/// Apps dropped on an owner they aren't all installed for, who you may install for, wait where they were dropped while
/// one sheet asks about the ones that aren't: Install, as the App Store installs, for them, one after another, their
/// progress shown where they wait, and they stay; or Cancel, and they're back where they came from. If one couldn't be
/// installed they're all back, and a toast says why.
function Asking(props: { owner: Owner; held: Held }) {
  const missing = () => carriedApps(props.held).filter(({ item }) => !installedFor(item, props.owner))
  const titles = () => and(missing().map(({ title }) => title))
  const install = async () => {
    const [owner, apps] = [props.owner, untrack(missing)]
    home.send({ type: "install" })
    for (const { item, title } of apps) {
      try {
        // Named as it is where it was taken from, so the entrypoint it's placed as is its own for them too.
        const done = JSON.parse(await call("install", { app: item.app, name: item.entrypoint.split("/")[0] }, { subject: owner.subject }))
        if (done.also?.length) toast(done.also.join(" "))
      } catch (error) {
        toast(`Couldn't install ${title} for ${named(owner)}. ${(error as Error).message ?? error}`)
        home.send({ type: "failed" })
        return void refresh(owners)
      }
    }
    home.send({ type: "installed" })
    refresh(owners)
  }
  const reach = () => {
    const it = missing().length > 1 ? "them" : "it"
    return props.owner.subject === "network" ? `Everyone in the network will have ${it}.` : props.owner.subject ? `Everyone in ${props.owner.name} will have ${it}.` : `${missing().length > 1 ? "They'll" : "It'll"} be yours.`
  }
  return (
    <Sheet heading={`Install ${titles()} for ${named(props.owner)}?`} open={state().matches({ editing: "asking" })} onClose={() => state().matches({ editing: "asking" }) && home.send({ type: "cancel" })}>
      <div class="asking">
        <div class="icons">
          <For each={missing()} keyed={({ item }) => item.entrypoint}>
            {(app) => <img src={app().item.icon} alt="" />}
          </For>
        </div>
        <p>{reach()}</p>
        <Button onClick={install}>Install</Button>
        <Button variant="secondary" onClick={() => home.send({ type: "cancel" })}>Cancel</Button>
      </div>
    </Sheet>
  )
}

/// An app dropped on an owner whose pages are someone else's to change: a sheet says so, and who can, and it goes back.
function Refused(props: { owner: Owner; held: Held }) {
  const title = () => and(carriedApps(props.held).map(({ title }) => title))
  const who = createMemo(async () => {
    const [{ users }, { people }] = await Promise.all([
      json<{ users: string[] }>("permissions_users", { artifact: props.owner.subject, relation: "owner" }),
      json<{ people: { who: string; called?: string }[] }>("sign-in_everyone").catch(() => ({ people: [] as { who: string; called?: string }[] })),
    ])
    const called = (who: string) => people.find((person) => person.who === who)?.called ?? who.replace(/^users\//, "")
    return new Intl.ListFormat("en", { type: "disjunction" }).format(users.map(called))
  })
  const dismiss = () => state().matches({ editing: "refused" }) && home.send({ type: "dismiss" })
  return (
    <Sheet heading={`You can't add apps for ${props.owner.name}`} open={state().matches({ editing: "refused" })} onClose={dismiss}>
      <div class="asking">
        <Errored fallback={<p>Ask one of its owners to add {title()}.</p>}>
          <Loading fallback={<p>&nbsp;</p>}>
            <p>{who() ? `Only ${who()} can. Ask them to add ${title()}.` : `Ask one of its owners to add ${title()}.`}</p>
          </Loading>
        </Errored>
        <Button onClick={dismiss}>OK</Button>
      </div>
    </Sheet>
  )
}

/// While pages are edited, + and Done float at the top, either side of the owner pill, over the pages, which stay
/// where they are, as on a phone. They fade in and out.
function EditBar() {
  let top!: HTMLDivElement
  onSettled(() => () => fadeOut(top))
  return (
    <div class="top" ref={top}>
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
  let main!: HTMLDivElement
  let sections!: HTMLDivElement
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
      const drafted = editingOwner(index) ? editing()!.pages : state().matches("editing") ? state().context.drafts.find((one) => one.owner === index)?.pages : undefined
      const edited = drafted ?? kept().find((one) => one.owner === index)?.pages ?? owner.pages
      const showing: Page[] = shown(edited).map((at) => edited[at])
      const cells = showing.map((page): Cell[] => page.items.map((item, at) => ({ item, index: at })))
      if (!cells.length) cells.push([])
      return cells.flatMap((page) => (editingOwner(index) ? [pack(page, (cell) => spans(cell.item)).flat()] : pack(page, (cell) => spans(cell.item))))
    }),
  )
  /// Each owner's screenfuls: Home's, each as many of their pages side by side as fit, from which page and how many;
  /// then the App Library, the same for everyone, a screenful.
  const screens = createMemo(() => {
    const fit = room().fit
    return pages().map((homes) => [...Array.from({ length: Math.ceil(homes.length / fit) }, (_, at) => ({ kind: "home" as Kind, from: at * fit, count: Math.min(fit, homes.length - at * fit) })), { kind: "library" as Kind, from: 0, count: 1 }])
  })
  /// Each owner's Home screenfuls, which their row of pages has.
  const homes = (owner: number) => screens()[owner]?.filter(({ kind }) => kind === "home") ?? []
  const laid = createMemo(() => ({ screens: screens().map((owner) => owner.map(({ kind }) => kind)), fit: room().fit }), { equals: (a, b) => JSON.stringify(a) === JSON.stringify(b) })
  // What was just kept shows until the Home Screen has been asked again.
  createEffect(
    () => props.owners,
    () => void setKept([]),
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

  /// The section of the screenful an owner is on: the one shown, for whoever's shown.
  const kindOf = (owner: number) => {
    const { last, screens } = state().context
    return screens[owner]?.[owner === seen().owner ? seen().screen : (last[owner] ?? 0)] ?? "home"
  }
  /// Each owner's Home screenfuls, across: `strips` are the windows onto them, each a screen wide, and `tracks` the rows
  /// that move in them.
  const tracks: Record<number, HTMLElement> = {}
  const height = () => main.clientHeight || root.clientHeight
  /// The owner shown, nearest where the owners are as they move up and down.
  const shownOwner = () => Math.max(0, Math.min(owned().length - 1, Math.round(down.value / height())))
  /// How far apart screenfuls are, a screen's width; how many an owner's row stops at, Home's and then the App
  /// Library's; and how far along its Home's last is.
  const pitch = () => root.clientWidth
  const stops = (owner: number) => untrack(screens)[owner]?.length || 1
  const homeEnd = (owner: number) => Math.max(0, untrack(() => homes(owner).length) - 1) * pitch()
  /// The owners, up and down, and where each is across: one way along Home's screenfuls and on to the App Library,
  /// which the owner shown takes the sections past Home along, as the pager moves them.
  const down: Spring = spring((y) => {
    main.style.translate = `0 ${-y}px`
    roll(y / height())
    look()
    rested(down)
  })
  const across: Spring[] = []
  const sideways = (owner: number) =>
    (across[owner] ??= spring((x) => {
      if (tracks[owner]) tracks[owner].style.translate = `${-Math.min(x, homeEnd(owner)) * rtl()}px 0`
      if (owner === shownOwner()) sections.style.translate = `${-Math.max(0, x - homeEnd(owner)) * rtl()}px 0`
      look()
      rested(across[owner])
    }))
  /// What's carried, while it's dragged, and what it does once the pages under it come to rest.
  let carried: HTMLElement | undefined
  let settledUnder: (() => void) | undefined
  const rested = (one: Spring | undefined) => one && one.value === one.target && !one.velocity && settledUnder?.()
  /// The owner and screenful nearest where they are, which the pills follow.
  const look = () => {
    const owner = shownOwner()
    setSeen({ owner, screen: Math.round(sideways(owner).value / pitch()) })
  }

  /// Moves the pages to `at`, as the machine says: gliding, or at once where they're only laid out again.
  const slide = (at: At, smooth: boolean) => {
    const move = smooth ? glide : jump
    move(down, at.owner * height())
    move(sideways(at.owner), at.screen * pitch())
  }

  /// A finger on the pages swipes them: across between an owner's pages, and up and down between owners, unless the
  /// page scrolls itself, as the App Library does, or they're being edited. Until it has moved 10 points it may be a
  /// tap or a hold; then it's a swipe, along the way it moved most, and the pages follow it from there. Across, on
  /// anything on them that scrolls sideways itself, it's that row's, unless it's at its end that way, as iOS's scroll
  /// views hand a swipe on. While something's carried, a second finger swipes as one does without it,
  /// along Home's pages and between owners, and the first stays with what it carries; its tap on an app stacks it.
  /// A mouse has the wheel and the keys.
  let swipe: { id: number; x: number; y: number; axis?: "x" | "y"; on: Element; row?: HTMLElement; owner: number; from: number; base: number; moves: { time: number; value: number }[] } | undefined
  let swiped = 0
  const carrying = () => state().matches({ editing: "dragging" })
  /// Whether a second finger is swiping the pages under what the first carries.
  const swiping = () => !!swipe?.axis && swipe.id !== carrier
  /// The nearest of what's under a finger that scrolls sideways itself and has more than it shows, short of the pages.
  const row = (on: Element) => {
    for (let at: Element | null = on; at && !at.matches(".page, .sections"); at = at.parentElement)
      if (/auto|scroll/.test(getComputedStyle(at).overflowX) && at.scrollWidth > at.clientWidth + 1) return at as HTMLElement
  }
  /// Whether `row` has more to scroll the way a finger moving `dx` points across takes it.
  const scrolls = (row: HTMLElement, dx: number) => {
    const along = Math.abs(row.scrollLeft)
    return -dx * rtl() > 0 ? along < row.scrollWidth - row.clientWidth - 1 : along > 1
  }
  /// How many screenfuls a swipe across has: Home's, while something's carried, which stays on them.
  const reach = (owner: number) => (carrying() ? untrack(() => homes(owner).length) || 1 : stops(owner))
  const press = (event: PointerEvent) => {
    const on = event.target as Element
    // The pills over Home are their own: the owner's list, and the dots, which a finger slides along.
    if (event.pointerType === "mouse" || event.button || !on.closest(".sections") || on.closest(".pill")) return
    // A category shrinking back into its place lets the pages under it have the finger, and while something's carried,
    // every finger but the one carrying it has them, one at a time.
    const second = carrying() && event.pointerId !== carrier && !swipe
    if (!second && !state().matches("nothing") && !state().matches({ category: "closing" }) && !(state().matches({ editing: "still" }) && !on.closest(".cell"))) return
    const owner = shownOwner()
    // A finger that lands while they move catches them.
    stop(down)
    stop(sideways(owner))
    swipe = { id: event.pointerId, x: event.clientX, y: event.clientY, on, row: row(on), owner, from: 0, base: 0, moves: [] }
  }
  const drag = (event: PointerEvent) => {
    if (!swipe || event.pointerId !== swipe.id) return
    if (!swipe.axis) {
      const [dx, dy] = [event.clientX - swipe.x, event.clientY - swipe.y]
      if (Math.hypot(dx, dy) < 10) return
      const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y"
      // A hold that began has the finger, and up and down is the page's own where it scrolls, and nobody's while
      // editing, but a second finger's while something's carried: an app, which goes to another owner, not a widget.
      const free = state().matches("nothing") || state().matches({ category: "closing" }) || state().matches({ editing: "still" }) || (carrying() && swipe.id !== carrier)
      const vertical = carrying() ? !state().context.held?.item?.widget : !state().matches("editing")
      if (!free || (axis === "y" && (!vertical || kindOf(swipe.owner) !== "home"))) return void (swipe = undefined)
      if (axis === "x" && swipe.row && scrolls(swipe.row, dx)) return void (swipe = undefined)
      const moving = axis === "x" ? sideways(swipe.owner) : down
      const size = axis === "x" ? pitch() : height()
      Object.assign(swipe, { axis, x: event.clientX, y: event.clientY, base: moving.value, from: Math.round(moving.value / size) })
      root.setPointerCapture(event.pointerId)
    }
    const x = swipe.axis === "x"
    const moved = x ? -(event.clientX - swipe.x) * rtl() : -(event.clientY - swipe.y)
    const end = x ? (reach(swipe.owner) - 1) * pitch() : (owned().length - 1) * height()
    const size = x ? pitch() : height()
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
    // A second finger's tap on an app stacks it on what's carried, and the pages it caught go on where they were going.
    if (!done.axis) {
      if (!carrying() || done.id === carrier) return
      slide(home.getSnapshot().context.at, true)
      if (event.type === "pointerup") stack(done.on)
      return
    }
    swiped = performance.now()
    const velocity = event.type === "pointercancel" ? 0 : speed(done.moves, event.timeStamp)
    const { at } = home.getSnapshot().context
    if (done.axis === "x") {
      const moving = sideways(done.owner)
      const screen = settle(done.from, moving.value / pitch(), velocity, reach(done.owner))
      glide(moving, screen * pitch(), velocity)
      // What's carried goes to the page swiped to, as it does held at the edge; there the finger places it.
      if (carrying() && screen !== at.screen) home.send({ type: "flip", by: screen > at.screen ? 1 : -1 })
      else home.send({ type: "settled", at: { owner: done.owner, screen } })
    } else {
      const owner = settle(done.from, down.value / height(), velocity, owned().length)
      glide(down, owner * height(), velocity)
      const by = owner > at.owner ? 1 : -1
      if (carrying() && owner !== at.owner) home.send({ type: "flip", owner: by, editing: draft(owned()[owner], owner) })
      else home.send({ type: "settled", at: { owner, screen: Math.round(sideways(owner).value / pitch()) } })
    }
  }
  /// The pages have the finger, not the web view the Home Screen is in: an iPhone's scrolls it up and down, bouncing,
  /// whatever `touch-action` says, unless its touches are cancelled. Up and down on a page that scrolls is that page's,
  /// and so is across, on a row that scrolls sideways, until the finger has said which way it's going.
  const touchMove = (event: TouchEvent) => {
    if (swipe && !(swipe.axis !== "x" && (event.target as Element).closest?.(".page.scrolls")) && !(!swipe.axis && swipe.row)) event.preventDefault()
  }
  /// An app a second finger tapped flies from its place onto what's carried, as an iPhone's does, with a tick.
  const joined = (cell: HTMLElement) => {
    haptic("selection")
    const [icon, to] = [cell.querySelector("img"), carried?.querySelector("img")]
    if (!icon || !to || still.matches) return
    const [from, end] = [icon.getBoundingClientRect(), to.getBoundingClientRect()]
    const ghost = icon.cloneNode() as HTMLImageElement
    Object.assign(ghost.style, { position: "fixed", zIndex: "29", left: `${from.x}px`, top: `${from.y}px`, width: `${from.width}px`, height: `${from.height}px`, borderRadius: "22.5%", pointerEvents: "none" })
    root.append(ghost)
    const moved = { translate: `${end.x + end.width / 2 - (from.x + from.width / 2)}px ${end.y + end.height / 2 - (from.y + from.height / 2)}px`, scale: `${end.width / from.width}` }
    ghost.animate([{ translate: "0 0", scale: "1" }, moved], { ...paced("settle"), fill: "forwards" }).finished.finally(() => ghost.remove())
  }
  /// How many are carried, on what's carried, once there's more than one, as an iPhone's count badge says.
  createEffect(
    createMemo(() => state().context.held?.stack?.length ?? 0),
    (stacked) => {
      if (carried && stacked) carried.dataset.count = String(stacked + 1)
      else delete carried?.dataset.count
    },
  )
  /// What a second finger tapped, while something's carried: an app on the page shown joins it.
  const stack = (on: Element) => {
    const cell = on.closest<HTMLElement>(".cell[data-index]:not(.lifted)")
    const page = cell?.closest<HTMLElement>("[data-page]")
    if (!cell || !page) return
    const before = home.getSnapshot().context.held?.stack?.length ?? 0
    // The tap opens nothing, nor takes it off with its ⊖.
    swiped = performance.now()
    home.send({ type: "stack", page: Number(page.dataset.page), index: Number(cell.dataset.index) })
    if ((home.getSnapshot().context.held?.stack?.length ?? 0) > before) joined(cell)
  }
  /// A finger that swiped taps nothing as it lifts.
  const tapped = (event: MouseEvent) => {
    if (performance.now() - swiped < 350) (event.stopPropagation(), event.preventDefault())
  }

  // Each effect below is driven by a memo, which changes only when whether it's so does: given the snapshot itself, an
  // effect runs again on every event, and the drag's would put what's carried down and lift it again at each move.
  /// What's carried, while it's dragged: a copy that lifts larger with a shadow and follows the finger one to one, and
  /// over what it would go before. Held at a side of the screen a moment, it goes on to the next page, or the one
  /// before; at its top or bottom, to the previous owner's pages, or the next one's. Let go, it springs into its place
  /// from where it was, at the speed it was moving, and where that's on an owner it isn't installed for, or whose
  /// pages aren't yours to change, what follows is asked or said.
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
      // It's what was lifted, as it's drawn, its widget's content and all, less its ⊖; an app from the App Library,
      // which isn't drawn on a page yet, its icon and name.
      const lifted = root.querySelector<HTMLElement>(".strip .cell.lifted")
      if (lifted) {
        copy.append(...[...lifted.childNodes].map((node) => node.cloneNode(true)))
        copy.querySelector(".minus")?.remove()
        if (item.size) copy.dataset.size = item.size
        copy.style.setProperty("--tile", getComputedStyle(lifted).getPropertyValue("--tile"))
      } else copy.innerHTML = `<span class="tile"><span class="icon"><img src="${(item as Entrypoint).icon}" alt=""></span><span>${(item as Entrypoint).name}</span></span>`
      root.append(copy)
      if (!still.matches) copy.animate([{ scale: `${scale}`, filter: "drop-shadow(0 0 0 oklch(0 0 0 / 0))" }, {}], paced("settle"))
      haptic("medium")
      const id = carrier
      carried = copy
      const moves: { time: number; x: number; y: number }[] = []
      let edge: ReturnType<typeof setTimeout> | undefined
      let side: "" | "left" | "right" | "up" | "down" = ""
      // A page, or an owner, turns after a moment at its edge, and again while it stays there, as an iPhone's does,
      // unless a second finger is turning them.
      const turn = () => {
        const { at } = home.getSnapshot().context
        if (swiping()) {
          // It waits for the second finger.
        } else if (side === "left" || side === "right") home.send({ type: "flip", by: side === "left" ? -1 : 1 })
        else {
          const [by, all] = [side === "up" ? -1 : 1, untrack(owners)!] as const
          if (all[at.owner + by]) home.send({ type: "flip", owner: by, editing: draft(all[at.owner + by], at.owner + by) })
        }
        const now = home.getSnapshot().context.at
        if (now.screen !== at.screen || now.owner !== at.owner) haptic("selection")
        edge = setTimeout(turn, 900)
      }
      const move = (event: PointerEvent) => {
        if (event.pointerId !== id) return
        follow(event.clientX, event.clientY)
        moves.push({ time: event.timeStamp, x: event.clientX, y: event.clientY })
        if (moves.length > 32) moves.shift()
        const strip = strips[home.getSnapshot().context.at.owner]
        const bounds = strip.getBoundingClientRect()
        const [x, y] = [event.clientX, event.clientY]
        const now = x < bounds.left + 28 ? "left" : x > bounds.right - 28 ? "right" : y < bounds.top + 36 ? "up" : y > bounds.bottom - 36 ? "down" : ""
        if (now !== side) {
          clearTimeout(edge)
          side = now
          if (side) edge = setTimeout(turn, 600)
        }
        if (side) clearTimeout(resting)
        else over(x, y)
      }
      // The place it's over, once it has rested there a moment, as an iPhone's does, rather than at every point it
      // passes: the place where the page's own packing would put it nearest where it's drawn, so the rest keep their
      // order and it lands only where it fits. While a second finger swipes the pages under it, it waits for them.
      let resting: ReturnType<typeof setTimeout> | undefined
      let wanted = ""
      const over = (x: number, y: number) => {
        if (swiping()) return
        const page = (document.elementsFromPoint(x, y) as HTMLElement[]).find((element) => element.dataset?.page !== undefined)
        const grid = page?.querySelector<HTMLElement>(".grid")
        const { editing, held } = home.getSnapshot().context
        if (!grid || !editing || !held?.item) return
        const at = Number(page!.dataset.page)
        const items = editing.pages[shown(editing.pages)[at]]?.items.filter((one) => !carries(held, one)) ?? []
        // Its top left, in the page's cells, from where it's drawn: the cells are a grid's tracks and gaps, drawn as
        // much larger as the pages are.
        const box = grid.getBoundingClientRect()
        const style = getComputedStyle(grid)
        const drawn = box.width / grid.offsetWidth
        const [across, down] = [(box.width + parseFloat(style.columnGap) * drawn) / COLUMNS, (box.height + parseFloat(style.rowGap) * drawn) / ROWS]
        const [left, top] = [x - grab.x + rect.x, y - grab.y + rect.y]
        const index = nearest(items, held.item, (left - box.left) / across + 1, (top - box.top) / down + 1)
        const place = at === held.page && index === held.index ? "" : `${at} ${index}`
        if (place === wanted) return
        wanted = place
        clearTimeout(resting)
        if (!place) return
        resting = setTimeout(() => {
          wanted = ""
          home.send({ type: "over", page: at, index })
        }, REST)
      }
      // Where it was last, so it can settle there once it's let go.
      let left = held
      // How fast the finger was going as it let go, which the copy goes on at.
      let thrown = { x: 0, y: 0 }
      // Once the pages a second finger turned come to rest, what's carried goes where the first is.
      settledUnder = () => !side && over(pointer.x, pointer.y)
      const drop = (event: PointerEvent) => {
        if (event.pointerId !== id) return
        left = home.getSnapshot().context.held ?? left
        if (event.type === "pointerup") thrown = { x: speed(moves.map(({ time, x }) => ({ time, value: x })), event.timeStamp), y: speed(moves.map(({ time, y }) => ({ time, value: y })), event.timeStamp) }
        const { held, at } = home.getSnapshot().context
        const owner = untrack(owners)![at.owner]
        const items = held?.item ? [held.item, ...(held.stack ?? [])] : []
        const to: Drop = !owner.arranges ? "refuse" : items.every((item) => item.widget || installedFor(item as Entrypoint, owner)) ? "place" : "ask"
        home.send({ type: "drop", to })
      }
      addEventListener("pointermove", move)
      addEventListener("pointerup", drop)
      addEventListener("pointercancel", drop)
      return () => {
        clearTimeout(edge)
        clearTimeout(resting)
        carried = undefined
        settledUnder = undefined
        // A second finger still swiping lets go of the pages: they go back to where what was carried was put down.
        if (swipe && swipe.id !== id) {
          swipe = undefined
          slide(home.getSnapshot().context.at, true)
        }
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
    const arranging = home.on("arrange", ({ edited }) => keep(edited))
    // The room it's given may have changed since it was first drawn, and every owner's pages stay on the page they
    // were. It's laid out for the new size first, so where the pages are is measured from what's drawn.
    const resized = () => {
      setRoom(measure(root.clientWidth, root.clientHeight))
      flush()
      // Once it's drawn again for the new size, where each cell is is where it is.
      requestAnimationFrame(() => reflow(cells(), false))
      const { at, last } = home.getSnapshot().context
      owned().forEach((_, owner) => jump(sideways(owner), (owner === at.owner ? at.screen : (last[owner] ?? 0)) * pitch()))
      jump(down, at.owner * height())
    }
    const watched = new ResizeObserver(resized)
    watched.observe(root)

    // A trackpad or a wheel: one gesture is one screenful or one owner at most, however hard it's flicked, as macOS's
    // Launchpad and an iPad with a trackpad page. A browser says nothing of where a gesture starts or ends, nor tells
    // its momentum from the fingers, so a gesture is a run of wheel events: its momentum follows the fingers with no
    // pause until it dies away, and is part of it, however its pushes rise and fall as they're coalesced. It ends once
    // the events stop for a moment, or turn back. It steps once its pushes along its axis add up to 30 points, and the
    // rest of it is let be. A mouse wheel's notches, where they come in lines or pages (`deltaMode`), are a step each.
    // Up and down on a page that scrolls itself is that page's.
    const QUIET = 120
    let gesture: { axis: "x" | "y"; sign: number; total: number; done: boolean } | undefined
    let last = -Infinity
    const wheel = (event: WheelEvent) => {
      if (!home.getSnapshot().matches("nothing")) return
      const quiet = event.timeStamp - last > QUIET
      last = event.timeStamp
      const along = (axis: "x" | "y") => (axis === "x" ? event.deltaX : event.deltaY)
      const notch = event.deltaMode !== WheelEvent.DOM_DELTA_PIXEL
      const back = !!gesture?.sign && Math.sign(along(gesture.axis)) === -gesture.sign && Math.abs(along(gesture.axis)) > 2
      if (!gesture || quiet || notch || back) {
        const axis = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? "x" : "y"
        gesture = { axis, sign: 0, total: 0, done: false }
      }
      gesture.sign ||= Math.sign(along(gesture.axis))
      if (gesture.axis === "y" && (event.target as Element).closest?.(".page.scrolls")) return
      event.preventDefault()
      if (gesture.done) return
      gesture.total += along(gesture.axis)
      if (!gesture.total || (!notch && Math.abs(gesture.total) < 30)) return
      gesture.done = true
      const by = Math.sign(gesture.total) as 1 | -1
      home.send(gesture.axis === "y" ? { type: "step", owner: by } : { type: "step", screen: (by * rtl()) as 1 | -1 })
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
      if ((event.target as Element).closest?.("dialog, input")) return
      // Escape closes the page overview, and otherwise ends editing, carrying something or not: nothing's open over it.
      if (event.key === "Escape" && home.getSnapshot().matches({ editing: "overview" })) return home.send({ type: "dismiss" })
      if (event.key === "Escape" && (home.getSnapshot().matches({ editing: "still" }) || home.getSnapshot().matches({ editing: "dragging" }))) return home.send({ type: "done" })
      if (!home.getSnapshot().matches("nothing")) return
      const to = ({ ArrowRight: { screen: rtl() }, ArrowLeft: { screen: -rtl() }, ArrowDown: { owner: 1 }, ArrowUp: { owner: -1 } } as Record<string, { screen: 1 | -1 } | { owner: 1 | -1 }>)[event.key]
      if (!to || ("owner" in to && kindOf(home.getSnapshot().context.at.owner) !== "home")) return
      event.preventDefault()
      home.send({ type: "step", ...to })
    }
    addEventListener("keydown", keys)
    return () => (sliding.unsubscribe(), arranging.unsubscribe(), watched.disconnect(), root.removeEventListener("wheel", wheel), removeEventListener("keydown", keys), root.removeEventListener("pointerdown", press), root.removeEventListener("pointermove", drag), root.removeEventListener("pointerup", lift), root.removeEventListener("pointercancel", lift), root.removeEventListener("click", tapped, true), root.removeEventListener("touchmove", touchMove))
  })

  // Editing begins with a tick, as an iPhone's does; lifting something into it plays its own.
  createEffect(
    createMemo(() => state().matches("editing")),
    (now) => void (now && home.getSnapshot().matches({ editing: "still" }) && haptic("selection")),
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
    <div class={["home", { wide: room().wide, editing: state().matches("editing"), carrying: state().matches({ editing: "dragging" }) }]} style={{ "--pages-scale": room().scale, "--pages-side": `${room().side}px`, "--pages-gap": `${room().gap}px`, "--pages": room().fit, "--pages-shown": screens()[state().context.at.owner]?.[state().context.at.screen]?.count ?? 1, "--hold": `${HOLD}ms` }} ref={root} onClick={(event) => Date.now() > quiet && state().matches({ editing: "still" }) && !(event.target as Element).closest(".cell, .top, .pill, .overview, .ui-dialog") && home.send({ type: "done" })}>
      <div class="sections" ref={sections}>
        <main class="home-section" aria-label="Home">
          <div class="owners" ref={main}>
            <For each={props.owners} keyed={(owner) => owner.key}>
              {(owner, index) => (
                <section class="owner" aria-label={owner().name}>
                  <div class="strip" ref={registered(strips, index)}>
                    <div class="track" ref={registered(tracks, index)}>
                      <For each={homes(index())} keyed={false}>
                        {(screen) => (
                          <div class="screen home">
                            <For each={(pages()[index()] ?? []).slice(screen().from, screen().from + screen().count)} keyed={false}>
                              {(placed, at) => <HomePage owner={owner()} index={index()} page={screen().from + at} placed={placed()} />}
                            </For>
                          </div>
                        )}
                      </For>
                    </div>
                  </div>
                </section>
              )}
            </For>
          </div>
          <OwnerPill owners={props.owners} />
        </main>
        <Library owner={props.owners[0]} places={places} />
      </div>
      <DotsPill owners={props.owners} laid={laid().screens} />
      <Show when={state().matches("editing")}>
        <EditBar />
      </Show>
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[0]} opened={opened()} places={places} />}</Show>
      <Show when={held()}>{(held) => <Holding owner={owner()} index={state().context.at.owner} held={held()} />}</Show>
      <Show when={state().matches({ editing: "overview" }) && editing()}>
        {(edited) => <Overview owner={props.owners[edited().owner]} index={edited().owner} />}
      </Show>
      <Show when={editing()}>{(edited) => <Gallery owner={props.owners[edited().owner]} />}</Show>
      <Show when={state().matches("editing") && !state().context.held?.item?.widget && state().context.held?.item && state().context.held}>
        {(held) => (
          <>
            <Asking owner={owner()} held={held()} />
            <Show when={owner().subject}>
              <Refused owner={owner()} held={held()} />
            </Show>
          </>
        )}
      </Show>
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
