// The web client in a browser (platform.js): it signs in as an OAuth client of the network's auth (PKCE, and the
// person's passkey at its sign-in page), keeps its refresh token in this browser, and is its person's device
// (device.js, and one-client's keyring in the SharedWorker, net.js), let in by another of theirs, by a passkey of theirs,
// or by whoever invited them.
import { waited } from "../shared/crypto.js";
import { answer, browserKeys, browserName, codeOf, joinAsk, passkeyKeysOf, signingInFor } from "./device.js";
import { del, put } from "./kept.js";
import { net } from "./net.js";
import { h, ui } from "./view.js";

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

const b64u = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
const random = (n) => b64u(crypto.getRandomValues(new Uint8Array(n)));

const KEPT = "one.signin";
const CLIENT = "one.client";

const auth = {
  kept: () => JSON.parse(localStorage.getItem(KEPT) || "null"),
  keep: (tokens) => localStorage.setItem(KEPT, JSON.stringify(tokens)),

  async meta() {
    const r = await fetch("/.well-known/oauth-authorization-server");
    if (!r.ok) throw new Error("Auth isn’t answering. Try again in a moment.");
    return r.json();
  },

  /// This browser, as a client of auth: registered the first time it signs in.
  async client(meta) {
    const kept = JSON.parse(localStorage.getItem(CLIENT) || "null");
    if (kept?.redirect === `${location.origin}/callback`) return kept.id;
    const r = await fetch(meta.registration_endpoint, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify({
        client_name: browserName(),
        redirect_uris: [`${location.origin}/callback`],
        token_endpoint_auth_method: "none",
        grant_types: ["authorization_code", "refresh_token"],
        response_types: ["code"],
      }),
    });
    const out = await r.json().catch(() => ({}));
    if (!r.ok || !out.client_id) throw new Error(out.error_description || out.message || "Couldn't start signing in. Try again.");
    localStorage.setItem(CLIENT, JSON.stringify({ id: out.client_id, redirect: `${location.origin}/callback` }));
    return out.client_id;
  },

  /// Off to auth's sign-in page, coming back to /callback, then to where this was.
  async start() {
    const meta = await auth.meta();
    const client = await auth.client(meta);
    const verifier = random(32), state = random(16);
    const challenge = b64u(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)));
    sessionStorage.setItem("one.pkce", JSON.stringify({ verifier, state, back: location.pathname + location.search }));
    const q = new URLSearchParams({
      response_type: "code", client_id: client, redirect_uri: `${location.origin}/callback`, scope: "openid profile offline_access",
      code_challenge: challenge, code_challenge_method: "S256", state,
      // The device it signs in for, which its sign-in then names, so the person's devices can let it in by it.
      join: await signingInFor(),
    });
    location.href = `${meta.authorization_endpoint}?${q}`;
  },

  /// Back from signing in: the code becomes tokens. → where to go now.
  async finish() {
    const q = new URLSearchParams(location.search);
    const pkce = JSON.parse(sessionStorage.getItem("one.pkce") || "null");
    sessionStorage.removeItem("one.pkce");
    if (q.get("error")) throw new Error(q.get("error_description") || q.get("error"));
    if (!pkce || q.get("state") !== pkce.state || !q.get("code")) throw new Error("That sign-in didn't start here. Sign in again.");
    const meta = await auth.meta();
    const client = JSON.parse(localStorage.getItem(CLIENT) || "null")?.id;
    auth.keep(await auth.grant(meta, { grant_type: "authorization_code", code: q.get("code"), redirect_uri: `${location.origin}/callback`, client_id: client, code_verifier: pkce.verifier }));
    return pkce.back && pkce.back !== "/callback" ? pkce.back : "/";
  },

  async grant(meta, fields) {
    const r = await fetch(meta.token_endpoint, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams(fields) });
    const out = await r.json().catch(() => ({}));
    if (!r.ok || !out.access_token) throw Object.assign(new Error(out.error_description || out.error || "Signing in didn't work."), { signedOut: r.status === 400 || r.status === 401 });
    return { access: out.access_token, refresh: out.refresh_token, until: Date.now() + (out.expires_in || 3600) * 1000 };
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
      const meta = await auth.meta();
      const client = JSON.parse(localStorage.getItem(CLIENT) || "null")?.id;
      try {
        const tokens = await auth.grant(meta, { grant_type: "refresh_token", refresh_token: kept.refresh, client_id: client });
        auth.keep({ ...tokens, refresh: tokens.refresh || kept.refresh });
        return tokens.access;
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
    try {
      const meta = await auth.meta();
      const client = JSON.parse(localStorage.getItem(CLIENT) || "null")?.id;
      if (kept?.refresh && meta.revocation_endpoint) {
        await fetch(meta.revocation_endpoint, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ token: kept.refresh, token_type_hint: "refresh_token", client_id: client }) });
      }
    } catch {}
  },
};

/// A request to the address that isn't a node's, with the access token: auth's own routes.
async function bearer(path, init = {}) {
  return fetch(path, { ...init, headers: { ...init.headers, authorization: `Bearer ${await auth.token()}` } });
}

export function browser() {
  /// This browser as its person's device, once it's listed as one (`join`): null until then.
  let device = null;

  /// Makes this browser a device of its person, before anything is asked of a node: one already, or let in by a passkey
  /// of theirs that's listed, or by one of their devices (or, for someone new, their inviter's), which it asks at
  /// auth and waits for there, showing its code (`waiting`). `api` is the request path, as a passkey's device asks.
  async function join(waiting, api) {
    // The service worker, which keeps this page and shows its notes, once someone's signed in.
    navigator.serviceWorker?.register("/sw.js", { type: "module" }).catch(() => {});
    const user = auth.user();
    const keys = await browserKeys(user);
    if (await net.start(user).catch(() => false)) return (device = keys);
    const invite = sessionStorage.getItem("one.invite");
    const ask = async () => answer(await bearer("/api/auth/devices/requests", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await joinAsk(keys, browserName(), invite)) }));
    await ask();
    if (await byPasskey(user, keys, api)) return (device = keys);
    waiting(await codeOf(keys.key, keys.seal), !!invite);
    // Auth says once it was approved; then the relay lets it in. One gone from auth (approved and cleared, refused, or
    // waited 15 minutes) is asked again, unless it's in. As every client does (`waited`).
    for (;;) {
      await sleep(2000);
      const r = await bearer(`/api/auth/devices/requests/${keys.key}`).catch(() => null);
      const said = waited(r?.status || 0, r?.ok ? await r.json().catch(() => ({})) : {});
      if (said.signedOut) throw Object.assign(new Error("Signed out."), { signedOut: true });
      // The network it was let into, as the device that approved it said: the only one it then takes; and where that
      // device read who's who to, which it starts from.
      if (said.root) await put(`root:${user}`, said.root);
      if (said.welcome) await put(`welcome:${user}`, said.welcome);
      if ((said.gone || said.approved) && (await net.start(user, true).catch(() => false))) break;
      if (said.gone) await ask().catch(() => {});
    }
    sessionStorage.removeItem("one.invite");
    return (device = keys);
  }

  /// Lets this browser in by a passkey its person listed, whose PRF output this tab has from signing in: it asks as that
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
  /// Whether this browser can get notifications: "default" until the person says, "granted", "denied", or null where
  /// it can't.
  const notifyState = () => (device && "PushManager" in window && "Notification" in window ? Notification.permission : null);

  return {
    panel: false,
    client: () => "web",
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
    /// Whether the relay says this browser isn't one of its person's devices any more: its key goes then. A client
    /// with no session yet (a tab that just took over the others') doesn't know.
    async removed() {
      if ((await net.still().catch(() => null)) !== false) return false;
      await forgetDevice();
      return true;
    },
    /// Out of its person's devices first (removing itself, which its keyring does as `leave`), so signing in again
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
    /// Once it's in: its person's own things' first key and where they run, made by the first of their devices.
    settle: () => net.settle().catch((err) => console.warn("making this person's own keys:", err)),
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

    /// This browser gets notifications once its person allows them (`ask`, from Settings › Devices): its push
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
