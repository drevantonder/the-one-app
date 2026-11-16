// What the app's own page (`clients/app/page`) asks of where it runs, in a browser: the network's edge at the address
// it was opened at. It asks the network's servers as MCP over HTTP (`/mcp`), as whoever signed in here with their
// passkey, and an app's files come from the edge too (`/apps/`). It's what the app's Rust half answers, the same
// commands by the same names, less what a browser can't do, which each says.

/** @import { Commands } from "../app/page/host.js" */

/// Where a person signs in: the authorization endpoint of the authorization server the network's metadata names (RFC
/// 9728, RFC 8414), kept in this browser's storage once it's looked, so going to sign in asks nothing more.
const AUTHORIZATION_ENDPOINT = "authorization_endpoint"
async function signIn() {
  const kept = localStorage.getItem(AUTHORIZATION_ENDPOINT)
  if (kept) return kept
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
export const listen = (event, heard) => {
  if (!hearing.has(event)) hearing.set(event, new Set())
  hearing.get(event)?.add(/** @type {(event: { payload: unknown }) => void} */ (heard))
}
/** @param {string} event @param {unknown} payload */
const emit = (event, payload) => hearing.get(event)?.forEach((heard) => heard({ payload }))

/// Where this browser's apps' files are, and this browser as Devices lists it, once it's signed in (`start`).
/** @type {string} */
let apps
/** @type {string} */
let device
/// Whether a server answered the last thing asked, how soon it's asked again while none does, and that ask.
let answering = true
let again = 1000
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
  const [started, was] = [performance.now(), answering]
  /** @param {string} [error] */
  const ended = (error) => {
    const took = performance.now() - started
    if ((error && (was || answering)) || took > SLOW) report({ ...span, took, error })
  }
  let answer
  try {
    const headers = { "content-type": "application/json", traceparent: `00-${span.traceId}-${span.spanId}-01` }
    answer = await fetch("/mcp", { method: "POST", headers, body: JSON.stringify({ jsonrpc: "2.0", id: 1, method, params }) })
  } catch {
    said(false)
    ended("Can't reach your network")
    throw new Error("Can't reach your network. Check this device is online.")
  }
  if (answer.status === 401) {
    location.replace(await signIn())
    throw new Error("Sign in again.")
  }
  said(answer.status !== 503)
  const { result, error } = await answer.json()
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
  const kept = reports()
  const browser = ["Edge", "Firefox", "Chrome", "Safari"].find((name) => navigator.userAgent.includes(name === "Edge" ? "Edg/" : `${name}/`))
  const attributes = { ...record.attributes, "service.name": "web", "user_agent.name": browser }
  if (kept.length < 300) localStorage.setItem(REPORTS, JSON.stringify([...kept, { ...record, attributes, time: Date.now() }]))
  if (answering) sendReports()
}
/// Reports are told a few seconds after the first of them, as one batch.
/** @type {ReturnType<typeof setTimeout> | undefined} */
let sending
const sendReports = () => void (sending ??= setTimeout(() => ((sending = undefined), send()), 5000))
async function send() {
  const kept = reports()
  if (!kept.length) return
  localStorage.removeItem(REPORTS)
  const said = { jsonrpc: "2.0", method: "notifications/one/measured", params: { records: kept } }
  const sent = await fetch("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(said) }).catch(() => undefined)
  if (!sent?.ok) localStorage.setItem(REPORTS, JSON.stringify([...kept, ...reports()].slice(0, 300)))
}
/// Since when no server has answered, which once one does is reported, if it was long enough to show.
let unanswered = 0

/// While no server answers, this browser asks again by itself, sooner at first and then less often, as what's
/// outside a device comes and goes, and no person should have to ask for what was going to come back anyway. Once
/// one answers, every open app asks again, for what it missed.
/** @param {boolean} now */
function said(now) {
  const was = answering
  if (now && !was) for (const view of views.keys()) refresh(view)
  if (now !== was) emit("answering", (answering = now))
  if (!now && was) unanswered = performance.now()
  if (now && !was && performance.now() - unanswered > 3000) report({ name: "one.client.unanswered", took: performance.now() - unanswered, error: "No server is answering" })
  else if (now && !was) sendReports()
  if (now) again = 1000
  else asking ??= setTimeout(() => {
    asking = undefined
    again = Math.min(again * 2, 30000)
    mcp("ping").catch(() => {})
  }, again)
}

/// What each open app was answered that only reads, by what it asked: it's answered that again until something
/// changes, as the app's client answers a view (`Subscription` in `core/client`).
/** @type {Map<string, Map<string, { method: string, params: Record<string, unknown>, answer: unknown }>>} */
const views = new Map()
/** @param {string} view */
const answered = (view) => /** @type {NonNullable<ReturnType<typeof views.get>>} */ (views.get(view) ?? views.set(view, new Map()).get(view))

/** @param {string} view @param {string} method @param {Record<string, unknown>} params @returns {Promise<any>} */
async function read(view, method, params) {
  const asked = JSON.stringify([method, params])
  const kept = answered(view).get(asked)
  if (kept) return kept.answer
  const answer = await mcp(method, params)
  answered(view).set(asked, { method, params, answer })
  return answer
}

/// Whether asking `method` only reads. A tool does when the server's list of tools says so.
/** @param {string} view @param {string} method @param {Record<string, unknown>} params */
async function reads(view, method, params) {
  if (["tools/list", "resources/list", "resources/read"].includes(method)) return true
  if (method !== "tools/call") return false
  const { tools } = await read(view, "tools/list", {})
  return tools.some((/** @type {{ name: string, annotations?: { readOnlyHint?: boolean } }} */ tool) => tool.name === params.name && tool.annotations?.readOnlyHint === true)
}

/// Asks again for everything `view` was answered, and tells it if any of it changed.
/** @param {string} view */
async function refresh(view) {
  let changed = false
  await Promise.all(
    [...(views.get(view)?.values() ?? [])].map(async (kept) => {
      // With no server answering, what was answered stays what's shown.
      const fresh = await mcp(kept.method, kept.params).catch(() => kept.answer)
      if (JSON.stringify(fresh) === JSON.stringify(kept.answer)) return
      kept.answer = fresh
      changed = true
    }),
  )
  if (changed) emit("changed", view)
}

/// The version of each plugin a server last listed, by who it's listed for.
/** @type {Map<string | undefined, Map<string, string>>} */
const versions = new Map()
/// The open apps for `who` whose plugins have a new version since the last list, each as its address names it
/// (`<plugin>[.<who, last part first>]`).
/** @param {string | undefined} who @param {{ uri: string }[]} resources */
function outdated(who, resources) {
  const now = new Map(resources.map(({ uri }) => /** @type {[string, string]} */ (uri.slice("ui://".length).split("/", 2))))
  const was = versions.get(who)
  versions.set(who, now)
  if (!was) return []
  const changed = [...now].filter(([plugin, version]) => was.has(plugin) && was.get(plugin) !== version).map(([plugin]) => plugin)
  const whose = who ? `.${who.split("/").reverse().join(".")}` : ""
  return changed.map((plugin) => plugin + whose)
}

/// What a browser can't do, as it's said where it's asked.
/** @param {string} why */
const cannot = (why) => () => Promise.reject(new Error(why))
const NO_KEY = "A browser can't hold your key. Use the app on your phone or computer for that."

/** @type {{ [Command in keyof Commands]?: (...args: Parameters<Commands[Command]>) => ReturnType<Commands[Command]> | Promise<ReturnType<Commands[Command]>> }} */
const commands = {
  /// Signed in here, or sent to sign in. A browser keeps nothing of a network's, so with its address not answering
  /// there's nothing to show.
  async start() {
    let answer
    try {
      answer = await fetch("/apps")
    } catch {
      return { unreached: "Your network's address didn't answer" }
    }
    if (answer.status === 401) return { signIn: await signIn() }
    if (!answer.ok) return { unreached: `Your network's address answered ${answer.status}` }
    ;({ apps, device } = await answer.json())
    sendReports()
    return {}
  },
  /// The files a server lists for `who`, and the open apps that are out of date.
  async apps({ who }) {
    const listed = await mcp("resources/list", who ? { _meta: { for: who } } : {})
    return { ...listed, outdated: outdated(who, listed.resources) }
  },
  async request({ view, method, params }) {
    if (await reads(view, method, params)) return read(view, method, params)
    const answer = await mcp(method, params)
    // A server tells every client it's connected to that something changed. A browser isn't connected, so it asks
    // again itself, for every open app.
    for (const view of views.keys()) refresh(view)
    return answer
  },
  refresh: ({ view }) => refresh(view),
  close: ({ view }) => void views.delete(view),
  client: () => ({ device, network: location.origin }),
  /// A file too large to go in a call goes where the network keeps it, by the link a server gives for it.
  async upload(file, { headers: { plugin } }) {
    const { content, isError } = await mcp("tools/call", { name: "upload", arguments: { plugin } })
    if (isError) throw new Error(content[0].text)
    const to = JSON.parse(content[0].text)
    const sent = await fetch(to.url, { method: to.method, headers: to.headers, body: file })
    if (!sent.ok) throw new Error("The file couldn't be sent. Try again.")
    return { upload: to.upload }
  },
  enter: () => location.reload(),
  serve: cannot("A browser can't be a server. Make your phone or computer one in its app."),
  update: cannot("A browser has the latest each time it opens."),
  recovery_key: cannot(NO_KEY),
  recover: cannot(NO_KEY),
  report: ({ record }) => report(record),
  opened: ({ view, ms }) => report({ name: "one.client.open.duration", value: ms, attributes: { "one.view": view } }),
  // What the app does as it's seen, which a browser doesn't: it hears notifications.
  visible() {},
  listen() {},
}

/** @type {typeof import("../app/page/host.js").invoke} */
export const invoke = (command, ...args) => {
  if (!commands[command]) return Promise.reject(new Error(`A browser has no ${command}`))
  return Promise.resolve(commands[command](...args))
}

/// Where an app's page is: the edge's, under where this browser's apps' files are, by what names its address
/// (`<plugin>[.<who, last part first>]`), as the app has its own address for each.
/** @type {typeof import("../app/page/host.js").address} */
export const address = (view) => `${apps}/${view.host}/${view.app}`

/// What an app's frame may do. Its address is this page's, so it's given an origin of its own, which reaches
/// nothing here: not this page, not another app, and not what this browser is signed in with.
export const sandbox = "allow-scripts allow-forms"

/// A web address an app opens, in a tab of its own.
/** @type {typeof import("../app/page/host.js").openLink} */
export const openLink = (url) => void open(url, "_blank", "noopener")
