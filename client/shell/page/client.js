// The window's own page: the frames apps are drawn in, and the bridge between each and the client.
//
// An app is a page of its plugin's, in a frame at an address of its own, so it reaches no other app and nothing
// here. It speaks to this page as an MCP App speaks to its host: JSON-RPC messages, posted. What it asks goes to
// the client's Rust half (`ask`), which answers what only reads from what it kept and says when that has changed.
//
// The home pages are an app too: the plugin `home`'s, which reaches this client from a server as every plugin
// does. It's the one this page shows first, under every other, and it opens the rest (`one/open`).
const { invoke } = window.__TAURI__.core
const { listen } = window.__TAURI__.event

const HOME = "home/app/index.html"
/// The apps that are open, by who each is for and what it is: `{ key, app, for, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
const open = new Map()
const key = (app, who) => `${who ?? ""}|${app}`
/// Each plugin's first app, by the plugin's name, among this user's, and one that isn't a part of Settings if it
/// has one: what a notification from it opens.
let first = new Map()
/// Who this client has listed files for: its user (`undefined`), and each group or the network an app was opened
/// for.
const listed = new Set()

/// Where an app's page is: under its plugin's name, then who it's for if that isn't its user, last part first.
/// Android's and Windows' web views load only http, so there the app's own scheme is the start of a host's name.
const host = (app, who) => [app.split("/")[0], ...(who ? who.split("/").reverse() : [])].join(".")
const address = (view) => `${location.protocol === "tauri:" ? "oneapp://" : "http://oneapp."}${view.host}.localhost/${view.app}`

/// Whether a server answered the last thing this client asked, and why its files couldn't be listed, if they
/// couldn't.
let answering = true
let unlisted
/// Where the window starts, which it asks as it opens: here, or on the page its device is let in by while it's
/// nobody's (`signIn`). `unreached` is why the client couldn't open, if it couldn't: its network's address didn't
/// answer. Then that's all the window says, until trying again opens it.
const { signIn, unreached = null } = await invoke("start")
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

/// Says over the home pages that no server is answering: with nothing to show, as what needs its user now, and
/// over what this device kept, as what can wait. Fine is silent.
function problem() {
  const box = document.getElementById("problem")
  const nothing = Boolean(unlisted) || unreached !== null
  box.hidden = (answering && !nothing) || Boolean(history.state)
  box.toggleAttribute("data-waits", !nothing)
  document.getElementById("what").textContent =
    unreached !== null ? "Can't reach your network" : unlisted ? "Can't show your home pages" : "No server is answering"
  document.getElementById("why").textContent = unreached !== null
    ? `${unreached}. Check this device is online.`
    : unlisted ?? "This is what this device kept. Check it's online and a server is on."
}

/// Lists the files a server has for `who`, which the client keeps, so an app opens with no server up. An open app
/// whose plugins have a new version loads again.
async function list(who) {
  listed.add(who)
  let outdated = []
  try {
    const { resources, ...rest } = await invoke("apps", { who })
    outdated = rest.outdated
    if (!who) {
      unlisted = undefined
      // An app is a page of an MCP App's kind, at ui://<plugin>/<version>/<page>.
      const pages = resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app")
      // The last one set for a plugin is the one kept: its first, and one that isn't a part of Settings before one that is.
      const order = pages.toReversed().sort((a, b) => Boolean(b._meta?.settings) - Boolean(a._meta?.settings))
      first = new Map(order.map(({ uri }) => uri.slice("ui://".length).split("/")).map(([plugin, _version, ...page]) => [plugin, `${plugin}/${page.join("/")}`]))
    }
  } catch (error) {
    if (!who) unlisted = `${error}`.replace(/[^.]$/, "$&.")
  }
  problem()
  for (const view of open.values()) {
    if (!outdated.includes(view.host)) continue
    post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
    await invoke("close", { view: view.key })
    view.drawn = false
    // Loaded again in place, so the way back still leads out of the app.
    view.frame.contentWindow.location.replace(address(view))
  }
}

/// Starts an app: its page loads in a frame of its own, behind what's showing until it's shown.
function load(app, who) {
  const at = key(app, who)
  if (open.has(at)) return open.get(at)
  if (!listed.has(who)) list(who)
  const view = { key: at, app, for: who, host: host(app, who) }
  view.frame = Object.assign(document.createElement("iframe"), { src: address(view), inert: true })
  view.frame.sandbox = "allow-scripts allow-same-origin allow-forms"
  // An app hands a link to the device's share sheet, or copies it, as its person presses Share (the kit's `share`).
  view.frame.allow = "clipboard-write; web-share"
  open.set(at, view)
  document.body.append(view.frame)
  return view
}

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let shownByDevice = false

/// Changes between the home pages and an app as a move, as the kit moves between an app's screens
/// (ui/src/screen.jsx): the browser slides the app in over the home pages, or with `back`, back out from over
/// them. Where the browser can't, or the user asked for no motion, it's a plain swap.
function move(change, back = false) {
  const plain = !globalThis.ViewTransition || !("types" in ViewTransition.prototype) || matchMedia("(prefers-reduced-motion: reduce)").matches
  if (plain || shownByDevice) return void change()
  document.startViewTransition({ update: change, types: [back ? "back" : "forward"] })
}

/// Shows the app that's open as `at` over the rest, or with none there, the home pages. `tapped` is when its icon
/// was tapped.
function show(at, tapped) {
  const view = open.get(at) ?? open.get(key(HOME))
  for (const other of open.values()) other.frame.inert = other !== view
  problem()
  if (!view) return
  view.tapped = tapped
  if (!view.drawn) return
  // Its user returned to it.
  invoke("again", { view: view.key })
  opened(view)
}

/// Says how long an app took from the tap on its icon to its content on screen: the second frame after it's
/// showing with what it drew from. The home pages' time is from when this page began to load.
function opened(view) {
  if (view.tapped === undefined) return
  const tapped = view.tapped
  view.tapped = undefined
  requestAnimationFrame(() => requestAnimationFrame(() => {
    const name = view.app === HOME ? "home" : view.app.split("/")[0]
    const { duration } = performance.measure(`open ${name}`, { start: tapped })
    invoke("opened", { view: name, ms: Math.round(duration) })
    // The device hears its person's notifications once they see their home.
    if (view.app === HOME) invoke("hear")
  }))
}

/// Shows this device's person their recovery key, or with `typed`, asks them for it and lets the device in with
/// it: theirs, or that of `who`, a group or the network the app asking is for. → `{ done }`: whether they saw it,
/// or the device is in now. It's over whatever app asked, and the app is told only that.
function recoveryKey(typed, who) {
  const dialog = document.getElementById("recovery")
  const [key, entry, problem, form] = [".key", ".entry", ".wrong", "form"].map((part) => dialog.querySelector(part))
  dialog.dataset.typed = typed
  entry.value = key.textContent = problem.textContent = dialog.returnValue = ""
  if (!typed) invoke("recovery_key", { who }).then((shown) => (key.textContent = shown), (error) => (problem.textContent = `${error}`))
  dialog.showModal()
  return new Promise((answered) => {
    form.onsubmit = async (event) => {
      event.preventDefault()
      if (typed && event.submitter?.value !== "cancel") {
        try {
          await invoke("recover", { key: entry.value, who })
        } catch (error) {
          return void (problem.textContent = `${error}`)
        }
      }
      dialog.returnValue = event.submitter?.value === "cancel" ? "" : "done"
      dialog.close()
    }
    dialog.onclose = () => {
      entry.value = key.textContent = ""
      answered({ done: dialog.returnValue === "done" })
    }
  })
}

const post = (view, message) => view.frame.contentWindow?.postMessage({ jsonrpc: "2.0", ...message }, "*")

/// What this page answers an app itself. The rest is the client's to answer.
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
  // A web address opens in the device's own browser: the window is sent there, and leaves it to the browser.
  if (method === "ui/open-link" && /^https?:/.test(new URL(params.url).protocol)) {
    location.assign(params.url)
    return {}
  }
  // What an app is told of its client, and the few things one changes of it.
  if (method === "one/client") {
    if (params.leave) return invoke("enter")
    // This device becomes a server of its person's, or stops being one, or takes the newest builds.
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
  // An app opened from a group's home page is answered for the group.
  const who = params._meta?.for ?? view.for
  if (who && !listed.has(who)) list(who)
  return invoke("ask", { view: view.key, method, params: who ? { ...params, _meta: { ...params._meta, for: who } } : params })
}

addEventListener("message", async ({ source, data }) => {
  const view = [...open.values()].find(({ frame }) => frame.contentWindow === source)
  if (!view || typeof data?.method !== "string") return
  const { id, method, params = {} } = data
  if (id === undefined) {
    // The app's user went back out of it. The app stays open behind the home pages.
    if (method === "ui/notifications/request-teardown" && !view.frame.inert && history.state) history.back()
    // Something is open over its page, or nothing is any more.
    if (method === "notifications/one/over") view.over = params.over === true
    return
  }
  try {
    post(view, { id, result: await answer(view, method, params) })
  } catch (error) {
    post(view, { id, error: { code: error.code ?? -32000, message: error.message ?? String(error) } })
  }
  // It has what it draws from once the client has answered it something.
  if (method.startsWith("ui/") || method.startsWith("one/")) return
  view.drawn = true
  if (!view.frame.inert) opened(view)
})

/// Shows an app over whatever's showing: by its plugin's name, and its page if it isn't the plugin's first app,
/// for `who`. With `behind` it only starts. → whether there is one here.
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

listen("changed", ({ payload: view }) => {
  if (view === "client") for (const who of listed) list(who)
  else if (open.has(view)) post(open.get(view), { method: "notifications/one/changed" })
})

/// The device's own back, where a device has one (the app's window there calls this): what's open over the app
/// that's showing shuts first. → whether anything was. If nothing was, the device goes back through this window's
/// history, which leads back through an app's screens and out of it, and from the home pages out of the app.
window.back = () => {
  const view = open.get(history.state) ?? open.get(key(HOME))
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
  answering = payload
  problem()
  // What couldn't be listed is asked for again now that a server answers.
  if (answering && unlisted) start()
})
document.getElementById("retry").addEventListener("click", () =>
  unreached !== null ? invoke("enter") : unlisted ? start() : invoke("again", { view: key(HOME) }),
)

/// The user returned to the window, so what's showing asks again. While the window is on screen, what's showing
/// asks again when a server says something changed.
function returned() {
  if (signIn || unreached !== null) return
  invoke("shown", { onScreen: !document.hidden })
  if (document.hidden) return
  if (unlisted) start()
  invoke("again", { view: key(HOME) })
  if (open.has(history.state)) invoke("again", { view: history.state })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

/// Lists this user's files, and shows the home pages once there are some to show them from.
async function start() {
  await list()
  if (unlisted) return
  if (open.has(key(HOME))) open.get(key(HOME)).frame.contentWindow.location.replace(address(open.get(key(HOME))))
  else load(HOME)
  // The app that was showing when the page was loaded again is gone, so the home pages show.
  if (history.state) history.replaceState(null, "")
  show(null, 0)
}

dress()
if (unreached !== null) problem()
else if (!signIn) {
  await start()
  invoke("shown", { onScreen: !document.hidden })
}
