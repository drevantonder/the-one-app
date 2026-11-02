// The Vault's items: each in one scope, with named fields (secret ones set, never read back), the hosts its secret
// fields go to, and the apps switched on for it. Secret fields are sealed under a key derived from the scope's key,
// so they open only for a session that opens the scope. Apps write `{{<item id>:field}}` (or `{{field}}`, or
// `{{<group id>/field}}`) where a field goes, and the vault fills it in on the way out: it makes the request to one
// of the item's hosts itself, so a secret never reaches a node. Sign-ins are items too: an app swaps a
// refresh token it never sees for an hour's access token, through the vault.

import { b64u, hkdf, random } from "../shared/crypto.js";

const ID = /^[0-9a-f]{16}$/;
const FIELD = /^[A-Za-z0-9_.-]{1,64}$/;
const HOST = /^(?=.{1,253}$)[a-z0-9]([a-z0-9-]*[a-z0-9])?(\.[a-z0-9]([a-z0-9-]*[a-z0-9])?)*$/;
const LIMIT = 32 << 20;

const itemsKey = (scopeKey) => hkdf(scopeKey, "one/items");

async function seal(key, value) {
  const nonce = random(12);
  const k = await crypto.subtle.importKey("raw", key, "AES-GCM", false, ["encrypt"]);
  const sealed = new Uint8Array(await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce }, k, new TextEncoder().encode(JSON.stringify(value))));
  const out = new Uint8Array(12 + sealed.length);
  out.set(nonce);
  out.set(sealed, 12);
  return out;
}

async function open(key, sealed) {
  sealed = new Uint8Array(sealed);
  const k = await crypto.subtle.importKey("raw", key, "AES-GCM", false, ["decrypt"]);
  return JSON.parse(new TextDecoder().decode(await crypto.subtle.decrypt({ name: "AES-GCM", iv: sealed.slice(0, 12) }, k, sealed.slice(12))));
}

export const schema = `
  create table if not exists items (id text primary key, scope text not null, data text not null, sealed blob,
    user text not null, time integer not null, made integer not null);
  create index if not exists items_scope on items (scope, made);
`;

/// An item as apps and people see it: plain fields' values, and which fields are secret.
function shown(row) {
  const data = JSON.parse(row.data);
  const fields = Object.fromEntries(Object.entries(data.fields).map(([n, f]) => [n, f.secret ? { secret: true } : { value: f.value }]));
  return { id: row.id, scope: row.scope, title: data.title, type: data.type, domain: data.domain, hosts: data.hosts, apps: data.apps, app: data.app, fields, time: row.made, changed: row.time, user: row.user };
}

export class Items {
  /// `vault` opens scopes' keys: `scopeKey(held, scope)`, and `held(held)`, the scopes a session opens.
  constructor(sql, vault) {
    this.sql = sql;
    this.vault = vault;
  }

  /// Every item in the scopes a session opens (or just `scope`), oldest first.
  list(held, scope) {
    const scopes = this.vault.held(held).filter((s) => !scope || s === scope);
    let rows = scopes.flatMap((s) => this.sql.exec("select * from items where scope = ? order by made, rowid", s).toArray());
    if (held.apps) rows = rows.filter((r) => JSON.parse(r.data).apps.some((a) => held.apps.includes(a)));
    return { items: rows.map(shown) };
  }

  /// Makes, changes or deletes an item: `{ scope, item: { id, title?, type?, domain?, fields?, hosts?, apps?,
  /// deleted? } }`. A field is `{ value, secret? }`, `{ from: <another field> }` to rename it, `{ generate: true }`
  /// for a new random secret, or null to drop it.
  async set(held, { scope, item }) {
    if (!item || !ID.test(item.id || "")) throw new Error("an item's id is 16 hex characters");
    const key = await itemsKey(await this.vault.scopeKey(held, scope));
    const row = this.sql.exec("select * from items where id = ?", item.id).toArray()[0];
    if (row && row.scope !== scope) throw new Error("that item is in another scope");
    if (item.deleted) {
      if (!row) throw new Error("no such item");
      this.sql.exec("delete from items where id = ?", item.id);
      return { id: item.id, deleted: true };
    }
    const now = Date.now();
    const before = row ? JSON.parse(row.data) : { title: "", type: "", domain: null, fields: {}, hosts: [], apps: [], app: item.app || null };
    const secrets = row?.sealed ? await open(key, row.sealed) : {};
    const data = { ...before, fields: { ...before.fields } };
    if (typeof item.title === "string") data.title = item.title.trim();
    if (!data.title) throw new Error("an item has a title");
    if (typeof item.type === "string") data.type = item.type.trim();
    if (item.domain !== undefined) data.domain = typeof item.domain === "string" && HOST.test(item.domain) ? item.domain : null;
    const values = { ...secrets };
    for (const [n, f] of Object.entries(before.fields)) if (!f.secret) values[n] = f.value;
    for (const [name, field] of Object.entries(item.fields || {})) {
      if (!FIELD.test(name)) throw new Error("a field's name is letters, digits, _, - and .");
      if (field === null) {
        delete data.fields[name];
        delete values[name];
        continue;
      }
      let value, secret;
      if (field.generate) [value, secret] = [b64u.encode(random(32)), true];
      else if (typeof field.from === "string") {
        if (!(field.from in values)) throw new Error(`no field ${field.from} to take`);
        [value, secret] = [values[field.from], field.secret ?? before.fields[field.from]?.secret ?? false];
      } else {
        if (typeof field.value !== "string") throw new Error("a field's value is text");
        [value, secret] = [field.value, field.secret ?? before.fields[name]?.secret ?? false];
      }
      values[name] = value;
      data.fields[name] = secret ? { secret: true } : { value };
    }
    if (Array.isArray(item.hosts)) {
      if (!item.hosts.every((h) => typeof h === "string" && HOST.test(h))) throw new Error("a host is a host name, like api.example.com");
      data.hosts = item.hosts;
    }
    if (Array.isArray(item.apps)) data.apps = item.apps.filter((a) => /^[a-z0-9][a-z0-9-]{0,63}$/.test(a));
    const secret = Object.fromEntries(Object.entries(data.fields).filter(([, f]) => f.secret).map(([n]) => [n, values[n]]));
    if (Object.keys(secret).length && !data.hosts.length) throw new Error("an item with a secret field is sent to one host or more");
    const sealed = Object.keys(secret).length ? await seal(key, secret) : null;
    this.sql.exec(
      "insert or replace into items (id, scope, data, sealed, user, time, made) values (?, ?, ?, ?, ?, ?, ?)",
      item.id, scope, JSON.stringify(data), sealed, held.user, now, row ? row.made : now,
    );
    return { id: item.id };
  }

  /// What `{{name}}` is for `app`, in a session: `{ id, hosts, value }` (no hosts for a plain field), or null.
  async lookup(held, app, name) {
    const scopes = this.vault.held(held);
    let row, field;
    const byId = name.match(/^([0-9a-f]{16}):(.+)$/);
    const inGroup = name.match(/^([0-9a-f]{16})\/(.+)$/);
    if (byId) {
      field = byId[2];
      row = this.sql.exec("select * from items where id = ?", byId[1]).toArray()[0];
      if (row && !scopes.includes(row.scope)) row = null;
    } else {
      field = inGroup ? inGroup[2] : name;
      const order = inGroup ? [`group:${inGroup[1]}`] : [`user:${held.user}`, "global"];
      for (const scope of order.filter((s) => scopes.includes(s))) {
        row = this.sql.exec("select * from items where scope = ? order by made, rowid", scope).toArray().find((r) => {
          const d = JSON.parse(r.data);
          return d.apps.includes(app) && field in d.fields;
        });
        if (row) break;
      }
    }
    if (!FIELD.test(field) || !row) return null;
    const data = JSON.parse(row.data);
    if (!data.apps.includes(app) || !data.fields[field]) return null;
    if (!data.fields[field].secret) return { id: row.id, hosts: [], value: data.fields[field].value };
    const secrets = await open(await itemsKey(await this.vault.scopeKey(held, row.scope)), row.sealed);
    return { id: row.id, hosts: data.hosts, value: secrets[field] };
  }

  /// `text` with each placeholder filled for a request to `host`; refuses when a secret field isn't sent there.
  async fill(held, app, text, host) {
    let out = "";
    let rest = text;
    for (;;) {
      const start = rest.indexOf("{{");
      const end = start < 0 ? -1 : rest.indexOf("}}", start + 2);
      if (end < 0) break;
      const name = rest.slice(start + 2, end);
      const found = await this.lookup(held, app, name);
      if (!found) {
        out += rest.slice(0, start + 2);
        rest = rest.slice(start + 2);
        continue;
      }
      if (found.hosts.length && !found.hosts.includes(host)) throw new Error(`${name} is sent only to ${found.hosts.join(", ")}, not ${host}`);
      out += rest.slice(0, start) + found.value;
      rest = rest.slice(end + 2);
    }
    return out + rest;
  }

  /// Makes an app's request with its fields filled in: `{ app, url, method?, headers?, body? }` (body as text, or
  /// body64) → `{ status, headers, body64 }`. Redirects come back to the app, so a secret never follows one.
  async fetch(held, { app, url, method = "GET", headers = {}, body, body64 }) {
    const target = new URL(url);
    if (target.protocol !== "https:") throw new Error("the vault sends only https requests");
    const host = target.hostname.toLowerCase();
    const filled = new Headers();
    for (const [k, v] of Object.entries(headers)) filled.set(k, await this.fill(held, app, String(v), host));
    let payload = body64 !== undefined ? b64u.decode(body64) : body;
    if (typeof payload === "string" && payload.includes("{{")) payload = await this.fill(held, app, payload, host);
    const resp = await fetch(await this.fill(held, app, url, host), { method, headers: filled, body: payload, redirect: "manual" });
    const bytes = new Uint8Array(await resp.arrayBuffer());
    if (bytes.length > LIMIT) throw new Error(`${host} answered with more than 32 MB`);
    return { status: resp.status, headers: Object.fromEntries(resp.headers), body64: b64u.encode(bytes) };
  }
}
