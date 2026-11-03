// The schedules service: work that runs while you're away. A person switches one of an app's schedules on in a scope
// (say, Morning summary, using Gmail). Each run is claimed with a conditional write, so it runs once. People see, and
// stop, what they've let run. Until standing work holds keys by a grant (a later piece), nothing is let run: the vault
// holds no one's keys to give a run.
//
// A Durable Object alarm wakes it for the next run, on Cloudflare and on any workerd alike.

import { DurableObject } from "cloudflare:workers";
import { bearer, Issuer } from "../shared/token.js";

const NAME = /^[a-z0-9][a-z0-9-]{0,63}$/;
const DAYS = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
/// The latest a run still happens after its time (or its period, if shorter).
const LATE = 3_600_000;

class Refused extends Error {
  constructor(status, message) {
    super(message);
    this.status = status;
  }
}
const refuse = (status, message) => {
  throw new Refused(status, message);
};
const json = (v, status = 200) => Response.json(v, { status });

/// When a schedule runs: `{ every: "15m" }`, or `{ at: "09:00", tz?, days? }`, as apps declare it.
export function when(spec) {
  if (typeof spec.every === "string") {
    const m = spec.every.trim().match(/^(\d+)([smhd])$/);
    if (!m) refuse(400, "every is like 15m, 6h or 1d");
    const ms = Number(m[1]) * { s: 1000, m: 60_000, h: 3_600_000, d: 86_400_000 }[m[2]];
    if (ms < 60_000) refuse(400, "a schedule runs at most once a minute");
    return { every: spec.every.trim(), ms };
  }
  const m = typeof spec.at === "string" && spec.at.match(/^(\d{1,2}):(\d{2})$/);
  if (!m || Number(m[1]) > 23 || Number(m[2]) > 59) refuse(400, "a schedule runs every so often (every: \"15m\") or at a time of day (at: \"09:00\")");
  const tz = spec.tz || "UTC";
  try {
    new Intl.DateTimeFormat("en", { timeZone: tz });
  } catch {
    refuse(400, `no time zone ${tz}`);
  }
  const days = (spec.days || []).map((d) => {
    const i = DAYS.findIndex((x) => String(d).toLowerCase().startsWith(x));
    if (i < 0) refuse(400, `${d} isn't a day of the week`);
    return i;
  });
  return { at: `${m[1].padStart(2, "0")}:${m[2]}`, hour: Number(m[1]), minute: Number(m[2]), tz, days };
}

/// The local date and offset from UTC in `tz` at the instant `t`.
function local(t, tz) {
  const parts = Object.fromEntries(new Intl.DateTimeFormat("en-US", { timeZone: tz, hourCycle: "h23", year: "numeric", month: "numeric", day: "numeric", hour: "numeric", minute: "numeric", second: "numeric" }).formatToParts(new Date(t)).map((p) => [p.type, p.value]));
  const asUtc = Date.UTC(+parts.year, +parts.month - 1, +parts.day, +parts.hour, +parts.minute, +parts.second);
  return { year: +parts.year, month: +parts.month, day: +parts.day, offset: asUtc - Math.floor(t / 1000) * 1000 };
}

/// The first time after `now` that `w` runs.
export function next(w, now) {
  if (w.ms) return (Math.floor(now / w.ms) + 1) * w.ms;
  const today = local(now, w.tz);
  for (let n = 0; n < 9; n++) {
    // Calendar days from today's date where it is, so a 23- or 25-hour day is still one day.
    const date = new Date(Date.UTC(today.year, today.month - 1, today.day + n));
    if (w.days.length && !w.days.includes((date.getUTCDay() + 6) % 7)) continue;
    const wall = Date.UTC(date.getUTCFullYear(), date.getUTCMonth(), date.getUTCDate(), w.hour, w.minute);
    // The wall time's instant, with the zone's offset the day before or after (at most one change between). A time
    // a change skips is moved on by it, and one it repeats is its first, as the node does it.
    const before = local(wall - 86_400_000, w.tz).offset, after = local(wall + 86_400_000, w.tz).offset;
    const fits = (o) => local(wall - o, w.tz).offset === o;
    const t = fits(before) && fits(after) ? Math.min(wall - before, wall - after) : fits(after) && !fits(before) ? wall - after : wall - before;
    if (t > now) return t;
  }
  return null;
}

export class Schedules extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`
      create table if not exists schedules (id text primary key, user text not null, app text not null, name text not null,
        scope text not null, title text not null, what text not null, spec text not null, op text not null, input text not null,
        node text not null, sealed blob not null, next integer, made integer not null);
      create table if not exists runs (schedule text not null, slot integer not null, at integer not null, status text not null,
        detail text, primary key (schedule, slot));
    `);
  }

  async fetch(request) {
    try {
      return await this.route(request, new URL(request.url));
    } catch (err) {
      if (err instanceof Refused) return json({ error: err.message }, err.status);
      console.error(err.stack || err);
      return json({ error: "the schedules service failed" }, 500);
    }
  }

  async route(request, url) {
    const token = bearer(request) || refuse(401, "bring an access token");
    this.issuer ||= new Issuer(this.env.ISSUER, () => this.env.AUTH.fetch(new Request(`${this.env.ISSUER}/jwks`)));
    const claims = await this.issuer.verify(token).catch((err) => refuse(401, `that access token won't do: ${err.message}`));
    // Only the install's own apps sign in for its address; a remote MCP client's token is for `<address>/mcp`.
    const address = this.env.ISSUER.replace(/\/api\/auth$/, "");
    if (![claims.aud].flat().includes(address)) refuse(401, `that sign-in isn't for ${address}`);
    const one = url.pathname.match(/^\/schedules\/([0-9a-f]{16})(\/run)?$/);
    if (request.method === "GET" && url.pathname === "/schedules") return json(this.list(claims.sub));
    if (request.method === "POST" && url.pathname === "/schedules") return json(await this.add(token, claims, await request.json()));
    if (one && request.method === "DELETE" && !one[2]) return json(await this.stop(claims.sub, one[1]));
    if (one && request.method === "POST" && one[2]) return json(await this.runNow(claims.sub, one[1]));
    refuse(404, "no such route");
  }

  /// Would let one of an app's schedules run in a scope while its person is away: `{ app, name, scope, title, what?,
  /// op, input?, every | at, tz?, days?, node }`, with the person's access token.
  async add(token, claims, body) {
    const { app, name, op, node } = body;
    if (!NAME.test(app || "") || !NAME.test(name || "")) refuse(400, "name the app and its schedule");
    if (typeof op !== "string" || !op) refuse(400, "name the op it runs");
    // A try install's kernel (`SIGN_IN` is "try", which only `one try` sets) runs beside its node, at 127.0.0.1.
    const reach = this.env.SIGN_IN === "try" ? /^(https:\/\/|http:\/\/127\.0\.0\.1[:/])/ : /^https:\/\//;
    if (typeof node !== "string" || !reach.test(node)) refuse(400, "name where it runs: the install's address, or a node's, over https");
    when(body);
    refuse(503, "Schedules can't run while you're away yet: standing work comes back as grants.");
  }

  /// What a person has let run: each with what it may use, when it runs next, and how its last run went.
  list(user) {
    const rows = this.sql.exec("select * from schedules where user = ? order by made", user).toArray();
    return {
      schedules: rows.map((r) => {
        const last = this.sql.exec("select slot, at, status, detail from runs where schedule = ? order by slot desc limit 1", r.id).toArray()[0] || null;
        return { id: r.id, app: r.app, name: r.name, scope: r.scope, title: r.title, what: r.what, ...JSON.parse(r.spec), op: r.op, node: r.node, next: r.next, last };
      }),
    };
  }

  /// Stops it.
  async stop(user, id) {
    this.mine(user, id);
    this.sql.exec("delete from schedules where id = ?", id);
    this.sql.exec("delete from runs where schedule = ?", id);
    return {};
  }

  mine(user, id) {
    const row = this.sql.exec("select * from schedules where id = ?", id).toArray()[0];
    if (!row || row.user !== user) refuse(404, "no such schedule");
    return row;
  }

  async runNow(user, id) {
    const row = this.mine(user, id);
    return { run: await this.fire(row, Date.now()) };
  }

  async wake() {
    const soonest = this.sql.exec("select min(next) t from schedules where next is not null").one().t;
    if (soonest) await this.ctx.storage.setAlarm(soonest);
    else await this.ctx.storage.deleteAlarm();
  }

  async alarm() {
    const now = Date.now();
    for (const row of this.sql.exec("select * from schedules where next <= ?", now).toArray()) {
      const w = when(JSON.parse(row.spec));
      if (now - row.next <= Math.min(LATE, w.ms || LATE)) await this.fire(row, row.next).catch((err) => console.error(err.stack || err));
      this.sql.exec("update schedules set next = ? where id = ?", next(w, now), row.id);
    }
    await this.wake();
  }

  /// Runs one slot of a schedule, once: the claim is a write that succeeds only for the first to make it.
  async fire(row, slot) {
    const run = `${row.id}/${slot}`;
    const claimed = this.sql.exec("insert into runs (schedule, slot, at, status) values (?, ?, ?, 'running') on conflict do nothing", row.id, slot, Date.now()).rowsWritten;
    if (!claimed) return { run, status: "already run" };
    const finish = (status, detail = null) => {
      this.sql.exec("update runs set status = ?, detail = ? where schedule = ? and slot = ?", status, detail, row.id, slot);
      this.sql.exec("delete from runs where schedule = ? and slot < (select min(slot) from (select slot from runs where schedule = ? order by slot desc limit 20))", row.id, row.id);
      return { run, status, detail };
    };
    return finish("failed", "nothing runs while you're away until standing work comes back as grants");
  }
}

export default {
  fetch(request, env) {
    return env.SCHEDULES.get(env.SCHEDULES.idFromName("schedules")).fetch(request);
  },
};
