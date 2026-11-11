// The URL is the place: `/` (You), `/page/<scope>`, `/accounts` (every page's), `/settings`, `/settings/<screen>?params=`
// (a Settings screen), `/app/<app>/<screen>?params=&scope=`; and `/vault?id=&scope=`, a request for an item, as a sheet
// over You. Each is a screen: the client's own (`own`, by name), or an app's. The sidebar lists the user's pages with their
// apps, and Settings with its items (`sidebar`). The window goes to each as the URL changes (`whenMoved`).
import { platform } from "./platform.js";
import { accountsOn, appBadge, appsOn, here, label, pages, settingsItems, store } from "./store.js";
import { ui } from "./view.js";

export function route(at = location) {
  const r = routeAt(at);
  // On a phone every page, Settings too, is the pages side by side, opened at that one.
  if (platform.phone() && r.kind === "own" && r.name === "pages") return own("pages", { swipe: true, scope: JSON.parse(r.params).scope || "user" });
  return r;
}
function routeAt(at) {
  const path = at.pathname.split("/").filter(Boolean).map(decodeURIComponent);
  const q = new URLSearchParams(at.search);
  if (path[0] === "page" && path[1]) return own("pages", { scope: path[1] });
  if (path[0] === "accounts") return own("accounts");
  if (path[0] === "app" && path[1]) return { kind: "screen", app: path[1], screen: path[2] || "main", params: q.get("params") || "{}", scope: q.get("scope") || undefined };
  if (path[0] === "settings") return path[1] ? { kind: "own", name: path[1], params: q.get("params") || "{}" } : own("pages", { scope: "settings" });
  return own("pages", { scope: "user" });
}
export const own = (name, params = {}) => ({ kind: "own", name, params: JSON.stringify(params) });
export const home = (r) => r.kind === "own" && r.name === "pages";

export function href(r) {
  const q = new URLSearchParams();
  if (r.kind === "own") {
    const params = JSON.parse(r.params || "{}");
    if (r.name === "pages") return params.scope === "settings" ? "/settings" : !params.scope || params.scope === "user" ? "/" : `/page/${encodeURIComponent(params.scope)}`;
    if (r.name === "accounts") return "/accounts";
    if (r.params && r.params !== "{}") q.set("params", r.params);
    return `/settings/${encodeURIComponent(r.name)}${q.size ? `?${q}` : ""}`;
  }
  if (r.params && r.params !== "{}") q.set("params", r.params);
  if (r.scope) q.set("scope", r.scope);
  return `/app/${encodeURIComponent(r.app)}/${encodeURIComponent(r.screen || "main")}${q.size ? `?${q}` : ""}`;
}

/// The page a screen is under, for Back when nothing in this tab came before it: a page's Accounts and apps under
/// that page, Settings' screens and apps under Settings.
function under(r) {
  if (r.kind === "own") return own("pages", { scope: ["accounts", "request"].includes(r.name) ? "user" : "settings" });
  const page = settingsItems().some((i) => i.app === r.app) ? "settings" : pages().find((p) => !p.hidden && appsOn(p).some((a) => a.name === r.app))?.scope;
  return own("pages", { scope: r.scope || page || "user" });
}

/// What's over the window (a sheet, a dialog, a menu) holds a history entry, so Back closes the top one and the page
/// stays, as Escape does: `hold(close)` → what to call once it's gone some other way, which takes its entry away. An
/// entry's `over` is how many are held at it.
const held = [];
/// What waits for the entries held to be behind us: going somewhere, once Back has closed what's over the window.
let unwinding = null;
export function hold(close) {
  const entry = { close };
  held.push(entry);
  history.pushState({ ...history.state, over: held.length }, "", location.href);
  return () => {
    const at = held.indexOf(entry);
    if (at < 0) return;
    held.splice(at, 1);
    if (at === held.length && history.state?.over === at + 1) unwind(null, 1);
  };
}
/// Closes the top of what's held, for a hardware Back → whether there was one.
export const closeTop = () => held.length > 0 && (held.at(-1).close(), true);
/// Back `steps` entries (by default to the one under every held), then `then`.
function unwind(then, steps = history.state?.over) {
  if (!unwinding) {
    unwinding = [];
    history.go(-steps);
  }
  if (then) unwinding.push(then);
}
// Reloaded with something held open: what it held is gone, so its entries are too, or Back would seem to do nothing.
if (history.state?.over) history.go(-history.state.over);

/// Goes somewhere: a sidebar row replaces where the window is; what a screen opens deeper than the sidebar is pushed
/// on top, so Back returns to it.
export function go(r, { pushed = false } = {}, opened) {
  if (unwinding || history.state?.over) return unwind(() => go(r, { pushed }, opened));
  // Already there (a double tap): one Back still leaves it.
  if (href(r) === location.pathname + location.search) return moved(opened);
  // Pushed on a screen, the sidebar keeps what it had selected, and the bar offers Back.
  const state = pushed ? { back: true, selection: history.state?.selection || location.pathname + location.search } : null;
  history.pushState(state, "", href(r));
  moved(opened);
}
/// Back: to what came before in this tab, or for a screen opened straight, to the page it's under.
export function back() {
  if (unwinding || history.state?.over) return unwind(back);
  if (history.state?.back) return history.back();
  history.replaceState(null, "", href(under(route())));
  moved();
}
/// The sidebar row selected: where the window is, or what a pushed screen was opened from.
const selection = () => history.state?.selection || location.pathname + location.search;

/// What goes where the URL now says (`whenMoved`): the window, drawn there.
const moves = [];
export const whenMoved = (fn) => moves.push(fn);
const moved = (opened) => moves.forEach((fn) => fn(opened));
addEventListener("popstate", () => {
  // What's held beyond the entry now at goes, as Back's meant it to.
  const depth = history.state?.over || 0;
  const closing = held.length > depth || unwinding;
  while (held.length > depth) held.pop().close();
  if (!closing) return moved();
  const then = unwinding || [];
  unwinding = null;
  then.forEach((fn) => fn());
});

/// Where each of the sidebar's links goes, as it was last listed.
const links = new Map();
/// Whether the sidebar lists `to`.
export const listed = (to) => links.has(href(to));
/// The route the sidebar's link `to` goes to, if it lists one.
export const linked = (to) => links.get(to);

/// The page a place in the window is on: a page itself, or the one its screen is under.
function pageOf(at) {
  const r = routeAt(new URL(at, location.href));
  return JSON.parse((home(r) ? r : under(r)).params).scope;
}

/// The sidebar's sections the user has collapsed on this device.
const COLLAPSED = "one.sidebar.collapsed";
const collapsed = new Set(JSON.parse(localStorage.getItem(COLLAPSED) || "[]"));
export function setSection(key, open) {
  if (open) collapsed.delete(key);
  else collapsed.add(key);
  localStorage.setItem(COLLAPSED, JSON.stringify([...collapsed]));
}

/// Where the sidebar last drew itself for: arriving somewhere else opens its section, which its user may collapse again
/// while they stay.
let drawnAt = null;

/// The sidebar: a section for each page the home shows, its apps and (the first's) Accounts in its home's order, and
/// Settings' last. What's selected is a row, or a section's name for a place in it that isn't one or while it's collapsed.
export function sidebar() {
  const at = selection(), now = pageOf(at);
  if (at !== drawnAt && collapsed.has(now)) setSection(now, true);
  drawnAt = at;
  links.clear();
  const link = (to) => (links.set(href(to), to), href(to));
  const row = (to, key, title, more) => ({ key, title, href: link(to), selected: href(to) === at, ...more });
  const section = (to, key, title, items) => row(to, key, title, { items, open: !collapsed.has(key), badge: items.reduce((n, i) => n + (i.badge || 0), 0) });
  const shown = pages().filter((p) => !p.hidden);
  // Accounts once, under You: every page's, listed together.
  const accounts = [...new Map(shown.flatMap(accountsOn).map((a) => [a.name, a])).values()];
  const sections = [
    ...shown.map((p, i) => section(own("pages", { scope: p.scope }), p.scope, p.title, [
      ...ui.arranged(appsOn(p), store.state.arrangements?.[p.scope]).map(({ app: a }) => row({ kind: "screen", app: a.name, screen: "main", params: "{}", scope: p.scope }, a.name, label(a), { app: a, badge: appBadge(a.name, p) })),
      ...(i === 0 ? [row(own("accounts"), "accounts", "Accounts", { folder: accounts })] : []),
    ])),
    section(own("pages", { scope: "settings" }), "settings", "Settings", settingsItems().map((i) => {
      const app = { name: i.name, title: i.title, icon: i.icon, color: i.color };
      if (i.url && !here(i.url)) return { key: i.name, title: i.title, href: i.url, external: true, app };
      return row(i.app ? { kind: "screen", app: i.app, screen: "main", params: "{}" } : own(i.name), i.name, i.title, { app, badge: i.badge });
    })),
  ];
  return sections.map((s) => (s.key === now && (!s.open || !links.has(at)) ? { ...s, selected: true } : s));
}
