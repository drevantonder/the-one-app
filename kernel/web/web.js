// The web client: the Mac app's window, in a browser. It signs in as an OAuth client of the install's auth service
// (PKCE, and the person's passkey at its sign-in page), keeps its refresh token in this browser, and calls a node's `/v1` on its
// own origin with the access token; the gateway sends each on. App screens are untrusted: each runs in a sandboxed
// frame, and this page answers its bridge as the Mac app's web view does, making each call itself. It draws at once
// what it showed last (the sidebar, and a still of the screen) and then what the node says now.

// The bridge and kit (src/screen/), loaded after this, give `one`; this page answers it for its own use (`__one.host`).
window.__one = {};

(() => {
  // What start.js, inline in the page's head, began and says: the token, the places, the pages kept.
  const first = window.__oneStart;
  const $ = (sel, el = document) => el.querySelector(sel);
  const esc = (s) => String(s ?? "").replace(/[&<>"']/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;", "'": "&#39;" })[c]);
  const b64u = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  const random = (n) => b64u(crypto.getRandomValues(new Uint8Array(n)));

  // MARK: Signing in

  const KEPT = "one.signin";
  // What the client's own screens keep between visits (one.cache), the window as it was last (LAST), and the screens
  // it showed (KEPT_SCREENS, `kept`): all forgotten with the sign-in.
  const SCREENS = "one.screens.";
  const LAST = "one.last.window";
  const forgetScreens = () => {
    Object.keys(localStorage).filter((k) => k.startsWith(SCREENS) || k === LAST).forEach((k) => localStorage.removeItem(k));
    window.caches?.delete("one.screens").catch(() => {});
  };
  const CLIENT = "one.client";

  const auth = {
    kept: () => JSON.parse(localStorage.getItem(KEPT) || "null"),
    keep: (tokens) => localStorage.setItem(KEPT, JSON.stringify(tokens)),

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
      const meta = await first.meta({ fresh: true });
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
      const meta = await first.meta({ fresh: true });
      const client = JSON.parse(localStorage.getItem(CLIENT) || "null")?.id;
      auth.keep(await first.grant(meta, { grant_type: "authorization_code", code: q.get("code"), redirect_uri: `${location.origin}/callback`, client_id: client, code_verifier: pkce.verifier }));
      return pkce.back && pkce.back !== "/callback" ? pkce.back : "/";
    },

    /// An access token good for a minute more (start.js's); signed out, this browser forgets what it kept.
    async token(stale) {
      try {
        return await first.token(stale);
      } catch (err) {
        if (err.signedOut) localStorage.removeItem(KEPT), forgetScreens();
        throw err;
      }
    },

    async signOut() {
      const kept = auth.kept();
      localStorage.removeItem(KEPT);
      forgetScreens();
      try {
        const meta = await first.meta({ fresh: true });
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
  const UNANSWERED = "Your devices aren’t answering. Make sure one of them is on and online.";

  /// How long to wait before asking again the `n`th time running (from 0) after the node didn't answer: at once, then
  /// a quarter second, doubling, up to five. A node restarting for an update is back within a moment.
  const retryWait = (n) => (n === 0 ? 0 : Math.min(5000, 125 << Math.min(n, 6)));
  const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
  /// No node answered: the gateway, or Cloudflare before it, couldn't reach one, so nothing ran.
  const unanswered = (status) => [502, 503, 530].includes(status);

  /// A request to the node, as this person: JSON in and out, or with `raw` the response itself (a 304 too). Asked again
  /// for a few seconds while no node answers, as one restarting doesn't; a call carries an id, so it runs once however
  /// often it's sent. What start.js already asked for is its answer, the first time.
  async function api(path, { method = "GET", body, query, raw, signal, headers: extra } = {}) {
    const url = new URL(path, location.origin);
    for (const [k, v] of Object.entries(query || {})) if (v != null) url.searchParams.set(k, typeof v === "string" ? v : JSON.stringify(v));
    if (url.pathname === "/v1/call" && body && typeof body === "object" && !body.id) body = { ...body, id: callId() };
    let early = method === "GET" ? first.take(url.pathname + url.search) : null;
    let token = early ? null : await auth.token();
    for (let renewed = false, misses = 0; ; ) {
      let r;
      try {
        if (early) r = await early.catch(() => null);
        early = null;
        if (!r) {
          token ||= await auth.token();
          const headers = { authorization: `Bearer ${token}`, ...extra };
          if (body !== undefined) headers["content-type"] = "application/json";
          r = await fetch(url, { method, headers, body: body === undefined ? undefined : JSON.stringify(body), signal });
        }
      } catch (err) {
        if (signal?.aborted || misses >= 5) throw err;
        await sleep(retryWait(misses++));
        continue;
      }
      if (unanswered(r.status) && misses < 5) {
        await sleep(retryWait(misses++));
        continue;
      }
      if (r.status === 401 && !renewed) {
        renewed = true;
        // Refused start.js's ask: the token it sent was the one kept.
        token = await auth.token(token ?? auth.kept()?.access);
        continue;
      }
      if (r.status === 401) throw Object.assign(new NodeError("Signed out."), { signedOut: true });
      if (raw && (r.ok || r.status === 304)) return r;
      const out = await r.json().catch(() => null);
      if (!r.ok || out?.error) throw new NodeError(out?.error || (r.status >= 500 ? UNANSWERED : "That didn’t work. Try again."));
      return out;
    }
  }

  /// This release's files by their content (`/web.js?v=…`), as the gateway's index.html names them: kept for good.
  const files = Object.fromEntries((document.querySelector('meta[name="one-files"]')?.content || "").split(" ").filter(Boolean).map((f) => f.split("=")));
  const asset = (name) => (files[name] ? `/${name}?v=${files[name]}` : `/${name}`);

  /// Phosphor's icons, those the bridge asks for (on this page, and in screens' frames), from the address by name, then
  /// the browser's cache: each named once.
  const icons = {};
  async function iconPaths(names = []) {
    const missing = [...new Set(names.filter((n) => typeof n === "string" && !(n in icons)))].sort();
    if (missing.length) {
      const asking = fetch(`/icons?names=${encodeURIComponent(missing.join(","))}&v=${files["icons.json"] || ""}`).then((r) => (r.ok ? r.json() : Promise.reject(new Error("No icons."))));
      for (const n of missing) icons[n] = asking.then((got) => got[n] || "");
      asking.catch(() => missing.forEach((n) => delete icons[n]));
    }
    return Object.fromEntries(await Promise.all(names.map(async (n) => [n, await icons[n]])));
  }
  window.__one.host = async (m) => (m.type === "copy" ? navigator.clipboard.writeText(m.text) : m.type === "icons" ? iconPaths(m.names) : null);

  /// Names a call wherever it's sent, so sent again it runs once.
  const callId = () => Array.from(crypto.getRandomValues(new Uint8Array(8)), (b) => b.toString(16).padStart(2, "0")).join("");

  /// The node's state or apps, asked for once a second at most, and not again while the node's events say nothing
  /// changed: the window and its own screens (Home, Settings) share the answer.
  const recent = {};
  let live = false;
  function shared(path) {
    if (recent[path] && (live || Date.now() - recent[path].at < 1000)) return recent[path].answer;
    const answer = api(path);
    recent[path] = { at: Date.now(), answer };
    answer.catch(() => delete recent[path]);
    return answer;
  }

  /// Runs an app's operation as this person → its result.
  async function call(app, op, input = {}) {
    return (await api("/v1/call", { method: "POST", body: { app, op, input, client: "web" } })).ok;
  }

  // MARK: What the window shows

  /// The sidebar's: the node's state, its apps and their badges. What's beside it is a screen the node serves.
  const store = { state: null, apps: [], badges: {} };

  const me = () => store.state?.user;
  const install = () => store.state?.install;
  // The white label's name, as the gateway's index.html says it, until the install's state does.
  const whiteLabel = { name: document.querySelector('meta[name="application-name"]')?.content || "" };
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
    const [state, list] = await Promise.all([shared("/v1/state"), apps ? shared("/v1/apps").catch(() => null) : null]);
    const regrouped = JSON.stringify([state.user, state.install?.groups]) !== JSON.stringify([store.state?.user, store.state?.install?.groups]);
    store.state = state;
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

  /// The node's events: what changed, so the window and its screens keep up. EventSource can't send a token, so it's a
  /// fetch that streams, only while the page shows: a page the browser keeps for Back would hold its connection, and
  /// over HTTP/1.1 a browser opens six to an address, so after a few visits the next page waited for one to close.
  let events = null;
  async function listen() {
    const mine = (events = new AbortController());
    for (let misses = 0; !mine.signal.aborted; ) {
      const started = Date.now();
      try {
        const r = await api("/v1/events", { raw: true, signal: mine.signal });
        live = true;
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
        if (err.signedOut) return (live = false), showSignedOut();
      }
      live = false;
      // At once after a stream that ran a while and ended (its node handing over to a new build), then waiting longer
      // each miss.
      if (Date.now() - started > 10_000) misses = 0;
      if (!mine.signal.aborted) await sleep(retryWait(misses++));
    }
  }
  addEventListener("pagehide", () => ((live = false), events?.abort()));
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

  /// The URL is the place (start.js's `route`): each is a screen, the client's own (`own`, by name), or an app's.
  const { route, own, phone, isPage, swiped, kept } = first;

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

  // A phone's width (start.js's `phone`) has no sidebar, as the iPhone and Android apps have none: the pages fill the
  // window, swiped between, and every screen opened from them is pushed over them, with Back.
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

  function render() {
    if (!store.state) return;
    if (!install()) return void (root.innerHTML = message("Not Set Up Yet", "Set it up on one of your computers first."));
    if (!$(".window", root)) root.innerHTML = `<div class="window"><nav class="sidebar"></nav><main class="detail"></main></div>`;
    drawSidebar();
    drawDetail();
  }

  function message(title, text, extra = "") {
    return `<div class="unavailable"><div class="unavailable-title">${esc(title)}</div><div class="unavailable-text">${text}</div>${extra}</div>`;
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
  /// phones have them), kept under what's pushed over them so Back finds them as they were. Opened straight at a
  /// screen, the pages load once it has what it asked for, so they don't hold it up.
  let home = null;
  function drawPhone(detail, r, key) {
    if (!home?.layer.isConnected) {
      frames = frames.filter((f) => f.sheet);
      detail.innerHTML = `<div class="home"></div><div class="pushed" hidden></div>`;
      const layer = $(".home", detail), at = swiped(isPage(r) ? r : under(r));
      const h = (home = { layer, f: null, load: () => h.f || ((h.f = frameFor(at, layer)), (h.f.home = true)) });
      shown = "";
    }
    const pushed = $(".pushed", detail);
    if (isPage(r)) {
      home.load();
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
    if (!home.f) settled(frames.find((f) => !f.sheet && !f.home)).then(home.load);
  }

  /// Once a screen is written in and has had what it asked for answered (or a few seconds, whatever it waits on).
  function settled(f, until = Date.now() + 4000) {
    return new Promise((done) => {
      const check = () => (f && (!f.loaded || f.busy || Date.now() - f.quiet < 300) && Date.now() < until ? setTimeout(check, 50) : done());
      check();
    });
  }

  /// The detail's bar: its title, as a Mac window's toolbar, with Back when a screen was pushed, and on a phone under
  /// every page.
  function bar(title) {
    const back = history.state?.back || (phone.matches && !isPage(route())) ? `<button class="back" data-back aria-label="Back">‹</button>` : "";
    return `<header class="bar">${back}<div class="bar-titles"><div class="bar-title">${esc(title)}</div></div></header>`;
  }

  // MARK: Screens

  /// Every screen showing: the one beside the sidebar, and the sheets over it, each an app's or the client's own, in a
  /// sandboxed frame whose bridge this page answers: an app's acting only as that app, and only the client's own as the
  /// person.
  let frames = [];

  // What this browser keeps of the screens it showed (start.js's `kept`), until it signs out: each route's page as the
  // node last gave it (`page`), and its still (`still`), the page as it last looked here without its scripts, which
  // bridge.js sends.
  const NONCE = /nonce(-|=")[0-9a-f]{32}/g;
  /// Two pages the same but for their nonce.
  const same = (a, b) => a?.replace(NONCE, "") === b?.replace(NONCE, "");
  /// A kept page with a nonce of its own, so no two loads share one.
  const renonce = (html) => {
    const nonce = callId() + callId();
    return html.replace(NONCE, (_, sep) => `nonce${sep}${nonce}`);
  };

  /// A route's page from the node, asked for once (the one the window opens on as it starts, beside the sidebar's),
  /// and kept. Asked as having the one kept, if there is: unchanged, it's that.
  let asked = null;
  function screenPage(r) {
    const key = JSON.stringify(r);
    if (asked?.key === key) {
      const { page } = asked;
      asked = null;
      return page;
    }
    return kept.get("page", key).then(async (was) => {
      const headers = was?.tag ? { "if-none-match": was.tag } : {};
      let resp = await api(first.address(r), { raw: true, headers });
      if (resp.status === 304 && was) return renonce(was.text);
      // Kept no longer: asked again for all of it.
      if (resp.status === 304) resp = await api(first.address(r), { raw: true });
      const html = await resp.text();
      kept.put("page", key, html, resp.headers.get("etag"));
      return html;
    });
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
    // The frame loads while its page is on its way, and shows once it's written in (`write`).
    frame.style.visibility = "hidden";
    into.append(frame);
    const draw = (page) => {
      f.page = page;
      f.html = look(page);
      if (f.waiting) write(f);
    };
    const fresh = screenPage(r);
    Promise.all([kept.get("page", f.key), sheet ? null : kept.get("still", f.key)]).then(([page, still]) => {
      if (f.html) return;
      if (still) showStill(f, still.text);
      if (page) draw(renonce(page.text));
    });
    fresh.then((page) => {
      if (!f.html) return draw(page);
      if (same(page, f.page)) return;
      f.page = page;
      f.html = look(page);
      frame.src = asset("frame.html");
    }, (err) => f.html || ((f.loaded = true), (into.innerHTML = message("Can’t Open This", esc(one.plain(err).title)))));
    return f;
  }

  /// A frame's page, written in once both are ready: it shows, or with a still, shows in its place once it's drawn.
  function write(f) {
    if (!f.html) return void (f.waiting = true);
    f.waiting = false;
    f.frame.contentWindow.postMessage({ oneScreen: f.html }, "*");
    if (f.loaded) return;
    f.loaded = true;
    f.quiet = Date.now();
    $(".screen-status", f.into)?.remove();
    // A screen that asks nothing shows in place of its still soon after it's written in.
    if (f.still) setTimeout(() => watch(f), 1500);
    else f.frame.style.visibility = "";
  }

  // MARK: Stills

  /// A screen's still, in a frame of its own as sandboxed as the screen's, answering nothing but its frame's ask for it.
  function showStill(f, html) {
    if (!f.into.isConnected) return;
    const frame = sandboxed(f.route);
    frame.classList.add("screen-still");
    frame.setAttribute("aria-hidden", "true");
    const scrolled = `<script>addEventListener("load", () => scrollTo(0, Number(document.documentElement.dataset.stillY) || 0))</script>`;
    f.still = { frame, html: look(html.replace("</head>", `${scrolled}</head>`)) };
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

  /// What makes a screen sit in the window as its own: the window's background shows through, and on a
  /// computer the Mac app's size of text.
  function look(html) {
    // This window's appearance, kept in step (see `scheme`); with the nonce of a page that runs only its own scripts
    // (the client's own screens), as one of them.
    const nonce = html.match(/^<!doctype html><html><head><meta charset="utf-8"><meta http-equiv="Content-Security-Policy" content="script-src 'nonce-([0-9a-f]+)'/)?.[1];
    const looks = `<script${nonce ? ` nonce="${nonce}"` : ""}>(() => { const scheme = (${scheme})(); let now = ${JSON.stringify(appearance())}; const apply = () => scheme(document, now);
      addEventListener("DOMContentLoaded", apply); addEventListener("load", apply);
      addEventListener("message", (e) => { if (e.source === parent && e.data?.oneScheme) apply(now = e.data.oneScheme); }); })()</script>`;
    const style = `<style>html, body { background: transparent !important; }
      @media (pointer: fine) { html { zoom: calc(13 / 17); } :root:root { --bg: transparent; } }</style>`;
    return html.includes("</head>") ? html.replace("</head>", `${style}${looks}</head>`) : style + looks + html;
  }

  /// The bridge's messages (src/screen/bridge.js), from the screens' frames only, each acting as its screen's app; and
  /// the client's own screens', as the person, which no app's may.
  addEventListener("message", async (e) => {
    const still = frames.find((x) => x.still?.frame.contentWindow === e.source)?.still;
    if (still && e.data?.oneFrame) return void still.frame.contentWindow.postMessage({ oneScreen: still.html }, "*");
    const f = frames.find((x) => x.frame.contentWindow === e.source);
    if (!f) return;
    const r = f.route, mine = r.kind === "own";
    if (e.data?.oneFrame) return write(f);
    if (typeof e.data?.oneSheets === "number") return void shade(f, e.data.oneSheets > 0);
    if (e.data?.oneStill !== undefined) return void (f.sheet || kept.put("still", f.key, e.data.oneStill));
    const { one: m, id } = e.data || {};
    if (!m || typeof m !== "object") return;
    // What it's waiting on: its still shows until it's been answered.
    f.busy++;
    watch(f);
    const answered = () => ((f.busy = Math.max(0, f.busy - 1)), (f.quiet = Date.now()));
    const reply = (ok) => (answered(), f.frame.contentWindow?.postMessage({ oneReply: id, ok: ok ?? null }, "*"));
    const fail = (err) => (answered(), f.frame.contentWindow?.postMessage({ oneReply: id, error: String(err?.message || err) }, "*"));
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
          // From a sheet, the sheet goes and it opens where the window is. What the sidebar lists (an app on a page,
          // Backups in Settings) opens as if picked there, with no Back; only what's deeper than it is pushed.
          f.sheet?.close(null);
          return go(to, { pushed: phone.matches ? !isPage(to) : ![...document.querySelectorAll(".sidebar [data-go]")].some((a) => a.getAttribute("href") === href(to)) });
        }
        case "close":
          reply(null);
          if (f.sheet) return f.sheet.close(m.value);
          if (history.state?.back || (phone.matches && !isPage(route()))) back();
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
          return reply({ name: browserName(), signOut: true });
        case "request":
          return reply((await openSheet(own("request", { scope: String(m.scope || ""), id: String(m.id || "") }))) === true);
        case "vault": {
          if (mine) return fail("Settings has no app to ask for.");
          const scope = r.scope || "user";
          const out = await api("/v1/vault/request", { method: "POST", body: { scope, app: r.app, need: String(m.need || "") } });
          if (!out?.id) throw new Error("The Vault didn’t take the request");
          return reply((await openSheet(own("request", { scope: out.scope, id: out.id }))) === true);
        }
        case "title": {
          // Empty while the screen shows its own large title, as the Mac app's bar is; the tab is named by what the
          // screen says it's called.
          const title = String(m.text ?? "");
          if (f.sheet) f.sheet.title.textContent = title || titleOf(r);
          // A phone's pages have no bar: what's pushed over them has its own.
          else if (!f.home) {
            const el = $(".detail .bar-title", root);
            if (el) el.textContent = title;
            document.title = title || String(m.name || "") || titleOf(r);
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
          const item = settingAt(url.href);
          if (item) { go(own(item.name)); return reply(null); }
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
  /// Whether a url is this site's: a page the kernel serves, shown in place rather than in a new tab.
  const here = (url) => new URL(url, location.href).origin === location.origin;
  /// The Settings item whose url is `url` on this site, if one is.
  const settingAt = (url) => settingsItems().find((i) => i.url && here(i.url) && new URL(i.url, location.href).pathname === new URL(url, location.href).pathname);
  const settingTitle = (name) => settingsItems().find((i) => i.name === name)?.title || "Settings";

  /// An item's icon: its symbol on its colour, as an app's tile.
  function settingIcon(item, size) {
    return one.appIcon({ name: item.name, title: item.title, icon: item.icon, color: item.color }, size);
  }

  /// `one.api` for a client's own screen: the node's /v1 at this origin, as this person; nothing else here.
  async function clientApi(m) {
    const url = new URL(String(m.path || ""), location.origin);
    if (url.origin !== location.origin || !url.pathname.startsWith("/v1/")) throw new Error("That isn’t on your devices.");
    const method = String(m.method || "GET").toUpperCase();
    const body = method === "GET" ? undefined : m.body ?? {};
    if (url.pathname === "/v1/call" && body && typeof body === "object") body.client = "web";
    if (method === "GET" && !url.search && ["/v1/state", "/v1/apps"].includes(url.pathname)) return shared(url.pathname);
    return api(url.pathname + url.search, { method, body });
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
    root.innerHTML = message("Can’t Reach Your Devices", esc(text), `<div class="unavailable-text">Trying again…</div>`);
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
    navigator.serviceWorker?.register("/sw.js").catch(() => {});
    // `/vault?id=…&scope=…`: an agent's link to a request for an item, answered over You.
    let request = null;
    if (location.pathname === "/vault") {
      const q = new URLSearchParams(location.search);
      request = { scope: q.get("scope"), id: q.get("id") };
      history.replaceState(null, "", "/");
    }
    // Everything the window opens on, asked for at once: its screen, beside the node's state and apps; meanwhile the
    // window as it was last.
    ask(phone.matches && isPage(route()) ? swiped(route()) : route());
    if (lastWindow()) render();
    else root.innerHTML = `<div class="starting">${one.loading("Starting")}</div>`;
    // A device that takes the request and never answers (asleep, stuck) is one that can't be reached, while it's still asked.
    const away = setTimeout(() => $(".window", root) || showUnreachable(UNANSWERED), 15000);
    for (let misses = 1; ; misses++) {
      try {
        await refreshState({ apps: true });
        break;
      } catch (err) {
        if (err.signedOut) return clearTimeout(away), showSignedOut();
        // What it showed last stays, as a screen whose node is restarting does.
        if (!$(".window", root)) showUnreachable(err instanceof NodeError ? err.message : UNANSWERED);
        await sleep(retryWait(misses));
      }
    }
    clearTimeout(away);
    if (request?.id && request.scope) openSheet(own("request", { scope: request.scope, id: request.id }));
    listen();
  }

  document.addEventListener("DOMContentLoaded", start);
})();
