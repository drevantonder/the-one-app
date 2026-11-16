// Telemetry's edge function: the Tail Worker of the network's other Workers (`core/edge/edge.js`). Each request
// one of them handled for the network becomes a span, kept with what servers recorded, so a request that failed or
// hung at the edge, and one that never reached a server, is in Telemetry too. A span has the request's route, status,
// duration and outcome, and what the Worker said of it in a line of its own, `{"telemetry": …}`: its `traceparent`,
// which a server it reached carried on, the `parent` it came in with, `attributes` such as `one.server`, an
// `error`, and the `start` and `end` of what it did, where that's less than the Worker's whole run. Nothing else of a
// request is kept: no query, header, cookie, body or other log line, so never a token or a code.
//
// Spans are gathered a minute at a time in a Durable Object and kept as one blob of Telemetry's own artifact for
// the network, `batches/<day>/<hour>/<to>-edge-<bytes>`, as a server keeps its batch (`Telemetry.Store`), in JSON.
import { DurableObject, env } from "cloudflare:workers";

declare global {
  namespace Cloudflare {
    interface Env {
      BATCH: DurableObjectNamespace<Batch>;
      OWN: { write(name: string, data: string): Promise<boolean> };
    }
  }
}

/** A span as Telemetry keeps one: OpenTelemetry's, with times in nanoseconds since 1970. */
interface Span {
  traceId: string;
  spanId: string;
  parentSpanId: string | null;
  name: string;
  kind: "server";
  start: number;
  end: number;
  attributes: Record<string, string | number | boolean>;
  status: { code: "ok" | "error"; message: string };
  events: [];
  server: "edge";
}

/** What a Worker says of a request, in a line `{"telemetry": …}` of its log. */
interface LogLine {
  traceparent?: string;
  parent?: string;
  attributes?: Record<string, string | number | boolean>;
  error?: string;
  /** When what the span is of began and ended, in milliseconds since 1970, where the Worker ran on past it. */
  start?: number;
  end?: number;
}

/**
 * What's noise on its own, and kept only when it fails: iroh's relay and its pings, who's on, what servers ask the
 * presigner, a file such as a page's script, what a Durable Object does, and any call between Workers but the relay's
 * `forward`.
 */
const ROUTINE = /^\/(ping|relay|relay\/seen|presigner\/.*)$|\.[a-z0-9]+$/;
/** How long a minute's spans wait to be kept, in milliseconds. */
const EVERY = 60_000;

const hex = (bytes: number) => Array.from(crypto.getRandomValues(new Uint8Array(bytes)), (byte) => byte.toString(16).padStart(2, "0")).join("");
const PARENT = /^00-([0-9a-f]{32})-([0-9a-f]{16})-[0-9a-f]{2}$/;

/** A path as a route: what names a sharing link or a session in it isn't kept. */
const route = (path: string) => path.replace(/^\/(link|apps)\/.*/, "/$1/*");

/** The span for one Worker's event, or null for one that's routine and went well. */
function span(item: TraceItem): Span | null {
  const event = item.event as (TraceItemFetchEventInfo & { rpcMethod?: string }) | null;
  const worker = (item.scriptName ?? "").replace(/^network-/, "");
  const request = event && "request" in event ? event.request : null;
  const path = request ? route(new URL(request.url).pathname) : null;
  const status = event && "response" in event ? event.response?.status : undefined;
  const record: LogLine = {};
  // `wrangler dev` says which Worker it was before what was logged.
  for (const text of item.logs.flatMap(({ message }) => message).filter((text) => typeof text === "string" && text.startsWith('{"telemetry"'))) {
    const line: LogLine = JSON.parse(text).telemetry;
    Object.assign(record, line, { attributes: { ...record.attributes, ...line.attributes } });
  }
  const thrown = item.exceptions[0];
  const upgraded = request?.headers.upgrade?.toLowerCase() === "websocket";
  const error =
    record.error ||
    (thrown && `${thrown.name}: ${thrown.message}`) ||
    // A WebSocket ends when either end leaves, which is no failure.
    (!upgraded && item.outcome === "canceled" && "The client went away before it was answered") ||
    (!upgraded && item.outcome !== "ok" && `It ended ${item.outcome}`) ||
    (status !== undefined && status >= 500 && `It answered ${status}`) ||
    undefined;
  // As OpenTelemetry names an HTTP server's span, and a call between Workers by the Worker and its method.
  const name = request ? `${request.method} ${path}` : event?.rpcMethod ? [worker, event.rpcMethod].filter(Boolean).join(" ") : null;
  const routine = item.executionModel === "durableObject" || (path ? ROUTINE.test(path) : event?.rpcMethod !== "forward");
  if (!name || (!error && routine)) return null;

  const [, trace = hex(16), id = hex(8)] = PARENT.exec(record.traceparent ?? "") ?? [];
  const start = Math.round((record.start ?? item.eventTimestamp ?? Date.now()) * 1e6);
  const end = record.end !== undefined ? Math.round(record.end * 1e6) : start + Math.round(((item as { wallTime?: number }).wallTime ?? 0) * 1e6);
  return {
    traceId: trace,
    spanId: id,
    parentSpanId: PARENT.exec(record.parent ?? "")?.[1] === trace ? PARENT.exec(record.parent!)![2] : null,
    name,
    kind: "server",
    start,
    end,
    attributes: {
      ...(worker && { "one.worker": worker }),
      ...(request && { "http.request.method": request.method, "http.route": path! }),
      ...(status !== undefined && { "http.response.status_code": status }),
      ...record.attributes,
    },
    status: { code: error ? "error" : "ok", message: (error ?? "").slice(0, 1000) },
    events: [],
    server: "edge",
  };
}

export default {
  async tail(items: TraceItem[]) {
    const spans = items.map(span).filter((span) => span !== null);
    if (spans.length) await env.BATCH.get(env.BATCH.idFromName("edge")).add(spans);
  },
} satisfies ExportedHandler;

/** A minute's spans, kept once it's over. */
export class Batch extends DurableObject {
  async add(spans: Span[]) {
    if (!(await this.ctx.storage.getAlarm())) {
      await this.ctx.storage.put("from", Date.now() * 1e6);
      await this.ctx.storage.setAlarm(Date.now() + EVERY);
    }
    await this.ctx.storage.put(`spans/${Date.now()}/${hex(4)}`, spans);
  }

  async alarm() {
    const spans = [...(await this.ctx.storage.list<Span[]>({ prefix: "spans/" })).values()].flat().slice(-2000);
    const from = (await this.ctx.storage.get<number>("from")) ?? Date.now() * 1e6;
    const to = Date.now() * 1e6;
    const data = JSON.stringify({ server: "edge", machine: "edge", from, to, spans, logs: [], metrics: [] });
    const at = new Date(to / 1e6).toISOString();
    const name = `batches/${at.slice(0, 10)}/${at.slice(11, 13)}/${to}-edge-${new TextEncoder().encode(data).length}`;
    await env.OWN.write(name, data);
    await this.ctx.storage.deleteAll();
  }
}
