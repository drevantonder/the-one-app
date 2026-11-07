// What the window shows, as the person's nodes say it: their state (`/v1/state`), their apps and their badges, kept
// here as the window was last so it draws at once; the devices waiting to be let in; and signing out, which forgets it.
// The window draws it again whenever it changes (`whenChanged`).
import { platform } from "./platform.js";
import { api, shared, whenRemoved } from "./request.js";

/// The sidebar's: the node's state, its apps and their badges. What's beside it is a screen the node serves.
export const store = { state: null, apps: [], badges: {} };

export const me = () => store.state?.user;
export const network = () => store.state?.network;
// The custom build's name, as the gateway's index.html says it, until the network's state does.
const customBuild = { name: document.querySelector('meta[name="application-name"]')?.content || "", cli: document.querySelector('meta[name="one-cli"]')?.content || "" };
export const config = () => store.state?.config || customBuild;
export const app = (name) => store.apps.find((a) => a.name === name);
/// Whether an app's tool only reads, as its app says: only what such a call answered is kept (one-client's `answers`).
export const reads = (name, tool) => !!app(name)?.tools?.some((t) => t.name === tool && t.reads === true);
export const label = (a) => a?.title || a?.name || "";
const hasScreens = (a) => !!a?.screens?.some((s) => s.name === "main");
const keepsAccounts = (a) => !!a?.tools?.some((t) => t.accounts);
const scopes = (a) => (a.added || []).map((s) => (s.startsWith("user:") ? "user" : s));

/// Every page this person can read, as the sidebar lists them: the node's (`pages` in `/v1/state`).
export const pages = () => store.state?.pages || [];
const page = (scope) => pages().find((p) => p.scope === scope);
export const appsOn = (p) => store.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
export const accountsOn = (p) => store.apps.filter((a) => keepsAccounts(a) && scopes(a).includes(p.scope)).sort((a, b) => label(a).localeCompare(label(b)));
export const appBadge = (name, p) => Math.max(0, store.badges[p.scope]?.[name] || 0);

/// Devices waiting that this device may let in, as last asked (window.js): Settings › Devices has a badge for each.
let waiting = [];
/// They're `now`: → whether that changed, and the window is drawn again if it did.
export function setWaiting(now) {
  const changed = now.map((r) => r.key).join() !== waiting.map((r) => r.key).join();
  waiting = now;
  if (changed) draw();
  return changed;
}
/// The state with a badge on Settings › Devices for each device waiting.
export function withWaiting(state) {
  if (!state || !waiting.length) return state;
  const items = (state.settings_items || []).map((i) => (i.name === "devices" ? { ...i, badge: (i.badge || 0) + waiting.length } : i));
  return { ...state, settings_items: items };
}

/// What Settings shows this person, in order, as the node says (`settings_items`).
export const settingsItems = () => withWaiting(store.state)?.settings_items || [];
/// Whether a url is this site's: a page the gateway serves, shown in place rather than in a new tab.
export const here = (url) => new URL(url, location.href).origin === location.origin;
/// The Settings item whose url is `url` on this site, if one is.
export const settingAt = (url) => settingsItems().find((i) => i.url && here(i.url) && new URL(i.url, location.href).pathname === new URL(url, location.href).pathname);

/// What a route is called: in the bar, a sheet, the tab.
export function titleOf(r) {
  if (r.kind === "own") {
    const params = JSON.parse(r.params || "{}");
    if (r.name === "pages") return params.scope === "settings" ? "Settings" : page(params.scope || "user")?.title || "";
    if (r.name === "accounts") return "Accounts";
    if (r.name === "request") return "Vault";
    if (r.name === "devtools") return "Developer";
    return settingsItems().find((i) => i.name === r.name)?.title || "Settings";
  }
  const a = app(r.app);
  return a?.screens?.find((s) => s.name === r.screen)?.title || label(a) || r.app;
}

/// The window as it was last (and when), drawn before the node answers.
const LAST = "one.last.window";
/// What the client's own screens keep between visits (`one.cache`).
export const SCREENS = "one.screens.";
/// The screens' pages and stills this device kept (screens.js), in the browser's cache.
export const KEPT_SCREENS = "one.screens";

/// What the client's own screens keep between visits, the window as it was last, the screens it showed and what their
/// calls answered (the device's, one-client's `answers`): all forgotten with the sign-in.
export function forgetScreens() {
  Object.keys(localStorage).filter((k) => k.startsWith(SCREENS) || k.startsWith(LAST)).forEach((k) => localStorage.removeItem(k));
  window.caches?.delete(KEPT_SCREENS).catch(() => {});
  return platform.forget();
}

/// The window as it was last, drawn before the node answers: → whether there was one.
export function lastWindow() {
  try {
    const kept = JSON.parse(localStorage.getItem(LAST) || "null");
    if (!kept?.state?.pages) return false;
    Object.assign(store, kept);
    return true;
  } catch {
    return false;
  }
}
const keepWindow = () => {
  try {
    localStorage.setItem(LAST, JSON.stringify(store));
    localStorage.setItem(`${LAST}.at`, String(Date.now()));
  } catch {}
};
export const lastAt = () => Number(localStorage.getItem(`${LAST}.at`)) || null;

/// While this device waits to be let in, the window isn't drawn: its join screen is.
let joining = false;
export const isJoining = () => joining;
export const setJoining = (on) => (joining = on);
/// Settled once this device has been let in as its person's device, or needn't be: what screens ask waits for it,
/// since only a device hands a node the keys it needs.
export const joined = (() => {
  let done;
  const p = new Promise((r) => (done = r));
  return Object.assign(p, { done });
})();

/// What draws the window again (`whenChanged`), each time what it shows changes.
const changes = [];
export const whenChanged = (fn) => changes.push(fn);
const draw = () => changes.forEach((fn) => fn());

/// Loads what the sidebar shows: the node's state and its apps together (its apps only if `apps`, or if who's in
/// which group changed), drawn as soon as they're in; then the badges, drawn as they come.
export async function refreshState({ apps = false } = {}) {
  const [state, list] = await Promise.all([shared("/v1/state"), apps && !joining ? shared("/v1/apps").catch(() => null) : null]);
  if (state.user) platform.nodes(state);
  const regrouped = JSON.stringify([state.user, state.network?.groups]) !== JSON.stringify([store.state?.user, store.state?.network?.groups]);
  store.state = state;
  // While it's being let in, its apps wait: they need the keys it's about to get.
  if (joining) return;
  if (list) store.apps = list;
  if (!list && (regrouped || !store.apps.length) && network()) return refreshApps();
  draw();
  keepWindow();
  badges();
}

export async function refreshApps() {
  if (!network()) return draw();
  store.apps = await shared("/v1/apps");
  draw();
  keepWindow();
  badges();
}

async function badges() {
  await Promise.all(pages().map(async (p) => (store.badges[p.scope] = await api("/v1/badges", { query: { scope: p.scope } }).catch(() => store.badges[p.scope] || {}))));
  draw();
  keepWindow();
}

/// What shows once it's signed out (`whenSignedOut`), with why.
const signedOut = [];
export const whenSignedOut = (fn) => signedOut.push(fn);

/// Signs out, back at the sign-in screen, which says `why` if given.
export async function signOut(why) {
  await forgetScreens();
  await platform.signOut();
  if (!why) return location.replace("/");
  signedOut.forEach((fn) => fn(why));
}
whenRemoved(signOut);

/// Signs out as a device: out of its person's devices first, so signing in again replaces it, then its tokens go.
export async function leave() {
  await platform.leave(api);
  return signOut();
}
