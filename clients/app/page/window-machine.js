// What the window shows, one state at a time, as a state machine (`machine`), with each open app's page and its move as its children.
import { invoke } from "./host.js"
import { appStore, list } from "./apps.js"
import { post } from "./bridge.js"
import { baseKey, go, open, openAppStore, own, phone, start, testflight } from "./client.js"
import { moveMachine } from "./move-machine.js"
import { pageMachine } from "./page-machine.js"
/** @import { Context, Moved } from "./host.js" */
const { enqueueActions, assign, fromPromise, sendTo, setup, spawnChild } = XState

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
export const machine = setup({
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
        failed: { guard: "launcher", target: "launcher_failed", actions: assign({ why: ({ event }) => event.why }) },
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
