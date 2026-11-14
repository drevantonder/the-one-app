import { LitElement, unsafeCSS } from "lit"
import base from "./base.css"

const shared = unsafeCSS(base)

/// An element's styles: what every element starts with, then its own `css` (the text of the CSS file beside it),
/// in a layer so that what every element does when hidden, pressed or focused wins over how any one looks.
export const look = (css) => [shared, more(css)]

/// More look, for an element that adds to the look of the one it extends: `[Field.styles, more(css)]`.
export const more = (css) => unsafeCSS(`@layer look{${css}}`)

/// Every element of the kit that's on the page now.
const here = new Set()

/// Draws them all again, when something they all draw from has changed: the pointer, or a part that arrived late.
export const redraw = () => here.forEach((element) => element.requestUpdate())

/// The few words the kit says itself, in English until a page says otherwise with `words`.
export const said = {
  back: "Back",
  cancel: "Cancel",
  clear: "Clear",
  close: "Close",
  file: "Choose a file",
  loading: "Loading",
  more: "More",
  ok: "OK",
  remove: "Remove",
}

/// `words({ back: "Zurück", cancel: "Abbrechen", close: "Schließen" })`: says the kit's own words in the user's
/// language, once, for every element on the page. A word the kit doesn't say throws the words it does.
export function words(theirs) {
  for (const name in theirs) if (!(name in said)) throw new Error(`The kit has no word "${name}". It has: ${Object.keys(said).join(", ")}`)
  Object.assign(said, theirs)
  redraw()
}

/// What every element of the kit is: a custom element drawn in its own shadow root, so an app's children stay the
/// app's (a framework can keep changing them) and an app's styles can't reach in.
export class Element extends LitElement {
  connectedCallback() {
    super.connectedCallback()
    here.add(this)
  }

  disconnectedCallback() {
    super.disconnectedCallback()
    here.delete(this)
  }

  /// Tells the page something happened here, as a native control would: an event that bubbles out of the element.
  emit(type) {
    this.dispatchEvent(new Event(type, { bubbles: true, composed: true }))
  }

  /// Draws again whenever the app changes what's inside the element, for those whose children are what they list:
  /// a select's options, a menu's items, the tabs.
  watchChildren() {
    const seen = new MutationObserver(() => this.requestUpdate())
    seen.observe(this, { childList: true, subtree: true, attributes: true, characterData: true })
  }
}

/// A focusable control inside is what focusing the element focuses.
export const delegatesFocus = { ...LitElement.shadowRootOptions, delegatesFocus: true }

/// The devices whose own pickers are better than ours: a phone or tablet, where a finger is the pointer. It
/// follows the device: a mouse plugged into a tablet is a mouse from then on.
export const touch = matchMedia("(pointer: coarse)")
touch.addEventListener("change", redraw)

// Which way text runs is the page's to say, and it can change its mind: Zag is told again.
new MutationObserver(redraw).observe(document.documentElement, { attributeFilter: ["dir"] })

/// Once the page has first been drawn. What it didn't need for that is loaded then, so the first screen waits for
/// none of it.
export const drawn = new Promise((done) => requestAnimationFrame(() => setTimeout(done)))

/// Whether the user asked their device for less motion: what the kit moves in JavaScript asks this first.
export const reduced = matchMedia("(prefers-reduced-motion: reduce)")

/// One of the kit's three paces (`quick`, `settle`, `travel`) as the timing of an animation of `element`'s:
/// tokens.css says how long each is and how it eases.
export function pace(element, name) {
  const tokens = getComputedStyle(element)
  const time = tokens.getPropertyValue(`--move-${name}`).trim()
  // A time is written in milliseconds or seconds, and an animation counts in milliseconds.
  return { duration: parseFloat(time) * (time.endsWith("ms") ? 1 : 1000), easing: tokens.getPropertyValue(`--ease-${name}`).trim() }
}
