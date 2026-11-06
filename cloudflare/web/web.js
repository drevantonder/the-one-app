// The web client: the Mac app's window, in a browser. It signs in as an OAuth client of the network's auth
// (PKCE, and the person's passkey at its sign-in page), keeps its refresh token in this browser, and asks the person's nodes over iroh (`net.js`). App screens are untrusted: each runs in a sandboxed
// frame, and this page answers its bridge as the Mac app's web view does, making each call itself. It draws at once
// what it showed last (the sidebar, and a still of the screen) and then what the node says now.

// The bridge (src/screen/), loaded after this, gives `one`; this page answers it for its own use (`__one.host`).
// The window itself, and what it shows over screens (menus, alerts, sheets), is @one/ui's (src/screen/ui), whose
// ui.js, loaded before this, keeps it on `OneUI`.
// This browser is a device of its person (device.js, and one-client's keyring in net-worker.js): it signs what changes
// who can do what, and hands each call the keys it needs.
import { del, put } from "./kept.js";
import { waited } from "../shared/crypto.js";
import { answer, browserKeys, browserName, codeOf, joinAsk, passkeyKeysOf, signingInFor } from "./device.js";
import { net } from "./net.js";
import { qrSvg } from "./qr.js";

window.__one = {};
const ui = OneUI["@one/ui"];
/// In the native app, the app itself (client/native/src/native.js): this device's client, which reaches the nodes and
/// holds its keys, and signs in through the system's browser. Everything else is this page's, as in a browser.
const native = window.__oneNative;
/// What the app says of itself and who's signed in there (`native.about`), once the page starts.
let about = null;
/// The app's menu bar (tray) panel, a small window of its own: this page showing only the client's own `panel` screen,
/// with the app's rows under it.
const panel = !!native && new URLSearchParams(location.search).has("panel");
/// On a Mac, a widget's own menu bar item opens the panel showing only that widget (`<app>.<widget>`), with no rows.
const panelWidget = panel ? new URLSearchParams(location.search).get("widget") : null;
/// The app on a phone: its home is the pages side by side, with no sidebar (`Shell`'s `phone`).
const phone = () => !!native && ["iPhone", "Android"].includes(about?.platform);
const h = OneUI.react.createElement;

(() => {
  const $ = (sel, el = document) => el.querySelector(sel);
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const b64u = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  const random = (n) => b64u(crypto.getRandomValues(new Uint8Array(n)));

  const KEPT = "one.signin";
  // What the client's own screens keep between visits (one.cache), the window as it was last (LAST), the screens it
  // showed (KEPT_SCREENS, `kept`) and what their calls answered (the device's, one-client's `answers`): all forgotten
  // with the sign-in.
  const SCREENS = "one.screens.";
  const LAST = "one.last.window";
  const KEPT_SCREENS = "one.screens";
  const forgetScreens = () => {
    Object.keys(localStorage).filter((k) => k.startsWith(SCREENS) || k.startsWith(LAST)).forEach((k) => localStorage.removeItem(k));
    window.caches?.delete(KEPT_SCREENS).catch(() => {});
    return native ? undefined : net.forget(auth.user());
  };
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
          if (err.signedOut) localStorage.removeItem(KEPT), forgetScreens();
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

    /// Signs out, back at the sign-in screen, which says `why` if given.
    async signOut(why) {
      const kept = auth.kept();
      localStorage.removeItem(KEPT);
      await forgetScreens();
      try {
        const meta = await auth.meta();
        const client = JSON.parse(localStorage.getItem(CLIENT) || "null")?.id;
        if (kept?.refresh && meta.revocation_endpoint) {
          await fetch(meta.revocation_endpoint, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ token: kept.refresh, token_type_hint: "refresh_token", client_id: client }) });
        }
      } catch {}
      if (why) return showSignedOut(why);
      location.href = "/";
    },
  };

  // The app signs in itself, keeping its tokens in the OS's secret store, never this page.
  if (native) {
    Object.assign(auth, {
      kept: () => (about?.signedIn ? {} : null),
      user: () => about?.user || null,
      start: () => native.signIn().then(() => location.reload()),
      async signOut(why) {
        await native.signOut().catch(() => {});
        await forgetScreens();
        about = await native.about();
        if (why) return showSignedOut(why);
        location.replace("/");
      },
    });
  }

  class NodeError extends Error {}
  const UNANSWERED = "Your devices aren’t answering. Make sure one of them is on and online.";

  /// This browser as its person's device, once it's listed as one (`join`): null until then.
  let device = null;

  /// How long to wait before asking again the `n`th time running (from 0) after the node didn't answer: at once, then
  /// a quarter second, doubling, up to five. A node restarting for an update is back within a moment.
  const retryWait = (n) => (n === 0 ? 0 : Math.min(5000, 125 << Math.min(n, 6)));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

  /// A request to the node, as this person: JSON in and out, or with `raw` the response itself. Only a browser that's
  /// one of their devices asks, of their nodes, itself, over iroh (net.js), saying whose it is (`x-one-user`), which a
  /// node checks against the device its connection proves. It goes with the keys it needed last time; a node that needs
  /// others says so (428), and it goes again once with them. Asked again for a few seconds while no node answers, as
  /// one restarting doesn't; a call carries an id, so it runs once however often it's sent.
  async function api(path, { method = "GET", body, query, raw, signal, since } = {}) {
    const url = new URL(path, location.origin);
    for (const [k, v] of Object.entries(query || {})) if (v != null) url.searchParams.set(k, typeof v === "string" ? v : JSON.stringify(v));
    if (url.pathname === "/v1/call" && body && typeof body === "object" && !body.id) body = { ...body, id: callId() };
    const trace = developer.on() ? developer.start(url.pathname, body, since) : null;
    try {
      const out = await sending(url, method, body, raw, signal, trace);
      if (trace) trace.status = raw ? out.status : 200;
      return out;
    } catch (err) {
      if (trace) trace.failed = true;
      throw err;
    } finally {
      if (trace) developer.keep(trace);
    }
  }

  /// `api`'s request, each send's trace kept in `trace` if it has one. The device sends it again with the keys a node
  /// asks for (one-client's keyring, in net-worker.js, or the native app's); a request that doesn't reach a node is
  /// asked again here.
  async function sending(url, method, body, raw, signal, trace) {
    const user = auth.user();
    const joining = performance.now();
    if (!user || !(native ? nativeIn : await net.start(user).catch(() => false))) throw Object.assign(new NodeError("This device hasn't been approved yet."), { status: 403 });
    if (trace && performance.now() - joining >= 1) trace.network = Math.round(performance.now() - joining);
    if (trace && !native && (trace.network || trace.first)) trace.joined = await net.took();
    const traced = (json) => {
      const said = developer.parse(json);
      for (const f of Array.isArray(said) ? said : [said]) {
        trace.fetches.push(f);
        if (f.client) trace.keys++;
      }
    };
    for (let misses = 0; ; ) {
      const headers = { "x-one-user": user };
      if (trace) headers["x-one-trace"] = "1";
      if (body !== undefined) headers["content-type"] = "application/json";
      const until = auth.kept()?.until || Date.now() + 60_000;
      const sent = body === undefined ? undefined : JSON.stringify(body);
      let r;
      const watch = watching();
      try {
        r = await Promise.race([(native || net).fetch(method, url.pathname + url.search, headers, sent, signal, until, !!trace), watch.out]);
        if (trace) traced(r.trace);
      } catch (err) {
        if (trace) traced(err.trace);
        if (err.notLetIn) {
          if (await takenOut()) throw gone();
          throw Object.assign(new NodeError(err.message), { status: 403 });
        }
        if (err.removed || signal?.aborted || misses >= 5) throw err;
        // A request that fails may be one a browser taken out of its person's devices can't make.
        if (await takenOut()) throw gone();
        await sleep(retryWait(misses++));
        continue;
      } finally {
        watch.stop();
      }
      if (raw && r.ok) return r;
      const out = await r.json().catch(() => null);
      if (!r.ok || out?.error) throw Object.assign(new NodeError(out?.error || (r.status >= 500 ? UNANSWERED : "That didn’t work. Try again.")), { status: r.status, code: out?.code });
      return out;
    }
  }

  /// Developer mode, on this browser alone (Settings › Developer, for admins): each request asks its nodes for a trace
  /// (`x-one-trace`), kept here, the latest `KEPT`, for screens to show (`traces`, Developer's Timing).
  const developer = {
    KEY: "one.developer",
    KEPT: 100,
    kept: [],
    on: () => localStorage.getItem(developer.KEY) === "1",
    set(on) {
      if (on) localStorage.setItem(developer.KEY, "1");
      else (localStorage.removeItem(developer.KEY), (developer.kept = []));
    },
    /// What a request asked, as a trace names it: a call's app and tool, a screen's app and name, else its path.
    /// Never its input or query. `since` (`{ at, perf, first }`) is when what it's for began, if earlier: a screen's
    /// page waits for this browser to be let in, and the screen the window opened on from the page's start.
    start(path, body, since) {
      const what = path === "/v1/call" ? { what: "call", app: body?.app, tool: body?.tool }
        : path.startsWith("/v1/screens/") ? { what: "page", app: decodeURIComponent(path.split("/")[3] || ""), screen: decodeURIComponent(path.split("/")[4] || "") }
        : { what: "request", path };
      const now = performance.now();
      return { ...what, at: Math.round(since ? since.at : Date.now()), started: since ? since.perf : now, waited: since ? Math.round(now - since.perf) : 0, first: !!since?.first, keys: 0, fetches: [] };
    },
    /// When a screen's page was asked for: the one the window opened on, from the page's start; any other (one asked
    /// after the client's own, like Settings, where Developer mode is turned on) from when it was asked.
    firstScreen: true,
    since() {
      const first = developer.firstScreen;
      developer.firstScreen = false;
      return first ? { at: performance.timeOrigin, perf: 0, first } : { at: Date.now(), perf: performance.now() };
    },
    parse: (json) => {
      try {
        return json ? JSON.parse(json) : {};
      } catch {
        return {};
      }
    },
    keep(trace) {
      if (trace.started !== undefined) trace.ms = Math.round(performance.now() - trace.started);
      delete trace.started;
      developer.kept.push(trace);
      if (developer.kept.length > developer.KEPT) developer.kept.shift();
    },
  };

  const REMOVED = "This device was removed. Sign in again.";
  let removed = false;

  /// Whether this browser isn't one of its person's devices any more, as the relay says, which a browser's requests can
  /// wait on for a long time without an answer. It's signed out then, not an app's problem: the sign-in says why.
  async function takenOut() {
    if (removed) return true;
    if (native) {
      if (!(await native.removed().catch(() => false))) return false;
      removed = true;
      await auth.signOut(REMOVED);
      return true;
    }
    // Only the relay saying so: a client with no session yet (a tab that just took over the others') doesn't know.
    if ((await net.still().catch(() => null)) !== false) return false;
    removed = true;
    await forgetDevice();
    await auth.signOut(REMOVED);
    return true;
  }

  /// This browser's key goes: a removed key can't be let in again, so signing in makes a new one. With it, what it kept
  /// as that device: the nodes it asks, and what its keyring kept (one-client's `keyring::Keep`: the network it was let
  /// into, who's who and what each kind of request needed), which the next approval names again. As a native app's
  /// device forgets (one-client's `Device::sign_out`).
  async function forgetDevice() {
    events?.abort();
    const user = auth.user();
    if (user) await Promise.all(["device", "nodes", "root", "welcome", "who.json", "needs.json", "invites.json"].map((k) => del(`${k}:${user}`))).catch(() => {});
  }

  /// Signs out as a device: out of its person's devices first (removing itself, which its keyring does as `leave`), so signing in again replaces it, then its
  /// key goes. One that can't get out in a few seconds (no node answering) goes anyway, and stays listed, last seen when
  /// it signed out, until it's removed in Settings › Devices.
  async function signOut() {
    if (native) return auth.signOut();
    if (device) await Promise.race([api("/v1/remove", { method: "POST", body: { key: device.key } }), sleep(20_000)]).catch((err) => console.warn("leaving your devices:", err));
    await navigator.serviceWorker?.getRegistration().then((r) => r?.pushManager.getSubscription()).then((s) => s?.unsubscribe()).catch(() => {});
    localStorage.removeItem(PUSHED);
    await forgetDevice();
    return auth.signOut();
  }

  const gone = () => Object.assign(new NodeError(REMOVED), { signedOut: true, removed: true });

  /// Fails, once this browser is taken out, while a request it waits on is still going: asked after a few seconds, then
  /// every few more. `stop` once the request has answered.
  function watching() {
    let done = false;
    const out = new Promise((_, fail) => {
      (async () => {
        for (let wait = 3000; !done; wait = 5000) {
          await sleep(wait);
          if (!done && (await takenOut())) fail(gone());
        }
      })();
    });
    out.catch(() => {});
    return { out, stop: () => (done = true) };
  }

  /// A request to the address that isn't a node's, with the access token: auth's own routes.
  async function bearer(path, init = {}) {
    return fetch(path, { ...init, headers: { ...init.headers, authorization: `Bearer ${await auth.token()}` } });
  }

  /// This release's files by their content (`/web.js?v=…`), as the gateway's index.html names them: kept for good.
  const files = Object.fromEntries((document.querySelector('meta[name="one-files"]')?.content || "").split(" ").filter(Boolean).map((f) => f.split("=")));
  const asset = (name) => (files[name] ? `/${name}?v=${files[name]}` : `/${name}`);

  /// Phosphor's icons, from the address once (and then the browser's cache), for those the bridge asks for: on this
  /// page, and in screens' frames. Asked for last, so that on a first visit the client's WebAssembly, which the first
  /// answer from the person's machines waits for, gets the connection first.
  let icons;
  async function iconPaths(names = []) {
    icons ||= fetch(asset("icons.json"), { priority: "low" }).then((r) => (r.ok ? r.json() : Promise.reject(new Error("No icons.")))).catch((err) => ((icons = null), Promise.reject(err)));
    const all = await icons;
    return Object.fromEntries(names.map((n) => [n, all[n]]));
  }
  const copy = (text) => (native ? native.copy(String(text)) : navigator.clipboard.writeText(text));
  window.__one.host = async (m) => (m.type === "copy" ? copy(m.text) : m.type === "icons" ? iconPaths(m.names) : null);

  /// Names a call wherever it's sent, so sent again it runs once.
  const callId = () => Array.from(crypto.getRandomValues(new Uint8Array(8)), (b) => b.toString(16).padStart(2, "0")).join("");

  /// The node's state or apps, asked for once a second at most: the window and its own screens (Home, Settings) ask
  /// together as it opens, and share the answer.
  const recent = {};
  function shared(path) {
    if (recent[path] && Date.now() - recent[path].at < 1000) return recent[path].answer;
    const answer = api(path);
    recent[path] = { at: Date.now(), answer };
    answer.catch(() => delete recent[path]);
    return answer;
  }

  async function call(app, tool, input = {}) {
    const same = reads(app, tool) ? JSON.stringify([app, tool, input]) : null;
    if (same && reading.has(same)) return reading.get(same);
    const asked = api("/v1/call", { method: "POST", body: { app, tool, input, client: about?.client || "web" } }).then((r) => r.ok);
    if (same) (reading.set(same, asked), asked.catch(() => {}).finally(() => reading.delete(same)));
    const out = await asked;
    if (device) net.renew(app).catch((err) => console.warn(`renewing ${app}'s schedules:`, err));
    return out;
  }
  const reading = new Map();

  /// The sidebar's: the node's state, its apps and their badges. What's beside it is a screen the node serves.
  const store = { state: null, apps: [], badges: {} };

  const me = () => store.state?.user;
  const network = () => store.state?.network;
  // The custom build's name, as the gateway's index.html says it, until the network's state does.
  const customBuild = { name: document.querySelector('meta[name="application-name"]')?.content || "", cli: document.querySelector('meta[name="one-cli"]')?.content || "" };
  const config = () => store.state?.config || customBuild;
  const app = (name) => store.apps.find((a) => a.name === name);
  /// Whether an app's tool only reads, as its app says: only what such a call answered is kept (answers.js).
  const reads = (name, tool) => !!app(name)?.tools?.some((t) => t.name === tool && t.reads === true);
  const label = (a) => a?.title || a?.name || "";
  const hasScreens = (a) => !!a?.screens?.some((s) => s.name === "main");
  const keepsAccounts = (a) => !!a?.tools?.some((t) => t.accounts);
  const scopes = (a) => (a.added || []).map((s) => (s.startsWith("user:") ? "user" : s));

  /// Every page this person can read, as the sidebar lists them: the node's (`pages` in `/v1/state`).
  const pages = () => store.state?.pages || [];
  const page = (scope) => pages().find((p) => p.scope === scope);
  const appsOn = (p) => store.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
  const accountsOn = (p) => store.apps.filter((a) => keepsAccounts(a) && scopes(a).includes(p.scope)).sort((a, b) => label(a).localeCompare(label(b)));
  const appBadge = (name, p) => Math.max(0, store.badges[p.scope]?.[name] || 0);

  /// The window as it was last, drawn before the node answers: → whether there was one.
  function lastWindow() {
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

  /// Loads what the sidebar shows: the node's state and its apps together (its apps only if `apps`, or if who's in
  /// which group changed), drawn as soon as they're in; then the badges, drawn as they come.
  async function refreshState({ apps = false } = {}) {
    const [state, list] = await Promise.all([shared("/v1/state"), apps && !joining ? shared("/v1/apps").catch(() => null) : null]);
    // The nodes this browser asks itself, in the order things run on them, the Cloudflare node by its run priority
    // (last, unless an admin moved it).
    const nodes = (state.network?.devices || []).filter((d) => d.serves_me).sort((a, b) => a.order - b.order).map((d) => d.key);
    const last = state.network?.cloudflare_node?.key;
    if (last && !nodes.includes(last)) nodes.push(last);
    const rank = (p) => (p === "last" ? 0xffffffff : p);
    const ranks = Object.fromEntries((state.network?.devices || []).filter((d) => d.priority != null).map((d) => [d.key, rank(d.priority)]));
    if (last) ranks[last] = rank(state.network.cloudflare_node.priority ?? "last");
    nodes.sort((a, b) => (ranks[a] ?? 1) - (ranks[b] ?? 1));
    if (!native && state.user && nodes.length) net.nodes(state.user.id, nodes, ranks, last);
    const regrouped = JSON.stringify([state.user, state.network?.groups]) !== JSON.stringify([store.state?.user, store.state?.network?.groups]);
    store.state = state;
    // While it's being let in, its apps wait: they need the keys it's about to get.
    if (joining) return;
    if (list) store.apps = list;
    if (!list && (regrouped || !store.apps.length) && network()) return refreshApps();
    render();
    keepWindow();
    badges();
  }

  async function refreshApps() {
    if (!network()) return render();
    store.apps = await shared("/v1/apps");
    render();
    keepWindow();
    badges();
  }

  async function badges() {
    await Promise.all(pages().map(async (p) => (store.badges[p.scope] = await api("/v1/badges", { query: { scope: p.scope } }).catch(() => store.badges[p.scope] || {}))));
    render();
    keepWindow();
  }

  /// The node's events: what changed, so the window and its screens keep up, only while the page shows. A node with
  /// none to say (the Cloudflare node, while no machine is on) answers 204, and it asks again in a minute.
  let events = null;
  async function listen() {
    const mine = (events = new AbortController());
    for (let misses = 0; !mine.signal.aborted; ) {
      const started = Date.now();
      try {
        const r = await api("/v1/events", { raw: true, signal: mine.signal });
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
        if (err.signedOut) return err.removed ? undefined : showSignedOut();
      }
      if (Date.now() - started > 10_000) misses = 0;
      if (!mine.signal.aborted) await sleep(retryWait(misses++));
    }
  }
  addEventListener("pagehide", () => events?.abort());
  // Back to a page the browser kept: what changed while it was away, then its events again.
  addEventListener("pageshow", (e) => e.persisted && auth.kept() && (refreshState().catch(() => {}), listen()));

  async function handle(event) {
    for (const path in recent) delete recent[path];
    for (const f of frames) f.frame.contentWindow?.postMessage({ oneEvent: event }, "*");
    if (["devices", "network"].includes(event.type)) await refreshState();
    if (event.type === "changed") {
      // The app's own widgets (the home screen's, the Widgets board's, the menu bar's) are drawn again too.
      native?.changed();
      if (!(event.apps || []).length) await refreshState();
      await refreshApps();
    }
  }

  /// The URL is the place: `/` (You), `/page/<scope>`, `/accounts` (every page's), `/settings`, `/settings/<screen>?params=`
  /// (a Settings screen), `/app/<app>/<screen>?params=&scope=`; and `/vault?id=&scope=`, a request for an item, as a sheet
  /// over You. Each is a screen: the client's own (`own`, by name), or an app's.
  function route(at = location) {
    const r = routeAt(at);
    // On a phone every page, Settings too, is the pages side by side, opened at that one.
    if (phone() && r.kind === "own" && r.name === "pages") return own("pages", { swipe: true, scope: JSON.parse(r.params).scope || "user" });
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
  const own = (name, params = {}) => ({ kind: "own", name, params: JSON.stringify(params) });

  function href(r) {
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
    const params = JSON.parse(r.params || "{}");
    if (r.kind === "own") return own("pages", { scope: ["accounts", "request"].includes(r.name) ? "user" : "settings" });
    return own("pages", { scope: r.scope || (settingsItems().some((i) => i.app === r.app) ? "settings" : "user") });
  }

  /// Goes somewhere: a sidebar row replaces where the window is; what a screen opens deeper than the sidebar is pushed
  /// on top, so Back returns to it.
  function go(r, { pushed = false } = {}) {
    // Pushed on a screen, the sidebar keeps what it had selected, and the bar offers Back.
    const state = pushed ? { back: true, selection: history.state?.selection || location.pathname + location.search } : null;
    history.pushState(state, "", href(r));
    render();
  }
  /// Back: to what came before in this tab, or for a screen opened straight, to the page it's under.
  function back() {
    if (history.state?.back) return history.back();
    history.replaceState(null, "", href(under(route())));
    render();
  }
  /// Settings' text size on an Android phone (`about.textScale`, how many times usual): the whole window zoomed by it, its
  /// screens' frames too, but never narrower than 320 pixels.
  function zoomed() {
    const scale = about?.textScale > 0 ? about.textScale : 1;
    document.documentElement.style.zoom = scale === 1 ? "" : Math.min(scale, innerWidth / 320);
  }

  /// A phone's hardware Back: what's over the window (a sheet, an alert, a menu) closes first, as Escape does; then a
  /// pushed screen goes back, and so does one a person has gone deeper than the home screen. → whether it took it: if not,
  /// the app leaves.
  function hardwareBack() {
    if ($("[role=alertdialog], [role=menu]") || (shaded && $(".shade.open"))) return document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true })), true;
    const r = route();
    if (history.state?.back || (phone() && !(r.kind === "own" && r.name === "pages"))) return back(), true;
    return false;
  }
  /// The sidebar row selected: where the window is, or what a pushed screen was opened from.
  const selection = () => history.state?.selection || location.pathname + location.search;
  addEventListener("popstate", () => render());

  const root = $("#root");
  /// What #root shows: the window, or what stands before it (signing in, waiting to be let in, a problem). Drawn at
  /// once, so what's in it is there for what comes next (a screen's frame going into the window).
  const view = OneUI["react-dom/client"].createRoot(root);
  const show = (element) => OneUI["react-dom"].flushSync(() => view.render(element));
  const message = (title, text, note) => h(ui.Unavailable, { title, text, note });

  /// While this browser waits to be approved, the window isn't drawn: its join screen is.
  let joining = false;
  /// Settled once this browser has been let in as its person's device, or needn't be: what screens ask waits for it,
  /// since only a device hands a node the keys it needs.
  const joined = (() => {
    let done;
    const p = new Promise((r) => (done = r));
    return Object.assign(p, { done });
  })();

  /// What's beside the sidebar, which this page keeps (the screen's frame, and Developer's panel under it) while the
  /// window draws around it; the bar's title over it, as the screen in it last said.
  const detail = Object.assign(document.createElement("div"), { className: "detail" });
  let barTitle = "";
  /// Whether the window is what shows, rather than what stands before it.
  let windowShown = false;
  let shown = "";

  function render() {
    if (!store.state || joining) return;
    if (!network()) return void ((windowShown = false), show(message("Not Set Up Yet", "Set it up on one of your computers first.")));
    if (panel) return drawPanel();
    const r = route(), key = JSON.stringify(r);
    // The screen where the window is, kept as it is while the rest redraws around it.
    const fresh = !(shown === key && frames.some((f) => !f.sheet && f.into.isConnected));
    if (fresh) (shown = key), (barTitle = r.kind === "own" && r.name === "pages" ? "" : titleOf(r));
    drawWindow();
    if (fresh) screen.show(detail, r);
  }

  /// The panel: the network's name, the `panel` screen (your devices, your widgets), and the app's rows under it, or with
  /// no one signed in here, Sign In… in the window. Each row closes it, as a menu's item does.
  function drawPanel() {
    const row = (label, run, checked) => h(ui.Button, { key: label, variant: "ghost", className: "w-full justify-between px-3 font-normal", onClick: run }, label, checked ? h(ui.Icon, { name: "check", className: "size-4" }) : null);
    const signedIn = !!store.state && !!network();
    const name = network()?.name || config().name;
    windowShown = true;
    if (panelWidget) {
      show(h("div", { className: "panel" }, signedIn ? h("div", { className: "panel-screen", ref: (el) => el && detail.parentElement !== el && el.append(detail) }) : h("p", { className: "panel-note" }, "Signed out")));
      if (signedIn && shown !== "panel") (shown = "panel"), screen.show(detail, own("panel", { here: about.node || "", widget: panelWidget, menubar: about.menubar }));
      return;
    }
    show(h("div", { className: "panel" },
      h("div", { className: "panel-head" }, h("img", { src: "/logo.png", alt: "" }), h("span", null, name)),
      signedIn ? h("div", { className: "panel-screen", ref: (el) => el && detail.parentElement !== el && el.append(detail) }) : h("p", { className: "panel-note" }, "Signed out"),
      h("div", { className: "panel-rows" },
        row(signedIn ? `Open ${config().name}` : "Sign In…", () => native.main()),
        about.login != null && row("Open at Login", async () => ((about.login = await native.login(!about.login)), drawPanel()), about.login),
        about.computerUse != null && row("Let Agents Use This Mac", async () => ((about.computerUse = await native.computerUse(!about.computerUse)), drawPanel()), about.computerUse),
        row("Quit", () => native.quit()))));
    if (signedIn && shown !== "panel") (shown = "panel"), screen.show(detail, own("panel", { here: about.node || "", menubar: about.menubar }));
  }

  /// Where a route is as a link the app opens in its window (`opened`), for the panel.
  function linkOf(r) {
    const q = new URLSearchParams();
    if (r.kind === "screen") {
      for (const [k, v] of Object.entries({ app: r.app, screen: r.screen, params: r.params, scope: r.scope })) if (v) q.set(k, v);
      return `${about.cli}://open?${q}`;
    }
    const params = JSON.parse(r.params || "{}");
    if (r.name === "pages") return params.scope === "settings" ? `${about.cli}://settings` : `${about.cli}://open?scope=${encodeURIComponent(params.scope || "user")}`;
    if (r.name === "accounts") return `${about.cli}://page?folder=accounts`;
    return `${about.cli}://settings/${encodeURIComponent(r.name)}`;
  }

  /// The sidebar's open or closed on a computer's width, as the person left it.
  const SIDEBAR = "one.sidebar";
  /// Where each of the sidebar's links goes, as the window last drew them.
  const links = new Map();

  /// The sidebar's badges, every one, as the native app counts them on its icon: said as the window is first drawn,
  /// then only when it changes.
  let badged = -1;
  function countBadges(drawn) {
    const total = drawn.flatMap((s) => s.items).reduce((n, i) => n + Math.max(0, i.badge || 0), 0);
    if (total !== badged) native?.badge?.((badged = total));
  }

  function drawWindow() {
    windowShown = true;
    const drawn = sections();
    countBadges(drawn);
    if (phone()) {
      const r = route();
      return void show(h(ui.Shell, { phone: true, sections: [], title: barTitle, back: !(r.kind === "own" && r.name === "pages"), onBack: back, screen: detail, banner: offline.banner() }));
    }
    show(h(ui.Shell, {
      sections: drawn,
      title: barTitle,
      back: !!history.state?.back,
      onBack: back,
      onGo: (to) => links.has(to) && go(links.get(to)),
      developer: developer.on(),
      onDeveloper: () => devtools.toggle(),
      open: localStorage.getItem(SIDEBAR) !== "closed",
      onOpenChange: (open) => (open ? localStorage.removeItem(SIDEBAR) : localStorage.setItem(SIDEBAR, "closed"), drawWindow()),
      screen: detail,
      banner: offline.banner(),
    }));
  }

  /// One section per page, listing its apps and its Accounts; then Settings', listing its items. An app Settings lists
  /// (the App Store) shows there only.
  function sections() {
    const at = selection();
    links.clear();
    const link = (to) => (links.set(href(to), to), href(to));
    const item = (to, key, title, more) => ({ key, title, href: link(to), selected: href(to) === at, ...more });
    const section = (to, key, title, items) => ({ key, title, href: link(to), selected: href(to) === at, items });
    const listed = (a) => settingsItems().some((i) => i.app === a.name);
    // Accounts once, under You: every page's, listed together.
    const accounts = [...new Map(pages().flatMap(accountsOn).map((a) => [a.name, a])).values()];
    const out = pages().map((p, i) => section(own("pages", { scope: p.scope }), p.scope, p.title, [
      ...appsOn(p).filter((a) => !listed(a)).map((a) => item({ kind: "screen", app: a.name, screen: "main", params: "{}", scope: p.scope }, a.name, label(a), { app: a, badge: appBadge(a.name, p) })),
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

  /// Developer, in Developer mode: the bar's </> opens its panel (src/screen/settings/devtools.jsx) for the screen in front,
  /// docked under it, where it stays open from screen to screen until closed. It's told the screen's app and title, and
  /// for the client's own, since when it showed.
  const devtools = {
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

  /// Every screen showing: the one beside the sidebar, and the sheets over it, each an app's or the client's own, in a
  /// sandboxed frame whose bridge this page answers: an app's acting only as that app, and only the client's own as the
  /// person.
  let frames = [];

  /// What this browser keeps of the screens it showed, until it signs out: each route's page as the node last gave it
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

  /// One of the client's own screens' pages, which act as the person, so never a node's: the address's
  /// `screens/<name>.html` (src/screen.rs's `client_template`), with a nonce of its own and its params filled in.
  async function ownPage(r) {
    const resp = await fetch(asset(`screens/${files[`screens/${r.name}.html`] ? r.name : "missing"}.html`));
    if (!resp.ok) throw new Error("This didn’t load. Try again.");
    const nonce = Array.from(crypto.getRandomValues(new Uint8Array(16)), (x) => x.toString(16).padStart(2, "0")).join("");
    const params = JSON.stringify(JSON.parse(r.params || "{}")).replaceAll("</", "<\\/");
    return (await resp.text()).replaceAll("__nonce__", nonce).replace('"__params__"', () => params);
  }
  const ask = (r) => (asked = { key: JSON.stringify(r), page: screenPage(r) }).page.catch(() => {});

  /// A page that links @one/ui and the bridge (`linked`, src/screen.rs: an app's as its node gives it, and the client's
  /// own), with them written in from this window's cache, where its own page has them: a sandboxed frame's cache is its
  /// alone, so it would download them again for every screen. Each that can't be had here stays linked.
  const LINKED = /<link rel="stylesheet" href="(\/screen\/[a-z/]+\.css\?v=[0-9a-f]+)">|<script( nonce="[0-9a-f]+")? src="(\/screen\/[a-z/]+\.js\?v=[0-9a-f]+)"><\/script>/g;
  const linkedFiles = {};
  const linkedFile = (url) =>
    (linkedFiles[url] ||= fetch(url).then((r) => (r.ok ? r.text() : Promise.reject(new Error(UNANSWERED)))).catch((err) => (delete linkedFiles[url], Promise.reject(err))));
  async function linkedIn(page) {
    const urls = [...new Set(Array.from(page.matchAll(LINKED), (m) => m[1] || m[3]))];
    if (!urls.length) return page;
    const texts = Object.fromEntries(await Promise.all(urls.map((url) => linkedFile(url).then((text) => [url, text], () => [url, null]))));
    return page.replace(LINKED, (whole, css, nonce = "", js) => (texts[css || js] == null ? whole : css ? `<style>${texts[css]}</style>` : `<script${nonce}>${texts[js]}</script>`));
  }

  /// A screen that couldn't open: the problem, where it would be.
  const failed = (into, text) => OneUI["react-dom/client"].createRoot(into).render(message("Can’t Open This", text));

  /// A sandboxed frame for a page this window writes in (frame.html takes it).
  function sandboxed(r) {
    const frame = document.createElement("iframe");
    frame.className = "screen-frame";
    frame.setAttribute("sandbox", "allow-scripts allow-forms");
    frame.setAttribute("referrerpolicy", "no-referrer");
    frame.src = asset("frame.html");
    frame.title = titleOf(r);
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
    const written = (page) => linkedIn(page).catch(() => page).then((html) => oneHost.look(html, appearance(), !!sheet, shellOf()));
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

  /// While the person's devices don't answer, what the window shows from what it kept, as one banner across its top
  /// every screen showing its kept page, and the window's state, each with how it's asked again (`retry`) and
  /// when what it shows was kept. Asked again by itself, and at Try Now; the banner goes once all have answered.
  const offline = {
    waiting: new Map(),
    shown: false,
    trying: false,
    timer: null,
    misses: 1,
    add(key, retry, at) {
      if (this.waiting.has(key)) return;
      this.waiting.set(key, { retry, at: at || Date.now() });
      setTimeout(() => this.waiting.size && !this.shown && ((this.shown = true), redraw()), 1500);
      if (!this.timer) this.later();
    },
    done(key) {
      if (!this.waiting.delete(key) || this.waiting.size) return;
      clearTimeout(this.timer);
      Object.assign(this, { timer: null, misses: 1, shown: false, trying: false });
      redraw();
    },
    later() {
      clearTimeout(this.timer);
      this.timer = setTimeout(() => this.now(), Math.max(5000, retryWait(this.misses++)));
    },
    async now() {
      clearTimeout(this.timer);
      this.trying = true;
      redraw();
      await Promise.all([...this.waiting].map(([key, w]) => w.retry().then(() => this.done(key), () => {})));
      this.trying = false;
      if (this.waiting.size) (this.later(), redraw());
    },
    /// The banner, once it's been a moment: one problem box, quiet, as it mends itself.
    banner() {
      if (!this.shown || !this.waiting.size) return null;
      const at = new Date(Math.min(...[...this.waiting.values()].map((w) => w.at)));
      const today = at.toDateString() === new Date().toDateString();
      const when = new Intl.DateTimeFormat(undefined, today ? { timeStyle: "short" } : { dateStyle: "medium", timeStyle: "short" }).format(at);
      return h(ui.Problem, {
        quiet: true, title: "Offline", text: `Showing what this device kept from ${when}.`,
        action: this.trying ? { label: "Trying…", run: () => {} } : { label: "Try Now", run: () => this.now() },
        className: "max-w-none rounded-none border-0 border-b",
      });
    },
  };
  const redraw = () => windowShown && !panel && drawWindow();

  /// A screen's still, in a frame of its own as sandboxed as the screen's, answering nothing but its frame's ask for it.
  function showStill(f, html) {
    if (!f.into.isConnected) return;
    const frame = sandboxed(f.route);
    frame.classList.add("screen-still");
    frame.setAttribute("aria-hidden", "true");
    const scrolled = `<script>addEventListener("load", () => scrollTo(0, Number(document.documentElement.dataset.stillY) || 0))</script>`;
    f.still = { frame, html: oneHost.look(html.replace("</head>", `${scrolled}</head>`), appearance(), false, shellOf()) };
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

  function titleOf(r) {
    if (r.kind === "own") {
      const params = JSON.parse(r.params || "{}");
      if (r.name === "pages") return params.scope === "settings" ? "Settings" : page(params.scope || "user")?.title || "";
      if (r.name === "accounts") return "Accounts";
      if (r.name === "request") return "Vault";
      if (r.name === "devtools") return "Developer";
      return settingTitle(r.name);
    }
    const a = app(r.app);
    return a?.screens?.find((s) => s.name === r.screen)?.title || label(a) || r.app;
  }

  const screen = {
    show(detail, r) {
      frames = frames.filter((f) => f.sheet);
      const title = titleOf(r);
      document.title = title;
      detail.innerHTML = `<div class="screen"></div>`;
      devtools.since = Date.now();
      devtools.draw();
      // A Settings item that's a page of this site (Passkeys) is that page, framed: it's the gateway's, not a node's.
      const item = r.kind === "own" && settingsItems().find((i) => i.name === r.name && i.url && here(i.url));
      if (item) return void ($(".screen", detail).innerHTML = `<iframe class="screen-frame" src="${esc(item.url)}" title="${esc(title)}" allow="publickey-credentials-get; publickey-credentials-create"></iframe>`);
      frameFor(r, $(".screen", detail));
    },
  };

  /// A sheet a screen opens in its frame dims the rest of the window too, as a sheet dims a Mac window: the same shade
  /// as the sheet's own, around the frame. A click on it, or Escape, closes the sheet, as one on its backdrop does.
  let shaded = null;
  function shade(f, on) {
    const el = $(".shade") || Object.assign(document.createElement("div"), { className: "shade" });
    if (!on) {
      if (shaded === f) (shaded = null), el.classList.remove("open");
      return;
    }
    // Over the sheets already open (the frame's own, if it's in one), under any opened after it (an alert).
    if (shaded !== f) document.body.append(el);
    shaded = f;
    place();
    void el.offsetHeight;
    el.classList.add("open");
  }
  function place() {
    const el = $(".shade");
    if (!shaded || !el) return;
    if (!shaded.frame.isConnected) return shade(shaded, false);
    const { left: l, top: t, right: r, bottom: b } = shaded.frame.getBoundingClientRect();
    el.style.clipPath = `polygon(evenodd, 0 0, 100% 0, 100% 100%, 0 100%, 0 0, ${l}px ${t}px, ${r}px ${t}px, ${r}px ${b}px, ${l}px ${b}px, ${l}px ${t}px)`;
  }
  addEventListener("resize", place);
  addEventListener("click", (e) => e.target.classList?.contains("shade") && shaded?.frame.contentWindow?.postMessage({ oneSheetsClose: true }, "*"));
  // Not while what's over it (an alert, a menu) takes the Escape itself.
  addEventListener("keydown", (e) => e.key === "Escape" && shaded && $(".shade.open") && !$("[role=alertdialog], [role=menu]") && shaded.frame.contentWindow?.postMessage({ oneSheetsClose: true }, "*"));

  /// A screen over the window, in a sheet: → what it gave `one.close` once it's gone, else null.
  function openSheet(r) {
    return new Promise((done) => {
      let value = null;
      const sheet = ui.showSheet({ title: titleOf(r), done: r.name === "request" ? "Cancel" : "Done" });
      const f = frameFor(r, sheet.body, { sheet: { close: (v) => ((value = v ?? null), sheet.close()), title: sheet.setTitle } });
      sheet.closed.then(() => ((frames = frames.filter((x) => x !== f)), done(value)));
    });
  }

  /// The bridge's messages (src/screen/bridge.js), from the screens' frames only, each acting as its screen's app; and
  /// the client's own screens', as the person, which no app's may.
  addEventListener("message", async (e) => {
    const still = frames.find((x) => x.still?.frame.contentWindow === e.source)?.still;
    if (still && e.data?.oneFrame) return void still.frame.contentWindow.postMessage({ oneScreen: still.html }, "*");
    const f = frames.find((x) => x.frame.contentWindow === e.source);
    if (!f) return;
    const r = f.route, mine = r.kind === "own";
    if (e.data?.oneFrame) return void f.frame.contentWindow.postMessage({ oneScreen: f.html }, "*");
    if (typeof e.data?.oneSheets === "number") return void shade(f, e.data.oneSheets > 0);
    // The panel's window fits its screen, up to most of the screen.
    if (typeof e.data?.oneHeight === "number") {
      if (panel && !f.sheet) native.fit(Math.min(e.data.oneHeight, window.screen.availHeight - 80) + ($(".panel-head")?.offsetHeight || 0) + ($(".panel-rows")?.offsetHeight || 0) + 2);
      return;
    }
    if (e.data?.oneStill !== undefined) return void (f.sheet || kept.put("still", f.key, e.data.oneStill));
    const { one: m, id } = e.data || {};
    if (!m || typeof m !== "object") return;
    // What a call answered last time, if it only reads: from this browser, at once, while the window still starts.
    if (m.type === "kept") {
      const tool = String(m.tool || "");
      const got = mine ? null : await (native || net).kept(auth.user(), r.app, tool, m.input ?? {}).catch(() => null);
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
          return call(r.app, tool, input);
        },
        open: (m) => {
          const params = JSON.stringify(m.params || {});
          // Any screen may send its person to choose the device that runs its app's schedules, in Settings › Devices.
          // An app's says only which app's schedules, and where: no code to approve.
          const to = m.app === "settings" && m.screen === "devices" ? { kind: "own", name: "devices", params: mine ? params : JSON.stringify({ schedules: m.params?.schedules, scope: m.params?.scope }) }
            : typeof m.app === "string" && m.app ? { kind: "screen", app: m.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope }
            : mine ? { kind: "own", name: String(m.screen || ""), params }
            : { kind: "screen", app: r.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope };
          if (panel) return void native.main(linkOf(to));
          if (m.sheet) return openSheet(to);
          // From a sheet, the sheet goes and it opens where the window is. What the sidebar lists (an app on a page,
          // Backups in Settings) opens as if picked there, with no Back; only what's deeper than it is pushed.
          f.sheet?.close(null);
          go(to, { pushed: !links.has(href(to)) });
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
            if (windowShown && title !== barTitle) (barTitle = title), drawWindow();
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
          else if (["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) outside(url.href);
          return null;
        },
        icons: iconPaths,
        blob: async (id) => (await api(`/v1/blobs/${encodeURIComponent(id)}`, { raw: true })).blob(),
        ...(native ? { copy, give: (how, name, blob) => native.save(name, blob, how === "preview"), haptic: (style) => native.haptic(style) } : {}),
        // An iPhone's document camera; and its menu for what a touch and hold lifted, shown with it.
        ...(about?.scan ? { scan: () => native.scan() } : {}),
        ...(about?.lifts ? { menu: async (m) => ((r) => (r?.lifted === false ? oneHost.menu(f.frame, m) : r?.index ?? null))(await native.menu(Array.isArray(m.items) ? m.items : [])) } : {}),
        traces: () => (developer.on() ? developer.kept : null),
        own: mine && {
          developer: (m) => ("on" in m && developer.set(!!m.on), developer.on()),
          api: clientApi,
          signOut: () => (signOut(), null),
          appearance: (m) => (m.value != null && setAppearance(String(m.value)), appearance()),
          // A Mac's widgets' own menu bar items: shown again, or hidden, from its panel.
          menubar: async (m) => (native && about.menubar ? (about.menubar = await native.menubar(String(m.kind || ""), !!m.show)) : []),
          device: () => (native ? { name: about.device, signOut: true, keys: true, notify: nativeNotify } : { name: browserName(), signOut: true, keys: !!device, notify: notifyState(), ...(device ? { key: device.key } : {}) }),
          notify: () => notifications(true),
          request: async (m) => (await openSheet(own("request", { scope: String(m.scope || ""), id: String(m.id || "") }))) === true,
          cache: (m) => {
            const key = SCREENS + String(m.key || "");
            if (!("value" in m)) return JSON.parse(localStorage.getItem(key) || "null");
            try {
              localStorage.setItem(key, JSON.stringify(m.value));
            } catch {}
            return null;
          },
        },
      }));
    } catch (err) {
      fail(err);
    }
  });

  /// What Settings shows this person, in order, as the node says (`settings_items`).
  const settingsItems = () => withWaiting(store.state)?.settings_items || [];
  /// Whether a url is this site's: a page the gateway serves, shown in place rather than in a new tab.
  const here = (url) => new URL(url, location.href).origin === location.origin;
  /// The Settings item whose url is `url` on this site, if one is.
  const settingAt = (url) => settingsItems().find((i) => i.url && here(i.url) && new URL(i.url, location.href).pathname === new URL(url, location.href).pathname);
  const settingTitle = (name) => settingsItems().find((i) => i.name === name)?.title || "Settings";

  /// `one.api` for a client's own screen: the node's /v1 at this origin, as this person; nothing else here. What
  /// changes who can do what, and letting devices in, this device answers itself, as its keyring signs it
  /// (one-client's `settings`), at the same paths. A passkey to list is this tab's: WebAuthn's PRF comes only to a page.
  async function clientApi(m) {
    const url = new URL(String(m.path || ""), location.origin);
    if (url.origin !== location.origin || !url.pathname.startsWith("/v1/")) throw new Error("That isn’t on your devices.");
    const method = String(m.method || "GET").toUpperCase();
    let body = method === "GET" ? undefined : m.body ?? {};
    if (url.pathname === "/v1/call" && body && typeof body === "object") body.client = about?.client || "web";
    if (url.pathname === "/v1/devices/passkey") {
      const p = passkeyToList();
      if (method === "GET") return { passkey: !!p };
      if (!p) throw new Error("Sign in with your passkey first.");
      body = { prf: b64u(p.prf), credential: p.credential };
    }
    if (method === "GET" && !url.search && url.pathname === "/v1/state") return withWaiting(await shared(url.pathname));
    if (method === "GET" && !url.search && url.pathname === "/v1/apps") return shared(url.pathname);
    const out = await api(url.pathname + url.search, { method, body });
    if (method !== "GET") {
      if (url.pathname === "/v1/devices/passkey") sessionStorage.removeItem("one.prf");
      if (url.pathname === "/v1/devices/refuse") (notMine.add(String(body?.code || "").toUpperCase().replace(/[^A-Z2-7]/g, "")), localStorage.setItem(NOT_MINE, JSON.stringify([...notMine].slice(-50))));
      await refreshState().catch(() => {});
    }
    return out;
  }

  /// Devices waiting that this browser may let in, as last asked: Settings › Devices has a badge for each, and the
  /// first time one is seen here, Devices opens to ask about it. Those its person said aren't theirs aren't asked again.
  let waiting = [];
  const NOT_MINE = "one.notMine";
  const notMine = new Set(JSON.parse(localStorage.getItem(NOT_MINE) || "[]"));
  const seen = new Set();
  let prompting = false;

  /// Asks auth which devices wait now, and tells the window and its screens if that changed. → them.
  async function waitingNow() {
    const now = (await api("/v1/devices/requests")).map((r) => ({ ...r, key: r.code.replace(" ", "") })).filter((r) => !notMine.has(r.key));
    const changed = now.map((r) => r.key).join() !== waiting.map((r) => r.key).join();
    waiting = now;
    if (changed) {
      render();
      for (const f of frames) f.frame.contentWindow?.postMessage({ oneEvent: { type: "devices" } }, "*");
    }
    const fresh = now.filter((r) => !seen.has(r.key));
    fresh.forEach((r) => seen.add(r.key));
    if (fresh.length && !prompting && !(route().kind === "own" && route().name === "devices")) {
      prompting = true;
      openSheet(own("devices")).finally(() => (prompting = false));
    }
    return now;
  }

  async function watchWaiting() {
    for (;;) {
      if (document.visibilityState === "visible") await waitingNow().catch(() => {});
      await sleep(10_000);
    }
  }

  /// The state with a badge on Settings › Devices for each device waiting.
  function withWaiting(state) {
    if (!state || !waiting.length) return state;
    const items = (state.settings_items || []).map((i) => (i.name === "devices" ? { ...i, badge: (i.badge || 0) + waiting.length } : i));
    return { ...state, settings_items: items };
  }

  /// The passkey this tab signed in with, if it isn't listed yet: its PRF output stays in this tab only.
  function passkeyToList() {
    const prf = sessionStorage.getItem("one.prf"), credential = sessionStorage.getItem("one.prf.credential");
    if (!prf || !credential || network()?.clients.some((c) => c.passkey === credential)) return null;
    return { prf: Uint8Array.from(atob(prf.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)), credential };
  }

  const LOOK = "one.appearance";
  const appearance = () => (["light", "dark"].includes(localStorage.getItem(LOOK)) ? localStorage.getItem(LOOK) : "automatic");

  function setAppearance(value) {
    if (value === "automatic") localStorage.removeItem(LOOK);
    else if (["light", "dark"].includes(value)) localStorage.setItem(LOOK, value);
    schemed(document, appearance());
    for (const f of frames) f.frame.contentWindow?.postMessage({ oneScheme: appearance() }, "*");
    native?.appearance?.(appearance());
  }

  /// What a screen's frame is told this page does beyond a browser's (oneHost.look): an iPhone's document camera, and
  /// its touch and hold, which lifts what a screen marks `data-one-held` with its menu.
  const shellOf = () => ({ ...(about?.scan ? { __oneCan: { scan: true } } : {}), ...(about?.lifts ? { __oneLifts: true } : {}) });

  const schemed = oneHost.scheme();

  function showSignedOut(error, note) {
    if (panel) return drawPanel();
    const name = config().name;
    document.title = name;
    windowShown = false;
    // The app asks where first: an address, or an invite link to one.
    if (native && !about.address) {
      const take = async (typed) => {
        try {
          await native.address(typed);
          about = await native.about();
          // An invite opens in the browser, where its person makes their passkey; then they sign in with it here.
          if (/\/invite#./.test(typed)) return (await native.open(new URL(typed.includes("://") ? typed : `https://${typed}`).href), showSignedOut(undefined, "Made your passkey? Sign in with it."));
          await auth.start();
        } catch (err) {
          showSignedOut(err.message);
        }
      };
      const whose = about.yours.charAt(0).toUpperCase() + about.yours.slice(1);
      const field = { label: "Address", placeholder: "one.example.com", note: `${whose}’s address, or the invite link you were sent.` };
      // A Mac whose own node is in no network yet joins one with a pairing code.
      const other = about.unpaired ? { label: "Join with a Pairing Code…", run: () => showPairing() } : undefined;
      return void show(h(ui.Welcome, { key: Math.random(), logo: "/logo.png", name, field, action: { label: "Sign In", invite: "Open Invite", run: take }, note: about.unpaired ? `This Mac isn’t part of ${about.yours} yet. Join it from another of your computers, with a pairing code.` : undefined, other, problem: error }));
    }
    const signIn = () => auth.start().catch((err) => showSignedOut(err.message));
    show(h(ui.Welcome, { key: Math.random(), logo: "/logo.png", name, action: { label: "Sign In", run: signIn }, note, problem: error }));
  }

  /// A Mac's own node joining a network with a pairing code another of its person's computers shows, then signing in
  /// to it here.
  function showPairing(error) {
    const join = async (code) => {
      try {
        await native.pair(code);
        about = await native.about();
        await auth.start();
      } catch (err) {
        showPairing(err.message);
      }
    };
    const field = { label: "Pairing Code", placeholder: "Pairing code", note: "Paste the pairing code another device shows. Each code works once." };
    show(h(ui.Welcome, { key: Math.random(), logo: "/logo.png", name: "Join with a Pairing Code", field, action: { label: "Join", run: join }, other: { label: "Cancel", run: () => showSignedOut() }, problem: error }));
  }

  /// Back from a sign-in at another site, in a browser that isn't signed in here: on to the app, which finishes it.
  function showBack(app, screen, params) {
    const { name, cli } = config();
    const link = `${cli}://open?${new URLSearchParams({ app, screen, params, scope: "user" })}`;
    document.title = name;
    show(h(ui.Welcome, { logo: "/logo.png", name, action: { label: `Open ${name}`, href: link } }));
    location.href = link;
  }

  function showUnreachable(text) {
    windowShown = false;
    show(message("Can’t Reach Your Devices", text, "Trying again…"));
  }

  /// Makes this browser a device of its person, before anything is asked of a node: one already, or let in by a passkey
  /// of theirs that's listed, or by one of their devices (or, for someone new, their inviter's), which it asks at
  /// auth and waits for there, showing its code. → its keys.
  async function join(user) {
    const keys = await browserKeys(user);
    if (await net.start(user).catch(() => false)) return keys;
    const invite = sessionStorage.getItem("one.invite");
    const ask = async () => answer(await bearer("/api/auth/devices/requests", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await joinAsk(keys, browserName(), invite)) }));
    await ask();
    if (await byPasskey(user, keys)) return keys;
    await showJoin(await codeOf(keys.key, keys.seal), !!invite);
    // Auth says once it was approved; then the relay lets it in. One gone from auth (approved and cleared, refused, or
    // waited 15 minutes) is asked again, unless it's in. As every client does (`waited`).
    for (;;) {
      await sleep(2000);
      const r = await bearer(`/api/auth/devices/requests/${keys.key}`).catch(() => null);
      const said = waited(r?.status || 0, r?.ok ? await r.json().catch(() => ({})) : {});
      if (said.signedOut) throw Object.assign(new NodeError("Signed out."), { signedOut: true });
      // The network it was let into, as the device that approved it said: the only one it then takes; and where that
      // device read who's who to, which it starts from.
      if (said.root) await put(`root:${user}`, said.root);
      if (said.welcome) await put(`welcome:${user}`, said.welcome);
      if ((said.gone || said.approved) && (await net.start(user, true).catch(() => false))) break;
      if (said.gone) await ask().catch(() => {});
    }
    sessionStorage.removeItem("one.invite");
    show(h(ui.Starting));
    return keys;
  }

  /// Lets this browser in by a passkey its person listed, whose PRF output this tab has from signing in: it asks as that
  /// passkey's device, which approves this browser's request by its code. → whether it's in.
  async function byPasskey(user, keys) {
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
    } finally {
      store.state = null;
    }
  }

  /// Once it's in: its person's own things' first key and where they run, made by the first of their devices.
  async function settle() {
    await net.settle().catch((err) => console.warn("making this person's own keys:", err));
    await refreshState().catch(() => {});
  }

  /// "Waiting for Approval": its code, and the code as a QR, a link that opens Approve with it in the browser of a
  /// phone that scans it.
  async function showJoin(code, invited) {
    document.title = "Waiting for Approval";
    const link = `${native ? about.address : location.origin}/approve#${code.replace(" ", "")}`;
    windowShown = false;
    show(h(ui.Join, {
      code,
      qr: qrSvg(link),
      text: invited ? "Whoever invited you approves it. Show them this code." : "Your other devices ask you to approve it. It shows this code there too.",
      note: invited ? "Or they scan this with their phone." : "Or scan this with your phone, to approve it in its browser.",
      onSignOut: () => signOut(),
    }));
  }

  /// The app's device let in, as `join` does a browser's: one already, or asking and showing its code until it's
  /// approved.
  let nativeIn = false;
  async function nativeJoin() {
    const asked = await native.join();
    if (!asked.in) {
      await showJoin(asked.code, asked.invited);
      await native.wait();
      show(h(ui.Starting));
    }
    nativeIn = true;
  }

  /// A link the app was opened with, there once this device is in:
  /// `<cli>://open?app=&screen=&params=&scope=` (an app's screen; with no app, the page `scope`), `<cli>://page?scope=`
  /// (`&folder=accounts`, its Accounts), `<cli>://settings[/<item>]`, `<cli>://vault?id=&scope=` (a request, as a sheet),
  /// `<cli>://signed-in` (the window in front, which the app does), and a link to the address's own pages.
  function opened(link) {
    let url;
    try {
      url = new URL(link);
    } catch {
      return;
    }
    const q = url.searchParams, part = url.pathname.split("/").filter(Boolean)[0];
    const sheet = (scope, id) => scope && id && joined.then(() => openSheet(own("request", { scope, id })));
    let to = null;
    if (url.protocol === `${about.cli}:`) {
      if (url.host === "open") to = q.get("app") ? { kind: "screen", app: q.get("app"), screen: q.get("screen") || "main", params: q.get("params") || "{}", scope: q.get("scope") || undefined } : own("pages", { scope: q.get("scope") || "user" });
      else if (url.host === "page") to = q.get("folder") === "accounts" ? own("accounts") : own("pages", { scope: q.get("scope") || "user" });
      else if (url.host === "settings") to = part ? { kind: "own", name: part, params: JSON.stringify(Object.fromEntries(q)) } : own("pages", { scope: "settings" });
      else if (url.host === "vault") return void sheet(q.get("scope"), q.get("id"));
    } else if (about.address && url.origin === new URL(about.address).origin) {
      // An invite while no one's signed in here: as if typed on the welcome, opened in the browser for its passkey.
      if (url.pathname === "/invite" && url.hash.length > 1 && !about.signedIn) {
        return void native.address(link).then(async () => ((about = await native.about()), await native.open(link), showSignedOut(undefined, "Made your passkey? Sign in with it.")), (err) => showSignedOut(err.message));
      }
      if (url.pathname === "/vault") return void sheet(q.get("scope"), q.get("id"));
      to = route(url);
    }
    if (to) joined.then(() => go(to));
  }

  /// A link elsewhere, in a new tab, or in the app, the system's browser.
  const outside = (href) => (native ? native.open(href).catch(() => {}) : window.open(href, "_blank", "noopener,noreferrer"));
  // In the app, a link of this page's to another site (Settings' that are pages of the address) goes to the browser.
  if (native) {
    addEventListener("click", (e) => {
      const a = e.target.closest?.("a[href]");
      if (!a || new URL(a.href, location.href).origin === location.origin) return;
      e.preventDefault();
      outside(a.href);
    });
  }

  async function start() {
    schemed(document, appearance());
    document.documentElement.style.visibility = "";
    if (native) {
      about = await native.about();
      // The app draws its next start in it from the first frame.
      native.appearance?.(appearance());
      native.onOpen = opened;
      native.onBack = hardwareBack;
      native.onScale = (scale) => ((about.textScale = scale), zoomed());
      zoomed();
      // The app menu's commands (a Mac's), which work wherever the focus is: the sidebar folded or unfolded, as its
      // own key does here; joining a network with a pairing code, while this Mac's node is in none.
      native.onCommand = (name) => {
        if (name === "sidebar") dispatchEvent(new KeyboardEvent("keydown", { key: "\\", metaKey: true }));
        else if (name === "join" && about.unpaired && !about.signedIn) showPairing();
      };
      native.appearance?.(appearance());
      if (about.opened) opened(about.opened);
    }
    // A new device's QR: Approve, with its code.
    if (location.pathname === "/approve") history.replaceState(null, "", href(own("devices", { approve: location.hash.slice(1) })));
    // `/back?state=<app>/<screen>[/<the app's own>]&…`: where a sign-in at another site sends the person back (an app's
    // `ctx.address`).
    // It opens that app's screen with the whole address as `params.back`, which finishes it. A browser that isn't
    // signed in here passes it on to the app on this phone or computer, which opens the same screen from `<cli>://open`.
    if (location.pathname === "/back") {
      const [, app, screen = "main"] = /^([a-z0-9][a-z0-9-]*)(?:\/([a-z0-9_-]+)(?:\/[A-Za-z0-9_-]*)?)?$/.exec(new URLSearchParams(location.search).get("state") || "") || [];
      const params = JSON.stringify({ back: location.href });
      if (!app) history.replaceState(null, "", "/");
      else if (auth.kept()) history.replaceState(null, "", href({ kind: "screen", app, screen, params }));
      else return showBack(app, screen, params);
    }
    if (location.pathname === "/callback") {
      try {
        history.replaceState(null, "", await auth.finish());
      } catch (err) {
        history.replaceState(null, "", "/");
        return showSignedOut(err.message);
      }
    }
    if (!auth.kept()) return showSignedOut();
    if (!native) navigator.serviceWorker?.register("/sw.js", { type: "module" }).catch(() => {});
    // `/vault?id=…&scope=…`: an agent's link to a request for an item, answered over You.
    let request = null;
    if (location.pathname === "/vault") {
      const q = new URLSearchParams(location.search);
      request = { scope: q.get("scope"), id: q.get("id") };
      history.replaceState(null, "", "/");
    }
    // Everything the window opens on, asked for at once: its screen, beside the node's state and apps; meanwhile the
    // window as it was last. Its screens' asks wait until this browser is its person's device (`joined`).
    ask(route());
    if (lastWindow()) render();
    else show(h(ui.Starting));
    joining = true;
    const user = auth.user();
    try {
      // A device from the start, as a node may need the keys it hands (the Cloudflare node, even for the state).
      if (native) await nativeJoin();
      else {
        device = await join(user);
      }
    } catch (err) {
      if (err.signedOut) return err.removed ? undefined : showSignedOut();
      return void show(message("Can’t Let This Device In", err.message));
    }
    // A device that takes the request and never answers (asleep, stuck) is one that can't be reached, while it's still asked.
    const away = setTimeout(() => windowShown || showUnreachable(UNANSWERED), 15000);
    for (let misses = 1; ; misses++) {
      try {
        await refreshState();
        offline.done("state");
        break;
      } catch (err) {
        if (err.signedOut) return clearTimeout(away), err.removed ? undefined : showSignedOut();
        // What it showed last stays, as a screen whose node is restarting does, saying so.
        if (!windowShown) showUnreachable(err instanceof NodeError ? err.message : UNANSWERED);
        else offline.add("state", () => refreshState(), Number(localStorage.getItem(`${LAST}.at`)) || null);
        await sleep(retryWait(misses));
      }
    }
    clearTimeout(away);
    if (!native && network() && me()) await settle();
    joining = false;
    joined.done();
    await refreshApps().catch(() => {});
    if (request?.id && request.scope) openSheet(own("request", { scope: request.scope, id: request.id }));
    listen();
    if (panel) return;
    watchWaiting();
    notifications(false).catch((err) => console.warn("notifications:", err));
  }

  /// What was last registered for pushes here: the subscription's endpoint, the device's key and the network's VAPID
  /// key, and when.
  const PUSHED = "one.pushed";
  /// Whether the app may show notifications, as it last said.
  let nativeNotify = null;

  /// Whether this browser can get notifications: "default" until the person says, "granted", "denied", or null where
  /// it can't.
  const notifyState = () => (device && "PushManager" in window && "Notification" in window ? Notification.permission : null);

  /// This browser gets notifications once its person allows them (`ask`, from Settings › Devices): its push
  /// subscription is registered with the push Worker as this device's, again whenever it changes and daily. → the
  /// permission, as `notifyState`.
  async function notifications(ask) {
    if (native) return (nativeNotify = await native.notify(ask).catch(() => null));
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
  }

  document.addEventListener("DOMContentLoaded", start);
})();
