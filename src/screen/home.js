// What the pages and Services screens know, the same on every client: the person's pages (theirs, their groups',
// everyone's), the apps on each and what each offers, all read from the node as the person looking (`one.api`).
window.home = (() => {
  const h = { state: null, apps: [], widgets: {}, badges: {} };
  const me = () => h.state?.user;
  const install = () => h.state?.install;
  const isAdmin = () => me()?.role === "admin";
  const label = (a) => a?.title || a?.name || "";
  const app = (name) => h.apps.find((a) => a.name === name);
  const has = (a, screen) => !!a?.screens?.some((s) => s.name === screen);
  const hasScreens = (a) => has(a, "main");
  const isService = (a) => !hasScreens(a) && !(a.widgets || []).length;
  const scopes = (a) => (a.installed || []).map((s) => (s.startsWith("user:") ? "user" : s));
  const userName = (id) => (id === me()?.id ? "You" : install()?.users.find((u) => u.id === id)?.name || "Someone");
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
    const i = install(), u = me();
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

  /// Whether this person may install into a scope: their own, a group they admin, or everyone's as an admin.
  function canArrange(scope) {
    if (scope === "user") return true;
    if (scope === "global") return isAdmin();
    const g = install()?.groups.find((g) => `group:${g.id}` === scope);
    return g?.members.some((m) => m.user === me()?.id && m.role === "admin") || false;
  }

  function owns(a) {
    const owner = a.owner, u = me();
    if (!owner || !u) return false;
    if (owner === "admins") return isAdmin();
    if (owner === `user:${u.id}`) return true;
    const g = owner.startsWith("group:") && install()?.groups.find((g) => g.id === owner.slice(6));
    return !!g && g.members.some((m) => m.user === u.id && m.role === "admin");
  }

  const appsOn = (p) => h.apps.filter((a) => hasScreens(a) && scopes(a).includes(p.scope));
  const servicesOn = (p) => h.apps.filter((a) => isService(a) && scopes(a).includes(p.scope)).sort((a, b) => label(a).localeCompare(label(b)));
  const badge = (name, p) => Math.max(0, h.badges[p.scope]?.[name] || 0);

  /// Every widget a page could show: those of the apps installed in its scope.
  function widgetsOn(p) {
    const here = new Set(h.apps.filter((a) => scopes(a).includes(p.scope)).map((a) => a.name));
    return (h.widgets[p.scope] || []).filter((w) => here.has(w.app));
  }

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

  /// What a page shows, in its person's order: the widgets they put there and its apps' icons, those installed since
  /// they last arranged it at the end, without those they took off it. A page never arranged shows its apps.
  function items(p) {
    const apps = appsOn(p), widgets = widgetsOn(p);
    const arrangement = h.state?.arrangements?.[p.scope];
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

  // MARK: Where things open

  /// The install's App Store, if it's on a page this sign-in opens.
  function store() {
    const s = app(h.state?.config?.store);
    return s && hasScreens(s) && pages().some((p) => scopes(s).includes(p.scope)) ? s : null;
  }
  const open = (a, p, screen = "main", params = {}) => () => one.open(screen, params, a.name, p ? { scope: p.scope } : {});
  const settings = (a, p, add) => (has(a, "settings") ? open(a, p, "settings", add ? { add: true } : {}) : null);
  const inStore = (a) => (store() && has(store(), "app") && (a.deploys || 0) > 0 ? open(store(), null, "app", { app: a.name }) : null);
  const addService = () => (store() ? open(store(), null, "main", { service: true }) : null);
  const openStore = () => store() && open(store(), null)();

  // MARK: Changing things

  async function call(a, op, input = {}) {
    return (await one.api("POST", "/v1/call", { app: a, op, input })).ok;
  }

  /// Says what went wrong, in the shell's own alert.
  const failed = (err) => one.alert({ title: "Couldn’t Do That", text: one.plain(err).title });

  /// Runs a change, then takes in what it changed; says what went wrong if it failed.
  async function change(work) {
    try {
      await work();
      await load();
    } catch (err) {
      failed(err);
    }
  }

  /// What an app's menu offers: its settings, its page in the App Store, taking it off this page, and deleting it.
  function actions(a, p) {
    const out = [];
    if (settings(a, p)) out.push({ title: "Settings", icon: "gear-six", run: settings(a, p) });
    if (inStore(a)) out.push({ title: "Open in App Store", icon: "bag", run: inStore(a) });
    if (canArrange(p.scope) && (a.deploys || 0) > 0) {
      out.push({ title: `Remove from ${p.title}`, icon: "minus-circle", danger: true, section: true, run: () => change(() => one.api("POST", "/v1/install", { app: a.name, scope: p.scope, installed: false })) });
    }
    if (owns(a) && (a.deploys || 0) > 0) {
      out.push({
        title: "Delete App…", icon: "trash", danger: true,
        run: async () => {
          const sure = await one.alert({
            title: `Delete ${label(a)}?`, action: "Delete App", danger: true,
            text: `${label(a)} leaves every page, for everyone. Restore it from Recently Deleted in the App Store.`,
          });
          if (sure) change(() => one.api("POST", "/v1/retire", { app: a.name, how: "deleted" }));
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

  /// Loads what the screens show: the node's state, its apps, and each page's widgets and badges.
  async function load() {
    [h.state, h.apps] = await Promise.all([one.api("GET", "/v1/state"), one.api("GET", "/v1/apps")]);
    await loadWidgets();
  }

  async function loadWidgets() {
    await Promise.all(allPages().map(async (p) => {
      [h.widgets[p.scope], h.badges[p.scope]] = await Promise.all([
        one.api("GET", `/v1/widgets?scope=${encodeURIComponent(p.scope)}`).catch(() => h.widgets[p.scope] || []),
        one.api("GET", `/v1/badges?scope=${encodeURIComponent(p.scope)}`).catch(() => ({})),
      ]);
    }));
  }

  /// Keeps up: `draw` runs again whenever the node says something changed, and when the soonest widget asks to be
  /// drawn again (a tree's `next`), between a minute and fifteen from now.
  function watch(draw) {
    let soon, redraw;
    const again = (work) => {
      clearTimeout(soon);
      soon = setTimeout(() => work().then(() => { draw(); schedule(); }).catch(() => {}), 250);
    };
    const schedule = () => {
      clearTimeout(redraw);
      const nexts = [];
      const walk = (n) => { if (n && typeof n === "object") { if (n.next) nexts.push(n.next); (n.c || []).forEach(walk); } };
      Object.values(h.widgets).flat().forEach((w) => Object.values(w.sizes || { root: w.root }).forEach(walk));
      if (nexts.length) redraw = setTimeout(() => again(loadWidgets), Math.min(15 * 60_000, Math.max(60_000, Math.min(...nexts) - Date.now())));
    };
    one.on("changed", (e) => {
      if (["changed", "devices", "install"].includes(e.type)) again(load);
    });
    schedule();
    h.redrawn = schedule;
  }

  return {
    data: h, me, install, label, app, has, hasScreens, allPages, pageChoices, pages, page, canArrange, appsOn, servicesOn, badge, widgetsOn, items,
    supported, tree, store, open, settings, inStore, addService, openStore, call, failed, change, actions, menu, load, loadWidgets, watch,
  };
})();
