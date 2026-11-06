// The App Store: the apps built into the release, whether or not the network has added them, and the apps made in the
// network; who owns each, where it's installed, its forks and its code; and "Add to…" each place the person has a
// page: themselves, their groups and everyone, saying who can add it where they can't.
// An ordinary app: it installs with ctx.install, with the caller's own rights. Installing a built-in app the network
// hasn't added yet adds it (admins only). Its screens are React, in screens/ and lib/, built into dist/ when it's
// deployed (`one deploy apps/apps`), which is committed because a release ships the app as it is.
import mainHtml from "./dist/main.html" with { type: "text" };
import appHtml from "./dist/app.html" with { type: "text" };
import addHtml from "./dist/add.html" with { type: "text" };
import sourceHtml from "./dist/source.html" with { type: "text" };
import activityHtml from "./dist/activity.html" with { type: "text" };
import codeHtml from "./dist/code.html" with { type: "text" };
import fileHtml from "./dist/file.html" with { type: "text" };

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
/// or the network's and they're an admin.
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

/// What a built-in app the network hasn't added says about itself: nothing runs it yet, so its code says (the first
/// `title`, `icon`, `color`, `tagline` and `description` after the line starting `export default` given as plain strings, as the built-in
/// apps give them before their tools; `advanced: true` too, which the store lists after the rest).
async function described(ctx, bundle) {
  const file = untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const all = file ? new TextDecoder().decode(file.bytes) : "";
  const code = all.slice(Math.max(0, all.search(/^export default/m)));
  const field = (key) => code.match(new RegExp(`\\b${key}:\\s*"([^"]*)"`))?.[1] || null;
  const list = (kind) => code.match(new RegExp(`\\buses:\\s*\\{[^}]*\\b${kind}:\\s*\\[([^\\]]*)\\]`))?.[1].match(/"[^"]+"/g)?.map((n) => n.slice(1, -1)) || [];
  const platforms = code.match(/\bservice:\s*\{[^}]*\bplatforms:\s*\[([^\]]*)\]/)?.[1].match(/"[^"]+"/g)?.map((n) => n.slice(1, -1));
  return { title: field("title"), icon: field("icon"), color: field("color"), tagline: field("tagline"), description: field("description"), needs: field("needs"), advanced: /\badvanced:\s*true\b/.test(code), uses: { apps: list("apps"), services: list("services") }, ...(platforms ? { service: { platforms } } : {}) };
}

/// What an app offers besides its tools: screens (a main screen or a widget, so it has a place on a home page), and
/// accounts (an `accounts` tool: what it signs in to, which Accounts lists). What it says about itself where it's
/// installed for the caller; elsewhere, its code.
async function offers(ctx, a) {
  if (a.ops) return { screens: (a.screens || []).some((x) => x.name === "main") || !!(a.widgets || []).length, accounts: a.ops.some((o) => o.name === "accounts") };
  const bundle = a.bundle || a.built_in;
  const file = bundle && untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const code = file ? new TextDecoder().decode(file.bytes) : "";
  return { screens: /\bmain:\s*\{/.test(code) || /\bwidgets:\s*\{\s*[\w"]/.test(code), accounts: /^\s*accounts:\s*\{/m.test(code) };
}

/// The platform a device's build is for, as an app's service names them.
const platformOf = (target) => (/apple|darwin/.test(target || "") ? "mac" : /windows/.test(target || "") ? "windows" : /linux/.test(target || "") ? "linux" : null);

/// Whether an app has a service.
const hasService = (a) => !!(a.service || a.uses?.services?.length);

/// Where an app's service runs, and where it may: `runs_on` (each node it runs on for a place where it's added, by
/// `name`, none once that node was removed, with `scope`, `online` and `problem`); `nodes`, the names of the caller's
/// own nodes on a platform its service is built for, and the Cloudflare node's for an admin when it's built for it; and
/// `problem`, its one problem box when no node it runs on is on and well, with `move` true when moving it fixes that.
/// Null for an app with no service.
async function where(ctx, a) {
  if (!hasService(a)) return null;
  const platforms = a.service?.platforms;
  const devices = await ctx.devices();
  const nodes = devices.filter((d) => d.user === ctx.me.user && (!platforms || platforms.includes(platformOf(d.target)))).map((d) => d.name);
  if (platforms?.includes("cloudflare") && ctx.me.role === "admin") nodes.push("cloudflare");
  const runs_on = a.runs_on || [];
  const title = a.title || a.name;
  let problem = null;
  if (a.installed?.length && !runs_on.some((r) => r.name && r.online && !r.problem)) {
    const r = runs_on[0];
    const text = !r ? "Choose the node it runs on." : !r.name ? "The node it ran on was removed. Choose where it runs now." : !r.online ? `${r.name} is off. Turn it on, or move it to another node.` : r.problem;
    problem = nodes.length || r ? { title: `${title} needs its service on a node that's on`, text, move: nodes.length > 0 } : { title: `${title} can't run on your devices yet`, text: "None of them is a platform its service is built for.", move: false };
  }
  return { runs_on, nodes, problem };
}

/// The node an app added in `scope` runs its service on: `node` if given, or the only one it may; none for an app with
/// no service. Asks where when there's a choice.
async function nodeFor(ctx, a, node) {
  const w = await where(ctx, a);
  if (!w) return undefined;
  if (node) {
    if (!w.nodes.some((n) => n.toLowerCase() === String(node).toLowerCase())) throw new Error(`${a.title || a.name} can't run on ${node}: it can run on ${w.nodes.join(", ") || "none of your nodes"}`);
    return node;
  }
  if (w.nodes.length > 1) throw new Error(`Where should ${a.title || a.name} run? Give node: ${w.nodes.join(" or ")}.`);
  return w.nodes[0];
}

/// The apps `a` uses, and those they use, that have a service.
async function usedWithService(ctx, a) {
  const out = [];
  const seen = new Set([a.name]);
  const next = [...(a.uses?.apps || [])];
  while (next.length) {
    const name = next.shift();
    if (seen.has(name)) continue;
    seen.add(name);
    const used = await withService(ctx, await find(ctx, name).catch(() => null) || { name });
    next.push(...(used.uses?.apps || []));
    if (hasService(used)) out.push(used);
  }
  return out;
}

async function find(ctx, name) {
  const all = await ctx.catalog();
  const app = all.find((a) => a.name === name);
  if (app) return app;
  const want = String(name || "").toLowerCase();
  const like = all.filter((a) => !a.deleted && (a.name.includes(want) || want.includes(a.name) || (a.title || "").toLowerCase() === want)).map((a) => a.name);
  throw new Error(`no app named ${name}: ${like.length ? `did you mean ${like.join(" or ")}?` : "apps_list gives their names"}`);
}

/// The catalog's apps, as the list and the screens say them, the ones everyone uses first: those deleted (`gone`) or not,
/// or, for `full`, every one with its picture, which is large.
async function listed(ctx, { full, gone }) {
  const name = namer(ctx);
  const every = await ctx.catalog();
  const all = every.filter((a) => full || !!a.deleted === !!gone);
  const offered = await Promise.all(all.map((a) => offers(ctx, a).catch(() => ({ screens: true, accounts: false }))));
  const apps = await Promise.all(all.map(async (a, i) => {
    const added = a.deploys > 0;
    const d = added || !a.built_in ? a : await described(ctx, a.built_in).catch(() => ({}));
    return {
      name: a.name, title: d.title || a.title || a.name, icon: d.icon || a.icon || null, color: d.color || a.color || null, ...(full ? { image: a.image || null, image_kind: a.image_kind || null } : {}),
      tagline: d.tagline || a.tagline || null, description: d.description || a.description || null,
      owner: a.owner, owner_name: name(a.owner),
      from: a.from || null, forks: a.forks || [], deploys: a.deploys, installed: a.installed || [],
      installed_names: (a.installed || []).map(name), uses: d.uses || a.uses || { apps: [], services: [] }, service: d.service || a.service || null, runs_on: a.runs_on || [],
      advanced: !!(d.advanced || a.advanced), built_in: !!a.built_in, added, up_to_date: a.built_in && added ? a.bundle === a.built_in : null,
      source: a.source || null, mine: mine(ctx, a.owner), deleted: a.deleted || null, screens: offered[i].screens, keeps_accounts: offered[i].accounts,
    };
  }));
  apps.sort((a, b) => a.advanced - b.advanced || a.title.localeCompare(b.title));
  return { apps, deleted: every.filter((a) => a.deleted).length };
}

const APP = { type: "string", description: "an app's name, from apps_list, like gmail" };
const NODE = { type: "string", description: "the node that runs its service: one of yours by name, or cloudflare, as apps_devices lists them" };

/// An app with what its code says of its service, for one not added yet that nothing runs.
async function withService(ctx, a) {
  return a.uses || a.service || !a.built_in ? a : { ...a, ...(await described(ctx, a.built_in).catch(() => ({}))) };
}

/// Times in what `health` gives, ms since 1970, as ISO 8601.
function iso(v) {
  if (Array.isArray(v)) return v.map(iso);
  if (!v || typeof v !== "object") return v;
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, typeof x === "number" && x > 1e12 && ["time", "first", "last", "next", "since"].includes(k) ? new Date(x).toISOString() : iso(x)]));
}

export default {
  name: "apps",
  title: "App Store",
  tagline: "Find and add apps",
  icon: "squares-four",
  color: "#007aff",
  description: "Every app you can add, and its code",
  instructions: "The App Store lists every app in this network, where each is installed and its code. apps_add installs one for you, a group you're an admin of, or everyone (admins); apps_code reads any app's code, to learn from or fork. Deleted apps wait in Recently Deleted: apps_list with deleted true lists them.",

  // The apps whose schedule, where you may switch it, failed its last run.
  async badge(ctx) {
    return new Set((await ctx.failing()).map((s) => s.app)).size;
  },

  tools: {
    list: {
      reads: true,
      description: "Every app, a line each: its title and name (for the other apps_ tools), where it's added (that you can see), whether it's tools only (no screens or widgets, like Gmail: agents use it, and its accounts are in Accounts), and what it's for; then the places you may add apps to. Deleted apps are left out: deleted true lists only them.",
      input: { type: "object", properties: { deleted: { type: "boolean", description: "only the apps in Recently Deleted, which apps_restore brings back" } } },
      async run(ctx, { deleted: gone } = {}) {
        const { apps, deleted } = await listed(ctx, { gone });
        const line = (a) => `${a.title} (${a.name}): ${a.deleted ? `deleted ${new Date(a.deleted.time).toISOString().slice(0, 10)}${a.mine ? "" : `, ${a.owner_name}'s`}` : a.installed_names.length ? `added for ${a.installed_names.join(", ")}` : "not added"}`
          + `${a.screens ? "" : ", tools only"}${a.up_to_date === false ? ", an update waiting" : ""}. ${a.description || a.tagline || ""}`.trimEnd();
        if (gone) return apps.length ? [...apps.map(line), "", "apps_restore brings back one you own; apps_erase deletes it for good."].join("\n") : "Nothing is in Recently Deleted.";
        return [
          ...apps.map(line), "",
          `You may add apps for: ${places(ctx).filter((p) => p.may).map((p) => p.scope === "user" ? "user (you)" : p.scope === "global" ? "global (everyone)" : p.name).join(", ")}.`,
          ...(deleted ? [`${deleted} deleted ${deleted === 1 ? "app is" : "apps are"} in Recently Deleted: apps_list with deleted true lists them.`] : []),
        ].join("\n");
      },
    },
    add: {
      reads: false,
      description: "Install an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only). It shows on that place's page. An app with a service runs it on node, one of your nodes it can run on (apps_devices lists them), or for everyone's or a group's, the Cloudflare node; asked when there's a choice.",
      input: { type: "object", required: ["app", "scope"], properties: { app: APP, scope: placeInput, node: NODE } },
      async run(ctx, { app, scope, node }) {
        const a = await withService(ctx, await find(ctx, app));
        const place = placeOf(ctx, scope);
        // Each app it uses whose service runs nowhere for this place yet runs it on node too if it can, or on the one
        // node it can; with a choice, its page asks where.
        for (const used of await usedWithService(ctx, a)) {
          const w = await where(ctx, used);
          if (used.runs_on?.some((r) => r.scope === (place === "user" ? `user:${ctx.me.user}` : place))) continue;
          const on = w.nodes.find((n) => node && n.toLowerCase() === String(node).toLowerCase()) || (w.nodes.length === 1 ? w.nodes[0] : undefined);
          if (on) ctx.install(used.name, place, true, on);
        }
        ctx.install(app, place, true, await nodeFor(ctx, a, node));
        return `Added ${a.title || app}.`;
      },
    },
    move: {
      reads: false,
      description: "Move an app's service to another node, for a place it's added to (by default where it runs now): it stops on the node it ran on and starts afresh on this one. apps_devices lists the nodes it can run on.",
      input: { type: "object", required: ["app", "node"], properties: { app: APP, node: NODE, scope: placeInput } },
      async run(ctx, { app, node, scope }) {
        const a = await withService(ctx, await find(ctx, app));
        if (!hasService(a)) throw new Error(`${a.title || app} has no service to move`);
        const place = scope ? placeOf(ctx, scope) : a.runs_on?.find((r) => r.scope)?.scope || a.installed?.find((s) => places(ctx).some((p) => p.may && p.id === s));
        if (!place) throw new Error(`${a.title || app} isn't added anywhere you may move it: apps_add adds it`);
        ctx.install(app, place === `user:${ctx.me.user}` ? "user" : place, true, await nodeFor(ctx, a, node));
        return `${a.title || app} runs on ${node}.`;
      },
    },
    remove: {
      confirm: true,
      reads: false,
      description: "Uninstall an app from a place: user (you), a group (by its name) or global (everyone). What it keeps there goes with it.",
      input: { type: "object", required: ["app", "scope"], properties: { app: APP, scope: placeInput } },
      async run(ctx, { app, scope }) {
        await find(ctx, app);
        ctx.install(app, placeOf(ctx, scope), false);
        return `Removed ${app}.`;
      },
    },
    delete: {
      confirm: true,
      reads: false,
      description: "Delete an app you own: it leaves every page and stops running, and waits in Recently Deleted, where everyone sees it and its owners restore it (apps_restore) or erase it for good.",
      input: { type: "object", required: ["app"], properties: { app: APP } },
      async run(ctx, { app }) {
        await find(ctx, app);
        ctx.retire(app, "deleted");
        return `Deleted ${app}: it's in Recently Deleted.`;
      },
    },
    restore: {
      reads: false,
      description: "Restore an app you own from Recently Deleted: it runs again, on the pages it was on.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "a deleted app's name, from apps_list with deleted true" } } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        if (!a.deleted) throw new Error(`${app} isn't deleted: apps_list with deleted true lists those that are`);
        ctx.retire(app, "restored");
        return `Restored ${app}.`;
      },
    },
    erase: {
      confirm: true,
      reads: false,
      description: "Erase a deleted app you own for good: it's gone from every list and its name can't be used again.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "a deleted app's name, from apps_list with deleted true" } } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        if (!a.deleted) throw new Error(`delete ${app} (apps_delete) before erasing it`);
        ctx.retire(app, "erased");
        return `Erased ${app}.`;
      },
    },
    source: {
      reads: false,
      description: "Set where an app you own keeps its code, shown on its page as Source: a web address: GitHub, any git host or page, or a repo's address in Repos (<your One App network's address>/repos/<owner>/<repo>.git). An empty link clears it. Built-in apps start with their folder in the open-source repo; forks start with none.",
      input: { type: "object", required: ["app", "link"], properties: { app: APP, link: { type: "string", description: "https://…, or a repo's address in Repos; empty for none" } } },
      async run(ctx, { app, link }) {
        const a = await find(ctx, app);
        if (!mine(ctx, a.owner)) throw new Error(`only ${app}'s owners set its source link`);
        if (!a.deploys && a.built_in) throw new Error(`${app} isn't added to this network yet: add it first`);
        ctx.source(app, (link || "").trim());
        return link?.trim() ? `${app}'s source is ${link.trim()}.` : `${app} has no source link now.`;
      },
    },
    health: {
      reads: true,
      description: "How an app is doing where it's installed for you: its schedules (when each runs, on or off, as whom) and their recent runs and failures; for its owners, who used it, when and from what, its errors grouped with counts, and its AI cost. Times are ISO 8601.",
      input: { type: "object", required: ["app"], properties: { app: APP } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return iso({ schedules: a.schedules || [], activity: a.activity || [], insights: a.insights || null });
      },
    },
    _list: {
      reads: true,
      description: "For the screens: every app with its details and picture, the places you may add apps to, and whether you're an admin.",
      input: { type: "object", properties: {} },
      async run(ctx) {
        return { apps: (await listed(ctx, { full: true })).apps, places: places(ctx), admin: ctx.me.role === "admin" };
      },
    },
    _health: {
      reads: true,
      description: "For the screens: health, with times as milliseconds since 1970.",
      input: { type: "object", required: ["app"], properties: { app: APP } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return { schedules: a.schedules || [], activity: a.activity || [], insights: a.insights || null };
      },
    },
    switch: {
      reads: false,
      description: "Switch one of an app's schedules on or off in a place where it's installed (user, a group by its name, or global); switched on, it runs as you.",
      input: { type: "object", required: ["app", "schedule", "scope", "on"], properties: { app: APP, schedule: { type: "string", description: "the schedule's name, from apps_health's schedules" }, scope: placeInput, on: { type: "boolean", description: "true to switch it on, false off" } } },
      async run(ctx, { app, schedule, scope, on }) {
        ctx.schedule(app, schedule, placeOf(ctx, scope), on);
        return `${schedule} is ${on ? "on" : "off"}.`;
      },
    },
    devices: {
      reads: true,
      description: "Where an app's service runs (runs_on: each node, whether it's on, and what went wrong), the nodes it can run on (nodes, for apps_add and apps_move), and its problem when it runs on no node that's on.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "the name of an app with a service, like whatsapp" } } },
      async run(ctx, { app }) {
        return where(ctx, await withService(ctx, await find(ctx, app)));
      },
    },
    code: {
      reads: true,
      description: "An app's code as deployed: its files and their sizes, or with path one of them as text. Everyone may read every app's code: start a new app from one that's close.",
      input: { type: "object", required: ["app"], properties: { app: APP, path: { type: "string", description: "a file's path, from apps_code without path, like app.js or src/app.js" } } },
      async run(ctx, { app, path }) {
        const a = await find(ctx, app);
        const bundle = a.bundle || a.built_in;
        if (!bundle) throw new Error(`${app} has no deploy yet`);
        const files = untar(await ctx.blob.get(bundle));
        if (!path) return { bundle, files: files.map((f) => ({ path: f.path, size: f.bytes.length })) };
        const file = files.find((f) => f.path === path);
        if (!file) throw new Error(`${app} has no ${path}: it has ${files.map((f) => f.path).join(", ")}`);
        return { path, text: new TextDecoder().decode(file.bytes) };
      },
    },
  },

  screens: {
    main: { title: "App Store", html: mainHtml },
    // An app's page, where the list has no room to show it beside.
    app: { title: "App", html: appHtml },
    // Sheets over them: where an app is added, and its source link.
    add: { title: "Add to…", html: addHtml },
    source: { title: "Source Link", html: sourceHtml },
    activity: { title: "Activity", html: activityHtml },
    code: { title: "Code", html: codeHtml },
    file: { title: "Code", html: fileHtml },
  },
};
