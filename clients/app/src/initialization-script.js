// What a computer's app does in every page of its window, which Mac's and Linux's web views leave to the app around
// them. It runs in every frame, since keys and clicks go to the frame they're in, and each frame goes back and forward
// through the window's history, which they share.

// A browser window's keys for back and forward: Cmd-[ and Cmd-] on a Mac, Alt-Left and Alt-Right elsewhere.
const mac = navigator.platform.startsWith("Mac")
addEventListener("keydown", (event) => {
  const { key, metaKey, altKey, ctrlKey, shiftKey } = event
  const step = mac
    ? metaKey && !altKey && !ctrlKey && !shiftKey && { "[": -1, "]": 1 }[key]
    : altKey && !metaKey && !ctrlKey && !shiftKey && { ArrowLeft: -1, ArrowRight: 1 }[key]
  if (!step || event.defaultPrevented) return
  event.preventDefault()
  history.go(step)
})

// Escape leaves an app at its first screen, as its Back does. The window says whether it's there, with nothing of
// the app's open over it (`home` in page/client.js), so it's told before the app hears the key and shuts what's open.
// Not while typing, where Escape is the field's.
addEventListener(
  "keydown",
  (event) => {
    if (event.key !== "Escape" || event.repeat || (event.target instanceof Element && event.target.closest("input, textarea, select, [contenteditable]"))) return
    if (!document.querySelector("dialog[open], :popover-open")) top?.postMessage({ method: "notifications/one/escape" }, "*")
  },
  true,
)

// A mouse's back and forward buttons.
addEventListener("mouseup", (event) => {
  const step = { 3: -1, 4: 1 }[event.button]
  if (!step) return
  event.preventDefault()
  history.go(step)
})

// A right-click opens the menu an app gives it, or the web view's own for text: a field's, or what's selected. Never
// the web view's menu for a page (Back, Reload, Open Frame in New Window), which an app's window doesn't have.
addEventListener("contextmenu", (event) => {
  const text = event.target instanceof Element && event.target.closest("input, textarea, [contenteditable]")
  if (!text && getSelection()?.isCollapsed !== false) event.preventDefault()
})

// A trackpad's swipe sideways, which the window takes to go back out of an app at its first screen as a browser's
// swipe between pages goes back (`wheel` in page/client.js): each frame tells it how its wheel moves, in pixels, and
// whether something of its own there scrolls that way (has more to show that way), whose swipe it is then. A pinch
// (Ctrl) isn't one.
addEventListener(
  "wheel",
  (event) => {
    if (event.ctrlKey) return
    const unit = [1, 16, innerWidth][event.deltaMode] ?? 1
    const x = event.deltaX * unit
    const y = event.deltaY * unit
    const scrolls = (/** @type {EventTarget} */ at) => {
      if (!(at instanceof Element)) return false
      const { overflowX, direction } = getComputedStyle(at)
      if (overflowX === "hidden" || overflowX === "clip" || (overflowX === "visible" && at !== document.scrollingElement)) return false
      const room = at.scrollWidth - at.clientWidth
      // A right-to-left element's scroll runs from 0 at its right edge to less than 0.
      const [left, right] = direction === "rtl" ? [-room, 0] : [0, room]
      return room >= 1 && (x < 0 ? at.scrollLeft > left + 0.5 : at.scrollLeft < right - 0.5)
    }
    const own = Math.abs(x) > Math.abs(y) && event.composedPath().some(scrolls)
    top?.postMessage({ method: "notifications/one/wheel", params: { x, y, at: performance.timeOrigin + event.timeStamp, own } }, "*")
  },
  { passive: true },
)

// Whether this frame holds something its person typed and hasn't sent: a field typed in that still holds text. A new
// build of the app, which it starts again on by itself, waits for no window on screen to (`clients/app/src/updater.rs`),
// as the window adds up what its frames say (`editing` in page/client.js). It's looked at again as what's typed may
// have been sent, which an app does by emptying the field.
const typed = new Set()
let editing = false
const kinds = ["checkbox", "radio", "range", "color", "button", "submit", "reset", "file", "hidden", "image"]
const field = (/** @type {EventTarget} */ at) =>
  at instanceof HTMLTextAreaElement || (at instanceof HTMLInputElement && !kinds.includes(at.type)) || (at instanceof HTMLElement && at.isContentEditable)
const holds = (/** @type {any} */ at) => at.isConnected && (at.isContentEditable ? at.textContent : at.value).trim() !== ""
const said = () => {
  for (const at of typed) if (!holds(at)) typed.delete(at)
  if (typed.size > 0 === editing) return
  editing = typed.size > 0
  top?.postMessage({ method: "notifications/one/editing", params: { editing } }, "*")
}
addEventListener(
  "input",
  (event) => {
    const at = event.composedPath()[0]
    if (event.isTrusted && field(at)) typed.add(at)
    said()
  },
  true,
)
for (const type of ["focusout", "submit", "keyup", "click"]) addEventListener(type, () => setTimeout(said), true)
addEventListener("pagehide", () => {
  typed.clear()
  said()
})
