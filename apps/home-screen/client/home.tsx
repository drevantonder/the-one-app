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
// whose snapshot is one signal here. It's written in strict mode, so a native host draws it as the DOM does.
import { createEffect, createMemo, createSignal, Errored, flush, For, Loading, onSettled, refresh, Show, untrack } from "solid-js"
import { createActor } from "xstate"
import {
  Badge,
  Box,
  negative,
  styles,
  tokens,
  Button,
  call,
  changed,
  client,
  confirm,
  covers,
  type Element,
  EditableText,
  Empty,
  haptic,
  Icon,
  type IconName,
  Image,
  keydown,
  type Keyed,
  type Layout,
  leave,
  link,
  List,
  type Located,
  Menu,
  open,
  pinShortcut,
  type Pressed,
  Problem,
  type Rect,
  reducedMotion,
  render,
  Row,
  safeArea,
  Search,
  Sheet,
  sizeChanged,
  Spinner,
  type Style,
  Text,
  toast,
  viewport,
  visible,
  type Wheeled,
  Widget,
} from "ui"
import { give, glide, jump, settle, speed, spring, type Spring, stop } from "./pager.ts"
import { at as gridAt, BAR, type Box as Place, cell as cellOf, DOTS, grid, gridHeight, LAUNCHER_BAR, LAUNCHER_FOOT, LAUNCHER_HEADING, LAUNCHER_SIDE, lefts, measure, page as pageOf, type Page as Laid, type Room, TOP_ROW } from "./layout.ts"
import { type At, carries, type Drop, type Editing, fitted, type Held, type Kind, machine, nearest, pack, type Page, type Placed, shown, spans } from "./machine.ts"
import type { Entrypoint, Category, Item, Listed, Pages, Size, Widget as WidgetItem } from "./types.ts"
import { src } from "./types.ts"

/// The Home Screen's own looks, by the design tokens' roles: Liquid Glass for its pills, + and Done, what's opened or
/// held over a frosted Home Screen, and the window's shadow on what floats.
const looks = styles({
  cover: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0 },
  frosted: { backgroundColor: tokens.colour.veil, backdropFilter: tokens.material.thin },
  pill: { flexDirection: "row", alignItems: "center", borderRadius: tokens.radius.full },
  glass: { paddingInline: tokens.grid[1], pointerEvents: "auto", backgroundColor: tokens.colour.glass, boxShadow: tokens.shadow.glass, backdropFilter: tokens.material.glass },
  panel: { position: "absolute", overflow: "hidden", borderRadius: tokens.radius.large, backgroundColor: tokens.colour.tile },
  // What's in an opened category, at the panel's size in its middle, which shows more of it as it grows.
  inPanel: { position: "absolute", paddingTop: tokens.grid[6], paddingInline: tokens.grid[4], paddingBottom: tokens.grid[4] },
  menu: { position: "absolute", overflow: "hidden", borderRadius: tokens.radius.large, backgroundColor: tokens.colour.glass, backdropFilter: tokens.material.glass, boxShadow: tokens.shadow.window, color: tokens.colour.label },
  overview: { position: "absolute", zIndex: 15, backgroundColor: tokens.colour.veil, backdropFilter: tokens.material.thin, transitionProperty: ["opacity"], transition: tokens.motion.settle },
  gone: { opacity: 0 },
  slots: { position: "relative" },
  slot: { position: "absolute", top: 0, start: 0, zIndex: 0, alignItems: "center", gap: tokens.grid[3] },
  carried: { zIndex: 1 },
  gliding: { transitionProperty: ["translateX", "translateY"], transition: tokens.motion.settle },
  miniature: { position: "relative", overflow: "hidden", borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.background, boxShadow: tokens.shadow.window },
  // An icon sinks under the finger, dimmed at once, for as long as it takes to hold, and comes back quickly.
  icon: { position: "relative", scale: { default: 1, pressed: 0.94 }, opacity: { default: 1, pressed: 0.85 }, transitionProperty: ["scale"], transition: tokens.motion.hold },
  tint: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, borderRadius: tokens.radius.large, backgroundColor: tokens.colour.tile },
  // The launcher's bar is its search, as Raycast's is: bare words, with no box, magnifier or ring.
  bare: { flexGrow: 1, height: tokens.length.control, font: tokens.type.large, color: tokens.colour.label },
  // What a finger carries floats over the pages on a soft shadow, which fades as it settles.
  lifted: { position: "absolute", boxShadow: tokens.shadow.lifted, opacity: 1, transitionProperty: ["opacity"], transition: tokens.motion.settle },
  liftedIcon: { borderRadius: tokens.radius.icon },
  liftedWidget: { borderRadius: tokens.radius.large },
  // The launcher's footer: two pills of Liquid Glass floating over its foot, as Raycast's are.
  floating: { flexDirection: "row", alignItems: "center", gap: tokens.grid[2], paddingInline: tokens.grid[3], borderRadius: tokens.radius.full, backgroundColor: tokens.colour.glass, boxShadow: tokens.shadow.glass, backdropFilter: tokens.material.glass, color: tokens.colour.secondaryLabel, pointerEvents: "auto" },
  key: { minWidth: tokens.grid[4], paddingInline: tokens.grid[1], alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.small, boxShadow: tokens.shadow.edge, backgroundColor: tokens.colour.quinaryLabel },
  // The launcher's panel's edge inside, lit along its top, as Raycast's window has, over everything in it.
  edge: { position: "absolute", top: 0, bottom: 0, start: 0, end: 0, zIndex: 20, boxShadow: tokens.shadow.edge, pointerEvents: "none" },
  round: { borderRadius: tokens.radius.large },
  heading: { position: "absolute", start: 0, end: 0, paddingTop: tokens.grid[3], paddingInline: tokens.grid[4] },
  picked: { backgroundColor: tokens.colour.selection },
  result: { flexDirection: "row", alignItems: "center", gap: tokens.grid[3], height: tokens.length.row, paddingInline: tokens.grid[2], borderRadius: tokens.radius.medium },
  chosen: { position: "absolute", top: negative(tokens.grid[1]), bottom: negative(tokens.grid[1]), start: negative(tokens.grid[1]), end: negative(tokens.grid[1]), borderRadius: tokens.radius.large, backgroundColor: tokens.colour.selection },
})

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

/// The red count on an app's icon, by who it's for and its badge tool (`badge`): what its app says waits for whoever's
/// looking, asked again with everything else as anything changes. One that couldn't be asked has none.
const [badges, setBadges] = createSignal<Record<string, number>>({})
/// Who an app is opened for, as `launch` opens it, and so who its badge is asked for.
const opensFor = (entrypoint: Entrypoint, owner: Owner) => (entrypoint.subject === "user" ? undefined : (entrypoint.subject ?? owner.subject))
const badgeOf = (entrypoint: Entrypoint, owner: Owner) => (entrypoint.badge ? badges()[`${opensFor(entrypoint, owner) ?? ""} ${entrypoint.badge}`] : undefined)
createEffect(owners, (all) => {
  const asked = new Map<string, Promise<number>>()
  for (const owner of all)
    for (const item of [...owner.pages.flatMap((page) => page.items), ...owner.library.flatMap((category) => category.entrypoints)]) {
      const entrypoint = item as Entrypoint
      if (!entrypoint.badge) continue
      const subject = opensFor(entrypoint, owner)
      const key = `${subject ?? ""} ${entrypoint.badge}`
      if (!asked.has(key)) asked.set(key, json<{ count?: number }>(entrypoint.badge, {}, subject).then(({ count }) => count ?? 0, () => 0))
    }
  Promise.all([...asked].map(async ([key, count]) => [key, await count] as const)).then((counted) => setBadges(Object.fromEntries(counted)))
})

/// The one machine, and its snapshot as one signal.
const home = createActor(machine)
const [state, setState] = createSignal(home.getSnapshot())
home.subscribe(setState)
home.start()

/// The owner and screenful nearest where the pages are as they move, which the dots follow during a swipe, as a
/// phone's do, rather than once it ends.
const [seen, setSeen] = createSignal<At>({ owner: 0, screen: 0 }, { equals: (a, b) => a.owner === b.owner && a.screen === b.screen })
/// Where the owners are as they move up and down, in owners: what the owner pill's names and the dots pill's rows roll
/// with.
const [rolled, setRolled] = createSignal(0)

const editing = () => state().context.editing
const editingOwner = (owner: number) => state().matches("editing") && editing()?.owner === owner

/// Whether this device puts an app on its own home screen (`pinShortcut`): Android's app does.
const [pins, setPins] = createSignal(false)
/// Whether this Home Screen is a computer's launcher (`client().launcher`): a panel the system summons over whatever's
/// in front, as Spotlight and Raycast are, which goes once an app opens from it. It's Raycast's chrome around Home's
/// pages: one search bar across its top, with the keyboard as it opens, and two pills floating over its foot, with the page dots and the
/// App Library's glyph, and what Return and ⌘K do. The arrow keys move a highlight over Home's apps and widgets, and
/// typing turns it into a list of the apps found (`leave` puts it away).
const [launcher, setLauncher] = createSignal(false)
/// A Mac's keys are ⌘, and a PC's Ctrl.
const [mac, setMac] = createSignal(false)
client().then(({ pinsShortcuts, launcher, mac }) => (setPins(!!pinsShortcuts), setLauncher(!!launcher), setMac(!!mac)), () => {})
/// What's typed in the launcher's search, and which of the apps it finds is chosen, which Return opens.
const [query, setQuery] = createSignal("")
const [chosen, setChosen] = createSignal(0)
/// What the launcher's arrow keys have chosen on Home: an app or a widget of an owner, by which of their pages shown
/// it's on and where on it.
type Selected = { owner: number; page: number; index: number }
const [selected, setSelected] = createSignal<Selected | undefined>(undefined, { equals: (a, b) => a?.owner === b?.owner && a?.page === b?.page && a?.index === b?.index })
/// Whether the launcher's Actions menu is open.
const [acting, setActing] = createSignal(false)
/// The apps of an App Library whose names have `words` in them: with a screen of their own, or `background`. Those
/// whose name starts with them come first, then those with a word that does, then the rest, each A to Z, as
/// Raycast's root search ranks what it finds.
const findIn = (library: Category[], words: string, background: boolean) => {
  const typed = words.trim().toLowerCase()
  const rank = (name: string) => (name.toLowerCase().startsWith(typed) ? 0 : name.toLowerCase().split(/[\s&-]+/).some((word) => word.startsWith(typed)) ? 1 : 2)
  return library
    .flatMap((category) => category.entrypoints)
    .filter((listed) => !listed.entrypoint === background && listed.name.toLowerCase().includes(typed))
    .sort((a, b) => rank(a.name) - rank(b.name) || a.name.localeCompare(b.name))
}

/** Where an icon or a widget is drawn, with how round its corners are there, which an app grows out of. */
type From = Rect & { radius: number }

/// Opens an app for whoever its owner is, for the group whose install it is, or for its person where it's about
/// whoever opens it. `behind` only starts it, as a finger lands on it. It grows out of its icon or widget, `from`, as
/// an iPhone's does.
const launch = (entrypoint: Entrypoint, owner: Owner, behind?: boolean, from?: From) =>
  open(entrypoint.entrypoint, { subject: entrypoint.subject === "user" ? undefined : (entrypoint.subject ?? owner.subject), behind, from })

/// The room the pages have and how they're laid out in it, a phone's until it's known.
const [room, setRoom] = createSignal<Room>(measure(390, 844, false, { top: 0, right: 0, bottom: 0, left: 0 }))
/// A page of the room, laid out.
const laidPage = createMemo(() => pageOf(room()))

/// An owner's pages as they're kept: each with what's on it and whether it's hidden, to edit.
const draft = (owner: Owner, index: number): Editing => ({ owner: index, pages: fitted(owner.pages.map((page) => ({ items: [...page.items], hidden: page.hidden }))) })

/// Owners' pages as they were just edited, shown until the Home Screen is asked again.
const [kept, setKept] = createSignal<Editing[]>([])

// A finger that stays this long holds what it's on, as a phone's does, and on a Home page, held this much longer
// still, it's lifted into editing, as an iPhone's menu gives way to the drag.
const HOLD = 450
const LONGER = 800
// What's carried makes room where it rests this long, as an iPhone's does, not at every place it passes.
const REST = 150

/// Until when a tap isn't one: just after something carried was put down, a hold, or a swipe.
let quiet = 0
const tapping = () => Date.now() > quiet
/// Where the pointer was last, and where it pressed what it lifted: what's carried stays where the finger took it,
/// `carrier`, whichever other fingers there are.
let pointer = { x: 0, y: 0 }
let grab = { x: 0, y: 0 }
let carrier = -1
/// What follows the finger carrying something, while it's dragged (`HomeScreen`'s): its moves, and its lifting.
let carry: { move: (event: Pressed) => void; drop: (event: Pressed) => void } | undefined

/// A widget on a page: what its app's tool answers, drawn, and asked again as its app says something changed. A widget
/// whose layout says `refresh` (seconds), as one that shows what only time changes does, is also asked again that
/// often while it's in view. A control that failed says so on the widget, and pressing it asks again; else pressing it
/// opens its app.
function Card(props: { item: WidgetItem; owner: Owner; still?: boolean }) {
  const tool = () => props.item.widget.replace("/", "_")
  const layout = createMemo(() => json<Layout & { refresh?: number }>(tool(), { size: props.item.size }, props.owner.subject))
  const [problem, setProblem] = createSignal<string>()
  const again = () => refresh(layout)
  onSettled(() => changed(again))
  const act = async (name: string | undefined, inputs: Record<string, unknown>) => {
    try {
      await call(`${props.item.widget.split("/")[0]}_${name}`, inputs, { subject: props.owner.subject })
    } catch (error) {
      return setProblem(`${(error as Error).message ?? error} Tap to try again.`)
    }
    again()
  }
  const retry = () => !props.still && (setProblem(undefined), again())
  const opens = (card: { getBoundingClientRect(): Rect }) => {
    if (props.still || !state().matches("nothing") || !props.item.opens || !tapping()) return
    const { x, y, width, height } = card.getBoundingClientRect()
    launch(props.item.opens, props.owner, false, { x, y, width, height, radius: 22 * (width / (props.item.size === "2x2" ? 170 : 352)) })
  }
  return (
    <Box style={{ position: "relative", display: "grid", flexGrow: 1 }}>
      <Errored fallback={(error) => <Widget name={props.item.name} size={props.item.size} problem={String((error() as Error).message ?? error())} />}>
        <Loading fallback={<Widget name={props.item.name} size={props.item.size} />}>
          <Widget name={props.item.name} size={props.item.size} layout={layout()} problem={problem()} onCall={props.still ? undefined : act} onOpen={problem() ? retry : props.item.opens ? opens : undefined} />
          <Refresh every={layout().refresh} then={again} />
        </Loading>
      </Errored>
    </Box>
  )
}

/// Runs `then` every `every` seconds, while the Home Screen is shown.
function Refresh(props: { every?: number; then: () => void }) {
  createEffect(
    () => props.every,
    (every) => {
      if (!every) return
      const timer = setInterval(() => untrack(visible) && props.then(), every * 1000)
      return () => clearInterval(timer)
    },
  )
  return null
}

/// What waits in an app, as its app counts it (`badge`): a red count at its icon's corner, as an iPhone's.
const Count = (props: { count?: number }) => (
  <Show when={props.count}>
    <Box aria-hidden="true" style={{ position: "absolute", top: -6, right: -6 }}>
      <Badge variant="destructive">{props.count}</Badge>
    </Box>
  </Show>
)

/// An icon with its name under it, `size` across, drawn `scale` times larger. The app starts as a finger lands on it,
/// and shows as the finger lifts, unless the finger held it, growing out of its icon.
function Tile(props: { name: string; icon: string; size: number; scale: number; badge?: number; onPress?: (behind: boolean, from?: From) => void }) {
  const from = (event: Located): From => {
    const tile = event.currentTarget.getBoundingClientRect()
    const size = props.size * props.scale
    return { x: tile.x + (tile.width - size) / 2, y: tile.y, width: size, height: size, radius: size * 0.225 }
  }
  return (
    <Box
      role="button"
      aria-label={props.name}
      aria-description={props.badge ? `${props.badge} waiting` : undefined}
      onPointerDown={(event) => event.button === 0 && props.onPress?.(true)}
      onClick={(event) => tapping() && props.onPress?.(false, from(event))}
      style={{ alignItems: "center", gap: 7, minWidth: 0, width: "100%" }}
    >
      <Box style={[looks.icon, { width: props.size, height: props.size }]}>
        <Image src={src(props.icon)} alt="" style={{ width: props.size, height: props.size, borderRadius: "xl" }} />
        <Count count={props.badge} />
      </Box>
      <Text style={{ font: "caption-1", lineClamp: 1, textAlign: "center", maxWidth: "100%" }}>{props.name}</Text>
    </Box>
  )
}

/// Taking an item off a Home page asks first, as an iPhone's does: an app stays in the App Library, and a widget's app
/// and data stay.
const removal = (item: Item): Parameters<typeof confirm>[0] =>
  item.widget ? { heading: "Remove Widget?", action: "Remove", destructive: true } : { heading: `Remove “${item.name}”?`, text: "It stays in your App Library.", action: "Remove from Home Screen" }

/// What a Home page holds as it's drawn: an item of the owner's, and where it is among the page's.
type Cell = { item: Item; index: number }

/** What the Home Screen's gestures listen for on what they're given. */
type Holds = Pick<Parameters<typeof Box>[0], "onPointerDown" | "onPointerMove" | "onPointerUp" | "onPointerCancel" | "onContextMenu">

/// Watches a finger or a mouse on what it's given, for the Home Screen's gestures: a press held still is a hold, as is
/// a right-click; a press that moves while editing, or once held, lifts it, and so does one held still a while longer
/// on a Home page. A mouse holds as a finger does, as an iPad's pointer does. `held` says what it is, as it is then. A
/// hold on a page's empty space has nothing to lift or offer, so it edits the page at once, as a phone's does. Only
/// whoever `arranges` lifts anything; a menu is held open for whoever it has one for (`menu`), as an App Library's has
/// for everyone. What's lifted follows the finger that lifted it (`carry`), wherever it goes.
function holds(held: () => Held | undefined, arranges: () => boolean, menu = arranges): Holds {
  let timer: ReturnType<typeof setTimeout> | undefined
  let start: Pressed | undefined
  let rect: Rect | undefined
  // Whether this press has held: a mouse's click as it lets go is the hold's end, not a tap.
  let fired = false
  const clear = () => {
    clearTimeout(timer)
    start = undefined
    fired = false
  }
  // `at` is where the pointer is now: a press held still a while longer lifts with no move to say so.
  const lift = (at: Pressed, from: Pressed) => {
    clear()
    const what = held()
    if (!what?.item || !arranges()) return
    grab = { x: from.clientX, y: from.clientY }
    pointer = { x: at.clientX, y: at.clientY }
    carrier = at.pointerId
    home.send({ type: "lift", held: { ...what, rect }, editing: home.getSnapshot().matches("editing") ? undefined : pending() })
  }
  const hold = (event: Located) => {
    const what = held()
    if (!menu() || !what || !home.getSnapshot().matches("nothing")) return
    fired = !!start
    if (!what.item && !what.listed) return arranges() && home.send({ type: "edit", editing: pending() })
    home.send({ type: "hold", held: { ...what, rect: event.currentTarget.getBoundingClientRect() } })
    const from = start
    if (from && what.from === "page") timer = setTimeout(() => home.getSnapshot().matches("holding") && lift(from, from), LONGER)
  }
  return {
    onPointerDown: (event) => {
      if (event.button || !menu()) return
      clear()
      start = event
      rect = event.currentTarget.getBoundingClientRect()
      // Editing, a press that moves lifts at once, wherever the pointer goes first; otherwise a press held still is a
      // hold.
      if (!home.getSnapshot().matches("editing")) timer = setTimeout(() => start && hold(start), HOLD)
    },
    onPointerMove: (event) => {
      if (carry && event.pointerId === carrier) return carry.move(event)
      if (!start || event.pointerId !== start.pointerId) return
      if (Math.hypot(event.clientX - start.clientX, event.clientY - start.clientY) < 8) return
      // Held, or editing, a press that moves carries what it's on; before that, it's a swipe.
      if (home.getSnapshot().matches("holding") || (home.getSnapshot().matches({ editing: "still" }) && held()?.item)) lift(event, start)
      else clear()
    },
    onPointerUp: (event) => {
      if (carry && event.pointerId === carrier) return carry.drop(event)
      if (event.pointerId !== start?.pointerId) return
      if (fired) quiet = Date.now() + 300
      clear()
    },
    onPointerCancel: (event) => {
      if (carry && event.pointerId === carrier) return carry.drop(event)
      if (event.pointerId === start?.pointerId) clear()
    },
    onContextMenu: (event) => {
      clear()
      hold(event)
    },
  }
}

/// The pages of the owner Home is on, to edit, as they're kept, for what's lifted before editing began.
const pending = () => {
  const at = state().context.at.owner
  return draft(untrack(owners)![at], at)
}

/// The jiggle, as an iPhone's: everything on a page being edited swings from side to side, as far as `shake` says,
/// which grows from nothing as editing begins and dies away as it ends, the swing held where it was. `wobble` is the
/// time it swings by, a frame at a time while it moves; nothing moves for whoever asked for less motion.
const [wobble, setWobble] = createSignal(0)
const [shake, setShake] = createSignal(0)
let shaking = 0
createEffect(
  () => state().matches("editing") && !reducedMotion(),
  (on) => {
    const from = untrack(shake)
    let begun = -1
    cancelAnimationFrame(shaking)
    const frame = (time: number) => {
      if (begun < 0) begun = time
      const done = Math.min(1, (time - begun) / 300)
      setShake(from + ((on ? 1 : 0) - from) * (1 - (1 - done) ** 3))
      if (on) setWobble(time)
      shaking = on || done < 1 ? requestAnimationFrame(frame) : 0
    }
    shaking = requestAnimationFrame(frame)
  },
)
/// How far an item swings, in degrees, now: each its own way through the swing, by where it is.
const swing = (index: number, widget: boolean) => {
  const period = widget ? 340 : 260
  return Math.sin(((wobble() + ((index * 97) % 240)) / period) * Math.PI) * shake() * 1.4
}

/// A Home page: its grid, each item where it was placed, laid out as `laid` says, drawn `scale` times larger. While its
/// owner's pages are edited, everything jiggles with its ⊖, and what's lifted leaves its place empty. An owner with
/// nothing on their pages has one grey line. An app dropped where it isn't installed waits there, dimmed, while it's
/// asked about, and shows its progress as it installs.
function HomePage(props: { owner: Owner; index: number; page: number; placed: Placed<Cell>[]; laid: Laid; scale: number; small?: boolean }) {
  const editingHere = () => !props.small && editingOwner(props.index)
  // What's carried leaves its place empty, where it'll land, wherever that is as the rest make room.
  const lifted = (cell: Cell) => editingHere() && state().matches({ editing: "dragging" }) && carries(state().context.held, cell.item)
  // Dropped where it isn't installed, what was stacked on it waits with it.
  const waiting = (cell: Cell) => editingHere() && carries(state().context.held, cell.item) && (state().matches({ editing: "asking" }) || state().matches({ editing: "installing" }) || state().matches({ editing: "refused" }))
  const arranges = () => props.owner.arranges && !props.small
  const removing = async (item: Item, index: number) => {
    if (await confirm(removal(item))) home.send({ type: "remove", page: props.page, index })
  }
  // In the launcher, what the arrow keys chose, or the pointer's on.
  const picked = (index: number) => {
    const now = selected()
    return launcher() && !props.small && now?.owner === props.index && now.page === props.page && now.index === index
  }
  const point = (index: number) => launcher() && !props.small && state().matches("nothing") && setSelected({ owner: props.index, page: props.page, index })
  // Made once: what a press began, its later moves read.
  const gestures = holds(() => (editingHere() ? undefined : { page: props.page, index: -1, from: "page" }), arranges)
  return (
    <Box
      role="group"
      aria-label={`Page ${props.page + 1}`}
      {...gestures}
      style={{ position: "relative", width: props.laid.width, height: props.laid.height }}
    >
      <Show when={props.placed.length || editingHere()} fallback={<Box style={{ position: "absolute", top: props.laid.top, left: 0, right: 0 }}><Empty>No apps on this page.</Empty></Box>}>
        <For each={props.placed} keyed={({ item }) => (item.item.widget ?? item.item.entrypoint) + (item.item.size ?? "")}>
          {(placed) => (
            <HomeCell
              owner={props.owner}
              index={props.index}
              page={props.page}
              placed={placed()}
              laid={props.laid}
              scale={props.scale}
              small={props.small}
              jiggles={editingHere()}
              lifted={lifted(placed().item)}
              waiting={waiting(placed().item)}
              picked={picked(placed().item.index)}
              held={() => ({ item: placed().item.item, page: props.page, index: placed().item.index, from: "page" })}
              arranges={arranges}
              onPoint={() => point(placed().item.index)}
              onRemove={arranges() && !waiting(placed().item) ? () => removing(placed().item.item, placed().item.index) : undefined}
            />
          )}
        </For>
      </Show>
    </Box>
  )
}

/// Where a cell is being put down, which stays empty until what was carried has settled into it.
const [landing, setLanding] = createSignal<{ owner: number; page: number; index: number }>()

/// Something on a Home page, where it's placed. While its page is edited it glides to where it now goes, as an
/// iPhone's do when an app is dragged among them or taken off, rather than jumping there: it's drawn where it was, then
/// springs to its new place (pager.ts's spring, which keeps its speed if it moves again on the way). As editing
/// begins, nothing moves.
function HomeCell(props: {
  owner: Owner
  index: number
  page: number
  placed: Placed<Cell>
  laid: Laid
  scale: number
  small?: boolean
  jiggles: boolean
  lifted: boolean
  waiting: boolean
  picked: boolean
  held: () => Held
  arranges: () => boolean
  onPoint: () => void
  onRemove?: () => void
}) {
  const box = () => cellOf(props.laid, props.placed)
  // Made once: what a press began, its later moves read.
  const gestures = holds(() => props.held(), () => props.arranges())
  const [dx, setDx] = createSignal(0)
  const [dy, setDy] = createSignal(0)
  const x = spring(setDx)
  const y = spring(setDy)
  let was: Place | undefined
  let wasEditing = false
  createEffect(
    () => ({ ...box(), editing: state().matches("editing") }),
    (now) => {
      if (was && now.editing && wasEditing && !props.small && (was.x !== now.x || was.y !== now.y)) {
        x.value += was.x - now.x
        y.value += was.y - now.y
        glide(x, 0)
        glide(y, 0)
      }
      was = now
      wasEditing = now.editing
    },
  )
  const item = () => props.placed.item.item
  const widget = () => !!item().widget
  const landingHere = () => {
    const now = landing()
    return now?.owner === props.index && now.page === props.page && now.index === props.placed.item.index
  }
  return (
    <Box
      {...gestures}
      onPointerMove={(event) => (props.onPoint(), gestures.onPointerMove?.(event))}
      style={{
        position: "absolute",
        left: box().x,
        top: box().y,
        width: box().width,
        height: box().height,
        translateX: dx(),
        translateY: dy(),
        rotate: props.jiggles ? swing(props.placed.item.index, widget()) : 0,
        opacity: props.lifted || landingHere() ? 0 : 1,
      }}
    >
      <Show when={props.picked}>
        {/* What's chosen has the highlight behind it: an app's, and a widget's own tint brightened by it. */}
        <Box aria-hidden="true" style={looks.chosen} />
      </Show>
      <Show
        when={widget()}
        fallback={
          <Box style={{ position: "relative", opacity: props.waiting ? 0.45 : 1 }}>
            <Tile
              name={(item() as Entrypoint).name}
              icon={(item() as Entrypoint).icon}
              size={props.laid.tile}
              scale={props.scale}
              badge={badgeOf(item() as Entrypoint, props.owner)}
              onPress={(behind, from) => state().matches("nothing") && launch(item() as Entrypoint, props.owner, behind, from)}
            />
          </Box>
        }
      >
        <Card item={item() as WidgetItem} owner={props.owner} still={props.small || props.jiggles} />
      </Show>
      <Show when={props.waiting && state().matches({ editing: "installing" })}>
        <Box role="status" aria-label={`Installing ${item().name}`} style={{ position: "absolute", top: props.laid.tile / 2 - 16, left: box().width / 2 - 16, width: 32, height: 32, borderRadius: "full", backgroundColor: "overlay", color: "primary-foreground", alignItems: "center", justifyContent: "center" }}>
          <Spinner />
        </Box>
      </Show>
      <Show when={props.jiggles && props.onRemove}>
        <Minus label={`Remove ${item().name}`} left={widget() ? -6 : Math.max(0, (box().width - props.laid.tile) / 2 - 8)} top={-6} onPress={() => props.onRemove?.()} />
      </Show>
    </Box>
  )
}

/// ⊖, at the corner of what it takes off: pressed, not held or lifted.
function Minus(props: { label: string; left: number; top: number; onPress: () => void }) {
  return (
    <Box
      role="button"
      aria-label={props.label}
      onClick={() => tapping() && props.onPress()}
      style={{ position: "absolute", zIndex: 2, left: props.left, top: props.top, width: 24, height: 24, borderRadius: "full", backgroundColor: "muted-foreground", color: "background", alignItems: "center", justifyContent: "center" }}
    >
      <Icon name="minus" />
    </Box>
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
/// Where each category's place was drawn as it was opened, which it grows out of and shrinks back into.
const places: Record<string, Rect> = {}

/// The App Library's search, its categories and what it found, laid out for the room it's in: categories as many
/// across as fit at an iPhone's size, each its icons in a square, whose padding and gaps follow its width.
function shelf(width: number) {
  const across = Math.max(2, Math.floor((width + 20) / (152 + 20)))
  const category = (width - (across - 1) * 20) / across
  const padding = { top: category * 0.072, inline: category * 0.066, bottom: category * 0.054 }
  const [columnGap, rowGap] = [category * 0.06, category * 0.036]
  const icon = (category - padding.inline * 2 - columnGap) / 2 - 18 - 4
  const found = Math.max(1, Math.floor((width + 12) / (76 + 12)))
  return { across, category, padding, columnGap, rowGap, icon, found }
}

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
function Library(props: { owner: Owner }) {
  const [query, setQuery] = createSignal("")
  // Tapped while a search shows it open, Background Apps closes only for that search.
  const [shut, setShut] = createSignal(false)
  const searching = () => !!query().trim()
  const inline = () => (room().wide ? room().side : 20)
  const width = () => Math.min(1440, room().width - 2 * inline())
  const laid = createMemo(() => shelf(width()))
  // Tapped again as it shrinks back into its place, it turns and grows again.
  const opening = (key: () => string) => (event: Located) => {
    // An app pressed in its square opens the app, not the category too.
    if (Date.now() - pressedApp < 100) return
    const back = state().matches({ category: "closing" }) && state().context.category === key()
    if (!state().matches("nothing") && !back) return
    places[key()] = event.currentTarget.getBoundingClientRect()
    home.send({ type: "open", category: key() })
  }
  const found = (background: boolean) => findIn(props.owner.library, query(), background)
  const apps = () => sorted(props.owner.library, false)
  const background = () => sorted(props.owner.library, true)
  const open = () => (searching() ? !shut() : unfolded())
  // Components, not JSX made again whenever the App Library's asked again: its rows stay, each with its icon.
  const Categories = (inner: { list: Category[]; prefix: string }) => (
    <Box style={{ display: "grid", gridTemplateColumns: `repeat(${laid().across}, 1fr)`, gap: 20 }}>
      <For each={inner.list} keyed={(category) => category.category}>
        {(category) => {
          const key = () => inner.prefix + category().category
          const shown = () => (category().entrypoints.length > 4 ? category().entrypoints.slice(0, 3) : category().entrypoints)
          return (
            <Box style={{ alignItems: "center", gap: 6, minWidth: 0 }}>
              <Box
                role="group"
                aria-label={category().category}
                onClick={opening(key)}
                style={{
                  display: "grid",
                  gridTemplateColumns: "repeat(2, 1fr)",
                  columnGap: laid().columnGap,
                  rowGap: laid().rowGap,
                  width: laid().category,
                  height: laid().category,
                  paddingTop: laid().padding.top,
                  paddingInline: laid().padding.inline,
                  paddingBottom: laid().padding.bottom,
                  position: "relative",
                }}
              >
                <Box aria-hidden="true" style={looks.tint} />
                <For each={shown()} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                  {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} icon={laid().icon} small />}
                </For>
                <Show when={category().entrypoints.length > 4}>
                  <Box role="button" aria-label={`All of ${category().category}`} onClick={opening(key)} style={{ display: "grid", gridTemplateColumns: "repeat(2, 1fr)", gap: 5, alignSelf: "center", justifyContent: "center", width: laid().icon + 14, height: laid().icon + 14, marginInline: "auto" }}>
                    <For each={category().entrypoints.slice(3, 7)} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                      {(entrypoint) => <Image src={src(entrypoint().icon)} alt="" style={{ width: (laid().icon + 9) / 2, height: (laid().icon + 9) / 2, borderRadius: "md" }} />}
                    </For>
                  </Box>
                </Show>
              </Box>
              {/* Its apps take the focus, and its small icons, which open it, so its name doesn't again: it's for a finger. */}
              <Box onClick={opening(key)} style={{ maxWidth: "100%" }}>
                <Text style={{ font: "footnote", color: "muted-foreground", lineClamp: 1 }}>{category().category}</Text>
              </Box>
            </Box>
          )
        }}
      </For>
    </Box>
  )
  const Found = (inner: { list: Listed[] }) => (
    <Box style={{ display: "grid", gridTemplateColumns: `repeat(${laid().found}, 1fr)`, rowGap: 16, columnGap: 12 }}>
      <For each={inner.list} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
        {(entrypoint) => <LibraryEntrypoint entrypoint={entrypoint()} owner={props.owner} icon={60} />}
      </For>
    </Box>
  )
  return (
    <Box role="region" aria-label="App Library" style={{ width: room().width, height: room().height, flexShrink: 0 }}>
      <Show when={!launcher()}>
        <Box style={{ paddingTop: room().safe.top + (TOP_ROW - BAR) / 2, paddingBottom: (TOP_ROW - BAR) / 2 - 12, alignItems: "center" }}>
          <Box style={{ width: Math.min(room().width - 2 * BAR - 16, 384) }}>
            <Search placeholder="App Library" aria-label="Search the App Library" value={query()} onInput={(event) => (setQuery(event.currentTarget.value), setShut(false))} />
          </Box>
        </Box>
      </Show>
      <Box style={{ flexGrow: 1, flexBasis: 0, overflow: "scroll", paddingInline: inline(), paddingTop: launcher() ? 16 : 12, paddingBottom: 96 + room().safe.bottom }}>
        <Box style={{ gap: 20, width: width(), marginInline: "auto" }}>
          <Show when={searching() ? found(false).length || found(true).length : props.owner.library.length} fallback={<Empty>{searching() ? "No apps match." : "No apps yet."}</Empty>}>
            <Show when={searching()} fallback={<Show when={apps().length}><Categories list={apps()} prefix="" /></Show>}>
              <Show when={found(false).length}><Found list={found(false)} /></Show>
            </Show>
            <Show when={searching() ? found(true).length : background().length}>
              <Box role="region" aria-label="Background Apps" style={{ gap: 12 }}>
                <Box
                  role="button"
                  aria-expanded={open() ? "true" : "false"}
                  onClick={() => (searching() ? setShut(!shut()) : unfold(!unfolded()))}
                  style={{ flexDirection: "row", alignItems: "center", alignSelf: "flex-start", gap: 4, marginBlock: 4, paddingBlock: 4, color: "muted-foreground" }}
                >
                  <Text style={{ font: "subheadline" }}>Background Apps</Text>
                  <Box style={{ rotate: open() ? 90 : 0, scale: 0.875 }}>
                    <Icon name="caret-right" along />
                  </Box>
                </Box>
                <Show when={open()}>
                  <Show when={searching()} fallback={<Categories list={background()} prefix={BEHIND} />}>
                    <Found list={found(true)} />
                  </Show>
                </Show>
              </Box>
            </Show>
          </Show>
        </Box>
      </Box>
    </Box>
  )
}

/// An app's icon in the App Library, `icon` across: a tap opens it, or for an app with no entrypoint, the App Store
/// at its page, and a finger that holds it opens its menu; moving it carries it onto whichever owner's pages it's
/// dropped on. In a category's square (`small`) its name is smaller.
function LibraryEntrypoint(props: { entrypoint: Listed; owner: Owner; icon: number; small?: boolean }) {
  const badge = () => (opens(props.entrypoint) ? badgeOf(opens(props.entrypoint)!, props.owner) : undefined)
  const from = (event: Located): From => {
    const at = event.currentTarget.getBoundingClientRect()
    return { x: at.x + (at.width - props.icon) / 2, y: at.y, width: props.icon, height: props.icon, radius: props.icon * 0.225 }
  }
  const held = holds(() => ({ item: opens(props.entrypoint), listed: props.entrypoint, page: 0, index: -1, from: "library" }), () => true)
  return (
    <Box
      role="button"
      aria-label={props.entrypoint.name}
      aria-description={badge() ? `${badge()} waiting` : undefined}
      {...held}
      onPointerDown={(event) => {
        if (state().matches("nothing") && opens(props.entrypoint)) launch(opens(props.entrypoint)!, props.owner, true)
        held.onPointerDown?.(event)
      }}
      onClick={(event) => {
        pressedApp = Date.now()
        if (tapping() && state().matches("nothing")) press(props.entrypoint, props.owner, false, from(event))
      }}
      style={{ alignItems: "center", gap: props.small ? 4 : 6, minWidth: 0 }}
    >
      <Box style={{ position: "relative", width: props.icon, height: props.icon }}>
        <Image src={src(props.entrypoint.icon)} alt="" style={{ width: props.icon, height: props.icon, borderRadius: "xl" }} />
        <Count count={badge()} />
      </Box>
      <Text style={{ font: props.small ? "caption-2" : "caption-1", lineClamp: 1, textAlign: "center", maxWidth: "100%" }}>{props.entrypoint.name}</Text>
    </Box>
  )
}
/// When an app in the App Library was last pressed, which its category's square, under it, doesn't take as its own.
let pressedApp = 0

/// An app of an App Library as an entrypoint, which it opens, or nothing, for an app with none.
const opens = (listed: Listed): Entrypoint | undefined => (listed.entrypoint ? (listed as Entrypoint) : undefined)
/// Opens an app of an App Library: its entrypoint, or for an app with none, the App Store at its page, by a link
/// (`one:app-store#app=…`), growing out of its icon as any app does.
const press = (listed: Listed, owner: Owner, behind?: boolean, from?: From) => {
  if (opens(listed)) return launch(opens(listed)!, owner, behind, from)
  const store = untrack(owners)?.[0]?.store
  if (behind || !store) return
  open(`${store}#app=${encodeURIComponent(listed.app)}`, { from })
}

/// Who a call about an install is for: a group or the network, or for a user's own, nobody said.
const subjectOf = (installed: string) => (installed.startsWith("users/") ? undefined : installed)
const and = (words: string[]) => new Intl.ListFormat("en", { type: "conjunction" }).format(words)

/// A value that moves to where it's told, `0` to `1`, over `duration` milliseconds, eased as iOS's are, a frame at a
/// time, and from wherever it got to when it's told to turn back: `done` is called once it's there. For whoever asked
/// for less motion, it's there at once.
function tween(duration: number, done: (at: number) => void) {
  const [value, setValue] = createSignal(0)
  let frame = 0
  let from = 0
  let to = 0
  let begun = -1
  const step = (time: number) => {
    if (begun < 0) begun = time
    const t = Math.min(1, (time - begun) / (duration * Math.abs(to - from) || 1))
    setValue(from + (to - from) * (1 - (1 - t) ** 3))
    if (t < 1) frame = requestAnimationFrame(step)
    else ((frame = 0), done(to))
  }
  return {
    value,
    to(target: number) {
      cancelAnimationFrame(frame)
      from = untrack(value)
      to = target
      begun = -1
      if (untrack(reducedMotion) || from === to) return void (setValue(to), done(to))
      frame = requestAnimationFrame(step)
    },
    stop: () => cancelAnimationFrame(frame),
  }
}

/// A category opened over its App Library: its name, and its apps in a panel that grows out of its square there, its
/// shape becoming the panel's, and shrinks back into it, as an iPhone's folder does, while what's behind it is dimmed. Its apps open only once it has
/// grown. Tapping beside the panel, Escape or the device's own back closes it, even as it grows: it turns back from
/// where it got to. For whoever asked for less motion, it fades in and out where it is.
function OpenCategory(props: { owner: Owner; opened: string }) {
  const category = () => openedCategory(props.owner.library, props.opened)
  const phase = () => (state().matches({ category: "opening" }) ? "opening" : state().matches({ category: "open" }) ? "open" : "closing")
  const close = () => !state().matches({ category: "closing" }) && home.send({ type: "close" })
  const zoom = tween(300, () => home.send({ type: "zoomed" }))
  createEffect(createMemo(phase), (now) => {
    if (now !== "open") zoom.to(now === "opening" ? 1 : 0)
  })
  onSettled(() => {
    const shut = covers(close)
    const keys = keydown((event) => event.key === "Escape" && close())
    return () => (shut(), keys(), zoom.stop())
  })
  // The panel, in the middle of the window: three across, as many rows as it has apps for.
  const panel = () => {
    const width = Math.min(352, room().width - 56)
    const rows = Math.max(1, Math.ceil((category()?.entrypoints.length ?? 0) / 3))
    const height = 24 + rows * 88 + (rows - 1) * 16 + 16
    const top = (viewport().height - (height + 20 + 34)) / 2 + 54
    return { x: (room().width - width) / 2, y: top, width, height }
  }
  const t = () => zoom.value()
  const moving = () => !reducedMotion() && !!places[props.opened]
  // Where it's drawn: its square in the App Library, becoming the panel as it opens.
  const shape = () => {
    const [to, from] = [panel(), places[props.opened]]
    if (!moving() || !from) return to
    const at = (a: number, b: number) => a + (b - a) * t()
    return { x: at(from.x, to.x), y: at(from.y, to.y), width: at(from.width, to.width), height: at(from.height, to.height) }
  }
  const tile = () => Math.min(60, 88 - 28)
  return (
    <Box
      role="dialog"
      aria-label={category()?.category}
      style={{ position: "absolute", top: 0, left: 0, width: room().width, height: viewport().height, zIndex: 10, pointerEvents: phase() === "closing" ? "none" : "auto" }}
    >
      {/* What's behind blurs through a veil. */}
      <Box aria-hidden="true" onClick={close} style={[looks.cover, looks.frosted, { opacity: t() }]} />
      <Text role="heading" aria-level={2} style={{ position: "absolute", left: 0, right: 0, top: panel().y - 54, font: "title", textAlign: "center", opacity: t(), translateY: moving() ? 12 * (1 - t()) : 0 }}>
        {category()?.category}
      </Text>
      <Box style={[looks.panel, { start: shape().x, top: shape().y, width: shape().width, height: shape().height, opacity: moving() ? 1 : t() }]}>
        <Box style={[looks.inPanel, { start: (shape().width - panel().width) / 2, top: (shape().height - panel().height) / 2, width: panel().width, height: panel().height }]}>
          <Box style={{ display: "grid", gridTemplateColumns: "repeat(3, 1fr)", rowGap: 16, columnGap: 12, opacity: Math.max(0, (t() - 0.3) / 0.7) }}>
            <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
              {(entrypoint) => (
                <Box style={{ height: 88 }}>
                  <Tile name={entrypoint().name} icon={entrypoint().icon} size={tile()} scale={1} badge={badgeOf(entrypoint() as Entrypoint, props.owner)} onPress={(behind, from) => phase() === "open" && press(entrypoint(), props.owner, behind, from)} />
                </Box>
              )}
            </For>
          </Box>
        </Box>
      </Box>
    </Box>
  )
}

/// A spring, as iOS's menus open: quick, a little past where it goes and back, `t` from 0 to 1.
const springy = (t: number) => {
  const [damping, natural] = [0.7, 10]
  const damped = natural * Math.sqrt(1 - damping ** 2)
  return 1 - Math.exp(-damping * natural * t) * (Math.cos(damped * t) + ((damping * natural) / damped) * Math.sin(damped * t))
}

/// What a finger holds, lifted over the dimmed Home Screen, as a phone's is, with its menu beside it, which grows out of
/// it with a spring: Edit Home Screen and Remove App on a Home page; in the App Library, who the app is installed for,
/// Add to Home Screen, onto the pages of the owner Home was last on, and Remove App. Moving the finger, or on a Home
/// page holding it a while longer, carries it into editing; anywhere else, Escape, a tap beside it or the device's own
/// back puts it down.
function Holding(props: { owner: Owner; index: number; held: Held }) {
  const dismiss = () => state().matches("holding") && home.send({ type: "dismiss" })
  const edit = () => home.send({ type: "edit", editing: draft(props.owner, props.index) })
  const removing = () => {
    const [owner, index, held] = [props.owner, props.index, props.held]
    dismiss()
    removeFromHome(owner, index, held.page, held.index, held.item!)
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
  const adding = () => {
    dismiss()
    addToHome(props.owner, props.index, props.held.item!)
  }
  const grow = tween(450, () => {})
  onSettled(() => {
    haptic("medium")
    grow.to(1)
    const shut = covers(dismiss)
    const keys = keydown((event) => event.key === "Escape" && dismiss())
    return () => (shut(), keys(), grow.stop())
  })
  // Where it is in the window, which its menu is under, or over where there's no room under it.
  const held = props.held.rect ?? { x: 0, y: 0, width: 0, height: 0 }
  const scale = untrack(room).scale
  const [width, height] = [untrack(viewport).width, untrack(viewport).height]
  const below = held.y + held.height < height * 0.6
  const left = Math.max(12, Math.min(width - 252, held.x))
  const item = props.held.item
  const library = props.held.from === "library"
  // It's added from here only where it opens, onto the pages of whoever Home was last on; elsewhere, it's carried there.
  const adds = library && !!item && addable(item, props.owner)
  // An app the device's own home screen opens by its link alone: one that opens for whoever opens it, not a group.
  const app = item && !item.widget ? (item as Entrypoint) : undefined
  const pinnable = pins() && !!app && (app.subject === "user" || (!app.subject && !props.owner.subject))
  const pinning = () => {
    dismiss()
    pinShortcut(app!.entrypoint).catch((error) => toast(error as Error))
  }
  const motion = () => (reducedMotion() ? 1 : springy(grow.value() * 1.2))
  // Its copy is laid out at a phone's size, drawn as much larger as the pages are, from its middle.
  const [copyWidth, copyHeight] = [held.width / scale, held.height / scale]
  const tile = untrack(laidPage).tile
  return (
    <Box style={{ position: "absolute", top: 0, left: 0, width: room().width, height: viewport().height, zIndex: 20 }}>
      {/* The Home Screen blurs through a veil under what's held, and its menu floats in glass, as a window does. */}
      <Box aria-hidden="true" onClick={dismiss} style={[looks.cover, looks.frosted, { opacity: Math.min(1, grow.value() * 2) }]} />
      <Show when={item ?? (props.held.listed as Item | undefined)}>
        {(item) => (
          <Box aria-hidden="true" style={{ position: "absolute", display: "grid", left: held.x + (held.width - copyWidth) / 2, top: held.y + (held.height - copyHeight) / 2, width: copyWidth, height: copyHeight, scale: scale * (1 + 0.06 * motion()), pointerEvents: "none" }}>
            <Show when={item().widget} fallback={library ? <Image src={src((item() as Entrypoint).icon)} alt="" style={{ width: copyWidth, height: copyWidth, borderRadius: "xl" }} /> : <Tile name={(item() as Entrypoint).name} icon={(item() as Entrypoint).icon} size={tile} scale={scale} />}>
              <Card item={item() as WidgetItem} owner={props.owner} still />
            </Show>
          </Box>
        )}
      </Show>
      <Box
        role="menu"
        aria-label={(item ?? props.held.listed)?.name}
        style={[
          looks.menu,
          {
            start: left,
            ...(below ? { top: held.y + held.height + 12 } : { bottom: height - held.y + 12 }),
            width: 240,
            opacity: Math.min(1, grow.value() * 3),
            scale: reducedMotion() ? 1 : 0.25 + 0.75 * motion(),
          },
        ]}
      >
        <Show when={library && props.held.listed}>
          {(listed) => (
            <Box style={{ paddingInline: 16, paddingBlock: 10, borderBottomWidth: 1 }}>
              <Text style={{ font: "footnote", color: "muted-foreground" }}>Installed for {whose(listed().installed.for)}</Text>
            </Box>
          )}
        </Show>
        <Show when={!library}>
          <HoldItem label="Edit Home Screen" icon="squares-four" onPress={edit} />
        </Show>
        <Show when={adds}>
          <HoldItem label={props.index ? `Add to ${props.owner.name}'s Home Screen` : "Add to Home Screen"} icon="plus-circle" onPress={adding} />
        </Show>
        <Show when={pinnable}>
          <HoldItem label="Add to Android Home Screen" icon="device-mobile" onPress={pinning} />
        </Show>
        <Show when={item || props.held.listed}>
          <HoldItem label={item?.widget ? "Remove Widget" : "Remove App"} why={cant()} icon="minus-circle" destructive={!cant()} onPress={library ? uninstalls : removing} />
        </Show>
      </Box>
    </Box>
  )
}

/// One of what can be done with what's held: its words, why it can't be done if it can't, greyed, and its icon.
function HoldItem(props: { label: string; why?: string; icon: IconName; destructive?: boolean; onPress: () => void }) {
  return (
    <Box
      role="menuitem"
      aria-disabled={props.why ? "true" : undefined}
      aria-description={props.why}
      onClick={() => !props.why && props.onPress()}
      style={{ flexDirection: "row", alignItems: "center", justifyContent: "space-between", gap: 16, minHeight: 44, paddingInline: 16, paddingBlock: props.why ? 10 : 0, borderTopWidth: 1, color: props.why ? "muted-foreground" : props.destructive ? "destructive" : undefined }}
    >
      <Box style={{ flexShrink: 1 }}>
        <Text>{props.label}</Text>
        <Show when={props.why}>{(why) => <Text style={{ font: "footnote", color: "muted-foreground" }}>{why()}</Text>}</Show>
      </Box>
      <Icon name={props.icon} />
    </Box>
  )
}

/// The app something on a page opens: an app's own, or a widget's app.
const opener = (item: Item): Entrypoint | undefined => (item.widget ? ((item as WidgetItem).opens ?? undefined) : (item as Entrypoint))

/// Whether an app opens on an owner's pages: it's installed for them, or for everyone.
const installedFor = (item: Entrypoint, owner: Owner) => owner.apps.some((one) => one.app === item.app)

/// Where an app is on an owner's pages, if it's there: which of the pages shown, and where on it.
const placeOf = (owner: Owner, item: Entrypoint) => {
  const name = theirs(item, owner)
  for (const [page, at] of shown(owner.pages).entries()) {
    const index = owner.pages[at].items.findIndex((other) => !other.widget && other.entrypoint === name)
    if (index >= 0) return { page, index }
  }
}
/// Whether an app can be added to an owner's pages from the App Library: whoever's adding arranges them, it opens
/// there, and it isn't on them already, even on a page they hide.
const addable = (item: Item, owner: Owner) =>
  !item.widget && owner.arranges && installedFor(item as Entrypoint, owner) && !owner.pages.some((page) => page.items.some((other) => !other.widget && other.entrypoint === theirs(item as Entrypoint, owner)))
/// Puts an app last on an owner's last page, as a phone's App Library does.
function addToHome(owner: Owner, at: number, item: Item) {
  const editing = draft(owner, at)
  const last = shown(editing.pages).at(-1)
  const pages = last === undefined ? [...editing.pages, { items: [item] }] : editing.pages.map((page, index) => (index === last ? { ...page, items: [...page.items, item] } : page))
  keep([{ ...editing, pages: fitted(pages) }])
}
/// Takes what's on an owner's page off it, once its alert says that's what's wanted: `page` is which of the pages shown.
async function removeFromHome(owner: Owner, at: number, page: number, index: number, item: Item) {
  const editing = draft(owner, at)
  if (!(await confirm(removal(item)))) return
  const kept = shown(editing.pages)[page]
  const pages = editing.pages.map((one, which) => (which === kept ? { ...one, items: one.items.toSpliced(index, 1) } : one))
  keep([{ ...editing, pages: pages.filter((one) => one.items.length) }])
}

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

/// How large a widget is drawn in the gallery, small, as it would be.
const PREVIEW: Record<Size, [number, number]> = { "2x2": [120, 120], "4x2": [248, 120], "4x4": [248, 248] }

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
                {(widget) => <Row heading={widget().name} start={widget().opens ? <Image src={src(widget().opens!.icon)} alt="" style={{ width: 32, height: 32, borderRadius: "md" }} /> : undefined} onClick={() => (setPicked(widget()), setSize(widget().sizes[0]))} />}
              </For>
            </List>
          </Show>
        }
      >
        {(widget) => (
          <Box style={{ gap: 16 }}>
            <Box role="radiogroup" aria-label="Size" style={{ flexDirection: "row", gap: 16, overflow: "scroll", padding: 4 }}>
              <For each={widget().sizes}>
                {(one) => (
                  <Box role="radio" aria-checked={size() === one ? "true" : "false"} aria-label={SIZES[one]} onClick={() => setSize(one)} style={{ alignItems: "center", gap: 8, flexShrink: 0, color: size() === one ? "foreground" : "muted-foreground" }}>
                    <Box style={{ display: "grid", width: PREVIEW[one][0], height: PREVIEW[one][1], padding: 2, borderRadius: "xl", borderWidth: size() === one ? 1 : 0, borderColor: "muted-foreground", pointerEvents: "none" }}>
                      <Card item={{ ...widget(), size: one }} owner={props.owner} still />
                    </Box>
                    <Text style={{ font: "footnote" }}>{SIZES[one]}</Text>
                  </Box>
                )}
              </For>
            </Box>
            <Button onClick={add}>Add Widget</Button>
          </Box>
        )}
      </Show>
    </Sheet>
  )
}

/// The page overview's thumbnails: a page laid out as a tall phone's, drawn this much smaller.
const MINIATURE = 136
const THUMB = grid(368, 654, 32, 20, 20, 0)

/// The page overview: every Home page small, in order, each with a tick for whether it's shown. Unticked, a page is
/// hidden and can be deleted; dragging one puts it elsewhere, following the finger, and taking another's place as it
/// passes over it, the others gliding aside to make room. Let go, it springs into its place. Done goes back to editing.
function Overview(props: { owner: Owner; index: number; going?: boolean }) {
  const pages = () => editing()?.pages ?? []
  const [dragging, setDragging] = createSignal<Page>()
  // The one last dragged, which follows the finger and springs back from where it's let go, `moved` from its place.
  const [carried, setCarried] = createSignal<Page>()
  const [moved, setMoved] = createSignal({ x: 0, y: 0 })
  const remove = async (page: number) => {
    if (await confirm({ heading: "Delete Page?", text: "Its apps stay in the App Library.", action: "Delete", destructive: true })) home.send({ type: "delete", page })
  }
  // The thumbnails' grid: as many across as fit, each a miniature with its tick under it.
  const across = () => Math.max(1, Math.floor((room().width - 48 + 16) / (MINIATURE + 16)))
  const pitch = () => ({ x: (room().width - 48 - (across() - 1) * 16) / across() + 16, y: (MINIATURE * 16) / 9 + 12 + 28 + 24 })
  let down: (Pressed & { at: number; laid: { x: number; y: number } }) | undefined
  let moves: { time: number; x: number; y: number }[] = []
  const place = (at: number) => ({ x: (at % across()) * pitch().x, y: Math.floor(at / across()) * pitch().y })
  const follow = (event: Pressed) => {
    if (!down) return { x: 0, y: 0 }
    const now = place(pages().indexOf(dragging()!))
    const at = { x: event.clientX - down.clientX - (now.x - down.laid.x), y: event.clientY - down.clientY - (now.y - down.laid.y) }
    setMoved(at)
    return at
  }
  const drag = (page: Page, index: number) => ({
    onPointerDown: (event: Pressed) => {
      if (event.button) return
      down = { ...event, at: index, laid: place(index) }
      moves = []
      setMoved({ x: 0, y: 0 })
      setDragging(page)
      setCarried(page)
    },
    onPointerMove: (event: Pressed) => {
      if (!down || event.pointerId !== down.pointerId) return
      follow(event)
      moves.push({ time: event.timeStamp, x: event.clientX, y: event.clientY })
      if (moves.length > 32) moves.shift()
      // The thumbnail it's over, by where its middle is.
      const box = event.currentTarget.getBoundingClientRect()
      const [column, row] = [Math.floor((box.x + box.width / 2 - 24 + 8) / pitch().x), Math.floor((box.y + box.height / 2 - (room().safe.top + TOP_ROW + 16) + 12) / pitch().y)]
      const to = Math.max(0, Math.min(pages().length - 1, row * across() + Math.max(0, Math.min(across() - 1, column))))
      const at = pages().indexOf(page)
      if (at < 0 || to === at) return
      home.send({ type: "reorder", from: at, to })
      flush()
      haptic("selection")
      follow(event)
    },
    onPointerUp: (event: Pressed) => end(event),
    onPointerCancel: (event: Pressed) => end(event),
  })
  const end = (event: Pressed) => {
    if (!down || event.pointerId !== down.pointerId) return
    down = undefined
    const at = untrack(moved)
    const thrown = event.timeStamp ? { x: speed(moves.map(({ time, x }) => ({ time, value: x })), event.timeStamp), y: speed(moves.map(({ time, y }) => ({ time, value: y })), event.timeStamp) } : { x: 0, y: 0 }
    const draw = () => setMoved({ x: x.value, y: y.value })
    const [x, y] = [spring(draw), spring(draw)]
    jump(x, at.x)
    jump(y, at.y)
    glide(x, 0, thrown.x)
    glide(y, 0, thrown.y)
    setTimeout(() => setDragging(), 0)
  }
  onSettled(() => {
    const shut = covers(() => home.send({ type: "dismiss" }))
    const keys = keydown((event) => event.key === "Escape" && home.send({ type: "dismiss" }))
    return () => (shut(), keys())
  })
  const scale = MINIATURE / THUMB.width
  return (
    // Over the blurred Home Screen, it fades out as it closes; its miniatures float, with a window's shadow.
    <Box role="dialog" aria-label="Pages" style={[looks.overview, props.going && looks.gone, { top: 0, start: 0, width: room().width, height: viewport().height }]}>
      <Box style={{ flexDirection: "row", justifyContent: "flex-end", alignItems: "center", paddingTop: room().safe.top, height: room().safe.top + TOP_ROW, paddingInline: 16 }}>
        <Button variant="secondary" onClick={() => home.send({ type: "dismiss" })}>Done</Button>
      </Box>
      <Box style={{ flexGrow: 1, flexBasis: 0, overflow: "scroll", paddingTop: 16, paddingInline: 24, paddingBottom: 48 + room().safe.bottom }}>
        {/* Each in its place by where it is in the pages, so that, as one's dragged past the others, they glide to theirs. */}
        <Box style={[looks.slots, { height: Math.ceil(pages().length / across()) * pitch().y - 24 }]}>
          <For each={pages()}>
            {(page, index) => (
              <Box
                style={[
                  looks.slot,
                  carried() === page ? looks.carried : looks.gliding,
                  { width: pitch().x - 16 },
                  { translateX: place(index()).x + (carried() === page ? moved().x : 0), translateY: place(index()).y + (carried() === page ? moved().y : 0) },
                ]}
              >
                <Box
                  role="button"
                  aria-label={`Page ${index() + 1}`}
                  {...drag(page, index())}
                  style={[looks.miniature, { width: MINIATURE, height: (MINIATURE * 16) / 9, scale: dragging() === page ? 1.05 : 1 }]}
                >
                  <Box style={{ position: "absolute", left: (MINIATURE - THUMB.width) / 2, top: ((MINIATURE * 16) / 9 - THUMB.height) / 2, scale, opacity: page.hidden ? 0.4 : 1, pointerEvents: "none" }}>
                    <HomePage owner={props.owner} index={props.index} page={index()} placed={(pack(page.items.map((item, at): Cell => ({ item, index: at })), (cell) => spans(cell.item))[0] ?? [])} laid={THUMB} scale={scale} small />
                  </Box>
                  <Show when={page.hidden}>
                    <Minus label={`Delete page ${index() + 1}`} left={6} top={6} onPress={() => remove(index())} />
                  </Show>
                </Box>
                <Box
                  role="checkbox"
                  aria-checked={page.hidden ? "false" : "true"}
                  aria-label={`Show page ${index() + 1}`}
                  onClick={() => home.send({ type: "hide", page: index(), hidden: !page.hidden })}
                  style={{ width: 28, height: 28, alignItems: "center", justifyContent: "center", borderRadius: "full", borderWidth: 1, borderColor: page.hidden ? "muted-foreground" : "foreground", backgroundColor: page.hidden ? undefined : "foreground", color: "background" }}
                >
                  <Show when={!page.hidden}>
                    <Icon name="check" />
                  </Show>
                </Box>
              </Box>
            )}
          </For>
        </Box>
      </Box>
    </Box>
  )
}

/// A window onto a row for each owner, which rolls as the owners move up and down, `at` in owners, each row `height`
/// tall: the row nearest is laid out, and so says how wide the window is, and the others are drawn above and below
/// it, fading as they roll away, as a picker's do.
function Roll(props: { count: number; height: number; row: (index: number) => Element }) {
  const nearest = () => Math.max(0, Math.min(props.count - 1, Math.round(rolled())))
  return (
    <Box style={{ position: "relative", height: props.height, overflow: "hidden", alignItems: "center" }}>
      <Box aria-hidden="true" style={{ height: props.height, opacity: 0, justifyContent: "center" }}>
        {props.row(nearest())}
      </Box>
      <For each={Array.from({ length: props.count }, (_, index) => index)}>
        {(index) => (
          <Box style={{ position: "absolute", left: 0, right: 0, top: (index - rolled()) * props.height, height: props.height, alignItems: "center", justifyContent: "center", opacity: Math.max(0, 1 - Math.abs(index - rolled()) * 1.5) }}>
            {props.row(index)}
          </Box>
        )}
      </For>
    </Box>
  )
}

/// An owner's page dots, a dot a screenful, the one shown brighter; tapping one goes there, and sliding along them
/// moves a screenful every 28 points, as an iPhone's do. While their pages are edited, a tap opens the page overview.
/// Only the owner shown's take a finger.
function Dots(props: { owner: Owner; index: number; count: number; size: number }) {
  const current = () => (seen().owner === props.index ? seen().screen : (state().context.last[props.index] ?? 0))
  let slid: { x: number; from: number; sliding: boolean; id: number } | undefined
  let quietDots = 0
  const slide = {
    onPointerDown: (event: Pressed) => {
      if (editingOwner(props.index) || seen().owner !== props.index) return
      slid = { x: event.clientX, from: current(), sliding: false, id: event.pointerId }
    },
    onPointerMove: (event: Pressed) => {
      if (!slid || event.pointerId !== slid.id) return
      const across = event.clientX - slid.x
      if (!slid.sliding && Math.abs(across) < 10) return
      slid.sliding = true
      const screen = Math.max(0, Math.min(props.count, slid.from + Math.trunc(across / 28)))
      if (screen === current()) return
      home.send({ type: "go", screen })
      haptic("selection")
    },
    onPointerUp: () => {
      if (slid?.sliding) quietDots = Date.now() + 300
      slid = undefined
    },
    onPointerCancel: () => (slid = undefined),
  }
  const press = (screen: number) => () => {
    if (seen().owner !== props.index) return
    if (editingOwner(props.index)) return home.send({ type: "overview" })
    if (Date.now() > quietDots) home.send({ type: "go", screen })
  }
  return (
    <Box role="navigation" aria-label={`${props.owner.name}'s pages`} aria-hidden={seen().owner !== props.index ? "true" : undefined} {...slide} style={{ flexDirection: "row", alignItems: "center", height: "100%" }}>
      <For each={Array.from({ length: Math.max(1, props.count) }, (_, at) => at)}>
        {(screen) => (
          <Box role="button" aria-label={`Page ${screen + 1} of ${props.count}`} aria-current={current() === screen ? "true" : undefined} onClick={press(screen)} style={{ height: "100%", justifyContent: "center", paddingInline: 1 }}>
            <Box style={{ width: props.size, height: props.size, marginInline: props.size * 0.64, borderRadius: "full", backgroundColor: current() === screen ? "foreground" : "muted-foreground", opacity: current() === screen ? 1 : 0.4 }} />
          </Box>
        )}
      </For>
    </Box>
  )
}

/// The owner pill, at the top of Home in the middle: whose pages they are, "You ⌄", which opens the list of every
/// owner, the current one ticked. Its name rolls as the owners move. While pages are edited, + and Done are either side
/// of it, and it opens nothing. In the launcher it's at the end of its search bar.
/// Whose pages these are, and the menu that chooses whose: in a pill, or in the launcher's bar a small chip, as
/// Raycast's is.
function OwnerPill(props: { owners: Owner[]; chip?: boolean }) {
  const at = () => state().context.at.owner
  return (
    <Menu.Root
      open={state().matches("listing")}
      onOpenChange={({ open }) => (open ? state().matches("nothing") && home.send({ type: "list" }) : state().matches("listing") && home.send({ type: "dismiss" }))}
      onSelect={({ value }) => home.send({ type: "pick", owner: Number(value) })}
    >
      <Menu.Trigger variant={props.chip ? "ghost" : "secondary"} aria-label={props.owners[at()]?.name}>
        <Box style={{ flexDirection: "row", alignItems: "center", gap: props.chip ? 8 : 5, color: props.chip ? "muted-foreground" : undefined }}>
          <Show when={props.chip}>
            <Image src="home.svg" alt="" style={{ width: 16, height: 16, borderRadius: "sm" }} />
          </Show>
          <Roll count={props.owners.length} height={BAR - 8} row={(index) => <Text style={{ font: props.chip ? "footnote" : "label", lineClamp: 1, maxWidth: 144 }}>{props.owners[index]?.name}</Text>} />
          <Box style={{ color: "muted-foreground", scale: 0.75 }}>
            <Icon name="caret-down" />
          </Box>
        </Box>
      </Menu.Trigger>
      <Menu.Positioner>
        <Menu.Content>
          <For each={props.owners}>
            {(owner, index) => (
              <Menu.Item value={String(index())}>
                <Box style={{ flexDirection: "row", alignItems: "center", gap: 8 }}>
                  <Box style={{ opacity: index() === at() ? 1 : 0 }}>
                    <Icon name="check" />
                  </Box>
                  <Text>{owner.name}</Text>
                </Box>
              </Menu.Item>
            )}
          </For>
        </Menu.Content>
      </Menu.Positioner>
    </Menu.Root>
  )
}

/// The launcher's search bar, across its top, as Raycast's is: the search, bare and large, which keeps the keyboard,
/// and the owner chip. With nothing typed, the arrow keys move the highlight over Home (`move`), Tab goes to the App
/// Library and back, Return opens what's highlighted and Escape puts the launcher away; with something typed, up and
/// down choose among the results (`Results`), Return opens the one chosen, and Escape empties it. ⌘K, or Ctrl+K on a
/// PC, opens Actions either way.
function LauncherBar(props: { owners: Owner[]; found: () => Listed[]; move: (key: string) => boolean; open: () => void; tab: () => void; library: boolean }) {
  const keys = (event: Keyed) => {
    if ((mac() ? event.metaKey : event.ctrlKey) && event.key.toLowerCase() === "k") {
      event.preventDefault()
      setActing(true)
      return
    }
    if (event.altKey || event.metaKey || event.ctrlKey) return
    const typed = !!query().trim()
    if (event.key === "Enter") {
      event.preventDefault()
      props.open()
    } else if (event.key === "Escape") {
      // With something typed, the field empties itself.
      if (!query()) void leave()
    } else if (!typed && event.key === "Tab") {
      event.preventDefault()
      props.tab()
    } else if (!typed) {
      if (props.move(event.key)) event.preventDefault()
    } else if (event.key === "ArrowDown" || event.key === "ArrowUp") {
      event.preventDefault()
      setChosen(Math.max(0, Math.min(props.found().length - 1, chosen() + (event.key === "ArrowDown" ? 1 : -1))))
    }
  }
  // The search has the keyboard as the launcher's summoned: it's drawn again each time it's shown.
  return (
    <Box style={{ position: "absolute", top: 0, left: 0, right: 0, height: LAUNCHER_BAR, zIndex: 6, flexDirection: "row", alignItems: "center", gap: 12, paddingInline: 16 }}>
      <Box style={{ flexGrow: 1 }}>
        <Show when={visible()}>
          <EditableText type="search" action="search" placeholder="Search apps…" aria-label="Search apps" autofocus value={query()} onKeyDown={keys} onInput={(event) => (setQuery(event.currentTarget.value), setChosen(0))} style={looks.bare} />
        </Show>
      </Box>
      <Show when={!query().trim() && !props.library}>
        <OwnerPill owners={props.owners} chip />
      </Show>
    </Box>
  )
}

/// What the launcher's search found, in place of Home's pages, as Raycast's root search lists it: under Results, a
/// row for each, its icon, its name, the app it's part of where that's another name, and what it is at its end; the
/// first chosen for Return. The pointer chooses one too, and a click opens it.
function Results(props: { owner: Owner; found: () => Listed[] }) {
  const from = (event: Located): From => {
    const at = event.currentTarget.getBoundingClientRect()
    return { x: at.x + 8, y: at.y + (at.height - 22) / 2, width: 22, height: 22, radius: 5 }
  }
  return (
    <Box style={{ position: "absolute", top: LAUNCHER_BAR, left: 0, right: 0, bottom: LAUNCHER_FOOT, zIndex: 5, overflow: "scroll", paddingInline: LAUNCHER_SIDE }}>
      <Show when={props.found().length} fallback={<Box style={{ marginBlock: 32 }}><Empty>No apps match.</Empty></Box>}>
        <Box style={{ height: LAUNCHER_HEADING, paddingTop: 12, paddingInline: LAUNCHER_SIDE }} aria-hidden="true">
          <Text style={{ font: "caption-2", color: "muted-foreground" }}>Results</Text>
        </Box>
        <Box role="listbox" aria-label="Results">
          <For each={props.found()} keyed={(listed) => listed.entrypoint ?? listed.app}>
            {(listed, at) => (
              <Box
                role="option"
                aria-label={listed().name}
                aria-selected={at() === chosen() ? "true" : "false"}
                onPointerMove={() => setChosen(at())}
                onClick={(event) => press(listed(), props.owner, false, from(event))}
                style={[looks.result, at() === chosen() && looks.picked]}
              >
                <Image src={src(listed().icon)} alt="" style={{ width: 22, height: 22, flexShrink: 0, borderRadius: "sm" }} />
                <Text style={{ font: "footnote", lineClamp: 1, flexShrink: 0, maxWidth: "60%" }}>{listed().name}</Text>
                <Show when={listed().title !== listed().name}>
                  <Text style={{ font: "footnote", color: "muted-foreground", lineClamp: 1, flexShrink: 1 }}>{listed().title}</Text>
                </Show>
                <Box style={{ flexGrow: 1 }} />
                <Text style={{ font: "footnote", color: "muted-foreground", flexShrink: 0 }}>App</Text>
              </Box>
            )}
          </For>
        </Box>
      </Show>
    </Box>
  )
}

/// What the launcher's Return opens, and its Actions are for: an app it found, or what's chosen on Home, with where it
/// is on Home, if it's there, and where it's drawn.
type Target = { item?: Item; listed?: Listed; place?: { page: number; index: number }; from?: From }
/// The name a target's opened by: an app's, or for a widget, its app's.
const targetName = (target: Target) => target.listed?.name ?? (target.item && (opener(target.item)?.name ?? target.item.name))
/// A key, as the footer and the Actions menu show it: in a small keycap, as Raycast's are.
const Key = (props: { children: Element }) => (
  <Box style={[looks.key, { height: 18 }]}>
    <Text style={{ font: "caption-2", color: "muted-foreground" }}>{props.children}</Text>
  </Box>
)

/// The launcher's footer, as Raycast's is: two pills floating over its foot. On the left, the Home Screen's icon and
/// name, and on Home its page dots and the App Library's glyph; on the right, what Return does, and Actions, ⌘K, a
/// menu of what can be done with it: Open, Add to or Remove from Home Screen, Show in App Library and its App Store
/// Page.
function LauncherFoot(props: { owners: Owner[]; laid: Kind[][]; target: () => Target | undefined; open: () => void; act: (action: string) => void }) {
  const owner = () => props.owners[state().context.at.owner]
  const store = () => props.owners[0]?.store
  const app = () => {
    const target = props.target()
    return target?.listed?.app ?? (target?.item && opener(target.item)?.app)
  }
  const item = () => props.target()?.item ?? (props.target()?.listed && opens(props.target()!.listed!))
  return (
    <Box role="toolbar" style={{ position: "absolute", bottom: 10, left: 10, right: 10, height: 36, zIndex: 6, flexDirection: "row", alignItems: "center", justifyContent: "space-between", pointerEvents: "none", opacity: state().matches("editing") ? 0 : 1 }}>
      <Box style={[looks.floating, { height: 36 }]}>
        <Image src="home.svg" alt="" style={{ width: 18, height: 18, borderRadius: "sm" }} />
        <Text style={{ font: "caption-1", color: "muted-foreground" }}>Home Screen</Text>
        <Show when={!query().trim()}>
          <DotsPill owners={props.owners} laid={props.laid} flat />
        </Show>
      </Box>
      <Show when={props.target()}>
        {(target) => (
          <Box style={[looks.floating, { height: 36 }]}>
            <Box role="button" onClick={props.open} style={{ flexDirection: "row", alignItems: "center", gap: 8, height: "100%" }}>
              <Text style={{ font: "caption-1", color: "foreground" }}>{query().trim() ? "Open App" : `Open ${targetName(target())}`}</Text>
              <Key>↵</Key>
            </Box>
            <Menu.Root
              open={acting()}
              onOpenChange={({ open }) => setActing(open)}
              onSelect={({ value }) => (setActing(false), props.act(value))}
            >
              <Menu.Trigger variant="ghost" aria-label={`Actions, ${mac() ? "⌘" : "Ctrl"} K`}>
                <Box style={{ flexDirection: "row", alignItems: "center", gap: 4 }}>
                  <Text style={{ font: "caption-1", color: "muted-foreground" }}>Actions</Text>
                  <Key>{mac() ? "⌘" : "Ctrl"}</Key>
                  <Key>K</Key>
                </Box>
              </Menu.Trigger>
              <Menu.Positioner>
                <Menu.Content>
                  <Menu.Item value="open">{`Open ${targetName(target())}`}</Menu.Item>
                  <Show when={target().place && owner()?.arranges}>
                    <Menu.Item value="remove">Remove from Home Screen</Menu.Item>
                  </Show>
                  <Show when={!target().place && item() && addable(item()!, owner())}>
                    <Menu.Item value="add">{state().context.at.owner ? `Add to ${owner().name}'s Home Screen` : "Add to Home Screen"}</Menu.Item>
                  </Show>
                  <Show when={item() && opener(item()!)}>
                    <Menu.Item value="library">Show in App Library</Menu.Item>
                  </Show>
                  <Show when={store() && app()}>
                    <Menu.Item value="store">App Store Page</Menu.Item>
                  </Show>
                </Menu.Content>
              </Menu.Positioner>
            </Menu.Root>
          </Box>
        )}
      </Show>
    </Box>
  )
}

/// The dots pill, at the bottom in the middle, just clear of the home indicator: the dots of the owner shown, then the
/// App Library's glyph, which says there's one more page and goes there, lit while it's shown, as the dots are. It
/// stays where it is as the owners move, and on the App Library too, its dots rolling to the next owner's and taking
/// their number. While pages are edited the glyph stays, dimmed, and goes nowhere. In the launcher's footer it's
/// `flat`: no pill of its own.
function DotsPill(props: { owners: Owner[]; laid: Kind[][]; flat?: boolean }) {
  const count = (owner: number) => (props.laid[owner] ?? []).filter((kind) => kind === "home").length
  const library = () => props.laid[seen().owner]?.[seen().screen] === "library"
  const go = () => {
    if (state().matches("editing")) return
    const screen = props.laid[state().context.at.owner]?.indexOf("library") ?? -1
    if (screen >= 0) home.send({ type: "go", screen })
  }
  const height = () => (props.flat ? 36 : DOTS)
  return (
    // iOS 26's Liquid Glass, the pages blurred through it with a rim of light and a soft shadow, but in the launcher's
    // footer, which is flat.
    <Box style={[looks.pill, !props.flat && looks.glass, { height: height() }]}>
      <Roll count={props.owners.length} height={height()} row={(index) => <Dots owner={props.owners[index]} index={index} count={count(index)} size={props.flat ? 5 : 7} />} />
      <Box
        role="button"
        aria-label="App Library"
        aria-current={library() ? "true" : undefined}
        aria-disabled={state().matches("editing") ? "true" : undefined}
        onClick={go}
        style={{ height: "100%", justifyContent: "center", paddingLeft: 2, paddingRight: 4, color: library() ? "foreground" : "muted-foreground", opacity: state().matches("editing") ? 0.35 : library() ? 1 : 0.4 }}
      >
        <Box style={{ scale: 0.75 }}>
          <Icon name="squares-four" />
        </Box>
      </Box>
    </Box>
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
      <Box style={{ alignItems: "center", gap: 12 }}>
        <Box style={{ flexDirection: "row", flexWrap: "wrap", justifyContent: "center", gap: 12 }}>
          <For each={missing()} keyed={({ item }) => item.entrypoint}>
            {(app) => <Image src={src(app().item.icon)} alt="" style={{ width: 64, height: 64, borderRadius: "xl" }} />}
          </For>
        </Box>
        <Text style={{ color: "muted-foreground", textAlign: "center" }}>{reach()}</Text>
        <Box style={{ alignSelf: "stretch", gap: 12, paddingTop: 8 }}>
          <Button onClick={install}>Install</Button>
          <Button variant="secondary" onClick={() => home.send({ type: "cancel" })}>Cancel</Button>
        </Box>
      </Box>
    </Sheet>
  )
}

/// An app dropped on an owner whose pages are someone else's to change: a sheet says so, and who can, and it goes back.
function Refused(props: { owner: Owner; held: Held }) {
  const title = () => and(carriedApps(props.held).map(({ title }) => title))
  const who = createMemo(async () => {
    const [{ users }, { people }] = await Promise.all([
      json<{ users: string[] }>("permissions_users", { artifact: props.owner.subject, relation: "owner" }),
      json<{ people: { who: string; called?: string }[] }>("sign-in_everyone"),
    ])
    const called = (who: string) => people.find((person) => person.who === who)?.called ?? who.replace(/^users\//, "")
    return new Intl.ListFormat("en", { type: "disjunction" }).format(users.map(called))
  })
  const dismiss = () => state().matches({ editing: "refused" }) && home.send({ type: "dismiss" })
  return (
    <Sheet heading={`You can't add apps for ${props.owner.name}`} open={state().matches({ editing: "refused" })} onClose={dismiss}>
      <Box style={{ alignItems: "center", gap: 12 }}>
        <Errored fallback={(error) => <Problem heading="Can't show who can" text={error() as Error} />}>
          <Loading fallback={<Text> </Text>}>
            <Text style={{ color: "muted-foreground", textAlign: "center" }}>{who() ? `Only ${who()} can. Ask them to add ${title()}.` : `Ask one of its owners to add ${title()}.`}</Text>
          </Loading>
        </Errored>
        <Box style={{ alignSelf: "stretch", paddingTop: 8 }}>
          <Button onClick={dismiss}>OK</Button>
        </Box>
      </Box>
    </Sheet>
  )
}

/// While pages are edited, + and Done float at the top, either side of the owner pill, over the pages, which stay
/// where they are, as on a phone; in the launcher, in its footer's place, each in Liquid Glass. They fade in, and out
/// as editing ends.
function EditBar(props: { going?: boolean }) {
  const fade = tween(200, () => {})
  createEffect(
    () => !!props.going,
    (going) => fade.to(going ? 0 : 1),
  )
  const top = () => (launcher() ? viewport().height - LAUNCHER_FOOT : room().safe.top)
  const height = () => (launcher() ? LAUNCHER_FOOT : TOP_ROW)
  const width = () => (room().wide && !launcher() ? Math.min(room().width - 2 * room().side, room().fit * (laidPage().width * room().scale + room().gap) - room().gap) : room().width)
  return (
    <Box style={{ position: "absolute", zIndex: 7, top: top(), left: (room().width - width()) / 2, width: width(), height: height(), flexDirection: "row", alignItems: "center", justifyContent: "space-between", paddingInline: launcher() ? 10 : 16, opacity: fade.value(), pointerEvents: "none" }}>
      <Box style={[looks.pill, looks.glass]}>
        <Button variant="ghost" size="icon" aria-label="Add Widget" onClick={() => state().matches({ editing: "still" }) && home.send({ type: "gallery" })}>
          <Icon name="plus" />
        </Button>
      </Box>
      <Box style={[looks.pill, looks.glass]}>
        <Button variant="ghost" onClick={() => home.send({ type: "done" })}>Done</Button>
      </Box>
    </Box>
  )
}

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
  // The room the pages have: the window, less the launcher's bars.
  createEffect(
    () => ({ size: viewport(), safe: safeArea(), launcher: launcher() }),
    ({ size, safe, launcher }) => void setRoom(measure(size.width, size.height - (launcher ? LAUNCHER_BAR + LAUNCHER_HEADING + LAUNCHER_FOOT : 0), launcher, safe)),
  )
  const sectionsTop = () => (launcher() ? LAUNCHER_BAR + LAUNCHER_HEADING : 0)

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
    (across[owner] ??= spring((x) => {
      xOf(owner)[1](x)
      look()
      rested(across[owner])
    }))
  /// What's carried, once the pages under it come to rest.
  let settledUnder: (() => void) | undefined
  const rested = (one: Spring | undefined) => one && one.value === one.target && !one.velocity && settledUnder?.()
  /// The owner and screenful nearest where they are, which the pills follow.
  const look = () => {
    const owner = shownOwner()
    setSeen({ owner, screen: Math.round(sideways(owner).value / pitch()) })
  }
  /// How far the sections are moved past Home, to the App Library: the owner shown's way along, past their Home.
  const sectionsX = () => {
    const owner = Math.max(0, Math.min(props.owners.length - 1, Math.round(downY() / Math.max(1, room().height))))
    return Math.max(0, xOf(owner)[0]() - Math.max(0, homes(owner).length - 1) * room().width)
  }

  /// Where a page of an owner is drawn in the window: its left and top, and how much larger it's drawn.
  const pageAt = (owner: number, page: number) => {
    const fit = untrack(room).fit
    const screen = Math.floor(page / fit)
    const count = untrack(screens)[owner]?.[screen]?.count ?? 1
    const left = lefts(untrack(room), untrack(laidPage), count)[page - screen * fit] ?? 0
    return { x: screen * pitch() - Math.min(sideways(owner).value, homeEnd(owner)) + left, y: sectionsTop() + owner * height() - down.value, scale: untrack(room).scale }
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
  // Put away, the launcher forgets what was typed, and comes back to Home's first page.
  createEffect(
    () => launcher() && !visible(),
    (hidden) => {
      if (!hidden) return
      setQuery("")
      setChosen(0)
      setActing(false)
      if (home.getSnapshot().matches("nothing") && home.getSnapshot().context.at.screen !== 0) home.send({ type: "go", screen: 0 })
      choose(first(ordered(home.getSnapshot().context.at)))
    },
    { defer: true },
  )
  /// What the launcher's search found: the apps of your App Library, whose names have what's typed in them.
  const found = createMemo(() => findIn(props.owners[0]?.library ?? [], query(), false))
  /// How tall the launcher's content is, as Raycast's and Spotlight's panels are: its bars, and between them the rows
  /// the Home pages shown use, or what its search found; the App Library and Home while it's edited, six rows' worth.
  /// Its client makes its window that tall (`sizeChanged`), as an MCP App's host fits its frame to what it says.
  const tall = () => {
    const bars = LAUNCHER_BAR + LAUNCHER_HEADING + LAUNCHER_FOOT
    if (query().trim()) return bars + (found().length ? found().length * 38 : 82)
    const { at } = state().context
    const laid = laidPage()
    const full = kindOf(at.owner) === "library" || state().matches("editing")
    if (full) return bars + (laid.top + gridHeight(laid) + laid.bottom) * room().scale
    const screen = screens()[at.owner]?.[at.screen]
    const shown = screen ? pages()[at.owner]?.slice(screen.from, screen.from + screen.count) ?? [] : []
    const bottom = Math.max(0, ...shown.flatMap((page) => page.map((placed) => cellOf(laid, placed)).map((box) => box.y + box.height)))
    return bars + ((bottom || laid.top + 120) + laid.bottom) * room().scale
  }
  let told = 0
  createEffect(
    // Never shorter than Actions' menu needs over the footer, as Raycast's panel has room for it.
    () => launcher() && Math.ceil(Math.max(tall(), LAUNCHER_BAR + 300)),
    (height) => {
      if (height && height !== told) sizeChanged({ height: (told = height) })
    },
  )
  /// What the launcher's Return opens and its Actions are for: the app its search chose, or what's highlighted on Home.
  const target = createMemo((): Target | undefined => {
    if (query().trim()) {
      const listed = found()[chosen()]
      const item = listed && opens(listed)
      return listed && { listed, item, place: item && placeOf(props.owners[state().context.at.owner], item) }
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
  /// Does what was picked from the launcher's Actions.
  const act = (action: string) => {
    const now = untrack(target)
    const { at, screens } = home.getSnapshot().context
    const [who, item] = [owned()[at.owner], now?.item]
    const app = item && opener(item)
    if (!now) return
    if (action === "open") openTarget()
    else if (action === "remove" && now.place && item) void removeFromHome(who, at.owner, now.place.page, now.place.index, item)
    else if (action === "add" && item) addToHome(who, at.owner, item)
    else if (action === "store" && app && owned()[0]?.store) open(`${owned()[0].store}#app=${encodeURIComponent(app.app)}`)
    else if (action === "library" && app && home.getSnapshot().matches("nothing")) {
      // The App Library, with the app's category open over it.
      const library = screens[at.owner]?.indexOf("library") ?? -1
      setQuery("")
      if (at.screen !== library) before = at.screen
      home.send({ type: "go", screen: library })
      if (sorted(owned()[0]?.library ?? [], false).some((category) => category.category === app.category)) home.send({ type: "open", category: app.category })
    }
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
      home.send({ type: "step", ...to })
      const now = home.getSnapshot().context.at
      if (now.owner === at.owner && now.screen === at.screen) return
      const cells = ordered(now)
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
  /// In the launcher, Tab goes to the App Library, and from there back to the Home screenful it came from.
  let before = 0
  const tab = () => {
    const { at, screens } = home.getSnapshot().context
    const library = screens[at.owner]?.indexOf("library") ?? -1
    if (!home.getSnapshot().matches("nothing") || library < 0) return
    if (at.screen === library) return home.send({ type: "go", screen: Math.min(before, library - 1) })
    before = at.screen
    home.send({ type: "go", screen: library })
  }

  /// Moves the pages to `at`, as the machine says: gliding, or at once where they're only laid out again.
  const slide = (at: At, smooth: boolean) => {
    const move = smooth ? glide : jump
    move(down, at.owner * height())
    move(sideways(at.owner), at.screen * pitch())
  }

  // A finger on the pages moves them, as a phone's: once it has moved far enough to say which way, along an owner's
  // screenfuls or between owners, the pages follow it one to one, giving at the ends; let go, they go to the nearest,
  // or on the way they were flicked. A mouse doesn't drag them: a trackpad or a wheel pages (`wheel`).
  let swipe: { id: number; x: number; y: number; axis?: "x" | "y"; owner: number; from: number; base: number; moves: { time: number; value: number }[] } | undefined
  const carrying = () => home.getSnapshot().matches({ editing: "dragging" })
  /// Whether a second finger is swiping the pages under what the first carries.
  const swiping = () => !!swipe?.axis && swipe.id !== carrier
  /// How many screenfuls a swipe across has: Home's, while something's carried, which stays on them.
  const reach = (owner: number) => (carrying() ? untrack(() => homes(owner).length) || 1 : stops(owner))
  const swipes = {
    onPointerDown: (event: Pressed) => {
      pointer = { x: event.clientX, y: event.clientY }
      if (event.pointerType === "mouse" || event.button) return
      // A category shrinking back into its place lets the pages under it have the finger, and while something's
      // carried, every finger but the one carrying it has them, one at a time.
      const second = carrying() && event.pointerId !== carrier && !swipe
      if (!second && !home.getSnapshot().matches("nothing") && !home.getSnapshot().matches({ category: "closing" }) && !home.getSnapshot().matches({ editing: "still" })) return
      const { owner } = home.getSnapshot().context.at
      stop(down)
      stop(sideways(owner))
      swipe = { id: event.pointerId, x: event.clientX, y: event.clientY, owner, from: 0, base: 0, moves: [] }
    },
    onPointerMove: (event: Pressed) => {
      pointer = { x: event.clientX, y: event.clientY }
      if (!swipe || event.pointerId !== swipe.id) return
      if (!swipe.axis) {
        const [dx, dy] = [event.clientX - swipe.x, event.clientY - swipe.y]
        if (Math.hypot(dx, dy) < 10) return
        const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y"
        // A hold that began has the finger, and up and down is the page's own where it scrolls, and nobody's while
        // editing, but a second finger's while something's carried: an app, which goes to another owner, not a widget.
        const free = home.getSnapshot().matches("nothing") || home.getSnapshot().matches({ category: "closing" }) || home.getSnapshot().matches({ editing: "still" }) || (carrying() && swipe.id !== carrier)
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
      swipe.moves.push({ time: event.timeStamp, value })
      if (swipe.moves.length > 32) swipe.moves.shift()
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
      if (carrying() && done.id !== carrier && !cancelled) stack(event.clientX, event.clientY)
      return
    }
    quiet = Date.now() + 350
    const velocity = cancelled ? 0 : speed(done.moves, event.timeStamp)
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
    quiet = Date.now() + 350
    home.send({ type: "stack", page: under.page, index: under.index })
    if ((home.getSnapshot().context.held?.stack?.length ?? 0) > before) haptic("selection")
  }

  // A trackpad or a wheel: one gesture is one screenful or one owner at most, however hard it's flicked, as macOS's
  // Launchpad and an iPad with a trackpad page. A gesture is a run of wheel events: its momentum follows the fingers
  // with no pause until it dies away, and is part of it. It ends once the events stop for a moment, or turn back. It
  // steps once its pushes along its axis add up to 30 points, and the rest of it is let be. Up and down over the App
  // Library, which scrolls itself, is its own.
  const QUIET = 120
  let gesture: { axis: "x" | "y"; sign: number; total: number; done: boolean } | undefined
  let lastWheel = -Infinity
  const wheel = (event: Wheeled) => {
    if (!home.getSnapshot().matches("nothing")) return
    const still = event.timeStamp - lastWheel > QUIET
    lastWheel = event.timeStamp
    const along = (axis: "x" | "y") => (axis === "x" ? event.deltaX : event.deltaY)
    const back = !!gesture?.sign && Math.sign(along(gesture.axis)) === -gesture.sign && Math.abs(along(gesture.axis)) > 2
    if (!gesture || still || back) gesture = { axis: Math.abs(event.deltaX) > Math.abs(event.deltaY) ? "x" : "y", sign: 0, total: 0, done: false }
    gesture.sign ||= Math.sign(along(gesture.axis))
    if (gesture.done) return
    gesture.total += along(gesture.axis)
    if (Math.abs(gesture.total) < 30) return
    gesture.done = true
    const by = Math.sign(gesture.total) as 1 | -1
    home.send(gesture.axis === "y" ? { type: "step", owner: by } : { type: "step", screen: by })
  }

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
      const rect = held.rect ?? { x: pointer.x - 30, y: pointer.y - 40, width: 60, height: 80 }
      setCopy({ held, rect, x: pointer.x - grab.x, y: pointer.y - grab.y, settling: false, lifted: 0 })
      const lifting = tween(200, () => {})
      lifting.to(1)
      createEffect(lifting.value, (lifted) => void setCopy((now) => now && { ...now, lifted }))
      haptic("medium")
      const id = carrier
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
      const move = (event: Pressed) => {
        if (event.pointerId !== id) return
        pointer = { x: event.clientX, y: event.clientY }
        setCopy((now) => now && { ...now, x: event.clientX - grab.x, y: event.clientY - grab.y })
        moves.push({ time: event.timeStamp, x: event.clientX, y: event.clientY })
        if (moves.length > 32) moves.shift()
        const [x, y] = [event.clientX, event.clientY]
        const top = sectionsTop()
        const now = x < 28 ? "left" : x > pitch() - 28 ? "right" : y < top + 36 ? "up" : y > top + height() - 36 ? "down" : ""
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
        const under = pageUnder(x - grab.x + rect.x, y - grab.y + rect.y) ?? pageUnder(x, y)
        if (!under || !editing || !held?.item) return
        const scale = untrack(room).scale
        const corner = pageAt(under.owner, under.page)
        const [left, top] = [(x - grab.x + rect.x - corner.x) / scale, (y - grab.y + rect.y - corner.y) / scale]
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
      settledUnder = () => !side && over(pointer.x, pointer.y)
      const drop = (event: Pressed) => {
        if (event.pointerId !== id) return
        left = home.getSnapshot().context.held ?? left
        if (event.timeStamp) thrown = { x: speed(moves.map(({ time, x }) => ({ time, value: x })), event.timeStamp), y: speed(moves.map(({ time, y }) => ({ time, value: y })), event.timeStamp) }
        const { held, at } = home.getSnapshot().context
        const owner = untrack(owners)![at.owner]
        const items = held?.item ? [held.item, ...(held.stack ?? [])] : []
        const to: Drop = !owner.arranges ? "refuse" : items.every((item) => item.widget || installedFor(item as Entrypoint, owner)) ? "place" : "ask"
        home.send({ type: "drop", to })
      }
      carry = { move, drop }
      return () => {
        carry = undefined
        clearTimeout(edge)
        clearTimeout(resting)
        settledUnder = undefined
        // A second finger still swiping lets go of the pages: they go back to where what was carried was put down.
        if (swipe && swipe.id !== id) {
          swipe = undefined
          slide(home.getSnapshot().context.at, true)
        }
        quiet = Date.now() + 400
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
  const library = () => kindOf(state().context.at.owner) === "library"
  return (
    <Box style={{ position: "relative", width: room().width, height: viewport().height, overflow: "hidden", color: "foreground" }}>
      <Box
        {...swipes}
        onWheel={wheel}
        style={{ position: "absolute", top: sectionsTop(), left: 0, width: room().width * 2, height: room().height, flexDirection: "row", translateX: -sectionsX(), opacity: launcher() && query().trim() ? 0 : 1 }}
      >
        <Box role="main" aria-label="Home" style={{ position: "relative", width: room().width, height: room().height, overflow: "hidden", flexShrink: 0 }}>
          <Box style={{ position: "absolute", top: 0, left: 0, width: room().width, translateY: -downY() }}>
            <For each={props.owners} keyed={(owner) => owner.key}>
              {(owner, index) => (
                <Box role="region" aria-label={owner().name} style={{ position: "relative", width: room().width, height: room().height, overflow: "hidden" }}>
                  <Box style={{ position: "absolute", top: 0, left: 0, flexDirection: "row", translateX: -Math.min(xOf(index())[0](), Math.max(0, homes(index()).length - 1) * room().width) }}>
                    <For each={homes(index())} keyed={false}>
                      {(screen) => (
                        <Box style={{ position: "relative", width: room().width, height: room().height }}>
                          <For each={(pages()[index()] ?? []).slice(screen().from, screen().from + screen().count)} keyed={false}>
                            {(placed, at) => (
                              <Box
                                style={{
                                  position: "absolute",
                                  top: (laidPage().height * (room().scale - 1)) / 2,
                                  left: lefts(room(), laidPage(), screen().count)[at] + (laidPage().width * (room().scale - 1)) / 2,
                                  scale: room().scale,
                                  ...(launcher() && at ? ({ borderLeftWidth: 1, borderColor: "border" } satisfies Style) : {}),
                                }}
                              >
                                <HomePage owner={owner()} index={index()} page={screen().from + at} placed={placed()} laid={laidPage()} scale={room().scale} />
                              </Box>
                            )}
                          </For>
                        </Box>
                      )}
                    </For>
                  </Box>
                </Box>
              )}
            </For>
          </Box>
        </Box>
        <Library owner={props.owners[0]} />
      </Box>
      <Show
        when={launcher()}
        fallback={
          <>
            <Show when={!library()}>
              <Box style={{ position: "absolute", zIndex: 6, top: room().safe.top, left: 0, right: 0, height: TOP_ROW, alignItems: "center", justifyContent: "center", pointerEvents: "none" }}>
                <Box style={{ pointerEvents: "auto", opacity: state().matches("editing") ? 0.6 : 1 }}>
                  <OwnerPill owners={props.owners} />
                </Box>
              </Box>
            </Show>
            <Box style={{ position: "absolute", zIndex: 6, bottom: room().safe.bottom + 16, left: 0, right: 0, alignItems: "center", pointerEvents: "none" }}>
              <Box style={{ pointerEvents: "auto" }}>
                <DotsPill owners={props.owners} laid={laid().screens} />
              </Box>
            </Box>
          </>
        }
      >
        <LauncherBar owners={props.owners} found={found} move={move} open={openTarget} tab={tab} library={library()} />
        <Show when={!query().trim()}>
          <Box aria-hidden="true" style={[looks.heading, { top: LAUNCHER_BAR, height: LAUNCHER_HEADING }]}>
            <Text style={{ font: "caption-2", color: "muted-foreground" }}>{library() ? "App Library" : "Home"}</Text>
          </Box>
        </Show>
        <Show when={query().trim()}>
          <Results owner={props.owners[0]} found={found} />
        </Show>
        <LauncherFoot owners={props.owners} laid={laid().screens} target={target} open={openTarget} act={act} />
      </Show>
      <Show when={launcher()}>
        <Box aria-hidden="true" style={[looks.edge, mac() && looks.round]} />
      </Show>
      <Show when={editBar()}>
        <EditBar going={editBarGoing()} />
      </Show>
      <Show when={copy()}>
        {(now) => {
          const scale = () => untrack(room).scale
          const widget = () => !!now().held.item?.widget
          const [width, height] = [now().rect.width / scale(), now().rect.height / scale()]
          const lift = () => (now().settling ? 1 : 1 + (widget() ? 0.05 : 0.12) * now().lifted)
          const tile = () => laidPage().tile
          return (
            <Box
              aria-hidden="true"
              style={{
                position: "absolute",
                zIndex: 30,
                display: "grid",
                left: now().rect.x + (now().rect.width - width) / 2,
                top: now().rect.y + (now().rect.height - height) / 2,
                width,
                height,
                translateX: now().x,
                translateY: now().y,
                scale: scale() * lift(),
                pointerEvents: "none",
              }}
            >
              <Box aria-hidden="true" style={[looks.lifted, widget() ? looks.liftedWidget : looks.liftedIcon, now().settling && { opacity: 0 }, widget() ? { top: 0, start: 0, width, height } : { top: 0, start: (width - tile()) / 2, width: tile(), height: tile() }]} />
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
            </Box>
          )
        }}
      </Show>
      <Show when={opened()}>{(opened) => <OpenCategory owner={props.owners[0]} opened={opened()} />}</Show>
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
    </Box>
  )
}

/// The Home Screen didn't draw: what DESIGN.md says of a launcher that doesn't, with Try Again, which asks for it
/// again, and what went wrong under it. Its client loads it again by itself once a newer version is out, as the older
/// one meeting what's new is the likeliest cause.
function Broken(props: { error: Error }) {
  return (
    <Box style={{ height: viewport().height, justifyContent: "center", padding: 20 }}>
      <Problem
        heading="Home Screen didn't open."
        text={new Error(`Try again in a moment.\n\n${typeof props.error?.cause === "string" ? props.error.cause : (props.error?.message ?? props.error)}`)}
        action={<Button variant="secondary" onClick={() => refresh(owners)}>Try Again</Button>}
      />
    </Box>
  )
}

render(() => (
  <Errored fallback={(error) => <Broken error={error() as Error} />}>
    <Loading>
      <HomeScreen owners={owners()} />
    </Loading>
  </Errored>
))
