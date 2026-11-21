import { createSignal, For } from "solid-js"
import { Pressable, type Scenes, type Styles, styles, Text, tokens, View } from "ui"
import { brand } from "./brand.ts"

const look = styles({
  row: { flexDirection: "row", gap: tokens.grid[3], alignItems: "flex-end" },
  column: { gap: tokens.grid[1], alignItems: "center" },
  stack: { gap: tokens.grid[4] },
  tile: { width: brand.tile, height: brand.tile, borderRadius: tokens.radius.medium, backgroundColor: brand.accent },
  ours: { width: brand.tile, height: brand.tile, borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.magenta },
  tint: { width: brand.tile, height: brand.tile, borderRadius: tokens.radius.medium, backgroundColor: tokens.colour.magentaTint },
  label: { font: tokens.type.small, color: tokens.colour.secondaryLabel },
  bar: (height: number, colour: `#${string}`) => ({ width: tokens.grid[8], height, borderRadius: tokens.radius.small, backgroundColor: colour }),
  track: { position: "relative", width: "100%", height: tokens.grid[8], borderRadius: tokens.radius.full, backgroundColor: tokens.colour.quinaryLabel },
  marker: (start: number, size: number, turn: number) => ({ position: "absolute", top: 0, start, width: size, height: size, borderRadius: tokens.radius.small, backgroundColor: tokens.colour.blue, rotate: turn }),
  cases: { flexDirection: "row", flexWrap: "wrap", gap: tokens.grid[4] },
  case: { width: 96, gap: tokens.grid[2] },
  room: { position: "relative", height: 136 },
  box: { width: 64, height: 64, alignItems: "center", justifyContent: "center", borderRadius: tokens.radius.small },
  name: { font: tokens.type.medium, color: "#ffffff" },
  first: { backgroundColor: tokens.colour.blue },
  second: { backgroundColor: tokens.colour.orange },
  relative: { position: "relative", top: 40 },
  absolute: { position: "absolute", top: 8, start: 0 },
  raised: { zIndex: 1 },
  highest: { zIndex: 2 },
  stacking: { zIndex: 0 },
  later: { position: "relative" },
  plain: { flexShrink: 0 },
})

/** Two boxes, the first (A) moved over the second (B), which follows it, in a box of its own or not, each with its
 * styles, as CSS paints them (CSS 2.1, Appendix E): what's positioned or says its `zIndex` after its stacking
 * context's flow, by `zIndex`, then in the tree's order. */
const order: [string, string, Styles[], Styles | undefined, Styles[]][] = [
  ["Relative", "1", [look.relative], undefined, []],
  ["In a box", "2", [look.relative], look.plain, []],
  ["zIndex 2 in a zIndex 0 box, then 1", "3", [look.relative, look.highest], look.stacking, [look.later, look.raised]],
  ["zIndex 1, then relative", "4", [look.relative, look.raised], look.plain, [look.later]],
  ["Absolute, then relative", "5", [look.absolute], undefined, [look.later]],
]

/** What a chart has from its data: a height and a colour for each bar. */
const bars: [number, `#${string}`][] = [
  [24, "#e5484d"],
  [64, "#f5a524"],
  [96, "#30a46c"],
  [48, "#3e63dd"],
]

export default {
  "Styling/An app's own tokens": () => (
    <View style={look.row}>
      <View style={look.column}>
        <View style={look.tile} />
        <Text style={look.label}>brand.accent</Text>
      </View>
      <View style={look.column}>
        <View style={look.ours} />
        <Text style={look.label}>magenta</Text>
      </View>
      <View style={look.column}>
        <View style={look.tint} />
        <Text style={look.label}>its tint</Text>
      </View>
    </View>
  ),
  "Styling/Dynamic styles": () => (
    <View style={look.stack}>
      <View style={look.row}>
        <For each={bars}>{([height, colour]) => <View style={look.bar(height, colour)} />}</For>
      </View>
      <View style={look.track}>
        <View style={look.marker(40, 20, 0)} />
        <View style={look.marker(120, 20, 45)} />
      </View>
    </View>
  ),
  "Styling/Painting order": {
    render: () => {
      const [pressed, press] = createSignal("nothing")
      const box = (name: string, style: Styles[]) => (
        <Pressable role="button" aria-label={name} onPress={() => press(name)} style={[look.box, ...style]}>
          <Text aria-hidden="true" style={look.name}>{name}</Text>
        </Pressable>
      )
      return (
        <View style={look.stack}>
          <Text style={look.label}>{`Pressed ${pressed()}`}</Text>
          <View style={look.cases}>
            <For each={order}>
              {([caption, n, first, around, second]) => (
                <View style={look.case}>
                  <Text style={look.label}>{caption}</Text>
                  <View style={look.room}>
                    {around ? <View style={around}>{box(`A${n}`, [look.first, ...first])}</View> : box(`A${n}`, [look.first, ...first])}
                    {box(`B${n}`, [look.second, ...second])}
                  </View>
                </View>
              )}
            </For>
          </View>
        </View>
      )
    },
    // A press reaches what's drawn over the rest: each presses where the two overlap, the middle of the one CSS draws on
    // top, and that's the one pressed.
    checks: Object.fromEntries(
      (
        [
          ["A relative box is over what follows it", "A1"],
          ["One in a box is over what follows the box", "A2"],
          ["A zIndex in a stacking context counts only in it", "B3"],
          ["zIndex 1 is over a relative box that follows it", "A4"],
          ["A relative box is over an absolute one before it", "B5"],
        ] as const
      ).map(([check, top]) => [check, [{ role: "button", name: top }, { expect: { text: `Pressed ${top}` } }]]),
    ),
  },
} satisfies Scenes
