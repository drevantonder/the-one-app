// The Home Screen's one state machine: where it is, what's open on top of it, one thing at a time, and editing owners'
// Home pages, as a phone's Home Screen is edited. home.tsx keeps its snapshot as one signal and draws from it, tells it
// what happened, and does what it says: moves the pages (`slide`), and keeps what was edited (`arrange`).
//
//   where it is: home { owner, screen } | library | store
//     step(screen): Home's screenfuls, then the App Library, then the App Store
//     step(owner): on Home only.   list ─pick─► another owner: on Home only
//   open ─► category (App Library), as now
//   holding (an app in the App Library) ─lift─► editing.dragging, with Home brought forward on the last owner
//   editing.dragging: over · flip(page ±1) · flip(owner ±1) · drop ─► still | asking | refused
//   asking ─install─► installing ─installed─► still (placed) · ─failed─► still (back, saying why) · ─cancel─► still (back)
//   refused ─dismiss─► still (back)
//   asking · refused ─link─► still (back), then the link · installing: a link waits, unfollowed, for the install
//
// An app the window shows over it leaves it as it is, a phone's Home Screen under an app: what's open is open, editing
// goes on, and an install finishes where it was dropped, or goes back, as it would have in front.
//
// Only Home is an owner's: you, everyone and each of your groups have Home pages, one owner above another. The App
// Library and the App Store come after Home's screenfuls, and are one each for everyone you can reach.
import { assign, emit, raise, setup } from "xstate"
import type { Item, Listed } from "./types.ts"

/// The sections, in order: Home, fixed grids of apps and widgets, as a phone's Home Screen pages, a screenful or more,
/// as many of its pages side by side as fit (one on a phone); then the App Library and the App Store, a screenful
/// each, which scroll up and down themselves.
export type Kind = "home" | "library" | "store"


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

const same = (a: Item, b: Item) => (a.widget ?? a.entrypoint) === (b.widget ?? b.entrypoint) && a.size === b.size

/** An owner's pages as they're edited, the hidden ones among them. */
export interface Editing {
  owner: number
  pages: Page[]
}

/** Where the Home Screen is: on Home, an owner and a screenful of their pages; past them, the App Library or the App
 * Store, the same for every owner, with `owner` the one Home goes back to. `screen` counts on from Home's screenfuls
 * through the App Library and the App Store, so a step is one along. */
export interface At {
  owner: number
  screen: number
}

/** What a finger holds: an item of a Home page, by the page shown and where it is on it, or empty space there, or an
 * app of the App Library (`listed`), which may be on its way to a page as an `item`. `rect` is where it was. */
export interface Held {
  item?: Item
  listed?: Listed
  page: number
  index: number
  from: "page" | "library"
  rect?: { x: number; y: number; width: number; height: number }
}

/** Where what's carried was dropped: on an owner it's installed for (`place`), on one it isn't but who you may
 * install for (`ask`), or on one whose pages you may not change (`refuse`). */
export type Drop = "place" | "ask" | "refuse"

interface Context {
  /** Each owner's screenfuls, each by its section, in order: Home's, then the App Library and the App Store. */
  screens: Kind[][]
  /** How many Home pages a screenful has side by side. */
  fit: number
  at: At
  /** The screenful each owner was last on, which moving to them lands on. */
  last: number[]
  /** The App Library's category that's open, by its name. */
  category?: string
  held?: Held
  /** The pages of the owner being edited, the one shown. */
  editing?: Editing
  /** Every other owner's pages edited since editing began, as they're kept on Done. */
  drafts: Editing[]
  /** Each owner's pages as they were when what's carried was lifted, which it goes back to. */
  before: Editing[]
}

export type Event =
  /** The owners and their screenfuls, each by its section, as they're laid out now, and how many Home pages each
   * screenful has. */
  | { type: "laid"; screens: Kind[][]; fit: number }
  /** The Home Screen came to rest here, as a finger or a scroll left it. */
  | { type: "settled"; at: At }
  /** One screenful along, or one owner: a key, a wheel or a trackpad. */
  | { type: "step"; screen: 1 | -1 }
  | { type: "step"; owner: 1 | -1 }
  /** A screenful of this owner's, by its section's icon or dot. */
  | { type: "go"; screen: number }
  /** A link to one of the screenfuls (`one:home-screen#library`): there at once, whatever was open. */
  | { type: "link"; screen: number }
  /** The owner switcher's list opened, an owner was picked from it, or it was dismissed. */
  | { type: "list" }
  | { type: "pick"; owner: number }
  | { type: "dismiss" }
  /** A category was tapped in the App Library, or closed. */
  | { type: "open"; category: string }
  | { type: "close" }
  /** An opened category has grown out of its place, or shrunk back into it: its animations have finished. */
  | { type: "zoomed" }
  /** A finger stayed on an item or on empty space, or a mouse right-clicked it, for its menu, by whoever arranges it. */
  | { type: "hold"; held: Held }
  /** Edit Home Screen, from that menu, or a hold on a page's empty space: the owner's pages, as they are, to edit. */
  | { type: "edit"; editing: Editing }
  /** An item, or an app from the App Library with the owner's pages to edit, lifted to be moved. */
  | { type: "lift"; held: Held; editing?: Editing }
  /** What's lifted is over a place: on the page shown, before the item at `index`, or last. */
  | { type: "over"; page: number; index: number }
  /** What's lifted was held at the side of the screen: it goes to the page past it, on the next screenful or the
   * previous, a new one past the last. Or at its top or bottom: it goes to the previous or the next owner's pages,
   * `editing` as they're kept. */
  | { type: "flip"; by: 1 | -1 }
  | { type: "flip"; owner: 1 | -1; editing: Editing }
  /** It was let go, and where it landed says what follows. */
  | { type: "drop"; to: Drop }
  /** The question an app dropped where it isn't installed asks: Install, and how that went, or Cancel. */
  | { type: "install" }
  | { type: "installed" }
  | { type: "failed" }
  | { type: "cancel" }
  /** An item's ⊖: off the page, still in the App Library. */
  | { type: "remove"; page: number; index: number }
  /** + opens the widget gallery; a widget picked in a size goes first on the screenful shown. */
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
 * keep owners' pages as they were edited, each that changed. */
export type Emitted = { type: "slide"; at: At; smooth: boolean } | { type: "arrange"; edited: Editing[] }

const within = (number: number, length: number) => Math.max(0, Math.min(length - 1, number))
/// How many of an owner's screenfuls are Home's.
const homes = (screens: Kind[]) => Math.max(1, screens.filter((kind) => kind === "home").length)

/** Where a step leads from `at`: the next or previous screenful, or the one the next or previous owner was last on. */
function stepped({ screens, at, last }: Context, event: Extract<Event, { type: "step" }>): At {
  if ("screen" in event) return { ...at, screen: within(at.screen + event.screen, screens[at.owner]?.length ?? 1) }
  const owner = within(at.owner + event.owner, screens.length)
  return { owner, screen: last[owner] ?? 0 }
}

/// Whether `at` is on Home.
const onHome = ({ screens, at }: Context) => (screens[at.owner]?.[at.screen] ?? "home") === "home"

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

/// Every owner's pages being edited: the one shown, and the rest.
const everyone = ({ editing, drafts }: Context) => (editing ? [editing, ...drafts] : drafts)

export const machine = setup({
  types: {} as { context: Context; events: Event; emitted: Emitted },
  guards: {
    // A step goes somewhere, and a vertical one only on Home: the App Library and the App Store scroll themselves.
    moves: ({ context, event }) => {
      const step = event as Extract<Event, { type: "step" }>
      const to = stepped(context, step)
      return (to.owner !== context.at.owner || to.screen !== context.at.screen) && ("screen" in step || onHome(context))
    },
    home: ({ context }) => onHome(context),
    // A page is deleted only once it's hidden, as a phone's is.
    hidden: ({ context, event }) => !!context.editing?.pages[(event as Extract<Event, { type: "delete" }>).page]?.hidden,
    // An app goes to another owner's pages, not a widget, which is its app's for the owner it's on.
    another: ({ context, event }) => {
      const { owner } = event as Extract<Event, { type: "flip"; owner: 1 | -1 }>
      return !context.held?.item?.widget && within(context.at.owner + owner, context.screens.length) !== context.at.owner
    },
    ask: ({ event }) => (event as Extract<Event, { type: "drop" }>).to === "ask",
    refuse: ({ event }) => (event as Extract<Event, { type: "drop" }>).to === "refuse",
  },
  actions: {
    // Every owner keeps the screenful they were on, as far as they still have it: in the App Library or the App Store,
    // still there, however many of Home's come before it.
    lay: assign(({ context, event }) => {
      const { screens, fit } = event as Extract<Event, { type: "laid" }>
      const last = screens.map((kinds, owner) => {
        const was = context.screens[owner]?.[context.last[owner] ?? 0] ?? "home"
        return was === "home" ? within(context.last[owner] ?? 0, homes(kinds)) : Math.max(0, kinds.indexOf(was))
      })
      const owner = within(context.at.owner, screens.length)
      return { screens, fit, last, at: { owner, screen: last[owner] ?? 0 } }
    }),
    // Where it is now, which its owner's pages remember.
    be: assign(({ context, event }) => {
      const { at } = event as Extract<Event, { type: "settled" }>
      return { at, last: context.last.with(at.owner, at.screen) }
    }),
    step: assign(({ context, event }) => {
      const at = stepped(context, event as Extract<Event, { type: "step" }>)
      return { at, last: context.last.with(at.owner, at.screen) }
    }),
    go: assign(({ context: { at, last, screens }, event }) => {
      const screen = within((event as Extract<Event, { type: "go" | "link" }>).screen, screens[at.owner]?.length ?? 1)
      return { at: { ...at, screen }, last: last.with(at.owner, screen) }
    }),
    // Another owner's Home, where they were last.
    pick: assign(({ context: { last, screens }, event }) => {
      const owner = within((event as Extract<Event, { type: "pick" }>).owner, screens.length)
      return { at: { owner, screen: last[owner] ?? 0 } }
    }),
    glide: emit(({ context: { at } }) => ({ type: "slide" as const, at, smooth: true })),
    jump: emit(({ context: { at } }) => ({ type: "slide" as const, at, smooth: false })),
    open: assign({ category: ({ event }) => (event as Extract<Event, { type: "open" }>).category }),
    forget: assign({ category: undefined }),
    hold: assign({ held: ({ event }) => (event as Extract<Event, { type: "hold" }>).held }),
    let: assign({ held: undefined }),
    edit: assign({ editing: ({ event }) => (event as Extract<Event, { type: "edit" }>).editing }),
    // Lifted from the App Library, it's on the last owner's last Home page to start with, whose screenful is shown.
    lift: assign(({ context, event }) => {
      const { held, editing = context.editing! } = event as Extract<Event, { type: "lift" }>
      const before = [editing, ...context.drafts]
      if (held.from === "page") return { held, editing, before }
      const page = Math.max(0, shown(editing.pages).length - 1)
      const moved = put(editing, held, page, Infinity)
      const screen = Math.floor(moved.held.page / context.fit)
      return { ...moved, before, at: { owner: editing.owner, screen }, last: context.last.with(editing.owner, screen) }
    }),
    over: assign(({ context, event }) => {
      const { page, index } = event as Extract<Event, { type: "over" }>
      return put(context.editing!, context.held!, page, index)
    }),
    // To the page just past the screen's edge it was held at: the next screenful's first, or the previous one's last.
    flip: assign(({ context, event }) => {
      const { by } = event as Extract<Event, { type: "flip"; by: 1 | -1 }>
      const { fit, at } = context
      const page = Math.max(0, by > 0 ? (at.screen + 1) * fit : at.screen * fit - 1)
      const moved = put(context.editing!, context.held!, page, Infinity)
      const screen = Math.floor(moved.held.page / fit)
      return { ...moved, at: { ...at, screen }, last: context.last.with(at.owner, screen) }
    }),
    // To the previous or the next owner's pages, as they're kept or already edited, on the first page of the screenful
    // they were last on: it leaves the pages it was on.
    turn: assign(({ context, event }) => {
      const { owner: by, editing: kept } = event as Extract<Event, { type: "flip"; owner: 1 | -1 }>
      const { at, fit, held, editing, screens, last } = context
      const owner = within(at.owner + by, screens.length)
      const left = { ...editing!, pages: editing!.pages.map((page) => ({ ...page, items: page.items.filter((item) => !same(item, held!.item!)) })) }
      const theirs = context.drafts.find((one) => one.owner === owner) ?? kept
      const drafts = [...context.drafts.filter((one) => one.owner !== owner), left]
      const before = context.before.some((one) => one.owner === owner) ? context.before : [...context.before, kept]
      const moved = put(theirs, held!, within(last[owner] ?? 0, homes(screens[owner] ?? [])) * fit, Infinity)
      const screen = Math.floor(moved.held.page / fit)
      return { ...moved, drafts, before, at: { owner, screen }, last: last.with(owner, screen) }
    }),
    // Every owner's pages as they were when it was lifted: it's back where it came from, and off the pages it was put
    // on, in the App Library, where it came from there.
    back: assign(({ context: { before, at } }) => {
      const editing = before.find((one) => one.owner === at.owner) ?? before[0]
      return { editing, drafts: before.filter((one) => one !== editing) }
    }),
    remove: assign(({ context, event }) => {
      const { page, index } = event as Extract<Event, { type: "remove" }>
      const pages = context.editing!.pages.map((one, at) => (at === shown(context.editing!.pages)[page] ? { ...one, items: one.items.toSpliced(index, 1) } : one))
      return { editing: { ...context.editing!, pages } }
    }),
    place: assign(({ context, event }) => {
      const { item } = event as Extract<Event, { type: "place" }>
      const editing = context.editing!
      const at = shown(editing.pages)[context.at.screen * context.fit] ?? editing.pages.length
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
    // What was edited is kept, every owner's, without the pages left empty, as a phone's go.
    keep: emit(({ context }) => ({ type: "arrange" as const, edited: everyone(context).map((one) => ({ ...one, pages: one.pages.filter((page) => page.items.length) })) })),
    stop: assign({ editing: undefined, held: undefined, drafts: [], before: [] }),
  },
}).createMachine({
  id: "home",
  context: { screens: [], fit: 1, at: { owner: 0, screen: 0 }, last: [], drafts: [], before: [] },
  initial: "nothing",
  on: {
    // Where it is follows the pages, whatever's open.
    laid: { actions: ["lay", "jump"] },
    settled: { actions: "be" },
    link: { target: ".nothing", actions: ["go", "jump"] },
  },
  states: {
    nothing: {
      on: {
        step: { guard: "moves", actions: ["step", "glide"] },
        go: { actions: ["go", "glide"] },
        list: { guard: "home", target: "listing" },
        open: { target: "category", actions: "open" },
        hold: { target: "holding", actions: "hold" },
        // A finger held on a page's empty space edits it at once, as a phone's does.
        edit: { target: "editing", actions: "edit" },
      },
    },
    // The owner switcher's list of every owner, on Home. Picking one ends it, in whichever order its menu tells of the
    // two.
    listing: {
      on: {
        dismiss: "nothing",
        pick: { target: "nothing", actions: ["pick", "glide"] },
      },
    },
    // A category, grown out of its place in the App Library as an iPhone's folder does: its apps are let be until
    // it's open, and it shrinks back into its place as it closes. Either can be changed midway, as a finger changes its
    // mind: closed as it grows, or tapped again as it shrinks, it turns back from where it got to. It's open, or gone,
    // once its animations say they're done (`zoomed`), whichever way they ended up going.
    category: {
      initial: "opening",
      exit: "forget",
      states: {
        opening: { on: { zoomed: "open", close: "closing" } },
        open: { on: { close: "closing" } },
        closing: { on: { zoomed: "#home.nothing", open: "opening" } },
      },
    },
    // What a finger holds, lifted over the blurred Home Screen, with its menu: Edit Home Screen, and taking it off.
    // Carried from the App Library, it brings Home forward, on the owner it was last on.
    holding: {
      exit: "let",
      on: {
        dismiss: "nothing",
        edit: { target: "editing", actions: "edit" },
        lift: { target: "editing.dragging", actions: ["lift", "glide"] },
      },
    },
    // Owners' Home pages, edited: everything jiggles, each item with its ⊖, + at the top for a widget and Done.
    editing: {
      initial: "still",
      exit: ["keep", "stop"],
      on: { done: "nothing" },
      states: {
        still: {
          entry: "let",
          on: {
            lift: { target: "dragging", actions: "lift" },
            remove: { actions: "remove" },
            gallery: "gallery",
            overview: "overview",
          },
        },
        // What's lifted follows the finger, and the rest make room where it's over. Held at a side it turns the page,
        // and at the top or the bottom, the owner.
        dragging: {
          on: {
            over: { actions: "over" },
            flip: [
              { guard: ({ event }) => "by" in event, actions: ["flip", "glide"] },
              { guard: "another", actions: ["turn", "glide"] },
            ],
            drop: [{ guard: "ask", target: "asking" }, { guard: "refuse", target: "refused" }, { target: "still" }],
          },
        },
        // Dropped on an owner it isn't installed for: Install it for them? It waits where it was dropped.
        asking: {
          on: {
            install: "installing",
            cancel: { target: "still", actions: "back" },
            done: {},
            // A link leaves editing, and what was edited is kept, so what was dropped goes back first.
            link: { target: "still", actions: ["back", raise(({ event }) => event)] },
          },
        },
        // It's installing for them, where it was dropped, until that's done, or it's back in the App Library.
        installing: {
          on: {
            installed: "still",
            failed: { target: "still", actions: "back" },
            done: {},
            // Followed now, it would keep what's installing on the page before it's installed, or after it failed.
            link: {},
          },
        },
        // Dropped on an owner whose pages are someone else's to change: it goes back once that's been said.
        refused: {
          on: {
            dismiss: { target: "still", actions: "back" },
            done: {},
            link: { target: "still", actions: ["back", raise(({ event }) => event)] },
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
  },
})
