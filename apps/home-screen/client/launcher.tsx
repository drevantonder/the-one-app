// The computer's launcher: its keys, its search floating over its card, and what the search found.
import { createEffect, createMemo, createSignal, For, Show, untrack } from "solid-js"
import { Button, ScrollView, Empty, Icon, type Keyed, leave, reducedMotion, sizeChanged, Text, TextInput, View, viewport, visible } from "ui"
import { along as alongOf, cell as cellOf, group, INSET, PAGE, PILLS, SEARCH, SEARCH_HEIGHT, CARD_TOP } from "./layout.ts"
import { type At, type Placed } from "./machine.ts"
import { type Item, type Listed } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, at, screensOf, editingNow, launcher, query, setQuery, chosenResult, setChosenResult, type Selected, selected, setSelected, room, laidPage } from "./store.ts"
import { type Owner, badgeOf, findAll, type From, launch, opens, press, opener } from "./owners.ts"
import { Card, Tile } from "./tiles.tsx"
import { type Cell, HomePage } from "./pages.tsx"
import { Library } from "./library.tsx"
import { OwnerPill, DotsPill } from "./pills.tsx"
import { type Screens } from "./screens.ts"
import { type Paging } from "./paging.ts"
import { lingering } from "./lingering.ts"

/// How long the launcher, put away, stays where it was: Raycast's Pop to Root Search's default.
const POP_TO_ROOT = 90_000

/// Where each found app is drawn, by its place in what was found, as it's laid out: which Up and Down move between.
const slots = new Map<number, { x: number; y: number }>()

/// The found app a row above or below `from` (`by` -1 or 1): in the nearest row that way, the one nearest across; the one
/// it's on where there is none.
const rowOf = (from: number, by: -1 | 1, count: number) => {
  const here = slots.get(from)
  if (!here) return from
  let found: number | undefined
  let [down, across] = [Infinity, Infinity]
  for (let at = 0; at < count; at++) {
    const place = slots.get(at)
    if (!place || (place.y - here.y) * by < 1) continue
    const [away, aside] = [(place.y - here.y) * by, Math.abs(place.x - here.x)]
    if (away < down - 1 || (away < down + 1 && aside < across)) [found, down, across] = [at, away, aside]
  }
  return found ?? from
}

/// The launcher's keys, wherever in it the keyboard is: with nothing typed, the arrow keys move the highlight over Home
/// (`move`), Return opens what's highlighted, and Escape goes back from the App
/// Library, then puts the launcher away; with something typed, Left and Right choose among what's found, Up and Down a row above or below, Return opens
/// the one chosen, and Escape empties the search.
export const launcherKeys = (props: { move: (key: string) => boolean; open: () => void; toggleLibrary: () => void; library: () => boolean; found: () => number }) => (event: Keyed) => {
  if (event.altKey || event.metaKey || event.ctrlKey) return
  const typed = !!query().trim()
  // What's open over it (a category, a held app's menu, the owners' list) has Escape and the arrows itself.
  if (!state().matches("nothing")) return
  if (event.key === "Enter") {
    event.preventDefault()
    props.open()
  } else if (event.key === "Escape") {
    // With something typed, it empties the search, wherever the keyboard is.
    if (query()) setQuery("")
    else if (props.library()) props.toggleLibrary()
    else void leave()
  } else if (!typed) {
    if (props.move(event.key)) event.preventDefault()
  } else {
    const [along, vertical] = [({ ArrowRight: 1, ArrowLeft: -1 } as Record<string, 1 | -1>)[event.key], ({ ArrowDown: 1, ArrowUp: -1 } as Record<string, 1 | -1>)[event.key]]
    if (!along && !vertical) return
    event.preventDefault()
    setChosenResult((now) => (along ? Math.max(0, Math.min(props.found() - 1, now + along)) : rowOf(now, vertical, props.found())))
  }
}

/// The launcher's search, floating over its card: bare and large, with the keyboard as the launcher's summoned, and
/// the last search selected, so typing starts a new one, as Spotlight's does.
export function LauncherSearch() {
  return (
    <View style={looks.launcherSearch}>
      <TextInput inputMode="search" enterKeyHint="search" placeholder="Search apps…" aria-label="Search apps" autoFocus selectTextOnFocus value={query()} onChangeText={(text) => (setQuery(text), setChosenResult(0))} style={looks.bare} />
    </View>
  )
}

/// What the launcher's search found, in its card, as the Home Screen draws apps and widgets: under Apps, the widget of
/// the app found first, where it has one, and beside it every app found, chosen for Return in turn; then, under
/// Widgets, the other widgets whose name or app has what's typed. The pointer opens any of them.
export function Results(props: { owner: Owner; found: Listed[]; words: string }) {
  const tile = 60
  const lead = () => leadOf(props.owner, props.found)
  const widgets = () => widgetsOf(props.owner, props.found, props.words)
  // What's found is the apps with a screen first, then the background apps, as `findAll` has them.
  const apps = () => props.found.filter((listed) => listed.entrypoint)
  const background = () => props.found.filter((listed) => !listed.entrypoint)
  const Heading = (inner: { children: string }) => <Text style={[looks.heading, looks.tall(HEADING)]}>{inner.children}</Text>
  // Each found app, `from` being how many were found before it, where Return and the arrow keys count it, and where it
  // is, which Up and Down read.
  const Found = (inner: { list: Listed[]; from: number }) => (
    <For each={inner.list} keyed={(listed) => listed.entrypoint ?? listed.app}>
      {(listed, at) => (
        <View onPointerMove={() => setChosenResult(inner.from + at())} onLayout={(event) => slots.set(inner.from + at(), event.currentTarget.getBoundingClientRect())} style={looks.result}>
          <Show when={chosenResult() === inner.from + at()}>
            <View aria-hidden="true" style={looks.chosen} />
          </Show>
          <Tile name={listed().name} icon={listed().icon} size={tile} scale={1} badge={opens(listed()) ? badgeOf(opens(listed())!, props.owner) : undefined} onPress={(behind, from) => press(listed(), props.owner, behind, from)} />
        </View>
      )}
    </For>
  )
  return (
    <View style={looks.results}>
      <Show when={props.found.length || widgets().length} fallback={<View style={looks.nothingFound}><Empty>No apps match.</Empty></View>}>
        <Show when={apps().length}>
          <Heading>Apps</Heading>
          <View style={looks.apps}>
            <Show when={lead()}>
              {(lead) => (
                <View style={[looks.lead, looks.sized(2 * 76 + 8, 2 * 88 - 8)]}>
                  <Card item={{ ...lead(), size: "2x2" }} owner={props.owner} />
                </View>
              )}
            </Show>
            <View style={looks.foundApps}>
              <Found list={apps()} from={0} />
            </View>
          </View>
        </Show>
        <Show when={background().length}>
          <Heading>Background Apps</Heading>
          <View style={looks.foundApps}>
            <Found list={background()} from={apps().length} />
          </View>
        </Show>
        <Show when={widgets().length}>
          <Heading>Widgets</Heading>
          <View style={looks.widgets}>
            <For each={widgets()} keyed={(one) => one.widget}>
              {(one) => (
                <View style={[looks.widgetCard, looks.sized(2 * 76 + 8, 2 * 88 - 8)]}>
                  <Card item={{ ...one(), size: "2x2" }} owner={props.owner} />
                </View>
              )}
            </For>
          </View>
        </Show>
      </Show>
    </View>
  )
}

/// The widget of the app a search found first, where it has one, which leads what it found; and the other widgets whose
/// name or app has what's typed.
const leadOf = (owner: Owner | undefined, found: Listed[]) => owner?.widgets.find((one) => one.opens?.app === found[0]?.app && one.sizes.includes("2x2"))

const widgetsOf = (owner: Owner | undefined, found: Listed[], words: string) => {
  const typed = words.trim().toLowerCase()
  const lead = leadOf(owner, found)
  return (owner?.widgets ?? []).filter((one) => one !== lead && `${one.name} ${one.opens?.name ?? ""}`.toLowerCase().includes(typed))
}

const HEADING = 20
/// The chevrons at the card's sides: how big, and how far in.
const TURN = 36
const TURN_INSET = 6

/// What the launcher's Return opens: an app it found, or what's chosen on Home, with where it is on Home, if it's there.
type Target = { item?: Item; listed?: Listed; place?: { page: number; index: number } }

export function createLauncher(props: { owners: Owner[] }, layout: Screens, paging: Paging) {
  const { owned, pages, screens, library } = layout
  const { card, cardLeft, pageAt } = paging
  /// The launcher's card: what it shows, Home, the App Library or what was found, all in one card of one size, under
  /// the search, in the middle of the room.
  const showing = () => (query().trim() ? "search" : library() ? "library" : "home")
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
    () => launcher() && `${at().owner} ${at().screen} ${pages().length}`,
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
  /// What the launcher's search found: the apps of your App Library, background ones last, whose names have what's typed in them.
  const found = createMemo(() => findAll(props.owners[0]?.library ?? [], query()))
  /// What the launcher's Return opens and its Actions are for: the app its search chose, or what's highlighted on Home.
  const target = createMemo((): Target | undefined => {
    if (query().trim()) {
      const listed = found()[chosenResult()]
      return listed && { listed, item: opens(listed) }
    }
    const now = selected()
    const [where, screens] = [at(), screensOf()]
    if (!launcher() || !now || now.owner !== where.owner || screens[where.owner]?.[where.screen] !== "home") return
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
  return { showing, words, cardRect, found, openTarget, move, toggleLibrary }
}

/// The launcher's panel: its search, and its card with Home, the App Library or what was found in it.
export function LauncherPanel(props: { owners: Owner[]; layout: Screens; paging: Paging; launching: Launching }) {
  const { pages, laid } = props.layout
  const { card, cardLeft, xOf, pitch, dragged, sectionsX, opacityOf, along, wheel } = props.paging
  const { showing, words, toggleLibrary } = props.launching
  // Under the pointer, the card shows a chevron at each side there's a screenful that way, which turns to it.
  const [hovered, setHovered] = createSignal(false)
  const turns = (by: 1 | -1) => {
    const to = at().screen + by
    return hovered() && browsing() && state().matches("nothing") && to >= 0 && to < (screensOf()[at().owner]?.length ?? 1)
  }
  // The owner shown, how far along their pages and the App Library are drawn, and how opaque they are: for whoever
  // asked for less motion, they fade out, go there and fade in, as iOS's pages do (`glide`).
  const shown = () => at().owner
  const across = () => xOf(shown())[0]()
  const opacity = () => opacityOf(shown())[0]()
  // Home and the App Library are both there, side by side, but fade out while what was found shows.
  const browsing = () => showing() !== "search"
  // What's over Home but not its pages, the owner pill and the dots, goes with its last screenful to the App Library,
  // one to one as the pages are dragged, and by their spring otherwise.
  const withHome = () => [dragged() === undefined && !reducedMotion() && looks.sliding, looks.shifted(-sectionsX())]
  // Where the App Library is drawn along: the screenful after the shown owner's last.
  const libraryAt = () => props.layout.homes(shown()).length * pitch()
  // The strip slides by its spring as it goes along, but not as another owner comes to be shown in its place, whose
  // pages are lined up where they go already.
  let was = untrack(shown)
  const slides = () => {
    const now = shown()
    const same = now === was
    was = now
    return same && !reducedMotion()
  }
  return (
    <>
        <View style={[looks.surface, looks.search, looks.at((viewport().width - SEARCH) / 2, 0, SEARCH, SEARCH_HEIGHT)]}>
          <LauncherSearch />
        </View>
        <View onWheel={wheel} onPointerEnter={() => setHovered(true)} onPointerLeave={() => setHovered(false)} style={[looks.surface, looks.card, looks.at(0, CARD_TOP, card().width, card().height), looks.shifted(cardLeft())]}>
          {/* Home's pages and the App Library after them, in one strip the host drags along as a trackpad's fingers or
              the pointer go (`along`) and slides to where they're going by its spring, as an iPhone's are. */}
          <View {...along} style={[looks.cover, slides() && looks.paging, looks.shifted(-across()), looks.opacity(opacity())]}>
          {/* Home: each owner's pages, the owner shown's in place, the others rolled above and below it, faded. */}
          <View role="main" aria-label="Home" inert={showing() !== "home" || undefined} style={[looks.fading, looks.cover, showing() !== "home" && looks.inert, looks.faded(browsing() ? 1 : 0, 1)]}>
            <For each={props.owners} keyed={(owner) => owner.key}>
              {(owner, index) => {
                const shown = () => index() === at().owner
                const Page = (inner: { page: number; placed: Placed<Cell>[] }) => (
                  <View style={[looks.absolute, looks.at(INSET + alongOf(room(), inner.page, pages()[index()]?.length ?? 1), 0, PAGE, room().height)]}>
                    <View style={[looks.absolute, looks.placedPage((laidPage().height * (room().scale - 1)) / 2, (laidPage().width * (room().scale - 1)) / 2, room().scale)]}>
                      <HomePage owner={owner()} index={index()} page={inner.page} placed={inner.placed} laid={laidPage()} scale={room().scale} />
                    </View>
                  </View>
                )
                // As wide as all their screenfuls, which the strip slides along: it clips what rolls above and below.
                return (
                  <View
                    role="region"
                    aria-label={owner().name}
                    inert={!shown() || undefined}
                    style={[looks.owner, looks.rolling, !shown() && looks.inert, looks.at(0, PILLS, room().width * Math.max(1, props.layout.homes(index()).length), room().height), looks.drifted(shown() ? 1 : 0, (index() - state().context.at.owner) * 40)]}
                  >
                    {/* Each other owner's pages are lined up where the shown owner's are, ready for a roll to them. */}
                    <View style={[looks.track, looks.shifted(across() - xOf(index())[0]())]}>
                      <For each={pages()[index()] ?? []} keyed={false}>
                        {(placed, at) => <Page page={at} placed={placed()} />}
                      </For>
                    </View>
                  </View>
                )
              }}
            </For>
          </View>
          {/* The App Library, the page after Home's last, scrolling what's taller; for whoever asked for less motion,
              cross-fading in over Home. */}
          <View inert={showing() !== "library" || undefined} style={[looks.layer, looks.fading, showing() !== "library" && looks.inert, looks.fill(INSET, 0, 0), looks.shifted(libraryAt()), looks.opacity(browsing() ? 1 : 0)]}>
            <Library owner={props.owners[0]} size={{ width: card().width - 2 * INSET, height: card().height - INSET - PILLS }} />
          </View>
          </View>
          {/* Whose pages they are at the top, as a phone has it: over Home, sliding off with it to the App Library. */}
          <View inert={showing() !== "home" || undefined} style={[looks.row, looks.top(0), looks.tall(PILLS), ...withHome(), looks.opacity(!browsing() ? 0 : (editingNow() ? 0.6 : 1) * opacity())]}>
            <OwnerPill owners={props.owners} />
          </View>
          {/* What was typed found, as it's typed, and as it goes, what it last found. */}
          <View inert={showing() !== "search" || undefined} style={[looks.owner, looks.fading, showing() !== "search" && looks.inert, looks.at(INSET, INSET, card().width - 2 * INSET, card().height - 2 * INSET), looks.drifted(showing() === "search" ? 1 : 0, showing() === "search" ? 0 : -8)]}>
            <ScrollView style={looks.cover}>
              <Show when={words()}>{(typed) => <Results owner={props.owners[0]} found={findAll(props.owners[0]?.library ?? [], typed())} words={typed()} />}</Show>
            </ScrollView>
          </View>
          {/* Their dots at the foot, with the App Library's glyph, floating, as an iPhone's are: over Home, sliding off with
              it to the App Library, which has none. */}
          <View inert={showing() !== "home" || undefined} style={[looks.row, looks.bottom(0), looks.tall(PILLS), ...withHome(), looks.opacity(browsing() ? opacity() : 0)]}>
            <DotsPill owners={props.owners} laid={laid().screens} glyph={false} />
          </View>
          {/* The chevrons at its sides, under the pointer, each where there's a screenful that way. */}
          <For each={[-1, 1] as const}>
            {(by) => (
              <View inert={!turns(by) || undefined} style={[looks.turn, looks.at(by < 0 ? TURN_INSET : card().width - TURN - TURN_INSET, PILLS + (room().height - TURN) / 2, TURN, TURN), looks.opacity(turns(by) ? 1 : 0)]}>
                <View style={[looks.pill, looks.glass, looks.ownerPill]}>
                  <Button variant="ghost" size="icon" shape="capsule" aria-label={by < 0 ? "Previous Page" : "Next Page"} onPress={() => home.send({ type: "step", screen: by })}>
                    <Icon name={by < 0 ? "caret-left" : "caret-right"} />
                  </Button>
                </View>
              </View>
            )}
          </For>
          {/* To the App Library and back, in one click, in the same place: its corner of the dots' row. */}
          <View inert={!browsing() || editingNow() || undefined} style={[looks.libraryToggle, looks.bottom(0), looks.tall(PILLS), looks.opacity(browsing() ? (editingNow() ? 0.35 : 1) : 0)]}>
            <View style={[looks.pill, looks.glass, looks.ownerPill]}>
              <Button variant="ghost" shape="capsule" onPress={toggleLibrary}>
                <Show
                  when={showing() === "library"}
                  fallback={
                    <>
                      <Icon name="squares-four" along />
                      App Library
                    </>
                  }
                >
                  <Icon name="caret-left" along />
                  Home
                </Show>
              </Button>
            </View>
          </View>
        </View>
    </>
  )
}

export type Launching = ReturnType<typeof createLauncher>
