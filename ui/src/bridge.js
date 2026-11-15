// The bridge: how an app's page reaches its client, which is all it reaches. It speaks as an MCP App speaks to its
// host, in JSON-RPC messages posted to the frame's parent, so one page is an app here and an MCP App in an
// assistant.

const waiting = new Map()
const listeners = new Set()
let asked = 0

addEventListener("message", ({ source, data }) => {
  if (source !== parent) return
  if (data?.method === "notifications/one/changed") for (const listener of listeners) listener()
  else if (data?.method === "ui/notifications/host-context-changed") dress(data.params)
  // The device's own back, where a device has one and something is open over the page: the last opened shuts.
  else if (data?.method === "notifications/one/back") over.at(-1)?.()
  else waiting.get(data?.id)?.(data)
})

const tell = (method, params = {}) => parent.postMessage({ jsonrpc: "2.0", method, params }, "*")

const ask = (method, params = {}) => new Promise((answered, refused) => {
  const id = ++asked
  waiting.set(id, ({ result, error }) => {
    waiting.delete(id)
    if (error) refused(new Error(error.message))
    else answered(result)
  })
  parent.postMessage({ jsonrpc: "2.0", id, method, params }, "*")
})

/// Light or dark, where the client says which (Settings › Appearance), and otherwise the device's own.
const dress = (context) => context && "theme" in context && (document.documentElement.style.colorScheme = context.theme ?? "")

/// A client answers a page once the page has said what it is, which it does before the first thing it asks.
let greeted
const greet = () => greeted ??= ask("ui/initialize", {
  protocolVersion: "2026-01-26",
  appInfo: { name: document.title, version: "1" },
  appCapabilities: {},
}).then(({ hostContext }) => (dress(hostContext), tell("ui/notifications/initialized")))

/// What's open over the page, the last opened last, each as the function that shuts it: a sheet, an alert, a
/// menu, a select's list, a calendar. A device's own back shuts the last of them before it goes anywhere, so the
/// client is told whether there are any.
const over = []
const overNow = () => greet().then(() => tell("notifications/one/over", { over: over.length > 0 }))

/// Says something has opened over the page, which `shut` shuts. → the function that says it has shut.
export function covers(shut) {
  over.push(shut)
  if (over.length === 1) overNow()
  return () => {
    if (!over.includes(shut)) return
    over.splice(over.indexOf(shut), 1)
    if (!over.length) overNow()
  }
}

/// `await call("notes_add", { text: "Milk" })`: calls a tool and gives its answer, which is text. A file a
/// tool answers with is its address, which an `<img>` takes as its `src`: a `data:` one, or for a large file, one
/// the client keeps the file at. A tool that refuses throws what it said. The client keeps the answer of a tool
/// that only reads, and answers with it at once. A screen that shows what a tool answers reads it in a memo:
/// `const notes = createMemo(() => call("notes_list").then(JSON.parse))`, under `<Loading>`.
/// An app opened from a group's home page is answered for the group, with nothing said. A page that shows more
/// than one of them says who each call is for: `call("home_page", {}, { for: "groups/family" })`, or `network`.
export async function call(tool, inputs = {}, who = {}) {
  await greet()
  const { content, isError } = await ask("tools/call", { name: tool, arguments: inputs, ...(who.for ? { _meta: { for: who.for } } : {}) })
  const text = content.map(({ type, text, uri, data, mimeType, resource }) => text ?? (type === "resource_link" ? uri : `data:${mimeType ?? resource.mimeType};base64,${data ?? resource.blob}`)).join("\n")
  if (isError) throw new Error(text)
  return text
}

/// `await call("notes_attach", { note: 4, name: file.name, type: file.type, upload: await upload(file) })`:
/// sends a file that's too large to go in a call straight to where the network keeps it, for a tool of this app's
/// plugin. → the name that tool takes for it, in the input its plugin says is an upload.
export const upload = (file) => greet().then(() => ask("one/upload", { file })).then(({ upload }) => upload)

/// `await showRecoveryKey()`: has the client show its person their recovery key, over the app. The app is told
/// only that they've seen it: no app's page is handed one. → whether it was shown, which only a device that
/// holds their key can.
export const showRecoveryKey = () => greet().then(() => ask("one/recovery-key")).then(({ done }) => done)

/// `await useRecoveryKey()`: has the client ask its person for their recovery key, and let this device in with
/// it. → whether the device holds their key now.
export const useRecoveryKey = () => greet().then(() => ask("one/recovery-key", { typed: true })).then(({ done }) => done)

/// `changed(() => refresh(notes))`: runs what it's given whenever what this page was answered has changed, to ask
/// again and draw it. → the function that stops it.
export function changed(listener) {
  listeners.add(listener)
  return () => listeners.delete(listener)
}


/// `<Screen heading="Notes" back onBack={leave}>`: tells the client the user went back out of the app, from its
/// first screen.
export const leave = () => greet().then(() => tell("ui/notifications/request-teardown"))

/// `open("claims")`: shows another app over this one, by its plugin's name, as tapping its icon or a notification
/// from it does. `open("notes/app/index.html", { for: "groups/family" })` names a page of the plugin's that isn't
/// its first app, and who it's for if not whoever this one is: a group, or `network`. It's how a home page opens
/// an app. With `behind: true` the app only starts, as a finger lands on its icon, and shows when it's opened
/// without. `open("https://...")`, a web address, opens it in the device's own browser, outside the app, as an
/// outside service's sign-in page is opened.
export const open = (app, who = {}) =>
  greet().then(() => ask("ui/open-link", /^https?:/.test(app) ? { url: app } : { url: `one:${app}`, for: who.for, behind: who.behind }))

/// `const { device, network, build, server } = await client()`: this device's key, as Devices lists it, the
/// network's address, if it has one, the build it's on, and its server's key if it's a server. `await client({
/// theme: "dark" })` has every app drawn dark, or `"light"`, or with `null` as the device is, and `client({ leave:
/// true })` starts the client again once its device has been taken out. `client({ serve: true })` makes this device
/// a server of its person's and `false` stops it being one, and `client({ update: true })` has it take the newest
/// builds: each answers `{ said }`, what it did, or throws why it couldn't.
export const client = (change = {}) => greet().then(() => ask("one/client", change))
