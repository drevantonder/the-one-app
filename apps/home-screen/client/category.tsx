import { createEffect, createMemo, createSignal, For } from "solid-js"
import { covers, keydown, Pressable, type Rect, ScrollView, Text, View, viewport } from "ui"
import { type Entrypoint } from "./types.ts"
import { looks } from "./looks.ts"
import { home, room, tapping, windowWidth } from "./store.ts"
import { type Owner, badgeOf, opens, press } from "./owners.ts"
import { holds } from "./holds.ts"
import { Count, Tile } from "./tiles.tsx"
import { handed, openedCategory, places } from "./library.tsx"

/// How large an opened category's apps' icons are, how wide each is with its name, and the gap between them across,
/// as `sizes.found` and `looks.openedApps` have them.
const [ICON, APP, APART] = [60, 76, 12]

/// A category opened over its App Library, as an iPhone's is: the whole screen (in the launcher, its card), its name as
/// a large title over all its apps in a grid, as Home's, which scrolls, over the App Library blurred and dimmed. Each of
/// its apps flies out of its own icon in its square there and back into it, as an iOS folder's do, wherever the square
/// is, the blur and the dim with them, on one spring the renderer draws, turning back from wherever they've got to:
/// closed, each is as large as that icon and on it, and the square's are handed over to it as it opens and back once
/// they're in the square again, so each app is drawn once. Its name and its apps' come in as they arrive, and go first.
/// Tapping beside its apps, Escape or the device's own back closes it at once, however far it's got: what's still moving
/// is the renderer's to finish. For whoever asked for less motion, it fades. It's made ahead (`made`, as a press goes
/// down on its square) and kept made, closed, in its square and not shown, taking nothing, so the frame it opens or
/// closes on only changes how it's drawn.
export function OpenCategory(props: { owner: Owner; made: string; open: boolean; within?: Rect }) {
  const category = () => openedCategory(props.owner.library, props.made)
  const close = () => home.send({ type: "close" })
  // Open, the device's own back and Escape close it.
  createEffect(
    () => props.open,
    (open) => {
      if (!open) return
      const shut = covers(close)
      const keys = keydown((event) => event.key === "Escape" && close())
      return () => (shut(), keys())
    },
  )
  const area = () => props.within ?? { x: 0, y: 0, width: windowWidth(), height: viewport().height }
  // Where its apps start: under its title, in from the area's side.
  const inset = () => (props.within ? { top: 24, inline: 24, bottom: 24 } : { top: room().safe.top + 108, inline: 24 + room().safe.left, bottom: 32 + room().safe.bottom })
  // Where each of its apps is in it, open, from its top left: measured as it's made, and again as the area's width
  // changes, which moves its apps' columns. Till then nothing in it has a transition, so it goes straight to where that
  // puts it, not on its way there as it opens.
  const [laid, setLaid] = createSignal<{ all?: Rect; apps: Record<number, Rect> }>({ apps: {} })
  const across = createMemo(() => area().width)
  createEffect(across, () => void setLaid({ apps: {} }))
  const measured = createMemo(() => {
    const { all, apps } = laid()
    const count = category()?.entrypoints.length ?? 0
    return all && Object.keys(apps).length === count ? Array.from({ length: count }, (_, index) => ({ ...apps[index], x: apps[index].x - all.x, y: apps[index].y - all.y })) : undefined
  })
  // Each app is drawn on its icon in its square, as it was drawn when it was pressed, while it's closed, as large as
  // that is. A scale is around its middle, and its icon is at its top, in the middle across.
  const square = () => places[props.made] ?? { apps: [] }
  const flight = (index: number) => {
    const at = measured()?.[index]
    const icon = square().apps[index]
    return props.open || !at || !icon ? undefined : { at, icon, scale: Math.max(icon.width / ICON, 0.01) }
  }
  const flown = (index: number) => {
    const flying = flight(index)
    if (!flying) return undefined
    const { at, icon, scale } = flying
    return looks.scaledAt(scale, icon.x + icon.width / 2 - area().x - at.x - at.width / 2, icon.y - area().y - at.y - at.height / 2 + (scale * at.height) / 2)
  }
  // A badge is as large as it is on the icon in the square, all the way, as an iPhone's is: it isn't scaled with its app,
  // but moves with its icon's top right corner, which is `scale` times as far from the app's middle there.
  const flownBadge = (index: number) => {
    const flying = flight(index)
    return flying && looks.scaledAt(1, flying.icon.x + flying.icon.width - area().x - flying.at.x - (flying.at.width + ICON) / 2, flying.icon.y - area().y - flying.at.y)
  }
  const named = () => (props.open ? looks.named : looks.unnamed)
  // In a launcher's card, its name and its apps are a column in the card's middle, as Home's pages are, as wide as its
  // apps take, as a Mac's Launchpad's folder is; a phone's are its whole screen, as an iPhone's.
  const column = () => {
    if (!props.within) return undefined
    const room = area().width - 2 * inset().inline
    const across = Math.max(1, Math.min(category()?.entrypoints.length ?? 1, Math.floor((room + APART) / (APP + APART))))
    return looks.wide(across * APP + (across - 1) * APART)
  }
  // A tap beside its apps closes it; one on an app is the app's (`Pressable`).
  const beside = () => tapping() && close()
  return (
    <View aria-hidden={props.open ? undefined : "true"} style={[looks.dialog, looks.categoryLayer, !props.open && looks.inert, looks.sized(windowWidth(), viewport().height)]}>
      {/* What's behind blurs and dims through a veil. */}
      <Pressable aria-hidden="true" onPress={close} style={[looks.cover, looks.frosted, looks.veils, !props.open && looks.unveiled, props.within && looks.round, props.within && looks.at(area().x, area().y, area().width, area().height)]} />
      <View role="dialog" aria-label={category()?.category} style={[looks.opening, props.within && looks.round, looks.at(area().x, area().y, area().width, area().height)]}>
        <View style={[looks.inOpened, handed(), props.open ? looks.handing : looks.unseen]}>
          <ScrollView style={[looks.openedArea, !props.open && looks.inert, looks.sized(area().width, area().height)]}>
            <Pressable onPress={beside} onLayout={measured() ? undefined : (event) => setLaid((laid) => ({ ...laid, all: event.currentTarget.getBoundingClientRect() }))} style={[looks.opened, props.within && looks.centred, looks.libraryPadding(inset().inline, inset().top, inset().bottom), looks.atLeast(area().height)]}>
              <Text role="heading" aria-level={2} style={[looks.categoryTitle, named(), column()]}>
                {category()?.category}
              </Text>
              <View style={[looks.openedApps, column()]}>
                <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                  {(entrypoint, index) => {
                    // What's held here is an app of the App Library, as there: its menu, or carried onto Home's pages.
                    const held = holds(() => ({ item: opens(entrypoint()), listed: entrypoint(), page: 0, index: -1, from: "library" }), () => true)
                    return (
                      <View style={looks.slots} onLayout={measured() ? undefined : (event) => setLaid((laid) => ({ ...laid, apps: { ...laid.apps, [index()]: event.currentTarget.getBoundingClientRect() } }))}>
                        <Tile name={entrypoint().name} icon={entrypoint().icon} size={ICON} scale={1} style={[measured() && looks.flies, flown(index())]} nameStyle={named()} badge={badgeOf(entrypoint() as Entrypoint, props.owner)} counted={false} holds={held} onPress={(behind, from) => press(entrypoint(), props.owner, behind, from)} />
                        <View aria-hidden="true" style={looks.badged}>
                          <View style={[looks.slots, looks.square(ICON), measured() && looks.flies, flownBadge(index())]}>
                            <Count count={badgeOf(entrypoint() as Entrypoint, props.owner)} />
                          </View>
                        </View>
                      </View>
                    )
                  }}
                </For>
              </View>
            </Pressable>
          </ScrollView>
        </View>
      </View>
    </View>
  )
}
