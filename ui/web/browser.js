// The web client in a browser (platform.js): it signs in as an OAuth client of the network's auth (PKCE, and the
// user's passkey at its sign-in page), keeps its refresh token in this browser, and is its user's device
// (device.js, and onet's keyring in the SharedWorker, net.js), let in by someone already in the network saying yes.
import { browserKeys, browserName, codeOf, signingInFor } from "./device.js";
import { del, get, put } from "./kept.js";
import { net } from "./net.js";
import { h, ui } from "./view.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

/// Whether this page was just signed in (`auth.finish`): only then does a key the relay refuses go.
let signedInNow = false;

const KEPT = "one.signin";
const CLIENT = "one.client";
/// Where auth sends this browser back to, signed in.
const back = () => `${location.origin}/callback`;

/// This browser as an OAuth client of its network's auth, each step onet's (`sign_in`, in the worker, net.js's
/// `auth`): its registration and tokens kept here.
const auth = {
  kept: () => JSON.parse(localStorage.getItem(KEPT) || "null"),
  /// What a grant gave, kept: the refresh token before anything else can ask, as each works once.
  keep: (t, was) => localStorage.setItem(KEPT, JSON.stringify({ access: t.access, refresh: t.refresh || was, until: Date.now() + t.expiresIn * 1000 })),
  /// Its id as a client of auth, as it registered, coming back here.
  client: () => ((kept) => (kept?.redirect === back() ? kept.id : null))(JSON.parse(localStorage.getItem(CLIENT) || "null")),

  /// Off to auth's sign-in page, registered the first time, coming back to /callback, then to where this was. The
  /// device it signs in for (`join`) its sign-in then names, for whoever answers it to check.
  async start() {
    const out = await net.auth({ step: "start", name: browserName(), back: back(), client: auth.client(), join: await signingInFor() });
    localStorage.setItem(CLIENT, JSON.stringify({ id: out.client, redirect: back() }));
    sessionStorage.setItem("one.pkce", JSON.stringify({ asked: out.asked, back: location.pathname + location.search }));
    location.href = out.url;
  },

  /// Back from signing in: the code becomes tokens. → where to go now.
  async finish() {
    const pkce = JSON.parse(sessionStorage.getItem("one.pkce") || "null");
    sessionStorage.removeItem("one.pkce");
    if (!pkce) throw new Error("That sign-in didn’t start here. Sign in again.");
    auth.keep((await net.auth({ step: "finish", asked: pkce.asked, got: Object.fromEntries(new URLSearchParams(location.search)) })).tokens);
    signedInNow = true;
    return pkce.back && pkce.back !== "/callback" ? pkce.back : "/";
  },

  /// An access token good for a minute more, new from the refresh token if it's older: one tab at a time, since
  /// each refresh token works once.
  async token(stale) {
    const fresh = (t) => t && t.access !== stale && t.until > Date.now() + 60_000;
    if (fresh(auth.kept())) return auth.kept().access;
    const refresh = async () => {
      const kept = auth.kept();
      if (fresh(kept)) return kept.access;
      if (!kept?.refresh) throw Object.assign(new Error("Signed out."), { signedOut: true });
      try {
        auth.keep((await net.auth({ step: "refresh", client: auth.client(), refresh: kept.refresh })).tokens, kept.refresh);
        return auth.kept().access;
      } catch (err) {
        if (err.signedOut) localStorage.removeItem(KEPT);
        throw err;
      }
    };
    return navigator.locks ? navigator.locks.request("one.refresh", refresh) : refresh();
  },

  user() {
    try {
      return JSON.parse(atob(auth.kept().access.split(".")[1].replaceAll("-", "+").replaceAll("_", "/"))).sub || null;
    } catch {
      return null;
    }
  },

  /// Its tokens gone, and revoked.
  async signOut() {
    const kept = auth.kept();
    localStorage.removeItem(KEPT);
    if (kept?.refresh) await net.auth({ step: "revoke", client: auth.client(), refresh: kept.refresh }).catch(() => {});
  },
};

export function browser() {
  /// This browser as its user's device, once it's listed as one (`join`): null until then.
  let device = null;

  /// Makes this browser a device of its user, before anything is asked of an app runtime: one already, or let in by one of their
  /// devices (or, for someone new, their inviter's), which it asks at auth and waits for there, showing its code
  /// (`waiting`). Just signed in, with a key the relay let in once and refuses now, it starts over with a new one: a
  /// removed key can't be let in again.
  async function join(waiting) {
    // The service worker, which keeps this page and shows its notes, once someone's signed in.
    navigator.serviceWorker?.register("/sw.js", { type: "module" }).catch(() => {});
    const user = auth.user();
    let keys = await browserKeys(user);
    const invite = sessionStorage.getItem("one.invite");
    const isIn = async () => (await put(`joined:${user}`, keys.key), sessionStorage.removeItem("one.invite"), (device = keys));
    // Just signed in, the worker may still hold the last session as this user, if another tab kept it alive.
    const started = await net.start(user, signedInNow).catch((err) => (err.unreachable ? Promise.reject(err) : false));
    if (signedInNow && (await get(`joined:${user}`)) === keys.key && (await net.still().catch(() => null)) === false) {
      await forgetDevice();
      keys = await browserKeys(user);
    } else if (started) {
      return isIn();
    }
    signedInNow = false;
    const ask = async () => net.ask(user, await auth.token(), browserName(), "browser", invite);
    const invited = await ask();
    waiting(await codeOf(keys.key, keys.seal), invited);
    await net.untilLetIn(user, () => auth.token(), ask);
    return isIn();
  }

  /// This browser's key goes, signing out or starting over: a removed key can't be let in again, so signing in makes a
  /// new one. With it, what it kept as that device: the devices it asks, and what its keyring kept (onet's
  /// `keyring::Keep`: the network it was let into, who's who and what each kind of request needed), which the next
  /// approval names again. As a native app's device forgets (onet's `Device::sign_out`).
  async function forgetDevice() {
    const user = auth.user();
    if (user) await Promise.all(["device", "joined", "nodes", "root", "welcome", "who.json", "needs.json", "invites.json"].map((k) => del(`${k}:${user}`))).catch(() => {});
  }

  /// What was last registered for notifications here: the subscription's endpoint, the device's key and the network's VAPID
  /// key, and when.
  const REGISTERED = "one.notifications";
  /// Whether this browser can get notifications: "default" until the user says, "granted", "denied", or null where
  /// it can't.
  const notifyState = () => (device && "PushManager" in window && "Notification" in window ? Notification.permission : null);

  return {
    panel: false,
    client: () => "web",
    behind: () => false,
    label: () => "",
    view: () => false,
    phone: () => false,
    address: () => location.origin,
    /// Back from signing in (`/callback`): its code becomes tokens, and the page goes where it was. → the problem, if
    /// that didn't work.
    async start() {
      if (location.pathname !== "/callback") return;
      try {
        history.replaceState(null, "", await auth.finish());
      } catch (err) {
        history.replaceState(null, "", "/");
        return err.message;
      }
    },
    signedIn: () => !!auth.kept(),
    user: () => auth.user(),
    until: () => auth.kept()?.until || Date.now() + 60_000,
    signIn: () => auth.start(),
    signOut: () => (navigator.clearAppBadge?.().catch(() => {}), auth.signOut()),
    welcome: (props) => h(ui.Welcome, props),

    join,
    reaches: (user) => net.start(user).catch(() => false),
    fetch: (...args) => net.fetch(...args),
    /// Whether the relay says this browser isn't one of its user's devices any more: it stops then, and keeps its key
    /// until signing in again (`join`). A client with no session yet (a tab that just took over the others') doesn't know.
    removed: async () => (await net.still().catch(() => null)) === false,
    /// Out of its user's devices first (removing itself, which its keyring does as `leave`), so signing in again
    /// replaces it, then its key goes. One that can't get out in a few seconds (no device answering) goes anyway, and
    /// stays listed, last seen when it signed out, until it's removed in Settings › Devices.
    async leave(api) {
      if (device) await Promise.race([api("/api/devices/remove", { method: "POST", body: { device: device.key } }), sleep(20_000)]).catch((err) => console.warn("leaving your devices:", err));
      await navigator.serviceWorker?.getRegistration().then((r) => r?.pushManager.getSubscription()).then((s) => s?.unsubscribe()).catch(() => {});
      localStorage.removeItem(REGISTERED);
      await forgetDevice();
    },
    nodes: (state) => net.ordered(state),
    /// Once it's in: its user's own things' first key and where they run, made by the first of their devices.
    settle: () => net.settle("browser").catch((err) => console.warn("making this user's own keys:", err)),
    kept: (app, tool, input) => net.kept(auth.user(), app, tool, input),
    renew: async (app) => device && net.renew(app),
    forget: () => net.forget(auth.user()),
    device: () => ({ name: browserName(), signOut: true, keys: !!device, notify: notifyState(), ...(device ? { key: device.key } : {}) }),

    changed() {},
    /// On the icon of this site installed as an app; 0 takes it off.
    badge: (count) => navigator.setAppBadge?.(count).catch(() => {}),
    appearance() {},
    copy: (text) => navigator.clipboard.writeText(text),
    open: (href) => window.open(href, "_blank", "noopener,noreferrer"),
    /// The address gives only the icons asked for. Asked for last, so that on a first visit the client's WebAssembly,
    /// which the first answer from the user's machines waits for, gets the connection first.
    icons: (names, v) => fetch(`/icons?${new URLSearchParams({ names: [...names].sort().join(","), v })}`, { priority: "low" }).then((r) => (r.ok ? r.json() : Promise.reject(new Error("No icons.")))),
    bridge: () => ({}),
    look: () => ({}),
    fit() {},

    /// This browser gets notifications once its user allows them (`ask`, from Settings › Devices): its push
    /// subscription is registered with the notifications Worker as this device's, again whenever it changes and daily. → the
    /// permission, as `notifyState`.
    async notify(ask) {
      if (!notifyState()) return null;
      if (ask && Notification.permission === "default") await Notification.requestPermission();
      if (Notification.permission !== "granted") return notifyState();
      const { vapid } = await (await fetch("/notifications/key")).json();
      if (!vapid) return notifyState();
      const reg = await navigator.serviceWorker.ready;
      let sub = await reg.pushManager.getSubscription();
      const was = JSON.parse(localStorage.getItem(REGISTERED) || "null");
      if (sub && was && was.vapid !== vapid) await sub.unsubscribe(), (sub = null);
      sub ||= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: Uint8Array.from(atob(vapid.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)) });
      if (was?.endpoint !== sub.endpoint || was?.key !== device.key || was?.vapid !== vapid || Date.now() - was.at > 86_400_000) {
        await net.registerNotifications(sub.toJSON());
        localStorage.setItem(REGISTERED, JSON.stringify({ endpoint: sub.endpoint, key: device.key, vapid, at: Date.now() }));
      }
      return notifyState();
    },
  };
}
