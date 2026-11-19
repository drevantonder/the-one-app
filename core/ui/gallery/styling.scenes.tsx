import { For } from "solid-js"
import { type Scenes, styles, Text, tokens, View } from "ui"
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
})

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
} satisfies Scenes
