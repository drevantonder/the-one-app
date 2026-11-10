// What the client's own screens (Settings, Home, the panel) ask of this page, as the user, which no app's may:
// `one.api` to their nodes and to this device's own answers (onet's `settings`, its keyring signing what changes
// who can do what), the look, and what they keep. screens.js answers their
// bridges with these, and with what opens over the window.
import { platform } from "./platform.js";
import { api, developer, shared } from "./request.js";
import { keeping, leave, refreshState, screensKey, withWaiting } from "./store.js";

/// `one.api` for a client's own screen: the node's /v1 at this origin, as this user; nothing else here. What
/// changes who can do what, and letting devices in, this device answers itself, as its keyring signs it
/// (onet's `settings`), at the same paths.
async function clientApi(m) {
  const url = new URL(String(m.path || ""), location.origin);
  if (url.origin !== location.origin || !url.pathname.startsWith("/v1/")) throw new Error("That isn’t on your devices.");
  const method = String(m.method || "GET").toUpperCase();
  let body = method === "GET" ? undefined : m.body ?? {};
  if (url.pathname === "/v1/call" && body && typeof body === "object") body.client = platform.client();
  if (method === "GET" && !url.search && url.pathname === "/v1/state") return withWaiting(await shared(url.pathname));
  if (method === "GET" && !url.search && url.pathname === "/v1/apps") return shared(url.pathname);
  const out = await api(url.pathname + url.search, { method, body });
  if (method !== "GET") {
    if (url.pathname === "/v1/devices/refuse") (notMine.add(String(body?.code || "").toUpperCase().replace(/[^A-Z2-7]/g, "")), localStorage.setItem(NOT_MINE, JSON.stringify([...notMine].slice(-50))));
    await refreshState().catch(() => {});
  }
  return out;
}

/// The devices waiting whose user said they aren't theirs (Settings › Devices' Not Mine), by code: not asked again.
const NOT_MINE = "one.notMine";
export const notMine = new Set(JSON.parse(localStorage.getItem(NOT_MINE) || "[]"));

const LOOK = "one.appearance";
/// Light, dark or automatic, as the user chose here (Settings › Appearance).
export const appearance = () => (["light", "dark"].includes(localStorage.getItem(LOOK)) ? localStorage.getItem(LOOK) : "automatic");
/// The page's own rules, switched to a look.
export const schemed = oneHost.scheme();

/// The user's look, `value`, kept here and by the app, and this page drawn in it. → it.
export function setAppearance(value) {
  if (value === "automatic") localStorage.removeItem(LOOK);
  else if (["light", "dark"].includes(value)) localStorage.setItem(LOOK, value);
  schemed(document, appearance());
  platform.appearance(appearance());
  return appearance();
}

/// What the client's own screens' bridges may ask beyond an app's, but for what opens over the window (screens.js).
export const ownAnswers = {
  developer: (m) => ("on" in m && developer.set(!!m.on), developer.on()),
  api: clientApi,
  signOut: () => (leave(), null),
  // A Mac's widgets' own menu bar items: shown again, or hidden, from its panel.
  device: () => platform.device(),
  notify: () => platform.notify(true),
  cache: (m) => {
    const key = screensKey(String(m.key || ""));
    if (!("value" in m)) return JSON.parse(localStorage.getItem(key) || "null");
    try {
      if (keeping()) localStorage.setItem(key, JSON.stringify(m.value));
    } catch {}
    return null;
  },
};
