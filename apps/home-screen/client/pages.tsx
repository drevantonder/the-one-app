// A Home page and what's on it: each app or widget where it's placed, jiggling while its page is edited, gliding to
// where it now goes.
import { createEffect, createSignal, For, Show, untrack } from "solid-js"
import { confirm, Empty, reducedMotion, Spinner, View } from "ui"
import { glide, spring } from "./pager.ts"
import { type Box as Place, cell as cellOf, type Page as Laid } from "./layout.ts"
import { carries, type Held, type Placed } from "./machine.ts"
import { type Entrypoint, type Item, type Widget as WidgetItem } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, editingNow, editingOwner, launcher, selected, setSelected, landing } from "./store.ts"
import { type Owner, badgeOf, launch, removal } from "./owners.ts"
import { Card, Tile, Minus } from "./tiles.tsx"
import { holds } from "./holds.ts"

/// What a Home page holds as it's drawn: an item of the owner's, and where it is among the page's.
export type Cell = { item: Item; index: number }

/// The jiggle, as an iPhone's: everything on a page being edited swings from side to side, as far as `shake` says,
/// which grows from nothing as editing begins and dies away as it ends, the swing held where it was. `wobble` is the
/// time it swings by, a frame at a time while it moves; nothing moves for whoever asked for less motion.
const [wobble, setWobble] = createSignal(0)

const [shake, setShake] = createSignal(0)

let shaking = 0
createEffect(
  () => editingNow() && !reducedMotion(),
  (on) => {
    const from = untrack(shake)
    let begun = -1
    cancelAnimationFrame(shaking)
    const frame = (time: number) => {
      if (begun < 0) begun = time
      const done = Math.min(1, (time - begun) / 300)
      setShake(from + ((on ? 1 : 0) - from) * (1 - (1 - done) ** 3))
      if (on) setWobble(time)
      shaking = on || done < 1 ? requestAnimationFrame(frame) : 0
    }
    shaking = requestAnimationFrame(frame)
  },
)

/// How far an item swings, in degrees, now: each its own way through the swing, by where it is.
const swing = (index: number, widget: boolean) => {
  const period = widget ? 340 : 260
  return Math.sin(((wobble() + ((index * 97) % 240)) / period) * Math.PI) * shake() * 1.4
}

/// A Home page: its grid, each item where it was placed, laid out as `laid` says, drawn `scale` times larger. While its
/// owner's pages are edited, everything jiggles with its ⊖, and what's lifted leaves its place empty. An owner with
/// nothing on their pages has one grey line. An app dropped where it isn't installed waits there, dimmed, while it's
/// asked about, and shows its progress as it installs.
export function HomePage(props: { owner: Owner; index: number; page: number; placed: Placed<Cell>[]; laid: Laid; scale: number; small?: boolean }) {
  const editingHere = () => !props.small && editingOwner(props.index)
  // What's carried leaves its place empty, where it'll land, wherever that is as the rest make room.
  const lifted = (cell: Cell) => editingHere() && state().matches({ editing: "dragging" }) && carries(state().context.held, cell.item)
  // Dropped where it isn't installed, what was stacked on it waits with it.
  const waiting = (cell: Cell) => editingHere() && carries(state().context.held, cell.item) && (state().matches({ editing: "asking" }) || state().matches({ editing: "installing" }) || state().matches({ editing: "refused" }))
  const arranges = () => props.owner.arranges && !props.small
  const removing = async (item: Item, index: number) => {
    if (await confirm(removal(item))) home.send({ type: "remove", page: props.page, index })
  }
  // In the launcher, what the arrow keys chose, or the pointer's on.
  const picked = (index: number) => {
    const now = selected()
    return launcher() && !props.small && now?.owner === props.index && now.page === props.page && now.index === index
  }
  const point = (index: number) => launcher() && !props.small && state().matches("nothing") && setSelected({ owner: props.index, page: props.page, index })
  // Made once: what a press began, its later moves read.
  const gestures = holds(() => (editingHere() ? undefined : { page: props.page, index: -1, from: "page" }), arranges)
  return (
    <View
      role="group"
      aria-label={`Page ${props.page + 1}`}
      {...gestures}
      style={[looks.slots, looks.sized(props.laid.width, props.laid.height)]}
    >
      <Show when={props.placed.length || editingHere()} fallback={<View style={[looks.empty, looks.top(props.laid.top)]}><Empty>No apps on this page.</Empty></View>}>
        <For each={props.placed} keyed={({ item }) => (item.item.widget ?? item.item.entrypoint) + (item.item.size ?? "")}>
          {(placed) => (
            <HomeCell
              owner={props.owner}
              index={props.index}
              page={props.page}
              placed={placed()}
              laid={props.laid}
              scale={props.scale}
              small={props.small}
              jiggles={editingHere()}
              lifted={lifted(placed().item)}
              waiting={waiting(placed().item)}
              picked={picked(placed().item.index)}
              held={() => ({ item: placed().item.item, page: props.page, index: placed().item.index, from: "page" })}
              arranges={arranges}
              onPoint={() => point(placed().item.index)}
              onRemove={arranges() && !waiting(placed().item) ? () => removing(placed().item.item, placed().item.index) : undefined}
            />
          )}
        </For>
      </Show>
    </View>
  )
}

/// Something on a Home page, where it's placed. While its page is edited it glides to where it now goes, as an
/// iPhone's do when an app is dragged among them or taken off, rather than jumping there: it's drawn where it was, then
/// springs to its new place (pager.ts's spring, which keeps its speed if it moves again on the way). As editing
/// begins, nothing moves.
function HomeCell(props: {
  owner: Owner
  index: number
  page: number
  placed: Placed<Cell>
  laid: Laid
  scale: number
  small?: boolean
  jiggles: boolean
  lifted: boolean
  waiting: boolean
  picked: boolean
  held: () => Held
  arranges: () => boolean
  onPoint: () => void
  onRemove?: () => void
}) {
  const box = () => cellOf(props.laid, props.placed)
  // Made once: what a press began, its later moves read.
  const gestures = holds(() => props.held(), () => props.arranges())
  const [dx, setDx] = createSignal(0)
  const [dy, setDy] = createSignal(0)
  const x = spring(setDx)
  const y = spring(setDy)
  let was: Place | undefined
  let wasEditing = false
  createEffect(
    () => ({ ...box(), editing: editingNow() }),
    (now) => {
      if (was && now.editing && wasEditing && !props.small && (was.x !== now.x || was.y !== now.y)) {
        x.value += was.x - now.x
        y.value += was.y - now.y
        glide(x, 0)
        glide(y, 0)
      }
      was = now
      wasEditing = now.editing
    },
  )
  const item = () => props.placed.item.item
  const widget = () => !!item().widget
  const landingHere = () => {
    const now = landing()
    return now?.owner === props.index && now.page === props.page && now.index === props.placed.item.index
  }
  return (
    <View
      {...gestures}
      onPointerMove={(event) => (props.onPoint(), gestures.onPointerMove?.(event))}
      style={looks.placed(box().x, box().y, box().width, box().height, dx(), dy(), props.jiggles ? swing(props.placed.item.index, widget()) : 0, props.lifted || landingHere() ? 0 : 1)}
    >
      <Show when={props.picked}>
        {/* What's chosen has the highlight behind it: an app's, and a widget's own tint brightened by it. */}
        <View aria-hidden="true" style={looks.chosen} />
      </Show>
      <Show
        when={widget()}
        fallback={
          <View style={[looks.slots, looks.opacity(props.waiting ? 0.45 : 1)]}>
            <Tile
              name={(item() as Entrypoint).name}
              icon={(item() as Entrypoint).icon}
              size={props.laid.tile}
              scale={props.scale}
              badge={badgeOf(item() as Entrypoint, props.owner)}
              onPress={(behind, from) => state().matches("nothing") && launch(item() as Entrypoint, props.owner, behind, from)}
            />
          </View>
        }
      >
        <Card item={item() as WidgetItem} owner={props.owner} still={props.small || props.jiggles} />
      </Show>
      <Show when={props.waiting && state().matches({ editing: "installing" })}>
        <View role="status" aria-label={`Installing ${item().name}`} style={[looks.installing, looks.at(box().width / 2 - 16, props.laid.tile / 2 - 16, 32, 32)]}>
          <Spinner />
        </View>
      </Show>
      <Show when={props.jiggles && props.onRemove}>
        <Minus label={`Remove ${item().name}`} left={widget() ? -6 : Math.max(0, (box().width - props.laid.tile) / 2 - 8)} top={-6} onPress={() => props.onRemove?.()} />
      </Show>
    </View>
  )
}
