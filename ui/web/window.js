// The window: the sidebar (routes.js's `sidebar`), the screen beside it (screens.js), and what stands before it
// (signing in, waiting to be let in, a problem), drawn again as what it shows changes. It draws at once what it showed
// last (the sidebar, and a still of the screen) and then what the app runtime says now; and it keeps up with the network's events
// and the devices waiting to be let in.
import { offline } from "./offline.js";
import { platform } from "./platform.js";
import { qrSvg } from "./qr.js";
import { api, fresh, retryWait, sleep, tooOld } from "./request.js";
import { back, closeTop, go, home, linked, own, route, setSection, sidebar, whenMoved } from "./routes.js";
import { detail, hasScreen, markOpen, openSheet, replacing, screen, setBarStart, tabWaiting, tell, whenTitled } from "./screens.js";
import { notMine } from "./settings.js";
import { config, isJoining, leave, network, refreshApps, refreshState, setWaiting, signOut, store, titleOf, whenChanged } from "./store.js";
import { h, logo, message, show, ui } from "./view.js";

/// The bar's title over the screen, as the screen in it last said; and whether it offers Back, as the screen it's over
/// does, on a phone that's every screen but the home screen.
let barTitle = "", barBack = false;
/// Whether the window is what shows, rather than what stands before it.
let windowShown = false;
export const windowShowing = () => windowShown;
let shown = "";

export function render(opened) {
  if (!store.state || isJoining()) return;
  if (!network()) return void ((windowShown = false), show(message("Not Set Up Yet", "Set it up on one of your computers first.")));
  if (platform.panel) return drawPanel();
  const r = route(), key = JSON.stringify(r);
  // The screen where the window is, kept as it is while the rest redraws around it.
  const fresh = !(shown === key && hasScreen());
  if (fresh) shown = key;
  else if (!replacing()) barBack = barBackAt(r);
  drawWindow();
  markOpen("window");
  if (fresh) screen.show(detail, r, () => ((barTitle = home(r) ? "" : titleOf(r)), (barBack = barBackAt(r)), drawWindow()), opened);
}
whenChanged(render);
whenMoved(render);
whenTitled((title) => windowShown && title !== barTitle && ((barTitle = title), drawWindow()));
offline.whenChanged(() => {
  // The banner says it for every screen: none needs its own "As of" under it.
  tell({ oneOffline: !!offline.banner() });
  if (windowShown && !platform.panel) drawWindow();
});

/// The sidebar's open or closed on a computer's width, as the user left it.
const SIDEBAR = "one.sidebar";

/// The sidebar's badges, every one, as the native app counts them on its icon: said as the window is first drawn,
/// then only when it changes.
let badged = -1;
function countBadges(drawn) {
  const total = drawn.reduce((n, s) => n + Math.max(0, s.badge || 0), 0);
  if (total === badged) return;
  platform.badge((badged = total));
  tabWaiting(total);
}

/// A newer version of the client needs a newer app than this one, which stays on what it has.
const barBackAt = (r) => (platform.phone() ? !home(r) : !!history.state?.back);

const appBehind = () => {
  if (!platform.behind() && !tooOld.now) return null;
  // A computer's window is a view of the computer itself, which `one update` updates, the window with it.
  const device = platform.view() ? store.state?.me?.key : (store.state?.network?.clients || []).find((c) => c.key === store.state?.me?.key)?.device;
  const storeName = { iPhone: "the App Store", Android: "Google Play" }[platform.label()];
  const action = platform.client() === "web" ? { label: "Update", run: () => location.reload() }
    : device ? { label: "Update", run: () => api("/api/devices/update", { method: "POST", body: { device } }) }
    : null;
  const text = storeName ? `Update it from ${storeName}.` : action ? "It can’t read what your other devices have moved on to." : "Its next build is on the way.";
  return h(ui.Problem, { quiet: true, title: "This device needs updating", text, action, className: "max-w-none rounded-none border-0 border-b" });
};
tooOld.changed = () => {
  if (windowShown && !platform.panel) drawWindow();
};

function drawWindow() {
  windowShown = true;
  const drawn = sidebar();
  countBadges(drawn);
  const banner = appBehind() || offline.banner();
  if (platform.phone()) return void show(h(ui.Shell, { phone: true, title: barTitle, back: barBack, onBack: back, screen: detail, banner }));
  show(h(ui.Shell, {
    sections: drawn,
    title: barTitle,
    back: barBack,
    onBack: back,
    onGo: (to) => linked(to) && go(linked(to)),
    onPress: (to) => linked(to) && screen.ready(detail, linked(to)),
    onSection: (key, open) => (setSection(key, open), drawWindow()),
    onBarStart: setBarStart,
    open: localStorage.getItem(SIDEBAR) !== "closed",
    onOpenChange: (open) => (open ? localStorage.removeItem(SIDEBAR) : localStorage.setItem(SIDEBAR, "closed"), drawWindow()),
    screen: detail,
    banner,
  }));
}

/// The app's menu bar panel: the network's name, the `panel` screen (your devices, your widgets), and the app's rows
/// under it, or with no one signed in here, Sign In… in the window. Each row closes it, as a menu's item does.
function drawPanel() {
  const signedIn = !!store.state && !!network();
  const screenHere = signedIn ? h("div", { className: "panel-screen", ref: (el) => el && detail.parentElement !== el && el.append(detail) }) : h("p", { className: "panel-note" }, "Signed out");
  windowShown = true;
  const row = ([label, run, checked]) => h(ui.Button, { key: label, variant: "ghost", className: "w-full justify-between px-3 font-normal", onClick: run }, label, checked ? h(ui.Icon, { name: "check", className: "size-4" }) : null);
  show(h("div", { className: "panel" },
    h("div", { className: "panel-head" }, h("img", { src: logo, alt: "" }), h("span", null, network()?.name || config().name)),
    screenHere,
    h("div", { className: "panel-rows" }, ...platform.panelRows(signedIn, config().name, drawPanel).map(row))));
  if (signedIn && shown !== "panel") (shown = "panel"), screen.show(detail, own("panel", platform.panelScreen()));
}

/// A phone's hardware Back: what's over the window or its screen (a sheet, an alert, a menu) closes first, as Escape does; then a
/// pushed screen goes back, and so does one a user has gone deeper than the home screen. → whether it took it: if
/// not, the app leaves.
export function hardwareBack() {
  if (closeTop()) return true;
  if (history.state?.back || (platform.phone() && !home(route()))) return back(), true;
  return false;
}

/// The app runtime's events: what changed, so the window and its screens keep up, only while the page shows. An app runtime with
/// none to say (oned on Cloudflare, while no machine is on) answers 204, and it asks again in a minute.
let events = null;
export async function listen() {
  const mine = (events = new AbortController());
  for (let misses = 0; !mine.signal.aborted; ) {
    const started = Date.now();
    try {
      const r = await api("/api/events", { raw: true, signal: mine.signal });
      // Heard from an app runtime again: whatever else is waiting on one is asked now, not when its turn comes.
      offline.done("events");
      if (offline.waiting.size) offline.now();
      if (r.status === 204) {
        await sleep(60_000);
        continue;
      }
      const reader = r.body.pipeThrough(new TextDecoderStream()).getReader();
      let buffer = "";
      for (;;) {
        const { value, done } = await reader.read();
        if (done) break;
        buffer += value;
        let at;
        while ((at = buffer.indexOf("\n\n")) >= 0) {
          const chunk = buffer.slice(0, at);
          buffer = buffer.slice(at + 2);
          const data = chunk.split("\n").filter((l) => l.startsWith("data:")).map((l) => l.slice(5).trim()).join("\n");
          if (data) handle(JSON.parse(data)).catch(() => {});
        }
      }
    } catch (err) {
      if (err.signedOut) return err.removed ? undefined : void signOut(err.message);
    }
    if (Date.now() - started > 10_000) misses = 0;
    // What shows may be out of date from here on, with nothing to say so until it's asked for again.
    if (!mine.signal.aborted) offline.add("events", () => api("/api/state"));
    if (!mine.signal.aborted) await sleep(retryWait(misses++));
  }
}
addEventListener("pagehide", () => events?.abort());
// Back to a page the browser kept: what changed while it was away, then its events again.
addEventListener("pageshow", (e) => e.persisted && platform.signedIn() && (refreshState().catch(() => {}), listen()));

async function handle(event) {
  fresh();
  tell({ oneEvent: event });
  if (["devices", "network"].includes(event.type)) await refreshState();
  if (event.type === "changed") {
    // The app's own widgets (the home screen's, the Widgets board's, the menu bar's) are drawn again too.
    platform.changed();
    if (!(event.apps || []).length) await refreshState();
    await refreshApps();
  }
}

/// Devices waiting that this device may let in, asked of auth every few seconds while the page shows: the first time
/// one is seen here, Devices opens to ask about it. Those its user said aren't theirs aren't asked again.
const seen = new Set();
let prompting = false;
export async function watchWaiting() {
  for (;;) {
    if (document.visibilityState === "visible") await waitingNow().catch(() => {});
    await sleep(10_000);
  }
}
async function waitingNow() {
  const now = (await api("/api/devices/requests", { method: "POST", body: {} })).map((r) => ({ ...r, key: r.code.replaceAll(" ", "") })).filter((r) => !notMine.has(r.key));
  if (setWaiting(now)) tell({ oneEvent: { type: "devices" } });
  const fresh = now.filter((r) => !seen.has(r.key));
  fresh.forEach((r) => seen.add(r.key));
  const r = route();
  if (fresh.length && !prompting && !(r.kind === "own" && r.name === "devices")) {
    prompting = true;
    openSheet(own("devices")).finally(() => (prompting = false));
  }
  return now;
}

export function showSignedOut(error, note) {
  if (platform.panel) return drawPanel();
  document.title = config().name;
  windowShown = false;
  const signIn = () => platform.signIn().catch((err) => showSignedOut(err.message));
  show(platform.welcome({ logo, title: config().name, action: { label: "Sign In", run: signIn }, note, problem: error }));
}
/// "Waiting for Approval": its code, and the code as a QR, a link that opens Approve with it in the browser of a
/// phone that scans it.
export function showJoin(code, invited) {
  // An invite already said yes: an admin's device lets this one in by itself as soon as one is online.
  if (invited) {
    document.title = "Letting You In";
    windowShown = false;
    return show(h(ui.Welcome, { key: "invited", logo, title: "Letting You In", text: "You'll be in as soon as an admin's device is online.", other: { label: "Sign Out", run: () => leave() } }));
  }
  document.title = "Waiting for Approval";
  const link = `${platform.address()}/approve#${code.replaceAll(" ", "")}`;
  windowShown = false;
  show(h(ui.Join, {
    logo,
    code,
    qr: qrSvg(link),
    text: "Approve it on your other device, or ask an admin.",
    onSignOut: () => leave(),
  }));
}

/// Back from a sign-in at another site, in a browser that isn't signed in here: on to the app, which finishes it.
export function showBack(app, screen, params) {
  const { name, cli } = config();
  const link = `${cli}://open?${new URLSearchParams({ app, screen, params, scope: "user" })}`;
  document.title = name;
  show(h(ui.Welcome, { logo, title: name, action: { label: `Open ${name}`, href: link } }));
  location.href = link;
}

export function showUnreachable() {
  windowShown = false;
  show(message("Can’t Reach Your Devices", "Trying again…"));
}
