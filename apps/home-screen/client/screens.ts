import { createEffect, createMemo, untrack } from "solid-js"
import { link, safeArea, viewport } from "ui"
import { launcherRoom, measure } from "./layout.ts"
import { type Kind, pack, type Page, shown, spans } from "./machine.ts"
import { home, state, seen, editing, editingOwner, launcher, room, setRoom, kept, setKept } from "./store.ts"
import { type Owner } from "./owners.ts"
import { type Cell } from "./pages.tsx"
import { lingering } from "./lingering.ts"

export function createScreens(props: { owners: Owner[] }) {
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
  return { owned, editBar, editBarGoing, overview, overviewGoing, pages, screens, homes, laid, library, kindOf }
}

export type Screens = ReturnType<typeof createScreens>
