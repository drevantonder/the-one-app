// The bridge on a native host, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other,
// through the host, and what the client says of where the page is shown. The UI library's components import it as
// `#bridge`, which is this in the native build and the DOM's in the DOM's.
import { configureClientErrors, createSignal } from "solid-js"
import { event } from "./drawing-primitives/renderer.native.ts"
import type { Host, Sandbox } from "./calls.ts"
import type { Event, Events } from "./drawing-primitives/renderer.ts"
import type * as Page from "./types/ui.d.ts"
import { readable } from "./words.ts"
// The bridge, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other, through the host.

const host = (globalThis as { host?: Host }).host
const waiting = new Map<number, (response: { result?: unknown; error?: { message: string } }) => void>()
const listeners = new Set<() => void>()
let requests = 0
/** What listens for each key pressed (`keydown`). */
const keys = new Set<(event: Page.Keyed) => void>()

;(globalThis as { ui?: Sandbox & Events }).ui = {
  // A key pressed goes to what has the keyboard, and once to the page (the host tells it, as node 0), for what listens for
  // every key (`keydown`), as the DOM's `document` does, whatever has the keyboard.
  event(told: Event) {
    event(told)
    if (told.name === "keydown" && told.id === 0) for (const listener of keys) listener({ key: told.key ?? "", shiftKey: !!told.shiftKey, altKey: !!told.altKey, ctrlKey: !!told.ctrlKey, metaKey: !!told.metaKey, preventDefault() {} })
  },
  message(message: { id?: number; method?: string; result?: unknown; error?: { message: string } }) {
    if (message.method === "notifications/one/changed") for (const listener of listeners) listener()
    else if (message.method === "ui/notifications/host-context-changed") dress((message as { params?: object }).params)
    // The device's own back, where something is open over the page: the last opened shuts.
    else if (message.method === "notifications/one/back") over.at(-1)?.shut()
    else if (message.id !== undefined) waiting.get(message.id)?.(message)
  },
}

/// A page's errors are logged to its client, as the DOM's bridge logs them (MCP's `notifications/message`), which tells
/// Telemetry: what threw in a listener or a timer, a promise nothing handled, and what an `<Errored>` boundary caught
/// and shows, with where it was thrown from its stack.
const logged = (message: unknown, file?: string, line?: number) =>
  host?.post({ jsonrpc: "2.0", method: "notifications/message", params: { level: "error", data: { message: String(message), file, line } } })
addEventListener("error", ({ message, filename, lineno }) => logged(message, filename, lineno))
addEventListener("unhandledrejection", ({ reason }) => logged(reason?.message ?? reason))
configureClientErrors({
  onError(error) {
    const [, file, line] = /([a-z]+:\/\/[^\s()]+?):(\d+):\d+/.exec((error as Error)?.stack ?? "") ?? []
    logged((error as Error)?.message ?? error, file, line ? Number(line) : undefined)
  },
})

function ask<T>(method: string, params: object): Promise<T> {
  const id = ++requests
  return new Promise((done, failed) => {
    waiting.set(id, ({ result, error }) => {
      waiting.delete(id)
      if (error) failed(new Error(error.message))
      else done(result as T)
    })
    host?.post({ jsonrpc: "2.0", id, method, params })
  })
}

/** What the client says of where the page is shown, as the DOM's bridge reads it: whether the window names the app
 * above it (a computer's app window, whose breadcrumb a first screen's title gives way to), whether its network
 * answers, what the device keeps for itself at each edge, the window's size and whether its person asked for less
 * motion. */
/** Whether the window names the app above it, which a first screen's title gives way to (the Screen's). */
export const [breadcrumb, setBreadcrumb] = createSignal(false)
const [connected, setConnected] = createSignal<ReturnType<typeof Page.connection>>("online")
const [insets, setInsets] = createSignal({ top: 0, right: 0, bottom: 0, left: 0 })
const [size, setSize] = createSignal({ width: 0, height: 0 })
const [still, setStill] = createSignal(false)
const [shown, setShown] = createSignal(true)
interface Context {
  breadcrumb?: boolean
  connection?: ReturnType<typeof Page.connection>
  safeAreaInsets?: ReturnType<typeof Page.safeArea>
  containerDimensions?: { width: number; height: number }
  reducedMotion?: boolean
  /** The link the page was opened to: what follows `#` in a page's address on the DOM (`link()`). */
  link?: string
  /** MCP Apps' own: whether a finger points at it, and a mouse hovers. */
  deviceCapabilities?: { touch?: boolean; hover?: boolean }
  /** Whether its window is on screen, as a computer's launcher put away isn't: the DOM's `visibilityState`. */
  visible?: boolean
}
// Under a finger from the start where the device is, as its host says before it's greeted, so what's drawn before then
// (a screen in its loading fallback) is drawn for a finger too.
const [fingered, setFingered] = createSignal(navigator.maxTouchPoints > 0)
const [opened, setOpened] = createSignal<string>()
function dress(context?: Context) {
  if (context?.link) setOpened(context.link)
  if (context?.deviceCapabilities && "touch" in context.deviceCapabilities) setFingered(context.deviceCapabilities.touch === true)
  if (context && "breadcrumb" in context) setBreadcrumb(context.breadcrumb === true)
  if (context?.connection) setConnected(context.connection)
  if (context?.safeAreaInsets) setInsets(context.safeAreaInsets)
  if (context?.containerDimensions) setSize(context.containerDimensions)
  if (context && "reducedMotion" in context) setStill(context.reducedMotion === true)
  if (context && "visible" in context) setShown(context.visible !== false)
}

/** A client answers a page once it has said what it is, which it does as it loads. */
const greeted = ask<{ hostContext?: object }>("ui/initialize", { protocolVersion: "2026-01-26", appInfo: { name: "", version: "1" }, appCapabilities: {} }).then(({ hostContext }) => {
  dress(hostContext)
  host?.post({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} })
})

export const call: typeof Page.call = async (tool, inputs = {}, { subject } = {}) => {
  await greeted
  const { content, isError } = await ask<{ content: { text?: string }[]; isError?: boolean }>("tools/call", { name: tool, arguments: inputs, ...(subject ? { _meta: { subject } } : {}) })
  const text = content.map(({ text }) => text ?? "").join("\n")
  if (isError) throw new Error(readable(text), { cause: text })
  return text
}

export const client = (async (change: Page.ClientChange = {}) => {
  await greeted
  return ask("one/client", change)
}) as typeof Page.client
export const safeArea: typeof Page.safeArea = () => insets()
export const viewport: typeof Page.viewport = () => size()
/** A native host says nothing yet of which way a page's words run, so they run left to right. */
export const direction: typeof Page.direction = () => "ltr"
/** Whether a finger points at it, as its host says (`deviceCapabilities.touch`): a phone's does, a computer's doesn't. */
export const touch = () => fingered()
export const reducedMotion: typeof Page.reducedMotion = () => still()
/** Whether the page is on screen, as its host says (`hostContext.visible`), as the DOM's `visibilityState` says it. */
export const visible: typeof Page.visible = () => shown()
export const sizeChanged: typeof Page.sizeChanged = async (size) => (await greeted, host?.post({ jsonrpc: "2.0", method: "ui/notifications/size-changed", params: size }))
export const pinShortcut: typeof Page.pinShortcut = async (entrypoint) => (await greeted, ask("one/pin-shortcut", { entrypoint }))
export const open: typeof Page.open = async (app, { subject, behind, from } = {}) => (await greeted, ask("ui/open-link", /^https?:/.test(app) ? { url: app } : { url: `one:${app}`, subject, behind, from }))

export const changed: typeof Page.changed = (listener) => {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}
/** What's open over the page, as the DOM's bridge keeps it (`covers`), which its host hears of, so its Back shuts it. */
const over: { shut: () => void; layer: boolean }[] = []
const said = async () => (await greeted, host?.post({ jsonrpc: "2.0", method: "notifications/one/over", params: { over: over.length > 0, layer: over.some(({ layer }) => layer) } }))
export const covers: typeof Page.covers = (shut, { layer = true } = {}) => {
  const one = { shut, layer }
  over.push(one)
  void said()
  return () => {
    if (!over.includes(one)) return
    over.splice(over.indexOf(one), 1)
    void said()
  }
}
export const connection: typeof Page.connection = () => connected()
/** Opening something is a step back closes, as the device's back closes what covers the page: a screen pushed over the
 * last, which isn't drawn over the page as a sheet or a menu is (a layer), as the DOM's history step isn't one. */
export const step = (close: () => void): (() => void) => covers(close, { layer: false })
/** A native host draws no move between screens yet: the change is made at once. */
export const go: typeof Page.go = (change) => change()

export const keydown: typeof Page.keydown = (listener) => {
  keys.add(listener)
  return () => void keys.delete(listener)
}

/** Back, where there's no history to go back through. */
export function back(): void {
  throw new Error("A native host has no history to go back through: a screen's Back says where it goes (`onBack`)")
}

/** Puts text on the clipboard, which its client does (`one/copy`): a page has none of its own. */
/** Sends a file the page chose where the network keeps it, as the DOM's bridge does: the host takes it as base64, the
 * only way a file crosses its JSON channel. */
export const upload: typeof Page.upload = async (file) => {
  const bytes = new Uint8Array(await file.arrayBuffer())
  let binary = ""
  for (let at = 0; at < bytes.length; at += 0x8000) binary += String.fromCharCode(...bytes.subarray(at, at + 0x8000))
  return (await greeted, (await ask<{ upload: string }>("one/upload", { body: btoa(binary) })).upload)
}
/** The device's own picker of files to open, as its client shows it (`one/files`): each file comes as base64, as an
 * upload goes. */
export const pickFiles: typeof Page.pickFiles = async (options = {}) => {
  await greeted
  const { files } = await ask<{ files: { name: string; body: string }[] }>("one/files", { accept: options.accept ?? [], multiple: !!options.multiple })
  return files.map((file) => new File([Uint8Array.from(atob(file.body), (character) => character.charCodeAt(0))], file.name))
}
export const copy = async (text: string): Promise<void> => (await greeted, void (await ask("one/copy", { text })))

/** Hands text to the device's share sheet, or copies it where there's none, as its client does (`one/share`). */
export const share: typeof Page.share = async (text) => (await greeted, (await ask<{ done: "shared" | "copied" | null }>("one/share", { text })).done ?? undefined)

/** Plays a haptic, as the DOM's does (`one/haptic`): a client without them says so, which is all there is to it. */
export const haptic: typeof Page.haptic = (kind) => void greeted.then(() => ask("one/haptic", { kind })).catch(() => {})

/** Tells the client the person is done with the app, as the DOM's does: MCP Apps' `request-teardown`. */
export const leave: typeof Page.leave = async () => (await greeted, host?.post({ jsonrpc: "2.0", method: "ui/notifications/request-teardown", params: {} }))

/** Has the client show its person their recovery key, or ask them for it, as the DOM's does (`one/recovery-key`). */
export const showRecoveryKey: typeof Page.showRecoveryKey = async ({ subject } = {}) => (await greeted, (await ask<{ done: boolean }>("one/recovery-key", { subject })).done)
export const useRecoveryKey: typeof Page.useRecoveryKey = async ({ subject } = {}) => (await greeted, (await ask<{ done: boolean }>("one/recovery-key", { typed: true, subject })).done)

/** What another app's client exports, from where the DOM's finds it: beside this app's page, under the client's root. */
export const component: typeof Page.component = (app, name) => import(/* @vite-ignore */ `/${app}/client/${name}.js`)
/** The link the page was opened to, as its host says (`hostContext.link`), as it comes. */
export const link: typeof Page.link = () => opened()
