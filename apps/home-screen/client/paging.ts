import { createEffect, createSignal, onSettled, untrack } from "solid-js"
import { type WheelEventData, WheelGestures } from "wheel-gestures"
import { createVelocityTracker, type Dragged, haptic, keydown, type Pressed, reducedMotion, viewport, type Wheeled } from "ui"
import { give, glide, jump, settle, spring, type Spring, stop } from "./pager.ts"
import { ACROSS, card as cardOf, cell as cellOf, INSET, lefts, PAGE, PILLS, CARD_TOP } from "./layout.ts"
import { type At, carries } from "./machine.ts"
import { home, setSeen, setAlong, setRolled, launcher, room, laidPage, quietFor, finger, state } from "./store.ts"
import { type Owner, draft, keep } from "./owners.ts"
import { type Screens } from "./screens.ts"

export function createPaging(props: { owners: Owner[] }, layout: Screens) {
  const { owned, pages, screens, homes, kindOf } = layout
  /// How tall and wide a screenful is, and how far its owners and its screenfuls are moved: the owners up and down
  /// (`down`), and each owner's along Home's screenfuls and on to the App Library (`across`), which the owner shown
  /// takes the sections past Home along, as the pager moves them.
  const height = () => untrack(room).height
  const pitch = () => untrack(room).width
  const stops = (owner: number) => untrack(screens)[owner]?.length || 1
  const homeEnd = (owner: number) => Math.max(0, untrack(() => homes(owner).length) - 1) * pitch()
  const [downY, setDownY] = createSignal(0)
  const xs: [() => number, (x: number) => void][] = []
  const xOf = (owner: number) => (xs[owner] ??= createSignal(0))
  /// How opaque each owner's pages are as they cross-fade, for whoever asked for less motion.
  const opacities: [() => number, (opacity: number) => void][] = []
  const opacityOf = (owner: number) => (opacities[owner] ??= createSignal(1))
  /// The owner shown, nearest where the owners are as they move up and down.
  const shownOwner = () => Math.max(0, Math.min(owned().length - 1, Math.round(down.value / height())))
  const down: Spring = spring((y) => {
    setDownY(y)
    setRolled(y / height())
    look()
    rested(down)
  })
  const across: Spring[] = []
  const sideways = (owner: number) =>
    (across[owner] ??= spring(
      (x) => {
        xOf(owner)[1](x)
        look()
        rested(across[owner])
      },
      (opacity) => opacityOf(owner)[1](opacity),
    ))
  /// What's carried, once the pages under it come to rest.
  const underCarry: { settled?: () => void } = {}
  const rested = (one: Spring | undefined) => one && one.value === one.target && !one.velocity && underCarry.settled?.()
  /// The owner and screenful nearest where they are, which the pills follow.
  const look = () => {
    const owner = shownOwner()
    setSeen({ owner, screen: Math.round(sideways(owner).value / pitch()) })
    setAlong(sideways(owner).value / pitch())
  }
  /// How far the sections are moved past Home, to the App Library: the owner shown's way along, past their Home.
  const sectionsX = () => {
    const owner = Math.max(0, Math.min(props.owners.length - 1, Math.round(downY() / Math.max(1, room().height))))
    return Math.max(0, xOf(owner)[0]() - Math.max(0, homes(owner).length - 1) * room().width)
  }

  const card = () => cardOf(room())
  /// How far an owner's pages are moved along so fewer than fill the card are in its middle.
  const lead = (owner: number) => (Math.max(0, ACROSS - (pages()[owner]?.length ?? 1)) * PAGE) / 2
  const cardLeft = () => (viewport().width - card().width) / 2
  /// How far an owner's pages are drawn moved along at `x`, as far as `x` along Home, and past its last screenful, on
  /// to the App Library, the page after it: in the launcher, whose card shows `ACROSS` pages side by side, as far again
  /// as the card is wide for the one turn there (`libraryReach` times), as an iPhone's App Library is a whole screen
  /// on. `xAt` is where `x` is for pages drawn moved `drawn` along.
  const libraryReach = () => (untrack(launcher) ? ACROSS : 1)
  const trackX = (owner: number, x = xOf(owner)[0]()) => {
    const end = homeEnd(owner)
    return x > end ? end + (x - end) * libraryReach() : x
  }
  const xAt = (owner: number, drawn: number) => {
    const end = homeEnd(owner)
    return drawn > end ? end + (drawn - end) / libraryReach() : drawn
  }
  /// Whether a wheel's turns are moving the pages one to one, as a trackpad's fingers do where the system says nothing
  /// of when they lift, which their spring doesn't follow.
  const [wheeling, setWheeling] = createSignal(false)
  /// Whether the pages are dragged from Home's last screenful, on the way to the App Library maybe: the owner pill over
  /// them, which the host's drag doesn't move, steps out of its way until they're back on Home.
  const [leaving, setLeaving] = createSignal(false)
  /// Where a page of an owner is drawn in the window: its left and top, and how much larger it's drawn.
  const pageAt = (owner: number, page: number) => {
    if (untrack(launcher)) return { x: untrack(cardLeft) + INSET + untrack(() => lead(owner)) + page * PAGE - Math.min(sideways(owner).value, homeEnd(owner)), y: CARD_TOP + PILLS, scale: untrack(room).scale }
    const fit = untrack(room).fit
    const screen = Math.floor(page / fit)
    const count = untrack(screens)[owner]?.[screen]?.count ?? 1
    const left = lefts(untrack(room), untrack(laidPage), count)[page - screen * fit] ?? 0
    return { x: screen * pitch() - Math.min(sideways(owner).value, homeEnd(owner)) + left, y: owner * height() - down.value, scale: untrack(room).scale }
  }
  /// Which page of the owner Home is on a point is over, if it's over one, and where on it, as it's laid out.
  const pageUnder = (x: number, y: number) => {
    const owner = home.getSnapshot().context.at.owner
    const laid = untrack(laidPage)
    const count = untrack(pages)[owner]?.length ?? 0
    for (let page = 0; page < count; page++) {
      const at = pageAt(owner, page)
      const [px, py] = [(x - at.x) / at.scale, (y - at.y) / at.scale]
      if (px >= 0 && py >= 0 && px < laid.width && py < laid.height) return { owner, page, x: px, y: py }
    }
  }
  /// What's on a page of the owner Home is on at a point, if anything.
  const cellUnder = (x: number, y: number) => {
    const under = pageUnder(x, y)
    if (!under) return
    const laid = untrack(laidPage)
    const placed = untrack(pages)[under.owner]?.[under.page]?.find((one) => {
      const box = cellOf(laid, one)
      return under.x >= box.x && under.x < box.x + box.width && under.y >= box.y && under.y < box.y + box.height
    })
    return placed && { page: under.page, index: placed.item.index, placed }
  }

  /// Moves the pages to `at`, as the machine says: gliding, or at once where they're only laid out again.
  const slide = (at: At, smooth: boolean) => {
    // The launcher's owners roll by its host's springs, so they're drawn where they're going; its pages slide as a
    // phone's do.
    const roll = smooth && !untrack(launcher) ? glide : jump
    roll(down, at.owner * height())
    const [moving, to] = [sideways(at.owner), at.screen * pitch()]
    // The pages are where they're going at once, and the host draws them sliding there (`along`); for whoever asked
    // for less motion, they cross-fade.
    if (smooth && untrack(reducedMotion)) glide(moving, to)
    else jump(moving, to)
  }

  /// Whether the pages are dragged along by the host, as a finger, a trackpad's fingers or a mouse goes, where nothing's
  /// carried: the screen's own drag, with no turn of the page's JS a move.
  const dragsAlong = () =>
    !reducedMotion() && (state().matches("nothing") || state().matches({ editing: "still" }) || state().matches({ category: "closing" }))
  /// The pages, along: the host drags them as the finger goes, giving past the first and the App Library as a rubber
  /// band, as an iPhone's do; let go, they go to the page the finger's motion would carry them nearest to (`settle`),
  /// springing there from where they are at the finger's speed.
  const along = {
    get drag() {
      return dragsAlong() ? ("x" as const) : undefined
    },
    get dragConstraints() {
      const owner = state().context.at.owner
      const drawn = trackX(owner)
      return { left: Math.max(0, trackX(owner, (stops(owner) - 1) * pitch()) - drawn), right: drawn }
    },
    // UIScrollView's rubber band.
    dragElastic: 0.55,
    onDragStart: () => {
      const { at } = home.getSnapshot().context
      setLeaving(at.screen === homes(at.owner).length - 1)
    },
    onDragEnd: ({ offset, velocity, canceled }: Dragged) => {
      setLeaving(false)
      if (canceled) return
      const owner = home.getSnapshot().context.at.owner
      const from = Math.round(sideways(owner).value / pitch())
      // Where it was let go, and how fast, as far along as the pages are: slower past Home, to the App Library.
      const at = untrack(() => xAt(owner, trackX(owner, from * pitch()) - offset.x))
      const screen = settle(from, at / pitch(), -velocity.x / (at > homeEnd(owner) ? libraryReach() : 1), stops(owner), pitch())
      quietFor(350)
      jump(sideways(owner), screen * pitch())
      home.send({ type: "settled", at: { owner, screen } })
    },
  }

  // A finger on the pages moves them, as a phone's: once it has moved far enough to say which way, along an owner's
  // screenfuls or between owners, the pages follow it one to one, giving at the ends; let go, they go to the nearest,
  // or on the way they were flicked. A mouse doesn't drag them: a trackpad or a wheel pages (`wheel`).
  let swipe: { id: number; x: number; y: number; axis?: "x" | "y"; owner: number; from: number; base: number; moves: ReturnType<typeof createVelocityTracker> } | undefined
  const carrying = () => home.getSnapshot().matches({ editing: "dragging" })
  /// Whether a second finger is swiping the pages under what the first carries.
  const swiping = () => !!swipe?.axis && swipe.id !== finger.carrier
  /// How many screenfuls a swipe across has: Home's, while something's carried, which stays on them.
  const reach = (owner: number) => (carrying() ? untrack(() => homes(owner).length) || 1 : stops(owner))
  const swipes = {
    onPointerDown: (event: Pressed) => {
      finger.pointer = { x: event.clientX, y: event.clientY }
      if (event.pointerType === "mouse" || event.button) return
      // A category shrinking back into its place lets the pages under it have the finger, and while something's
      // carried, every finger but the one carrying it has them, one at a time.
      const second = carrying() && event.pointerId !== finger.carrier && !swipe
      if (!second && !home.getSnapshot().matches("nothing") && !home.getSnapshot().matches({ category: "closing" }) && !home.getSnapshot().matches({ editing: "still" })) return
      const { owner } = home.getSnapshot().context.at
      stop(down)
      stop(sideways(owner))
      swipe = { id: event.pointerId, x: event.clientX, y: event.clientY, owner, from: 0, base: 0, moves: createVelocityTracker() }
    },
    onPointerMove: (event: Pressed) => follow(event, true),
    onPointerUp: (event: Pressed) => lift(event, false),
    onPointerCancel: (event: Pressed) => lift(event, true),
  }
  /// The pages follow a finger, its moves `tracked` for how fast it goes as it lifts.
  const follow = (event: Pressed, tracked: boolean) => {
    finger.pointer = { x: event.clientX, y: event.clientY }
    if (!swipe || event.pointerId !== swipe.id) return
    if (!swipe.axis) {
      const [dx, dy] = [event.clientX - swipe.x, event.clientY - swipe.y]
      if (Math.hypot(dx, dy) < 10) return
      const axis = Math.abs(dx) > Math.abs(dy) ? "x" : "y"
      // A hold that began has the finger, and up and down is the page's own where it scrolls, and nobody's while
      // editing, but a second finger's while something's carried: an app, which goes to another owner, not a widget.
      const free = home.getSnapshot().matches("nothing") || home.getSnapshot().matches({ category: "closing" }) || home.getSnapshot().matches({ editing: "still" }) || (carrying() && swipe.id !== finger.carrier)
      const vertical = carrying() ? !home.getSnapshot().context.held?.item?.widget : !home.getSnapshot().matches("editing")
      // Along, a phone's pages are the host's to drag (`along`), where it drags them.
      const refused = !free || (axis === "y" && (!vertical || kindOf(swipe.owner) !== "home")) || (axis === "x" && dragsAlong())
      // Either way the pages go on: all of them, or the way the finger doesn't swipe, which it takes at once below.
      slide(home.getSnapshot().context.at, true)
      if (refused) return void (swipe = undefined)
      const moving = axis === "x" ? sideways(swipe.owner) : down
      const size = axis === "x" ? pitch() : height()
      Object.assign(swipe, { axis, x: event.clientX, y: event.clientY, base: moving.value, from: Math.round(moving.value / size) })
    }
    const x = swipe.axis === "x"
    const moved = x ? -(event.clientX - swipe.x) : -(event.clientY - swipe.y)
    const end = x ? (reach(swipe.owner) - 1) * pitch() : (owned().length - 1) * height()
    const size = x ? pitch() : height()
    const value = swipe.base + moved
    if (tracked) swipe.moves.add(event.timeStamp, value)
    jump(x ? sideways(swipe.owner) : down, value < 0 ? give(value, size) : value > end ? end + give(value - end, size) : value)
  }
  const lift = (event: Pressed, cancelled: boolean) => {
    if (!swipe || event.pointerId !== swipe.id) return
    // Where the finger lifted is where it last was: moves come once a frame, so the last may be short of it. Its lift
    // is no move of its own, as Android's VelocityTracker ignores ACTION_UP: it's at the last move's place, later, and
    // taken as one it read a quick swipe as stopping, which went back rather than on (19 Nov, on an iPhone: a lift
    // 35 ms after its last move made 670 points a second read -290).
    if (!cancelled && swipe.axis) follow(event, false)
    const done = swipe
    swipe = undefined
    // A finger that didn't swipe lets go of the pages it caught, and a second finger's tap on an app stacks it on
    // what's carried.
    if (!done.axis) {
      slide(home.getSnapshot().context.at, true)
      if (carrying() && done.id !== finger.carrier && !cancelled) stack(event.clientX, event.clientY)
      return
    }
    quietFor(350)
    // In points a second, as the pages' spring takes it.
    const velocity = cancelled ? 0 : done.moves.velocity(event.timeStamp) * 1000
    const { at } = home.getSnapshot().context
    if (done.axis === "x") {
      const moving = sideways(done.owner)
      const screen = settle(done.from, moving.value / pitch(), velocity, reach(done.owner), pitch())
      glide(moving, screen * pitch(), velocity)
      // What's carried goes to the page swiped to, as it does held at the edge; there the finger places it.
      if (carrying() && screen !== at.screen) home.send({ type: "flip", by: screen > at.screen ? 1 : -1 })
      else home.send({ type: "settled", at: { owner: done.owner, screen } })
    } else {
      const owner = settle(done.from, down.value / height(), velocity, owned().length, height())
      glide(down, owner * height(), velocity)
      const by = owner > at.owner ? 1 : -1
      if (carrying() && owner !== at.owner) home.send({ type: "flip", owner: by, editing: draft(owned()[owner], owner) })
      else home.send({ type: "settled", at: { owner, screen: Math.round(sideways(owner).value / pitch()) } })
    }
  }
  /// What a second finger tapped, while something's carried: an app on the page shown joins it, with a tick.
  const stack = (x: number, y: number) => {
    const under = cellUnder(x, y)
    if (!under || carries(home.getSnapshot().context.held, under.placed.item.item)) return
    const before = home.getSnapshot().context.held?.stack?.length ?? 0
    // The tap opens nothing, nor takes it off with its ⊖.
    quietFor(350)
    home.send({ type: "stack", page: under.page, index: under.index })
    if ((home.getSnapshot().context.held?.stack?.length ?? 0) > before) haptic("selection")
  }

  // A trackpad's two fingers along the pages move them one to one, as a finger does, giving at the ends, and as they
  // lift the pages settle as a finger's do, at the speed they were going, and the momentum that follows is let be.
  // Where the system says when they lift (a Mac's, as the wheel event's `phase`), that's when; elsewhere, as on the
  // DOM, wheel-gestures tells, by where the momentum begins, or the events stop. Up and down, or a wheel, is one owner
  // a gesture at most, however hard it's flicked, as macOS's Launchpad and an iPad with a trackpad page, once its
  // pushes add up to 30 points. Up and down over the App Library, which scrolls itself, is its own.
  const trackpad = WheelGestures({ preventWheelAction: false, reverseSign: false })
  let gesture: { axis?: "x" | "y"; owner: number; from: number; base: number; done: boolean } | undefined
  trackpad.on("wheel", ({ isStart, isEnding, isMomentum, axisMovement: [x, y], axisVelocity, event }) => {
    if (isStart) {
      gesture = undefined
      if (!home.getSnapshot().matches("nothing")) return
      gesture = { owner: home.getSnapshot().context.at.owner, base: 0, from: 0, done: false }
    }
    if (!gesture || gesture.done) return
    // Which way it goes is the first way it moves: fingers set down (`mayBegin`) don't move. Along, they catch the
    // pages where they are, as a finger does.
    if (!gesture.axis) {
      if (!event.deltaX && !event.deltaY) return
      gesture.axis = Math.abs(event.deltaX) > Math.abs(event.deltaY) ? "x" : "y"
      const moving = sideways(gesture.owner)
      if (gesture.axis === "x") stop(moving)
      Object.assign(gesture, { base: trackX(gesture.owner, moving.value) - x + event.deltaX, from: Math.round(moving.value / pitch()) })
    }
    if (gesture.axis === "y") {
      if (Math.abs(y) < 30) return
      gesture.done = true
      return home.send({ type: "step", owner: Math.sign(y) as 1 | -1 })
    }
    const moving = sideways(gesture.owner)
    const end = (stops(gesture.owner) - 1) * pitch()
    const lifted = isMomentum || isEnding || (event as Wheeled).phase === "ended" || (event as Wheeled).phase === "cancelled"
    if (!lifted) {
      // One to one with the fingers, as they're drawn: slower past Home, to the App Library.
      const value = xAt(gesture.owner, gesture.base + x)
      setWheeling(true)
      return jump(moving, value < 0 ? give(value, pitch()) : value > end ? end + give(value - end, pitch()) : value)
    }
    gesture.done = true
    setWheeling(false)
    const velocity = (isEnding ? 0 : axisVelocity[0] * 1000) / (moving.value > homeEnd(gesture.owner) ? libraryReach() : 1)
    const screen = settle(gesture.from, moving.value / pitch(), velocity, stops(gesture.owner), pitch())
    if (untrack(reducedMotion)) glide(moving, screen * pitch(), velocity)
    else jump(moving, screen * pitch())
    home.send({ type: "settled", at: { owner: gesture.owner, screen } })
  })
  const wheel = (event: Wheeled) => trackpad.feedWheel({ deltaX: event.deltaX, deltaY: event.deltaY, deltaMode: 0, timeStamp: event.timeStamp, momentum: event.momentum, phase: event.phase } as WheelEventData)

  onSettled(() => {
    const sliding = home.on("slide", ({ at, smooth }) => slide(at, smooth))
    const arranging = home.on("arrange", ({ edited }) => keep(edited))
    // The arrow keys move as a swipe does. Escape ends editing.
    const keys = keydown((event) => {
      if (event.altKey || event.metaKey || event.ctrlKey) return
      // Escape closes the page overview, and otherwise ends editing, carrying something or not: nothing's open over it.
      if (event.key === "Escape" && home.getSnapshot().matches({ editing: "overview" })) return home.send({ type: "dismiss" })
      if (event.key === "Escape" && (home.getSnapshot().matches({ editing: "still" }) || home.getSnapshot().matches({ editing: "dragging" }))) return home.send({ type: "done" })
      if (!home.getSnapshot().matches("nothing") || untrack(launcher)) return
      const to = ({ ArrowRight: { screen: 1 }, ArrowLeft: { screen: -1 }, ArrowDown: { owner: 1 }, ArrowUp: { owner: -1 } } as Record<string, { screen: 1 | -1 } | { owner: 1 | -1 }>)[event.key]
      if (!to || ("owner" in to && kindOf(home.getSnapshot().context.at.owner) !== "home")) return
      event.preventDefault()
      home.send({ type: "step", ...to })
    })
    return () => (sliding.unsubscribe(), arranging.unsubscribe(), keys())
  })
  // The room it's given may change, and every owner's pages stay on the page they were.
  createEffect(
    () => ({ width: room().width, height: room().height, fit: room().fit }),
    () => {
      const { at, last } = home.getSnapshot().context
      owned().forEach((_, owner) => jump(sideways(owner), (owner === at.owner ? at.screen : (last[owner] ?? 0)) * pitch()))
      jump(down, at.owner * height())
    },
  )
  /// A second finger still swiping lets go of the pages when the one carrying something does: they go back to where what
  /// was carried was put down.
  const release = (id: number) => {
    if (swipe && swipe.id !== id) {
      swipe = undefined
      slide(home.getSnapshot().context.at, true)
    }
  }
  return { height, pitch, downY, xOf, opacityOf, underCarry, sectionsX, card, lead, cardLeft, trackX, wheeling, leaving, pageAt, pageUnder, swiping, swipes, along, wheel, release }
}

export type Paging = ReturnType<typeof createPaging>
