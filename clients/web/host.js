// What the app's own page (`clients/app/page`) asks of where it runs, in a browser: the network's edge at the address
// it was opened at. It asks the network's servers as MCP over HTTP (`/mcp`), as whoever signed in here with their
// passkey, and an app's files come from the edge too (`/apps/`). It's what the app's Rust half answers, the same
// commands by the same names, less what a browser can't do, which each says.

/** @import { Commands } from "../app/page/host.js" */

/// Where a person signs in: the authorization endpoint of the authorization server the network's metadata names (RFC
/// 9728, RFC 8414), kept in this browser's storage once it's looked, so going to sign in asks nothing more.
const AUTHORIZATION_ENDPOINT = "authorization_endpoint"
async function signIn() {
  const stored = localStorage.getItem(AUTHORIZATION_ENDPOINT)
  if (stored) return stored
  const { authorization_servers } = await (await fetch("/.well-known/oauth-protected-resource")).json()
  const issuer = new URL(authorization_servers[0])
  const { authorization_endpoint } = await (await fetch(`${issuer.origin}/.well-known/oauth-authorization-server${issuer.pathname}`.replace(/\/$/, ""))).json()
  localStorage.setItem(AUTHORIZATION_ENDPOINT, authorization_endpoint)
  return authorization_endpoint
}

/// What hears each event, by its name.
/** @type {Map<string, Set<(event: { payload: unknown }) => void>>} */
const hearing = new Map()
/** @type {typeof import("../app/page/host.js").listen} */
export const listen = (event, handler) => {
  if (!hearing.has(event)) hearing.set(event, new Set())
  hearing.get(event)?.add(/** @type {(event: { payload: unknown }) => void} */ (handler))
}
/** @param {string} event @param {unknown} payload */
const emit = (event, payload) => hearing.get(event)?.forEach((handler) => handler({ payload }))

/// Where this browser's apps' files are, and this browser as Devices lists it, once it's signed in (`start`).
/** @type {string} */
let apps
/** @type {string} */
let device
/// Whether its network answers, as the app's client says it (`Connection` in core/client), how soon it's asked
/// again while it doesn't, and that ask.
/** @type {import("../app/page/host.js").Connection} */
let connection = "connecting"
/// How long it waits before asking again, doubling from `FIRST` to `MOST`, each wait a random time from half of it
/// to all of it (jitter, so browsers that lost the network together don't all ask together once it's back).
const [FIRST, MOST] = [1000, 3000]
let again = FIRST
/** @type {ReturnType<typeof setTimeout> | undefined} */
let asking

/// How long a request may take before it's reported as slow, as the app's client has it.
const SLOW = 2000
/** Random bytes in hex, as a trace's ids are. @param {number} bytes */
const hex = (bytes) => Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (byte) => byte.toString(16).padStart(2, "0")).join("")

/// Asks the network's servers `method`, as this browser's person. → its result. A browser that's no longer
/// signed in goes to sign in again.
/** @param {string} method @param {Record<string, any>} params @returns {Promise<any>} */
async function mcp(method, params = {}) {
  // The span this request is, carried as W3C's `traceparent`, which the edge's span and a server's are under, so a
  // trace runs from this browser on. It's reported if it fails or is slow, as the app's client reports its own
  // (`span` in `core/client`), but not when it failed for no server answering while none was already: that's the
  // outage, reported once it's over (`said`).
  const tool = method === "tools/call" ? params.name : undefined
  const span = { name: tool ? `${method} ${tool}` : method, traceId: hex(16), spanId: hex(8), kind: "client", attributes: { "mcp.method.name": method, "gen_ai.tool.name": tool, "one.reached": "edge" } }
  const [started, was] = [performance.now(), connection]
  /** @param {string} [error] */
  const ended = (error) => {
    const took = performance.now() - started
    if ((error && (was === "online" || connection === "online")) || took > SLOW) report({ ...span, took, error })
  }
  let response
  try {
    const headers = { "content-type": "application/json", traceparent: `00-${span.traceId}-${span.spanId}-01` }
    response = await fetch("/mcp", { method: "POST", headers, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) })
  } catch {
    answered(false)
    ended("Can't reach your network")
    throw new Error("Can't reach your network. Check this device is online.")
  }
  if (response.status === 401) {
    location.replace(await signIn())
    throw new Error("Sign in again.")
  }
  answered(response.status !== 503)
  const { result, error } = await response.json()
  ended(error?.message ?? (result?.isError ? (result.content?.[0]?.text ?? "The tool failed") : undefined))
  if (error) throw Object.assign(new Error(error.message), { code: error.code })
  return result
}
/// What this browser reported to Telemetry (`report`), kept in its storage until a server is told, a few hundred
/// at most, as the app's client keeps them on disk. Telling reports nothing itself.
const REPORTS = "reports"
/** @returns {Record<string, unknown>[]} */
const reports = () => JSON.parse(localStorage.getItem(REPORTS) ?? "[]")
/** @param {Record<string, any>} record */
function report(record) {
  const stored = reports()
  const browser = ["Edge", "Firefox", "Chrome", "Safari"].find((name) => navigator.userAgent.includes(name === "Edge" ? "Edg/" : `${name}/`))
  const attributes = { ...record.attributes, "service.name": "web", "user_agent.name": browser, "os.type": os() }
  if (stored.length < 300) localStorage.setItem(REPORTS, JSON.stringify([...stored, { ...record, attributes, time: Date.now() }]))
  if (connection === "online") sendReports()
}
/// The device's operating system, by the names the app's client reports its own by (Rust's `env::consts::OS`). An
/// iPad's Safari says it's a Mac, but has a touch screen.
const os = () => {
  const agent = navigator.userAgent
  if (/iPhone|iPad|iPod/.test(agent) || (agent.includes("Macintosh") && navigator.maxTouchPoints > 1)) return "ios"
  if (agent.includes("Android")) return "android"
  if (agent.includes("Mac OS X")) return "macos"
  if (agent.includes("Windows")) return "windows"
  if (agent.includes("CrOS")) return "chromeos"
  if (agent.includes("Linux")) return "linux"
  return undefined
}
/// Reports are told a few seconds after the first of them, as one batch.
/** @type {ReturnType<typeof setTimeout> | undefined} */
let sending
const sendReports = () => void (sending ??= setTimeout(() => ((sending = undefined), send()), 5000))
async function send() {
  const stored = reports()
  if (!stored.length) return
  localStorage.removeItem(REPORTS)
  const notification = { jsonrpc: "2.0", method: "notifications/one/measured", params: { records: stored } }
  const sent = await fetch("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(notification) }).catch(() => undefined)
  if (!sent?.ok) localStorage.setItem(REPORTS, JSON.stringify([...stored, ...reports()].slice(0, 300)))
}
/// Since when no server has answered, which once one does is reported, if it was long enough to show.
let unanswered = 0

/// The moves its connection makes, from each state, as the app's client has them (`Connection::moves_to`).
const MOVES = { connecting: ["online", "offline"], offline: ["connecting"], online: ["offline"] }
/// Moves its connection to `next`, if that's a move there is from where it is.
/** @param {import("../app/page/host.js").Connection} next */
function connected(next) {
  if (!MOVES[connection].includes(next)) return
  connection = next
  emit("connection", next)
}

/// Says whether its network answered what was just asked, which leaves it offline until it's tried again
/// (`tryAgain`), and an answer to what was asked while it was offline is a try that was answered. While it doesn't
/// answer, this browser asks again by itself, every few seconds at most, as what's outside a device comes and goes,
/// and no person should have to ask for what was going to come back anyway: at once as the browser says it's online
/// again or the page is seen again, and otherwise as `again` says. Once it answers, every open app asks again, for
/// what it missed.
/** @param {boolean} now */
function answered(now) {
  const lost = connection === "offline" || unanswered > 0
  if (now) connected("connecting")
  const was = connection
  connected(now ? "online" : "offline")
  if (was === connection) return
  if (now && lost) {
    tools = undefined
    for (const view of views.keys()) refresh(view)
  }
  if (!now) unanswered ||= performance.now()
  if (now && unanswered && performance.now() - unanswered > 3000) report({ name: "one.client.unanswered", took: performance.now() - unanswered, error: "No server is answering" })
  else if (now) sendReports()
  if (now) [again, unanswered] = [FIRST, 0]
  else tryAgain()
}
/// Tries its network again once `again` has passed, or with `now`, at once: by opening again while it hasn't, and
/// by asking a server once it has.
function tryAgain(now = false) {
  clearTimeout(asking)
  asking = setTimeout(
    () => {
      again = Math.min(again * 2, MOST)
      connected("connecting")
      if (apps) mcp("ping").catch(() => {})
      else opening()
    },
    now ? 0 : again * (0.5 + Math.random() / 2),
  )
}
for (const [target, event] of /** @type {const} */ ([[window, "online"], [document, "visibilitychange"]]))
  target.addEventListener(event, () => connection === "offline" && document.visibilityState === "visible" && tryAgain(true))

/// What each open app was answered that only reads, by what it asked: it's answered that again until something
/// changes, as the app's client answers a view (`Subscription` in `core/client`).
/** @type {Map<string, Map<string, { method: string, params: Record<string, unknown>, response: unknown }>>} */
const views = new Map()
/** @param {string} view */
const responses = (view) => /** @type {NonNullable<ReturnType<typeof views.get>>} */ (views.get(view) ?? views.set(view, new Map()).get(view))

/** @param {string} view @param {string} method @param {Record<string, unknown>} params @returns {Promise<any>} */
async function read(view, method, params) {
  const request = JSON.stringify([method, params])
  const cached = responses(view).get(request)
  if (cached) return cached.response
  const response = await mcp(method, params)
  responses(view).set(request, { method, params, response })
  return response
}

/// The tools a server lists, by which a call is known to only read: asked for as soon as this browser is signed in,
/// as nearly every app's first call waits on it, and again once something may have changed them.
/** @type {Promise<{ name: string, annotations?: { readOnlyHint?: boolean } }[]> | undefined} */
let tools
const listTools = () =>
  (tools = mcp("tools/list").then(
    (listed) => listed.tools,
    (error) => {
      tools = undefined
      throw error
    },
  ))

/// Whether asking `method` only reads. A tool does when the server's list of tools says so.
/** @param {string} view @param {string} method @param {Record<string, unknown>} params */
async function reads(view, method, params) {
  if (["tools/list", "resources/list", "resources/read"].includes(method)) return true
  if (method !== "tools/call") return false
  return (await (tools ?? listTools())).some((/** @type {{ name: string, annotations?: { readOnlyHint?: boolean } }} */ tool) => tool.name === params.name && tool.annotations?.readOnlyHint === true)
}

/// Asks again for everything `view` was answered, and tells it if any of it changed.
/** @param {string} view */
async function refresh(view) {
  let changed = false
  await Promise.all(
    [...(views.get(view)?.values() ?? [])].map(async (cached) => {
      // With no server answering, what was answered stays what's shown.
      const fresh = await mcp(cached.method, cached.params).catch(() => cached.response)
      if (JSON.stringify(fresh) === JSON.stringify(cached.response)) return
      cached.response = fresh
      changed = true
    }),
  )
  if (changed) emit("changed", view)
}

/// The version of each app a server last listed, by the subject it's listed for.
/** @type {Map<string | undefined, Map<string, string>>} */
const versions = new Map()
/// The open apps for `subject` whose apps have a new version since the last list, each as its address names it
/// (`<app>[.<subject, last part first>]`).
/** @param {string | undefined} subject @param {{ uri: string }[]} resources */
function outdated(subject, resources) {
  const now = new Map(resources.map(({ uri }) => /** @type {[string, string]} */ (uri.slice("ui://".length).split("/", 2))))
  const was = versions.get(subject)
  versions.set(subject, now)
  if (!was) return []
  const changed = [...now].filter(([app, version]) => was.has(app) && was.get(app) !== version).map(([app]) => app)
  const whose = subject ? `.${subject.split("/").reverse().join(".")}` : ""
  return changed.map((app) => app + whose)
}

/// How many moves of its session this browser has told the page of.
let told = 0
/// Asks the network's address whether this browser is signed in, as it opens: → where it is with its network, as the
/// app's client says a device's session (`Session` in core/client), and, while it's to sign in, where it does.
/** @returns {Promise<Omit<import("../app/page/host.js").Told, "seq">>} */
async function opened() {
  let response
  try {
    response = await fetch("/apps")
  } catch {
    answered(false)
    return { session: { state: "opening" } }
  }
  if (response.status === 401) {
    answered(true)
    return { session: { state: "signIn" }, link: await signIn() }
  }
  if (!response.ok) {
    answered(false)
    return { session: { state: "opening" } }
  }
  ;({ apps, device } = await response.json())
  answered(true)
  listTools().catch(() => {})
  return { session: { state: "signedIn" } }
}
/// Asks again, while it's still opening, and tells the page where it is now, if it's moved.
async function opening() {
  const now = await opened()
  if (now.session.state !== "opening") emit("session", { seq: ++told, ...now })
}

/// What a browser can't do, as it's said where it's asked.
/** @param {string} why */
const cannot = (why) => () => Promise.reject(new Error(why))
const NO_KEY = "A browser can't hold your key. Use the app on your phone or computer for that."

/** @type {{ [Command in keyof Commands]?: (...args: Parameters<Commands[Command]>) => ReturnType<Commands[Command]> | Promise<ReturnType<Commands[Command]>> }} */
const commands = {
  /// Where this browser is with its network, as the app's client says a device's session (`Session` in core/client):
  /// signed in here, or sent to sign in, and whether its network answers. A browser keeps nothing of a network's, so
  /// with its address not answering it's still opening, and asks again by itself (`tryAgain`).
  async start() {
    return { seq: (told = 0), ...(await opened()), connection }
  },
  /// The files a server lists for `subject`, and the open apps that are out of date.
  async apps({ subject }) {
    const listed = await mcp("resources/list", subject ? { _meta: { subject } } : {})
    return { ...listed, outdated: outdated(subject, listed.resources) }
  },
  async request({ view, method, params }) {
    if (await reads(view, method, params)) return read(view, method, params)
    const response = await mcp(method, params)
    // A server tells every client it's connected to that something changed. A browser isn't connected, so it asks
    // again itself, for every open app.
    tools = undefined
    for (const view of views.keys()) refresh(view)
    return response
  },
  refresh: ({ view }) => refresh(view),
  close: ({ view }) => void views.delete(view),
  client: () => ({ device, network: location.origin }),
  /// A file too large to go in a call goes where the network keeps it, by the link a server gives for it.
  async upload(file, { headers: { app } }) {
    const { content, isError } = await mcp("tools/call", { name: "upload", arguments: { app } })
    if (isError) throw new Error(content[0].text)
    const to = JSON.parse(content[0].text)
    const sent = await fetch(to.url, { method: to.method, headers: to.headers, body: file })
    if (!sent.ok) throw new Error("The file couldn't be sent. Try again.")
    return { upload: to.upload }
  },
  retry: () => tryAgain(true),
  sign_out: () => location.reload(),
  serve: cannot("A browser can't be a server. Make your phone or computer one in its app."),
  update: cannot("A browser has the latest each time it opens."),
  recovery_key: cannot(NO_KEY),
  recover: cannot(NO_KEY),
  report: ({ record }) => report(record),
  opened: ({ view, ms }) => report({ name: "one.client.open.duration", value: ms, attributes: { "one.view": view } }),
  // What the app does as it's seen, which a browser doesn't: it hears notifications.
  visible() {},
  listen() {},
  // A browser's back and forward gestures are its own, and so is its status bar.
  navigation_gestures() {},
  status_bar() {},
}

/// Light or dark for the browser's own bars, as its `theme-color` (index.html) names the page's background in each:
/// the chosen one's, or with `null` each as the device is.
const themeColors = [...document.querySelectorAll('meta[name="theme-color"]')].map((meta) => [meta, meta.media])
/** @type {typeof import("../app/page/host.js").setTheme} */
export const setTheme = (theme) => {
  for (const [meta, media] of themeColors) meta.media = theme ? (media.includes(theme) ? "all" : "not all") : media
}

/** @type {typeof import("../app/page/host.js").invoke} */
export const invoke = (command, ...args) => {
  if (!commands[command]) return Promise.reject(new Error(`A browser has no ${command}`))
  return Promise.resolve(commands[command](...args))
}

/// Where an app's page is: the edge's, under where this browser's apps' files are, by what names its address
/// (`<app>[.<subject, last part first>]`), as the app has its own address for each. It's at the version a server
/// last listed, which the edge keeps (`core/edge/mcp/mcp.ts`), or before its subject is listed, the latest.
/** @type {typeof import("../app/page/host.js").address} */
export const address = (view) => {
  const [app, ...page] = view.entrypoint.split("/")
  const version = versions.get(view.subject)?.get(app)
  return `${apps}/${view.host}/${version ? `${app}@${version}` : app}/${page.join("/")}`
}

/// What an app's frame may do. Its address is this page's, so it's given an origin of its own, which reaches
/// nothing here: not this page, not another app, and not what this browser is signed in with.
export const sandbox = "allow-scripts allow-forms"

/// A web address an app opens, in a tab of its own.
/** @type {typeof import("../app/page/host.js").openLink} */
export const openLink = (url) => void open(url, "_blank", "noopener")

/// A haptic, as a short vibration where the browser has one and lets this page vibrate (Chrome on Android, after a
/// tap), and nothing elsewhere: iOS's Safari and a computer's browsers have none.
/** @type {Record<import("../app/page/host.js").Haptic, number | number[]>} */
const VIBRATIONS = { light: 10, soft: 8, medium: 15, rigid: 15, heavy: 25, selection: 5, success: [10, 60, 10], warning: [20, 80, 20], error: [20, 40, 20, 40, 20] }
/** @type {typeof import("../app/page/host.js").haptic} */
export const haptic = (kind) => void navigator.vibrate?.(VIBRATIONS[kind])
