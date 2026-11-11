// Telemetry: where a network says how it's doing and reads it back. Spans, logs and metrics, in OpenTelemetry's names, go
// into Cloudflare Basin, written through its pipelines' bindings (`SPANS`, `LOGS`, `METRICS`), which need no token, and
// read by HTTP SQL (`SQL`, with `SQL_TOKEN`), which has no binding. A sink's table can't change once made, so its columns
// (`TABLES`) are few and stable and everything else is in `attrs`, a JSON string.
//
// It opens nothing: what's here is operational and unsealed, each value a closed-set name or a number but for a failure's
// cleaned message (insights/src/message.rs).
//
//   POST /telemetry/write { spans, logs, metrics, at, by, sig } → 204
//   POST /telemetry/otlp  Cloudflare's export of the Workers' own traces and logs (`otlp`), gzipped or not → 204
//   POST /telemetry/read  { queries: [name], since, services, service?, name?, status?, trace?, least?, limit?, at, by, sig }
//                         → { <name>: [row] }
//   hourly                each of `SOURCES`' metric rows: Cloudflare's use the hour before, the relay's sockets now
//
// Each is a request signed by a key on the relay's list (`one/telemetry`). A row that isn't what its table holds is left
// out here, as Basin would take it and drop it in silence. A read runs only `QUERIES`, each value a number or a token
// (`word`), which is what makes them safe to put in SQL: Basin's SQL has no parameters.
import { checkRequest, rolesOf } from "../shared/crypto.js";
import { Refused, answer, refusal, refuse, textOf } from "../shared/http.js";
import { TELEMETRY } from "../shared/protocol.js";
import { routeOf } from "../gateway/worker.js";
import { cloudflareUsage } from "./usage.js";

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

/// Each table's columns, a check each and whether a row has to have it: the schema `oned/cloudflare/src/basin.rs` makes its stream with, held to it by tests/contracts.rs.
export const TABLES = {
  spans: { time: [asTime, 1], trace: [(v) => hex(v, 32), 1], span: [(v) => hex(v, 16), 1], parent: [(v) => hex(v, 16)], name: [word, 1], service: [word, 1], version: [word], instance: [(v) => hex(v, 64), 1], kind: [word, 1], status: [word, 1], duration: [count, 1], message: [text(512)], attrs: [json, 1] },
  logs: { time: [asTime, 1], trace: [(v) => hex(v, 32)], span: [(v) => hex(v, 16)], severity_number: [severity, 1], severity_text: [word, 1], body: [word, 1], service: [word, 1], version: [word], instance: [(v) => hex(v, 64), 1], attrs: [json, 1] },
  metrics: { time: [asTime, 1], name: [word, 1], type: [word, 1], unit: [text(32), 1], service: [word, 1], instance: [(v) => hex(v, 64), 1], count: [count, 1], sum: [number, 1], buckets: [json], attrs: [json, 1] },
};

/// `row` as its table holds it, or null. A device names only itself; an app runtime also what its clients reported through it.
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
  const said = (await answer(env.RELAY.allowed(by))) || null;
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

/// The Workers whose own traces and logs are kept: never `node`, which reports its calls as every app runtime does, whatever a
/// request says of itself, and not this one, as an export of what it did would be the next thing it was handed (insights/src/lib.rs).
export const TRACED = ["gateway", "auth", "relay", "url-signer", "notifications", "onet"];
const token = (v) => typeof v === "string" && /^[A-Za-z0-9_.+-]{1,72}$/.test(v);
const nanos = (v) => Number(BigInt(v ?? 0) / 1000000n);
const OTLP_PATH = "/telemetry/otlp";
/// Who it's from isn't a device, so a Worker's own spans have no device of their own.
const NOBODY = "0".repeat(64);
/// As OTEL's tail sampling does: a trace is kept whole if it failed or was slow, or is one in 20 of a Worker's requests
/// (insights/src/lib.rs). Every request is counted first, kept or not, so rates come from counts.
const SLOW_MS = 1000;
const SAMPLED_ONE_IN = 20;
/// What a Worker answers as a request. Its other events (each WebSocket message the relay takes is one, some 130,000 an hour,
/// and each RPC) have no caller to report to and aren't requests: they're kept only if they failed or were slow, and not counted.
const REQUEST = /^(GET|HEAD|POST|PUT|PATCH|DELETE|OPTIONS)(\s|$)/;
const sampledTrace = (trace) => parseInt(trace.slice(-8), 16) % SAMPLED_ONE_IN === 0;
/// What every Worker's requests add up to, kept or not, as OTEL's span metrics connector names it.
const SPAN_METRIC = "traces.span.metrics.duration";
/// How Cloudflare says an invocation that failed ended (`cloudflare.outcome`), kept as its `error.type` when it threw nothing:
/// what tells running out of memory or CPU, or being cut off, from a bug. Any other is `_OTHER`.
const OUTCOMES = ["exceededCpu", "exceededMemory", "canceled", "responseStreamDisconnected", "scriptNotFound", "loadShed", "killSwitch", "daemonDown"];
/// The errors a JavaScript engine names (insights/src/lib.rs's `EXCEPTIONS`): any other name is `Error`.
export const EXCEPTIONS = ["Error", "TypeError", "RangeError", "ReferenceError", "SyntaxError", "URIError", "EvalError", "AggregateError"];

/// Words that say the next one is a secret, as in `password: hunter` or `Bearer abc` (insights/src/message.rs).
const NAMES_A_SECRET = ["token", "key", "secret", "password", "passwd", "pwd", "auth", "authorization", "bearer", "apikey", "api_key", "cookie", "session"];

/// What a Worker said of a failure, kept to its plain words: Cloudflare's own ("internal error; reference = …", "Network
/// connection lost.") are what explains a failure there. Any other word, a value of any kind, is `<value>`, as are a word
/// all of hex, one a word naming a secret goes before, and so anything insights/src/message.rs would hide, which also
/// makes failures that differ only by one read the same. It never passes through an app runtime, so this is its cleaner.
export function plainWords(said) {
  // `named`: the word before named a secret, and an `=` or `:` would say this one is it; `secret`: this one is.
  let named = false;
  let secret = false;
  const words = String(said ?? "").slice(0, 2048).split(/(\s+)/).map((w) => {
    if (!w || /^\s+$/.test(w)) return w;
    if (/^[=:;,.()-]+$/.test(w)) {
      secret = named && /^[=:]$/.test(w);
      named = false;
      return w;
    }
    const plain = /^[A-Za-z][A-Za-z'-]*[.,;:!?]?$/.test(w) && !/^[0-9A-Fa-f]{16,}[.,;:!?]?$/.test(w);
    const kept = plain && !secret ? w : "<value>";
    const bare = w.replace(/[.,;:!?]$/, "").toLowerCase();
    const names = NAMES_A_SECRET.includes(bare);
    secret = names && (w.endsWith(":") || bare === "bearer");
    named = names && bare === w.toLowerCase();
    return kept;
  });
  const kept = words.join("").trim();
  return kept.length > 512 ? `${kept.slice(0, 512)}…` : kept;
}

const attribute = (attrs, key) => {
  const v = (attrs ?? []).find((a) => a.key === key)?.value;
  return v?.stringValue ?? v?.intValue;
};

/// The first frame of a stack, as `file:line:col`, or "".
function frameOf(stack) {
  const at = /([A-Za-z0-9_.\/-]+\.js):(\d+):(\d+)/.exec(String(stack ?? ""));
  return at ? `${at[1].split("/").pop()}:${at[2]}:${at[3]}` : "";
}

/// A span's exception event, as OTEL's: its type, the first frame of its stack (`file:line:col`), and its plain words.
function exceptionOf(s) {
  const event = (s.events ?? []).find((e) => e.name === "exception");
  if (!event) return null;
  const named = String(attribute(event.attributes, "exception.type") ?? "");
  return { type: EXCEPTIONS.includes(named) ? named : "Error", frame: frameOf(attribute(event.attributes, "exception.stacktrace")), message: plainWords(attribute(event.attributes, "exception.message")) };
}

/// A request span's path, from Cloudflare's `url.path` or `url.full`: read for its route alone, never kept.
function pathOf(s) {
  const path = attribute(s.attributes, "url.path");
  if (typeof path === "string" && path.startsWith("/")) return path;
  try {
    return new URL(attribute(s.attributes, "url.full")).pathname;
  } catch {
    return null;
  }
}

/// Which of `TRACED` a resource is, by its `service.name`, `<cli>-<name>`.
const jobOf = (resource, cli) => String((resource.resource?.attributes ?? []).find((a) => a.key === "service.name")?.value?.stringValue ?? "").replace(`${cli}-`, "");

/// OTLP/JSON from Cloudflare, each Worker's spans as rows for the spans table, `spans`: only its name if that's a plain
/// token, an RPC's as what it called (`<entrypoint>.<method>`), its ids, timing, kind and status, a status code, how it
/// ended if that's why it failed, and an exception's type, frame and plain words. Its other attributes are Cloudflare's,
/// and may name a URL: none are kept. And `metrics`, what every request adds up to, a minute
/// at a time, kept or not.
export function workerSpans(request, cli) {
  const spans = [];
  for (const resource of request.resourceSpans ?? []) {
    const job = jobOf(resource, cli);
    if (!TRACED.includes(job)) continue;
    for (const scope of resource.scopeSpans ?? []) spans.push(...(scope.spans ?? []).filter((s) => hex(s.traceId, 32) && hex(s.spanId, 16)).map((s) => ({ s, job })));
  }
  const failed = ({ s }) => s.status?.code === 2 || s.status?.code === "STATUS_CODE_ERROR";
  const took = ({ s }) => Math.max(0, nanos(s.endTimeUnixNano) - nanos(s.startTimeUnixNano));
  const server = ({ s }) => s.kind === 2 || s.kind === "SPAN_KIND_SERVER";
  // A Durable Object's own fetch, which its Worker's request made: Cloudflare bills it as the object's, not a request.
  const asked = (x) => server(x) && REQUEST.test(x.s.name) && (attribute(x.s.attributes, "cloudflare.execution_model") ?? "stateless") === "stateless";
  const nameOf = (x) => {
    const { s } = x;
    const method = attribute(s.attributes, "jsrpc.method");
    if (token(method)) return [attribute(s.attributes, "cloudflare.entrypoint"), method].filter(token).join(".");
    // The gateway's requests by the route it sent each to, as an app runtime's by its route.
    const path = pathOf(s);
    if (x.job === "gateway" && asked(x) && path) return routeOf(path).route;
    return token(s.name) ? s.name : "span";
  };
  const counts = new Map();
  for (const x of spans.filter(asked)) {
    const minute = nanos(x.s.startTimeUnixNano) - (nanos(x.s.startTimeUnixNano) % 60_000);
    const status = failed(x) ? "STATUS_CODE_ERROR" : "STATUS_CODE_UNSET";
    const key = `${minute}|${x.job}|${nameOf(x)}|${status}`;
    const had = counts.get(key) ?? { time: minute, name: SPAN_METRIC, type: "histogram", unit: "ms", service: `worker.${x.job}`, instance: NOBODY, count: 0, sum: 0, buckets: Array(BOUNDS.length + 1).fill(0), attrs: { "span.name": nameOf(x), "span.kind": "SPAN_KIND_SERVER", "status.code": status } };
    const ms = took(x), bucket = BOUNDS.findIndex((b) => ms <= b);
    had.count += 1;
    had.sum += ms;
    had.buckets[bucket < 0 ? BOUNDS.length : bucket] += 1;
    counts.set(key, had);
  }
  const metrics = [...counts.values()].map((m) => ({ ...m, buckets: JSON.stringify(m.buckets), attrs: JSON.stringify(m.attrs) })).filter((row) => kept(row, TABLES.metrics, NOBODY, false));
  const worth = new Set(spans.filter((x) => failed(x) || took(x) >= SLOW_MS).map(({ s }) => s.traceId));
  // A request reached through another Worker's binding has that Worker's span as its parent, and comes in an export of its
  // own: by its id alone, every export picks the same traces.
  const requests = new Set(spans.filter(asked).map(({ s }) => s.traceId));
  const rows = spans.filter(({ s }) => worth.has(s.traceId) || (requests.has(s.traceId) && sampledTrace(s.traceId))).map((x) => {
    const { s, job } = x;
    const code = attribute(s.attributes, "http.response.status_code");
    const kind = server(x) ? "server" : s.kind === 3 || s.kind === "SPAN_KIND_CLIENT" ? "client" : "internal";
    const thrown = failed(x) ? exceptionOf(s) : null;
    // Kept whole for failing or being slow, it stands for itself alone; sampled, for each of the 20 it was one of.
    const attrs = { ...(code !== undefined ? { "http.response.status_code": Number(code) } : {}), "one.sampled": worth.has(s.traceId) ? 1 : SAMPLED_ONE_IN };
    const outcome = attribute(s.attributes, "cloudflare.outcome");
    if (failed(x)) attrs["error.type"] = thrown?.type ?? (OUTCOMES.includes(outcome) ? outcome : "_OTHER");
    if (thrown) {
      attrs.events = [{ name: "exception", attributes: { "exception.type": thrown.type, ...(thrown.frame ? { "exception.stacktrace": `at ${thrown.frame}` } : {}) } }];
      attrs["one.fingerprint"] = `${thrown.type}|${thrown.frame}|`;
    }
    return { time: nanos(s.startTimeUnixNano), trace: s.traceId, span: s.spanId, ...(hex(s.parentSpanId, 16) ? { parent: s.parentSpanId } : {}), name: nameOf(x), service: `worker.${job}`, instance: NOBODY, kind, status: failed(x) ? "error" : "unset", duration: took(x), ...(thrown?.message ? { message: thrown.message } : {}), attrs: JSON.stringify(attrs) };
  }).filter((row) => kept(row, TABLES.spans, NOBODY, false));
  return { spans: rows, metrics };
}

/// OTEL's severity numbers by the text a record may carry instead: an error is 17 and over.
const SEVERITIES = { trace: 1, debug: 5, info: 9, log: 9, warn: 13, warning: 13, error: 17, fatal: 21 };
const ERROR = 17;
const FATAL = 21;

function severityOf(r) {
  if (Number.isSafeInteger(r.severityNumber)) return r.severityNumber;
  const named = /^SEVERITY_NUMBER_([A-Z]+)(\d?)$/.exec(String(r.severityNumber ?? ""));
  if (named) return (SEVERITIES[named[1].toLowerCase()] ?? 0) + Math.max(0, Number(named[2] || 1) - 1);
  return SEVERITIES[String(r.severityText ?? "").toLowerCase()] ?? 0;
}

/// OTLP/JSON from Cloudflare, each Worker's log records as rows for the logs table: only what says something broke, a line
/// at error level or an exception, and of it only its time, trace and span ids, severity, and as OTEL's `exception.*`: its
/// type, the first frame of its stack and its first line's plain words. Every attribute Cloudflare adds is left unread
/// (a request's URL and headers, the caller's address, its `cf`), as is a body that isn't text.
export function workerLogs(request, cli) {
  const rows = [];
  for (const resource of request.resourceLogs ?? []) {
    const job = jobOf(resource, cli);
    if (!TRACED.includes(job)) continue;
    for (const scope of resource.scopeLogs ?? []) {
      for (const r of scope.logRecords ?? []) {
        const thrown = attribute(r.attributes, "exception.type") !== undefined;
        const severity = severityOf(r);
        if (severity < ERROR && !thrown) continue;
        const text = String(attribute(r.attributes, "exception.message") ?? r.body?.stringValue ?? "");
        const [first, ...rest] = text.split("\n");
        // A stack logged as text starts with its error's name: `TypeError: …`.
        const prefixed = /^([A-Za-z]+): /.exec(first);
        const named = String(attribute(r.attributes, "exception.type") ?? prefixed?.[1] ?? "");
        const type = thrown || EXCEPTIONS.includes(named) ? (EXCEPTIONS.includes(named) ? named : "Error") : null;
        const frame = frameOf(attribute(r.attributes, "exception.stacktrace") ?? rest.join("\n"));
        const message = plainWords(prefixed && EXCEPTIONS.includes(prefixed[1]) ? first.slice(prefixed[0].length) : first);
        const attrs = { ...(type ? { "exception.type": type } : {}), ...(message ? { "exception.message": message } : {}), ...(frame ? { "exception.stacktrace": `at ${frame}` } : {}) };
        const time = nanos(r.timeUnixNano || r.observedTimeUnixNano) || Date.now();
        const row = { time, ...(hex(r.traceId, 32) ? { trace: r.traceId } : {}), ...(hex(r.spanId, 16) ? { span: r.spanId } : {}), severity_number: Math.min(Math.max(severity, ERROR), 24), severity_text: severity >= FATAL ? "FATAL" : "ERROR", body: type ? "exception" : "log", service: `worker.${job}`, instance: NOBODY, attrs: JSON.stringify(attrs) };
        if (kept(row, TABLES.logs, NOBODY, false)) rows.push(row);
      }
    }
  }
  return rows;
}

/// How often this path says what it was handed, each way that can go, as a device's canary does: an export that's refused
/// or all dropped leaves no span to show it ever came.
const EXPORT_SAID_EVERY = 15 * 60_000;
const exportSaid = {};

/// One log record of an export: how many spans and log records it held and how many were kept (`read`), that it couldn't be read
/// (`unreadable`), or that it came without the token (`refused`), which anyone can make happen, so it's said no more often.
async function sayExport(env, result, found, kept) {
  const now = Date.now();
  if (now - (exportSaid[result] ?? 0) < EXPORT_SAID_EVERY) return;
  exportSaid[result] = now;
  const attrs = { "one.result": result, "one.found": found, "one.new": kept };
  await env.LOGS.send([{ time: now, severity_number: result === "read" ? 9 : 17, severity_text: result === "read" ? "INFO" : "ERROR", body: "telemetry.export", service: "worker.telemetry", instance: NOBODY, attrs: JSON.stringify(attrs) }]);
}

/// Cloudflare's export of the Workers' traces or logs: it can't sign, so it's a header only the deploy that made it knows. A log
/// export of Cloudflare's is gzipped, which nothing here unpacks unasked.
async function otlp(request, env) {
  if (!env.OTLP_TOKEN || request.headers.get("authorization") !== `Bearer ${env.OTLP_TOKEN}`) {
    if (env.OTLP_TOKEN) await sayExport(env, "refused", 0, 0);
    refuse(401, "only Cloudflare's export of the Workers' traces and logs, with its token");
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
  const held = (list, inner, leaf) => (list ?? []).reduce((n, r) => n + (r[inner] ?? []).reduce((m, s) => m + (s[leaf] ?? []).length, 0), 0);
  const found = held(parsed.resourceSpans, "scopeSpans", "spans") + held(parsed.resourceLogs, "scopeLogs", "logRecords");
  const { spans, metrics } = workerSpans(parsed, env.CLI);
  const rows = spans.slice(0, MOST_ROWS);
  const logs = workerLogs(parsed, env.CLI).slice(0, MOST_ROWS);
  await Promise.all([rows.length && env.SPANS.send(rows), metrics.length && env.METRICS.send(metrics.slice(0, MOST_ROWS)), logs.length && env.LOGS.send(logs)]);
  await sayExport(env, "read", found, rows.length + logs.length);
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
/// A server span counted by its own row: one sampled where it was recorded (`one.sampled`) is counted in `SPAN_METRIC` instead.
const UNSAMPLED = `json_get_int(attrs, 'one.sampled') IS NULL`;
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
const NEWEST = `MAX(CAST("time" AS VARCHAR) || message) AS message, MAX(CAST("time" AS VARCHAR) || trace) AS trace, MAX(CAST("time" AS VARCHAR) || instance) AS instance`;
const newestOf = (row) => ({ ...row, message: row.message?.slice(13) ?? null, trace: row.trace?.slice(13) ?? null, instance: row.instance?.slice(13) ?? null });
const newest = (table, columns, clause, limit) => `SELECT ${columns} FROM ${NAMESPACE}.${table} ${clause} ORDER BY "time" DESC LIMIT ${limit}`;
const SPAN = `"time", trace, span, parent, name, service, version, instance, kind, status, "duration", message, attrs`;
const LOG = `"time", trace, span, severity_number, severity_text, body, service, version, instance, attrs`;
const errorOnly = (a, key) => (a.status === "error" ? [key] : a.status === "unset" ? [`NOT (${key})`] : []);

/// The reads there are, each the table it reads, from what's asked to its SQL, and what it gives.
const QUERIES = {
  // The newest traces a reader filters for, whole (`traces`).
  traces: { table: "spans", run: traces },
  // Every request, an hour at a time: what rates and durations add up from.
  requests: { table: "spans", sql: (a) => `SELECT ${hour} AS hour, service, name, COUNT(*) AS "count", SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END) AS errors, SUM("duration") AS "sum", ${buckets()} FROM ${NAMESPACE}.spans ${where(a, [`kind = 'server'`, UNSAMPLED])} GROUP BY ${hour}, service, name ORDER BY hour DESC LIMIT ${a.limit}`, rows: asBuckets, counted: true },
  spans: { table: "spans", sql: (a) => newest("spans", SPAN, where(a, errorOnly(a, `status = 'error'`)), a.limit) },
  logs: { table: "logs", sql: (a) => newest("logs", LOG, where({ ...a, named: "body" }, errorOnly(a, `severity_number >= 17`)), a.limit) },
  metrics: { table: "metrics", sql: (a) => `SELECT ${hour} AS hour, name, "type", unit, service, instance, attrs, SUM("count") AS "count", SUM("sum") AS "sum", ${summed()} FROM ${NAMESPACE}.metrics ${where(a)} GROUP BY ${hour}, name, "type", unit, service, instance, attrs ORDER BY hour DESC LIMIT ${a.limit}`, rows: asBuckets },
  // What broke, as often as it did: its type, the frame that threw it and the build it ran in, in that one attribute.
  errors: { table: "spans", sql: (a) => `SELECT service, name, status, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", MIN("time") AS first_seen, MAX("time") AS last_seen, ${NEWEST} FROM ${NAMESPACE}.spans ${where(a, [`${attr("one.fingerprint")} IS NOT NULL`, ...(a.status === "error" || a.status === "unset" ? [`status = ${quoted(a.status)}`] : [])])} GROUP BY service, name, status, ${attr("one.fingerprint")} ORDER BY last_seen DESC LIMIT ${Math.min(a.limit, ERRORS_READ)}`, rows: (r) => r.map(newestOf) },
  // What a build did, per operation: rates and durations, each with the first time this build was seen.
  builds: { table: "spans", sql: (a) => `SELECT ${attr("one.build")} AS build, service, name, MIN("time") AS first_seen, COUNT(*) AS "count", SUM(CASE WHEN status = 'error' THEN 1 ELSE 0 END) AS errors, SUM("duration") AS "sum", ${buckets()} FROM ${NAMESPACE}.spans ${where(a, [`kind = 'server'`, `${attr("one.build")} IS NOT NULL`])} GROUP BY ${attr("one.build")}, service, name ORDER BY first_seen DESC LIMIT ${a.limit}`, rows: asBuckets },
  // What broke in each build: the same fingerprints as `errors`, by the build that ran them.
  build_errors: { table: "spans", sql: (a) => `SELECT ${attr("one.build")} AS build, service, name, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", MIN("time") AS first_seen, MAX("time") AS last_seen, ${NEWEST} FROM ${NAMESPACE}.spans ${where(a, [`status = 'error'`, `${attr("one.fingerprint")} IS NOT NULL`])} GROUP BY ${attr("one.build")}, service, name, ${attr("one.fingerprint")} ORDER BY last_seen DESC LIMIT ${Math.min(a.limit, ERRORS_READ)}`, rows: (r) => r.map(newestOf) },
};

/// The most traces a list finds at once.
const MOST_TRACES = 500;

// The newest traces with a span that's what a reader filters for (service, name, status, at least `least` ms), found over
// the whole range and each read whole: a list that's never crowded out by whatever's most common.
async function traces(env, a) {
  const found = await query(env, `SELECT trace, MAX("time") AS at FROM ${NAMESPACE}.spans ${where(a, [...errorOnly(a, `status = 'error'`), ...(a.least ? [`"duration" >= ${a.least}`] : [])])} GROUP BY trace ORDER BY at DESC LIMIT ${Math.min(a.limit, MOST_TRACES)}`);
  const ids = found.map((r) => r.trace).filter((t) => hex(t, 32));
  if (!ids.length) return [];
  return query(env, newest("spans", SPAN, where({ since: a.since, services: a.services }, [`trace IN (${ids.map(quoted).join(", ")})`]), MOST_READ));
}

// `requests` and `errors` read the same spans, so a view that shows both is one trip to Basin: each is split out here, its
// `status` applied only to `errors`, as it is when they're separate. Each kept row is a kind of span at a status in an hour.
// A server span sampled where it was recorded counts in neither: `counted` adds what its `SPAN_METRIC` says.
const COUNTED_KIND = `CASE WHEN ${UNSAMPLED} THEN kind ELSE 'sampled' END`;
const SUMMARY = (a) => `SELECT ${hour} AS hour, service, name, ${COUNTED_KIND} AS kind, status, ${attr("one.fingerprint")} AS fingerprint, COUNT(*) AS "count", SUM("duration") AS "sum", ${buckets()}, MIN("time") AS first_seen, MAX("time") AS last_seen, ${NEWEST} FROM ${NAMESPACE}.spans ${where(a, [`(kind = 'server' OR ${attr("one.fingerprint")} IS NOT NULL)`])} GROUP BY ${hour}, service, name, ${COUNTED_KIND}, status, ${attr("one.fingerprint")} ORDER BY hour DESC LIMIT ${a.limit}`;

// What the server spans sampled where they were recorded add up to, an hour at a time, as `requests` are.
const COUNTED = (a) => `SELECT ${hour} AS hour, service, ${attr("span.name")} AS name, ${attr("status.code")} AS status, SUM("count") AS "count", SUM("sum") AS "sum", ${summed()} FROM ${NAMESPACE}.metrics ${where({ ...a, named: attr("span.name") }, [`name = '${SPAN_METRIC}'`])} GROUP BY ${hour}, service, ${attr("span.name")}, ${attr("status.code")} ORDER BY hour DESC LIMIT ${a.limit}`;

/// Adds an hour's requests of an operation to `requests`, by hour, service and name.
function add(requests, { hour, service, name, count, errors, sum, buckets }) {
  const key = `${hour}|${service}|${name}`;
  const had = requests.get(key);
  if (!had) return requests.set(key, { hour, service, name, count, errors, sum, buckets });
  had.count += count;
  had.errors += errors;
  had.sum += sum;
  had.buckets = had.buckets.map((n, i) => n + buckets[i]);
}

/// `counted` (`COUNTED`'s rows) as requests.
const countedRequests = (counted) => asBuckets(counted).map((r) => ({ hour: r.hour, service: r.service, name: r.name, count: Number(r.count), errors: r.status === "STATUS_CODE_ERROR" ? Number(r.count) : 0, sum: Number(r.sum), buckets: r.buckets }));

function summary(rows, counted, a) {
  const requests = new Map(), errors = new Map();
  for (const row of countedRequests(counted)) add(requests, row);
  for (const row of asBuckets(rows).map(newestOf)) {
    const count = Number(row.count), sum = Number(row.sum);
    if (row.kind === "server") add(requests, { hour: row.hour, service: row.service, name: row.name, count, errors: row.status === "error" ? count : 0, sum, buckets: row.buckets });
    if (row.fingerprint && (!a.status || (a.status !== "error" && a.status !== "unset") || row.status === a.status)) {
      const key = `${row.service}|${row.name}|${row.status}|${row.fingerprint}`;
      const had = errors.get(key);
      if (had) {
        had.count += count;
        had.first_seen = Math.min(had.first_seen, row.first_seen);
        if (row.last_seen >= had.last_seen) Object.assign(had, { last_seen: row.last_seen, message: row.message ?? had.message, trace: row.trace, instance: row.instance });
      } else {
        errors.set(key, { service: row.service, name: row.name, status: row.status, fingerprint: row.fingerprint, count, first_seen: row.first_seen, last_seen: row.last_seen, message: row.message, trace: row.trace, instance: row.instance });
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
  const rows = Promise.all([query(env, sql), query(env, COUNTED(a))]).then(([r, counted]) => summary(r, counted, a));
  remembered.set(key, { at: Date.now(), rows });
  rows.catch(() => remembered.delete(key));
  return rows;
}

function asked(fields) {
  const a = { since: fields.since ?? 0, services: fields.services, limit: Math.min(fields.limit ?? 100, MOST_READ) };
  if (!count(a.since) || !count(a.limit) || !a.limit) refuse(400, "since and limit are numbers");
  a.since -= a.since % 60_000;
  if (!Array.isArray(a.services) || a.services.length > MOST_SERVICES || !a.services.every(word)) refuse(400, "services are names");
  if (fields.least !== undefined && fields.least !== null) {
    if (!count(fields.least)) refuse(400, "least is a number");
    a.least = fields.least;
  }
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
    const { table, sql, rows = (r) => r, counted, run } = QUERIES[name];
    // A metric belongs to no trace: asked for one, a table without the column has none of it.
    if (a.trace && !TABLES[table].trace) return [name, []];
    if (run) return [name, await run(env, a)];
    if (!counted || a.trace) return [name, rows(await query(env, sql(a)))];
    const [own, more] = await Promise.all([query(env, sql(a)), query(env, COUNTED(a))]);
    const all = new Map();
    for (const row of [...rows(own).map((r) => ({ ...r, count: Number(r.count), errors: Number(r.errors), sum: Number(r.sum) })), ...countedRequests(more)]) add(all, row);
    return [name, [...all.values()]];
  }).concat(together ? [summaryOf(env, a).then((s) => [["requests", s.requests], ["errors", s.errors]])] : []));
  return Response.json(Object.fromEntries(results.flatMap((r) => (typeof r[0] === "string" ? [r] : r))));
}

/// The relay's sockets as it's asked and what it refused and ended since it was last asked, as `worker.relay`'s readings:
/// how near it is to what one object holds shows before it's reached.
async function relayStats(env, at, instance) {
  const { open, counted } = await answer(env.RELAY.stats());
  const row = (name, type, unit, sum, attrs) => ({ time: at, name, type, unit, service: "worker.relay", instance, count: 1, sum, attrs: JSON.stringify(attrs) });
  return [
    ...Object.entries(open).map(([kind, n]) => row("relay.sockets", "gauge", "{socket}", n, { "relay.socket": kind })),
    ...Object.entries(counted).map(([what, n]) => row("relay.events", "counter", "{event}", n, { "relay.event": what })),
  ];
}

/// What's recorded of the world outside the network, each a function of the hour it's run in that gives metric rows.
const SOURCES = { cloudflare: cloudflareUsage, relay: relayStats };

/// Each source's rows, and a log of how many each gave or that it failed, which is how it's seen to run.
async function hourly(env, at) {
  await Promise.all(Object.entries(SOURCES).map(async ([source, rowsOf]) => {
    let attrs;
    try {
      const rows = (await rowsOf(env, at, NOBODY)).filter((row) => kept(row, TABLES.metrics, NOBODY, false)).slice(0, MOST_ROWS);
      if (rows.length) await env.METRICS.send(rows);
      attrs = { "one.source": source, "one.result": "read", "one.found": rows.length };
    } catch (err) {
      console.error(err.stack || err);
      attrs = { "one.source": source, "one.result": "failed" };
    }
    const failed = attrs["one.result"] === "failed";
    await env.LOGS.send([{ time: Date.now(), severity_number: failed ? 17 : 9, severity_text: failed ? "ERROR" : "INFO", body: "telemetry.source", service: "worker.telemetry", instance: NOBODY, attrs: JSON.stringify(attrs) }]);
  }));
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
  async scheduled(event, env) {
    await hourly(env, event.scheduledTime);
  },
};
