import { For } from "solid-js"
import { Box, Image, Text, type Scenes } from "ui"

/// A letter on a colour, as an SVG an app draws for an item with no icon of its own: its fill in oklch(), as CSS
/// colours are, and its letter as SVG text.
const tile = (letter: string, fill: string) =>
  `data:image/svg+xml,${encodeURIComponent(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" style="fill:${fill}"/><text x="16" y="16" dy="0.35em" text-anchor="middle" font-family="system-ui, sans-serif" font-size="16" font-weight="600" fill="#fff">${letter}</text></svg>`)}`

const tiles = [
  ["G", "oklch(0.55 0.13 110)"],
  ["C", "oklch(0.55 0.13 15)"],
  ["E", "oklch(0.55 0.13 140 / 80%)"],
  ["H", "#3b82f6"],
] as const

export default {
  "Images/SVG tiles": () => (
    <Box style={{ gap: 12 }}>
      <For each={tiles}>
        {(tile_) => (
          <Box style={{ flexDirection: "row", alignItems: "center", gap: 12 }}>
            <Image src={tile(tile_[0], tile_[1])} alt="" style={{ width: 32, height: 32, borderRadius: "md" }} />
            <Text>{tile_[1]}</Text>
          </Box>
        )}
      </For>
    </Box>
  ),
} satisfies Scenes
