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
    name: app.name, title: app.title, icon: app.icon, instructions: app.instructions || "",
    ops: Object.entries(app.ops || {}).map(([name, o]) => ({ name, description: o.description || "", input: o.input || {} })),
    widgets: Object.entries(app.widgets || {}).map(([name, w]) => ({ name, title: w.title || name })),
    screens: Object.entries(app.screens || {}).map(([name, s]) => ({ name, title: s.title || name })),
    uses: { apps: app.uses?.apps || [], drivers: app.uses?.drivers || [] },
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

/// What a call, widget or hook gets: the caller, and a handle on each scope the node lets it reach.
function context(env, body, appends) {
  const handle = (s) => ({
    id: s.id, name: s.name, role: s.role,
    query: (q, params = []) => env.STORES.get(env.STORES.idFromName(s.id)).query(s.id, q, params),
    append: (kind, data, blobs = []) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ scope: s.id, kind, data: data ?? null, blobs });
    },
    /// Sets one of this app's secrets in this scope (null deletes it): never read back, only sent to `hosts`.
    secret: (name, value, hosts = []) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ secret: name, scope: s.id, value: value ?? null, hosts });
    },
  });
  const scopes = body.scopes.map(handle);
  // What the node does for a call: other apps' and drivers' operations, blobs, the install's apps.
  const ask = async (path, init = {}) => {
    const r = await fetchOut(new Request(`http://one.internal/${path}`, { ...init, headers: { "x-one-call": body.token } }));
    if (!r.ok) throw new Error(await r.text());
    return r;
  };
  // `ctx.apps.<name>.<op>(input)`, for exactly the apps (or drivers) the app says it uses.
  const used = (kind) => new Proxy({}, {
    get(_, name) {
      if (typeof name !== "string" || name === "then") return undefined;
      if (!(app.uses?.[kind] || []).includes(name)) throw new Error(`${app.name} doesn't use ${name}: add it to uses.${kind}`);
      return new Proxy({}, {
        get(_, op) {
          if (typeof op !== "string" || op === "then") return undefined;
          return async (input = {}) => (await (await ask("call", { method: "POST", body: JSON.stringify({ app: name, op, input }) })).json()).ok;
        },
      });
    },
  });
  const find = (id) => scopes.find((s) => s.id === id) || null;
  return {
    me: body.me,
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
    /// Every app in the install, as `GET /v1/apps` gives them to the caller.
    catalog: async () => (await ask("catalog")).json(),
    /// Installs (or with `installed: false`, uninstalls) an app in a scope, with the caller's own rights there.
    install: (name, scope, installed = true) => {
      if (body.read_only) throw new Error("this call only reads: it runs on data opened for it");
      appends.push({ install: name, scope, installed });
    },
  };
}

export default {
  async fetch(req, env) {
    const url = new URL(req.url);
    const what = url.pathname.split("/").pop();
    if (what === "describe") return json({ ok: describe() });
    if (what === "screen") {
      const screen = app.screens?.[url.searchParams.get("name")];
      return screen ? json({ ok: screen.html }) : json({ error: `${app.name} has no screen ${url.searchParams.get("name")}` }, 404);
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
      const ctx = context(env, body, appends);
      let run;
      if (what === "call") {
        const op = app.ops?.[body.op];
        if (!op) throw new Error(`${app.name} has no operation ${body.op}`);
        run = () => op.run(ctx, body.input ?? {});
      } else if (what === "widget") {
        run = () => app.widgets[body.name].render(ctx);
      } else if (what === "hook") {
        const hook = app.hooks?.[body.name];
        if (!hook) throw new Error(`${app.name} has no hook ${body.name}`);
        run = () => hook(ctx, body.input);
      } else {
        throw new Error(`no such request ${what}`);
      }
      const result = await calls.run(body.token, run);
      return json({ ok: result, appends: what === "call" ? appends : [] });
    } catch (err) {
      return json({ error: String(err?.message ?? err) }, 400);
    }
  },
};
