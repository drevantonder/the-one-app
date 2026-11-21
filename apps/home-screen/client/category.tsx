import { createEffect, createSignal, For } from "solid-js"
import { covers, keydown, type Laid, Pressable, type Rect, ScrollView, Text, View, viewport } from "ui"
import { type Entrypoint } from "./types.ts"
import { looks } from "./looks.ts"
import { home, room, tapping, windowWidth } from "./store.ts"
import { type Owner, badgeOf, opens, press } from "./owners.ts"
import { holds } from "./holds.ts"
import { Tile } from "./tiles.tsx"
import { openedCategory, places } from "./library.tsx"

/// How large an opened category's apps' icons are.
const ICON = 60

/// A category opened over its App Library, as an iPhone's is: the whole screen (in the launcher, its card), its name as
/// a large title over all its apps in a grid, as Home's, which scrolls, over the App Library blurred and dimmed. It
/// grows out of its square there and shrinks back into it, the blur and the dim with it, on one spring the renderer
/// draws, turning back from wherever it's got to: closed, it's its square, exactly, with what's in it drawn as large as
/// the square's icons and its first app on the square's first, so its apps grow out of the square's own and shrink back
/// into them, as an iOS folder's do, wherever the square is. Tapping beside its apps, Escape or the device's own back
/// closes it at once, however far it's grown: what's still moving is the renderer's to finish. For whoever asked for
/// less motion, it fades. It's made ahead (`made`, as a press goes down on its square) and kept made, closed, shrunk
/// into its square and not shown, taking nothing, so the frame it opens or closes on only changes how it's drawn.
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
  // Where its first app's icon is in it, open: measured from where it and the first app are drawn, as large as each is
  // drawn, once as it's made and again as the area's width changes, which moves its apps' columns. Till then it has no
  // transition, so it goes straight to where that puts it, not on its way there as it opens.
  const [first, setFirst] = createSignal<{ x: number; y: number }>()
  createEffect(() => area().width, () => setFirst(undefined))
  let drawn: { all?: Rect; app?: Rect } = {}
  const measured = (part: "all" | "app") => (event: Laid) => {
    drawn = { ...drawn, [part]: event.currentTarget.getBoundingClientRect() }
    const { all, app } = drawn
    if (!all || !app) return
    const scale = all.width / area().width
    setFirst({ x: (app.x + app.width / 2 - all.x) / scale - ICON / 2, y: (app.y - all.y) / scale })
    drawn = {}
  }
  // Its square, as it was drawn when it was pressed; open, the whole area. What's in it is the area's size, drawn as
  // large as the square's icons while it's closed, its first app's icon on the square's first, and cut to the square.
  // A scale is around the middle, so it moves by what puts that icon there. Until its first app's measured, it's drawn
  // as small as the square, from its top left.
  const square = () => places[props.made] ?? { x: area().x + area().width / 2, y: area().y + area().height / 2, width: 0, height: 0, first: { x: area().x + area().width / 2, y: area().y + area().height / 2, width: 0, height: 0 } }
  const box = () => (props.open ? area() : square())
  const from = () => first() ?? { x: 0, y: 0 }
  const to = () => (first() ? square().first : square())
  const small = () => (props.open ? 1 : first() ? square().first.width / ICON : square().width / area().width)
  const shift = (length: number, at: number, onto: number) => (props.open ? 0 : onto - length / 2 - small() * (at - length / 2))
  // A tap beside its apps closes it; one on an app is the app's (`Pressable`).
  const beside = () => tapping() && close()
  return (
    <View aria-hidden={props.open ? undefined : "true"} style={[looks.dialog, looks.categoryLayer, !props.open && looks.inert, looks.sized(windowWidth(), viewport().height)]}>
      {/* What's behind blurs and dims through a veil. */}
      <Pressable aria-hidden="true" onPress={close} style={[looks.cover, looks.frosted, looks.veils, !props.open && looks.unveiled, props.within && looks.round, props.within && looks.at(area().x, area().y, area().width, area().height)]} />
      <View role="dialog" aria-label={category()?.category} style={[looks.grows, !props.open && looks.shrunk, props.open && props.within && looks.round, looks.at(area().x, area().y, box().width, box().height), looks.moved(box().x - area().x, box().y - area().y)]}>
        <View style={[looks.shows, !props.open && looks.hides]}>
          <ScrollView style={[looks.openedArea, first() && looks.contents, !props.open && looks.inert, looks.sized(area().width, area().height), looks.scaledAt(small(), shift(area().width, from().x, to().x - square().x), shift(area().height, from().y, to().y - square().y))]}>
            <Pressable onPress={beside} onLayout={first() ? undefined : measured("all")} style={[looks.opened, looks.libraryPadding(inset().inline, inset().top, inset().bottom), looks.atLeast(area().height)]}>
              <Text role="heading" aria-level={2} style={looks.categoryTitle}>
                {category()?.category}
              </Text>
              <View style={looks.openedApps}>
                <For each={category()?.entrypoints ?? []} keyed={(entrypoint) => entrypoint.entrypoint ?? entrypoint.app}>
                  {(entrypoint, index) => {
                    // What's held here is an app of the App Library, as there: its menu, or carried onto Home's pages.
                    const held = holds(() => ({ item: opens(entrypoint()), listed: entrypoint(), page: 0, index: -1, from: "library" }), () => true)
                    return <Tile name={entrypoint().name} icon={entrypoint().icon} size={ICON} scale={1} onLayout={index() === 0 && !first() ? measured("app") : undefined} badge={badgeOf(entrypoint() as Entrypoint, props.owner)} holds={held} onPress={(behind, from) => press(entrypoint(), props.owner, behind, from)} />
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
