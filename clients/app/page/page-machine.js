// An open app's page, as a state machine (`pageMachine` is a child of the window's, `window-machine.js`).
import { asksToReload } from "./host.js"
import { onScreen, reloadPage } from "./client.js"
import { report } from "./util.js"
/** @import { Paged, View } from "./host.js" */
const { assign, sendParent, setup } = XState

/// How long an app may take from its tap to drawing before it's reported as never having opened.
const NEVER = 15000
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
export const pageMachine = setup({
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
    // Its renderer couldn't open it, and says why, so it has failed at once (`unopened`).
    unopened: assign({ why: ({ event }) => (event.type === "unopened" ? event.why : undefined) }),
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
      on: { drawn: "current", stale: "reloading", retry: "reloading", unopened: { target: "failed", actions: "unopened" } },
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
      on: { drawn: "current", stale: { target: "reloading", reenter: true }, retry: { target: "reloading", reenter: true }, unopened: { target: "failed", actions: "unopened" } },
      ...waiting,
    },
    failed: {
      entry: ["failed", "tellFailed"],
      on: { drawn: "current", retry: { target: "loading", actions: ["clear", "load"] } },
    },
  },
})
