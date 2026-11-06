// Your pages, as a home screen: each a grid of the widgets you put there and its apps' icons, then its Accounts folder;
// Settings is a page of its items. With `scope`, that page alone (a window whose sidebar picks it; "settings" for
// Settings); with `swipe`, every page side by side then Settings, opened at `scope`, to swipe between (a phone's shell),
// under a pill of dots.
// Touch and hold, or right-click, for what an icon or widget offers: its sizes, its settings, taking it off, and
// arranging the page, where everything jiggles and drags anywhere. Holding something and moving starts arranging
// with it in hand; holding empty space starts it too.
//
// A client's own screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into dist/pages.html,
// which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useLayoutEffect, useRef, useState } from "react"
import { flushSync } from "react-dom"
import { createRoot } from "react-dom/client"
import { AppIcon, Badge, Button, Empty, Icon, Label, Problem, ResponsiveDialog, Select, SelectContent, SelectItem, SelectTrigger, SelectValue, Skeleton, Switch, Tabs, TabsList, TabsTrigger, cn, useWidth, WidgetCard } from "@one/ui"
import { home } from "./lib/model.js"

const { scope: chosen, swipe } = one.params || {}
const SETTINGS = "settings"
const CELLS = { small: [2, 2], medium: [4, 2], large: [4, 4], xl: [8, 4] }
const NAMES = { small: "Small", medium: "Medium", large: "Large", xl: "Extra Large" }
const SHAPES = { small: "square", medium: "rectangle", large: "squares-four", xl: "columns" }
const HOLD = 450
// Whether the shell lifts what a finger holds itself (the iPhone's), and answers its own `contextmenu`.
const lifts = !!window.__oneLifts
const editHomePage = { title: "Edit Home Page", icon: "dots-nine", run: () => edit(true) }

// What the screen is doing: the pages' view, arranging (`editing`), the page shown (`at`, swiping), what a drag has
// reordered or a corner has resized before it's kept (`order`, `sizes`), the widgets whose new options are being drawn
// (`refreshing`), each grid's columns and cell, and the dialogs over it.
// Drawn from a snapshot of it (`snapshot`), so what's drawn is always that and no more.
const S = { editing: false, at: chosen || "user", order: {}, sizes: {}, refreshing: new Set(), geometry: {}, dialog: null, phase: "loading", problem: null, render: () => {}, view: [] }
// What a finger or the mouse is doing: holding something (`held`), or carrying it (`drag`).
let held = null, drag = null, quietUntil = 0, redrawLater = false, flipNext = false
/// Just after a hold or a drag, the tap it ends in isn't a tap.
const quiet = () => Date.now() < quietUntil
const effective = (size, columns) => (CELLS[size][0] > columns ? "large" : size)
const columnsOf = (scope) => S.geometry[scope]?.columns || 4

/// Draws again; nothing moves under a finger, so what changed meanwhile is drawn once it lets go.
function draw() {
  if (drag || held?.ready || held?.resize) return void (redrawLater = true)
  S.render()
}
/// What's drawn: the pages as they are now, and the state of the screen around them.
const snapshot = () => ({
  phase: S.phase, problem: S.problem, editing: S.editing, at: S.at, dialog: S.dialog, refreshing: [...S.refreshing],
  view: S.phase === "ready" ? viewOf() : [],
  attention: S.phase === "ready" && (home.data.state?.settings_items || []).some((i) => i.badge),
})
const positions = () => (swipe ? [...home.pages().map((p) => p.scope), SETTINGS] : [chosen || "user"])
const titleOf = (scope) => (scope === SETTINGS ? "Settings" : home.page(scope)?.title || "")

// MARK: What each page shows

/// An icon on the grid: `key` and `kept` say what it is to arranging (as the page keeps it).
const tile = (slot, extra) => ({ slot, kind: "app", ...extra })

/// What a page's widgets act through: their app's tools, and their links.
const widgetHost = (p) => ({
  async run(app, tool, input) {
    try {
      await home.call(app, tool, input)
    } catch (err) {
      return one.plain(err).title
    }
    await home.loadWidgets([p.scope])
    draw()
  },
  follow(link, w) {
    if (S.editing || quiet()) return
    if (typeof link === "string") return void (/^https?:/i.test(link) && one.link(link))
    if (link?.screen) one.open(link.screen, link.params || {}, w.app, { scope: p.scope })
  },
})

/// A widget on a page: where it opens (its own link, else its app), and what its menu offers.
function widgetItem(w, kept, p) {
  const a = home.app(w.app), key = `widget:${kept.id}`
  const size = S.sizes[key] || w.size || "medium"
  const name = w.title || home.label(a) || w.widget
  return {
    slot: key, key, kind: "widget", w: { ...w, title: name }, kept, size, name, host: widgetHost(p), p,
    open: !home.tree(w, size)?.link && a && home.hasScreens(a) ? home.open(a, p) : null,
    menu: (e) => home.menu(widgetActions(w, kept, p, effective(size, columnsOf(p.scope)), columnsOf(p.scope)), e),
  }
}

/// A widget still to come (or that couldn't be read): a grey card where it goes, at its size, to move or remove.
function pendingItem(kept, p) {
  const key = `widget:${kept.id}`
  const size = CELLS[S.sizes[key] || kept.size] ? S.sizes[key] || kept.size : "medium"
  const menu = home.canArrange(p.scope) ? (e) => home.menu([{ ...editHomePage, section: true }, { title: "Remove Widget", icon: "minus-circle", danger: true, section: true, run: () => remove(p, { key }) }], e) : undefined
  return { slot: key, key, kind: "pending", kept, size, name: home.label(home.app(kept.app)) || kept.widget, menu }
}

/// A page's items: its widgets and apps in its person's order, then its Accounts folder.
function pageView(p) {
  const arrange = home.canArrange(p.scope)
  // Empty only with nothing on it at all: Accounts' folder, on the first page, is something (Andre).
  const first = p.scope === home.pages()[0]?.scope
  const empty = !first && !home.appsOn(p).length && !home.widgetsOn(p).length && !home.waiting(p)
  let list = home.items(p).map((item) => {
    if (item.pending) return pendingItem(item.kept, p)
    if (item.widget) return widgetItem(item.widget, item.kept, p)
    const a = item.app, page = home.page(p.scope) || p
    return tile(`app:${a.name}`, {
      key: `app:${a.name}`, kept: { app: a.name }, name: home.label(a), icon: a, badge: home.badge(a.name, p), open: home.open(a, p),
      menu: (e) => home.menu([...(arrange ? [{ ...editHomePage, section: true }] : []), ...addActions(page, a), ...home.actions(a, page)], e),
    })
  })
  const order = S.order[p.scope]
  if (order) list = order.map((slot) => list.find((i) => i.slot === slot)).filter(Boolean).concat(list.filter((i) => !order.includes(i.slot)))
  // Accounts once, on the first page: every page's, listed together, opened as a page.
  if (first) {
    const apps = [...new Map(home.pages().flatMap(home.accountsOn).map((a) => [a.name, a])).values()]
    list.push({ slot: "accounts", kind: "accounts", name: "Accounts", apps, open: () => one.open("accounts"), menu: arrange ? (e) => home.menu([editHomePage], e) : undefined })
  }
  return {
    scope: p.scope, p, title: p.title, subtitle: p.subtitle, arrange, empty, store: !!home.store(), list,
    said: empty ? (arrange ? (p.scope === "user" ? "No apps yet." : `No apps on ${p.title} yet.`) : `When its admins add apps to ${p.title}, they show up here.`) : "",
  }
}

/// Settings, as a page: its items' icons.
const settingsView = () => ({
  scope: SETTINGS, title: "Settings", subtitle: home.network()?.name || "",
  list: (home.data.state.settings_items || []).map((item) => tile(`settings:${item.name}`, {
    name: item.title, icon: { name: item.name, title: item.title, icon: item.icon, color: item.color }, badge: item.badge || 0,
    // An item with an app (the App Store) opens that app; one with a url (Passkeys), the browser.
    open: () => (item.url ? one.link(item.url) : item.app ? one.open("main", {}, item.app) : one.open(item.name)),
  })),
})

function viewOf() {
  return positions().map((scope) => (scope === SETTINGS ? settingsView() : home.page(scope) ? pageView(home.page(scope)) : { scope, missing: true, list: [] }))
}

/// Where each item goes: first place it fits, row by row, widgets starting on an even column; a widget wider than
/// there's room for is large.
function pack(list, columns) {
  const taken = []
  return list.map((item) => {
    const wide = item.kind === "widget" || item.kind === "pending"
    const size = wide ? effective(item.size, columns) : null
    const [w, h] = wide ? CELLS[size] : [1, 1]
    for (let y = 0; ; y++) {
      for (let x = 0; x <= columns - w; x += w > 1 ? 2 : 1) {
        let free = true
        for (let row = y; row < y + h && free; row++) for (let col = x; col < x + w; col++) if (taken[row]?.[col]) { free = false; break }
        if (!free) continue
        for (let row = y; row < y + h; row++) for (let col = x; col < x + w; col++) (taken[row] ||= [])[col] = true
        return { item, size, col: x + 1, row: y + 1, w, h }
      }
    }
  })
}

// MARK: Changing a page

const keptOf = (p) => home.items(p).map((i) => (i.app ? { app: i.app.name } : i.kept))
const hiddenOf = (p) => home.data.state.arrangements?.[p.scope]?.hidden || []
const keyOf = (kept) => (kept.widget ? `widget:${kept.id}` : `app:${kept.app}`)
let keeping
/// What was drawn last, kept by the shell, so the next time the pages open they're drawn at once, then brought up to date.
const keep = () => {
  clearTimeout(keeping)
  keeping = setTimeout(() => one.cache("pages", { state: home.data.state, apps: home.data.apps, widgets: home.data.widgets, badges: home.data.badges }).catch(() => {}), 500)
}

/// Keeps a page as `items` (each as the page keeps it), with `hidden` the apps taken off it, drawn at once; if it
/// can't be kept, the page goes back to how it was. With `reload`, its widgets are read again after.
async function save(p, items, hidden = hiddenOf(p), { reload = false } = {}) {
  const state = home.data.state, before = state.arrangements
  const was = before?.[p.scope] || {}
  state.arrangements = { ...(before || {}), [p.scope]: { ...was, items, hidden } }
  const after = state.arrangements
  draw()
  try {
    await one.api("POST", "/v1/arrange", { page: p.scope, items, hidden, ...(was.pages ? { pages: was.pages } : {}) })
    if (reload) await home.loadWidgets([p.scope]), draw()
    keep()
  } catch (err) {
    // Unless something newer was drawn meanwhile.
    if (home.data.state.arrangements === after) home.data.state.arrangements = before, draw()
    home.failed(err)
  }
}

const resize = (p, kept, size) => save(p, keptOf(p).map((k) => (k.widget && k.id === kept.id ? { ...k, size } : k)))

/// Takes a widget off a page, or an app's icon: the app stays installed there, under Edit's Taken Off This Page.
function remove(p, item) {
  one.tap("medium")
  const kept = keptOf(p).filter((k) => keyOf(k) !== item.key)
  const app = item.key.startsWith("app:") && item.key.slice(4)
  save(p, kept, app && !hiddenOf(p).includes(app) ? [...hiddenOf(p), app] : hiddenOf(p))
}

/// What touching and holding a widget offers, as the Home Screen's: its sizes in a row, then Edit Widget (its
/// options), Edit Home Page, Copy What Went Wrong if it broke, and Remove Widget.
function widgetActions(w, kept, p, size, columns) {
  // On a page they can't arrange, only what's there: no sizes, options or removing.
  const arrange = home.canArrange(p.scope)
  const sizes = arrange ? home.supported(w).filter((s) => CELLS[s][0] <= columns) : []
  const out = sizes.length > 1 ? sizes.map((s) => ({ title: NAMES[s], icon: SHAPES[s], row: true, checked: s === size, run: () => s !== size && resize(p, kept, s) })) : []
  if (arrange && (w.options || []).length) out.push({ title: "Edit Widget", icon: "info", section: out.length > 0, run: () => (S.dialog = { kind: "options", w, kept, p }, S.render()) })
  if (arrange) out.push({ ...editHomePage, section: !out.length || out.at(-1).row })
  if (w.error) out.push({ title: "Copy What Went Wrong", icon: "copy", run: () => one.copy(w.error) })
  if (arrange) out.push({ title: "Remove Widget", icon: "minus-circle", danger: true, section: true, run: () => remove(p, { key: `widget:${kept.id}` }) })
  return out
}

/// Adding one of a page's widgets, where it offers one (only `a`'s, for an app's menu).
const addActions = (p, a) => (home.canArrange(p.scope) && home.widgetsOn(p).some((w) => !a || w.app === a.name) ? [{ title: "Add Widget…", icon: "plus-square", run: () => (S.dialog = { kind: "add", p, only: a, columns: columnsOf(p.scope) }, S.render()) }] : [])

/// While arranging, Edit's menu: Add Widget…, and the apps taken off the page, to put back.
function editMenu(p, at) {
  const hidden = hiddenOf(p).map(home.app).filter((a) => a && home.appsOn(p).includes(a))
  const out = addActions(p)
  hidden.forEach((a, i) => out.push({ title: `Put Back ${home.label(a)}`, icon: "plus-circle", section: i === 0 && out.length > 0, run: () => save(p, [...keptOf(p), { app: a.name }], hiddenOf(p).filter((n) => n !== a.name)) }))
  // With nothing of its own to add, where apps come from.
  if (!out.length) out.push(home.store() ? { title: "App Store", icon: "squares-four", run: home.openStore } : { title: "Nothing to Add", run: () => {} })
  home.menu(out, at)
}

/// Edit Pages…: the groups' pages, dragged into an order of your own, each switched off to hide it; yours first and
/// everyone's last stay. Kept with how you arrange your own page, so it follows you to every device.
function editPages() {
  const groups = home.allPages().filter((p) => p.scope.startsWith("group:"))
  if (!groups.length) return void one.alert({ title: "No Groups’ Pages", text: "When you’re in a group, its page shows up here, to move or hide." })
  S.dialog = { kind: "pages", groups }
  S.render()
}

function keepPages(pages) {
  const state = home.data.state, before = state.arrangements, mine = before?.user
  const items = mine?.items || keptOf(home.page("user") || { scope: "user" })
  state.arrangements = { ...(before || {}), user: { ...(mine || {}), items, hidden: mine?.hidden || [], pages } }
  const after = state.arrangements
  draw()
  one.api("POST", "/v1/arrange", { page: "user", items, hidden: mine?.hidden || [], pages }).then(keep, (err) => {
    if (home.data.state.arrangements === after) home.data.state.arrangements = before, draw()
    home.failed(err)
  })
}

// MARK: Arranging

/// Starts or ends arranging.
function edit(on) {
  if (S.editing === on) return
  S.editing = on
  if (on) one.tap("medium")
  S.render()
}

/// The element of an item on a page, and the item.
function itemAt(target) {
  const el = target.closest?.("[data-cell]")
  const scope = el?.closest("[data-page]")?.dataset.page
  const item = el && S.view.find((v) => v.scope === scope)?.list.find((i) => i.slot === el.dataset.cell)
  return item && { item, el, scope, p: home.page(scope) }
}

const cellsOf = (scope) => [...document.querySelectorAll(`[data-page="${CSS.escape(scope)}"] [data-cell]`)]

/// Resizing a widget from its corner: the size whose cells come nearest the finger, as it moves; kept when it lifts.
function resizing(e, item, p, el) {
  e.preventDefault()
  e.stopPropagation()
  const start = item.size, { cell, gap, columns } = S.geometry[p.scope]
  const { left, top } = el.getBoundingClientRect()
  const sizes = home.supported(item.w).filter((s) => CELLS[s][0] <= columns)
  held = { resize: true }
  const move = (m) => {
    const w = (m.clientX - left) / (cell + gap), h = (m.clientY - top) / (cell + gap)
    const near = sizes.reduce((a, b) => (Math.hypot(CELLS[a][0] - w, CELLS[a][1] - h) <= Math.hypot(CELLS[b][0] - w, CELLS[b][1] - h) ? a : b))
    if (near !== (S.sizes[item.key] || item.size)) { S.sizes[item.key] = near; flipNext = true; S.render(true); one.tap() }
  }
  const up = () => {
    held = null
    removeEventListener("pointermove", move)
    removeEventListener("pointerup", up)
    removeEventListener("pointercancel", up)
    const size = S.sizes[item.key]
    delete S.sizes[item.key]
    if (size && size !== start) resize(p, item.kept, size)
    else S.render()
  }
  addEventListener("pointermove", move)
  addEventListener("pointerup", up)
  addEventListener("pointercancel", up)
}

/// The hold's moment: what's held swells (empty space starts arranging); then moving carries it, letting go opens its menu.
function lift() {
  if (!held || held.ready) return
  held.ready = true
  quietUntil = Date.now() + 600
  if (held.page) {
    held = null
    return edit(true)
  }
  one.tap("medium")
  held.el.style.scale = "1.06"
}

// A finger's moves and its lifting, from its pointer events; after a long press Android's web view cancels the
// pointer but goes on sending touch events (`touched`), which carry it from there.
let touched = false
const mine = (e) => (drag && e.pointerId === drag.id) || (held && !held.resize && e.pointerId === held.id)

function moved(x, y) {
  if (drag) return void dragTo(x, y)
  const far = Math.hypot(x - held.x, y - held.y)
  if (!held.ready) {
    // Moved before the hold: a scroll or a swipe, not a hold.
    if (far > 8) clearTimeout(held.timer), (held = null)
    return
  }
  if (held.page || far < (S.editing ? 4 : 10) || !home.canArrange(held.scope)) return
  const { item, el, scope, p, id, x: x0, y: y0 } = held
  held = null
  el.style.scale = ""
  edit(true)
  begin(p, scope, item.slot, x0, y0, id)
  dragTo(x, y)
}

function lifted(target) {
  if (drag) return void drop()
  clearTimeout(held.timer)
  const h = held
  held = null
  if (h.el) h.el.style.scale = ""
  // Arranging, a tap on empty space is Done.
  if (S.editing && h.page && !target.closest?.("[data-cell], [role=dialog]")) return edit(false)
  // A hold let go where it was: its menu.
  if (!S.editing && h.ready && h.item?.menu) {
    quietUntil = Date.now() + 400
    h.item.menu(h.el)
  }
}

/// Picks up an item on the page now drawn: a picture of it follows the finger, its place kept open.
function begin(p, scope, slot, x, y, id) {
  const el = cellsOf(scope).find((c) => c.dataset.cell === slot)
  if (!el) return
  const r = el.getBoundingClientRect()
  const ghost = el.cloneNode(true)
  ghost.removeAttribute("data-cell")
  ghost.className = "pointer-events-none fixed left-0 top-0 z-20 m-0 grid scale-[1.07] drop-shadow-xl transition-[scale] duration-150 [&>[data-ctl]]:hidden"
  Object.assign(ghost.style, { width: `${r.width}px`, height: `${r.height}px`, gridColumn: "", gridRow: "", translate: "", animation: "none" })
  document.body.append(ghost)
  el.style.visibility = "hidden"
  el.style.animation = "none"
  const view = S.view.find((v) => v.scope === scope)
  drag = { p, scope, slot, el, ghost, id, dx: x - r.left, dy: y - r.top, before: view.list.map((i) => i.slot) }
  one.tap("medium")
  dragTo(x, y)
}

/// The picture under the finger; over another item it takes that one's place, the rest moving aside; past the last,
/// it goes at the end. The Accounts folder stays last.
function dragTo(x, y) {
  const d = drag
  d.ghost.style.transform = `translate(${x - d.dx}px, ${y - d.dy}px)`
  const order = S.view.find((v) => v.scope === d.scope).list.map((i) => i.slot)
  const movable = order.filter((s) => s !== "accounts")
  let over = null, bottom = 0
  for (const el of cellsOf(d.scope)) {
    const r = el.getBoundingClientRect(), slot = el.dataset.cell
    bottom = Math.max(bottom, r.bottom)
    if (slot !== d.slot && slot !== "accounts" && x >= r.left && x <= r.right && y >= r.top && y <= r.bottom) over = slot
  }
  const from = order.indexOf(d.slot)
  let to = -1
  if (over && over !== d.over) to = order.indexOf(over)
  else if (!over && y > bottom && d.slot !== movable.at(-1)) to = order.indexOf(movable.at(-1))
  d.over = over
  if (to < 0 || to === from) return
  order.splice(from, 1)
  order.splice(to, 0, d.slot)
  S.order[d.scope] = order
  flipNext = true
  S.render(true)
  one.tap()
}

/// Lets go: the picture settles into the place kept for it, and the page keeps its new order.
function drop() {
  const d = drag
  drag = null
  quietUntil = Date.now() + 300
  const r = d.el.getBoundingClientRect()
  d.ghost.style.transition = "transform 0.22s ease, scale 0.22s ease"
  d.ghost.style.scale = "1"
  d.ghost.style.transform = `translate(${r.left}px, ${r.top}px)`
  setTimeout(() => {
    d.ghost.remove()
    d.el.style.visibility = ""
    d.el.style.animation = ""
    const view = S.view.find((v) => v.scope === d.scope)
    const order = view.list.filter((i) => i.key)
    const changed = order.map((i) => i.key).join(" ") !== d.before.filter((s) => s !== "accounts").join(" ")
    delete S.order[d.scope]
    if (changed) save(d.p, order.map((i) => i.kept))
    else S.render()
    redrawLater = false
  }, 220)
}

// The iPhone's shell: a finger that moves what it lifted carries it, the menu gone, as on the Home Screen.
window.__oneDrag = (phase, x, y) => {
  if (phase === "start" && !drag) {
    const el = document.querySelector("[data-one-lifted]") || document.elementFromPoint(x, y)
    const found = el && itemAt(el)
    if (!found?.item.key || !found.p || !home.canArrange(found.p.scope)) return false
    el.style?.removeProperty("visibility")
    el.removeAttribute?.("data-one-lifted")
    edit(true)
    const r = found.el.getBoundingClientRect()
    begin(found.p, found.scope, found.item.slot, Math.min(Math.max(x, r.left), r.right), Math.min(Math.max(y, r.top), r.bottom), "shell")
    return true
  }
  if (drag?.id !== "shell") return false
  if (phase === "move") dragTo(x, y)
  else drop()
  return true
}

function pointerDown(e) {
  if (e.button !== 0 || drag || held || e.target.closest("[data-ctl]")) return
  const scope = e.target.closest("[data-page]")?.dataset.page
  if (!scope || scope === SETTINGS || !home.page(scope)) return
  const found = itemAt(e.target)
  if (found && !found.item.key && !found.item.menu) return
  // On a page they can't arrange, a hold only opens what's held's menu.
  if (!home.canArrange(scope) && !found?.item.menu) return
  // Arranging, an item moves as soon as it's moved.
  if (S.editing) {
    if (found?.item.key) held = { ...found, x: e.clientX, y: e.clientY, id: e.pointerId, ready: true }
    else held = { page: true, scope, x: e.clientX, y: e.clientY, id: e.pointerId }
    return
  }
  // A finger on an icon or widget where the shell lifts it is the shell's to hold.
  if (found && lifts && e.pointerType === "touch") return
  // Not on a widget's own controls.
  if (found && e.target.closest("[data-slot=widget] :is(button, input, label, a, select)")) return
  held = { ...found, scope, page: !found, x: e.clientX, y: e.clientY, id: e.pointerId, touch: e.pointerType !== "mouse" }
  held.timer = setTimeout(() => lift(), HOLD)
}

// A right-click (or a long press the browser calls one) opens an item's menu, or Edit Home Page on empty space.
// (Set as a property, not a listener: the iPhone's shell lifts whatever listens for it.)
function contextMenu(e) {
  if (!e.target.closest("[data-page]") || e.target.closest("[data-ctl]")) return
  e.preventDefault()
  // A hold of the page's own is under way: it opens the menu when the finger lifts.
  if (held && held.touch) return void lift()
  if (S.editing || drag) return
  const found = itemAt(e.target)
  if (found?.item.menu) {
    one.tap("medium")
    return found.item.menu(e)
  }
  const scope = e.target.closest("[data-page]")?.dataset.page
  if (scope && scope !== SETTINGS && home.page(scope)) home.menu([...addActions(home.page(scope)), ...(home.canArrange(scope) ? [editHomePage] : [])], e)
}

// MARK: Drawing

/// An icon and its name; badged with what needs attention.
function Tile({ name, icon, size, badge, onClick, children }) {
  return (
    <button type="button" onClick={onClick} className="flex min-w-0 flex-col items-center gap-1.5 pt-0.5 text-foreground outline-none [-webkit-tap-highlight-color:transparent] focus-visible:[&>span:first-child]:ring-[3px] focus-visible:[&>span:first-child]:ring-ring/50 active:[&>span:first-child]:opacity-70">
      <span className="relative block rounded-[22%] leading-[0]">
        {children || <AppIcon app={icon} size={size} />}
        {badge > 0 && <Badge variant="destructive" className="absolute -right-2 -top-2 h-6 min-w-6 px-1.5 text-sm font-semibold tabular-nums dark:bg-destructive">{badge > 99 ? "99+" : badge}</Badge>}
      </span>
      <span className="line-clamp-2 max-w-full text-center text-[13px] leading-tight">{name}</span>
    </button>
  )
}

/// A folder, as iOS draws one: a frosted, see-through rounded square, apart from the page and from app icons, with its
/// apps' icons small in a 3×3 grid from the top left (nine at most); its empty places are just empty.
function Folder({ apps, size }) {
  const mini = (size * 0.64) / 3
  return (
    <span
      className="grid content-start bg-linear-to-b from-zinc-400/15 to-zinc-500/30 shadow-[0_1px_2px_rgb(0_0_0/0.08),inset_0_0_0_0.5px_rgb(0_0_0/0.08),inset_0_1px_0_rgb(255_255_255/0.5)] backdrop-blur-xl backdrop-saturate-150 dark:from-zinc-300/35 dark:to-zinc-400/25 dark:shadow-[0_1px_2px_rgb(0_0_0/0.4),inset_0_0_0_0.5px_rgb(255_255_255/0.18),inset_0_1px_0_rgb(255_255_255/0.12)]"
      style={{ width: size, height: size, borderRadius: size * 0.225, gridTemplateColumns: `repeat(3, ${mini}px)`, gridAutoRows: `${mini}px`, gap: size * 0.05, padding: size * 0.13 }}
    >
      {apps.slice(0, 9).map((a) => <AppIcon key={a.name} app={a} size={mini} />)}
    </span>
  )
}

/// One place on a grid: its item, and while arranging its − badge, and a widget's corner to resize it.
function Cell({ page, at, editing, refreshing, nth, icon }) {
  const { item, size, col, row, w, h } = at
  const el = useRef(null)
  const wide = item.kind === "widget" || item.kind === "pending"
  const arrange = editing && !!item.key
  const resizable = arrange && item.kind === "widget" && home.supported(item.w).length > 1
  let body
  if (item.kind === "widget") {
    body = (
      <WidgetCard
        w={item.w}
        host={item.host}
        tree={home.tree(item.w, size)}
        className={cn("h-full", item.open && "cursor-pointer active:opacity-80", refreshing && "opacity-50 transition-opacity")}
        onClick={item.open ? (e) => !S.editing && !quiet() && !e.defaultPrevented && !e.target.closest("button, input, label, a") && item.open() : undefined}
      />
    )
  } else if (item.kind === "pending") {
    body = <div aria-hidden="true" className="h-full animate-pulse rounded-3xl bg-muted motion-reduce:animate-none" style={{ zoom: "var(--unzoom, 1)" }} />
  } else {
    body = (
      <Tile name={item.name} icon={item.icon} size={icon} badge={item.badge} onClick={() => !S.editing && !quiet() && (one.tap(), item.open())}>
        {item.kind === "accounts" && <Folder apps={item.apps} size={icon} />}
      </Tile>
    )
  }
  return (
    <div
      ref={el}
      data-cell={item.slot}
      data-one-held={lifts && !editing && !!item.menu ? "" : undefined}
      className={cn("relative grid min-h-0 min-w-0 transition-[scale] duration-200", editing && "cursor-grab touch-none motion-safe:animate-[jiggle_0.28s_ease-in-out_infinite] [&>*]:pointer-events-none [&>[data-ctl]]:pointer-events-auto")}
      style={{
        gridColumn: `${col} / span ${w}`,
        gridRow: `${row} / span ${h}`,
        "--wobble": wide ? "0.5deg" : "1.6deg",
        ...(editing ? { animationDuration: nth % 3 === 0 ? "0.26s" : nth % 2 === 0 ? "0.31s" : "0.28s", animationDelay: nth % 2 === 0 ? "-0.12s" : "0s" } : {}),
      }}
    >
      {body}
      {arrange && (
        <Button
          data-ctl
          variant="outline"
          size="icon-xs"
          aria-label={`Remove ${item.name}`}
          className="absolute -top-2 z-10 size-6 rounded-full shadow-md"
          style={{ left: wide ? -8 : `calc(50% - ${icon / 2}px - 8px)` }}
          onClick={(e) => { e.stopPropagation(); remove(page.p, item) }}
        >
          <Icon name="minus" />
        </Button>
      )}
      {resizable && (
        <button
          data-ctl
          type="button"
          aria-label="Resize"
          className="absolute -bottom-1.5 -right-1.5 z-10 size-[30px] cursor-nwse-resize touch-none"
          onPointerDown={(e) => resizing(e, item, page.p, el.current)}
        >
          <span className="absolute bottom-1.5 right-1.5 size-3.5 rounded-br-[10px] border-b-[3px] border-r-[3px] border-foreground/55" />
        </button>
      )}
    </div>
  )
}

/// How many columns a grid of `width` has: four across a phone's width, more where there's room, each cell at least
/// 92 wide. Widgets are 2, 4 or 8 columns wide, so the columns are a multiple of 4 (a strip of 2 beside a row of them
/// is a hole), unless that makes the cells larger than 132: then the most even ones.
function columnsFor(width, gap) {
  const cellOf = (columns) => (width - gap * (columns - 1)) / columns
  let columns = 4
  while (cellOf(columns + 2) >= 92) columns += 2
  const four = columns - (columns % 4)
  return cellOf(four) <= 132 ? four : columns
}

/// A grid as a home screen's: cells as many as the room has, their size filling it, four across a phone's width; each
/// item in the first place it fits.
function Grid({ page, editing, refreshing }) {
  const [ref, width] = useWidth()
  const narrow = width < 500
  const gap = narrow ? 14 : 18
  const columns = narrow ? 4 : columnsFor(width, gap)
  const cell = (width - gap * (columns - 1)) / columns
  // An icon and its name fit a cell, however large the text is.
  const icon = Math.round(Math.max(32, Math.min(68, cell - 24)))
  const placed = width ? pack(page.list, columns) : []
  useLayoutEffect(() => { S.geometry[page.scope] = { columns, cell, gap } })
  // Each item glides from where it was to where it goes, once what moves them has said so.
  const was = useRef(new Map())
  useLayoutEffect(() => {
    const next = new Map()
    for (const el of ref.current.children) {
      const r = el.getBoundingClientRect(), before = was.current.get(el.dataset.cell)
      next.set(el.dataset.cell, r)
      if (!flipNext || !before) continue
      const dx = before.left - r.left, dy = before.top - r.top
      if (!dx && !dy) continue
      el.style.transition = "none"
      el.style.translate = `${dx}px ${dy}px`
      el.getBoundingClientRect()
      el.style.transition = "translate 0.25s ease, scale 0.2s"
      el.style.translate = ""
    }
    was.current = next
    flipNext = false
  })
  return (
    <div ref={ref} className="relative grid" style={{ gap, gridAutoRows: cell > 0 ? cell : undefined, gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`, "--icon": `${icon}px` }}>
      {placed.map((at, nth) => <Cell key={at.item.slot} page={page} at={at} editing={editing} nth={nth} icon={icon} refreshing={refreshing.includes(at.item.kept?.id)} />)}
    </div>
  )
}

/// A page: its title (alone in a window), then one grid, as a home screen's.
function Page({ page, editing, refreshing, bottom }) {
  const { scope, title, subtitle, arrange } = page
  return (
    <section
      data-page={scope}
      className={cn("h-full min-w-0 flex-[0_0_100%] snap-start overflow-y-auto px-5 pt-5 [scroll-snap-stop:always]", editing && swipe && "pt-[calc(60px+env(safe-area-inset-top))]")}
      style={{ paddingBottom: bottom }}
    >
      {!swipe && (
        <div className="flex items-start gap-3 px-1 pb-5">
          <div className="min-w-0 flex-1">
            <h1 className="text-3xl font-bold tracking-tight">{title}</h1>
            {subtitle && <p className="text-sm text-muted-foreground">{subtitle}</p>}
          </div>
          {arrange && (
            <div data-ctl className="mt-1.5 flex shrink-0 gap-2">
              {editing && <Button variant="outline" size="sm" onClick={(e) => editMenu(home.page(scope), e.currentTarget)}>Edit</Button>}
              <Button variant={editing ? "default" : "outline"} size="sm" onClick={() => edit(!editing)}>{editing ? "Done" : "Edit Home Page"}</Button>
            </div>
          )}
        </div>
      )}
      {page.missing ? (
        <Empty>This device can't open this page. Sign out and in again to choose it.</Empty>
      ) : (
        <>
          {page.empty && <Empty action={page.arrange && page.store ? { label: "Open the App Store", run: home.openStore } : undefined}>{page.said}</Empty>}
          <Grid page={page} editing={editing && !!arrange} refreshing={refreshing} />
        </>
      )}
    </section>
  )
}

/// Under the pages: the one shown's name, then a dot for each, with a cog for Settings. Slide along it to change page.
function Dots({ scopes, at, strip, attention }) {
  const [sliding, setSliding] = useState(false)
  const go = (scope, now = false) => {
    const el = strip.current, i = scopes.indexOf(scope)
    if (now) {
      S.at = scope
      el.scrollTo({ left: i * el.clientWidth, behavior: "instant" })
      S.render()
    } else el.scrollTo({ left: i * el.clientWidth, behavior: "smooth" })
  }
  // A finger sliding along the dots changes page, one for every 40 points, as an iPhone's page dots do.
  const slide = (e) => {
    const capsule = e.currentTarget, x0 = e.clientX, start = scopes.indexOf(S.at)
    let moving = false
    const move = (m) => {
      const dx = m.clientX - x0
      if (!moving && Math.abs(dx) < 12) return
      if (!moving) moving = true, setSliding(true), capsule.setPointerCapture?.(e.pointerId)
      const i = Math.min(scopes.length - 1, Math.max(0, start + Math.trunc(dx / 40)))
      if (scopes[i] !== S.at) { go(scopes[i], true); one.tap() }
    }
    const up = () => {
      capsule.removeEventListener("pointermove", move)
      capsule.removeEventListener("pointerup", up)
      capsule.removeEventListener("pointercancel", up)
      if (moving) quietUntil = Date.now() + 300, setSliding(false)
    }
    capsule.addEventListener("pointermove", move)
    capsule.addEventListener("pointerup", up)
    capsule.addEventListener("pointercancel", up)
  }
  const page = scopes.indexOf(at) + 1
  return (
    <nav data-ctl className="pointer-events-none fixed inset-x-0 bottom-[calc(10px+env(safe-area-inset-bottom))] flex flex-col items-center gap-1.5">
      <div className="max-w-[70%] text-center text-[13px] font-semibold text-muted-foreground">{titleOf(at)}</div>
      <div onPointerDown={slide} className={cn("pointer-events-auto flex h-11 touch-none items-center rounded-full border bg-background/90 shadow-lg backdrop-blur-xl transition-transform", sliding && "scale-[1.08]")}>
        <button
          type="button"
          aria-label={`${titleOf(at)}, page ${page} of ${scopes.length}`}
          className="flex h-11 items-center gap-1.5 pl-[18px] pr-1.5"
          onClick={(e) => !quiet() && home.menu([
            ...scopes.map((s) => ({ title: titleOf(s), checked: s === at, run: () => go(s) })),
            { title: "Edit Pages…", icon: "list-bullets", section: true, run: editPages },
            ...(at !== SETTINGS && home.canArrange(at) ? [editHomePage] : []),
          ], e.currentTarget.parentElement)}
        >
          {scopes.slice(0, -1).map((s) => <i key={s} className={cn("block h-[7px] rounded-full bg-muted-foreground/40 transition-[width]", s === at ? "w-4 bg-foreground" : "w-[7px]")} />)}
        </button>
        <button type="button" aria-label="Settings" className={cn("relative flex h-11 items-center pl-1.5 pr-[18px] text-lg", at === SETTINGS ? "text-foreground" : "text-muted-foreground")} onClick={() => !quiet() && go(SETTINGS)}>
          <Icon name="gear-six-fill" className="size-[18px]" />
          {attention && <span className="absolute right-3.5 top-2.5 size-[7px] rounded-full bg-destructive" />}
        </button>
      </div>
    </nav>
  )
}

// MARK: Dialogs

/// A widget's options: a switch or a choice each, kept with this copy of it on the page, once it's closed.
function Options({ w, kept, p, close }) {
  const value = (o, changed) => (o.name in changed ? changed[o.name] : kept.options && o.name in kept.options ? kept.options[o.name] : o.default ?? null)
  const [changed, setChanged] = useState({})
  const set = (o, v) => {
    one.tap()
    setChanged((c) => ({ ...c, [o.name]: v }))
  }
  function done() {
    close()
    const options = { ...(kept.options || {}), ...changed }
    for (const o of w.options || []) if (JSON.stringify(options[o.name] ?? null) === JSON.stringify(o.default ?? null)) delete options[o.name]
    const next = Object.keys(options).length ? options : undefined
    if (JSON.stringify(next) === JSON.stringify(kept.options)) return
    // Dimmed until it's drawn with them.
    S.refreshing.add(kept.id)
    save(p, keptOf(p).map((k) => (k.widget && k.id === kept.id ? { ...k, options: next } : k)), hiddenOf(p), { reload: true }).finally(() => { S.refreshing.delete(kept.id); draw() })
  }
  return (
    <ResponsiveDialog open onOpenChange={(open) => !open && done()} title={w.title || w.widget} action={<Button size="sm" onClick={done}>Done</Button>}>
      <div className="flex flex-col divide-y overflow-y-auto">
        {(w.options || []).map((o, i) => (
          <div key={o.name} className="flex min-h-12 items-center justify-between gap-3 px-4 py-2">
            <Label htmlFor={`opt-${i}`}>{o.title}</Label>
            {o.choices ? (
              <Select value={String(Math.max(0, o.choices.findIndex((c) => JSON.stringify(c.value) === JSON.stringify(value(o, changed)))))} onValueChange={(j) => set(o, o.choices[j].value)}>
                <SelectTrigger id={`opt-${i}`} className="max-w-[60%]"><SelectValue /></SelectTrigger>
                <SelectContent>{o.choices.map((c, j) => <SelectItem key={j} value={String(j)}>{c.title}</SelectItem>)}</SelectContent>
              </Select>
            ) : (
              <Switch id={`opt-${i}`} checked={value(o, changed) === true} onCheckedChange={(on) => set(o, on)} />
            )}
          </div>
        ))}
      </div>
    </ResponsiveDialog>
  )
}

/// Add Widget: each widget once, as it would look on the page at the size chosen, with Add Widget, which puts it at the
/// page's end.
function AddWidget({ p, only, columns, close }) {
  const widgets = home.widgetsOn(p).filter((w) => !only || w.app === only.name)
  const shown = widgets.map((w) => ({ w, sizes: home.supported(w).filter((s) => CELLS[s][0] <= columns) })).filter((x) => x.sizes.length)
  return (
    <ResponsiveDialog open onOpenChange={(open) => !open && close()} title={only ? `Add Widget · ${home.label(only)}` : `Add Widget · ${p.title}`} action={<Button size="sm" variant="ghost" onClick={close}>Cancel</Button>}>
      <div className="flex flex-col divide-y overflow-y-auto">
        {shown.length ? shown.map(({ w, sizes }) => <Addable key={`${w.app}:${w.widget}`} w={w} sizes={sizes} p={p} close={close} />) : <Empty>{`No app on ${p.title} has a widget.`}</Empty>}
      </div>
    </ResponsiveDialog>
  )
}

function Addable({ w, sizes, p, close }) {
  const a = home.app(w.app), title = w.title || home.label(a) || w.widget
  const [size, setSize] = useState(sizes.includes(w.size) ? w.size : sizes[0])
  const [ref, room] = useWidth()
  // As big as on the page: its cells, as that grid has them, drawn at that size and no larger than the room there is.
  const { cell = 80, gap = 14 } = S.geometry[p.scope] || {}, [cw, ch] = CELLS[size]
  const width = cw * cell + (cw - 1) * gap, height = ch * cell + (ch - 1) * gap, fit = Math.min(1, Math.max(0, room - 32) / width || 1)
  const quietHost = { run: async () => {}, follow: () => {} }
  const add = () => {
    const id = [...crypto.getRandomValues(new Uint8Array(5))].map((b) => b.toString(16).padStart(2, "0")).join("")
    one.tap("success")
    close()
    save(p, [...keptOf(p), { app: w.app, widget: w.widget, id, size }])
  }
  return (
    <section ref={ref} className="flex flex-col gap-3 p-4">
      <div className="flex items-center gap-2.5">
        {a && <AppIcon app={a} size={28} />}
        <b className="text-sm font-medium">{title}</b>
      </div>
      {sizes.length > 1 && (
        <Tabs value={size} onValueChange={(s) => (one.tap(), setSize(s))}>
          <TabsList className="w-full">{sizes.map((s) => <TabsTrigger key={s} value={s}>{NAMES[s]}</TabsTrigger>)}</TabsList>
        </Tabs>
      )}
      <div className="pointer-events-none mx-auto py-1" style={{ width: width * fit, height: height * fit + 8 }}>
        <WidgetCard w={{ ...w, title }} host={quietHost} tree={home.tree(w, size)} style={{ width, height, transform: `scale(${fit})`, transformOrigin: "0 0" }} />
      </div>
      <Button variant="secondary" className="w-full" onClick={add}>Add Widget</Button>
    </section>
  )
}

/// Edit Pages…'s rows: dragged into order by their grips, each switched off to hide it.
function EditPages({ groups, close }) {
  const kept = home.pageChoices()
  const [order, setOrder] = useState(() => [...groups].sort((a, b) => (kept.order.includes(a.scope) ? kept.order.indexOf(a.scope) : kept.order.length + groups.indexOf(a)) - (kept.order.includes(b.scope) ? kept.order.indexOf(b.scope) : kept.order.length + groups.indexOf(b))).map((p) => p.scope))
  const [hidden, setHidden] = useState(() => new Set(kept.hidden))
  const [moving, setMoving] = useState(null)
  const list = useRef(null)
  const title = (scope) => groups.find((g) => g.scope === scope)?.title
  // A row follows the finger by its grip, the others moving aside as it passes them.
  function grip(e, scope) {
    e.preventDefault()
    const rows = () => [...list.current.children]
    let y0 = e.clientY, at = order.indexOf(scope), now = order
    setMoving({ scope, dy: 0 })
    one.tap("medium")
    const move = (m) => {
      const els = rows(), prev = els[at - 1], next = els[at + 1]
      if (next && m.clientY > next.getBoundingClientRect().top + next.offsetHeight / 2) {
        y0 += next.offsetHeight
        now = now.slice(); now.splice(at, 1); now.splice(at + 1, 0, scope); at++
        setOrder(now)
        one.tap()
      } else if (prev && m.clientY < prev.getBoundingClientRect().bottom - prev.offsetHeight / 2) {
        y0 -= prev.offsetHeight
        now = now.slice(); now.splice(at, 1); now.splice(at - 1, 0, scope); at--
        setOrder(now)
        one.tap()
      }
      setMoving({ scope, dy: m.clientY - y0 })
    }
    const up = () => {
      setMoving(null)
      removeEventListener("pointermove", move)
      removeEventListener("pointerup", up)
      removeEventListener("pointercancel", up)
    }
    addEventListener("pointermove", move)
    addEventListener("pointerup", up)
    addEventListener("pointercancel", up)
  }
  function done() {
    close()
    const pages = { order, hidden: order.filter((s) => hidden.has(s)) }
    if (JSON.stringify(pages) !== JSON.stringify({ order: kept.order, hidden: kept.hidden })) keepPages(pages)
  }
  return (
    <ResponsiveDialog open onOpenChange={(open) => !open && done()} title="Pages" action={<Button size="sm" onClick={done}>Done</Button>}>
      <div className="flex flex-col overflow-y-auto">
        <div ref={list} className="divide-y">
          {order.map((scope) => (
            <div key={scope} className={cn("relative flex min-h-12 touch-none items-center gap-3 bg-background px-4", moving?.scope === scope && "z-10 shadow-lg")} style={moving?.scope === scope ? { transform: `translateY(${moving.dy}px)` } : undefined}>
              <span className="cursor-grab text-xl text-muted-foreground" onPointerDown={(e) => grip(e, scope)}><Icon name="dots-six-vertical" className="size-5" /></span>
              <span className="min-w-0 flex-1 truncate text-sm">{title(scope)}</span>
              <Switch aria-label={`Show ${title(scope)}`} checked={!hidden.has(scope)} onCheckedChange={(on) => { setHidden((h) => { const n = new Set(h); on ? n.delete(scope) : n.add(scope); return n }); one.tap() }} />
            </div>
          ))}
        </div>
        <p className="px-4 pb-2 pt-3 text-[13px] text-muted-foreground">Drag to reorder; switch one off to hide it.</p>
      </div>
    </ResponsiveDialog>
  )
}

function Dialogs({ dialog: d }) {
  if (!d) return null
  const close = () => (S.dialog = null, S.render())
  if (d.kind === "options") return <Options key={d.kept.id} w={d.w} kept={d.kept} p={d.p} close={close} />
  if (d.kind === "add") return <AddWidget p={d.p} only={d.only} columns={d.columns} close={close} />
  return <EditPages groups={d.groups} close={close} />
}

// MARK: The screen

/// Grey tiles where the pages will be, while they're read.
const Waiting = () => (
  <div role="status" aria-label="Loading" className="grid gap-4 p-5" style={{ gridTemplateColumns: "repeat(4, minmax(0, 1fr))" }}>
    {Array.from({ length: 12 }, (_, i) => <Skeleton key={i} className={cn("aspect-square", i === 0 ? "col-span-2 row-span-2 rounded-3xl" : i === 1 ? "col-span-2 row-span-2 rounded-3xl" : "rounded-2xl")} />)}
  </div>
)

function Pages() {
  const [snap, setSnap] = useState({ phase: "loading", view: [], refreshing: [] })
  const [slow, setSlow] = useState(false)
  const strip = useRef(null)
  const { view, phase, editing, at, dialog, refreshing, attention } = snap
  const scopes = view.map((v) => v.scope)

  // What draws the screen again: with `now`, at once, for what a finger is carrying.
  useLayoutEffect(() => {
    S.render = (now) => {
      if (swipe && S.editing && S.at === SETTINGS) S.editing = false
      const next = snapshot()
      S.view = next.view
      if (now) flushSync(() => setSnap(next))
      else setSnap(next)
    }
    S.render()
  }, [])

  // Nothing kept (the first time on this device): grey tiles only if reading takes a while.
  useEffect(() => {
    const wait = setTimeout(() => setSlow(true), 300)
    return () => clearTimeout(wait)
  }, [])

  // The pointers the screen follows beyond the page, and its menu, which is set as a property.
  useEffect(() => {
    const up = (e) => mine(e) && lifted(e.target)
    const move = (e) => mine(e) && moved(e.clientX, e.clientY)
    const cancel = (e) => {
      if (!mine(e)) return
      if (e.pointerType === "touch" && (drag || held?.ready)) return void (touched = true)
      if (drag) return void drop()
      clearTimeout(held.timer)
      if (held.el) held.el.style.scale = ""
      held = null
    }
    const touchMove = (e) => {
      // Once a hold has lifted something, the finger carries it rather than scrolling the page.
      if (drag || held?.ready || held?.resize) e.preventDefault()
      if (touched && e.touches[0]) moved(e.touches[0].clientX, e.touches[0].clientY)
    }
    const touchEnd = (e) => {
      if (!touched) return
      touched = false
      if (drag || held) lifted(e.target)
    }
    const key = (e) => { if (e.key === "Escape" && S.editing && !document.querySelector("[role=dialog]")) edit(false) }
    document.addEventListener("pointerdown", pointerDown)
    addEventListener("pointermove", move)
    addEventListener("pointerup", up)
    addEventListener("pointercancel", cancel)
    document.addEventListener("touchmove", touchMove, { passive: false })
    document.addEventListener("touchend", touchEnd)
    document.addEventListener("touchcancel", touchEnd)
    addEventListener("keydown", key)
    document.oncontextmenu = contextMenu
    return () => {
      document.removeEventListener("pointerdown", pointerDown)
      removeEventListener("pointermove", move)
      removeEventListener("pointerup", up)
      removeEventListener("pointercancel", cancel)
      document.removeEventListener("touchmove", touchMove)
      document.removeEventListener("touchend", touchEnd)
      document.removeEventListener("touchcancel", touchEnd)
      removeEventListener("keydown", key)
      document.oncontextmenu = null
    }
  }, [])

  // The shell's own zoom for the person's text size: a widget keeps its own.
  useLayoutEffect(() => {
    const unzoom = () => document.documentElement.style.setProperty("--unzoom", 1 / (parseFloat(getComputedStyle(document.documentElement).zoom) || 1))
    unzoom()
    const seen = new ResizeObserver(unzoom)
    seen.observe(document.documentElement)
    return () => seen.disconnect()
  }, [])

  // Swiping: opened at the page asked for.
  const key = scopes.join(" ")
  useLayoutEffect(() => {
    if (!swipe || !strip.current || !scopes.length) return
    if (!scopes.includes(S.at)) S.at = "user"
    strip.current.scrollLeft = scopes.indexOf(S.at) * strip.current.clientWidth
  }, [key])

  if (phase === "error") return <div className="p-5"><Problem title={snap.problem.title} text={snap.problem.next} action={{ label: "Try Again", run: S.retry }} /></div>
  if (phase !== "ready") return slow ? <Waiting /> : null
  return (
    <div className="fixed inset-0 select-none overflow-hidden [-webkit-touch-callout:none] [-webkit-user-select:none]">
      <style>{"@keyframes jiggle { 0% { rotate: calc(var(--wobble) * -1); } 50% { rotate: var(--wobble); } 100% { rotate: calc(var(--wobble) * -1); } }"}</style>
      <div
        ref={strip}
        className="flex h-full snap-x snap-mandatory overflow-x-auto overflow-y-hidden overscroll-x-contain [scrollbar-width:none] [&::-webkit-scrollbar]:hidden"
        onScroll={swipe ? (e) => {
          const i = Math.round(e.currentTarget.scrollLeft / Math.max(1, e.currentTarget.clientWidth))
          if (scopes[i] && scopes[i] !== S.at) { S.at = scopes[i]; S.render() }
        } : undefined}
      >
        {view.map((page) => <Page key={page.scope} page={page} editing={editing && page.scope !== SETTINGS} refreshing={refreshing} bottom={swipe ? "calc(120px + env(safe-area-inset-bottom))" : 20} />)}
      </div>
      {swipe && <Dots scopes={scopes} at={at} strip={strip} attention={attention} />}
      {swipe && editing && at !== SETTINGS && (
        <div data-ctl className="pointer-events-none fixed inset-x-4 top-[calc(10px+env(safe-area-inset-top))] z-10 flex justify-between">
          <Button className="pointer-events-auto" variant="outline" onClick={(e) => { const p = home.page(S.at); if (p) editMenu(p, e.currentTarget) }}>Edit</Button>
          <Button className="pointer-events-auto" onClick={() => edit(false)}>Done</Button>
        </div>
      )}
      <Dialogs dialog={dialog} />
    </div>
  )
}

// MARK: Loading

let watching = false
/// Reads everything, drawing it as it comes, and keeps it for next time; then keeps up with what changes. Done once
/// the pages are drawn: each page's widgets come after, a grey card in each one's place until they do.
function refresh() {
  return new Promise((drawn, failed) => {
    home.load(() => { S.phase = "ready"; draw(); drawn() }).then(() => {
      draw()
      keep()
      if (!watching) home.watch(() => { draw(); keep() }), (watching = true)
    }, failed)
  })
}

async function start() {
  one.title("")
  const kept = await one.cache("pages").catch(() => null)
  const first = !kept?.state
  if (!first) {
    // Drawn from what was kept, at once, then from what's read: offline, it stays as it was, trying again.
    Object.assign(home.data, kept)
    S.phase = "ready"
    S.render()
  }
  S.retry = () => { one.tap(); S.phase = "loading"; S.render(); start() }
  for (let wait = 2000; ; wait = Math.min(wait * 2, 30000)) {
    try {
      return await refresh()
    } catch (err) {
      // The first time, with nothing to show: the problem, with Try Again; after, what was kept stays.
      if (first) return S.phase = "error", S.problem = one.plain(err), S.render()
      await new Promise((ok) => setTimeout(ok, wait))
    }
  }
}

createRoot(document.getElementById("root")).render(<Pages />)
start()
