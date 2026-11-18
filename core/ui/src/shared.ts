// What every component of the UI library draws from: the words it says, the device it's on, and how things move.
import { createSignal } from "solid-js"

export { messages, readable, setMessages, whole, words } from "./words.ts"

/// Whether a media query matches, as a signal that follows the device.
function matching(query: string) {
  const media = matchMedia(query)
  const [matches, set] = createSignal(media.matches)
  media.addEventListener("change", () => set(media.matches))
  return matches
}

/// The devices whose own pickers are better than ours: a phone or tablet, where a finger is the pointer. It
/// follows the device: a mouse plugged into a tablet is a mouse from then on.
export const touch = matching("(pointer: coarse)")

/// Whether the user asked their device for less motion: what the UI library moves in JavaScript asks this first.
export const reduced = matchMedia("(prefers-reduced-motion: reduce)")

/// Which way text runs, as the page says on `<html dir>`. It can change its mind, and Zag is told again.
const [direction, turn] = createSignal<"ltr" | "rtl">(document.documentElement.dir === "rtl" ? "rtl" : "ltr")
new MutationObserver(() => turn(document.documentElement.dir === "rtl" ? "rtl" : "ltr")).observe(document.documentElement, { attributeFilter: ["dir"] })
export const dir = direction

/// One of the UI library's three paces (`quick`, `settle`, `travel`) as the timing of an animation of `element`'s:
/// tokens.css says how long each is and how it eases.
export function pace(element: Element, name: "quick" | "settle" | "travel"): { duration: number; easing: string } {
  const tokens = getComputedStyle(element)
  const time = tokens.getPropertyValue(`--move-${name}`).trim()
  // A time is written in milliseconds or seconds, and an animation counts in milliseconds.
  return { duration: parseFloat(time) * (time.endsWith("ms") ? 1 : 1000), easing: tokens.getPropertyValue(`--ease-${name}`).trim() }
}

/// `await share(link)`: hands text to the device's own share sheet where it has one, and copies it otherwise. Call
/// it as a press is handled: a device shares only what its person just asked to. → `"shared"`, `"copied"`, or
/// `undefined` if they closed the sheet.
export async function share(text: string): Promise<"shared" | "copied" | undefined> {
  if (!navigator.share) return navigator.clipboard.writeText(text).then(() => "copied")
  try {
    await navigator.share({ text })
    return "shared"
  } catch (error) {
    if (!(error instanceof Error && error.name === "AbortError")) throw error
  }
}

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// `seen(device.seen)`: when a device or a machine that's off was last on, as its grey line says it, in the largest
/// unit that's whole: "Seen 2 minutes ago". `at` is a time, or milliseconds since 1970.
export function seen(at: string | number | Date): string {
  const minutes = Math.round((new Date(at).getTime() - Date.now()) / 60_000)
  if (minutes > -60) return `Seen ${ago.format(Math.min(minutes, -1), "minute")}`
  if (minutes > -48 * 60) return `Seen ${ago.format(Math.round(minutes / 60), "hour")}`
  return `Seen ${ago.format(Math.round(minutes / (24 * 60)), "day")}`
}
