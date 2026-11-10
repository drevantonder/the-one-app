// The App Store: the apps built into the release, whether or not the network has added them, and the apps made in the
// network; who owns each, where it's added, its forks and its code; and "Add to" or "Manage" for the
// places the user may put an app: themselves, the groups they're an admin of and, for an admin, everyone.
// An ordinary app: it adds apps with ctx.add, with the caller's own rights. Adding a built-in app the network
// hasn't added yet adds it (admins only). Its screens are React, in screens/ and lib/, built into dist/ when it's
// deployed (`one deploy apps/apps`), which is committed because a release ships the app as it is.
import mainHtml from "./dist/main.html" with { type: "text" };
import appHtml from "./dist/app.html" with { type: "text" };
import sourceHtml from "./dist/source.html" with { type: "text" };
import activityHtml from "./dist/activity.html" with { type: "text" };
import codeHtml from "./dist/code.html" with { type: "text" };
import fileHtml from "./dist/file.html" with { type: "text" };

/// `why` says who can, in two words, where the caller can't.
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

const hasService = (a) => !!a.nodes;

/// Where an app's service runs, and where it may: `runs_on` (each node it runs on for a place where it's added, by
/// `name`, none once that node was removed, with `scope`, `online` and `problem`); `nodes`, as the catalog gives them;
/// and `problem`, its one problem box when no node it runs on is on and well, with `move` true when moving it fixes that.
/// Null for an app with no service.
function where(a) {
  if (!hasService(a)) return null;
  const nodes = a.nodes;
  const runs_on = a.runs_on || [];
  const title = a.title || a.name;
  let problem = null;
  if (a.added?.length && !runs_on.some((r) => r.name && r.online && !r.problem)) {
    const r = runs_on[0];
    const text = !r ? "Choose the device it runs on." : !r.name ? "Its device was removed. Choose another." : !r.online ? `${r.name} is off. Turn it on, or move it.` : r.problem;
    problem = nodes.length || r ? { title: `${title} needs a device that's on`, text, move: nodes.length > 0 } : { title: `${title} can't run on your devices yet`, text: a.needs ? `${a.needs}.` : "It isn’t built for any of them.", move: false };
  }
  return { runs_on, nodes, problem };
}

function nodeFor(a, node) {
  const w = where(a);
  if (!w) return undefined;
  if (node) {
    if (!w.nodes.some((n) => n.toLowerCase() === String(node).toLowerCase())) throw new Error(`${a.title || a.name} can't run on ${node}: it can run on ${w.nodes.join(", ") || "none of your devices"}`);
    return node;
  }
  if (w.nodes.length > 1) throw new Error(`Where should ${a.title || a.name} run? Give node: ${w.nodes.join(" or ")}.`);
  return w.nodes[0];
}

async function find(ctx, name) {
  const all = await ctx.catalog();
  const app = all.find((a) => a.name === name);
  if (app) return app;
  const want = String(name || "").toLowerCase();
  const like = all.filter((a) => !a.deleted && (a.name.includes(want) || want.includes(a.name) || (a.title || "").toLowerCase() === want)).map((a) => a.name);
  throw new Error(`no app named ${name}: ${like.length ? `did you mean ${like.join(" or ")}?` : "apps_list gives their names"}`);
}

/// Those deleted (`gone`) or not, or, for `full`, every one with its picture, which is large.
async function listed(ctx, { full, gone }) {
  const name = namer(ctx);
  const every = await ctx.catalog();
  const all = every.filter((a) => full || !!a.deleted === !!gone);
  const apps = all.map((a) => {
    const deployed = a.deploys > 0;
    return {
      name: a.name, title: a.title || a.name, icon: a.icon || null, color: a.color || null, ...(full ? { image: a.image || null, image_kind: a.image_kind || null } : {}),
      tagline: a.tagline || null, description: a.description || null,
      owner: a.owner, owner_name: name(a.owner),
      from: a.from || null, forks: a.forks || [], deploys: a.deploys, added: a.added || [],
      added_names: (a.added || []).map(name), uses: a.uses || { apps: [], services: [] }, service: a.service || null, runs_on: a.runs_on || [],
      advanced: !!a.advanced, built_in: !!a.built_in, deployed, up_to_date: a.built_in && deployed ? a.bundle === a.built_in : null,
      source: a.source || null, mine: mine(ctx, a.owner), deleted: a.deleted || null, screens: !!a.offers?.screens, opens: !!a.offers?.opens, keeps_accounts: !!a.offers?.accounts, signs_in: !!a.offers?.accounts && !!a.screens?.some?.((s) => s.name === "settings"),
    };
  });
  apps.sort((a, b) => a.advanced - b.advanced || a.title.localeCompare(b.title));
  return { apps, deleted: every.filter((a) => a.deleted).length };
}

const APP = { type: "string", description: "an app's name, from apps_list, like gmail" };
const NODE = { type: "string", description: "the node that runs its service: one of yours by name, or cloudflare, as apps_devices lists them" };

function iso(v) {
  if (Array.isArray(v)) return v.map(iso);
  if (!v || typeof v !== "object") return v;
  return Object.fromEntries(Object.entries(v).map(([k, x]) => [k, typeof x === "number" && x > 1e12 && ["time", "first", "last", "next", "since"].includes(k) ? new Date(x).toISOString() : iso(x)]));
}

// What apps_health gives: its schedules, recent runs and, for its owners, who used it and what it cost.
const HEALTH = {
  type: "object",
  required: ["schedules", "activity"],
  properties: {
    schedules: { type: "array", items: { type: "object", additionalProperties: true } },
    activity: { type: "array", items: { type: "object", additionalProperties: true } },
    insights: { type: ["object", "null"], description: "for its owners only", additionalProperties: true },
  },
};

export default {
  name: "apps",
  title: "App Store",
  tagline: "Find and add apps",
  icon: "squares-four",
  color: "#007aff",
  description: "Every app you can add, and its code",
  instructions: "The App Store lists every app in this network, where each is added and its code. add adds one for you, a group you're an admin of, or everyone (admins); apps_code reads any app's code, to learn from or fork. Deleted apps wait in Recently Deleted: apps_list with deleted true lists them.",

  // The apps whose schedule, where you may switch it, failed its last run.
  async badge(ctx) {
    return new Set((await ctx.failing()).map((s) => s.app)).size;
  },

  tools: {
    list: {
      title: "Every app",
      reads: true,
      description: "Every app, a line each: its title and name (for the other apps_ tools), where it's added (that you can see), whether it's tools only (no screens or widgets, like Gmail: agents use it, and its accounts are in Accounts), and what it's for; then the places you may add apps to. Deleted apps are left out: deleted true lists only them.",
      input: { type: "object", properties: { deleted: { type: "boolean", description: "only the apps in Recently Deleted, which apps_restore brings back" } } },
      output: { type: "string", description: "a line for each app, then the places you may add apps to" },
      async run(ctx, { deleted: gone } = {}) {
        const { apps, deleted } = await listed(ctx, { gone });
        const line = (a) => `${a.title} (${a.name}): ${a.deleted ? `deleted ${new Date(a.deleted.time).toISOString().slice(0, 10)}${a.mine ? "" : `, ${a.owner_name}'s`}` : a.added_names.length ? `added for ${a.added_names.join(", ")}` : "not added"}`
          + `${a.screens ? "" : ", tools only"}${a.up_to_date === false ? ", an update waiting" : ""}. ${a.description || a.tagline || ""}`.trimEnd();
        if (gone) return apps.length ? [...apps.map(line), "", "apps_restore brings back one you own; apps_erase deletes it for good."].join("\n") : "Nothing is in Recently Deleted.";
        return [
          ...apps.map(line), "",
          `You may add apps for: ${places(ctx).filter((p) => p.may).map((p) => p.scope === "user" ? "user (you)" : p.scope === "global" ? "global (everyone)" : p.name).join(", ")}.`,
          ...(deleted ? [`${deleted} deleted ${deleted === 1 ? "app is" : "apps are"} in Recently Deleted: apps_list with deleted true lists them.`] : []),
        ].join("\n");
      },
    },
    _add: {
      title: "Add an app in a place (for the screens)",
      reads: false,
      description: "For the screens: add an app in a place, as add does, on the node it already runs on or else the first it can.",
      input: { type: "object", required: ["app", "scope"], properties: { app: APP, scope: placeInput } },
      output: { type: "null" },
      async run(ctx, { app, scope }) {
        const w = where(await find(ctx, app));
        ctx.add(app, placeOf(ctx, scope), true, w?.runs_on.find((r) => r.name)?.name || w?.nodes[0]);
      },
    },
    move: {
      title: "Move an app's service to another node",
      reads: false,
      description: "Move an app's service to another node, for a place it's added to (by default where it runs now): it stops on the node it ran on and starts afresh on this one. apps_devices lists the nodes it can run on.",
      input: { type: "object", required: ["app", "node"], properties: { app: APP, node: NODE, scope: placeInput } },
      output: { type: "string", description: "where it runs now, in words" },
      async run(ctx, { app, node, scope }) {
        const a = await find(ctx, app);
        if (!hasService(a)) throw new Error(`${a.title || app} has no service to move`);
        const place = scope ? placeOf(ctx, scope) : a.runs_on?.find((r) => r.scope)?.scope || a.added?.find((s) => places(ctx).some((p) => p.may && p.id === s));
        if (!place) throw new Error(`${a.title || app} isn't added anywhere you may move it: add adds it`);
        ctx.add(app, place === `user:${ctx.me.user}` ? "user" : place, true, nodeFor(a, node));
        return `${a.title || app} runs on ${node}.`;
      },
    },
    _remove: {
      title: "Remove an app from a place (for the screens)",
      reads: false,
      description: "For the screens: remove an app from a place, as remove does.",
      input: { type: "object", required: ["app", "scope"], properties: { app: APP, scope: placeInput } },
      output: { type: "null" },
      async run(ctx, { app, scope }) {
        ctx.add(app, placeOf(ctx, scope), false);
      },
    },
    delete: {
      title: "Delete an app",
      confirm: true,
      reads: false,
      description: "Delete an app you own: it leaves every page and stops running, and waits in Recently Deleted, where everyone sees it and its owners restore it (apps_restore) or erase it for good.",
      input: { type: "object", required: ["app"], properties: { app: APP } },
      output: { type: "string", description: "that it is deleted, in words" },
      async run(ctx, { app }) {
        await find(ctx, app);
        ctx.retire(app, "deleted");
        return `Deleted ${app}: it's in Recently Deleted.`;
      },
    },
    restore: {
      title: "Restore a deleted app",
      reads: false,
      description: "Restore an app you own from Recently Deleted: it runs again, on the pages it was on.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "a deleted app's name, from apps_list with deleted true" } } },
      output: { type: "string", description: "that it is restored, in words" },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        if (!a.deleted) throw new Error(`${app} isn't deleted: apps_list with deleted true lists those that are`);
        ctx.retire(app, "restored");
        return `Restored ${app}.`;
      },
    },
    erase: {
      title: "Erase a deleted app for good",
      confirm: true,
      reads: false,
      description: "Erase a deleted app you own for good: it's gone from every list and its name can't be used again.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "a deleted app's name, from apps_list with deleted true" } } },
      output: { type: "string", description: "that it is erased, in words" },
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        if (!a.deleted) throw new Error(`delete ${app} (apps_delete) before erasing it`);
        ctx.retire(app, "erased");
        return `Erased ${app}.`;
      },
    },
    source: {
      title: "Set where an app keeps its code",
      reads: false,
      description: "Set where an app you own keeps its code, shown on its page as Source: a web address: GitHub, any git host or page, or a repo's address in Repos (<your One App network's address>/repos/<owner>/<repo>.git). An empty link clears it. Built-in apps start with their folder in the open-source repo; forks start with none.",
      input: { type: "object", required: ["app", "link"], properties: { app: APP, link: { type: "string", description: "https://…, or a repo's address in Repos; empty for none" } } },
      output: { type: "string", description: "what its source link is now, in words" },
      async run(ctx, { app, link }) {
        const a = await find(ctx, app);
        if (!mine(ctx, a.owner)) throw new Error(`only ${app}'s owners set its source link`);
        if (!a.deploys && a.built_in) throw new Error(`${app} isn't added to this network yet: add it first`);
        ctx.source(app, (link || "").trim());
        return link?.trim() ? `${app}'s source is ${link.trim()}.` : `${app} has no source link now.`;
      },
    },
    health: {
      title: "How an app is doing",
      reads: true,
      description: "How an app is doing where it's added for you: its schedules (when each runs, on or off, as whom) and their recent runs and failures; for its owners, who used it, when and from what, its errors grouped with counts, and its AI cost. Times are ISO 8601.",
      input: { type: "object", required: ["app"], properties: { app: APP } },
      output: HEALTH,
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return iso({ schedules: a.schedules || [], activity: a.activity || [], insights: a.insights || null });
      },
    },
    _list: {
      title: "Every app with details (for the screens)",
      reads: true,
      description: "For the screens: every app with its details and picture, the places you may add apps to, and whether you're an admin.",
      input: { type: "object", properties: {} },
      output: {
        type: "object",
        required: ["apps", "places", "admin", "failing"],
        properties: {
          apps: { type: "array", items: { type: "object", required: ["name", "title", "owner", "added"], additionalProperties: true } },
          places: { type: "array", items: { type: "object", required: ["scope", "id", "name", "may"], additionalProperties: true } },
          admin: { type: "boolean" },
          failing: { type: "array", items: { type: "string" }, description: "the apps with a schedule, that you may switch, that failed its last run" },
        },
      },
      async run(ctx) {
        // The Cloudflare node doesn't know which schedules failed.
        const failing = await ctx.failing().catch(() => []);
        return { apps: (await listed(ctx, { full: true })).apps, places: places(ctx), admin: ctx.me.role === "admin", failing: [...new Set(failing.map((s) => s.app))] };
      },
    },
    _health: {
      title: "How an app is doing, times as milliseconds (for the screens)",
      reads: true,
      description: "For the screens: health, with times as milliseconds since 1970.",
      input: { type: "object", required: ["app"], properties: { app: APP } },
      output: HEALTH,
      async run(ctx, { app }) {
        const a = await find(ctx, app);
        return { schedules: a.schedules || [], activity: a.activity || [], insights: a.insights || null };
      },
    },
    switch: {
      title: "Switch a schedule on or off",
      reads: false,
      description: "Switch one of an app's schedules on or off in a place where it's added (user, a group by its name, or global); switched on, it runs as you.",
      input: { type: "object", required: ["app", "schedule", "scope", "on"], properties: { app: APP, schedule: { type: "string", description: "the schedule's name, from apps_health's schedules" }, scope: placeInput, on: { type: "boolean", description: "true to switch it on, false off" } } },
      output: { type: "string", description: "whether the schedule is on, in words" },
      async run(ctx, { app, schedule, scope, on }) {
        ctx.schedule(app, schedule, placeOf(ctx, scope), on);
        return `${schedule} is ${on ? "on" : "off"}.`;
      },
    },
    devices: {
      title: "Where an app's service runs",
      reads: true,
      description: "Where an app's service runs (runs_on: each node, whether it's on, and what went wrong), the nodes it can run on (nodes, for add and apps_move), and its problem when it runs on no node that's on.",
      input: { type: "object", required: ["app"], properties: { app: { type: "string", description: "the name of an app with a service, like whatsapp" } } },
      output: {
        type: ["object", "null"],
        description: "null for an app with no service",
        properties: {
          runs_on: { type: "array", items: { type: "object", additionalProperties: true }, description: "each node it runs on, { name, scope, online, problem }" },
          nodes: { type: "array", items: { type: "string" }, description: "the nodes it can run on" },
          problem: { type: ["object", "null"], description: "{ title, text, move } when it runs on no node that's on", additionalProperties: true },
        },
      },
      async run(ctx, { app }) {
        return where(await find(ctx, app));
      },
    },
    code: {
      title: "An app's code",
      reads: true,
      description: "An app's code as deployed: its files and their sizes, or with path one of them as text. Everyone may read every app's code: start a new app from one that's close.",
      input: { type: "object", required: ["app"], properties: { app: APP, path: { type: "string", description: "a file's path, from apps_code without path, like app.js or src/app.js" } } },
      output: {
        type: "object",
        description: "{ files } without path, { path, text } with it",
        properties: {
          files: { type: "array", items: { type: "object", required: ["path", "size"], properties: { path: { type: "string" }, size: { type: "integer" } } } },
          path: { type: "string" },
          text: { type: "string" },
          app: { type: "string" },
          bundle: { type: "string", description: "the deploy it is of" },
        },
      },
      async run(ctx, { app, path }) {
        await find(ctx, app);
        return ctx.code(app, path);
      },
    },
  },

  screens: {
    main: { title: "App Store", html: mainHtml },
    app: { title: "App", html: appHtml },
    source: { title: "Source Link", html: sourceHtml },
    activity: { title: "Activity", html: activityHtml },
    code: { title: "Code", html: codeHtml },
    file: { title: "Code", html: fileHtml },
  },
};
