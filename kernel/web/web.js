// The web client: the Mac app's window, in a browser. It signs in as an OAuth client of the install's auth service
// (PKCE, and the person's passkey at its sign-in page), keeps its refresh token in this browser, and calls a node's `/v1` on its
// own origin with the access token; the gateway sends each on. App screens are untrusted: each runs in a sandboxed
// frame, and this page answers its bridge as the Mac app's web view does, making each call itself. Nothing is cached.

// The bridge and kit (src/screen/), loaded after this, give `one`; this page answers it for its own use (`__one.host`).
// This browser is a device of its person (device.js): it signs what changes who can do what, and hands each call the
// keys it needs.
import { answer, browserKeys, browserName, codeOf, Device, joinAsk } from "./device.js";
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
  // What the client's own screens keep between visits (one.cache), forgotten with the sign-in.
  const SCREENS = "one.screens.";
  const forgetScreens = () => Object.keys(localStorage).filter((k) => k.startsWith(SCREENS)).forEach((k) => localStorage.removeItem(k));
  const CLIENT = "one.client";

  const auth = {
    kept: () => JSON.parse(localStorage.getItem(KEPT) || "null"),
    keep: (tokens) => localStorage.setItem(KEPT, JSON.stringify(tokens)),

    /// The auth service's endpoints, as it says them.
    async meta() {
      const r = await fetch("/.well-known/oauth-authorization-server");
      if (!r.ok) throw new Error("The sign-in service isn't answering.");
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

    /// Who the access token says this is (its `sub`), for a browser whose person isn't in yet.
    user() {
      try {
        return JSON.parse(atob(auth.kept().access.split(".")[1].replaceAll("-", "+").replaceAll("_", "/"))).sub || null;
      } catch {
        return null;
      }
    },
    /// Whether the node has said who this is: once it has, a 403 is a refusal, not someone not yet let in.
    person: () => !!store.state?.user,

    async signOut() {
      const kept = auth.kept();
      localStorage.removeItem(KEPT);
      forgetScreens();
      try {
        const meta = await auth.meta();
        const client = JSON.parse(localStorage.getItem(CLIENT) || "null")?.id;
        if (kept?.refresh && meta.revocation_endpoint) {
          await fetch(meta.revocation_endpoint, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams({ token: kept.refresh, token_type_hint: "refresh_token", client_id: client }) });
        }
      } catch {}
      location.href = "/";
    },
  };

  // MARK: The node

  class NodeError extends Error {}

  /// This browser as its person's device, once it's listed as one (`join`): null until then.
  let device = null;

  /// A request to the node, as this person: JSON in and out, or with `raw` the response itself. Once this browser is
  /// one of their devices, it asks their nodes itself, over iroh (net.js); until then, through the address, with its
  /// access token. As a device, it goes with the keys it needed last time; a node that needs others says so (428), and
  /// it goes again once with them.
  async function api(path, { method = "GET", body, query, raw, signal } = {}) {
    const url = new URL(path, location.origin);
    for (const [k, v] of Object.entries(query || {})) if (v != null) url.searchParams.set(k, typeof v === "string" ? v : JSON.stringify(v));
    const route = Device.route(url.pathname, body);
    const user = auth.user();
    const direct = !!user && (await net.start(user).catch(() => false));
    let token = direct ? null : await auth.token();
    for (let tries = 0, needed = false; ; tries++) {
      const headers = direct ? {} : { authorization: `Bearer ${token}` };
      if (body !== undefined) headers["content-type"] = "application/json";
      // Keys go only to a node this browser reaches itself, which its connection proves.
      const keys = direct && device && (await device.handoff(route, auth.kept()?.until || Date.now() + 60_000));
      if (keys) headers["x-one-keys"] = keys;
      const sent = body === undefined ? undefined : JSON.stringify(body);
      const r = direct ? await net.fetch(method, url.pathname + url.search, headers, sent, signal) : await fetch(url, { method, headers, body: sent, signal });
      if (r.status === 401 && tries === 0) {
        token = await auth.token(token);
        continue;
      }
      if (r.status === 401) throw Object.assign(new NodeError("Signed out."), { signedOut: true });
      if (r.status === 403 && !auth.person()) throw Object.assign(new NodeError("Not approved yet."), { status: 403 });
      if (r.status === 428 && direct && device && !needed) {
        needed = true;
        await device.learn(route, await r.json(), r.node);
        continue;
      }
      if (raw && r.ok) return r;
      const out = await r.json().catch(() => null);
      if (!r.ok || out?.error) throw Object.assign(new NodeError(out?.error || (r.status >= 500 ? "Your device isn't answering." : "That didn't work. Try again.")), { status: r.status });
      return out;
    }
  }

  /// A request to the address that isn't a node's, with the access token: auth's own routes.
  async function bearer(path, init = {}) {
    return fetch(path, { ...init, headers: { ...init.headers, authorization: `Bearer ${await auth.token()}` } });
  }

  /// Phosphor's icons, from the node once, for those the bridge asks for: on this page, and in screens' frames.
  let icons;
  async function iconPaths(names = []) {
    icons ||= api("/v1/icons").catch((err) => ((icons = null), Promise.reject(err)));
    const all = await icons;
    return Object.fromEntries(names.map((n) => [n, all[n]]));
  }
  window.__one.host = async (m) => (m.type === "copy" ? navigator.clipboard.writeText(m.text) : m.type === "icons" ? iconPaths(m.names) : null);

  /// Runs an app's operation as this person → its result.
  async function call(app, op, input = {}) {
    return (await api("/v1/call", { method: "POST", body: { app, op, input, client: "web" } })).ok;
  }

  // MARK: What the window shows

  /// The sidebar's: the node's state, its apps and their badges. What's beside it is a screen the node serves.
  const store = { state: null, apps: [], badges: {} };

  const me = () => store.state?.user;
  const install = () => store.state?.install;
  // The white label's name and accent, as the gateway's index.html says them, until the install's state does.
  const whiteLabel = { name: document.querySelector('meta[name="application-name"]')?.content || "", accent: document.querySelector('meta[name="theme-color"]')?.content || "" };
  const config = () => store.state?.config || whiteLabel;
  const app = (name) => store.apps.find((a) => a.name === name);
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

  /// Loads what the sidebar shows: the node's state, then its apps and badges.
  async function refreshState() {
    const state = await api("/v1/state");
    // The nodes this browser asks itself, in the order things run on them.
    const nodes = (state.install?.devices || []).filter((d) => d.serves_me).sort((a, b) => a.order - b.order).map((d) => d.key);
    if (state.user && nodes.length) net.nodes(state.user.id, nodes);
    const regrouped = JSON.stringify([state.user, state.install?.groups]) !== JSON.stringify([store.state?.user, store.state?.install?.groups]);
    const first = !store.state;
    store.state = state;
    // While it's being let in, its apps wait: they need the keys it's about to get.
    if (joining) return;
    if (first || regrouped || !store.apps.length) await refreshApps();
    else render();
  }

  async function refreshApps() {
    if (!install()) return render();
    store.apps = await api("/v1/apps");
    await Promise.all(pages().map(async (p) => (store.badges[p.scope] = await api("/v1/badges", { query: { scope: p.scope } }).catch(() => ({})))));
    render();
  }

  /// The node's events: what changed, so the window and its screens keep up. EventSource can't send a token, so it's a
  /// fetch that streams, only while the page shows: a page the browser keeps for Back would hold its connection, and
  /// over HTTP/1.1 a browser opens six to an address, so after a few visits the next page waited for one to close.
  let events = null;
  async function listen() {
    const mine = (events = new AbortController());
    while (!mine.signal.aborted) {
      try {
        const r = await api("/v1/events", { raw: true, signal: mine.signal });
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
        if (err.signedOut) return showSignedOut();
      }
      if (!mine.signal.aborted) await new Promise((r) => setTimeout(r, 3000));
    }
  }
  addEventListener("pagehide", () => events?.abort());
  // Back to a page the browser kept: what changed while it was away, then its events again.
  addEventListener("pageshow", (e) => e.persisted && auth.kept() && (refreshState().catch(() => {}), listen()));

  async function handle(event) {
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

  /// Goes somewhere: a sidebar row replaces where the window is; what a screen opens is pushed on top, so Back
  /// returns to it.
  function go(r, { pushed = false } = {}) {
    // Pushed on a screen, the sidebar keeps what it had selected, and the bar offers Back.
    const state = pushed ? { back: true, selection: history.state?.selection || location.pathname + location.search } : null;
    history.pushState(state, "", href(r));
    render();
  }
  /// The sidebar row selected: where the window is, or what a pushed screen was opened from.
  const selection = () => history.state?.selection || location.pathname + location.search;
  addEventListener("popstate", () => render());

  // MARK: Drawing

  const root = $("#root");

  /// While this browser waits to be approved, the window isn't drawn: its join screen is.
  let joining = false;

  function render() {
    if (!store.state || joining) return;
    if (!install()) return void (root.innerHTML = message("Nothing Here Yet", `This device isn't set up yet. Set it up from the ${esc(config().name || "app")} app on one of your devices.`));
    if (!$(".window", root)) root.innerHTML = `<div class="window"><nav class="sidebar"></nav><main class="detail"></main></div>`;
    drawSidebar();
    drawDetail();
  }

  function message(title, text, extra = "") {
    return `<div class="unavailable"><div class="unavailable-title">${esc(title)}</div><div class="unavailable-text">${text}</div>${extra}</div>`;
  }

  /// A page's Services folder: its services' icons, nine at most, small, on frosted glass; with none, nine faint
  /// empty places.
  function folder(apps, size) {
    const cells = apps.length ? apps.slice(0, 9).map((a) => one.appIcon(a, size / 4)).join("") : `<i></i>`.repeat(9);
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
    // One with a url (Passkeys) opens it in a new tab.
    const items = settingsItems().map((item) => item.url
      ? `<a class="nav-row" href="${esc(item.url)}" target="_blank" rel="noopener">${settingIcon(item, 20)}<span class="nav-label">${esc(item.title)}</span></a>` : row(item.app ? { kind: "screen", app: item.app, screen: "main", params: "{}" } : { kind: "own", name: item.name, params: "{}" }, settingIcon(item, 20), item.title, item.badge)).join("");
    $(".sidebar", root).innerHTML = `<div class="nav-pages">${sections}<section>${head(own("pages", { scope: "settings" }), "Settings")}${items}</section></div>`;
  }

  // Links are real links, for a new tab or a copied address; a plain click goes in place.
  document.addEventListener("click", (e) => {
    if (e.target.closest?.("[data-back]")) return history.back();
    const a = e.target.closest?.("[data-go]");
    if (!a || e.metaKey || e.ctrlKey || e.shiftKey || e.button) return;
    e.preventDefault();
    go(JSON.parse(a.dataset.go));
  });

  let shown = "";

  /// What's beside the sidebar: the screen where the window is, kept as it is while the rest redraws around it.
  function drawDetail() {
    const r = route(), key = JSON.stringify(r);
    if (shown === key && frames.some((f) => !f.sheet && f.frame.isConnected)) return;
    shown = key;
    screen.show($(".detail", root), r);
  }

  /// The detail's bar: its title, as a Mac window's toolbar, with Back when a screen was pushed.
  function bar(title) {
    const back = history.state?.back ? `<button class="back" data-back aria-label="Back">‹</button>` : "";
    return `<header class="bar">${back}<div class="bar-titles"><div class="bar-title">${esc(title)}</div></div></header>`;
  }

  // MARK: Screens

  /// Every screen showing: the one beside the sidebar, and the sheets over it, each an app's or the client's own, in a
  /// sandboxed frame whose bridge this page answers: an app's acting only as that app, and only the client's own as the
  /// person.
  let frames = [];

  /// What a route shows, as its page from the node, in a new frame. → the frame's record.
  function frameFor(r, into, { sheet } = {}) {
    const frame = document.createElement("iframe");
    frame.className = "screen-frame";
    frame.setAttribute("sandbox", "allow-scripts allow-forms");
    frame.setAttribute("referrerpolicy", "no-referrer");
    frame.src = "/frame.html";
    frame.title = titleOf(r);
    const f = { frame, route: r, sheet };
    frames.push(f);
    const path = r.kind === "own" ? `/v1/client-screens/${encodeURIComponent(r.name)}` : `/v1/screens/${encodeURIComponent(r.app)}/${encodeURIComponent(r.screen)}`;
    into.innerHTML = `<div class="screen-status">${one.loading()}</div>`;
    api(path, { query: { params: r.params || "{}", scope: r.scope }, raw: true }).then((resp) => resp.text()).then((html) => {
      f.html = look(html);
      into.append(frame);
      frame.addEventListener("load", () => setTimeout(() => $(".screen-status", into)?.remove(), 50), { once: true });
    }, (err) => (into.innerHTML = message("Can’t Open This", esc(err.message))));
    return f;
  }

  function titleOf(r) {
    if (r.kind === "own") {
      const params = JSON.parse(r.params || "{}");
      if (r.name === "pages") return params.scope === "settings" ? "Settings" : page(params.scope || "user")?.title || "";
      if (r.name === "services") return "Services";
      if (r.name === "request") return "Vault";
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
      // A page shows its own large title.
      detail.innerHTML = bar(r.kind === "own" && r.name === "pages" ? "" : title) + `<div class="screen"></div>`;
      frameFor(r, $(".screen", detail));
    },
  };

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

  /// What makes a screen sit in the window as its own: the window's background shows through, the install's accent,
  /// and on a computer the Mac app's size of text.
  function look(html) {
    const accent = /^#[0-9a-f]{3,8}$/i.test(config().accent || "") ? config().accent : "";
    // This window's appearance, kept in step (see `scheme`); with the nonce of a page that runs only its own scripts
    // (the client's own screens), as one of them.
    const nonce = html.match(/^<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="script-src 'nonce-([0-9a-f]+)'/)?.[1];
    const looks = `<script${nonce ? ` nonce="${nonce}"` : ""}>(() => { const scheme = (${scheme})(); let now = ${JSON.stringify(appearance())}; const apply = () => scheme(document, now);
      addEventListener("DOMContentLoaded", apply); addEventListener("load", apply);
      addEventListener("message", (e) => { if (e.source === parent && e.data?.oneScheme) apply(now = e.data.oneScheme); }); })()</script>`;
    const style = `<style>html, body { background: transparent !important; }${accent ? ` :root:root { --accent: ${accent}; }` : ""}
      @media (pointer: fine) { html { zoom: calc(13 / 17); } :root:root { --bg: transparent; --card: rgba(0, 0, 0, 0.04); } }
      @media (pointer: fine) and (prefers-color-scheme: dark) { :root:root { --card: rgba(255, 255, 255, 0.05); } }</style>`;
    return html.includes("</head>") ? html.replace("</head>", `${style}${looks}</head>`) : style + looks + html;
  }

  /// The bridge's messages (src/screen/bridge.js), from the screens' frames only, each acting as its screen's app; and
  /// the client's own screens', as the person, which no app's may.
  addEventListener("message", async (e) => {
    const f = frames.find((x) => x.frame.contentWindow === e.source);
    if (!f) return;
    const r = f.route, mine = r.kind === "own";
    if (e.data?.oneFrame) return void f.frame.contentWindow.postMessage({ oneScreen: f.html }, "*");
    const { one: m, id } = e.data || {};
    if (!m || typeof m !== "object") return;
    const reply = (ok) => f.frame.contentWindow?.postMessage({ oneReply: id, ok: ok ?? null }, "*");
    const fail = (err) => f.frame.contentWindow?.postMessage({ oneReply: id, error: String(err?.message || err) }, "*");
    try {
      if (["api", "signOut", "appearance", "device", "request", "cache"].includes(m.type) && !mine) return fail("Only Settings can do that.");
      switch (m.type) {
        case "call":
          if (mine) return fail("Settings has no app to call.");
          return reply(await call(r.app, String(m.op || ""), m.input ?? {}));
        case "open": {
          const params = JSON.stringify(m.params || {});
          const to = typeof m.app === "string" && m.app ? { kind: "screen", app: m.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope }
            : mine ? { kind: "own", name: String(m.screen || ""), params }
            : { kind: "screen", app: r.app, screen: String(m.screen || "main"), params, scope: m.scope || r.scope };
          if (m.sheet) return reply(await openSheet(to));
          reply(null);
          // From a sheet, the sheet goes and it opens where the window is.
          f.sheet?.close(null);
          return go(to, { pushed: true });
        }
        case "close":
          reply(null);
          if (f.sheet) return f.sheet.close(m.value);
          if (history.state?.back) history.back();
          return;
        case "api":
          return reply(await clientApi(m));
        case "signOut":
          reply(null);
          return auth.signOut();
        case "appearance":
          if (m.value != null) setAppearance(String(m.value));
          return reply(appearance());
        case "device":
          return reply({ name: browserName(), build: deviceBuild(), signOut: true, keys: !!device });
        case "request":
          return reply((await openSheet(own("request", { scope: String(m.scope || ""), id: String(m.id || "") }))) === true);
        case "vault": {
          if (mine) return fail("Settings has no app to ask for.");
          const scope = r.scope || "user";
          const out = await api("/v1/vault/request", { method: "POST", body: { scope, app: r.app, need: String(m.need || "") } });
          if (!out?.id) throw new Error("The Vault didn’t take the request");
          return reply((await openSheet(own("request", { scope, id: out.id }))) === true);
        }
        case "title": {
          // Empty while the screen shows its own large title, as the Mac app's bar is.
          const title = String(m.text ?? "");
          if (f.sheet) f.sheet.title.textContent = title || titleOf(r);
          else {
            const el = $(".detail .bar-title", root);
            if (el) el.textContent = title;
            document.title = title || titleOf(r);
          }
          return reply(null);
        }
        case "menu":
          return reply(await menu(f.frame, m));
        case "alert":
          return reply(await alertSheet(m));
        case "save": case "preview":
          await file(m.type, m.file || {});
          return reply(null);
        case "copy":
          await navigator.clipboard.writeText(String(m.text ?? ""));
          return reply(null);
        case "link": {
          const url = new URL(String(m.url), location.href);
          if (["http:", "https:", "mailto:", "tel:"].includes(url.protocol)) window.open(url.href, "_blank", "noopener,noreferrer");
          return reply(null);
        }
        case "scan":
          return fail("Scanning documents needs the iPhone, iPad or Android app. Choose the files instead.");
        case "icons":
          return reply(await iconPaths(Array.isArray(m.names) ? m.names.map(String) : []));
        case "icon": case "haptic":
          return reply(null);
        case "cache": {
          const key = SCREENS + String(m.key || "");
          if (!("value" in m)) return reply(JSON.parse(localStorage.getItem(key) || "null"));
          try {
            localStorage.setItem(key, JSON.stringify(m.value));
          } catch {}
          return reply(null);
        }
        default:
          return fail("Unknown message");
      }
    } catch (err) {
      fail(err);
    }
  });

  /// `one.menu`: a small menu at its point in the screen's frame, or beside the element it's for (`rect`) → the index
  /// chosen, or null once it's closed by a click elsewhere or Escape. Its `row` items lead, side by side as their icons;
  /// a `section` starts after a line.
  function menu(frame, m) {
    return new Promise((done) => {
      document.querySelector(".menu-backdrop")?.click();
      const entries = Array.isArray(m.items) ? m.items : [];
      if (!entries.length) return done(null);
      // Over everything, screens' frames too, so a click anywhere else closes it.
      const back = document.createElement("div");
      back.className = "menu-backdrop";
      const el = document.createElement("div");
      el.className = "menu";
      el.tabIndex = -1;
      const icon = (e) => (e.icon ? one.icon(String(e.icon), "menu-icon") : "");
      const checks = entries.some((e) => e.checked && !e.row);
      const row = entries.map((e, i) => [e, i]).filter(([e]) => e.row);
      el.innerHTML = (row.length ? `<div class="menu-row">${row.map(([e, i]) => `<button class="menu-choice${e.checked ? " on" : ""}" data-i="${i}" title="${esc(e.title)}" aria-label="${esc(e.title)}">${icon(e) || esc(e.title)}</button>`).join("")}</div>` : "") +
        entries.map((e, i) => (e.row ? "" : `${e.section && i > row.length ? '<div class="menu-line"></div>' : ""}<button class="menu-item${e.danger ? " danger" : ""}" data-i="${i}">` +
          `${checks ? `<span class="menu-check">${e.checked ? "✓" : ""}</span>` : ""}<span class="menu-title">${esc(e.title)}</span>${icon(e)}</button>`)).join("");
      back.append(el);
      document.body.append(back);
      // Its point in the frame, in the window: beside its element, opening away from the edge it's near.
      const box = frame.getBoundingClientRect();
      const r = Array.isArray(m.rect) && m.rect.length === 4 ? m.rect.map(Number) : null;
      let x = box.left + Number(m.x || 0), y = box.top + Number(m.y || 0);
      if (r) {
        const top = box.top + r[1], bottom = top + r[3];
        x = box.left + r[0] + r[2] / 2 - el.offsetWidth / 2;
        y = bottom + el.offsetHeight + 8 > innerHeight && top - el.offsetHeight - 6 > 0 ? top - el.offsetHeight - 6 : bottom + 6;
      }
      el.style.top = `${Math.max(8, Math.min(y, innerHeight - el.offsetHeight - 8))}px`;
      el.style.left = `${Math.max(8, Math.min(x, innerWidth - el.offsetWidth - 8))}px`;
      const close = (i) => { back.remove(); done(i); };
      back.addEventListener("mousedown", (e) => e.target === back && close(null));
      back.addEventListener("click", (e) => e.target === back && close(null));
      back.addEventListener("contextmenu", (e) => { e.preventDefault(); close(null); });
      el.addEventListener("keydown", (e) => e.key === "Escape" && close(null));
      el.focus();
      el.querySelectorAll("[data-i]").forEach((b) => (b.onclick = () => close(Number(b.dataset.i))));
    });
  }

  /// `one.alert`: its title and text, then OK; or Cancel and its action → whether that was chosen.
  function alertSheet(m) {
    return new Promise((done) => {
      let chose = false;
      const { body, close } = one.sheet({ title: String(m.title || ""), done: m.action ? "Cancel" : "OK", onClose: () => done(chose) });
      body.innerHTML = (m.text ? `<p class="sheet-text">${esc(m.text)}</p>` : "") +
        (m.action ? `<div class="sheet-actions"><button class="button ${m.danger ? "danger" : "primary"}" data-yes>${esc(m.action)}</button></div>` : "");
      const yes = $("[data-yes]", body);
      if (yes) yes.onclick = () => { chose = true; close(); };
    });
  }

  /// `one.save` and `one.preview`: a download, or a tab showing it. Only what can't run script shows in a tab, as
  /// this origin's: anything else is saved.
  async function file(how, f) {
    let blob;
    if (f.base64) {
      const bytes = Uint8Array.from(atob(f.base64), (c) => c.charCodeAt(0));
      blob = new Blob([bytes], { type: "application/octet-stream" });
    } else if (f.blob || f.file) {
      blob = await (await api(`/v1/blobs/${encodeURIComponent(f.blob || f.file)}`, { raw: true })).blob();
    } else throw new Error("No file to show.");
    const type = String(f.type || "");
    const viewable = /^(image\/(png|jpeg|gif|webp|avif)|application\/pdf|text\/plain|audio\/|video\/)/.test(type);
    const url = URL.createObjectURL(new Blob([blob], { type: viewable ? type : "application/octet-stream" }));
    setTimeout(() => URL.revokeObjectURL(url), 60_000);
    if (how === "preview" && viewable) return void window.open(url, "_blank", "noopener");
    const a = Object.assign(document.createElement("a"), { href: url, download: f.name || "file" });
    document.body.append(a);
    a.click();
    a.remove();
  }

  // MARK: Settings

  /// What Settings shows this person, in order, as the node says (`settings_items`).
  const settingsItems = () => store.state?.settings_items || [];
  const settingTitle = (name) => settingsItems().find((i) => i.name === name)?.title || "Settings";

  /// An item's icon: its symbol on its colour, as an app's tile.
  function settingIcon(item, size) {
    return one.appIcon({ name: item.name, title: item.title, icon: item.icon, color: item.color }, size);
  }

  /// `one.api` for a client's own screen: the node's /v1 at this origin, as this person; nothing else here. What
  /// changes who can do what, and letting devices in, this browser does itself (`signs`).
  async function clientApi(m) {
    const url = new URL(String(m.path || ""), location.origin);
    if (url.origin !== location.origin || !url.pathname.startsWith("/v1/")) throw new Error("That isn't your device's.");
    const method = String(m.method || "GET").toUpperCase();
    const body = method === "GET" ? undefined : m.body ?? {};
    if (url.pathname === "/v1/call" && body && typeof body === "object") body.client = "web";
    const own = signs[`${method} ${url.pathname}`];
    if (own) {
      if (!device) throw new Error("This browser hasn't been approved yet.");
      const out = await own(body || {});
      await refreshState().catch(() => {});
      return out ?? {};
    }
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
    "POST /v1/name": (b) => write(members.rename({ what: "key", id: device.me.key, name: String(b.name || "").trim() })),
    "POST /v1/retire": (b) => write(members.retire({ app: b.app, owner: app(b.app)?.owner, how: b.how })),
    // An app on everyone's page is a change to who can do what; on yours or a group's, a node writes it.
    "POST /v1/install": (b) => (b.scope === "global" ? write(members.install({ app: b.app, installed: b.installed !== false })) : api("/v1/install", { method: "POST", body: b })),
    "POST /v1/invite-link": (b) => device.invite(String(b.name || "")),
    "POST /v1/grant": (b) => write(members.grant({ endpoint: b.endpoint, scope: b.scope, app: b.app, until: 0 })),
    "GET /v1/devices/passkey": async () => ({ passkey: !!passkeyToList() }),
    // As a machine's node answers it: without `confirm`, which device it is; with, it's let in.
    "POST /v1/devices/approve": async (b) => {
      const code = String(b.code || "").toUpperCase().replace(/[^A-Z2-7]/g, "");
      const r = (await device.requests()).find((x) => x.code.replace(" ", "") === code);
      if (!r) throw new Error("No device is waiting with that code. Check it, or start again on that device.");
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

  /// The passkey this tab signed in with, if it isn't listed yet: its PRF output stays in this tab only.
  function passkeyToList() {
    const prf = sessionStorage.getItem("one.prf"), credential = sessionStorage.getItem("one.prf.credential");
    if (!prf || !credential || install()?.clients.some((c) => c.passkey === credential)) return null;
    return { prf: Uint8Array.from(atob(prf.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)), credential };
  }

  /// "linux runs 3157ba4": what the device that answers runs, for telling builds apart.
  function deviceBuild() {
    const s = install()?.devices.find((x) => x.key === store.state?.device?.key);
    return s?.build ? `${s.name} runs ${s.build}` : null;
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

  /// Light or dark whatever the system says: a document's rules for the other are switched off and its own on, and
  /// "automatic" puts them back. Its source also runs in screens' frames (`look`), so it keeps to itself.
  function scheme() {
    const kept = new Map();
    return (doc, value) => {
      doc.documentElement.style.colorScheme = value === "automatic" ? "" : value;
      const walk = (rules) => {
        for (const rule of rules) {
          if (rule.media && /prefers-color-scheme/.test(kept.get(rule) ?? rule.media.mediaText)) {
            if (!kept.has(rule)) kept.set(rule, rule.media.mediaText);
            rule.media.mediaText = value === "automatic" ? kept.get(rule)
              : kept.get(rule).replace(/\(\s*prefers-color-scheme\s*:\s*(light|dark)\s*\)/g, (_, s) => (s === value ? "(min-width: 0px)" : "(max-width: 0px) and (min-width: 1px)"));
          }
          if (rule.cssRules) walk(rule.cssRules);
        }
      };
      for (const sheet of doc.styleSheets) {
        try { walk(sheet.cssRules); } catch {}
      }
    };
  }
  const schemed = scheme();

  // MARK: Starting

  function showSignedOut(error) {
    const name = config().name;
    document.title = name;
    root.innerHTML = `<div class="welcome"><img class="welcome-mark" src="/logo.png" alt=""><h1>${esc(name)}</h1>` +
      `<button class="button primary big" data-signin>Sign In</button>${error ? `<div class="error welcome-error">${esc(error)}</div>` : ""}</div>`;
    $("[data-signin]", root).onclick = async (e) => {
      e.target.disabled = true;
      try {
        await auth.start();
      } catch (err) {
        showSignedOut(err.message);
      }
    };
  }

  function showUnreachable(text) {
    const name = config().name;
    root.innerHTML = message(`Can’t Reach ${name}`, esc(text), `<div class="unavailable-text">It keeps trying.</div>`);
  }

  /// Makes this browser a device of its person: listed already, or let in by a passkey of theirs that's listed, or
  /// by one of their devices (or, for someone new, their inviter's), which it asks and waits for, showing its code.
  async function join(user) {
    const keys = await browserKeys(user);
    const listed = () => [...(install()?.devices || []), ...(install()?.clients || [])].some((e) => e.key === keys.key);
    // Once it's in: their own things' first key and where they run, made by the first of their devices.
    const made = async () => {
      device = new Device(keys, { state: () => store.state, api, bearer });
      await device.settleOwn().catch((err) => console.warn("making this person's own keys:", err));
      await refreshState().catch(() => {});
      return device;
    };
    if (listed()) return made();
    const prf = sessionStorage.getItem("one.prf"), credential = sessionStorage.getItem("one.prf.credential");
    if (prf && install()?.clients.some((c) => c.passkey === credential && c.user === user)) {
      try {
        await new Device(keys, { state: () => store.state, api, bearer }).approveByPasskey(Uint8Array.from(atob(prf.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)));
        await refreshState();
        if (listed()) return await net.start(user, true).catch(() => {}), made();
      } catch (err) {
        console.warn("the passkey didn't let this browser in:", err);
      }
    }
    const invite = sessionStorage.getItem("one.invite");
    const ask = async () => answer(await bearer("/api/auth/devices/requests", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await joinAsk(keys, browserName(), invite)) }));
    await ask();
    await showJoin(await codeOf(keys.key, keys.seal), !!invite);
    for (let n = 1; ; n++) {
      await new Promise((r) => setTimeout(r, 2000));
      await refreshState().catch(() => {});
      if (listed()) break;
      // Asked again once it stopped waiting (15 minutes), or was turned down.
      if (n % 10 === 0 && (await bearer(`/api/auth/devices/requests/${keys.key}`)).status === 404) await ask().catch(() => {});
    }
    sessionStorage.removeItem("one.invite");
    root.innerHTML = "";
    // Let in, it reaches its nodes itself from now on.
    await net.start(user, true).catch(() => {});
    return made();
  }

  /// "Approve This Browser": its code, and the code as a QR, a link that opens Approve on a phone that scans it.
  async function showJoin(code, invited) {
    const name = config().name;
    document.title = "Approve This Browser";
    const link = `${location.origin}/approve#${code.replace(" ", "")}`;
    root.innerHTML = `<div class="welcome join"><h1>Approve This Browser</h1>` +
      `<p class="join-text">${invited ? "Whoever invited you approves it. Show them this code." : `Open ${esc(name)} on a device you already use, then Settings › Devices › Approve.`}</p>` +
      `<div class="join-code" aria-label="Code">${esc(code)}</div><div class="join-qr">${qrSvg(link)}</div>` +
      `<p class="join-note">Or scan this with that device.</p><button class="button" data-out>Sign Out</button></div>`;
    $("[data-out]", root).onclick = () => auth.signOut();
  }

  async function start() {
    if (/^#[0-9a-f]{3,8}$/i.test(whiteLabel.accent)) document.documentElement.style.setProperty("--accent", whiteLabel.accent);
    schemed(document, appearance());
    // A new device's QR: Approve, with its code.
    if (location.pathname === "/approve") history.replaceState(null, "", href(own("devices", { approve: location.hash.slice(1) })));
    if (location.pathname === "/callback") {
      try {
        history.replaceState(null, "", await auth.finish());
      } catch (err) {
        history.replaceState(null, "", "/");
        return showSignedOut(err.message);
      }
    }
    if (!auth.kept()) return showSignedOut();
    root.innerHTML = `<div class="starting">${one.loading("Starting")}</div>`;
    joining = true;
    // Someone new isn't anyone a node answers until their first device is let in.
    let newcomer = null;
    for (let wait = 1000; ; wait = Math.min(wait * 2, 15_000)) {
      try {
        await refreshState();
        break;
      } catch (err) {
        if (err.signedOut) return showSignedOut();
        if (err.status === 403 && (newcomer = auth.user())) break;
        showUnreachable(err.message);
        await new Promise((r) => setTimeout(r, wait));
      }
    }
    if (newcomer || (install() && me())) {
      try {
        await join(newcomer || me().id);
      } catch (err) {
        if (err.signedOut) return showSignedOut();
        return void (root.innerHTML = message("Can’t Approve This Browser", esc(err.message)));
      }
    }
    joining = false;
    await refreshApps().catch(() => {});
    // `/vault?id=…&scope=…`: an agent's link to a request for an item, answered over You.
    let ask = null;
    if (location.pathname === "/vault") {
      const q = new URLSearchParams(location.search);
      ask = { kind: "vault", scope: q.get("scope"), id: q.get("id") };
      history.replaceState(null, "", "/");
    }
    render();
    if (ask?.id && ask.scope) openSheet(own("request", { scope: ask.scope, id: ask.id }));
    listen();
  }

  document.addEventListener("DOMContentLoaded", start);
})();
