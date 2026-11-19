// What's done on a screen, as it's sent to it: down a `video`'s data channel (renderer.ts), or as Computer Use's
// actions where there's only a picture of it (../live-screen.tsx).

/** What the person does on a stream, as its data channel carries it to the screen. */
export type Input = { t: "move"; x: number; y: number } | { t: "button"; b: number; down: boolean; mods: string[] } | { t: "key"; key: string; mods: string[] } | { t: "text"; text: string }

/** A key pressed, with the modifiers held, as the DOM's `KeyboardEvent` has them. */
type Pressed = { key: string; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; metaKey: boolean }

// Keys as the browser names them, as X does, which is how a screen is told them.
const KEYS: Record<string, string> = { Enter: "Return", Backspace: "BackSpace", Tab: "Tab", Escape: "Escape", Delete: "Delete", ArrowLeft: "Left", ArrowRight: "Right", ArrowUp: "Up", ArrowDown: "Down", Home: "Home", End: "End", PageUp: "Prior", PageDown: "Next" }

/// A key as X names it, where it's sent as a key: not a letter typed alone, which comes as what's typed, nor a paste,
/// which is its field's own and comes as what's typed too.
export function named(event: Pressed): string | undefined {
  const name = KEYS[event.key] ?? (event.key.length === 1 ? event.key.toLowerCase() : undefined)
  const modified = event.ctrlKey || event.metaKey || event.altKey
  if (!name || (!modified && event.key.length === 1) || (modified && event.key.toLowerCase() === "v")) return
  return name
}

/** The device's own keys for full screen, F11, or ⌃⌘F on a Mac, which are the page's, never the screen's. */
export const fullScreen = (event: Pressed) => event.key === "F11" || (event.ctrlKey && event.metaKey && event.key.toLowerCase() === "f")

/** The modifiers held, as X names them: a Mac's ⌘ is Control, as Chrome's shortcuts on Linux take it. */
export const mods = (event: Omit<Pressed, "key">, mac: boolean) =>
  [(event.ctrlKey || (mac && event.metaKey)) && "Control_L", event.altKey && "Alt_L", event.shiftKey && "Shift_L", !mac && event.metaKey && "Super_L"].filter((mod): mod is string => !!mod)
