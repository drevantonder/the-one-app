// The app's own page: the home screen, and the bridge between each open app and the client.
//
// An app is a page of its plugin's, in a frame at an address of its own, so it reaches no other app and nothing
// here. It speaks to this page as an MCP App speaks to its host: JSON-RPC messages, posted. What it asks goes to
// the client's Rust half (`ask`), which answers what only reads from what it kept and says when that has changed.
const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const home = document.getElementById("home")
/// The apps a server listed, by their plugin's name: `{ plugin, name, icon, page, version }`.
let apps = new Map()
/// The apps that are open, by their plugin's name: `{ plugin, version, frame, tapped }`. An app stays open behind
/// the home screen once it's been opened, as on a phone, so going back to it shows it as it was.
const open = new Map()

/// Where an app's page is. Android's and Windows' web views load only http, so there the app's own scheme is the
/// start of a host's name.
const address = ({ plugin, page }) =>
  `${location.protocol === "tauri:" ? "oneapp://" : "http://oneapp."}${plugin}.localhost/${plugin}/${page}`

/// Draws the home screen from the apps a server listed. An open app whose plugin has a new version loads again.
async function list() {
  let listed
  try {
    listed = (await invoke("ask", { view: "home", method: "resources/list", params: {} })).resources
  } catch {
    listed = []
  }
  // An app is a page of an MCP App's kind, at ui://<plugin>/<version>/<page>.
  apps = new Map(listed.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app").map(({ uri, name, icons }) => {
    const [plugin, version, ...page] = uri.slice("ui://".length).split("/")
    return [plugin, { plugin, version, page: page.join("/"), name, icon: icons?.[0]?.src }]
  }))
  document.getElementById("empty").hidden = apps.size > 0
  document.getElementById("apps").replaceChildren(...[...apps.values()].map((app) => {
    const tile = document.createElement("button")
    tile.append(Object.assign(new Image(), { src: app.icon, alt: "" }), Object.assign(document.createElement("span"), { textContent: app.name }))
    tile.addEventListener("click", () => {
      history.pushState(app.plugin, "")
      show(app.plugin, performance.now())
    })
    return tile
  }))
  for (const view of open.values()) {
    const app = apps.get(view.plugin)
    if (app?.version === view.version) continue
    post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
    await invoke("close", { view: view.plugin })
    if (app) Object.assign(view, { version: app.version }).frame.src = address(app)
    else view.frame.remove(), open.delete(view.plugin)
  }
}

/// Shows an app over the home screen, or the home screen with no app named. `tapped` is when its icon was tapped.
function show(plugin, tapped) {
  for (const view of open.values()) view.frame.hidden = view.plugin !== plugin
  const app = apps.get(plugin)
  home.inert = Boolean(app)
  if (!app) return
  const view = open.get(plugin)
  if (view) {
    // Its user returned to it.
    invoke("again", { view: plugin })
    view.tapped = tapped
    return opened(view)
  }
  const frame = Object.assign(document.createElement("iframe"), { src: address(app), title: app.name })
  frame.sandbox = "allow-scripts allow-same-origin allow-forms"
  open.set(plugin, { plugin, version: app.version, frame, tapped })
  document.body.append(frame)
}

/// Says how long an app took from the tap on its icon to its content on screen: the second frame after it had
/// what it drew with.
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
    if (method === "ui/notifications/request-teardown" && !view.frame.hidden) history.back()
    return
  }
  try {
    post(view, { id, result: await answer(view, method, params) })
  } catch (error) {
    post(view, { id, error: { code: error.code ?? -32000, message: error.message ?? String(error) } })
  }
  if (!method.startsWith("ui/")) opened(view)
})

listen("changed", ({ payload: view }) => {
  if (view === "home") list()
  else if (open.has(view)) post(open.get(view), { method: "notifications/one/changed" })
})

// The device's own way back, and this page's, both end here.
addEventListener("popstate", ({ state }) => show(state, performance.now()))

/// The user returned to the window, so what's showing asks again.
function returned() {
  if (document.hidden) return
  invoke("again", { view: "home" })
  if (apps.has(history.state)) invoke("again", { view: history.state })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

await list()
show(history.state)
