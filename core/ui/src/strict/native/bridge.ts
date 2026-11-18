// The bridge on a native host, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other,
// through the host, and what the client says of where the page is shown. The UI library's components import it as
// `#bridge`, which is this in the native build and the DOM's in the DOM's.
import { createSignal } from "solid-js"
import { event } from "./renderer.ts"
import type * as Strict from "../types/ui.d.ts"
import { readable } from "../../words.ts"
// The bridge, as the DOM's (../../bridge.ts): the JSON-RPC a page and its client say to each other, through the host.

const host = (globalThis as { host?: { post(message: object): void } }).host
const waiting = new Map<number, (response: { result?: unknown; error?: { message: string } }) => void>()
const listeners = new Set<() => void>()
let requests = 0

;(globalThis as { ui?: object }).ui = {
  event,
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
