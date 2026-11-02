// `one try client web`'s hands (src/try_install.rs): in the try install's browser, over the DevTools protocol, the web
// client is opened at the try install's address and signed in, as a person would, pressing Sign In, the try install's
// sign-in going straight on; then a picture of it. Bun runs it: <devtools port> <address> <picture>.
const [port, address, picture] = process.argv.slice(2);
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = pages.find((p) => p.type === "page" && p.url.startsWith(address)) || pages.find((p) => p.type === "page");
if (!page) throw new Error("the browser has no page open");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
let id = 0;
const waiting = new Map();
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  waiting.get(m.id)?.(m);
  waiting.delete(m.id);
};
const send = (method, params = {}) => new Promise((resolve) => { waiting.set(++id, resolve); ws.send(JSON.stringify({ id, method, params })); });
const evaluate = async (expression) => (await send("Runtime.evaluate", { expression, returnByValue: true })).result?.result?.value;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 860, deviceScaleFactor: 1, mobile: false });
if (!(await evaluate("location.href")).startsWith(address)) await send("Page.navigate", { url: address });
const until = Date.now() + 60_000;
for (;;) {
  if (Date.now() > until) throw new Error(`still at ${await evaluate("location.href")}`);
  await sleep(500);
  // Signed in once the web client keeps its tokens (web.js's KEPT).
  const at = await evaluate(`({ href: location.href, signIn: !!document.querySelector("[data-signin]"), kept: !!localStorage.getItem("one.signin") })`);
  if (!at) continue;
  if (at.signIn) await evaluate(`document.querySelector("[data-signin]").click()`);
  else if (at.kept && at.href.startsWith(address) && new URL(at.href).pathname !== "/callback") break;
}
// What signing in shows, drawn.
await sleep(2500);
const shot = await send("Page.captureScreenshot", { format: "png" });
await Bun.write(picture, Buffer.from(shot.result.data, "base64"));
ws.close();
