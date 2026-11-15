// What every component of the kit draws from: the words it says, the device it's on, and how things move.
import { createSignal, createStore } from "solid-js"

/// The few words the kit says itself, in English until a page says otherwise with `words`.
export const [said, say] = createStore({
  back: "Back",
  cancel: "Cancel",
  clear: "Clear",
  close: "Close",
  file: "Choose a file",
  loading: "Loading",
  more: "More",
  ok: "OK",
  remove: "Remove",
})

/// `words({ back: "Zurück", cancel: "Abbrechen", close: "Schließen" })`: says the kit's own words in the user's
/// language, once, for every component on the page. A word the kit doesn't say throws the words it does.
export function words(theirs) {
  for (const name in theirs) if (!(name in said)) throw new Error(`The kit has no word "${name}". It has: ${Object.keys(said).join(", ")}`)
  say((words) => void Object.assign(words, theirs))
}

/// Whether a media query matches, as a signal that follows the device.
function matching(query) {
  const media = matchMedia(query)
  const [matches, set] = createSignal(media.matches)
  media.addEventListener("change", () => set(media.matches))
  return matches
}

/// The devices whose own pickers are better than ours: a phone or tablet, where a finger is the pointer. It
/// follows the device: a mouse plugged into a tablet is a mouse from then on.
export const touch = matching("(pointer: coarse)")

/// Whether the user asked their device for less motion: what the kit moves in JavaScript asks this first.
export const reduced = matchMedia("(prefers-reduced-motion: reduce)")

/// Which way text runs, as the page says on `<html dir>`. It can change its mind, and Zag is told again.
const [direction, turn] = createSignal(document.documentElement.dir === "rtl" ? "rtl" : "ltr")
new MutationObserver(() => turn(document.documentElement.dir === "rtl" ? "rtl" : "ltr")).observe(document.documentElement, { attributeFilter: ["dir"] })
export const dir = direction

/// One of the kit's three paces (`quick`, `settle`, `travel`) as the timing of an animation of `element`'s:
/// tokens.css says how long each is and how it eases.
export function pace(element, name) {
  const tokens = getComputedStyle(element)
  const time = tokens.getPropertyValue(`--move-${name}`).trim()
  // A time is written in milliseconds or seconds, and an animation counts in milliseconds.
  return { duration: parseFloat(time) * (time.endsWith("ms") ? 1 : 1000), easing: tokens.getPropertyValue(`--ease-${name}`).trim() }
}

/// `await share(link)`: hands text to the device's own share sheet where it has one, and copies it otherwise. Call
/// it as a press is handled: a device shares only what its person just asked to. → `"shared"`, `"copied"`, or
/// `undefined` if they closed the sheet.
export async function share(text) {
  if (!navigator.share) return navigator.clipboard.writeText(text).then(() => "copied")
  try {
    await navigator.share({ text })
    return "shared"
  } catch (error) {
    if (error.name !== "AbortError") throw error
  }
}

const ago = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" })
/// `seen(device.seen)`: when a device or a machine that's off was last on, as its grey line says it, in the largest
/// unit that's whole: "Seen 2 minutes ago". `at` is a time, or milliseconds since 1970.
export function seen(at) {
  const minutes = Math.round((new Date(at) - Date.now()) / 60_000)
  if (minutes > -60) return `Seen ${ago.format(Math.min(minutes, -1), "minute")}`
  if (minutes > -48 * 60) return `Seen ${ago.format(Math.round(minutes / 60), "hour")}`
  return `Seen ${ago.format(Math.round(minutes / (24 * 60)), "day")}`
}
