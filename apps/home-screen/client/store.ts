// State every part of the Home Screen reads: the one machine and its snapshot, where the pages are as they move,
// which kind of device this is, and what a finger is doing.
import { createMemo, createSignal } from "solid-js"
import { createActor } from "xstate"
import { client, type Pressed, viewport } from "ui"
import { measure, page as pageOf, type Room } from "./layout.ts"
import { type At, type Editing, machine } from "./machine.ts"

/// The one machine, and its snapshot as one signal.
export const home = createActor(machine)

export const [state, setState] = createSignal(home.getSnapshot())
home.subscribe(setState)
home.start()

/// The owner and screenful nearest where the pages are as they move, which the dots follow during a swipe, as a
/// phone's do, rather than once it ends.
export const [seen, setSeen] = createSignal<At>({ owner: 0, screen: 0 }, { equals: (a, b) => a.owner === b.owner && a.screen === b.screen })

/// Where the owner shown's pages are as they move, in screenfuls, which the dots light along continuously.
export const [along, setAlong] = createSignal(0)

/// Where the owners are as they move up and down, in owners: what the owner pill's names and the dots pill's rows roll
/// with.
export const [rolled, setRolled] = createSignal(0)

/// What of the snapshot the Home Screen draws from, each a memo of its own, so what changes elsewhere in it (a
/// category opening or closing, a menu) redraws none of it: where it is, each owner's screenfuls and the one each was
/// last on, and while editing, what's edited, drafted and held.
export const at = createMemo(() => state().context.at)
export const screensOf = createMemo(() => state().context.screens)
export const lastOf = createMemo(() => state().context.last)
export const editingNow = createMemo(() => state().matches("editing"))
export const editing = createMemo(() => state().context.editing)
export const drafts = createMemo(() => state().context.drafts)

export const editingOwner = (owner: number) => editingNow() && editing()?.owner === owner

/// Whether this device puts an app on its own home screen (`pinShortcut`): Android's app does.
export const [pins, setPins] = createSignal(false)

/// Whether this Home Screen is a computer's launcher (`client().launcher`): a panel the system summons over whatever's
/// in front, as Spotlight and Raycast are, which goes once an app opens from it: its search floating on its own, with
/// the keyboard as it opens, and under it a card of Home's pages, two at a time, with the owner switcher at its top and
/// the page dots at its foot, which changes size for the App Library and for what's found as it's typed (`Pair`).
/// The arrow keys move a highlight over Home's apps and widgets (`leave` puts it away).
export const [launcher, setLauncher] = createSignal(false)
client().then(({ pinsShortcuts, launcher }) => (setPins(!!pinsShortcuts), setLauncher(!!launcher)), () => {})

/// What's typed in the launcher's search, and which of what it found is chosen, which Return opens.
export const [query, setQuery] = createSignal("")

export const [chosenResult, setChosenResult] = createSignal(0)

/// What the launcher's arrow keys have chosen on Home: an app or a widget of an owner, by which of their pages shown
/// it's on and where on it.
export type Selected = { owner: number; page: number; index: number }

export const [selected, setSelected] = createSignal<Selected | undefined>(undefined, { equals: (a, b) => a?.owner === b?.owner && a?.page === b?.page && a?.index === b?.index })

/// The room the pages have and how they're laid out in it, a phone's until it's known.
export const [room, setRoom] = createSignal<Room>(measure(390, 844, { top: 0, right: 0, bottom: 0, left: 0 }))

/// A page of the room, laid out.
/// How wide the Home Screen's window is, which what's opened over it covers: a phone's page, but a computer launcher's
/// whole panel, which is wider than the one page the room has.
export const windowWidth = () => (launcher() ? viewport().width : room().width)

export const laidPage = createMemo(() => pageOf(room()))

/// Owners' pages as they were just edited, shown until the Home Screen is asked again.
export const [kept, setKept] = createSignal<Editing[]>([])

/// Until when a tap isn't one: just after something carried was put down, a hold, or a swipe.
let quiet = 0

export const tapping = () => Date.now() > quiet

export const quietFor = (ms: number) => void (quiet = Date.now() + ms)

/// Where the pointer was last, and where it pressed what it lifted: what's carried stays where the finger took it,
/// `carrier`, whichever other fingers there are. `carry` is what follows the finger carrying something, while it's
/// dragged: its moves, and its lifting.
export const finger: { pointer: { x: number; y: number }; grab: { x: number; y: number }; carrier: number; carry?: { move: (event: Pressed) => void; drop: (event: Pressed) => void } } = { pointer: { x: 0, y: 0 }, grab: { x: 0, y: 0 }, carrier: -1 }

/// How far the App Library is scrolled down, as it last said: a mouse's wheel turned up at its top goes back to Home.
export const libraryScroll = { scrollTop: 0 }

/// Where a cell is being put down, which stays empty until what was carried has settled into it.
export const [landing, setLanding] = createSignal<{ owner: number; page: number; index: number }>()
