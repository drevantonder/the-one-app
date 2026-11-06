// What the pages and Accounts screens know, the same on every client: the person's pages (theirs, their groups',
// everyone's), the apps on each and what each offers, all read from the node as the person looking (`one.api`).
export const home = (() => {
  // `failed`: the pages whose widgets couldn't be read, so theirs stand as grey cards until they can.
  const h = { state: null, apps: [], widgets: {}, badges: {}, failed: new Set() };
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
  const userName = (id) => (id === me()?.id ? "You" : network()?.users.find((u) => u.id === id)?.name || "Someone");
  // People by name, you last, as "you".
  const named = (ids) => ids.slice().sort((a, b) => (a === me()?.id) - (b === me()?.id) || userName(a).localeCompare(userName(b)))
    .map((id) => (id === me()?.id ? "you" : userName(id)));
  const and = (names) => (names.length < 2 ? names.join("") : `${names.slice(0, -1).join(", ")} and ${names.at(-1)}`);
  // Who a page is for, and who chooses its apps: a few people by name, more as how many; with only you, nothing.
  const whose = (ids, admins) =>
    ids.length === 1 && ids[0] === me()?.id ? "" : `${ids.length <= 3 ? and(named(ids)).replace(/^y/, "Y") : `${ids.length} people`} · ${chosenBy(admins)}`;

  function chosenBy(admins) {
    const names = named(admins);
    if (!names.length) return "apps chosen by admins";
    if (names.length === 1) return `apps chosen by ${names[0]}`;
    if (names.length === 2) return `apps chosen by ${names[0]} and ${names[1]}`;
    return `apps chosen by ${names.length} admins`;
  }

  /// Every page this person can read: theirs, their groups', then everyone's.
  function allPages() {
    const i = network(), u = me();
    if (!i || !u) return [];
    const out = [{ scope: "user", title: "You", subtitle: "" }];
    for (const g of i.groups.filter((g) => g.members.some((m) => m.user === u.id)).sort((a, b) => a.name.localeCompare(b.name))) {
      const admins = g.members.filter((m) => m.role === "admin").map((m) => m.user);
      out.push({ scope: `group:${g.id}`, title: g.name, subtitle: whose(g.members.map((m) => m.user), admins) });
    }
    out.push({ scope: "global", title: "Everyone", subtitle: whose(i.users.map((x) => x.id), i.users.filter((x) => x.role === "admin").map((x) => x.id)) });
    return out;
  }
  /// How this person ordered their groups' pages and which they hid (Edit Pages…), kept with their own page's arrangement.
  function pageChoices() {
    const kept = h.state?.arrangements?.user?.pages;
    return { order: kept?.order || [], hidden: kept?.hidden || [] };
  }

  /// The pages this person shows, in their order: theirs first, then their groups' as they ordered them (any they
  /// haven't ordered after, by name), everyone's last.
  function pages() {
    const all = allPages(), { order, hidden } = pageChoices();
    const rank = (p) => (order.includes(p.scope) ? order.indexOf(p.scope) : order.length + all.indexOf(p));
    const groups = all.filter((p) => p.scope.startsWith("group:") && !hidden.includes(p.scope)).sort((a, b) => rank(a) - rank(b));
    return [...all.filter((p) => p.scope === "user"), ...groups, ...all.filter((p) => p.scope === "global")];
  }
  const page = (scope) => allPages().find((p) => p.scope === scope);

  /// Whether this person may add apps to a scope: their own, a group they admin, or everyone's as an admin.
  function canArrange(scope) {
    if (scope === "user") return true;
    if (scope === "global") return isAdmin();
    const g = network()?.groups.find((g) => `group:${g.id}` === scope);
    return g?.members.some((m) => m.user === me()?.id && m.role === "admin") || false;
  }

  function owns(a) {
    const owner = a.owner, u = me();
    if (!owner || !u) return false;
    if (owner === "admins") return isAdmin();
    if (owner === `user:${u.id}`) return true;
    const g = owner.startsWith("group:") && network()?.groups.find((g) => g.id === owner.slice(6));
    return !!g && g.members.some((m) => m.user === u.id && m.role === "admin");
  }

  const appsOn = (p) => h.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
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

  /// A widget as one copy of it on a page draws: at its size, with the options chosen for it.
  function placed(w, item) {
    const copy = { ...w, instance: item.id, sizes: { ...(w.sizes || {}) } };
    if (item.size && supported(w).includes(item.size)) copy.size = item.size;
    const mine = item.id && w.placed?.[item.id];
    if (mine?.sizes) Object.assign(copy.sizes, mine.sizes);
    if (mine?.error) copy.error = mine.error;
    return copy;
  }

  /// What a page shows, in its person's order: the widgets they put there and its apps' icons, those added since
  /// they last arranged it at the end, without those they took off it. A page never arranged shows its apps. A widget
  /// still to come keeps its place (`pending`), at its size, as a grey card.
  function items(p) {
    const apps = appsOn(p), widgets = widgetsOn(p);
    const arrangement = h.state?.arrangements?.[p.scope];
    if (!arrangement) return apps.map((a) => ({ app: a }));
    const out = [], seen = new Set();
    for (const item of arrangement.items || []) {
      if (item.widget) {
        const w = widgets.find((w) => w.app === item.app && w.widget === item.widget);
        if (!item.id || seen.has(`widget:${item.id}`)) continue;
        if (!w && !(waiting(p) && h.apps.some((a) => a.name === item.app && scopes(a).includes(p.scope)))) continue;
        seen.add(`widget:${item.id}`);
        out.push(w ? { widget: placed(w, item), kept: item } : { pending: true, kept: item });
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

  // MARK: Where things open

  /// The network's App Store, if it's on a page this sign-in opens.
  function store() {
    const s = app(h.state?.config?.store);
    return s && hasScreens(s) && pages().some((p) => scopes(s).includes(p.scope)) ? s : null;
  }
  const open = (a, p, screen = "main", params = {}) => () => one.open(screen, params, a.name, p ? { scope: p.scope } : {});
  const settings = (a, p, add) => (has(a, "settings") ? open(a, p, "settings", add ? { add: true } : {}) : null);
  const inStore = (a) => (store() && has(store(), "app") && (a.deploys || 0) > 0 ? open(store(), null, "app", { app: a.name }) : null);
  const addAccountApp = () => (store() ? open(store(), null, "main", { accounts: true }) : null);
  const openStore = () => store() && open(store(), null)();

  // MARK: Changing things

  async function call(a, tool, input = {}) {
    return (await one.api("POST", "/v1/call", { app: a, tool, input })).ok;
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
      out.push({ title: `Remove from ${p.title}`, icon: "minus-circle", danger: true, section: true, run: () => change(() => one.api("POST", "/v1/add", { app: a.name, scope: p.scope, added: false }),
        (apps) => apps.map((x) => (x.name === a.name ? { ...x, added: (x.added || []).filter((s) => (s.startsWith("user:") ? "user" : s) !== p.scope) } : x))) });
    }
    if (owns(a) && (a.deploys || 0) > 0) {
      out.push({
        title: "Delete App…", icon: "trash", danger: true,
        run: async () => {
          const sure = await one.alert({
            title: `Delete ${label(a)}?`, action: "Delete App", danger: true,
            text: `${label(a)} leaves every page, for everyone. Restore it from Recently Deleted in the App Store.`,
          });
          if (sure) change(() => one.api("POST", "/v1/retire", { app: a.name, how: "deleted" }), (apps) => apps.filter((x) => x.name !== a.name));
        },
      });
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
    [h.state, h.apps] = await Promise.all([one.api("GET", "/v1/state"), one.api("GET", "/v1/apps")]);
    drawn?.();
    await loadWidgets(null, drawn);
  }

  /// Reads the widgets and badges of the pages in `only` (scopes), or of every page.
  async function loadWidgets(only, drawn) {
    const q = (p) => `scope=${encodeURIComponent(p.scope)}`;
    await Promise.all(allPages().filter((p) => !only || only.includes(p.scope)).flatMap((p) => [
      one.api("GET", `/v1/widgets?${q(p)}`).then((w) => { h.widgets[p.scope] = w; h.failed.delete(p.scope); }, () => void h.failed.add(p.scope)).then(() => drawn?.()),
      // Badges that couldn't be read stay as they were.
      one.api("GET", `/v1/badges?${q(p)}`).then((b) => { h.badges[p.scope] = b; }, () => {}).then(() => drawn?.()),
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
        const work = all ? load(draw) : Promise.all([one.api("GET", "/v1/apps").then((a) => { h.apps = a; }), loadWidgets([...only], draw)]);
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
    data: h, me, network, label, app, has, hasScreens, allPages, pageChoices, pages, page, canArrange, appsOn, accountsOn, accountsTool, badge, widgetsOn, waiting, items,
    supported, tree, store, open, settings, inStore, addAccountApp, openStore, call, failed, change, actions, menu, load, loadWidgets, watch,
  };
})();
