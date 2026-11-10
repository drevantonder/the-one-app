// The web client in the app's own window (platform.js): the app itself (client/native/src/native.js) is this device's
// client, which reaches the nodes and holds its keys, and signs in through the system's browser. Beyond a page, it
// asks for its network's address first, joins a Mac's own node to one with a pairing code, opens the links the app was
// opened with, takes a phone's hardware Back and text size, and may be the menu bar panel, a small window of its own:
// this page showing only the client's own `panel` screen, with the app's rows under it.
import { codeOf } from "./device.js";
import { net } from "./net.js";
import { $, h, logo, show, ui } from "./view.js";

export function app(native) {
  /// What the app says of itself and who's signed in there (`native.about`), once the page starts.
  let about = null;
  /// Whether this device is let in, once `join` says so.
  let isIn = false;
  /// Whether the app may show notifications, as it last said.
  let notify = null;
  /// Every icon, read once.
  let allIcons = null;
  const query = new URLSearchParams(location.search);
  const panel = query.has("panel");

  /// Settings' text size on an Android phone (`about.textScale`, how many times usual): the whole window zoomed by it, its
  /// screens' frames too, but never narrower than 320 pixels.
  function zoomed() {
    const scale = about?.textScale > 0 ? about.textScale : 1;
    document.documentElement.style.zoom = scale === 1 ? "" : Math.min(scale, innerWidth / 320);
  }

  /// A link the app was opened with, there once this device is in: `<cli>://open?app=&screen=&params=&scope=` (an
  /// app's screen; with no app, the page `scope`), `<cli>://page?scope=` (`&folder=accounts`, its Accounts),
  /// `<cli>://settings[/<item>]`, `<cli>://vault?id=&scope=` (a request, as a sheet), `<cli>://signed-in` (the window in
  /// front, which the app does), and a link to the address's own pages: to `window.go`, or `window.request`.
  function opened(link, window) {
    let url;
    try {
      url = new URL(link);
    } catch {
      return;
    }
    const q = url.searchParams, part = url.pathname.split("/").filter(Boolean)[0];
    const own = (name, params = {}) => ({ kind: "own", name, params: JSON.stringify(params) });
    const sheet = (scope, id) => scope && id && window.request(scope, id);
    if (url.protocol === `${about.cli}:`) {
      if (url.host === "open") return window.go(q.get("app") ? { kind: "screen", app: q.get("app"), screen: q.get("screen") || "main", params: q.get("params") || "{}", scope: q.get("scope") || undefined } : own("pages", { scope: q.get("scope") || "user" }));
      if (url.host === "page") return window.go(q.get("folder") === "accounts" ? own("accounts") : own("pages", { scope: q.get("scope") || "user" }));
      if (url.host === "settings") return window.go(part ? { kind: "own", name: part, params: JSON.stringify(Object.fromEntries(q)) } : own("pages", { scope: "settings" }));
      if (url.host === "vault") return void sheet(q.get("scope"), q.get("id"));
    } else if (about.address && url.origin === new URL(about.address).origin) {
      // An invite while no one's signed in here: as if typed on the welcome, opened in the browser for its passkey.
      if (url.pathname === "/invite" && url.hash.length > 1 && !about.signedIn) {
        return void native.address(link).then(async () => ((about = await native.about()), await native.open(link), window.signedOut(undefined, "Made your passkey? Sign in with it.")), (err) => window.signedOut(err.message));
      }
      if (url.pathname === "/vault") return void sheet(q.get("scope"), q.get("id"));
      window.go(window.route(url));
    }
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

  /// A Mac's own node joining a network with a pairing code another of its user's computers shows, then signing in
  /// to it here; Cancel goes back to `welcome`.
  function showPairing(welcome, error) {
    const join = async (code) => {
      try {
        await native.pair(code);
        about = await native.about();
        await platform.signIn();
      } catch (err) {
        showPairing(welcome, err.message);
      }
    };
    const field = { label: "Pairing Code", placeholder: "Pairing code" };
    show(h(ui.Welcome, { key: "pairing", logo, title: "Join with a Pairing Code", field, action: { label: "Join", run: join }, other: { label: "Cancel", run: () => welcome() }, problem: error }));
  }

  /// This device as its user's, where this page has the client (`net.has`, a bundle or build with its WebAssembly): the
  /// client's Rust runs here, as a browser's does, asking the app only to sign and open with its key.
  let keys = null;
  const inPage = {
    async join(waiting) {
      const user = about.user;
      keys = await native.keys();
      if (!(await net.start(user))) {
        const ask = async () => net.ask(user, await native.token(), about.joinName, about.joinPlatform, about.invite || undefined);
        const invited = await ask();
        const code = await codeOf(keys.key, keys.seal);
        waiting(code, invited);
        native.waiting(code);
        await net.untilLetIn(user, () => native.token(), ask);
      }
      isIn = true;
      native.joined();
    },
    reaches: (user) => net.start(user).catch(() => false),
    fetch: (...args) => net.fetch(...args),
    took: () => net.took(),
    removed: async () => (await net.still().catch(() => null)) === false,
    async leave(api) {
      if (keys) await Promise.race([api("/v1/devices/remove", { method: "POST", body: { device: keys.key } }), new Promise((r) => setTimeout(r, 20_000))]).catch((err) => console.warn("leaving your devices:", err));
    },
    nodes: (state) => net.ordered(state),
    settle: () => net.settle().catch((err) => console.warn("making this user's own keys:", err)),
    kept: (app, tool, input) => net.kept(about.user, app, tool, input),
    renew: (app) => net.renew(app),
    forget: () => net.forget(about.user),
    device: () => ({ name: about.device, signOut: true, keys: true, notify, ...(keys ? { key: keys.key } : {}) }),
  };

  /// Where the page has no client (a shell built with no WebAssembly): the app's own Rust is this device, and answers for it.
  const inShell = {
    /// The app's device let in: one already, or asking and showing its code (`waiting`) until it's approved.
    async join(waiting) {
      const asked = await native.join();
      if (!asked.in) {
        waiting(asked.code, asked.invited);
        await native.wait();
      }
      isIn = true;
    },
    reaches: async () => isIn,
    fetch: (...args) => native.fetch(...args),
    took: async () => undefined,
    removed: () => native.removed().catch(() => false),
    async leave() {},
    nodes() {},
    async settle() {},
    kept: (app, tool, input) => native.kept(about.user, app, tool, input),
    renew: (app) => native.renew(app),
    forget: async () => {},
    device: () => ({ name: about.device, signOut: true, keys: true, notify }),
  };

  /// The page's client needs Ed25519 in its WebCrypto for the session's own key (its endpoint's): an older Android WebView has
  /// none, and on one the app's own Rust answers instead.
  const runsClient = () => (net.has ? crypto.subtle.importKey("raw", new Uint8Array(32), { name: "Ed25519" }, false, ["verify"]).then(() => true, () => false) : false);

  const platform = {
    ...inShell,
    panel,
    client: () => about?.client || "web",
    /// The newest version of the client needs a newer app than this one.
    behind: () => !!about?.behind,
    /// The app on a phone: its home is the pages side by side, with no sidebar (`Shell`'s `phone`).
    phone: () => ["iPhone", "Android"].includes(about?.platform),
    address: () => about.address,
    /// `window`'s: `go(route)`, `route(url)`, `request(scope, id)`, `signedOut(problem, note)`, `back()` (the hardware Back →
    /// whether it took it), and `look` (light, dark or automatic).
    async start(window) {
      about = await native.about();
      Object.assign(platform, (await runsClient()) ? inPage : inShell);
      // The app draws its next start in it from the first frame.
      native.appearance(window.look);
      native.onOpen = (link) => opened(link, window);
      native.onBack = window.back;
      native.onScale = (scale) => ((about.textScale = scale), zoomed());
      zoomed();
      oneHost.appBehaviour();
      document.head.append(Object.assign(document.createElement("style"), { textContent: oneHost.APP_RULES }));
      // The app menu's commands (a Mac's), which work wherever the focus is: the sidebar folded or unfolded, as its
      // own key does here; joining a network with a pairing code, while this Mac's node is in none.
      native.onCommand = (name) => {
        if (name === "sidebar") dispatchEvent(new KeyboardEvent("keydown", { key: "\\", metaKey: true }));
        else if (name === "join" && about.unpaired && !about.signedIn) showPairing(() => window.signedOut());
      };
      // A link of this page's to another site (Settings' that are pages of the address) goes to the system's browser.
      addEventListener("click", (e) => {
        const a = e.target.closest?.("a[href]");
        if (!a || new URL(a.href, location.href).origin === location.origin) return;
        e.preventDefault();
        platform.open(a.href);
      });
      if (about.opened) opened(about.opened, window);
    },
    signedIn: () => !!about?.signedIn,
    user: () => about?.user || null,
    until: () => Date.now() + 60_000,
    signIn: () => native.signIn().then(() => location.reload()),
    async signOut() {
      await native.signOut().catch(() => {});
      about = await native.about();
    },
    /// The sign-in screen (`props`, @one/ui's Welcome's); but first, where: an address, or an invite link to one.
    welcome(shown) {
      const again = (problem, note) => show(platform.welcome({ ...shown, problem, note }));
      // Visible on purpose: a store's reviewer has to find it, and Apple rejects hidden ways in.
      const demo = about.demo ? { label: "Try a Demo", run: () => native.demo().then(() => location.reload(), (err) => again(err.message)) } : undefined;
      const props = demo ? { ...shown, other: demo, note: shown.note ?? "Anyone trying the demo can see what's entered in it." } : shown;
      if (about.address) return h(ui.Welcome, props);
      const take = async (typed) => {
        try {
          await native.address(typed);
          about = await native.about();
          // An invite opens in the browser, where its user makes their passkey; then they sign in with it here.
          if (/\/invite#./.test(typed)) return (await native.open(new URL(typed.includes("://") ? typed : `https://${typed}`).href), again(undefined, "Made your passkey? Sign in with it."));
          await platform.signIn();
        } catch (err) {
          again(err.message);
        }
      };
      const field = { label: "Address", placeholder: "one.example.com" };
      // A Mac whose own node is in no network yet joins one with a pairing code.
      const other = about.unpaired ? { label: "Join with a Pairing Code…", run: () => showPairing(() => again()) } : props.other;
      return h(ui.Welcome, { ...props, field, action: { label: "Sign In", invite: "Open Invite", run: take }, other });
    },

    changed: () => native.changed(),
    badge: (count) => native.badge(count),
    appearance: (value) => native.appearance(value),
    copy: (text) => native.copy(String(text)),
    open: (href) => native.open(href).catch(() => {}),
    /// The app's files are on this device, so every icon's one read.
    icons: (names, v) => (allIcons ||= fetch(`/icons.json?v=${v}`).then((r) => (r.ok ? r.json() : Promise.reject(new Error("No icons.")))).catch((err) => ((allIcons = null), Promise.reject(err)))),
    notify: async (ask) => (notify = await native.notify(ask).catch(() => null)),
    /// What a screen's bridge may ask of the app beyond a page: copying, saving or previewing a file, a phone's haptic;
    /// an iPhone's document camera, and its menu for what a touch and hold lifted, shown with it.
    bridge: (frame) => ({
      copy: (text) => platform.copy(text),
      give: (how, name, blob) => native.save(name, blob, how === "preview"),
      haptic: (style) => native.haptic(style),
      ...(about?.scan ? { scan: () => native.scan() } : {}),
      ...(about?.lifts ? { menu: async (m) => ((r) => (r?.lifted === false ? oneHost.menu(frame, m) : r?.index ?? null))(await native.menu(Array.isArray(m.items) ? m.items : [])) } : {}),
    }),
    /// What a screen's frame is told the app does beyond a browser's (oneHost.look): an iPhone's document camera, and its
    /// touch and hold, which lifts what a screen marks `data-one-held` with its menu.
    look: () => ({ __oneApp: true, ...(about?.scan ? { __oneCan: { scan: true } } : {}), ...(about?.lifts ? { __oneLifts: true } : {}) }),

    /// The panel's: what its screen is told (this device's node), and its rows under it, each `[label, run, checked]`
    /// (checked null for none).
    panelScreen: () => ({ here: about.node || "" }),
    panelRows: (signedIn, name, redraw) => [
      [signedIn ? `Open ${name}` : "Sign In…", () => native.main(), null],
      about.login != null && ["Open at Login", async () => ((about.login = await native.login(!about.login)), redraw()), about.login],
      ["Quit", () => native.quit(), null],
    ].filter(Boolean),
    /// The panel's window fits its screen, up to most of the screen.
    fit(height) {
      if (panel) native.fit(Math.min(height, window.screen.availHeight - 80) + ($(".panel-head")?.offsetHeight || 0) + ($(".panel-rows")?.offsetHeight || 0) + 2);
    },
    /// The window in front, at `route`, from the panel.
    main: (r) => native.main(linkOf(r)),
  };
  return platform;
}
