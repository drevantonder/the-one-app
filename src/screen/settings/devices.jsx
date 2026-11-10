// This user's devices, in three tabs: Devices (each of theirs, with Rename and Remove, Add Device, their
// passkeys), Run Priority (the order every device runs apps in, an admin's to change) and Enabled Services (what each of their
// devices runs, switched by tapping). Devices waiting to be let in show on top of every tab, approved by tapping;
// under Devices, where this device holds its own keys (`device.keys`, the web client), which of their devices run
// which apps' schedules.
//
// A Settings screen written in React: @one/ui's build (src/screen/ui, `bun run build`) makes it into dist/devices.html,
// which the client builds in (src/screen.rs). React and @one/ui come from the page; `one` is the bridge.
import { useEffect, useRef, useState } from "react"
import { createRoot } from "react-dom/client"
import { AppIcon, Button, Empty, Hint, Icon, Input, Label, LargeTitle, List, ListAdd, ListRow, ListSkeleton, LoadFailure, Problem, ResponsiveDialog, Section, Spinner, Switch, Tabs, TabsList, TabsTrigger, cn, useDialog } from "@one/ui"

const api = one.api
const said = (err) => one.plain(err).title
const yours = one.config.yours
const shown = (name) => (String(name || "").endsWith(".local") ? name.slice(0, -6) : String(name || ""))
// How long ago a device was last on: "5 min ago", "2 h ago", then the day.
const ago = (t) => {
  const s = one.ago(t)
  return /^\d/.test(s) ? `${s} ago` : s
}
const ordinal = (n) => n + (n % 100 > 10 && n % 100 < 14 ? "th" : ["th", "st", "nd", "rd"][n % 10] || "th")
// Run priority: a number from 1, or "last", always after every number.
const rank = (p) => (p === "last" ? 1000 : Number(p) || 1)
const day = (t) => new Date(t).toLocaleDateString(undefined, { day: "numeric", month: "short", year: "numeric" })

// What a device is, from what it says: its kind and build for a machine, its name for the rest, which our own apps
// give ("iPhone", "The One App on Andre’s MacBook Air", "Chrome on Mac").
const BROWSER = /^(Chrome|Safari|Firefox|Edge|A browser)( on (.+))?$/
const osOf = (target, name = "") => {
  if (/apple/.test(target || "") || /\b(iPhone|iPad|Mac|MacBook|iMac)\b/i.test(name)) return "apple-logo"
  if (/windows/.test(target || "") || /\bWindows\b/i.test(name)) return "windows-logo"
  if (/android/.test(target || "") || /\b(Android|Pixel|Galaxy)\b/i.test(name)) return "android-logo"
  if (/linux/.test(target || "") || /\bLinux\b/i.test(name)) return "linux-logo"
  return ""
}
const typeOf = (name) => {
  if (/phone|\b(Android|Pixel|Galaxy)\b/i.test(name)) return "device-mobile"
  if (/\b(iPad|Tablet)\b/i.test(name)) return "device-tablet"
  if (/\b(MacBook|Laptop)\b/i.test(name)) return "laptop"
  return "desktop"
}
// A device's picture: its type, with its OS's glyph at its corner.
const glyph = (type, os = "", off = false) => ({ type, os, off })
// A browser's logo with its OS's glyph, from its name ("Chrome on Mac"); a device's type, from its name, for the rest.
const iconByName = (name) => {
  const [browsing, browser, , on] = name.match(BROWSER) || []
  return browsing ? glyph({ Chrome: "google-chrome-logo", Safari: "compass" }[browser] || "browser", osOf("", on || "")) : glyph(typeOf(name), osOf("", name))
}
// A client's picture, from what it says it is; one that hasn't yet, from its name.
const OS = { mac: "apple-logo", iphone: "apple-logo", windows: "windows-logo", linux: "linux-logo", android: "android-logo" }
const iconOf = (c) => {
  if (c.platform === "browser") return BROWSER.test(c.name) ? iconByName(c.name) : glyph("browser")
  if (c.platform === "cloudflare") return glyph("cloud")
  const type = typeOf(c.name)
  return glyph(type === "desktop" && /iphone|android/.test(c.platform || "") ? "device-mobile" : type, OS[c.platform] || osOf("", c.name))
}
const CLOUDFLARE = { cloud: true }

// What a device signed in with, as a grey line says it: "a@example.com · Google", or "Passkey".
function signInLabel(a) {
  if (a?.passkey) return "Passkey"
  const p = a?.provider
  if (!p) return ""
  let host = p.issuer
  try { host = new URL(p.issuer).host } catch {}
  const by = { "accounts.google.com": "Google", "appleid.apple.com": "Apple", "login.microsoftonline.com": "Microsoft" }[host] || host
  return [p.email, by].filter(Boolean).join(" · ")
}

// Each switch flipped here, by device and part: what was asked (`on`, and for Try Again the problem it retries), and
// how far it got: `sent`, then `written` once the node wrote it or `failed` with why it couldn't. Shown as asked until
// its device says it did, or for `LATE`; then, if it still hasn't, its dot says so.
const asked = {}
// Try Again's restart can come and go between two draws, so the same problem a few seconds on is its answer.
const LATE = 30_000, AGAIN = 10_000
// How long a device has to come back on a new version before it counts as not having.
const UPDATE_WITHIN = 5 * 60_000

/// The device a node entry is, for what's recorded against a device (its priority, its grants, its parts): itself, or
/// the device it's the app runtime of.
const deviceKey = (node) => node.device || node.key

// What the screen knows of the devices, from what the node says: each of theirs with what's on it, their passkeys, and
// what each part of a device is doing. `again` gives what Try Again runs.
function model(x, again) {
  const { state, device, apps, seen, seenRead, versions } = x
  const me = state.user?.id
  const admin = state.user?.role === "admin"
  const appOf = (name) => apps.find((a) => a.name === name)
  const appTitle = (name) => appOf(name)?.title || name
  const holds = (s) => (s.runs_for || []).includes(`user:${me}`)
  const userName = (id) => state.network?.users.find((u) => u.id === id)?.name || "someone"

  // What exists, grouped by the device it's on, as each says by its key: a machine's oned and the sign-ins of its
  // apps; a client with no device link, such as a browser, is a device of its own.
  function grouped() {
    // This device, as its node says, or by its own key: a browser's, which signs in, or the one asking.
    const mark = (e) => ({ ...e, me: e.me || [device.key, state.me?.key].includes(e.key) })
    const devices = (state.network?.devices || []).map(mark), clients = (state.network?.clients || []).map(mark)
    const cards = [], passkeys = [], stale = []
    const card = (user, name) => {
      const c = { user, name: shown(name), node: null, clients: [] }
      cards.push(c)
      return c
    }
    const byKey = new Map()
    const computes = devices.filter((d) => (d.roles || []).includes("compute"))
    for (const d of computes.filter((d) => !d.device)) byKey.set(d.key, Object.assign(card(d.user, d.name), { node: d }))
    // A device's app runtime, folded under that device: the device's own card if it still runs things itself, else its
    // only card, under the device's key.
    for (const d of computes.filter((d) => d.device)) {
      if (byKey.has(d.device)) byKey.get(d.device).runtime = d
      else byKey.set(d.device, Object.assign(card(d.user, d.name), { node: d }))
    }
    for (const c of clients) {
      if (c.passkey) {
        passkeys.push(c)
        continue
      }
      // One listed before sign-ins named their device names it itself when its app next runs: one that hasn't been
      // let in for a week never will, and folds into one row.
      if (!c.device && !c.platform && !c.me && seenRead && Date.now() - Math.max(seen[c.key] || 0, c.since || 0) > 7 * 86_400_000) {
        stale.push(c)
        continue
      }
      // An app's sign-in, under the device it's on: its oned's card, or a phone's own, which a sign-in there again
      // replaces.
      if (c.device && !byKey.has(c.device)) byKey.set(c.device, card(c.user, c.name))
      ;(byKey.get(c.device || c.key) || card(c.user, c.name)).clients.push(c)
    }
    for (const c of cards) {
      c.id = c.node?.key || c.clients[0]?.key
      // A device with a runtime of its own goes by the device's name, in its client row.
      const named = c.node?.device && c.clients.find((e) => e.key === c.node.device)
      if (named) c.name = shown(named.name)
      // Its oned last, then the device itself: removing it takes what's part of it, its apps' sign-ins first here.
      c.keys = [...c.clients, c.runtime, c.node].filter(Boolean).map((e) => e.key).sort((a, b) => (a === c.node?.device) - (b === c.node?.device))
      c.me = [c.node, ...c.clients].some((e) => e?.me)
      c.online = !!c.node?.online
      c.runs = !!c.node
      c.priority = c.node?.priority ?? null
      // Its sign-in now: this one, else the last seen, else the newest; the rest are old.
      const now = [...c.clients].sort((a, b) => b.me - a.me || (seen[b.key] || 0) - (seen[a.key] || 0) || (b.since || 0) - (a.since || 0))
      c.signIn = now[0] || null
      c.old = now.slice(1)
      c.seen = Math.max(0, ...c.keys.map((k) => seen[k] || 0))
      c.version = versions[c.node?.key] || versions[c.signIn?.key] || null
      c.icon = c.node
        ? glyph(c.node.kind === "laptop" ? "laptop" : /apple|windows/.test(c.node.target || "") ? "desktop" : "hard-drives", osOf(c.node.target, c.name), !c.online)
        : iconOf({ ...c.signIn, name: c.signIn?.name || c.name })
    }
    // Theirs, and others' that run their apps: all of those for an admin, who sets where things run.
    const shownCards = cards.filter((c) => c.user === me || (c.node && (admin || holds(c.node))))
    shownCards.sort((a, b) => (a.runs ? rank(a.priority) : 2000) - (b.runs ? rank(b.priority) : 2000) || a.name.localeCompare(b.name))
    return { cards: shownCards, passkeys: passkeys.filter((p) => p.user === me), stale: stale.filter((c) => c.user === me) }
  }
  const drawn = grouped()

  /// A part as its dot shows it, from what its device says and what was asked here.
  function partNow(c, p) {
    const k = `${deviceKey(c.node)} ${p.name}`, a = asked[k]
    if (!a) return p
    // Its device did it: on or off as asked, and for Try Again, past the problem it retried.
    if (a.state !== "failed" && p.on === a.on && (!a.again || p.doing || p.problem !== a.again)) return delete asked[k], p
    if (a.state === "failed") return p.on === a.on ? (delete asked[k], p) : { ...p, failed: a.error, wanted: a.on }
    if (a.again && p.on === a.on && a.state === "written" && Date.now() - a.at > AGAIN) return delete asked[k], p
    if (Date.now() - a.at < LATE) return { ...p, on: a.on, doing: a.on ? "Starting" : "Stopping", problem: null }
    return { ...p, on: a.on, doing: null, problem: null, late: true }
  }
  const partTitle = (p) => (p.name === "apps" ? "Runs apps" : appOf(p.name)?.title || p.title || p.name)
  /// What a device can run, as it says (Runs apps, then its services), each as asked here, with how to switch it. One
  /// that doesn't list them yet: Runs apps as it is, and its services, on.
  function partsOf(c) {
    if (Array.isArray(c.node?.parts)) return c.node.parts.map((p) => ({ ...partNow(c, p), part: { device: deviceKey(c.node), name: p.name } }))
    return [
      ...(c.node ? [{ name: "apps", on: true, fixed: true }] : []),
      ...(c.node?.installed_services || []).map((s) => ({ name: s.name, on: true, part: { service: s.name, device: deviceKey(c.node) } })),
    ]
  }
  /// What's wrong with a part, as one problem with what fixes it: a switch that failed or is late, or a part that
  /// isn't running.
  function problemOf(c, p) {
    const title = partTitle(p), part = p.part
    const retry = (on, retried) => ({ label: "Try Again", run: again(part, on, retried) })
    if (p.failed) return { title: `Couldn’t turn ${title} ${p.wanted ? "on" : "off"}`, text: p.failed, action: retry(p.wanted) }
    if (p.late) return { title: `${c.name} hasn’t turned ${title} ${p.on ? "on" : "off"} yet`, text: "It may be offline.", action: retry(p.on) }
    if (p.on && p.problem) return { title: `${title} isn’t running on ${c.name}`, text: p.problem, action: retry(true, p.problem) }
    if (p.on && p.sandbox) return { title: `${title} runs without its sandbox on ${c.name}`, text: p.sandbox }
    return null
  }
  const problemsOf = (c) => (c.user === me ? partsOf(c).map((p) => problemOf(c, p)).filter(Boolean) : [])

  /// Every device that runs apps, and Cloudflare, in columns: one per number in use, first first, then Last.
  function board() {
    const entries = drawn.cards.filter((c) => c.runs).map((c) => ({
      id: c.id, name: c.name, icon: c.icon, whose: c.user === me ? "" : `${userName(c.user)}’s`, p: c.priority,
      keys: [deviceKey(c.node)],
    }))
    const s = state.network?.cloudflare_node
    if (s) entries.push({ id: "cloudflare", name: "Cloudflare", icon: CLOUDFLARE, whose: "", p: s.priority ?? "last", keys: ["cloudflare"] })
    const numbers = [...new Set(entries.filter((e) => e.p !== "last").map((e) => rank(e.p)))].sort((a, b) => a - b)
    return { cols: numbers.map((n) => entries.filter((e) => e.p !== "last" && rank(e.p) === n)), last: entries.filter((e) => e.p === "last") }
  }

  /// Their own devices that can run something, and Cloudflare, each with what it can run: Cloudflare always runs
  /// apps, and runs the services built for it that an admin switches on, for the apps in the network.
  function gridColumns() {
    const cols = drawn.cards.filter((c) => c.user === me && c.runs).map((c) => ({ c, name: c.name, icon: c.icon, parts: partsOf(c) }))
    const s = state.network?.cloudflare_node
    if ((cols.length || admin) && s) {
      const parts = (s.parts || [{ name: "apps", on: true, fixed: true }]).filter((p) => p.name === "apps" || appOf(p.name))
      cols.push({ c: null, name: "Cloudflare", icon: CLOUDFLARE, parts: parts.map((p) => (p.fixed || !admin ? { ...p, fixed: true } : { ...p, part: { device: s.key, name: p.name } })) })
    }
    return cols
  }

  // A device of theirs let in within the day: "MacBook Pro joined ·
  // Sydney", with Remove, in case it isn't theirs.
  function justJoined() {
    const since = Date.now() - 86_400_000
    return (state.network?.clients || []).filter((c) => c.user === me && !c.passkey && !c.me && (c.since || 0) > since && ![device.key, state.me?.key].includes(c.key)).sort((a, b) => b.since - a.since)
  }

  // App schedules: "linux runs WhatsApp's schedules for you", until when, with Stop. They last 30 days, and longer as
  // they use the app.
  const forWhom = (scope) => (scope === `user:${me}` ? "you" : scope === "global" ? "everyone" : state.network?.groups.find((g) => `group:${g.id}` === scope)?.name || "a group")
  // The apps that run on their own, every so often, in a scope (their own, unless it's said).
  const scheduled = (scope = `user:${me}`) => apps.filter((a) => (a.schedules || []).some((s) => s.scope === scope && !s.removed))
  // The devices that may run an app's schedules in a scope: those that hold it.
  const runners = (scope) => (state.network?.devices || []).filter((d) => (d.runs_for || []).includes(scope))

  return { me, admin, appOf, appTitle, userName, drawn, partsOf, partTitle, problemOf, problemsOf, board, gridColumns, justJoined, forWhom, scheduled, runners }
}

// ---- The screen ----

function Devices() {
  const [data, setData] = useState(null)
  const [problem, setProblem] = useState(null)
  const [error, setError] = useState("")
  const [tab, setTab] = useState(["devices", "priority", "services"].includes(one.params?.tab) ? one.params.tab : "devices")
  // Which devices' problems are open (Devices), and which cell's (Services).
  const [opened, setOpened] = useState({})
  const [cellOpen, setCellOpen] = useState("")
  // A new device's code, from its QR's link (`/approve#CODE`), asked about once; and what it asked.
  const [scan, setScan] = useState(null)
  // Each time one opens it starts afresh.
  const [dialog, show, setDialogOpen] = useDialog()
  const [busyCode, setBusyCode] = useState("")
  // The devices being asked to update.
  const [updating, setUpdating] = useState({})
  const [, bump] = useState(0)
  const apps = useRef([])
  const dragging = useRef(false)

  async function load() {
    try {
      const [state, device] = await Promise.all([api("GET", "/api/state"), one.device()])
      // Who waits, from a node or the web client itself; one with no code to compare is an older node's, not shown.
      let waiting = [], seen = null, versions = {}
      const asks = [
        api("POST", "/api/devices/versions", {}).then((v) => (versions = v), () => {}),
        !apps.current.length ? api("GET", "/api/apps").then((a) => (apps.current = a), () => {}) : null,
        api("POST", "/api/devices/requests", {}).then((w) => (waiting = w.filter((r) => r.code)), () => {}),
      ]
      if (device.keys) {
        asks.push(api("GET", "/api/devices/seen").then((s) => (seen = s || {}), () => {}))
      }
      await Promise.all(asks)
      // Not while they're dragging.
      if (dragging.current) return
      setData({ state, device, apps: apps.current, waiting, seen: seen || {}, seenRead: seen !== null, versions })
      setProblem(null)
      return { device, waiting }
    } catch (e) {
      setProblem(one.plain(e))
    }
  }
  // A change made here, then the page as it is after.
  async function act(fn) {
    setError("")
    try {
      await fn()
    } catch (err) {
      setError(said(err))
      one.tap("error")
    }
    await load()
  }

  // The same screen first loaded gets the one asked about from its QR, the app that sent them to choose where its
  // schedules run, and Add Device (a Mac's Pair a Computer…, `add`).
  useEffect(() => {
    const schedulesAsked = one.params?.schedules ? { app: String(one.params.schedules), scope: one.params.scope && String(one.params.scope) } : null
    load().then((first) => {
      if (!first) return
      // One it already asks about at the top is answered there.
      const typed = String(one.params?.approve || "").toUpperCase().replace(/[^A-Z2-7]/g, "")
      if (typed && !first.waiting.some((r) => typed.startsWith(r.code.replaceAll(" ", "")))) approveScanned(typed)
      if (schedulesAsked && first.device.keys) show({ schedules: schedulesAsked })
      else if (one.params?.add) show({ add: true })
    })
    one.on("changed", () => load())
  }, [])

  /// A device's code from its QR, not waiting at the top: which device it is, and Approve.
  async function approveScanned(typed) {
    const code = String(typed).toUpperCase().replace(/[^A-Z2-7]/g, "")
    try {
      const r = await api("POST", "/api/devices/approve", { code })
      setScan({ code, r })
    } catch (err) {
      setScan({ error: said(err) })
    }
  }

  /// A part of a device switched on or off, which its oned then sets up and starts, or stops: nothing's lost by
  /// switching one off (a WhatsApp link stays, and runs again when it's back on), so nothing asks first.
  /// Switched on again while it's on (`again`, the problem it had), it starts afresh, as Try Again does. What
  /// happens to it shows on its dot (`partNow`), which is drawn again once it's late.
  async function setPart(part, on, retried = null) {
    const k = `${part.device} ${part.name ?? part.service}`, at = Date.now()
    asked[k] = { on, again: retried, at, state: "sent" }
    bump((n) => n + 1)
    for (const after of retried ? [AGAIN, LATE] : [LATE]) setTimeout(() => asked[k]?.at === at && load(), after + 500)
    try {
      await api("POST", "/api/service", { device: part.device, name: part.name ?? part.service, on })
      if (asked[k]?.at === at) asked[k].state = "written"
    } catch (err) {
      if (asked[k]?.at === at) asked[k] = { ...asked[k], state: "failed", error: said(err) }
      // A device that doesn't list its parts shows nothing for it: the page says it instead.
      if (part.service) throw err
      one.tap("error")
    }
  }

  const m = data && model(data, (part, on, retried) => () => act(() => setPart(part, on, retried)))

  /// Puts a device (or Cloudflare) in a column (its index, "new" or "last"), and numbers the columns again from 1, so
  /// one emptied closes up: written into the network's `priority` setting, each of a device's endpoints alike.
  function place(id, to) {
    const { cols, last } = m.board()
    const e = [...cols.flat(), ...last].find((x) => x.id === id)
    if (!e) return
    let order = cols.map((c) => c.filter((x) => x.id !== id))
    const end = last.filter((x) => x.id !== id)
    if (to === "last") end.push(e)
    else if (to === "new") order.push([e])
    else order[Number(to)]?.push(e)
    order = order.filter((c) => c.length)
    const network = data.state.network
    let set = {}
    try {
      set = JSON.parse(network.settings?.priority || "{}") || {}
    } catch {}
    // Only what's still a device of the network, or Cloudflare.
    const known = new Set([...(network.devices || []).map(deviceKey), "cloudflare"])
    set = Object.fromEntries(Object.entries(set).filter(([k]) => known.has(k)))
    order.forEach((c, i) => c.forEach((x) => x.keys.forEach((k) => (set[k] = i + 1))))
    end.forEach((x) => x.keys.forEach((k) => (set[k] = "last")))
    const value = JSON.stringify(set)
    if (value === network.settings?.priority) return
    // Shown at once, as it will be.
    setData({
      ...data,
      state: {
        ...data.state,
        network: {
          ...network,
          devices: (network.devices || []).map((d) => (deviceKey(d) in set ? { ...d, priority: set[deviceKey(d)] } : d)),
          cloudflare_node: network.cloudflare_node && "cloudflare" in set ? { ...network.cloudflare_node, priority: set.cloudflare } : network.cloudflare_node,
          settings: { ...network.settings, priority: value },
        },
      },
    })
    one.tap()
    act(() => api("POST", "/api/settings", { name: "priority", value }))
  }

  /// The way to move a device without dragging: 1st, 2nd…, New, Last.
  const menuAt = useRef(0)
  async function moveMenu(id, at) {
    if (Date.now() - menuAt.current < 800) return
    menuAt.current = Date.now()
    const { cols } = m.board()
    const from = cols.findIndex((c) => c.some((x) => x.id === id))
    // A new column is the same place for one alone in the last.
    const fresh = !(from === cols.length - 1 && cols[from].length === 1)
    const choices = [...cols.map((_, i) => i), ...(fresh ? ["new"] : []), "last"]
    const items = choices.map((c) =>
      c === "last" ? { title: "Last", checked: from < 0, section: true } : c === "new" ? { title: "New", icon: "plus" } : { title: ordinal(c + 1), checked: c === from })
    const i = await one.menu(items, at).catch(() => null)
    menuAt.current = Date.now()
    if (i != null) place(id, choices[i])
  }
  useDrag(dragging, (id, to) => latest.current.place(id, to), (id, at) => latest.current.moveMenu(id, at))
  latest.current = { place: (...a) => place(...a), moveMenu: (...a) => moveMenu(...a) }

  /// A device's ⋯: Rename, its sign-ins, Remove (or Sign Out, for this one).
  async function menu(item, passkey, at) {
    const items = passkey ? [] : [{ title: "Rename", icon: "pencil-simple" }]
    // What its sign-ins were let in with, which a new device's sign-in is checked against.
    const signedIn = (passkey ? [item] : item.clients).filter((c) => c.sign_in || c.linked?.length)
    if (signedIn.length || item.me) items.push({ title: "Sign-Ins…", icon: "key" })
    if (item.me) items.push({ title: "This Device’s Code", icon: "info" })
    items.push(item.me ? { title: "Sign Out", icon: "sign-out", danger: true } : { title: "Remove…", icon: "minus-circle", danger: true })
    const i = await one.menu(items, at).catch(() => null)
    if (i == null) return
    if (items[i].title === "Rename") show({ rename: item })
    else if (items[i].title === "Sign-Ins…") show({ signIns: signedIn, me: item.me })
    else if (items[i].title === "This Device’s Code") act(async () => one.alert({ title: "This Device’s Code", text: (await api("GET", "/api/devices/code")).code }))
    else if (item.me) signOut()
    else remove(item.name, passkey ? [item.key] : item.keys, !passkey && item.node && item.online ? item.node.key : null)
  }

  /// Removes a device, everything on it: it opens nothing new. One that runs oned and is on can also erase what it
  /// keeps first, which it does itself, so only while it's reachable: that asks in a dialog, the rest in the alert.
  async function remove(name, keys, erasable) {
    if (erasable) return show({ remove: { name, keys, erasable } })
    if (!(await one.alert({ title: `Remove ${name}?`, action: "Remove", danger: true }))) return
    one.tap("medium")
    act(async () => {
      for (const key of keys) await api("POST", "/api/devices/remove", { device: key })
    })
  }
  async function removeOld(card) {
    const n = card.old.length
    const of = card.name ? ` of ${card.name}` : ""
    if (!(await one.alert({ title: `Remove ${n} old sign-in${n > 1 ? "s" : ""}${of}?`, action: "Remove", danger: true }))) return
    act(async () => {
      for (const c of card.old) await api("POST", "/api/devices/remove", { device: c.key })
    })
  }
  async function signOut() {
    if (await one.alert({ title: `Sign Out of ${yours} on this device?`, action: "Sign Out", danger: true })) one.signOut()
  }

  /// Asks each of `cards` in turn to take the newest version, going on only once the one before is back on it: one that
  /// doesn't come back stops the rest, as whatever's wrong with it may be wrong with what they'd take too.
  async function update(cards) {
    setError("")
    one.tap("medium")
    for (const c of cards) {
      setUpdating({ [c.node.key]: true })
      try {
        await api("POST", "/api/devices/update", { device: c.node.key })
        if (!(await cameBack(c.node.key))) {
          setError(`${c.name} didn’t come back on the new version.`)
          one.tap("error")
          break
        }
      } catch (err) {
        setError(said(err))
        one.tap("error")
        break
      }
    }
    setUpdating({})
    await load()
  }

  /// Whether a device reports the version it was updated to within `UPDATE_WITHIN`. It's away while it restarts, and
  /// this device too if it's the one updating, so a question that fails only means it isn't back yet.
  async function cameBack(key) {
    const until = Date.now() + UPDATE_WITHIN
    while (Date.now() < until) {
      await new Promise((done) => setTimeout(done, 3000))
      const now = await api("POST", "/api/devices/versions", {}).catch(() => null)
      if (now?.[key] && !now[key].behind) return true
    }
    return false
  }

  /// A dot tapped: its problem opened or closed, or its part switched.
  function tapCell(key) {
    const ci = key.slice(0, key.indexOf(" ")), name = key.slice(key.indexOf(" ") + 1)
    const x = m.gridColumns()[ci], p = x?.parts.find((q) => q.name === name)
    if (!p) return
    if (x.c && m.problemOf(x.c, p)) return setCellOpen(cellOpen === key ? "" : key)
    if (p.fixed || !p.part) return
    one.tap()
    act(() => setPart(p.part, !p.on))
  }

  const refuse = async (code, run) => {
    one.tap()
    setBusyCode(code)
    await act(run)
    setBusyCode("")
  }

  const tabs = (
    <Tabs value={tab} onValueChange={setTab}>
      <TabsList className="max-w-full justify-start overflow-x-auto">
        <TabsTrigger value="devices">Devices</TabsTrigger>
        <TabsTrigger value="priority">Run Priority</TabsTrigger>
        <TabsTrigger value="services">Enabled Services</TabsTrigger>
      </TabsList>
    </Tabs>
  )
  let body
  if (!data) {
    body = problem ? <LoadFailure problem={problem} retry={() => load()} />
      : <>{tabs}<ListSkeleton rows={3} /></>
  } else {
    const { waiting, device } = data
    const joined = m.justJoined()
    body = (
      <>
        {!!waiting.length && (
          <Section title="Waiting for you">
            {waiting.map((r) => <Waiting key={r.code} r={r} busy={busyCode === r.code} answer={refuse} />)}
          </Section>
        )}
        {joined.map((c) => <Joined key={c.key} c={c} remove={() => remove(c.name, [c.key], null)} />)}
        {scan?.error && <Problem title={scan.error} />}
        {scan?.r && <Scanned scan={scan} done={() => (setScan(null), load())} cancel={() => setScan(null)} />}
        {tabs}
        {tab === "priority" ? (
          <Priority m={m} move={moveMenu} />
        ) : tab === "services" ? (
          <Services m={m} cellOpen={cellOpen} tap={tapCell} />
        ) : (
          <DevicesTab
            m={m}
            data={data}
            opened={opened}
            flag={(id) => setOpened((o) => ({ ...o, [id]: !o[id] }))}
            menu={menu}
            removeOld={removeOld}
            signOut={signOut}
            schedule={() => show({ schedules: {} })}
            restore={() => show({ restore: {} })}
            act={act}
            updating={updating}
            update={update}
          />
        )}
        {error && <Problem title={error} />}
      </>
    )
  }

  return (
    <div className="@container flex flex-col gap-6">
      <div className="max-w-5xl"><LargeTitle action={<Button variant={data?.waiting.length ? "outline" : "default"} disabled={!data} onClick={() => show({ add: true })}>Add Device</Button>}>Devices</LargeTitle></div>
      {body}
      {data && dialog.for?.rename && <Rename key={dialog.n} card={dialog.for.rename} open={dialog.open} setOpen={setDialogOpen} done={() => (setDialogOpen(false), load())} />}
      {data && dialog.for?.remove && <RemoveDevice key={dialog.n} {...dialog.for.remove} open={dialog.open} setOpen={setDialogOpen} done={() => (setDialogOpen(false), load())} />}
      {data && dialog.for?.signIns && <SignIns key={dialog.n} clients={dialog.for.signIns} me={dialog.for.me} open={dialog.open} setOpen={setDialogOpen} done={() => (setDialogOpen(false), load())} />}
      {data && dialog.for?.add && <AddDevice key={dialog.n} data={data} open={dialog.open} setOpen={setDialogOpen} />}
      {data && dialog.for?.restore && <Restore key={dialog.n} open={dialog.open} setOpen={setDialogOpen} done={() => (setDialogOpen(false), load())} />}
      {data && dialog.for?.schedules && <AppSchedules key={dialog.n} m={m} asked={dialog.for.schedules} open={dialog.open} setOpen={setDialogOpen} done={() => (setDialogOpen(false), load())} />}
    </div>
  )
}

const latest = { current: { place() {}, moveMenu() {} } }

// A heading over a part of the page.
const Note = ({ children }) => <p className="max-w-2xl text-sm text-muted-foreground">{children}</p>

// Cloudflare's own logo, its orange cloud.
function Cloudflare({ className }) {
  return (
    <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
      <path fill="#F38020" d="M16.5088 16.8447c.1475-.5068.0908-.9707-.1553-1.3154-.2246-.3164-.6045-.499-1.0615-.5205l-8.6592-.1123a.1559.1559 0 0 1-.1333-.0713c-.0283-.042-.0351-.0986-.021-.1553.0278-.084.1123-.1484.2036-.1562l8.7359-.1123c1.0351-.0489 2.1601-.8868 2.5537-1.9136l.499-1.3013c.0215-.0561.0293-.1128.0147-.168-.5625-2.5463-2.835-4.4453-5.5499-4.4453-2.5039 0-4.6284 1.6177-5.3876 3.8614-.4927-.3658-1.1187-.5625-1.794-.499-1.2026.119-2.1665 1.083-2.2861 2.2856-.0283.31-.0069.6128.0635.894C1.5683 13.171 0 14.7754 0 16.752c0 .1748.0142.3515.0352.5273.0141.083.0844.1475.1689.1475h15.9814c.0909 0 .1758-.0645.2032-.1553l.12-.4268z" />
      <path fill="#FBAE40" d="M19.2656 11.2813c-.0771 0-.1611 0-.2383.0112-.0566 0-.1054.0415-.127.0976l-.3378 1.1744c-.1475.5068-.0918.9707.1543 1.3164.2256.3164.6055.498 1.0625.5195l1.8437.1133c.0557 0 .1055.0263.1329.0703.0283.043.0351.1074.0214.1562-.0283.084-.1132.1485-.204.1553l-1.921.1123c-1.041.0488-2.1582.8867-2.5527 1.914l-.1406.3585c-.0283.0713.0215.1416.0986.1416h6.5977c.0771 0 .1474-.0489.169-.126.1122-.4082.1757-.837.1757-1.2803 0-2.6025-2.125-4.727-4.7344-4.727" />
    </svg>
  )
}

// A device's picture: its type, with its OS's glyph at its corner, nothing behind the glyph; dimmed while it's off.
function DeviceIcon({ icon, className }) {
  if (icon.cloud) return <span className={cn("grid size-8 shrink-0 place-items-center", className)}><Cloudflare className="size-8" /></span>
  return (
    <span className={cn("relative grid size-8 shrink-0 place-items-center", icon.off && "text-muted-foreground", className)}>
      <Icon name={icon.type} className="size-7" />
      {icon.os && <Icon name={icon.os} className="absolute -right-1.5 -bottom-1 size-4" />}
    </span>
  )
}

// A row: a device's picture, its name and one grey line, and what it ends with.
function DeviceRow({ icon, title, sub, children }) {
  return (
    <div className="flex min-w-0 items-center gap-3 px-3 py-2.5 text-sm">
      <DeviceIcon icon={icon} />
      <span className="min-w-0 flex-1">
        <span className="block truncate font-medium">{title}</span>
        {sub && <span className="block truncate text-muted-foreground">{sub}</span>}
      </span>
      {children}
    </div>
  )
}

// A device waiting to be let in: which it is, where from, what approving it records it signed in with, its code to
// check, and the answers. Approve is the one that matters.
function Waiting({ r, busy, answer }) {
  const sub = [r.yours ? "" : r.lost ? `${r.user}, with no device left` : `${r.user}’s first device`, r.place].filter(Boolean).join(" · ")
  return (
    <div role="group" aria-label={`${r.device} wants to join`} className="flex w-full max-w-2xl flex-col gap-4 rounded-xl border bg-card p-4 text-card-foreground @xl:flex-row @xl:items-center">
      <div className="flex min-w-0 flex-1 items-center gap-3 text-sm">
        <DeviceIcon icon={iconOf({ name: r.device, platform: r.platform })} />
        <span className="min-w-0 flex-1">
          <span className="block font-medium break-words">{r.device}</span>
          {sub && <span className="block break-words text-muted-foreground">{sub}</span>}
          {signInLabel(r.sign_in) && <span className="block break-words text-muted-foreground">{signInLabel(r.sign_in)}</span>}
          {r.replaces && <span className="block break-words text-muted-foreground">Replaces {r.replaces.join(", ")}</span>}
          {r.by_link && <span className="block break-words text-muted-foreground">Signed in from a link, not on this device. Approve only if you started this.</span>}
        </span>
      </div>
      <div className="text-center font-mono text-3xl font-semibold tracking-widest select-all">
        <div>{r.code.slice(0, 9)}</div>
        {r.code.length > 9 && <div>{r.code.slice(10)}</div>}
      </div>
      <div className="flex gap-2 @xl:justify-end">
        <Button variant="outline" className="flex-1 @xl:flex-none" disabled={busy} onClick={() => answer(r.code, () => api("POST", "/api/devices/refuse", { code: r.code }))}>{r.yours ? "Not Me" : "Not Them"}</Button>
        <Button className="flex-1 @xl:flex-none" disabled={busy} onClick={() => answer(r.code, () => api("POST", "/api/devices/approve", { code: r.code, confirm: true }))}>
          {busy && <Spinner className="text-primary-foreground" />}Approve
        </Button>
      </div>
    </div>
  )
}

// A device of theirs let in within the day: "MacBook Pro joined · Sydney", with Remove, in case it isn't theirs.
function Joined({ c, remove }) {
  return (
    <List>
      <DeviceRow icon={iconOf(c)} title={`${c.name} joined`} sub={[c.place, ago(c.since)].filter(Boolean).join(" · ")}>
        <Button variant="ghost" size="sm" className="text-destructive" onClick={remove}>Remove</Button>
      </DeviceRow>
    </List>
  )
}

// A device's code from its QR, not waiting at the top: which device it is, and Approve.
function Scanned({ scan, done, cancel }) {
  const { code, r } = scan
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  async function approve() {
    one.tap()
    setBusy(true)
    setError("")
    try {
      await api("POST", "/api/devices/approve", { code, confirm: true })
      done()
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }
  return (
    <div role="group" aria-label={`Approve ${r.device}?`} className="flex w-full max-w-2xl flex-col gap-3 rounded-xl border bg-card p-4 text-card-foreground">
      <div>
        <div className="font-medium">Approve {r.device}?</div>
        <div className="text-sm text-muted-foreground">{r.new ? `${r.user}’s first device. They join ${yours}.` : r.user === undefined || r.yours ? "It can open everything you can." : `${r.user}’s device.`}</div>
        {signInLabel(r.sign_in) && <div className="text-sm text-muted-foreground">{signInLabel(r.sign_in)}</div>}
        {r.replaces && <div className="text-sm text-muted-foreground">Replaces {r.replaces.join(", ")}</div>}
      </div>
      {error && <Problem title={error} />}
      <div className="flex justify-end gap-2">
        <Button variant="outline" disabled={busy} onClick={cancel}>Cancel</Button>
        <Button disabled={busy} onClick={approve}>{busy && <Spinner className="text-primary-foreground" />}Approve</Button>
      </div>
    </div>
  )
}

// ---- Devices ----

// A phone's app comes from its store, so it says so instead of offering Update.
const STORE = { iphone: "the App Store", android: "Google Play" }
// A device's version, as a grey line says it.
const versionLine = (c) => {
  const store = STORE[c.signIn?.platform]
  return [c.version && `Version ${c.version.version.slice(0, 7)}`, store && `Updates from ${store}`].filter(Boolean).join(" · ")
}

function DevicesTab({ m, data, opened, flag, menu, removeOld, signOut, schedule, restore, act, updating, update }) {
  const { device, state } = data
  const { cards, passkeys, stale } = m.drawn
  const mine = cards.filter((c) => c.user === m.me)
  // App schedules, only for someone with some running, or an app that runs on its own: an app's screen asks for the
  // rest.
  const appSchedules = device.keys ? (state.network?.app_schedules || []).filter((g) => g.until > Date.now()) : []
  const scheduling = !!device.keys && (appSchedules.length > 0 || m.scheduled().length > 0)
  const recovery = []
  if (state.network?.recovery_used && (m.admin || state.network.recovery_used.user === m.me)) {
    const r = state.network.recovery_used
    recovery.push(`The recovery key last let in a device of ${r.user === m.me ? "yours" : `${m.userName(r.user)}’s`}, ${day(r.time)}.`)
  }
  const holder = !!device.keys && !!state.network?.users?.find((u) => u.me)?.recovery_holder
  const side = scheduling || passkeys.length > 0 || recovery.length > 0 || holder
  // Those a newer version is published for and that can hear the ask, this device first: a bad version is best found on the machine in front of you.
  const behind = mine.filter((c) => c.node && c.online && c.version?.behind).sort((a, b) => b.me - a.me)
  // Cloudflare has one version for what's at the network's address, and its admins update it, last: it deploys with what its computer has.
  const cloudflare = data.versions.cloudflare
  const cloudflareCard = m.admin && cloudflare?.behind ? { name: "Cloudflare", node: { key: "cloudflare" } } : null
  const all = cloudflareCard ? [...behind, cloudflareCard] : behind
  return (
    <div className={cn("grid max-w-5xl items-start gap-x-8 gap-y-8", side && "@3xl:grid-cols-[minmax(0,1.3fr)_minmax(0,1fr)]")}>
      <div className="flex min-w-0 flex-col gap-4">
        {all.length > 1 && (
          <Button variant="outline" className="self-start" disabled={Object.keys(updating).length > 0} onClick={() => update(all)}>Update All</Button>
        )}
        {!!mine.length && (
          <List>
            {mine.map((c) => {
              const twin = mine.some((o) => o !== c && o.name === c.name)
              const sub = c.me ? "This device" : c.seen && Date.now() - c.seen > 3_600_000 ? ago(c.seen) : twin && c.signIn?.since ? `Joined ${ago(c.signIn.since)}` : ""
              const problems = m.problemsOf(c), open = problems.length > 0 && opened[c.id]
              return (
                <div
                  key={c.id}
                  onContextMenu={(e) => (e.preventDefault(), menu(c, false, { x: e.clientX, y: e.clientY }))}
                >
                  <DeviceRow icon={c.icon} title={c.name} sub={[sub, versionLine(c)].filter(Boolean).join(" · ")}>
                    {behind.includes(c) && (
                      <Button variant="outline" size="sm" disabled={Object.keys(updating).length > 0} onClick={() => update([c])}>
                        {updating[c.node.key] && <Spinner />}Update
                      </Button>
                    )}
                    {!!problems.length && (
                      <Button
                        variant="ghost"
                        size="icon-sm"
                        className="text-warning"
                        aria-expanded={!!open}
                        aria-label={`${problems.length > 1 ? `${problems.length} problems` : "A problem"} on ${c.name}`}
                        onClick={() => flag(c.id)}
                      >
                        <Icon name="warning-fill" className="size-5" />
                      </Button>
                    )}
                    <More name={c.name} onClick={(e) => menu(c, false, e.currentTarget)} />
                  </DeviceRow>
                  {open && problems.map((p, i) => <div key={i} className="px-3 pb-3 pl-14"><Problem warn {...p} /></div>)}
                  <OldSignIns c={c} remove={() => removeOld(c)} />
                </div>
              )
            })}
          </List>
        )}
        {cloudflare && (
          <List>
            <DeviceRow icon={CLOUDFLARE} title="Cloudflare" sub={[`Version ${cloudflare.version.slice(0, 7)}`, cloudflare.node && cloudflare.node !== cloudflare.version && `Apps run on ${cloudflare.node.slice(0, 7)}`].filter(Boolean).join(" · ")}>
              {cloudflareCard && (
                <Button variant="outline" size="sm" disabled={Object.keys(updating).length > 0} onClick={() => update([cloudflareCard])}>
                  {updating.cloudflare && <Spinner />}Update
                </Button>
              )}
            </DeviceRow>
          </List>
        )}
        {!!stale.length && <List><OldSignIns c={{ old: stale }} remove={() => removeOld({ old: stale })} /></List>}
        {/* A device that isn’t listed yet still signs out. */}
        {device.signOut && !cards.some((c) => c.me) && (
          <Button variant="ghost" className="self-start text-destructive" onClick={signOut}>Sign Out</Button>
        )}
        {/* This browser, or the app, gets notifications once its user says so here: asked only when they tap it. */}
        {device.notify === "default" && (
          <Button variant="ghost" className="self-start" onClick={() => act(async () => (device.notify = await one.notify()))}>
            <Icon name="bell" />Get Notifications Here
          </Button>
        )}
      </div>
      {!!side && (
        <div className="flex min-w-0 flex-col gap-8">
          {scheduling && (
            <Section title="App Schedules">
              <List>
                {appSchedules.map((g) => {
                  const on = (state.network?.devices || []).find((d) => deviceKey(d) === g.endpoint)
                  const until = new Date(g.until).toLocaleDateString(undefined, { day: "numeric", month: "long" })
                  return (
                    <div key={`${g.endpoint} ${g.app} ${g.scope}`} className="flex items-center gap-3 px-3 py-2.5 text-sm">
                      <span className="min-w-0 flex-1">
                        <span className="block font-medium">{(on && shown(on.name)) || "A device"} runs {m.appTitle(g.app)}’s schedules for {m.forWhom(g.scope)}</span>
                        <span className="block text-muted-foreground">Until {until}, and longer as {g.scope === `user:${m.me}` ? "you use it" : "it’s used"}</span>
                      </span>
                      <Button variant="ghost" size="sm" className="text-destructive" onClick={() => act(() => api("POST", "/api/devices/stop_schedules", { device: g.endpoint, app: g.app, scope: g.scope }))}>Stop</Button>
                    </div>
                  )
                })}
                <ListAdd icon={<Icon name="arrows-clockwise" />} onClick={schedule}>Run an App’s Schedules</ListAdd>
              </List>
              {!!appSchedules.length && <Note>Once one stops, that device can’t open the app’s data.</Note>}
            </Section>
          )}
          {!!passkeys.length && (
            <Section title="Passkeys">
              <List>
                {passkeys.map((p) => (
                  <div key={p.key} onContextMenu={(e) => (e.preventDefault(), menu(p, true, { x: e.clientX, y: e.clientY }))}>
                    <DeviceRow icon={glyph("key")} title={p.name} sub={data.seen[p.key] && Date.now() - data.seen[p.key] > 3_600_000 ? ago(data.seen[p.key]) : ""}>
                      <More name={p.name} onClick={(e) => menu(p, true, e.currentTarget)} />
                    </DeviceRow>
                  </div>
                ))}
              </List>
            </Section>
          )}
          {holder && (
            <List>
              <ListAdd icon={<Icon name="key" />} onClick={restore}>Restore Someone</ListAdd>
            </List>
          )}
          {recovery.length > 0 && <div className="flex flex-col gap-2">{recovery.map((line) => <Note key={line}>{line}</Note>)}</div>}
        </div>
      )}
    </div>
  )
}

const More = ({ name, onClick }) => (
  <Button variant="ghost" size="icon-sm" aria-label={`More for ${name}`} onClick={onClick}><Icon name="dots-three-outline-fill" className="size-5" /></Button>
)

const OldSignIns = ({ c, remove }) =>
  c.old.length > 0 ? (
    <div className="flex items-center gap-3 px-3 py-1.5 pl-14 text-sm text-muted-foreground">
      <span className="flex-1">{c.old.length} old sign-in{c.old.length > 1 ? "s" : ""}</span>
      <Button variant="ghost" size="sm" className="text-destructive" onClick={remove}>Remove</Button>
    </div>
  ) : null

// ---- Priority ----

/// Which devices run apps first, as columns, first to last: stacked on a phone, side by side where there's room, each
/// column taking its share of the width. An admin moves a device by dragging it, or by its menu.
function Priority({ m, move }) {
  const { cols, last } = m.board()
  const can = m.admin
  const card = (e, where) => {
    const inner = (
      <>
        <DeviceIcon icon={e.icon} />
        <span className="min-w-0 flex-1 text-left">
          <span className="block truncate font-medium">{e.name}</span>
          {e.whose && <span className="block truncate text-muted-foreground">{e.whose}</span>}
        </span>
      </>
    )
    const look = "flex w-full items-center gap-3 rounded-lg border bg-background px-3 py-2 text-sm"
    return can ? (
      <button
        key={e.id}
        data-pcard={e.id}
        aria-haspopup="menu"
        aria-label={`${e.name}${e.whose ? `, ${e.whose}` : ""}, ${where}. Move`}
        className={cn(look, "cursor-grab touch-pan-y select-none [-webkit-touch-callout:none]")}
        onClick={(ev) => (quiet.now ? (quiet.now = false) : move(e.id, ev.currentTarget))}
      >
        {inner}
        <span data-grip className="shrink-0 touch-none text-muted-foreground"><Icon name="dots-six-vertical" className="size-5" /></span>
      </button>
    ) : (
      <div key={e.id} className={look}>{inner}</div>
    )
  }
  const col = (id, title, list) => (
    <section key={id} data-col={id} aria-label={title} className="flex min-h-24 flex-1 basis-0 flex-col gap-2 rounded-xl border bg-card p-2 @2xl:min-h-44 @2xl:min-w-40">
      <h3 className="px-1 pt-1 text-sm font-medium">{title}</h3>
      {list.map((e) => card(e, title))}
    </section>
  )
  const then = (key) => <div key={key} aria-hidden="true" className="grid h-6 shrink-0 place-items-center text-muted-foreground @2xl:h-auto @2xl:w-6"><Icon name="caret-right" className="size-5 rotate-90 @2xl:rotate-0" /></div>
  const parts = cols.map((c, i) => col(i, ordinal(i + 1), c))
  if (can) parts.push(
    <section key="new" data-col="new" aria-label="New" className="grid min-h-12 place-items-center rounded-xl border border-dashed p-2 text-muted-foreground @2xl:w-16 @2xl:flex-none">
      <Icon name="plus" className="size-5" />
    </section>
  )
  if (can || last.length) parts.push(col("last", "Last", last))
  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-xl font-semibold tracking-tight">Which devices run your apps first?</h2>
      <div className="flex flex-col @2xl:flex-row @2xl:items-stretch">
        {parts.flatMap((p, i) => (i ? [then(`then${i}`), p] : [p]))}
      </div>
    </div>
  )
}

// The click that ends a drag or a long press isn't a tap.
const quiet = { now: false }

// Dragging a device between columns: by its grip, or anywhere on it with a mouse. Pressed and held on a touch screen, it
// gives the Move menu instead. `dragging` is true while one is lifted, so the page isn't redrawn under it.
function useDrag(dragging, place, menu) {
  useEffect(() => {
    let drag = null
    const down = (e) => {
      quiet.now = false
      const card = e.target.closest?.("[data-pcard]")
      if (!card || e.button !== 0) return
      const now = e.pointerType === "mouse" || !!e.target.closest("[data-grip]")
      drag = { card, id: card.dataset.pcard, x: e.clientX, y: e.clientY, now, lifted: null, over: null }
      if (!now) drag.press = setTimeout(() => {
        const d = drag
        drag = null
        if (d) (quiet.now = true), menu(d.id, d.card)
      }, 500)
    }
    const move = (e) => {
      if (!drag) return
      if (!drag.lifted) {
        if (Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 6) return
        clearTimeout(drag.press)
        if (!drag.now) return void (drag = null)
        const r = drag.card.getBoundingClientRect()
        const el = drag.card.cloneNode(true)
        el.classList.add("fixed", "z-[60]", "m-0", "pointer-events-none", "shadow-lg", "-rotate-1")
        Object.assign(el.style, { width: `${r.width}px`, left: `${r.left}px`, top: `${r.top}px` })
        document.body.append(el)
        Object.assign(drag, { lifted: el, dx: drag.x - r.left, dy: drag.y - r.top })
        drag.card.classList.add("opacity-30")
        dragging.current = true
        one.tap()
      }
      e.preventDefault()
      Object.assign(drag.lifted.style, { left: `${e.clientX - drag.dx}px`, top: `${e.clientY - drag.dy}px` })
      const over = document.elementFromPoint(e.clientX, e.clientY)?.closest("[data-col]") || null
      if (over !== drag.over) drag.over?.classList.remove("ring-2", "ring-ring"), over?.classList.add("ring-2", "ring-ring"), (drag.over = over)
      if (e.clientY < 60) scrollBy(0, -12)
      else if (e.clientY > innerHeight - 60) scrollBy(0, 12)
    }
    const up = (e) => {
      const d = drag
      drag = null
      if (!d) return
      clearTimeout(d.press)
      if (!d.lifted) return
      d.lifted.remove()
      d.card.classList.remove("opacity-30")
      d.over?.classList.remove("ring-2", "ring-ring")
      dragging.current = false
      quiet.now = true
      if (e.type === "pointerup" && d.over) place(d.id, d.over.dataset.col)
    }
    const context = (e) => {
      const card = e.target.closest?.("[data-pcard]")
      if (card) e.preventDefault(), (drag = null), menu(card.dataset.pcard, { x: e.clientX, y: e.clientY })
    }
    const keys = () => (quiet.now = false)
    document.addEventListener("pointerdown", down)
    document.addEventListener("pointermove", move, { passive: false })
    document.addEventListener("pointerup", up)
    document.addEventListener("pointercancel", up)
    document.addEventListener("keydown", keys)
    document.addEventListener("contextmenu", context)
    return () => {
      document.removeEventListener("pointerdown", down)
      document.removeEventListener("pointermove", move)
      document.removeEventListener("pointerup", up)
      document.removeEventListener("pointercancel", up)
      document.removeEventListener("keydown", keys)
      document.removeEventListener("contextmenu", context)
    }
  }, [])
}

// ---- Services ----

/// What runs where, services down and devices across, a dot each: on a phone it scrolls sideways, the names staying.
function Services({ m, cellOpen, tap }) {
  const cols = m.gridColumns()
  if (!cols.length) return <Empty>None of your devices runs services.</Empty>
  const titles = {}
  for (const p of cols.flatMap((x) => x.parts)) titles[p.name] ||= m.partTitle(p)
  const names = Object.keys(titles).sort((a, b) => (b === "apps") - (a === "apps") || titles[a].localeCompare(titles[b]))
  // Services whose apps declare they run as the user: said, never silently.
  const asYou = names.filter((n) => cols.some((x) => x.parts.some((p) => p.name === n && p.on && p.user))).map((n) => titles[n])
  const icon = (name) => (name === "apps" ? { name: "apps", title: "Runs apps", icon: "squares-four", color: "#007aff" } : { ...(m.appOf(name) || {}), name, title: titles[name] })
  let open = null
  const cell = (x, ci, name) => {
    const p = x.parts.find((q) => q.name === name)
    if (!p) return <td key={ci} />
    const where = `${titles[name]} on ${x.name}`, key = `${ci} ${name}`
    const problem = x.c && m.problemOf(x.c, p)
    if (problem && cellOpen === key) open = problem
    const fixed = p.fixed || !p.part
    const said = problem ? `${where}: ${problem.title}` : `${where}${p.doing ? `: ${p.doing}…` : ""}`
    return (
      <td key={ci} className="px-1 text-center">
        <button
          className={cn("inline-grid h-10 w-11 place-items-center rounded-md outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50", fixed && !problem ? "cursor-default" : "cursor-pointer")}
          {...(problem ? { "aria-expanded": cellOpen === key } : { role: "switch", "aria-checked": !!p.on })}
          aria-disabled={fixed && !problem ? true : undefined}
          aria-label={said}
          onClick={() => tap(key)}
        >
          <span
            aria-hidden="true"
            className={cn(
              "grid size-5 place-items-center rounded-full border-2 text-[13px] font-extrabold",
              problem ? "border-warning bg-warning text-background" : p.on ? "border-foreground bg-foreground" : "border-muted-foreground/40",
              p.doing && !problem && "animate-pulse motion-reduce:animate-none",
            )}
          >
            {problem ? "!" : ""}
          </span>
        </button>
      </td>
    )
  }
  const rows = names.map((n) => (
    <tr key={n}>
      <th scope="row" className="sticky left-0 z-10 bg-card py-2 pr-3 pl-3 text-left font-medium">
        <span className="flex max-w-40 items-center gap-2.5"><AppIcon app={icon(n)} size={28} /><span className="truncate">{titles[n]}</span></span>
      </th>
      {cols.map((x, ci) => cell(x, ci, n))}
    </tr>
  ))
  return (
    <div className="flex flex-col gap-3">
      <div className="max-w-full w-fit overflow-x-auto rounded-xl border bg-card text-sm text-card-foreground">
        <table className="min-w-full border-separate border-spacing-0">
          <thead>
            <tr>
              <th className="sticky left-0 z-10 bg-card" />
              {cols.map((x, ci) => (
                <th key={ci} scope="col" className="min-w-24 px-1 pt-3.5 pb-1.5 align-bottom text-[13px] font-medium text-muted-foreground">
                  <DeviceIcon icon={x.icon} className="mx-auto mb-1.5" />
                  <span className="mx-auto block max-w-24 truncate">{x.name}</span>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>{rows}</tbody>
        </table>
      </div>
      {open && <Problem warn {...open} />}
      {asYou.length > 0 && <Hint>{list(asYou)} {asYou.length > 1 ? "run" : "runs"} as you, with your files.</Hint>}
    </div>
  )
}

/// Names as a user says them: "A, B and C".
const list = (names) => (names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : names[0] || "")

// ---- Dialogs ----

// A dialog's one button, in its head, with a spinner while it works.
const Go = ({ children, busy, disabled, variant = "default", onClick }) => (
  <Button size="sm" variant={variant} disabled={busy || disabled} onClick={onClick}>{busy && <Spinner className="text-current" />}{children}</Button>
)
const Cancel = ({ setOpen }) => <Button variant="ghost" size="sm" onClick={() => setOpen(false)}>Cancel</Button>

/// For someone holding the recovery key: gives the device showing a code the keys of a user who lost every device. Says
/// whose it is, then does it.
function Restore({ open, setOpen, done }) {
  const [code, setCode] = useState("")
  const [asked, setAsked] = useState("")
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  async function go() {
    if (!code.trim() || busy) return
    setBusy(true)
    setError("")
    try {
      if (!asked) {
        setAsked((await api("POST", "/api/devices/restore", { code })).said)
        setBusy(false)
        return
      }
      await api("POST", "/api/devices/restore", { code, confirm: true })
      one.tap("success")
      done()
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="Restore Someone" action={<Go busy={busy} disabled={!code.trim()} onClick={go}>{asked ? "Restore" : "Next"}</Go>}>
      <div className="flex flex-col gap-4 p-4">
        <Input autoFocus value={code} onChange={(e) => (setCode(e.target.value), setAsked(""))} placeholder="Code on their new device" aria-label="Code on their new device" autoComplete="off" spellCheck={false} enterKeyHint="done" onKeyDown={(e) => e.key === "Enter" && go()} />
        {asked && <Note>{asked}</Note>}
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

/// Renames a device: its node, or its app's sign-ins.
function Rename({ card, open, setOpen, done }) {
  const keys = card.runs ? [deviceKey(card.node)] : card.clients.map((c) => c.key)
  const [name, setName] = useState(card.name)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  const ok = !!name.trim() && name.trim() !== card.name
  async function save() {
    if (!ok || busy) return
    setBusy(true)
    setError("")
    try {
      for (const key of keys) await api("POST", "/api/name", { key, name: name.trim() })
      one.tap("success")
      done()
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="Rename" action={<Go busy={busy} disabled={!ok} onClick={save}>Save</Go>}>
      <div className="flex flex-col gap-4 p-4">
        <Input autoFocus value={name} onChange={(e) => setName(e.target.value)} aria-label="Name" autoComplete="off" spellCheck={false} enterKeyHint="done" onKeyDown={(e) => e.key === "Enter" && save()} />
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

/// Removes a device that's on, and so can erase what it keeps first: asked in a dialog, since there's a choice.
function RemoveDevice({ name, keys, erasable, open, setOpen, done }) {
  const [erase, setErase] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState("")
  async function go() {
    setBusy(true)
    setError("")
    try {
      if (erase) await api("POST", "/api/erase", { key: erasable })
      for (const key of keys) await api("POST", "/api/devices/remove", { device: key })
      one.tap("success")
      done()
    } catch (e) {
      setError(said(e))
      setBusy(false)
    }
  }
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title={`Remove ${name}?`} action={<><Cancel setOpen={setOpen} /><Go variant="destructive" busy={busy} onClick={go}>Remove</Go></>}>
      <div className="flex flex-col gap-4 p-4">
        <List>
          <Label className="justify-between px-3 py-3 font-normal">Also erase its data<Switch checked={erase} onCheckedChange={setErase} /></Label>
        </List>
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

/// What a device's sign-ins signed in with, and what its user linked from it. Each can be removed, on every device of
/// theirs and at auth. This device links another in a page at the network's address, in a browser; coming back, what it
/// linked is shown, and recorded only once they say so, as auth could have made it.
function SignIns({ clients, me, open, setOpen, done }) {
  const [busy, setBusy] = useState(null)
  const [error, setError] = useState("")
  const [linked, setLinked] = useState(null)
  const rows = clients.flatMap((c) => [
    ...(c.sign_in ? [{ id: `${c.key}.`, key: c.key, label: signInLabel(c.sign_in) }] : []),
    ...(c.linked || []).map((a, n) => ({ id: `${c.key}.${n}`, key: c.key, account: a, label: signInLabel(a) })),
  ])
  async function forget(r) {
    setBusy(r.id)
    setError("")
    try {
      await api("POST", "/api/devices/forget-sign-in", { key: r.key, ...(r.account ? { account: r.account } : {}) })
      one.tap("success")
      done()
    } catch (e) {
      setError(said(e))
      setBusy(null)
    }
  }
  const look = async () => {
    try {
      setLinked((await api("POST", "/api/devices/record-sign-in", {})).account || null)
    } catch (e) { setError(said(e)) }
  }
  async function keep() {
    setBusy("linked")
    setError("")
    try {
      await api("POST", "/api/devices/record-sign-in", { confirm: true })
      one.tap("success")
      setLinked(null)
      done()
    } catch (e) { setError(said(e)) }
    setBusy(null)
  }
  useEffect(() => {
    if (!me || !open) return
    look()
    const back = () => document.visibilityState === "visible" && look()
    document.addEventListener("visibilitychange", back)
    return () => document.removeEventListener("visibilitychange", back)
  }, [me, open])
  // Asked for as the dialog opens, so the button is a plain link, which every browser and app opens.
  const [url, setUrl] = useState("")
  useEffect(() => {
    if (me && open) api("POST", "/api/devices/link-sign-in", {}).then((r) => setUrl(r.url), (e) => setError(said(e)))
  }, [me, open])
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="Sign-Ins" action={<Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Done</Button>}>
      <div className="flex flex-col gap-4 overflow-y-auto p-4">
        {!!rows.length && <List>
          {rows.map((r) => (
            <div key={r.id} className="flex items-center gap-3 px-3 py-2 text-sm">
              <span className="min-w-0 flex-1 truncate font-medium">{r.label}</span>
              <Button variant="ghost" size="sm" className="text-destructive" disabled={busy !== null} onClick={() => forget(r)}>Remove</Button>
            </div>
          ))}
        </List>}
        {linked && <div className="flex items-center gap-3 rounded-xl border p-3 text-sm">
          <span className="min-w-0 flex-1 truncate font-medium">{signInLabel(linked)}</span>
          <Button size="sm" disabled={busy !== null} onClick={keep}>Link It</Button>
        </div>}
        {me && url && <Button variant="secondary" asChild><a href={url} target="_blank" rel="noopener">Link a Sign-In</a></Button>}
        {error && <Problem title={error} />}
      </div>
    </ResponsiveDialog>
  )
}

// A line to copy, as it is: its text selects at a tap, and its button copies it.
function Line({ text }) {
  const [copied, setCopied] = useState(false)
  async function copy() {
    await one.copy(text)
    one.tap("success")
    setCopied(true)
    setTimeout(() => setCopied(false), 1500)
  }
  return (
    <div className="mt-2 flex items-center gap-1 rounded-lg bg-muted py-1 pr-1 pl-3">
      <span className="min-w-0 flex-1 overflow-x-auto font-mono text-[13px] whitespace-nowrap select-all">{text}</span>
      <Button variant="ghost" size="icon-sm" aria-label={copied ? "Copied" : "Copy"} onClick={copy}><Icon name={copied ? "check" : "copy"} /></Button>
    </div>
  )
}

/// The ways a device joins: a browser, a computer or a server by one line (the computer's installs the app, which signs
/// in and asks to be approved).
function AddDevice({ data, open, setOpen }) {
  const { state } = data
  const address = state.network?.settings?.address || ""
  // A server's line holds a code made when this opens.
  const [line, setLine] = useState(state.config?.install ? { loading: true } : null)
  useEffect(() => {
    if (!state.config?.install) return
    api("POST", "/api/pair", {}).then(
      ({ code }) => setLine({ text: `${state.config.install} && ~/.local/bin/${state.config.cli} join ${code}` }),
      (err) => setLine({ error: said(err) }),
    )
  }, [])
  const way = (icon, title, sub, extra) => (
    <div key={title} className="flex items-start gap-3 px-3 py-3 text-sm">
      <Icon name={icon} className="mt-0.5 size-6 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <div className="font-medium">{title}</div>
        <div className="text-muted-foreground">{sub}</div>
        {extra}
      </div>
    </div>
  )
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title="Add Device" action={<Button variant="secondary" size="sm" onClick={() => setOpen(false)}>Done</Button>}>
      <div className="overflow-y-auto p-4">
        <List>
          {address && way("browser", "A browser", "Open this and sign in. It shows up here to approve.", <Line text={address} />)}
          {state.config?.install && way("desktop", "A computer", `Run this on a Mac or Linux computer, then sign in. It shows up here to approve.`, <Line text={state.config.install} />)}
          {line && way("hard-drives", "A server", "Run this on it. It joins as you, with everything you can see and do.",
            line.loading ? <div className="mt-2 flex items-center gap-2 text-muted-foreground"><Spinner />Making a code</div>
              : line.error ? <div className="mt-2"><Problem title={line.error} /></div>
              : <Line text={line.text} />)}
        </List>
      </div>
    </ResponsiveDialog>
  )
}

/// Runs which app's (unless it's given) schedules on which device, asked, then set.
function AppSchedules({ m, asked, open, setOpen, done }) {
  const scope = asked.scope || `user:${m.me}`
  const [app, setApp] = useState(asked.app || null)
  const [error, setError] = useState("")
  const rows = (items, footer) => (
    <div className="flex flex-col gap-4 overflow-y-auto p-4">
      {items.length ? <List>{items}</List> : <Empty className="py-6">None yet</Empty>}
      {footer && <Note>{footer}</Note>}
      {error && <Problem title={error} />}
    </div>
  )
  async function choose(d) {
    setError("")
    try {
      await api("POST", "/api/devices/run_schedules", { device: deviceKey(d), app, scope })
      done()
    } catch (err) {
      setError(said(err))
    }
  }
  return (
    <ResponsiveDialog open={open} onOpenChange={setOpen} title={app ? `Run ${m.appTitle(app)}’s Schedules On…` : "Run an App’s Schedules"} action={<Cancel setOpen={setOpen} />}>
      {app
        ? rows(m.runners(scope).map((d) => <ListRow key={d.key} title={shown(d.name)} onClick={() => choose(d)} />), "Choose one that stays on.")
        : rows(m.scheduled(scope).map((a) => <ListRow key={a.name} title={a.title || a.name} onClick={() => setApp(a.name)} />))}
    </ResponsiveDialog>
  )
}

createRoot(document.getElementById("root")).render(<Devices />)
