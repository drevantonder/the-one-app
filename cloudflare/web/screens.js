// Every screen showing: the one beside the sidebar, and the sheets over it, each an app's or the client's own, in a
// sandboxed frame whose bridge (src/screen/bridge.js) this page answers: an app's acting only as that app, and only the
// client's own as the user (settings.js).
import { offline } from "./offline.js";
import { platform } from "./platform.js";
import { UNANSWERED, api, call, developer } from "./request.js";
import { back, go, listed, own, route } from "./routes.js";
import { appearance, ownAnswers, setAppearance } from "./settings.js";
import { KEPT_SCREENS, here, joined, reads, settingAt, settingsItems, titleOf } from "./store.js";
import { $, h, ui } from "./view.js";

let frames = [];

/// What's beside the sidebar, which this page keeps (the screen's frame, and Developer's panel under it) while the
/// window draws around it.
export const detail = Object.assign(document.createElement("div"), { className: "detail" });

/// What the screen beside the sidebar says it's called, to the window's bar (`whenTitled`).
const titled = [];
export const whenTitled = (fn) => titled.push(fn);

/// This release's files by their content (`/web.js?v=…`), as the gateway's index.html names them: kept for good.
const files = Object.fromEntries((document.querySelector('meta[name="one-files"]')?.content || "").split(" ").filter(Boolean).map((f) => f.split("=")));
const asset = (name) => (files[name] ? `/${name}?v=${files[name]}` : `/${name}`);

/// Phosphor's icons, from the address once (and then the browser's cache), for those the bridge asks for: on this
/// page, and in screens' frames. Asked for last, so that on a first visit the client's WebAssembly, which the first
/// answer from the user's machines waits for, gets the connection first.
let icons;
export async function iconPaths(names = []) {
  icons ||= fetch(asset("icons.json"), { priority: "low" }).then((r) => (r.ok ? r.json() : Promise.reject(new Error("No icons.")))).catch((err) => ((icons = null), Promise.reject(err)));
  const all = await icons;
  return Object.fromEntries(names.map((n) => [n, all[n]]));
}

/// Whether the window's own screen (not a sheet's) is showing.
export const hasScreen = () => frames.some((f) => !f.sheet && f.into.isConnected);
/// Says `m` to every screen showing.
export const tell = (m) => {
  for (const f of frames) f.frame.contentWindow?.postMessage(m, "*");
};

/// What this device keeps of the screens it showed, until it signs out: each route's page as the node last gave it
/// (`page`), and its still (`still`), the page as it last looked here without its scripts, which bridge.js sends.
const kept = {
  // An https URL of its own, which the cache takes on any page's origin (the app's tauri:// one too, where it
  // takes only http and https).
  url: (kind, key) => `https://kept.invalid/${kind}/${encodeURIComponent(key)}`,
  async get(kind, key) {
    try {
      return (await (await (await caches.open(KEPT_SCREENS)).match(kept.url(kind, key)))?.text()) ?? null;
    } catch {
      return null;
    }
  },
  put(kind, key, html) {
    if (typeof html !== "string" || html.length > 2_000_000) return;
    const at = { "x-kept-at": String(Date.now()) };
    window.caches?.open(KEPT_SCREENS).then((c) => c.put(kept.url(kind, key), new Response(html, { headers: at }))).catch(() => {});
  },
  async when(kind, key) {
    try {
      return Number((await (await caches.open(KEPT_SCREENS)).match(kept.url(kind, key)))?.headers.get("x-kept-at")) || null;
    } catch {
      return null;
    }
  },
};
/// Two pages the same but for their nonce.
const same = (a, b) => a?.replace(/nonce(-|=")[0-9a-f]{32}/g, "") === b?.replace(/nonce(-|=")[0-9a-f]{32}/g, "");

/// A route's page, asked for once (the one the window opens on as it starts, beside the sidebar's): an app's from
/// the node, and kept; the client's own from this client's files (`ownPage`).
let asked = null;
function screenPage(r) {
  const key = JSON.stringify(r);
  if (asked?.key === key) {
    const { page } = asked;
    asked = null;
    return page;
  }
  // Asked now, whether or not Developer mode is on yet: only the screen the window opened on counts from its start.
  const since = developer.since();
  if (r.kind === "own") return ownPage(r);
  const path = `/v1/screens/${encodeURIComponent(r.app)}/${encodeURIComponent(r.screen)}`;
  return joined.then(() => api(path, { query: { params: r.params || "{}", scope: r.scope, linked: true }, raw: true, since: developer.on() ? since : undefined }).then((resp) => resp.text()).then((html) => (kept.put("page", key, html), html)));
}

/// One of the client's own screens' pages, which act as the user, so never a node's: the address's
/// `screens/<name>.html` (src/screen.rs's `client_template`), with a nonce of its own and its params filled in.
async function ownPage(r) {
  const resp = await fetch(asset(`screens/${files[`screens/${r.name}.html`] ? r.name : "missing"}.html`));
  if (!resp.ok) throw new Error("This didn’t load. Try again.");
  const nonce = Array.from(crypto.getRandomValues(new Uint8Array(16)), (x) => x.toString(16).padStart(2, "0")).join("");
  const params = JSON.stringify(JSON.parse(r.params || "{}")).replaceAll("</", "<\\/");
  return (await resp.text()).replaceAll("__nonce__", nonce).replace('"__params__"', () => params);
}
/// The page of the screen the window opens on, asked for as it starts.
export const ask = (r) => (asked = { key: JSON.stringify(r), page: screenPage(r) }).page.catch(() => {});

/// A page that links @one/ui and the bridge (`linked`, src/screen.rs: an app's as its node gives it, and the client's
/// own), with them written in from this window's cache, where its own page has them: a sandboxed frame's cache is its
/// alone, so it would download them again for every screen. Each that can't be had here stays linked. Its import map
/// is this client's own too, not the one the node wrote in, which says what @one/ui exported when that node was built.
const LINKED = /<link rel="stylesheet" href="(\/screen\/[a-z/]+\.css\?v=[0-9a-f]+)">|<script( nonce="[0-9a-f]+")? src="(\/screen\/[a-z/]+\.js\?v=[0-9a-f]+)"><\/script>/g;
const IMPORT_MAP = /<script( nonce="[0-9a-f]+")? type="importmap">.*?<\/script>/s;
const linkedFiles = {};
const linkedFile = (url) =>
  (linkedFiles[url] ||= fetch(url).then((r) => (r.ok ? r.text() : Promise.reject(new Error(UNANSWERED)))).catch((err) => (delete linkedFiles[url], Promise.reject(err))));
async function linkedIn(page) {
  const urls = [...new Set(Array.from(page.matchAll(LINKED), (m) => m[1] || m[3]))];
  if (!urls.length) return page;
  const map = asset("screen/ui/importmap.json");
  const texts = Object.fromEntries(await Promise.all([...urls, map].map((url) => linkedFile(url).then((text) => [url, text], () => [url, null]))));
  return page
    .replace(LINKED, (whole, css, nonce = "", js) => (texts[css || js] == null ? whole : css ? `<style>${texts[css]}</style>` : `<script${nonce}>${texts[js]}</script>`))
    .replace(IMPORT_MAP, (whole, nonce = "") => (texts[map] == null ? whole : `<script${nonce} type="importmap">${texts[map]}</script>`));
}

/// A screen that couldn't open: the problem, where it would be.
const failed = (into, text) => OneUI["react-dom/client"].createRoot(into).render(h(ui.Unavailable, { title: "Can’t Open This", text }));

/// A sandboxed frame for a page this window writes in (frame.html takes it).
function sandboxed(r) {
  const frame = document.createElement("iframe");
  frame.className = "screen-frame";
  frame.setAttribute("sandbox", "allow-scripts allow-forms");
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.src = asset("frame.html");
  frame.setAttribute("aria-label", titleOf(r));
  return frame;
}

/// What a route shows, in a new frame: its page as kept here if there is one, else as the node gives it now; one
/// that's changed since (its app deployed) starts the screen again. → the frame's record. Beside the sidebar, its
/// still shows until the screen has drawn and had its calls answered.
function frameFor(r, into, { sheet } = {}) {
  const frame = sandboxed(r);
  const f = { frame, route: r, sheet, into, key: JSON.stringify(r), busy: 0, quiet: 0 };
  frames.push(f);
  // A spinner only for a wait that's long enough to notice.
  into.innerHTML = "";
  setTimeout(() => f.loaded || f.still || !into.isConnected || $(".screen-status", into) || into.insertAdjacentHTML("afterbegin", `<div class="screen-status"><span class="spinner"></span></div>`), 400);
  const written = (page) => linkedIn(page).catch(() => page).then((html) => oneHost.look(html, appearance(), !!sheet, platform.look()));
  const draw = async (page) => {
    f.page = page;
    const html = await written(page);
    if (f.page !== page || f.html) return;
    f.html = html;
    if (!into.isConnected) return;
    if (f.still) frame.style.visibility = "hidden";
    into.append(frame);
    frame.addEventListener("load", () => {
      f.loaded = true;
      $(".screen-status", into)?.remove();
      // A screen that asks nothing shows in place of its still soon after it loads.
      if (f.still) setTimeout(() => watch(f), 1500);
    }, { once: true });
  };
  const fresh = screenPage(r);
  // The client's own aren't kept: one kept before they came from its files came from a node.
  const looking = developer.on() && r.kind !== "own" && { what: "page", app: r.app, screen: r.screen, source: "kept", at: Date.now(), started: performance.now() };
  const tried = Promise.all([r.kind === "own" ? null : kept.get("page", f.key), sheet ? null : kept.get("still", f.key)]).then(([page, still]) => {
    if (looking && page) developer.keep(looking);
    if (f.page) return;
    if (still) showStill(f, still);
    if (page) draw(page);
  });
  /// The page as its node gives it now: drawn, or, if it's changed since the kept one, the screen started again.
  const update = async (page) => {
    if (!f.html) return draw(page);
    if (same(page, f.page)) return;
    f.page = page;
    const html = await written(page);
    if (f.page !== page) return;
    f.html = html;
    frame.src = asset("frame.html");
  };
  // A kept page whose fresh one is slow to come (its devices gone quiet, not refusing) is the window's being offline too.
  let came = false;
  const away = async () => offline.add(f.key, async () => update(await screenPage(r)), await kept.when("page", f.key));
  setTimeout(() => came || tried.then(() => !came && f.page && f.into.isConnected && away()), 5000);
  fresh.then((page) => ((came = true), offline.done(f.key), update(page)), async (err) => {
    await tried;
    if (f.page) return away();
    f.loaded = true;
    failed(into, one.plain(err).title);
  });
  return f;
}

/// A screen's still, in a frame of its own as sandboxed as the screen's, answering nothing but its frame's ask for it.
function showStill(f, html) {
  if (!f.into.isConnected) return;
  const frame = sandboxed(f.route);
  frame.classList.add("screen-still");
  frame.setAttribute("aria-hidden", "true");
  const scrolled = `<script>addEventListener("load", () => scrollTo(0, Number(document.documentElement.dataset.stillY) || 0))</script>`;
  f.still = { frame, html: oneHost.look(html.replace("</head>", `${scrolled}</head>`), appearance(), false, platform.look()) };
  $(".screen-status", f.into)?.remove();
  f.into.prepend(frame);
}

/// The screen showing in place of its still, once it's had its calls answered (and nothing more asked for a moment),
/// or after a few seconds whatever it's waiting on.
function watch(f, until = Date.now() + 4000) {
  if (!f.still || f.watched) return;
  f.watched = true;
  const check = () => {
    if ((f.busy || Date.now() - f.quiet < 50) && Date.now() < until) return void setTimeout(check, 20);
    f.frame.style.visibility = "";
    f.still.frame.remove();
    f.still = null;
  };
  check();
}

export const screen = {
  show(detail, r) {
    frames = frames.filter((f) => f.sheet);
    const title = titleOf(r);
    document.title = title;
    detail.innerHTML = `<div class="screen"></div>`;
    devtools.since = Date.now();
    devtools.draw();
    // A Settings item that's a page of this site (Passkeys) is that page, framed: it's the gateway's, not a node's.
    const item = r.kind === "own" && settingsItems().find((i) => i.name === r.name && i.url && here(i.url));
    if (item) {
      const frame = Object.assign(document.createElement("iframe"), { className: "screen-frame", src: item.url });
      frame.setAttribute("aria-label", title);
      frame.setAttribute("allow", "publickey-credentials-get; publickey-credentials-create");
      return void $(".screen", detail).append(frame);
    }
    frameFor(r, $(".screen", detail));
  },
};

/// Developer, in Developer mode: the bar's </> opens its panel (src/screen/settings/devtools.jsx) for the screen in front,
/// docked under it, where it stays open from screen to screen until closed. It's told the screen's app and title, and
/// for the client's own, since when it showed.
export const devtools = {
  KEY: "one.devtools",
  HEIGHT: "one.devtools.height",
  since: Date.now(),
  open: () => localStorage.getItem(devtools.KEY) === "1",
  route() {
    const r = route();
    return own("devtools", r.kind === "screen" ? { app: r.app, screen: r.screen, title: titleOf(r) } : { title: titleOf(r), since: devtools.since });
  },
  toggle() {
    if (devtools.open()) localStorage.removeItem(devtools.KEY);
    else localStorage.setItem(devtools.KEY, "1");
    devtools.draw();
  },
  /// Its top edge drags it taller or shorter, kept for next time. The screens' frames would take the pointer as it
  /// moves over them, so they let it pass while it's dragged.
  resizable(dock) {
    const edge = Object.assign(document.createElement("div"), { className: "devtools-edge" });
    edge.setAttribute("role", "separator");
    edge.setAttribute("aria-orientation", "horizontal");
    dock.append(edge);
    edge.addEventListener("pointerdown", (e) => {
      e.preventDefault();
      edge.setPointerCapture(e.pointerId);
      const start = e.clientY, from = dock.offsetHeight, most = dock.parentElement.offsetHeight - 120;
      document.body.classList.add("dragging");
      const move = (m) => (dock.style.height = `${Math.round(Math.min(most, Math.max(120, from + start - m.clientY)))}px`);
      const up = () => {
        edge.removeEventListener("pointermove", move);
        document.body.classList.remove("dragging");
        localStorage.setItem(devtools.HEIGHT, String(dock.offsetHeight));
      };
      edge.addEventListener("pointermove", move);
      edge.addEventListener("pointerup", up, { once: true });
      edge.addEventListener("pointercancel", up, { once: true });
    });
  },
  draw() {
    let dock = $(".devtools", detail);
    if (!developer.on() || !devtools.open()) {
      if (dock) (frames = frames.filter((f) => f !== dock.f)), dock.remove();
      return;
    }
    const r = devtools.route(), key = JSON.stringify(r);
    if (dock?.key === key) return;
    if (!dock) {
      detail.append((dock = Object.assign(document.createElement("div"), { className: "devtools" })));
      const height = Number(localStorage.getItem(devtools.HEIGHT));
      if (height) dock.style.height = `${height}px`;
    }
    frames = frames.filter((f) => f !== dock.f);
    dock.key = key;
    dock.f = frameFor(r, dock);
    dock.f.dock = true;
    devtools.resizable(dock);
  },
};

/// Whether something is over the window that Escape closes: an alert or a menu.
export const overWindow = () => !!$("[role=alertdialog], [role=menu]");

/// A screen over the window, in a sheet: → what it gave `one.close` once it's gone, else null.
export function openSheet(r) {
  return new Promise((done) => {
    let value = null;
    const sheet = ui.showSheet({ title: titleOf(r), done: r.name === "request" ? "Cancel" : "Done" });
    const f = frameFor(r, sheet.body, { sheet: { close: (v) => ((value = v ?? null), sheet.close()), title: sheet.setTitle } });
    sheet.closed.then(() => ((frames = frames.filter((x) => x !== f)), done(value)));
  });
}

/// The bridge's messages (src/screen/bridge.js), from the screens' frames only, each acting as its screen's app; and
/// the client's own screens', as the user, which no app's may.
addEventListener("message", async (e) => {
  const still = frames.find((x) => x.still?.frame.contentWindow === e.source)?.still;
  if (still && e.data?.oneFrame) return void still.frame.contentWindow.postMessage({ oneScreen: still.html }, "*");
  const f = frames.find((x) => x.frame.contentWindow === e.source);
  if (!f) return;
  const r = f.route, mine = r.kind === "own";
  if (e.data?.oneFrame) return void f.frame.contentWindow.postMessage({ oneScreen: f.html }, "*");
  // The panel's window fits its screen.
  if (typeof e.data?.oneHeight === "number") return void (f.sheet || platform.fit(e.data.oneHeight));
  if (e.data?.oneStill !== undefined) return void (f.sheet || kept.put("still", f.key, e.data.oneStill));
  const { one: m, id } = e.data || {};
  if (!m || typeof m !== "object") return;
  // What a call answered last time, if it only reads: from this device, at once, while the window still starts.
  if (m.type === "kept") {
    const got = mine ? null : await platform.kept(r.app, String(m.tool || ""), m.input ?? {}).catch(() => null);
    return void f.frame.contentWindow?.postMessage({ oneReply: id, ok: got }, "*");
  }
  f.busy++;
  watch(f);
  await joined;
  const answered = () => ((f.busy = Math.max(0, f.busy - 1)), (f.quiet = Date.now()));
  const reply = (ok) => (answered(), f.frame.contentWindow?.postMessage({ oneReply: id, ok: ok ?? null }, "*"));
  const fail = (err) => (answered(), f.frame.contentWindow?.postMessage({ oneReply: id, error: String(err?.message || err), code: err?.code }, "*"));
  try {
    reply(await oneHost.answer(m, {
      frame: f.frame,
      call: async (tool, input) => {
        if (mine) throw new Error("Settings has no app to call.");
        return call(r.app, tool, input, reads(r.app, tool));
      },
      open: (m) => {
        const params = JSON.stringify(m.params || {});
        // Any screen may send its user to choose the device that runs its app's schedules, in Settings › Devices.
        // An app's says only which app's schedules, and where: no code to approve.
        const to = m.app === "settings" && m.screen === "devices" ? { kind: "own", name: "devices", params: mine ? params : JSON.stringify({ schedules: m.params?.schedules, scope: m.params?.scope }) }
          : typeof m.app === "string" && m.app ? { kind: "screen", app: m.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope }
          : mine ? { kind: "own", name: String(m.screen || ""), params }
          : { kind: "screen", app: r.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope };
        if (platform.panel) return void platform.main(to);
        if (m.sheet) return openSheet(to);
        // From a sheet, the sheet goes and it opens where the window is. What the sidebar lists (an app on a page,
        // Backups in Settings) opens as if picked there, with no Back; only what's deeper than it is pushed.
        f.sheet?.close(null);
        go(to, { pushed: !listed(to) });
        return null;
      },
      close: (value) => {
        if (f.dock) return (localStorage.removeItem(devtools.KEY), devtools.draw(), null);
        if (f.sheet) f.sheet.close(value);
        else if (history.state?.back) back();
        return null;
      },
      title: (title, m) => {
        // Empty while the screen shows its own large title; the tab is named by what the screen says it's called.
        // A sheet too says nothing while the screen's large title shows (`one.largeTitle`, which names it).
        if (f.sheet) f.sheet.title(title || (m.name ? "" : titleOf(r)));
        // Developer's panel has the bar of the screen it's under.
        else if (!f.dock && f.into.isConnected) {
          titled.forEach((fn) => fn(title));
          document.title = title || String(m.name || "") || titleOf(r);
        }
        return null;
      },
      vault: async (need) => {
        if (mine) throw new Error("Settings has no app to ask for.");
        const out = await api("/v1/vault/request", { method: "POST", body: { scope: r.scope || "user", app: r.app, need } });
        if (!out?.id) throw new Error("The Vault didn’t take the request");
        return (await openSheet(own("request", { scope: out.scope, id: out.id }))) === true;
      },
      link: (to) => {
        const url = new URL(to, location.href);
        const item = settingAt(url.href);
        if (item) go(own(item.name));
        else if (["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) platform.open(url.href);
        return null;
      },
      icons: iconPaths,
      blob: async (id) => (await api(`/v1/blobs/${encodeURIComponent(id)}`, { raw: true })).blob(),
      ...platform.bridge(f.frame),
      traces: () => (developer.on() ? developer.kept : null),
      own: mine && {
        ...ownAnswers,
        appearance: (m) => (m.value != null && tell({ oneScheme: setAppearance(String(m.value)) }), appearance()),
        request: async (m) => (await openSheet(own("request", { scope: String(m.scope || ""), id: String(m.id || "") }))) === true,
      },
    }));
  } catch (err) {
    fail(err);
  }
});
