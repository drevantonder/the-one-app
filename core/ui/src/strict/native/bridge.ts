// The bridge on a native host, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other,
// through the host, and what the client says of where the page is shown. The UI library's components import it as
// `#bridge`, which is this in the native build and the DOM's in the DOM's.
import { createSignal } from "solid-js"
import { event } from "./renderer.ts"
import type { Event } from "../protocol.ts"
import type * as Strict from "../types/ui.d.ts"
import { readable } from "../../words.ts"
// The bridge, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other, through the host.

const host = (globalThis as { host?: { post(message: object): void } }).host
const waiting = new Map<number, (response: { result?: unknown; error?: { message: string } }) => void>()
const listeners = new Set<() => void>()
let requests = 0
/** What listens for each key pressed (`keydown`). */
const keys = new Set<(event: Strict.Keyed) => void>()

;(globalThis as { ui?: object }).ui = {
  // A key pressed where the keyboard is goes to what has it, then to what listens for every key (`keydown`).
  event(told: Event) {
    event(told)
    if (told.name === "keydown") for (const listener of keys) listener({ key: told.key ?? "", shiftKey: !!told.shiftKey, altKey: !!told.altKey, ctrlKey: !!told.ctrlKey, metaKey: !!told.metaKey, preventDefault() {} })
  },
  message(message: { id?: number; method?: string; result?: unknown; error?: { message: string } }) {
    if (message.method === "notifications/one/changed") for (const listener of listeners) listener()
    else if (message.method === "ui/notifications/host-context-changed") dress((message as { params?: object }).params)
    else if (message.id !== undefined) waiting.get(message.id)?.(message)
  },
}

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
const [connected, setConnected] = createSignal<ReturnType<typeof Strict.connection>>("online")
const [insets, setInsets] = createSignal({ top: 0, right: 0, bottom: 0, left: 0 })
const [size, setSize] = createSignal({ width: 0, height: 0 })
const [still, setStill] = createSignal(false)
interface Context {
  breadcrumb?: boolean
  connection?: ReturnType<typeof Strict.connection>
  safeAreaInsets?: ReturnType<typeof Strict.safeArea>
  containerDimensions?: { width: number; height: number }
  reducedMotion?: boolean
}
function dress(context?: Context) {
  if (context && "breadcrumb" in context) setBreadcrumb(context.breadcrumb === true)
  if (context?.connection) setConnected(context.connection)
  if (context?.safeAreaInsets) setInsets(context.safeAreaInsets)
  if (context?.containerDimensions) setSize(context.containerDimensions)
  if (context && "reducedMotion" in context) setStill(context.reducedMotion === true)
}

/** A client answers a page once it has said what it is, which it does as it loads. */
const greeted = ask<{ hostContext?: object }>("ui/initialize", { protocolVersion: "2026-01-26", appInfo: { name: "", version: "1" }, appCapabilities: {} }).then(({ hostContext }) => {
  dress(hostContext)
  host?.post({ jsonrpc: "2.0", method: "ui/notifications/initialized", params: {} })
})

export const call: typeof Strict.call = async (tool, inputs = {}, { subject } = {}) => {
  await greeted
  const { content, isError } = await ask<{ content: { text?: string }[]; isError?: boolean }>("tools/call", { name: tool, arguments: inputs, ...(subject ? { _meta: { subject } } : {}) })
  const text = content.map(({ text }) => text ?? "").join("\n")
  if (isError) throw new Error(readable(text), { cause: text })
  return text
}

export const client = (async (change: Strict.ClientChange = {}) => {
  await greeted
  return ask("one/client", change)
}) as typeof Strict.client
export const safeArea: typeof Strict.safeArea = () => insets()
export const viewport: typeof Strict.viewport = () => size()
/** A native host says nothing yet of which way a page's words run, so they run left to right. */
export const direction: typeof Strict.direction = () => "ltr"
/** A native host on a computer is pointed at by a mouse; a phone's says so once it draws natively. */
export const touch = () => false
export const reducedMotion: typeof Strict.reducedMotion = () => still()
export const sizeChanged: typeof Strict.sizeChanged = async (size) => (await greeted, host?.post({ jsonrpc: "2.0", method: "ui/notifications/size-changed", params: size }))
export const pinShortcut: typeof Strict.pinShortcut = async (entrypoint) => (await greeted, ask("one/pin-shortcut", { entrypoint }))
export const open: typeof Strict.open = async (app, { subject, behind, from } = {}) => (await greeted, ask("ui/open-link", /^https?:/.test(app) ? { url: app } : { url: `one:${app}`, subject, behind, from }))

export const changed: typeof Strict.changed = (listener) => {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}
export const covers: typeof Strict.covers = () => () => {}
export const connection: typeof Strict.connection = () => connected()
/** Opening something is a step back closes, as the device's back closes what covers the page. */
export const step = covers
/** A native host draws no move between screens yet: the change is made at once. */
export const go: typeof Strict.go = (change) => change()

export const keydown: typeof Strict.keydown = (listener) => {
  keys.add(listener)
  return () => void keys.delete(listener)
}

/** Back, where there's no history to go back through. */
export function back(): void {
  throw new Error("A native host has no history to go back through: a screen's Back says where it goes (`onBack`)")
}

/** Puts text on the clipboard. */
export function copy(_text: string): Promise<void> {
  return Promise.reject(new Error("A native host has no clipboard yet (core/ui/src/strict/native/bridge.ts)"))
}
