// The bridge between each app's frame and the client: what a page asks the window, what it's answered, and what it tells the window.
import { haptic, invoke, openLink, sandbox } from "./host.js"
import { behind, list, listed, names, trusted } from "./apps.js"
import { chosen, dress, insets, theme, titleBar, titleRow } from "./chrome.js"
import { bar, baseKey, depth, go, host, key, located, nested, on, open, opened, own, page, panel, phone, shown, within } from "./client.js"
import { gestures } from "./gestures.js"
import { shortcut } from "./shortcuts.js"
import { byId, exception } from "./util.js"
/** @import { View } from "./host.js" */

/// How tall the launcher's content last said it is (`ui/notifications/size-changed`), and how tall its panel was made.
let asked = 0
/** @type {number | undefined} */
let made
/// A computer's launcher panel is as tall as the launcher says while it shows it, as Raycast's and Spotlight's grow and
/// shrink to what they show, and its tallest while it shows anything else: its steps, a problem, an update.
export function fit() {
  if (!panel()) return
  const height = shown.getSnapshot().matches("launcher") && asked ? asked : undefined
  if (height === made) return
  made = height
  invoke("resize", { height })
}
/** @param {View & { window?: Window }} view @param {Record<string, unknown>} message */
export const post = (view, message) => (view.window ?? view.frame.contentWindow)?.postMessage({ jsonrpc: "2.0", ...message }, "*")
/// The page in an app's frame a message came from: one known by its window, or one saying its number from a frame of
/// the page it was made for, which it's known by from then on.
/** @param {MessageEventSource | null} source @param {any} data */
function frameOf(source, data) {
  for (const [frame, view] of nested) {
    if (view.window === source) return view
    const from = /** @type {Window | null} */ (source)
    if (!view.window && from && data?.frame === frame && from.parent === view.in.frame.contentWindow) return Object.assign(view, { window: from })
  }
}
/// The frames that hold something their person typed and haven't sent (`initialization-script.js`): the window says
/// whether any does, which a new build of the app waits for (`clients/app/src/updater.rs`).
const typedIn = new Set()
addEventListener("message", ({ data, source }) => {
  if (data?.method !== "notifications/one/editing") return
  const before = typedIn.size > 0
  if (data.params?.editing) typedIn.add(source)
  else typedIn.delete(source)
  if (typedIn.size > 0 !== before) invoke("editing", { editing: typedIn.size > 0 }).catch(() => {})
})
/// Shows this device's person their recovery key, or with `typed`, asks them for it and lets the device in with
/// it: theirs, or that of `subject`, a group or the network the app asking is for. → `{ done }`: whether they saw it,
/// or the device is in now. It's over whatever app asked, and the app is told only that.
/** @param {boolean} typed @param {string} [subject] @returns {Promise<{ done: boolean }>} */
function recoveryKey(typed, subject) {
  const dialog = /** @type {HTMLDialogElement} */ (byId("recovery"))
  const [key, entry, problem, form, approve] = /** @type {[HTMLElement, HTMLInputElement, HTMLElement, HTMLFormElement, HTMLButtonElement]} */ ([".key", ".entry", ".wrong", "form", 'button.typed[data-variant="default"]'].map((part) => dialog.querySelector(part)))
  dialog.dataset.typed = String(typed)
  entry.value = key.textContent = problem.textContent = dialog.returnValue = ""
  // Its main button waits for the key, as a system's dialog's default button waits for what it needs.
  approve.disabled = true
  entry.oninput = () => (approve.disabled = entry.value.trim() === "")
  if (!typed) invoke("recovery_key", { subject }).then((shown) => (key.textContent = shown), (error) => (problem.textContent = `${error}`))
  dialog.showModal()
  return new Promise((resolve) => {
    form.onsubmit = async (event) => {
      event.preventDefault()
      const pressed = /** @type {HTMLButtonElement | null} */ (event.submitter)
      if (typed && pressed?.value !== "cancel") {
        try {
          await invoke("recover", { key: entry.value, subject })
        } catch (error) {
          return void (problem.textContent = `${error}`)
        }
      }
      dialog.returnValue = pressed?.value === "cancel" ? "" : "done"
      dialog.close()
    }
    dialog.onclose = () => {
      entry.value = key.textContent = ""
      resolve({ done: dialog.returnValue === "done" })
    }
  })
}
/// The haptics an app may ask for, by iOS's names.
/** @type {unknown[]} */
const HAPTICS = ["light", "medium", "heavy", "rigid", "soft", "selection", "success", "warning", "error"]
/// What this page answers an app itself. The rest is the client's to answer, as from the app whose page it is,
/// which this page says, whatever the page does: a server has an untrusted app's page call only what its server code
/// may (`OneCore.Mcp`). An untrusted app's page asks for whoever it was opened for, and changes nothing of this
/// device but its own status bar; a trusted one's may do anything its server code may, which is all its person may.
/** @param {View} view @param {string} method @param {Record<string, any>} params */
async function respond(view, method, params) {
  const app = view.entrypoint.split("/")[0]
  const trusts = async () => (await trusted.get(view.subject))?.has(app) === true
  /** Who the page asks for: whoever it says, if its app is trusted. @param {string} [said] */
  const whom = async (said) => (said && (await trusts()) ? said : view.subject)
  if (method === "ui/initialize") {
    // A page in a frame of another's has none of the window's edges to keep clear, and nothing names it above it.
    const inFrame = [...nested.values()].includes(/** @type {any} */ (view))
    return {
      protocolVersion: "2026-01-26",
      hostInfo: { name: "one", version: "0.1.0" },
      hostCapabilities: { serverTools: {}, serverResources: {} },
      hostContext: {
        theme: theme(),
        displayMode: "fullscreen",
        availableDisplayModes: ["fullscreen"],
        containerDimensions: { width: view.frame.clientWidth, height: view.frame.clientHeight },
        locale: navigator.language,
        timeZone: Intl.DateTimeFormat().resolvedOptions().timeZone,
        deviceCapabilities: { touch: matchMedia("(pointer: coarse)").matches, hover: matchMedia("(hover: hover)").matches },
        safeAreaInsets: inFrame ? { top: 0, right: 0, bottom: 0, left: 0 } : insets(view.frame),
        ...(!inFrame && titleRow()),
        // Whether the window names it above it, in its title bar (`titled`), so its first screen doesn't.
        titled: Boolean(own) && !titleBar.hidden && !inFrame,
        // Whether its network answers, which the window keeps it told of.
        connection: shown.getSnapshot().context.connection,
      },
    }
  }
  if (method === "ping") return {}
  // Another app's page, in a frame of this one's, which this page answers as that app's (`framed`): only a trusted
  // app's page shows one, as it may do anything its person may.
  // A frame of its own that's gone: its page is forgotten.
  if (method === "one/frame" && typeof params.forget === "string") {
    if (nested.get(params.forget)?.in === view) nested.delete(params.forget)
    return {}
  }
  if (method === "one/frame") {
    if (!(await trusts())) throw new Error(`${names.get(view.entrypoint) || app} is installed untrusted, so it shows no other app's page.`)
    const [entrypoint, fragment] = String(params.src).split("#", 2)
    const subject = await whom(params.subject)
    if (subject && !listed.has(subject)) list(subject)
    const place = { key: key(entrypoint, subject), entrypoint, subject, host: host(entrypoint, subject) }
    const frame = crypto.randomUUID()
    nested.set(frame, { ...place, frame: view.frame, in: view })
    const url = new URL(located(place), location.href)
    url.searchParams.set("frame", frame)
    return { url: `${url}${fragment ? `#${fragment}` : ""}`, sandbox, frame }
  }
  if (method === "ui/request-display-mode") return { mode: "fullscreen" }
  // A file too large to go in a call: the client sends it where the network keeps it, for a tool of that app.
  if (method === "one/upload") return invoke("upload", await params.file.arrayBuffer(), { headers: { app } })
  // A person's recovery key is shown and typed on this page, which is the client's own: no app's page is handed one.
  if (method === "one/recovery-key") return recoveryKey(params.typed === true, await whom(params.subject))
  // Another app, shown over this one, as tapping its icon or a notification from it does: by its app's name,
  // and its page if it isn't the app's first app, for whoever this one is for unless it says. `behind` starts
  // it as a finger lands on its icon, and it shows as the finger lifts, out of the icon at `from` in its page.
  if (method === "ui/open-link" && params.url?.startsWith("one:")) {
    if (!go(params.url.slice("one:".length), await whom(params.subject), params.behind, within(view.frame, params.from))) throw new Error("That app isn't on this device")
    return {}
  }
  // A web address opens in the device's own browser, outside the app.
  if (method === "ui/open-link" && /^https?:/.test(new URL(params.url).protocol)) {
    openLink(params.url)
    return {}
  }
  // An app put on the device's own home screen, which Android's app does (`MainActivity`): by its entrypoint.
  if (method === "one/pin-shortcut") {
    const pin = globalThis.androidPin
    const app = pin && (await shortcut(params.entrypoint))
    if (!pin || !app) throw new Error("This device can't put that app on its home screen")
    pin.postMessage(JSON.stringify(app))
    return {}
  }
  // A haptic plays here, not in the app's frame: only this page reaches the device's, and Chrome lets no frame of
  // another address vibrate.
  if (method === "one/haptic") {
    if (!HAPTICS.includes(params.kind)) throw new Error(`There's no haptic ${params.kind}`)
    await haptic(params.kind)
    return {}
  }
  // What an app is told of its client, and the few things one changes of it.
  if (method === "one/client") {
    const changes = Object.keys(params).filter((change) => change !== "statusBar")
    if (changes.length && !(await trusts())) throw new Error(`${names.get(view.entrypoint) || app} is installed untrusted, so it doesn't change this device (${changes.join(", ")}).`)
    if (params.leave) return invoke("sign_out")
    // The device's status bar, hidden while this app shows, as an iPhone's Home Screen hides it while it's edited.
    if ("statusBar" in params) {
      view.statusBar = params.statusBar !== false
      bar()
      return { message: "" }
    }
    // A phone whose push failed tries again.
    if (params.push) return invoke("push")
    // A phone's own Settings page, where its person turns on what only they may: Android's Accessibility, or the
    // app's App Info, and an iPhone's page for the app.
    if (params.settings) {
      if (globalThis.androidSettings) globalThis.androidSettings.postMessage(params.settings)
      else if (params.settings === "app") await invoke("settings")
      else throw new Error("This device has no Settings to open here.")
      return { message: "" }
    }
    // This device becomes a server of its person's, or stops being one, or takes the latest builds.
    if ("serve" in params) return { message: await invoke("serve", { on: params.serve === true }) }
    // It updates as the window shows it, over every app, where an update is ready.
    if (params.update) {
      if (!shown.getSnapshot().context.updateReady) on({ type: "updateReady", ready: !phone && (await behind()) })
      on({ type: "update" })
      if (!shown.getSnapshot().matches("updating")) throw new Error("There's no update for this device.")
      return { message: "Updating…" }
    }
    if ("theme" in params) {
      if (params.theme) localStorage.setItem("theme", params.theme)
      else localStorage.removeItem("theme")
      dress()
    }
    const client = await invoke("client")
    return { ...client, theme: chosen(), pinsShortcuts: !!globalThis.androidPin, settings: !!globalThis.androidSettings || client.settings === true, launcher: panel() }
  }
  if (method.startsWith("ui/")) throw Object.assign(new Error(`This client has no ${method}`), { code: -32601 })
  // An app opened from a group's page is answered for the group. What else an untrusted app's page says of its
  // request, who it's from among it, isn't passed on.
  const subject = await whom(params._meta?.subject)
  if (subject && !listed.has(subject)) list(subject)
  const meta = { ...((await trusts()) && params._meta), app, ...(subject && { subject }) }
  return invoke("request", { view: view.key, method, params: { ...params, _meta: meta } })
}
addEventListener("message", async ({ source, data }) => {
  const view = [...open.values()].find(({ frame }) => frame.contentWindow === source) ?? frameOf(source, data)
  if (!view || typeof data?.method !== "string") return
  const { id, method, params = {} } = data
  // An app's page, or a page in its frame, logs its errors as an MCP App logs to its host, and they're its app's,
  // whatever it says.
  const errored = id === undefined && method === "notifications/message" && params.level === "error"
  if (errored) exception(params.data?.message, params.data?.file, params.data?.line, view.entrypoint.split("/")[0])
  // A page in a frame is its frame's page's: it says how tall it is to that page.
  const child = [...nested].find(([, one]) => one === view)
  if (id === undefined && child) {
    const [frame, { in: hosting }] = child
    if (method === "ui/notifications/size-changed" && typeof params.height === "number") post(hosting, { method: "notifications/one/framed", params: { frame, height: params.height } })
    return
  }
  if (id === undefined) {
    // The app's user went back out of it. The app stays open behind the launcher.
    if (method === "ui/notifications/request-teardown" && !view.frame.inert && depth) history.back()
    else if (method === "ui/notifications/request-teardown" && panel() && view.key === baseKey()) invoke("hide")
    // The launcher's content says how tall it is, which its panel is made while it shows it.
    if (method === "ui/notifications/size-changed" && panel() && view.key === baseKey() && typeof params.height === "number") {
      asked = params.height
      fit()
    }
    // Its top bar is the window's title bar: what's empty in it moves the window, and double-clicked does what a
    // title bar does (`titleBar`).
    if (method === "one/drag-window") invoke("plugin:window|start_dragging")
    if (method === "one/title-bar-double-click") invoke("title_bar_double_click")
    // Its page has drawn: what it first asked for has settled (`render`), a problem shown as it failed included.
    if (method === "notifications/one/drawn") {
      page(view, { type: "drawn" })
      if (!view.frame.inert) opened(view)
    }
    // Something is open over its page, or nothing is any more, and whether it's on its first screen.
    if (method === "notifications/one/over") {
      view.over = params.over === true
      view.layer = params.layer === true
      view.first = params.first
      gestures()
    }
    if (errored) page(view, { type: "errored" })
    // The renderer's frame couldn't open its page (clients/web/renderer), and says why.
    if (method === "notifications/one/seen" && typeof params.unopened === "string") page(view, { type: "unopened", why: params.unopened })
    return
  }
  try {
    post(view, { id, result: await respond(view, method, params) })
  } catch (error) {
    const { code, message } = /** @type {{ code?: number, message?: string }} */ (error)
    post(view, { id, error: { code: code ?? -32000, message: message ?? String(error) } })
  }
})
