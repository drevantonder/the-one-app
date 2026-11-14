// The bridge: how an app's page reaches its client, which is all it reaches. It speaks as an MCP App speaks to its
// host, in JSON-RPC messages posted to the frame's parent, so one page is an app here and an MCP App in an
// assistant.

const waiting = new Map()
const listeners = new Set()
let asked = 0

addEventListener("message", ({ source, data }) => {
  if (source !== parent) return
  if (data?.method === "notifications/one/changed") for (const listener of listeners) listener()
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

/// `await call("tracker_add", { tracker: "water" })`: calls a tool and gives its answer, which is text. A file a
/// tool answers with is its `data:` address, which an `<img>` takes as its `src`. A tool that refuses throws what
/// it said. The client keeps the answer of a tool that only reads, and answers with it at once.
export async function call(tool, inputs = {}) {
  await greet()
  const { content, isError } = await ask("tools/call", { name: tool, arguments: inputs })
  const text = content.map(({ text, data, mimeType, resource }) => text ?? `data:${mimeType ?? resource.mimeType};base64,${data ?? resource.blob}`).join("\n")
  if (isError) throw new Error(text)
  return text
}

/// `changed(show)`: runs `show` whenever what this page was answered has changed, to ask again and draw it.
export const changed = (listener) => void listeners.add(listener)

/// Tells the client the user went back out of the app, from its first screen.
export const leave = () => greet().then(() => tell("ui/notifications/request-teardown"))
