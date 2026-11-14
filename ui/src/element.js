import { LitElement, unsafeCSS } from "lit"
import base from "./base.css"

const shared = unsafeCSS(base)

/// An element's styles: what every element starts with, then its own `css` (the text of the CSS file beside it),
/// in a layer so that what every element does when hidden, pressed or focused wins over how any one looks.
export const look = (css) => [shared, more(css)]

/// More look, for an element that adds to the look of the one it extends: `[Field.styles, more(css)]`.
export const more = (css) => unsafeCSS(`@layer look{${css}}`)

/// What every element of the kit is: a custom element drawn in its own shadow root, so an app's children stay the
/// app's (a framework can keep changing them) and an app's styles can't reach in.
export class Element extends LitElement {
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

/// The devices whose own pickers are better than ours: a phone or tablet, where a finger is the pointer.
export const touch = matchMedia("(pointer: coarse)")
