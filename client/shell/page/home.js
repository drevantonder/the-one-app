// The app's own page: the home screen, and the bridge between each open app and the client.
//
// An app is a page of its plugin's, in a frame at an address of its own, so it reaches no other app and nothing
// here. It speaks to this page as an MCP App speaks to its host: JSON-RPC messages, posted. What it asks goes to
// the client's Rust half (`ask`), which answers what only reads from what it kept and says when that has changed.
const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const home = document.getElementById("home")
/// The apps a server listed, by their plugin's name: `{ plugin, name, icon, page }`.
let apps = new Map()
/// The apps that are open, by their plugin's name: `{ plugin, frame, loaded, tapped, drawn }`. An app stays open
/// behind the home screen once it's been opened, as on a phone, so going back to it shows it as it was.
const open = new Map()

/// Where an app's page is. Android's and Windows' web views load only http, so there the app's own scheme is the
/// start of a host's name.
const address = ({ plugin, page }) =>
  `${location.protocol === "tauri:" ? "oneapp://" : "http://oneapp."}${plugin}.localhost/${plugin}/${page}`

/// Draws the home screen from the apps a server listed. An open app whose plugins have a new version loads again.
/// With no server answering and nothing kept from one, it says so.
async function list() {
  let listed, why
  try {
    listed = await invoke("apps")
  } catch (error) {
    why = `${error}.`
    listed = { resources: [], outdated: [] }
  }
  document.getElementById("problem").hidden = !why
  document.getElementById("why").textContent = why ?? ""
  // An app is a page of an MCP App's kind, at ui://<plugin>/<version>/<page>.
  apps = new Map(listed.resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app").map(({ uri, name, icons }) => {
    const [plugin, _version, ...page] = uri.slice("ui://".length).split("/")
    return [plugin, { plugin, page: page.join("/"), name, icon: icons?.[0]?.src }]
  }))
  document.getElementById("empty").hidden = apps.size > 0 || Boolean(why)
  document.getElementById("apps").replaceChildren(...[...apps.values()].map((app) => {
    const tile = document.createElement("button")
    tile.append(Object.assign(new Image(), { src: app.icon, alt: "" }), Object.assign(document.createElement("span"), { textContent: app.name }))
    // An app starts as a finger lands on its icon, and shows as the finger lifts.
    tile.addEventListener("pointerdown", () => load(app))
    tile.addEventListener("click", () => {
      show(app.plugin, performance.now())
      // The way back out of an app is an entry in this window's history, made once the app's page has loaded: a
      // browser takes a frame back through its own screens only from an entry that knew the frame's page.
      open.get(app.plugin).loaded.then(() => history.pushState(app.plugin, ""))
    })
    return tile
  }))
  for (const view of open.values()) {
    const app = apps.get(view.plugin)
    if (app && !listed.outdated.includes(view.plugin)) continue
    post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
    await invoke("close", { view: view.plugin })
    view.drawn = false
    // Loaded again in place, so the way back still leads out of the app.
    if (app) view.frame.contentWindow.location.replace(address(app))
    else view.frame.remove(), open.delete(view.plugin)
  }
  // The app that was showing is gone, so the home screen shows.
  if (history.state && !apps.has(history.state)) history.back()
}

/// Starts an app: its page loads in a frame of its own, behind the home screen until it's shown.
function load(app) {
  if (open.has(app.plugin)) return
  const frame = Object.assign(document.createElement("iframe"), { src: address(app), title: app.name, inert: true })
  frame.sandbox = "allow-scripts allow-same-origin allow-forms"
  const loaded = new Promise((loaded) => frame.addEventListener("load", loaded, { once: true }))
  open.set(app.plugin, { plugin: app.plugin, frame, loaded })
  document.body.append(frame)
}

/// Shows an app over the home screen, or the home screen with no app named. `tapped` is when its icon was tapped.
function show(plugin, tapped) {
  const app = apps.get(plugin)
  if (app) load(app)
  for (const view of open.values()) view.frame.inert = view.plugin !== plugin
  home.inert = Boolean(app)
  if (!app) return
  const view = open.get(plugin)
  view.tapped = tapped
  if (!view.drawn) return
  // Its user returned to it.
  invoke("again", { view: plugin })
  opened(view)
}

/// Says how long an app took from the tap on its icon to its content on screen: the second frame after it's
/// showing with what it drew from.
function opened(view) {
  if (view.tapped === undefined) return
  const tapped = view.tapped
  view.tapped = undefined
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const { duration } = performance.measure(`open ${view.plugin}`, { start: tapped })
    invoke("opened", { view: view.plugin, ms: Math.round(duration) })
  }))
}

const post = (view, message) => view.frame.contentWindow?.postMessage({ jsonrpc: "2.0", ...message }, "*")

/// What this page answers an app itself. The rest is the client's to answer.
function answer(view, method, params) {
  if (method === "ui/initialize") {
    return {
      protocolVersion: "2026-01-26",
      hostInfo: { name: "one", version: "0.1.0" },
      hostCapabilities: { serverTools: {}, serverResources: {} },
      hostContext: {
        theme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light",
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
  if (method.startsWith("ui/")) throw Object.assign(new Error(`This client has no ${method}`), { code: -32601 })
  return invoke("ask", { view: view.plugin, method, params })
}

addEventListener("message", async ({ source, data }) => {
  const view = [...open.values()].find(({ frame }) => frame.contentWindow === source)
  if (!view || typeof data?.method !== "string") return
  const { id, method, params = {} } = data
  if (id === undefined) {
    // The app's user went back out of it. The app stays open behind the home screen.
    if (method === "ui/notifications/request-teardown" && !view.frame.inert) history.back()
    return
  }
  try {
    post(view, { id, result: await answer(view, method, params) })
  } catch (error) {
    post(view, { id, error: { code: error.code ?? -32000, message: error.message ?? String(error) } })
  }
  // It has what it draws from once the client has answered it something.
  if (method.startsWith("ui/")) return
  view.drawn = true
  if (!view.frame.inert) opened(view)
})

listen("changed", ({ payload: view }) => {
  if (view === "home") list()
  else if (open.has(view)) post(open.get(view), { method: "notifications/one/changed" })
})

// The device's own way back, and this page's, both end here.
addEventListener("popstate", ({ state }) => show(state, performance.now()))

document.getElementById("retry").addEventListener("click", list)

/// The user returned to the window, so what's showing asks again. While the window is on screen, servers say when
/// something changed.
function returned() {
  invoke("watch", { watching: !document.hidden })
  if (document.hidden) return
  if (!document.getElementById("problem").hidden) list()
  invoke("again", { view: "home" })
  if (apps.has(history.state)) invoke("again", { view: history.state })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

await list()
invoke("watch", { watching: !document.hidden })
show(history.state)
requestAnimationFrame(() => requestAnimationFrame(() => invoke("opened", { view: "home", ms: Math.round(performance.now()) })))
