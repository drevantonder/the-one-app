// Every app runs inside this shim, in workerd. The app module (./app.js) is the app's code; this turns
// it into a Worker whose Durable Object, one per space, holds the app's views in SQLite and answers
// the node: folds, operations, widgets and hooks. See SPEC.md, "Apps".
import app from "./app.js";
import { DurableObject } from "cloudflare:workers";

const json = (v, status = 200) => new Response(JSON.stringify(v ?? null), { status, headers: { "content-type": "application/json" } });

function describe() {
  return {
    name: app.name, title: app.title, icon: app.icon, instructions: app.instructions || "",
    ops: Object.entries(app.ops || {}).map(([name, o]) => ({ name, description: o.description || "", input: o.input || {} })),
    widgets: Object.entries(app.widgets || {}).map(([name, w]) => ({ name, title: w.title || name })),
    screens: Object.entries(app.screens || {}).map(([name, s]) => ({ name, title: s.title || name })),
  };
}

export class Views extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec("create table if not exists _one (key text primary key, value text)");
    if (!this.sql.exec("select 1 from _one where key = 'schema'").toArray().length) this.reset();
  }

  /// Empties the views and makes them again from the app's schema.
  reset() {
    const tables = this.sql.exec("select name from sqlite_master where type = 'table' and name not like '\\_%' escape '\\'").toArray();
    for (const { name } of tables) this.sql.exec(`drop table "${name.replaceAll('"', '""')}"`);
    if (app.schema) this.sql.exec(app.schema);
    this.sql.exec("insert or replace into _one values ('schema', '1')");
  }

  query(q, params = []) {
    if (!/^\s*(select|with)\b/i.test(q)) throw new Error("query only reads; fold handlers write with db.exec");
    return this.sql.exec(q, ...params).toArray();
  }

  /// A fold sees no clock and no randomness, so it gives the same views on every device.
  fold(fresh, entries) {
    if (fresh) this.reset();
    const db = { exec: (q, params = []) => void this.sql.exec(q, ...params), query: (q, params) => this.query(q, params) };
    const saved = [Math.random, Date.now];
    Math.random = () => { throw new Error("folds have no randomness: they must give the same views everywhere"); };
    Date.now = () => { throw new Error("folds read the time from entry.time"); };
    try {
      for (const e of entries) {
        try {
          app.fold?.[e.kind]?.(db, e);
        } catch (err) {
          console.warn(`${app.name} fold of ${e.kind} failed: ${err.message}`);
        }
      }
    } finally {
      [Math.random, Date.now] = saved;
    }
  }

  async fetch(req) {
    const what = new URL(req.url).pathname.split("/").pop();
    const body = await req.json();
    try {
      if (what === "fold") {
        this.fold(body.fresh, body.entries);
        return json({ ok: null });
      }
      const appends = [];
      const ctx = {
        me: body.me,
        now: () => body.now,
        query: (q, params) => this.query(q, params),
        append: (kind, data) => void appends.push({ kind, data: data ?? null }),
      };
      let result;
      if (what === "call") {
        const op = app.ops?.[body.op];
        if (!op) throw new Error(`${app.name} has no operation ${body.op}`);
        result = await op.run(ctx, body.input ?? {});
      } else if (what === "widget") {
        result = await app.widgets[body.name].render(ctx);
      } else if (what === "hook") {
        const hook = app.hooks?.[body.name];
        if (!hook) throw new Error(`${app.name} has no hook ${body.name}`);
        result = await hook(ctx, body.input);
      } else {
        throw new Error(`no such request ${what}`);
      }
      return json({ ok: result, appends });
    } catch (err) {
      return json({ error: String(err?.message ?? err) }, 400);
    }
  }
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
    const space = url.searchParams.get("space");
    if (!space) return json({ error: "which space?" }, 400);
    return env.VIEWS.get(env.VIEWS.idFromName(space)).fetch(req);
  },
};
