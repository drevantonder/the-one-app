// The window's own page: the frames apps are drawn in, and the bridge between each and the client.
//
// An app is a page of its app's, in a frame at an address of its own, so it reaches no other app and nothing
// here. It speaks to this page as an MCP App speaks to its host: JSON-RPC messages, posted. What it asks goes to
// where the page runs (`host.js`): the app's Rust half, or in a browser, the network's edge (`clients/web/host.js`),
// which answers what only reads from what it kept and says when that has changed.
//
// The launcher is an app too, which reaches this client from a server as every app's does: the app in the category
// `launcher`, which a network has exactly one of. It's the one this page shows first, under every other, and it
// opens the rest (`one/open`).
import { address, asksToReload, haptic, invoke, listen, openLink, sandbox, setTheme } from "./host.js"
/** @import { Context, Moved, Paged, Rect, Shown, State, View } from "./host.js" */

/// Reports what went wrong here to Telemetry, through the client, which keeps it until a server can be told: what
/// this page showed its person, an error of its own or of an app's page, an app that never drew. Names, files and
/// what was shown only, never what anyone typed.
/** @param {string} name @param {string} error @param {Record<string, unknown>} [attributes] */
const report = (name, error, attributes = {}) => invoke("report", { record: { name, error, attributes } }).catch(() => {})
/// An error as OpenTelemetry's `exception` says one: its message, and the file and line it was thrown at, by its
/// path alone, which leaves out anything a query had in it.
/** @param {unknown} message @param {unknown} [file] @param {unknown} [line] @param {string} [app] */
function exception(message, file, line, app) {
  const path = typeof file === "string" && URL.canParse(file) ? new URL(file).pathname : undefined
  report("exception", String(message).slice(0, 500), { "code.file.path": path, "code.line.number": typeof line === "number" ? line : undefined, "one.app": app })
}
addEventListener("error", ({ message, filename, lineno }) => exception(message, filename, lineno))
addEventListener("unhandledrejection", ({ reason }) => exception(reason?.message ?? reason))
/// How long an app may take from its tap to drawing before it's reported as never having opened.
const NEVER = 15000
/** An element of the page's own, which is there. @param {string} id */
const byId = (id) => /** @type {HTMLElement} */ (document.getElementById(id))
/// The launcher's page, as this user's apps list it, once they're listed: `<app>/<page>`. And the App Store's,
/// which installs one where there's none: the app in the category `app-store`, if there is one.
let launcher = ""
let appStore = ""
/// Where an app's settings are: the app in the category `settings`, if there is one, which a link to an app with no
/// page of its own opens at that app.
let settings = ""
/// On a computer, each app opens in a window of its own, as the computer's own app (`windows`, `hosts.rs`): the
/// Home Screen's window shows the launcher alone, and an app's own window shows that app (`own`), each where the
/// launcher is in a phone's. Either opens every other app in its own window.
let windows = false
/// On Android, the runtime version of the renderer the app draws natively with, over this page (`runtimeVersion` in
/// src/lib.rs): an app whose page is needs that version or an older one opens there, any other in a frame
/// here. 0 where it doesn't.
let runtimeVersion = 0
/// The runtime version each app's native page needs, by its entrypoint, as its `runtimeVersion` says, and the apps
/// that opened in a frame here as their native window couldn't.
/** @type {Map<string, number>} */
let needs = new Map()
const framed = new Set()
/// On a phone, the launcher is drawn natively, as its root window, under every app's (`open_launcher`), where it can
/// be; otherwise it's in a frame here, as an app that can't be is.
let nativeLauncher = false
/// Whether the native launcher was last told it shows, which it doesn't while an app in a frame here is in front, and
/// whether something is open over its page, which back shuts first.
let launcherShown = true
let launcherOver = false
/** @type {{ entrypoint: string, subject?: string, name?: string | null } | undefined} */
let own
/// The page under every other in this window: the launcher's, or in an app's own window, that app's.
const base = () => own?.entrypoint ?? launcher
/// What the page under every other is called where the window says it's opening it, or that it didn't open.
const baseName = () => (own ? names.get(own.entrypoint) || "This app" : "Home Screen")
/// A computer's launcher (`computer.rs`): this page in a panel the system summons, whose app is the launcher alone.
/// It's put away as an app opens from it, on Escape, and once the launcher is done with it (`leave`).
const panel = () => windows && !own
const baseKey = () => key(base(), own?.subject)
/// The apps that are open, by who each is for and what it is: `{ key, entrypoint, subject, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
/** @type {Map<string, View>} */
const open = new Map()
/** @param {string} entrypoint @param {string} [subject] */
const key = (entrypoint, subject) => `${subject ?? ""}|${entrypoint}`
/// The pages in an app's frames (the UI library's `Frame`), each another app's, answered as its own app's page is,
/// by the number its address has (`frame`): known by its window once it has said it from a frame of the page it's in.
/// (`framed`, below, is something else: Android's apps drawn in this page's frames rather than natively.)
/** @type {Map<string, View & { in: View, window?: Window }>} */
const nested = new Map()
/// Each app's first app, by the app's name, among this user's: what a notification from it, or a `one:` link to it, opens.
/** @type {Map<string, string>} */
let first = new Map()
/// Every file of every app listed, by `<app>/<path>`: what a link to a page can open.
/** @type {Set<string>} */
let files = new Set()
/// Each app's name and icon, by its page, as this user's apps list them.
/** @type {Map<string, string>} */
let names = new Map()
/** @type {Map<string, string>} */
let icons = new Map()
/// Who this client has listed files for: its user (`undefined`), and each group or the network an app was opened
/// for.
/** @type {Set<string | undefined>} */
const listed = new Set()
/// The apps each of those lists says are trusted, by who it's for, as each was last listed. A page of any other app,
/// one installed untrusted or not listed yet, asks only what its app's server code may, behind its sandbox, for
/// whoever it was opened for, and changes nothing of this device (`respond`).
/** @type {Map<string | undefined, Promise<Set<string>>>} */
const trusted = new Map()

/// What an app's address is named by: its app's name, then who it's for (its subject) if that isn't its user, last part first.
/** @param {string} entrypoint @param {string} [subject] */
const host = (entrypoint, subject) => [entrypoint.split("/")[0], ...(subject ? subject.split("/").reverse() : [])].join(".")

/// A phone's app is its network's, which its owner builds and updates, and a computer's updates itself: what
/// `start` says.
let phone = false
/// Whether this is an iPhone's app from TestFlight, whose Update opens TestFlight.
let testflight = false

/// Light or dark, as Colour Mode has it on this device, or as the device is.
const scheme = matchMedia("(prefers-color-scheme: dark)")
const chosen = () => /** @type {"light" | "dark" | null} */ (localStorage.getItem("theme"))
const theme = () => chosen() ?? (scheme.matches ? "dark" : "light")
function dress() {
  document.documentElement.style.colorScheme = chosen() ?? ""
  setTheme(chosen())
  for (const name of ["light", "dark"]) document.documentElement.classList.toggle(name, theme() === name)
  for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { theme: theme() } })
}
scheme.addEventListener("change", dress)
/// Colour Mode is the device's: on a computer each app's window has a page of its own, so each is told as it's chosen
/// in any of them, and told what was chosen as it opens.
/** @param {unknown} theme */
function heardTheme(theme) {
  if (theme === undefined || theme === chosen()) return
  if (theme === "light" || theme === "dark") localStorage.setItem("theme", theme)
  else localStorage.removeItem("theme")
  dress()
}
listen("theme", ({ payload }) => heardTheme(payload))
// Where the window goes from here, a sign-in page, isn't told Colour Mode: the bars follow the device there, as it does.
addEventListener("pagehide", () => setTheme(null))

/// How far in from each edge of the window what the device keeps for itself reaches (an iPhone's status bar, home
/// indicator and notch), which only this page is told: Android's window is already clear of its own.
const safeArea = Object.assign(document.createElement("div"), { id: "safe-area" })
document.body.append(safeArea)
/// What of that an app's frame reaches, as MCP Apps tell an app (`safeAreaInsets`): its screen keeps clear of it,
/// and what scrolls in it passes under it.
/** @param {HTMLIFrameElement} frame */
function insets(frame) {
  const safe = getComputedStyle(safeArea)
  // Where it's laid out, not where a move has it now: an app that loads as it slides in is told where it ends up.
  const at = { top: frame.offsetTop, right: frame.offsetLeft + frame.offsetWidth, bottom: frame.offsetTop + frame.offsetHeight, left: frame.offsetLeft }
  const reach = (/** @type {string} */ edge, /** @type {number} */ from) => Math.max(0, Math.round(parseFloat(safe.getPropertyValue(`padding-${edge}`)) - from))
  return { top: reach("top", at.top), right: reach("right", innerWidth - at.right), bottom: reach("bottom", innerHeight - at.bottom), left: reach("left", at.left) }
}
/// Each app is told again as the window turns or its frame moves.
function placed() {
  for (const view of open.values())
    post(view, { method: "ui/notifications/host-context-changed", params: { safeAreaInsets: insets(view.frame), ...titleRow() } })
}
addEventListener("resize", placed)
// The banner moves every frame down as it comes in.
byId("banner").addEventListener("animationend", placed)

/// An iPhone's keyboard covers the window rather than making it smaller, as Android's does: while it's up, the
/// window's apps end above it, so a sheet sits on it and what's typed in can be scrolled to. Pinched in, the window
/// is as it was.
function keyboard() {
  const { height = innerHeight, scale = 1 } = visualViewport ?? {}
  document.body.style.height = scale === 1 && height < innerHeight ? `${height}px` : ""
  placed()
}
visualViewport?.addEventListener("resize", keyboard)

const { assign, createActor, enqueueActions, fromCallback, fromPromise, sendParent, sendTo, setup, spawnChild } = XState

/// Each open app's page, one state at a time, as its person's owner signed it off, a child of the window's
/// (`machine`), which starts it as the app opens and tells it whether it's in front of its person: loading, then
/// current until its app or the UI library is updated under it, which makes it stale. A stale page loads again by
/// itself only at a boundary, as an iPhone updates an app while it's closed and a web app loads its new version on the
/// next navigation: once it's off screen (another app over it, the window hidden, or the window showing something of
/// its own over every app, as Update Required), or as it's opened again. Nothing is reloaded under its person: in the
/// app, which updates itself as a desktop app and a phone's do, it loads again too once its window isn't in use, as
/// another has the keyboard (`unused`), and in a browser it says so quietly, "Updated · Reload", and loads again once
/// its person presses Reload (`asksToReload`). One that's showing an
/// error loads again at once, as its old version meeting what's new is likely why, and there's nothing of theirs under
/// it. A page that doesn't draw in 15 s on screen has failed, with Try Again, until it draws after all, as a page
/// loaded again behind another app may only once it's shown, when it's current. A page has drawn as a browser's Largest
/// Contentful Paint says a page is shown: once it has rendered and what it first asked for is answered (`render` in the
/// UI library tells, `notifications/one/drawn`), not at its first answered request, so a page that stays blank or on its
/// skeleton hasn't, and one that shows its problem as a read failed has. Only time it's on screen counts: while
/// it's off screen, the 15 s wait (`away`), and go on from where they were once it's in front again (`seen`). It tells
/// the window as it draws and as it fails, and the window follows each of its moves (`syncSnapshot`), which it says
/// what the page needs of its person by. One that hasn't drawn loads again as it's tried again (`retry`), as once its
/// network talks with this client again, which refused what it asked meanwhile. The launcher's page is one too. Its
/// frame is busy (`aria-busy`) while it loads.
/// While a page is loading, as its parts: on screen, where the 15 s it has to draw in run, or not, where they wait.
/** @type {any} */
const waiting = {
  initial: "looking",
  states: {
    looking: { always: [{ guard: "away", target: "away" }, { target: "seen" }] },
    seen: {
      entry: "counts",
      exit: "stops",
      on: { away: "away" },
      after: { never: { target: "#page.failed", actions: assign({ why: ({ context }) => `${context.view.entrypoint} didn't draw in ${NEVER / 1000} s.` }) } },
    },
    away: { on: { seen: { guard: "front", target: "seen" }, opened: "seen" } },
  },
}
const pageMachine = setup({
  types: /** @type {{ context: { view: View, why?: string, errored?: boolean, waited: number, since: number }, input: View, events: Paged }} */ ({}),
  // What's left of the 15 s a page has to draw in, on screen.
  delays: { never: ({ context }) => Math.max(0, NEVER - context.waited) },
  guards: {
    away: ({ context }) => !onScreen(context.view),
    front: ({ context }) => onScreen(context.view),
    unused: () => !asksToReload && !document.hasFocus(),
    errored: ({ context }) => context.errored === true,
  },
  actions: {
    load: ({ context }) => reloadPage(context.view),
    // Loaded again, it's shown no error yet.
    clear: assign({ errored: false }),
    failed: ({ context }) => {
      const { view, why } = context
      // How long it took to open is told only of an open that ended with it drawn.
      view.tapped = undefined
      report("one.client.open", why ?? "", { "one.view": view.entrypoint.split("/")[0] })
    },
    // The window hears it drew, or didn't: a launcher that never draws leaves it nothing to show.
    drawn: sendParent(({ context }) => ({ type: "drawn", key: context.view.key })),
    tellFailed: sendParent(({ context }) => ({ type: "failed", key: context.view.key, why: context.why ?? "" })),
    // The wait for it to draw starts afresh as it's loaded, counts while it's on screen, and stops while it isn't.
    unwaited: assign({ waited: 0 }),
    counts: assign({ since: () => performance.now() }),
    stops: assign({ waited: ({ context }) => context.waited + performance.now() - context.since }),
    busy: ({ context }) => context.view.frame.setAttribute("aria-busy", "true"),
    idle: ({ context }) => context.view.frame.removeAttribute("aria-busy"),
  },
}).createMachine({
  id: "page",
  initial: "loading",
  context: ({ input }) => ({ view: input, waited: 0, since: 0 }),
  // What its page logs as an error, as an `<Errored>` boundary that's drawn its problem does, is what it shows: there's
  // nothing of its person's to keep there.
  on: { errored: { actions: assign({ errored: true }) } },
  states: {
    // Loaded against what's gone before it's even drawn, as a kept launcher is when the list that's come since
    // names a newer one, it loads again at once: there's nothing of its person's on it yet.
    loading: {
      entry: ["unwaited", "busy"],
      exit: "idle",
      on: { drawn: "current", stale: "reloading", retry: "reloading" },
      ...waiting,
    },
    // Tried again, as the window opens again, it says again that it's drawn.
    current: {
      entry: "drawn",
      on: { stale: "stale", retry: { actions: "drawn" } },
    },
    stale: {
      always: [
        { guard: "away", target: "reloading" },
        { guard: "unused", target: "reloading" },
        { guard: "errored", target: "reloading" },
      ],
      on: { away: "reloading", unused: { guard: "unused", target: "reloading" }, opened: "reloading", reload: "reloading", errored: "reloading" },
    },
    reloading: {
      entry: ["clear", "load", "unwaited", "busy"],
      exit: "idle",
      on: { drawn: "current", stale: { target: "reloading", reenter: true }, retry: { target: "reloading", reenter: true } },
      ...waiting,
    },
    failed: {
      entry: ["failed", "tellFailed"],
      on: { drawn: "current", retry: { target: "loading", actions: ["clear", "load"] } },
    },
  },
})

/// What each of a move's two frames does over it, as keyframes from its start to its end, and how long it takes: an
/// app opened from its icon or widget (`icon`) grows out of it as an iPhone's does, showing itself over it in its first
/// moments, as the launcher sinks back under it; going back, it shrinks back into it as the launcher comes forward, and
/// fades into it as it gets there. With no icon, it slides in over what was there from the end of the line, which moves
/// back a quarter, and going back, back out. Pulled home by the rail's line (`pulled`), as iOS's swipe home is, its
/// first part (to `HELD`) follows the finger in, its right edge under it, shrinking to a card over the Home Screen,
/// dimmed; let go, it goes on into its icon, or with none, shrinks away. For whoever asked for less motion, what comes
/// fades in over what was there. The one drawn over the other is `upper`, with `corner` its corners' radius while it
/// moves: a phone's, which its screen's own hide while it fills it, as an iPhone's app is. What moves is only moved,
/// grown and faded, which a browser draws without drawing the app again, as Core Animation does a layer: a clip that
/// changed as it moved had WebKit draw the app again each frame, half of them late.
/** @param {View} leaving @param {View} coming @param {boolean} back @param {Rect} [icon] @param {boolean} [pulled] @returns {{ upper: View, ms: number, corner?: number, parts: [View, Keyframe[]][] }} */
function moves(leaving, coming, back, icon, pulled) {
  const style = getComputedStyle(document.documentElement)
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return { upper: coming, ms: parseFloat(style.getPropertyValue("--move-settle")), parts: [[coming, [{ opacity: 0 }, { opacity: 1 }]]] }
  const [app, under] = back ? [leaving, coming] : [coming, leaving]
  const { x, y, width, height } = app.frame.getBoundingClientRect()
  // The app in its icon: its middle on the icon's, as large as fits in it.
  const small = icon && {
    translate: `${icon.x + icon.width / 2 - (x + width / 2)}px ${icon.y + icon.height / 2 - (y + height / 2)}px`,
    scale: `${Math.min(icon.width / width, icon.height / height)}`,
  }
  const whole = { scale: "1", translate: "0px 0px" }
  if (pulled) {
    // The card at `HELD`: its right edge as far in as the finger, `HELD` of the width.
    const held = { scale: `${CARD}`, translate: `${(width / 2) * (1 - CARD) - HELD * width}px 0px`, offset: HELD }
    const gone = small ?? { scale: "0.5", translate: `${-0.3 * width}px 0px` }
    return {
      upper: app,
      ms: 400,
      corner: CORNER / CARD,
      parts: [
        [app, [whole, held, gone]],
        [app, [{ opacity: 1 }, { opacity: 1, offset: small ? HELD + (1 - HELD) * FADES : HELD }, { opacity: 0 }]],
        // The Home Screen behind it dimmed at first, as iOS's is, so the card stands out from it: dimmed, not sunk,
        // so no undimmed edge shows round it.
        [under, [{ filter: "brightness(0.7)" }, { filter: "brightness(1)" }]],
      ],
    }
  }
  if (small) {
    const sunk = { scale: "0.92" }
    const up = { scale: "1" }
    return {
      upper: app,
      ms: 400,
      corner: CORNER / CARD,
      parts: back
        ? [[app, [whole, small]], [app, [{ opacity: 1 }, { opacity: 1, offset: FADES }, { opacity: 0 }]], [under, [sunk, up]]]
        : [[app, [small, whole]], [app, [{ opacity: 0 }, { opacity: 1, offset: 1 - FADES }, { opacity: 1 }]], [under, [up, sunk]]],
    }
  }
  const end = style.direction === "rtl" ? -1 : 1
  const at = (/** @type {number} */ share) => ({ translate: `${share * end}% 0px` })
  return {
    upper: app,
    ms: parseFloat(style.getPropertyValue("--move-travel")),
    parts: back ? [[app, [at(0), at(100)]], [under, [at(-25), at(0)]]] : [[app, [at(100), at(0)]], [under, [at(0), at(-25)]]],
  }
}

/// How far through the pull home its first part goes, where the app is a card (`moves`), how small, and its corners.
const HELD = 0.4
const CARD = 0.8
const CORNER = 40
/// How far into its icon an app has gone as it starts to fade into it, and out of it as it has shown itself.
const FADES = 0.75

/// Takes `body.p` to `body.target` as a critically damped spring, as the Home Screen's pages have
/// (apps/home-screen/client/pager.ts), from as fast as it's going (`velocity`), drawing each frame (`seek`) until it's
/// still there (`settled`). Its `stiffness` is set so that from still it lands as the ease it replaces did, in the
/// move's time: within 3 % of it all the way. → what stops it.
/** @param {{ p: number, target: number, velocity: number, stiffness: number }} body @param {(p: number) => void} seek @param {() => void} settled */
function spring(body, seek, settled) {
  let last = performance.now()
  /** @param {number} now */
  const step = (now) => {
    // A frame late, as when the window was hidden, is taken as one, so nothing jumps.
    const dt = Math.max(0, Math.min(1 / 30, (now - last) / 1000))
    last = now
    let { p, velocity } = body
    for (let at = 0; at < dt; at += 1 / 240) {
      const slice = Math.min(1 / 240, dt - at)
      velocity += (-body.stiffness * (p - body.target) - 2 * Math.sqrt(body.stiffness) * velocity) * slice
      p += velocity * slice
    }
    const still = Math.abs(p - body.target) < 0.001 && Math.abs(velocity) < 0.05
    body.velocity = still ? 0 : velocity
    seek(still ? body.target : p)
    if (still) settled()
    else frame = requestAnimationFrame(step)
  }
  let frame = requestAnimationFrame(step)
  return () => cancelAnimationFrame(frame)
}
/// How stiff a spring is that lands in `ms` from still.
/** @param {number} ms */
const stiffness = (ms) => (27 * 400 / ms) ** 2

/// The window's moves between its launcher and its apps, and from app to app, one at a time: a child of the window's
/// (`machine`), which tells it where to go only while it shows apps (tagged `pages`), and to stop where it was going
/// (`freeze`) as it covers them all. The Home Screen is never told: it's a frame like any other here.
///
/// The launcher and every open app are live frames, so a move animates the two frames themselves (Web Animations),
/// both live and taking taps all the while: a tap on either is never lost. Its animations (`moves`) are paused, and
/// one progress `p`, 0 where it came from and 1 where it goes, sets where they are; a spring takes `p` to where it's
/// going, so turning back, or a finger letting go, carries on from where it is, as fast as it was going.
///
///                     show(to): its history entry is there already
///     ┌───────┐ ─────────────────────────────────► ┌───────────┐ ─┐ show(where it came from): turns back, p → 0
///     │ still │                                    │  playing  │ ◄┘ show(where it goes): nothing new
///     └───────┘ ◄── settled: layer(where p is) ─── │ from → to │ ─┐ show(anything else): ends where it was going,
///         │         and notes()                    └───────────┘ ◄┘   then moves from there to it
///         │                                         │       ▲
///         │                                   grab: │       │ release: p → 1 if far or flicked, else → 0,
///         │                                  caught │       │ or where the device's gesture ended; the
///         │                                         │       │ window's history follows it
///         │                              where p is ▼       │
///         │   grab [back leads somewhere: an   ┌───────────┐
///         │   app at its first screen, with    │ tracking  │  drag(p): drawn there
///         └──────── nothing over it] ────────► └───────────┘
///             (with `home`, an app at any of its screens)
///
///     freeze, from any: still, where it was going, which is always where the window's history is.
///     show, while tracking: as while playing, the finger keeping it unless it goes somewhere else.
///     popped(to), drawn already by the device: still at `to`, with nothing drawn; while playing toward it, nothing.
///
/// A computer's trackpad swipe sends `grab`, `drag` and `release` (`wheel`), as do Android's back gesture
/// (`backStarted`), an iPhone's swipe in from the edge (`window.grab`), and a finger pulling the rail's line, whose
/// `grab` says `home`: from any of an app's screens, it goes back to the Home Screen's entry, past the app's own.
/** @typedef {{ from: View, to: View, upper: View, parts: Animation[], p: number, target: number, velocity: number, stiffness: number }} Move */
/** @param {Move} move */
const toward = (move) => (move.target === 1 ? move.to : move.from)
/** @param {Move} move @param {number} p */
function seek(move, p) {
  move.p = p
  for (const part of move.parts) part.currentTime = p * 1000
}
const moveMachine = setup({
  types: /** @type {{ context: { move?: Move }, events: Shown }} */ ({}),
  actors: {
    spring: fromCallback(/** @param {{ input: Move, sendBack: (event: Shown) => void }} spring */ ({ input: move, sendBack }) => spring(move, (p) => seek(move, p), () => sendBack({ type: "settled" }))),
  },
  guards: {
    here: ({ event }) => "to" in event && event.to === front,
    there: ({ context, event }) => "to" in event && !!context.move && event.to === toward(context.move),
    returning: ({ context, event }) => "to" in event && !!context.move && event.to === (context.move.target === 1 ? context.move.from : context.move.to),
    // The line takes an app home from any of its screens, as long as nothing is drawn over it; back only from its first.
    backs: ({ event }) => ("home" in event && event.home ? depth > 0 && !front?.layer && !covering() : home()),
  },
  actions: {
    // The frame coming is seen and takes taps from the start, over or under the one going, and its page is told
    // it's opened.
    begin: assign(({ event }) => {
      const pulled = event.type === "grab" && event.home === true
      // Pulled home, it goes into the icon it came out of only where it came from the Home Screen.
      const grabbed = () => (pulled ? { to: showing(null), icon: depth === 1 ? stack[1].from : undefined } : { to: showing(stack[depth - 1].key), icon: stack[depth].from })
      const show = /** @type {Extract<Shown, { type: "show" }>} */ (event.type === "grab" ? { ...grabbed(), back: true, tapped: performance.now() } : event)
      const from = /** @type {View} */ (front)
      const { upper, ms, corner, parts } = moves(from, show.to, show.back, show.icon, pulled)
      show.to.frame.inert = false
      upper.frame.dataset.move = ""
      upper.frame.style.borderRadius = corner ? `${corner}px` : ""
      const animations = parts.map(([view, keyframes]) => {
        const part = view.frame.animate(keyframes, { duration: 1000, fill: "both" })
        part.pause()
        return part
      })
      reveal(show.to, show.tapped)
      /** @type {Move} */
      // It goes where the window's history is: a finger that takes it from still hasn't gone anywhere yet.
      const move = { from, to: show.to, upper, parts: animations, p: 0, target: event.type === "grab" ? 0 : 1, velocity: 0, stiffness: stiffness(ms) }
      seek(move, 0)
      return { move }
    }),
    turn: ({ context }) => {
      const move = /** @type {Move} */ (context.move)
      move.target = 1 - move.target
    },
    // Ends where it was going, or with `popped`, where the device went.
    end: assign(({ context, event }) => {
      const move = context.move
      const to = event.type === "popped" ? event.to : move && toward(move)
      for (const part of move?.parts ?? []) part.cancel()
      if (move) {
        delete move.upper.frame.dataset.move
        move.upper.frame.style.borderRadius = ""
      }
      if (to) layer(to.key)
      notes()
      return { move: undefined }
    }),
    // Shown without a move: where it is already, or drawn by the device.
    place: ({ event }) => {
      if (!("to" in event)) return
      layer(event.to.key)
      reveal(event.to, event.tapped)
      notes()
    },
    // Let go far enough or fast enough, it carries on, and otherwise goes back; the window's history follows it.
    release: ({ context, event }) => {
      const move = /** @type {Move} */ (context.move)
      const { velocity, end } = /** @type {Extract<Shown, { type: "release" }>} */ (event)
      move.target = end ?? (Math.abs(velocity) > 1 ? (velocity > 0 ? 1 : 0) : Math.round(move.p))
      move.velocity = velocity
      const to = toward(move)
      if (move.target === 1 && depth > 0 && to === showing(null)) backHome()
      else for (const step of [-1, 1]) if (stack[depth + step] && showing(stack[depth + step].key) === to) history.go(step)
    },
  },
}).createMachine({
  id: "move",
  initial: "still",
  context: {},
  on: { freeze: { target: ".still", actions: "end" } },
  states: {
    still: {
      on: {
        show: [{ guard: "here", actions: "place" }, { target: "playing", actions: "begin" }],
        popped: { actions: "place" },
        grab: { guard: "backs", target: "tracking", actions: "begin" },
      },
    },
    playing: {
      invoke: { src: "spring", input: ({ context }) => /** @type {Move} */ (context.move) },
      on: {
        show: [{ guard: "there" }, { guard: "returning", actions: "turn" }, { target: "playing", reenter: true, actions: ["end", "begin"] }],
        popped: [{ guard: "there" }, { target: "still", actions: "end" }],
        settled: { target: "still", actions: "end" },
        grab: "tracking",
      },
    },
    tracking: {
      on: {
        show: [{ guard: "there" }, { guard: "returning", actions: "turn" }, { target: "playing", actions: ["end", "begin"] }],
        drag: { actions: ({ context, event }) => seek(/** @type {Move} */ (context.move), event.p) },
        release: { target: "playing", actions: "release" },
        popped: { target: "still", actions: "end" },
      },
    },
  },
})

/// What the window shows, one state at a time: while it opens, until it can start the launcher; the launcher, once
/// it has drawn; signing in, where the device isn't in its network; why there's no launcher to show, with Apps to
/// install one; and an update, which is all it shows once a server has refused this client for being behind it
/// (`Connection::Behind` in core/client), as an app that requires an update does. Only `on` moves it, by the moves `machine`
/// has, and a move that isn't there is refused. A link to an app (`one:`) is kept until the launcher has drawn, and
/// then opened over it. While it opens or updates, the window is busy (`aria-busy`). Whether its network answers is
/// the client's (`Connection` in core/client), which the window follows and never decides: it says it as it opens,
/// and as a banner over the launcher.
///
/// Each open app's page is its child (`pageMachine`), which it starts as the app is loaded (`load`) and passes
/// what's said of it (`page`). It alone owns the screen: in the states that show apps (tagged `pages`), the page in
/// front is on screen (`seen`); in every other, whatever it shows covers them all, so every page is off screen
/// (`away`), and says nothing over it.
///
/// Refused for being behind, it needs updating, and is asked again whether it is (`checking`) as its person presses
/// Try Again, as its network answers again, and as the window comes back on screen, as an iPhone app checks as it
/// comes back to the foreground: a server updated or rolled back since may talk with it again. A computer's app
/// updates itself; a phone's is its network's, which its owner updates. An update that leaves this app running, as a
/// device that's also a server takes its build without starting again, is checked the same way, and so is one that
/// failed, as its person tries again: what it said before is never shown as its network's answer now.
/// The window's children that are pages, by their apps' keys: not what it invokes, as an update.
/** @param {import("xstate").AnyActorRef} window @returns {string[]} */
const pages = (window) => Object.keys(window.getSnapshot()?.children ?? {}).filter((child) => open.has(child))
/** @param {Moved} event */
const loaded = (event) => /** @type {Extract<Moved, { type: "load" }>} */ (event).view
/** @param {Moved} event */
const paged = (event) => /** @type {Extract<Moved, { type: "page" }>} */ (event)
/// A finger taking the window's move, moving it and letting go, while it shows apps.
const fingers = /** @type {const} */ ({ grab: { actions: "move" }, drag: { actions: "move" }, release: { actions: "move" } })
const machine = setup({
  types: /** @type {{ context: Context, events: Moved }} */ ({}),
  actors: {
    page: pageMachine,
    move: moveMachine,
    // A computer's app takes the latest builds and starts again on them; one that's also a server says what it did.
    update: fromPromise(() => invoke("update")),
    // Asks its network again, as itself, which a server refuses where this client is still behind it.
    retry: fromPromise(() => invoke("retry")),
  },
  guards: {
    signIn: ({ event }) => event.type === "session" && event.told.session.state === "signIn",
    online: ({ event }) => event.type === "connection" && event.connection === "online",
    waiting: ({ context }) => !context.starting,
    updateReady: ({ context }) => context.updateReady,
    behind: ({ context }) => context.behind === true,
    // A phone's app is its network's, which its owner builds and updates, unless it came from TestFlight, which
    // updates it.
    updates: () => !phone || testflight,
    appStore: () => Boolean(appStore),
    launcher: ({ event }) => (event.type === "drawn" || event.type === "failed") && event.key === baseKey(),
  },
  actions: {
    told: assign({ told: ({ event, context }) => (event.type === "session" ? event.told : context.told) }),
    connection: assign(({ event }) => {
      if (event.type !== "connection") return {}
      for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { connection: event.connection } })
      return { connection: event.connection }
    }),
    updateReady: assign(({ event, context }) => {
      if (event.type !== "updateReady" || event.ready === context.updateReady) return {}
      return { updateReady: event.ready }
    }),
    // Its apps are listed once it's told where its device is, unless it's to sign in.
    list: ({ context }) => void (context.told && context.told.session.state !== "signIn" && list()),
    start: () => start(),
    // An app's own window leaves signing in to the Home Screen's, which closes it once the device is in.
    signIn: ({ context }) => void (own ? invoke("front") : context.told?.link && location.replace(context.told.link)),
    // The device hears its person's notifications once they see their home, and a phone looks again at whether it
    // may show them each time its home is back in front.
    listen: () => void invoke("listen"),
    openAppStore: () => openAppStore(),
    link: assign({ link: ({ event, context }) => (event.type === "link" ? event.link : context.link) }),
    follow: assign({ link: ({ context }) => void (context.link && go(context.link)) }),
    // An app's page starts as its app is loaded, and is told what's said of it.
    load: spawnChild("page", { id: ({ event }) => loaded(event).key, input: ({ event }) => loaded(event), syncSnapshot: true }),
    page: sendTo(({ event }) => paged(event).key, ({ event }) => paged(event).paged),
    // Every page is told whether it might be on screen, and each works out whether it's the one in front.
    away: enqueueActions(({ enqueue, self }) => {
      for (const child of pages(self)) enqueue.sendTo(child, { type: "away" })
    }),
    // Every page is told the window may not be in use, and each that's stale and works it out loads again.
    unused: enqueueActions(({ enqueue, self }) => {
      for (const child of pages(self)) enqueue.sendTo(child, { type: "unused" })
    }),
    // A move goes on only while the window shows apps: otherwise what it shows covers them, and the move stops where
    // it was going, and what's shown changes at once.
    move: sendTo("move", ({ event }) => event),
    place: sendTo("move", ({ event }) => ({ type: "popped", to: /** @type {Extract<Moved, { type: "show" }>} */ (event).to, tapped: /** @type {Extract<Moved, { type: "show" }>} */ (event).tapped })),
    freeze: sendTo("move", { type: "freeze" }),
    // Talked with again, every page that couldn't load meanwhile loads again.
    retry: enqueueActions(({ enqueue, self }) => {
      for (const child of pages(self)) enqueue.sendTo(child, { type: "retry" })
    }),
    seen: enqueueActions(({ enqueue, self }) => {
      for (const child of pages(self)) enqueue.sendTo(child, { type: "seen" })
    }),
  },
}).createMachine({
  id: "window",
  initial: "opening",
  context: { connection: "connecting", starting: false, updateReady: false },
  entry: spawnChild("move", { id: "move" }),
  on: {
    unnative: { target: ".native_failed", actions: assign({ why: ({ event }) => event.why }) },
    show: { actions: "place" },
    popped: { actions: "move" },
    session: { actions: "told" },
    connection: { actions: "connection" },
    updateReady: { actions: "updateReady" },
    link: { actions: "link" },
    load: { actions: "load" },
    page: { actions: "page" },
    // The window hidden, every page is off screen; shown again, the one in front is on screen, where it shows apps.
    away: { actions: "away" },
    unused: { actions: "unused" },
    seen: { actions: "seen" },
  },
  states: {
    // Waits until it can start the launcher: its device in its network and its network answering, or a launcher
    // kept from before.
    opening: {
      tags: "pages",
      entry: [assign({ starting: false }), "list", "seen"],
      on: {
        show: { actions: "move" },
        ...fingers,
        session: [{ guard: "signIn", target: "sign_in", actions: "told" }, { actions: ["told", "list"] }],
        connection: [{ guard: "online", actions: ["connection", "list"] }, { actions: "connection" }],
        listed: { guard: "waiting", actions: [assign({ starting: true }), "start"] },
        drawn: { guard: "launcher", target: "launcher" },
        none: { target: "no_launcher", actions: assign({ only: ({ event }) => event.only }) },
        tooOld: "needs_updating",
        failed: { guard: "launcher", target: "launcher_failed", actions: assign({ why: ({ event }) => event.why }) },
        unchecked: { target: "launcher_failed", actions: assign({ why: ({ event }) => event.why }) },
      },
    },
    // The network's own sign-in page, which the client brings the window back from once its person has approved
    // it, where it opens again (`follows` in `src/lib.rs`).
    sign_in: { entry: ["signIn", "away", "freeze"] },
    launcher: {
      tags: "pages",
      entry: ["listen", "follow", "seen"],
      on: {
        show: { actions: "move" },
        ...fingers,
        update: { guard: "updateReady", target: "updating" },
        tooOld: "needs_updating",
        link: { actions: ["link", "follow"] },
        seen: { actions: ["seen", "listen"] },
      },
    },
    // A launcher installed meanwhile, as by an agent, is listed as its network says something changed.
    no_launcher: {
      entry: ["away", "freeze"],
      on: { openAppStore: { guard: "appStore", target: "app_store" }, listed: "opening", tooOld: "needs_updating" },
    },
    // The App Store, shown with no launcher under it, until a launcher is installed: it's the page in front.
    app_store: {
      tags: "pages",
      entry: ["openAppStore", "seen"],
      on: { show: { actions: "move" }, ...fingers, listed: "opening", tooOld: "needs_updating" },
    },
    // This computer's native windows couldn't start, so there's no Home Screen: it says why, and that's all.
    native_failed: {},
    // A launcher that draws after all is shown, as an app's page that does is (`pageMachine`).
    launcher_failed: {
      tags: "pages",
      entry: "seen",
      on: { show: { actions: "move" }, ...fingers, retry: "opening", drawn: { guard: "launcher", target: "launcher" }, tooOld: "needs_updating" },
    },
    // Update Required: Update on a computer, and everywhere, Try Again, as the window comes back on screen, and
    // as its network answers again.
    needs_updating: {
      entry: [assign({ behind: true }), "away", "freeze"],
      on: {
        update: { guard: "updates", target: "updating" },
        retry: "checking",
        seen: "checking",
        connection: [{ guard: "online", target: "checking", actions: "connection" }, { actions: "connection" }],
      },
    },
    // Asks its network again: where it talks with this client, the window opens again; refused, it needs updating.
    // Not answered, it opens again where it wasn't behind, and says why there (`steps`).
    checking: {
      entry: ["away", "freeze"],
      invoke: {
        src: "retry",
        onDone: { target: "opening", actions: [assign({ behind: false }), "retry"] },
        onError: [{ guard: "behind", target: "needs_updating" }, { target: "opening" }],
      },
      on: { tooOld: "needs_updating" },
    },
    // It starts again on the new build once it has updated, as an app does: a device that's also a server once the
    // program keeping its server running has taken it, which says so meanwhile. One that takes its build without
    // starting again, or whose update was under way already, carries on here, asked again whether it's behind.
    updating: {
      entry: [assign({ message: undefined }), "away", "freeze"],
      invoke: {
        src: "update",
        onDone: { target: "checking", actions: assign({ message: ({ event }) => event.output }) },
        onError: { target: "update_failed", actions: assign({ why: ({ event }) => `${event.error}` }) },
      },
    },
    // Try Again asks its network afresh, as `checking` does, never repeating what an update said before: where
    // it talks with this app now, it opens; where it doesn't, Update Required, with Update.
    update_failed: {
      entry: ["away", "freeze"],
      on: { retry: "checking" },
    },
  },
})
const shown = createActor(machine)
/** @param {Moved} event */
const on = (event) => shown.send(event)

/** @param {View} view @param {Paged} event */
const page = (view, event) => on({ type: "page", key: view.key, paged: event })
/// Where an app's page is now, as the window's child.
/** @param {View} view */
const pageOf = (view) => /** @type {import("xstate").SnapshotFrom<typeof pageMachine> | undefined} */ (shown.getSnapshot().children[view.key]?.getSnapshot())
// The window hidden, every page is off screen; shown again, the one in front is on screen.
document.addEventListener("visibilitychange", () => on({ type: document.hidden ? "away" : "seen" }))
/// Where the window's move is now.
const moving = () => /** @type {import("xstate").SnapshotFrom<typeof moveMachine> | undefined} */ (shown.getSnapshot().children.move?.getSnapshot())
/// Whether an app's page is in front of its person: the window's answer, the one showing while it shows apps, in a
/// window that's on screen.
/** @param {View} view @returns {boolean} */
const onScreen = (view) => !document.hidden && shown.getSnapshot().hasTag("pages") && view === showing(here())

/// What the window says as it opens: the steps it takes to its person's home, the first of which says whether its
/// network answers. With a launcher kept from before, it's starting that alone.
/** @param {Context} context @returns {{ steps: string[], at: number, text?: string }} */
function steps({ connection, starting }) {
  if (starting && connection !== "online") return { steps: [`Opening ${baseName()}`], at: 0 }
  const reached = connection === "offline" || connection === "reconnecting" ? "Can't reach your network" : "Connecting to your network"
  const at = starting || connection === "online" ? 1 : 0
  return { steps: [reached, `Opening ${baseName()}`], at, text: at === 0 && reached !== "Connecting to your network" ? "Check this device is online." : undefined }
}

/// What each problem says: its heading, its one sentence, what's behind it for Show Details, and the button that
/// fixes it, if there is one.
/** @param {import("xstate").SnapshotFrom<typeof machine>} now @returns {[string, string, string | undefined, string | undefined] | undefined} */
function problemOf(now) {
  if (now.matches("launcher_failed")) return [`${baseName()} didn't open.`, "Try again in a moment.", now.context.why, "Try Again"]
  if (now.matches("native_failed")) return [`${baseName()} can't open its windows on this device.`, "Its graphics didn't start.", now.context.why, undefined]
  if (now.matches("update_failed")) return ["Couldn't update", now.context.why ?? "", undefined, "Try Again"]
}
/// What a computer's launcher says while its servers say it's behind them: Update Required, with Update.
/** @type {[string, string, undefined, string]} */
const UPDATE_REQUIRED = ["Update Required", "This version of the One App no longer works with your network.", undefined, "Update"]

/// How tall the launcher's content last said it is (`ui/notifications/size-changed`), and how tall its panel was made.
let asked = 0
/** @type {number | undefined} */
let made
/// A computer's launcher panel is as tall as the launcher says while it shows it, as Raycast's and Spotlight's grow and
/// shrink to what they show, and its tallest while it shows anything else: its steps, a problem, an update.
function fit() {
  if (!panel()) return
  const height = shown.getSnapshot().matches("launcher") && asked ? asked : undefined
  if (height === made) return
  made = height
  invoke("resize", { height })
}

/// Draws what the window shows of itself, in its middle, over the frames: its mark and steps while it opens, a
/// spinner and a line while it updates, why there's no Home Screen, or a problem; and over the launcher, a banner
/// while its network doesn't answer. The launcher is the frames alone.
/** @type {string | undefined} */
let lastProblem
function draw() {
  const now = shown.getSnapshot()
  const { context } = now
  byId("state").hidden = now.matches("launcher") || now.matches("app_store") || now.matches("sign_in")
  document.body.ariaBusy = now.matches("opening") || now.matches("updating") ? "true" : null
  const opening = now.matches("opening")
  byId("mark").hidden = byId("steps").hidden = !opening
  const opened = steps(context)
  byId("reconnect").hidden = !opening || opened.text === undefined
  if (opening) {
    byId("steps").replaceChildren(
      ...opened.steps.map((step, i) => {
        const item = Object.assign(document.createElement("li"), { className: i < opened.at ? "done" : i === opened.at ? "now" : "" })
        const mark = i < opened.at ? icon(CHECK) : i === opened.at ? spinner() : Object.assign(document.createElement("span"), { className: "next" })
        const words = Object.assign(document.createElement("span"), { textContent: step })
        if (i === opened.at && opened.text) words.append(Object.assign(document.createElement("small"), { textContent: opened.text }))
        item.append(mark, words)
        return item
      }),
    )
  }
  // Behind, it's asked again under the alert; updated, under the spinner, with what the update said.
  const behind = now.matches("needs_updating") || (now.matches("checking") && context.behind === true)
  const updating = now.matches("updating") || (now.matches("checking") && !behind)
  byId("status").hidden = !updating
  byId("status-text").textContent = context.message ?? ""
  byId("status-text").hidden = !(updating && context.message)
  byId("empty").hidden = !now.matches("no_launcher")
  byId("empty-text").textContent = context.only ?? (appStore ? "" : "Ask your AI to install a Home Screen")
  byId("empty-text").hidden = !byId("empty-text").textContent
  byId("open-app-store").hidden = Boolean(context.only) || !appStore
  // Behind, a computer's launcher says so where it shows everything else, as one problem box with Update, as its app
  // updates itself (DESIGN.md). A phone's is an alert, as iOS's Update Required, which always has a way on: its app
  // is updated by its network's owner, so there it has Try Again, and Update too where it came from TestFlight, which
  // opens TestFlight.
  const required = /** @type {HTMLDialogElement} */ (byId("update-required"))
  byId("update-button").hidden = phone && !testflight
  byId("update-owner").hidden = !phone || testflight
  byId("try-again-button").hidden = !phone
  byId("try-again-button").toggleAttribute("disabled", now.matches("checking"))
  byId("try-again-button").ariaBusy = now.matches("checking") ? "true" : null
  if (behind && phone && !required.open) required.showModal()
  if (!(behind && phone) && required.open) required.close()
  const problem = behind && !phone ? UPDATE_REQUIRED : problemOf(now)
  byId("problem-button").dataset.variant = behind ? "default" : "outline"
  byId("problem-button").toggleAttribute("disabled", now.matches("checking"))
  byId("problem-button").ariaBusy = now.matches("checking") ? "true" : null
  byId("problem").hidden = !problem
  if (problem) {
    const [heading, text, details, action] = problem
    byId("problem-heading").textContent = heading
    byId("problem-text").textContent = text
    byId("problem-details").hidden = !details
    byId("problem-details-text").textContent = details ?? ""
    byId("problem-action").hidden = !action
    byId("problem-button").textContent = action ?? ""
  }
  // What's shown is reported once, as it's shown.
  const said = behind ? "Update Required: This version of the One App no longer works with your network." : problem && `${problem[0]}: ${problem[1]}`
  if (said && said !== lastProblem) report("one.client.problem", said)
  lastProblem = said
  fit()
  notes()
  rail()
  // Over the launcher: its network not answering, and trying again once it has lost it. Answering, nothing.
  const banner = now.matches("launcher") && (context.connection === "offline" || context.connection === "reconnecting")
  if (byId("banner").hidden === banner) {
    byId("banner").hidden = !banner
    placed()
  }
  byId("banner-spinner").hidden = context.connection !== "reconnecting"
  byId("banner-text").textContent = context.connection === "reconnecting" ? "Reconnecting…" : "Can't reach your network"
  byId("banner-retry").hidden = context.connection !== "offline"
}

/// The UI library's check icon and spinner, which the window draws its steps with, as the UI library does.
const CHECK = "M229.66,77.66l-128,128a8,8,0,0,1-11.32,0l-56-56a8,8,0,0,1,11.32-11.32L96,188.69,218.34,66.34a8,8,0,0,1,11.32,11.32Z"
/** @param {string} path */
function icon(path) {
  const svg = document.createElementNS("http://www.w3.org/2000/svg", "svg")
  svg.setAttribute("class", "ui-icon")
  svg.setAttribute("viewBox", "0 0 256 256")
  svg.setAttribute("aria-hidden", "true")
  const drawn = document.createElementNS("http://www.w3.org/2000/svg", "path")
  drawn.setAttribute("d", path)
  svg.append(drawn)
  return svg
}
function spinner() {
  const shown = /** @type {HTMLElement} */ (byId("banner-spinner").cloneNode(true))
  shown.removeAttribute("id")
  shown.hidden = false
  return shown
}

/// Whether this device is behind the latest build of its program for its kind of machine, as its network says
/// (`builds`): a browser, or an app whose network says nothing of its build, isn't. Throws why, where its network
/// couldn't say.
async function behind() {
  const { build, target } = await invoke("client")
  if (!build || !target) return false
  const latest = (await call("builds")).latest?.[`one/${target}`]?.build
  return Boolean(latest) && latest !== build
}

/// Calls the network's tool `name` as this device's client, and gives what it answered, or throws the error it said.
/** @param {string} name @param {Record<string, unknown>} [inputs] @returns {Promise<any>} */
async function call(name, inputs = {}) {
  const result = /** @type {{ structuredContent?: unknown, content: { text: string }[], isError?: boolean }} */ (await invoke("request", { view: "client", method: "tools/call", params: { name, arguments: inputs } }))
  if (result.isError) throw new Error(result.content[0]?.text)
  return result.structuredContent ?? JSON.parse(result.content[0].text)
}

/// Lists the files a server has for `subject`, which the client keeps, so an app opens with no server up. An open app
/// whose apps have a new version loads again. This user's own list has the launcher in it, which starts, or none:
/// which is a server's word, so a list kept from before is asked for again first, and while its network doesn't
/// answer, the window waits. With none, this app needs updating where its network says an update is ready for it
/// (`behind`): otherwise it's the network that has none right now. A launcher missing from a later list, as during a
/// release, leaves the one that's open as it is. Each of this user's lists says again whether an update is ready, by
/// one check while its network answers; a check that fails leaves what was said as it was, and with no launcher says
/// why in its problem box, with Try Again, since it can't say which it is.
/** @param {string} [subject] */
async function list(subject) {
  listed.add(subject)
  const listing = invoke("apps", { subject })
  const apps = (/** @type {import("./host.js").Resource[]} */ resources) => new Set(resources.filter(({ _meta }) => _meta?.trusted === true).map(({ uri }) => uri.slice("ui://".length).split("/")[0]))
  const before = trusted.get(subject) ?? Promise.resolve(new Set())
  trusted.set(subject, listing.then(({ resources }) => apps(resources), () => before))
  // Nothing kept, and its network didn't answer: it's listed again once it does.
  const got = await listing.catch(() => undefined)
  if (!got) return
  let { resources, outdated } = got
  // A screen runs its own app and the UI library, which every one of them runs.
  const library = outdated.some((host) => host.split(".")[0] === "ui")
  for (const view of open.values()) if (library || outdated.includes(view.host)) page(view, { type: "stale" })
  if (!subject) {
    if (!launcherIn(resources) && shown.getSnapshot().matches("opening")) {
      await invoke("refresh", { view: "client" })
      ;({ resources } = await invoke("apps", { subject }))
    }
    launcher = launcherIn(resources) || launcher
    if (own) document.title = names.get(own.entrypoint) || document.title
    crumb()
    shortcuts()
    if (launcher || own) on({ type: "listed" })
    if (shown.getSnapshot().context.connection !== "online") return
    const late = await behind().catch((/** @type {Error} */ error) => error)
    if (late instanceof Error) {
      if (launcher || own) exception(`Couldn't check for an update: ${late.message}`)
      else on({ type: "unchecked", why: late.message })
      return
    }
    if (!launcher && !own) on(late ? { type: "tooOld" } : { type: "none", only: await only() })
    on({ type: "updateReady", ready: !phone && late })
  }
}

/// The launcher's page among the files listed, and with them, which app each app opens first (`first`) and the
/// App Store's page (`appStore`). An app is a page of an MCP App's kind, at ui://<app>/<version>/<page>.
/** @param {import("./host.js").Resource[]} resources */
function launcherIn(resources) {
  const pages = resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app")
  const named = (/** @type {string} */ uri) => {
    const [app, _version, ...page] = uri.slice("ui://".length).split("/")
    return `${app}/${page.join("/")}`
  }
  // The last one set for an app is the one kept: its first.
  first = new Map(pages.toReversed().map(({ uri }) => [named(uri).split("/")[0], named(uri)]))
  files = new Set(resources.map(({ uri }) => named(uri)))
  names = new Map(pages.map(({ uri, name }) => [named(uri), name ?? ""]))
  icons = new Map(pages.flatMap(({ uri, icons }) => (icons?.[0] ? [[named(uri), icons[0].src]] : [])))
  needs = new Map(pages.flatMap(({ uri, _meta }) => (typeof _meta?.runtimeVersion === "number" ? [[named(uri), _meta.runtimeVersion]] : [])))
  const found = (/** @type {string} */ category) => pages.find(({ _meta }) => _meta?.category === category)?.uri
  appStore = found("app-store") ? named(/** @type {string} */ (found("app-store"))) : ""
  settings = found("settings") ? named(/** @type {string} */ (found("settings"))) : ""
  const home = found("launcher") ? named(/** @type {string} */ (found("launcher"))) : ""
  // The launcher's app opens the launcher, as `one:home-screen` does.
  if (home) first.set(home.split("/")[0], home)
  return home
}

/// Who may install for everyone, if not this person, who then can't install a Home Screen: "Only Andre can install a
/// Home Screen for Everyone", by the names they go by, as the App Store says it. Nothing where they can, or where
/// the network can't say.
async function only() {
  try {
    const [{ who }, { users }] = await Promise.all([call("permissions_check", { artifact: "network" }), call("permissions_users", { artifact: "network", relation: "installer" })])
    if (users.includes(who) || users.includes("everyone")) return undefined
    const { people } = await call("sign-in_everyone").catch(() => ({ people: [] }))
    const called = (/** @type {string} */ user) => people.find((/** @type {{ who: string }} */ person) => person.who === user)?.called ?? user.replace(/^users\//, "").replace(/^./, (first) => first.toUpperCase())
    const names = new Intl.ListFormat("en", { type: "disjunction" }).format(users.map(called))
    return users.length ? `Only ${names} can install a Home Screen for Everyone` : "Nobody can install a Home Screen for Everyone"
  } catch {
    return undefined
  }
}

/// Where an app's page is, with the Colour Mode chosen on this device (`theme`), which the UI library puts the page in
/// before it draws (its `colour-mode.js`), so a dark choice on a light device never shows white while it loads.
/** @param {Pick<View, "host" | "entrypoint">} view */
const located = (view) => {
  const theme = chosen()
  return theme ? `${address(view)}?theme=${theme}` : address(view)
}

/// Loads an app's page again, in place, so the way back still leads out of the app: told first, as an MCP App is
/// before its host takes it down, and what it was answered forgotten.
/** @param {View} view */
function reloadPage(view) {
  post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
  invoke("close", { view: view.key }).finally(() => /** @type {Window} */ (view.frame.contentWindow).location.replace(located(view)))
}

/// An app whose native window didn't open, and why, until it's tried again or another opens.
/** @type {{ entrypoint: string, link: string, subject?: string, why: string } | undefined} */
let unopened
/// Says over the app that's showing what its page needs of its person, if anything: that it's been updated, with
/// Reload, or that it didn't open, with Try Again: only while the window shows its apps, as it alone owns the screen.
/// An app whose native window didn't open says so over what's showing.
function notes() {
  const view = showing(here())
  const now = view && shown.getSnapshot().hasTag("pages") ? pageOf(view) : undefined
  byId("update").hidden = !asksToReload || !now?.matches("stale")
  byId("page-problem").hidden = !unopened && !now?.matches("failed")
  const failed = unopened ?? (view && now?.matches("failed") ? { entrypoint: view.entrypoint, why: now.context.why } : undefined)
  if (failed) {
    byId("page-problem-heading").textContent = `${names.get(failed.entrypoint) || "This app"} didn't open`
    byId("page-problem-details-text").textContent = failed.why ?? ""
  }
}
byId("reload").addEventListener("click", () => {
  const view = showing(here())
  if (view) page(view, { type: "reload" })
})
byId("page-problem-button").addEventListener("click", () => {
  if (unopened) {
    const { link, subject } = unopened
    go(link, subject)
    return
  }
  const view = showing(here())
  if (!view) return
  view.tapped = performance.now()
  page(view, { type: "retry" })
})

/// Starts an app: its page loads in a frame of its own, behind what's showing until it's shown. A `fragment` is the
/// place in its page a link names, which the app follows (its `hashchange`), open already or not. An open one is
/// taken there in place, with no step of its own in the window's history: one there would be the step the window's
/// Back takes, rather than the one it made to show the app.
/** @param {string} entrypoint @param {string} [subject] @param {string} [fragment] @returns {View} */
function load(entrypoint, subject, fragment) {
  const at = key(entrypoint, subject)
  const opened = open.get(at)
  if (opened && fragment) /** @type {Window} */ (opened.frame.contentWindow).location.replace(`${located(opened)}#${fragment}`)
  if (opened) return opened
  if (!listed.has(subject)) list(subject)
  const place = { key: at, entrypoint, subject, host: host(entrypoint, subject) }
  const frame = Object.assign(document.createElement("iframe"), { src: `${located(place)}${fragment ? `#${fragment}` : ""}`, inert: true })
  /** @type {View} */
  const view = { ...place, frame }
  view.frame.setAttribute("sandbox", sandbox)
  // An app hands a link to the device's share sheet, or copies it, as its person presses Share (the UI library's `share`),
  // and shows a part of itself full screen (a live screen's Full Screen).
  view.frame.allow = "clipboard-write; web-share; fullscreen"
  open.set(at, view)
  document.body.append(view.frame)
  on({ type: "load", view })
  return view
}

/// The app showing, or the launcher: what goes as another shows.
/** @type {View | undefined} */
let front

/// The way back through this window: for each of its history entries from the launcher's, the key of the app it
/// shows (`null` for the launcher's), where it is in the browser's whole history, which counts the entries an app's
/// own screens add too, and the icon it was opened from, which it goes back into. An entry says how deep it is
/// (`history.state`), so going back or forward knows which way it goes, and a link to the launcher goes all the way
/// back to it.
/** @type {{ key: string | null, at: number, from?: Rect }[]} */
let stack = [{ key: null, at: history.length - 1 }]
/// How deep the entry the window is on is: 0 at the launcher's.
let depth = 0
/// The key of the app the window's entry shows, or `null` for the launcher.
const here = () => stack[depth]?.key ?? null
/// Goes back to the launcher's history entry, past each app's screens on the way, the one showing's too: where the
/// Navigation API is, straight to it, as it lists the window's own entries apart from its apps'; elsewhere by as many
/// as the window and the apps under the one showing added.
function backHome() {
  const entries = globalThis.navigation?.entries()
  const first = globalThis.navigation && entries?.[(navigation.currentEntry?.index ?? 0) - depth]
  if (first) navigation.traverseTo(first.key)
  else history.go(Math.min(-depth, stack[0].at - stack[depth].at))
}
/// The entry the window is on becomes the launcher's, as the app it showed is gone, or as the page loaded again on
/// an app's entry.
function restart() {
  history.replaceState(null, "")
  stack = [{ key: null, at: stack[depth]?.at ?? history.length - 1 }]
  depth = 0
}

/// The device's status bar, as the app that's showing would have it: shown unless it asked otherwise.
let statusBar = true
function bar() {
  const shown = front?.statusBar !== false
  if (shown === statusBar) return
  statusBar = shown
  invoke("status_bar", { shown })
}

/// Where `rect`, a place in a frame's page, is in the window: an icon an app is opened from.
/** @param {HTMLIFrameElement} frame @param {Rect} [rect] @returns {Rect | undefined} */
function within(frame, rect) {
  if (!rect || ![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) || !(rect.width > 0 && rect.height > 0)) return undefined
  const { x, y } = frame.getBoundingClientRect()
  return { x: x + rect.x, y: y + rect.y, width: rect.width, height: rect.height, radius: Number.isFinite(rect.radius) ? rect.radius : undefined }
}

/// The app that's open as `at` shows, or with none there, the launcher, at once.
/** @param {string | null} at @param {number} tapped */
function show(at, tapped) {
  const view = layer(at)
  if (!view) return
  reveal(view, tapped)
  notes()
}

/// An app coming on screen: `tapped` is when its icon was tapped. Opened again, a stale page loads its new version as
/// it opens, and a current one asks again what it shows.
/** @param {View} view @param {number} tapped */
function reveal(view, tapped) {
  view.tapped = tapped
  page(view, { type: "opened" })
  if (!pageOf(view)?.matches("current")) return
  // Its user returned to it.
  invoke("refresh", { view: view.key })
  opened(view)
}

/// Has the app that's open as `at` showing, or with none there, the launcher, and every other off screen. → the one
/// showing.
/** @param {string | null} at */
function layer(at) {
  const view = showing(at)
  front = view
  const home = !view || view.key === baseKey()
  if (nativeLauncher && home !== launcherShown) invoke("launcher_shown", { shown: (launcherShown = home) })
  if (view) used(view)
  bar()
  for (const other of open.values()) {
    other.frame.inert = other !== view
    if (other !== view) page(other, { type: "away" })
  }
  gestures()
  return view
}

/// Whether the device's own back and forward gestures are on: an iPhone's swipe in from the edge, a trackpad's swipe.
/// They go through this window's history, so they're off while something is open over the app that's showing, as a
/// phone's are under a sheet, and its Back or Escape shuts that first. At an app's first screen a computer's swipe is
/// the window's own (`wheel`), which takes the app home as its Back does, so the web view's, which would slide a
/// picture of what's under it, is off there too.
let swiping = true
/// Whether the device's swipe takes the app that's showing home, there being one of its own: an iPhone's.
let taking = false
/// Whether back has anything to go back from: an app over the launcher, or something open over the app that's
/// showing. Where it hasn't, Android shows going home as its back gesture is made (`MainActivity`).
/** @type {boolean | undefined} */
let backs
function gestures() {
  const view = showing(here())
  const takes = home()
  const on = !view?.over && !takes
  if (on !== swiping || takes !== taking) invoke("navigation_gestures", { on, home: takes })
  swiping = on
  taking = takes
  const back = !!view?.over || (!!view && view.entrypoint !== launcher) || (nativeLauncher && launcherOver && (!view || view.key === baseKey()))
  if (back !== backs) globalThis.androidBack?.postMessage(String(back))
  backs = back
  rail()
}
/// The window's own sheet or alert open over every app (its recovery key, Update Required), if one is.
const covering = () => /** @type {HTMLDialogElement | null} */ (document.querySelector("dialog[open]"))
/// Whether back from what's showing takes an app home: it's at its first screen, with nothing open over it, the
/// app's or the window's.
const home = () => depth > 0 && front?.first === true && !front.over && !covering()

/// A trackpad's two-finger swipe sideways, as each frame tells of its wheel (`src/initialization-script.js`), the
/// way a browser's swipe between pages goes back: at an app's first screen, swiping back takes it home, the move
/// following the fingers, and as they lift it carries on if it's past halfway or flicked, and otherwise springs
/// back. Once they've lifted, the wheel moves on by itself only ever slower (momentum), which is left alone; a quicker
/// move is the fingers again, which catch the move where it is. Up and down is never taken, nor a swipe that scrolls
/// something of the app's own that way, or one while the app has its own screens to go back to, whose web view's
/// swipe it is.
/** @type {{ x: number, y: number, from: number, recent: [number, number][], velocity: number, speed: number, slowing: number, began: number, own: boolean, taken?: boolean, lifted?: boolean } | undefined} */
let swipe
/// How long the wheel is still before the fingers are taken to have lifted: it says nothing while they're still.
const LIFTED = 150
let lifting = 0
/** @type {unknown} */
let released
/** @param {{ x: number, y: number, at: number, own: boolean }} moved */
function wheel({ x, y, at, own }) {
  clearTimeout(lifting)
  lifting = setTimeout(() => {
    lift()
    swipe = undefined
  }, LIFTED)
  const speed = Math.abs(x) + Math.abs(y)
  // A move quicker than the one before, by more than a wheel's moves wander, is the fingers'.
  const quicker = !!swipe && speed > swipe.speed * 1.15
  if (swipe?.lifted && !quicker) return void (swipe.speed = speed)
  if (!swipe || swipe.lifted) swipe = { x: 0, y: 0, from: 0, recent: [], velocity: 0, speed, slowing: 0, began: speed, own: false }
  // Slowing over several moves, never quicker, from a swipe's speed, is what's left once the fingers lift.
  if (quicker) Object.assign(swipe, { slowing: 0, began: speed })
  else swipe.slowing += 1
  swipe.speed = speed
  swipe.x += x
  swipe.y += y
  swipe.own ||= own
  const end = getComputedStyle(document.documentElement).direction === "rtl" ? -1 : 1
  if (swipe.taken === undefined) {
    // Too little yet to say which way it goes.
    if (Math.abs(swipe.x) + Math.abs(swipe.y) < 10) return
    const move = moving()
    // A move the fingers let go of is caught where it is, as long as it's still going.
    const caught = !!released && move?.matches("playing") === true && move.context.move === released
    swipe.taken = false
    if (Math.abs(swipe.x) <= Math.abs(swipe.y) || swipe.own || !(caught || (swipe.x * end < 0 && home()))) return
    on({ type: "grab" })
    swipe.taken = moving()?.matches("tracking") === true
    swipe.from = move?.context.move?.p ?? 0
  }
  if (!swipe.taken) return
  if (swipe.slowing >= 6 && swipe.began >= 4 && speed <= swipe.began * 0.8) return lift()
  const p = swipe.from - (swipe.x * end) / innerWidth
  // How fast it goes, over its last tenth of a second, which a page too busy to hear each move as it came leaves as
  // it was.
  swipe.recent = [...swipe.recent.filter(([then]) => at - then <= 100), [at, p]]
  const [[since, was]] = swipe.recent
  if (at - since >= 20) swipe.velocity = ((p - was) / (at - since)) * 1000
  on({ type: "drag", p: Math.min(1, Math.max(0, p)) })
}
/// The fingers have lifted: a move they took is let go, as fast as it was going.
function lift() {
  if (!swipe) return
  if (swipe.taken) {
    on({ type: "release", velocity: swipe.velocity })
    released = moving()?.context.move
  }
  swipe.taken = false
  swipe.lifted = true
}
/// The frames that hold something their person typed and haven't sent (`initialization-script.js`): the window says
/// whether any does, which a new build of the app waits for (`clients/app/src/updater.rs`).
const typedIn = new Set()
addEventListener("message", ({ data, source }) => {
  if (data?.method !== "notifications/one/editing") return
  const before = typedIn.size > 0
  if (data.params?.editing) typedIn.add(source)
  else typedIn.delete(source)
  if (typedIn.size > 0 !== before) invoke("editing", { editing: typedIn.size > 0 }).catch(() => {})
})
addEventListener("message", ({ data }) => {
  if (data?.method === "notifications/one/wheel") wheel(data.params)
  // Escape on a computer, at an app's first screen with nothing open over it, takes it home as its Back does, and on
  // the launcher puts it away.
  if (data?.method === "notifications/one/escape" && home()) history.back()
  else if (data?.method === "notifications/one/escape" && panel() && !depth && !open.get(baseKey())?.over) invoke("hide")
})

/// The app that's open as `at`, or with none there, the one under every other: the launcher, or while there's none,
/// the App Store.
/** @param {string | null} at */
const showing = (at) => open.get(at ?? "") ?? open.get(shown.getSnapshot().matches("app_store") ? key(appStore) : baseKey())

/// The rail: every app's way back to the Home Screen, above every app on a phone or a tablet: a home line on the
/// window's right edge, iOS's home indicator on its side, which a finger pulls in to go home (`pull`). It keeps out of
/// the way while something of the app's is open over it (a sheet, a menu), as the device's back does. A computer has
/// none: each app is a window of its own there, which the computer's own Dock or taskbar goes between, and whose title
/// bar's breadcrumb goes home (`titleBar`).
///
/// The apps used lately on this device (`recent`) are what a long press on Android's app icon lists (`shortcuts`).
const RECENT = 8
/// The apps used lately on this device, by their keys, newest first.
/** @type {string[]} */
let recent = []
try {
  recent = JSON.parse(localStorage.getItem("recent") ?? "[]")
} catch {}
/// The apps shown since they were opened, which coming in front again doesn't move.
/** @type {WeakSet<View>} */
const fronted = new WeakSet()
/// An app has come in front: opened, it's the newest used.
/** @param {View} view */
function used(view) {
  if (view.entrypoint === launcher || fronted.has(view)) return
  fronted.add(view)
  recent = [view.key, ...recent.filter((was) => was !== view.key)].slice(0, RECENT)
  try {
    localStorage.setItem("recent", JSON.stringify(recent))
  } catch {}
  shortcuts()
}
/// The apps used lately that are still this user's, the open ones first.
function recents() {
  const listed = recent
    .map((at) => ({ key: at, subject: at.slice(0, at.indexOf("|")) || undefined, entrypoint: at.slice(at.indexOf("|") + 1) }))
    .filter(({ entrypoint }) => names.has(entrypoint) && entrypoint !== launcher)
  return [...listed.filter(({ key }) => open.has(key)), ...listed.filter(({ key }) => !open.has(key))]
}

/// Android's shortcuts to apps here (`MainActivity`): the apps used lately, newest first, which a long press on the
/// app's icon lists, and every app installed, so one put on the home screen greys out once it's gone. An app opened
/// for a group has no link of its own to open it by, so it isn't among them.
let posted = ""
async function shortcuts() {
  if (!globalThis.androidShortcuts) return
  const ones = recents().filter(({ subject }) => !subject)
  const now = JSON.stringify([ones.map(({ entrypoint }) => entrypoint), [...names.keys()]])
  if (now === posted) return
  posted = now
  const recent = await Promise.all(ones.map(({ entrypoint }) => shortcut(entrypoint)))
  globalThis.androidShortcuts.postMessage(JSON.stringify({ recent: recent.filter(Boolean), installed: [...names.keys()] }))
}

/// An app's shortcut as Android takes one: its entrypoint, name, and icon as a PNG of its square, 108 dp across at
/// the densest screens, as an adaptive icon is drawn, since Android draws no SVG. → nothing where it has no icon.
/** @param {string} entrypoint @returns {Promise<{ id: string, name: string, icon: string } | undefined>} */
async function shortcut(entrypoint) {
  const src = icons.get(entrypoint)
  if (!src) return undefined
  const picture = new Image()
  picture.src = src
  await picture.decode()
  const canvas = Object.assign(document.createElement("canvas"), { width: 432, height: 432 })
  canvas.getContext("2d")?.drawImage(picture, 0, 0, 432, 432)
  return { id: entrypoint, name: names.get(entrypoint) || entrypoint, icon: canvas.toDataURL("image/png").slice("data:image/png;base64,".length) }
}

const line = byId("line")

/// Whether the rail's line shows: while the window shows its apps and has a Home Screen to go back to, over an app,
/// at any of its screens, unless something is drawn over it, a sheet or a menu (`data-away`). Android is told where
/// the line is, which its back gesture leaves to it.
/** @type {string | undefined} */
let excluded
/// The fingers on the line, by their pointers, where each came down, and the one pulling it (`pull`, below).
/** @type {Map<number, { x: number, y: number }>} */
const touching = new Map()
/** @type {{ id: number, x: number, recent: [number, number][] } | undefined} */
let pull
function rail() {
  const now = shown.getSnapshot()
  // A computer's windows have none: its own Dock or taskbar goes between them.
  const homed = Boolean(launcher) && !windows && now.hasTag("pages") && !now.matches("app_store")
  const lined = homed && here() !== null && !showing(here())?.layer
  line.toggleAttribute("data-away", !lined)
  // A line that went home under a finger comes back with its app.
  if (lined && !pull) line.toggleAttribute("data-pulled", false)
  const at = lined ? line.getBoundingClientRect() : undefined
  const edge = at ? [at.left, at.top, at.right, at.bottom].map(Math.round).join(",") : ""
  if (edge !== excluded) globalThis.androidEdge?.postMessage(edge)
  excluded = edge
}
addEventListener("resize", rail)

/// How far in, as a share of the window's width, past which a finger let go of the line goes home.
const PULLED = 0.18
/// A finger on the line, pulling it in: once it has come in far enough to say so, the window's move to the Home
/// Screen follows it (`grab` with `home`, and `drag`), the app as far shrunk as `p`, and let go, it goes home if it
/// came in past `PULLED` or was flicked in, and otherwise springs back. A tap does nothing. The first finger to come
/// in takes it, and only its lifting lets go: another on the line meanwhile, as the hand holding the phone touches
/// its edge, is ignored, as iOS's swipe home ignores a second finger.
line.addEventListener("pointerdown", (event) => {
  if (event.button !== 0) return
  line.setPointerCapture(event.pointerId)
  touching.set(event.pointerId, { x: event.clientX, y: event.clientY })
})
line.addEventListener("pointermove", (event) => {
  const down = touching.get(event.pointerId)
  if (!pull && down) {
    // Too little yet to say which way it goes; up or down isn't a pull.
    if (Math.hypot(event.clientX - down.x, event.clientY - down.y) < 8) return
    touching.delete(event.pointerId)
    if (!(down.x > event.clientX && Math.abs(event.clientX - down.x) > Math.abs(event.clientY - down.y))) return
    on({ type: "grab", home: true })
    if (moving()?.matches("tracking")) pull = { id: event.pointerId, x: down.x, recent: [] }
  }
  if (event.pointerId !== pull?.id) return
  const inward = (pull.x - event.clientX) / innerWidth
  // The app follows the finger, and past the card (`HELD`) slows to a stop, as a rubber band does.
  const p = HELD * Math.tanh(Math.max(0, inward) / HELD)
  pull.recent = [...pull.recent.filter(([then]) => event.timeStamp - then <= 100), [event.timeStamp, inward]]
  line.toggleAttribute("data-pulled", true)
  on({ type: "drag", p })
})
/** @param {PointerEvent} event */
function letGo(event) {
  touching.delete(event.pointerId)
  if (event.pointerId !== pull?.id) return
  const moves = pull.recent
  // How fast it was coming in over its last tenth of a second, in widths a second.
  const [[since, was] = [0, 0]] = moves
  const [at, inward] = moves.at(-1) ?? [0, 0]
  const velocity = at - since >= 20 ? ((inward - was) / (at - since)) * 1000 : 0
  const home = event.type !== "pointercancel" && (inward >= PULLED || velocity > 0.6)
  if (!home) line.toggleAttribute("data-pulled", false)
  on({ type: "release", velocity: Math.max(0, velocity) * (home ? 1 : 0), end: home ? 1 : 0 })
  // Going home, it stays hidden as the window's history follows.
  pull = undefined
}
line.addEventListener("pointerup", letGo)
line.addEventListener("pointercancel", letGo)

/// When the window last came on screen.
let onScreenSince = 0
document.addEventListener("visibilitychange", () => {
  if (!document.hidden) onScreenSince = performance.now()
})

/// Says how long an app took from the tap on its icon to its content on screen: the second frame after it's
/// showing with what it drew from. The launcher's time is from when this page began to load. An open the window
/// was hidden during isn't told: the system draws no frame for a hidden window, so it would be timed to whenever
/// its person looked, as a Mac's app that started hidden was, at 23 minutes.
/** @param {View} view */
function opened(view) {
  if (view.tapped === undefined) return
  const tapped = view.tapped
  view.tapped = undefined
  requestAnimationFrame(() => requestAnimationFrame(() => {
    if (onScreenSince <= tapped) {
      const name = view.entrypoint.split("/")[0]
      const { duration } = performance.measure(`open ${name}`, { start: tapped })
      invoke("opened", { view: name, ms: Math.round(duration) })
    }
  }))
}

/// Shows this device's person their recovery key, or with `typed`, asks them for it and lets the device in with
/// it: theirs, or that of `subject`, a group or the network the app asking is for. → `{ done }`: whether they saw it,
/// or the device is in now. It's over whatever app asked, and the app is told only that.
/** @param {boolean} typed @param {string} [subject] @returns {Promise<{ done: boolean }>} */
function recoveryKey(typed, subject) {
  const dialog = /** @type {HTMLDialogElement} */ (byId("recovery"))
  const [key, entry, problem, form] = /** @type {[HTMLElement, HTMLInputElement, HTMLElement, HTMLFormElement]} */ ([".key", ".entry", ".wrong", "form"].map((part) => dialog.querySelector(part)))
  dialog.dataset.typed = String(typed)
  entry.value = key.textContent = problem.textContent = dialog.returnValue = ""
  if (!typed) invoke("recovery_key", { subject }).then((shown) => (key.textContent = shown), (error) => (problem.textContent = `${error}`))
  dialog.showModal()
  return new Promise((resolve) => {
    form.onsubmit = async (event) => {
      event.preventDefault()
      const pressed = /** @type {HTMLButtonElement | null} */ (event.submitter)
      if (typed && pressed?.value !== "cancel") {
        try {
          await invoke("recover", { key: entry.value, subject })
        } catch (error) {
          return void (problem.textContent = `${error}`)
        }
      }
      dialog.returnValue = pressed?.value === "cancel" ? "" : "done"
      dialog.close()
    }
    dialog.onclose = () => {
      entry.value = key.textContent = ""
      resolve({ done: dialog.returnValue === "done" })
    }
  })
}

/** @param {View & { window?: Window }} view @param {Record<string, unknown>} message */
const post = (view, message) => (view.window ?? view.frame.contentWindow)?.postMessage({ jsonrpc: "2.0", ...message }, "*")

/// The page in an app's frame a message came from: one known by its window, or one saying its number from a frame of
/// the page it was made for, which it's known by from then on.
/** @param {MessageEventSource | null} source @param {any} data */
function frameOf(source, data) {
  for (const [frame, view] of nested) {
    if (view.window === source) return view
    const from = /** @type {Window | null} */ (source)
    if (!view.window && from && data?.frame === frame && from.parent === view.in.frame.contentWindow) return Object.assign(view, { window: from })
  }
}

/// The haptics an app may ask for, by iOS's names.
/** @type {unknown[]} */
const HAPTICS = ["light", "medium", "heavy", "rigid", "soft", "selection", "success", "warning", "error"]
/// What this page answers an app itself. The rest is the client's to answer, as from the app whose page it is,
/// which this page says, whatever the page does: a server has an untrusted app's page call only what its server code
/// may (`OneCore.Mcp`). An untrusted app's page asks for whoever it was opened for, and changes nothing of this
/// device but its own status bar; a trusted one's may do anything its server code may, which is all its person may.
/** @param {View} view @param {string} method @param {Record<string, any>} params */
async function respond(view, method, params) {
  const app = view.entrypoint.split("/")[0]
  const trusts = async () => (await trusted.get(view.subject))?.has(app) === true
  /** Who the page asks for: whoever it says, if its app is trusted. @param {string} [said] */
  const whom = async (said) => (said && (await trusts()) ? said : view.subject)
  if (method === "ui/initialize") {
    // A page in a frame of another's has none of the window's edges to keep clear, and nothing names it above it.
    const inFrame = [...nested.values()].includes(/** @type {any} */ (view))
    return {
      protocolVersion: "2026-01-26",
      hostInfo: { name: "one", version: "0.1.0" },
      hostCapabilities: { serverTools: {}, serverResources: {} },
      hostContext: {
        theme: theme(),
        displayMode: "fullscreen",
        availableDisplayModes: ["fullscreen"],
        containerDimensions: { width: view.frame.clientWidth, height: view.frame.clientHeight },
        locale: navigator.language,
        timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        deviceCapabilities: { touch: matchMedia("(pointer: coarse)").matches, hover: matchMedia("(hover: hover)").matches },
        safeAreaInsets: inFrame ? { top: 0, right: 0, bottom: 0, left: 0 } : insets(view.frame),
        ...(!inFrame && titleRow()),
        // Whether the window names it above it, in its title bar's breadcrumb (`titled`), so its first screen doesn't.
        breadcrumb: Boolean(own) && !titleBar.hidden && !inFrame,
        // Whether its network answers, which the window keeps it told of.
        connection: shown.getSnapshot().context.connection,
      },
    }
  }
  if (method === "ping") return {}
  // Another app's page, in a frame of this one's, which this page answers as that app's (`framed`): only a trusted
  // app's page shows one, as it may do anything its person may.
  // A frame of its own that's gone: its page is forgotten.
  if (method === "one/frame" && typeof params.forget === "string") {
    if (nested.get(params.forget)?.in === view) nested.delete(params.forget)
    return {}
  }
  if (method === "one/frame") {
    if (!(await trusts())) throw new Error(`${names.get(view.entrypoint) || app} is installed untrusted, so it shows no other app's page.`)
    const [entrypoint, fragment] = String(params.src).split("#", 2)
    const subject = await whom(params.subject)
    if (subject && !listed.has(subject)) list(subject)
    const place = { key: key(entrypoint, subject), entrypoint, subject, host: host(entrypoint, subject) }
    const frame = crypto.randomUUID()
    nested.set(frame, { ...place, frame: view.frame, in: view })
    const url = new URL(located(place), location.href)
    url.searchParams.set("frame", frame)
    return { url: `${url}${fragment ? `#${fragment}` : ""}`, sandbox, frame }
  }
  if (method === "ui/request-display-mode") return { mode: "fullscreen" }
  // A file too large to go in a call: the client sends it where the network keeps it, for a tool of that app.
  if (method === "one/upload") return invoke("upload", await params.file.arrayBuffer(), { headers: { app } })
  // A person's recovery key is shown and typed on this page, which is the client's own: no app's page is handed one.
  if (method === "one/recovery-key") return recoveryKey(params.typed === true, await whom(params.subject))
  // Another app, shown over this one, as tapping its icon or a notification from it does: by its app's name,
  // and its page if it isn't the app's first app, for whoever this one is for unless it says. `behind` starts
  // it as a finger lands on its icon, and it shows as the finger lifts, out of the icon at `from` in its page.
  if (method === "ui/open-link" && params.url?.startsWith("one:")) {
    if (!go(params.url.slice("one:".length), await whom(params.subject), params.behind, within(view.frame, params.from))) throw new Error("That app isn't on this device")
    return {}
  }
  // A web address opens in the device's own browser, outside the app.
  if (method === "ui/open-link" && /^https?:/.test(new URL(params.url).protocol)) {
    openLink(params.url)
    return {}
  }
  // An app put on the device's own home screen, which Android's app does (`MainActivity`): by its entrypoint.
  if (method === "one/pin-shortcut") {
    const pin = globalThis.androidPin
    const app = pin && (await shortcut(params.entrypoint))
    if (!pin || !app) throw new Error("This device can't put that app on its home screen")
    pin.postMessage(JSON.stringify(app))
    return {}
  }
  // A haptic plays here, not in the app's frame: only this page reaches the device's, and Chrome lets no frame of
  // another address vibrate.
  if (method === "one/haptic") {
    if (!HAPTICS.includes(params.kind)) throw new Error(`There's no haptic ${params.kind}`)
    await haptic(params.kind)
    return {}
  }
  // What an app is told of its client, and the few things one changes of it.
  if (method === "one/client") {
    const changes = Object.keys(params).filter((change) => change !== "statusBar")
    if (changes.length && !(await trusts())) throw new Error(`${names.get(view.entrypoint) || app} is installed untrusted, so it doesn't change this device (${changes.join(", ")}).`)
    if (params.leave) return invoke("sign_out")
    // The device's status bar, hidden while this app shows, as an iPhone's Home Screen hides it while it's edited.
    if ("statusBar" in params) {
      view.statusBar = params.statusBar !== false
      bar()
      return { message: "" }
    }
    // A phone whose push failed tries again.
    if (params.push) return invoke("push")
    // A phone's own Settings page, where its person turns on what only they may: Android's Accessibility, or the
    // app's App Info, and an iPhone's page for the app.
    if (params.settings) {
      if (globalThis.androidSettings) globalThis.androidSettings.postMessage(params.settings)
      else if (params.settings === "app") await invoke("settings")
      else throw new Error("This device has no Settings to open here.")
      return { message: "" }
    }
    // This device becomes a server of its person's, or stops being one, or takes the latest builds.
    if ("serve" in params) return { message: await invoke("serve", { on: params.serve === true }) }
    // It updates as the window shows it, over every app, where an update is ready.
    if (params.update) {
      if (!shown.getSnapshot().context.updateReady) on({ type: "updateReady", ready: !phone && (await behind()) })
      on({ type: "update" })
      if (!shown.getSnapshot().matches("updating")) throw new Error("There's no update for this device.")
      return { message: "Updating…" }
    }
    if ("theme" in params) {
      if (params.theme) localStorage.setItem("theme", params.theme)
      else localStorage.removeItem("theme")
      dress()
    }
    const client = await invoke("client")
    return { ...client, theme: chosen(), pinsShortcuts: !!globalThis.androidPin, settings: !!globalThis.androidSettings || client.settings === true, launcher: panel() }
  }
  if (method.startsWith("ui/")) throw Object.assign(new Error(`This client has no ${method}`), { code: -32601 })
  // An app opened from a group's page is answered for the group. What else an untrusted app's page says of its
  // request, who it's from among it, isn't passed on.
  const subject = await whom(params._meta?.subject)
  if (subject && !listed.has(subject)) list(subject)
  const meta = { ...((await trusts()) && params._meta), app, ...(subject && { subject }) }
  return invoke("request", { view: view.key, method, params: { ...params, _meta: meta } })
}

addEventListener("message", async ({ source, data }) => {
  const view = [...open.values()].find(({ frame }) => frame.contentWindow === source) ?? frameOf(source, data)
  if (!view || typeof data?.method !== "string") return
  const { id, method, params = {} } = data
  // An app's page, or a page in its frame, logs its errors as an MCP App logs to its host, and they're its app's,
  // whatever it says.
  const errored = id === undefined && method === "notifications/message" && params.level === "error"
  if (errored) exception(params.data?.message, params.data?.file, params.data?.line, view.entrypoint.split("/")[0])
  // A page in a frame is its frame's page's: it says how tall it is to that page.
  const child = [...nested].find(([, one]) => one === view)
  if (id === undefined && child) {
    const [frame, { in: hosting }] = child
    if (method === "ui/notifications/size-changed" && typeof params.height === "number") post(hosting, { method: "notifications/one/framed", params: { frame, height: params.height } })
    return
  }
  if (id === undefined) {
    // The app's user went back out of it. The app stays open behind the launcher.
    if (method === "ui/notifications/request-teardown" && !view.frame.inert && depth) history.back()
    else if (method === "ui/notifications/request-teardown" && panel() && view.key === baseKey()) invoke("hide")
    // The launcher's content says how tall it is, which its panel is made while it shows it.
    if (method === "ui/notifications/size-changed" && panel() && view.key === baseKey() && typeof params.height === "number") {
      asked = params.height
      fit()
    }
    // Its top bar is the window's title bar: what's empty in it moves the window, and double-clicked does what a
    // title bar does (`titleBar`).
    if (method === "one/drag-window") invoke("plugin:window|start_dragging")
    if (method === "one/title-bar-double-click") invoke("title_bar_double_click")
    // Its page has drawn: what it first asked for has settled (`render`), a problem shown as it failed included.
    if (method === "notifications/one/drawn") {
      page(view, { type: "drawn" })
      if (!view.frame.inert) opened(view)
    }
    // Something is open over its page, or nothing is any more, and whether it's on its first screen.
    if (method === "notifications/one/over") {
      view.over = params.over === true
      view.layer = params.layer === true
      view.first = params.first
      gestures()
    }
    if (errored) page(view, { type: "errored" })
    return
  }
  try {
    post(view, { id, result: await respond(view, method, params) })
  } catch (error) {
    const { code, message } = /** @type {{ code?: number, message?: string }} */ (error)
    post(view, { id, error: { code: code ?? -32000, message: message ?? String(error) } })
  }
})

/// Shows an app over whatever's showing, by what a `one:` link says after its scheme: `<app>[/<page>][#<fragment>]`,
/// its app's name, and its page if it isn't the app's first app, and the place in it, for `subject`. With `behind`
/// it only starts. The launcher, linked to, is gone back to, past every app open over it. → whether there is one
/// here.
/** @param {string} link @param {string} [subject] @param {boolean} [behind] @param {Rect} [from] */
function go(link, subject, behind, from) {
  const [entrypoint, fragment] = link.split("#", 2)
  const page = entrypoint.includes("/") ? entrypoint : first.get(entrypoint)
  // An app with no page of its own to open, or a page it no longer has (a link kept from before, an older phone's),
  // opens where its settings are, at that app (its own `#<app>`).
  if (files.size && (!page || !files.has(page)) && settings && page !== settings)
    return go(`${settings}#${entrypoint.split("/")[0]}${fragment ? `&${fragment}` : ""}`, subject, behind, from)
  if (!page) return false
  // On a computer, an app opens in its own window, and the Home Screen comes forward in its own.
  if (windows && key(page, subject) !== baseKey()) {
    if (behind) return true
    if (page === launcher) {
      invoke("front")
      return true
    }
    invoke("open_window", { entrypoint: page, subject, link: fragment ? `${page}#${fragment}` : page })
    rail()
    return true
  }
  // On a phone, an app opens natively, over this page, and one that needs a newer host in a frame here. One whose
  // native window should have opened and didn't says why, as a frame that didn't open does (`notes`). Its window is
  // told where its icon is, which pulled home by its rail's line it goes back into.
  const at = key(page, subject)
  const needed = needs.get(page) ?? 0
  if (runtimeVersion && needed && needed <= runtimeVersion && !framed.has(at) && at !== baseKey()) {
    if (behind) return true
    unopened = undefined
    notes()
    const icon = from && [from.x, from.y, from.width, from.height]
    invoke("open_window", { entrypoint: page, subject, link: fragment ? `${page}#${fragment}` : page, icon }).then(
      (opened) => opened || (framed.add(at), go(link, subject, behind, from)),
      (why) => {
        unopened = { entrypoint: page, link, subject, why: String(why) }
        notes()
      },
    )
    return true
  }
  const tapped = performance.now()
  const next = load(page, subject, fragment)
  if (behind) return true
  if (next.key === baseKey()) {
    if (depth) backHome()
    return true
  }
  // The way back out of an app is an entry in this window's history, made as it's opened, so the device's back
  // leads out of the app however soon it's pressed.
  if (here() !== next.key) {
    depth += 1
    history.pushState(depth, "")
    // It goes back into the icon it came out of, or slides out if it came from none.
    stack.splice(depth, Infinity, { key: next.key, at: history.length - 1, from })
    rail()
  }
  on({ type: "show", to: next, back: false, icon: from, tapped })
  return true
}

// A notification from an app was tapped, or a `one:` link opened (`links` in src/lib.rs).
listen("open", ({ payload: link }) => on({ type: "link", link }))
/// A phone's push plugin says a notification was tapped, even the tap that started the app, once this page hears
/// it, which is once there are apps to open; and each push token its push service gives it, which it registers if
/// it's new.
async function hearTaps() {
  await window.__TAURI__.core.addPluginListener("notifications", "push-token", ({ token }) => invoke("push_token", { token }))
  await window.__TAURI__.core.addPluginListener("notifications", "notificationClicked", (tapped) => invoke("tapped", { tapped }))
  await invoke("plugin:notifications|set_click_listener_active", { active: true })
}

// A server says which open apps were loaded against a version that's gone.
listen("stale", ({ payload: hosts }) => {
  for (const view of open.values()) if (hosts.includes(view.host)) page(view, { type: "stale" })
})

listen("changed", ({ payload: view }) => {
  if (view === "client") for (const subject of listed) list(subject)
  else if (open.has(view)) post(/** @type {View} */ (open.get(view)), { method: "notifications/one/changed" })
  for (const one of nested.values()) if (one.key === view) post(one, { method: "notifications/one/changed" })
})

/// An iPhone's swipe in from the left edge (`edge` in src/lib.rs), which is the window's own: WebKit's swipe between
/// pages, which would go forward from the right edge too, where the rail's line is, is off. At an app's first screen,
/// or while a move is still going, the move follows the finger, `p` as far across the window as it's come, as a pop
/// does, and let go at `velocity` (across the window a second), it carries on if it's past halfway or flicked, or
/// springs back. Anywhere else it goes back a screen through the window's history, as the device's back does, once
/// it's let go a third of the way across or flicked. Taken away, as by a call, it does nothing.
/** @type {{ p: number } | undefined} */
let stepping
window.grab = () => {
  on({ type: "grab" })
  stepping = moving()?.matches("tracking") ? undefined : { p: 0 }
}
/** @param {number} p */
window.drag = (p) => {
  if (stepping) stepping.p = p
  else on({ type: "drag", p: Math.min(1, Math.max(0, p)) })
}
/** @param {number} velocity @param {0 | 1} [end] */
window.release = (velocity, end) => {
  if (!stepping) return on({ type: "release", velocity, end })
  const { p } = stepping
  stepping = undefined
  if (end ?? (velocity > 0.5 || p > 1 / 3)) history.back()
}

/// Android's back gesture, as its finger comes in from the edge (`MainActivity`): where back takes an app home, the
/// move follows it in the shape of Android's back to home, the app drawn a little way (`BACK`) toward where it goes as
/// the finger comes all the way in, and once Android says it's done (`back`), carries on home from there, or
/// cancelled, goes back.
let backing = false
/// How far Android's back gesture draws the move, at most, before it's let go: the app as small as 0.9 of itself
/// going into its icon, as Android's own back to home has it.
const BACK = 0.12
/** @type {{ p: number, at: number, velocity: number }} */
const backed = { p: 0, at: 0, velocity: 0 }
window.backStarted = () => {
  if (!home()) return
  on({ type: "grab" })
  backing = moving()?.matches("tracking") === true
  Object.assign(backed, { p: 0, at: performance.now(), velocity: 0 })
}
/** @param {number} progress */
window.backProgressed = (progress) => {
  if (!backing) return
  // Most of the way in the gesture's first part, slowing after, as Android's own does (its decelerate curve).
  const p = BACK * (1 - (1 - Math.min(1, Math.max(0, progress))) ** 3)
  const now = performance.now()
  const dt = (now - backed.at) / 1000
  if (dt > 0) backed.velocity = 0.6 * ((p - backed.p) / dt) + 0.4 * backed.velocity
  Object.assign(backed, { p, at: now })
  on({ type: "drag", p })
}
window.backCancelled = () => {
  if (backing) on({ type: "release", velocity: backed.velocity, end: 0 })
  backing = false
}
/// The device's own back, where a device has one (the app's window there calls this): Android's gesture, done, takes
/// the app it drew going home the rest of the way, and otherwise what's open over the app that's showing shuts first,
/// the window's own sheet as it would to Escape (an alert that can't be shut stays).
/// → whether either was. If neither was, the device goes back through this window's history, which leads back through
/// an app's screens and out of it, and from the launcher out of the app.
window.back = () => {
  if (backing) {
    backing = false
    on({ type: "release", velocity: backed.velocity, end: 1 })
    return true
  }
  const sheet = covering()
  if (sheet) {
    sheet.requestClose()
    return true
  }
  const view = open.get(here() ?? "") ?? open.get(baseKey())
  if (!view?.over || view.frame.inert) return false
  post(view, { method: "notifications/one/back" })
  return true
}

/// A tap on an iPhone's status bar (the app's window there calls this): the app that's showing scrolls to its top.
window.scrollToTop = () => {
  const view = showing(here())
  if (view) post(view, { method: "notifications/one/scroll-to-top" })
}

// The device's own way back, and this page's, both end here.
addEventListener("popstate", (event) => {
  const tapped = performance.now()
  const to = typeof event.state === "number" ? event.state : 0
  const back = to < depth
  // Going back, the app that was showing goes back into the icon it came out of, if it came out of one; forward, it
  // comes out of it again.
  const icon = stack[back ? depth : to]?.from
  depth = to
  rail()
  const view = /** @type {View} */ (showing(here()))
  // The device has drawn the move itself (an iPhone's swipe in from the edge): it isn't drawn a second time.
  on(event.hasUAVisualTransition ? { type: "popped", to: view, tapped } : { type: "show", to: view, back, icon, tapped })
})

// Back online, or back in front, its network is tried again now if it didn't answer.
for (const [target, event] of /** @type {const} */ ([[window, "online"], [document, "visibilitychange"]]))
  target.addEventListener(event, () => document.visibilityState === "visible" && invoke("reconnect").catch(() => {}))
// Its network is tried again now, rather than when it next would be.
for (const retry of ["reconnect", "banner-retry"]) byId(retry).addEventListener("click", () => invoke("retry").catch(() => {}))
byId("try-again-button").addEventListener("click", () => on({ type: "retry" }))
byId("problem-button").addEventListener("click", () => on({ type: shown.getSnapshot().matches("needs_updating") ? "update" : "retry" }))
byId("update-button").addEventListener("click", () => on({ type: "update" }))
// Nothing dismisses it: the window works again once it's updated, or its network talks with it again.
byId("update-required").addEventListener("cancel", (event) => event.preventDefault())
byId("open-app-store").addEventListener("click", () => on({ type: "openAppStore" }))

/// The user returned to the window, so what's showing asks again, and its apps' files are listed again, for a new
/// version of any. While the window is on screen, what's showing asks again when a server says something changed.
function returned() {
  if (!shown.getSnapshot().matches("launcher")) return
  invoke("visible", { onScreen: !document.hidden })
  // Put away, the window's pages are off screen (`away`), where its stale ones load their new versions.
  if (document.hidden) return
  // Summoned, the launcher has the keyboard, in its search (the Home Screen's).
  if (panel()) open.get(baseKey())?.frame.focus()
  for (const subject of listed) list(subject)
  invoke("refresh", { view: baseKey() })
  const view = here()
  if (view) invoke("refresh", { view })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

/// Shows the launcher, once this user's files have listed one: loaded, or loaded again where it was open, natively
/// on a phone where it can be.
function start() {
  const home = open.get(baseKey())
  if (home) page(home, { type: "retry" })
  else if (nativeLauncher) on({ type: "drawn", key: baseKey() })
  else if (phone && !own && runtimeVersion && (needs.get(launcher) ?? Infinity) <= runtimeVersion && !framed.has(baseKey())) openLauncher()
  else load(base(), own?.subject)
  // The app that was showing when the page was loaded again is gone, so the launcher shows.
  restart()
  show(null, 0)
}

/// Opens the launcher natively, as the phone's root window: drawn, it's the launcher here; or its frame is.
function openLauncher() {
  invoke("open_launcher", { entrypoint: launcher }).then(
    (opened) => {
      if (!opened) return framedLauncher()
      nativeLauncher = launcherShown = true
      on({ type: "drawn", key: baseKey() })
    },
    framedLauncher,
  )
}

/// The launcher in a frame here, as it can't be drawn natively, or no longer can (`launcher`).
function framedLauncher() {
  nativeLauncher = false
  framed.add(baseKey())
  load(base(), own?.subject)
  show(null, 0)
}
listen("launcher", ({ payload }) => {
  if (payload?.native === false && nativeLauncher) framedLauncher()
  if (typeof payload?.over === "boolean") (launcherOver = payload.over), gestures()
})
// The native launcher opens an app, or starts it as a finger lands on its icon, as its frame's page asks (`go`).
listen("go", ({ payload }) => nativeLauncher && go(payload.link, payload.subject ?? undefined, payload.behind, payload.from ?? undefined))

/// Shows the App Store with no launcher under it, which its person installs one with. There's nothing to go back
/// to from it.
function openAppStore() {
  const view = load(appStore)
  restart()
  show(view.key, performance.now())
}

/// Follows where the device is with its network as the client says it (`State`), as it moves: each move of its
/// session heard once and in order (the page may hear one as it opens before `start` has said where it was), whether
/// its network answers, and its servers saying it's behind them, after which it asks them nothing more and says only
/// that it needs updating. Being behind isn't a move of whether its network answers: Update Required asks again as
/// its network answers after it didn't, not as another window's Try Again is answered.
let heard = -1
let heardBehind = false
/** @param {State} state */
function hear(state) {
  const { connection } = state
  const behind = connection === "behind"
  if (connection !== "behind" && connection !== shown.getSnapshot().context.connection) on({ type: "connection", connection })
  if (state.seq > heard) {
    heard = state.seq
    on({ type: "session", told: state })
  }
  if (behind && !heardBehind) on({ type: "tooOld" })
  heardBehind = behind
}

/// A window whose title bar is over the page (`titleBar`), as a Mac's with no title band is: the page keeps its
/// height clear at its top (`--safe-top`), as every app is told, under the window's own controls (a Mac's traffic
/// lights). Held there, it drags the window, and double-clicked, it does what a title bar does (on a Mac, as it's set:
/// zoom). An app's own window has its breadcrumb there, just after the window's controls: Home, which brings the Home
/// Screen forward and keeps this window open, as Window → Home Screen does (`host.rs`), then the app (`crumb`).
const titleBar = byId("title-bar")
/// Where the title row is over an app's page, which its top bar is: where in it the page's own may start, after the
/// window's controls and breadcrumb, and how far from its end, before the controls there. Nothing where the window
/// has no title bar over its page.
function titleRow() {
  if (titleBar.hidden) return {}
  const style = getComputedStyle(document.documentElement)
  const px = (/** @type {string} */ name) => parseFloat(style.getPropertyValue(name)) || 0
  const nav = byId("title-bar-nav")
  const start = nav.hidden ? px("--title-bar-start") : nav.getBoundingClientRect().right + 12
  return { titleBar: { start: Math.round(start), end: Math.round(px("--title-bar-end") + 16) } }
}
/** @param {import("./host").TitleBar} at */
function titled({ height, start, end }) {
  document.documentElement.style.setProperty("--title-bar-height", `${height}px`)
  document.documentElement.style.setProperty("--title-bar-start", `${start}px`)
  document.documentElement.style.setProperty("--title-bar-end", `${end}px`)
  document.documentElement.toggleAttribute("data-title-bar", true)
  titleBar.hidden = false
  byId("title-bar-nav").hidden = !own
  crumb()
  // Open apps keep it clear too, as it changes.
  placed()
}
/// The breadcrumb's last: this window's app, by its icon and name, once its apps are listed.
function crumb() {
  if (!own) return
  const [picture, name] = byId("title-bar-app").children
  const src = icons.get(own.entrypoint)
  if (src) /** @type {HTMLImageElement} */ (picture).src = src
  picture.toggleAttribute("hidden", !src)
  name.textContent = names.get(own.entrypoint) || own.name || ""
  // The page's top bar starts after it, wherever its name ends.
  placed()
}
/// Where it was pressed, until the pointer moves far enough from there to drag the window, as a title bar's own does:
/// a drag that started at the press would take the pointer from the page (a Linux window manager's does), and the
/// second press of a double-click with it.
/** @type {{ x: number, y: number } | undefined} */
let pressedAt
/// A press in the title row on nothing that takes it, on the window's own page (an app's page says its own).
const inTitleRow = (/** @type {MouseEvent} */ event) =>
  !titleBar.hidden &&
  event.button === 0 &&
  event.clientY < titleBar.offsetHeight &&
  !(event.target instanceof Element && event.target.closest("a, button, input, iframe, [role=button], [tabindex]"))
document.addEventListener("mousedown", (event) => {
  if (!inTitleRow(event)) return
  event.preventDefault()
  pressedAt = event.detail === 1 ? { x: event.clientX, y: event.clientY } : undefined
})
document.addEventListener("mousemove", (event) => {
  if (!pressedAt || !(event.buttons & 1) || Math.hypot(event.clientX - pressedAt.x, event.clientY - pressedAt.y) < 4) return
  pressedAt = undefined
  invoke("plugin:window|start_dragging")
})
document.addEventListener("mouseup", (event) => {
  pressedAt = undefined
  if (event.detail === 2 && inTitleRow(event)) invoke("title_bar_double_click")
})
/// Goes to the Home Screen from an app's own window, keeping it open: the breadcrumb's Home, as Window → Home Screen
/// does (`host.rs`).
function showHomeScreen() {
  invoke("front")
}
byId("home-screen").addEventListener("click", showHomeScreen)
// A Mac's Window → Home Screen is ⇧⌘H (`host.rs`), which its tooltip says, as a Mac app's toolbar buttons do.
if (navigator.platform.startsWith("Mac")) byId("home-screen").title = "Home Screen (⇧⌘H)"

dress()
shown.subscribe(draw)
shown.start()
// The window isn't in use once another window, or another app, has the keyboard: not one of its own frames, which
// takes it from this page too, but leaves it the document's (`hasFocus`).
addEventListener("blur", () => setTimeout(() => shown.send({ type: "unused" })))
await listen("state", ({ payload }) => hear(payload))
await listen("title_bar", ({ payload }) => titled(payload))
// Where the window starts: where its device is with its network, as the client says it (`State`), which it follows
// from then on. A phone's app hears a tap on a notification through its push plugin.
const { taps, testflight: fromTestflight, state, open: link, windows: inWindows, runtimeVersion: drawn, window: ownWindow, titleBar: overTitleBar, theme: kept, unnative } = await invoke("start")
phone = Boolean(taps)
testflight = Boolean(fromTestflight)
windows = Boolean(inWindows)
runtimeVersion = Number(drawn) || 0
own = ownWindow ?? undefined
if (own?.name) document.title = own.name
if (overTitleBar) titled(overTitleBar)
heardTheme(kept)
hear(state)
if (link) on({ type: "link", link })
if (unnative) on({ type: "unnative", why: unnative })
invoke("visible", { onScreen: !document.hidden }).catch(() => {})
if (taps) hearTaps()
