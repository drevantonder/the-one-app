// Where a popup goes beside what opened it: a menu under its button, a select's list under its face, flipped to the
// other side where there isn't room, and kept inside the window. Copied from Vercel's UI SDK (Apache-2.0) at
// ab55044caee66cc8228fa7ed3525088a1b7b5af7: `computePlacement` and what it uses from
// packages/core/src/foundation/platform/position.ts, and foundation/platform/sides.ts. Pure: the anchor's rectangle
// and the popup's size are what the host lays out (`onLayout`), and the window's is `viewport()`.

/** A side of the anchor. `start` and `end` are logical: left and right, swapped in right-to-left layouts. */
export type Side = "top" | "bottom" | "left" | "right" | "start" | "end"
/** A side on screen, after logical sides are resolved. */
type PhysicalSide = "top" | "bottom" | "left" | "right"
/** Which edge of the anchor the popup lines up with. */
export type Align = "start" | "center" | "end"
type Rect = { x: number; y: number; width: number; height: number }
// The visible part of the window.
type Viewport = { x?: number; y?: number; width: number; height: number }

/** How a popup is placed: its side, its alignment, how far from the anchor (`sideOffset`) and along it
 * (`alignOffset`), and how far it stays inside the window's edges (`collisionPadding`). */
export type Placement = { side: Side; align: Align; sideOffset: number; alignOffset: number; collisionPadding: number; rtl?: boolean }

// `start` and `end` on screen.
function physicalSide(side: Side, rtl = false): PhysicalSide {
  if (side === "start") return rtl ? "right" : "left"
  if (side === "end") return rtl ? "left" : "right"
  return side
}

const opposite: Record<PhysicalSide, PhysicalSide> = { top: "bottom", bottom: "top", left: "right", right: "left" }

const isVertical = (side: Side) => side === "top" || side === "bottom"

function place(anchor: Rect, size: { width: number; height: number }, side: PhysicalSide, align: Align, options: Placement) {
  const { sideOffset, alignOffset } = options
  const vertical = side === "top" || side === "bottom"
  let x = 0
  let y = 0
  if (side === "bottom") y = anchor.y + anchor.height + sideOffset
  if (side === "top") y = anchor.y - size.height - sideOffset
  if (side === "right") x = anchor.x + anchor.width + sideOffset
  if (side === "left") x = anchor.x - size.width - sideOffset
  if (vertical) {
    if (align === "start") x = anchor.x + alignOffset
    if (align === "center") x = anchor.x + anchor.width / 2 - size.width / 2 + alignOffset
    if (align === "end") x = anchor.x + anchor.width - size.width - alignOffset
  } else {
    if (align === "start") y = anchor.y + alignOffset
    if (align === "center") y = anchor.y + anchor.height / 2 - size.height / 2 + alignOffset
    if (align === "end") y = anchor.y + anchor.height - size.height - alignOffset
  }
  return { x, y }
}

function overflow(side: PhysicalSide, point: { x: number; y: number }, size: { width: number; height: number }, viewport: Required<Viewport>, padding: number) {
  if (side === "bottom") return point.y + size.height - (viewport.y + viewport.height - padding)
  if (side === "top") return viewport.y + padding - point.y
  if (side === "right") return point.x + size.width - (viewport.x + viewport.width - padding)
  return viewport.x + padding - point.x
}

function available(side: PhysicalSide, anchor: Rect, viewport: Required<Viewport>, options: Placement) {
  const { sideOffset, collisionPadding: padding } = options
  const height =
    side === "bottom" ? viewport.y + viewport.height - padding - (anchor.y + anchor.height + sideOffset) : side === "top" ? anchor.y - sideOffset - padding - viewport.y : viewport.height - padding * 2
  const width =
    side === "right" ? viewport.x + viewport.width - padding - (anchor.x + anchor.width + sideOffset) : side === "left" ? anchor.x - sideOffset - padding - viewport.x : viewport.width - padding * 2
  return { width: Math.max(0, width), height: Math.max(0, height) }
}

// Start and end along the top or bottom follow the reading direction.
const physicalAlign = (side: PhysicalSide, align: Align, rtl = false): Align => (rtl && isVertical(side) && align !== "center" ? (align === "start" ? "end" : "start") : align)

const clamp = (value: number, min: number, max: number) => Math.min(Math.max(value, min), Math.max(min, max))

/** Where a popup of `size` goes beside `anchor` in `visible`: its corner (`x`, `y`), the side it went to, and the room
 * it has there (`available`), which a long list scrolls in. */
export function computePlacement(anchor: Rect, size: { width: number; height: number }, visible: Viewport, placement: Placement) {
  const viewport = { x: 0, y: 0, ...visible }
  const options = { ...placement, side: physicalSide(placement.side, placement.rtl) }
  const align = physicalAlign(options.side, options.align, options.rtl)

  let side = options.side
  let point = place(anchor, size, side, align, options)
  const over = overflow(side, point, size, viewport, options.collisionPadding)
  if (over > 0) {
    const other = opposite[side]
    const otherPoint = place(anchor, size, other, align, options)
    if (overflow(other, otherPoint, size, viewport, options.collisionPadding) < over) {
      side = other
      point = otherPoint
    }
  }

  const padding = options.collisionPadding
  const room = available(side, anchor, viewport, options)
  // Beside an anchor at least its size, like a context area opened from the
  // keyboard, with room on neither side, it goes on screen inside the anchor.
  // A popup not measured yet has no size to fit.
  if (overflow(side, point, size, viewport, padding) > 0) {
    if ((side === "top" || side === "bottom") && size.height && anchor.height >= size.height) {
      point.y = clamp(clamp(point.y, viewport.y + padding, viewport.y + viewport.height - size.height - padding), anchor.y, anchor.y + anchor.height - size.height)
      room.height = Math.max(0, viewport.height - padding * 2)
    } else if ((side === "left" || side === "right") && size.width && anchor.width >= size.width) {
      point.x = clamp(clamp(point.x, viewport.x + padding, viewport.x + viewport.width - size.width - padding), anchor.x, anchor.x + anchor.width - size.width)
      room.width = Math.max(0, viewport.width - padding * 2)
    }
  }

  // Shifts along the anchor to stay on screen, but never past the anchor's
  // far edge, so a popup whose anchor scrolls away leaves with it. An
  // `alignOffset` that alone takes it past the anchor is kept.
  if (side === "top" || side === "bottom") {
    const x = clamp(point.x, viewport.x + padding, viewport.x + viewport.width - size.width - padding)
    point.x = x === point.x ? x : clamp(x, anchor.x - size.width, anchor.x + anchor.width)
  } else {
    const y = clamp(point.y, viewport.y + padding, viewport.y + viewport.height - size.height - padding)
    point.y = y === point.y ? y : clamp(y, anchor.y - size.height, anchor.y + anchor.height)
  }

  return { ...point, side, align: options.align, available: room }
}
