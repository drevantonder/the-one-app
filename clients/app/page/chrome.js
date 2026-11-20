// What the device keeps for itself that the window keeps clear of or follows: Colour Mode, the safe area and the keyboard, and the title bar.
import { invoke, listen, setTheme } from "./host.js"
import { icons, names } from "./apps.js"
import { post } from "./bridge.js"
import { open, own } from "./client.js"
import { byId } from "./util.js"

/// Light or dark, as Colour Mode has it on this device, or as the device is.
const scheme = matchMedia("(prefers-color-scheme: dark)")
export const chosen = () => /** @type {"light" | "dark" | null} */ (localStorage.getItem("theme"))
export const theme = () => chosen() ?? (scheme.matches ? "dark" : "light")
export function dress() {
  document.documentElement.style.colorScheme = chosen() ?? ""
  setTheme(chosen())
  for (const name of ["light", "dark"]) document.documentElement.classList.toggle(name, theme() === name)
  for (const view of open.values()) post(view, { method: "ui/notifications/host-context-changed", params: { theme: theme() } })
}
scheme.addEventListener("change", dress)
/// Colour Mode is the device's: on a computer each app's window has a page of its own, so each is told as it's chosen
/// in any of them, and told what was chosen as it opens.
/** @param {unknown} theme */
export function heardTheme(theme) {
  if (theme === undefined || theme === chosen()) return
  if (theme === "light" || theme === "dark") localStorage.setItem("theme", theme)
  else localStorage.removeItem("theme")
  dress()
}
listen("theme", ({ payload }) => heardTheme(payload))
// Where the window goes from here, a sign-in page, isn't told Colour Mode: the bars follow the device there, as it does.
addEventListener("pagehide", () => setTheme(null))
/// How far in from each edge of the window what the device keeps for itself reaches (an iPhone's status bar, home
/// indicator and notch), which only this page is told: Android's window is already clear of its own.
const safeArea = Object.assign(document.createElement("div"), { id: "safe-area" })
document.body.append(safeArea)
/// What of that an app's frame reaches, as MCP Apps tell an app (`safeAreaInsets`): its screen keeps clear of it,
/// and what scrolls in it passes under it.
/** @param {HTMLIFrameElement} frame */
export function insets(frame) {
  const safe = getComputedStyle(safeArea)
  // Where it's laid out, not where a move has it now: an app that loads as it slides in is told where it ends up.
  const at = { top: frame.offsetTop, right: frame.offsetLeft + frame.offsetWidth, bottom: frame.offsetTop + frame.offsetHeight, left: frame.offsetLeft }
  const reach = (/** @type {string} */ edge, /** @type {number} */ from) => Math.max(0, Math.round(parseFloat(safe.getPropertyValue(`padding-${edge}`)) - from))
  return { top: reach("top", at.top), right: reach("right", innerWidth - at.right), bottom: reach("bottom", innerHeight - at.bottom), left: reach("left", at.left) }
}
/// Each app is told again as the window turns or its frame moves.
export function placed() {
  for (const view of open.values())
    post(view, { method: "ui/notifications/host-context-changed", params: { safeAreaInsets: insets(view.frame), ...titleRow() } })
}
addEventListener("resize", placed)
// The banner moves every frame down as it comes in.
byId("banner").addEventListener("animationend", placed)

/// An iPhone's keyboard covers the window rather than making it smaller, as Android's does: while it's up, the
/// window's apps end above it, so a sheet sits on it and what's typed in can be scrolled to. Pinched in, the window
/// is as it was.
function keyboard() {
  const { height = innerHeight, scale = 1 } = visualViewport ?? {}
  document.body.style.height = scale === 1 && height < innerHeight ? `${height}px` : ""
  placed()
}
visualViewport?.addEventListener("resize", keyboard)
/// A window whose title bar is over the page (`titleBar`), as a Mac's with no title band is: the page keeps its
/// height clear at its top (`--safe-top`), as every app is told, under the window's own controls (a Mac's traffic
/// lights). Held there, it drags the window, and double-clicked, it does what a title bar does (on a Mac, as it's set:
/// zoom). An app's own window has its title there, just after the window's controls: the app's icon and name
/// (`appTitle`).
export const titleBar = byId("title-bar")
/// Where the title row is over an app's page, which its top bar is: where in it the page's own may start, after the
/// window's controls and title, and how far from its end, before the controls there. Nothing where the window
/// has no title bar over its page.
export function titleRow() {
  if (titleBar.hidden) return {}
  const style = getComputedStyle(document.documentElement)
  const px = (/** @type {string} */ name) => parseFloat(style.getPropertyValue(name)) || 0
  const title = byId("title-bar-app")
  const start = title.hidden ? px("--title-bar-start") : title.getBoundingClientRect().right + 12
  return { titleBar: { start: Math.round(start), end: Math.round(px("--title-bar-end") + 16) } }
}
/** @param {import("./host").TitleBar} at */
export function titled({ height, start, end }) {
  document.documentElement.style.setProperty("--title-bar-height", `${height}px`)
  document.documentElement.style.setProperty("--title-bar-start", `${start}px`)
  document.documentElement.style.setProperty("--title-bar-end", `${end}px`)
  document.documentElement.toggleAttribute("data-title-bar", true)
  titleBar.hidden = false
  byId("title-bar-app").hidden = !own
  appTitle()
  // Open apps keep it clear too, as it changes.
  placed()
}
/// The window's title: its app, by its icon and name, once its apps are listed.
export function appTitle() {
  if (!own) return
  const [picture, name] = byId("title-bar-app").children
  const src = icons.get(own.entrypoint)
  if (src) /** @type {HTMLImageElement} */ (picture).src = src
  picture.toggleAttribute("hidden", !src)
  name.textContent = names.get(own.entrypoint) || own.name || ""
  // The page's top bar starts after it, wherever its name ends.
  placed()
}
/// Where it was pressed, until the pointer moves far enough from there to drag the window, as a title bar's own does:
/// a drag that started at the press would take the pointer from the page (a Linux window manager's does), and the
/// second press of a double-click with it.
/** @type {{ x: number, y: number } | undefined} */
let pressedAt
/// A press in the title row on nothing that takes it, on the window's own page (an app's page says its own).
const inTitleRow = (/** @type {MouseEvent} */ event) =>
  !titleBar.hidden &&
  event.button === 0 &&
  event.clientY < titleBar.offsetHeight &&
  !(event.target instanceof Element && event.target.closest("a, button, input, iframe, [role=button], [tabindex]"))
document.addEventListener("mousedown", (event) => {
  if (!inTitleRow(event)) return
  event.preventDefault()
  pressedAt = event.detail === 1 ? { x: event.clientX, y: event.clientY } : undefined
})
document.addEventListener("mousemove", (event) => {
  if (!pressedAt || !(event.buttons & 1) || Math.hypot(event.clientX - pressedAt.x, event.clientY - pressedAt.y) < 4) return
  pressedAt = undefined
  invoke("plugin:window|start_dragging")
})
document.addEventListener("mouseup", (event) => {
  pressedAt = undefined
  if (event.detail === 2 && inTitleRow(event)) invoke("title_bar_double_click")
})
