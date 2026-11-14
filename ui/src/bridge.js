// The bridge: how an app's page reaches its client, which is all it reaches. It speaks as an MCP App speaks to its
// host, in JSON-RPC messages posted to the frame's parent, so one page is an app here and an MCP App in an
// assistant.

const waiting = new Map()
const listeners = new Set()
let asked = 0

addEventListener("message", ({ source, data }) => {
  if (source !== parent) return
  if (data?.method === "notifications/one/changed") for (const listener of listeners) listener()
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

/// A client answers a page once the page has said what it is, which it does before the first thing it asks.
let greeted
const greet = () => greeted ??= ask("ui/initialize", {
  protocolVersion: "2026-01-26",
  appInfo: { name: document.title, version: "1" },
  appCapabilities: {},
}).then(() => tell("ui/notifications/initialized"))

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

/// `await call("tracker_add", { tracker: "water" })`: calls a tool and gives its answer, which is text. A file a
/// tool answers with is its address, which an `<img>` takes as its `src`: a `data:` one, or for a large file, one
/// the client keeps the file at. A tool that refuses throws what it said. The client keeps the answer of a tool
/// that only reads, and answers with it at once.
export async function call(tool, inputs = {}) {
  await greet()
  const { content, isError } = await ask("tools/call", { name: tool, arguments: inputs })
  const text = content.map(({ type, text, uri, data, mimeType, resource }) => text ?? (type === "resource_link" ? uri : `data:${mimeType ?? resource.mimeType};base64,${data ?? resource.blob}`)).join("\n")
  if (isError) throw new Error(text)
  return text
}

/// `await call("claims_attach", { expense: 4, name: file.name, type: file.type, upload: await upload(file) })`:
/// sends a file that's too large to go in a call straight to where the network keeps it, for a tool of this app's
/// plugin. → the name that tool takes for it, in the input its plugin says is an upload.
export const upload = (file) => greet().then(() => ask("one/upload", { file })).then(({ upload }) => upload)

/// `changed(show)`: runs `show` whenever what this page was answered has changed, to ask again and draw it.
export const changed = (listener) => void listeners.add(listener)

/// `screen.addEventListener("back", (event) => (event.preventDefault(), leave()))`: tells the client the user went
/// back out of the app, from its first screen.
export const leave = () => greet().then(() => tell("ui/notifications/request-teardown"))
