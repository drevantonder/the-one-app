// The Home Screen: two sections side by side, Home and the App Library, as an iPhone's. Home is every owner's pages:
// yours, everyone's, then each of your groups', one owner above another, each a row of screenfuls, each as many of
// their pages side by side as fit, a phone's page drawn larger to suit the window's height on a wide one, and centred
// where there are fewer (layout.ts). The App Library, every app you can reach, by category, is one page, for everyone,
// a screen wide, that scrolls up and down. Left and right moves a screenful, as a phone's pages do, through Home's
// screenfuls and on to the App Library; up and down on Home moves between owners. The pages move by hand, as a
// phone's do (pager.ts). On Home, a small pill at the top in the middle says whose pages they are and opens a list of
// every owner. At the bottom, just clear of the home indicator, the dots pill has the owner's page dots, then the App
// Library's glyph, lit there. The App Store is an app like any other, in the App Library. A page never scrolls: it's a
// fixed grid, and what doesn't fit goes on to the next. Whoever arranges an owner's pages edits them as a phone's Home
// Screen is edited: touch and hold an app or a widget (press and hold it, or right-click it, with a mouse), then Edit
// Home Screen, or hold it a while longer, or touch and hold a page's empty space. An app held in the App Library is
// carried onto any owner's pages. Where it is, what's open over it and the editing are one state machine (machine.ts),
// whose snapshot is one signal here. It's written on the UI library's primitives, so a native host draws it as the DOM does.
import { createEffect, createMemo, createSignal, Errored, For, Loading, onSettled, refresh, Show, untrack } from "solid-js"
import { type WheelEventData, WheelGestures } from "wheel-gestures"
import { Button, ScrollView, covers, createVelocityTracker, haptic, keydown, link, type Pressed, Problem, type Rect, reducedMotion, render, safeArea, sizeChanged, View, viewport, visible, type Wheeled } from "ui"
import { give, glide, jump, settle, spring, type Spring, stop, thrower } from "./pager.ts"
import { ACROSS, at as gridAt, card as cardOf, cell as cellOf, GAP, group, INSET, launcherRoom, lefts, measure, PAGE, PILLS, SEARCH, SEARCH_HEIGHT, TOP_ROW } from "./layout.ts"
import { type At, carries, type Drop, type Held, type Kind, nearest, pack, type Page, type Placed, shown, spans } from "./machine.ts"
import { type Entrypoint, type Widget as WidgetItem } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, seen, setSeen, setAlong, setRolled, editing, editingOwner, launcher, query, setQuery, chosenResult, setChosenResult, type Selected, selected, setSelected, room, setRoom, laidPage, kept, setKept, quietFor, finger, setLanding } from "./store.ts"
import { type Owner, owners, findIn, type From, launch, draft, opens, press, opener, installedFor, keep } from "./owners.ts"
import { Card, Count, Tile } from "./tiles.tsx"
import { type Cell, HomePage } from "./pages.tsx"
import { tween } from "./tween.ts"
import { Library } from "./library.tsx"
import { OpenCategory } from "./category.tsx"
import { Holding } from "./holding.tsx"
import { Overview } from "./overview.tsx"
import { Gallery, Asking, Refused, EditBar } from "./editing.tsx"
import { OwnerPill, DotsPill } from "./pills.tsx"
import { launcherKeys, LauncherSearch, Results, type Target } from "./launcher.tsx"

/// How long the launcher, put away, stays where it was: Raycast's Pop to Root Search's default.
const POP_TO_ROOT = 90_000

/// Where the launcher's card's top is, under its search.
const CARD_TOP = SEARCH_HEIGHT + GAP

// What's carried makes room where it rests this long, as an iPhone's does, not at every place it passes.
const REST = 150

/// What's shown while `when` gives it, kept as it was for `ms` after, as it goes: what to show, and whether it's going.
function lingering<T>(when: () => T | false | undefined | null, ms: number): [() => T | undefined, () => boolean] {
  const [shown, setShown] = createSignal<T>()
  const [going, setGoing] = createSignal(false)
  let timer: ReturnType<typeof setTimeout> | undefined
  createEffect(when, (now) => {
    clearTimeout(timer)
    if (now) return void (setGoing(false), setShown(() => now))
    if (untrack(shown) === undefined) return
    setGoing(true)
    timer = setTimeout(() => (setShown(undefined), setGoing(false)), ms)
  })
  return [shown, going]
}

function HomeScreen(props: { owners: Owner[] }) {
  // The owners as they are when a spring, a gesture or what the machine sends asks, outside anything that tracks them.
  const owned = () => untrack(() => props.owners)
  // + and Done, and the pages' overview, fade out as they go, at the settle pace.
  const [editBar, editBarGoing] = lingering(() => state().matches("editing"), 200)
  const [overview, overviewGoing] = lingering(() => state().matches({ editing: "overview" }) && editing(), 200)
  // The room the pages have: the window, or in the launcher, a page in its card.
  createEffect(
    () => ({ size: viewport(), safe: safeArea(), launcher: launcher() }),
    ({ size, safe, launcher }) => void setRoom(launcher ? launcherRoom(safe) : measure(size.width, size.height, safe)),
  )

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
    // The launcher shows `ACROSS` pages side by side and turns them one at a time: a screenful from each page but the last,
    // which while they're edited has its own, with room beside it for another.
    if (launcher())
      return pages().map((homes, owner) => [
        ...Array.from({ length: Math.max(1, homes.length - (editingOwner(owner) ? ACROSS - 2 : ACROSS - 1)) }, (_, at) => ({ kind: "home" as Kind, from: at, count: Math.min(ACROSS, homes.length - at) })),
        { kind: "library" as Kind, from: 0, count: 1 },
      ])
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
  /// there is one.
  createEffect(
    () => link() === "library" && { screens: laid().screens },
    (now) => {
      const library = now ? (now.screens[home.getSnapshot().context.at.owner]?.indexOf("library") ?? -1) : -1
      if (library >= 0) home.send({ type: "link", screen: library })
    },
  )

  const library = () => kindOf(state().context.at.owner) === "library"
  /// The section of the screenful an owner is on: the one shown, for whoever's shown.
  const kindOf = (owner: number) => {
    const { last, screens } = state().context
    return screens[owner]?.[owner === seen().owner ? seen().screen : (last[owner] ?? 0)] ?? "home"
  }
  /// How tall and wide a screenful is, and how far its owners and its screenfuls are moved: the owners up and down
  /// (`down`), and each owner's along Home's screenfuls and on to the App Library (`across`), which the owner shown
  /// takes the sections past Home along, as the pager moves them.
  const height = () => untrack(room).height
  const pitch = () => untrack(room).width
  const stops = (owner: number) => untrack(screens)[owner]?.length || 1
  const homeEnd = (owner: number) => Math.max(0, untrack(() => homes(owner).length) - 1) * pitch()
  const [downY, setDownY] = createSignal(0)
  const xs: [() => number, (x: number) => void][] = []
  const xOf = (owner: number) => (xs[owner] ??= createSignal(0))
  /// How opaque each owner's pages are as they cross-fade, for whoever asked for less motion.
  const opacities: [() => number, (opacity: number) => void][] = []
  const opacityOf = (owner: number) => (opacities[owner] ??= createSignal(1))
  /// The owner shown, nearest where the owners are as they move up and down.
  const shownOwner = () => Math.max(0, Math.min(owned().length - 1, Math.round(down.value / height())))
  const down: Spring = spring((y) => {
    setDownY(y)
    setRolled(y / height())
    look()
    rested(down)
  })
  const across: Spring[] = []
  const sideways = (owner: number) =>
    (across[owner] ??= spring(
      (x) => {
        xOf(owner)[1](x)
        look()
        rested(across[owner])
      },
      (opacity) => opacityOf(owner)[1](opacity),
    ))
  /// What's carried, once the pages under it come to rest.
  let settledUnder: (() => void) | undefined
  const rested = (one: Spring | undefined) => one && one.value === one.target && !one.velocity && settledUnder?.()
  /// The owner and screenful nearest where they are, which the pills follow.
  const look = () => {
    const owner = shownOwner()
    setSeen({ owner, screen: Math.round(sideways(owner).value / pitch()) })
    setAlong(sideways(owner).value / pitch())
  }
  /// How far the sections are moved past Home, to the App Library: the owner shown's way along, past their Home.
  const sectionsX = () => {
    const owner = Math.max(0, Math.min(props.owners.length - 1, Math.round(downY() / Math.max(1, room().height))))
    return Math.max(0, xOf(owner)[0]() - Math.max(0, homes(owner).length - 1) * room().width)
  }

  /// The launcher's card: what it shows, Home, the App Library or what was found, all in one card of one size, under
  /// the search, in the middle of the room.
  const showing = () => (query().trim() ? "search" : library() ? "library" : "home")
  const card = () => cardOf(room())
  /// How far an owner's pages are moved along so fewer than fill the card are in its middle.
  const lead = (owner: number) => (Math.max(0, ACROSS - (pages()[owner]?.length ?? 1)) * PAGE) / 2
  const cardLeft = () => (viewport().width - card().width) / 2
  /// How far an owner's pages are moved along in the card, at `x`: there, and past Home's last screenful, on the way
  /// to the App Library, which cross-fades in, giving rather than sliding on.
  const trackX = (owner: number, x = xOf(owner)[0]()) => {
    const end = Math.max(0, homes(owner).length - 1) * PAGE
    return x > end ? end + give(x - end, PAGE) : x
  }
  /// How far an owner's pages have been turned in the card, in points, which the host's own spring moves them by, as
  /// it draws: by the arrow keys and the dots, from rest, and as a swipe's fingers lift, at their speed (`flingOf`, of
  /// the way a second). Where they are is the rest, which follows the fingers.
  const turns: [() => number, (x: number) => void][] = []
  const turnOf = (owner: number) => (turns[owner] ??= createSignal(0))
  const flings: [() => number | undefined, (speed: number | undefined) => void][] = []
  const flingOf = (owner: number) => (flings[owner] ??= createSignal<number | undefined>(undefined))
  /// Whether the launcher's pages turn by the host's spring: unless they're to cross-fade.
  const turnsByHost = () => untrack(launcher) && !untrack(reducedMotion)
  /// Turns an owner's pages in the launcher to `to` by the host's spring, from where they're drawn, setting off at
  /// `velocity`, in points a second, where a swipe left them moving.
  const turn = (owner: number, to: number, velocity?: number) => {
    const moving = sideways(owner)
    stop(moving)
    const by = untrack(() => trackX(owner, to) - trackX(owner, moving.value))
    flingOf(owner)[1](velocity && by ? velocity / by : undefined)
    turnOf(owner)[1](untrack(turnOf(owner)[0]) + by)
    jump(moving, to)
  }
  /// What was typed, kept a moment as it's emptied, while what it found fades.
  const [words] = lingering(() => query().trim() && query(), 400)
  const cardRect = () => ({ x: cardLeft(), y: CARD_TOP, width: card().width, height: card().height })
  // The launcher's panel: its search and its card, as large as they are, which never changes; its host centres it.
  createEffect(
    () => launcher() && group(room()),
    (size) => {
      if (size) void sizeChanged({ width: Math.ceil(size.width), height: Math.ceil(size.height) })
    },
  )

  /// Where a page of an owner is drawn in the window: its left and top, and how much larger it's drawn.
  const pageAt = (owner: number, page: number) => {
    if (untrack(launcher)) return { x: untrack(cardLeft) + INSET + untrack(() => lead(owner)) + page * PAGE - Math.min(sideways(owner).value, homeEnd(owner)), y: CARD_TOP + PILLS, scale: untrack(room).scale }
    const fit = untrack(room).fit
    const screen = Math.floor(page / fit)
    const count = untrack(screens)[owner]?.[screen]?.count ?? 1
    const left = lefts(untrack(room), untrack(laidPage), count)[page - screen * fit] ?? 0
    return { x: screen * pitch() - Math.min(sideways(owner).value, homeEnd(owner)) + left, y: owner * height() - down.value, scale: untrack(room).scale }
  }
  /// Which page of the owner Home is on a point is over, if it's over one, and where on it, as it's laid out.
  const pageUnder = (x: number, y: number) => {
    const owner = home.getSnapshot().context.at.owner
    const laid = untrack(laidPage)
    const count = untrack(pages)[owner]?.length ?? 0
    for (let page = 0; page < count; page++) {
      const at = pageAt(owner, page)
      const [px, py] = [(x - at.x) / at.scale, (y - at.y) / at.scale]
      if (px >= 0 && py >= 0 && px < laid.width && py < laid.height) return { owner, page, x: px, y: py }
    }
  }
  /// What's on a page of the owner Home is on at a point, if anything.
  const cellUnder = (x: number, y: number) => {
    const under = pageUnder(x, y)
    if (!under) return
    const laid = untrack(laidPage)
    const placed = untrack(pages)[under.owner]?.[under.page]?.find((one) => {
      const box = cellOf(laid, one)
      return under.x >= box.x && under.x < box.x + box.width && under.y >= box.y && under.y < box.y + box.height
    })
    return placed && { page: under.page, index: placed.item.index, placed }
  }

  /// In the launcher, what's on the screenful at `at`, as it's read: page by page, then row by row, and across each.
  const ordered = (at: At): Selected[] => {
    const screen = untrack(screens)[at.owner]?.[at.screen]
    if (!screen || screen.kind !== "home") return []
    return Array.from({ length: screen.count }, (_, page) => screen.from + page).flatMap((page) =>
      [...(untrack(pages)[at.owner]?.[page] ?? [])].sort((a, b) => a.row - b.row || a.column - b.column).map((placed) => ({ owner: at.owner, page, index: placed.item.index })),
    )
  }
  const placedOf = (chosen: Selected) => untrack(pages)[chosen.owner]?.[chosen.page]?.find((placed) => placed.item.index === chosen.index)
  const same = (a: Selected | undefined, b: Selected | undefined) => a?.owner === b?.owner && a?.page === b?.page && a?.index === b?.index
  /// What's chosen as a screenful shows: its first app, as Raycast chooses its first item, or a widget where it has none.
  const first = (cells: Selected[]) => cells.find((cell) => !placedOf(cell)?.item.item.widget) ?? cells[0]
  const choose = (cell: Selected | undefined) => setSelected(cell)
  // Whatever moves the pages, the highlight stays on the screenful shown: on what it was on, if that's there.
  createEffect(
    () => launcher() && `${state().context.at.owner} ${state().context.at.screen} ${pages().length}`,
    (on) => {
      if (!on) return
      const cells = ordered(home.getSnapshot().context.at)
      if (!cells.some((cell) => same(cell, untrack(selected)))) choose(first(cells))
    },
  )
  // Put away, the launcher is where it was if it's back within 90 seconds, what was typed too; after that it forgets
  // it and comes back to Home's first page, as Raycast's Pop to Root Search does.
  createEffect(
    () => launcher() && !visible(),
    (hidden) => {
      if (!hidden) return
      const back = setTimeout(() => {
        setQuery("")
        setChosenResult(0)
        if (home.getSnapshot().matches("nothing") && home.getSnapshot().context.at.screen !== 0) home.send({ type: "go", screen: 0 })
        choose(first(ordered(home.getSnapshot().context.at)))
      }, POP_TO_ROOT)
      return () => clearTimeout(back)
    },
    { defer: true },
  )
  /// What the launcher's search found: the apps of your App Library, whose names have what's typed in them.
  const found = createMemo(() => findIn(props.owners[0]?.library ?? [], query(), false))
  /// What the launcher's Return opens and its Actions are for: the app its search chose, or what's highlighted on Home.
  const target = createMemo((): Target | undefined => {
    if (query().trim()) {
      const listed = found()[chosenResult()]
      return listed && { listed, item: opens(listed) }
    }
    const now = selected()
    const { at, screens } = state().context
    if (!launcher() || !now || now.owner !== at.owner || screens[at.owner]?.[at.screen] !== "home") return
    const cell = pages()[now.owner]?.[now.page]?.find(({ item }) => item.index === now.index)
    return cell && { item: cell.item.item, place: { page: now.page, index: now.index } }
  })
  /// Where what's chosen on Home is drawn, which it grows out of as it opens.
  const fromChosen = (chosen: Selected): From | undefined => {
    const placed = placedOf(chosen)
    if (!placed) return
    const laid = untrack(laidPage)
    const at = pageAt(chosen.owner, chosen.page)
    const box = cellOf(laid, placed)
    if (placed.item.item.widget) return { x: at.x + box.x * at.scale, y: at.y + box.y * at.scale, width: box.width * at.scale, height: box.height * at.scale, radius: 22 * at.scale }
    const size = laid.tile * at.scale
    return { x: at.x + box.x * at.scale + (box.width * at.scale - size) / 2, y: at.y + box.y * at.scale, width: size, height: size, radius: size * 0.225 }
  }
  /// Opens what the launcher's Return opens, growing out of its icon or widget.
  const openTarget = () => {
    const now = untrack(target)
    if (now?.listed) return press(now.listed, owned()[0], false)
    const opened = now?.item && opener(now.item)
    const chosen = untrack(selected)
    if (opened) launch(opened, owned()[home.getSnapshot().context.at.owner], false, chosen && fromChosen(chosen))
  }
  /// The launcher's arrow keys, with nothing typed: left and right move the highlight along what's on the pages shown,
  /// and at either end turn the page, on to the App Library past the last; up and down move it up and down a page,
  /// and past its top or bottom to the owner above or below.
  const move = (key: string) => {
    if (!["ArrowRight", "ArrowLeft", "ArrowUp", "ArrowDown"].includes(key)) return false
    const snapshot = home.getSnapshot()
    if (!snapshot.matches("nothing")) return true
    const { at, screens } = snapshot.context
    // Each move goes somewhere, or nowhere, as the step does.
    const step = (to: { screen: 1 | -1 } | { owner: 1 | -1 }, last: boolean) => {
      const before = new Set(ordered(at).map((cell) => cell.page))
      home.send({ type: "step", ...to })
      const now = home.getSnapshot().context.at
      if (now.owner === at.owner && now.screen === at.screen) return
      // Along an owner's pages, onto the page that's just come into view, as the launcher's turn one at a time.
      const all = ordered(now)
      const cells = now.owner === at.owner && all.some((cell) => !before.has(cell.page)) ? all.filter((cell) => !before.has(cell.page)) : all
      choose(last ? cells.at(-1) : cells[0])
    }
    if (screens[at.owner]?.[at.screen] === "library") {
      if (key === "ArrowLeft") step({ screen: -1 }, true)
      return true
    }
    const cells = ordered(at)
    const now = cells.findIndex((cell) => same(cell, untrack(selected)))
    const current = cells[now]
    if (!current) return choose(first(cells)), true
    if (key === "ArrowRight" || key === "ArrowLeft") {
      const next = cells[now + (key === "ArrowRight" ? 1 : -1)]
      if (next) choose(next)
      else step({ screen: key === "ArrowRight" ? 1 : -1 }, key === "ArrowLeft")
      return true
    }
    // Up and down: the nearest row that way on the same page, and the nearest of what's on it across.
    const placed = (cell: Selected) => placedOf(cell)!
    const top = (cell: Selected) => placed(cell).row
    const bottom = (cell: Selected) => placed(cell).row + placed(cell).height - 1
    const middle = (cell: Selected) => placed(cell).column + (placed(cell).width - 1) / 2
    const up = key === "ArrowUp"
    const way = cells.filter((cell) => cell.page === current.page && (up ? bottom(cell) < top(current) : top(cell) > bottom(current)))
    const row = up ? Math.max(...way.map(bottom)) : Math.min(...way.map(top))
    const near = way.filter((cell) => (up ? bottom(cell) : top(cell)) === row).sort((a, b) => Math.abs(middle(a) - middle(current)) - Math.abs(middle(b) - middle(current)))[0]
    if (near) choose(near)
    else step({ owner: up ? -1 : 1 }, false)
    return true
  }
  /// In the launcher, to the App Library, and from there back to the Home screenful it came from.
  let before = 0
  const toggleLibrary = () => {
    const { at, screens } = home.getSnapshot().context
    const library = screens[at.owner]?.indexOf("library") ?? -1
    if (!home.getSnapshot().matches("nothing") || library < 0) return
    if (at.screen === library) return home.send({ type: "go", screen: Math.min(before, library - 1) })
    before = at.screen
    home.send({ type: "go", screen: library })
  }

  /// Moves the pages to `at`, as the machine says: gliding, or at once where they're only laid out again.
  const slide = (at: At, smooth: boolean) => {
    // The launcher's owners roll by its host's springs, so they're drawn where they're going; its pages slide as a
    // phone's do.
    const roll = smooth && !untrack(launcher) ? glide : jump
    roll(down, at.owner * height())
    const [moving, to] = [sideways(at.owner), at.screen * pitch()]
    if (smooth && turnsByHost()) return turn(at.owner, to)
    if (smooth) glide(moving, to)
    else jump(moving, to)
  }

  // A finger on the pages moves them, as a phone's: once it has moved far enough to say which way, along an owner's
  // screenfuls or between owners, the pages follow it one to one, giving at the ends; let go, they go to the nearest,
  // or on the way they were flicked. A mouse doesn't drag them: a trackpad or a wheel pages (`wheel`).
  let swipe: { id: number; x: number; y: number; axis?: "x" | "y"; owner: number; from: number; base: number; moves: ReturnType<typeof createVelocityTracker> } | undefined
  const carrying = () => home.getSnapshot().matches({ editing: "dragging" })
  /// Whether a second finger is swiping the pages under what the first carries.
  const swiping = () => !!swipe?.axis && swipe.id !== finger.carrier
  /// How many screenfuls a swipe across has: Home's, while something's carried, which stays on them.
  const reach = (owner: number) => (carrying() ? untrack(() => homes(owner).length) || 1 : stops(owner))
  const swipes = {
    onPointerDown: (event: Pressed) => {
      finger.pointer = { x: event.clientX, y: event.clientY }
      if (event.pointerType === "mouse" || event.button) return
      // A category shrinking back into its place lets the pages under it have the finger, and while something's
      // carried, every finger but the one carrying it has them, one at a time.
      const second = carrying() && event.pointerId !== finger.carrier && !swipe
      if (!second && !home.getSnapshot().matches("nothing") && !home.getSnapshot().matches({ category: "closing" }) && !home.getSnapshot().matches({ editing: "still" })) return
      const { owner } = home.getSnapshot().context.at
      stop(down)
      stop(sideways(owner))
      swipe = { id: event.pointerId, x: event.clientX, y: event.clientY, owner, from: 0, base: 0, moves: createVelocityTracker() }
    },
    onPointerMove: (event: Pressed) => {
      finger.pointer = { x: event.clientX, y: event.clientY }
      if (!swipe || event.pointerId !== swipe.id) return
      if (!swipe.axis) {
        const [dx, dy] = [event.clientX - swipe.x, event.clientY - swipe.y]
        if (Math.hypot(dx, dy) < 10) return
        const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y"
        // A hold that began has the finger, and up and down is the page's own where it scrolls, and nobody's while
        // editing, but a second finger's while something's carried: an app, which goes to another owner, not a widget.
        const free = home.getSnapshot().matches("nothing") || home.getSnapshot().matches({ category: "closing" }) || home.getSnapshot().matches({ editing: "still" }) || (carrying() && swipe.id !== finger.carrier)
        const vertical = carrying() ? !home.getSnapshot().context.held?.item?.widget : !home.getSnapshot().matches("editing")
        const refused = !free || (axis === "y" && (!vertical || kindOf(swipe.owner) !== "home"))
        // Either way the pages go on: all of them, or the way the finger doesn't swipe, which it takes at once below.
        slide(home.getSnapshot().context.at, true)
        if (refused) return void (swipe = undefined)
        const moving = axis === "x" ? sideways(swipe.owner) : down
        const size = axis === "x" ? pitch() : height()
        Object.assign(swipe, { axis, x: event.clientX, y: event.clientY, base: moving.value, from: Math.round(moving.value / size) })
      }
      const x = swipe.axis === "x"
      const moved = x ? -(event.clientX - swipe.x) : -(event.clientY - swipe.y)
      const end = x ? (reach(swipe.owner) - 1) * pitch() : (owned().length - 1) * height()
      const size = x ? pitch() : height()
      const value = swipe.base + moved
      swipe.moves.add(event.timeStamp, value)
      jump(x ? sideways(swipe.owner) : down, value < 0 ? give(value, size) : value > end ? end + give(value - end, size) : value)
    },
    onPointerUp: (event: Pressed) => lift(event, false),
    onPointerCancel: (event: Pressed) => lift(event, true),
  }
  const lift = (event: Pressed, cancelled: boolean) => {
    if (!swipe || event.pointerId !== swipe.id) return
    // Where the finger lifted is where it last was: moves come once a frame, so the last may be short of it.
    if (!cancelled && swipe.axis) swipes.onPointerMove(event)
    const done = swipe
    swipe = undefined
    // A finger that didn't swipe lets go of the pages it caught, and a second finger's tap on an app stacks it on
    // what's carried.
    if (!done.axis) {
      slide(home.getSnapshot().context.at, true)
      if (carrying() && done.id !== finger.carrier && !cancelled) stack(event.clientX, event.clientY)
      return
    }
    quietFor(350)
    // In points a second, as the pages' spring takes it.
    const velocity = cancelled ? 0 : done.moves.velocity(event.timeStamp) * 1000
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
  /// What a second finger tapped, while something's carried: an app on the page shown joins it, with a tick.
  const stack = (x: number, y: number) => {
    const under = cellUnder(x, y)
    if (!under || carries(home.getSnapshot().context.held, under.placed.item.item)) return
    const before = home.getSnapshot().context.held?.stack?.length ?? 0
    // The tap opens nothing, nor takes it off with its ⊖.
    quietFor(350)
    home.send({ type: "stack", page: under.page, index: under.index })
    if ((home.getSnapshot().context.held?.stack?.length ?? 0) > before) haptic("selection")
  }

  // A trackpad's two fingers along the pages move them one to one, as a finger does, giving at the ends, and as they
  // lift the pages settle as a finger's do, at the speed they were going, and the momentum that follows is let be.
  // Where the system says when they lift (a Mac's, as the wheel event's `phase`), that's when; elsewhere, as on the
  // DOM, wheel-gestures tells, by where the momentum begins, or the events stop. Up and down, or a wheel, is one owner
  // a gesture at most, however hard it's flicked, as macOS's Launchpad and an iPad with a trackpad page, once its
  // pushes add up to 30 points. Up and down over the App Library, which scrolls itself, is its own.
  const trackpad = WheelGestures({ preventWheelAction: false, reverseSign: false })
  let gesture: { axis?: "x" | "y"; owner: number; from: number; base: number; done: boolean } | undefined
  trackpad.on("wheel", ({ isStart, isEnding, isMomentum, axisMovement: [x, y], axisVelocity, event }) => {
    if (isStart) {
      gesture = undefined
      if (!home.getSnapshot().matches("nothing")) return
      gesture = { owner: home.getSnapshot().context.at.owner, base: 0, from: 0, done: false }
    }
    if (!gesture || gesture.done) return
    // Which way it goes is the first way it moves: fingers set down (`mayBegin`) don't move. Along, they catch the
    // pages where they are, as a finger does.
    if (!gesture.axis) {
      if (!event.deltaX && !event.deltaY) return
      gesture.axis = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? "x" : "y"
      const moving = sideways(gesture.owner)
      if (gesture.axis === "x") stop(moving)
      Object.assign(gesture, { base: moving.value - x + event.deltaX, from: Math.round(moving.value / pitch()) })
    }
    if (gesture.axis === "y") {
      if (Math.abs(y) < 30) return
      gesture.done = true
      return home.send({ type: "step", owner: Math.sign(y) as 1 | -1 })
    }
    const moving = sideways(gesture.owner)
    const end = (stops(gesture.owner) - 1) * pitch()
    const lifted = isMomentum || isEnding || (event as Wheeled).phase === "ended" || (event as Wheeled).phase === "cancelled"
    if (!lifted) {
      const value = gesture.base + x
      return jump(moving, value < 0 ? give(value, pitch()) : value > end ? end + give(value - end, pitch()) : value)
    }
    gesture.done = true
    const velocity = isEnding ? 0 : axisVelocity[0] * 1000
    const screen = settle(gesture.from, moving.value / pitch(), velocity, stops(gesture.owner))
    if (turnsByHost()) turn(gesture.owner, screen * pitch(), velocity)
    else glide(moving, screen * pitch(), velocity)
    home.send({ type: "settled", at: { owner: gesture.owner, screen } })
  })
  const wheel = (event: Wheeled) => trackpad.feedWheel({ deltaX: event.deltaX, deltaY: event.deltaY, deltaMode: 0, timeStamp: event.timeStamp, momentum: event.momentum, phase: event.phase } as WheelEventData)

  // What's carried, while it's dragged: a copy that lifts larger and follows the finger one to one, over what it would
  // go before. Held at a side of the screen a moment, it goes on to the next page, or the one before; at its top or
  // bottom, to the previous owner's pages, or the next one's. Let go, it springs into its place from where it was, at
  // the speed it was moving, and where that's on an owner it isn't installed for, or whose pages aren't yours to
  // change, what follows is asked or said.
  const [copy, setCopy] = createSignal<{ held: Held; rect: Rect; x: number; y: number; settling: boolean; lifted: number }>()
  createEffect(
    createMemo(() => state().matches({ editing: "dragging" })),
    (dragging) => {
      if (!dragging) return
      const held = home.getSnapshot().context.held!
      const rect = held.rect ?? { x: finger.pointer.x - 30, y: finger.pointer.y - 40, width: 60, height: 80 }
      setCopy({ held, rect, x: finger.pointer.x - finger.grab.x, y: finger.pointer.y - finger.grab.y, settling: false, lifted: 0 })
      const lifting = tween(200, () => {})
      lifting.to(1)
      createEffect(lifting.value, (lifted) => void setCopy((now) => now && { ...now, lifted }))
      haptic("medium")
      const id = finger.carrier
      const moves = thrower()
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
      const move = (event: Pressed) => {
        if (event.pointerId !== id) return
        finger.pointer = { x: event.clientX, y: event.clientY }
        setCopy((now) => now && { ...now, x: event.clientX - finger.grab.x, y: event.clientY - finger.grab.y })
        moves.add(event)
        const [x, y] = [event.clientX, event.clientY]
        // The pages' edges: the window's, or the launcher's card's, inside its pills.
        const area = untrack(launcher) ? { left: cardLeft(), right: cardLeft() + card().width, top: CARD_TOP + PILLS, bottom: CARD_TOP + card().height - PILLS } : { left: 0, right: pitch(), top: 0, bottom: height() }
        const now = x < area.left + 28 ? "left" : x > area.right - 28 ? "right" : y < area.top + 36 ? "up" : y > area.bottom - 36 ? "down" : ""
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
        const { editing, held } = home.getSnapshot().context
        // Its top left, which the page's cells are counted from.
        const under = pageUnder(x - finger.grab.x + rect.x, y - finger.grab.y + rect.y) ?? pageUnder(x, y)
        if (!under || !editing || !held?.item) return
        const scale = untrack(room).scale
        const corner = pageAt(under.owner, under.page)
        const [left, top] = [(x - finger.grab.x + rect.x - corner.x) / scale, (y - finger.grab.y + rect.y - corner.y) / scale]
        const cells = gridAt(untrack(laidPage), left, top)
        const items = editing.pages[shown(editing.pages)[under.page]]?.items.filter((one) => !carries(held, one)) ?? []
        const index = nearest(items, held.item, cells.column, cells.row)
        const place = under.page === held.page && index === held.index ? "" : `${under.page} ${index}`
        if (place === wanted) return
        wanted = place
        clearTimeout(resting)
        if (!place) return
        resting = setTimeout(() => {
          wanted = ""
          home.send({ type: "over", page: under.page, index })
        }, REST)
      }
      // Where it was last, so it can settle there once it's let go.
      let left = held
      // How fast the finger was going as it let go, which the copy goes on at.
      let thrown = { x: 0, y: 0 }
      // Once the pages a second finger turned come to rest, what's carried goes where the first is.
      settledUnder = () => !side && over(finger.pointer.x, finger.pointer.y)
      const drop = (event: Pressed) => {
        if (event.pointerId !== id) return
        left = home.getSnapshot().context.held ?? left
        if (event.timeStamp) thrown = moves.thrown(event.timeStamp)
        const { held, at } = home.getSnapshot().context
        const owner = untrack(owners)![at.owner]
        const items = held?.item ? [held.item, ...(held.stack ?? [])] : []
        const to: Drop = !owner.arranges ? "refuse" : items.every((item) => item.widget || installedFor(item as Entrypoint, owner)) ? "place" : "ask"
        home.send({ type: "drop", to })
      }
      finger.carry = { move, drop }
      return () => {
        finger.carry = undefined
        clearTimeout(edge)
        clearTimeout(resting)
        settledUnder = undefined
        // A second finger still swiping lets go of the pages: they go back to where what was carried was put down.
        if (swipe && swipe.id !== id) {
          swipe = undefined
          slide(home.getSnapshot().context.at, true)
        }
        quietFor(400)
        haptic("light")
        // What was carried settles into its place as it's let go, rather than vanishing over it: a spring from where
        // the finger left it, at the speed it was going, to its place, wherever that is as it goes.
        const owner = home.getSnapshot().context.at.owner
        const placed = untrack(pages)[owner]?.[left.page]?.find((one) => one.item.index === left.index)
        if (!placed || untrack(reducedMotion)) return void setCopy()
        setLanding({ owner, page: left.page, index: left.index })
        const target = () => {
          const at = pageAt(owner, left.page)
          const box = cellOf(untrack(laidPage), placed)
          return { x: at.x + (box.x + box.width / 2) * at.scale - (rect.x + rect.width / 2), y: at.y + (box.y + box.height / 2) * at.scale - (rect.y + rect.height / 2) }
        }
        const draw = () => {
          // Its place may still be on its way, as the pages turn: the copy goes after it.
          const to = target()
          if (Math.abs(x.target - to.x) > 0.5) glide(x, to.x)
          if (Math.abs(y.target - to.y) > 0.5) glide(y, to.y)
          setCopy((now) => now && { ...now, x: x.value, y: y.value, settling: true })
          if (x.value === x.target && y.value === y.target && !x.velocity && !y.velocity) (setCopy(), setLanding())
        }
        const [x, y] = [spring(draw), spring(draw)]
        const from = untrack(copy)!
        const to = target()
        Object.assign(x, { value: from.x, target: to.x })
        Object.assign(y, { value: from.y, target: to.y })
        glide(x, to.x, thrown.x)
        glide(y, to.y, thrown.y)
      }
    },
  )

  onSettled(() => {
    const sliding = home.on("slide", ({ at, smooth }) => slide(at, smooth))
    const arranging = home.on("arrange", ({ edited }) => keep(edited))
    // The arrow keys move as a swipe does. Escape ends editing.
    const keys = keydown((event) => {
      if (event.altKey || event.metaKey || event.ctrlKey) return
      // Escape closes the page overview, and otherwise ends editing, carrying something or not: nothing's open over it.
      if (event.key === "Escape" && home.getSnapshot().matches({ editing: "overview" })) return home.send({ type: "dismiss" })
      if (event.key === "Escape" && (home.getSnapshot().matches({ editing: "still" }) || home.getSnapshot().matches({ editing: "dragging" }))) return home.send({ type: "done" })
      if (!home.getSnapshot().matches("nothing") || untrack(launcher)) return
      const to = ({ ArrowRight: { screen: 1 }, ArrowLeft: { screen: -1 }, ArrowDown: { owner: 1 }, ArrowUp: { owner: -1 } } as Record<string, { screen: 1 | -1 } | { owner: 1 | -1 }>)[event.key]
      if (!to || ("owner" in to && kindOf(home.getSnapshot().context.at.owner) !== "home")) return
      event.preventDefault()
      home.send({ type: "step", ...to })
    })
    return () => (sliding.unsubscribe(), arranging.unsubscribe(), keys())
  })
  // The room it's given may change, and every owner's pages stay on the page they were.
  createEffect(
    () => ({ width: room().width, height: room().height, fit: room().fit }),
    () => {
      const { at, last } = home.getSnapshot().context
      owned().forEach((_, owner) => jump(sideways(owner), (owner === at.owner ? at.screen : (last[owner] ?? 0)) * pitch()))
      jump(down, at.owner * height())
    },
  )
  // Editing begins with a tick, as an iPhone's does; lifting something into it plays its own.
  createEffect(
    createMemo(() => state().matches("editing")),
    (now) => void (now && home.getSnapshot().matches({ editing: "still" }) && haptic("selection")),
    { defer: true },
  )
  // While pages are edited, the device's own back ends editing, as Done does, and the window's rail keeps out of the
  // way, as iOS's chrome does while its icons are rearranged: something of the Home Screen's own is open over it.
  createEffect(
    createMemo(() => state().matches("editing")),
    (now) => (now ? covers(() => home.send({ type: "done" })) : undefined),
  )

  const opened = () => state().context.category
  const held = () => (state().matches("holding") ? state().context.held : undefined)
  const owner = () => props.owners[state().context.at.owner]
  return (
    <View
      onKeyDown={launcher() ? launcherKeys({ move, open: openTarget, toggleLibrary, library, found: () => found().length }) : undefined}
      style={[looks.root, looks.sized(launcher() ? viewport().width : room().width, viewport().height)]}
    >
      <Show
        when={launcher()}
        fallback={
          <>
        <View
          {...swipes}
          onWheel={wheel}
          style={[looks.sections, looks.sized(room().width * 2, room().height), looks.shifted(-sectionsX())]}
        >
          <View role="main" aria-label="Home" style={[looks.clipped, looks.rigid, looks.sized(room().width, room().height)]}>
            <View style={[looks.owners, looks.wide(room().width), looks.lowered(-downY())]}>
              <For each={props.owners} keyed={(owner) => owner.key}>
                {(owner, index) => (
                  <View role="region" aria-label={owner().name} style={[looks.clipped, looks.sized(room().width, room().height)]}>
                    <View style={[looks.screens, looks.slid(-Math.min(xOf(index())[0](), Math.max(0, homes(index()).length - 1) * room().width), opacityOf(index())[0]())]}>
                      <For each={homes(index())} keyed={false}>
                        {(screen) => (
                          <View style={[looks.slots, looks.sized(room().width, room().height)]}>
                            <For each={(pages()[index()] ?? []).slice(screen().from, screen().from + screen().count)} keyed={false}>
                              {(placed, at) => (
                                <View
                                  style={[looks.absolute, looks.placedPage((laidPage().height * (room().scale - 1)) / 2, lefts(room(), laidPage(), screen().count)[at] + (laidPage().width * (room().scale - 1)) / 2, room().scale)]}
                                >
                                  <HomePage owner={owner()} index={index()} page={screen().from + at} placed={placed()} laid={laidPage()} scale={room().scale} />
                                </View>
                              )}
                            </For>
                          </View>
                        )}
                      </For>
                    </View>
                  </View>
                )}
              </For>
            </View>
          </View>
          <Library owner={props.owners[0]} />
        </View>
            <Show when={!library()}>
              <View style={[looks.topRow, looks.top(room().safe.top), looks.tall(TOP_ROW)]}>
                <View style={[looks.live, looks.opacity(state().matches("editing") ? 0.6 : 1)]}>
                  <OwnerPill owners={props.owners} />
                </View>
              </View>
            </Show>
            <View style={[looks.bottomRow, looks.bottom(room().safe.bottom + 16)]}>
              <View style={looks.live}>
                <DotsPill owners={props.owners} laid={laid().screens} />
              </View>
            </View>
          </>
        }
      >
        <View style={[looks.surface, looks.search, looks.at((viewport().width - SEARCH) / 2, 0, SEARCH, SEARCH_HEIGHT)]}>
          <LauncherSearch />
        </View>
        <View onWheel={wheel} style={[looks.surface, looks.card, looks.at(0, CARD_TOP, card().width, card().height), looks.shifted(cardLeft())]}>
          {/* Home: each owner's pages, the owner shown's in place, the others rolled above and below it, faded. */}
          <View role="main" aria-label="Home" inert={showing() !== "home" || undefined} style={[looks.fading, looks.cover, showing() !== "home" && looks.inert, looks.faded(showing() === "home" ? 1 : 0, showing() === "home" ? 1 : 1.03)]}>
            <For each={props.owners} keyed={(owner) => owner.key}>
              {(owner, index) => {
                const shown = () => index() === state().context.at.owner
                const Page = (inner: { page: number; placed: Placed<Cell>[] }) => (
                  <View style={[looks.slots, looks.rigid, looks.sized(PAGE, room().height)]}>
                    <View style={[looks.absolute, looks.placedPage((laidPage().height * (room().scale - 1)) / 2, (laidPage().width * (room().scale - 1)) / 2, room().scale)]}>
                      <HomePage owner={owner()} index={index()} page={inner.page} placed={inner.placed} laid={laidPage()} scale={room().scale} />
                    </View>
                  </View>
                )
                return (
                  <View
                    role="region"
                    aria-label={owner().name}
                    inert={!shown() || undefined}
                    style={[looks.owner, looks.rolling, !shown() && looks.inert, looks.at(INSET, PILLS, ACROSS * PAGE, room().height), looks.drifted(shown() ? 1 : 0, (index() - state().context.at.owner) * 40)]}
                  >
                    <View style={[looks.track, looks.slid(lead(index()) + turnOf(index())[0]() - trackX(index()), opacityOf(index())[0]())]}>
                      <View style={[looks.turning, flingOf(index())[0]() !== undefined && looks.flung(flingOf(index())[0]()!), looks.shifted(-turnOf(index())[0]())]}>
                        <For each={pages()[index()] ?? []} keyed={false}>
                          {(placed, at) => <Page page={at} placed={placed()} />}
                        </For>
                      </View>
                    </View>
                  </View>
                )
              }}
            </For>
          </View>
          {/* The App Library, in the same card, cross-fading in, scrolling what's taller. */}
          <View inert={showing() !== "library" || undefined} style={[looks.layer, looks.fading, showing() !== "library" && looks.inert, looks.fill(INSET, 0, 0), looks.faded(showing() === "library" ? 1 : 0, showing() === "library" ? 1 : 0.965)]}>
            <Library owner={props.owners[0]} size={{ width: card().width - 2 * INSET, height: card().height - INSET - PILLS }} />
          </View>
          {/* What was typed found, as it's typed, and as it goes, what it last found. */}
          <View inert={showing() !== "search" || undefined} style={[looks.owner, looks.fading, showing() !== "search" && looks.inert, looks.at(INSET, INSET, card().width - 2 * INSET, card().height - 2 * INSET), looks.drifted(showing() === "search" ? 1 : 0, showing() === "search" ? 0 : -8)]}>
            <ScrollView style={looks.cover}>
              <Show when={words()}>{(typed) => <Results owner={props.owners[0]} found={findIn(props.owners[0]?.library ?? [], typed(), false)} words={typed()} />}</Show>
            </ScrollView>
          </View>
          {/* Whose pages they are at the top, as a phone has it, and their dots at the foot, with the App Library's glyph. */}
          <View inert={showing() !== "home" || undefined} style={[looks.row, looks.fading, showing() !== "home" && looks.inert, looks.top(0), looks.tall(PILLS), looks.opacity(showing() === "home" ? (state().matches("editing") ? 0.6 : 1) : 0)]}>
            <OwnerPill owners={props.owners} />
          </View>
          <View inert={showing() === "search" || undefined} style={[looks.row, looks.fading, showing() === "search" && looks.inert, looks.bottom(0), looks.tall(PILLS), looks.opacity(showing() === "search" ? 0 : 1)]}>
            <DotsPill owners={props.owners} laid={laid().screens} pages={(owner) => pages()[owner]?.length ?? 1} />
          </View>
        </View>
      </Show>
      <Show when={editBar()}>
        <EditBar going={editBarGoing()} within={launcher() ? cardRect() : undefined} />
      </Show>
      <Show when={copy()}>
        {(now) => {
          const scale = () => untrack(room).scale
          const widget = () => !!now().held.item?.widget
          const [width, height] = [now().rect.width / scale(), now().rect.height / scale()]
          const lift = () => (now().settling ? 1 : 1 + (widget() ? 0.05 : 0.12) * now().lifted)
          const tile = () => laidPage().tile
          return (
            <View
              aria-hidden="true"
              style={[looks.carry, looks.carryAt(now().rect.x + (now().rect.width - width) / 2, now().rect.y + (now().rect.height - height) / 2, width, height, now().x, now().y, scale() * lift())]}
            >
              <View aria-hidden="true" style={[looks.lifted, widget() ? looks.liftedWidget : looks.liftedIcon, now().settling && looks.opacity(0), widget() ? looks.at(0, 0, width, height) : looks.at((width - tile()) / 2, 0, tile(), tile())]} />
              <Show when={now().held.item}>
                {(item) => (
                  <Show when={item().widget} fallback={<Tile name={(item() as Entrypoint).name} icon={(item() as Entrypoint).icon} size={laidPage().tile} scale={scale()} />}>
                    <Card item={item() as WidgetItem} owner={owner()} still />
                  </Show>
                )}
              </Show>
              <Show when={now().held.stack?.length}>
                <Count count={(now().held.stack?.length ?? 0) + 1} />
              </Show>
            </View>
          )
        }}
      </Show>
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[0]} opened={opened()} within={launcher() ? cardRect() : undefined} />}</Show>
      <Show when={held()}>{(held) => <Holding owner={owner()} index={state().context.at.owner} held={held()} />}</Show>
      <Show when={overview()}>
        {(edited) => <Overview owner={props.owners[edited().owner]} index={edited().owner} going={overviewGoing()} />}
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
    </View>
  )
}

/// The Home Screen didn't draw: what DESIGN.md says of a launcher that doesn't, with Try Again, which asks for it
/// again, and what went wrong under it. Its client loads it again by itself once a newer version is out, as the older
/// one meeting what's new is the likeliest cause.
function Broken(props: { error: Error }) {
  return (
    <View style={[looks.broken, looks.tall(viewport().height)]}>
      <Problem
        heading="Home Screen didn't open."
        text={new Error(`Try again in a moment.\n\n${typeof props.error?.cause === "string" ? props.error.cause : (props.error?.message ?? props.error)}`)}
        action={<Button variant="secondary" onPress={() => refresh(owners)}>Try Again</Button>}
      />
    </View>
  )
}

render(() => (
  <Errored fallback={(error) => <Broken error={error() as Error} />}>
    <Loading>
      <HomeScreen owners={owners()} />
    </Loading>
  </Errored>
))
