// What the pages and Accounts screens know, the same on every client: the user's pages (theirs, their groups',
// everyone's), the apps on each and what each offers, all read from the node as the user looking (`one.api`).
import { arranged } from "@one/ui";

export const home = (() => {
  // `failed`: the pages whose widgets couldn't be read, so theirs stand as grey cards until they can. `late`: widgets
  // too slow to come with their page, being asked for alone. `read`: the pages read since this screen started.
  const h = { state: null, apps: [], widgets: {}, badges: {}, failed: new Set(), late: new Set(), read: new Set() };
  const me = () => h.state?.user;
  const network = () => h.state?.network;
  const isAdmin = () => me()?.role === "admin";
  const label = (a) => a?.title || a?.name || "";
  const app = (name) => h.apps.find((a) => a.name === name);
  const has = (a, screen) => !!a?.screens?.some((s) => s.name === screen);
  const hasScreens = (a) => has(a, "main");
  // The tool that lists an app's accounts, the one it says does (`accounts: true`): none for an app without.
  const accountsTool = (a) => a?.tools?.find((o) => o.accounts)?.name;
  const scopes = (a) => (a.added || []).map((s) => (s.startsWith("user:") ? "user" : s));
  /// Every page this user can read, as the node lists them (`pages` in `/api/state`): theirs, their groups' in the
  /// order they chose, everyone's last. The ones they hid are marked, and left out of `pages`.
  const allPages = () => h.state?.pages || [];
  const pages = () => allPages().filter((p) => !p.hidden);
  const page = (scope) => allPages().find((p) => p.scope === scope);

  /// Whether this user may add apps to a scope: their own, a group they admin, or everyone's as an admin.
  function canArrange(scope) {
    if (scope === "user") return true;
    if (scope === "global") return isAdmin();
    const g = network()?.groups.find((g) => `group:${g.id}` === scope);
    return g?.members.some((m) => m.user === me()?.id && m.role === "admin") || false;
  }

  const appsOn = (p) => h.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
  /// The apps added to a page that have no screen to open: only an AI uses them.
  const forAiOn = (p) => h.apps.filter((a) => !hasScreens(a) && scopes(a).includes(p.scope));
  const accountsOn = (p) => h.apps.filter((a) => accountsTool(a) && scopes(a).includes(p.scope)).sort((a, b) => label(a).localeCompare(label(b)));
  const badge = (name, p) => Math.max(0, h.badges[p.scope]?.[name] || 0);

  /// Every widget a page could show: those of the apps added to its scope.
  function widgetsOn(p) {
    const here = new Set(h.apps.filter((a) => scopes(a).includes(p.scope)).map((a) => a.name));
    return (h.widgets[p.scope] || []).filter((w) => here.has(w.app));
  }

  /// Whether a page's widgets are still to come: not read yet, or reading them failed.
  const waiting = (p) => !h.widgets[p.scope] || h.failed.has(p.scope);

  const SIZES = ["small", "medium", "large", "xl"];
  const supported = (w) => SIZES.filter((s) => !w.sizes || w.sizes[s]);
  const tree = (w, size) => w.sizes?.[size] || w.root;

  /// Which widget on which page, as a page's `late` knows it.
  const lateKey = (scope, w) => `${scope}/${w.app}.${w.widget}`;
  const HOUR = 3_600_000;

  /// A widget as one copy of it on a page draws: at its size, with the options chosen for it. Over an hour old, once
  /// reading it again hasn't brought anything newer, it says when it's from (`old`), as the home-screen widgets do.
  function placed(w, item, scope) {
    const copy = { ...w, instance: item.id, sizes: { ...(w.sizes || {}) } };
    if (h.read.has(scope) && !h.late.has(lateKey(scope, w)) && w.fetched && Date.now() - w.fetched > HOUR) copy.old = w.fetched;
    if (item.size && supported(w).includes(item.size)) copy.size = item.size;
    const mine = item.id && w.placed?.[item.id];
    if (mine?.sizes) Object.assign(copy.sizes, mine.sizes);
    if (mine?.error) copy.error = mine.error;
    return copy;
  }

  /// What a page shows, in its user's order: the widgets they put there and its apps' icons, those added since
  /// they last arranged it at the end, without those they took off it. A page never arranged shows its apps. A widget
  /// still to come keeps its place (`pending`), at its size, as a grey card.
  function items(p) {
    const widgets = widgetsOn(p), seen = new Set();
    return arranged(appsOn(p), h.state?.arrangements?.[p.scope], (item) => {
      const w = widgets.find((w) => w.app === item.app && w.widget === item.widget);
      if (!item.id || seen.has(item.id)) return null;
      if (!w && !((waiting(p) || h.late.has(lateKey(p.scope, item))) && h.apps.some((a) => a.name === item.app && scopes(a).includes(p.scope)))) return null;
      seen.add(item.id);
      return w ? { widget: placed(w, item, p.scope), kept: item } : { pending: true, kept: item };
    });
  }

  // MARK: Where things open

  /// The network's App Store, if it's on a page this sign-in opens.
  function store() {
    const s = app(h.state?.config?.store);
    return s && hasScreens(s) && pages().some((p) => scopes(s).includes(p.scope)) ? s : null;
  }
  /// What a tile runs to open `screen`, with the element it was opened from; `.press`, as the press comes down.
  const opens = (screen, params, app, options = {}) => Object.assign((from) => one.open(screen, params, app, { ...options, from }), { press: () => one.opening(screen, params, app, options) });
  const open = (a, p, screen = "main", params = {}) => opens(screen, params, a.name, p ? { scope: p.scope } : {});
  const settings = (a, p, add) => (has(a, "settings") ? open(a, p, "settings", add ? { add: true } : {}) : null);
  const inStore = (a) => (store() && has(store(), "app") && (a.deploys || 0) > 0 ? open(store(), null, "app", { app: a.name }) : null);
  const addAccountApp = () => (store() ? open(store(), null, "main", { accounts: true }) : null);
  const openStore = () => store() && open(store(), null)();

  // MARK: Changing things

  async function call(a, tool, input = {}) {
    return (await one.api("POST", "/api/call", { app: a, tool, input })).ok;
  }

  /// Says what went wrong, in the shell's own alert.
  const failed = (err) => one.alert({ title: "Couldn’t Do That", text: one.plain(err).title });

  /// Runs a change, then takes in what it changed; says what went wrong if it failed. With `guess`, what it does to
  /// the apps is drawn at once, and undone if it fails.
  async function change(work, guess) {
    const was = h.apps;
    if (guess) h.apps = guess(h.apps), h.redraw?.();
    try {
      await work();
      await load(h.redraw);
    } catch (err) {
      if (guess) h.apps = was, h.redraw?.();
      failed(err);
    }
  }

  /// What an app's menu offers: its settings, its page in the App Store, taking it off this page, and deleting it.
  function actions(a, p) {
    const out = [];
    if (settings(a, p)) out.push({ title: "Settings", icon: "gear-six", run: settings(a, p) });
    if (inStore(a)) out.push({ title: "Open in App Store", icon: "bag", run: inStore(a) });
    if (canArrange(p.scope) && (a.deploys || 0) > 0) {
      out.push({ title: `Remove from ${p.title}`, icon: "minus-circle", danger: true, section: true, run: () => change(() => one.api("POST", "/api/remove/remove", { app: a.name, scope: p.scope }),
        (apps) => apps.map((x) => (x.name === a.name ? { ...x, added: (x.added || []).filter((s) => (s.startsWith("user:") ? "user" : s) !== p.scope) } : x))) });
    }
    return out;
  }

  /// Shows a menu of `entries` at `at` (an event's point, or an element), and runs the one chosen.
  async function menu(entries, at) {
    if (!entries.length) return;
    if (at instanceof Event) at = { x: at.clientX, y: at.clientY };
    const i = await one.menu(entries, at).catch(() => null);
    if (i != null) entries[i]?.run();
  }

  // MARK: Reading

  /// Loads what the screens show: the node's state, its apps, and each page's widgets and badges. `drawn` runs as
  /// soon as there's something new to draw: the state and apps, then each page's widgets and badges as they come, so
  /// a slow one holds up only itself.
  async function load(drawn) {
    [h.state, h.apps] = await Promise.all([one.api("GET", "/api/state"), one.api("GET", "/api/apps")]);
    drawn?.();
    await loadWidgets(null, drawn);
  }

  /// Takes in a page's widgets as read: each stamped with when; one too slow to come with the rest stays as it was
  /// last kept (else a grey card) and is asked for alone, drawn when it comes.
  function took(scope, read, drawn) {
    const was = h.widgets[scope] || [], now = Date.now();
    const same = (w) => (x) => x.app === w.app && x.widget === w.widget;
    h.widgets[scope] = read.flatMap((w) => (w.late ? was.filter(same(w)) : [{ ...w, fetched: now }]));
    for (const w of read.filter((w) => w.late && !h.late.has(lateKey(scope, w)))) {
      const key = lateKey(scope, w);
      h.late.add(key);
      one.api("GET", `/api/widgets?scope=${encodeURIComponent(scope)}&kind=${encodeURIComponent(`${w.app}.${w.widget}`)}`).then(([fresh]) => {
        if (fresh) h.widgets[scope] = [...(h.widgets[scope] || []).filter((x) => !same(fresh)(x)), { ...fresh, fetched: Date.now() }];
      }, () => void h.failed.add(scope)).then(() => {
        h.late.delete(key);
        (drawn || h.redraw)?.();
      });
    }
  }

  /// Reads the widgets and badges of the pages in `only` (scopes), or of every page.
  async function loadWidgets(only, drawn) {
    const q = (p) => `scope=${encodeURIComponent(p.scope)}`;
    await Promise.all(allPages().filter((p) => !only || only.includes(p.scope)).flatMap((p) => [
      one.api("GET", `/api/widgets?${q(p)}`).then((w) => { took(p.scope, w, drawn); h.failed.delete(p.scope); }, () => void h.failed.add(p.scope)).then(() => { h.read.add(p.scope); drawn?.(); }),
      // Badges that couldn't be read stay as they were.
      one.api("GET", `/api/badges?${q(p)}`).then((b) => { h.badges[p.scope] = b; }, () => {}).then(() => drawn?.()),
    ]));
  }

  /// The pages an app is on: those to read again when its data changes.
  const pagesOf = (names) => allPages().filter((p) => h.apps.some((a) => names.includes(a.name) && scopes(a).includes(p.scope))).map((p) => p.scope);

  /// Keeps up: `draw` runs again whenever the node says something changed, and when the soonest widget asks to be
  /// drawn again (a tree's `next`), between a minute and fifteen from now. An app's data changing reads again only
  /// the pages it's on; anything else (who's in, what's where, how pages are arranged) reads everything.
  function watch(draw) {
    let soon, redraw, all = false;
    const only = new Set();
    const again = (these) => {
      if (these) these.forEach((s) => only.add(s));
      else all = true;
      clearTimeout(soon);
      soon = setTimeout(() => {
        // The apps too, for their icons.
        const work = all ? load(draw) : Promise.all([one.api("GET", "/api/apps").then((a) => { h.apps = a; }), loadWidgets([...only], draw)]);
        all = false;
        only.clear();
        work.then(() => { draw(); schedule(); retry(); }).catch(() => {});
      }, 250);
    };
    // Pages whose widgets couldn't be read are asked again in a while.
    let later;
    const retry = () => { clearTimeout(later); if (h.failed.size) later = setTimeout(() => again([...h.failed]), 10_000); };
    const schedule = () => {
      clearTimeout(redraw);
      const nexts = [];
      const walk = (n) => { if (n && typeof n === "object") { if (n.next) nexts.push(n.next); (n.c || []).forEach(walk); } };
      Object.values(h.widgets).flat().forEach((w) => Object.values(w.sizes || { root: w.root }).forEach(walk));
      if (nexts.length) redraw = setTimeout(() => again(allPages().map((p) => p.scope)), Math.min(15 * 60_000, Math.max(60_000, Math.min(...nexts) - Date.now())));
    };
    one.on("changed", (e) => {
      if (e.type === "changed" && e.apps?.length) again(pagesOf(e.apps));
      else if (["changed", "devices", "network"].includes(e.type)) again();
    });
    schedule();
    retry();
    h.redrawn = schedule;
    h.redraw = draw;
  }

  return {
    data: h, me, network, label, app, has, hasScreens, allPages, pages, page, canArrange, appsOn, forAiOn, accountsOn, accountsTool, badge, widgetsOn, waiting, items,
    supported, tree, store, open, opens, settings, inStore, addAccountApp, openStore, call, failed, change, actions, menu, load, loadWidgets, watch,
  };
})();
