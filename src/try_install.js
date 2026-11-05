// `one try client web`'s hands (src/try_install.rs), in one of the try install's browsers, over the DevTools protocol.
// Bun runs it:
//   sign-in <devtools port> <address> <picture> [invite link]: the web client is opened at the try install's address
//     and signed in, as a person would, pressing Sign In, the try install's sign-in going straight on. With an invite
//     link, its person first opens it and joins, pictured as <picture>-invite and -joined. Prints the code it shows to
//     be approved (pictured as <picture>-approve), or nothing if it's a device of its person's already.
//   picture <devtools port> <address> <picture>: once the web client's window is drawn, a picture of it.
const [command, port, address, picture, invite] = process.argv.slice(2);
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
const send = (method, params = {}, sessionId) => new Promise((resolve) => { waiting.set(++id, resolve); ws.send(JSON.stringify({ id, method, params, sessionId })); });
const evaluate = async (expression, session) => (await send("Runtime.evaluate", { expression, returnByValue: true, awaitPromise: true }, session)).result?.result?.value;
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
const shoot = async (file) => {
  const shot = await send("Page.captureScreenshot", { format: "png" });
  await Bun.write(file, Buffer.from(shot.result.data, "base64"));
};
/// Waits for `test` (an expression) to be true on the page, for up to a minute.
const until = async (test, doing) => {
  const by = Date.now() + 60_000;
  while (!(await evaluate(test))) {
    if (Date.now() > by) throw new Error(`${doing}: still at ${await evaluate("location.href")}`);
    await sleep(250);
  }
};

await send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 860, deviceScaleFactor: 1, mobile: false });

const named = (suffix) => picture.replace(/\.png$/, `-${suffix}.png`);
if (command === "picture") {
  await until(`!!document.querySelector("[data-slot=shell-screen]")`, "the web client wasn't let in");
  await sleep(2500);
  await shoot(picture);
  ws.close();
  process.exit(0);
}
if (invite) {
  await send("Page.navigate", { url: invite });
  await until(`!!document.querySelector("#go:not([disabled])")`, "the invite page didn't open");
  await sleep(300);
  await shoot(named("invite"));
  await evaluate(`document.querySelector("#go").click()`);
  await until(`!document.querySelector("#done").hidden`, "joining didn't work");
  await shoot(named("joined"));
  await evaluate(`document.querySelector("#done a.button").click()`);
  await sleep(500);
}
if (!(await evaluate("location.href")).startsWith(address)) await send("Page.navigate", { url: address });
const by = Date.now() + 60_000;
for (;;) {
  if (Date.now() > by) throw new Error(`still at ${await evaluate("location.href")}`);
  await sleep(500);
  // Signed in once the web client keeps its tokens (web.js's KEPT).
  const at = await evaluate(`({ href: location.href, signIn: !!document.querySelector("[data-slot=welcome-action]"), kept: !!localStorage.getItem("one.signin") })`);
  if (!at) continue;
  if (at.signIn) await evaluate(`document.querySelector("[data-slot=welcome-action]").click()`);
  else if (at.kept && at.href.startsWith(address) && new URL(at.href).pathname !== "/callback") break;
}
// Its code to be approved, once it shows one; nothing once its window is drawn.
await until(`!!document.querySelector("[data-slot=join-code], [data-slot=shell-screen]")`, "the web client didn't start");
await sleep(500);
const code = await evaluate(`document.querySelector("[data-slot=join-code]")?.textContent.replace(" ", "") || ""`);
if (code) await shoot(named("approve"));
console.log(code);
ws.close();
