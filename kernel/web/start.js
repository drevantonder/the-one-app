// What the web client asks for first, started as its page arrives, before its styles and web.js: an access token, the
// node's state and apps, and the page of the screen the window opens on, so they come in while web.js does. The
// gateway's index.html has it inline, with its hash in the page's policy; web.js takes what it started (`take`), and
// uses its token, places and kept pages, so each is said once.
window.__oneStart = (() => {
  const SIGNIN = "one.signin", CLIENT = "one.client", META = "one.auth";
  const read = (key) => {
    try {
      return JSON.parse(localStorage.getItem(key) || "null");
    } catch {
      return null;
    }
  };

  // MARK: The access token

  /// The auth service's endpoints, as it says them: kept, so a token is renewed in one trip.
  async function meta({ fresh } = {}) {
    if (!fresh && read(META)?.token_endpoint) return read(META);
    const r = await fetch("/.well-known/oauth-authorization-server");
    if (!r.ok) throw new Error("The sign-in service isn’t answering. Try again in a moment.");
    const out = await r.json();
    localStorage.setItem(META, JSON.stringify(out));
    return out;
  }

  async function grant(meta, fields) {
    const r = await fetch(meta.token_endpoint, { method: "POST", headers: { "content-type": "application/x-www-form-urlencoded" }, body: new URLSearchParams(fields) });
    const out = await r.json().catch(() => ({}));
    if (!r.ok || !out.access_token) throw Object.assign(new Error(out.error_description || out.error || "Signing in didn't work."), { signedOut: r.status === 400 || r.status === 401 });
    return { access: out.access_token, refresh: out.refresh_token, until: Date.now() + (out.expires_in || 3600) * 1000 };
  }

  /// An access token good for a minute more, new from the refresh token if it's older: one tab at a time, since
  /// each refresh token works once.
  async function token(stale) {
    const good = (t) => t && t.access !== stale && t.until > Date.now() + 60_000;
    if (good(read(SIGNIN))) return read(SIGNIN).access;
    const renew = async () => {
      const kept = read(SIGNIN);
      if (good(kept)) return kept.access;
      if (!kept?.refresh) throw Object.assign(new Error("Signed out."), { signedOut: true });
      const fields = { grant_type: "refresh_token", refresh_token: kept.refresh, client_id: read(CLIENT)?.id };
      // Endpoints kept from before that no longer answer are asked for again.
      const tokens = await grant(await meta(), fields).catch((err) => (err.signedOut ? Promise.reject(err) : meta({ fresh: true }).then((m) => grant(m, fields))));
      localStorage.setItem(SIGNIN, JSON.stringify({ ...tokens, refresh: tokens.refresh || kept.refresh }));
      return tokens.access;
    };
    return navigator.locks ? navigator.locks.request("one.refresh", renew) : renew();
  }

  // MARK: Where the window is

  const own = (name, params = {}) => ({ kind: "own", name, params: JSON.stringify(params) });

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

  /// A phone's width has no sidebar: its pages, side by side to swipe between, are the screen its pages show in.
  const phone = matchMedia("(max-width: 700px)");
  const isPage = (r) => r.kind === "own" && r.name === "pages";
  const swiped = (r) => own("pages", { swipe: true, scope: JSON.parse(r.params).scope || "user" });

  /// Where a route's page is on the node, as the path and query the window asks it with.
  function address(r) {
    const url = new URL(r.kind === "own" ? `/v1/client-screens/${encodeURIComponent(r.name)}` : `/v1/screens/${encodeURIComponent(r.app)}/${encodeURIComponent(r.screen)}`, location.origin);
    url.searchParams.set("params", r.params || "{}");
    if (r.scope != null) url.searchParams.set("scope", r.scope);
    return url.pathname + url.search;
  }

  // MARK: Kept pages

  /// Each route's page as the node last gave it, with what names its content (its ETag), until this person signs out.
  const KEPT_SCREENS = "one.screens";
  const kept = {
    url: (kind, key) => `/${kind}/${encodeURIComponent(key)}`,
    /// → `{ text, tag }`, or null.
    async get(kind, key) {
      try {
        const r = await (await caches.open(KEPT_SCREENS)).match(kept.url(kind, key));
        return r ? { text: await r.text(), tag: r.headers.get("x-etag") } : null;
      } catch {
        return null;
      }
    },
    put(kind, key, text, tag) {
      if (typeof text !== "string" || text.length > 2_000_000) return;
      window.caches?.open(KEPT_SCREENS).then((c) => c.put(kept.url(kind, key), new Response(text, { headers: tag ? { "x-etag": tag } : {} }))).catch(() => {});
    },
  };

  /// The window's first asks, by path and query, until web.js takes them: none without a sign-in, nor where the page
  /// does something else first (finishing a sign-in, a Vault request).
  const asked = new Map();
  const send = (path, headers = {}) => token().then((t) => fetch(path, { headers: { authorization: `Bearer ${t}`, ...headers } }));
  const ask = (path, answer) => (answer.catch(() => {}), asked.set(path, answer));
  if (read(SIGNIN)?.refresh && !["/callback", "/vault"].includes(location.pathname)) {
    ask("/v1/state", send("/v1/state"));
    ask("/v1/apps", send("/v1/apps"));
    // Asked as having the page kept here, if it is: unchanged, the node answers 304, and the kept one shows.
    const r = phone.matches && isPage(route()) ? swiped(route()) : route();
    const path = address(r);
    ask(path, kept.get("page", JSON.stringify(r)).then((was) => send(path, was?.tag ? { "if-none-match": was.tag } : {})));
  }
  /// What was asked for `path` (a Response's promise), once: null if it wasn't.
  const take = (path) => {
    const answer = asked.get(path) || null;
    asked.delete(path);
    return answer;
  };

  return { meta, grant, token, own, route, phone, isPage, swiped, address, kept, take };
})();
