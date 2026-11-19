// The renderer's gate on a person's own devices: a client for the check page (check/), drawn by the renderer in the
// frame, as the web client is for an app's page. It answers what the page asks a client, and sends each check the
// person answers to the network's Telemetry (`one.client.web.check`), with what the frame saw happen (`seen`): its
// first frame, each kind of edit its real field had (typed, a suggestion, dictation, a password manager's fill, an input
// method), and a screen reader's presses. Telemetry has it once this browser is signed in at this address.
const frame = /** @type {HTMLIFrameElement} */ (document.getElementById("page"))
const said = /** @type {HTMLElement} */ (document.getElementById("said"))
// Built with this renderer's own UI library, so it names no runtime version it needs.
frame.src = "/renderer/#/renderer/check/index.html"
/** What the frame saw happen, by kind. @type {Record<string, unknown>} */
const seen = {}

/** Which browser and system this is, as the web client says it (clients/web/host.js). */
const browser = ["Edge", "Firefox", "Chrome", "Safari"].find((name) => navigator.userAgent.includes(name === "Edge" ? "Edg/" : `${name}/`))
const ios = /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.userAgent.includes("Macintosh") && navigator.maxTouchPoints > 1)

/** @param {string} check @param {boolean} worked @param {string} [note] */
async function told(check, worked, note) {
  const os = ios ? "ios" : navigator.userAgent.includes("Mac OS X") ? "macos" : undefined
  const edits = Object.keys(seen).filter((kind) => kind.startsWith("edit:")).map((kind) => kind.slice(5)).sort().join(",")
  const read = Object.keys(seen).some((kind) => kind.startsWith("screen reader:"))
  // A metric, 1 where it worked and 0 where it didn't, which Telemetry's `metric` reads by its attributes; and where it
  // didn't, its note and what the frame saw, as an error, which its `errors` read.
  const attributes = { "one.check": check, "user_agent.name": browser, "os.type": os, "one.check.edits": edits, "one.check.screen_reader": read, "service.name": "web" }
  const records = [{ name: "one.client.web.check", value: worked ? 1 : 0, attributes, time: Date.now() }]
  if (!worked) records.push({ name: "one.client.web.check", error: `${check} didn't work on ${browser} (${os}): ${note ?? "said so"}; seen ${JSON.stringify(seen)}`, attributes: { ...attributes, "user_agent.original": navigator.userAgent }, time: Date.now() })
  const sent = await fetch("/mcp", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", method: "notifications/one/measured", params: { records } }) }).catch(() => undefined)
  said.textContent = sent?.ok ? `Sent “${check}”: ${worked ? "it worked" : "it didn't"}.` : sent?.status === 401 ? "Sign in to your network in this browser first, then come back." : "It couldn't be sent: is this device online?"
}

addEventListener("message", ({ source, data }) => {
  if (source !== frame.contentWindow || !data || typeof data !== "object") return
  const reply = (/** @type {object} */ body) => frame.contentWindow?.postMessage({ jsonrpc: "2.0", id: data.id, ...body }, "*")
  if (data.method === "notifications/one/seen") Object.assign(seen, data.params)
  else if (data.method === "notifications/one/checked") told(data.params.check, !!data.params.worked, data.params.note)
  else if (data.method === "ui/initialize") reply({ result: { hostContext: { theme: matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light", deviceCapabilities: { touch: matchMedia("(pointer: coarse)").matches, hover: matchMedia("(hover: hover)").matches } } } })
  else if (data.id !== undefined) reply({ error: { code: -32601, message: `${data.method} isn't answered here` } })
})
