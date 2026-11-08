// The web client in a browser (platform.js): it signs in as an OAuth client of the network's auth (PKCE, and the
// user's passkey at its sign-in page), keeps its refresh token in this browser, and is its user's device
// (device.js, and one-client's keyring in the SharedWorker, net.js), let in by another of theirs, by a passkey of theirs,
// or by whoever invited them.
import { browserKeys, browserName, codeOf, passkeyKeysOf, signingInFor } from "./device.js";
import { del } from "./kept.js";
import { net } from "./net.js";
import { h, ui } from "./view.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const KEPT = "one.signin";
const CLIENT = "one.client";
/// Where auth sends this browser back to, signed in.
const back = () => `${location.origin}/callback`;

/// This browser as an OAuth client of its network's auth, each step one-client's (`sign_in`, in the worker, net.js's
/// `auth`): its registration and tokens kept here.
const auth = {
  kept: () => JSON.parse(localStorage.getItem(KEPT) || "null"),
  /// What a grant gave, kept: the refresh token before anything else can ask, as each works once.
  keep: (t, was) => localStorage.setItem(KEPT, JSON.stringify({ access: t.access, refresh: t.refresh || was, until: Date.now() + t.expiresIn * 1000 })),
  /// Its id as a client of auth, as it registered, coming back here.
  client: () => ((kept) => (kept?.redirect === back() ? kept.id : null))(JSON.parse(localStorage.getItem(CLIENT) || "null")),

  /// Off to auth's sign-in page, registered the first time, coming back to /callback, then to where this was. The
  /// device it signs in for (`join`) its sign-in then names, so the user's devices can let it in by it.
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

  /// Makes this browser a device of its user, before anything is asked of a node: one already, or let in by a passkey
  /// of theirs that's listed, or by one of their devices (or, for someone new, their inviter's), which it asks at
  /// auth and waits for there, showing its code (`waiting`). `api` is the request path, as a passkey's device asks.
  async function join(waiting, api) {
    // The service worker, which keeps this page and shows its notes, once someone's signed in.
    navigator.serviceWorker?.register("/sw.js", { type: "module" }).catch(() => {});
    const user = auth.user();
    const keys = await browserKeys(user);
    if (await net.start(user).catch(() => false)) return (device = keys);
    const invite = sessionStorage.getItem("one.invite");
    const ask = async () => net.ask(user, await auth.token(), browserName(), invite);
    await ask();
    if (await byPasskey(user, keys, api)) return (device = keys);
    waiting(await codeOf(keys.key, keys.seal), !!invite);
    // Auth says once it was approved; then the relay lets it in. One gone from auth (approved and cleared, refused, or
    // waited 15 minutes) is asked again, unless it's in.
    for (;;) {
      await sleep(2000);
      const said = await net.waited(user, await auth.token()).catch(() => "waiting");
      if (said === "signedOut") throw Object.assign(new Error("Signed out."), { signedOut: true });
      if (["gone", "approved"].includes(said) && (await net.start(user, true).catch(() => false))) break;
      if (said === "gone") await ask().catch(() => {});
    }
    sessionStorage.removeItem("one.invite");
    return (device = keys);
  }

  /// Lets this browser in by a passkey its user listed, whose PRF output this tab has from signing in: it asks as that
  /// passkey's device, which approves this browser's request by its code. → whether it's in.
  async function byPasskey(user, keys, api) {
    const prf = sessionStorage.getItem("one.prf");
    if (!prf) return false;
    const bytes = Uint8Array.from(atob(prf.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0));
    try {
      const pk = await passkeyKeysOf(bytes);
      if (!(await net.start(user, true, { key: pk.key, sign: pk.sign, seal: pk.opens.secret, sealKey: pk.seal }))) return false;
      await api("/v1/devices/approve", { method: "POST", body: { code: await codeOf(keys.key, keys.seal), confirm: true } });
      return await net.start(user, true);
    } catch (err) {
      console.warn("the passkey didn't let this browser in:", err);
      return false;
    }
  }

  /// This browser's key goes: a removed key can't be let in again, so signing in makes a new one. With it, what it kept
  /// as that device: the nodes it asks, and what its keyring kept (one-client's `keyring::Keep`: the network it was let
  /// into, who's who and what each kind of request needed), which the next approval names again. As a native app's
  /// device forgets (one-client's `Device::sign_out`).
  async function forgetDevice() {
    const user = auth.user();
    if (user) await Promise.all(["device", "nodes", "root", "welcome", "who.json", "needs.json", "invites.json"].map((k) => del(`${k}:${user}`))).catch(() => {});
  }

  /// What was last registered for pushes here: the subscription's endpoint, the device's key and the network's VAPID
  /// key, and when.
  const PUSHED = "one.pushed";
  /// Whether this browser can get notifications: "default" until the user says, "granted", "denied", or null where
  /// it can't.
  const notifyState = () => (device && "PushManager" in window && "Notification" in window ? Notification.permission : null);

  return {
    panel: false,
    client: () => "web",
    behind: () => false,
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
    signOut: () => auth.signOut(),
    welcome: (props) => h(ui.Welcome, props),

    join,
    reaches: (user) => net.start(user).catch(() => false),
    fetch: (...args) => net.fetch(...args),
    took: () => net.took(),
    /// Whether the relay says this browser isn't one of its user's devices any more: its key goes then. A client
    /// with no session yet (a tab that just took over the others') doesn't know.
    async removed() {
      if ((await net.still().catch(() => null)) !== false) return false;
      await forgetDevice();
      return true;
    },
    /// Out of its user's devices first (removing itself, which its keyring does as `leave`), so signing in again
    /// replaces it, then its key goes. One that can't get out in a few seconds (no node answering) goes anyway, and
    /// stays listed, last seen when it signed out, until it's removed in Settings › Devices.
    async leave(api) {
      if (device) await Promise.race([api("/v1/remove", { method: "POST", body: { key: device.key } }), sleep(20_000)]).catch((err) => console.warn("leaving your devices:", err));
      await navigator.serviceWorker?.getRegistration().then((r) => r?.pushManager.getSubscription()).then((s) => s?.unsubscribe()).catch(() => {});
      localStorage.removeItem(PUSHED);
      await forgetDevice();
    },
    /// The nodes this browser asks itself, in the order things run on them, the Cloudflare node by its run priority
    /// (last, unless an admin moved it).
    nodes(state) {
      const nodes = (state.network?.devices || []).filter((d) => d.serves_me).sort((a, b) => a.order - b.order).map((d) => d.key);
      const last = state.network?.cloudflare_node?.key;
      if (last && !nodes.includes(last)) nodes.push(last);
      const rank = (p) => (p === "last" ? 0xffffffff : p);
      const ranks = Object.fromEntries((state.network?.devices || []).filter((d) => d.priority != null).map((d) => [d.key, rank(d.priority)]));
      if (last) ranks[last] = rank(state.network.cloudflare_node.priority ?? "last");
      nodes.sort((a, b) => (ranks[a] ?? 1) - (ranks[b] ?? 1));
      if (nodes.length) net.nodes(state.user.id, nodes, ranks, last);
    },
    /// Once it's in: its user's own things' first key and where they run, made by the first of their devices.
    settle: () => net.settle().catch((err) => console.warn("making this user's own keys:", err)),
    kept: (app, tool, input) => net.kept(auth.user(), app, tool, input),
    renew: async (app) => device && net.renew(app),
    forget: () => net.forget(auth.user()),
    device: () => ({ name: browserName(), signOut: true, keys: !!device, notify: notifyState(), ...(device ? { key: device.key } : {}) }),

    changed() {},
    badge() {},
    appearance() {},
    copy: (text) => navigator.clipboard.writeText(text),
    open: (href) => window.open(href, "_blank", "noopener,noreferrer"),
    menubar: async () => [],
    bridge: () => ({}),
    look: () => ({}),
    fit() {},

    /// This browser gets notifications once its user allows them (`ask`, from Settings › Devices): its push
    /// subscription is registered with the push Worker as this device's, again whenever it changes and daily. → the
    /// permission, as `notifyState`.
    async notify(ask) {
      if (!notifyState()) return null;
      if (ask && Notification.permission === "default") await Notification.requestPermission();
      if (Notification.permission !== "granted") return notifyState();
      const { vapid } = await (await fetch("/push/key")).json();
      if (!vapid) return notifyState();
      const reg = await navigator.serviceWorker.ready;
      let sub = await reg.pushManager.getSubscription();
      const was = JSON.parse(localStorage.getItem(PUSHED) || "null");
      if (sub && was && was.vapid !== vapid) await sub.unsubscribe(), (sub = null);
      sub ||= await reg.pushManager.subscribe({ userVisibleOnly: true, applicationServerKey: Uint8Array.from(atob(vapid.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)) });
      if (was?.endpoint !== sub.endpoint || was?.key !== device.key || was?.vapid !== vapid || Date.now() - was.at > 86_400_000) {
        await net.push(sub.toJSON());
        localStorage.setItem(PUSHED, JSON.stringify({ endpoint: sub.endpoint, key: device.key, vapid, at: Date.now() }));
      }
      return notifyState();
    },
  };
}
