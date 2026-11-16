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
/** @import { View } from "./host.js" */

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
/// The apps that are open, by who each is for and what it is: `{ key, app, for, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
/** @type {Map<string, View>} */
const open = new Map()
/** @param {string} app @param {string} [who] */
const key = (app, who) => `${who ?? ""}|${app}`
/// Each plugin's first app, by the plugin's name, among this user's, and one that isn't a part of Settings if it
/// has one: what a notification from it opens.
/** @type {Map<string, string>} */
let first = new Map()
/// Who this client has listed files for: its user (`undefined`), and each group or the network an app was opened
/// for.
/** @type {Set<string | undefined>} */
const listed = new Set()

/// What an app's address is named by: its plugin's name, then who it's for if that isn't its user, last part first.
/** @param {string} app @param {string} [who] */
const host = (app, who) => [app.split("/")[0], ...(who ? who.split("/").reverse() : [])].join(".")

/// Whether a server answered the last thing this client asked, and since when none has, and why its files couldn't
/// be listed, if they couldn't.
let answering = true
let unanswered = 0
/** @type {string | undefined} */
let unlisted
/// Whether this app is too old for its network: its servers list no launcher, which every network has, as when how
/// apps are listed has changed since this app was built.
let tooOld = false
/// Where the window starts, which it asks as it opens: here, or on the page its device is let in by while it's
/// nobody's (`signIn`). `unreached` is why the client couldn't open, if it couldn't: its network's address didn't
/// answer. Then that's all the window says, until trying again opens it.
const { signIn, unreached = null, taps } = await invoke("start")
/// A phone's app comes from its store, and a computer's updates itself.
const phone = Boolean(taps)
if (signIn) location.replace(signIn)

/// Light or dark, as Settings › Appearance has it on this device, or as the device is.
const scheme = matchMedia("(prefers-color-scheme: dark)")
const chosen = () => localStorage.getItem("theme")
const theme = () => chosen() ?? (scheme.matches ? "dark" : "light")
function dress() {
  document.documentElement.style.colorScheme = chosen() ?? ""
  for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { theme: theme() } })
}
scheme.addEventListener("change", dress)

/// Says over the launcher what keeps it from showing anything: the network's address didn't answer, its pages
/// couldn't be listed, or this app is too old for the network. Fine is silent.
/** @type {string | undefined} */
let said
function problem() {
  // A problem is said in place of the mark.
  const nothing = unreached !== null || Boolean(unlisted) || tooOld
  if (nothing) byId("opening").hidden = true
  byId("problem").hidden = !nothing || Boolean(history.state)
  byId("what").textContent = tooOld ? "This app needs updating" : unreached !== null ? "Can't reach your network" : "Can't show your Home Screen pages"
  byId("why").textContent = tooOld
    ? phone ? `Get the latest version from the ${/iPhone|iPad/.test(navigator.userAgent) ? "App Store" : "Play Store"}.` : "Your network is on a newer version."
    : unreached !== null ? `${unreached}. Check this device is online.` : (unlisted ?? "")
  const action = byId("retry")
  action.textContent = tooOld ? "Update" : "Try Again"
  action.dataset.variant = tooOld ? "default" : "outline"
  action.hidden = tooOld && phone
  // What's shown is reported once, as it's shown.
  const shown = nothing ? `${byId("what").textContent}: ${byId("why").textContent}` : undefined
  if (shown && shown !== said) report("one.client.problem", shown)
  said = shown
  status()
}

/// How long no server answering goes unsaid, as a moment's gap while the client reconnects by itself, and how long
/// before the window says it's showing what was saved.
const BLIP = 3000
const LONG = 15000
/** @type {ReturnType<typeof setTimeout> | undefined} */
let saying
/// Says in one quiet line, over whatever's showing, that no server is answering for now: nothing for a blip, then
/// that it's reconnecting, and once that's taken a while, or the device is offline, that it shows what was saved.
function status() {
  clearTimeout(saying)
  const line = byId("status")
  const waited = performance.now() - unanswered
  line.hidden = answering || unreached !== null || Boolean(unlisted) || tooOld || waited < BLIP
  if (!answering && waited < LONG) saying = setTimeout(status, (waited < BLIP ? BLIP : LONG) - waited)
  line.textContent = !navigator.onLine ? "You're offline. Showing what was saved." : waited < LONG ? "Reconnecting…" : "Reconnecting… Showing what was saved."
}
addEventListener("offline", status)
// Back online, the client asks at once, rather than when it next would have.
addEventListener("online", () => {
  status()
  if (!answering) invoke("refresh", { view: key(launcher) })
})

/// Lists the files a server has for `who`, which the client keeps, so an app opens with no server up. An open app
/// whose plugins have a new version loads again.
/** @param {string} [who] */
async function list(who) {
  listed.add(who)
  try {
    const { resources, outdated } = await invoke("apps", { who })
    for (const view of open.values()) if (outdated.includes(view.host)) view.stale = true
    if (!who) {
      unlisted = undefined
      // An app is a page of an MCP App's kind, at ui://<plugin>/<version>/<page>.
      const pages = resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app")
      const named = (/** @type {string} */ uri) => {
        const [plugin, _version, ...page] = uri.slice("ui://".length).split("/")
        return `${plugin}/${page.join("/")}`
      }
      // The last one set for a plugin is the one kept: its first, and one with no category before one with one.
      const order = pages.toReversed().sort((a, b) => Number(Boolean(b._meta?.category)) - Number(Boolean(a._meta?.category)))
      first = new Map(order.map(({ uri }) => [named(uri).split("/")[0], named(uri)]))
      launcher = named(pages.find(({ _meta }) => _meta?.category === "launcher")?.uri ?? "ui:///")
      tooOld = launcher === "/"
      if (tooOld) launcher = ""
    }
  } catch (error) {
    if (!who) unlisted = `${error}`.replace(/[^.]$/, "$&.")
  }
  problem()
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
/** @param {string} app @param {string} [who] @returns {View} */
function load(app, who) {
  const at = key(app, who)
  const opened = open.get(at)
  if (opened) return opened
  if (!listed.has(who)) list(who)
  const place = { key: at, app, for: who, host: host(app, who) }
  const view = { ...place, frame: Object.assign(document.createElement("iframe"), { src: address(place), inert: true }) }
  view.frame.setAttribute("sandbox", sandbox)
  if (app === launcher) view.frame.className = "launcher"
  // An app hands a link to the device's share sheet, or copies it, as its person presses Share (the UI library's `share`).
  view.frame.allow = "clipboard-write; web-share"
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
  problem()
  reload()
  if (!view) return
  view.tapped = tapped
  setTimeout(() => {
    if (view.tapped === tapped && !view.drawn) report("one.client.open", `Didn't open in ${NEVER / 1000} s`, { "one.view": view.app.split("/")[0] })
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
  if (home && view) post(home, { method: "notifications/one/showing", params: { app: view.app, for: view.for } })
  return view
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
    // The device hears its person's notifications once they see their home, which has drawn over the mark.
    if (view.app === launcher) {
      byId("opening").hidden = true
      invoke("listen")
    }
  }))
}

/// Shows this device's person their recovery key, or with `typed`, asks them for it and lets the device in with
/// it: theirs, or that of `who`, a group or the network the app asking is for. → `{ done }`: whether they saw it,
/// or the device is in now. It's over whatever app asked, and the app is told only that.
/** @param {boolean} typed @param {string} [who] @returns {Promise<{ done: boolean }>} */
function recoveryKey(typed, who) {
  const dialog = /** @type {HTMLDialogElement} */ (byId("recovery"))
  const [key, entry, problem, form] = /** @type {[HTMLElement, HTMLInputElement, HTMLElement, HTMLFormElement]} */ ([".key", ".entry", ".wrong", "form"].map((part) => dialog.querySelector(part)))
  dialog.dataset.typed = String(typed)
  entry.value = key.textContent = problem.textContent = dialog.returnValue = ""
  if (!typed) invoke("recovery_key", { who }).then((shown) => (key.textContent = shown), (error) => (problem.textContent = `${error}`))
  dialog.showModal()
  return new Promise((answered) => {
    form.onsubmit = async (event) => {
      event.preventDefault()
      const pressed = /** @type {HTMLButtonElement | null} */ (event.submitter)
      if (typed && pressed?.value !== "cancel") {
        try {
          await invoke("recover", { key: entry.value, who })
        } catch (error) {
          return void (problem.textContent = `${error}`)
        }
      }
      dialog.returnValue = pressed?.value === "cancel" ? "" : "done"
      dialog.close()
    }
    dialog.onclose = () => {
      entry.value = key.textContent = ""
      answered({ done: dialog.returnValue === "done" })
    }
  })
}

/** @param {View} view @param {Record<string, unknown>} message */
const post = (view, message) => view.frame.contentWindow?.postMessage({ jsonrpc: "2.0", ...message }, "*")

/// What this page answers an app itself. The rest is the client's to answer.
/** @param {View} view @param {string} method @param {Record<string, any>} params */
async function answer(view, method, params) {
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
  if (method === "one/recovery-key") return recoveryKey(params.typed === true, params.for ?? view.for)
  // Another app, shown over this one, as tapping its icon or a notification from it does: by its plugin's name,
  // and its page if it isn't the plugin's first app, for whoever this one is for unless it says. `behind` starts
  // it as a finger lands on its icon, and it shows as the finger lifts.
  if (method === "ui/open-link" && params.url?.startsWith("one:")) {
    if (!go(params.url.slice("one:".length), params.for ?? view.for, params.behind)) throw new Error("That app isn't on this device")
    return {}
  }
  // A web address opens in the device's own browser, outside the app.
  if (method === "ui/open-link" && /^https?:/.test(new URL(params.url).protocol)) {
    openLink(params.url)
    return {}
  }
  // What an app is told of its client, and the few things one changes of it.
  if (method === "one/client") {
    if (params.leave) return invoke("enter")
    // This device becomes a server of its person's, or stops being one, or takes the latest builds.
    if ("serve" in params) return { said: await invoke("serve", { on: params.serve === true }) }
    if (params.update) return { said: await invoke("update") }
    if ("theme" in params) {
      if (params.theme) localStorage.setItem("theme", params.theme)
      else localStorage.removeItem("theme")
      dress()
    }
    return { ...(await invoke("client")), theme: chosen() }
  }
  if (method.startsWith("ui/")) throw Object.assign(new Error(`This client has no ${method}`), { code: -32601 })
  // An app opened from a group's page is answered for the group.
  const who = params._meta?.for ?? view.for
  if (who && !listed.has(who)) list(who)
  return invoke("request", { view: view.key, method, params: who ? { ...params, _meta: { ...params._meta, for: who } } : params })
}

addEventListener("message", async ({ source, data }) => {
  const view = [...open.values()].find(({ frame }) => frame.contentWindow === source)
  if (!view || typeof data?.method !== "string") return
  const { id, method, params = {} } = data
  if (id === undefined) {
    // The app's user went back out of it. The app stays open behind the launcher.
    if (method === "ui/notifications/request-teardown" && !view.frame.inert && history.state) history.back()
    // Something is open over its page, or nothing is any more.
    if (method === "notifications/one/over") view.over = params.over === true
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
    post(view, { id, result: await answer(view, method, params) })
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
/// for `who`. With `behind` it only starts. → whether there is one here.
/** @param {string} app @param {string} [who] @param {boolean} [behind] */
function go(app, who, behind) {
  const page = app.includes("/") ? app : first.get(app)
  if (!page) return false
  const tapped = performance.now()
  const next = load(page, who)
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
  if (view === "client") for (const who of listed) list(who)
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
  if (answering && !payload) unanswered = performance.now()
  answering = payload
  problem()
  // What couldn't be listed is asked for again now that a server answers.
  if (answering && unlisted) start()
})
byId("retry").addEventListener("click", async () => {
  if (tooOld) {
    // A computer's app takes the latest builds and starts again on them; one that's also a server says what it did.
    byId("why").textContent = await invoke("update").catch((error) => `${error}`)
  } else if (unreached !== null) invoke("enter")
  else start()
})

/// The user returned to the window, so what's showing asks again, and its apps' files are listed again, for a new
/// version of any. While the window is on screen, what's showing asks again when a server says something changed.
function returned() {
  if (signIn || unreached !== null) return
  invoke("visible", { onScreen: !document.hidden })
  if (document.hidden) return void reload()
  if (unlisted) start()
  else for (const who of listed) list(who)
  invoke("refresh", { view: key(launcher) })
  if (open.has(history.state)) invoke("refresh", { view: history.state })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

/// Lists this user's files, and shows the launcher once there are some to show it from.
async function start() {
  await list()
  if (unlisted || tooOld) return
  const home = open.get(key(launcher))
  if (home) /** @type {Window} */ (home.frame.contentWindow).location.replace(address(home))
  else load(launcher)
  // The app that was showing when the page was loaded again is gone, so the launcher shows.
  if (history.state) history.replaceState(null, "")
  show(null, 0)
}

dress()
if (unreached !== null) problem()
else if (!signIn) {
  await start()
  invoke("visible", { onScreen: !document.hidden })
  if (taps) hearTaps()
}
