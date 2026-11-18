// The bridge: how an app's page reaches its client, which is all it reaches. It speaks as an MCP App speaks to its
// host, in JSON-RPC messages posted to the frame's parent, so one page is an app here and an MCP App in an
// assistant.
import { configureClientErrors, createSignal } from "solid-js"
import { readable, reduced } from "./shared.ts"

/** A JSON-RPC response, as a client answers what a page asked. */
interface JsonRpcResponse {
  id: number
  result?: unknown
  error?: { message: string }
}

/** What the client says of where the page is shown: light or dark, how far in from each edge what the device
 * keeps for itself reaches, whether the window names the app above it (an app's own window on a computer, in its
 * title bar's breadcrumb), whether its network answers, and whether an update for this device is ready. */
interface HostContext {
  theme?: "light" | "dark" | null
  safeAreaInsets?: { top: number; right: number; bottom: number; left: number }
  breadcrumb?: boolean
  connection?: Connection
  updateReady?: boolean
}

/** Whether this device reaches its network (`connection()`). */
export type Connection = "connecting" | "online" | "offline" | "reconnecting"

/** Who a call or an app is for, its subject: a group (`groups/family`), or `network`. Whoever the app is for, unless said. */
export interface Subject {
  subject?: string
}

const waiting = new Map<number, (response: JsonRpcResponse) => void>()
const listeners = new Set<() => void>()
let requests = 0

addEventListener("message", ({ source, data }) => {
  if (source !== parent) return
  if (data?.method === "notifications/one/changed") for (const listener of listeners) listener()
  else if (data?.method === "ui/notifications/host-context-changed") dress(data.params)
  // The device's own back, where a device has one and something is open over the page: the last opened shuts.
  else if (data?.method === "notifications/one/back") over.at(-1)?.()
  // A tap on an iPhone's status bar: what scrolls goes back to its top.
  else if (data?.method === "notifications/one/scroll-to-top") toTop()
  else waiting.get(data?.id)?.(data)
})

const notify = (method: string, params: object = {}) => parent.postMessage({ jsonrpc: "2.0", method, params }, "*")

/// A page's errors are logged to its client, as an MCP App logs to its host (MCP's `notifications/message`), which
/// tells Telemetry: the message, and the file and line. Those an `<Errored>` boundary caught and shows are too, once
/// each, as Solid tells of them (`configureClientErrors`), with where they were thrown from their stack.
const logged = (message: unknown, file?: string, line?: number) =>
  notify("notifications/message", { level: "error", data: { message: String(message), file, line } })
addEventListener("error", ({ message, filename, lineno }) => logged(message, filename, lineno))
addEventListener("unhandledrejection", ({ reason }) => logged(reason?.message ?? reason))
configureClientErrors({
  onError(error) {
    const [, file, line] = /([a-z]+:\/\/[^\s()]+?):(\d+):\d+/.exec((error as Error)?.stack ?? "") ?? []
    logged((error as Error)?.message ?? error, file, line ? Number(line) : undefined)
  },
})

const ask = <Result>(method: string, params: object = {}) => new Promise<Result>((resolve, refused) => {
  const id = ++requests
  waiting.set(id, ({ result, error }) => {
    waiting.delete(id)
    if (error) refused(new Error(error.message))
    else resolve(result as Result)
  })
  parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*")
})

const [connected, setConnected] = createSignal<Connection>("online")
const [updatable, setUpdatable] = createSignal(false)

/// `<Button disabled={connection() === "offline"}>Send</Button>`: whether this device reaches its network now, as it
/// changes: `"online"`, `"connecting"` as it opens, `"offline"`, or `"reconnecting"` once it has lost it. For what can't be done without it: what was answered stays
/// shown either way, and the client says it's offline itself. A client that doesn't say is `"online"`.
export const connection = () => (greet(), connected())

/// `<Show when={updateReady()}><Button onClick={() => client({ update: true })}>Update</Button></Show>`: whether an
/// update for this device is ready, as it changes, which `client({ update: true })` takes. The launcher shows it.
export const updateReady = () => (greet(), updatable())

/// Light or dark, where the client says which (Colour Mode), and otherwise the device's own: as
/// `color-scheme` for colours, and as a `light` or `dark` class for what can't follow it (tokens.css). And what the
/// device keeps for itself at each edge, as `--safe-top` and the rest, whether the window names the app above it
/// (`data-breadcrumb`, which a first screen's title gives way to), whether its network answers and whether an update
/// is ready.
function dress(context: HostContext | undefined) {
  if (context?.connection) setConnected(context.connection)
  if (context && "updateReady" in context) setUpdatable(context.updateReady === true)
  const root = document.documentElement
  if (context?.safeAreaInsets) for (const [edge, inset] of Object.entries(context.safeAreaInsets)) root.style.setProperty(`--safe-${edge}`, `${inset}px`)
  if (context && "breadcrumb" in context) root.toggleAttribute("data-breadcrumb", context.breadcrumb === true)
  if (!context || !("theme" in context)) return
  root.style.colorScheme = context.theme ?? ""
  for (const name of ["light", "dark"]) root.classList.toggle(name, context.theme === name)
}

// A field the page has the keyboard in stays in sight as the page is made smaller for the keyboard.
addEventListener("resize", () => {
  if (document.activeElement?.matches("input, textarea, select, [contenteditable]")) document.activeElement.scrollIntoView({ block: "nearest" })
})

/// Scrolls the page, and each side of a split, back to its top.
function toTop() {
  const behavior = reduced.matches ? "instant" : "smooth"
  for (const scrolled of [document.scrollingElement, ...document.querySelectorAll(".ui-split > .panes > *")]) scrolled?.scrollTo({ top: 0, behavior })
}

/// A client answers a page once the page has said what it is, which it does before the first thing it asks.
let greeted: Promise<void> | undefined
const greet = () => greeted ??= ask<{ hostContext?: HostContext }>("ui/initialize", {
  protocolVersion: "2026-01-26",
  appInfo: { name: document.title, version: "1" },
  appCapabilities: {},
}).then(({ hostContext }) => (dress(hostContext), notify("ui/notifications/initialized"), said()))

/// What's open over the page, the last opened last, each as the function that shuts it: a sheet, an alert, a
/// menu, a select's list, a calendar. A device's own back shuts the last of them before it goes anywhere, so the
/// client is told whether there are any, and with it whether the page is on its first screen, with nothing of its own
/// to go back to (the Navigation API's `canGoBack`, in its frame), where the device's back gesture leaves the app. A
/// browser without the Navigation API doesn't say.
const over: (() => void)[] = []
const said = () => notify("notifications/one/over", { over: over.length > 0, first: globalThis.navigation && !navigation.canGoBack })
// A page that hasn't said what it is yet says this as it does.
const overNow = () => (greeted ? greeted.then(said) : greet())
globalThis.navigation?.addEventListener("currententrychange", () => void overNow())

/// `const shut = covers(close)`: says something of the app's own has opened over its page, as a sheet or a menu
/// does, which `close` closes: the device's own back (an iPhone's swipe in from the edge, Android's Back) calls
/// `close` before it goes anywhere else. → the function to call once it has closed, however it was.
export function covers(shut: () => void): () => void {
  over.push(shut)
  if (over.length === 1) overNow()
  return () => {
    if (!over.includes(shut)) return
    over.splice(over.indexOf(shut), 1)
    if (!over.length) overNow()
  }
}

/** A piece of a tool's answer, as MCP says it. */
interface Content {
  type: string
  text?: string
  uri?: string
  data?: string
  mimeType?: string
  resource?: { mimeType?: string; blob?: string }
}

/// `await call("notes_add", { text: "Milk" })`: calls a tool and gives its answer, which is text. A file a
/// tool answers with is its address, which an `<img>` takes as its `src`: a `data:` one, or for a large file, one
/// the client keeps the file at. A tool that refuses throws an `Error` whose `message` is what a person reads of
/// what it said (`readable`), and whose `cause` is all of it, as an agent reads it. The client keeps the answer of a tool
/// that only reads, and answers with it at once. A screen that shows what a tool answers reads it in a memo:
/// `const notes = createMemo(() => call("notes_list").then(JSON.parse))`, under `<Loading>`.
/// An app opened from a group's Home Screen page is answered for the group, with nothing said. A page that shows more
/// than one of them says who each call is for: `call("home-screen_pages", {}, { subject: "groups/family" })`, or `network`.
export async function call(tool: string, inputs: object = {}, { subject }: Subject = {}): Promise<string> {
  await greet()
  const { content, isError } = await ask<{ content: Content[]; isError?: boolean }>("tools/call", { name: tool, arguments: inputs, ...(subject ? { _meta: { subject } } : {}) })
  const text = content.map(({ type, text, uri, data, mimeType, resource }) => text ?? (type === "resource_link" ? uri : `data:${mimeType ?? resource?.mimeType};base64,${data ?? resource?.blob}`)).join("\n")
  if (isError) throw new Error(readable(text), { cause: text })
  return text
}

/// `await call("notes_attach", { note: 4, name: file.name, type: file.type, upload: await upload(file) })`:
/// sends a file that's too large to go in a call straight to where the network keeps it, for a tool of this app's
/// app. → the name that tool takes for it, in the input its app says is an upload.
export const upload = (file: File | Blob): Promise<string> => greet().then(() => ask<{ upload: string }>("one/upload", { file })).then(({ upload }) => upload)

/// `await showRecoveryKey()`: has the client show its person their recovery key, over the app. The app is told
/// only that they've seen it: no app's page is handed one. → whether it was shown, which only a device that
/// holds their key can. It's the key of whoever the app is for, or as a call says it, `{ subject: "network" }`'s.
export const showRecoveryKey = ({ subject }: Subject = {}): Promise<boolean> => greet().then(() => ask<{ done: boolean }>("one/recovery-key", { subject })).then(({ done }) => done)

/// `await useRecoveryKey()`: has the client ask its person for their recovery key, and let this device in with
/// it. → whether the device holds their key now. Whose, as `showRecoveryKey` says.
export const useRecoveryKey = ({ subject }: Subject = {}): Promise<boolean> => greet().then(() => ask<{ done: boolean }>("one/recovery-key", { typed: true, subject })).then(({ done }) => done)

/// `haptic("medium")`: plays a haptic, by iOS's name: an impact (`"light"`, `"medium"`, `"heavy"`, `"rigid"`,
/// `"soft"`) for something physical, a lift, a snap, a landing; `"selection"` as a value changes step by step; or an
/// outcome (`"success"`, `"warning"`, `"error"`). An iPhone and Android's app play it, as does Chrome on Android, where
/// the device has them on; elsewhere it does nothing.
export const haptic = (kind: Haptic): void =>
  // An assistant showing the page as an MCP App has no haptics, and says so: that it played none is all there is.
  void greet().then(() => ask("one/haptic", { kind })).catch(() => {})

/** A haptic, by iOS's name (`haptic`). */
export type Haptic = "light" | "medium" | "heavy" | "rigid" | "soft" | "selection" | "success" | "warning" | "error"

/// `changed(() => refresh(notes))`: runs what it's given whenever what this page was answered has changed, to ask
/// again and draw it. → the function that stops it.
export function changed(listener: () => void): () => void {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}


/// `leave()`: tells the client the user is done with the app, as MCP Apps' `request-teardown`: the window goes back to
/// the Home Screen, as the device's back does from the app's first screen. An app's first screen has no Back for it:
/// the window's rail and the device's back leave every app.
export const leave = () => greet().then(() => notify("ui/notifications/request-teardown"))

/// `open("claims")`: shows another app over this one, by its app's name, as tapping its icon or a notification
/// from it does. `open("notes/client/index.html", { subject: "groups/family" })` names a page of the app's that isn't
/// its first app, and who it's for if not whoever this one is: a group, or `network`. It's how a Home Screen page opens
/// an app. With `behind: true` the app only starts, as a finger lands on its icon, and shows when it's opened
/// without. `from` is where its icon or widget is in this page, with its corners' `radius`, which the app grows out
/// of as it shows and goes back into as it closes, as an iPhone's do. `open("https://...")`, a web address, opens it
/// in the device's own browser, outside the app, as an outside service's sign-in page is opened.
export const open = (app: string, { subject, behind, from }: Subject & { behind?: boolean; from?: { x: number; y: number; width: number; height: number; radius?: number } } = {}): Promise<unknown> =>
  greet().then(() => ask("ui/open-link", /^https?:/.test(app) ? { url: app } : { url: `one:${app}`, subject, behind, from }))

/// `pinShortcut("notes/client/index.html")`: puts an app, by its entrypoint, on the device's own home screen, as its
/// own apps go there, which opens it here. Android asks its person where; a device that can't, which `client()` says
/// (`pinsShortcuts`), throws.
export const pinShortcut = (entrypoint: string): Promise<unknown> => greet().then(() => ask("one/pin-shortcut", { entrypoint }))

/// `const { device, network, build, server, push } = await client()`: this device's key, as Devices lists it, the
/// network's address, if it has one, the build it's on, its server's key if it's a server, and on a phone or in a
/// browser, whether it's woken by its notifications. `await client({ theme: "dark" })` has every app drawn dark, or `"light"`, or with
/// `null` as the device is, and `client({ leave: true })` signs this device out once it's been removed from its
/// person's devices. `client({ serve: true })` makes this device a server of its person's and `false` stops it being
/// one, `client({ update: true })` has it take the latest builds, and `client({ push: true })` has a phone whose push
/// failed try again, or a browser turn its notifications on, from its person's tap: each answers `{ message }`, what it did, or throws why it couldn't. `client({ statusBar: false })`
/// hides the device's status bar while this app is showing, as an iPhone's Home Screen does while it's edited, and
/// `true` shows it again; a device with none, or whose app can't, keeps what it has. An update shows over every app
/// as it's under way, and the app starts again on the new build. On Android, `client({ settings: "accessibility" })`
/// opens its Accessibility settings, and `"app"` the app's App Info, where its person allows what only they may.
export function client(): Promise<Client>
export function client(change: ClientChange): Promise<{ message: string }>
export function client(change: ClientChange = {}) {
  return greet().then(() => ask("one/client", change))
}

/** What `client()` answers. */
export interface Client {
  device: string
  network?: string
  build: string
  server?: string
  /** Where a phone or a browser is with being woken by its notifications, `homeScreen` in an iPhone's browser, which is only from its Home Screen: nothing on a computer. */
  push?: { state: "notAsked" | "denied" | "registering" | "registered" | "homeScreen" } | { state: "failed"; why: string }
  /** Whether a phone's person turned on Computer Use in its Accessibility settings: nothing where a device has none. */
  computerUse?: { on: boolean }
  /** What `client({ theme })` chose for every app here: `null` where it's as the device is. */
  theme: "light" | "dark" | null
  /** Whether `pinShortcut` puts an app on the device's own home screen: on Android's app. */
  pinsShortcuts?: boolean
  /** Whether the page is a computer's launcher, the panel its Home Screen is shown in, which goes once an app opens. */
  launcher?: boolean
}

/** What `client(change)` changes. */
export interface ClientChange {
  theme?: "light" | "dark" | null
  statusBar?: boolean
  leave?: boolean
  serve?: boolean
  update?: boolean
  push?: boolean
  settings?: "accessibility" | "app"
}
