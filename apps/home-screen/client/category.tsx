import { createEffect, For } from "solid-js"
import { covers, keydown, Pressable, type Rect, ScrollView, Text, View, viewport } from "ui"
import { type Entrypoint } from "./types.ts"
import { looks } from "./looks.ts"
import { home, room, tapping, windowWidth } from "./store.ts"
import { type Owner, badgeOf, opens, press } from "./owners.ts"
import { holds } from "./holds.ts"
import { Tile } from "./tiles.tsx"
import { openedCategory, places } from "./library.tsx"

/** How small it is in its square, as `looks.grows` draws it there. */
const SHRUNK = 0.3

/// A category opened over its App Library, as an iPhone's is: the whole screen (in the launcher, its card), its name as
/// a large title over all its apps in a grid, as Home's, which scrolls, over the App Library blurred and dimmed. It
/// grows out of its square there and shrinks back into it, the blur and the dim with it, on one spring the renderer
/// draws, turning back from wherever it's got to. Tapping beside its apps, Escape or the device's own back closes it at
/// once, however far it's grown: what's still moving is the renderer's to finish. For whoever asked for less motion,
/// it fades. It's made ahead (`made`, as a press goes down on its square) and kept made, closed, shrunk into its square
/// and not shown, taking nothing, so the frame it opens or closes on only changes how it's drawn.
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
  // It grows out of its square, as it was drawn when it was tapped, from where its title and first apps sit there, to
  // where they sit open: what grows (from `SHRUNK`, as `looks.grows` has it) does so around the one point that puts
  // them there, as large as it takes to hold the whole area around it.
  const grows = () => {
    const from = places[props.made] ?? { x: area().x + area().width / 2, y: area().y + area().height / 2 }
    const [x, y] = [(from.x - SHRUNK * (area().x + inset().inline)) / (1 - SHRUNK), (from.y - SHRUNK * (area().y + inset().top)) / (1 - SHRUNK)]
    const [across, down] = [Math.max(x - area().x, area().x + area().width - x), Math.max(y - area().y, area().y + area().height - y)]
    return { x: x - across, y: y - down, width: 2 * across, height: 2 * down }
  }
  // A tap on an app opens it, and one beside them, which the grid's Pressable has too, closes it.
  let app = false
  const beside = () => (app ? (app = false) : tapping() && close())
  return (
    <View role="dialog" aria-label={category()?.category} aria-hidden={props.open ? undefined : "true"} style={[looks.dialog, looks.categoryLayer, !props.open && looks.inert, looks.sized(windowWidth(), viewport().height)]}>
      {/* What's behind blurs and dims through a veil. */}
      <Pressable aria-hidden="true" onPress={close} style={[looks.cover, looks.frosted, looks.veils, !props.open && looks.unveiled, props.within && looks.round, props.within && looks.at(area().x, area().y, area().width, area().height)]} />
      <View style={[looks.grows, !props.open && looks.shrunk, looks.at(grows().x, grows().y, grows().width, grows().height)]}>
        <ScrollView style={[looks.openedArea, !props.open && looks.inert, props.within && looks.round, looks.at(area().x - grows().x, area().y - grows().y, area().width, area().height)]}>
          <Pressable onPress={beside} style={[looks.opened, looks.libraryPadding(inset().inline, inset().top, inset().bottom), looks.atLeast(area().height)]}>
            <Text role="heading" aria-level={2} style={looks.categoryTitle}>
              {category()?.category}
            </Text>
            <View style={looks.openedApps}>
              <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                {(entrypoint) => {
                  // What's held here is an app of the App Library, as there: its menu, or carried onto Home's pages.
                  const held = holds(() => ({ item: opens(entrypoint()), listed: entrypoint(), page: 0, index: -1, from: "library" }), () => true)
                  const pressed = (behind: boolean, from?: Parameters<typeof press>[3]) => ((app = !behind), press(entrypoint(), props.owner, behind, from))
                  return <Tile name={entrypoint().name} icon={entrypoint().icon} size={60} scale={1} badge={badgeOf(entrypoint() as Entrypoint, props.owner)} holds={held} onPress={pressed} />
                }}
              </For>
            </View>
          </Pressable>
        </ScrollView>
      </View>
    </View>
  )
}
