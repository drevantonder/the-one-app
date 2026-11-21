// The Home Screen's apps and widgets as each is drawn wherever it is: a widget, an icon with its name and its red count,
// and the ⊖ that takes one off.
import { createEffect, createMemo, createSignal, Errored, Loading, onSettled, refresh, Show, untrack } from "solid-js"
import { Badge, call, changed, Icon, Image, type Laid, type Layout, type Located, Pressable, type Rect, type Styles, Text, View, visible, Widget } from "ui"
import { type Widget as WidgetItem, src } from "./types.ts"
import { corner } from "./layout.ts"
import { looks } from "./looks.ts"
import { type Holds } from "./holds.ts"
import { state, tapping } from "./store.ts"
import { json, type Owner, type From, launch } from "./owners.ts"

/// A widget on a page: what its app's tool answers, drawn, and asked again as its app says something changed. A widget
/// whose layout says `refresh` (seconds), as one that shows what only time changes does, is also asked again that
/// often while it's in view. A control that failed says so on the widget, and pressing it asks again; else pressing it
/// opens its app.
export function Card(props: { item: WidgetItem; owner: Owner; still?: boolean }) {
  const tool = () => props.item.widget.replace("/", "_")
  const layout = createMemo(() => json<Layout & { refresh?: number }>(tool(), { size: props.item.size }, props.owner.subject))
  const [problem, setProblem] = createSignal<string>()
  const again = () => refresh(layout)
  onSettled(() => changed(again))
  const act = async (name: string | undefined, inputs: Record<string, unknown>) => {
    try {
      await call(`${props.item.widget.split("/")[0]}_${name}`, inputs, { subject: props.owner.subject })
    } catch (error) {
      return setProblem(`${(error as Error).message ?? error} Tap to try again.`)
    }
    again()
  }
  const retry = () => !props.still && (setProblem(undefined), again())
  const opens = (card: { getBoundingClientRect(): Rect }) => {
    if (props.still || !state().matches("nothing") || !props.item.opens || !tapping()) return
    const { x, y, width, height } = card.getBoundingClientRect()
    launch(props.item.opens, props.owner, false, { x, y, width, height, radius: 22 * (width / (props.item.size === "2x2" ? 170 : 352)) })
  }
  return (
    <View style={looks.widgetCell}>
      <Errored fallback={(error) => <Widget name={props.item.name} size={props.item.size} problem={String((error() as Error).message ?? error())} />}>
        <Loading fallback={<Widget name={props.item.name} size={props.item.size} />}>
          <Widget name={props.item.name} size={props.item.size} layout={layout()} problem={problem()} onCall={props.still ? undefined : act} onOpen={problem() ? retry : props.item.opens ? opens : undefined} />
          <Refresh every={layout().refresh} then={again} />
        </Loading>
      </Errored>
    </View>
  )
}

/// Runs `then` every `every` seconds, while the Home Screen is shown.
function Refresh(props: { every?: number; then: () => void }) {
  createEffect(
    () => props.every,
    (every) => {
      if (!every) return
      const timer = setInterval(() => untrack(visible) && props.then(), every * 1000)
      return () => clearInterval(timer)
    },
  )
  return null
}

/// What waits in an app, as its app counts it (`badge`): a red count at its icon's corner, as an iPhone's.
export const Count = (props: { count?: number }) => (
  <Show when={props.count}>
    <View aria-hidden="true" style={looks.count}>
      <Badge variant="destructive">{props.count}</Badge>
    </View>
  </Show>
)

/// An icon with its name under it, `size` across, drawn `scale` times larger. The app starts as a finger lands on it,
/// and shows as the finger lifts, unless the finger held it, growing out of its icon. `style` moves it, and `nameStyle`
/// its name. Its badge is drawn at its icon's corner, unless `counted` is false: then whoever flies it draws the badge.
export function Tile(props: { name: string; icon: string; size: number; scale: number; badge?: number; counted?: boolean; onPress?: (behind: boolean, from?: From) => void; holds?: Holds; onLayout?: (event: Laid) => void; style?: Styles; nameStyle?: Styles }) {
  const from = (event: Located): From => {
    const tile = event.currentTarget.getBoundingClientRect()
    const size = props.size * props.scale
    return { x: tile.x + (tile.width - size) / 2, y: tile.y, width: size, height: size, radius: size * 0.225 }
  }
  return (
    <Pressable
      role="button"
      aria-label={props.name}
      aria-description={props.badge ? `${props.badge} waiting` : undefined}
      {...props.holds}
      onPointerDown={(event) => {
        if (event.button === 0) props.onPress?.(true)
        props.holds?.onPointerDown?.(event)
      }}
      onPress={(event) => tapping() && props.onPress?.(false, from(event))}
      onLayout={props.onLayout}
      style={[looks.tileBox, props.style]}
    >
      <View style={[looks.icon, looks.square(props.size)]}>
        <Image src={src(props.icon)} alt="" style={[looks.iconImage(corner(props.size)), looks.square(props.size)]} />
        <Count count={props.counted === false ? undefined : props.badge} />
      </View>
      <Text style={[looks.name, props.nameStyle]}>{props.name}</Text>
    </Pressable>
  )
}

/// ⊖, at the corner of what it takes off: pressed, not held or lifted.
export function Minus(props: { label: string; left: number; top: number; onPress: () => void }) {
  return (
    <Pressable
      role="button"
      aria-label={props.label}
      onPress={() => tapping() && props.onPress()}
      style={[looks.minus, looks.at(props.left, props.top, 24, 24)]}
    >
      <Icon name="minus" />
    </Pressable>
  )
}
