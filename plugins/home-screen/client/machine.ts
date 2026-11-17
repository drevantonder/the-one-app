// The Home Screen's one state machine: where it is, `{ owner, page }`, what's open on top of it, one thing at a time,
// and editing an owner's Home pages, as a phone's Home Screen is edited. home.tsx keeps its snapshot as one signal and
// draws from it, tells it what happened, and does what it says: moves the pages (`slide`), and keeps what was edited
// (`arrange`).
import { assign, emit, setup } from "xstate"
import type { Item } from "./types.ts"

/// What a page is. Every owner (you, everyone, each of your groups) has a list of pages, each of a kind, and each kind
/// says how it behaves: whether it scrolls up and down itself, which keeps a vertical swipe on it rather than moving
/// between owners; whether the page dots show over it; and whether every owner has just one, so that picking another
/// owner from it lands on theirs rather than on the page they were last on. The owner switcher shows over every kind.
export const kinds = {
  /** A fixed grid of apps and widgets, as a phone's Home Screen page. */
  home: { scrolls: false, dots: true, one: false },
  /** The App Library, an owner's last page: every app installed for them, by category. */
  library: { scrolls: true, dots: false, one: true },
} as const

export type Kind = keyof typeof kinds

/// A Home page's grid: four across and six down, as a phone's, whatever room it has.
export const COLUMNS = 4
export const ROWS = 6
const SPANS: Record<string, [number, number]> = { "2x2": [2, 2], "4x2": [4, 2], "4x4": [4, 4] }

/** Something on a Home page where it sits: its column and row, from 1, and how many of each it spans. */
export interface Placed<T> {
  item: T
  column: number
  row: number
  width: number
  height: number
}

/// Places `items` on Home pages: each where it first fits on the last page, row by row, a widget from an even column,
/// and what doesn't fit starting the next page, as a phone fills its Home Screen. There's always one page.
export function pack<T>(items: T[], spans: (item: T) => [number, number] = () => [1, 1]): Placed<T>[][] {
  const pages: Placed<T>[][] = [[]]
  let taken: boolean[][] = []
  const free = (width: number, height: number) => {
    for (let row = 0; row + height <= ROWS; row++)
      for (let column = 0; column + width <= COLUMNS; column += width > 1 ? 2 : 1)
        if (Array.from({ length: height }, (_, r) => Array.from({ length: width }, (_, c) => taken[row + r]?.[column + c])).flat().every((cell) => !cell)) return { row, column }
  }
  for (const item of items) {
    const [width, height] = spans(item)
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

/// The cells an item of a page takes: an app one, a widget its size.
export const spans = (item: Item): [number, number] => (item.widget ? (SPANS[item.size!] ?? [2, 2]) : [1, 1])

/** A Home page as it's edited: what's on it, and whether it's hidden. */
export interface Page {
  items: Item[]
  hidden?: boolean
}

/// Pages as they fit: what doesn't fit on one goes to the start of the next, a new one past the last if it must.
export function fitted(pages: Page[]): Page[] {
  const out = pages.map((page) => ({ ...page, items: [...page.items] }))
  for (let index = 0; index < out.length; index++) {
    const [kept, ...spilled] = pack(out[index].items, spans)
    if (!spilled.length) continue
    const over = spilled.flat().map(({ item }) => item)
    out[index].items = kept.map(({ item }) => item)
    if (index + 1 === out.length || out[index + 1].hidden) out.splice(index + 1, 0, { items: [] })
    out[index + 1].items.unshift(...over)
  }
  return out
}

/// Which of `pages` are shown, by where each is among them: a hidden one isn't.
export const shown = (pages: Page[]) => pages.flatMap((page, index) => (page.hidden ? [] : [index]))

const same = (a: Item, b: Item) => (a.widget ?? a.app) === (b.widget ?? b.app) && a.size === b.size

/** Where the Home Screen is: an owner, and a page of their list. */
export interface At {
  owner: number
  page: number
}

/** A category of an owner's App Library, opened: whose, and its name there. */
export interface Opened {
  owner: number
  name: string
}

/** What a finger holds: an item of a Home page, by the page shown and where it is on it, or empty space there, or an
 * app of the App Library on its way to a page. `rect` is where it was. */
export interface Held {
  item?: Item
  page: number
  index: number
  from: "page" | "library"
  rect?: { x: number; y: number; width: number; height: number }
}

/** An owner's Home pages as they're edited, the hidden ones among them. */
export interface Editing {
  owner: number
  pages: Page[]
}

interface Context {
  /** Each owner's pages, by kind, in order. */
  pages: Kind[][]
  at: At
  /** The page each owner was last on, which moving to them lands on. */
  last: number[]
  category?: Opened
  held?: Held
  editing?: Editing
}

export type Event =
  /** The owners and their pages, each by its kind, as they're laid out now. */
  | { type: "laid"; pages: Kind[][] }
  /** The Home Screen came to rest here, as a finger or a scroll left it. */
  | { type: "settled"; at: At }
  /** One page along, or one owner: a key, a wheel or a trackpad. */
  | { type: "step"; page: 1 | -1 }
  | { type: "step"; owner: 1 | -1 }
  /** A page of this owner's, by its dot. */
  | { type: "go"; page: number }
  /** The owner switcher's list opened, an owner was picked from it, or it was dismissed. */
  | { type: "list" }
  | { type: "pick"; owner: number }
  | { type: "dismiss" }
  /** A category was tapped in an App Library, or closed. */
  | { type: "open"; category: Opened }
  | { type: "close" }
  /** The window shows an app over the Home Screen, or the Home Screen again. */
  | { type: "launch" }
  | { type: "home" }
  /** A finger stayed on an item or on empty space, or a mouse right-clicked it, for its menu, by whoever arranges it. */
  | { type: "hold"; held: Held }
  /** Edit Home Screen, from that menu, or a hold on a page's empty space: the owner's pages, as they are, to edit. */
  | { type: "edit"; editing: Editing }
  /** An item, or an app from the App Library with the owner's pages to edit, lifted to be moved. */
  | { type: "lift"; held: Held; editing?: Editing }
  /** What's lifted is over a place: on the page shown, before the item at `index`, or last. */
  | { type: "over"; page: number; index: number }
  /** What's lifted was held at the edge of the page: it goes to the next or the previous, a new one past the last. */
  | { type: "flip"; by: 1 | -1 }
  | { type: "drop" }
  /** An item's ⊖: off the page, still in the App Library. */
  | { type: "remove"; page: number; index: number }
  /** + opens the widget gallery; a widget picked in a size goes first on the page shown. */
  | { type: "gallery" }
  | { type: "place"; item: Item }
  /** The page dots open the page overview, where pages are put in order, hidden and deleted. */
  | { type: "overview" }
  | { type: "hide"; page: number; hidden: boolean }
  | { type: "delete"; page: number }
  | { type: "reorder"; from: number; to: number }
  /** Done, or a tap on empty space: what was edited is kept. */
  | { type: "done" }

/** What the machine asks of the page: to be at `at`, gliding there, or at once where it's only laid out again; and to
 * keep an owner's pages as they were edited. */
export type Emitted = { type: "slide"; at: At; smooth: boolean } | { type: "arrange"; editing: Editing }

const within = (number: number, length: number) => Math.max(0, Math.min(length - 1, number))

/** Where a step leads from `at`: the next or previous page, or the page the next or previous owner was last on. */
function stepped({ pages, at, last }: Context, event: Extract<Event, { type: "step" }>): At {
  if ("page" in event) return { ...at, page: within(at.page + event.page, pages[at.owner]?.length ?? 1) }
  const owner = within(at.owner + event.owner, pages.length)
  return { owner, page: last[owner] ?? 0 }
}

/// The pages being edited with what's held put at `index` of the page shown at `page`, from wherever it was.
function put(editing: Editing, held: Held, page: number, index: number): { editing: Editing; held: Held } {
  const item = held.item!
  const pages = editing.pages.map((one) => ({ ...one, items: one.items.filter((other) => !same(other, item)) }))
  const at = shown(pages)[page] ?? pages.length
  if (at === pages.length) pages.push({ items: [] })
  const items = pages[at].items
  items.splice(Math.max(0, Math.min(index, items.length)), 0, item)
  const fit = fitted(pages)
  const now = fit.flatMap((one, where) => (one.items.some((other) => same(other, item)) ? [where] : []))[0] ?? at
  return { editing: { ...editing, pages: fit }, held: { ...held, page: shown(fit).indexOf(now), index: fit[now].items.findIndex((other) => same(other, item)), from: "page" } }
}

export const machine = setup({
  types: {} as { context: Context; events: Event; emitted: Emitted },
  // A category grows out of its place as an iPhone's folder does, or is there at once for whoever asked for less motion.
  delays: { zoom: () => (matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : 320) },
  guards: {
    // A step goes somewhere, and a vertical one moves between owners only from a page that doesn't scroll itself.
    moves: ({ context, event }) => {
      const step = event as Extract<Event, { type: "step" }>
      const to = stepped(context, step)
      const here = context.pages[context.at.owner]?.[context.at.page] ?? "home"
      return (to.owner !== context.at.owner || to.page !== context.at.page) && ("page" in step || !kinds[here].scrolls)
    },
    // A page is deleted only once it's hidden, as a phone's is.
    hidden: ({ context, event }) => !!context.editing?.pages[(event as Extract<Event, { type: "delete" }>).page]?.hidden,
  },
  actions: {
    // Every owner keeps the page they were on, as far as they still have it.
    lay: assign(({ context, event }) => {
      const { pages } = event as Extract<Event, { type: "laid" }>
      const last = pages.map((kinds, owner) => within(context.last[owner] ?? 0, kinds.length))
      const owner = within(context.at.owner, pages.length)
      return { pages, last, at: { owner, page: last[owner] ?? 0 } }
    }),
    // Where it is now, which its owner's pages remember.
    be: assign(({ context, event }) => {
      const { at } = event as Extract<Event, { type: "settled" }>
      return { at, last: context.last.with(at.owner, at.page) }
    }),
    step: assign(({ context, event }) => {
      const at = stepped(context, event as Extract<Event, { type: "step" }>)
      return { at, last: context.last.with(at.owner, at.page) }
    }),
    go: assign(({ context: { at, last, pages }, event }) => {
      const page = within((event as Extract<Event, { type: "go" }>).page, pages[at.owner]?.length ?? 1)
      return { at: { ...at, page }, last: last.with(at.owner, page) }
    }),
    pick: assign(({ context: { at, last, pages }, event }) => {
      const owner = within((event as Extract<Event, { type: "pick" }>).owner, pages.length)
      const here = pages[at.owner]?.[at.page] ?? "home"
      const theirs = kinds[here].one ? pages[owner]?.indexOf(here) ?? -1 : -1
      return { at: { owner, page: theirs >= 0 ? theirs : (last[owner] ?? 0) } }
    }),
    glide: emit(({ context: { at } }) => ({ type: "slide" as const, at, smooth: true })),
    jump: emit(({ context: { at } }) => ({ type: "slide" as const, at, smooth: false })),
    open: assign({ category: ({ event }) => (event as Extract<Event, { type: "open" }>).category }),
    forget: assign({ category: undefined }),
    hold: assign({ held: ({ event }) => (event as Extract<Event, { type: "hold" }>).held }),
    let: assign({ held: undefined }),
    edit: assign({ editing: ({ event }) => (event as Extract<Event, { type: "edit" }>).editing }),
    // Lifted from the App Library, it's on the owner's last Home page to start with, which is shown.
    lift: assign(({ context, event }) => {
      const { held, editing = context.editing! } = event as Extract<Event, { type: "lift" }>
      if (held.from === "page") return { held, editing }
      const page = Math.max(0, shown(editing.pages).length - 1)
      const moved = put(editing, held, page, Infinity)
      return { ...moved, at: { owner: editing.owner, page: moved.held.page }, last: context.last.with(editing.owner, moved.held.page) }
    }),
    over: assign(({ context, event }) => {
      const { page, index } = event as Extract<Event, { type: "over" }>
      return put(context.editing!, context.held!, page, index)
    }),
    flip: assign(({ context, event }) => {
      const { by } = event as Extract<Event, { type: "flip" }>
      const page = Math.max(0, context.held!.page + by)
      const moved = put(context.editing!, context.held!, page, Infinity)
      return { ...moved, at: { ...context.at, page: moved.held.page }, last: context.last.with(context.at.owner, moved.held.page) }
    }),
    remove: assign(({ context, event }) => {
      const { page, index } = event as Extract<Event, { type: "remove" }>
      const pages = context.editing!.pages.map((one, at) => (at === shown(context.editing!.pages)[page] ? { ...one, items: one.items.toSpliced(index, 1) } : one))
      return { editing: { ...context.editing!, pages } }
    }),
    place: assign(({ context, event }) => {
      const { item } = event as Extract<Event, { type: "place" }>
      const editing = context.editing!
      const at = shown(editing.pages)[context.at.page] ?? editing.pages.length
      const pages = editing.pages.map((one) => ({ ...one }))
      if (at === pages.length) pages.push({ items: [] })
      pages[at] = { ...pages[at], items: [item, ...pages[at].items] }
      return { editing: { ...editing, pages: fitted(pages) } }
    }),
    hide: assign(({ context, event }) => {
      const { page, hidden } = event as Extract<Event, { type: "hide" }>
      const editing = context.editing!
      // One page stays shown.
      if (hidden && shown(editing.pages).length <= 1 && !editing.pages[page].hidden) return {}
      return { editing: { ...editing, pages: editing.pages.with(page, { ...editing.pages[page], hidden }) } }
    }),
    delete: assign(({ context, event }) => {
      const { page } = event as Extract<Event, { type: "delete" }>
      return { editing: { ...context.editing!, pages: context.editing!.pages.toSpliced(page, 1) } }
    }),
    reorder: assign(({ context, event }) => {
      const { from, to } = event as Extract<Event, { type: "reorder" }>
      const pages = context.editing!.pages.toSpliced(from, 1)
      pages.splice(to, 0, context.editing!.pages[from])
      return { editing: { ...context.editing!, pages } }
    }),
    // What was edited is kept, without the pages left empty, as a phone's go.
    keep: emit(({ context }) => ({ type: "arrange" as const, editing: { ...context.editing!, pages: context.editing!.pages.filter((page) => page.items.length) } })),
    stop: assign({ editing: undefined, held: undefined }),
  },
}).createMachine({
  id: "home",
  context: { pages: [], at: { owner: 0, page: 0 }, last: [] },
  initial: "nothing",
  on: {
    // Where it is follows the pages, whatever's open.
    laid: { actions: ["lay", "jump"] },
    settled: { actions: "be" },
    // Picking an owner ends the list, in whichever order its menu tells of the two.
    pick: { target: ".nothing", actions: ["pick", "glide"] },
    // An app shown over the Home Screen, as by a notification, closes what was open, and keeps what was edited.
    launch: ".app",
  },
  states: {
    nothing: {
      on: {
        step: { guard: "moves", actions: ["step", "glide"] },
        go: { actions: ["go", "glide"] },
        list: "listing",
        open: { target: "category", actions: "open" },
        hold: { target: "holding", actions: "hold" },
        // A finger held on a page's empty space edits it at once, as a phone's does.
        edit: { target: "editing", actions: "edit" },
      },
    },
    // The owner switcher's list of every owner.
    listing: {
      on: { dismiss: "nothing" },
    },
    // A category, grown out of its place in the App Library: taps are let be until it's open, and it shrinks back
    // into its place as it closes.
    category: {
      initial: "opening",
      exit: "forget",
      states: {
        opening: { after: { zoom: "open" } },
        open: { on: { close: "closing" } },
        closing: { after: { zoom: "#home.nothing" } },
      },
    },
    // What a finger holds, lifted over the blurred Home Screen, with its menu: Edit Home Screen, and taking it off.
    holding: {
      exit: "let",
      on: {
        dismiss: "nothing",
        edit: { target: "editing", actions: "edit" },
        lift: { target: "editing.dragging", actions: ["lift", "glide"] },
      },
    },
    // An owner's Home pages, edited: everything jiggles, each item with its ⊖, + at the top for a widget and Done.
    editing: {
      initial: "still",
      exit: ["keep", "stop"],
      on: { done: "nothing" },
      states: {
        still: {
          on: {
            lift: { target: "dragging", actions: "lift" },
            remove: { actions: "remove" },
            gallery: "gallery",
            overview: "overview",
          },
        },
        // What's lifted follows the finger, and the rest make room where it's over.
        dragging: {
          exit: "let",
          on: {
            over: { actions: "over" },
            flip: { actions: ["flip", "glide"] },
            drop: "still",
          },
        },
        // The widget gallery: a widget, in a size, to place.
        gallery: {
          on: {
            place: { target: "still", actions: "place" },
            dismiss: "still",
          },
        },
        // Every Home page small, to put in order, hide or show, and delete once hidden.
        overview: {
          on: {
            hide: { actions: "hide" },
            delete: { guard: "hidden", actions: "delete" },
            reorder: { actions: "reorder" },
            dismiss: "still",
          },
        },
      },
    },
    // An app the window shows over the Home Screen, until its person goes back.
    app: { on: { home: "nothing" } },
  },
})
