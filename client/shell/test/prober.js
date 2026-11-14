// Prober's app: tries each way an app might reach past its own frame, and tells its plugin what came of each.
// `reached` is whether it got anything it shouldn't have.

const found = {}
async function attempt(tried, reach) {
  try {
    const got = await reach()
    found[tried] = got === undefined ? { reached: false, said: "nothing there" } : { reached: true, said: String(got).slice(0, 80) }
  } catch (error) {
    found[tried] = { reached: false, said: `${error.name}: ${error.message}`.slice(0, 120) }
  }
}
// A request that an answer came back to reached what it asked.
const fetched = async (address, options) => `answered ${(await fetch(address, options)).status}`
// A script from somewhere, which says whether it loaded.
const loaded = (address) => new Promise((load, refuse) => {
  document.head.append(Object.assign(document.createElement("script"), { src: address, onload: () => load("loaded"), onerror: () => refuse(new Error("didn't load")) }))
})

// The shell's page.
await attempt("the shell's page", () => parent.document.title)
await attempt("the shell's Tauri object", () => parent.__TAURI__ && "it's there")
await attempt("the shell's storage", () => parent.localStorage.length)
// Sending the shell's window elsewhere: if that worked, nothing would be left to pass on what this found.
try { top.location.href = "about:blank" } catch {}
await attempt("opening a window of its own", () => open("about:blank") ? "opened" : undefined)

// The app's Rust half, which the shell's page asks through Tauri's IPC.
await attempt("Tauri's object in this frame", () => window.__TAURI__ && "it's there")
await attempt("Tauri's IPC from this frame", () => window.__TAURI_INTERNALS__?.invoke("ask", { view: "home", method: "tools/list", params: {} }).then(() => "answered"))
await attempt("Tauri's IPC over its own protocol", () => fetched("ipc://localhost/ask", { method: "POST", body: "{}" }))
await attempt("Tauri's IPC as a web address", () => fetched("http://ipc.localhost/ask", { method: "POST", body: "{}" }))
await attempt("the shell's own files", () => fetched("tauri://localhost/home.js"))
await attempt("the internet", () => fetched("https://example.com/"))

// The other app. Its files are the network's to read, as every plugin's are. What it holds while it runs is its own.
const others = Array.from({ length: parent.frames.length }, (_, at) => parent.frames[at]).filter((frame) => frame !== window)
if (!others.length) found["this proof"] = { reached: true, said: "no other app was open beside this one, so reaching one wasn't tried" }
await attempt("another app's page", () => others.map((frame) => frame.document.body.textContent).join())
await attempt("another app's storage", () => others.map((frame) => frame.localStorage.getItem("secret")).join())
await attempt("another app's storage, from here", () => localStorage.getItem("secret") ?? undefined)
await attempt("another app's address, asked from here", () => fetched(`${location.protocol}//${location.host.replace("a-prober", "a-kept")}/a-kept/app.html`))
await attempt("another app's address, as a script here", () => loaded(`${location.protocol}//${location.host.replace("a-prober", "a-kept")}/a-kept/app.html`))

// Its own tool, through the bridge, which is what it does reach, once the shell has had time to be sent elsewhere.
await new Promise((later) => setTimeout(later, 500))
found["sending the shell's window elsewhere"] = { reached: false, said: "the shell's page is still here to pass this on" }
parent.postMessage({ jsonrpc: "2.0", id: 1, method: "tools/call", params: { name: "a-prober_found", arguments: found } }, "*")
