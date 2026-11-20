// The window's moves between its launcher and its apps, as a state machine (`moveMachine` is a child of the window's, `window-machine.js`).
import { backHome, depth, front, layer, notes, reveal, showing, stack } from "./client.js"
import { covering, home } from "./gestures.js"
/** @import { Rect, Shown, View } from "./host.js" */
const { assign, fromCallback, setup } = XState

/// What each of a move's two frames does over it, as keyframes from its start to its end, and how long it takes: an
/// app opened from its icon or widget (`icon`) grows out of it as an iPhone's does, showing itself over it in its first
/// moments, as the launcher sinks back under it; going back, it shrinks back into it as the launcher comes forward, and
/// fades into it as it gets there. With no icon, it slides in over what was there from the end of the line, which moves
/// back a quarter, and going back, back out. Pulled home by the rail's line (`pulled`), as iOS's swipe home is, its
/// first part (to `HELD`) follows the finger in, its right edge under it, shrinking to a card over the Home Screen,
/// dimmed; let go, it goes on into its icon, or with none, shrinks away. For whoever asked for less motion, what comes
/// fades in over what was there. The one drawn over the other is `upper`, with `corner` its corners' radius while it
/// moves: a phone's, which its screen's own hide while it fills it, as an iPhone's app is. What moves is only moved,
/// grown and faded, which a browser draws without drawing the app again, as Core Animation does a layer: a clip that
/// changed as it moved had WebKit draw the app again each frame, half of them late.
/** @param {View} leaving @param {View} coming @param {boolean} back @param {Rect} [icon] @param {boolean} [pulled] @returns {{ upper: View, ms: number, corner?: number, parts: [View, Keyframe[]][] }} */
function moves(leaving, coming, back, icon, pulled) {
  const style = getComputedStyle(document.documentElement)
  if (matchMedia("(prefers-reduced-motion: reduce)").matches) return { upper: coming, ms: parseFloat(style.getPropertyValue("--move-settle")), parts: [[coming, [{ opacity: 0 }, { opacity: 1 }]]] }
  const [app, under] = back ? [leaving, coming] : [coming, leaving]
  const { x, y, width, height } = app.frame.getBoundingClientRect()
  // The app in its icon: its middle on the icon's, as large as fits in it.
  const small = icon && {
    translate: `${icon.x + icon.width / 2 - (x + width / 2)}px ${icon.y + icon.height / 2 - (y + height / 2)}px`,
    scale: `${Math.min(icon.width / width, icon.height / height)}`,
  }
  const whole = { scale: "1", translate: "0px 0px" }
  if (pulled) {
    // The card at `HELD`: its right edge as far in as the finger, `HELD` of the width.
    const held = { scale: `${CARD}`, translate: `${(width / 2) * (1 - CARD) - HELD * width}px 0px`, offset: HELD }
    const gone = small ?? { scale: "0.5", translate: `${-0.3 * width}px 0px` }
    return {
      upper: app,
      ms: 400,
      corner: CORNER / CARD,
      parts: [
        [app, [whole, held, gone]],
        [app, [{ opacity: 1 }, { opacity: 1, offset: small ? HELD + (1 - HELD) * FADES : HELD }, { opacity: 0 }]],
        // The Home Screen behind it dimmed at first, as iOS's is, so the card stands out from it: dimmed, not sunk,
        // so no undimmed edge shows round it.
        [under, [{ filter: "brightness(0.7)" }, { filter: "brightness(1)" }]],
      ],
    }
  }
  if (small) {
    const sunk = { scale: "0.92" }
    const up = { scale: "1" }
    return {
      upper: app,
      ms: 400,
      corner: CORNER / CARD,
      parts: back
        ? [[app, [whole, small]], [app, [{ opacity: 1 }, { opacity: 1, offset: FADES }, { opacity: 0 }]], [under, [sunk, up]]]
        : [[app, [small, whole]], [app, [{ opacity: 0 }, { opacity: 1, offset: 1 - FADES }, { opacity: 1 }]], [under, [up, sunk]]],
    }
  }
  const end = style.direction === "rtl" ? -1 : 1
  const at = (/** @type {number} */ share) => ({ translate: `${share * end}% 0px` })
  return {
    upper: app,
    ms: parseFloat(style.getPropertyValue("--move-travel")),
    parts: back ? [[app, [at(0), at(100)]], [under, [at(-25), at(0)]]] : [[app, [at(100), at(0)]], [under, [at(0), at(-25)]]],
  }
}

/// How far through the pull home its first part goes, where the app is a card (`moves`), how small, and its corners.
export const HELD = 0.4
const CARD = 0.8
const CORNER = 40
/// How far into its icon an app has gone as it starts to fade into it, and out of it as it has shown itself.
const FADES = 0.75

/// Takes `body.p` to `body.target` as a critically damped spring, as the Home Screen's pages have
/// (apps/home-screen/client/pager.ts), from as fast as it's going (`velocity`), drawing each frame (`seek`) until it's
/// still there (`settled`). Its `stiffness` is set so that from still it lands as the ease it replaces did, in the
/// move's time: within 3 % of it all the way. → what stops it.
/** @param {{ p: number, target: number, velocity: number, stiffness: number }} body @param {(p: number) => void} seek @param {() => void} settled */
function spring(body, seek, settled) {
  let last = performance.now()
  /** @param {number} now */
  const step = (now) => {
    // A frame late, as when the window was hidden, is taken as one, so nothing jumps.
    const dt = Math.max(0, Math.min(1 / 30, (now - last) / 1000))
    last = now
    let { p, velocity } = body
    for (let at = 0; at < dt; at += 1 / 240) {
      const slice = Math.min(1 / 240, dt - at)
      velocity += (-body.stiffness * (p - body.target) - 2 * Math.sqrt(body.stiffness) * velocity) * slice
      p += velocity * slice
    }
    const still = Math.abs(p - body.target) < 0.001 && Math.abs(velocity) < 0.05
    body.velocity = still ? 0 : velocity
    seek(still ? body.target : p)
    if (still) settled()
    else frame = requestAnimationFrame(step)
  }
  let frame = requestAnimationFrame(step)
  return () => cancelAnimationFrame(frame)
}
/// How stiff a spring is that lands in `ms` from still.
/** @param {number} ms */
const stiffness = (ms) => (27 * 400 / ms) ** 2

/// The window's moves between its launcher and its apps, and from app to app, one at a time: a child of the window's
/// (`machine`), which tells it where to go only while it shows apps (tagged `pages`), and to stop where it was going
/// (`freeze`) as it covers them all. The Home Screen is never told: it's a frame like any other here.
///
/// The launcher and every open app are live frames, so a move animates the two frames themselves (Web Animations),
/// both live and taking taps all the while: a tap on either is never lost. Its animations (`moves`) are paused, and
/// one progress `p`, 0 where it came from and 1 where it goes, sets where they are; a spring takes `p` to where it's
/// going, so turning back, or a finger letting go, carries on from where it is, as fast as it was going.
///
///                     show(to): its history entry is there already
///     ┌───────┐ ─────────────────────────────────► ┌───────────┐ ─┐ show(where it came from): turns back, p → 0
///     │ still │                                    │  playing  │ ◄┘ show(where it goes): nothing new
///     └───────┘ ◄── settled: layer(where p is) ─── │ from → to │ ─┐ show(anything else): ends where it was going,
///         │         and notes()                    └───────────┘ ◄┘   then moves from there to it
///         │                                         │       ▲
///         │                                   grab: │       │ release: p → 1 if far or flicked, else → 0,
///         │                                  caught │       │ or where the device's gesture ended; the
///         │                                         │       │ window's history follows it
///         │                              where p is ▼       │
///         │   grab [back leads somewhere: an   ┌───────────┐
///         │   app at its first screen, with    │ tracking  │  drag(p): drawn there
///         └──────── nothing over it] ────────► └───────────┘
///             (with `home`, an app at any of its screens)
///
///     freeze, from any: still, where it was going, which is always where the window's history is.
///     show, while tracking: as while playing, the finger keeping it unless it goes somewhere else.
///     popped(to), drawn already by the device: still at `to`, with nothing drawn; while playing toward it, nothing.
///
/// A computer's trackpad swipe sends `grab`, `drag` and `release` (`wheel`), as do Android's back gesture
/// (`backStarted`), an iPhone's swipe in from the edge (`window.grab`), and a finger pulling the rail's line, whose
/// `grab` says `home`: from any of an app's screens, it goes back to the Home Screen's entry, past the app's own.
/** @typedef {{ from: View, to: View, upper: View, parts: Animation[], p: number, target: number, velocity: number, stiffness: number }} Move */
/** @param {Move} move */
const toward = (move) => (move.target === 1 ? move.to : move.from)
/** @param {Move} move @param {number} p */
function seek(move, p) {
  move.p = p
  for (const part of move.parts) part.currentTime = p * 1000
}
export const moveMachine = setup({
  types: /** @type {{ context: { move?: Move }, events: Shown }} */ ({}),
  actors: {
    spring: fromCallback(/** @param {{ input: Move, sendBack: (event: Shown) => void }} spring */ ({ input: move, sendBack }) => spring(move, (p) => seek(move, p), () => sendBack({ type: "settled" }))),
  },
  guards: {
    here: ({ event }) => "to" in event && event.to === front,
    there: ({ context, event }) => "to" in event && !!context.move && event.to === toward(context.move),
    returning: ({ context, event }) => "to" in event && !!context.move && event.to === (context.move.target === 1 ? context.move.from : context.move.to),
    // The line takes an app home from any of its screens, as long as nothing is drawn over it; back only from its first.
    backs: ({ event }) => ("home" in event && event.home ? depth > 0 && !front?.layer && !covering() : home()),
  },
  actions: {
    // The frame coming is seen and takes taps from the start, over or under the one going, and its page is told
    // it's opened.
    begin: assign(({ event }) => {
      const pulled = event.type === "grab" && event.home === true
      // Pulled home, it goes into the icon it came out of only where it came from the Home Screen.
      const grabbed = () => (pulled ? { to: showing(null), icon: depth === 1 ? stack[1].from : undefined } : { to: showing(stack[depth - 1].key), icon: stack[depth].from })
      const show = /** @type {Extract<Shown, { type: "show" }>} */ (event.type === "grab" ? { ...grabbed(), back: true, tapped: performance.now() } : event)
      const from = /** @type {View} */ (front)
      const { upper, ms, corner, parts } = moves(from, show.to, show.back, show.icon, pulled)
      show.to.frame.inert = false
      upper.frame.dataset.move = ""
      upper.frame.style.borderRadius = corner ? `${corner}px` : ""
      const animations = parts.map(([view, keyframes]) => {
        const part = view.frame.animate(keyframes, { duration: 1000, fill: "both" })
        part.pause()
        return part
      })
      reveal(show.to, show.tapped)
      /** @type {Move} */
      // It goes where the window's history is: a finger that takes it from still hasn't gone anywhere yet.
      const move = { from, to: show.to, upper, parts: animations, p: 0, target: event.type === "grab" ? 0 : 1, velocity: 0, stiffness: stiffness(ms) }
      seek(move, 0)
      return { move }
    }),
    turn: ({ context }) => {
      const move = /** @type {Move} */ (context.move)
      move.target = 1 - move.target
    },
    // Ends where it was going, or with `popped`, where the device went.
    end: assign(({ context, event }) => {
      const move = context.move
      const to = event.type === "popped" ? event.to : move && toward(move)
      for (const part of move?.parts ?? []) part.cancel()
      if (move) {
        delete move.upper.frame.dataset.move
        move.upper.frame.style.borderRadius = ""
      }
      if (to) layer(to.key)
      notes()
      return { move: undefined }
    }),
    // Shown without a move: where it is already, or drawn by the device.
    place: ({ event }) => {
      if (!("to" in event)) return
      layer(event.to.key)
      reveal(event.to, event.tapped)
      notes()
    },
    // Let go far enough or fast enough, it carries on, and otherwise goes back; the window's history follows it.
    release: ({ context, event }) => {
      const move = /** @type {Move} */ (context.move)
      const { velocity, end } = /** @type {Extract<Shown, { type: "release" }>} */ (event)
      move.target = end ?? (Math.abs(velocity) > 1 ? (velocity > 0 ? 1 : 0) : Math.round(move.p))
      move.velocity = velocity
      const to = toward(move)
      if (move.target === 1 && depth > 0 && to === showing(null)) backHome()
      else for (const step of [-1, 1]) if (stack[depth + step] && showing(stack[depth + step].key) === to) history.go(step)
    },
  },
}).createMachine({
  id: "move",
  initial: "still",
  context: {},
  on: { freeze: { target: ".still", actions: "end" } },
  states: {
    still: {
      on: {
        show: [{ guard: "here", actions: "place" }, { target: "playing", actions: "begin" }],
        popped: { actions: "place" },
        grab: { guard: "backs", target: "tracking", actions: "begin" },
      },
    },
    playing: {
      invoke: { src: "spring", input: ({ context }) => /** @type {Move} */ (context.move) },
      on: {
        show: [{ guard: "there" }, { guard: "returning", actions: "turn" }, { target: "playing", reenter: true, actions: ["end", "begin"] }],
        popped: [{ guard: "there" }, { target: "still", actions: "end" }],
        settled: { target: "still", actions: "end" },
        grab: "tracking",
      },
    },
    tracking: {
      on: {
        show: [{ guard: "there" }, { guard: "returning", actions: "turn" }, { target: "playing", actions: ["end", "begin"] }],
        drag: { actions: ({ context, event }) => seek(/** @type {Move} */ (context.move), event.p) },
        release: { target: "playing", actions: "release" },
        popped: { target: "still", actions: "end" },
      },
    },
  },
})
