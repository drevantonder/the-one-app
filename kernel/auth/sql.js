// Kysely over a Durable Object's SQLite (`ctx.storage.sql`), which is how Better Auth keeps its tables here, on
// Cloudflare and on any workerd alike. Its calls are synchronous, so each query is one call.

import { SqliteAdapter, SqliteIntrospector, SqliteQueryCompiler } from "kysely";

/// A query as it's run here: one that lists the tables (Better Auth's migrations, as they compare its tables and their
/// indexes with its own) lists only the database's, not the Durable Object's (`_cf_…`, where its alarm is kept), which
/// it may not read: listing their columns or indexes would fail it all.
const ours = (sql) => sql.replace(/"?sqlite_master"?/g, "(select * from sqlite_master where substr(name, 1, 4) != '_cf_')");

class Connection {
  constructor(sql) {
    this.sql = sql;
  }

  async executeQuery({ sql, parameters }) {
    const cursor = this.sql.exec(ours(sql), ...parameters.map((p) => (p instanceof Date ? p.toISOString() : typeof p === "boolean" ? Number(p) : p)));
    const rows = cursor.toArray();
    return { rows, numAffectedRows: BigInt(cursor.rowsWritten) };
  }

  async *streamQuery() {
    throw new Error("streaming isn't needed");
  }
}

class Driver {
  constructor(sql) {
    this.connection = new Connection(sql);
  }
  async init() {}
  async acquireConnection() {
    return this.connection;
  }
  // A Durable Object runs one request's synchronous queries at a time, so Better Auth's few transactions run as their
  // statements in order.
  async beginTransaction() {}
  async commitTransaction() {}
  async rollbackTransaction() {}
  async releaseConnection() {}
  async destroy() {}
}

export class DurableObjectDialect {
  constructor(sql) {
    this.sql = sql;
  }
  createAdapter() {
    return new SqliteAdapter();
  }
  createDriver() {
    return new Driver(this.sql);
  }
  createQueryCompiler() {
    return new SqliteQueryCompiler();
  }
  createIntrospector(db) {
    return new SqliteIntrospector(db);
  }
}
