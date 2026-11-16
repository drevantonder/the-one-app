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
