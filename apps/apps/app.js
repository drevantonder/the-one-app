// The App Store: every app in the install, who owns it, where it's installed, its forks and its code; and "Add to…"
// the places the person may install it: themselves, groups they're an admin of, and everyone if they're an admin.
// An ordinary app: it installs with ctx.install, with the caller's own rights.

/// Where the caller may install apps: [{ scope, name }].
function places(ctx) {
  const out = [{ scope: "user", id: `user:${ctx.me.user}`, name: "You" }];
  for (const g of ctx.groups) if (g.role === "admin") out.push({ scope: g.id, id: g.id, name: g.name });
  if (ctx.me.role === "admin") out.push({ scope: "global", id: "global", name: "Everyone" });
  return out;
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

async function find(ctx, name) {
  const app = (await ctx.catalog()).find((a) => a.name === name);
  if (!app) throw new Error(`no app named ${name}`);
  return app;
}

export default {
  name: "apps",
  title: "App Store",
  icon: "square.grid.2x2",
  instructions: "The App Store lists every app in this install, where each is installed and its code. apps_add installs one for you, a group you're an admin of, or everyone (admins).",

  ops: {
    list: {
      description: "Every app in the install: owner, where it's installed (that you can see), forks, and the places you may add it to.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        const name = namer(ctx);
        const apps = (await ctx.catalog()).map((a) => ({
          name: a.name, title: a.title || a.name, icon: a.icon || null, owner: a.owner, owner_name: name(a.owner),
          from: a.from || null, forks: a.forks || [], deploys: a.deploys, installed: a.installed || [],
          installed_names: (a.installed || []).map(name), uses: a.uses || { apps: [], drivers: [] },
        }));
        apps.sort((a, b) => a.title.localeCompare(b.title));
        return { apps, places: places(ctx) };
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
      description: "Uninstall an app from a place: user, group:<id> or global.",
      input: { type: "object", required: ["app", "scope"], properties: { app: { type: "string" }, scope: { type: "string" } } },
      async run(ctx, { app, scope }) {
        ctx.install(app, scope, false);
        return `Removed ${app}.`;
      },
    },
    code: {
      description: "An app's code as deployed: its files, or with path one of them. Everyone may read every app's code.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string" }, path: { type: "string" } } },
      async run(ctx, { app, path }) {
        const a = await find(ctx, app);
        if (!a.bundle) throw new Error(`${app} has no deploy yet`);
        const files = untar(await ctx.blob.get(a.bundle));
        if (!path) return { bundle: a.bundle, files: files.map((f) => ({ path: f.path, size: f.bytes.length })) };
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
  .initial { font-size: 17px; border-radius: 9px; }
  .tags { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 3px; }
  .tag { font-size: 12px; padding: 1px 7px; border-radius: 10px; background: var(--line); color: var(--muted); }
</style>
<div id="list"></div>
<script>
async function show() {
  const { apps } = await one.call("list");
  const row = (a) => \`<a class="row" onclick='one.open("app", \${JSON.stringify({ app: a.name })})'>
    <div class="icon initial\${a.installed.length ? "" : " grey"}">\${one.esc(a.title[0].toUpperCase())}</div>
    <div class="main">
      <div class="title"><span>\${one.esc(a.title)}</span></div>
      <div class="sub">\${one.esc(a.name)} · by \${one.esc(a.owner_name)}\${a.from ? " · fork of " + one.esc(a.from) : ""}</div>
      \${a.installed.length ? '<div class="tags">' + a.installed_names.map((n) => '<span class="tag">' + one.esc(n) + "</span>").join("") + "</div>" : ""}
    </div>
    <div class="chev">›</div>
  </a>\`;
  document.getElementById("list").innerHTML = '<div class="list">' + apps.map(row).join("") + "</div>";
}
one.on("changed", show);
show();
</script>`,
    },
    app: {
      title: "App",
      html: `
<style>
  .head { display: flex; gap: 14px; align-items: center; margin: 12px 4px 4px; }
  .head .icon { width: 60px; height: 60px; font-size: 26px; border-radius: 14px; }
  .head h1 { font-size: 22px; margin: 0; }
  .pill { font: inherit; font-size: 15px; font-weight: 600; padding: 5px 14px; border-radius: 15px; border: 0; background: var(--accent); color: white; }
  .pill.off { background: var(--line); color: var(--accent); }
  .pill:disabled { opacity: 0.5; }
  #error { margin: 8px 4px; }
</style>
<div id="app"></div>
<div id="error" class="error small"></div>
<script>
const name = one.params.app;
one.title(name);
async function show() {
  const [{ apps, places }, code] = await Promise.all([one.call("list"), one.call("code", { app: name }).catch(() => null)]);
  const a = apps.find((x) => x.name === name);
  one.title(a.title);
  const place = (p) => {
    const on = a.installed.includes(p.id);
    return \`<div class="row plain"><div class="main"><div class="title"><span>\${one.esc(p.name)}</span></div></div>
      <button class="pill\${on ? " off" : ""}" onclick='toggle(this, \${JSON.stringify(p.scope)}, \${on})'>\${on ? "Remove" : "Add"}</button></div>\`;
  };
  const file = (f) => \`<a class="row plain" onclick='one.open("file", \${JSON.stringify({ app: name, path: f.path })})'>
      <div class="main"><div class="title"><span>\${one.esc(f.path)}</span><span class="when">\${f.size < 1024 ? f.size + " B" : Math.round(f.size / 1024) + " KB"}</span></div></div><div class="chev">›</div></a>\`;
  const link = (n) => \`<a class="row plain" onclick='one.open("app", \${JSON.stringify({ app: n })})'><div class="main"><div class="title"><span>\${one.esc(n)}</span></div></div><div class="chev">›</div></a>\`;
  const uses = [...a.uses.apps, ...a.uses.drivers];
  document.getElementById("app").innerHTML = \`
    <div class="head"><div class="icon">\${one.esc(a.title[0].toUpperCase())}</div>
      <div><h1>\${one.esc(a.title)}</h1><div class="muted small">\${one.esc(a.name)} · by \${one.esc(a.owner_name)} · \${a.deploys} deploy\${a.deploys === 1 ? "" : "s"}</div></div></div>
    <h2 class="section">Add to…</h2>
    <div class="list">\${places.map(place).join("")}</div>
    \${uses.length ? '<h2 class="section">Uses</h2><div class="list">' + uses.map((u) => '<div class="row plain"><div class="main">' + one.esc(u) + "</div></div>").join("") + "</div>" : ""}
    \${a.from ? '<h2 class="section">Forked from</h2><div class="list">' + link(a.from) + "</div>" : ""}
    \${a.forks.length ? '<h2 class="section">Forks</h2><div class="list">' + a.forks.map(link).join("") + "</div>" : ""}
    \${code ? '<h2 class="section">Code</h2><div class="list">' + code.files.filter((f) => f.path.startsWith("src/")).map(file).join("") + file(code.files.find((f) => f.path === "app.js")) + "</div>" : ""}\`;
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
    file: {
      title: "Code",
      html: `
<style>
  pre { background: var(--card); border-radius: var(--radius); padding: 12px 14px; overflow-x: auto; font: 12px/1.45 ui-monospace, SFMono-Regular, Menlo, monospace; margin: 8px 0; }
</style>
<pre id="code"></pre>
<script>
one.title(one.params.path);
one.call("code", { app: one.params.app, path: one.params.path }).then((f) => { document.getElementById("code").textContent = f.text; });
</script>`,
    },
  },
};
