// The window's own page: the frames apps are drawn in, and the bridge between each and the client.
//
// An app is a page of its plugin's, in a frame at an address of its own, so it reaches no other app and nothing
// here. It speaks to this page as an MCP App speaks to its host: JSON-RPC messages, posted. What it asks goes to
// where the page runs (`host.js`): the app's Rust half, or in a browser, the network's edge (`clients/web/host.js`),
// which answers what only reads from what it kept and says when that has changed.
//
// The launcher is an app too, which reaches this client from a server as every plugin's does: the app in the category
// `launcher`, which a network has exactly one of. It's the one this page shows first, under every other, and it
// opens the rest (`one/open`).
import { address, invoke, listen, openLink, sandbox } from "./host.js"
/** @import { Moved, Shown, Told, View } from "./host.js" */

/// Reports what went wrong here to Telemetry, through the client, which keeps it until a server can be told: what
/// this page showed its person, an error of its own or of an app's page, an app that never drew. Names, files and
/// what was shown only, never what anyone typed.
/** @param {string} name @param {string} error @param {Record<string, unknown>} [attributes] */
const report = (name, error, attributes = {}) => invoke("report", { record: { name, error, attributes } }).catch(() => {})
/// An error as OpenTelemetry's `exception` says one: its message, and the file and line it was thrown at, by its
/// path alone, which leaves out anything a query had in it.
/** @param {unknown} message @param {unknown} [file] @param {unknown} [line] @param {string} [plugin] */
function exception(message, file, line, plugin) {
  const path = typeof file === "string" && URL.canParse(file) ? new URL(file).pathname : undefined
  report("exception", String(message).slice(0, 500), { "code.file.path": path, "code.line.number": typeof line === "number" ? line : undefined, "one.plugin": plugin })
}
addEventListener("error", ({ message, filename, lineno }) => exception(message, filename, lineno))
addEventListener("unhandledrejection", ({ reason }) => exception(reason?.message ?? reason))
/// How long an app may take from its tap to drawing before it's reported as never having opened.
const NEVER = 15000
/** An element of the page's own, which is there. @param {string} id */
const byId = (id) => /** @type {HTMLElement} */ (document.getElementById(id))
/// The launcher's page, as this user's apps list it, once they're listed: `<plugin>/<page>`.
let launcher = ""
/// The room the launcher keeps along the start of its frame beside the app that's showing, which that app's frame
/// leaves clear: a CSS length, as the launcher says (`notifications/one/beside`), or none.
let room = ""
/// The apps that are open, by who each is for and what it is: `{ key, app, subject, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
/** @type {Map<string, View>} */
const open = new Map()
/** @param {string} app @param {string} [subject] */
const key = (app, subject) => `${subject ?? ""}|${app}`
/// Each plugin's first app, by the plugin's name, among this user's, and one that isn't a part of Settings if it
/// has one: what a notification from it opens.
/** @type {Map<string, string>} */
let first = new Map()
/// Who this client has listed files for: its user (`undefined`), and each group or the network an app was opened
/// for.
/** @type {Set<string | undefined>} */
const listed = new Set()

/// What an app's address is named by: its plugin's name, then who it's for (its subject) if that isn't its user, last part first.
/** @param {string} app @param {string} [subject] */
const host = (app, subject) => [app.split("/")[0], ...(subject ? subject.split("/").reverse() : [])].join(".")

/// A phone's app is its network's, which its owner builds and updates, and a computer's updates itself: what
/// `start` says.
let phone = false

/// Light or dark, as Settings › Appearance has it on this device, or as the device is.
const scheme = matchMedia("(prefers-color-scheme: dark)")
const chosen = () => localStorage.getItem("theme")
const theme = () => chosen() ?? (scheme.matches ? "dark" : "light")
function dress() {
  document.documentElement.style.colorScheme = chosen() ?? ""
  for (const name of ["light", "dark"]) document.documentElement.classList.toggle(name, theme() === name)
  for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { theme: theme() } })
}
scheme.addEventListener("change", dress)

/// What the window shows of itself, one state at a time, which only `on` moves: while its device's session opens,
/// the steps it takes to its person's home; their home, once the launcher has drawn; or why it can't show, and an
/// update under way. While the device isn't in its network, the window is on the network's own sign-in page
/// instead, which the client brings it back from (`follows` in `src/lib.rs`).
/** @type {Shown} */
let shown = { state: "opening", step: 0 }

/// The moves there are, from `now` on `event`: the next state, or nothing where `event` moves nothing from `now`.
/** @param {Shown} now @param {Moved} event @returns {Shown | undefined} */
function next(now, event) {
  // Whether it waits for its apps to be listed: as it opens, or again after it updated or couldn't show home.
  const listing = (now.state === "opening" && now.step === 1) || now.state === "updating" || now.state === "unlisted" || now.state === "tooOld"
  switch (event.type) {
    // The device's session moved: it's opening again, it's in and lists its apps next, or its network didn't answer.
    case "session":
      if (event.session.state === "opening") return { state: "opening", step: 0 }
      if (event.session.state === "unreachable") return { state: "unreachable" }
      if (event.session.state === "signedIn" && (now.state === "unreachable" || (now.state === "opening" && now.step === 0))) return { state: "opening", step: 1 }
      return
    // Its apps were listed with the launcher among them, which loads next, or without one, or not at all, or the
    // launcher never drew.
    case "listed":
      return listing ? { state: "opening", step: 2 } : undefined
    case "unlisted":
      return listing || (now.state === "opening" && now.step === 2) ? { state: "unlisted", why: event.why } : undefined
    case "tooOld":
      return listing && now.state !== "tooOld" ? { state: "tooOld" } : undefined
    // The launcher drew: home shows.
    case "drawn":
      return now.state === "opening" && now.step === 2 ? { state: "home" } : undefined
    // Its person tried again, or pressed Update.
    case "retry":
      return now.state === "unlisted" ? { state: "opening", step: 1 } : now.state === "updateFailed" ? { state: "updating" } : undefined
    case "update":
      return now.state === "tooOld" ? { state: "updating" } : undefined
    // What a device that's also a server was told as it asked its server to update, or why an update failed.
    case "updated":
      return now.state === "updating" ? { state: "updating", message: event.message } : undefined
    case "updateFailed":
      return now.state === "updating" ? { state: "updateFailed", why: event.why } : undefined
  }
}

/// Moves what the window shows on `event`, if that's a move there is, draws it, and starts what the new state does.
/** @param {Moved} event */
function on(event) {
  const moved = next(shown, event)
  if (!moved) return
  const was = shown
  shown = moved
  draw()
  if (moved.state === "opening" && moved.step === 1) list()
  if (moved.state === "opening" && moved.step === 2) start()
  if (moved.state === "updating" && was.state !== "updating") {
    // A computer's app takes the latest builds and starts again on them; one that's also a server says what it did.
    invoke("update").then(
      (message) => on({ type: "updated", message }),
      (error) => on({ type: "updateFailed", why: `${error}` }),
    )
  }
}

/// The steps the window takes as it opens, each ticked as it's done.
const STEPS = ["Connecting to your network", "Getting your apps", "Opening Home Screen"]

/// What each problem says: its heading, its one sentence, and the button that fixes it, if there is one.
/** @param {Shown} now @returns {[string, string, string | undefined] | undefined} */
function problemOf(now) {
  switch (now.state) {
    case "unreachable":
      return ["Can't reach your network", "Check this device is online.", "Try Again"]
    case "unlisted":
      return ["Can't show your Home Screen", now.why, "Try Again"]
    case "tooOld":
      return ["This app needs updating", phone ? "Its owner updates this app." : "An update is ready.", phone ? undefined : "Update"]
    case "updateFailed":
      return ["Couldn't update", now.why, "Try Again"]
  }
}

/// Draws what the window shows of itself, in its middle, over the frames: its mark and steps while it opens, a
/// spinner and a line while it updates, or a problem. Home is the frames alone, and whether a server is answering.
/** @type {string | undefined} */
let lastProblem
function draw() {
  const now = shown
  byId("state").hidden = now.state === "home"
  byId("mark").hidden = byId("steps").hidden = now.state !== "opening"
  if (now.state === "opening") {
    byId("steps").replaceChildren(
      ...STEPS.map((step, i) => {
        const item = Object.assign(document.createElement("li"), { className: i < now.step ? "done" : i === now.step ? "now" : "" })
        const mark = i < now.step ? icon(CHECK) : i === now.step ? spinner() : Object.assign(document.createElement("span"), { className: "next" })
        item.append(mark, Object.assign(document.createElement("span"), { textContent: step }))
        return item
      }),
    )
  }
  byId("status").hidden = now.state !== "updating"
  if (now.state === "updating") byId("status-text").textContent = now.message ?? ""
  byId("status-text").hidden = !(now.state === "updating" && now.message)
  const problem = problemOf(now)
  byId("problem").hidden = !problem
  if (problem) {
    const [heading, text, action] = problem
    byId("problem-heading").textContent = heading
    byId("problem-text").textContent = text
    byId("problem-action").hidden = !action
    byId("retry").textContent = action ?? ""
    byId("retry").dataset.variant = action === "Update" ? "default" : "outline"
  }
  // What's shown is reported once, as it's shown.
  const said = problem && `${problem[0]}: ${problem[1]}`
  if (said && said !== lastProblem) report("one.client.problem", said)
  lastProblem = said
  status()
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
const spinner = () => /** @type {HTMLElement} */ (byId("connection-spinner").cloneNode(true))

/// Whether a server answered the last thing this client asked, and since when none has.
let answering = true
let unanswered = 0

/// How long no server answering goes unsaid, as a moment's gap while the client reconnects by itself, and how long
/// before the window says it's showing what was saved.
const BLIP = 3000
const LONG = 15000
/** @type {ReturnType<typeof setTimeout> | undefined} */
let saying
/// Whether a server is answering, while home shows: one that is, or none for a blip, which say nothing; then none for
/// a few seconds, reconnecting, and for longer, which says it shows what was saved; and the device offline.
/** @returns {"answering" | "blip" | "reconnecting" | "waiting" | "offline"} */
function connection() {
  if (answering) return "answering"
  if (!navigator.onLine) return "offline"
  const waited = performance.now() - unanswered
  return waited < BLIP ? "blip" : waited < LONG ? "reconnecting" : "waiting"
}
/// Says the connection in one quiet line at the foot of the window, over whatever's showing: a spinner and that it's
/// reconnecting, and nothing while it's fine. A blip and reconnecting each end by the clock.
function status() {
  clearTimeout(saying)
  const now = shown.state === "home" ? connection() : "answering"
  byId("connection").hidden = now === "answering" || now === "blip"
  byId("connection-spinner").hidden = now === "offline"
  byId("connection-text").textContent = { answering: "", blip: "", reconnecting: "Reconnecting…", waiting: "Reconnecting… Showing what was saved.", offline: "You're offline. Showing what was saved." }[now]
  if (now === "blip" || now === "reconnecting") saying = setTimeout(status, (now === "blip" ? BLIP : LONG) - (performance.now() - unanswered))
}
addEventListener("offline", status)
// Back online, the client asks at once, rather than when it next would have.
addEventListener("online", () => {
  status()
  if (!answering) invoke("refresh", { view: key(launcher) })
})

/// Whether this device is behind the latest build of its program for its kind of machine, as its network says
/// (`builds`): a browser, or an app whose network says nothing of its build, isn't.
async function behind() {
  const { build, target } = await invoke("client")
  if (!build || !target) return false
  const response = /** @type {{ structuredContent?: { latest?: Record<string, { build: string }> } }} */ (await invoke("request", { view: "client", method: "tools/call", params: { name: "builds" } }))
  const latest = response.structuredContent?.latest?.[`one/${target}`]?.build
  return Boolean(latest) && latest !== build
}

/// Lists the files a server has for `subject`, which the client keeps, so an app opens with no server up. An open app
/// whose plugins have a new version loads again. This user's own list says whether home can show: with a launcher
/// in it, without one, or not at all. A launcher missing from a later list, as during a release, leaves the one that's
/// open as it is. Its list missing a launcher says this app needs updating only where its network says an update is
/// ready for it (`behind`): otherwise it's the network that has none right now.
/** @param {string} [subject] */
async function list(subject) {
  listed.add(subject)
  try {
    const { resources, outdated } = await invoke("apps", { subject })
    for (const view of open.values()) if (outdated.includes(view.host)) view.stale = true
    if (!subject) {
      // An app is a page of an MCP App's kind, at ui://<plugin>/<version>/<page>.
      const pages = resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app")
      const named = (/** @type {string} */ uri) => {
        const [plugin, _version, ...page] = uri.slice("ui://".length).split("/")
        return `${plugin}/${page.join("/")}`
      }
      // The last one set for a plugin is the one kept: its first, and one with no category before one with one.
      const order = pages.toReversed().sort((a, b) => Number(Boolean(b._meta?.category)) - Number(Boolean(a._meta?.category)))
      first = new Map(order.map(({ uri }) => [named(uri).split("/")[0], named(uri)]))
      const found = pages.find(({ _meta }) => _meta?.category === "launcher")
      if (found) launcher = named(found.uri)
      if (launcher) on({ type: "listed" })
      else on((await behind().catch(() => false)) ? { type: "tooOld" } : { type: "unlisted", why: "Your network has no Home Screen right now." })
    }
  } catch (error) {
    if (!subject) on({ type: "unlisted", why: `${error}`.replace(/[^.]$/, "$&.") })
  }
  reload()
}

/// Loads again each open app that has a new version, as a web app picks up its own, unless its person may be in
/// the middle of something there: it's the one showing, and has the keyboard. That one says an update is ready, and
/// loads again once they press Reload, leave it, or put the window away. With `now`, every one does.
async function reload(now = false) {
  for (const view of open.values()) {
    const busy = !document.hidden && view === showing(history.state) && document.activeElement === view.frame
    if (!view.stale || (busy && !now)) continue
    view.stale = false
    post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
    await invoke("close", { view: view.key })
    view.drawn = false
    // Loaded again in place, so the way back still leads out of the app.
    const page = /** @type {Window} */ (view.frame.contentWindow)
    page.location.replace(address(view))
  }
  byId("update").hidden = ![...open.values()].some(({ stale }) => stale)
}
byId("reload").addEventListener("click", () => reload(true))

/// Starts an app: its page loads in a frame of its own, behind what's showing until it's shown.
/** @param {string} app @param {string} [subject] @returns {View} */
function load(app, subject) {
  const at = key(app, subject)
  const opened = open.get(at)
  if (opened) return opened
  if (!listed.has(subject)) list(subject)
  const place = { key: at, app, subject, host: host(app, subject) }
  const view = { ...place, frame: Object.assign(document.createElement("iframe"), { src: address(place), inert: true }) }
  view.frame.setAttribute("sandbox", sandbox)
  if (app === launcher) view.frame.className = "launcher"
  // An app hands a link to the device's share sheet, or copies it, as its person presses Share (the UI library's `share`),
  // and shows a part of itself full screen (a live screen's Full Screen).
  view.frame.allow = "clipboard-write; web-share; fullscreen"
  open.set(at, view)
  document.body.append(view.frame)
  return view
}

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let shownByDevice = false

/// Changes between the launcher and an app as a move, as the UI library moves between an app's screens
/// (core/ui/src/screen.tsx): the browser slides the app in over the launcher, or with `back`, back out from over
/// them. Where the browser can't, or the user asked for no motion, it's a plain swap.
/** @param {() => void} change */
function move(change, back = false) {
  const plain = !globalThis.ViewTransition || !("types" in ViewTransition.prototype) || matchMedia("(prefers-reduced-motion: reduce)").matches
  // Beside the launcher's sidebar, an app takes the place of the last, as a sidebar's apps do.
  if (plain || shownByDevice || room) return void change()
  document.startViewTransition({ update: change, types: [back ? "back" : "forward"] })
}

/// Shows the app that's open as `at` over the rest, or with none there, the launcher. `tapped` is when its icon
/// was tapped.
/** @param {string | null} at @param {number} tapped */
function show(at, tapped) {
  const view = layer(at)
  reload()
  if (!view) return
  view.tapped = tapped
  setTimeout(() => {
    if (view.tapped !== tapped || view.drawn) return
    report("one.client.open", `Didn't open in ${NEVER / 1000} s`, { "one.view": view.app.split("/")[0] })
    // A launcher that never draws leaves nothing to show: trying again loads it again.
    if (view.app === launcher) on({ type: "unlisted", why: "It didn't open. Try again in a moment." })
  }, NEVER)
  if (!view.drawn) return
  // Its user returned to it.
  invoke("refresh", { view: view.key })
  opened(view)
}

/// Has the app that's open as `at` showing, or with none there, the launcher, and the launcher told which, for what it
/// keeps beside every app where it keeps room. → the one showing.
/** @param {string | null} at */
function layer(at) {
  const view = showing(at)
  const home = open.get(key(launcher))
  for (const other of open.values()) other.frame.inert = other !== view && !(other === home && room)
  if (home && view) post(home, { method: "notifications/one/showing", params: { app: view.app, subject: view.subject } })
  gestures()
  return view
}

/// Whether the device's own back and forward gestures are on: an iPhone's swipe in from the edge, a trackpad's swipe.
/// They go through this window's history, so they're off while something is open over the app that's showing, as a
/// phone's are under a sheet, and its Back or Escape shuts that first.
let swiping = true
function gestures() {
  const on = !showing(history.state)?.over
  if (on !== swiping) invoke("navigation_gestures", { on })
  swiping = on
}
/// The app that's open as `at`, or with none there, the launcher.
/** @param {string | null} at */
const showing = (at) => open.get(at ?? "") ?? open.get(key(launcher))

/// Says how long an app took from the tap on its icon to its content on screen: the second frame after it's
/// showing with what it drew from. The launcher's time is from when this page began to load.
/** @param {View} view */
function opened(view) {
  if (view.tapped === undefined) return
  const tapped = view.tapped
  view.tapped = undefined
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const name = view.app.split("/")[0]
    const { duration } = performance.measure(`open ${name}`, { start: tapped })
    invoke("opened", { view: name, ms: Math.round(duration) })
    // The device hears its person's notifications once they see their home, which has drawn over the steps.
    if (view.app === launcher) {
      on({ type: "drawn" })
      invoke("listen")
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

/** @param {View} view @param {Record<string, unknown>} message */
const post = (view, message) => view.frame.contentWindow?.postMessage({ jsonrpc: "2.0", ...message }, "*")

/// What this page answers an app itself. The rest is the client's to answer.
/** @param {View} view @param {string} method @param {Record<string, any>} params */
async function respond(view, method, params) {
  if (method === "ui/initialize") {
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
        safeAreaInsets: { top: 0, right: 0, bottom: 0, left: 0 },
      },
    }
  }
  if (method === "ping") return {}
  if (method === "ui/request-display-mode") return { mode: "fullscreen" }
  // A file too large to go in a call: the client sends it where the network keeps it, for a tool of the app's plugin.
  if (method === "one/upload") return invoke("upload", await params.file.arrayBuffer(), { headers: { plugin: view.app.split("/")[0] } })
  // A person's recovery key is shown and typed on this page, which is the client's own: no app's page is handed one.
  if (method === "one/recovery-key") return recoveryKey(params.typed === true, params.subject ?? view.subject)
  // Another app, shown over this one, as tapping its icon or a notification from it does: by its plugin's name,
  // and its page if it isn't the plugin's first app, for whoever this one is for unless it says. `behind` starts
  // it as a finger lands on its icon, and it shows as the finger lifts.
  if (method === "ui/open-link" && params.url?.startsWith("one:")) {
    if (!go(params.url.slice("one:".length), params.subject ?? view.subject, params.behind)) throw new Error("That app isn't on this device")
    return {}
  }
  // A web address opens in the device's own browser, outside the app.
  if (method === "ui/open-link" && /^https?:/.test(new URL(params.url).protocol)) {
    openLink(params.url)
    return {}
  }
  // What an app is told of its client, and the few things one changes of it.
  if (method === "one/client") {
    if (params.leave) return invoke("sign_out")
    // A phone whose push failed tries again.
    if (params.push) return invoke("push")
    // This device becomes a server of its person's, or stops being one, or takes the latest builds.
    if ("serve" in params) return { message: await invoke("serve", { on: params.serve === true }) }
    if (params.update) return { message: await invoke("update") }
    if ("theme" in params) {
      if (params.theme) localStorage.setItem("theme", params.theme)
      else localStorage.removeItem("theme")
      dress()
    }
    return { ...(await invoke("client")), theme: chosen() }
  }
  if (method.startsWith("ui/")) throw Object.assign(new Error(`This client has no ${method}`), { code: -32601 })
  // An app opened from a group's page is answered for the group.
  const subject = params._meta?.subject ?? view.subject
  if (subject && !listed.has(subject)) list(subject)
  return invoke("request", { view: view.key, method, params: subject ? { ...params, _meta: { ...params._meta, subject } } : params })
}

addEventListener("message", async ({ source, data }) => {
  const view = [...open.values()].find(({ frame }) => frame.contentWindow === source)
  if (!view || typeof data?.method !== "string") return
  const { id, method, params = {} } = data
  if (id === undefined) {
    // The app's user went back out of it. The app stays open behind the launcher.
    if (method === "ui/notifications/request-teardown" && !view.frame.inert && history.state) history.back()
    // Something is open over its page, or nothing is any more.
    if (method === "notifications/one/over") {
      view.over = params.over === true
      gestures()
    }
    // The launcher, loaded again, is told which app is showing beside it.
    if (method === "ui/notifications/initialized" && view.app === launcher) layer(history.state)
    // The launcher says how much room it keeps beside other apps, as the window's size changes.
    if (method === "notifications/one/beside" && view.app === launcher) {
      room = typeof params.width === "string" ? params.width : ""
      document.body.style.setProperty("--beside", room || "0px")
      document.body.toggleAttribute("data-beside", Boolean(room))
      layer(history.state)
    }
    // An app's page logs its errors as an MCP App logs to its host, and they're its plugin's, whatever it says.
    if (method === "notifications/message" && params.level === "error") exception(params.data?.message, params.data?.file, params.data?.line, view.app.split("/")[0])
    return
  }
  try {
    post(view, { id, result: await respond(view, method, params) })
  } catch (error) {
    const { code, message } = /** @type {{ code?: number, message?: string }} */ (error)
    post(view, { id, error: { code: code ?? -32000, message: message ?? String(error) } })
  }
  // It has what it draws from once the client has answered it something.
  if (method.startsWith("ui/") || method.startsWith("one/")) return
  view.drawn = true
  if (!view.frame.inert) opened(view)
})

/// Shows an app over whatever's showing: by its plugin's name, and its page if it isn't the plugin's first app,
/// for `subject`. With `behind` it only starts. → whether there is one here.
/** @param {string} app @param {string} [subject] @param {boolean} [behind] */
function go(app, subject, behind) {
  const page = app.includes("/") ? app : first.get(app)
  if (!page) return false
  const tapped = performance.now()
  const next = load(page, subject)
  if (behind) return true
  const showing = history.state === next.key
  move(() => show(next.key, tapped))
  // The way back out of an app is an entry in this window's history, made as it's opened, so the device's back
  // leads out of the app however soon it's pressed.
  if (!showing) history.pushState(next.key, "")
  return true
}

// A notification from an app was tapped.
listen("open", ({ payload: plugin }) => go(plugin))
/// A phone's push plugin says a notification was tapped, even the tap that started the app, once this page hears
/// it, which is once there are apps to open.
async function hearTaps() {
  await window.__TAURI__.core.addPluginListener("notifications", "notificationClicked", (tapped) => invoke("tapped", { tapped }))
  await invoke("plugin:notifications|set_click_listener_active", { active: true })
}

listen("changed", ({ payload: view }) => {
  if (view === "client") for (const subject of listed) list(subject)
  else if (open.has(view)) post(/** @type {View} */ (open.get(view)), { method: "notifications/one/changed" })
})

/// The device's own back, where a device has one (the app's window there calls this): what's open over the app
/// that's showing shuts first. → whether anything was. If nothing was, the device goes back through this window's
/// history, which leads back through an app's screens and out of it, and from the launcher out of the app.
window.back = () => {
  const view = open.get(history.state) ?? open.get(key(launcher))
  if (!view?.over || view.frame.inert) return false
  post(view, { method: "notifications/one/back" })
  return true
}

// The device's own way back, and this page's, both end here.
addEventListener("popstate", (event) => {
  const { state } = event
  const tapped = performance.now()
  shownByDevice = Boolean(event.hasUAVisualTransition)
  move(() => show(state, tapped), !open.has(state))
  shownByDevice = false
})

listen("answering", ({ payload }) => {
  const back = !answering && payload
  if (answering && !payload) unanswered = performance.now()
  answering = payload
  status()
  // What couldn't be listed is asked for again now that a server answers, and so is what a server that updated
  // lists once it's back.
  if (back && (shown.state === "unlisted" || shown.state === "updating")) list()
})
byId("retry").addEventListener("click", () => {
  if (shown.state === "unreachable") invoke("retry")
  else on({ type: shown.state === "tooOld" ? "update" : "retry" })
})

/// The user returned to the window, so what's showing asks again, and its apps' files are listed again, for a new
/// version of any. While the window is on screen, what's showing asks again when a server says something changed.
function returned() {
  if (shown.state !== "home") return
  invoke("visible", { onScreen: !document.hidden })
  if (document.hidden) return void reload()
  for (const subject of listed) list(subject)
  invoke("refresh", { view: key(launcher) })
  if (open.has(history.state)) invoke("refresh", { view: history.state })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

/// Shows the launcher, once this user's files have listed one: loaded, or loaded again where it was open.
function start() {
  const home = open.get(key(launcher))
  if (home) /** @type {Window} */ (home.frame.contentWindow).location.replace(address(home))
  else load(launcher)
  // The app that was showing when the page was loaded again is gone, so the launcher shows.
  if (history.state) history.replaceState(null, "")
  show(null, 0)
}

/// Follows the device's session as it moves, each move heard once and in order: the page may hear one as it opens
/// before `start` has said where it was. While the device isn't in its network, the window goes where it signs in.
let heard = -1
/** @param {Told} told */
function hear({ seq, session, link }) {
  if (seq <= heard) return
  heard = seq
  if (session.state === "signIn" && link) location.replace(link)
  else on({ type: "session", session })
}

dress()
draw()
await listen("session", ({ payload }) => hear(payload))
// Where the window starts: where its device is with its network, as the client says it (`Session`), which it
// follows from then on. A phone's app hears a tap on a notification through its push plugin.
const { taps, ...told } = await invoke("start")
phone = Boolean(taps)
hear(told)
invoke("visible", { onScreen: !document.hidden }).catch(() => {})
if (taps) hearTaps()
