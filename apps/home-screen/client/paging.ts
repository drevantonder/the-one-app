import { createMemo, createEffect, createSignal, onSettled, untrack } from "solid-js"
import { createVelocityTracker, type Dragged, haptic, keydown, type Pressed, reducedMotion, viewport, type Wheeled } from "ui"
import { give, glide, jump, settle, spring, type Spring, stop } from "./pager.ts"
import { along as alongOf, card as cardOf, cell as cellOf, INSET, lefts, PILLS, CARD_TOP } from "./layout.ts"
import { type At, carries } from "./machine.ts"
import { home, setSeen, setAlong, setRolled, launcher, room, laidPage, quietFor, finger, state, at, libraryScroll } from "./store.ts"
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
  /// How far the host holds the pages dragged along from where they are, as they're dragged (`along`), and none while
  /// they aren't: where they're drawn is where they are less it. `held` is it as it's said, which `look` reads in the
  /// same turn, where the signal, which draws, says it once that turn is done.
  const [dragged, setDragged] = createSignal<number>()
  let held = 0
  const drag = (offset: number | undefined) => {
    held = offset ?? 0
    setDragged(offset)
  }
  /// The owner and screenful nearest where they're drawn, which the dots light.
  const look = () => {
    const owner = shownOwner()
    const drawn = sideways(owner).value - held
    // Past an end, as a rubber band, it's the end's.
    setSeen({ owner, screen: Math.max(0, Math.min(stops(owner) - 1, Math.round(drawn / pitch()))) })
    setAlong(drawn / pitch())
  }
  /// How far the sections are drawn past Home, to the App Library: the owner shown's way along, past their Home, as the
  /// host drags them too. What's over Home and not its pages, the owner pill and the dots, goes as far with it, as an
  /// iPhone's dots go with its last page.
  const sectionsX = () => {
    const owner = Math.max(0, Math.min(props.owners.length - 1, Math.round(downY() / Math.max(1, room().height))))
    return Math.max(0, xOf(owner)[0]() - (dragged() ?? 0) - Math.max(0, homes(owner).length - 1) * room().width)
  }

  const card = () => cardOf(room())
  const cardLeft = () => (viewport().width - card().width) / 2
  /// Where a page of an owner is drawn in the window: its left and top, and how much larger it's drawn.
  const pageAt = (owner: number, page: number) => {
    if (untrack(launcher)) return { x: untrack(cardLeft) + INSET + alongOf(untrack(room), page) - Math.min(sideways(owner).value, homeEnd(owner)), y: CARD_TOP + PILLS, scale: untrack(room).scale }
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
  const dragsAlong = createMemo(() => !reducedMotion() && (state().matches("nothing") || state().matches({ editing: "still" })))
  /// The pages, along: the host drags them as the finger goes, giving past the first and the App Library as a rubber
  /// band, as an iPhone's do; let go, they go to the page the finger's motion would carry them nearest to (`settle`),
  /// springing there from where they are at the finger's speed.
  const along = {
    get drag() {
      return dragsAlong() ? ("x" as const) : undefined
    },
    get dragConstraints() {
      const owner = at().owner
      const x = xOf(owner)[0]()
      return { left: Math.max(0, (stops(owner) - 1) * pitch() - x), right: x }
    },
    // UIScrollView's rubber band.
    dragElastic: 0.55,
    onDragStart: () => drag(0),
    onDrag: ({ offset }: Pick<Dragged, "offset">) => {
      drag(offset.x)
      look()
    },
    onDragEnd: ({ offset, velocity, canceled }: Dragged) => {
      drag(undefined)
      if (canceled) return look()
      const owner = home.getSnapshot().context.at.owner
      const from = Math.round(sideways(owner).value / pitch())
      const screen = settle(from, from - offset.x / pitch(), -velocity.x, stops(owner), pitch())
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
      // While something's carried, every finger but the one carrying it has them, one at a time.
      const second = carrying() && event.pointerId !== finger.carrier && !swipe
      if (!second && !home.getSnapshot().matches("nothing") && !home.getSnapshot().matches({ editing: "still" })) return
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
      const free = home.getSnapshot().matches("nothing") || home.getSnapshot().matches({ editing: "still" }) || (carrying() && swipe.id !== finger.carrier)
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

  // A mouse's wheel, which turns by notches, turns the pages a screenful a notch, each by the pages' spring, as an
  // iPad's Home Screen does with a mouse: down or right on, past the last on to the App Library, and up or left back,
  // out of the App Library once it's scrolled to its top, which it otherwise scrolls. It never moves between owners:
  // the owner pill does.
  const notched = (notches: number, down: boolean) => {
    const { at, screens } = home.getSnapshot().context
    if (!home.getSnapshot().matches("nothing")) return
    if (screens[at.owner]?.[at.screen] === "library") {
      if (notches < 0 && (!down || libraryScroll.scrollTop <= 0)) home.send({ type: "step", screen: -1 })
      return
    }
    for (let turned = 0; turned < Math.abs(notches); turned++) home.send({ type: "step", screen: Math.sign(notches) as 1 | -1 })
  }
  // A trackpad's fingers where the host doesn't drag the pages along with them (up and down, and wherever it says
  // nothing of them), and a wheel that turns smoothly, page as macOS's Launchpad and an iPad with a trackpad do: one
  // screenful along or one owner up and down a gesture, however hard it's flicked, once its pushes add up to 30 points
  // one way. A gesture starts as the fingers touch (`began`) or the wheel turns after a quiet moment, as a wheel has no
  // phases, and what the system scrolls on with once the fingers have lifted (`momentum`) is the gesture's, which
  // moves nothing, as a Mac's paging scroll view and Safari's swipe between pages let it be. Up and down over the App
  // Library, which scrolls itself, is its own.
  let gesture: { at: number; x: number; y: number; done: boolean } | undefined
  const wheel = (event: Wheeled) => {
    if (event.notchesY || event.notchesX) return notched(event.notchesY || event.notchesX!, !!event.notchesY)
    if (event.momentum) return
    const quiet = !gesture || event.timeStamp - gesture.at > 250
    if (event.phase === "began" || (quiet && event.phase !== "ended" && event.phase !== "cancelled")) gesture = { at: event.timeStamp, x: 0, y: 0, done: false }
    if (!gesture) return
    Object.assign(gesture, { at: event.timeStamp, x: gesture.x + event.deltaX, y: gesture.y + event.deltaY })
    if (gesture.done || !home.getSnapshot().matches("nothing")) return
    const [x, y] = [Math.abs(gesture.x), Math.abs(gesture.y)]
    if (Math.max(x, y) < 30) return
    gesture.done = true
    if (x > y) home.send({ type: "step", screen: Math.sign(gesture.x) as 1 | -1 })
    else home.send({ type: "step", owner: Math.sign(gesture.y) as 1 | -1 })
  }

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
  return { height, pitch, downY, xOf, opacityOf, underCarry, sectionsX, dragged, card, cardLeft, pageAt, pageUnder, swiping, swipes, along, wheel, release }
}

export type Paging = ReturnType<typeof createPaging>
