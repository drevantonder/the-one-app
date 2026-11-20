// The pills: the owner's name, which opens the list of every owner, and the page dots with the App Library's glyph.
import { For } from "solid-js"
import { type Element, haptic, Icon, Menu, Pressable, type Pressed, Text, View } from "ui"
import { BAR, DOTS } from "./layout.ts"
import { type Kind } from "./machine.ts"
import { looks } from "./looks.ts"
import { home, state, seen, along, rolled, at, screensOf, lastOf, editingNow, editingOwner, launcher } from "./store.ts"
import { type Owner } from "./owners.ts"

/// A window onto a row for each owner, which rolls as the owners move up and down, `at` in owners, each row `height`
/// tall: the row nearest is laid out, and so says how wide the window is, and the others are drawn above and below
/// it, fading as they roll away, as a picker's do.
function Roll(props: { count: number; height: number; row: (index: number) => Element }) {
  const nearest = () => Math.max(0, Math.min(props.count - 1, Math.round(rolled())))
  // In the launcher each row goes where it's going by a spring, as the owners' pages roll.
  const rolling = () => launcher() && looks.rolling
  return (
    <View style={[looks.roll, looks.tall(props.height)]}>
      <View inert aria-hidden="true" style={[looks.rollSizer, looks.tall(props.height)]}>
        {props.row(nearest())}
      </View>
      <For each={Array.from({ length: props.count }, (_, index) => index)}>
        {(index) => (
          <View inert={index !== nearest() || undefined} style={[looks.row, rolling(), looks.top(0), looks.drifted(Math.max(0, 1 - Math.abs(index - rolled()) * 1.5), (index - rolled()) * props.height), looks.tall(props.height)]}>
            {props.row(index)}
          </View>
        )}
      </For>
    </View>
  )
}

/// An owner's page dots, a dot a screenful, the one shown brighter; tapping one goes there, and sliding along them
/// moves a screenful every 28 points, as an iPhone's do. While their pages are edited, a tap opens the page overview.
/// Only the owner shown's take a finger.
function Dots(props: { owner: Owner; index: number; count: number; size: number }) {
  const current = () => (seen().owner === props.index ? seen().screen : (lastOf()[props.index] ?? 0))
  // A dot a screenful, as many pages side by side as fit, the one shown lit; as the pages move, each is lit as much as
  // its screenful shows.
  // The App Library, after the last screenful of pages, lights the glyph and none of them.
  const library = () => screensOf()[props.index]?.indexOf("library") ?? -1
  const onLibrary = () => library() >= 0 && current() >= library()
  const lit = (screen: number) => !onLibrary() && screen === current()
  const shown = (screen: number) => {
    const from = seen().owner === props.index ? along() : current()
    return Math.max(0, Math.min(1, screen + 1 - from, from + 1 - screen))
  }
  const last = () => props.count - 1
  let slid: { x: number; from: number; sliding: boolean; id: number } | undefined
  let quietDots = 0
  const slide = {
    onPointerDown: (event: Pressed) => {
      if (editingOwner(props.index) || seen().owner !== props.index) return
      slid = { x: event.clientX, from: current(), sliding: false, id: event.pointerId }
    },
    onPointerMove: (event: Pressed) => {
      if (!slid || event.pointerId !== slid.id) return
      const across = event.clientX - slid.x
      if (!slid.sliding && Math.abs(across) < 10) return
      slid.sliding = true
      const screen = Math.max(0, Math.min(props.count, slid.from + Math.trunc(across / 28)))
      if (screen === current()) return
      home.send({ type: "go", screen })
      haptic("selection")
    },
    onPointerUp: () => {
      if (slid?.sliding) quietDots = Date.now() + 300
      slid = undefined
    },
    onPointerCancel: () => (slid = undefined),
  }
  const press = (screen: number) => () => {
    if (seen().owner !== props.index) return
    if (editingOwner(props.index)) return home.send({ type: "overview" })
    if (Date.now() > quietDots) home.send({ type: "go", screen: Math.max(0, Math.min(screen, last())) })
  }
  return (
    <View role="navigation" aria-label={`${props.owner.name}'s pages`} aria-hidden={seen().owner !== props.index ? "true" : undefined} {...slide} style={looks.dots}>
      <For each={Array.from({ length: Math.max(1, props.count) }, (_, at) => at)}>
        {(screen) => (
          <Pressable role="button" aria-label={`Page ${screen + 1} of ${props.count}`} aria-current={lit(screen) ? "true" : undefined} onPress={press(screen)} style={looks.dotButton}>
            <View style={[looks.dot, shown(screen) >= 0.5 && looks.dotLit, looks.dotAt(props.size, props.size * 0.64, 0.4 + 0.6 * shown(screen))]} />
          </Pressable>
        )}
      </For>
    </View>
  )
}

/// The owner pill, at the top of Home in the middle, and of the launcher's card: whose pages they are, "You ⌄", which
/// opens the list of every owner, the current one ticked, in a pill of Liquid Glass as the dots have. Its name rolls as
/// the owners move. While pages are edited, + and Done are either side of it, and it opens nothing.
export function OwnerPill(props: { owners: Owner[] }) {
  const shown = () => at().owner
  return (
    <Menu.Root
      open={state().matches("listing")}
      onOpenChange={(open) => (open ? state().matches("nothing") && home.send({ type: "list" }) : state().matches("listing") && home.send({ type: "dismiss" }))}
      onSelect={(value) => home.send({ type: "pick", owner: Number(value) })}
    >
      <View style={[looks.pill, looks.glass, looks.ownerPill]}>
        <Menu.Trigger variant="ghost" shape="capsule" aria-label={props.owners[shown()]?.name}>
          <View style={looks.ownerRow}>
            <Roll count={props.owners.length} height={BAR - 8} row={(index) => <Text style={looks.ownerName}>{props.owners[index]?.name}</Text>} />
            <View style={looks.caret}>
              <Icon name="caret-down" />
            </View>
          </View>
        </Menu.Trigger>
      </View>
      <Menu.Positioner>
        <Menu.Content>
          {/* Each owner's item stays as the owners are asked for again, as anything changes, so a press on one isn't
              lost to a new one drawn in its place. */}
          <For each={props.owners} keyed={(owner) => owner.key}>
            {(owner, index) => (
              <Menu.Item value={String(index())}>
                <View style={looks.ownerItem}>
                  <View style={looks.opacity(index() === shown() ? 1 : 0)}>
                    <Icon name="check" />
                  </View>
                  <Text>{owner().name}</Text>
                </View>
              </Menu.Item>
            )}
          </For>
        </Menu.Content>
      </Menu.Positioner>
    </Menu.Root>
  )
}

/// The dots, at the bottom in the middle, just clear of the home indicator, floating with nothing behind them, as an
/// iPhone's are: the dots of the owner shown, then the App Library's glyph, which says there's one more page and goes
/// there, lit as it's reached, as the dots are. They stay where they are as the owners move, their dots rolling to the
/// next owner's and taking their number, and go with Home to the App Library, which has none. While pages are edited
/// the glyph stays, dimmed, and goes nowhere.
export function DotsPill(props: { owners: Owner[]; laid: Kind[][] }) {
  const count = (owner: number) => (props.laid[owner] ?? []).filter((kind) => kind === "home").length
  const library = () => props.laid[seen().owner]?.[seen().screen] === "library"
  const go = () => {
    if (state().matches("editing")) return
    const screen = props.laid[state().context.at.owner]?.indexOf("library") ?? -1
    if (screen >= 0) home.send({ type: "go", screen })
  }
  return (
    <View style={[looks.pill, looks.live, looks.tall(DOTS)]}>
      <Roll count={props.owners.length} height={DOTS} row={(index) => <Dots owner={props.owners[index]} index={index} count={count(index)} size={7} />} />
      <Pressable
        role="button"
        aria-label="App Library"
        aria-current={library() ? "true" : undefined}
        aria-disabled={editingNow() ? "true" : undefined}
        onPress={go}
        style={[looks.libraryGlyph, library() && looks.lit, looks.opacity(editingNow() ? 0.35 : library() ? 1 : 0.4)]}
      >
        <View style={looks.smaller}>
          <Icon name="squares-four" />
        </View>
      </Pressable>
    </View>
  )
}
