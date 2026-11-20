import { createEffect, For } from "solid-js"
import { covers, keydown, Pressable, type Rect, ScrollView, Text, View, viewport } from "ui"
import { type Entrypoint } from "./types.ts"
import { looks } from "./looks.ts"
import { home, room, tapping, windowWidth } from "./store.ts"
import { type Owner, badgeOf, opens, press } from "./owners.ts"
import { holds } from "./holds.ts"
import { Tile } from "./tiles.tsx"
import { openedCategory, places } from "./library.tsx"

/// A category opened over its App Library, as an iPhone's is: the whole screen (in the launcher, its card), its name as
/// a large title over all its apps in a grid, as Home's, which scrolls, over the App Library blurred and dimmed. It
/// grows out of its square there and shrinks back into it, the blur and the dim with it, on one spring the renderer
/// draws, turning back from wherever it's got to: closed, it's its square, exactly, with what's in it drawn as small as
/// the square is beside the area, from its top left, so it grows from where the square is drawn and shrinks back into
/// it, as an iOS folder does. Tapping beside its apps, Escape or the device's own back closes it at
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
  // Its square, as it was drawn when it was pressed; open, the whole area. What's in it is the area's size, drawn as
  // small as the square is beside the area while it's closed, from its top left (a scale is around the middle, so it
  // moves by as much as that takes away at each side).
  const square = () => places[props.made] ?? { x: area().x + area().width / 2, y: area().y + area().height / 2, width: 0, height: 0 }
  const box = () => (props.open ? area() : square())
  const small = () => (props.open ? 1 : square().width / area().width)
  const shift = (length: number) => (-(1 - small()) * length) / 2
  // A tap beside its apps closes it; one on an app is the app's (`Pressable`).
  const beside = () => tapping() && close()
  return (
    <View aria-hidden={props.open ? undefined : "true"} style={[looks.dialog, looks.categoryLayer, !props.open && looks.inert, looks.sized(windowWidth(), viewport().height)]}>
      {/* What's behind blurs and dims through a veil. */}
      <Pressable aria-hidden="true" onPress={close} style={[looks.cover, looks.frosted, looks.veils, !props.open && looks.unveiled, props.within && looks.round, props.within && looks.at(area().x, area().y, area().width, area().height)]} />
      <View role="dialog" aria-label={category()?.category} style={[looks.grows, !props.open && looks.shrunk, props.open && props.within && looks.round, looks.at(area().x, area().y, box().width, box().height), looks.moved(box().x - area().x, box().y - area().y)]}>
        <ScrollView style={[looks.openedArea, looks.contents, !props.open && looks.inert, looks.sized(area().width, area().height), looks.scaledAt(small(), shift(area().width), shift(area().height))]}>
          <Pressable onPress={beside} style={[looks.opened, looks.libraryPadding(inset().inline, inset().top, inset().bottom), looks.atLeast(area().height)]}>
            <Text role="heading" aria-level={2} style={looks.categoryTitle}>
              {category()?.category}
            </Text>
            <View style={looks.openedApps}>
              <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                {(entrypoint) => {
                  // What's held here is an app of the App Library, as there: its menu, or carried onto Home's pages.
                  const held = holds(() => ({ item: opens(entrypoint()), listed: entrypoint(), page: 0, index: -1, from: "library" }), () => true)
                  return <Tile name={entrypoint().name} icon={entrypoint().icon} size={60} scale={1} badge={badgeOf(entrypoint() as Entrypoint, props.owner)} holds={held} onPress={(behind, from) => press(entrypoint(), props.owner, behind, from)} />
                }}
              </For>
            </View>
          </Pressable>
        </ScrollView>
      </View>
    </View>
  )
}
