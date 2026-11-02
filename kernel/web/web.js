// The web client: the Mac app's window, in a browser. It signs in as an OAuth client of the install's auth service
// (PKCE, and the person's passkey at its sign-in page), keeps its refresh token in this browser, and calls a node's `/v1` on its
// own origin with the access token; the gateway sends each on. App screens are untrusted: each runs in a sandboxed
// frame, and this page answers its bridge as the Mac app's web view does, making each call itself. Nothing is cached.

// The bridge and kit (src/screen/), loaded after this, give `one`; this page answers it for its own use (`__one.host`).
window.__one = {};

(() => {
  const $ = (sel, el = document) => el.querySelector(sel);
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const b64u = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  const random = (n) => b64u(crypto.getRandomValues(new Uint8Array(n)));

  // MARK: Signing in

  const KEPT = "one.signin";
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
      if (!r.ok || !out.client_id) throw new Error(out.error_description || out.message || "This browser couldn't sign up to sign in.");
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
          if (err.signedOut) localStorage.removeItem(KEPT);
          throw err;
        }
      };
      return navigator.locks ? navigator.locks.request("one.refresh", refresh) : refresh();
    },

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
      location.href = "/";
    },
  };

  /// What the person's signed-in clients call this browser: "Chrome on Mac".
  function browserName() {
    const ua = navigator.userAgent;
    const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "A browser";
    const os = /iPhone|iPad/.test(ua) ? "iPhone" : /Mac OS X/.test(ua) ? "Mac" : /Android/.test(ua) ? "Android" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "";
    return os ? `${browser} on ${os}` : browser;
  }

  // MARK: The node

  class NodeError extends Error {}

  /// A request to the node, as this person: JSON in and out, or with `raw` the response itself.
  async function api(path, { method = "GET", body, query, raw } = {}) {
    const url = new URL(path, location.origin);
    for (const [k, v] of Object.entries(query || {})) if (v != null) url.searchParams.set(k, typeof v === "string" ? v : JSON.stringify(v));
    let token = await auth.token();
    for (let tries = 0; ; tries++) {
      const headers = { authorization: `Bearer ${token}` };
      if (body !== undefined) headers["content-type"] = "application/json";
      const r = await fetch(url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body) });
      if (r.status === 401 && tries === 0) {
        token = await auth.token(token);
        continue;
      }
      if (r.status === 401) throw Object.assign(new NodeError("Signed out."), { signedOut: true });
      if (raw && r.ok) return r;
      const out = await r.json().catch(() => null);
      if (!r.ok || out?.error) throw new NodeError(out?.error || (r.status >= 500 ? "Your server isn't answering." : `It didn't work (${r.status}).`));
      return out;
    }
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

  const store = {
    state: null,
    apps: [],
    widgets: {},
    badges: {},
    failure: null,
    /// A sheet over the window: `{ kind: "services" | "vault" | "choose", … }`.
    sheet: null,
  };

  const me = () => store.state?.user;
  const install = () => store.state?.install;
  const isAdmin = () => me()?.role === "admin";
  const config = () => store.state?.config || {};
  const app = (name) => store.apps.find((a) => a.name === name);
  const label = (a) => a?.title || a?.name || "";
  const has = (a, screen) => !!a?.screens?.some((s) => s.name === screen);
  const hasScreens = (a) => has(a, "main");
  const isService = (a) => !hasScreens(a) && !(a.widgets || []).length;
  const scopes = (a) => (a.installed || []).map((s) => (s.startsWith("user:") ? "user" : s));
  const userName = (id) => (id === me()?.id ? "You" : install()?.users.find((u) => u.id === id)?.name || String(id).slice(0, 6));

  function people(n) {
    return n === 1 ? "1 person" : `${n} people`;
  }

  function chosenBy(admins) {
    const names = admins.slice().sort((a, b) => (a === me()?.id) - (b === me()?.id) || userName(a).localeCompare(userName(b)))
      .map((id) => (id === me()?.id ? "you" : userName(id)));
    if (!names.length) return "apps chosen by admins";
    if (names.length === 1) return `apps chosen by ${names[0]}`;
    if (names.length === 2) return `apps chosen by ${names[0]} and ${names[1]}`;
    return `apps chosen by ${names.length} admins`;
  }

  /// Every page this person can read: theirs, their groups', then everyone's.
  function pages() {
    const i = install(), u = me();
    if (!i || !u) return [];
    const devices = i.servers.filter((s) => s.user === u.id).length + i.clients.filter((c) => c.user === u.id).length;
    const out = [{ scope: "user", title: "You", subtitle: devices === 1 ? "Just you · 1 device" : `Just you · ${devices} devices`, whose: "For you only" }];
    for (const g of i.groups.filter((g) => g.members.some((m) => m.user === u.id)).sort((a, b) => a.name.localeCompare(b.name))) {
      const admins = g.members.filter((m) => m.role === "admin").map((m) => m.user);
      out.push({ scope: `group:${g.id}`, title: g.name, subtitle: `${people(g.members.length)} · ${chosenBy(admins)}`, whose: `For ${g.name}`, group: g.id });
    }
    out.push({ scope: "global", title: "Everyone", subtitle: `${people(i.users.length)} · ${chosenBy(i.users.filter((x) => x.role === "admin").map((x) => x.id))}`, whose: "For everyone" });
    return out;
  }
  const page = (scope) => pages().find((p) => p.scope === scope);

  /// Whether this person may install into a scope: their own, a group they admin, or everyone's as an admin.
  function canArrange(scope) {
    if (scope === "user") return true;
    if (scope === "global") return isAdmin();
    const g = install()?.groups.find((g) => `group:${g.id}` === scope);
    return g?.members.some((m) => m.user === me()?.id && m.role === "admin") || false;
  }

  const appsOn = (p) => store.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
  const servicesOn = (p) => store.apps.filter((a) => isService(a) && scopes(a).includes(p.scope)).sort((a, b) => label(a).localeCompare(label(b)));
  const appBadge = (name, p) => Math.max(0, store.badges[p.scope]?.[name] || 0);

  function owns(a) {
    const owner = a.owner, u = me();
    if (!owner || !u) return false;
    if (owner === "admins") return isAdmin();
    if (owner === `user:${u.id}`) return true;
    const g = owner.startsWith("group:") && install()?.groups.find((g) => g.id === owner.slice(6));
    return !!g && g.members.some((m) => m.user === u.id && m.role === "admin");
  }

  /// Every widget a page could show: those of the apps installed in its scope.
  function widgetsOn(p) {
    const here = new Set(store.apps.filter((a) => scopes(a).includes(p.scope)).map((a) => a.name));
    return (store.widgets[p.scope] || []).filter((w) => here.has(w.app));
  }

  /// What a page shows, in its person's order: the widgets they put there and its apps' icons, those installed since
  /// they last arranged it at the end, without those they took off it. A page never arranged shows its apps.
  function items(p) {
    const apps = appsOn(p), widgets = widgetsOn(p);
    const arrangement = store.state?.arrangements?.[p.scope];
    if (!arrangement) return apps.map((a) => ({ app: a }));
    const out = [], seen = new Set();
    for (const item of arrangement.items || []) {
      if (item.widget) {
        const w = widgets.find((w) => w.app === item.app && w.widget === item.widget);
        if (!item.id || !w || seen.has(`widget:${item.id}`)) continue;
        seen.add(`widget:${item.id}`);
        out.push({ widget: placed(w, item), kept: item });
      } else {
        const a = apps.find((a) => a.name === item.app);
        if (a && !seen.has(`app:${a.name}`)) {
          seen.add(`app:${a.name}`);
          out.push({ app: a });
        }
      }
    }
    const hidden = new Set(arrangement.hidden || []);
    return out.concat(apps.filter((a) => !seen.has(`app:${a.name}`) && !hidden.has(a.name)).map((a) => ({ app: a })));
  }

  /// A widget as one copy of it on a page draws: at its size, with the options chosen for it.
  function placed(w, item) {
    const copy = { ...w, instance: item.id, sizes: { ...(w.sizes || {}) } };
    if (item.size && supported(w).includes(item.size)) copy.size = item.size;
    const mine = item.id && w.placed?.[item.id];
    if (mine?.sizes) Object.assign(copy.sizes, mine.sizes);
    if (mine?.error) copy.error = mine.error;
    return copy;
  }

  const SIZES = ["small", "medium", "large", "xl"];
  const CELLS = { small: [2, 2], medium: [4, 2], large: [4, 4], xl: [8, 4] };
  const supported = (w) => SIZES.filter((s) => !w.sizes || w.sizes[s]);
  const tree = (w, size) => w.sizes?.[size] || w.root;

  /// Loads what the window shows: the node's state, then its apps, widgets and badges.
  async function refreshState() {
    const state = await api("/v1/state");
    const regrouped = JSON.stringify([state.user, state.install?.groups]) !== JSON.stringify([store.state?.user, store.state?.install?.groups]);
    const first = !store.state;
    store.state = state;
    store.failure = null;
    if (first || regrouped || !store.apps.length) await refreshApps();
  }

  async function refreshApps() {
    if (!install()) return;
    store.apps = await api("/v1/apps");
    await refreshWidgets();
  }

  async function refreshWidgets() {
    const all = pages();
    await Promise.all(all.map(async (p) => {
      store.widgets[p.scope] = await api("/v1/widgets", { query: { scope: p.scope } }).catch(() => store.widgets[p.scope] || []);
      store.badges[p.scope] = await api("/v1/badges", { query: { scope: p.scope } }).catch(() => ({}));
    }));
    render();
    scheduleRedraw();
  }

  /// Draws the widgets again when the soonest asks to be (a tree's `next`), between a minute and fifteen from now.
  let redraw;
  function scheduleRedraw() {
    clearTimeout(redraw);
    const nexts = [];
    const walk = (n) => { if (n && typeof n === "object") { if (n.next) nexts.push(n.next); (n.c || []).forEach(walk); } };
    Object.values(store.widgets).flat().forEach((w) => Object.values(w.sizes || { root: w.root }).forEach(walk));
    if (!nexts.length) return;
    const wait = Math.min(15 * 60_000, Math.max(60_000, Math.min(...nexts) - Date.now()));
    redraw = setTimeout(() => refreshWidgets().catch(() => {}), wait);
  }

  /// The node's events: what changed, so the window and the open screen keep up. EventSource can't send a token, so
  /// it's a fetch that streams.
  async function listen() {
    for (;;) {
      try {
        const r = await api("/v1/events", { raw: true });
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
      await new Promise((r) => setTimeout(r, 3000));
    }
  }

  async function handle(event) {
    screen.event(event);
    if (["servers", "install"].includes(event.type)) await refreshState();
    if (event.type === "changed") {
      if (!(event.apps || []).length) await refreshState();
      await refreshApps();
    }
    // An app's own change (a sign-in finished on its settings screen) lists its accounts again.
    if (event.type === "changed") redrawServices(event.apps?.length ? event.apps : undefined);
  }

  // MARK: Where the window is

  /// The URL is the place: `/` (You), `/page/<scope>`, `/services/<scope>`, `/app/<app>/<screen>?params=&scope=`,
  /// `/settings`, `/settings/<item>?params=` (a Settings screen); and `/vault?id=&scope=`, a request for an item, as a
  /// sheet over You.
  function route() {
    const path = location.pathname.split("/").filter(Boolean).map(decodeURIComponent);
    const q = new URLSearchParams(location.search);
    if (path[0] === "page" && path[1]) return { kind: "page", scope: path[1] };
    if (path[0] === "services" && path[1]) return { kind: "services", scope: path[1] };
    if (path[0] === "app" && path[1]) return { kind: "screen", app: path[1], screen: path[2] || "main", params: q.get("params") || "{}", scope: q.get("scope") || undefined };
    if (path[0] === "settings") return path[1] ? { kind: "setting", name: path[1], params: q.get("params") || "{}" } : { kind: "settings" };
    return { kind: "page", scope: "user" };
  }

  function href(r) {
    if (r.kind === "page") return r.scope === "user" ? "/" : `/page/${encodeURIComponent(r.scope)}`;
    if (r.kind === "services") return `/services/${encodeURIComponent(r.scope)}`;
    if (r.kind === "settings") return "/settings";
    const q = new URLSearchParams();
    if (r.kind === "setting") {
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

  const screenRoute = (a, p, screen = "main", params = {}) => ({ kind: "screen", app: a.name, screen, params: JSON.stringify(params), scope: p?.scope });
  const settingsRoute = (a, p, add) => (has(a, "settings") ? screenRoute(a, p, "settings", add ? { add: true } : {}) : null);
  function storeRoute(a) {
    const s = storeApp();
    return s && has(s, "app") && (a.deploys || 0) > 0 ? { kind: "screen", app: s.name, screen: "app", params: JSON.stringify({ app: a.name }) } : null;
  }
  function serviceStoreRoute() {
    const s = storeApp();
    return s ? { kind: "screen", app: s.name, screen: "main", params: JSON.stringify({ service: true }) } : null;
  }

  // MARK: Drawing

  const root = $("#root");

  function render() {
    if (!store.state) return;
    if (!install()) return void (root.innerHTML = message("Nothing Here Yet", `This server isn't in an install yet. Set it up from the ${esc(config().name || "app")} app on one of your devices.`));
    if (!$(".window", root)) {
      root.innerHTML = `<div class="window"><nav class="sidebar"></nav><main class="detail"></main></div>`;
      new ResizeObserver(() => layoutGrid()).observe($(".detail", root));
    }
    drawSidebar();
    drawDetail();
  }

  function message(title, text, extra = "") {
    return `<div class="unavailable"><div class="unavailable-title">${esc(title)}</div><div class="unavailable-text">${text}</div>${extra}</div>`;
  }

  const icon = (a, size) => one.appIcon(a, size);

  /// A page's Services folder: its services' icons, nine at most, small, on frosted glass; with none, nine faint
  /// empty places.
  function folder(apps, size) {
    const cells = apps.length ? apps.slice(0, 9).map((a) => one.appIcon(a, size / 4)).join("") : `<i></i>`.repeat(9);
    return `<span class="folder" style="--size:${size}px">${cells}</span>`;
  }

  function badge(n) {
    return n > 0 ? `<span class="badge">${n > 99 ? "99+" : n}</span>` : "";
  }

  /// One section per page, listing its apps and its Services; then Settings', listing its items.
  function drawSidebar() {
    const at = selection();
    const selected = (x) => (href(x) === at ? " selected" : "");
    const sections = pages().map((p) => {
      const rows = appsOn(p).map((a) => {
        const to = screenRoute(a, p);
        return `<a class="nav-row${selected(to)}" href="${esc(href(to))}" data-go='${esc(JSON.stringify(to))}'>${icon(a, 20)}<span class="nav-label">${esc(label(a))}</span>${appBadge(a.name, p) ? `<span class="nav-count">${appBadge(a.name, p)}</span>` : ""}</a>`;
      }).join("");
      const services = { kind: "services", scope: p.scope };
      const pageRoute = { kind: "page", scope: p.scope };
      return `<section><a class="nav-head${selected(pageRoute)}" href="${esc(href(pageRoute))}" data-go='${esc(JSON.stringify(pageRoute))}'>${esc(p.title)}</a>${rows}` +
        `<a class="nav-row${selected(services)}" href="${esc(href(services))}" data-go='${esc(JSON.stringify(services))}'>${folder(servicesOn(p), 20)}<span class="nav-label">Services</span></a></section>`;
    }).join("");
    const settings = { kind: "settings" };
    const items = settingsItems().map((item) => {
      const to = { kind: "setting", name: item.name };
      return `<a class="nav-row${selected(to)}" href="${esc(href(to))}" data-go='${esc(JSON.stringify(to))}'>${settingIcon(item, 20)}<span class="nav-label">${esc(item.title)}</span>${item.badge ? `<span class="nav-count">${item.badge}</span>` : ""}</a>`;
    }).join("");
    $(".sidebar", root).innerHTML = `<div class="nav-pages">${sections}<section><a class="nav-head${selected(settings)}" href="/settings" data-go='${esc(JSON.stringify(settings))}'>Settings</a>${items}</section></div>`;
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

  function drawDetail() {
    const r = route();
    const detail = $(".detail", root);
    const key = JSON.stringify(r);
    // An open screen stays as it is while the rest of the window redraws around it.
    if ((r.kind === "screen" || r.kind === "setting") && shown === key && $(".screen-frame", detail)) return;
    const again = shown === key;
    shown = key;
    if (r.kind === "page") {
      const p = page(r.scope) || pages()[0];
      if (p) drawPage(detail, p);
      else detail.innerHTML = message("Nothing Here Yet", "This browser was let into none of your pages. Sign out in Settings and sign in again to choose them.");
    }
    else if (r.kind === "services") {
      const p = page(r.scope);
      // Drawn again in place, keeping the accounts it listed, as what's on the page changes.
      const there = again && $(".services-view", detail);
      if (there && p) return void drawServices(there, p);
      detail.innerHTML = bar("Services", p?.whose) + `<div class="scroll"><div class="services-view"></div></div>`;
      if (p) drawServices($(".services-view", detail), p);
    } else if (r.kind === "settings") drawSettings(detail);
    else screen.show(detail, settingsApp(r) || r);
  }

  /// The detail's bar: its title and subtitle, as a Mac window's toolbar, with Back when a screen was pushed.
  function bar(title, subtitle = "", end = "") {
    const back = history.state?.back ? `<button class="back" data-back aria-label="Back">‹</button>` : "";
    return `<header class="bar">${back}<div class="bar-titles"><div class="bar-title">${esc(title)}</div>${subtitle ? `<div class="bar-subtitle">${esc(subtitle)}</div>` : ""}</div><div class="bar-end">${end}</div></header>`;
  }

  // MARK: Pages

  /// A page: its title, then one grid, as the Home Screen's: the widgets its person put there and its apps' icons,
  /// in their order, then the Services folder.
  function drawPage(detail, p) {
    document.title = p.title;
    const list = items(p);
    const empty = !appsOn(p).length && !widgetsOn(p).length;
    detail.innerHTML = bar("") + `<div class="scroll"><div class="page"><div class="page-header"><h1>${esc(p.title)}</h1><div class="page-subtitle">${esc(p.subtitle)}</div></div>` +
      (empty ? `<div class="empty-page"><div class="empty-page-title">Nothing here yet</div><div class="empty-page-text">${canArrange(p.scope) ? `Apps you add to ${esc(p.title)} show up here.` : `When its admins add apps to ${esc(p.title)}, they show up here.`}</div>${canArrange(p.scope) && storeApp() ? `<button class="button primary" data-store>Open the App Store</button>` : ""}</div>` : "") +
      `<div class="grid"></div></div></div>`;
    $("[data-store]", detail)?.addEventListener("click", () => openStore());
    const grid = $(".grid", detail);
    grid.__items = list.map((item) => {
      if (item.app) {
        const a = item.app, to = screenRoute(a, p);
        const el = document.createElement("a");
        el.className = "tile";
        el.href = href(to);
        el.dataset.go = JSON.stringify(to);
        el.innerHTML = `<span class="tile-icon">${icon(a, 60)}${badge(appBadge(a.name, p))}</span><span class="tile-name">${esc(label(a))}</span>`;
        el.addEventListener("contextmenu", (e) => { e.preventDefault(); menu(e, actions(a, p)); });
        return { el, span: [1, 1] };
      }
      const w = item.widget, a = app(w.app);
      const size = w.size || "medium";
      const el = oneWidgets.card({ ...w, title: w.title || label(a) || w.widget }, widgetHost(p), tree(w, size));
      el.classList.add("widget");
      el.dataset.size = size;
      // A widget without a link of its own opens its app.
      if (!tree(w, size)?.link && a && hasScreens(a)) el.addEventListener("click", (e) => { if (!e.defaultPrevented) go(screenRoute(a, p)); });
      el.addEventListener("contextmenu", (e) => { e.preventDefault(); menu(e, widgetActions(w, item.kept, p, size)); });
      return { el, span: CELLS[size] || CELLS.medium, widget: w };
    });
    const services = { kind: "services", scope: p.scope };
    const f = document.createElement("a");
    f.className = "tile";
    f.href = href(services);
    f.innerHTML = `<span class="tile-icon">${folder(servicesOn(p), 60)}</span><span class="tile-name">Services</span>`;
    f.addEventListener("click", (e) => { if (e.metaKey || e.ctrlKey) return; e.preventDefault(); openSheet({ kind: "services", scope: p.scope }); });
    grid.__items.push({ el: f, span: [1, 1] });
    grid.append(...grid.__items.map((i) => i.el));
    layoutGrid();
  }

  /// What a page's widgets act through: their app's operations, and their links.
  function widgetHost(p) {
    return {
      async run(appName, op, input) {
        try {
          await call(appName, op, input);
        } catch (err) {
          return err.message;
        }
        await refreshWidgets();
      },
      follow(link, w) {
        if (typeof link === "string") return void (/^https?:/i.test(link) && window.open(link, "_blank", "noopener,noreferrer"));
        if (link?.screen) go({ kind: "screen", app: w.app, screen: link.screen, params: JSON.stringify(link.params || {}), scope: p.scope });
      },
    };
  }

  /// Lays a page's grid out as the Home Screen does: square cells an icon wide, as many across as fit; each item in
  /// the first place it fits, row by row, widgets starting on an even column; an xl widget where it doesn't fit is large.
  function layoutGrid() {
    const grid = $(".detail .grid", root);
    if (!grid?.__items) return;
    // A phone's is four across its width; wider, cells stay an icon's size and more fit across.
    const narrow = grid.clientWidth < 500;
    const gap = narrow ? 12 : 14;
    const cell = narrow ? Math.max(1, (grid.clientWidth - gap * 3) / 4) : 82;
    const columns = narrow ? 4 : Math.max(4, Math.floor((grid.clientWidth + gap) / (cell + gap)));
    grid.style.gap = `${gap}px`;
    grid.style.gridAutoRows = `${cell}px`;
    grid.style.setProperty("--icon", `${Math.max(40, Math.min(60, cell - 19))}px`);
    const taken = [];
    for (const item of grid.__items) {
      let [w, h] = item.span;
      if (item.widget && w > columns) {
        [w, h] = CELLS.large;
        item.el.replaceWith(item.el = refit(item, "large"));
      }
      for (let y = 0; ; y++) {
        let found = -1;
        for (let x = 0; x <= columns - w; x += w > 1 ? 2 : 1) {
          let free = true;
          for (let row = y; row < y + h && free; row++) for (let col = x; col < x + w; col++) if (taken[row]?.[col]) { free = false; break; }
          if (free) { found = x; break; }
        }
        if (found < 0) continue;
        for (let row = y; row < y + h; row++) for (let col = found; col < found + w; col++) (taken[row] ||= [])[col] = true;
        item.el.style.gridColumn = `${found + 1} / span ${w}`;
        item.el.style.gridRow = `${y + 1} / span ${h}`;
        break;
      }
    }
    grid.style.gridTemplateColumns = `repeat(${columns}, ${cell}px)`;
  }

  function refit(item, size) {
    const w = item.widget, a = app(w.app), p = page(route().scope) || pages()[0];
    item.span = CELLS[size];
    const el = oneWidgets.card({ ...w, title: w.title || label(a) || w.widget }, widgetHost(p), tree(w, size));
    el.classList.add("widget");
    el.dataset.size = size;
    return el;
  }

  /// The install's App Store, if it's on a page this browser's sign-in opens.
  function storeApp() {
    const s = app(config().store);
    return s && hasScreens(s) && pages().some((p) => scopes(s).includes(p.scope)) ? s : null;
  }

  function openStore() {
    const s = storeApp();
    if (s) go({ kind: "screen", app: s.name, screen: "main", params: "{}" });
  }

  // MARK: Services

  /// A page's Services: a section per app that keeps accounts, its accounts each with a dot saying whether it's
  /// signed in, then Add Account; then the rest, a row each opening its page in the App Store; then Add a Service.
  async function drawServices(el, p, inSheet) {
    const all = servicesOn(p), withAccounts = all.filter((a) => (a.ops || []).some((o) => o.name === "accounts")), others = all.filter((a) => !withAccounts.includes(a));
    const accounts = el.__accounts ||= {};
    const opened = (to) => () => { if (inSheet) closeSheet(); go(to); };
    const sections = withAccounts.map((a) => {
      const list = accounts[a.name];
      const rows = list === undefined ? `<div class="row plain muted"><span class="spinner small"></span>Loading…</div>`
        : list instanceof Error ? `<div class="row plain warn-text">${esc(list.message)}</div>`
        : !list.length ? `<div class="row plain muted">No accounts yet</div>`
        : list.map((x) => `<div class="row plain account"><span class="status ${x.ok ? "ok" : "off"}" title="${x.ok ? "Signed in" : "Needs signing in again"}"></span><div class="main"><div class="title"><span>${esc(x.account)}</span></div>${x.ok ? "" : `<div class="sub wrap">${esc(x.why || "Needs signing in again")}</div>`}</div>${x.ok || !settingsRoute(a, p, true) ? "" : `<button class="button plain" data-signin="${esc(a.name)}">Sign In</button>`}</div>`).join("");
      const add = settingsRoute(a, p, true) ? `<button class="row plain add" data-add="${esc(a.name)}"><span class="plus">+</span>Add Account</button>` : "";
      return `<div class="service-head">${icon(a, 28)}<span class="service-title">${esc(label(a))}</span><button class="more" data-more="${esc(a.name)}" aria-label="More for ${esc(label(a))}">⋯</button></div><div class="list">${rows}${add}</div>`;
    }).join("");
    const rest = others.length ? `<div class="list others">${others.map((a) => {
      const to = storeRoute(a) || settingsRoute(a, p);
      return `<button class="row${to ? "" : " still"}" data-open="${esc(a.name)}">${icon(a, 28)}<div class="main"><div class="title"><span>${esc(label(a))}</span></div>${a.description ? `<div class="sub">${esc(a.description)}</div>` : ""}</div>${to ? `<span class="chev">›</span>` : ""}</button>`;
    }).join("")}</div>` : "";
    const addService = canArrange(p.scope) && serviceStoreRoute() ? `<div class="list"><button class="row plain add" data-add-service><span class="plus">+</span>Add a Service</button></div>` : "";
    el.innerHTML = sections + rest + (all.length ? "" : `<div class="muted none">No services yet.</div>`) + addService;
    el.querySelectorAll("[data-add]").forEach((b) => (b.onclick = () => addAccount(app(b.dataset.add), p, opened(settingsRoute(app(b.dataset.add), p, true)))));
    el.querySelectorAll("[data-signin]").forEach((b) => (b.onclick = opened(settingsRoute(app(b.dataset.signin), p, true))));
    el.querySelectorAll("[data-open]").forEach((b) => {
      const a = app(b.dataset.open), to = storeRoute(a) || settingsRoute(a, p);
      if (to) b.onclick = opened(to);
    });
    el.querySelectorAll("[data-more]").forEach((b) => (b.onclick = () => menu(b, actions(app(b.dataset.more), p, inSheet))));
    $("[data-add-service]", el)?.addEventListener("click", opened(serviceStoreRoute()));
    // Each app's accounts, listed once; a change to the app lists them again.
    for (const a of withAccounts) {
      if (accounts[a.name] !== undefined && !el.__stale?.has(a.name)) continue;
      el.__stale?.delete(a.name);
      call(a.name, "accounts", {}).then((list) => (Array.isArray(list) ? list : Promise.reject(new Error(`${label(a)} doesn't list its accounts yet.`))))
        .catch((err) => err).then((out) => {
          accounts[a.name] = out;
          if (el.isConnected) drawServices(el, p, inSheet);
        });
    }
  }

  /// Add Account: the person's accounts in the Vault this app wants and doesn't use yet, to switch it on for one;
  /// with none, its sign-in.
  async function addAccount(a, p, signIn) {
    const keeps = Object.values(a.vault || {}).some((need) => need.accounts);
    const items = keeps ? await api("/v1/vault").catch(() => []) : [];
    const there = items.filter((i) => (i.wanted_by || []).includes(a.name) && !(i.apps || []).includes(a.name));
    if (!there.length) return signIn();
    const { body, close } = one.sheet({ title: "Add Account", done: "Cancel" });
    body.innerHTML = `<div class="list">${there.map((i) => `<button class="row" data-item="${esc(i.id)}">${itemIcon(i)}<div class="main"><div class="title"><span>${esc(i.title)}</span></div>${i.type && i.type !== i.title ? `<div class="sub">${esc(i.type)}</div>` : ""}</div></button>`).join("")}</div>` +
      `<div class="list"><button class="row plain add" data-other><span class="plus">+</span>Sign In to Another Account</button></div>`;
    body.querySelectorAll("[data-item]").forEach((b) => (b.onclick = async () => {
      const item = there.find((i) => i.id === b.dataset.item);
      b.disabled = true;
      await api("/v1/vault", { method: "POST", body: { scope: item.scope, id: item.id, apps: [...item.apps, a.name] } }).catch((err) => alertError(err));
      close();
      redrawServices([a.name]);
    }));
    $("[data-other]", body).onclick = () => { close(); signIn(); };
  }

  function itemIcon(i) {
    return one.appIcon({ name: i.title, title: i.title, image: i.image, image_kind: "icon" }, 30);
  }

  function redrawServices(names) {
    for (const el of document.querySelectorAll(".services-view")) {
      (el.__stale ||= new Set());
      for (const n of names || Object.keys(el.__accounts || {})) el.__stale.add(n);
    }
    if (store.sheet?.kind === "services") drawSheet();
    else if (route().kind === "services" && $(".detail .services-view", root) && page(route().scope)) drawServices($(".detail .services-view", root), page(route().scope));
  }

  /// What an app's ⋯ menu offers: its settings, its page in the App Store, taking it off this page, and deleting it.
  function actions(a, p, inSheet) {
    const out = [];
    const goTo = (to) => () => { if (inSheet) closeSheet(); go(to); };
    const settings = settingsRoute(a, p);
    if (settings) out.push({ label: "Settings", run: goTo(settings) });
    const inStore = storeRoute(a);
    if (inStore) out.push({ label: "Open in App Store", run: goTo(inStore) });
    if (canArrange(p.scope) && (a.deploys || 0) > 0) {
      out.push({ label: `Remove from ${p.title}`, danger: true, run: () => change(() => api("/v1/install", { method: "POST", body: { app: a.name, scope: p.scope, installed: false } })) });
    }
    if (owns(a) && (a.deploys || 0) > 0) {
      out.push({ label: "Delete App…", danger: true, run: () => confirmDelete(a) });
    }
    return out;
  }

  const SIZE_NAMES = { small: "Small", medium: "Medium", large: "Large", xl: "Extra Large" };

  /// What right-clicking a widget offers, as touching and holding one does on a Mac: its sizes, then Copy What Went
  /// Wrong if it broke, and Remove Widget.
  function widgetActions(w, kept, p, size) {
    const out = supported(w).map((s) => ({ label: `${s === size ? "✓ " : ""}${SIZE_NAMES[s]}`, run: () => s !== size && arrange(p, (item) => (item.id === kept.id ? { ...item, size: s } : item)) }));
    if (w.error) out.push({ label: "Copy What Went Wrong", run: () => navigator.clipboard.writeText(w.error) });
    out.push({ label: "Remove Widget", danger: true, run: () => arrange(p, (item) => (item.id === kept.id ? null : item)) });
    return out;
  }

  /// Keeps a page in its person's order with `edit` applied to each item (null takes it off), then draws it again.
  async function arrange(p, edit) {
    const kept = items(p).map((i) => (i.app ? { app: i.app.name } : i.kept)).map(edit).filter(Boolean);
    const hidden = store.state?.arrangements?.[p.scope]?.hidden || [];
    store.state.arrangements = { ...(store.state.arrangements || {}), [p.scope]: { items: kept, hidden } };
    render();
    try {
      await api("/v1/arrange", { method: "POST", body: { page: p.scope, items: kept, hidden } });
    } catch (err) {
      alertError(err);
    }
  }

  function confirmDelete(a) {
    const { body, close } = one.sheet({ title: `Delete ${label(a)}?`, done: "Cancel" });
    body.innerHTML = `<p class="sheet-text">${esc(label(a))} leaves every page, for everyone, and stops running. It waits in the App Store’s Recently Deleted, with its data, until its owners erase it.</p><div class="sheet-actions"><button class="button danger" data-delete>Delete App</button></div>`;
    $("[data-delete]", body).onclick = () => {
      close();
      change(() => api("/v1/retire", { method: "POST", body: { app: a.name, how: "deleted" } }));
    };
  }

  /// Runs a change, then takes in what it changed; says what went wrong if it failed.
  async function change(work) {
    try {
      await work();
      await refreshState();
      await refreshApps();
    } catch (err) {
      alertError(err);
    }
  }

  function alertError(err) {
    const { body } = one.sheet({ title: "Couldn’t Do That", done: "OK" });
    body.innerHTML = `<p class="sheet-text">${esc(err.message || err)}</p>`;
  }

  /// A small menu by `anchor` (a button, or a right-click), closed by a choice, a click elsewhere or Escape.
  function menu(anchor, entries) {
    document.querySelector(".menu")?.remove();
    if (!entries.length) return;
    const m = document.createElement("div");
    m.className = "menu";
    m.innerHTML = entries.map((e, i) => `<button class="menu-item${e.danger ? " danger" : ""}" data-i="${i}">${esc(e.label)}</button>`).join("");
    document.body.append(m);
    // By a button, under its right edge; at a right-click, where it was.
    const at = anchor instanceof Event ? { top: anchor.clientY, left: anchor.clientX } : (({ bottom, right }) => ({ top: bottom + 4, left: right - m.offsetWidth }))(anchor.getBoundingClientRect());
    m.style.top = `${Math.max(8, Math.min(at.top, innerHeight - m.offsetHeight - 8))}px`;
    m.style.left = `${Math.max(8, Math.min(at.left, innerWidth - m.offsetWidth - 8))}px`;
    const close = () => { m.remove(); document.removeEventListener("mousedown", outside, true); document.removeEventListener("keydown", key); };
    const outside = (e) => { if (!m.contains(e.target)) close(); };
    const key = (e) => e.key === "Escape" && close();
    setTimeout(() => document.addEventListener("mousedown", outside, true));
    document.addEventListener("keydown", key);
    m.querySelectorAll("[data-i]").forEach((b) => (b.onclick = () => { close(); entries[b.dataset.i].run(); }));
  }

  // MARK: Sheets

  let sheetHandle = null;

  function openSheet(sheet) {
    closeSheet();
    store.sheet = sheet;
    if (sheet.kind === "services") {
      const p = page(sheet.scope);
      sheetHandle = one.sheet({ title: "Services", onClose: () => { store.sheet = null; sheetHandle = null; } });
      sheetHandle.body.innerHTML = `<div class="sheet-sub">${esc(p?.whose || "")}</div><div class="services-view"></div>`;
      sheetHandle.body.closest(".sheet").classList.add("services-sheet");
      drawSheet();
    }
    if (sheet.kind === "vault") vault.open(sheet);
  }

  function drawSheet() {
    if (store.sheet?.kind !== "services" || !sheetHandle) return;
    const p = page(store.sheet.scope);
    if (p) drawServices($(".services-view", sheetHandle.body), p, true);
  }

  function closeSheet() {
    sheetHandle?.close();
    sheetHandle = null;
  }

  // MARK: The Vault's requests

  /// A request for an item an app may use: who asks and why, then the person's items that have the fields it wants,
  /// to pick one, or a new one, whose values go to the node and nowhere else. The asker never sees a secret.
  const vault = {
    async open({ scope, id, done }) {
      let request, picked = null, error = "", busy = false, finished = false;
      const values = {};
      const handle = one.sheet({ title: "Vault", done: "Cancel", onClose: () => { finished = true; done?.(); } });
      handle.body.closest(".sheet").classList.add("vault-sheet");
      const draw = () => {
        const b = handle.body;
        if (!request) {
          b.innerHTML = error ? message("Can’t Find This Request", esc(error)) : one.loading();
          return;
        }
        const a = app(request.app);
        const wanted = request.fields || (request.name ? [{ name: request.name, secret: true }] : []);
        const what = request.title || request.name || "";
        const asker = request.agent || request.by;
        let html = `<div class="list"><div class="row vault-who">${one.appIcon(a || { name: request.app }, 44)}<div class="main"><div class="title"><span>${esc(a ? `${label(a)} wants ${what}` : what)}</span></div>${request.why ? `<div class="sub wrap">${esc(request.why)}</div>` : ""}</div></div></div>` +
          `<div class="footer">${esc(request.agent ? `${asker} · ${request.scope_name}` : request.scope_name)}</div>`;
        if (request.state === "waiting") {
          html += `<div class="list">${(request.matches || []).map((m) => `<button class="row" data-pick="${esc(m.id)}">${itemIcon({ title: m.title })}<div class="main"><div class="title"><span>${esc(m.title)}</span></div>${m.type ? `<div class="sub">${esc(m.type)}</div>` : ""}</div>${picked === m.id ? `<span class="check">✓</span>` : ""}</button>`).join("")}` +
            `<button class="row" data-pick=""><span class="new-item">+</span><div class="main"><div class="title"><span>New Item</span></div></div>${picked === null ? `<span class="check">✓</span>` : ""}</button></div>`;
          if (picked === null) {
            html += `<div class="list fields"><label class="row plain"><span class="field-name">Title</span><input data-title value="${esc(values._title ?? what)}"></label>` +
              wanted.map((f) => `<label class="row plain"><span class="field-name">${esc(fieldLabel(f.name))}</span><input data-field="${esc(f.name)}" type="${f.secret ? "password" : "text"}" autocomplete="${f.secret ? "new-password" : "off"}" spellcheck="false" value="${esc(values[f.name] || "")}"></label>`).join("") + `</div>` +
              (request.hosts?.length ? `<div class="footer">Sent only to ${esc(request.hosts.join(", "))}</div>` : "");
          }
          html += `<div class="list"><button class="row plain decline" data-decline>Decline</button></div>`;
          if (error) html += `<div class="error vault-error">${esc(error)}</div>`;
          html += `<div class="sheet-actions"><button class="button primary" data-allow>Allow</button></div>`;
        } else {
          html += `<div class="list"><div class="row plain muted">${esc(request.state === "set" ? "Allowed." : request.state === "declined" ? "Declined." : `This request expired. Ask ${asker} to ask again.`)}</div></div>`;
        }
        b.innerHTML = html;
        b.querySelectorAll("[data-pick]").forEach((x) => (x.onclick = () => { picked = x.dataset.pick || null; draw(); }));
        b.querySelectorAll("[data-field], [data-title]").forEach((x) => (x.oninput = () => { values[x.dataset.field ?? "_title"] = x.value; ready(); }));
        const allow = $("[data-allow]", b);
        const ready = () => { if (allow) allow.disabled = busy || (picked === null && !wanted.every((f) => values[f.name])); };
        ready();
        if (allow) allow.onclick = () => answer(false);
        $("[data-decline]", b)?.addEventListener("click", () => answer(true));
      };
      const answer = async (declined) => {
        if (busy) return;
        busy = true;
        error = "";
        draw();
        const body = { scope: request.scope, id: request.id };
        if (declined) body.declined = true;
        else if (picked) body.item = picked;
        else {
          const { _title, ...fields } = values;
          if (_title?.trim()) body.title = _title.trim();
          body.fields = fields;
        }
        try {
          await api("/v1/vault/answer", { method: "POST", body });
          for (const k of Object.keys(values)) delete values[k];
          handle.close();
          refreshState().then(render).catch(() => {});
        } catch (err) {
          error = err.message;
          busy = false;
          draw();
        }
      };
      draw();
      try {
        request = (await api("/v1/vault/requests", { query: { scope, id } }))[0];
        if (!request) error = "It isn’t for you, or it’s from another install.";
        else picked = request.matches?.[0]?.id ?? null;
      } catch (err) {
        error = err.message;
      }
      draw();
      // Answered somewhere else (another device, an agent's CLI): the sheet goes.
      while (!finished && request?.state === "waiting") {
        await new Promise((r) => setTimeout(r, 2100));
        if (finished || busy) continue;
        const now = (await api("/v1/vault/requests", { query: { scope, id } }).catch(() => []))[0];
        if (now && now.state !== "waiting") handle.close();
      }
      return handle;
    },

    /// Whether the request was answered with an item.
    async state(scope, id) {
      return (await api("/v1/vault/requests", { query: { scope, id } }).catch(() => []))[0]?.state;
    },
  };

  /// A field's name as a person reads it: "api_key" → "API key".
  function fieldLabel(name) {
    const words = String(name).replace(/[_-]+/g, " ").trim().split(/\s+/);
    return words.map((w, i) => (/^(api|url|id|ssh|pem|aws|s3|ip)$/i.test(w) ? w.toUpperCase() : i ? w.toLowerCase() : w[0].toUpperCase() + w.slice(1).toLowerCase())).join(" ");
  }

  // MARK: App screens

  /// An app's screen, or a Settings screen: the page its node serves, in a sandboxed frame whose bridge this page
  /// answers, an app's acting only as that app, and only Settings' as the person.
  const screen = {
    frame: null,
    route: null,

    async show(detail, r) {
      const a = app(r.app);
      const title = r.kind === "setting" ? settingTitle(r.name) : a?.screens?.find((s) => s.name === r.screen)?.title || label(a) || r.app;
      document.title = title;
      detail.innerHTML = bar(title) + `<div class="screen"><div class="screen-status">${one.loading()}</div></div>`;
      const frame = document.createElement("iframe");
      frame.className = "screen-frame";
      frame.setAttribute("sandbox", "allow-scripts allow-forms");
      frame.setAttribute("referrerpolicy", "no-referrer");
      frame.src = "/frame.html";
      frame.title = title;
      this.frame = frame;
      this.route = r;
      let html;
      try {
        const path = r.kind === "setting" ? `/v1/settings-screens/${encodeURIComponent(r.name)}` : `/v1/screens/${encodeURIComponent(r.app)}/${encodeURIComponent(r.screen)}`;
        html = await api(path, { query: { params: r.params || "{}", scope: r.scope }, raw: true }).then((resp) => resp.text());
      } catch (err) {
        if (this.frame === frame) $(".screen", detail).innerHTML = message("Can’t Open This", esc(err.message));
        return;
      }
      if (this.frame !== frame) return;
      frame.__html = look(html);
      $(".screen", detail).append(frame);
      frame.addEventListener("load", () => setTimeout(() => $(".screen-status", detail)?.remove(), 50), { once: true });
    },

    /// Node events reach the open screen as the bridge's `one.__event`.
    event(e) {
      this.frame?.contentWindow?.postMessage({ oneEvent: e }, "*");
    },
  };

  /// What makes a screen sit in the window as its own: the window's background shows through, the install's accent,
  /// and on a computer the Mac app's size of text.
  function look(html) {
    const accent = /^#[0-9a-f]{3,8}$/i.test(config().accent || "") ? config().accent : "";
    // This window's appearance, kept in step (see `scheme`).
    const looks = `<script>(() => { const scheme = (${scheme})(); let now = ${JSON.stringify(appearance())}; const apply = () => scheme(document, now);
      addEventListener("DOMContentLoaded", apply); addEventListener("load", apply);
      addEventListener("message", (e) => { if (e.source === parent && e.data?.oneScheme) apply(now = e.data.oneScheme); }); })()</script>`;
    const style = `<style>html, body { background: transparent !important; }${accent ? ` :root:root { --accent: ${accent}; }` : ""}
      @media (pointer: fine) and (min-width: 600px) { html { zoom: calc(13 / 17); } :root:root { --bg: transparent; --card: rgba(0, 0, 0, 0.04); } }
      @media (pointer: fine) and (min-width: 600px) and (prefers-color-scheme: dark) { :root:root { --card: rgba(255, 255, 255, 0.05); } }</style>`;
    return html.includes("</head>") ? html.replace("</head>", `${style}${looks}</head>`) : style + looks + html;
  }

  /// The bridge's messages, from the screen's frame only, each acting as that screen's app; and a Settings screen's,
  /// as the person, which no app's may.
  addEventListener("message", async (e) => {
    const frame = screen.frame;
    if (!frame || e.source !== frame.contentWindow) return;
    const r = screen.route;
    const own = r.kind === "setting";
    if (e.data?.oneFrame) return void frame.contentWindow.postMessage({ oneScreen: frame.__html }, "*");
    const { one: m, id } = e.data || {};
    if (!m || typeof m !== "object") return;
    const reply = (ok) => frame.contentWindow?.postMessage({ oneReply: id, ok: ok ?? null }, "*");
    const fail = (err) => frame.contentWindow?.postMessage({ oneReply: id, error: String(err?.message || err) }, "*");
    try {
      if (["api", "signOut", "appearance", "device", "request"].includes(m.type) && !own) return fail("Only Settings can do that.");
      switch (m.type) {
        case "call":
          if (own) return fail("Settings has no app to call.");
          return reply(await call(r.app, String(m.op || ""), m.input ?? {}));
        case "open": {
          const to = typeof m.app === "string" && m.app ? { kind: "screen", app: m.app, screen: String(m.screen || "main"), params: JSON.stringify(m.params || {}), scope: r.scope }
            : own ? { kind: "setting", name: String(m.screen || ""), params: JSON.stringify(m.params || {}) }
            : { kind: "screen", app: r.app, screen: String(m.screen || "main"), params: JSON.stringify(m.params || {}), scope: r.scope };
          reply(null);
          return go(to, { pushed: true });
        }
        case "api":
          return reply(await settingsApi(m));
        case "signOut":
          reply(null);
          return auth.signOut();
        case "appearance":
          if (m.value != null) setAppearance(String(m.value));
          return reply(appearance());
        case "device":
          return reply({ name: browserName(), build: serverBuild(), signOut: true });
        case "request": {
          const scope = String(m.scope || ""), id = String(m.id || "");
          await new Promise((done) => vault.open({ scope, id, done }));
          return reply((await vault.state(scope, id)) === "set");
        }
        case "title": {
          // Empty while the screen shows its own large title, as the Mac app's bar is.
          const title = String(m.text ?? "");
          const el = $(".detail .bar-title", root);
          if (el) el.textContent = title;
          document.title = title || (own ? settingTitle(r.name) : label(app(r.app)));
          return reply(null);
        }
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
        case "vault":
          return reply(await askVault(r, String(m.need || "")));
        case "scan":
          return fail("Scanning documents needs the iPhone or iPad app. Choose the files instead.");
        case "icons":
          return reply(await iconPaths(Array.isArray(m.names) ? m.names.map(String) : []));
        case "icon": case "haptic":
          return reply(null);
        default:
          return fail("Unknown message");
      }
    } catch (err) {
      fail(err);
    }
  });

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

  /// `one.vault(need)`: asks for what the app declares, in the Vault's sheet → whether it's allowed once it's gone.
  async function askVault(r, need) {
    const scope = r.scope || "user";
    const out = await api("/v1/vault/request", { method: "POST", body: { scope, app: r.app, need } });
    if (!out?.id) throw new Error("The Vault didn’t take the request");
    await new Promise((done) => vault.open({ scope, id: out.id, done }));
    return (await vault.state(scope, out.id)) === "set";
  }

  // MARK: Settings

  /// What Settings shows this person, in order, as the node says (`settings_items`).
  const settingsItems = () => store.state?.settings_items || [];
  const settingTitle = (name) => settingsItems().find((i) => i.name === name)?.title || "Settings";

  /// An item's icon: theirs is their avatar; the rest, its symbol on its colour, as an app's tile.
  function settingIcon(item, size) {
    const u = me();
    if (item.name === "account" && u) return one.avatar(u.id, u.name).replace('class="avatar"', `class="avatar tile-avatar" data-size="${size}"`).replace('style="', `style="--size:${size}px;`);
    return one.appIcon({ name: item.name, title: item.title, icon: item.icon, color: item.color }, size);
  }

  /// An item that opens an app (the App Store) opens that app's screen.
  function settingsApp(r) {
    const a = r.kind === "setting" && settingsItems().find((i) => i.name === r.name)?.app;
    return a ? { kind: "screen", app: a, screen: "main", params: "{}" } : null;
  }

  /// Settings, as a page: its title and the install's name, then its items' icons in a page's grid.
  function drawSettings(detail) {
    document.title = "Settings";
    detail.innerHTML = bar("") + `<div class="scroll"><div class="page"><div class="page-header"><h1>Settings</h1><div class="page-subtitle">${esc(install()?.name || "")}</div></div><div class="grid"></div></div></div>`;
    const grid = $(".grid", detail);
    grid.__items = settingsItems().map((item) => {
      const to = { kind: "setting", name: item.name };
      const el = document.createElement("a");
      el.className = "tile";
      el.href = href(to);
      el.dataset.go = JSON.stringify(to);
      el.innerHTML = `<span class="tile-icon">${settingIcon(item, 60)}${badge(item.badge || 0)}</span><span class="tile-name">${esc(item.title)}</span>`;
      return { el, span: [1, 1] };
    });
    grid.append(...grid.__items.map((i) => i.el));
    layoutGrid();
  }

  /// `one.api` for a Settings screen: the node's /v1 at this origin, as this person; nothing else here.
  async function settingsApi(m) {
    const url = new URL(String(m.path || ""), location.origin);
    if (url.origin !== location.origin || !url.pathname.startsWith("/v1/")) throw new Error("That isn't your server's.");
    const method = String(m.method || "GET").toUpperCase();
    return api(url.pathname + url.search, { method, body: method === "GET" ? undefined : m.body ?? {} });
  }

  /// "linux runs 3157ba4": what the server that answers runs, for telling builds apart.
  function serverBuild() {
    const s = install()?.servers.find((x) => x.key === store.state?.server?.key);
    return s?.build ? `${s.name} runs ${s.build}` : null;
  }

  // MARK: Appearance

  const LOOK = "one.appearance";
  const appearance = () => (["light", "dark"].includes(localStorage.getItem(LOOK)) ? localStorage.getItem(LOOK) : "automatic");

  function setAppearance(value) {
    if (value === "automatic") localStorage.removeItem(LOOK);
    else if (["light", "dark"].includes(value)) localStorage.setItem(LOOK, value);
    schemed(document, appearance());
    screen.frame?.contentWindow?.postMessage({ oneScheme: appearance() }, "*");
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
    const name = store.state?.config?.name || "The One App";
    document.title = name;
    root.innerHTML = `<div class="welcome"><img class="welcome-mark" src="/logo.png" alt=""><h1>Welcome to ${esc(name)}</h1>` +
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
    const name = store.state?.config?.name || "The One App";
    root.innerHTML = message(`Can’t Reach ${name}`, esc(text), `<div class="unavailable-text">It keeps trying.</div>`);
  }

  async function start() {
    schemed(document, appearance());
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
    for (let wait = 1000; ; wait = Math.min(wait * 2, 15_000)) {
      try {
        await refreshState();
        break;
      } catch (err) {
        if (err.signedOut) return showSignedOut();
        showUnreachable(err.message);
        await new Promise((r) => setTimeout(r, wait));
      }
    }
    // `/vault?id=…&scope=…`: an agent's link to a request for an item, answered over You.
    let ask = null;
    if (location.pathname === "/vault") {
      const q = new URLSearchParams(location.search);
      ask = { kind: "vault", scope: q.get("scope"), id: q.get("id") };
      history.replaceState(null, "", "/");
    }
    render();
    if (ask?.id && ask.scope) openSheet(ask);
    listen();
  }

  document.addEventListener("DOMContentLoaded", start);
})();
