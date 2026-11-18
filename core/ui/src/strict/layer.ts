// A layer's own box (primitives.tsx, native/primitives.tsx): the whole window, over the page, letting presses through
// to the page where nothing of the layer's is.
import { styles } from "../styles.ts"

export const { layer } = styles({
  layer: { position: "fixed", top: 0, bottom: 0, start: 0, end: 0, zIndex: 50, pointerEvents: "none" },
})
