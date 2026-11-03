// The App Store: the apps built into the release, whether or not the install has added them, and the apps made in the
// install; who owns each, where it's installed, its forks and its code; and "Add to…" each place the person has a
// page: themselves, their groups and everyone, saying who can add it where they can't.
// An ordinary app: it installs with ctx.install, with the caller's own rights. Installing a built-in app the install
// hasn't added yet adds it (admins only).

/// The caller's pages, where apps install: [{ scope, id, name, may, why }]; `why` says who can, in two words, when they
/// can't.
function places(ctx) {
  const out = [{ scope: "user", id: `user:${ctx.me.user}`, name: "You", may: true }];
  for (const g of ctx.groups) {
    const may = g.role === "admin";
    out.push({ scope: g.id, id: g.id, name: g.name, may, why: may ? null : "Only admins" });
  }
  const admin = ctx.me.role === "admin";
  out.push({ scope: "global", id: "global", name: "Everyone", may: admin, why: admin ? null : "Only admins" });
  return out;
}

/// A place as given: its scope (user, group:<id>, global) or its name as places() gives it (You, a group's, Everyone).
function placeOf(ctx, given) {
  const want = String(given || "").trim();
  const p = places(ctx).find((p) => [p.scope, p.id].includes(want) || p.name.toLowerCase() === want.toLowerCase());
  if (!p) throw new Error(`no place ${want}: give user (you), global (everyone) or a group by its name (${places(ctx).filter((p) => p.id.startsWith("group:")).map((p) => p.name).join(", ") || "you're in no group"})`);
  return p.scope;
}

const placeInput = { type: "string", description: "user (you), global (everyone) or a group's name" };

/// Whether the caller acts for an owner (an app's, a device's): it's theirs, a group's they're an admin of, or admins'
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
/// `title`, `icon`, `color`, `tagline` and `description` after the line starting `export default` given as plain strings, as the built-in
/// apps give them before their ops).
async function described(ctx, bundle) {
  const file = untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const all = file ? new TextDecoder().decode(file.bytes) : "";
  const code = all.slice(Math.max(0, all.search(/^export default/m)));
  const field = (key) => code.match(new RegExp(`\\b${key}:\\s*"([^"]*)"`))?.[1] || null;
  const parts = code.match(/\buses:\s*\{[^}]*\bsidecars:\s*\[([^\]]*)\]/)?.[1].match(/"[^"]+"/g)?.map((n) => n.slice(1, -1)) || [];
  return { title: field("title"), icon: field("icon"), color: field("color"), tagline: field("tagline"), description: field("description"), needs: field("needs"), uses: { apps: [], sidecars: parts } };
}

/// Whether an app is a service: it has no main screen and no widget, so shells keep it in each page's Services folder
/// rather than giving it an icon. What it says about itself where it's installed for the caller; elsewhere, its code.
async function isService(ctx, a) {
  if (a.ops) return !(a.screens || []).some((x) => x.name === "main") && !(a.widgets || []).length;
  const bundle = a.bundle || a.built_in;
  const file = bundle && untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const code = file ? new TextDecoder().decode(file.bytes) : "";
  return !/\bmain:\s*\{/.test(code) && !/\bwidgets:\s*\{\s*[\w"]/.test(code);
}

/// Where a service runs, in a few words, and what each of the caller's devices offers for it: "Runs on linux, mac",
/// or what it needs (its `needs`) when none of their devices runs it; the devices they own it may be installed on (where
/// an owner removed it) or removed from; and whether they own none, so they ask an owner instead. Its device parts are the
/// sidecars it uses, by their names.
async function where(ctx, a) {
  const devices = await ctx.devices();
  const parts = a.uses?.sidecars || [];
  if (!parts.length) return null;
  const owns = (s) => (s.owners || []).some((o) => mine(ctx, o));
  const runs = devices.filter((s) => (s.sidecars || []).some((d) => parts.includes(d)));
  const removed = devices.filter((s) => !runs.includes(s) && (s.removed_sidecars || []).some((d) => parts.includes(d.name)));
  const line = runs.length ? "Runs on " + runs.map((s) => s.name).join(", ") : a.needs || "Runs on none of your devices";
  return {
    line, runs: runs.map((s) => s.name), needs: a.needs || null,
    install: removed.filter(owns).map((s) => s.name), remove: runs.filter(owns).map((s) => s.name),
    ask: !runs.some(owns) && !removed.some(owns),
  };
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

/// The Add to… sheet, from an app's row or its page: a switch for each of the person's pages, dimmed with two words of
/// reason where they can't use it. Runs in the page: the screens include it.
function addTo(name, { apps, places, admin }) {
  let error = "";
  const sheet = one.sheet({ title: apps.find((a) => a.name === name)?.title || name });
  const draw = () => {
    const a = apps.find((x) => x.name === name);
    if (!a) return sheet.close();
    const first = !a.added && !admin ? "Only admins" : null;
    const row = (p) => {
      const why = first || p.why, on = a.installed.includes(p.id);
      return '<div class="row plain' + (why ? " dim" : "") + '"><div class="main"><div class="title"><span>' + one.esc(p.name) + "</span></div>"
        + (why ? '<div class="sub">' + one.esc(why) + "</div>" : "") + "</div>"
        + '<input type="checkbox" class="switch" aria-label="' + one.esc(p.name) + '" data-scope="' + one.esc(p.scope) + '"' + (on ? " checked" : "") + (why ? " disabled" : "") + "></div>";
    };
    sheet.body.innerHTML = '<div style="display:flex;justify-content:center;margin:2px 0 14px">' + one.appIcon(a, 64) + "</div>"
      + '<div class="list">' + places.map(row).join("") + "</div>"
      + (error ? '<div class="footer error">' + one.esc(error) + "</div>" : "");
    for (const box of sheet.body.querySelectorAll("[data-scope]")) box.onchange = () => flip(box);
  };
  // The switch stays where it was put, and goes back only if the change fails.
  const flip = async (box) => {
    one.tap();
    error = "";
    const a = apps.find((x) => x.name === name), id = places.find((p) => p.scope === box.dataset.scope)?.id, was = a.installed;
    a.installed = box.checked ? [...was, id] : was.filter((x) => x !== id);
    try {
      await one.call(box.checked ? "add" : "remove", { app: name, scope: box.dataset.scope, confirm: true });
    } catch (err) {
      a.installed = was;
      error = one.plain(err).title;
      return draw();
    }
    one.call("list", { full: true }).then((got) => (({ apps, places, admin } = got), draw()), () => {});
  };
  draw();
}

/// Whether an app is on any of the person's pages, so its button says Manage instead of Add to….
function onAPage(a, places) {
  return a.installed.some((id) => places.some((p) => p.id === id));
}

export default {
  name: "apps",
  title: "App Store",
  icon: "squares-four",
  color: "#007aff",
  description: "Every app you can add, and its code",
  instructions: "The App Store lists every app in this install, where each is installed and its code. apps_add installs one for you, a group you're an admin of, or everyone (admins).",

  // The apps whose schedule, where you may switch it, failed its last run.
  async badge(ctx) {
    return new Set((await ctx.failing()).map((s) => s.app)).size;
  },

  ops: {
    list: {
      reads: true,
      description: "Every app, a line each: its name, where it's added (that you can see), whether it's a service (no screen of its own, like Gmail: it's in each page's Services), and what it's for; then the places you may add apps to. With service, only services.",
      // A screen asks with full for every app's details and picture, which are large.
      input: { type: "object", properties: { service: { type: "boolean", description: "only services" } } },
      async run(ctx, { service: only, full } = {}) {
        const images = full;
        const name = namer(ctx);
        const all = (await ctx.catalog()).filter((a) => !a.deleted || !only);
        const services = await Promise.all(all.map((a) => isService(ctx, a).catch(() => false)));
        const apps = await Promise.all(all.filter((a, i) => services[i] || !only).map(async (a) => {
          const added = a.deploys > 0;
          const d = added || !a.built_in ? a : await described(ctx, a.built_in).catch(() => ({}));
          return {
            name: a.name, title: d.title || a.title || a.name, icon: d.icon || a.icon || null, color: d.color || a.color || null, ...(images ? { image: a.image || null, image_kind: a.image_kind || null } : {}),
            tagline: d.tagline || a.tagline || null, description: d.description || a.description || null,
            owner: a.owner, owner_name: name(a.owner),
            from: a.from || null, forks: a.forks || [], deploys: a.deploys, installed: a.installed || [],
            installed_names: (a.installed || []).map(name), uses: a.uses || { apps: [], sidecars: [] },
            built_in: !!a.built_in, added, up_to_date: a.built_in && added ? a.bundle === a.built_in : null,
            source: a.source || null, mine: mine(ctx, a.owner), deleted: a.deleted || null, service: services[all.indexOf(a)],
          };
        }));
        apps.sort((a, b) => a.title.localeCompare(b.title));
        if (full) return { apps, places: places(ctx), admin: ctx.me.role === "admin" };
        const line = (a) => `${a.title} (${a.name}): ${a.deleted ? "deleted" : a.installed_names.length ? `added for ${a.installed_names.join(", ")}` : "not added"}`
          + `${a.service ? ", a service" : ""}${a.up_to_date === false ? ", an update waiting" : ""}. ${a.tagline || a.description || ""}`.trimEnd();
        return [...apps.map(line), "", `You may add apps for: ${places(ctx).filter((p) => p.may).map((p) => p.scope === "user" ? "user (you)" : p.scope === "global" ? "global (everyone)" : p.name).join(", ")}.`].join("\n");
      },
    },
    add: {
      description: "Install an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only).",
      input: { type: "object", required: ["app", "scope"], properties: { app: { type: "string" }, scope: placeInput } },
      async run(ctx, { app, scope }) {
        await find(ctx, app);
        ctx.install(app, placeOf(ctx, scope));
        return `Added ${app}.`;
      },
    },
    remove: {
      confirm: true,
      description: "Uninstall an app from a place: user (you), a group (by its name) or global (everyone).",
      input: { type: "object", required: ["app", "scope"], properties: { app: { type: "string" }, scope: placeInput } },
      async run(ctx, { app, scope }) {
        ctx.install(app, placeOf(ctx, scope), false);
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
    source: {
      description: "Set where an app you own keeps its code, shown on its page as Source: a web address: GitHub, any git host or page, or a repo's address in Repos (http://localhost:<port>/repos/<repo>.git). An empty link clears it. Built-in apps start with their folder in the open-source repo; forks start with none.",
      input: { type: "object", required: ["app", "link"], properties: { app: { type: "string" }, link: { type: "string", description: "https://…, or a repo's address in Repos; empty for none" } } },
      async run(ctx, { app, link }) {
        const a = await find(ctx, app);
        if (!mine(ctx, a.owner)) throw new Error(`only ${app}'s owners set its source link`);
        if (!a.deploys && a.built_in) throw new Error(`${app} isn't added to this install yet: add it first`);
        ctx.source(app, (link || "").trim());
        return link?.trim() ? `${app}'s source is ${link.trim()}.` : `${app} has no source link now.`;
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
      description: "Switch one of an app's schedules on or off in a place where it's installed (user, a group by its name, or global); switched on, it runs as you.",
      input: { type: "object", required: ["app", "schedule", "scope", "on"], properties: { app: { type: "string" }, schedule: { type: "string" }, scope: placeInput, on: { type: "boolean" } } },
      async run(ctx, { app, schedule, scope, on }) {
        ctx.schedule(app, schedule, placeOf(ctx, scope), on);
        return `${schedule} is ${on ? "on" : "off"}.`;
      },
    },
    devices: {
      reads: true,
      description: "Where a service runs (\"Runs on linux, mac\"), or what it needs when none of your devices runs it; the devices you own it may be installed on or removed from; and ask: true when you own none of those, so you ask a device's owner to install it.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" } } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return where(ctx, a.ops || a.uses ? a : { ...a, ...(await described(ctx, a.built_in).catch(() => ({}))) });
      },
    },
    install_on: {
      description: "Install a service on a device you own, where an owner removed it: it runs there again.",
      input: { type: "object", required: ["app", "device"], properties: { app: { type: "string" }, device: { type: "string", description: "the device, by name" } } },
      async run(ctx, { app, device }) {
        const a = await find(ctx, app);
        for (const part of a.uses?.sidecars || []) ctx.sidecar(part, device, true);
        return `${a.title || app} is installed on ${device}.`;
      },
    },
    remove_from: {
      confirm: true,
      description: "Remove a service from a device you own: it stops running there, and goes on running on your other devices that have it. What it keeps on that device stays, and install_on puts it back.",
      input: { type: "object", required: ["app", "device"], properties: { app: { type: "string" }, device: { type: "string", description: "the device, by name" } } },
      async run(ctx, { app, device }) {
        const a = await find(ctx, app);
        const there = (await ctx.devices()).find((s) => s.name === device);
        for (const part of a.uses?.sidecars || []) if ((there?.sidecars || []).includes(part)) ctx.sidecar(part, device, false);
        return `${a.title || app} is removed from ${device}.`;
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
  #tabs { margin: 10px 0 4px; }
  .store .row { padding-top: 10px; padding-bottom: 10px; }
  .store .row + .row::before { left: 76px; }
  .store .row .title { font-weight: 500; }
</style>
<div id="top"></div>
<div id="search"></div>
<div class="segmented" id="tabs"><button data-tab="apps">Apps</button><button data-tab="services">Services</button></div>
<div id="list" class="store"></div>
<script>
${addTo}
${onAPage}
// A page's Add a Service opens it with { service: true, to: the page's scope }: only services, each added to that page
// with Add, which then goes back to it. Anywhere else, services get their own tab, as they'd bury the rest.
const to = one.params.service ? one.params.to || null : null;
document.getElementById("top").innerHTML = one.largeTitle(to ? "Add a Service" : "App Store");
if (to) document.getElementById("tabs").style.display = "none";
let tab = one.params.service ? "services" : "apps";
let query = "";
let data = null;
function pick(t) {
  tab = t;
  for (const b of document.querySelectorAll("#tabs button")) b.classList.toggle("on", b.dataset.tab === tab);
  draw();
}
document.getElementById("tabs").addEventListener("click", (e) => e.target.dataset.tab && pick(e.target.dataset.tab));
one.search("#search", { placeholder: "Search", onSearch: (q) => ((query = q.trim().toLowerCase()), draw()) });
// The sections: the apps built into the release, and those made in this install (each install words that its own way,
// in its settings).
const MADE_HERE = one.config?.made_here || "Made here";

// Added to the page it's for, or Add, where they may (Only admins may add one no one has yet).
function here(a) {
  const place = data.places.find((p) => p.scope === to);
  if (place && a.installed.includes(place.id)) return { button: '<button class="button capsule" disabled>Added</button>' };
  const why = !place ? "Not yours to change" : !a.added && !data.admin ? "Only admins" : place.why;
  return { why, button: '<button class="button capsule" data-add-here="' + one.esc(a.name) + '"' + (why ? " disabled" : "") + ">Add</button>" };
}
const row = (a) => {
  const h = to && !a.deleted ? here(a) : null;
  return \`<div class="row" data-app="\${one.esc(a.name)}">
    \${one.appIcon(a, 48)}
    <div class="main"><div class="title"><span>\${one.esc(a.title)}</span></div><div class="sub">\${one.esc(h?.why || a.tagline || a.description || "")}</div></div>
    \${h ? h.button : a.deleted ? (a.mine ? '<button class="button capsule" data-restore="' + one.esc(a.name) + '">Restore</button>' : "")
      : '<button class="button capsule" data-add="' + one.esc(a.name) + '">' + (onAPage(a, data.places) ? "Manage" : "Add to…") + "</button>"}
  </div>\`;
};
const section = (title, list) => list.length ? '<h2 class="section">' + one.esc(title) + '</h2><div class="list">' + list.map(row).join("") + "</div>" : "";

function html() {
  const found = data.apps.filter((a) => !!a.service === (tab === "services") && (!query || [a.title, a.name, a.tagline || "", a.description || ""].some((t) => t.toLowerCase().includes(query))));
  if (!found.length) return one.empty(query ? one.esc("Nothing matches “" + query + "”.") : "No results.");
  return section("Built In", found.filter((a) => a.built_in && !a.deleted)) + section(MADE_HERE, found.filter((a) => !a.built_in && !a.deleted)) + section("Recently Deleted", found.filter((a) => a.deleted));
}
function draw() {
  if (data) document.getElementById("list").innerHTML = html();
}
document.getElementById("list").addEventListener("click", (e) => {
  const add = e.target.closest("[data-add]");
  if (add) return addTo(add.dataset.add, data);
  const adding = e.target.closest("[data-add-here]");
  if (adding) {
    one.tap();
    adding.disabled = true;
    return one.call("add", { app: adding.dataset.addHere, scope: to })
      .then(() => one.close(), (err) => ((adding.disabled = false), one.alert({ title: "Couldn’t Add It", text: one.plain(err).title })));
  }
  const restore = e.target.closest("[data-restore]");
  // Restored at once, into Built In or Made Here; put back if it fails.
  if (restore) {
    const a = data.apps.find((x) => x.name === restore.dataset.restore);
    if (a) (a.deleted = null), draw();
    return one.call("restore", { app: restore.dataset.restore }).catch(show);
  }
  const r = e.target.closest("[data-app]");
  if (r) one.open("app", { app: r.dataset.app });
});
const show = () => one.render("#list", async () => ((data = await one.call("list", { full: true })), html()), { loading: "Loading apps" });
one.on("changed", show);
pick(tab);
show();
</script>`,
    },
    app: {
      title: "App",
      html: `
<style>
  .hero { display: flex; gap: 16px; margin: 14px 4px 20px; }
  .hero .text { flex: 1; min-width: 0; display: flex; flex-direction: column; }
  .hero h1 { font-size: 22px; font-weight: 700; margin: 4px 0 2px; line-height: 1.2; }
  .hero .tag { color: var(--muted); font-size: 15px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .hero .act { margin-top: auto; padding-top: 10px; }
  .about { margin: 0 4px; line-height: 1.45; }
  .needs { margin: 14px 4px 0; color: var(--muted); }
  .made { text-align: center; color: var(--muted); font-size: 13px; margin: 32px 16px 0; }
  .works .row + .row::before { left: 58px; }
  #error { margin: 8px 4px; }
  .danger-row { color: var(--danger); justify-content: center; }
  .action-row { color: var(--tint); justify-content: center; }
  .link-field { flex: 1; min-width: 0; font: inherit; border: 0; outline: none; background: none; color: var(--text); padding: 0; }
</style>
<div id="app"></div>
<div id="ask"></div>
<div id="error" class="error small"></div>
<script>
${addTo}
${onAPage}
const name = one.params.app;
one.title("");
let data = null;

/// The app's page, section by section: each takes what show() loaded and gives HTML, or "" to leave itself out.
const sections = [
  // Under the name, a line only where there's one apart from the description, which follows in full: when a deleted
  // app was deleted, or a tagline the app declares.
  function hero({ a, places }) {
    const tag = a.deleted ? "Deleted " + ago(a.deleted.time) : a.tagline || "";
    const act = a.deleted
      ? (a.mine ? '<button class="button capsule" onclick="retire(this, \\'restore\\')">Restore</button>' : "")
      : '<button class="button capsule" data-add>' + (onAPage(a, places) ? "Manage" : "Add to…") + "</button>";
    return \`<div class="hero">\${one.appIcon(a, 100)}<div class="text"><h1>\${one.esc(a.title)}</h1>
      \${tag ? '<div class="tag">' + one.esc(tag) + "</div>" : ""}<div class="act">\${act}</div></div></div>
      \${a.description ? '<div class="about">' + one.esc(a.description) + "</div>" : ""}\`;
  },
  // A service's devices, under Runs On (or what it needs, where it runs on none): those it runs on, with Remove for
  // their owners, then those of theirs an owner removed it from, to install it again.
  function needs({ where }) {
    if (!where) return "";
    const device = (name, op) => '<div class="row plain"><div class="main"><div class="title"><span>' + one.esc(name) + "</span></div></div>"
      + (op ? '<button class="button' + (op === "install_on" ? " primary" : "") + '" data-op="' + op + '" data-device="' + one.esc(name) + '">' + (op === "install_on" ? "Install" : "Remove") + "</button>" : "") + "</div>";
    const rows = [...where.runs.map((s) => device(s, where.remove.includes(s) && "remove_from")), ...where.install.map((s) => device(s, "install_on"))];
    if (!rows.length) return '<div class="needs">' + one.esc(where.line) + "</div>";
    return '<h2 class="section">' + one.esc(where.runs.length ? "Runs On" : where.needs || "Runs On") + '</h2><div class="list">' + rows.join("") + "</div>";
  },
  function schedules({ health }) {
    // One a deploy removed no longer runs: it isn't the app's any more.
    const list = (health?.schedules || []).filter((s) => !s.removed);
    if (!list.length) return "";
    const places = new Set(list.map((s) => s.scope)).size > 1;
    const row = (s) => {
      let line;
      if (s.invalid) line = '<span class="error">' + one.esc(s.invalid) + "</span>";
      else if (s.last && !s.last.ok) line = '<span class="error">Failed ' + ago(s.last.time) + ": " + one.esc(one.plain(s.last.error).title) + "</span>";
      else if (s.on && s.runs_as) line = one.esc(cap(s.when)) + (places ? " · " + one.esc(s.scope_name) : "") + (s.next ? " · next " + soon(s.next) : "");
      else if (s.on) line = "Off. Switch it on to run it as you.";
      else line = one.esc(cap(s.when)) + (s.switched ? " · off since " + ago(s.switched.time) : "");
      const toggle = !s.invalid ? \`<input type="checkbox" class="switch"\${s.on && s.runs_as ? " checked" : ""}\${s.may_switch ? "" : " disabled"} onchange='switchIt(this, \${JSON.stringify({ schedule: s.name, scope: s.scope })})'>\` : "";
      return \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(s.description || s.name)}</span></div><div class="sub">\${line}</div></div>\${toggle}</div>\`;
    };
    return '<h2 class="section">Schedules</h2><div class="list">' + list.map(row).join("") + "</div>";
  },
  function works({ a, apps }) {
    const rows = a.uses.apps.map((n) => link(apps.find((x) => x.name === n) || { name: n, title: n }));
    return rows.length ? '<h2 class="section">Works With</h2><div class="list works">' + rows.join("") + "</div>" : "";
  },
  function forks({ a, apps }) {
    const find = (n) => apps.find((x) => x.name === n) || { name: n, title: n };
    return (a.from ? '<h2 class="section">Forked From</h2><div class="list works">' + link(find(a.from)) + "</div>" : "")
      + (a.forks.length ? '<h2 class="section">Forks</h2><div class="list works">' + a.forks.map((n) => link(find(n))).join("") + "</div>" : "");
  },
  // Where its code is kept: Source opens it, a repo in Repos or a page in the browser; its owners add or change it.
  function source({ a }) {
    const rows = [];
    if (a.source) rows.push(sourceRow(a.source));
    if (a.mine && a.added && !a.deleted) rows.push('<a class="row plain action-row" onclick="editSource()">' + (a.source ? "Edit Source Link" : "Add Source Link") + "</a>");
    return rows.length ? '<div class="list" style="margin-top:28px">' + rows.join("") + "</div>" : "";
  },
  function owner({ a, health }) {
    const busy = health && (health.activity?.length || health.insights?.errors?.length || health.insights?.usage?.length || health.insights?.cost?.length);
    const rows = [];
    if (busy) rows.push(\`<a class="row plain" onclick='one.open("activity", \${JSON.stringify({ app: a.name })})'><div class="main"><div class="title"><span>Activity</span></div></div><div class="chev">›</div></a>\`);
    if (a.mine && a.added && !a.deleted) rows.push('<a class="row plain danger-row" onclick="retire(null, \\'delete\\')">Delete App</a>');
    if (a.mine && a.deleted) rows.push('<a class="row plain danger-row" onclick="retire(null, \\'erase\\')">Erase App</a>');
    return rows.length ? '<div class="list" style="margin-top:28px">' + rows.join("") + "</div>" : "";
  },
  function made({ a, code }) {
    return '<div class="made">Made by ' + one.esc(a.owner_name) + (code ? \` · <a onclick='one.open("code", \${JSON.stringify({ app: a.name, dir: "" })})'>View code</a>\` : "") + "</div>";
  },
];

/// The Source row: a repo's address in Repos opens it there, anything else in the browser.
function sourceRow(link) {
  const repo = /^http:[/][/](localhost|127[.]0[.]0[.]1):[0-9]+[/]repos[/]([a-z0-9][a-z0-9._-]*?)([.]git)?[/]?$/.exec(link)?.[2];
  const open = repo ? \` onclick='one.open("repo", \${JSON.stringify({ repo })}, "repos")'\` : ' href="' + one.esc(link) + '" target="_blank" rel="noopener"';
  return '<a class="row plain"' + open + '><div class="main"><div class="title"><span>Source</span></div></div><div class="chev">›</div></a>';
}

/// The sheet that sets the source link, or with Remove Link (or an empty field) clears it.
function editSource() {
  const a = data.apps.find((x) => x.name === name);
  const sheet = one.sheet({ title: "Source Link", done: "Save" });
  sheet.body.innerHTML = '<div class="list"><div class="row plain"><input id="link" class="link-field" type="url" placeholder="https://github.com/…" autocapitalize="none" autocorrect="off" spellcheck="false"></div></div>'
    + '<div id="link-error" class="footer error"></div>'
    + (a.source ? '<div class="list" style="margin-top:20px"><a class="row plain danger-row" id="unlink">Remove Link</a></div>' : "");
  const field = sheet.body.querySelector("#link");
  field.value = a.source || "";
  setTimeout(() => field.focus(), 300);
  const save = async (link) => {
    try {
      await one.call("source", { app: name, link });
      sheet.close();
      show();
    } catch (err) {
      sheet.body.querySelector("#link-error").textContent = one.plain(err).title;
    }
  };
  sheet.body.closest(".sheet").querySelector("[data-done]").onclick = () => save(field.value.trim());
  field.addEventListener("keydown", (e) => e.key === "Enter" && save(field.value.trim()));
  sheet.body.querySelector("#unlink")?.addEventListener("click", () => save(""));
}

/// A row that opens another app's page: its icon and name.
function link(x) {
  return \`<a class="row plain" onclick='one.open("app", \${JSON.stringify({ app: x.name })})'>\${one.appIcon(x, 30)}<div class="main"><div class="title"><span>\${one.esc(x.title)}</span></div></div><div class="chev">›</div></a>\`;
}

// The page draws as soon as the app's own entry comes, and each section that waits on another call fills in as that
// one answers; until then it shows what it showed before.
const got = {};
let watched = null;
const titled = new IntersectionObserver(([e]) => one.title(e.isIntersecting ? "" : data?.apps.find((x) => x.name === name)?.title || ""));
function paint() {
  if (!got.list) return;
  const el = document.getElementById("app");
  const a = got.list.apps.find((x) => x.name === name);
  const html = a ? sections.map((s) => s({ a, apps: got.list.apps, places: got.list.places, admin: got.list.admin, code: got.code, health: got.health, where: got.where })).join("")
    : one.notFound({ title: "This app isn't here.", text: "It may have been erased, or the link is old.", home: "Back to the App Store" });
  if (html === el.__html) return;
  el.innerHTML = el.__html = html;
  if (!a) return;
  const h1 = document.querySelector(".hero h1");
  if (watched) titled.unobserve(watched);
  titled.observe((watched = h1));
  document.querySelector("[data-add]")?.addEventListener("click", () => addTo(name, data));
  for (const b of document.querySelectorAll("[data-op]")) b.onclick = () => place(b, b.dataset.op, b.dataset.device);
}
async function show() {
  const later = { code: one.call("code", { app: name }), health: one.call("health", { app: name }), where: one.call("devices", { app: name }) };
  for (const [k, p] of Object.entries(later)) p.then((v) => ((got[k] = v), paint()), () => {});
  data = got.list = await one.call("list", { full: true });
  paint();
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
    return go().catch((err) => { document.getElementById("error").textContent = one.plain(err).title; show(); });
  }
  const erase = op === "erase", title = data.apps.find((x) => x.name === name)?.title || name;
  document.getElementById("ask").scrollIntoView({ behavior: "smooth", block: "center" });
  await one.confirm("#ask", {
    title: erase ? "Erase " + title + " for good?" : "Delete " + title + "?",
    detail: erase ? "Its name can't be used again." : "You can restore it from Recently Deleted.",
    action: erase ? "Erase" : "Delete", danger: true, run: go,
  });
}
/// When a time in the next while is: "in 7m", "in 3h", "tomorrow".
function soon(ms) {
  const m = Math.max(0, Math.round((ms - Date.now()) / 60000));
  return m < 1 ? "now" : m < 60 ? "in " + m + "m" : m < 1440 ? "in " + Math.round(m / 60) + "h" : "in " + Math.round(m / 1440) + "d";
}
// The switch stays where it was put while the call goes, and goes back only if it fails.
async function switchIt(box, { schedule, scope }) {
  if (box.__busy) return void (box.checked = !box.checked);
  box.__busy = true;
  document.getElementById("error").textContent = "";
  try {
    await one.call("switch", { app: name, schedule, scope, on: box.checked });
  } catch (err) {
    box.checked = !box.checked;
    document.getElementById("error").textContent = one.plain(err).title;
  }
  box.__busy = false;
  show();
}
/// Installs a service on a device, or removes it from one, then shows where it runs now.
async function place(button, op, device) {
  button.disabled = true;
  button.insertAdjacentHTML("afterbegin", '<span class="spinner small" aria-hidden="true"></span>');
  document.getElementById("error").textContent = "";
  try {
    await one.call(op, { app: name, device, confirm: true });
  } catch (err) {
    document.getElementById("error").textContent = one.plain(err).title;
  }
  show();
}
one.on("changed", show);
one.render("#app", async () => void (await show()), { skeleton: 3 });
</script>`,
    },
    activity: {
      title: "Activity",
      html: `
<div id="activity"></div>
<script>
/// How an app is doing, for its owners: its errors, its schedules' recent runs, who used it, and its AI cost.
const sections = [
  function errors({ insights }) {
    const list = insights?.errors || [];
    const row = (e) => \`<div class="row plain"><div class="main"><div class="title"><span class="error">\${one.esc(e.error || "Failed")}</span><span class="when">\${e.count}×</span></div>
        <div class="sub">\${one.esc(e.op)} · \${one.ago(e.last)}\${e.user ? " · " + one.esc(e.user) : ""}</div></div></div>\`;
    return list.length ? '<h2 class="section">Errors</h2><div class="list">' + list.slice(0, 20).map(row).join("") + "</div>" : "";
  },
  function runs({ activity }) {
    const row = (r) => \`<div class="row plain"><div class="main"><div class="title"><span>\${r.ok ? "" : '<span class="error">✕</span> '}\${one.esc(r.schedule)}</span><span class="when">\${one.ago(r.time)}</span></div>
        <div class="sub\${r.ok ? "" : " error"}">\${r.ok ? one.esc(r.device || "") + " · " + (r.ms < 1000 ? r.ms + " ms" : (r.ms / 1000).toFixed(1) + " s") : one.esc(r.error || "Failed")}</div></div></div>\`;
    return activity?.length ? '<h2 class="section">Runs</h2><div class="list">' + activity.slice(0, 12).map(row).join("") + "</div>" : "";
  },
  function usedBy({ insights }) {
    const list = insights?.usage || [];
    const what = { cli: "CLI", mcp: "Agent", phone: "iPhone", mac: "Mac", browser: "Browser", schedule: "Schedule", app: "Another app", node: "The device", api: "A program on the device" };
    const row = (u) => \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(u.user)}</span><span class="when">\${one.ago(u.last)}</span></div>
        <div class="sub">\${one.esc(u.agent || what[u.client] || u.client)}</div></div></div>\`;
    return list.length ? '<h2 class="section">Used By</h2><div class="list">' + list.slice(0, 20).map(row).join("") + "</div>" : "";
  },
  function cost({ insights }) {
    const list = insights?.cost || [];
    const usd = (n) => "$" + (n < 1 ? n.toFixed(4) : n.toFixed(2));
    return list.length ? '<h2 class="section">AI Cost</h2>' + one.info(list.map((c) => [c.model, usd(c.usd)])) : "";
  },
];
one.render("#activity", async () => {
  if (!one.params.app) return one.notFound();
  const h = await one.call("health", { app: one.params.app });
  const html = sections.map((s) => s(h)).join("");
  return html || one.empty("Nothing yet.");
});
</script>`,
    },
    code: {
      title: "Code",
      html: `
<style>.code-icon { width: 20px; height: 20px; color: var(--muted); } .code-list { margin-top: 8px; }</style>
<div id="code"></div>
<script>
const name = one.params.app;
one.title((one.params.dir || "").replace(/\\/$/, "").split("/").pop() || "Code");
${folder}
one.render("#code", async () => !name ? one.notFound() : folder((await one.call("code", { app: name })).files, one.params.dir), { loading: "Loading code" });
</script>`,
    },
    file: {
      title: "Code",
      html: `
<div id="code" style="margin-top: 8px"></div>
<script>
one.title((one.params.path || "").split("/").pop() || "Code");
one.render("#code", async () => {
  if (!one.params.app || !one.params.path) return one.notFound();
  const f = await one.call("code", { app: one.params.app, path: one.params.path });
  setTimeout(() => one.code("#code", f.text, { path: f.path }));
  return "";
}, { loading: "Loading code" });
</script>`,
    },
  },
};
