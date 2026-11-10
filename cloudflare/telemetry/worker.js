// Telemetry: where a network says how it's doing and reads it back. Spans, logs and metrics, in OpenTelemetry's names, go
// into Cloudflare Basin, written through its pipelines' bindings (`SPANS`, `LOGS`, `METRICS`), which need no token, and
// read by HTTP SQL (`SQL`, with `SQL_TOKEN`), which has no binding. A sink's table can't change once made, so its columns
// (`TABLES`) are few and stable and everything else is in `attrs`, a JSON string.
//
// It opens nothing: what's here is operational and unsealed, each value a closed-set name or a number but for a failure's
// cleaned message (src/insights/message.rs).
//
//   POST /telemetry/write { spans, logs, metrics, at, by, sig } → 204
//   POST /telemetry/otlp  Cloudflare's export of the Workers' own traces (`otlp`), gzipped or not → 204
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
/// Cloudflare's export hands over ~5 MB of OTLP/JSON at a time, unpacked by the time it's read, and resends the same batch for as long as it's refused.
const MOST_EXPORT = 32 << 20;
const MOST_ROWS = 5000;
const MOST_ATTRS = 16 * 1024;
const MOST_SERVICES = 1000;
/// Basin's SQL refuses a greater LIMIT.
const MOST_READ = 10_000;
const ERRORS_READ = 500;
const PAST = 8 * 86_400_000;
const FUTURE = 300_000;
const ALLOWED_FOR = 5_000;
/// How long the same summary is answered from memory: Basin SQL takes 0.7–1.2 s whatever it's asked (measured 2026-11-10), and a
/// reading is a person or an agent looking again.
const SUMMARY_FOR = 30_000;
const MOST_SUMMARIES = 32;
/// OTEL's default duration boundaries, in ms: bucket `i` is what's over boundary `i - 1` and at most boundary `i`, and the last what's over them all.
export const BOUNDS = [0, 5, 10, 25, 50, 75, 100, 250, 500, 750, 1000, 2500, 5000, 7500, 10000];
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

/// The Workers whose own traces are kept: those that open nothing, never `mcp` or `node`, whatever a request says of itself,
/// and not this one, as an export of its traces would be the next thing it was handed (src/insights.rs).
export const TRACED = ["gateway", "auth", "relay", "signer", "push"];
const token = (v) => typeof v === "string" && /^[A-Za-z0-9_.+-]{1,72}$/.test(v);
const nanos = (v) => Number(BigInt(v ?? 0) / 1000000n);
const OTLP_PATH = "/telemetry/otlp";
/// Who it's from isn't a device, so a Worker's own spans have no device of their own.
const NOBODY = "0".repeat(64);
/// As a device's: a request always, and the rest of a trace if it failed, was slow or is one in 20 (src/insights.rs).
const SLOW_MS = 1000;
const SAMPLED_ONE_IN = 20;
/// What a Worker answers as a request. Its other events (each WebSocket message the relay takes is one, some 200,000 an hour)
/// have no caller to report to: they're kept only if they failed or were slow.
const REQUEST = /^(GET|HEAD|POST|PUT|PATCH|DELETE|OPTIONS)$/;
const sampledTrace = (trace) => parseInt(trace.slice(-8), 16) % SAMPLED_ONE_IN === 0;

/// OTLP/JSON from Cloudflare, each Worker's spans as rows for the spans table: only its name if that's a plain token, its
/// ids, timing, kind and status, and a status code. Its other attributes are Cloudflare's, and may name a URL: none are kept.
export function workerSpans(request, cli) {
  const spans = [];
  for (const resource of request.resourceSpans ?? []) {
    const named = (resource.resource?.attributes ?? []).find((a) => a.key === "service.name")?.value?.stringValue ?? "";
    const job = String(named).replace(`${cli}-`, "");
    if (!TRACED.includes(job)) continue;
    for (const scope of resource.scopeSpans ?? []) spans.push(...(scope.spans ?? []).map((s) => ({ s, job })));
  }
  const failed = ({ s }) => s.status?.code === 2 || s.status?.code === "STATUS_CODE_ERROR";
  const took = ({ s }) => nanos(s.endTimeUnixNano) - nanos(s.startTimeUnixNano);
  const worth = new Set(spans.filter((x) => failed(x) || took(x) >= SLOW_MS).map(({ s }) => s.traceId));
  const requests = new Set(spans.filter(({ s }) => !hex(s.parentSpanId, 16) && REQUEST.test(s.name)).map(({ s }) => s.traceId));
  return spans.filter(({ s }) => hex(s.traceId, 32) && hex(s.spanId, 16) && (worth.has(s.traceId) || (requests.has(s.traceId) && (!hex(s.parentSpanId, 16) || sampledTrace(s.traceId))))).map((x) => {
    const { s, job } = x;
    const code = s.attributes?.find((a) => a.key === "http.response.status_code")?.value?.intValue;
    const kind = s.kind === 2 || s.kind === "SPAN_KIND_SERVER" ? "server" : s.kind === 3 || s.kind === "SPAN_KIND_CLIENT" ? "client" : "internal";
    const attrs = { ...(code !== undefined ? { "http.response.status_code": Number(code) } : {}), ...(failed(x) ? { "error.type": "_OTHER" } : {}) };
    return { time: nanos(s.startTimeUnixNano), trace: s.traceId, span: s.spanId, ...(hex(s.parentSpanId, 16) ? { parent: s.parentSpanId } : {}), name: token(s.name) ? s.name : "span", service: `worker.${job}`, instance: NOBODY, kind, status: failed(x) ? "error" : "unset", duration: took(x), attrs: JSON.stringify(attrs) };
  }).filter((row) => kept(row, TABLES.spans, NOBODY, false));
}

/// How often this path says what it was handed, each way that can go, as a device's canary does: an export that's refused
/// or all dropped leaves no span to show it ever came.
const EXPORT_SAID_EVERY = 15 * 60_000;
const exportSaid = {};

/// One log record of an export: how many spans it held and how many were kept (`read`), that it couldn't be read
/// (`unreadable`), or that it came without the token (`refused`), which anyone can make happen, so it's said no more often.
async function sayExport(env, result, found, kept) {
  const now = Date.now();
  if (now - (exportSaid[result] ?? 0) < EXPORT_SAID_EVERY) return;
  exportSaid[result] = now;
  const attrs = { "one.result": result, "one.found": found, "one.new": kept };
  await env.LOGS.send([{ time: now, severity_number: result === "read" ? 9 : 17, severity_text: result === "read" ? "INFO" : "ERROR", body: "telemetry.export", service: "worker.telemetry", instance: NOBODY, attrs: JSON.stringify(attrs) }]);
}

/// Cloudflare's export of the Workers' traces: it can't sign, so it's a header only the deploy that made it knows. A log
/// export of Cloudflare's is gzipped, which nothing here unpacks unasked.
async function otlp(request, env) {
  if (!env.OTLP_TOKEN || request.headers.get("authorization") !== `Bearer ${env.OTLP_TOKEN}`) {
    if (env.OTLP_TOKEN) await sayExport(env, "refused", 0, 0);
    refuse(401, "only Cloudflare's export of the Workers' traces, with its token");
  }
  const gzipped = request.headers.get("content-encoding") === "gzip" && request.body;
  const body = await textOf(gzipped ? new Response(request.body.pipeThrough(new DecompressionStream("gzip"))) : request, MOST_EXPORT).catch(() => undefined);
  if (body === null) refuse(413, "that's too big");
  let parsed;
  try {
    parsed = JSON.parse(body);
  } catch {
    await sayExport(env, "unreadable", 0, 0);
    refuse(400, "OTLP here is JSON, gzipped or not");
  }
  const found = (parsed.resourceSpans ?? []).reduce((n, r) => n + (r.scopeSpans ?? []).reduce((m, s) => m + (s.spans ?? []).length, 0), 0);
  const rows = workerSpans(parsed, env.CLI).slice(0, MOST_ROWS);
  if (rows.length) await env.SPANS.send(rows);
  await sayExport(env, "read", found, rows.length);
  return new Response(null, { status: 204 });
}

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

// A filter of a table, by what a read asks: `since` and which services always, and the rest as given. Basin partitions each
// table by day of `__ingest_ts` and prunes by that alone: a bound on "time" reads every file (measured 2026-11-10, 171 files
// against 3). A row can be `FUTURE` ahead of when it came.
function where(a, more = []) {
  const parts = [`__ingest_ts >= '${new Date(Math.max(0, a.since - FUTURE)).toISOString()}'`, `"time" >= ${a.since}`, `service IN (${a.services.map(quoted).join(", ")})`, ...more];
  if (a.service) parts.push(`service = ${quoted(a.service)}`);
  if (a.name) parts.push(`${a.named || "name"} = ${quoted(a.name)}`);
  if (a.trace) parts.push(`trace = ${quoted(a.trace)}`);
  return `WHERE ${parts.join(" AND ")}`;
}

// What a group's newest span said, and its trace: a time in ms is 13 digits for the next two centuries, so the greatest of
// each time before its text is the newest's, and `newestOf` takes the time back off.
const NEWEST = `MAX(CAST("time" AS VARCHAR) || message) AS message, MAX(CAST("time" AS VARCHAR) || trace) AS trace`;
const newestOf = (row) => ({ ...row, message: row.message?.slice(13) ?? null, trace: row.trace?.slice(13) ?? null });
const newest = (table, columns, clause, limit) => `SELECT ${columns} FROM ${NAMESPACE}.${table} ${clause} ORDER BY "time" DESC LIMIT ${limit}`;
const SPAN = `"time", trace, span, parent, name, service, version, instance, kind, status, "duration", message, attrs`;
const LOG = `"time", trace, span, severity_number, severity_text, body, service, version, instance, attrs`;
const errorOnly = (a, key) => (a.status === "error" ? [key] : a.status === "unset" ? [`NOT (${key})`] : []);

/// The reads there are, each from what's asked to its SQL and what it gives.
const QUERIES = {
  // Every request, an hour at a time: what rates and durations add up from.
  requests: { sql: (a) => `SELECT ${hour} AS hour, service, name, COUNT(*) AS "count", SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END) AS errors, SUM("duration") AS "sum", ${buckets()} FROM ${NAMESPACE}.spans ${where(a, [`kind = 'server'`])} GROUP BY ${hour}, service, name ORDER BY hour DESC LIMIT ${a.limit}`, rows: asBuckets },
  spans: { sql: (a) => newest("spans", SPAN, where(a, errorOnly(a, `status = 'error'`)), a.limit) },
  logs: { sql: (a) => newest("logs", LOG, where({ ...a, named: "body" }, errorOnly(a, `severity_number >= 17`)), a.limit) },
  metrics: { sql: (a) => `SELECT ${hour} AS hour, name, "type", unit, service, instance, attrs, SUM("count") AS "count", SUM("sum") AS "sum", ${summed()} FROM ${NAMESPACE}.metrics ${where(a)} GROUP BY ${hour}, name, "type", unit, service, instance, attrs ORDER BY hour DESC LIMIT ${a.limit}`, rows: asBuckets },
  // What broke, as often as it did: its type, the frame that threw it and the build it ran in, in that one attribute.
  errors: { sql: (a) => `SELECT service, name, status, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", MIN("time") AS first_seen, MAX("time") AS last_seen, ${NEWEST} FROM ${NAMESPACE}.spans ${where(a, [`${attr("one.fingerprint")} IS NOT NULL`, ...(a.status === "error" || a.status === "unset" ? [`status = ${quoted(a.status)}`] : [])])} GROUP BY service, name, status, ${attr("one.fingerprint")} ORDER BY last_seen DESC LIMIT ${Math.min(a.limit, ERRORS_READ)}`, rows: (r) => r.map(newestOf) },
  // What a build did, per operation: rates and durations, each with the first time this build was seen.
  builds: { sql: (a) => `SELECT ${attr("one.build")} AS build, service, name, MIN("time") AS first_seen, COUNT(*) AS "count", SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END) AS errors, SUM("duration") AS "sum", ${buckets()} FROM ${NAMESPACE}.spans ${where(a, [`kind = 'server'`, `${attr("one.build")} IS NOT NULL`])} GROUP BY ${attr("one.build")}, service, name ORDER BY first_seen DESC LIMIT ${a.limit}`, rows: asBuckets },
  // What broke in each build: the same fingerprints as `errors`, by the build that ran them.
  build_errors: { sql: (a) => `SELECT ${attr("one.build")} AS build, service, name, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", MIN("time") AS first_seen, MAX("time") AS last_seen, ${NEWEST} FROM ${NAMESPACE}.spans ${where(a, [`status = 'error'`, `${attr("one.fingerprint")} IS NOT NULL`])} GROUP BY ${attr("one.build")}, service, name, ${attr("one.fingerprint")} ORDER BY last_seen DESC LIMIT ${Math.min(a.limit, ERRORS_READ)}`, rows: (r) => r.map(newestOf) },
};

// `requests` and `errors` read the same spans, so a view that shows both is one trip to Basin: each is split out here, its
// `status` applied only to `errors`, as it is when they're separate. Each kept row is a kind of span at a status in an hour.
const SUMMARY = (a) => `SELECT ${hour} AS hour, service, name, kind, status, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", SUM("duration") AS "sum", ${buckets()}, MIN("time") AS first_seen, MAX("time") AS last_seen, ${NEWEST} FROM ${NAMESPACE}.spans ${where(a, [`(kind = 'server' OR ${attr("one.fingerprint")} IS NOT NULL)`])} GROUP BY ${hour}, service, name, kind, status, ${attr("one.fingerprint")} ORDER BY hour DESC LIMIT ${a.limit}`;

function summary(rows, a) {
  const requests = new Map(), errors = new Map();
  for (const row of asBuckets(rows).map(newestOf)) {
    const count = Number(row.count), sum = Number(row.sum);
    if (row.kind === "server") {
      const key = `${row.hour}|${row.service}|${row.name}`;
      const had = requests.get(key);
      if (had) {
        had.count += count;
        had.errors += row.status === "error" ? count : 0;
        had.sum += sum;
        had.buckets = had.buckets.map((n, i) => n + row.buckets[i]);
      } else {
        requests.set(key, { hour: row.hour, service: row.service, name: row.name, count, errors: row.status === "error" ? count : 0, sum, buckets: row.buckets });
      }
    }
    if (row.fingerprint && (!a.status || (a.status !== "error" && a.status !== "unset") || row.status === a.status)) {
      const key = `${row.service}|${row.name}|${row.status}|${row.fingerprint}`;
      const had = errors.get(key);
      if (had) {
        had.count += count;
        had.first_seen = Math.min(had.first_seen, row.first_seen);
        if (row.last_seen >= had.last_seen) Object.assign(had, { last_seen: row.last_seen, message: row.message ?? had.message, trace: row.trace });
      } else {
        errors.set(key, { service: row.service, name: row.name, status: row.status, fingerprint: row.fingerprint, count, first_seen: row.first_seen, last_seen: row.last_seen, message: row.message, trace: row.trace });
      }
    }
  }
  return { requests: [...requests.values()], errors: [...errors.values()].sort((x, y) => y.last_seen - x.last_seen).slice(0, Math.min(a.limit, ERRORS_READ)) };
}

/// What's kept of summaries asked for lately, each while `SUMMARY_FOR` lasts: the promise, so a second asking waits for the first.
const remembered = new Map();

async function summaryOf(env, a) {
  const sql = SUMMARY(a), key = `${a.status}|${sql}`;
  const had = remembered.get(key);
  if (had && Date.now() - had.at < SUMMARY_FOR) return had.rows;
  if (remembered.size >= MOST_SUMMARIES) remembered.clear();
  const rows = query(env, sql).then((r) => summary(r, a));
  remembered.set(key, { at: Date.now(), rows });
  rows.catch(() => remembered.delete(key));
  return rows;
}

function asked(fields) {
  const a = { since: fields.since ?? 0, services: fields.services, limit: Math.min(fields.limit ?? 100, MOST_READ) };
  if (!count(a.since) || !count(a.limit) || !a.limit) refuse(400, "since and limit are numbers");
  a.since -= a.since % 60_000;
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
  const together = names.includes("requests") && names.includes("errors") && !a.trace;
  const results = await Promise.all(names.filter((n) => !together || (n !== "requests" && n !== "errors")).map(async (name) => {
    const { sql, rows = (r) => r } = QUERIES[name];
    return [name, rows(await query(env, sql(a)))];
  }).concat(together ? [summaryOf(env, a).then((s) => [["requests", s.requests], ["errors", s.errors]])] : []));
  return Response.json(Object.fromEntries(results.flatMap((r) => (typeof r[0] === "string" ? [r] : r))));
}

const ROUTES = { "/telemetry/write": write, "/telemetry/read": read, [OTLP_PATH]: otlp };

export default {
  async fetch(request, env) {
    const path = new URL(request.url).pathname;
    // An OTLP exporter may add its signal's own path (`/v1/traces`) to the address it's given.
    const route = path.startsWith(`${OTLP_PATH}/`) ? otlp : ROUTES[path];
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
