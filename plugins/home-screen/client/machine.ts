// The Home Screen's one state machine: where it is, `{ owner, page }`, and what's open on top of it, one thing at a
// time. home.tsx keeps its snapshot as one signal and draws from it, tells it what happened, and moves the pages where
// it says (`slide`).
import { assign, emit, setup } from "xstate"

/// What a page is. Every owner (you, everyone, each of your groups) has a list of pages, each of a kind, and each kind
/// says how it behaves: whether it scrolls up and down itself, which keeps a vertical swipe on it rather than moving
/// between owners, and whether the dots and the owner switcher show over it.
export const kinds = {
  /** A fixed grid of apps and widgets, as a phone's Home Screen page. */
  home: { scrolls: false, chrome: true },
  /** The App Library, an owner's last page: every app installed for them, by category. */
  library: { scrolls: true, chrome: false },
} as const

export type Kind = keyof typeof kinds

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

interface Context {
  /** Each owner's pages, by kind, in order. */
  pages: Kind[][]
  at: At
  /** The page each owner was last on, which moving to them lands on. */
  last: number[]
  category?: Opened
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

/** What the machine asks of the pages: to be at `at`, gliding there, or at once where they're only laid out again. */
export interface Slide {
  type: "slide"
  at: At
  smooth: boolean
}

const within = (number: number, length: number) => Math.max(0, Math.min(length - 1, number))

/** Where a step leads from `at`: the next or previous page, or the page the next or previous owner was last on. */
function stepped({ pages, at, last }: Context, event: Extract<Event, { type: "step" }>): At {
  if ("page" in event) return { ...at, page: within(at.page + event.page, pages[at.owner]?.length ?? 1) }
  const owner = within(at.owner + event.owner, pages.length)
  return { owner, page: last[owner] ?? 0 }
}

export const machine = setup({
  types: {} as { context: Context; events: Event; emitted: Slide },
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
    pick: assign(({ context: { last, pages }, event }) => {
      const owner = within((event as Extract<Event, { type: "pick" }>).owner, pages.length)
      return { at: { owner, page: last[owner] ?? 0 } }
    }),
    glide: emit(({ context: { at } }) => ({ type: "slide" as const, at, smooth: true })),
    jump: emit(({ context: { at } }) => ({ type: "slide" as const, at, smooth: false })),
    open: assign({ category: ({ event }) => (event as Extract<Event, { type: "open" }>).category }),
    forget: assign({ category: undefined }),
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
    // An app shown over the Home Screen, as by a notification, closes what was open.
    launch: ".app",
  },
  states: {
    nothing: {
      on: {
        step: { guard: "moves", actions: ["step", "glide"] },
        go: { actions: ["go", "glide"] },
        list: "listing",
        open: { target: "category", actions: "open" },
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
    // An app the window shows over the Home Screen, until its person goes back.
    app: { on: { home: "nothing" } },
  },
})
