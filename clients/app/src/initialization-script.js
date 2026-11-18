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
