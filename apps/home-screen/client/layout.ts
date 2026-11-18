// Where everything on the Home Screen is drawn, in CSS pixels, worked out from the room it's given, as a phone's own
// home screen lays out its grid: every page, every cell on one and the controls over them. What's drawn is placed by
// these numbers, and what a finger does is read by them too: which page and cell a point is over, and where an icon is
// for an app to grow out of it.
import { COLUMNS, ROWS, type Placed } from "./machine.ts"

/** The controls over the pages (the owner pill, + and Done, the App Library's search): a fingertip tall. */
export const BAR = 44
/** The row they're in, under the status bar, and the page starts under it. */
export const TOP_ROW = BAR + 16
/** The dots pill: its height, and how far it is from the foot, clear of the home indicator. */
export const DOTS = 32
/** A phone's page, which a wide window draws larger, as many side by side as fit. */
export const PHONE = 390
/** The launcher's search bar across its top, as Raycast's is; the room its footer's pills float in, over its foot;
 * and its section's heading ("Home", "Results"), as Raycast's are. */
export const LAUNCHER_BAR = 64
export const LAUNCHER_FOOT = 56
export const LAUNCHER_HEADING = 30
/** How far in from the launcher's sides the pages and rows are, as Raycast's list is. */
export const LAUNCHER_SIDE = 8
/** A grid's gaps, across and down, and the tallest a row is. */
const COLUMN_GAP = 12
const ROW_GAP = 16
const ROW = 88

export interface Insets {
  top: number
  right: number
  bottom: number
  left: number
}

/** The room the pages have, and how they're laid out in it: whether it's wide, how many pages fit side by side, how
 * much larger than a phone's they're drawn (`scale`), and the room beside them and between them. */
export interface Room {
  width: number
  height: number
  wide: boolean
  fit: number
  scale: number
  side: number
  gap: number
  launcher: boolean
  safe: Insets
}

/// The pages' room, `width` by `height`: on a wide window, a phone's pages drawn larger to suit its height, twice at
/// most, so a bigger screen shows more pages rather than larger icons, as many whole ones side by side as fit; a
/// launcher's two across its panel, as near a phone's size as fit, with a hairline between them.
export function measure(width: number, height: number, launcher: boolean, safe: Insets): Room {
  const wide = width >= 544
  const scale = !wide ? 1 : launcher ? Math.min(1, (width - 2 * LAUNCHER_SIDE) / (2 * PHONE)) : Math.max(1, Math.min(2, height / 900))
  const [side, gap] = launcher ? [LAUNCHER_SIDE, 0] : [32 * scale, 48 * Math.min(scale, 1.6)]
  const fit = launcher && wide ? 2 : wide ? Math.max(1, Math.floor((width - 2 * side + gap) / (PHONE * scale + gap))) : 1
  return { width, height, wide, fit, scale, side, gap, launcher, safe }
}

/** A page as it's laid out, before it's drawn larger: its size, what's kept clear inside it, and its grid's tracks. */
export interface Page {
  width: number
  height: number
  top: number
  left: number
  right: number
  bottom: number
  column: number
  row: number
  /** An icon's size, as a row leaves room for it with its name under it. */
  tile: number
}

/// A page of `room`, laid out at a phone's size: under the top row and clear of what the device keeps at its edges;
/// in a launcher, with the padding Raycast's grid has, its rows a phone's tallest.
export function page(room: Room): Page {
  const width = room.wide ? Math.min((room.width - 2 * room.side - (room.fit - 1) * room.gap) / room.fit, 560 * room.scale) / room.scale : room.width
  const height = room.height / room.scale
  const [top, left, right, bottom] = room.launcher ? [4 / room.scale, 12 / room.scale, 12 / room.scale, 4 / room.scale] : [room.safe.top + TOP_ROW + 8, 20 + room.safe.left, 20 + room.safe.right, 0]
  return grid(width, height, top, left, right, bottom, room.launcher)
}

/// A page `width` by `height`, with `top`, `left`, `right` and `bottom` kept clear: four columns, and six rows, each as
/// tall as the page has room for under the dots pill, or a phone's tallest (`tallest`).
export function grid(width: number, height: number, top: number, left: number, right: number, bottom: number, tallest = false): Page {
  const column = (width - left - right - (COLUMNS - 1) * COLUMN_GAP) / COLUMNS
  const row = tallest ? ROW : Math.max(40, Math.min(ROW, (height - top - bottom - (ROWS - 1) * ROW_GAP - 64) / ROWS))
  return { width, height, top, left, right, bottom, column, row, tile: Math.min(60, row - 28) }
}

/** Where something is, in CSS pixels. */
export interface Box {
  x: number
  y: number
  width: number
  height: number
}

/// Where a placed cell is on its page, as it's laid out.
export function cell(page: Page, placed: Pick<Placed<unknown>, "column" | "row" | "width" | "height">): Box {
  return {
    x: page.left + (placed.column - 1) * (page.column + COLUMN_GAP),
    y: page.top + (placed.row - 1) * (page.row + ROW_GAP),
    width: placed.width * page.column + (placed.width - 1) * COLUMN_GAP,
    height: placed.height * page.row + (placed.height - 1) * ROW_GAP,
  }
}

/// How tall a page's grid is, all six rows.
export const gridHeight = (page: Page) => ROWS * page.row + (ROWS - 1) * ROW_GAP

/// Which column and row a point on a page is at, from the grid's top left, in cells, with a fraction: how the page's
/// own packing finds where something carried would go.
export function at(page: Page, x: number, y: number) {
  const across = (page.width - page.left - page.right + COLUMN_GAP) / COLUMNS
  const down = (gridHeight(page) + ROW_GAP) / ROWS
  return { column: (x - page.left) / across + 1, row: (y - page.top) / down + 1 }
}

/// Where each page of a screenful of `count` is across it: its left edge, drawn, in the middle of the screenful
/// where there are fewer than fit.
export function lefts(room: Room, laid: Page, count: number) {
  const drawn = laid.width * room.scale
  const start = (room.width - (count * drawn + (count - 1) * room.gap)) / 2
  return Array.from({ length: count }, (_, at) => start + at * (drawn + room.gap))
}
