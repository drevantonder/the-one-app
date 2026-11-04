// The web client: the Mac app's window, in a browser. It signs in as an OAuth client of the install's auth service
// (PKCE, and the person's passkey at its sign-in page), keeps its refresh token in this browser, and asks the person's nodes over iroh (`net.js`). App screens are untrusted: each runs in a sandboxed
// frame, and this page answers its bridge as the Mac app's web view does, making each call itself. It draws at once
// what it showed last (the sidebar, and a still of the screen) and then what the node says now.

// The bridge and kit (src/screen/), loaded after this, give `one`; this page answers it for its own use (`__one.host`).
// This browser is a device of its person (device.js): it signs what changes who can do what, and hands each call the
// keys it needs.
import * as answers from "./answers.js";
import { del, put } from "./kept.js";
import { BEHIND, waited } from "../shared/crypto.js";
import { answer, browserKeys, browserName, codeOf, Device, joinAsk, passkeyKeysOf } from "./device.js";
import { members } from "./members.js";
import { net } from "./net.js";
import { qrSvg } from "./qr.js";

window.__one = {};

(() => {
  const $ = (sel, el = document) => el.querySelector(sel);
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const b64u = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  const random = (n) => b64u(crypto.getRandomValues(new Uint8Array(n)));

  // MARK: Signing in

  const KEPT = "one.signin";
  // What the client's own screens keep between visits (one.cache), the window as it was last (LAST), the screens it
  // showed (KEPT_SCREENS, `kept`) and what their calls answered (answers.js): all forgotten with the sign-in.
  const SCREENS = "one.screens.";
  const LAST = "one.last.window";
  const KEPT_SCREENS = "one.screens";
  // What each kind of request needed (device.js), which another sign-in here needn't start from.
  const NEEDS = "one.needs";
  const forgetScreens = () => {
    Object.keys(localStorage).filter((k) => k.startsWith(SCREENS) || k === LAST || k === NEEDS).forEach((k) => localStorage.removeItem(k));
    window.caches?.delete(KEPT_SCREENS).catch(() => {});
    return answers.forget();
  };
  const CLIENT = "one.client";

  const auth = {
    kept: () => JSON.parse(localStorage.getItem(KEPT) || "null"),
    keep: (tokens) => localStorage.setItem(KEPT, JSON.stringify(tokens)),

    /// The auth service's endpoints, as it says them.
    async meta() {
      const r = await fetch("/.well-known/oauth-authorization-server");
      if (!r.ok) throw new Error("The sign-in service isn’t answering. Try again in a moment.");
      return r.json();
    },

    /// This browser, as a client of the auth service: registered the first time it signs in.
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

    /// Off to the auth service's sign-in, coming back to /callback, then to where this was.
    async start() {
      const meta = await auth.meta();
      const client = await auth.client(meta);
      const verifier = random(32), state = random(16);
      const challenge = b64u(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(verifier)));
      sessionStorage.setItem("one.pkce", JSON.stringify({ verifier, state, back: location.pathname + location.search }));
      const q = new URLSearchParams({
        response_type: "code", client_id: client, redirect_uri: `${location.origin}/callback`, scope: "openid profile offline_access",
        code_challenge: challenge, code_challenge_method: "S256", state,
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

    /// Who the access token says this is (its `sub`).
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

  // MARK: The node

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

  /// `api`'s request, each send's trace kept in `trace` if it has one.
  async function sending(url, method, body, raw, signal, trace) {
    const route = Device.route(url.pathname, body);
    const user = auth.user();
    const joining = performance.now();
    if (!user || !(await net.start(user).catch(() => false))) throw Object.assign(new NodeError("This device hasn't been approved yet."), { status: 403 });
    // How long it waited to join the network, if it did, and what that took.
    if (trace && performance.now() - joining >= 1) trace.network = Math.round(performance.now() - joining);
    if (trace && (trace.network || trace.first)) trace.joined = await net.took();
    for (let needed = 0, misses = 0, behind = 0; ; ) {
      const headers = { "x-one-user": user };
      if (trace) headers["x-one-trace"] = "1";
      if (body !== undefined) headers["content-type"] = "application/json";
      // Keys go only to the one node the request is sent to, which this browser reaches itself, as its connection
      // proves: sealed to it once the client picks it. One sent while another of its kind learns what it needs goes
      // with what that learned.
      await device?.learning.get(route)?.done.catch(() => {});
      const sentWith = device?.needs[route];
      const handing = performance.now(), opened = [];
      const until = auth.kept()?.until || Date.now() + 60_000;
      const keys = sentWith?.need?.length ? (node) => device.handoff(route, until, node, trace ? noting(handing, opened) : undefined) : undefined;
      const sent = body === undefined ? undefined : JSON.stringify(body);
      let r;
      const watch = watching();
      try {
        r = await Promise.race([net.fetch(method, url.pathname + url.search, headers, sent, signal, keys), watch.out]);
        trace?.fetches.push(developer.parse(r.trace));
      } catch (err) {
        trace?.fetches.push(developer.parse(err.trace));
        if (opened.length) trace.fetches.at(-1).handed = handedIn(opened);
        if (err.removed || signal?.aborted || misses >= 5) throw err;
        // A request that fails may be one a browser taken out of its person's devices can't make.
        if (await takenOut()) throw gone();
        await sleep(retryWait(misses++));
        continue;
      } finally {
        watch.stop();
      }
      // The keys it opened to hand with it, if it asked for any (after a reload, say).
      if (opened.length) trace.fetches.at(-1).handed = handedIn(opened);
      // A node that doesn't list this browser may not have read its approval yet: the relay says if it's out, else it's
      // asked again in a moment (`BEHIND`), as every client does.
      if (r.status === 403 && (await r.clone().json().catch(() => null))?.code === "not_let_in") {
        if (await takenOut()) throw gone();
        if (behind < BEHIND.length) {
          await sleep(BEHIND[behind++]);
          continue;
        }
        throw Object.assign(new NodeError("Your devices haven't caught up with this one yet. Try again in a moment."), { status: 403 });
      }
      // A node may find what it needs as it goes: the serverless node, its logs' keys first, then a call's.
      if (r.status === 428 && device && needed < 3) {
        needed++;
        if (trace) trace.keys = needed;
        // What this browser did before sending it again, each thing it asked for timed.
        const learning = performance.now(), steps = [];
        try {
          await device.learn(route, await r.json(), r.node, trace ? noting(learning, steps) : undefined, sentWith);
        } finally {
          const last = trace?.fetches.at(-1);
          if (last) last.client = { ms: Math.round(performance.now() - learning), steps };
        }
        continue;
      }
      if (raw && r.ok) return r;
      const out = await r.json().catch(() => null);
      if (!r.ok || out?.error) throw Object.assign(new NodeError(out?.error || (r.status >= 500 ? UNANSWERED : "That didn’t work. Try again.")), { status: r.status });
      return out;
    }
  }

  /// Times each thing `fetch` asks for, from `start`, into `steps` (Device's `note`).
  const noting = (start, steps) => async (step, fetch) => {
    const at = performance.now();
    try {
      return await fetch();
    } finally {
      steps.push({ ...step, at: Math.round(at - start), ms: Math.round(performance.now() - at) });
    }
  };

  /// The keys a request opened before it was sent, as its trace says it: how long, and each thing asked for.
  const handedIn = (steps) => ({ ms: Math.max(...steps.map((s) => s.at + s.ms)), steps });

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
    /// What a request asked, as a trace names it: a call's app and operation, a screen's app and name, else its path.
    /// Never its input or query. `since` (`{ at, perf, first }`) is when what it's for began, if earlier: a screen's
    /// page waits for this browser to be let in, and the screen the window opened on from the page's start.
    start(path, body, since) {
      const what = path === "/v1/call" ? { what: "call", app: body?.app, op: body?.op }
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
    if (await net.still().catch(() => true)) return false;
    removed = true;
    await forgetDevice();
    await auth.signOut(REMOVED);
    return true;
  }

  /// This browser's key goes: a removed key can't be let in again, so signing in makes a new one. With it, what it kept
  /// as that device: the nodes it asks, and the install it was let into and its entries, which the next approval names
  /// again. As a native app's device forgets (one-client's `Device::sign_out`).
  async function forgetDevice() {
    events?.abort();
    const user = auth.user();
    if (user) await Promise.all([`device:${user}`, `nodes:${user}`, `genesis:${user}`, `global:${user}`].map(del)).catch(() => {});
  }

  /// Signs out as a device: out of its person's devices first (`Device.leave`), so signing in again replaces it, then its
  /// key goes. One that can't get out in a few seconds (no node answering) goes anyway, and stays listed, last seen when
  /// it signed out, until it's removed in Settings › Devices.
  async function signOut() {
    if (device) await Promise.race([device.leave(), sleep(20_000)]).catch((err) => console.warn("leaving your devices:", err));
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
  window.__one.host = async (m) => (m.type === "copy" ? navigator.clipboard.writeText(m.text) : m.type === "icons" ? iconPaths(m.names) : null);

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

  /// Runs an app's operation as this person → its result.
  async function call(app, op, input = {}) {
    // A call that only reads, asked while the same is under way (a screen that started again as its page changed),
    // waits for that one's answer rather than running again.
    const same = reads(app, op) ? JSON.stringify([app, op, input]) : null;
    if (same && reading.has(same)) return reading.get(same);
    const asked = api("/v1/call", { method: "POST", body: { app, op, input, client: "web" } }).then((r) => r.ok);
    if (same) (reading.set(same, asked), asked.catch(() => {}).finally(() => reading.delete(same)));
    const out = await asked;
    // Using an app keeps the devices keeping it running for this person doing so.
    device?.renewGrants(app);
    return out;
  }
  const reading = new Map();

  // MARK: What the window shows

  /// The sidebar's: the node's state, its apps and their badges. What's beside it is a screen the node serves.
  const store = { state: null, apps: [], badges: {} };

  const me = () => store.state?.user;
  const install = () => store.state?.install;
  // The white label's name, as the gateway's index.html says it, until the install's state does.
  const whiteLabel = { name: document.querySelector('meta[name="application-name"]')?.content || "", cli: document.querySelector('meta[name="one-cli"]')?.content || "" };
  const config = () => store.state?.config || whiteLabel;
  const app = (name) => store.apps.find((a) => a.name === name);
  /// Whether an app's operation only reads, as its app says: only what such a call answered is kept (answers.js).
  const reads = (name, op) => !!app(name)?.ops?.some((o) => o.name === op && o.reads === true);
  const label = (a) => a?.title || a?.name || "";
  const hasScreens = (a) => !!a?.screens?.some((s) => s.name === "main");
  const isService = (a) => !hasScreens(a) && !(a.widgets || []).length;
  const scopes = (a) => (a.installed || []).map((s) => (s.startsWith("user:") ? "user" : s));

  /// Every page this person can read, as the sidebar lists them: theirs, their groups', then everyone's.
  function pages() {
    const i = install(), u = me();
    if (!i || !u) return [];
    const groups = i.groups.filter((g) => g.members.some((m) => m.user === u.id)).sort((a, b) => a.name.localeCompare(b.name));
    return [{ scope: "user", title: "You", whose: "For you only" }, ...groups.map((g) => ({ scope: `group:${g.id}`, title: g.name, whose: `For ${g.name}` })), { scope: "global", title: "Everyone", whose: "For everyone" }];
  }
  const page = (scope) => pages().find((p) => p.scope === scope);
  const appsOn = (p) => store.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
  const servicesOn = (p) => store.apps.filter((a) => isService(a) && scopes(a).includes(p.scope)).sort((a, b) => label(a).localeCompare(label(b)));
  const appBadge = (name, p) => Math.max(0, store.badges[p.scope]?.[name] || 0);

  /// The window as it was last, drawn before the node answers: → whether there was one.
  function lastWindow() {
    try {
      const kept = JSON.parse(localStorage.getItem(LAST) || "null");
      if (!kept?.state?.install) return false;
      Object.assign(store, kept);
      return true;
    } catch {
      return false;
    }
  }
  const keepWindow = () => {
    try {
      localStorage.setItem(LAST, JSON.stringify(store));
    } catch {}
  };

  /// Loads what the sidebar shows: the node's state and its apps together (its apps only if `apps`, or if who's in
  /// which group changed), drawn as soon as they're in; then the badges, drawn as they come.
  async function refreshState({ apps = false } = {}) {
    const [state, list] = await Promise.all([shared("/v1/state"), apps && !joining ? shared("/v1/apps").catch(() => null) : null]);
    // The nodes this browser asks itself, in the order things run on them, and the serverless node last.
    const nodes = (state.install?.devices || []).filter((d) => d.serves_me).sort((a, b) => a.order - b.order).map((d) => d.key);
    const last = state.install?.serverless?.key;
    if (last && !nodes.includes(last)) nodes.push(last);
    const rank = (p) => (p === "last" ? 0xffffffff : p);
    const ranks = Object.fromEntries((state.install?.devices || []).filter((d) => d.priority != null).map((d) => [d.key, rank(d.priority)]));
    if (state.user && nodes.length) net.nodes(state.user.id, nodes, ranks);
    const regrouped = JSON.stringify([state.user, state.install?.groups]) !== JSON.stringify([store.state?.user, store.state?.install?.groups]);
    store.state = state;
    // While it's being let in, its apps wait: they need the keys it's about to get.
    if (joining) return;
    if (list) store.apps = list;
    if (!list && (regrouped || !store.apps.length) && install()) return refreshApps();
    render();
    keepWindow();
    badges();
  }

  async function refreshApps() {
    if (!install()) return render();
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
  /// none to say (the serverless node, while no machine is on) answers 204, and it asks again in a minute.
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
      // At once after a stream that ran a while and ended (its node handing over to a new build), then waiting longer
      // each miss.
      if (Date.now() - started > 10_000) misses = 0;
      if (!mine.signal.aborted) await sleep(retryWait(misses++));
    }
  }
  addEventListener("pagehide", () => events?.abort());
  // Back to a page the browser kept: what changed while it was away, then its events again.
  addEventListener("pageshow", (e) => e.persisted && auth.kept() && (refreshState().catch(() => {}), listen()));

  async function handle(event) {
    // What changed is asked for again, never shared from before.
    for (const path in recent) delete recent[path];
    for (const f of frames) f.frame.contentWindow?.postMessage({ oneEvent: event }, "*");
    if (["devices", "install"].includes(event.type)) await refreshState();
    if (event.type === "changed") {
      if (!(event.apps || []).length) await refreshState();
      await refreshApps();
    }
  }

  // MARK: Where the window is

  /// The URL is the place: `/` (You), `/page/<scope>`, `/services/<scope>`, `/settings`, `/settings/<screen>?params=`
  /// (a Settings screen), `/app/<app>/<screen>?params=&scope=`; and `/vault?id=&scope=`, a request for an item, as a sheet
  /// over You. Each is a screen: the client's own (`own`, by name), or an app's.
  function route() {
    const path = location.pathname.split("/").filter(Boolean).map(decodeURIComponent);
    const q = new URLSearchParams(location.search);
    if (path[0] === "page" && path[1]) return own("pages", { scope: path[1] });
    if (path[0] === "services" && path[1]) return own("services", { scope: path[1] });
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
      if (r.name === "services") return `/services/${encodeURIComponent(params.scope || "user")}`;
      if (r.params && r.params !== "{}") q.set("params", r.params);
      return `/settings/${encodeURIComponent(r.name)}${q.size ? `?${q}` : ""}`;
    }
    if (r.params && r.params !== "{}") q.set("params", r.params);
    if (r.scope) q.set("scope", r.scope);
    return `/app/${encodeURIComponent(r.app)}/${encodeURIComponent(r.screen || "main")}${q.size ? `?${q}` : ""}`;
  }

  /// A phone's width has no sidebar, as the iPhone and Android apps have none: the pages fill the window, swiped
  /// between, and every screen opened from them is pushed over them, with Back.
  const phone = matchMedia("(max-width: 700px)");
  const isPage = (r) => r.kind === "own" && r.name === "pages";
  /// The page a screen is under, for Back on a phone when nothing in this tab came before it: a page's Services and apps
  /// under that page, Settings' screens and apps under Settings.
  function under(r) {
    const params = JSON.parse(r.params || "{}");
    if (r.kind === "own") return own("pages", { scope: r.name === "services" ? params.scope || "user" : r.name === "request" ? "user" : "settings" });
    return own("pages", { scope: r.scope || (settingsItems().some((i) => i.app === r.app) ? "settings" : "user") });
  }

  /// Goes somewhere: a sidebar row replaces where the window is; what a screen opens deeper than the sidebar is pushed
  /// on top, so Back returns to it. On a phone, all but a page is pushed.
  function go(r, { pushed = phone.matches && !isPage(r) } = {}) {
    // Pushed on a screen, the sidebar keeps what it had selected, and the bar offers Back.
    const state = pushed ? { back: true, selection: history.state?.selection || location.pathname + location.search } : null;
    history.pushState(state, "", href(r));
    // A phone's pages open again at the one gone to.
    if (phone.matches && isPage(r)) home = null;
    render();
  }
  /// Back: to what came before in this tab, or on a phone opened straight at a screen, to the page it's under.
  function back() {
    if (history.state?.back) return history.back();
    history.replaceState(null, "", href(under(route())));
    render();
  }
  /// The sidebar row selected: where the window is, or what a pushed screen was opened from.
  const selection = () => history.state?.selection || location.pathname + location.search;
  addEventListener("popstate", () => render());
  // Between a phone's width and a computer's, the window is drawn again as the other.
  phone.addEventListener("change", () => {
    shown = "";
    home = null;
    frames = frames.filter((f) => f.sheet);
    const detail = $(".detail", root);
    if (detail) detail.innerHTML = "";
    render();
  });

  // MARK: Drawing

  const root = $("#root");

  /// While this browser waits to be approved, the window isn't drawn: its join screen is.
  let joining = false;
  /// Settled once this browser has been let in as its person's device, or needn't be: what screens ask waits for it,
  /// since only a device hands a node the keys it needs.
  const joined = (() => {
    let done;
    const p = new Promise((r) => (done = r));
    return Object.assign(p, { done });
  })();

  function render() {
    if (!store.state || joining) return;
    if (!install()) return void (root.innerHTML = message("Not Set Up Yet", "Set it up on one of your computers first."));
    if (!$(".window", root)) root.innerHTML = `<div class="window"><nav class="sidebar"></nav><main class="detail"></main></div>`;
    drawSidebar();
    drawDetail();
  }

  function message(title, text, extra = "") {
    return `<div class="unavailable">${one.problem({ title, text })}${extra}</div>`;
  }

  /// A page's Services folder: its services' icons, four at most, on frosted glass; with none, four faint empty places.
  function folder(apps, size) {
    const cells = apps.length ? apps.slice(0, 4).map((a) => one.appIcon(a, size * 0.36)).join("") : `<i></i>`.repeat(4);
    return `<span class="folder" style="--size:${size}px">${cells}</span>`;
  }

  /// One section per page, listing its apps and its Services; then Settings', listing its items. An app Settings lists
  /// (the App Store) shows there only.
  function drawSidebar() {
    const at = selection();
    const row = (to, icon, name, count) => `<a class="nav-row${href(to) === at ? " selected" : ""}" href="${esc(href(to))}" data-go='${esc(JSON.stringify(to))}'>${icon}<span class="nav-label">${esc(name)}</span>${count ? `<span class="nav-count">${count}</span>` : ""}</a>`;
    const head = (to, name) => `<a class="nav-head${href(to) === at ? " selected" : ""}" href="${esc(href(to))}" data-go='${esc(JSON.stringify(to))}'>${esc(name)}</a>`;
    const sections = pages().map((p) => `<section>${head(own("pages", { scope: p.scope }), p.title)}` +
      appsOn(p).filter((a) => !settingsItems().some((i) => i.app === a.name)).map((a) => row({ kind: "screen", app: a.name, screen: "main", params: "{}", scope: p.scope }, one.appIcon(a, 20), label(a), appBadge(a.name, p))).join("") +
      row(own("services", { scope: p.scope }), folder(servicesOn(p), 20), "Services") + `</section>`).join("");
    // One with another site's url opens it in a new tab; one here (Passkeys) shows beside the sidebar.
    const items = settingsItems().map((item) => item.url && !here(item.url)
      ? `<a class="nav-row" href="${esc(item.url)}" target="_blank" rel="noopener">${settingIcon(item, 20)}<span class="nav-label">${esc(item.title)}</span></a>` : row(item.app ? { kind: "screen", app: item.app, screen: "main", params: "{}" } : { kind: "own", name: item.name, params: "{}" }, settingIcon(item, 20), item.title, item.badge)).join("");
    $(".sidebar", root).innerHTML = `<div class="nav-pages">${sections}<section>${head(own("pages", { scope: "settings" }), "Settings")}${items}</section></div>`;
  }

  // Links are real links, for a new tab or a copied address; a plain click goes in place.
  document.addEventListener("click", (e) => {
    if (e.target.closest?.("[data-back]")) return back();
    const a = e.target.closest?.("[data-go]");
    if (!a || e.metaKey || e.ctrlKey || e.shiftKey || e.button) return;
    e.preventDefault();
    go(JSON.parse(a.dataset.go));
  });

  let shown = "";

  /// What's beside the sidebar: the screen where the window is, kept as it is while the rest redraws around it.
  function drawDetail() {
    const r = route(), key = JSON.stringify(r);
    if (phone.matches) return drawPhone($(".detail", root), r, key);
    if (shown === key && frames.some((f) => !f.sheet && f.into.isConnected)) return;
    shown = key;
    screen.show($(".detail", root), r);
  }

  /// A phone's window: every page side by side then Settings, to swipe between with their dots (pages.html, as the
  /// phones have them), kept under what's pushed over them so Back finds them as they were.
  let home = null;
  function drawPhone(detail, r, key) {
    if (!home?.layer.isConnected) {
      frames = frames.filter((f) => f.sheet);
      detail.innerHTML = `<div class="home"></div><div class="pushed" hidden></div>`;
      const layer = $(".home", detail), at = isPage(r) ? r : under(r);
      home = { layer, f: frameFor(own("pages", { swipe: true, scope: JSON.parse(at.params).scope || "user" }), layer) };
      home.f.home = true;
      shown = "";
    }
    const pushed = $(".pushed", detail);
    if (isPage(r)) {
      shown = key;
      frames = frames.filter((f) => f.sheet || f.home);
      pushed.hidden = true;
      pushed.innerHTML = "";
      home.layer.inert = false;
      document.title = titleOf(r);
      return;
    }
    if (shown === key && frames.some((f) => !f.sheet && !f.home && f.into.isConnected)) return;
    shown = key;
    pushed.classList.toggle("entering", pushed.hidden);
    pushed.hidden = false;
    home.layer.inert = true;
    screen.show(pushed, r);
  }

  /// The detail's bar: its title, as a Mac window's toolbar, with Back when a screen was pushed, and on a phone under
  /// every page.
  function bar(title) {
    const back = history.state?.back || (phone.matches && !isPage(route())) ? `<button class="back" data-back aria-label="Back">‹</button>` : "";
    const dev = developer.on() ? `<button class="dev-button" data-devtools aria-label="Developer" title="Developer">&lt;/&gt;</button>` : "";
    return `<header class="bar">${back}<div class="bar-titles"><div class="bar-title">${esc(title)}</div></div>${dev}</header>`;
  }

  /// Developer, in Developer mode: the bar's </> opens its panel (src/screen/devtools.html) for the screen in front,
  /// docked under it on a computer's width, where it stays open from screen to screen until closed, and a sheet on a
  /// phone's. It's told the screen's app and title, and for the client's own, since when it showed.
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
      if (phone.matches) return void openSheet(devtools.route());
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
      const detail = $(".detail", root);
      let dock = detail && $(".devtools", detail);
      if (!detail || phone.matches || !developer.on() || !devtools.open()) {
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
  document.addEventListener("click", (e) => e.target.closest?.("[data-devtools]") && devtools.toggle());

  // MARK: Screens

  /// Every screen showing: the one beside the sidebar, and the sheets over it, each an app's or the client's own, in a
  /// sandboxed frame whose bridge this page answers: an app's acting only as that app, and only the client's own as the
  /// person.
  let frames = [];

  /// What this browser keeps of the screens it showed, until it signs out: each route's page as the node last gave it
  /// (`page`), and its still (`still`), the page as it last looked here without its scripts, which bridge.js sends.
  const kept = {
    url: (kind, key) => `/${kind}/${encodeURIComponent(key)}`,
    async get(kind, key) {
      try {
        return (await (await (await caches.open(KEPT_SCREENS)).match(kept.url(kind, key)))?.text()) ?? null;
      } catch {
        return null;
      }
    },
    put(kind, key, html) {
      if (typeof html !== "string" || html.length > 2_000_000) return;
      window.caches?.open(KEPT_SCREENS).then((c) => c.put(kept.url(kind, key), new Response(html))).catch(() => {});
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
    return joined.then(() => api(path, { query: { params: r.params || "{}", scope: r.scope }, raw: true, since: developer.on() ? since : undefined }).then((resp) => resp.text()).then((html) => (kept.put("page", key, html), html)));
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
    setTimeout(() => f.loaded || f.still || !into.isConnected || $(".screen-status", into) || into.insertAdjacentHTML("afterbegin", `<div class="screen-status">${one.loading()}</div>`), 400);
    const draw = (page) => {
      f.page = page;
      f.html = oneHost.look(page, appearance(), !!sheet);
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
    Promise.all([r.kind === "own" ? null : kept.get("page", f.key), sheet ? null : kept.get("still", f.key)]).then(([page, still]) => {
      if (looking && page) developer.keep(looking);
      if (f.html) return;
      if (still) showStill(f, still);
      if (page) draw(page);
    });
    fresh.then((page) => {
      if (!f.html) return draw(page);
      if (same(page, f.page)) return;
      f.page = page;
      f.html = oneHost.look(page, appearance(), !!sheet);
      frame.src = asset("frame.html");
    }, (err) => f.html || ((f.loaded = true), (into.innerHTML = message("Can’t Open This", one.plain(err).title))));
    return f;
  }

  // MARK: Stills

  /// A screen's still, in a frame of its own as sandboxed as the screen's, answering nothing but its frame's ask for it.
  function showStill(f, html) {
    if (!f.into.isConnected) return;
    const frame = sandboxed(f.route);
    frame.classList.add("screen-still");
    frame.setAttribute("aria-hidden", "true");
    const scrolled = `<script>addEventListener("load", () => scrollTo(0, Number(document.documentElement.dataset.stillY) || 0))</script>`;
    f.still = { frame, html: oneHost.look(html.replace("</head>", `${scrolled}</head>`), appearance()) };
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
      if (r.name === "services") return "Services";
      if (r.name === "request") return "Vault";
      if (r.name === "devtools") return "Developer";
      return settingTitle(r.name);
    }
    const a = app(r.app);
    return a?.screens?.find((s) => s.name === r.screen)?.title || label(a) || r.app;
  }

  const screen = {
    show(detail, r) {
      frames = frames.filter((f) => f.sheet || f.home);
      const title = titleOf(r);
      document.title = title;
      // A page shows its own large title.
      detail.innerHTML = bar(r.kind === "own" && r.name === "pages" ? "" : title) + `<div class="screen"></div>`;
      devtools.since = Date.now();
      devtools.draw();
      // A Settings item that's a page of this site (Passkeys) is that page, framed: it's the kernel's, not a node's.
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
  addEventListener("keydown", (e) => e.key === "Escape" && shaded && $(".shade.open") && !$(".shade.open ~ .sheet-backdrop.open") && shaded.frame.contentWindow?.postMessage({ oneSheetsClose: true }, "*"));

  /// A screen over the window, in a sheet: → what it gave `one.close` once it's gone, else null.
  function openSheet(r) {
    return new Promise((done) => {
      let value = null;
      // A request is answered in it, or not: Cancel, not Done.
      const handle = one.sheet({ title: titleOf(r), done: r.name === "request" ? "Cancel" : "Done", onClose: () => { frames = frames.filter((x) => x !== f); done(value); } });
      handle.body.closest(".sheet").classList.add("screen-sheet");
      handle.body.classList.add("screen");
      const f = frameFor(r, handle.body, { sheet: { close: (v) => { value = v ?? null; handle.close(); }, title: handle.body.closest(".sheet").querySelector(".sheet-title") } });
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
    if (e.data?.oneStill !== undefined) return void (f.sheet || kept.put("still", f.key, e.data.oneStill));
    const { one: m, id } = e.data || {};
    if (!m || typeof m !== "object") return;
    // What a call answered last time, if it only reads: from this browser, at once, while the window still starts.
    if (m.type === "kept") {
      const op = String(m.op || "");
      const got = !mine && reads(r.app, op) ? await answers.kept(auth.user(), r.app, op, m.input ?? {}).catch(() => null) : null;
      return void f.frame.contentWindow?.postMessage({ oneReply: id, ok: got }, "*");
    }
    // What it's waiting on: its still shows until it's been answered.
    f.busy++;
    watch(f);
    await joined;
    const answered = () => ((f.busy = Math.max(0, f.busy - 1)), (f.quiet = Date.now()));
    const reply = (ok) => (answered(), f.frame.contentWindow?.postMessage({ oneReply: id, ok: ok ?? null }, "*"));
    const fail = (err) => (answered(), f.frame.contentWindow?.postMessage({ oneReply: id, error: String(err?.message || err) }, "*"));
    try {
      reply(await oneHost.answer(m, {
        frame: f.frame,
        call: async (op, input) => {
          if (mine) throw new Error("Settings has no app to call.");
          const out = await call(r.app, op, input);
          if (reads(r.app, op)) answers.keep(auth.user(), r.app, op, input, out).catch(() => {});
          return out;
        },
        open: (m) => {
          const params = JSON.stringify(m.params || {});
          // Any screen may send its person to choose the device that keeps its app running, in Settings › Devices.
          // An app's says only which app to keep running, and where: no code to approve.
          const to = m.app === "settings" && m.screen === "devices" ? { kind: "own", name: "devices", params: mine ? params : JSON.stringify({ keep: m.params?.keep, scope: m.params?.scope }) }
            : typeof m.app === "string" && m.app ? { kind: "screen", app: m.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope }
            : mine ? { kind: "own", name: String(m.screen || ""), params }
            : { kind: "screen", app: r.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope };
          if (m.sheet) return openSheet(to);
          // From a sheet, the sheet goes and it opens where the window is. What the sidebar lists (an app on a page,
          // Backups in Settings) opens as if picked there, with no Back; only what's deeper than it is pushed.
          f.sheet?.close(null);
          go(to, { pushed: phone.matches ? !isPage(to) : ![...document.querySelectorAll(".sidebar [data-go]")].some((a) => a.getAttribute("href") === href(to)) });
          return null;
        },
        close: (value) => {
          if (f.dock) return (localStorage.removeItem(devtools.KEY), devtools.draw(), null);
          if (f.sheet) f.sheet.close(value);
          else if (history.state?.back || (phone.matches && !isPage(route()))) back();
          return null;
        },
        title: (title, m) => {
          // Empty while the screen shows its own large title, as the Mac app's bar is; the tab is named by what the
          // screen says it's called.
          if (f.sheet) f.sheet.title.textContent = title || titleOf(r);
          // A phone's pages have no bar: what's pushed over them has its own.
          else if (!f.home) {
            const el = $(".detail .bar-title", root);
            if (el) el.textContent = title;
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
          else if (["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) window.open(url.href, "_blank", "noopener,noreferrer");
          return null;
        },
        icons: iconPaths,
        blob: async (id) => (await api(`/v1/blobs/${encodeURIComponent(id)}`, { raw: true })).blob(),
        traces: () => (developer.on() ? developer.kept : null),
        own: mine && {
          developer: (m) => ("on" in m && developer.set(!!m.on), developer.on()),
          api: clientApi,
          signOut: () => (signOut(), null),
          appearance: (m) => (m.value != null && setAppearance(String(m.value)), appearance()),
          device: () => ({ name: browserName(), signOut: true, keys: !!device, ...(device ? { key: device.me.key } : {}) }),
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

  // MARK: Settings

  /// What Settings shows this person, in order, as the node says (`settings_items`).
  const settingsItems = () => withWaiting(store.state)?.settings_items || [];
  /// Whether a url is this site's: a page the kernel serves, shown in place rather than in a new tab.
  const here = (url) => new URL(url, location.href).origin === location.origin;
  /// The Settings item whose url is `url` on this site, if one is.
  const settingAt = (url) => settingsItems().find((i) => i.url && here(i.url) && new URL(i.url, location.href).pathname === new URL(url, location.href).pathname);
  const settingTitle = (name) => settingsItems().find((i) => i.name === name)?.title || "Settings";

  /// An item's icon: its symbol on its colour, as an app's tile.
  function settingIcon(item, size) {
    return one.appIcon({ name: item.name, title: item.title, icon: item.icon, color: item.color }, size);
  }

  /// `one.api` for a client's own screen: the node's /v1 at this origin, as this person; nothing else here. What
  /// changes who can do what, and letting devices in, this browser does itself (`signs`).
  async function clientApi(m) {
    const url = new URL(String(m.path || ""), location.origin);
    if (url.origin !== location.origin || !url.pathname.startsWith("/v1/")) throw new Error("That isn’t on your devices.");
    const method = String(m.method || "GET").toUpperCase();
    const body = method === "GET" ? undefined : m.body ?? {};
    if (url.pathname === "/v1/call" && body && typeof body === "object") body.client = "web";
    const own = signs[`${method} ${url.pathname}`];
    if (own) {
      if (!device) throw new Error("This device hasn't been approved yet.");
      const out = await own(body || {});
      await refreshState().catch(() => {});
      return out ?? {};
    }
    if (method === "GET" && !url.search && url.pathname === "/v1/state") return withWaiting(await shared(url.pathname));
    if (method === "GET" && !url.search && url.pathname === "/v1/apps") return shared(url.pathname);
    return api(url.pathname + url.search, { method, body });
  }

  /// Signs `content` in `log` as this browser, and writes it.
  const write = (content, log = "global") => device.write(log, content);

  /// What Settings' screens change that this browser signs itself, as its person's device: what a node's route of the
  /// same path would do, built here (members.js), never by a node.
  const signs = {
    "POST /v1/role": (b) => write(members.role({ user: b.user, role: b.role })),
    "POST /v1/remove": (b) => (b.user ? device.removePerson(String(b.user)) : device.remove(String(b.key || ""))),
    // Where this person's own things run besides their own machines: their device's say, nobody else's.
    "POST /v1/runs": (b) => write(members.runs({ on: (b.on || []).map(String) })),
    "POST /v1/settings": (b) => write(members.setting({ name: b.name, value: b.value ?? null })),
    "POST /v1/group": async (b) => {
      const name = String(b.name || "").trim();
      if (!name) throw new Error("Give the group a name.");
      const group = Array.from(crypto.getRandomValues(new Uint8Array(8)), (x) => x.toString(16).padStart(2, "0")).join("");
      await write(members.group({ group, name }));
      return { group };
    },
    "POST /v1/invite": (b) => write(members.invite({ group: b.group, user: b.user, role: b.role || "member" })),
    "POST /v1/accept": (b) => write(members.accept({ group: b.group })),
    "POST /v1/member": (b) => write(members.member({ group: b.group, user: b.user, role: b.role ?? null })),
    "POST /v1/handle": (b) => write(members.handle({ ...(b.group ? { what: "group", id: String(b.group) } : { what: "user", id: me().id }), handle: String(b.handle || "").trim() })),
    // `key`: one of this person's devices; none, this browser.
    "POST /v1/name": (b) => write(members.rename({ what: "key", id: String(b.key || device.me.key), name: String(b.name || "").trim() })),
    // A service removed from one of this person's devices (`device`, its key), or with `installed` let register again.
    "POST /v1/service": (b) => write(members.service({ device: String(b.device || ""), name: String(b.name || ""), on: b.on === true })),
    "POST /v1/retire": (b) => write(members.retire({ app: b.app, owner: app(b.app)?.owner, how: b.how })),
    // An app on everyone's page is a change to who can do what; on yours or a group's, a node writes it.
    "POST /v1/install": (b) => (b.scope === "global" ? write(members.install({ app: b.app, installed: b.installed !== false })) : api("/v1/install", { method: "POST", body: b })),
    "POST /v1/invite-link": (b) => device.invite(String(b.name || "")),
    // As a machine's node answers it: `device` by its key; `scope` user (theirs), global or group:<id>; `days`, 0 to stop.
    "POST /v1/grant": (b) => device.grant({ endpoint: String(b.device || ""), app: String(b.app || ""), scope: !b.scope || b.scope === "user" ? `user:${me().id}` : String(b.scope), days: b.days ?? undefined }),
    "GET /v1/devices/passkey": async () => ({ passkey: !!passkeyToList() }),
    // `{ key: ms }`: when each device was last let onto the install's network.
    "GET /v1/devices/seen": () => device.seen(),
    // As a machine's node answers it, each with its code, which the device asking shows too, to compare.
    "GET /v1/devices/requests": async () => (await waitingNow()).map((r) => ({ device: r.name, person: r.person, code: r.code, made: r.made, ...(r.place ? { place: r.place } : {}), ...(r.mine ? { yours: true } : { new: true }) })),
    // `{ code }`: one waiting that isn't theirs, turned down, and not asked about again here.
    "POST /v1/devices/refuse": async (b) => {
      const code = String(b.code || "").toUpperCase().replace(/[^A-Z2-7]/g, "");
      const r = (await device.requests()).find((x) => x.code.replace(" ", "") === code);
      if (!r) return;
      notMine.add(r.key);
      localStorage.setItem(NOT_MINE, JSON.stringify([...notMine].slice(-50)));
      await device.refuse(r);
    },
    // As a machine's node answers it: without `confirm`, which device it is; with, it's let in.
    "POST /v1/devices/approve": async (b) => {
      const code = String(b.code || "").toUpperCase().replace(/[^A-Z2-7]/g, "");
      const found = (await device.requests()).filter((x) => x.code.replace(" ", "") === code);
      if (!found.length) throw new Error("No device is waiting with that code. Check it, or start again on that device.");
      if (found.length > 1) throw new Error("Two devices are waiting with that code, so neither was let in. Start again on yours, for a new code.");
      const [r] = found;
      if (b.confirm !== true) return { device: r.name, person: r.person, new: !r.mine };
      await device.approve(r);
    },
    "POST /v1/devices/passkey": async () => {
      const p = passkeyToList();
      if (!p) throw new Error("Sign in with your passkey first.");
      await device.listPasskey(p.prf, p.credential);
      sessionStorage.removeItem("one.prf");
    },
  };

  // MARK: Devices waiting to be let in

  /// Devices waiting that this browser may let in, as last asked: Settings › Devices has a badge for each, and the
  /// first time one is seen here, Devices opens to ask about it. Those its person said aren't theirs aren't asked again.
  let waiting = [];
  const NOT_MINE = "one.notMine";
  const notMine = new Set(JSON.parse(localStorage.getItem(NOT_MINE) || "[]"));
  const seen = new Set();
  let prompting = false;

  /// Asks auth which devices wait now, and tells the window and its screens if that changed. → them.
  async function waitingNow() {
    const now = (await device.requests()).filter((r) => !notMine.has(r.key));
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

  /// Every 10 seconds while the page shows.
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
    if (!prf || !credential || install()?.clients.some((c) => c.passkey === credential)) return null;
    return { prf: Uint8Array.from(atob(prf.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)), credential };
  }

  // MARK: Appearance

  const LOOK = "one.appearance";
  const appearance = () => (["light", "dark"].includes(localStorage.getItem(LOOK)) ? localStorage.getItem(LOOK) : "automatic");

  function setAppearance(value) {
    if (value === "automatic") localStorage.removeItem(LOOK);
    else if (["light", "dark"].includes(value)) localStorage.setItem(LOOK, value);
    schemed(document, appearance());
    for (const f of frames) f.frame.contentWindow?.postMessage({ oneScheme: appearance() }, "*");
  }

  const schemed = oneHost.scheme();

  // MARK: Starting

  function showSignedOut(error) {
    const name = config().name;
    document.title = name;
    root.innerHTML = `<div class="welcome"><img class="welcome-mark" src="/logo.png" alt=""><h1>${esc(name)}</h1>` +
      `<button class="button primary big" data-signin>Sign In</button>${error ? one.problem({ title: error }) : ""}</div>`;
    $("[data-signin]", root).onclick = async (e) => {
      e.target.disabled = true;
      try {
        await auth.start();
      } catch (err) {
        showSignedOut(err.message);
      }
    };
  }

  /// Back from a sign-in at another site, in a browser that isn't signed in here: on to the app, which finishes it.
  function showBack(app, screen, params) {
    const { name, cli } = config();
    const link = `${cli}://open?${new URLSearchParams({ app, screen, params, scope: "user" })}`;
    document.title = name;
    root.innerHTML = `<div class="welcome"><img class="welcome-mark" src="/logo.png" alt=""><h1>${esc(name)}</h1>` +
      `<a class="button primary big" href="${esc(link)}">Open ${esc(name)}</a></div>`;
    location.href = link;
  }

  function showUnreachable(text) {
    root.innerHTML = message("Can’t Reach Your Devices", text, `<div class="unavailable-text">Trying again…</div>`);
  }

  /// Makes this browser a device of its person, before anything is asked of a node: one already, or let in by a passkey
  /// of theirs that's listed, or by one of their devices (or, for someone new, their inviter's), which it asks at
  /// auth and waits for there, showing its code. → its keys.
  async function join(user) {
    const keys = await browserKeys(user);
    if (await net.start(user).catch(() => false)) return keys;
    if (await byPasskey(user, keys)) return keys;
    const invite = sessionStorage.getItem("one.invite");
    const ask = async () => answer(await bearer("/api/auth/devices/requests", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await joinAsk(keys, browserName(), invite)) }));
    await ask();
    await showJoin(await codeOf(keys.key, keys.seal), !!invite);
    // Auth says once it was approved; then the relay lets it in. One gone from auth (approved and cleared, refused, or
    // waited 15 minutes) is asked again, unless it's in. As every client does (`waited`).
    for (;;) {
      await sleep(2000);
      const r = await bearer(`/api/auth/devices/requests/${keys.key}`).catch(() => null);
      const said = waited(r?.status || 0, r?.ok ? await r.json().catch(() => ({})) : {});
      if (said.signedOut) throw Object.assign(new NodeError("Signed out."), { signedOut: true });
      // The install it was let into, as the device that approved it said: the only one it then takes.
      if (said.genesis) await put(`genesis:${user}`, said.genesis);
      if ((said.gone || said.approved) && (await net.start(user, true).catch(() => false))) break;
      if (said.gone) await ask().catch(() => {});
    }
    sessionStorage.removeItem("one.invite");
    root.innerHTML = "";
    return keys;
  }

  /// Lets this browser in by a passkey its person listed, whose PRF output this tab has from signing in: it asks as that
  /// passkey's device, which approves this browser. → whether it's in.
  async function byPasskey(user, keys) {
    const prf = sessionStorage.getItem("one.prf");
    if (!prf) return false;
    const bytes = Uint8Array.from(atob(prf.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0));
    try {
      const pk = await passkeyKeysOf(bytes);
      if (!(await net.start(user, true, { key: pk.key, sign: pk.sign }))) return false;
      await refreshState();
      await new Device(keys, { user, state: () => store.state, api, bearer, replay: net.global }).approveByPasskey(bytes);
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
    await device.settleOwn().catch((err) => console.warn("making this person's own keys:", err));
    await refreshState().catch(() => {});
  }

  /// "Waiting for Approval": its code, and the code as a QR, a link that opens Approve with it in the browser of a
  /// phone that scans it.
  async function showJoin(code, invited) {
    document.title = "Waiting for Approval";
    const link = `${location.origin}/approve#${code.replace(" ", "")}`;
    root.innerHTML = `<div class="welcome join"><h1>Waiting for Approval</h1>` +
      `<p class="join-text">${invited ? "Whoever invited you approves it. Show them this code." : "Your other devices ask you to approve it. It shows this code there too."}</p>` +
      `<div class="join-code" aria-label="Code">${esc(code)}</div><div class="join-qr">${qrSvg(link)}</div>` +
      `<p class="join-note">${invited ? "Or they scan this with their phone." : "Or scan this with your phone, to approve it in its browser."}</p><button class="button" data-out>Sign Out</button></div>`;
    $("[data-out]", root).onclick = () => signOut();
  }

  async function start() {
    schemed(document, appearance());
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
    navigator.serviceWorker?.register("/sw.js").catch(() => {});
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
    else root.innerHTML = `<div class="starting">${one.loading("Starting")}</div>`;
    joining = true;
    const user = auth.user();
    try {
      // A device from the start, as a node may need the keys it hands (the serverless node, even for the state).
      device = new Device(await join(user), { user, state: () => store.state, api, bearer, replay: net.global });
      await device.load();
    } catch (err) {
      if (err.signedOut) return err.removed ? undefined : showSignedOut();
      return void (root.innerHTML = message("Can’t Let This Device In", err.message));
    }
    // A device that takes the request and never answers (asleep, stuck) is one that can't be reached, while it's still asked.
    const away = setTimeout(() => $(".window", root) || showUnreachable(UNANSWERED), 15000);
    for (let misses = 1; ; misses++) {
      try {
        await refreshState();
        break;
      } catch (err) {
        if (err.signedOut) return clearTimeout(away), err.removed ? undefined : showSignedOut();
        // What it showed last stays, as a screen whose node is restarting does.
        if (!$(".window", root)) showUnreachable(err instanceof NodeError ? err.message : UNANSWERED);
        await sleep(retryWait(misses));
      }
    }
    clearTimeout(away);
    if (install() && me()) await settle();
    joining = false;
    joined.done();
    await refreshApps().catch(() => {});
    if (request?.id && request.scope) openSheet(own("request", { scope: request.scope, id: request.id }));
    listen();
    watchWaiting();
  }

  document.addEventListener("DOMContentLoaded", start);
})();
