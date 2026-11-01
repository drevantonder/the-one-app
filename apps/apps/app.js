// The App Store: the apps built into the release, whether or not the install has added them, and the apps made in the
// install; who owns each, where it's installed, its forks and its code; and "Add to…" each place the person has a
// page: themselves, their groups and everyone, saying who can add it where they can't.
// An ordinary app: it installs with ctx.install, with the caller's own rights. Installing a built-in app the install
// hasn't added yet adds it (admins only).

/// The caller's pages, where apps install: [{ scope, id, name, may, why }]; `why` says who can when they can't.
function places(ctx) {
  const out = [{ scope: "user", id: `user:${ctx.me.user}`, name: "You", may: true }];
  for (const g of ctx.groups) {
    const may = g.role === "admin";
    out.push({ scope: g.id, id: g.id, name: g.name, may, why: may ? null : `Only ${g.name}'s admins can add apps to its page.` });
  }
  const admin = ctx.me.role === "admin";
  out.push({ scope: "global", id: "global", name: "Everyone", may: admin, why: admin ? null : "Only the install's admins can add apps for everyone. Settings shows who they are." });
  return out;
}

/// Whether the caller acts for an owner (an app's, a server's): it's theirs, a group's they're an admin of, or admins'
/// or the install's and they're an admin.
function mine(ctx, owner) {
  if (owner === `user:${ctx.me.user}`) return true;
  if (owner === "admins" || owner === "install") return ctx.me.role === "admin";
  return ctx.groups.some((g) => g.id === owner && g.role === "admin");
}

/// What to call a scope or owner: You, a group's name, Everyone, admins.
function namer(ctx) {
  const groups = Object.fromEntries(ctx.groups.map((g) => [g.id, g.name]));
  return (id) => {
    if (id === `user:${ctx.me.user}`) return "You";
    if (id === "global") return "Everyone";
    if (id === "admins") return "Admins";
    if (groups[id]) return groups[id];
    return id.startsWith("user:") ? "Someone" : "A group";
  };
}

/// A bundle's files: a plain ustar tar.
function untar(bytes) {
  const files = [];
  const text = (from, to) => new TextDecoder().decode(bytes.subarray(from, to)).replace(/\0.*$/s, "").trim();
  for (let at = 0; at + 512 <= bytes.length; ) {
    if (bytes.subarray(at, at + 512).every((b) => b === 0)) break;
    const name = text(at, at + 100), prefix = text(at + 345, at + 500);
    const size = parseInt(text(at + 124, at + 136), 8) || 0;
    const type = String.fromCharCode(bytes[at + 156] || 48);
    if (type === "0") files.push({ path: prefix ? `${prefix}/${name}` : name, bytes: bytes.subarray(at + 512, at + 512 + size) });
    at += 512 + Math.ceil(size / 512) * 512;
  }
  return files;
}

/// What a built-in app the install hasn't added says about itself: nothing runs it yet, so its code says (the first
/// `title`, `icon`, `color` and `description` after `export default` given as plain strings, as the built-in apps
/// give them before their ops).
async function described(ctx, bundle) {
  const file = untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const all = file ? new TextDecoder().decode(file.bytes) : "";
  const code = all.slice(Math.max(0, all.indexOf("export default")));
  const field = (key) => code.match(new RegExp(`\\b${key}:\\s*"([^"]*)"`))?.[1] || null;
  return { title: field("title"), icon: field("icon"), color: field("color"), description: field("description") };
}

/// Whether an app is a connector (SPEC.md "Connectors"): it has accounts, sign_in and sign_out, and a settings screen
/// but no main one. What it says about itself where it's installed for the caller; elsewhere, its code.
async function isConnector(ctx, a) {
  const has = (list, name) => (list || []).some((x) => x.name === name);
  if (a.ops) return has(a.ops, "accounts") && has(a.ops, "sign_in") && has(a.ops, "sign_out") && !has(a.screens, "main");
  const bundle = a.bundle || a.built_in;
  const file = bundle && untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const code = file ? new TextDecoder().decode(file.bytes) : "";
  return ["accounts", "sign_in", "sign_out", "settings"].every((k) => new RegExp(`\\b${k}:\\s*\\{`).test(code)) && !/\bmain:\s*\{/.test(code);
}

/// Every driver on the caller's servers, and each driver an app uses that none of them runs: what it is, where it runs
/// (version, who installed it, whether the caller may remove it there), where it was removed, and the apps that use it.
async function drivers(ctx) {
  const [servers, catalog] = await Promise.all([ctx.servers(), ctx.catalog()]);
  const owns = (s) => (s.owners || []).some((o) => mine(ctx, o));
  const by = new Map();
  const driver = (name) => {
    if (!by.has(name)) by.set(name, { name, title: name, icon: null, color: null, image: null, description: null, install: null, servers: [], removed: [], apps: [] });
    return by.get(name);
  };
  for (const s of servers) {
    for (const d of s.installed_drivers || []) {
      const it = driver(d.name);
      for (const k of ["title", "icon", "color", "image", "description", "install"]) it[k] = it[k] && it[k] !== it.name ? it[k] : d[k] ?? it[k];
      it.servers.push({ server: s.name, online: s.online, here: s.here, version: d.version || null, built_in: !!d.built_in, installed_by: d.installed_by || null, users: d.users || [], may_remove: owns(s) });
    }
    for (const d of s.removed_drivers || []) driver(d.name).removed.push({ server: s.name, by: d.by || null, time: d.time, may_allow: owns(s) });
  }
  for (const a of catalog.filter((a) => !a.deleted)) for (const name of a.uses?.drivers || []) driver(name).apps.push({ name: a.name, title: a.title || a.name });
  // A driver without a picture of its own (its Mac app's icon) shows its app's.
  for (const d of by.values()) {
    const app = catalog.find((a) => a.name === d.name);
    d.image = d.image || app?.image || null;
    if (d.title === d.name && app?.title) d.title = app.title;
  }
  return { drivers: [...by.values()].sort((a, b) => a.title.localeCompare(b.title)), servers: servers.filter(owns).map((s) => s.name) };
}

async function find(ctx, name) {
  const app = (await ctx.catalog()).find((a) => a.name === name);
  if (!app) throw new Error(`no app named ${name}`);
  return app;
}

/// A folder of an app's source (src/ in its bundle) for the Code screens, folders first; at the top, the module it
/// runs as. Runs in the page: the screens include its source.
function folder(files, dir) {
  const here = new Map();
  for (const f of files.filter((f) => f.path.startsWith("src/" + dir))) {
    const [first, ...rest] = f.path.slice(4 + dir.length).split("/");
    here.set(rest.length ? first + "/" : first, rest.length ? { dir: dir + first + "/" } : f);
  }
  const natural = new Intl.Collator(undefined, { numeric: true }).compare;
  const names = [...here.keys()].sort((a, b) => b.endsWith("/") - a.endsWith("/") || natural(a, b));
  const built = files.find((f) => f.path === "app.js");
  if (!dir && built) names.push("app.js, as it runs"), here.set("app.js, as it runs", built);
  const row = (n) => {
    const f = here.get(n);
    const open = f.dir ? ["code", { app: one.params.app, dir: f.dir }] : ["file", { app: one.params.app, path: f.path }];
    return `<a class="row plain" onclick="${one.esc(`one.open(${JSON.stringify(open[0])}, ${JSON.stringify(open[1])})`)}">${one.icon(f.dir ? "folder" : "file", "code-icon")}
      <div class="main"><div class="title"><span>${one.esc(n.replace(/\/$/, ""))}</span><span class="when">${f.dir ? "" : one.size(f.size)}</span></div></div><div class="chev">›</div></a>`;
  };
  return '<div class="list code-list">' + names.map(row).join("") + "</div>";
}

/// How to install a driver on a server you own, for the Drivers screens. Runs in the page: the screens include it.
function install(servers, name) {
  const driver = name || "<driver>";
  const where = servers.length ? servers[0] : "<server>";
  return `<h2 class="section">Install a driver</h2><div class="list"><div class="row plain"><div class="main">
    <div class="sub">A server's owner installs a driver there. From ${one.esc(one.config.name)}'s code, on any machine that reaches the server over ssh:</div>
    <pre class="small" style="margin:8px 0;white-space:pre-wrap"><code>providers/install.sh ${one.esc(driver)} ${one.esc(where)}</code></pre>
    <div class="sub small">${servers.length ? "You own " + one.esc(servers.join(", ")) + "." : "You own none of the servers, so ask one of a server's owners."} Settings shows each server's owners.</div>
  </div></div></div>`;
}

export default {
  name: "apps",
  title: "App Store",
  icon: "square.grid.2x2",
  color: "#007aff",
  description: "Every app you can add, and its code",
  instructions: "The App Store lists every app in this install, where each is installed and its code. apps_add installs one for you, a group you're an admin of, or everyone (admins).",

  // The apps whose schedule, where you may switch it, failed its last run.
  async badge(ctx) {
    return (await ctx.catalog()).filter((a) => (a.schedules || []).some((s) => s.on && s.may_switch && s.last && !s.last.ok)).length;
  },

  ops: {
    list: {
      reads: true,
      description: "Every app: the built-in ones (built_in), added to the install or not and up to date or not, and the ones made in the install; owner, where it's installed (that you can see), forks, and the places you may add it to. Each says whether it's a connector: it only connects an account (Gmail, say), and shells keep it in each page's Connectors folder. With connector, only those.",
      // `connected` is connector's old name, which shells built before still send.
      input: { type: "object", properties: { connector: { type: "boolean", description: "only connectors" }, connected: { type: "boolean", description: "the same as connector" } } },
      async run(ctx, { connector, connected } = {}) {
        const only = connector ?? connected;
        const name = namer(ctx);
        const all = (await ctx.catalog()).filter((a) => !a.deleted || !only);
        const connectors = await Promise.all(all.map((a) => isConnector(ctx, a).catch(() => false)));
        const apps = await Promise.all(all.filter((a, i) => connectors[i] || !only).map(async (a) => {
          const added = a.deploys > 0;
          const d = added || !a.built_in ? a : await described(ctx, a.built_in).catch(() => ({}));
          return {
            name: a.name, title: d.title || a.title || a.name, icon: d.icon || a.icon || null, color: d.color || a.color || null, image: a.image || null, description: d.description || a.description || null,
            owner: a.owner, owner_name: name(a.owner),
            from: a.from || null, forks: a.forks || [], deploys: a.deploys, installed: a.installed || [],
            installed_names: (a.installed || []).map(name), uses: a.uses || { apps: [], drivers: [] },
            built_in: !!a.built_in, added, up_to_date: a.built_in && added ? a.bundle === a.built_in : null,
            mine: mine(ctx, a.owner), deleted: a.deleted || null, connector: connectors[all.indexOf(a)],
            connected: connectors[all.indexOf(a)], // connector's old name, for whatever read it before
          };
        }));
        apps.sort((a, b) => a.title.localeCompare(b.title));
        return { apps, places: places(ctx), admin: ctx.me.role === "admin" };
      },
    },
    add: {
      description: "Install an app, and the apps it uses, in a place: user (you), group:<id> (a group you're an admin of) or global (everyone; admins only).",
      input: { type: "object", required: ["app", "scope"], properties: { app: { type: "string" }, scope: { type: "string" } } },
      async run(ctx, { app, scope }) {
        await find(ctx, app);
        ctx.install(app, scope);
        return `Added ${app}.`;
      },
    },
    remove: {
      confirm: true,
      description: "Uninstall an app from a place: user, group:<id> or global.",
      input: { type: "object", required: ["app", "scope"], properties: { app: { type: "string" }, scope: { type: "string" } } },
      async run(ctx, { app, scope }) {
        ctx.install(app, scope, false);
        return `Removed ${app}.`;
      },
    },
    delete: {
      confirm: true,
      description: "Delete an app you own: it leaves every page and stops running, and waits in Recently Deleted, where everyone sees it and its owners restore it or erase it for good.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" } } },
      async run(ctx, { app }) {
        ctx.retire(app, "deleted");
        return `Deleted ${app}: it's in Recently Deleted.`;
      },
    },
    restore: {
      description: "Restore an app you own from Recently Deleted: it runs again, on the pages it was on.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" } } },
      async run(ctx, { app }) {
        ctx.retire(app, "restored");
        return `Restored ${app}.`;
      },
    },
    erase: {
      confirm: true,
      description: "Erase a deleted app you own for good: it's gone from every list and its name can't be used again.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" } } },
      async run(ctx, { app }) {
        ctx.retire(app, "erased");
        return `Erased ${app}.`;
      },
    },
    health: {
      description: "How an app is doing where it's installed for you: for its owners, who used it, when and from what, its errors grouped with counts, and its AI cost; its schedules (when each runs, on or off, as whom, whether a deploy changed or removed it) and their recent runs and failures.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" } } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return { schedules: a.schedules || [], activity: a.activity || [], insights: a.insights || null };
      },
    },
    switch: {
      description: "Switch one of an app's schedules on or off in a place where it's installed (user, group:<id> or global); switched on, it runs as you.",
      input: { type: "object", required: ["app", "schedule", "scope", "on"], properties: { app: { type: "string" }, schedule: { type: "string" }, scope: { type: "string" }, on: { type: "boolean" } } },
      async run(ctx, { app, schedule, scope, on }) {
        ctx.schedule(app, schedule, scope, on);
        return `${schedule} is ${on ? "on" : "off"}.`;
      },
    },
    drivers: {
      reads: true,
      description: "Every driver on your servers, the native code apps use (a shell, WhatsApp's linked phone, a browser), and each one an app uses that none of them runs: what it is, which servers run it (its version there, who installed it, whom it serves), where an owner removed it, and the apps that use it. servers lists the servers you own, where you may install and remove drivers.",
      input: { type: "object", properties: { name: { type: "string", description: "only this driver" } } },
      async run(ctx, { name } = {}) {
        const all = await drivers(ctx);
        return name ? { ...all, drivers: all.drivers.filter((d) => d.name === name) } : all;
      },
    },
    remove_driver: {
      confirm: true,
      description: "Remove a driver from a server you own: it stops serving there, and apps that use it go to another server that runs it, or say none does. Its own data stays on the server; allow_driver undoes it.",
      input: { type: "object", required: ["driver", "server"], properties: { driver: { type: "string" }, server: { type: "string", description: "the server, by name" } } },
      async run(ctx, { driver, server }) {
        ctx.driver(driver, server, false);
        return `Removed ${driver} from ${server}.`;
      },
    },
    allow_driver: {
      description: "Allow a driver an owner removed from a server you own again: it serves there again while it runs.",
      input: { type: "object", required: ["driver", "server"], properties: { driver: { type: "string" }, server: { type: "string", description: "the server, by name" } } },
      async run(ctx, { driver, server }) {
        ctx.driver(driver, server, true);
        return `${driver} is allowed on ${server} again.`;
      },
    },
    code: {
      reads: true,
      description: "An app's code as deployed: its files, or with path one of them. Everyone may read every app's code.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" }, path: { type: "string" } } },
      async run(ctx, { app, path }) {
        const a = await find(ctx, app);
        const bundle = a.bundle || a.built_in;
        if (!bundle) throw new Error(`${app} has no deploy yet`);
        const files = untar(await ctx.blob.get(bundle));
        if (!path) return { bundle, files: files.map((f) => ({ path: f.path, size: f.bytes.length })) };
        const file = files.find((f) => f.path === path);
        if (!file) throw new Error(`${app} has no ${path}`);
        return { path, text: new TextDecoder().decode(file.bytes) };
      },
    },
  },

  screens: {
    main: {
      title: "App Store",
      html: `
<style>
  .tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 3px; }
  .tag { font-size: 12px; padding: 1px 7px; border-radius: 10px; background: var(--line); color: var(--muted); }
</style>
<div class="segmented" id="tabs"><button data-tab="apps">Apps</button><button data-tab="connectors">Connectors</button><button data-tab="drivers">Drivers</button></div>
<p id="about" class="muted small" style="margin:0 4px 10px" hidden></p>
<input id="search" class="search" type="search" placeholder="Find an app" autocomplete="off">
<div id="list"></div>
<script>
${install}
// Connectors would bury the rest, so they get their own tab. A shell's Add a Connector opens it with { connector: true }
// ({ connected: true } from shells built before), on that tab.
let tab = one.params.connector || one.params.connected ? "connectors" : one.params.drivers ? "drivers" : "apps";
const ABOUT = {
  connectors: "Connectors link an account for you and your agents. Add one to a page, then sign in from that page’s Connectors.",
  drivers: "Drivers are the native code apps use: a shell, a local model, WhatsApp’s linked phone, a desktop’s browser. Each runs on the servers whose owners installed it.",
};
function pick(t) {
  tab = t;
  for (const b of document.querySelectorAll("#tabs button")) b.classList.toggle("on", b.dataset.tab === tab);
  const about = document.getElementById("about");
  about.textContent = ABOUT[tab] || "";
  about.hidden = !ABOUT[tab];
  document.getElementById("search").placeholder = tab === "drivers" ? "Find a driver" : "Find an app";
  draw();
}
document.getElementById("tabs").addEventListener("click", (e) => e.target.dataset.tab && pick(e.target.dataset.tab));
// The sections: the apps built into the release, and those made in this install (each install words that its own way,
// in its settings).
const BUILT_IN = "Built In";
const MADE_HERE = one.config?.made_here || "Made here";
const CLI = one.config?.cli || "one";

/// A built-in app's state here; for an app made here, what it was forked from.
function status(a) {
  if (!a.built_in) return a.from ? "Fork of " + a.from : "";
  if (!a.added) return "Not added";
  return a.up_to_date ? "Up to date" : "Other version";
}

let apps = [];
const row = (a) => \`<a class="row" onclick='one.open("app", \${JSON.stringify({ app: a.name })})'>
    \${one.appIcon(a, 44)}
    <div class="main">
      <div class="title"><span>\${one.esc(a.title)}</span><span class="when">\${one.esc(status(a))}</span></div>
      <div class="sub">\${one.esc(a.description || "Made by " + a.owner_name)}</div>
      \${a.installed.length ? '<div class="tags">' + a.installed_names.map((n) => '<span class="tag">' + one.esc(n) + "</span>").join("") + "</div>" : ""}
    </div>
    <div class="chev">›</div>
  </a>\`;
const section = (title, list) => list.length ? '<h2 class="section">' + one.esc(title) + '</h2><div class="list">' + list.map(row).join("") + "</div>" : "";

let drivers = null;
/// Where a driver runs, in a few words.
const runsOn = (d) => d.servers.length ? "On " + d.servers.map((s) => s.server).join(", ") : d.removed.length ? "Removed from " + d.removed.map((r) => r.server).join(", ") : "On none of your servers";
const driverRow = (d) => \`<a class="row" onclick='one.open("driver", \${JSON.stringify({ driver: d.name })})'>
    \${one.appIcon(d, 44)}
    <div class="main">
      <div class="title"><span>\${one.esc(d.title)}</span></div>
      \${d.description ? '<div class="sub">' + one.esc(d.description) + "</div>" : ""}
      <div class="sub small">\${one.esc(runsOn(d))}</div>
    </div>
    <div class="chev">›</div>
  </a>\`;
function drawDrivers(q) {
  if (!drivers) return (document.getElementById("list").innerHTML = '<div class="empty">Loading drivers…</div>');
  const found = drivers.drivers.filter((d) => !q || [d.title, d.name, d.description || ""].some((t) => t.toLowerCase().includes(q)));
  const running = found.filter((d) => d.servers.length), not = found.filter((d) => !d.servers.length);
  document.getElementById("list").innerHTML = (found.length
    ? (running.length ? '<h2 class="section">On your servers</h2><div class="list">' + running.map(driverRow).join("") + "</div>" : "")
      + (not.length ? '<h2 class="section">Not running</h2><div class="list">' + not.map(driverRow).join("") + "</div>" : "")
    : '<div class="empty">No drivers match.</div>') + install(drivers.servers);
}
function draw() {
  const q = document.getElementById("search").value.trim().toLowerCase();
  if (tab === "drivers") return drawDrivers(q);
  const found = apps.filter((a) => !!a.connector === (tab === "connectors") && (!q || [a.title, a.name, a.description || ""].some((t) => t.toLowerCase().includes(q))));
  document.getElementById("list").innerHTML = found.length
    ? section(BUILT_IN, found.filter((a) => a.built_in && !a.deleted)) + section(MADE_HERE, found.filter((a) => !a.built_in && !a.deleted)) + section("Recently Deleted", found.filter((a) => a.deleted))
    : '<div class="empty">No apps match.<div class="small" style="margin-top:6px">Make your own: put an app.js in a folder and run <code>' + one.esc(CLI) + ' deploy</code> on it.</div></div>';
}
async function show() {
  [apps, drivers] = await Promise.all([one.call("list").then((l) => l.apps), one.call("drivers").catch(() => ({ drivers: [], servers: [] }))]);
  draw();
}
document.getElementById("search").addEventListener("input", draw);
one.on("changed", show);
pick(tab);
show();
</script>`,
    },
    app: {
      title: "App",
      html: `
<style>
  .head { display: flex; gap: 14px; align-items: center; margin: 12px 4px 4px; }
  .head h1 { font-size: 22px; margin: 0; }
  .about { margin: 12px 4px 0; }
  .pill { font: inherit; font-size: 15px; font-weight: 600; padding: 5px 14px; border-radius: 15px; border: 0; background: var(--accent); color: white; }
  .pill.off { background: var(--line); color: var(--accent); }
  .pill:disabled { opacity: 0.4; }
  .pill.danger { background: var(--danger); }
  #error { margin: 8px 4px; }
  .code-icon { width: 20px; height: 20px; color: var(--muted); }
</style>
<div id="app"></div>
<div id="ask"></div>
<div id="error" class="error small"></div>
<script>
${folder}
const name = one.params.app;
one.title(name);

/// The app's page, section by section: each takes what show() loaded and gives HTML, or "" to leave itself out.
const sections = [
  function head({ a }) {
    const state = a.built_in ? (!a.added ? "Built in, not added" : a.up_to_date ? "Built in, up to date" : "Built in, running another version than this release's: the next update deploys the release's") : "Made by " + a.owner_name;
    return \`<div class="head">\${one.appIcon(a, 60)}
      <div><h1>\${one.esc(a.title)}</h1><div class="muted small">\${one.esc(a.name)} · \${one.esc(state)} · \${a.deploys} deploy\${a.deploys === 1 ? "" : "s"}</div></div></div>
      \${a.description ? '<div class="about">' + one.esc(a.description) + "</div>" : ""}\`;
  },
  function deleted({ a }) {
    if (!a.deleted) return "";
    const by = a.deleted.by ? " by " + one.esc(a.deleted.by) : "";
    const buttons = a.mine
      ? \`<div class="row plain"><div class="main"><div class="title"><span>Restore</span></div><div class="sub small">It runs again, on the pages it was on.</div></div><button class="pill" onclick="retire(this, 'restore')">Restore</button></div>
         <div class="row plain"><div class="main"><div class="title"><span>Erase</span></div><div class="sub small">Gone for good: its name can't be used again.</div></div><button class="pill danger" onclick="retire(this, 'erase')">Erase</button></div>\`
      : '<div class="row plain"><div class="main sub small">Only its owners can restore or erase it.</div></div>';
    return '<h2 class="section">Deleted ' + ago(a.deleted.time) + by + '</h2><div class="list">' + buttons + "</div>";
  },
  function addTo({ a, places, admin }) {
    if (a.deleted) return "";
    const first = !a.added && !admin ? \`Only an admin can add \${a.title} to this install. Settings shows who they are.\` : null;
    const place = (p) => {
      const on = a.installed.includes(p.id);
      const why = first || p.why;
      return \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(p.name)}</span></div>
          \${why ? '<div class="sub small">' + one.esc(why) + "</div>" : ""}</div>
        <button class="pill\${on ? " off" : ""}"\${why ? " disabled" : ""} onclick='toggle(this, \${JSON.stringify(p.scope)}, \${on})'>\${on ? "Remove" : "Add"}</button></div>\`;
    };
    return '<h2 class="section">Add to…</h2><div class="list">' + places.map(place).join("") + "</div>";
  },
  function schedules({ health }) {
    const list = health?.schedules || [];
    if (!list.length) return "";
    const places = new Set(list.map((s) => s.scope)).size > 1;
    const row = (s) => {
      const notes = [];
      if (s.invalid) notes.push('<span class="error">' + one.esc(s.invalid) + "</span>");
      else if (s.removed) notes.push("Removed by a deploy" + (s.was ? ": it ran " + one.esc(s.was) : "") + ".");
      else {
        notes.push(one.esc(cap(s.when)) + (places ? " · " + one.esc(s.scope_name) : ""));
        if (s.changed) notes.push("A deploy changed it " + ago(s.changed.time) + "; it ran " + one.esc(s.changed.was) + " before.");
        if (s.on && s.runs_as) notes.push("Runs as " + one.esc(s.runs_as) + (s.next ? ", next " + soon(s.next) : ""));
        else if (s.on) notes.push("Nobody can run it: whoever added it here can't any more. Switch it on to run it as you.");
        else if (s.switched) notes.push("Switched off by " + one.esc(s.switched.by) + " " + ago(s.switched.time));
        if (s.last && !s.last.ok) notes.push('<span class="error">Its last run failed ' + ago(s.last.time) + ": " + one.esc(s.last.error || "") + "</span>");
      }
      const live = !s.invalid && !s.removed;
      const toggle = live ? \`<input type="checkbox" class="switch"\${s.on && s.runs_as ? " checked" : ""}\${s.may_switch ? "" : " disabled"} onchange='switchIt(this, \${JSON.stringify({ schedule: s.name, scope: s.scope })})'>\` : "";
      return \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(s.description || s.name)}</span></div>
        \${notes.map((n) => '<div class="sub small">' + n + "</div>").join("")}</div>\${toggle}</div>\`;
    };
    return '<h2 class="section">Schedules</h2><div class="list">' + list.map(row).join("") + "</div>";
  },
  function errors({ health }) {
    const list = health?.insights?.errors || [];
    if (!list.length) return "";
    const total = list.reduce((n, e) => n + e.count, 0);
    const row = (e) => \`<div class="row plain"><div class="main"><div class="title"><span class="error">\${one.esc(e.error || "Failed")}</span><span class="when">\${e.count}×</span></div>
        <div class="sub small">\${one.esc(e.op)} · last \${ago(e.last)}\${e.user ? " by " + one.esc(e.user) : ""}</div></div></div>\`;
    return '<h2 class="section">Errors · ' + total + '</h2><div class="list">' + list.slice(0, 20).map(row).join("") + "</div>";
  },
  function activity({ health }) {
    const runs = health?.activity || [];
    if (!runs.length) return "";
    const failed = runs.filter((r) => !r.ok).length;
    const row = (r) => \`<div class="row plain"><div class="main"><div class="title"><span>\${r.ok ? "✓" : '<span class="error">✕</span>'} \${one.esc(r.schedule)}</span><span class="when">\${one.ago(r.time)}</span></div>
        \${r.ok ? "" : '<div class="sub small error">' + one.esc(r.error || "Failed") + "</div>"}
        <div class="sub small">\${r.ok ? "Ran" : "Failed"} on \${one.esc(r.server || "a server")} as \${one.esc(r.user)} · \${r.ms < 1000 ? r.ms + " ms" : (r.ms / 1000).toFixed(1) + " s"}</div></div></div>\`;
    return '<h2 class="section">Activity' + (failed ? " · " + failed + " failed" : "") + '</h2><div class="list">' + runs.slice(0, 8).map(row).join("") + "</div>";
  },
  function usage({ health }) {
    const list = health?.insights?.usage || [];
    if (!list.length) return "";
    const what = { cli: "CLI", mcp: "Agent", phone: "iPhone", mac: "Mac", browser: "Browser", schedule: "Schedule", app: "Another app", node: "The server", api: "A program on the server" };
    const row = (u) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(u.user)}</span><span class="when">\${one.ago(u.last)}</span></div>
        <div class="sub small">\${one.esc(u.agent || what[u.client] || u.client)} · \${u.days} day\${u.days === 1 ? "" : "s"}</div></div></div>\`;
    return '<h2 class="section">Used by</h2><div class="list">' + list.slice(0, 20).map(row).join("") + "</div>";
  },
  function cost({ health }) {
    const list = health?.insights?.cost || [];
    if (!list.length) return "";
    const usd = (n) => "$" + (n < 1 ? n.toFixed(4) : n.toFixed(2));
    const total = list.reduce((n, c) => n + c.usd, 0);
    const row = (c) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(c.model)}</span><span class="when">\${usd(c.usd)}</span></div>
        <div class="sub small">\${c.calls} call\${c.calls === 1 ? "" : "s"} · \${c.input.toLocaleString()} tokens in, \${c.output.toLocaleString()} out</div></div></div>\`;
    return '<h2 class="section">AI cost · ' + usd(total) + '</h2><div class="list">' + list.map(row).join("") + "</div>";
  },
  function uses({ a }) {
    const all = [...a.uses.apps, ...a.uses.drivers];
    return all.length ? '<h2 class="section">Uses</h2><div class="list">' + all.map((u) => '<div class="row plain"><div class="main">' + one.esc(u) + "</div></div>").join("") + "</div>" : "";
  },
  function forkedFrom({ a }) {
    return a.from ? '<h2 class="section">Forked from</h2><div class="list">' + link(a.from) + "</div>" : "";
  },
  function forks({ a }) {
    return a.forks.length ? '<h2 class="section">Forks</h2><div class="list">' + a.forks.map(link).join("") + "</div>" : "";
  },
  function remove({ a }) {
    if (!a.mine || a.deleted || !a.added) return "";
    return \`<div class="list" style="margin-top:24px"><a class="row plain" onclick="retire(null, 'delete')"><div class="main"><div class="title"><span class="error">Delete App</span></div></div></a></div>
      <div class="muted small" style="margin:6px 16px">It leaves every page and stops running. You can restore it from Recently Deleted.</div>\`;
  },
  function code({ code }) {
    return code ? '<h2 class="section">Code</h2>' + folder(code.files, "") : "";
  },
];

function link(n) {
  return \`<a class="row plain" onclick='one.open("app", \${JSON.stringify({ app: n })})'><div class="main"><div class="title"><span>\${one.esc(n)}</span></div></div><div class="chev">›</div></a>\`;
}

async function show() {
  const [{ apps, places, admin }, code, health] = await Promise.all([one.call("list"), one.call("code", { app: name }).catch(() => null), one.call("health", { app: name }).catch(() => null)]);
  const a = apps.find((x) => x.name === name);
  if (!a) return (document.getElementById("app").innerHTML = '<div class="empty">' + one.esc(name) + " was erased.</div>");
  one.title(a.title);
  document.getElementById("app").innerHTML = sections.map((s) => s({ a, places, admin, code, health })).join("");
}
const cap = (t) => t.charAt(0).toUpperCase() + t.slice(1);
/// "just now", "5 min ago".
const ago = (t) => (one.ago(t) === "now" ? "just now" : one.ago(t) + " ago");
async function retire(button, op) {
  const go = async () => {
    await one.call(op, { app: name, confirm: true });
    if (op === "erase") history.back();
    show();
  };
  if (op === "restore") {
    button.disabled = true;
    return go().catch((err) => { document.getElementById("error").textContent = err.message; show(); });
  }
  const erase = op === "erase";
  document.getElementById("ask").scrollIntoView({ behavior: "smooth", block: "center" });
  await one.confirm("#ask", {
    title: erase ? "Erase " + name + " for good?" : "Delete " + name + "?",
    detail: erase ? "It's gone from every list, and its name can't be used again." : "It leaves every page and stops running. You can restore it from Recently Deleted.",
    action: erase ? "Erase" : "Delete", danger: true, run: go,
  });
}
/// When a time in the next while is: "in 7m", "in 3h", "tomorrow".
function soon(ms) {
  const m = Math.max(0, Math.round((ms - Date.now()) / 60000));
  return m < 1 ? "now" : m < 60 ? "in " + m + "m" : m < 1440 ? "in " + Math.round(m / 60) + "h" : "in " + Math.round(m / 1440) + "d";
}
async function switchIt(box, { schedule, scope }) {
  box.disabled = true;
  document.getElementById("error").textContent = "";
  try {
    await one.call("switch", { app: name, schedule, scope, on: box.checked });
  } catch (err) {
    document.getElementById("error").textContent = err.message;
  }
  show();
}
async function toggle(button, scope, on) {
  button.disabled = true;
  document.getElementById("error").textContent = "";
  try {
    await one.call(on ? "remove" : "add", { app: name, scope });
  } catch (err) {
    document.getElementById("error").textContent = err.message;
  }
  show();
}
one.on("changed", show);
show();
</script>`,
    },
    driver: {
      title: "Driver",
      html: `
<style>
  .head { display: flex; gap: 14px; align-items: center; margin: 12px 4px 4px; }
  .head h1 { font-size: 22px; margin: 0; }
  .about { margin: 12px 4px 0; }
  .pill { font: inherit; font-size: 15px; font-weight: 600; padding: 5px 14px; border-radius: 15px; border: 0; background: var(--accent); color: white; }
  .pill.danger { background: var(--danger); }
  .pill:disabled { opacity: 0.4; }
  .dot { width: 9px; height: 9px; border-radius: 50%; background: #34c759; flex: none; margin-right: 12px; }
  .dot.off { background: var(--muted); }
  #error { margin: 8px 4px; }
</style>
<div id="driver"></div>
<div id="ask"></div>
<div id="error" class="error small"></div>
<script>
${install}
const name = one.params.driver;
one.title(name);
async function show() {
  const { drivers, servers } = await one.call("drivers", { name });
  const d = drivers[0];
  if (!d) return (document.getElementById("driver").innerHTML = '<div class="empty">None of your servers runs ' + one.esc(name) + ", and no app uses it.</div>");
  one.title(d.title);
  const by = (s) => s.built_in ? "Built into every server" : s.installed_by ? "Installed by " + s.installed_by : "";
  const server = (s) => \`<div class="row plain"><span class="dot\${s.online ? "" : " off"}"></span><div class="main">
      <div class="title"><span>\${one.esc(s.server)}</span></div>
      <div class="sub small">\${one.esc([s.online ? "Running" : "Offline", by(s), s.users.length ? "for " + s.users.join(", ") : ""].filter(Boolean).join(" · "))}</div></div>
      \${s.may_remove ? \`<button class="pill danger" onclick='removeIt(\${JSON.stringify(s.server)})'>Remove</button>\` : ""}</div>\`;
  const removed = (r) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(r.server)}</span></div>
      <div class="sub small">Removed\${r.by ? " by " + one.esc(r.by) : ""} \${one.esc(one.ago(r.time))} ago</div></div>
      \${r.may_allow ? \`<button class="pill" onclick='allow(this, \${JSON.stringify(r.server)})'>Allow</button>\` : ""}</div>\`;
  const app = (a) => \`<a class="row plain" onclick='one.open("app", \${JSON.stringify({ app: a.name })})'><div class="main"><div class="title"><span>\${one.esc(a.title)}</span></div></div><div class="chev">›</div></a>\`;
  document.getElementById("driver").innerHTML = \`<div class="head">\${one.appIcon(d, 60)}<div><h1>\${one.esc(d.title)}</h1><div class="muted small">\${one.esc(d.name)} · driver</div></div></div>
    \${d.description ? '<div class="about">' + one.esc(d.description) + "</div>" : ""}
    <h2 class="section">Runs on</h2><div class="list">\${d.servers.length ? d.servers.map(server).join("") : '<div class="row plain"><div class="main sub">None of your servers runs it.</div></div>'}</div>
    \${d.removed.length ? '<h2 class="section">Removed</h2><div class="list">' + d.removed.map(removed).join("") + "</div>" : ""}
    \${d.apps.length ? '<h2 class="section">Used by</h2><div class="list">' + d.apps.map(app).join("") + "</div>" : ""}
    \${d.install ? '<h2 class="section">Install</h2><div class="list"><div class="row plain"><div class="main sub">' + one.esc(d.install) + "</div></div></div>" : install(servers, name)}\`;
}
async function removeIt(server) {
  document.getElementById("ask").scrollIntoView({ behavior: "smooth", block: "center" });
  await one.confirm("#ask", {
    title: "Remove " + name + " from " + server + "?",
    detail: "It stops serving there, and apps that use it go to another server that runs it, or say none does. What it keeps on the server stays, and you can allow it again.",
    action: "Remove", danger: true, run: () => one.call("remove_driver", { driver: name, server, confirm: true }).then(show),
  });
}
async function allow(button, server) {
  button.disabled = true;
  document.getElementById("error").textContent = "";
  try { await one.call("allow_driver", { driver: name, server }); } catch (err) { document.getElementById("error").textContent = err.message; }
  show();
}
one.on("changed", show);
show();
</script>`,
    },
    code: {
      title: "Code",
      html: `
<style>.code-icon { width: 20px; height: 20px; color: var(--muted); } .code-list { margin-top: 8px; }</style>
<div id="code"></div>
<script>
const name = one.params.app;
one.title(one.params.dir.replace(/\\/$/, "").split("/").pop());
${folder}
one.render("#code", async () => folder((await one.call("code", { app: name })).files, one.params.dir), { loading: "Loading code" });
</script>`,
    },
    file: {
      title: "Code",
      html: `
<div id="code" style="margin-top: 8px"></div>
<script>
one.title(one.params.path.split("/").pop());
one.render("#code", async () => {
  const f = await one.call("code", { app: one.params.app, path: one.params.path });
  setTimeout(() => one.code("#code", f.text, { path: f.path }));
  return "";
}, { loading: "Loading code" });
</script>`,
    },
  },
};
