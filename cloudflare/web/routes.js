// The URL is the place: `/` (You), `/page/<scope>`, `/accounts` (every page's), `/settings`, `/settings/<screen>?params=`
// (a Settings screen), `/app/<app>/<screen>?params=&scope=`; and `/vault?id=&scope=`, a request for an item, as a sheet
// over You. Each is a screen: the client's own (`own`, by name), or an app's. The sidebar lists the person's pages,
// their apps and Settings' items (`sections`). The window goes to each as the URL changes (`whenMoved`).
import { platform } from "./platform.js";
import { accountsOn, appBadge, appsOn, here, label, pages, settingsItems } from "./store.js";

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
  return own("pages", { scope: r.scope || (settingsItems().some((i) => i.app === r.app) ? "settings" : "user") });
}

/// Goes somewhere: a sidebar row replaces where the window is; what a screen opens deeper than the sidebar is pushed
/// on top, so Back returns to it.
export function go(r, { pushed = false } = {}) {
  // Pushed on a screen, the sidebar keeps what it had selected, and the bar offers Back.
  const state = pushed ? { back: true, selection: history.state?.selection || location.pathname + location.search } : null;
  history.pushState(state, "", href(r));
  moved();
}
/// Back: to what came before in this tab, or for a screen opened straight, to the page it's under.
export function back() {
  if (history.state?.back) return history.back();
  history.replaceState(null, "", href(under(route())));
  moved();
}
/// The sidebar row selected: where the window is, or what a pushed screen was opened from.
const selection = () => history.state?.selection || location.pathname + location.search;

/// What goes where the URL now says (`whenMoved`): the window, drawn there.
const moves = [];
export const whenMoved = (fn) => moves.push(fn);
const moved = () => moves.forEach((fn) => fn());
addEventListener("popstate", moved);

/// Where each of the sidebar's links goes, as it was last listed.
const links = new Map();
/// Whether the sidebar lists `to`.
export const listed = (to) => links.has(href(to));
/// The route the sidebar's link `to` goes to, if it lists one.
export const linked = (to) => links.get(to);

/// The sidebar: one section per page, listing its apps and its Accounts; then Settings', listing its items. An app
/// Settings lists (the App Store) shows there only.
export function sections() {
  const at = selection();
  links.clear();
  const link = (to) => (links.set(href(to), to), href(to));
  const item = (to, key, title, more) => ({ key, title, href: link(to), selected: href(to) === at, ...more });
  const section = (to, key, title, items) => ({ key, title, href: link(to), selected: href(to) === at, items });
  const inSettings = (a) => settingsItems().some((i) => i.app === a.name);
  // Accounts once, under You: every page's, listed together.
  const accounts = [...new Map(pages().flatMap(accountsOn).map((a) => [a.name, a])).values()];
  const out = pages().map((p, i) => section(own("pages", { scope: p.scope }), p.scope, p.title, [
    ...appsOn(p).filter((a) => !inSettings(a)).map((a) => item({ kind: "screen", app: a.name, screen: "main", params: "{}", scope: p.scope }, a.name, label(a), { app: a, badge: appBadge(a.name, p) })),
    ...(i === 0 ? [item(own("accounts"), "accounts", "Accounts", { folder: accounts })] : []),
  ]));
  // One with another site's url opens it in a new tab; one here (Passkeys) shows beside the sidebar.
  const items = settingsItems().map((i) => {
    const app = { name: i.name, title: i.title, icon: i.icon, color: i.color };
    if (i.url && !here(i.url)) return { key: i.name, title: i.title, href: i.url, external: true, app };
    return item(i.app ? { kind: "screen", app: i.app, screen: "main", params: "{}" } : { kind: "own", name: i.name, params: "{}" }, i.name, i.title, { app, badge: i.badge });
  });
  return [...out, section(own("pages", { scope: "settings" }), "settings", "Settings", items)];
}
