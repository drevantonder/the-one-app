// The App Store: the apps built into the release, whether or not the install has added them, and the apps made in the
// install; who owns each, where it's installed, its forks and its code; and "Add to…" each place the person has a
// page: themselves, their groups and everyone, saying who can add it where they can't.
// An ordinary app: it installs with ctx.install, with the caller's own rights. Installing a built-in app the install
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
/// apps give them before their tools; `advanced: true` too, which the store lists after the rest).
async function described(ctx, bundle) {
  const file = untar(await ctx.blob.get(bundle)).find((f) => f.path === "app.js");
  const all = file ? new TextDecoder().decode(file.bytes) : "";
  const code = all.slice(Math.max(0, all.search(/^export default/m)));
  const field = (key) => code.match(new RegExp(`\\b${key}:\\s*"([^"]*)"`))?.[1] || null;
  const list = (kind) => code.match(new RegExp(`\\buses:\\s*\\{[^}]*\\b${kind}:\\s*\\[([^\\]]*)\\]`))?.[1].match(/"[^"]+"/g)?.map((n) => n.slice(1, -1)) || [];
  return { title: field("title"), icon: field("icon"), color: field("color"), tagline: field("tagline"), description: field("description"), needs: field("needs"), advanced: /\badvanced:\s*true\b/.test(code), uses: { apps: list("apps"), services: list("services") } };
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

/// Where a service runs, in a few words, and what each of the caller's devices offers for it: "Runs on linux, mac",
/// or what it needs (its `needs`) when none of their devices runs it; the devices they own it may be installed on (where
/// it can run, set up when it's switched on, or an owner removed it) or removed from; and whether they own none, so they ask an owner instead. Its device parts are the
/// services it uses, by their names.
async function where(ctx, a) {
  const devices = await ctx.devices();
  const parts = a.uses?.services || [];
  if (!parts.length) return null;
  const owns = (s) => (s.owners || []).some((o) => mine(ctx, o));
  const runs = devices.filter((s) => (s.services || []).some((d) => parts.includes(d)));
  const can = (s) => (s.parts || []).some((p) => parts.includes(p.name) && !p.on) || (s.off_services || []).some((d) => parts.includes(d.name));
  const removed = devices.filter((s) => !runs.includes(s) && can(s));
  const line = runs.length ? "Runs on " + runs.map((s) => s.name).join(", ") : a.needs || "Runs on none of your devices";
  return {
    line, runs: runs.map((s) => s.name), needs: a.needs || null,
    install: removed.filter(owns).map((s) => s.name), remove: runs.filter(owns).map((s) => s.name),
    ask: !runs.some(owns) && !removed.some(owns),
  };
}

async function find(ctx, name) {
  const all = await ctx.catalog();
  const app = all.find((a) => a.name === name);
  if (app) return app;
  const want = String(name || "").toLowerCase();
  const like = all.filter((a) => !a.deleted && (a.name.includes(want) || want.includes(a.name) || (a.title || "").toLowerCase() === want)).map((a) => a.name);
  throw new Error(`no app named ${name}: ${like.length ? `did you mean ${like.join(" or ")}?` : "apps_list gives their names"}`);
}

const APP = { type: "string", description: "an app's name, from apps_list, like gmail" };

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
  instructions: "The App Store lists every app in this install, where each is installed and its code. apps_add installs one for you, a group you're an admin of, or everyone (admins); apps_code reads any app's code, to learn from or fork. Deleted apps wait in Recently Deleted: apps_list with deleted true lists them.",

  // The apps whose schedule, where you may switch it, failed its last run.
  async badge(ctx) {
    return new Set((await ctx.failing()).map((s) => s.app)).size;
  },

  tools: {
    list: {
      reads: true,
      description: "Every app, a line each: its title and name (for the other apps_ tools), where it's added (that you can see), whether it's tools only (no screens or widgets, like Gmail: agents use it, and its accounts are in Accounts), and what it's for; then the places you may add apps to. Deleted apps are left out: deleted true lists only them.",
      // A screen asks with full for every app's details and picture, which are large.
      input: { type: "object", properties: { deleted: { type: "boolean", description: "only the apps in Recently Deleted, which apps_restore brings back" } } },
      async run(ctx, { deleted: gone, full } = {}) {
        const images = full;
        const name = namer(ctx);
        const every = await ctx.catalog();
        const all = every.filter((a) => full || !!a.deleted === !!gone);
        const offered = await Promise.all(all.map((a) => offers(ctx, a).catch(() => ({ screens: true, accounts: false }))));
        const apps = await Promise.all(all.map(async (a, i) => {
          const added = a.deploys > 0;
          const d = added || !a.built_in ? a : await described(ctx, a.built_in).catch(() => ({}));
          return {
            name: a.name, title: d.title || a.title || a.name, icon: d.icon || a.icon || null, color: d.color || a.color || null, ...(images ? { image: a.image || null, image_kind: a.image_kind || null } : {}),
            tagline: d.tagline || a.tagline || null, description: d.description || a.description || null,
            owner: a.owner, owner_name: name(a.owner),
            from: a.from || null, forks: a.forks || [], deploys: a.deploys, installed: a.installed || [],
            installed_names: (a.installed || []).map(name), uses: a.uses || { apps: [], services: [] },
            advanced: !!(d.advanced || a.advanced), built_in: !!a.built_in, added, up_to_date: a.built_in && added ? a.bundle === a.built_in : null,
            source: a.source || null, mine: mine(ctx, a.owner), deleted: a.deleted || null, screens: offered[i].screens, keeps_accounts: offered[i].accounts,
          };
        }));
        apps.sort((a, b) => a.advanced - b.advanced || a.title.localeCompare(b.title));
        if (full) return { apps, places: places(ctx), admin: ctx.me.role === "admin" };
        const line = (a) => `${a.title} (${a.name}): ${a.deleted ? `deleted ${new Date(a.deleted.time).toISOString().slice(0, 10)}${a.mine ? "" : `, ${a.owner_name}'s`}` : a.installed_names.length ? `added for ${a.installed_names.join(", ")}` : "not added"}`
          + `${a.screens ? "" : ", tools only"}${a.up_to_date === false ? ", an update waiting" : ""}. ${a.description || a.tagline || ""}`.trimEnd();
        if (gone) return apps.length ? [...apps.map(line), "", "apps_restore brings back one you own; apps_erase deletes it for good."].join("\n") : "Nothing is in Recently Deleted.";
        const deleted = every.filter((a) => a.deleted).length;
        return [
          ...apps.map(line), "",
          `You may add apps for: ${places(ctx).filter((p) => p.may).map((p) => p.scope === "user" ? "user (you)" : p.scope === "global" ? "global (everyone)" : p.name).join(", ")}.`,
          ...(deleted ? [`${deleted} deleted ${deleted === 1 ? "app is" : "apps are"} in Recently Deleted: apps_list with deleted true lists them.`] : []),
        ].join("\n");
      },
    },
    add: {
      reads: false,
      description: "Install an app, and the apps it uses, in a place: user (you), a group you're an admin of (by its name) or global (everyone; admins only). It shows on that place's page.",
      input: { type: "object", required: ["app", "scope"], properties: { app: APP, scope: placeInput } },
      async run(ctx, { app, scope }) {
        const a = await find(ctx, app);
        ctx.install(app, placeOf(ctx, scope));
        return `Added ${a.title || app}.`;
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
      description: "Set where an app you own keeps its code, shown on its page as Source: a web address: GitHub, any git host or page, or a repo's address in Repos (<your One App's address>/repos/<owner>/<repo>.git). An empty link clears it. Built-in apps start with their folder in the open-source repo; forks start with none.",
      input: { type: "object", required: ["app", "link"], properties: { app: APP, link: { type: "string", description: "https://…, or a repo's address in Repos; empty for none" } } },
      async run(ctx, { app, link }) {
        const a = await find(ctx, app);
        if (!mine(ctx, a.owner)) throw new Error(`only ${app}'s owners set its source link`);
        if (!a.deploys && a.built_in) throw new Error(`${app} isn't added to this install yet: add it first`);
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
      description: "Where an app's service runs (\"Runs on linux, mac\"), or what it needs when none of your devices runs it; the devices you own it may be installed on or removed from; and ask: true when you own none of those, so you ask a device's owner to install it.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "the name of an app with a service, like imessage" } } },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return where(ctx, a.ops || a.uses ? a : { ...a, ...(await described(ctx, a.built_in).catch(() => ({}))) });
      },
    },
    install_on: {
      reads: false,
      description: "Install an app's service on a device you own that can run it: the device sets it up and runs it, as switching it on in Settings › Devices does. apps_devices says where it may go.",
      input: { type: "object", required: ["app", "device"], properties: { app: { type: "string", description: "the name of an app with a service, like imessage" }, device: { type: "string", description: "a device's name, from apps_devices' install" } } },
      async run(ctx, { app, device }) {
        const a = await find(ctx, app);
        const all = await ctx.devices();
        if (!all.some((s) => s.name === device)) throw new Error(`no device called ${device}: your devices are ${all.map((s) => s.name).join(", ")}`);
        for (const part of a.uses?.services || []) ctx.service(part, device, true);
        return `${a.title || app} is installed on ${device}.`;
      },
    },
    remove_from: {
      reads: false,
      confirm: true,
      description: "Remove an app's service from a device you own: it stops running there, and goes on running on your other devices that have it. What it keeps on that device stays, and install_on puts it back.",
      input: { type: "object", required: ["app", "device"], properties: { app: { type: "string", description: "the name of an app with a service, like imessage" }, device: { type: "string", description: "a device's name, from apps_devices' remove" } } },
      async run(ctx, { app, device }) {
        const a = await find(ctx, app);
        const all = await ctx.devices();
        const there = all.find((s) => s.name === device);
        if (!there) throw new Error(`no device called ${device}: your devices are ${all.map((s) => s.name).join(", ")}`);
        for (const part of a.uses?.services || []) if ((there?.services || []).includes(part)) ctx.service(part, device, false);
        return `${a.title || app} is removed from ${device}.`;
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
