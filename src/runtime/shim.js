// Every app runs inside this shim, in workerd. The app module (./app.js) is the app's deployed code; this turns it
// into a Worker with a store per scope (a Durable Object holding SQLite) that answers the node: replays, operations,
// widgets, hooks and screens. See SPEC.md, "Apps".
import app from "./app.js";
import { DurableObject } from "cloudflare:workers";
import { AsyncLocalStorage } from "node:async_hooks";

// Every request goes out through the node (the Worker's global outbound), carrying the token of the call it's made
// in, so the node knows whose it is: it fills in that call's secrets, and refuses them to other hosts.
const calls = new AsyncLocalStorage();
const fetchOut = globalThis.fetch;
globalThis.fetch = (input, init) => {
  const req = new Request(input, init);
  const token = calls.getStore();
  if (token) req.headers.set("x-one-call", token);
  return fetchOut(req);
};

const json = (v, status = 200) => new Response(JSON.stringify(v ?? null), { status, headers: { "content-type": "application/json" } });

function describe() {
  return {
    name: app.name, title: app.title, icon: app.icon, color: app.color, domain: app.domain, description: app.description || "", instructions: app.instructions || "",
    ...(app.connector ? { connector: true } : {}), ...(app.needs ? { needs: String(app.needs) } : {}),
    ops: Object.entries(app.ops || {}).map(([name, o]) => ({ name, description: o.description || "", input: o.input || {}, ...(o.reads ? { reads: true } : {}), ...(o.confirm ? { confirm: true } : {}) })),
    widgets: Object.entries(app.widgets || {}).map(([name, w]) => {
      const size = w.size || "medium";
      const options = Object.entries(w.options || {}).map(([name, o]) => ({ name, title: o.title || name, default: o.default ?? null }));
      return { name, title: w.title || name, description: w.description || "", size, sizes: w.sizes?.length ? w.sizes : [size], ...(options.length ? { options } : {}) };
    }),
    badge: typeof app.badge === "function",
    screens: Object.entries(app.screens || {}).map(([name, s]) => ({ name, title: s.title || name })),
    uses: { apps: app.uses?.apps || [], drivers: app.uses?.drivers || [] },
    schedules: Object.entries(app.schedules || {}).map(([name, s]) => ({ name, ...s })),
  };
}

/// The SQL that makes an empty store of a scope: `user`, `group` or `global`.
function schemaFor(scope) {
  const kind = scope.split(":")[0];
  return app.schema?.[kind] ?? null;
}

export class Store extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec("create table if not exists _one (key text primary key, value text)");
  }

  /// Empties the store and makes it again from the app's schema for its scope.
  reset(scope) {
    const tables = this.sql.exec("select name from sqlite_master where type = 'table' and name not like '\\_%' escape '\\'").toArray();
    for (const { name } of tables) this.sql.exec(`drop table "${name.replaceAll('"', '""')}"`);
    const schema = schemaFor(scope);
    if (schema) this.sql.exec(schema);
    this.sql.exec("insert or replace into _one values ('scope', ?)", scope);
  }

  ready(scope) {
    if (!this.sql.exec("select 1 from _one where key = 'scope'").toArray().length) this.reset(scope);
  }

  query(scope, q, params = []) {
    this.ready(scope);
    if (!/^\s*(select|with)\b/i.test(q)) throw new Error("query only reads; `on` handlers write with db.exec");
    return this.sql.exec(q, ...params).toArray();
  }

  /// Replays entries into the store. The `on` handlers see no clock and no randomness, so they make the same store on
  /// every server.
  replay(scope, fresh, entries) {
    if (fresh) this.reset(scope);
    else this.ready(scope);
    if (!schemaFor(scope)) return;
    const db = { exec: (q, params = []) => void this.sql.exec(q, ...params), query: (q, params) => this.query(scope, q, params) };
    const saved = [Math.random, Date.now];
    Math.random = () => { throw new Error("on handlers have no randomness: they must make the same store everywhere"); };
    Date.now = () => { throw new Error("on handlers read the time from entry.time"); };
    try {
      for (const e of entries) {
        try {
          app.on?.[e.kind]?.(db, e);
        } catch (err) {
          console.warn(`${app.name} on ${e.kind} failed: ${err.message}`);
        }
      }
    } finally {
      [Math.random, Date.now] = saved;
    }
  }

  /// Deletes the store: its server no longer holds its scope.
  async drop() {
    await this.ctx.storage.deleteAll();
  }
}

/// Asks the node something for the call with `token`: other apps' and drivers' operations, blobs, the install's apps.
async function askNode(token, path, init = {}) {
  const r = await fetchOut(new Request(`http://one.internal/${path}`, { ...init, headers: { "x-one-call": token } }));
  if (!r.ok) throw new Error(await r.text());
  return r;
}

/// After a call returned: waits for the work it left running (ctx.waitUntil), hands the node what that work appended,
/// and says it's done.
async function finish(token, later, appends) {
  let error = null;
  for (let i = 0; i < later.length; i++) {
    try {
      await later[i];
    } catch (err) {
      error ||= String(err?.message ?? err);
      console.warn(`${app.name}: work after a call failed: ${error}`);
    }
  }
  try {
    if (appends.length) await askNode(token, "appends", { method: "POST", body: JSON.stringify({ appends: appends.splice(0) }) });
  } catch (err) {
    error ||= String(err?.message ?? err);
  }
  await askNode(token, "done", { method: "POST", body: JSON.stringify({ error }) }).catch(() => {});
}

/// A widget's options' defaults, as its render sees them when a person chose none.
function optionDefaults(widget) {
  return Object.fromEntries(Object.entries(widget?.options || {}).map(([name, o]) => [name, o.default ?? null]));
}

/// A widget's options for a shell to offer: each one's choices (`{ value, title }`), worked out from the data where
/// the app says how; one with none and a true or false default is a switch.
async function widgetOptions(widget, ctx) {
  const out = [];
  for (const [name, o] of Object.entries(widget?.options || {})) {
    const raw = typeof o.choices === "function" ? await o.choices(ctx) : o.choices;
    const choices = Array.isArray(raw) ? raw.map((c) => (typeof c === "object" && c ? { value: c.value ?? null, title: String(c.title ?? c.value) } : { value: c, title: String(c) })) : undefined;
    out.push({ name, title: o.title || name, default: o.default ?? null, ...(choices ? { choices } : {}) });
  }
  return out;
}

/// What a call, widget or hook gets: the caller, and a handle on each scope the node lets it reach.
function context(env, body, appends, later) {
  const handle = (s) => ({
    id: s.id, name: s.name, role: s.role,
    query: (q, params = []) => env.STORES.get(env.STORES.idFromName(s.id)).query(s.id, q, params),
    append: (kind, data, blobs = []) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ scope: s.id, kind, data: data ?? null, blobs });
    },
    /// Says this scope no longer needs a blob its entries named: once no scope does, servers delete it.
    forget: (hash) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ forget: String(hash), scope: s.id });
    },
    /// Sets one of this app's secrets in this scope (null deletes it): never read back, only sent to `hosts`.
    secret: (name, value, hosts = []) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ secret: name, scope: s.id, value: value ?? null, hosts });
    },
  });
  const scopes = body.scopes.map(handle);
  // What the node does for a call: other apps' and drivers' operations, blobs, the install's apps.
  const ask = (path, init) => askNode(body.token, path, init);
  // `ctx.apps.<name>.<op>(input, { server })`, for exactly the apps (or drivers) the app says it uses; `server` (a
  // name) runs it there, like a driver on the machine that has what it needs.
  const used = (kind) => new Proxy({}, {
    get(_, name) {
      if (typeof name !== "string" || name === "then") return undefined;
      if (!(app.uses?.[kind] || []).includes(name)) throw new Error(`${app.name} doesn't use ${name}: add it to uses.${kind}`);
      return new Proxy({}, {
        get(_, op) {
          if (typeof op !== "string" || op === "then") return undefined;
          return async (input = {}, { server } = {}) => (await (await ask("call", { method: "POST", body: JSON.stringify({ app: name, driver: kind === "drivers", op, input, server }) })).json()).ok;
        },
      });
    },
  });
  const find = (id) => scopes.find((s) => s.id === id) || null;
  return {
    me: body.me,
    /// The app whose ctx.apps made this call, if one did.
    from: body.from ?? null,
    /// Says what this call spent on AI, { model, usd, input, output }: it shows in the insights of the app that called
    /// it (ctx.from), or else of this one.
    cost: (spent) => {
      if (body.read_only) return;
      appends.push({ cost: { model: String(spent?.model ?? "?"), usd: Number(spent?.usd) || 0, input: Number(spent?.input) || 0, output: Number(spent?.output) || 0 } });
    },
    /// Keeps work going after the call returns (an AI read, a copy to Drive), up to 10 minutes: its requests and
    /// calls still go out as this call's, and what it appends is written when it's done.
    waitUntil: (promise) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      later.push(Promise.resolve(promise));
    },
    now: () => body.now,
    user: scopes.find((s) => s.id.startsWith("user:")) || null,
    global: find("global"),
    groups: scopes.filter((s) => s.id.startsWith("group:")),
    group: (id) => find(id.startsWith("group:") ? id : `group:${id}`),
    apps: used("apps"),
    drivers: used("drivers"),
    blob: {
      put: async (bytes) => (await (await ask("blob", { method: "POST", body: bytes })).json()).blob,
      get: async (hash) => new Uint8Array(await (await ask(`blob/${hash}`)).arrayBuffer()),
    },
    /// The Vault, as this app may use it: the items it's switched on for (a secret field as { secret: true }, never its
    /// value), and making, changing and letting go of items. A field goes into a request as {{<item id>:<field>}}.
    vault: {
      items: async () => (await ask("vault")).json(),
      /// Makes an item switched on for this app ({ scope?, title, type?, domain?, fields: { name: { value, secret? } },
      /// hosts }), in the caller's scope by default; with `same`, a plain field's name, changes the item of that type
      /// whose field has that value instead (every secret field given anew). Gives its id.
      async create({ scope = "user", same, ...item }) {
        if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
        const at = scope === "user" ? scopes.find((s) => s.id.startsWith("user:"))?.id : scope;
        if (!at) throw new Error("this server doesn't keep your data: use one of your own servers");
        let id = null;
        if (same) id = (await (await ask("vault/match", { method: "POST", body: JSON.stringify({ scope: at, type: item.type ?? "", field: same, value: item.fields?.[same]?.value }) })).json()).id;
        id ||= [...crypto.getRandomValues(new Uint8Array(8))].map((b) => b.toString(16).padStart(2, "0")).join("");
        appends.push({ vault: id, scope: at, item, same: same ?? null });
        return id;
      },
      /// Changes an item it's switched on for: { title?, type?, domain?, fields?, hosts? }, its hosts only with every
      /// secret field given anew.
      update: (id, changes) => {
        if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
        appends.push({ vault: String(id), item: changes ?? {} });
      },
      /// Switches this app off for an item (signing out an account); the item goes once no app uses it.
      release: (id) => {
        if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
        appends.push({ vault: String(id), release: true });
      },
    },
    /// Pushes a notification to the caller's phones, { title, body?, screen?, params? }; tapping it opens this app's
    /// screen. The install delivers it, if its push setting is on: { sent: false, why } when it's off.
    notify: async (push) => {
      if (body.read_only) throw new Error("this call only reads: it can't push");
      return (await (await ask("notify", { method: "POST", body: JSON.stringify(push) })).json()).ok;
    },
    /// Every app in the install, as `GET /v1/apps` gives them to the caller.
    catalog: async () => (await ask("catalog")).json(),
    /// The install's servers, as `GET /v1/state` gives them to the caller: online or not, and each one's load.
    servers: async () => (await ask("servers")).json(),
    /// Installs (or with `installed: false`, uninstalls) an app in a scope, with the caller's own rights there.
    install: (name, scope, installed = true) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ install: name, scope, installed });
    },
    /// Deletes an app ("deleted": to Recently Deleted), restores it ("restored") or erases it for good ("erased"),
    /// for someone acting for its owner.
    retire: (name, how) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ retire: name, how });
    },
    /// Removes a driver from a server (by name), or with installed lets it register there again, for someone acting
    /// for one of the server's owners.
    driver: (name, server, installed = false) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ driver: name, server, installed });
    },
    /// Switches one of an app's schedules on or off in a scope, with the caller's own rights there.
    schedule: (name, schedule, scope, on = true) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ switch: schedule, app: name, scope, on });
    },
  };
}

export default {
  async fetch(req, env, execution) {
    const url = new URL(req.url);
    const what = url.pathname.split("/").pop();
    if (what === "describe") return json({ ok: describe() });
    if (what === "screen") {
      const screen = app.screens?.[url.searchParams.get("name")];
      return json({ ok: screen ? screen.html : null });
    }
    const body = await req.json();
    try {
      if (what === "replay") {
        await env.STORES.get(env.STORES.idFromName(body.scope)).replay(body.scope, body.fresh, body.entries);
        return json({ ok: null });
      }
      if (what === "drop") {
        await env.STORES.get(env.STORES.idFromName(body.scope)).drop();
        return json({ ok: null });
      }
      const appends = [];
      const later = [];
      const ctx = context(env, body, appends, later);
      let run;
      if (what === "call") {
        const op = app.ops?.[body.op];
        if (!op) throw new Error(`${app.name} has no operation ${body.op}`);
        run = () => op.run(ctx, body.input ?? {});
      } else if (what === "widget") {
        const widget = app.widgets[body.name];
        ctx.size = body.size || widget.size || "medium";
        ctx.options = { ...optionDefaults(widget), ...(body.options || {}) };
        run = () => widget.render(ctx);
      } else if (what === "options") {
        run = () => widgetOptions(app.widgets[body.name], ctx);
      } else if (what === "badge") {
        run = () => app.badge(ctx);
      } else if (what === "hook") {
        const hook = app.hooks?.[body.name];
        if (!hook) throw new Error(`${app.name} has no hook ${body.name}`);
        run = () => hook(ctx, body.input);
      } else {
        throw new Error(`no such request ${what}`);
      }
      const result = await calls.run(body.token, run);
      const keep = what === "call" && later.length > 0;
      const reply = json({ ok: result, appends: what === "call" ? appends.splice(0) : [], later: keep });
      if (keep) execution.waitUntil(calls.run(body.token, () => finish(body.token, later, appends)));
      return reply;
    } catch (err) {
      return json({ error: String(err?.message ?? err) }, 400);
    }
  },
};
