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

/// Whether the caller acts for an app's owner: it's theirs, a group's they're an admin of, or admins' and they're one.
function mine(ctx, owner) {
  if (owner === `user:${ctx.me.user}`) return true;
  if (owner === "admins") return ctx.me.role === "admin";
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

export default {
  name: "apps",
  title: "App Store",
  icon: "square.grid.2x2",
  color: "#007aff",
  description: "Every app you can add, and its code",
  instructions: "The App Store lists every app in this install, where each is installed and its code. apps_add installs one for you, a group you're an admin of, or everyone (admins).",

  ops: {
    list: {
      reads: true,
      description: "Every app: the built-in ones (built_in), added to the install or not and up to date or not, and the ones made in the install; owner, where it's installed (that you can see), forks, and the places you may add it to.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const name = namer(ctx);
        const apps = await Promise.all((await ctx.catalog()).map(async (a) => {
          const added = a.deploys > 0;
          const d = added || !a.built_in ? a : await described(ctx, a.built_in).catch(() => ({}));
          return {
            name: a.name, title: d.title || a.title || a.name, icon: d.icon || a.icon || null, color: d.color || a.color || null, image: a.image || null, description: d.description || a.description || null,
            owner: a.owner, owner_name: name(a.owner),
            from: a.from || null, forks: a.forks || [], deploys: a.deploys, installed: a.installed || [],
            installed_names: (a.installed || []).map(name), uses: a.uses || { apps: [], drivers: [] },
            built_in: !!a.built_in, added, up_to_date: a.built_in && added ? a.bundle === a.built_in : null,
            mine: mine(ctx, a.owner), deleted: a.deleted || null,
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
<input id="search" class="search" type="search" placeholder="Find an app" autocomplete="off">
<div id="list"></div>
<script>
// The sections: the apps built into the release, and those made in this install (each brand words that its own way).
const BUILT_IN = "Built In";
const MADE_HERE = one.brand.made_here;

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

function draw() {
  const q = document.getElementById("search").value.trim().toLowerCase();
  const found = apps.filter((a) => !q || [a.title, a.name, a.description || ""].some((t) => t.toLowerCase().includes(q)));
  document.getElementById("list").innerHTML = found.length
    ? section(BUILT_IN, found.filter((a) => a.built_in && !a.deleted)) + section(MADE_HERE, found.filter((a) => !a.built_in && !a.deleted)) + section("Recently Deleted", found.filter((a) => a.deleted))
    : '<div class="empty">No apps match.<div class="small" style="margin-top:6px">Make your own: put an app.js in a folder and run <code>one deploy</code> on it.</div></div>';
}
async function show() {
  apps = (await one.call("list")).apps;
  draw();
}
document.getElementById("search").addEventListener("input", draw);
one.on("changed", show);
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
    const what = { cli: "CLI", mcp: "Agent", phone: "iPhone", mac: "Mac", browser: "Browser", schedule: "Schedule", app: "Another app", node: "The node", api: "Local API" };
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
