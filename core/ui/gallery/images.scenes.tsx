import { For } from "solid-js"
import { Image, type Scenes, styles, Text, tokens, View } from "ui"

const look = styles({
  column: { gap: tokens.grid[3] },
  row: { flexDirection: "row", alignItems: "center", gap: tokens.grid[3] },
  tile: { width: tokens.grid[8], height: tokens.grid[8], borderRadius: tokens.radius.medium },
  large: (width: number, height: number) => ({ width, height, borderRadius: tokens.radius.medium }),
})

/// A letter on a colour, as an SVG an app draws for an item with no icon of its own: its fill in oklch(), as CSS
/// colours are, and its letter as SVG text.
const tile = (letter: string, fill: string) =>
  `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" style="fill:${fill}"/><text x="16" y="16" dy="0.35em" text-anchor="middle" font-family="system-ui, sans-serif" font-size="16" font-weight="600" fill="#fff">${letter}</text></svg>`)}`

/// A picture that takes a while to decode, as a large photo does: two circles blurred, which resvg draws at the
/// size it's shown. Its frame, drawn as it decodes in the background, doesn't wait for it (the harness's
/// `worst_paint_ms`).
const blurred = `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 300"><filter id="b"><feGaussianBlur stdDeviation="30"/></filter><g filter="url(#b)"><circle cx="120" cy="120" r="90" fill="#e33"/><circle cx="280" cy="190" r="110" fill="#33e"/></g></svg>`)}`

const tiles = [
  ["G", "oklch(0.55 0.13 110)"],
  ["C", "oklch(0.55 0.13 15)"],
  ["E", "oklch(0.55 0.13 140 / 80%)"],
  ["H", "#3b82f6"],
] as const

export default {
  "Images/SVG tiles": () => (
    <View style={look.column}>
      <For each={tiles}>
        {(tile_) => (
          <View style={look.row}>
            <Image src={tile(tile_[0], tile_[1])} alt="" style={look.tile} />
            <Text>{tile_[1]}</Text>
          </View>
        )}
      </For>
    </View>
  ),
  "Images/Slow to decode": () => <Image src={blurred} alt="" style={look.large(358, 268)} />,
  // A picture whose bytes aren't one, beside one that is: tried again, then shown broken, never as nothing.
  "Images/Broken": () => (
    <View style={look.row}>
      <Image src={tile("G", "oklch(0.55 0.13 110)")} alt="" style={look.tile} />
      <Image src="data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAKAAAABkCAIAAAA" alt="A broken picture" style={look.tile} />
    </View>
  ),
} satisfies Scenes
