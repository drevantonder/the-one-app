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
