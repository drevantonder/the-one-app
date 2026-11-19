import { createEffect, createMemo, For, onSettled } from "solid-js"
import { covers, keydown, Pressable, type Rect, reducedMotion, Text, View, viewport } from "ui"
import { type Entrypoint } from "./types.ts"
import { looks } from "./looks.ts"
import { home, state, windowWidth } from "./store.ts"
import { type Owner, badgeOf, opens, press } from "./owners.ts"
import { holds } from "./holds.ts"
import { Tile } from "./tiles.tsx"
import { tween } from "./tween.ts"
import { openedCategory, places } from "./library.tsx"

/// A category opened over its App Library: its name, and its apps in a panel that grows out of its square there, its
/// shape becoming the panel's, and shrinks back into it, as an iPhone's folder does, while what's behind it is dimmed. Its apps open only once it has
/// grown. Tapping beside the panel, Escape or the device's own back closes it, even as it grows: it turns back from
/// where it got to. For whoever asked for less motion, it fades in and out where it is.
export function OpenCategory(props: { owner: Owner; opened: string; within?: Rect }) {
  const category = () => openedCategory(props.owner.library, props.opened)
  const phase = () => (state().matches({ category: "opening" }) ? "opening" : state().matches({ category: "open" }) ? "open" : "closing")
  const close = () => !state().matches({ category: "closing" }) && home.send({ type: "close" })
  const zoom = tween(300, () => home.send({ type: "zoomed" }))
  createEffect(createMemo(phase), (now) => {
    if (now !== "open") zoom.to(now === "opening" ? 1 : 0)
  })
  onSettled(() => {
    const shut = covers(close)
    const keys = keydown((event) => event.key === "Escape" && close())
    return () => (shut(), keys(), zoom.stop())
  })
  // The panel, in the middle of the window: three across, as many rows as it has apps for.
  // In the launcher, in the middle of its card, which it veils.
  const area = () => props.within ?? { x: 0, y: 0, width: windowWidth(), height: viewport().height }
  const panel = () => {
    const width = Math.min(352, area().width - 56)
    const rows = Math.max(1, Math.ceil((category()?.entrypoints.length ?? 0) / 3))
    const height = 24 + rows * 88 + (rows - 1) * 16 + 16
    const top = area().y + (area().height - (height + 20 + 34)) / 2 + 54
    return { x: area().x + (area().width - width) / 2, y: top, width, height }
  }
  const t = () => zoom.value()
  const moving = () => !reducedMotion() && !!places[props.opened]
  // Where it's drawn: its square in the App Library, becoming the panel as it opens.
  const shape = () => {
    const [to, from] = [panel(), places[props.opened]]
    if (!moving() || !from) return to
    const at = (a: number, b: number) => a + (b - a) * t()
    return { x: at(from.x, to.x), y: at(from.y, to.y), width: at(from.width, to.width), height: at(from.height, to.height) }
  }
  const tile = () => Math.min(60, 88 - 28)
  return (
    <View
      role="dialog"
      aria-label={category()?.category}
      style={[looks.dialog, looks.categoryLayer, phase() === "closing" && looks.inert, looks.sized(windowWidth(), viewport().height)]}
    >
      {/* What's behind blurs through a veil. */}
      <Pressable aria-hidden="true" onPress={close} style={[looks.cover, looks.frosted, props.within && looks.round, props.within && looks.at(area().x, area().y, area().width, area().height), looks.opacity(t())]} />
      <Text role="heading" aria-level={2} style={[looks.categoryTitle, looks.titled(area().x, area().width, panel().y - 54, t(), moving() ? 12 * (1 - t()) : 0)]}>
        {category()?.category}
      </Text>
      <View style={[looks.panel, looks.at(shape().x, shape().y, shape().width, shape().height), looks.opacity(moving() ? 1 : t())]}>
        <View style={[looks.inPanel, looks.at((shape().width - panel().width) / 2, (shape().height - panel().height) / 2, panel().width, panel().height)]}>
          <View style={[looks.opened, looks.opacity(Math.max(0, (t() - 0.3) / 0.7))]}>
            <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
              {(entrypoint) => {
                // What's held here is an app of the App Library, as there: its menu, or carried onto Home's pages.
                const held = holds(() => ({ item: opens(entrypoint()), listed: entrypoint(), page: 0, index: -1, from: "library" }), () => true)
                return (
                  <View style={looks.tall(88)}>
                    <Tile name={entrypoint().name} icon={entrypoint().icon} size={tile()} scale={1} badge={badgeOf(entrypoint() as Entrypoint, props.owner)} holds={held} onPress={(behind, from) => phase() === "open" && press(entrypoint(), props.owner, behind, from)} />
                  </View>
                )
              }}
            </For>
          </View>
        </View>
      </View>
    </View>
  )
}
