import { createEffect, createMemo, createSignal, Show, untrack } from "solid-js"
import { haptic, type Pressed, type Rect, reducedMotion, View } from "ui"
import { glide, spring, thrower } from "./pager.ts"
import { at as gridAt, cell as cellOf, PILLS, CARD_TOP } from "./layout.ts"
import { carries, type Drop, type Held, nearest, shown } from "./machine.ts"
import { type Entrypoint, type Widget as WidgetItem } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, launcher, room, laidPage, quietFor, finger, setLanding } from "./store.ts"
import { type Owner, owners, draft, installedFor } from "./owners.ts"
import { Card, Count, Tile } from "./tiles.tsx"
import { tween } from "./tween.ts"
import { type Screens } from "./screens.ts"
import { type Paging } from "./paging.ts"

// What's carried makes room where it rests this long, as an iPhone's does, not at every place it passes.
const REST = 150

export function createCarry(props: { owners: Owner[] }, layout: Screens, paging: Paging) {
  const { pages } = layout
  const { height, pitch, underCarry, card, cardLeft, pageAt, pageUnder, swiping, release } = paging
  // What's carried, while it's dragged: a copy that lifts larger and follows the finger one to one, over what it would
  // go before. Held at a side of the screen a moment, it goes on to the next page, or the one before; at its top or
  // bottom, to the previous owner's pages, or the next one's. Let go, it springs into its place from where it was, at
  // the speed it was moving, and where that's on an owner it isn't installed for, or whose pages aren't yours to
  // change, what follows is asked or said.
  const [copy, setCopy] = createSignal<Copy>()
  createEffect(
    createMemo(() => state().matches({ editing: "dragging" })),
    (dragging) => {
      if (!dragging) return
      const held = home.getSnapshot().context.held!
      const rect = held.rect ?? { x: finger.pointer.x - 30, y: finger.pointer.y - 40, width: 60, height: 80 }
      setCopy({ held, rect, x: finger.pointer.x - finger.grab.x, y: finger.pointer.y - finger.grab.y, settling: false, lifted: 0 })
      const lifting = tween(200, () => {})
      lifting.to(1)
      createEffect(lifting.value, (lifted) => void setCopy((now) => now && { ...now, lifted }))
      haptic("medium")
      const id = finger.carrier
      const moves = thrower()
      let edge: ReturnType<typeof setTimeout> | undefined
      let side: "" | "left" | "right" | "up" | "down" = ""
      // A page, or an owner, turns after a moment at its edge, and again while it stays there, as an iPhone's does,
      // unless a second finger is turning them.
      const turn = () => {
        const { at } = home.getSnapshot().context
        if (swiping()) {
          // It waits for the second finger.
        } else if (side === "left" || side === "right") home.send({ type: "flip", by: side === "left" ? -1 : 1 })
        else {
          const [by, all] = [side === "up" ? -1 : 1, untrack(owners)!] as const
          if (all[at.owner + by]) home.send({ type: "flip", owner: by, editing: draft(all[at.owner + by], at.owner + by) })
        }
        const now = home.getSnapshot().context.at
        if (now.screen !== at.screen || now.owner !== at.owner) haptic("selection")
        edge = setTimeout(turn, 900)
      }
      const move = (event: Pressed) => {
        if (event.pointerId !== id) return
        finger.pointer = { x: event.clientX, y: event.clientY }
        setCopy((now) => now && { ...now, x: event.clientX - finger.grab.x, y: event.clientY - finger.grab.y })
        moves.add(event)
        const [x, y] = [event.clientX, event.clientY]
        // The pages' edges: the window's, or the launcher's card's, inside its pills.
        const area = untrack(launcher) ? { left: cardLeft(), right: cardLeft() + card().width, top: CARD_TOP + PILLS, bottom: CARD_TOP + card().height - PILLS } : { left: 0, right: pitch(), top: 0, bottom: height() }
        const now = x < area.left + 28 ? "left" : x > area.right - 28 ? "right" : y < area.top + 36 ? "up" : y > area.bottom - 36 ? "down" : ""
        if (now !== side) {
          clearTimeout(edge)
          side = now
          if (side) edge = setTimeout(turn, 600)
        }
        if (side) clearTimeout(resting)
        else over(x, y)
      }
      // The place it's over, once it has rested there a moment, as an iPhone's does, rather than at every point it
      // passes: the place where the page's own packing would put it nearest where it's drawn, so the rest keep their
      // order and it lands only where it fits. While a second finger swipes the pages under it, it waits for them.
      let resting: ReturnType<typeof setTimeout> | undefined
      let wanted = ""
      const over = (x: number, y: number) => {
        if (swiping()) return
        const { editing, held } = home.getSnapshot().context
        // Its top left, which the page's cells are counted from.
        const under = pageUnder(x - finger.grab.x + rect.x, y - finger.grab.y + rect.y) ?? pageUnder(x, y)
        if (!under || !editing || !held?.item) return
        const scale = untrack(room).scale
        const corner = pageAt(under.owner, under.page)
        const [left, top] = [(x - finger.grab.x + rect.x - corner.x) / scale, (y - finger.grab.y + rect.y - corner.y) / scale]
        const cells = gridAt(untrack(laidPage), left, top)
        const items = editing.pages[shown(editing.pages)[under.page]]?.items.filter((one) => !carries(held, one)) ?? []
        const index = nearest(items, held.item, cells.column, cells.row)
        const place = under.page === held.page && index === held.index ? "" : `${under.page} ${index}`
        if (place === wanted) return
        wanted = place
        clearTimeout(resting)
        if (!place) return
        resting = setTimeout(() => {
          wanted = ""
          home.send({ type: "over", page: under.page, index })
        }, REST)
      }
      // Where it was last, so it can settle there once it's let go.
      let left = held
      // How fast the finger was going as it let go, which the copy goes on at.
      let thrown = { x: 0, y: 0 }
      // Once the pages a second finger turned come to rest, what's carried goes where the first is.
      underCarry.settled = () => !side && over(finger.pointer.x, finger.pointer.y)
      const drop = (event: Pressed) => {
        if (event.pointerId !== id) return
        left = home.getSnapshot().context.held ?? left
        if (event.timeStamp) thrown = moves.thrown(event.timeStamp)
        const { held, at } = home.getSnapshot().context
        const owner = untrack(owners)![at.owner]
        const items = held?.item ? [held.item, ...(held.stack ?? [])] : []
        const to: Drop = !owner.arranges ? "refuse" : items.every((item) => item.widget || installedFor(item as Entrypoint, owner)) ? "place" : "ask"
        home.send({ type: "drop", to })
      }
      finger.carry = { move, drop }
      return () => {
        finger.carry = undefined
        clearTimeout(edge)
        clearTimeout(resting)
        underCarry.settled = undefined
        release(id)
        quietFor(400)
        haptic("light")
        // What was carried settles into its place as it's let go, rather than vanishing over it: a spring from where
        // the finger left it, at the speed it was going, to its place, wherever that is as it goes.
        const owner = home.getSnapshot().context.at.owner
        const placed = untrack(pages)[owner]?.[left.page]?.find((one) => one.item.index === left.index)
        if (!placed || untrack(reducedMotion)) return void setCopy()
        setLanding({ owner, page: left.page, index: left.index })
        const target = () => {
          const at = pageAt(owner, left.page)
          const box = cellOf(untrack(laidPage), placed)
          return { x: at.x + (box.x + box.width / 2) * at.scale - (rect.x + rect.width / 2), y: at.y + (box.y + box.height / 2) * at.scale - (rect.y + rect.height / 2) }
        }
        const draw = () => {
          // Its place may still be on its way, as the pages turn: the copy goes after it.
          const to = target()
          if (Math.abs(x.target - to.x) > 0.5) glide(x, to.x)
          if (Math.abs(y.target - to.y) > 0.5) glide(y, to.y)
          setCopy((now) => now && { ...now, x: x.value, y: y.value, settling: true })
          if (x.value === x.target && y.value === y.target && !x.velocity && !y.velocity) (setCopy(), setLanding())
        }
        const [x, y] = [spring(draw), spring(draw)]
        const from = untrack(copy)!
        const to = target()
        Object.assign(x, { value: from.x, target: to.x })
        Object.assign(y, { value: from.y, target: to.y })
        glide(x, to.x, thrown.x)
        glide(y, to.y, thrown.y)
      }
    },
  )
  return { copy }
}

/// What a finger carries as it's drawn: what's held, where it was lifted from, where it is now, and how far it has lifted.
export type Copy = { held: Held; rect: Rect; x: number; y: number; settling: boolean; lifted: number }

/// What a finger carries, drawn from where it was lifted.
export function Carried(props: { now: () => Copy; owner: Owner }) {
  const now = props.now
  const owner = () => props.owner
  const scale = () => untrack(room).scale
  const widget = () => !!now().held.item?.widget
  const [width, height] = [now().rect.width / scale(), now().rect.height / scale()]
  const lift = () => (now().settling ? 1 : 1 + (widget() ? 0.05 : 0.12) * now().lifted)
  const tile = () => laidPage().tile
  return (
    <View
      aria-hidden="true"
      style={[looks.carry, looks.carryAt(now().rect.x + (now().rect.width - width) / 2, now().rect.y + (now().rect.height - height) / 2, width, height, now().x, now().y, scale() * lift())]}
    >
      <View aria-hidden="true" style={[looks.lifted, widget() ? looks.liftedWidget : looks.liftedIcon, now().settling && looks.opacity(0), widget() ? looks.at(0, 0, width, height) : looks.at((width - tile()) / 2, 0, tile(), tile())]} />
      <Show when={now().held.item}>
        {(item) => (
          <Show when={item().widget} fallback={<Tile name={(item() as Entrypoint).name} icon={(item() as Entrypoint).icon} size={laidPage().tile} scale={scale()} />}>
            <Card item={item() as WidgetItem} owner={owner()} still />
          </Show>
        )}
      </Show>
      <Show when={now().held.stack?.length}>
        <Count count={(now().held.stack?.length ?? 0) + 1} />
      </Show>
    </View>
  )
}
