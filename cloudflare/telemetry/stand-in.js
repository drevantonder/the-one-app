// A stand-in for Cloudflare Basin, which exists only on Cloudflare: where a network's Workers run on one machine (a
// throwaway network's, or `cloudflare serve`'s), the telemetry Worker writes to and reads from this instead. It answers
// what the Worker asks of Basin, the pipelines' `send` and SQL over HTTP, from one SQLite database in a Durable Object,
// so the Worker has no idea which it's talking to. Only the two functions it doesn't have are turned into SQLite's. As in
// Basin, a row that doesn't fit is taken and dropped, and an unknown table is an error.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import { TABLES } from "./worker.js";

// A JavaScript number comes to SQLite as a float, and these are divided as integers.
const INTEGERS = ["time", "duration", "severity_number", "count"];
const store = (env) => env.STORE.get(env.STORE.idFromName("telemetry"));
const sqlite = (sql) => sql.replace(/\bFROM telemetry\./g, "FROM ").replace(/json_get_str\(attrs, '([^']+)'\)/g, `json_extract(attrs, '$."$1"')`).replace(/json_get_int\(buckets, (\d+)\)/g, "json_extract(buckets, '$[$1]')");

export class Store extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    for (const [table, columns] of Object.entries(TABLES)) {
      ctx.storage.sql.exec(`CREATE TABLE IF NOT EXISTS ${table} (${Object.keys(columns).map((c) => `"${c}"${INTEGERS.includes(c) ? " INTEGER" : ""}`).join(", ")})`);
    }
  }

  send(table, rows) {
    const columns = Object.keys(TABLES[table]);
    for (const row of rows) {
      if (columns.some((c) => TABLES[table][c][1] && row[c] === undefined)) continue;
      this.ctx.storage.sql.exec(`INSERT INTO ${table} VALUES (${columns.map(() => "?").join(", ")})`, ...columns.map((c) => row[c] ?? null));
    }
  }

  query(sql) {
    return this.ctx.storage.sql.exec(sqlite(sql)).toArray();
  }
}

const stream = (table) => class extends WorkerEntrypoint {
  send(rows) {
    return store(this.env).send(table, rows);
  }
};

export const Spans = stream("spans");
export const Logs = stream("logs");
export const Metrics = stream("metrics");

export default {
  async fetch(request, env) {
    try {
      const { query } = await request.json();
      return Response.json({ success: true, result: { rows: await store(env).query(query) } });
    } catch (err) {
      return Response.json({ success: false, errors: [{ message: String(err.message || err) }] }, { status: 400 });
    }
  },
};
