// `one try client web`'s hands (src/try_install.rs), in one of the try install's browsers, over the DevTools protocol.
// Bun runs it:
//   sign-in <devtools port> <address> <picture> [invite link]: the web client is opened at the try install's address
//     and signed in, as a person would, pressing Sign In, the try install's sign-in going straight on; then a picture
//     of it. With an invite link, its person first opens it and joins, pictured as <picture>-invite and -joined.
//   invite <devtools port> <address> <name> <picture>: as the person signed in to the web client there, an invite link
//     for someone new, made with Add Someone in Settings › Users, pictured; printed.
const [what, port, address, ...rest] = process.argv.slice(2);
const pages = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
const page = pages.find((p) => p.type === "page" && p.url.startsWith(address)) || pages.find((p) => p.type === "page");
if (!page) throw new Error("the browser has no page open");
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((resolve, reject) => { ws.onopen = resolve; ws.onerror = reject; });
let id = 0;
const waiting = new Map();
// The sessions of the page's frames, each a screen of its own (in a process of its own), once attached.
const frames = new Set();
ws.onmessage = (e) => {
  const m = JSON.parse(e.data);
  if (m.method === "Target.attachedToTarget" && m.params.targetInfo.type === "iframe") frames.add(m.params.sessionId);
  if (m.method === "Target.detachedFromTarget") frames.delete(m.params.sessionId);
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

if (what === "invite") {
  const [name, picture] = rest;
  await send("Target.setAutoAttach", { autoAttach: true, waitForDebuggerOnStart: false, flatten: true });
  await send("Page.navigate", { url: `${address}/settings/users` });
  // Settings › Users, in its frame: Add Someone…, their name, Make Link.
  let users;
  for (const by = Date.now() + 60_000; !users; await sleep(250)) {
    if (Date.now() > by) throw new Error("Settings › Users didn't show Add Someone");
    for (const frame of frames) if (await evaluate(`!!document.querySelector("[data-add]")`, frame)) users = frame;
  }
  await evaluate(`document.querySelector("[data-add]").click()`, users);
  await sleep(500);
  await evaluate(`(() => { const name = document.querySelector("#name"); name.value = ${JSON.stringify(name)}; name.dispatchEvent(new Event("input")); document.querySelector("#go").click(); })()`, users);
  let link;
  for (const by = Date.now() + 30_000; !link; await sleep(250)) {
    if (Date.now() > by) throw new Error(`Add Someone made no link: ${await evaluate(`document.querySelector("#note")?.textContent`, users)}`);
    link = await evaluate(`document.querySelector(".made .link span")?.textContent`, users);
  }
  await sleep(300);
  await shoot(picture);
  await evaluate(`document.querySelector("[data-done]")?.click()`, users);
  console.log(link);
} else {
  const [picture, invite] = rest;
  const named = (suffix) => picture.replace(/\.png$/, `-${suffix}.png`);
  if (invite) {
    await send("Page.navigate", { url: invite });
    await until(`!!document.querySelector("#passkey:not([disabled])")`, "the invite page didn't open");
    await sleep(300);
    await shoot(named("invite"));
    await evaluate(`document.querySelector("#passkey").click()`);
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
    const at = await evaluate(`({ href: location.href, signIn: !!document.querySelector("[data-signin]"), kept: !!localStorage.getItem("one.signin") })`);
    if (!at) continue;
    if (at.signIn) await evaluate(`document.querySelector("[data-signin]").click()`);
    else if (at.kept && at.href.startsWith(address) && new URL(at.href).pathname !== "/callback") break;
  }
  // What signing in shows, drawn.
  await sleep(2500);
  await shoot(picture);
}
ws.close();
