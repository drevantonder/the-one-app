// Telemetry: where a network says how it's doing and reads it back. Spans, logs and metrics, in OpenTelemetry's names, go
// into Cloudflare Basin, written through its pipelines' bindings (`SPANS`, `LOGS`, `METRICS`), which need no token, and
// read by HTTP SQL (`SQL`, with `SQL_TOKEN`), which has no binding. A sink's table can't change once made, so its columns
// (`TABLES`) are few and stable and everything else is in `attrs`, a JSON string.
//
// It opens nothing: what's here is operational and unsealed, each value a closed-set name or a number but for a failure's
// cleaned message (src/insights/message.rs).
//
//   POST /telemetry/write { spans, logs, metrics, at, by, sig } → 204
//   POST /telemetry/read  { queries: [name], since, services, service?, name?, status?, trace?, limit?, at, by, sig }
//                         → { <name>: [row] }
//
// Each is a request signed by a key on the relay's list (`one/telemetry`). A row that isn't what its table holds is left
// out here, as Basin would take it and drop it in silence. A read runs only `QUERIES`, each value a number or a token
// (`word`), which is what makes them safe to put in SQL: Basin's SQL has no parameters.
import { checkRequest, rolesOf } from "../shared/crypto.js";
import { Refused, refusal, refuse, textOf } from "../shared/http.js";
import { TELEMETRY } from "../shared/protocol.js";

/// What Basin's own limit on a request (5 MB) leaves room for.
const MOST_BYTES = 4 << 20;
const MOST_ROWS = 5000;
const MOST_ATTRS = 16 * 1024;
const MOST_SERVICES = 1000;
/// Basin's SQL refuses a greater LIMIT.
const MOST_READ = 10_000;
const ERRORS_READ = 500;
const PAST = 8 * 86_400_000;
const FUTURE = 300_000;
const ALLOWED_FOR = 5_000;
/// OTEL's default duration boundaries, in ms: bucket `i` is what's over boundary `i - 1` and at most boundary `i`, and the last what's over them all.
const BOUNDS = [0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000];
const NAMESPACE = "telemetry";

const word = (v) => typeof v === "string" && /^[A-Za-z0-9_.:+\/-]{1,128}$/.test(v);
const hex = (v, n) => typeof v === "string" && v.length === n && /^[0-9a-f]+$/.test(v);
const count = (v) => Number.isSafeInteger(v) && v >= 0;
const quoted = (v) => `'${v}'`;

const asTime = (v) => count(v) && Math.abs(v - Date.now()) <= PAST;
const severity = (v) => Number.isSafeInteger(v) && v >= 0 && v <= 24;
const text = (most) => (v) => typeof v === "string" && v.length <= most;
const json = (v) => typeof v === "string" && v.length <= MOST_ATTRS;
const number = (v) => typeof v === "number" && Number.isFinite(v);

/// Each table's columns, a check each and whether a row has to have it: the schema `src/cloudflare/basin.rs` makes its stream with, held to it by tests/contracts.rs.
export const TABLES = {
  spans: { time: [asTime, 1], trace: [(v) => hex(v, 32), 1], span: [(v) => hex(v, 16), 1], parent: [(v) => hex(v, 16)], name: [word, 1], service: [word, 1], version: [word], instance: [(v) => hex(v, 64), 1], kind: [word, 1], status: [word, 1], duration: [count, 1], message: [text(512)], attrs: [json, 1] },
  logs: { time: [asTime, 1], trace: [(v) => hex(v, 32)], span: [(v) => hex(v, 16)], severity_number: [severity, 1], severity_text: [word, 1], body: [word, 1], service: [word, 1], version: [word], instance: [(v) => hex(v, 64), 1], attrs: [json, 1] },
  metrics: { time: [asTime, 1], name: [word, 1], type: [word, 1], unit: [text(32), 1], service: [word, 1], instance: [(v) => hex(v, 64), 1], count: [count, 1], sum: [number, 1], buckets: [json], attrs: [json, 1] },
};

/// `row` as its table holds it, or null. A device names only itself; a node also what its clients reported through it.
function kept(row, columns, by, relays) {
  if (!row || typeof row !== "object") return null;
  const out = {};
  for (const [name, [ok, needed]] of Object.entries(columns)) {
    const value = row[name];
    if (value === undefined || value === null) {
      if (needed) return null;
    } else if (!ok(value)) {
      return null;
    } else {
      out[name] = value;
    }
  }
  if (out.instance !== by && !relays) out.instance = by;
  return out;
}

const known = new Map();

async function who(env, by) {
  const now = Date.now();
  const had = known.get(by);
  if (had && now - had.at < ALLOWED_FOR) return had.said;
  const said = (await env.RELAY.allowed(by)) || null;
  if (known.size > 10_000) known.clear();
  known.set(by, { at: now, said });
  return said;
}

async function signed(request, env, most) {
  const body = await textOf(request, most);
  if (body === null) refuse(413, "that's too big");
  let checked;
  try {
    checked = await checkRequest(TELEMETRY, JSON.parse(body));
  } catch (err) {
    refuse(401, err.message);
  }
  const device = await who(env, checked.by);
  if (!device) refuse(403, `only ${env.YOURS}'s devices reach its telemetry`);
  return { ...checked, device };
}

const STREAMS = { spans: "SPANS", logs: "LOGS", metrics: "METRICS" };

async function write(request, env) {
  const { by, fields, device } = await signed(request, env, MOST_BYTES);
  const relays = rolesOf(device.role).includes("compute");
  await Promise.all(Object.entries(STREAMS).map(async ([table, stream]) => {
    const rows = (Array.isArray(fields[table]) ? fields[table] : []).slice(0, MOST_ROWS).map((r) => kept(r, TABLES[table], by, relays)).filter(Boolean);
    if (rows.length) await env[stream].send(rows);
  }));
  return new Response(null, { status: 204 });
}

const attr = (key) => `json_get_str(attrs, '${key}')`;
const buckets = () => BOUNDS.map((high, i) => `SUM(CASE WHEN "duration" <= ${high}${i ? ` AND "duration" > ${BOUNDS[i - 1]}` : ""} THEN 1 ELSE 0 END) AS b${i}`).concat(`SUM(CASE WHEN "duration" > ${BOUNDS.at(-1)} THEN 1 ELSE 0 END) AS b${BOUNDS.length}`).join(", ");
const summed = () => [...BOUNDS, null].map((_, i) => `SUM(json_get_int(buckets, ${i})) AS b${i}`).join(", ");
const hour = `("time" / 3600000)`;
const asBuckets = (rows) => rows.map((row) => {
  const out = {};
  const counts = [];
  for (const [k, v] of Object.entries(row)) {
    const b = /^b(\d+)$/.exec(k);
    if (b) counts[Number(b[1])] = Number(v);
    else out[k] = v;
  }
  return { ...out, buckets: counts };
});

// A filter of a table, by what a read asks: `since` and which services always, and the rest as given.
function where(a, more = []) {
  const parts = [`"time" >= ${a.since}`, `service IN (${a.services.map(quoted).join(", ")})`, ...more];
  if (a.service) parts.push(`service = ${quoted(a.service)}`);
  if (a.name) parts.push(`${a.named || "name"} = ${quoted(a.name)}`);
  if (a.trace) parts.push(`trace = ${quoted(a.trace)}`);
  return `WHERE ${parts.join(" AND ")}`;
}

const newest = (table, columns, clause, limit) => `SELECT ${columns} FROM ${NAMESPACE}.${table} ${clause} ORDER BY "time" DESC LIMIT ${limit}`;
const SPAN = `"time", trace, span, parent, name, service, version, instance, kind, status, "duration", message, attrs`;
const LOG = `"time", trace, span, severity_number, severity_text, body, service, version, instance, attrs`;
const errorOnly = (a, key) => (a.status === "error" ? [key] : a.status === "unset" ? [`NOT (${key})`] : []);

/// The reads there are, each from what's asked to its SQL and what it gives.
const QUERIES = {
  // Every request, an hour at a time: what rates and durations add up from.
  requests: { sql: (a) => `SELECT ${hour} AS hour, service, name, COUNT(*) AS "count", SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END) AS errors, SUM("duration") AS "sum", ${buckets()} FROM ${NAMESPACE}.spans ${where(a, [`kind = 'server'`, ...errorOnly(a, `status = 'error'`)])} GROUP BY ${hour}, service, name ORDER BY hour DESC LIMIT ${a.limit}`, rows: asBuckets },
  spans: { sql: (a) => newest("spans", SPAN, where(a, errorOnly(a, `status = 'error'`)), a.limit) },
  logs: { sql: (a) => newest("logs", LOG, where({ ...a, named: "body" }, errorOnly(a, `severity_number >= 17`)), a.limit) },
  metrics: { sql: (a) => `SELECT ${hour} AS hour, name, "type", unit, service, instance, attrs, SUM("count") AS "count", SUM("sum") AS "sum", ${summed()} FROM ${NAMESPACE}.metrics ${where(a)} GROUP BY ${hour}, name, "type", unit, service, instance, attrs ORDER BY hour DESC LIMIT ${a.limit}`, rows: asBuckets },
  // What broke, as often as it did: its type, the frame that threw it and the build it ran in, in that one attribute.
  errors: { sql: (a) => `SELECT service, name, status, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", MIN("time") AS first_seen, MAX("time") AS last_seen, MIN(message) AS message FROM ${NAMESPACE}.spans ${where(a, [`${attr("one.fingerprint")} IS NOT NULL`, ...(a.status === "error" || a.status === "unset" ? [`status = ${quoted(a.status)}`] : [])])} GROUP BY service, name, status, ${attr("one.fingerprint")} ORDER BY last_seen DESC LIMIT ${Math.min(a.limit, ERRORS_READ)}` },
};

function asked(fields) {
  const a = { since: fields.since ?? 0, services: fields.services, limit: Math.min(fields.limit ?? 100, MOST_READ) };
  if (!count(a.since) || !count(a.limit) || !a.limit) refuse(400, "since and limit are numbers");
  if (!Array.isArray(a.services) || a.services.length > MOST_SERVICES || !a.services.every(word)) refuse(400, "services are names");
  for (const key of ["service", "name", "status", "trace"]) {
    const given = fields[key];
    if (given !== undefined && given !== null && !word(given)) refuse(400, `${key} is a name`);
    if (given) a[key] = given;
  }
  return a;
}

/// `sql` run by Basin SQL. It refuses a default User-Agent with error 1010.
async function query(env, sql) {
  const said = await fetch(env.SQL, { method: "POST", headers: { authorization: `Bearer ${env.SQL_TOKEN}`, "content-type": "application/json", "user-agent": `${env.CLI}-telemetry` }, body: JSON.stringify({ query: sql }) });
  const body = await said.json().catch(() => ({}));
  if (!said.ok || body.success !== true) refuse(502, `the store said ${said.status}: ${(body.errors || []).map((e) => e.message).join("; ")}`);
  return body.result.rows;
}

async function read(request, env) {
  const { fields } = await signed(request, env, 64 * 1024);
  const a = asked(fields);
  const names = Array.isArray(fields.queries) ? fields.queries.filter((n) => Object.hasOwn(QUERIES, n)) : [];
  if (!a.services.length) return Response.json(Object.fromEntries(names.map((n) => [n, []])));
  const results = await Promise.all(names.map(async (name) => {
    const { sql, rows = (r) => r } = QUERIES[name];
    return [name, rows(await query(env, sql(a)))];
  }));
  return Response.json(Object.fromEntries(results));
}

const ROUTES = { "/telemetry/write": write, "/telemetry/read": read };

export default {
  async fetch(request, env) {
    const route = ROUTES[new URL(request.url).pathname];
    if (!route || request.method !== "POST") return refusal(404, "telemetry answers only POST /telemetry/write and /telemetry/read");
    try {
      return await route(request, env);
    } catch (err) {
      if (err instanceof Refused) return refusal(err.status, err.message);
      console.error(err.stack || err);
      return refusal(500, "telemetry failed");
    }
  },
};
