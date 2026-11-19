// The bridge: how an app's page reaches its client, which is all it reaches. It speaks as an MCP App speaks to its
// host, in JSON-RPC messages posted to the frame's parent, so one page is an app here and an MCP App in an
// assistant.
import { configureClientErrors, createSignal, flush, onSettled } from "solid-js"
import { readable, reduced } from "./shared.ts"
import type { JSX } from "@solidjs/web"
import type { Client, ClientChange } from "./types/ui.d.ts"

/** A JSON-RPC response, as a client answers what a page asked. */
interface JsonRpcResponse {
  id: number
  result?: unknown
  error?: { message: string }
}

/** What the client says of where the page is shown: light or dark, how far in from each edge what the device
 * keeps for itself reaches, whether the window names the app above it (an app's own window on a computer, in its
 * title bar's breadcrumb), and whether its network answers. */
interface HostContext {
  theme?: "light" | "dark" | null
  safeAreaInsets?: { top: number; right: number; bottom: number; left: number }
  breadcrumb?: boolean
  /** Where a computer's window has its title row over the page, the page's top bar: where in it the page's own may
   * start, after the window's controls and breadcrumb, and how much of its end the window's controls take. */
  titleBar?: { start: number; end: number }
  connection?: Connection
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
  if (source !== host) return
  if (data?.method === "notifications/one/changed") for (const listener of listeners) listener()
  else if (data?.method === "ui/notifications/host-context-changed") dress(data.params)
  // The device's own back, where a device has one and something is open over the page: the last opened shuts.
  else if (data?.method === "notifications/one/back") over.at(-1)?.shut()
  // A tap on an iPhone's status bar: what scrolls goes back to its top.
  else if (data?.method === "notifications/one/scroll-to-top") toTop()
  // How tall a page in one of this page's frames says its content is.
  else if (data?.method === "notifications/one/framed") sized.get(data.params?.frame)?.(data.params?.height)
  else waiting.get(data?.id)?.(data)
})

/// Where this page's client is: its parent, or, for a page in another app's frame (`Frame`), the window that page is
/// in, which knows it by the number in its address (`frame`), as its own app's.
const framed = new URLSearchParams(location.search).get("frame")
const host: Window = (framed && parent !== top && top) || parent
const say = (message: object) => host.postMessage({ jsonrpc: "2.0", ...message, ...(framed && host !== parent && { frame: framed }) }, "*")
const notify = (method: string, params: object = {}) => say({ method, params })

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

/// What the page has asked its client that isn't answered yet.
const asked = new Set<Promise<unknown>>()

const ask = <Result>(method: string, params: object = {}) => {
  const answer = new Promise<Result>((resolve, refused) => {
    const id = ++requests
    waiting.set(id, ({ result, error }) => {
      waiting.delete(id)
      if (error) refused(new Error(error.message))
      else resolve(result as Result)
    })
    say({ id, method, params })
  })
  const answered = () => asked.delete(answer)
  asked.add(answer)
  answer.then(answered, answered)
  return answer
}

const [connected, setConnected] = createSignal<Connection>("online")

const [shown, setShown] = createSignal(document.visibilityState === "visible")
document.addEventListener("visibilitychange", () => setShown(document.visibilityState === "visible"))

/// `createEffect(visible, (now, was) => now && was === false && refresh(phone))`: whether this page is shown, as it
/// changes: not while its app is in the background or its person is in another app, as Settings. For what's asked
/// again once they're back.
export const visible = (): boolean => shown()

/// `<Button disabled={connection() === "offline"}>Send</Button>`: whether this device reaches its network now, as it
/// changes: `"online"`, `"connecting"` as it opens, `"offline"`, or `"reconnecting"` once it has lost it. For what can't be done without it: what was answered stays
/// shown either way, and the client says it's offline itself. A client that doesn't say is `"online"`.
export const connection = () => (greet(), connected())

/// Light or dark, where the client says which (Colour Mode), and otherwise the device's own: as
/// `color-scheme` for colours, and as a `light` or `dark` class for what can't follow it (tokens.css). And what the
/// device keeps for itself at each edge, as `--safe-top` and the rest (tokens.ts's `safeTop`), whether the window names
/// the app above it (`breadcrumb`), and whether its network answers.
function dress(context: HostContext | undefined) {
  if (context?.connection) setConnected(context.connection)
  const root = document.documentElement
  if (context?.safeAreaInsets) for (const [edge, inset] of Object.entries(context.safeAreaInsets)) root.style.setProperty(`--safe-${edge}`, `${inset}px`)
  if (context?.safeAreaInsets) setInsets(context.safeAreaInsets)
  if (context && "breadcrumb" in context) setNamed(context.breadcrumb === true)
  if (context?.titleBar) {
    root.style.setProperty("--title-bar-start", `${context.titleBar.start}px`)
    root.style.setProperty("--title-bar-end", `${context.titleBar.end}px`)
    titled = true
  }
  if (!context || !("theme" in context)) return
  root.style.colorScheme = context.theme ?? ""
  for (const name of ["light", "dark"]) root.classList.toggle(name, context.theme === name)
}

const [insets, setInsets] = createSignal({ top: 0, right: 0, bottom: 0, left: 0 })

/// Whether the window's title row is over this page (`titleBar`): then its top bar is the window's title bar, which
/// moves the window held anywhere nothing is pressed, and does what the system's does double-clicked, as macOS's
/// System Settings' and Raycast's unified toolbars do.
let titled = false
const pressable = "a, button, input, select, textarea, [contenteditable], [tabindex], [role=button], [role=link], [role=tab], [role=switch], [role=checkbox], [role=searchbox], [role=combobox], [role=menuitem], [role=option]"
const inTitleRow = (event: MouseEvent) =>
  titled && event.button === 0 && event.clientY < insets().top && !(event.target instanceof Element && event.target.closest(pressable))
/// Where it was pressed, until the pointer moves far enough to drag the window, as a title bar's own does: a drag that
/// started at the press would take the pointer from the page, and the second press of a double-click with it.
let pressedAt: { x: number; y: number } | undefined
document.addEventListener("mousedown", (event) => {
  pressedAt = inTitleRow(event) && event.detail === 1 ? { x: event.clientX, y: event.clientY } : undefined
})
document.addEventListener("mousemove", (event) => {
  if (!pressedAt || !(event.buttons & 1) || Math.hypot(event.clientX - pressedAt.x, event.clientY - pressedAt.y) < 4) return
  pressedAt = undefined
  notify("one/drag-window")
})
document.addEventListener("mouseup", (event) => {
  pressedAt = undefined
  if (event.detail === 2 && inTitleRow(event)) notify("one/title-bar-double-click")
})
/// `<View style={look.top(safeArea().top)}>`, with `top: (paddingTop: number) => ({ paddingTop })` in `styles()`: what the device keeps for itself at each edge of the window (a
/// status bar, a notch, a home indicator), in CSS pixels, as it changes: CSS's `--safe-top` and the rest, as numbers,
/// for what's laid out by hand.
export const safeArea = () => (greet(), insets())

const [size, setSize] = createSignal({ width: innerWidth, height: innerHeight })
addEventListener("resize", () => setSize({ width: innerWidth, height: innerHeight }))
/// `viewport().width`: the window's size, in CSS pixels, as it changes, for what's laid out by hand.
export const viewport = () => size()
// Whether what points at the page is a finger, as it changes: the device's own pickers are for it.
const coarse = matchMedia("(pointer: coarse)")
const [fingered, setFingered] = createSignal(coarse.matches)
coarse.addEventListener("change", () => setFingered(coarse.matches))
export const touch = () => fingered()
/// `direction()`: which way the page's words run, as its language has them: `rtl` for Arabic or Hebrew.
export const direction = (): "ltr" | "rtl" => (getComputedStyle(document.documentElement).direction === "rtl" ? "rtl" : "ltr")

const [still, setStill] = createSignal(reduced.matches)
reduced.addEventListener("change", () => setStill(reduced.matches))
/// `reducedMotion()`: whether whoever's looking asked for less motion, as it changes, for what moves by hand.
export const reducedMotion = () => still()

/// `keydown((event) => event.key === "Escape" && close())`: each key pressed that isn't something's own, not in a
/// field, a menu or a dialog, which take their keys themselves. → what stops it.
export function keydown(listener: (event: KeyboardEvent) => void): () => void {
  // A shortcut held with ⌘ or Ctrl (⌘K, ⌘↵) is heard from a field too, as a computer's app's menus hear it.
  const heard = (event: KeyboardEvent) => !event.defaultPrevented && (event.metaKey || event.ctrlKey || !(event.target as Element).closest?.("input, textarea, select, dialog, [role=menu], [role=dialog]")) && listener(event)
  addEventListener("keydown", heard)
  return () => removeEventListener("keydown", heard)
}

// A field the page has the keyboard in stays in sight as the page is made smaller for the keyboard.
addEventListener("resize", () => {
  if (document.activeElement?.matches("input, textarea, select, [contenteditable]")) document.activeElement.scrollIntoView({ block: "nearest" })
})

/// Scrolls the page, and everything in it that scrolls (a screen, each side of a split), back to its top.
function toTop() {
  const behavior = reduced.matches ? "instant" : "smooth"
  for (const scrolled of [document.scrollingElement, ...document.querySelectorAll("[data-scrolls]")]) scrolled?.scrollTo({ top: 0, behavior })
}

const [named, setNamed] = createSignal(false)
/** Whether the window names the app above it, in its title bar's breadcrumb (an app's own window on a computer),
 * which a first screen's title gives way to. */
export const breadcrumb = () => named()

/// Whether the device has itself just shown the user going back (an iPhone's swipe in from the edge): the page
/// mustn't show it a second time.
let travelled = false
addEventListener("popstate", (event) => {
  travelled = event.hasUAVisualTransition
  setTimeout(() => (travelled = false))
}, true)

/// `go(() => setOpen(note))`: changes which screen shows, as a move. `change` puts the new screen in place: on a native
/// host at once, and on the DOM the browser slides to it from what was there (its View Transitions): in over it, or
/// with `back`, back out from over it. The browser takes its picture of the new screen once it's drawn, with what it was still loading, as the View
/// Transitions guide has it: its picture of the old one stays on screen until then. For whoever asked for less motion
/// one fades into the other, and where the browser can't, it's a plain swap. From one page to another the browser
/// does this by itself (ui.css).
export function go(change: () => void, { back = false }: { back?: boolean } = {}): void {
  if (!globalThis.ViewTransition || !("types" in ViewTransition.prototype) || travelled) return void change()
  // What's still fading (a row let go of a moment ago) is finished first, so the move takes the screen as it settles.
  // Chromium's page crashed taking it mid-fade. What never ends, a spinner, keeps going.
  for (const fade of document.getAnimations()) if (fade.effect?.getComputedTiming().endTime !== Infinity) fade.finish()
  const update = () => new Promise<void>((settled) => (change(), onSettled(settled), flush()))
  document.startViewTransition({ update, types: reduced.matches ? [] : [back ? "ui-back" : "ui-forward", "ui-page"] })
}

// Back to a page left earlier is the device's to show. The browser would slide it in as if it were new.
addEventListener("pageswap", ({ activation, viewTransition }) => {
  if (activation?.navigationType === "traverse" && activation.from && activation.entry.index < activation.from.index) viewTransition?.skipTransition()
})

// The page was loaded again with something open as a step (`step`): what was open is gone, so it starts shut.
if (history.state?.uiStep) history.replaceState({ ...history.state, uiStep: 0 }, "")
const depth = () => Number(history.state?.uiStep ?? 0)

// `const done = step(close)`: something opened is a step in the browser's history, as a split's detail pushed over
// its list is, so the device's back (and the browser's) goes back from it, calling `close`. Steps open in one another
// are one each, so back closes the innermost first. → what to call once it's closed however it was, which takes its
// step out of the history if it's still there.
export function step(close: () => void): () => void {
  const mine = depth() + 1
  history.pushState({ ...history.state, uiStep: mine }, "")
  const left = () => depth() < mine && (removeEventListener("popstate", left), close())
  addEventListener("popstate", left)
  return () => {
    removeEventListener("popstate", left)
    if (depth() >= mine) history.back()
  }
}

/** Back through the browser's history, as the device's own back gesture goes. */
export const back = () => history.back()

/** Puts text on the clipboard. */
export const copy = (text: string) => navigator.clipboard.writeText(text)

/// A client answers a page once the page has said what it is, which it does before the first thing it asks.
let greeted: Promise<void> | undefined
const greet = () => greeted ??= ask<{ hostContext?: HostContext }>("ui/initialize", {
  protocolVersion: "2026-01-26",
  appInfo: { name: document.title, version: "1" },
  appCapabilities: {},
}).then(({ hostContext }) => (dress(hostContext), notify("ui/notifications/initialized"), said()))

/// Tells the client the page has drawn, as a browser's Largest Contentful Paint marks a page as shown, and not before:
/// `render` calls it once the page has rendered and everything it asked for has been answered, an error as well, which
/// the page's `<Errored>` shows, and what they drew is in the page. A page still on its skeleton, or blank, hasn't.
export async function tellDrawn() {
  await greet()
  do await Promise.allSettled(asked)
  while (await new Promise((later) => setTimeout(() => later(asked.size))))
  notify("notifications/one/drawn")
}

/// What the device's own back shuts before it goes anywhere, the last opened last, each as the function that shuts it:
/// a sheet, an alert, a menu, a select's list, a calendar, each a layer drawn over the page, and a screen's own Back,
/// which isn't. The client is told whether there are any, whether any is a layer, which the window's rail keeps out of
/// the way of, and whether the page is on its first screen, with nothing of its own to go back to (the Navigation
/// API's `canGoBack`, in its frame), where the device's back gesture leaves the app. A browser without the Navigation
/// API doesn't say.
const over: { shut: () => void; layer: boolean }[] = []
const said = () => notify("notifications/one/over", { over: over.length > 0, layer: over.some(({ layer }) => layer), first: globalThis.navigation && !navigation.canGoBack })
// A page that hasn't said what it is yet says this as it does.
const overNow = () => (greeted ? greeted.then(said) : greet())
globalThis.navigation?.addEventListener("currententrychange", () => void overNow())

/// `const shut = covers(close)`: says something of the app's own has opened over its page, as a sheet or a menu
/// does, which `close` closes: the device's own back (an iPhone's swipe in from the edge, Android's Back) calls
/// `close` before it goes anywhere else. `{ layer: false }` where nothing is drawn over the page, as a screen's own
/// Back. → the function to call once it has closed, however it was.
export function covers(shut: () => void, { layer = true }: { layer?: boolean } = {}): () => void {
  const one = { shut, layer }
  over.push(one)
  overNow()
  return () => {
    if (!over.includes(one)) return
    over.splice(over.indexOf(one), 1)
    overNow()
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
/// tool answers with is its address, which an `<Image>` takes as its `src`: a `data:` one, or for a large file, one
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
/// `const [file] = await pickFiles({ accept: [".png", "image/*"] })`: the device's own picker of files to open, from a
/// press, as a browser shows it for `<input type="file">`. → the files chosen, none where it was cancelled.
export function pickFiles(options: { accept?: string[]; multiple?: boolean } = {}): Promise<File[]> {
  return new Promise((resolve) => {
    const input = Object.assign(document.createElement("input"), { type: "file", accept: (options.accept ?? []).join(","), multiple: !!options.multiple })
    input.addEventListener("change", () => resolve([...(input.files ?? [])]))
    input.addEventListener("cancel", () => resolve([]))
    input.click()
  })
}
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

/// `createEffect(link, (to) => ...)`: the link the page was opened to last, `one:settings#vault/accounts&request=4`'s `vault/accounts&request=4`, as it
/// changes: a link followed while the page is open is told too, the same one again as well. The page's address
/// forgets it once it's read, so following it again is news. None until a link names more than the app.
export function link(): string | undefined {
  return (linked ??= follow())()
}
let linked: (() => string | undefined) | undefined
function follow() {
  const read = () => {
    const to = location.hash.slice(1)
    if (to) history.replaceState(history.state, "", location.pathname + location.search)
    return to || undefined
  }
  const [to, setTo] = createSignal(read(), { equals: false })
  addEventListener("hashchange", () => {
    const now = read()
    if (now) setTo(now)
  })
  return to
}

/// `const { Summary } = await component("chrome-profiles", "profile")`: what another app's client exports
/// (its client/package.json names it), as a module the client serves beside this app's page: its components, drawn
/// here with what they're given.
export const component = (app: string, name: string): Promise<Record<string, (props: Record<string, unknown>) => JSX.Element>> =>
  import(/* @vite-ignore */ new URL(`../../${app}/client/${name}.js`, document.baseURI).href)

// `const { url, sandbox } = await frame("chrome-profiles/client/profile.html#work")`: where another app's page is, as
// its own app's, for `subject`, which `Frame` draws in an iframe: its client answers it as that app's, as it does
// this page as its own. Only a trusted app's page may.
// A frame's page that's gone, by its number, which its client forgets.
export const unframe = (frame: string) => greet().then(() => ask("one/frame", { forget: frame }))

/// Each frame's (`frame`) way to be told how tall its page says its content is, by its number.
export const sized = new Map<string, (height: number) => void>()
export const frame = (src: string, subject?: string) => greet().then(() => ask<{ url: string; sandbox: string; frame: string }>("one/frame", { src, subject }))

/// `sizeChanged({ height: 420 })`: tells the client how large the page's content is, in CSS pixels, as an MCP App tells
/// its host (`ui/notifications/size-changed`). A window sized by what it shows, a computer's launcher, takes it; one
/// whose page fills it keeps its size.
export const sizeChanged = (size: { width?: number; height?: number }) => greet().then(() => notify("ui/notifications/size-changed", size))

/// `pinShortcut("notes/client/index.html")`: puts an app, by its entrypoint, on the device's own home screen, as its
/// own apps go there, which opens it here. Android asks its person where; a device that can't, which `client()` says
/// (`pinsShortcuts`), throws.
export const pinShortcut = (entrypoint: string): Promise<unknown> => greet().then(() => ask("one/pin-shortcut", { entrypoint }))

/// `const { device, network, build, server, push } = await client()`: this device's key, as Devices lists it, the
/// network's address, if it has one, the build it's on, its server's key and state if it's a server, and on a phone or in a
/// browser, whether it's woken by its notifications. `await client({ theme: "dark" })` has every app drawn dark, or `"light"`, or with
/// `null` as the device is, and `client({ leave: true })` signs this device out once it's been removed from its
/// person's devices. `client({ serve: true })` makes this device a server of its person's and `false` stops it being
/// one, `client({ update: true })` has it take the latest builds, and `client({ push: true })` has a phone whose push
/// failed try again, or a browser turn its notifications on, from its person's tap: each answers `{ message }`, what it did, or throws why it couldn't. `client({ statusBar: false })`
/// hides the device's status bar while this app is showing, as an iPhone's Home Screen does while it's edited, and
/// `true` shows it again; a device with none, or whose app can't, keeps what it has. An update shows over every app
/// as it's under way, and the app starts again on the new build. On Android, `client({ settings: "accessibility" })`
/// opens its Accessibility settings, and `"app"` the app's App Info, on an iPhone its page in Settings, where its
/// person allows what only they may.
export function client(): Promise<Client>
export function client(change: ClientChange): Promise<{ message: string }>
export function client(change: ClientChange = {}): Promise<Client | { message: string }> {
  const mac = /Mac|iPhone|iPad/.test(navigator.platform)
  return greet()
    .then(() => ask<Client & { message: string }>("one/client", change))
    .then((answer) => {
      if (Object.keys(change).length) return answer
      // A page shown in a computer's launcher is drawn over its panel, the system's glass (ui.css).
      document.documentElement.toggleAttribute("data-launcher", answer.launcher === true)
      return { ...answer, mac }
    })
}

