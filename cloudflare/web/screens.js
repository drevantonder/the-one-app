// Every screen showing: the one beside the sidebar, and the sheets over it, each an app's or the client's own, in a
// sandboxed frame whose bridge (src/screen/bridge.js) this page answers: an app's acting only as that app, and only the
// client's own as the user (settings.js).
import { move, settle } from "./moves.js";
import { offline } from "./offline.js";
import { platform } from "./platform.js";
import { CODES } from "../shared/protocol.js";
import { UNANSWERED, api, call, developer } from "./request.js";
import { back, go, home, listed, own, route } from "./routes.js";
import { appearance, ownAnswers, setAppearance } from "./settings.js";
import { KEPT_SCREENS, app, config, here, joined, keeping, reads, settingAt, settingsItems, titleOf } from "./store.js";
import { $, h, reveal, ui } from "./view.js";

let frames = [];

/// What's beside the sidebar, which this page keeps (the screen's frame, and Developer's panel under it) while the
/// window draws around it.
export const detail = Object.assign(document.createElement("div"), { className: "detail" });

/// What the screen beside the sidebar says it's called, to the window's bar (`whenTitled`).
const titled = [];
export const whenTitled = (fn) => titled.push(fn);

/// The web client's files by their content (`/web.js?v=…`), as the gateway's index.html names them: kept for good.
const files = Object.fromEntries((document.querySelector('meta[name="one-files"]')?.content || "").split(" ").filter(Boolean).map((f) => f.split("=")));
const asset = (name) => (files[name] ? `/${name}?v=${files[name]}` : `/${name}`);

/// Phosphor's icons, each asked of the platform once (and then the browser's cache), for those the bridge asks for: on
/// this page, and in screens' frames.
const icons = {};
export async function iconPaths(names = []) {
  const unasked = names.filter((n) => !icons[n]);
  if (unasked.length) {
    const got = platform.icons(unasked, files["icons.json"]);
    for (const n of unasked) icons[n] = got.then((all) => all[n], (err) => (delete icons[n], Promise.reject(err)));
  }
  return Object.fromEntries(await Promise.all(names.map(async (n) => [n, await icons[n]])));
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
  url: (kind, key) => `https://kept.invalid/${encodeURIComponent(platform.user())}/${kind}/${encodeURIComponent(key)}`,
  /// What was kept, if it was drawn at `width` (a still is, as a page isn't; at another it would show squashed).
  async get(kind, key, width) {
    try {
      const got = await (await caches.open(KEPT_SCREENS)).match(kept.url(kind, key));
      return (width && got?.headers.get("x-kept-width") !== String(width) ? null : await got?.text()) ?? null;
    } catch {
      return null;
    }
  },
  put(kind, key, html, width) {
    if (!keeping() || typeof html !== "string" || html.length > 2_000_000) return;
    const headers = { "x-kept-at": String(Date.now()), ...(width ? { "x-kept-width": String(width) } : {}) };
    window.caches?.open(KEPT_SCREENS).then((c) => c.put(kept.url(kind, key), new Response(html, { headers }))).catch(() => {});
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
const KIT = "/screen/ui/ui.js";
const linkedFiles = {};
const linkedFile = (url) =>
  (linkedFiles[url] ||= fetch(url).then((r) => (r.ok ? r.text() : Promise.reject(new Error(UNANSWERED)))).catch((err) => (delete linkedFiles[url], Promise.reject(err))));
async function linkedIn(page, kitted) {
  const urls = [...new Set(Array.from(page.matchAll(LINKED), (m) => m[1] || m[3]))];
  if (!urls.length) return page;
  const map = asset("screen/ui/importmap.json");
  const texts = Object.fromEntries(await Promise.all([...urls, map].map((url) => linkedFile(url).then((text) => [url, text], () => [url, null]))));
  return page
    .replace(LINKED, (whole, css, nonce = "", js) => (kitted && js?.startsWith(KIT) ? "" : texts[css || js] == null ? whole : css ? `<style>${texts[css]}</style>` : `<script${nonce}>${texts[js]}</script>`))
    .replace(IMPORT_MAP, (whole, nonce = "") => (texts[map] == null ? whole : `<script${nonce} type="importmap">${texts[map]}</script>`));
}

/// A screen that couldn't open: the problem, where it would be.
const failed = (into, text) => OneUI["react-dom/client"].createRoot(into).render(h(ui.Unavailable, { title: "Can’t Open This", text }));

/// A screen of an app in Recently Deleted: one problem where it would be, with Restore for those who may, as the App
/// Store says (`_list`'s `mine`); restored, the screen opens.
async function deleted(f, r) {
  const { frame, into } = f;
  frame.remove();
  $(".screen-status", into)?.remove();
  into.innerHTML = "";
  screenDrew(f);
  const store = config().store;
  const mine = await call(store, "_list", {}, true).then((l) => l.apps?.some((a) => a.name === r.app && a.mine), () => false);
  if (!into.isConnected) return;
  const root = OneUI["react-dom/client"].createRoot(into);
  const box = (props) => root.render(h("div", { style: { padding: 24, maxWidth: 560 } }, h(ui.Problem, props)));
  const restore = async () => {
    try {
      await call(store, "restore", { app: r.app });
      root.unmount();
      frames = frames.filter((x) => x !== f);
      const again = frameFor(r, into, { sheet: f.sheet });
      if (showing === f) announced(again, () => {});
    } catch (err) {
      box({ title: "Couldn’t Restore It", text: one.plain(err).title, action: { label: "Try Again", run: restore } });
    }
  };
  box({ warn: true, title: `${titleOf(r)} Was Deleted`, ...(mine ? { action: { label: "Restore", run: restore } } : { text: "Its owner can restore it." }) });
}

/// What a frame loads, told the window's appearance so its empty page is the window's colour (frame.html).
const framePage = () => `${asset("frame.html")}#${appearance()}`;

/// A sandboxed frame for a page this window writes in (frame.html takes it), named for `r` if it's for one yet.
function sandboxed(r) {
  const frame = document.createElement("iframe");
  frame.className = "screen-frame";
  frame.setAttribute("sandbox", "allow-scripts allow-forms");
  frame.setAttribute("referrerpolicy", "no-referrer");
  frame.src = framePage();
  if (r) frame.setAttribute("aria-label", titleOf(r));
  return frame;
}

/// A frame with @one/ui already running in it and nothing yet shown, hidden in a layer of its own where the next screen
/// will go (a frame can't move without reloading), so the next screen needs only its own page written in. Made once the
/// window is quiet, and again after it's used; `spareRefused` if this browser can't keep it (frame.html).
let spare = null, spareRefused = false;
function warmSpare() {
  const box = $(".screen", detail);
  if (spare?.into.isConnected || spareRefused || platform.panel || !box) return;
  const src = $(`script[src^="${KIT}"]`)?.getAttribute("src");
  if (!src) return;
  const into = Object.assign(document.createElement("div"), { className: "screen-layer coming" });
  const frame = sandboxed();
  frame.setAttribute("aria-hidden", "true");
  into.append(frame);
  box.append(into);
  spare = { into, frame, src };
}
const whenQuiet = (f) => f.drawn.then(() => setTimeout(warmSpare, 800));
/// `r`'s frame in a layer of `box`, hidden while `from` shows: the spare's, if there's one ready.
function screenIn(box, r, from) {
  const ready = spare?.kitted && spare.into.isConnected ? spare : null;
  if (!ready) return frameFor(r, layerFor(box, r, from));
  spare = null;
  const { into, frame } = ready;
  into.className = from ? "screen-layer coming" : "screen-layer";
  if (platform.phone() && !home(r)) into.dataset.bar = "";
  frame.removeAttribute("aria-hidden");
  frame.setAttribute("aria-label", titleOf(r));
  return frameFor(r, into, { kitted: frame });
}

/// What a route shows, in a new frame: its page as kept here if there is one, else as the node gives it now; one
/// that's changed since (its app deployed) starts the screen again. → the frame's record, whose `drawn` is once its still
/// or the screen itself has drawn. Beside the sidebar, its still shows until the screen says it has drawn.
function frameFor(r, into, { sheet, kitted } = {}) {
  const frame = kitted || sandboxed(r);
  const f = { frame, route: r, sheet, into, key: JSON.stringify(r), kitted: !!kitted };
  f.drawn = new Promise((done) => (f.saysDrawn = done));
  frames.push(f);
  // A spinner only for a wait that's long enough to notice.
  if (!kitted) into.innerHTML = "";
  setTimeout(() => f.loaded || f.shown || f.still || !into.isConnected || $(".screen-status", into) || into.insertAdjacentHTML("afterbegin", `<div class="screen-status"><span class="spinner"></span></div>`), 400);
  // A screen that's nothing yet after 5 seconds: the spinner gives way to one problem, which goes as the screen loads.
  setTimeout(() => {
    if (f.loaded || f.page || f.still || !into.isConnected) return;
    $(".screen-status", into)?.remove();
    const status = Object.assign(document.createElement("div"), { className: "screen-status" });
    into.prepend(status);
    OneUI["react-dom/client"].createRoot(status).render(h(ui.Problem, { quiet: true, title: "Not opening yet", text: UNANSWERED, className: "m-6" }));
  }, 5000);
  const written = (page, kit) => linkedIn(page, kit).catch(() => page).then((html) => oneHost.look(html, appearance(), !!sheet, platform.look()));
  const draw = async (page) => {
    f.page = page;
    const html = await written(page, f.kitted);
    if (f.page !== page || f.html) return;
    f.html = html;
    if (!into.isConnected) return;
    if (f.still) frame.style.visibility = "hidden";
    if (f.kitted) frame.contentWindow.postMessage({ oneScreen: html }, "*");
    else into.append(frame);
  };
  const fresh = screenPage(r);
  // The client's own aren't kept: one kept before they came from its files came from a node.
  const looking = developer.on() && r.kind !== "own" && { what: "page", app: r.app, screen: r.screen, source: "kept", at: Date.now(), started: performance.now() };
  const tried = Promise.all([r.kind === "own" ? null : kept.get("page", f.key), sheet ? null : kept.get("still", f.key, into.clientWidth)]).then(([page, still]) => {
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
    f.kitted = false;
    const html = await written(page);
    if (f.page !== page) return;
    f.html = html;
    frame.src = framePage();
  };
  // A kept page whose fresh one is slow to come (its devices gone quiet, not refusing) is the window's being offline too.
  let came = false;
  const away = async () => offline.add(f.key, async () => update(await screenPage(r)), await kept.when("page", f.key));
  setTimeout(() => came || tried.then(() => !came && f.page && f.into.isConnected && away()), 5000);
  fresh.then((page) => ((came = true), offline.done(f.key), update(page)), async (err) => {
    await tried;
    if (err.code === CODES.appDeleted) return (came = true), offline.done(f.key), deleted(f, r);
    if (f.page) return away();
    screenDrew(f);
    failed(into, one.plain(err).title);
  });
  return f;
}

/// How far along the row of a screen's large title the window's own buttons reach, which the title starts past.
let barStart = 0;
export function setBarStart(px) {
  if (px === barStart) return;
  barStart = px;
  frames.filter((f) => !f.sheet && !f.dock).forEach((f) => f.frame.contentWindow?.postMessage({ oneBar: px }, "*"));
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
  if (html.includes("large-title")) {
    f.into.dataset.large = "";
    // Its large title says its name, so the bar doesn't say it too while this shows.
    f.said ??= "";
    if (f.begun) saying(f);
  }
  // One that never says it has drawn shows anyway, whatever it's waiting on.
  setTimeout(() => f.still?.frame === frame && screenDrew(f), 4000);
}

/// The screen has drawn: it shows in place of its still, and its wait is over.
function screenDrew(f) {
  f.loaded = true;
  $(".screen-status", f.into)?.remove();
  f.frame.style.visibility = "";
  f.still?.frame.remove();
  f.still = null;
  f.saysDrawn();
}

/// The screen showing, and the one being drawn behind it to take its place; and an app grown out of its icon on a
/// phone, for Back to shrink it into again.
let showing = null, coming = null, grown = null;
/// How long the screen showing waits for the next to draw before making way for its spinner.
const WAIT_TO_DRAW = 1000;

/// A route's tile as the window draws it, if it has one (an app's, or a Settings item's).
const tileOf = (r) => (r.kind === "screen" ? app(r.app) : settingsItems().find((i) => i.name === r.name)) || null;

/// How the screen `r` takes the place of what shows: on a phone an app opened from its icon (`opened`, where it was)
/// grows from it, and Back to where it was opened from shrinks it there; on a computer, pages cross and screens rise.
function how(r, opened) {
  const key = JSON.stringify(r);
  if (platform.phone()) {
    const tile = opened && tileOf(r);
    if (tile) {
      grown = { key, rect: opened, tile, width: innerWidth, height: innerHeight };
      return { grow: grown };
    }
    if (grown && home(r) && grown.key === showing?.key && grown.width === innerWidth && grown.height === innerHeight) {
      const shrink = grown;
      grown = null;
      return { shrink };
    }
    return { rise: true };
  }
  return home(r) && showing && home(showing.route) ? { cross: true } : { rise: true };
}

/// Whether a next screen is being drawn behind the one showing, whose bar stays till it goes.
export const replacing = () => !!coming;

/// The screen's frames as they're dropped with their layer.
function drop(layer) {
  layer.remove();
  frames = frames.filter((f) => f.into !== layer);
}

/// The pages left for a screen, a page's frame hidden where it is (moving a frame reloads it) so Back to it is instant
/// and it keeps its place, by route. On a phone they're one, a frame of the pages side by side, wherever its URL opened it.
const parked = new Map();
const PARKED = 3;
const parkKey = (r) => (platform.phone() ? "pages" : JSON.stringify(r));
/// What shows goes: a page, parked; anything else, gone.
function retire(layer) {
  const f = frames.find((x) => x.into === layer);
  if (!f || !home(f.route)) return drop(layer);
  layer.removeAttribute("style");
  layer.dataset.parked = "";
  parked.delete(parkKey(f.route));
  parked.set(parkKey(f.route), f);
  for (const old of [...parked.keys()].slice(0, -PARKED)) (drop(parked.get(old).into), parked.delete(old));
}

/// The window begins to show `f`: its bar says so (`begin`), and what it's said it's called.
function announced(f, begin) {
  showing = f;
  f.begun = true;
  begin();
  if ("said" in f) saying(f);
}
function saying(f) {
  titled.forEach((fn) => fn(f.said));
  document.title = f.said || String(f.name || "") || titleOf(f.route);
}

/// A layer for `r`'s screen in `box`, hidden while `from` still shows.
function layerFor(box, r, from) {
  const into = Object.assign(document.createElement("div"), { className: "screen-layer" });
  if (platform.phone() && !home(r)) into.dataset.bar = "";
  if (from) into.classList.add("coming");
  box.append(into);
  return into;
}

/// The screen a press has started before it lets go (`screen.ready`), for `show` to take up.
let readied = null;

export const screen = {
  /// Starts `r`'s screen behind what shows, as the press that opens it comes down. If it isn't opened, the next `show`
  /// drops it.
  ready(detail, r) {
    const key = JSON.stringify(r);
    const box = $(".screen", detail);
    if (!box || readied?.key === key || showing?.key === key || !showing?.into.isConnected || parked.has(parkKey(r))) return;
    if (r.kind === "own" && settingsItems().some((i) => i.name === r.name && i.url && here(i.url))) return;
    if (readied) drop(readied.into);
    readied = screenIn(box, r, showing.into);
  },

  /// Shows `r` where the window shows screens. What shows stays until `r`'s next has drawn; a nav away from it goes the
  /// same way. `begin` puts the window's bar on it, when it starts to show (as a phone's goes back, when it's left).
  show(detail, r, begin = () => {}, opened) {
    document.title = titleOf(r);
    devtools.since = Date.now();
    devtools.draw();
    settle();
    if (coming) (drop(coming.into), (coming = null));
    const key = JSON.stringify(r);
    const early = readied?.key === key ? readied : null;
    if (readied && readied !== early) drop(readied.into);
    readied = null;
    if (showing?.key === key && showing.into.isConnected) return void announced(showing, begin);
    const box = $(".screen", detail) || detail.insertAdjacentElement("afterbegin", Object.assign(document.createElement("div"), { className: "screen" }));
    const from = showing?.into.isConnected ? showing.into : null;
    if (!from && showing) drop(showing.into);
    for (const [key, f] of parked) if (!f.into.isConnected) (drop(f.into), parked.delete(key));
    if (from) frames.filter((f) => f.into === from).forEach((f) => (f.leaving = true));
    const left = parked.get(parkKey(r));
    if (left && from) {
      if (early) drop(early.into);
      parked.delete(parkKey(r));
      left.leaving = false;
      delete left.into.dataset.parked;
      const goes = how(r, opened);
      if (!goes.shrink) announced(left, begin);
      return move(from, left.into, goes, () => {
        retire(from);
        if (goes.shrink) announced(left, begin);
      });
    }
    if (early && !from) drop(early.into);
    // A Settings item that's a page of this site (Passkeys) is that page, framed: it's the gateway's, not a node's.
    const item = r.kind === "own" && settingsItems().find((i) => i.name === r.name && i.url && here(i.url));
    let f;
    if (early && from) f = early;
    else if (item) {
      const frame = Object.assign(document.createElement("iframe"), { className: "screen-frame", src: item.url });
      frame.setAttribute("aria-label", titleOf(r));
      frame.setAttribute("allow", "publickey-credentials-get; publickey-credentials-create");
      f = { into: layerFor(box, r, from), route: r, key: JSON.stringify(r), drawn: new Promise((done) => frame.addEventListener("load", done, { once: true })) };
      f.into.append(frame);
    } else f = screenIn(box, r, from);
    const into = f.into;
    whenQuiet(f);
    if (!from) {
      announced(f, begin);
      Promise.race([f.drawn, new Promise((done) => setTimeout(done, WAIT_TO_DRAW))]).then(() => reveal());
      return;
    }
    coming = f;
    const goes = how(r, opened);
    Promise.race([f.drawn, new Promise((done) => setTimeout(done, WAIT_TO_DRAW))]).then(() => {
      if (coming !== f) return;
      coming = null;
      if (!goes.shrink) announced(f, begin);
      move(from, into, goes, () => {
        retire(from);
        if (goes.shrink) announced(f, begin);
      });
    });
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
    // Its screen draws in a frame that fills the body, so until it says how tall it is the body is a short wait.
    sheet.fit(128);
    const f = frameFor(r, sheet.body, { sheet: { close: (v) => ((value = v ?? null), sheet.close()), title: sheet.setTitle, fit: sheet.fit } });
    sheet.closed.then(() => ((frames = frames.filter((x) => x !== f)), done(value)));
  });
}

/// A request for an item, in a sheet → whether it was allowed. Closed without an answer, it's declined; if that can't
/// reach a node the request just waits in Requests until it expires.
export async function openRequest(scope, id) {
  const answer = await openSheet(own("request", { scope, id }));
  if (answer === null) await api("/v1/vault/answer", { method: "POST", body: { scope, id, declined: true } }).catch(() => {});
  return answer === true;
}

/// The bridge's messages (src/screen/bridge.js), from the screens' frames only, each acting as its screen's app; and
/// the client's own screens', as the user, which no app's may.
addEventListener("message", async (e) => {
  if (spare && e.source === spare.frame.contentWindow) {
    if (e.data?.oneFrame) linkedFile(spare.src).then((kit) => e.source?.postMessage({ oneKit: kit }, "*")).catch(() => {});
    else if (e.data?.oneKitted === true) spare.kitted = true;
    else if (e.data?.oneKitted === false) ((spareRefused = true), spare.into.remove(), (spare = null));
    return;
  }
  const still = frames.find((x) => x.still?.frame.contentWindow === e.source)?.still;
  if (still && e.data?.oneFrame) return void still.frame.contentWindow.postMessage({ oneScreen: still.html }, "*");
  if (still && e.data?.oneWritten) return void frames.find((x) => x.still === still)?.saysDrawn();
  const f = frames.find((x) => x.frame.contentWindow === e.source);
  if (!f) return;
  const r = f.route, mine = r.kind === "own";
  if (e.data?.oneFrame) return void f.frame.contentWindow.postMessage({ oneScreen: f.html }, "*");
  // The panel's window, and a sheet, fit their screen.
  if (typeof e.data?.oneHeight === "number") return void (f.sheet ? f.sheet.fit(e.data.oneHeight) : platform.fit(e.data.oneHeight));
  if (e.data?.oneStill !== undefined) return void (f.sheet || kept.put("still", f.key, e.data.oneStill, f.frame.clientWidth));
  if (e.data?.oneShown) return void ((f.shown = true), $(".screen-status", f.into)?.remove());
  if (e.data?.oneDrawn) return void screenDrew(f);
  const { one: m, id } = e.data || {};
  if (!m || typeof m !== "object") return;
  /// Where this screen's `one.open` goes.
  const opensTo = (m) => {
    const params = JSON.stringify(m.params || {});
    // Any screen may send its user to choose the device that runs its app's schedules, in Settings › Devices.
    // An app's says only which app's schedules, and where: no code to approve.
    return m.app === "settings" && m.screen === "devices" ? { kind: "own", name: "devices", params: mine ? params : JSON.stringify({ schedules: m.params?.schedules, scope: m.params?.scope }) }
      : typeof m.app === "string" && m.app ? { kind: "screen", app: m.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope }
      : mine ? { kind: "own", name: String(m.screen || ""), params }
      : { kind: "screen", app: r.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope };
  };
  // What a call answered last time, if it only reads: from this device, at once, while the window still starts.
  if (m.type === "kept") {
    const got = mine ? null : await platform.kept(r.app, String(m.tool || ""), m.input ?? {}).catch(() => null);
    return void f.frame.contentWindow?.postMessage({ oneReply: id, ok: got }, "*");
  }
  // What's here needs no node: the icons, and what this device keeps for its own screens (`one.cache`, the home's last
  // pages), so a screen offline starts from them.
  if (m.type !== "icons" && !(mine && m.type === "cache")) await joined;
  const reply = (ok) => f.frame.contentWindow?.postMessage({ oneReply: id, ok: ok ?? null }, "*");
  const fail = (err) => f.frame.contentWindow?.postMessage({ oneReply: id, error: String(err?.message || err), code: err?.code }, "*");
  try {
    reply(await oneHost.answer(m, {
      frame: f.frame,
      call: async (tool, input) => {
        if (mine) throw new Error("Settings has no app to call.");
        return call(r.app, tool, input, reads(r.app, tool));
      },
      opening: (m) => {
        if (!m.sheet && !platform.panel) screen.ready(detail, opensTo(m));
        return null;
      },
      open: (m) => {
        const to = opensTo(m);
        if (platform.panel) return void platform.main(to);
        if (m.sheet) return openSheet(to);
        // From a sheet, the sheet goes and it opens where the window is. What the sidebar lists (an app on a page,
        // Backups in Settings) opens as if picked there, with no Back; only what's deeper than it is pushed.
        f.sheet?.close(null);
        // Where it was opened from, in the window: a screen says where in its own frame.
        const at = f.frame.getBoundingClientRect();
        go(to, { pushed: !listed(to) }, Array.isArray(m.rect) && m.rect.length === 4 && m.rect.every(Number.isFinite) ? [m.rect[0] + at.left, m.rect[1] + at.top, m.rect[2], m.rect[3]] : undefined);
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
        else if (!f.dock && f.into.isConnected && !f.leaving) {
          f.said = title;
          f.name = m.name;
          // A large title lies in the bar's row.
          if (m.large && !("large" in f.into.dataset)) {
            f.into.dataset.large = "";
            f.frame.contentWindow?.postMessage({ oneBar: barStart }, "*");
          }
          if (f.begun) saying(f);
        }
        return null;
      },
      vault: async (need) => {
        if (mine) throw new Error("Settings has no app to ask for.");
        const out = await api("/v1/vault/request", { method: "POST", body: { scope: r.scope || "user", app: r.app, need } });
        if (!out?.id) throw new Error("The Vault didn’t take the request");
        return openRequest(out.scope, out.id);
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
        appearance: (m) => (m.value == null ? appearance() : new Promise((done) => ui.transition(() => (tell({ oneScheme: setAppearance(String(m.value)) }), done(appearance()))))),
        request: (m) => openRequest(String(m.scope || ""), String(m.id || "")),
      },
    }));
  } catch (err) {
    fail(err);
  }
});
