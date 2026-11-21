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
import { address, asksToReload, invoke, listen, sandbox } from "./host.js"
import { appStore, files, first, launcher, list, listed, names, needs, settings } from "./apps.js"
import { post } from "./bridge.js"
import { chosen, dress, heardTheme, titled } from "./chrome.js"
import { draw } from "./draw.js"
import { gestures } from "./gestures.js"
import { moveMachine } from "./move-machine.js"
import { pageMachine } from "./page-machine.js"
import { rail } from "./rail.js"
import { used } from "./shortcuts.js"
import { byId } from "./util.js"
import { machine } from "./window-machine.js"
/** @import { Moved, Paged, Rect, State, View } from "./host.js" */

/// On a computer, each app opens in a window of its own, as the computer's own app (`windows`, `hosts.rs`): the
/// Home Screen's window shows the launcher alone, and an app's own window shows that app (`own`), each where the
/// launcher is in a phone's. Either opens every other app in its own window.
export let windows = false
/// On a phone, the runtime version of the renderer the app draws every app natively with, over this page
/// (`runtimeVersion` in src/lib.rs): an app whose page needs a newer one says so, as this app needs updating. 0 where
/// it doesn't, as in a browser, which draws each app in a frame here with the renderer it loads.
let runtimeVersion = 0
/// Why a phone can't open an app's page natively: its page needs a newer renderer than the app's, or newer tables.
const NEWER = "It needs a newer version of the One App. Update it, then open it again."
/// On a phone, the launcher is drawn natively, as its root window, under every app's (`open_launcher`).
export let nativeLauncher = false
/// Whether the native launcher was last told it shows, which it doesn't while an app in a frame here is in front, and
/// whether something is open over its page, which back shuts first.
let launcherShown = true
export let launcherOver = false
/** @type {{ entrypoint: string, subject?: string, name?: string | null } | undefined} */
export let own
/// The page under every other in this window: the launcher's, or in an app's own window, that app's.
const base = () => own?.entrypoint ?? launcher
/// What the page under every other is called where the window says it's opening it, or that it didn't open.
export const baseName = () => (own ? names.get(own.entrypoint) || "This app" : "Home Screen")
/// A computer's launcher (`computer.rs`): this page in a panel the system summons, whose app is the launcher alone.
/// It's put away as an app opens from it, on Escape, and once the launcher is done with it (`leave`).
export const panel = () => windows && !own
export const baseKey = () => key(base(), own?.subject)
/// The apps that are open, by who each is for and what it is: `{ key, entrypoint, subject, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
/** @type {Map<string, View>} */
export const open = new Map()
/** @param {string} entrypoint @param {string} [subject] */
export const key = (entrypoint, subject) => `${subject ?? ""}|${entrypoint}`
/// The pages in an app's frames (the UI library's `Frame`), each another app's, answered as its own app's page is,
/// by the number its address has (`frame`): known by its window once it has said it from a frame of the page it's in.
/** @type {Map<string, View & { in: View, window?: Window }>} */
export const nested = new Map()

/// What an app's address is named by: its app's name, then who it's for (its subject) if that isn't its user, last part first.
/** @param {string} entrypoint @param {string} [subject] */
export const host = (entrypoint, subject) => [entrypoint.split("/")[0], ...(subject ? subject.split("/").reverse() : [])].join(".")

/// A phone's app is its network's, which its owner builds and updates, and a computer's updates itself: what
/// `start` says.
export let phone = false
/// Whether this is an iPhone's app from TestFlight, whose Update opens TestFlight.
export let testflight = false



const { createActor } = XState



export const shown = createActor(machine)
/** @param {Moved} event */
export const on = (event) => shown.send(event)

/** @param {View} view @param {Paged} event */
export const page = (view, event) => on({ type: "page", key: view.key, paged: event })
/// Where an app's page is now, as the window's child.
/** @param {View} view */
const pageOf = (view) => /** @type {import("xstate").SnapshotFrom<typeof pageMachine> | undefined} */ (shown.getSnapshot().children[view.key]?.getSnapshot())
// The window hidden, every page is off screen; shown again, the one in front is on screen.
document.addEventListener("visibilitychange", () => on({ type: document.hidden ? "away" : "seen" }))
/// Where the window's move is now.
export const moving = () => /** @type {import("xstate").SnapshotFrom<typeof moveMachine> | undefined} */ (shown.getSnapshot().children.move?.getSnapshot())
/// Whether an app's page is in front of its person: the window's answer, the one showing while it shows apps, in a
/// window that's on screen.
/** @param {View} view @returns {boolean} */
export const onScreen = (view) => !document.hidden && shown.getSnapshot().hasTag("pages") && view === showing(here())





/// Where an app's page is, with the Colour Mode chosen on this device (`theme`), which the UI library puts the page in
/// before it draws (its `colour-mode.js`), so a dark choice on a light device never shows white while it loads.
/** @param {Pick<View, "host" | "entrypoint" | "subject">} view */
export const located = (view) => {
  const theme = chosen()
  return theme ? `${address(view)}?theme=${theme}` : address(view)
}

/// Loads an app's page again, in place, so the way back still leads out of the app: told first, as an MCP App is
/// before its host takes it down, and what it was answered forgotten.
/** @param {View} view */
export function reloadPage(view) {
  post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
  invoke("close", { view: view.key }).finally(() => /** @type {Window} */ (view.frame.contentWindow).location.replace(located(view)))
}

/// An app whose native window didn't open, and why, until it's tried again or another opens.
/** @type {{ entrypoint: string, link: string, subject?: string, why: string } | undefined} */
let unopened
/// Says over the app that's showing what its page needs of its person, if anything: that it's been updated, with
/// Reload, or that it didn't open, with Try Again: only while the window shows its apps, as it alone owns the screen.
/// An app whose native window didn't open says so over what's showing.
export function notes() {
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
export let front

/// The way back through this window: for each of its history entries from the launcher's, the key of the app it
/// shows (`null` for the launcher's), where it is in the browser's whole history, which counts the entries an app's
/// own screens add too, and the icon it was opened from, which it goes back into. An entry says how deep it is
/// (`history.state`), so going back or forward knows which way it goes, and a link to the launcher goes all the way
/// back to it.
/** @type {{ key: string | null, at: number, from?: Rect }[]} */
export let stack = [{ key: null, at: history.length - 1 }]
/// How deep the entry the window is on is: 0 at the launcher's.
export let depth = 0
/// The key of the app the window's entry shows, or `null` for the launcher.
export const here = () => stack[depth]?.key ?? null
/// Goes back to the launcher's history entry, past each app's screens on the way, the one showing's too: where the
/// Navigation API is, straight to it, as it lists the window's own entries apart from its apps'; elsewhere by as many
/// as the window and the apps under the one showing added.
export function backHome() {
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
export function bar() {
  const shown = front?.statusBar !== false
  if (shown === statusBar) return
  statusBar = shown
  invoke("status_bar", { shown })
}

/// Where `rect`, a place in a frame's page, is in the window: an icon an app is opened from.
/** @param {HTMLIFrameElement} frame @param {Rect} [rect] @returns {Rect | undefined} */
export function within(frame, rect) {
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
export function reveal(view, tapped) {
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
export function layer(at) {
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


/// The app that's open as `at`, or with none there, the one under every other: the launcher, or while there's none,
/// the App Store.
/** @param {string | null} at */
export const showing = (at) => open.get(at ?? "") ?? open.get(shown.getSnapshot().matches("app_store") ? key(appStore) : baseKey())

///


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
export function opened(view) {
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






/// Shows an app over whatever's showing, by what a `one:` link says after its scheme: `<app>[/<page>][#<fragment>]`,
/// its app's name, and its page if it isn't the app's first app, and the place in it, for `subject`. With `behind`
/// it only starts. The launcher, linked to, is gone back to, past every app open over it. → whether there is one
/// here.
/** @param {string} link @param {string} [subject] @param {boolean} [behind] @param {Rect} [from] */
export function go(link, subject, behind, from) {
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
  // On a phone, an app opens natively, over this page. One whose native window didn't open, or that needs a newer
  // renderer than this app's, says why, as a frame that didn't open does (`notes`). Its window is told where its icon
  // is, which pulled home by its rail's line it goes back into. A page that names no runtime version (an app's page
  // that's no entrypoint, as a sign-in page) opens in a frame here until every phone's app draws those natively too.
  if (runtimeVersion && needs.has(page) && key(page, subject) !== baseKey()) {
    if (behind) return true
    /** @param {unknown} why */
    const failed = (why) => {
      unopened = { entrypoint: page, link, subject, why: String(why) }
      notes()
    }
    if ((needs.get(page) ?? 0) > runtimeVersion) return failed(NEWER), true
    unopened = undefined
    notes()
    /** @type {[number, number, number, number] | undefined} */
    const icon = from && [from.x, from.y, from.width, from.height]
    invoke("open_window", { entrypoint: page, subject, link: fragment ? `${page}#${fragment}` : page, icon }).then((opened) => opened || failed(NEWER), failed)
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
export function start() {
  const home = open.get(baseKey())
  if (home) page(home, { type: "retry" })
  else if (nativeLauncher) on({ type: "drawn", key: baseKey() })
  else if (phone && !own && runtimeVersion) openLauncher()
  else load(base(), own?.subject)
  // The app that was showing when the page was loaded again is gone, so the launcher shows.
  restart()
  show(null, 0)
}

/// Opens the launcher natively, as the phone's root window: drawn, it's the launcher here; one that needs a newer
/// renderer, or doesn't open, says why.
function openLauncher() {
  /** @param {unknown} why */
  const failed = (why) => on({ type: "failed", key: baseKey(), why: String(why) })
  if ((needs.get(launcher) ?? 0) > runtimeVersion) return failed(NEWER)
  // An app before 21 Nov answers whether it opened, and says nothing of why not: its page loads from the network, so it
  // meets this one. Nothing of what it answers is used from the first TestFlight build after this one on.
  invoke("open_launcher", { entrypoint: launcher }).then((opened) => {
    if (opened === false) return failed(NEWER)
    nativeLauncher = launcherShown = true
    on({ type: "drawn", key: baseKey() })
  }, failed)
}
// The native launcher closed: its page didn't open after all, or crashed once it had, which the window says, with
// why (a build before 21 Nov says none: its page needed a newer renderer); or its servers are ahead (`behind`), which
// it says as Update Required (`tooOld`).
listen("launcher", ({ payload }) => {
  if (payload?.native === false && nativeLauncher) {
    nativeLauncher = false
    if (!payload.behind) on({ type: "failed", key: baseKey(), why: payload.why ?? NEWER, crashed: payload.crashed === true })
  }
  if (typeof payload?.over === "boolean") (launcherOver = payload.over), gestures()
})
// The native launcher opens an app, or starts it as a finger lands on its icon, as its frame's page asks (`go`).
listen("go", ({ payload }) => nativeLauncher && go(payload.link, payload.subject ?? undefined, payload.behind, payload.from ?? undefined))

/// Shows the App Store with no launcher under it, which its person installs one with. There's nothing to go back
/// to from it.
export function openAppStore() {
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
