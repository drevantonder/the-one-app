// The vault server: every scope's keys, sealed so it can open none of them alone.
//
// Each person has an X25519 key pair. Its secret half is kept only in copies, each sealed under one client's secret
// (a passkey's PRF output, an OAuth client's refresh token) and kept by a hash of that secret; a session is a copy
// sealed under an access token, kept until the token expires. Each scope's key is sealed to the public half of every
// person who may open it, and to the install's recovery key. A request that brings an access token gets the keys it
// names from its session, and each one is recorded where its person can see it.
//
// The auth server is the only one that sees clients' secrets (it issues them), so it alone calls the routes under
// /internal/, with the kernel's shared secret. Everyone else brings an access token.

import { DurableObject } from "cloudflare:workers";
import { appKey, b64u, databaseKey, idOf, keyPair, openSealed, openUnder, random, sealTo, sealUnder } from "../shared/crypto.js";
import { bucketUrls } from "../shared/s3.js";
import { bearer, verify } from "../shared/token.js";

const NAME = /^[a-z0-9][a-z0-9-]{0,63}$/;
const SCOPE = /^(global|user:[0-9a-f]{16}|group:[0-9a-f]{16})$/;
/// How long a copy waiting for its client's first token (sealed under an authorization code) is kept.
const PENDING = 10 * 60_000;
/// How long presigned URLs last.
const URLS = 900;

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

/// The secret in a request: base64url (a PRF output) or text (a token or code), as bytes.
function secretOf(v) {
  if (typeof v !== "string" || v.length < 16) refuse(400, "a secret is at least 16 characters");
  return new TextEncoder().encode(v);
}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`, which every disk takes.
export const folderOf = (scope) => scope.replace(":", "-");

export class Vault extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`
      create table if not exists people (user text primary key, public blob not null, recovery blob not null, made integer not null);
      create table if not exists scopes (scope text primary key, recovery blob not null, made integer not null);
      create table if not exists grants (user text not null, scope text not null, sealed blob not null, primary key (user, scope));
      create table if not exists copies (id text primary key, user text not null, kind text not null, name text not null,
        client text, sealed blob not null, made integer not null, until integer);
      create index if not exists copies_user on copies (user);
      create table if not exists unseals (at integer not null, user text not null, client text, what text not null);
      create index if not exists unseals_user on unseals (user, at);
    `);
  }

  async fetch(request) {
    try {
      return await this.route(request, new URL(request.url));
    } catch (err) {
      if (err instanceof Refused) return json({ error: err.message }, err.status);
      console.error(err.stack || err);
      return json({ error: "the vault failed" }, 500);
    }
  }

  async route(request, url) {
    const path = url.pathname;
    const body = request.method === "POST" ? await request.json().catch(() => refuse(400, "the body isn't JSON")) : {};
    if (path.startsWith("/internal/")) {
      if (!this.env.KERNEL_SECRET || request.headers.get("x-kernel-secret") !== this.env.KERNEL_SECRET) refuse(403, "only the auth server asks that");
      switch (`${request.method} ${path}`) {
        case "POST /internal/passkey": return json(await this.passkey(body));
        case "POST /internal/grant": return json(await this.grant(body));
        case "POST /internal/rekey": return json(await this.rekey(body));
        case "POST /internal/scopes": return json(await this.scopesOf(body));
        case "POST /internal/forget": return json(await this.forget(body));
        case "POST /internal/recovery": return json(this.recovery());
      }
      refuse(404, "no such route");
    }
    const token = bearer(request) || refuse(401, "bring an access token");
    const claims = await this.verify(token);
    switch (`${request.method} ${path}`) {
      case "POST /keys": return json(await this.keys(token, claims, body));
      case "POST /urls": return json(await this.urls(token, claims, body));
      case "POST /share": return json(await this.share(token, claims, body));
      case "GET /copies": return json(this.copies(claims.sub));
      case "GET /unseals": return json(this.unseals(claims.sub, Number(url.searchParams.get("before")) || Date.now() + 1));
    }
    const remove = path.match(/^\/copies\/([A-Za-z0-9_-]{43})$/);
    if (request.method === "DELETE" && remove) return json(this.remove(claims.sub, remove[1]));
    refuse(404, "no such route");
  }

  // Tokens.

  async verify(token) {
    const now = Date.now();
    for (const fresh of [false, true]) {
      if (fresh || !this.jwks || this.jwksAt < now - 300_000) {
        const resp = await this.env.AUTH.fetch(new Request(`${this.env.ISSUER}/jwks`));
        if (!resp.ok) refuse(503, "the auth server didn't give its keys");
        this.jwks = await resp.json();
        this.jwksAt = now;
      }
      try {
        return await verify(token, this.jwks, this.env.ISSUER, now);
      } catch (err) {
        if (fresh || !/key the auth server/.test(err.message)) refuse(401, `that access token won't do: ${err.message}`);
      }
    }
  }

  // Copies.

  /// Opens the copy sealed under `secret`: → its row and what it holds, `{ user, key, public, scopes }`.
  async open(secret, kinds) {
    const id = await idOf(secret);
    const row = this.sql.exec("select * from copies where id = ?", id).toArray()[0];
    if (!row || !kinds.includes(row.kind) || (row.until && row.until <= Date.now())) return null;
    const plain = JSON.parse(new TextDecoder().decode(await openUnder(secret, row.sealed)));
    return { row, held: plain };
  }

  async seal(secret, { user, kind, name, client = null, held, until = null }) {
    const id = await idOf(secret);
    const sealed = await sealUnder(secret, JSON.stringify(held));
    this.sql.exec("insert or replace into copies (id, user, kind, name, client, sealed, made, until) values (?, ?, ?, ?, ?, ?, ?, ?)", id, user, kind, name, client, sealed, Date.now(), until);
    return id;
  }

  /// A passkey's copy, for `user`. Their first makes their key pair and their scope's key (and the install's, for
  /// the first person); a later one is opened from another of their passkeys, or a code from the recovery key.
  async passkey({ user, secret, name, from }) {
    if (!/^[0-9a-f]{16}$/.test(user || "")) refuse(400, "not a user id");
    secret = secretOf(secret);
    const known = this.sql.exec("select user from people where user = ?", user).toArray()[0];
    let held;
    if (!known) {
      const pair = await keyPair();
      const recovery = b64u.decode(this.env.RECOVERY_KEY || refuse(503, "the vault has no recovery key"));
      this.sql.exec("insert into people (user, public, recovery, made) values (?, ?, ?, ?)", user, pair.public, await sealTo(recovery, pair.secret), Date.now());
      const scopes = [`user:${user}`];
      if (!this.sql.exec("select scope from scopes where scope = 'global'").toArray().length) scopes.push("global");
      for (const scope of scopes) await this.makeScope(scope, user, pair.public);
      held = { user, key: b64u.encode(pair.secret), public: b64u.encode(pair.public), scopes: null };
    } else {
      const opened = from && (await this.open(secretOf(from), ["passkey", "recovery"]));
      if (!opened || opened.held.user !== user) refuse(403, "a new passkey is added with one you have already");
      held = opened.held;
      if (opened.row.kind === "recovery") this.sql.exec("delete from copies where id = ?", opened.row.id);
    }
    return { id: await this.seal(secret, { user, kind: "passkey", name: String(name || "Passkey"), held }) };
  }

  async makeScope(scope, user, publicKey) {
    const key = random(32);
    const recovery = b64u.decode(this.env.RECOVERY_KEY);
    this.sql.exec("insert into scopes (scope, recovery, made) values (?, ?, ?)", scope, await sealTo(recovery, key), Date.now());
    this.sql.exec("insert into grants (user, scope, sealed) values (?, ?, ?)", user, scope, await sealTo(publicKey, key));
  }

  /// A copy for a client being added, opened with a passkey (`from`) and sealed under its authorization code (`to`)
  /// until its first token, limited to `scopes` (all of the person's, if not given).
  async grant({ user, from, to, name, client, scopes }) {
    const opened = await this.open(secretOf(from), ["passkey"]);
    if (!opened || opened.held.user !== user) refuse(403, "that passkey isn't this person's");
    const held = { ...opened.held, scopes: narrow(opened.held.scopes, scopes) };
    return { id: await this.seal(secretOf(to), { user, kind: "pending", name: String(name || "Client"), client, held, until: Date.now() + PENDING }) };
  }

  /// Moves a client's copy from its old secret (an authorization code, or the refresh token it just used) to its new
  /// refresh token, and opens a session under the access token it was given with it, until that expires.
  async rekey({ from, to, token, until }) {
    const opened = await this.open(secretOf(from), ["pending", "client"]);
    if (!opened) return { moved: false };
    const { row, held } = opened;
    await this.seal(secretOf(to), { user: row.user, kind: "client", name: row.name, client: row.client, held });
    this.sql.exec("delete from copies where id = ?", row.id);
    if (token) await this.session(token, row, held, until);
    return { moved: true };
  }

  async session(token, row, held, until) {
    const expires = Math.min(Number(until) || 0, Date.now() + 3_600_000);
    if (!(expires > Date.now())) refuse(400, "a session needs its token's expiry");
    this.sql.exec("delete from copies where kind = 'session' and until <= ?", Date.now());
    await this.seal(secretOf(token), { user: row.user, kind: "session", name: row.name, client: row.client, held, until: expires });
  }

  /// The scopes a passkey opens, for a consent screen to offer.
  async scopesOf({ user, secret }) {
    const opened = await this.open(secretOf(secret), ["passkey"]);
    if (!opened || opened.held.user !== user) refuse(403, "that passkey isn't this person's");
    return { scopes: this.held(opened.held) };
  }

  /// Deletes the copy a secret opens (a refresh token that was revoked).
  async forget({ secret }) {
    this.sql.exec("delete from copies where id = ?", await idOf(secretOf(secret)));
    return {};
  }

  /// Everything sealed to the recovery key: each scope's key, and each person's key pair's secret half.
  recovery() {
    const scopes = this.sql.exec("select scope, recovery from scopes order by scope").toArray();
    const people = this.sql.exec("select user, recovery from people order by user").toArray();
    return {
      scopes: scopes.map((r) => ({ scope: r.scope, sealed: b64u.encode(r.recovery) })),
      people: people.map((r) => ({ user: r.user, sealed: b64u.encode(r.recovery) })),
    };
  }

  /// The scopes a copy may open.
  held(held) {
    const granted = this.sql.exec("select scope from grants where user = ?", held.user).toArray().map((r) => r.scope);
    return held.scopes ? granted.filter((s) => held.scopes.includes(s)) : granted;
  }

  copies(user) {
    const rows = this.sql.exec("select id, kind, name, client, made from copies where user = ? and kind in ('passkey', 'client') order by made, rowid", user).toArray();
    return { copies: rows };
  }

  remove(user, id) {
    const row = this.sql.exec("select user from copies where id = ?", id).toArray()[0];
    if (!row || row.user !== user) refuse(404, "no such copy");
    this.sql.exec("delete from copies where id = ?", id);
    const left = this.sql.exec("select count(*) n from copies where user = ? and kind in ('passkey', 'client')", user).one().n;
    return { left };
  }

  // Keys.

  /// What a session opens, by the token it was sealed under: → `{ held, client }`.
  async sessionOf(token) {
    const opened = await this.open(new TextEncoder().encode(token), ["session"]);
    if (!opened) refuse(403, "that token has no keys: its client signs in again");
    return opened;
  }

  async scopeKey(held, scope) {
    if (!SCOPE.test(scope || "")) refuse(400, `${scope} isn't a scope`);
    if (held.scopes && !held.scopes.includes(scope)) refuse(403, `this client may not use ${scope}`);
    const grant = this.sql.exec("select sealed from grants where user = ? and scope = ?", held.user, scope).toArray()[0];
    if (!grant) refuse(403, `${scope} isn't open to this person`);
    return openSealed({ secret: b64u.decode(held.key), public: b64u.decode(held.public) }, grant.sealed);
  }

  /// The keys a call names: `{ keys: [{ scope, app, database? }] }` → each with its `key`, and until when the
  /// session that gave them lasts.
  async keys(token, claims, { keys }) {
    if (!Array.isArray(keys) || !keys.length || keys.length > 64) refuse(400, "name 1 to 64 keys");
    const { row, held } = await this.sessionOf(token);
    const out = [];
    for (const want of keys) {
      if (!NAME.test(want.app || "")) refuse(400, `${want.app} isn't an app's name`);
      if (want.database !== undefined && !NAME.test(want.database)) refuse(400, `${want.database} isn't a database's name`);
      let key = await appKey(await this.scopeKey(held, want.scope), want.app);
      if (want.database !== undefined) key = await databaseKey(key, want.database);
      out.push({ ...pick(want), key: b64u.encode(key) });
    }
    this.record(held.user, row.client || claims.azp, out.map((k) => [k.scope, k.app, k.database].filter(Boolean).join("/")));
    return { keys: out, until: row.until };
  }

  record(user, client, what) {
    this.sql.exec("insert into unseals (at, user, client, what) values (?, ?, ?, ?)", Date.now(), user, client || null, what.join(" "));
  }

  unseals(user, before) {
    const rows = this.sql.exec("select at, client, what from unseals where user = ? and at < ? order by at desc limit 100", user, before).toArray();
    return { unseals: rows.map((r) => ({ ...r, what: r.what.split(" ") })) };
  }

  /// Presigned URLs to read and create objects, and to replace a database's `newest` (only while it's still the
  /// version named by its ETag): `{ read: [name], create: [name], replace: [{ name, etag }] }`. Each name is
  /// `<scope's folder>/<app>/…` in a scope the session opens, or `log/…`, the install's log, for one that opens
  /// Everyone's.
  async urls(token, claims, { read = [], create = [], replace = [] }) {
    if (![read, create, replace].every(Array.isArray) || read.length + create.length + replace.length > 256) refuse(400, "name at most 256 objects");
    const bucket = this.env.BUCKET ? JSON.parse(this.env.BUCKET) : refuse(503, "the install has no bucket");
    const { held } = await this.sessionOf(token);
    const scopes = this.held(held);
    const open = new Set(scopes.map(folderOf));
    const reachable = (name) => {
      const parts = String(name).split("/");
      if (parts.some((p) => !p || p === "." || p === "..")) return false;
      if (parts[0] === "log") return scopes.includes("global") && parts.length > 1;
      return open.has(parts[0]) && NAME.test(parts[1] || "") && parts.length > 2;
    };
    for (const name of [...read, ...create, ...replace.map((r) => r?.name)]) {
      if (!reachable(name)) refuse(403, `${name} isn't an object this session may reach`);
    }
    for (const r of replace) {
      if (!r.name.endsWith("/newest") || typeof r.etag !== "string" || !r.etag) refuse(403, "only a database's newest is replaced, and only by its ETag");
    }
    return { ...(await bucketUrls(bucket, { read, create, replace }, URLS)), until: Date.now() + URLS * 1000 };
  }

  /// Opens a scope to another person: `{ scope, user }`, by someone whose session opens it.
  async share(token, claims, { scope, user }) {
    const { held } = await this.sessionOf(token);
    const key = await this.scopeKey(held, scope);
    const person = this.sql.exec("select public from people where user = ?", user).toArray()[0] || refuse(404, "that person has no keys yet");
    this.sql.exec("insert or replace into grants (user, scope, sealed) values (?, ?, ?)", user, scope, await sealTo(person.public, key));
    return {};
  }
}

/// A client's scopes: those asked for, within what the passkey allows.
function narrow(allowed, asked) {
  if (!Array.isArray(asked)) return allowed;
  return allowed ? asked.filter((s) => allowed.includes(s)) : asked.filter((s) => SCOPE.test(s));
}

const pick = ({ scope, app, database }) => (database === undefined ? { scope, app } : { scope, app, database });

export default {
  fetch(request, env) {
    return env.VAULT.get(env.VAULT.idFromName("vault")).fetch(request);
  },
};
