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
import { address, haptic, invoke, listen, openLink, sandbox } from "./host.js"
/** @import { Context, Moved, Paged, Told, View } from "./host.js" */

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
/// The launcher's page, as this user's apps list it, once they're listed: `<plugin>/<page>`. And the Plugins app's,
/// which installs one where there's none: the app in the category `plugins`, if there is one.
let launcher = ""
let plugins = ""
/// The apps that are open, by who each is for and what it is: `{ key, app, subject, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
/** @type {Map<string, View>} */
const open = new Map()
/** @param {string} app @param {string} [subject] */
const key = (app, subject) => `${subject ?? ""}|${app}`
/// Each plugin's first app, by the plugin's name, among this user's: what a notification from it opens.
/** @type {Map<string, string>} */
let first = new Map()
/// Each app's name, by its page, as this user's apps list them.
/** @type {Map<string, string>} */
let names = new Map()
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

/// Light or dark, as Colour Mode has it on this device, or as the device is.
const scheme = matchMedia("(prefers-color-scheme: dark)")
const chosen = () => localStorage.getItem("theme")
const theme = () => chosen() ?? (scheme.matches ? "dark" : "light")
function dress() {
  document.documentElement.style.colorScheme = chosen() ?? ""
  for (const name of ["light", "dark"]) document.documentElement.classList.toggle(name, theme() === name)
  for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { theme: theme() } })
}
scheme.addEventListener("change", dress)

/// How far in from each edge of the window what the device keeps for itself reaches (an iPhone's status bar, home
/// indicator and notch), which only this page is told: Android's window is already clear of its own.
const safeArea = Object.assign(document.createElement("div"), { id: "safe-area" })
document.body.append(safeArea)
/// What of that an app's frame reaches, as MCP Apps tell an app (`safeAreaInsets`): its screen keeps clear of it,
/// and what scrolls in it passes under it. The launcher's frame is kept clear of it, so it's told none.
/** @param {HTMLIFrameElement} frame */
function insets(frame) {
  const safe = getComputedStyle(safeArea)
  const style = getComputedStyle(frame)
  const at = frame.getBoundingClientRect()
  const inside = (/** @type {string} */ edge) => parseFloat(style.getPropertyValue(`padding-${edge}`)) + parseFloat(style.getPropertyValue(`border-${edge}-width`))
  const reach = (/** @type {string} */ edge, /** @type {number} */ from) => Math.max(0, Math.round(parseFloat(safe.getPropertyValue(`padding-${edge}`)) - from - inside(edge)))
  return { top: reach("top", at.top), right: reach("right", innerWidth - at.right), bottom: reach("bottom", innerHeight - at.bottom), left: reach("left", at.left) }
}
/// Each app is told again as the window turns or its frame moves.
function placed() {
  for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { safeAreaInsets: insets(view.frame) } })
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

/// What the window shows of itself, one state at a time: while it opens, until it can start the launcher; the
/// launcher, once it has drawn; signing in, where the device isn't in its network; why there's no launcher to show,
/// with Plugins to install one; and an update. Only `on` moves it, by the moves `machine` has, and a move that isn't
/// there is refused. Whether its network answers is the client's (`Connection` in core/client), which the
/// window follows and never decides: it says it as it opens, and as a banner over the launcher.
const { assign, createActor, fromPromise, setup } = XState
const machine = setup({
  types: /** @type {{ context: Context, events: Moved }} */ ({}),
  actors: {
    // A computer's app takes the latest builds and starts again on them; one that's also a server says what it did.
    update: fromPromise(() => invoke("update")),
  },
  guards: {
    signIn: ({ event }) => event.type === "session" && event.told.session.state === "signIn",
    online: ({ event }) => event.type === "connection" && event.connection === "online",
    waiting: ({ context }) => !context.starting,
    updateReady: ({ context }) => context.updateReady,
    // A phone's app is its network's, which its owner builds and updates.
    updates: () => !phone,
    plugins: () => Boolean(plugins),
  },
  actions: {
    told: assign({ told: ({ event, context }) => (event.type === "session" ? event.told : context.told) }),
    connection: assign(({ event, context }) => {
      if (event.type !== "connection") return {}
      for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { connection: event.connection } })
      return { connection: event.connection, lost: context.lost || event.connection === "offline" }
    }),
    updateReady: assign(({ event, context }) => {
      if (event.type !== "updateReady" || event.ready === context.updateReady) return {}
      for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { updateReady: event.ready } })
      return { updateReady: event.ready }
    }),
    // Its apps are listed once it's told where its device is, unless it's to sign in.
    list: ({ context }) => void (context.told && context.told.session.state !== "signIn" && list()),
    start: () => start(),
    signIn: ({ context }) => void (context.told?.link && location.replace(context.told.link)),
    // The device hears its person's notifications once they see their home.
    listen: () => void invoke("listen"),
    openPlugins: () => openPlugins(),
  },
}).createMachine({
  id: "window",
  initial: "opening",
  context: { connection: "connecting", lost: false, starting: false, updateReady: false },
  on: {
    session: { actions: "told" },
    connection: { actions: "connection" },
    updateReady: { actions: "updateReady" },
  },
  states: {
    // Waits until it can start the launcher: its device in its network and its network answering, or a launcher
    // kept from before.
    opening: {
      entry: [assign({ starting: false }), "list"],
      on: {
        session: [{ guard: "signIn", target: "sign_in", actions: "told" }, { actions: ["told", "list"] }],
        connection: [{ guard: "online", actions: ["connection", "list"] }, { actions: "connection" }],
        listed: { guard: "waiting", actions: [assign({ starting: true }), "start"] },
        drawn: "launcher",
        none: { target: "no_launcher", actions: assign({ only: ({ event }) => event.only }) },
        tooOld: "needs_updating",
        failed: { target: "launcher_failed", actions: assign({ why: ({ event }) => event.why }) },
      },
    },
    // The network's own sign-in page, which the client brings the window back from once its person has approved
    // it, where it opens again (`follows` in `src/lib.rs`).
    sign_in: { entry: "signIn" },
    launcher: {
      entry: "listen",
      on: { update: { guard: "updateReady", target: "updating" } },
    },
    no_launcher: {
      on: { openPlugins: { guard: "plugins", target: "plugins" } },
    },
    // The Plugins app, shown with no launcher under it, until a launcher is installed.
    plugins: {
      entry: "openPlugins",
      on: { listed: "opening" },
    },
    launcher_failed: {
      on: { retry: "opening" },
    },
    needs_updating: {
      on: { update: { guard: "updates", target: "updating" } },
    },
    // It starts again on the new build once it has updated, as an app does: a device that's also a server once the
    // program keeping its server running has taken it, which says so meanwhile.
    updating: {
      entry: assign({ message: undefined }),
      invoke: {
        src: "update",
        onDone: { actions: assign({ message: ({ event }) => event.output }) },
        onError: { target: "update_failed", actions: assign({ why: ({ event }) => `${event.error}` }) },
      },
    },
    update_failed: {
      on: { retry: "updating" },
    },
  },
})
const shown = createActor(machine)
/** @param {Moved} event */
const on = (event) => shown.send(event)

/// Each open app's page, one state at a time, as its person's owner signed it off: loading, then current until its
/// plugin or the UI library is updated under it, which makes it stale. A stale page loads again by itself only at a
/// boundary, as an iPhone updates an app while it's closed and a web app loads its new version on the next
/// navigation: once it's off screen (another app over it, the window hidden), or as it's opened again. On screen,
/// it says so quietly, "Updated · Reload", and loads again only once its person presses Reload: nothing is reloaded
/// under them. A page that doesn't draw has failed, with Try Again. The launcher's page is one too.
const pageMachine = setup({
  types: /** @type {{ context: { view: View, why?: string }, input: View, events: Paged }} */ ({}),
  guards: {
    away: ({ context }) => !onScreen(context.view),
  },
  actions: {
    load: ({ context }) => reloadPage(context.view),
    failed: ({ context }) => {
      const { view, why } = context
      // How long it took to open is told only of an open that ended with it drawn.
      view.tapped = undefined
      report("one.client.open", why ?? "", { "one.view": view.app.split("/")[0] })
      // A launcher that never draws leaves nothing to show.
      if (view.app === launcher) on({ type: "failed", why: why ?? "" })
      notes()
    },
    notes: () => notes(),
  },
}).createMachine({
  id: "page",
  initial: "loading",
  context: ({ input }) => ({ view: input }),
  states: {
    loading: {
      on: { drawn: "current" },
      after: { [NEVER]: { target: "failed", actions: assign({ why: ({ context }) => `${context.view.app} didn't draw in ${NEVER / 1000} s.` }) } },
    },
    current: {
      on: { stale: "stale" },
    },
    stale: {
      entry: "notes",
      exit: "notes",
      always: { guard: "away", target: "reloading" },
      on: { away: "reloading", opened: "reloading", reload: "reloading" },
    },
    reloading: {
      entry: "load",
      on: { drawn: "current" },
      after: { [NEVER]: { target: "failed", actions: assign({ why: ({ context }) => `${context.view.app} didn't draw in ${NEVER / 1000} s.` }) } },
    },
    failed: {
      entry: "failed",
      exit: "notes",
      on: { retry: { target: "loading", actions: "load" } },
    },
  },
})
/** @param {View} view @param {Paged} event */
const page = (view, event) => view.page.send(event)
/// Whether an app's page is in front of its person: the one showing, in a window that's on screen.
/** @param {View} view */
const onScreen = (view) => !document.hidden && view === showing(history.state)

/// What the window says as it opens: the steps it takes to its person's home, the first of which says whether its
/// network answers. With a launcher kept from before, it's starting that alone.
/** @param {Context} context @returns {{ steps: string[], at: number, text?: string }} */
function steps({ connection, lost, starting }) {
  if (starting && connection !== "online") return { steps: ["Opening Home Screen"], at: 0 }
  const reached = connection === "offline" || lost ? "Can't reach your network" : "Connecting to your network"
  const at = starting || connection === "online" ? 1 : 0
  return { steps: [reached, "Opening Home Screen"], at, text: at === 0 && reached !== "Connecting to your network" ? "Check this device is online." : undefined }
}

/// What each problem says: its heading, its one sentence, what's behind it for Show Details, and the button that
/// fixes it, if there is one.
/** @param {import("xstate").SnapshotFrom<typeof machine>} now @returns {[string, string, string | undefined, string | undefined] | undefined} */
function problemOf(now) {
  if (now.matches("launcher_failed")) return ["Home Screen didn't open.", "Try again in a moment.", now.context.why, "Try Again"]
  if (now.matches("needs_updating")) return phone ? ["This app needs updating", "Its owner updates this app.", undefined, undefined] : ["This app needs updating", "An update is ready.", undefined, "Update"]
  if (now.matches("update_failed")) return ["Couldn't update", now.context.why ?? "", undefined, "Try Again"]
}

/// Draws what the window shows of itself, in its middle, over the frames: its mark and steps while it opens, a
/// spinner and a line while it updates, why there's no Home Screen, or a problem; and over the launcher, a banner
/// while its network doesn't answer. The launcher is the frames alone.
/** @type {string | undefined} */
let lastProblem
function draw() {
  const now = shown.getSnapshot()
  const { context } = now
  byId("state").hidden = now.matches("launcher") || now.matches("plugins") || now.matches("sign_in")
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
  byId("status").hidden = !now.matches("updating")
  byId("status-text").textContent = context.message ?? ""
  byId("status-text").hidden = !(now.matches("updating") && context.message)
  byId("empty").hidden = !now.matches("no_launcher")
  byId("empty-text").textContent = context.only ?? (plugins ? "" : "Ask your AI to install a Home Screen")
  byId("empty-text").hidden = !byId("empty-text").textContent
  byId("open-plugins").hidden = Boolean(context.only) || !plugins
  const problem = problemOf(now)
  byId("problem").hidden = !problem
  if (problem) {
    const [heading, text, details, action] = problem
    byId("problem-heading").textContent = heading
    byId("problem-text").textContent = text
    byId("problem-details").hidden = !details
    byId("problem-details-text").textContent = details ?? ""
    byId("problem-action").hidden = !action
    byId("problem-button").textContent = action ?? ""
    byId("problem-button").dataset.variant = action === "Update" ? "default" : "outline"
  }
  // What's shown is reported once, as it's shown.
  const said = problem && `${problem[0]}: ${problem[1]}`
  if (said && said !== lastProblem) report("one.client.problem", said)
  lastProblem = said
  // Over the launcher: its network not answering, and trying again once it has lost it. Answering, nothing.
  const banner = now.matches("launcher") && (context.connection === "offline" || (context.connection === "connecting" && context.lost))
  if (byId("banner").hidden === banner) {
    byId("banner").hidden = !banner
    placed()
  }
  byId("banner-spinner").hidden = context.connection !== "connecting"
  byId("banner-text").textContent = context.connection === "connecting" ? "Reconnecting…" : "Can't reach your network"
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
/// (`builds`): a browser, or an app whose network says nothing of its build, isn't.
async function behind() {
  const { build, target } = await invoke("client")
  if (!build || !target) return false
  const response = /** @type {{ structuredContent?: { latest?: Record<string, { build: string }> } }} */ (await invoke("request", { view: "client", method: "tools/call", params: { name: "builds" } }))
  const latest = response.structuredContent?.latest?.[`one/${target}`]?.build
  return Boolean(latest) && latest !== build
}

/// Lists the files a server has for `subject`, which the client keeps, so an app opens with no server up. An open app
/// whose plugins have a new version loads again. This user's own list has the launcher in it, which starts, or none:
/// which is a server's word, so a list kept from before is asked for again first, and while its network doesn't
/// answer, the window waits. With none, this app needs updating where its network says an update is ready for it
/// (`behind`): otherwise it's the network that has none right now. A launcher missing from a later list, as during a
/// release, leaves the one that's open as it is. Each of this user's lists says again whether an update is ready.
/** @param {string} [subject] */
async function list(subject) {
  listed.add(subject)
  try {
    let { resources, outdated } = await invoke("apps", { subject })
    // A screen runs its own plugin and the UI library, which every one of them runs.
    const library = outdated.some((host) => host.split(".")[0] === "ui")
    for (const view of open.values()) if (library || outdated.includes(view.host)) page(view, { type: "stale" })
    if (!subject) {
      if (!launcherIn(resources) && shown.getSnapshot().matches("opening")) {
        await invoke("refresh", { view: "client" })
        ;({ resources } = await invoke("apps", { subject }))
      }
      launcher = launcherIn(resources) || launcher
      if (launcher) on({ type: "listed" })
      else if (shown.getSnapshot().context.connection === "online") on((await behind().catch(() => false)) ? { type: "tooOld" } : { type: "none", only: await only() })
      on({ type: "updateReady", ready: !phone && (await behind().catch(() => false)) })
    }
  } catch {
    // Nothing kept, and its network didn't answer: it's listed again once it does.
  }
}

/// The launcher's page among the files listed, and with them, which app each plugin opens first (`first`) and the
/// Plugins app's page (`plugins`). An app is a page of an MCP App's kind, at ui://<plugin>/<version>/<page>.
/** @param {import("./host.js").Resource[]} resources */
function launcherIn(resources) {
  const pages = resources.filter(({ mimeType }) => mimeType === "text/html;profile=mcp-app")
  const named = (/** @type {string} */ uri) => {
    const [plugin, _version, ...page] = uri.slice("ui://".length).split("/")
    return `${plugin}/${page.join("/")}`
  }
  // The last one set for a plugin is the one kept: its first.
  first = new Map(pages.toReversed().map(({ uri }) => [named(uri).split("/")[0], named(uri)]))
  names = new Map(pages.map(({ uri, name }) => [named(uri), name ?? ""]))
  const found = (/** @type {string} */ category) => pages.find(({ _meta }) => _meta?.category === category)?.uri
  plugins = found("plugins") ? named(/** @type {string} */ (found("plugins"))) : ""
  return found("launcher") ? named(/** @type {string} */ (found("launcher"))) : ""
}

/// Who may install for everyone, if not this person, who then can't install a Home Screen: "Only Andre can install a
/// Home Screen for Everyone", by the names they go by, as the Plugins app says it. Nothing where they can, or where
/// the network can't say.
async function only() {
  /** @param {string} name @param {Record<string, unknown>} [inputs] @returns {Promise<any>} */
  const call = async (name, inputs = {}) => {
    const result = /** @type {{ structuredContent?: unknown, content: { text: string }[], isError?: boolean }} */ (await invoke("request", { view: "client", method: "tools/call", params: { name, arguments: inputs } }))
    if (result.isError) throw new Error(result.content[0]?.text)
    return result.structuredContent ?? JSON.parse(result.content[0].text)
  }
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

/// Loads an app's page again, in place, so the way back still leads out of the app: told first, as an MCP App is
/// before its host takes it down, and what it was answered forgotten.
/** @param {View} view */
function reloadPage(view) {
  post(view, { id: "teardown", method: "ui/resource-teardown", params: { reason: "A new version" } })
  invoke("close", { view: view.key }).finally(() => /** @type {Window} */ (view.frame.contentWindow).location.replace(address(view)))
}

/// Says over the app that's showing what its page needs of its person, if anything: that it's been updated, with
/// Reload, or that it didn't open, with Try Again.
function notes() {
  const view = showing(history.state)
  const now = view?.page.getSnapshot()
  byId("update").hidden = !now?.matches("stale")
  byId("page-problem").hidden = !now?.matches("failed")
  if (view && now?.matches("failed")) {
    byId("page-problem-heading").textContent = `${names.get(view.app) ?? "This app"} didn't open`
    byId("page-problem-details-text").textContent = now.context.why ?? ""
  }
}
byId("reload").addEventListener("click", () => {
  const view = showing(history.state)
  if (view) page(view, { type: "reload" })
})
byId("page-problem-button").addEventListener("click", () => {
  const view = showing(history.state)
  if (!view) return
  view.tapped = performance.now()
  page(view, { type: "retry" })
})

/// Starts an app: its page loads in a frame of its own, behind what's showing until it's shown.
/** @param {string} app @param {string} [subject] @returns {View} */
function load(app, subject) {
  const at = key(app, subject)
  const opened = open.get(at)
  if (opened) return opened
  if (!listed.has(subject)) list(subject)
  const place = { key: at, app, subject, host: host(app, subject) }
  const frame = Object.assign(document.createElement("iframe"), { src: address(place), inert: true })
  /** @type {View} */
  const view = Object.assign({ ...place, frame }, { page: /** @type {View["page"]} */ (/** @type {unknown} */ (undefined)) })
  view.page = createActor(pageMachine, { input: view })
  view.page.subscribe(() => notes())
  view.page.start()
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
  if (plain || shownByDevice) return void change()
  document.startViewTransition({ update: change, types: [back ? "back" : "forward"] })
}

/// Shows the app that's open as `at` over the rest, or with none there, the launcher. `tapped` is when its icon
/// was tapped.
/** @param {string | null} at @param {number} tapped */
function show(at, tapped) {
  const view = layer(at)
  if (!view) return
  view.tapped = tapped
  // Opened again, a stale page loads its new version as it opens.
  page(view, { type: "opened" })
  notes()
  if (!view.page.getSnapshot().matches("current")) return
  // Its user returned to it.
  invoke("refresh", { view: view.key })
  opened(view)
}

/// Has the app that's open as `at` showing, or with none there, the launcher, and the launcher told which, so it
/// knows whether it's showing. → the one showing.
/** @param {string | null} at */
function layer(at) {
  const view = showing(at)
  const home = open.get(key(launcher))
  for (const other of open.values()) {
    other.frame.inert = other !== view
    if (other !== view) page(other, { type: "away" })
  }
  if (home && view) post(home, { method: "notifications/one/showing", params: { app: view.app, subject: view.subject } })
  gestures()
  return view
}

/// Whether the device's own back and forward gestures are on: an iPhone's swipe in from the edge, a trackpad's swipe.
/// They go through this window's history, so they're off while something is open over the app that's showing, as a
/// phone's are under a sheet, and its Back or Escape shuts that first.
let swiping = true
/// Whether back has anything to go back from: an app over the launcher, or something open over the app that's
/// showing. Where it hasn't, Android shows going home as its back gesture is made (`MainActivity`).
/** @type {boolean | undefined} */
let backs
function gestures() {
  const view = showing(history.state)
  const on = !view?.over
  if (on !== swiping) invoke("navigation_gestures", { on })
  swiping = on
  const back = !on || (!!view && view.app !== launcher)
  if (back !== backs) globalThis.androidBack?.postMessage(String(back))
  backs = back
}
/// The app that's open as `at`, or with none there, the one under every other: the launcher, or while there's none,
/// the Plugins app.
/** @param {string | null} at */
const showing = (at) => open.get(at ?? "") ?? open.get(key(shown.getSnapshot().matches("plugins") ? plugins : launcher))

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
    // Their home has drawn over the steps.
    if (view.app === launcher) on({ type: "drawn" })
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

/// The haptics an app may ask for, by iOS's names.
/** @type {unknown[]} */
const HAPTICS = ["light", "medium", "heavy", "rigid", "soft", "selection", "success", "warning", "error"]
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
        safeAreaInsets: insets(view.frame),
        // Whether its network answers, and whether an update for this device is ready, which the window keeps it
        // told of: the launcher shows Update while one is.
        connection: shown.getSnapshot().context.connection,
        updateReady: shown.getSnapshot().context.updateReady,
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
  // A haptic plays here, not in the app's frame: only this page reaches the device's, and Chrome lets no frame of
  // another address vibrate.
  if (method === "one/haptic") {
    if (!HAPTICS.includes(params.kind)) throw new Error(`There's no haptic ${params.kind}`)
    await haptic(params.kind)
    return {}
  }
  // What an app is told of its client, and the few things one changes of it.
  if (method === "one/client") {
    if (params.leave) return invoke("sign_out")
    // A phone whose push failed tries again.
    if (params.push) return invoke("push")
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
    // The launcher, loaded again, is told which app is showing.
    if (method === "ui/notifications/initialized" && view.app === launcher) layer(history.state)
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
  if (view.page.getSnapshot().matches("current")) return
  page(view, { type: "drawn" })
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

// A server says which open apps were loaded against a version that's gone.
listen("stale", ({ payload: hosts }) => {
  for (const view of open.values()) if (hosts.includes(view.host)) page(view, { type: "stale" })
})

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

/// A tap on an iPhone's status bar (the app's window there calls this): the app that's showing scrolls to its top.
window.scrollToTop = () => {
  const view = showing(history.state)
  if (view) post(view, { method: "notifications/one/scroll-to-top" })
}

// The device's own way back, and this page's, both end here.
addEventListener("popstate", (event) => {
  const { state } = event
  const tapped = performance.now()
  shownByDevice = Boolean(event.hasUAVisualTransition)
  move(() => show(state, tapped), !open.has(state))
  shownByDevice = false
})

listen("connection", ({ payload }) => on({ type: "connection", connection: payload }))
// Its network is tried again now, rather than when it next would be.
for (const retry of ["reconnect", "banner-retry"]) byId(retry).addEventListener("click", () => invoke("retry"))
byId("problem-button").addEventListener("click", () => on({ type: shown.getSnapshot().matches("needs_updating") ? "update" : "retry" }))
byId("open-plugins").addEventListener("click", () => on({ type: "openPlugins" }))

/// The user returned to the window, so what's showing asks again, and its apps' files are listed again, for a new
/// version of any. While the window is on screen, what's showing asks again when a server says something changed.
function returned() {
  if (!shown.getSnapshot().matches("launcher")) return
  invoke("visible", { onScreen: !document.hidden })
  // Put away, the window's stale pages load their new versions.
  if (document.hidden) return void open.forEach((view) => page(view, { type: "away" }))
  for (const subject of listed) list(subject)
  invoke("refresh", { view: key(launcher) })
  if (open.has(history.state)) invoke("refresh", { view: history.state })
}
document.addEventListener("visibilitychange", returned)
addEventListener("focus", returned)

/// Shows the launcher, once this user's files have listed one: loaded, or loaded again where it was open.
function start() {
  const home = open.get(key(launcher))
  if (home) page(home, { type: "retry" })
  else load(launcher)
  // The app that was showing when the page was loaded again is gone, so the launcher shows.
  if (history.state) history.replaceState(null, "")
  show(null, 0)
}

/// Shows the Plugins app with no launcher under it, which its person installs one with. There's nothing to go back
/// to from it.
function openPlugins() {
  const view = load(plugins)
  if (history.state) history.replaceState(null, "")
  show(view.key, performance.now())
}

/// Follows the device's session as it moves, each move heard once and in order: the page may hear one as it opens
/// before `start` has said where it was.
let heard = -1
/** @param {Told} told */
function hear(told) {
  if (told.seq <= heard) return
  heard = told.seq
  on({ type: "session", told })
}

dress()
shown.subscribe(draw)
shown.start()
await listen("session", ({ payload }) => hear(payload))
// Where the window starts: where its device is with its network, as the client says it (`Session`), and whether its
// network answers (`Connection`), which it follows from then on. A phone's app hears a tap on a notification through
// its push plugin.
const { taps, connection, ...told } = await invoke("start")
phone = Boolean(taps)
on({ type: "connection", connection })
hear(told)
invoke("visible", { onScreen: !document.hidden }).catch(() => {})
if (taps) hearTaps()
