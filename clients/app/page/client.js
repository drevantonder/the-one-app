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
import { address, haptic, invoke, listen, openLink, sandbox, setTheme } from "./host.js"
/** @import { Context, Moved, Paged, Rect, Told, View } from "./host.js" */

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
/// The apps that are open, by who each is for and what it is: `{ key, entrypoint, subject, host, frame, tapped, drawn, over }`.
/// An app stays open behind the one that's showing once it's been opened, as on a phone, so going back to it shows
/// it as it was.
/** @type {Map<string, View>} */
const open = new Map()
/** @param {string} entrypoint @param {string} [subject] */
const key = (entrypoint, subject) => `${subject ?? ""}|${entrypoint}`
/// Each app's first app, by the app's name, among this user's: what a notification from it, or a `one:` link to it, opens.
/** @type {Map<string, string>} */
let first = new Map()
/// Each app's name, by its page, as this user's apps list them.
/** @type {Map<string, string>} */
let names = new Map()
/// Who this client has listed files for: its user (`undefined`), and each group or the network an app was opened
/// for.
/** @type {Set<string | undefined>} */
const listed = new Set()

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
// Where the window goes from here, a sign-in page, isn't told Colour Mode: the bars follow the device there, as it does.
addEventListener("pagehide", () => setTheme(null))

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

const { assign, createActor, enqueueActions, fromPromise, sendParent, sendTo, setup, spawnChild } = XState

/// Each open app's page, one state at a time, as its person's owner signed it off, a child of the window's
/// (`machine`), which starts it as the app opens and tells it whether it's in front of its person: loading, then
/// current until its app or the UI library is updated under it, which makes it stale. A stale page loads again by
/// itself only at a boundary, as an iPhone updates an app while it's closed and a web app loads its new version on the
/// next navigation: once it's off screen (another app over it, the window hidden, or the window showing something of
/// its own over every app, as Update Required), or as it's opened again. On screen, it says so quietly, "Updated ·
/// Reload", and loads again only once its person presses Reload: nothing is reloaded under them. One that's showing an
/// error loads again at once, as its old version meeting what's new is likely why, and there's nothing of theirs under
/// it. A page that doesn't draw in 15 s on screen has failed, with Try Again, until it draws after all, as a page
/// loaded again behind another app may only once it's shown, when it's current. Only time it's on screen counts: while
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
        { guard: "errored", target: "reloading" },
      ],
      on: { away: "reloading", opened: "reloading", reload: "reloading", errored: "reloading" },
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

/// What the window shows, one state at a time: while it opens, until it can start the launcher; the launcher, once
/// it has drawn; signing in, where the device isn't in its network; why there's no launcher to show, with Apps to
/// install one; and an update, which is all it shows once a server has refused this client for being behind it
/// (`behind` from core/client), as an app that requires an update does. Only `on` moves it, by the moves `machine`
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
const machine = setup({
  types: /** @type {{ context: Context, events: Moved }} */ ({}),
  actors: {
    page: pageMachine,
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
    launcher: ({ event }) => (event.type === "drawn" || event.type === "failed") && event.key === key(launcher),
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
  context: { connection: "connecting", lost: false, starting: false, updateReady: false },
  on: {
    session: { actions: "told" },
    connection: { actions: "connection" },
    updateReady: { actions: "updateReady" },
    link: { actions: "link" },
    load: { actions: "load" },
    page: { actions: "page" },
    // The window hidden, every page is off screen; shown again, the one in front is on screen, where it shows apps.
    away: { actions: "away" },
    seen: { actions: "seen" },
  },
  states: {
    // Waits until it can start the launcher: its device in its network and its network answering, or a launcher
    // kept from before.
    opening: {
      tags: "pages",
      entry: [assign({ starting: false }), "list", "seen"],
      on: {
        session: [{ guard: "signIn", target: "sign_in", actions: "told" }, { actions: ["told", "list"] }],
        connection: [{ guard: "online", actions: ["connection", "list"] }, { actions: "connection" }],
        listed: { guard: "waiting", actions: [assign({ starting: true }), "start"] },
        drawn: { guard: "launcher", target: "launcher" },
        none: { target: "no_launcher", actions: assign({ only: ({ event }) => event.only }) },
        tooOld: "needs_updating",
        failed: { guard: "launcher", target: "launcher_failed", actions: assign({ why: ({ event }) => event.why }) },
      },
    },
    // The network's own sign-in page, which the client brings the window back from once its person has approved
    // it, where it opens again (`follows` in `src/lib.rs`).
    sign_in: { entry: ["signIn", "away"] },
    launcher: {
      tags: "pages",
      entry: ["listen", "follow", "seen"],
      on: { update: { guard: "updateReady", target: "updating" }, tooOld: "needs_updating", link: { actions: ["link", "follow"] } },
    },
    // A launcher installed meanwhile, as by an agent, is listed as its network says something changed.
    no_launcher: {
      entry: "away",
      on: { openAppStore: { guard: "appStore", target: "app_store" }, listed: "opening", tooOld: "needs_updating" },
    },
    // The App Store, shown with no launcher under it, until a launcher is installed: it's the page in front.
    app_store: {
      tags: "pages",
      entry: ["openAppStore", "seen"],
      on: { listed: "opening", tooOld: "needs_updating" },
    },
    // A launcher that draws after all is shown, as an app's page that does is (`pageMachine`).
    launcher_failed: {
      tags: "pages",
      entry: "seen",
      on: { retry: "opening", drawn: { guard: "launcher", target: "launcher" }, tooOld: "needs_updating" },
    },
    // Update Required: Update on a computer, and everywhere, Try Again, as the window comes back on screen, and
    // as its network answers again.
    needs_updating: {
      entry: [assign({ behind: true }), "away"],
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
      entry: "away",
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
      entry: [assign({ message: undefined }), "away"],
      invoke: {
        src: "update",
        onDone: { target: "checking", actions: assign({ message: ({ event }) => event.output }) },
        onError: { target: "update_failed", actions: assign({ why: ({ event }) => `${event.error}` }) },
      },
    },
    // Try Again asks its network afresh, as `checking` does, never repeating what an update said before: where
    // it talks with this app now, it opens; where it doesn't, Update Required, with Update.
    update_failed: {
      entry: "away",
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
/// Whether an app's page is in front of its person: the window's answer, the one showing while it shows apps, in a
/// window that's on screen.
/** @param {View} view @returns {boolean} */
const onScreen = (view) => !document.hidden && shown.getSnapshot().hasTag("pages") && view === showing(history.state)

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
  // Being behind isn't a problem: it's an alert, as iOS's Update Required, which always has a way on. A computer's
  // app updates itself, with Update; a phone's is updated by its network's owner, so there it has Try Again, and
  // Update too where it came from TestFlight, which opens TestFlight.
  const required = /** @type {HTMLDialogElement} */ (byId("update-required"))
  byId("update-button").hidden = phone && !testflight
  byId("update-owner").hidden = !phone || testflight
  byId("try-again-button").hidden = !phone
  byId("try-again-button").toggleAttribute("disabled", now.matches("checking"))
  byId("try-again-button").ariaBusy = now.matches("checking") ? "true" : null
  if (behind && !required.open) required.showModal()
  if (!behind && required.open) required.close()
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
  }
  // What's shown is reported once, as it's shown.
  const said = behind ? "Update Required: This version of The One App no longer works with your network." : problem && `${problem[0]}: ${problem[1]}`
  if (said && said !== lastProblem) report("one.client.problem", said)
  lastProblem = said
  notes()
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
/// whose apps have a new version loads again. This user's own list has the launcher in it, which starts, or none:
/// which is a server's word, so a list kept from before is asked for again first, and while its network doesn't
/// answer, the window waits. With none, this app needs updating where its network says an update is ready for it
/// (`behind`): otherwise it's the network that has none right now. A launcher missing from a later list, as during a
/// release, leaves the one that's open as it is. Each of this user's lists says again whether an update is ready.
/** @param {string} [subject] */
async function list(subject) {
  listed.add(subject)
  try {
    let { resources, outdated } = await invoke("apps", { subject })
    // A screen runs its own app and the UI library, which every one of them runs.
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
  names = new Map(pages.map(({ uri, name }) => [named(uri), name ?? ""]))
  const found = (/** @type {string} */ category) => pages.find(({ _meta }) => _meta?.category === category)?.uri
  appStore = found("app-store") ? named(/** @type {string} */ (found("app-store"))) : ""
  const home = found("launcher") ? named(/** @type {string} */ (found("launcher"))) : ""
  // The launcher's app opens the launcher, as `one:home-screen` does.
  if (home) first.set(home.split("/")[0], home)
  return home
}

/// Who may install for everyone, if not this person, who then can't install a Home Screen: "Only Andre can install a
/// Home Screen for Everyone", by the names they go by, as the App Store says it. Nothing where they can, or where
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

/// Says over the app that's showing what its page needs of its person, if anything: that it's been updated, with
/// Reload, or that it didn't open, with Try Again: only while the window shows its apps, as it alone owns the screen.
function notes() {
  const view = showing(history.state)
  const now = view && shown.getSnapshot().hasTag("pages") ? pageOf(view) : undefined
  byId("update").hidden = !now?.matches("stale")
  byId("page-problem").hidden = !now?.matches("failed")
  if (view && now?.matches("failed")) {
    byId("page-problem-heading").textContent = `${names.get(view.entrypoint) ?? "This app"} didn't open`
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

/// Starts an app: its page loads in a frame of its own, behind what's showing until it's shown. A `fragment` is the
/// place in its page a link names, which the app follows (its `hashchange`), open already or not.
/** @param {string} entrypoint @param {string} [subject] @param {string} [fragment] @returns {View} */
function load(entrypoint, subject, fragment) {
  const at = key(entrypoint, subject)
  const opened = open.get(at)
  if (opened && fragment) opened.frame.src = `${located(opened)}#${fragment}`
  if (opened) return opened
  if (!listed.has(subject)) list(subject)
  const place = { key: at, entrypoint, subject, host: host(entrypoint, subject) }
  const frame = Object.assign(document.createElement("iframe"), { src: `${located(place)}${fragment ? `#${fragment}` : ""}`, inert: true })
  /** @type {View} */
  const view = { ...place, frame }
  view.frame.setAttribute("sandbox", sandbox)
  if (entrypoint === launcher) view.frame.className = "launcher"
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

/// The device's status bar, as the app that's showing would have it: shown unless it asked otherwise.
let statusBar = true
function bar() {
  const shown = front?.statusBar !== false
  if (shown === statusBar) return
  statusBar = shown
  invoke("status_bar", { shown })
}

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let shownByDevice = false

/// Where `rect`, a place in a frame's page, is in the window: an icon an app is opened from.
/** @param {HTMLIFrameElement} frame @param {Rect} [rect] @returns {Rect | undefined} */
function within(frame, rect) {
  if (!rect || ![rect.x, rect.y, rect.width, rect.height].every(Number.isFinite) || !(rect.width > 0 && rect.height > 0)) return undefined
  const { x, y } = frame.getBoundingClientRect()
  return { x: x + rect.x, y: y + rect.y, width: rect.width, height: rect.height }
}

/// Changes between the launcher and an app as a move, as the UI library moves between an app's screens
/// (core/ui/src/screen.tsx): the browser slides the app in over the launcher, or with `back`, back out from over
/// them. An app opened from its icon (`from`) grows out of it instead, as an iPhone's does, and goes back into it.
/// For whoever asked for less motion, one fades into the other, and where the browser can't, it's a plain swap.
/** @param {() => void} change @param {Rect} [from] */
function move(change, back = false, from) {
  if (!globalThis.ViewTransition || !("types" in ViewTransition.prototype) || shownByDevice) return void change()
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return void document.startViewTransition({ update: change, types: ["fade"] })
  if (!from) return void document.startViewTransition({ update: change, types: [back ? "back" : "forward"] })
  // The app, drawn as small as the icon is wide, its middle on the icon's, and cut to a square of it with the icon's
  // corners (client.css's `grow`): the window's size is the app's, and the icon's is a share of its width.
  const [width, height] = [innerWidth, innerHeight]
  const scale = from.width / width
  const root = document.documentElement.style
  root.setProperty("--icon-x", `${from.x + from.width / 2 - width / 2}px`)
  root.setProperty("--icon-y", `${from.y + from.height / 2 - height / 2}px`)
  root.setProperty("--icon-scale", String(scale))
  root.setProperty("--icon-inset", `${Math.max(0, (height - width) / 2)}px ${Math.max(0, (width - height) / 2)}px`)
  root.setProperty("--icon-radius", `${(from.width * 0.225) / scale}px`)
  document.startViewTransition({ update: change, types: [back ? "close" : "open"] })
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
  const back = !on || (!!view && view.entrypoint !== launcher)
  if (back !== backs) globalThis.androidBack?.postMessage(String(back))
  backs = back
}
/// The app that's open as `at`, or with none there, the one under every other: the launcher, or while there's none,
/// the App Store.
/** @param {string | null} at */
const showing = (at) => open.get(at ?? "") ?? open.get(key(shown.getSnapshot().matches("app_store") ? appStore : launcher))

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
  // A file too large to go in a call: the client sends it where the network keeps it, for a tool of that app.
  if (method === "one/upload") return invoke("upload", await params.file.arrayBuffer(), { headers: { app: view.entrypoint.split("/")[0] } })
  // A person's recovery key is shown and typed on this page, which is the client's own: no app's page is handed one.
  if (method === "one/recovery-key") return recoveryKey(params.typed === true, params.subject ?? view.subject)
  // Another app, shown over this one, as tapping its icon or a notification from it does: by its app's name,
  // and its page if it isn't the app's first app, for whoever this one is for unless it says. `behind` starts
  // it as a finger lands on its icon, and it shows as the finger lifts, out of the icon at `from` in its page.
  if (method === "ui/open-link" && params.url?.startsWith("one:")) {
    if (!go(params.url.slice("one:".length), params.subject ?? view.subject, params.behind, within(view.frame, params.from))) throw new Error("That app isn't on this device")
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
    // The device's status bar, hidden while this app shows, as an iPhone's Home Screen hides it while it's edited.
    if ("statusBar" in params) {
      view.statusBar = params.statusBar !== false
      bar()
      return { message: "" }
    }
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
    // An app's page logs its errors as an MCP App logs to its host, and they're its app's, whatever it says.
    if (method === "notifications/message" && params.level === "error") {
      exception(params.data?.message, params.data?.file, params.data?.line, view.entrypoint.split("/")[0])
      page(view, { type: "errored" })
    }
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
  if (pageOf(view)?.matches("current")) return
  page(view, { type: "drawn" })
  if (!view.frame.inert) opened(view)
})

/// Shows an app over whatever's showing, by what a `one:` link says after its scheme: `<app>[/<page>][#<fragment>]`,
/// its app's name, and its page if it isn't the app's first app, and the place in it, for `subject`. With `behind`
/// it only starts. The launcher, linked to, is gone back to. → whether there is one here.
/** @param {string} link @param {string} [subject] @param {boolean} [behind] @param {Rect} [from] */
function go(link, subject, behind, from) {
  const [entrypoint, fragment] = link.split("#", 2)
  const page = entrypoint.includes("/") ? entrypoint : first.get(entrypoint)
  if (!page) return false
  const tapped = performance.now()
  const next = load(page, subject, fragment)
  if (behind) return true
  if (next.entrypoint === launcher) {
    if (history.state) history.back()
    return true
  }
  const showing = history.state === next.key
  // It goes back into the icon it came out of, or slides out if it came from none.
  next.from = from
  move(() => show(next.key, tapped), false, from)
  // The way back out of an app is an entry in this window's history, made as it's opened, so the device's back
  // leads out of the app however soon it's pressed.
  if (!showing) history.pushState(next.key, "")
  return true
}

// A notification from an app was tapped, or a `one:` link opened (`links` in src/lib.rs).
listen("open", ({ payload: link }) => on({ type: "link", link }))
/// A phone's push plugin says a notification was tapped, even the tap that started the app, once this page hears
/// it, which is once there are apps to open.
async function hearTaps() {
  await window.__TAURI__.core.addPluginListener("notifications", "notificationClicked", (tapped) => invoke("tapped", { tapped }))
  await invoke("plugin:notifications|set_click_listener_active", { active: true })
}

// A server refused this client for being behind it: it asks nothing more, and says only that it needs updating.
listen("behind", () => on({ type: "tooOld" }))

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
  // Back to the launcher, the app that was showing goes back into the icon it came out of.
  const back = !open.has(state)
  move(() => show(state, tapped), back, back ? front?.from : undefined)
  shownByDevice = false
})

listen("connection", ({ payload }) => on({ type: "connection", connection: payload }))
// Its network is tried again now, rather than when it next would be.
for (const retry of ["reconnect", "banner-retry"]) byId(retry).addEventListener("click", () => invoke("retry").catch(() => {}))
for (const retry of ["problem-button", "try-again-button"]) byId(retry).addEventListener("click", () => on({ type: "retry" }))
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

/// Shows the App Store with no launcher under it, which its person installs one with. There's nothing to go back
/// to from it.
function openAppStore() {
  const view = load(appStore)
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
const { taps, testflight: fromTestflight, connection, open: link, ...told } = await invoke("start")
phone = Boolean(taps)
testflight = Boolean(fromTestflight)
on({ type: "connection", connection })
if (link) on({ type: "link", link })
hear(told)
invoke("visible", { onScreen: !document.hidden }).catch(() => {})
if (taps) hearTaps()
