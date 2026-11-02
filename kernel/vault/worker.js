// The vault: every scope's keys, sealed so it can open none of them alone, on an install whose people sign in with a
// passkey.
//
// Each person has an X25519 key pair. Its secret half is kept only in copies, each sealed under one client's secret
// (a passkey's PRF output, an OAuth client's refresh token) and kept by a hash of that secret; a session is a copy
// sealed under an access token, kept until the token expires or its client signs out. On an install whose people sign
// in with Google (`SIGN_IN`), a person's first copy is sealed under a secret only the vault has (`VAULT_KEY`), which it
// opens itself once the auth service says Google signed them in: so the kernel can open everyone's keys there. A try
// install's kernel (`SIGN_IN` is "try", which only `one try` sets) does the same for its one person, with no Google.
// Each scope's key is sealed to the public half of every person who may open it, and to the install's recovery key. A
// request that brings an access token gets the keys it names from its session.
//
// The auth service is the only one that sees clients' secrets (it issues them), so it alone calls the routes under
// /internal/, with the kernel's shared secret. Everyone else brings an access token.

import { DurableObject } from "cloudflare:workers";
import { b64u, databaseKey, hkdf, idOf, keyPair, openAppKey, openSealed, openUnder, random, sealAppKey, sealTo, sealUnder } from "../shared/crypto.js";
import { bucketUrls } from "../shared/s3.js";
import { Items, schema } from "./items.js";
import { bearer, Issuer } from "../shared/token.js";

/// Whether the vault opens people's keys itself, with its own key: on an install whose people sign in with Google, and a
/// try install's.
const opensItself = (env) => env.SIGN_IN === "google" || env.SIGN_IN === "try";
const NAME = /^[a-z0-9][a-z0-9-]{0,63}$/;
const SCOPE = /^(global|user:[0-9a-f]{16}|group:[0-9a-f]{16})$/;
const USER = /^[0-9a-f]{16}$/;
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

/// Where an app's sealed key is kept: `_key` in its folder, a name no database can have.
const appKeyName = (scope, app) => `${folderOf(scope)}/${app}/_key`;

export class Vault extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`
      create table if not exists people (user text primary key, public blob not null, recovery blob not null, made integer not null);
      create table if not exists scopes (scope text primary key, recovery blob not null, made integer not null);
      create table if not exists grants (user text not null, scope text not null, sealed blob not null, primary key (user, scope));
      create table if not exists copies (id text primary key, user text not null, kind text not null, name text not null,
        client text, sealed blob not null, made integer not null, until integer, line text);
      create index if not exists copies_user on copies (user);
      create table if not exists admins (scope text not null, user text not null, primary key (scope, user));
    `);
    this.sql.exec(schema);
    // A client's copies (pending, its own, and its sessions) share a line, so signing it out ends its sessions too.
    if (!this.sql.exec("select name from pragma_table_info('copies') where name = 'line'").toArray().length) this.sql.exec("alter table copies add column line text");
    // When each scope's key was settled (see `settle`): until then, a scope's key the nodes made may replace its own.
    if (!this.sql.exec("select name from pragma_table_info('scopes') where name = 'given'").toArray().length) {
      this.sql.exec("alter table scopes add column given integer");
      if (this.sql.exec("select name from sqlite_master where name = 'unseals'").toArray().length) {
        this.sql.exec(`update scopes set given = 1 where exists (select 1 from unseals where what like '%"' || scopes.scope || '/%')`);
        this.sql.exec("drop table unseals");
      }
    }
    // A hash of each scope's key, so `kernel keys` can say who may open it again without replacing it.
    if (!this.sql.exec("select name from pragma_table_info('scopes') where name = 'hash'").toArray().length) this.sql.exec("alter table scopes add column hash text");
    this.items = new Items(this.sql, this);
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
      if (!this.env.KERNEL_SECRET || request.headers.get("x-kernel-secret") !== this.env.KERNEL_SECRET) refuse(403, "only the auth service asks that");
      switch (`${request.method} ${path}`) {
        case "POST /internal/passkey": return json(await this.passkey(body));
        case "POST /internal/grant": return json(await this.grant(body));
        case "POST /internal/own": return json(await this.own(body));
        case "POST /internal/rekey": return json(await this.rekey(body));
        case "POST /internal/forget": return json(await this.forget(body));
        case "POST /internal/forget-passkeys": return json(this.forgetPasskeys(body));
        case "POST /internal/recovery": return json(this.recovery());
        case "POST /internal/recover": return json(await this.recover(body));
        case "POST /internal/scope-key": return json(await this.scopeKeyGiven(body));
        case "POST /internal/standing": return json(await this.standing(body));
        case "POST /internal/run": return json(await this.run(body));
      }
      refuse(404, "no such route");
    }
    const token = bearer(request) || refuse(401, "bring an access token");
    const claims = token.startsWith("run.") ? await this.runClaims(token) : await this.verify(token);
    switch (`${request.method} ${path}`) {
      case "GET /session": return json(await this.sessionAbout(token));
      case "POST /keys": return json(await this.keys(token, claims, body));
      case "POST /urls": return json(await this.urls(token, claims, body));
      case "POST /erase": return json(await this.erase(token, claims, body));
      case "POST /invite": return json(await this.invite(token, claims, body));
      case "POST /role": return json(this.role(token, claims, body));
      case "GET /items": return json(this.items.list((await this.sessionOf(token)).held, url.searchParams.get("scope")));
      case "POST /items": return json(await this.items.set((await this.sessionOf(token)).held, body).catch((err) => refuse(err.status || 400, err.message)));
      case "POST /fetch": return json(await this.fetchFor(token, claims, body));
      case "POST /fill": return json(await this.fillFor(token, claims, body));
      case "GET /copies": return json(this.copies(claims.sub));
    }
    const remove = path.match(/^\/copies\/([A-Za-z0-9_-]{43})$/);
    if (request.method === "DELETE" && remove) return json(this.remove(claims.sub, remove[1]));
    refuse(404, "no such route");
  }

  // Tokens.

  async verify(token) {
    this.issuer ||= new Issuer(this.env.ISSUER, () => this.env.AUTH.fetch(new Request(`${this.env.ISSUER}/jwks`)));
    return this.issuer.verify(token).catch((err) => refuse(401, `that access token won't do: ${err.message}`));
  }

  /// A run's token isn't a JWT: it's the secret its session is sealed under, so only having it opens anything.
  async runClaims(token) {
    const { row } = await this.sessionOf(token);
    return { sub: row.user, azp: row.client };
  }

  /// Whose a token's session is: who it's for, the client, and until when.
  async sessionAbout(token) {
    const { row } = await this.sessionOf(token);
    return { user: row.user, client: row.client, name: row.name, until: row.until };
  }

  /// Work a person lets run while they're away: a copy of what their session opens, limited to `scopes` and `apps`,
  /// sealed under a new secret that only the schedules service keeps. → `{ secret }`.
  async standing({ token, scopes, apps, name }) {
    await this.verify(token);
    const { row, held } = await this.sessionOf(token);
    if (!Array.isArray(apps) || !apps.length || !apps.every((a) => NAME.test(a))) refuse(400, "name the apps it may use");
    const limited = { ...held, scopes: this.held(held).filter((s) => Array.isArray(scopes) && scopes.includes(s)), apps };
    if (!limited.scopes.length) refuse(403, "this session can't let that run");
    const secret = b64u.encode(random(32));
    await this.seal(secretOf(secret), { user: row.user, kind: "standing", name: String(name || "Schedule"), client: "schedules", held: limited });
    return { secret };
  }

  /// A run of standing work: a session for it, under a new token, until `until` (an hour at most). → `{ token }`.
  async run({ secret, until }) {
    const opened = await this.open(secretOf(secret), ["standing"]);
    if (!opened) refuse(404, "that work isn't let run any more");
    const token = `run.${b64u.encode(random(32))}`;
    await this.session(token, opened.row, opened.held, Math.min(Number(until) || 0, Date.now() + 3_600_000));
    return { token, user: opened.row.user };
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

  async seal(secret, { user, kind, name, client = null, held, until = null, line = null }) {
    const id = await idOf(secret);
    const sealed = await sealUnder(secret, JSON.stringify(held));
    this.sql.exec("insert or replace into copies (id, user, kind, name, client, sealed, made, until, line) values (?, ?, ?, ?, ?, ?, ?, ?, ?)", id, user, kind, name, client, sealed, Date.now(), until, line);
    return id;
  }

  /// A passkey's copy, for `user`. Their first makes their keys (`person`); a later one is opened from another of
  /// their passkeys, or an invite link made with the recovery key.
  async passkey({ user, secret, name, from, client = null }) {
    if (opensItself(this.env)) refuse(403, "people here don't sign in with a passkey");
    if (!USER.test(user || "")) refuse(400, "not a user id");
    secret = secretOf(secret);
    const known = this.sql.exec("select user from people where user = ?", user).toArray()[0];
    let held;
    if (!known) {
      held = await this.person(user);
    } else {
      const opened = from && (await this.open(secretOf(from), ["passkey", "recovery"]));
      if (!opened || opened.held.user !== user) refuse(403, "a new passkey is added with one you have already");
      held = opened.held;
      if (opened.row.kind === "recovery") this.sql.exec("delete from copies where id = ?", opened.row.id);
    }
    return { id: await this.seal(secret, { user, kind: "passkey", name: String(name || "Passkey"), client, held }) };
  }

  /// Makes `user`'s key pair and their scope's key (and the install's, for the first person): → what their copies hold.
  async person(user) {
    const pair = await keyPair();
    const recovery = b64u.decode(this.env.RECOVERY_KEY || refuse(503, "the vault has no recovery key"));
    this.sql.exec("insert into people (user, public, recovery, made) values (?, ?, ?, ?)", user, pair.public, await sealTo(recovery, pair.secret), Date.now());
    const scopes = [`user:${user}`];
    if (!this.sql.exec("select scope from scopes where scope = 'global'").toArray().length) scopes.push("global");
    for (const scope of scopes) await this.makeScope(scope, user, pair.public);
    return { user, key: b64u.encode(pair.secret), public: b64u.encode(pair.public), scopes: null };
  }

  /// The secret a person's own copy is sealed under on an install whose people sign in with Google: only the vault
  /// can make it.
  async ownSecret(user) {
    const key = await crypto.subtle.importKey("raw", b64u.decode(this.env.VAULT_KEY || refuse(503, "the vault has no key of its own")), { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
    return new Uint8Array(await crypto.subtle.sign("HMAC", key, new TextEncoder().encode(`one/vault\0${user}`)));
  }

  /// A copy for a client being added, on an install whose people sign in with Google, once the auth service says
  /// Google signed `user` in (or a try install's, for its one person): opened with the vault's own key, and sealed
  /// under the client's authorization code (`to`) as `grant` seals it. Their first sign-in makes their keys, and opens
  /// Everyone's to them.
  async own({ user, to, name, client }) {
    if (!opensItself(this.env)) refuse(403, "people here open their keys with a passkey");
    if (!USER.test(user || "")) refuse(400, "not a user id");
    const own = await this.ownSecret(user);
    let held = (await this.open(own, ["own"]))?.held;
    if (!held) {
      if (this.sql.exec("select user from people where user = ?", user).toArray().length) refuse(409, "this person's keys were made with a passkey");
      held = await this.person(user);
      await this.everyone(held);
      await this.seal(own, { user, kind: "own", name: "Google", held });
    }
    return { id: await this.seal(secretOf(to), { user, kind: "pending", name: String(name || "Client"), client, held, until: Date.now() + PENDING, line: b64u.encode(random(16)) }) };
  }

  /// Opens Everyone's scope to someone new, with the key of anyone the vault opens itself who has it.
  async everyone(held) {
    if (this.sql.exec("select user from grants where user = ? and scope = 'global'", held.user).toArray().length) return;
    for (const { user } of this.sql.exec("select user from grants where scope = 'global'").toArray()) {
      const other = (await this.open(await this.ownSecret(user), ["own"]))?.held;
      if (!other) continue;
      const key = await this.scopeKey(other, "global");
      this.sql.exec("insert into grants (user, scope, sealed) values (?, 'global', ?)", held.user, await sealTo(b64u.decode(held.public), key));
      return;
    }
  }

  /// A copy of `user`'s key pair's secret half (`key`, opened with the recovery key, for someone who lost every
  /// passkey), sealed under their invite link's secret until it expires, for their new passkey to be added with.
  async recover({ user, key, secret, until }) {
    const [person] = this.sql.exec("select public from people where user = ?", user).toArray();
    if (!person) refuse(404, "nobody has keys as that user");
    if (!(Number(until) > Date.now())) refuse(400, "say until when it's kept");
    const held = { user, key, public: b64u.encode(person.public), scopes: null };
    return { id: await this.seal(secretOf(secret), { user, kind: "recovery", name: "Recovery", held, until: Number(until) }) };
  }

  /// A scope's key and who may open it, as the install's log says, given by an admin's machine (`kernel keys`): it's
  /// sealed to each of `people` who has keys here, and nobody else opens it here any more; `admins` (of Everyone's or
  /// a group's) may say who no longer does (`role`). A key the install's nodes made before it had a kernel replaces
  /// one this vault made while the scope's key isn't settled, so nothing was sealed under the one it replaces; after
  /// that, only the scope's own key is taken.
  async scopeKeyGiven({ scope, key, people, admins }) {
    if (!SCOPE.test(scope || "")) refuse(400, `${scope} isn't a scope`);
    const bytes = b64u.decode(String(key || ""));
    if (bytes.length !== 32) refuse(400, "a scope's key is 32 bytes");
    const hash = await keyHash(bytes);
    const [had] = this.sql.exec("select given, hash from scopes where scope = ?", scope).toArray();
    if (had?.given && !had.hash) refuse(409, `nobody has opened ${scope} here since the vault learnt to check its key: sign in, and run kernel keys again`);
    if (had?.given && had.hash !== hash) refuse(409, `${scope}'s key was settled here already, and it isn't that one`);
    if (!had?.given) {
      const recovery = b64u.decode(this.env.RECOVERY_KEY || refuse(503, "the vault has no recovery key"));
      this.sql.exec("insert or replace into scopes (scope, recovery, made, given, hash) values (?, ?, ?, ?, ?)", scope, await sealTo(recovery, bytes), Date.now(), Date.now(), hash);
    }
    const may = new Map();
    for (const user of Array.isArray(people) ? people : []) {
      const [person] = this.sql.exec("select public from people where user = ?", String(user)).toArray();
      if (person) may.set(String(user), person.public);
    }
    for (const { user } of this.sql.exec("select user from grants where scope = ?", scope).toArray()) {
      if (!may.has(user)) this.sql.exec("delete from grants where user = ? and scope = ?", user, scope);
    }
    for (const [user, publicKey] of may) this.sql.exec("insert or replace into grants (user, scope, sealed) values (?, ?, ?)", user, scope, await sealTo(publicKey, bytes));
    this.sql.exec("delete from admins where scope = ?", scope);
    if (!scope.startsWith("user:")) {
      for (const user of Array.isArray(admins) ? admins : []) if (USER.test(String(user))) this.sql.exec("insert or ignore into admins (scope, user) values (?, ?)", scope, String(user));
    }
    return { granted: [...may.keys()] };
  }

  /// Someone's role in scopes changed in the install's log, said by whoever changed it as soon as they did: `{ user,
  /// scopes, role }`. With no role, they open none of them here any more (only `kernel keys` gives one again); as a
  /// member, they no longer say who's in it; as an admin, they now do. Only an admin of the install says so, or of a
  /// group for its scope, or someone of themselves leaving a group: admins as the vault last heard from `kernel keys`
  /// and from this.
  role(token, claims, { user, scopes, role = null }) {
    if (token.startsWith("run.")) refuse(403, "work left running says nobody's role");
    if (!USER.test(user || "")) refuse(400, "not a user id");
    if (![null, "member", "admin"].includes(role)) refuse(400, "a role is admin, member or none");
    if (!Array.isArray(scopes) || !scopes.length || scopes.length > 256 || !scopes.every((s) => SCOPE.test(s || ""))) refuse(400, "name 1 to 256 scopes");
    const admin = (scope) => this.sql.exec("select 1 from admins where scope = ? and user = ?", scope, claims.sub).toArray().length > 0;
    for (const scope of scopes) {
      if (role && scope.startsWith("user:")) refuse(400, "nobody is made an admin of someone's own scope");
      const may = admin("global") || (scope.startsWith("group:") && (admin(scope) || (role === null && user === claims.sub)));
      if (!may) refuse(403, `only an admin of ${scope} says who's in it`);
    }
    for (const scope of scopes) {
      if (role === "admin") this.sql.exec("insert or ignore into admins (scope, user) values (?, ?)", scope, user);
      else this.sql.exec("delete from admins where scope = ? and user = ?", scope, user);
      if (role === null) this.sql.exec("delete from grants where scope = ? and user = ?", scope, user);
    }
    return {};
  }

  async makeScope(scope, user, publicKey) {
    const key = random(32);
    const recovery = b64u.decode(this.env.RECOVERY_KEY);
    this.sql.exec("insert into scopes (scope, recovery, made, hash) values (?, ?, ?, ?)", scope, await sealTo(recovery, key), Date.now(), await keyHash(key));
    this.sql.exec("insert into grants (user, scope, sealed) values (?, ?, ?)", user, scope, await sealTo(publicKey, key));
  }

  /// A copy for a client being added, opened with a passkey (`from`) and sealed under its authorization code (`to`)
  /// until its first token: the client is its person, and opens what they do.
  async grant({ user, from, to, name, client }) {
    const opened = await this.open(secretOf(from), ["passkey"]);
    if (!opened || opened.held.user !== user) refuse(403, "that passkey isn't this person's");
    return { id: await this.seal(secretOf(to), { user, kind: "pending", name: String(name || "Client"), client, held: opened.held, until: Date.now() + PENDING, line: b64u.encode(random(16)) }) };
  }

  /// Moves a client's copy from its old secret (an authorization code, or the refresh token it just used) to its new
  /// refresh token, and opens a session under the access token it was given with it, until that expires.
  async rekey({ from, to, token, until }) {
    const opened = await this.open(secretOf(from), ["pending", "client"]);
    if (!opened) return { moved: false };
    const { row, held } = opened;
    const line = row.line || row.id;
    await this.seal(secretOf(to), { user: row.user, kind: "client", name: row.name, client: row.client, held, line });
    this.sql.exec("delete from copies where id = ?", row.id);
    if (token) await this.session(token, { ...row, line }, held, until);
    return { moved: true };
  }

  async session(token, row, held, until) {
    const expires = Math.min(Number(until) || 0, Date.now() + 3_600_000);
    if (!(expires > Date.now())) refuse(400, "a session needs its token's expiry");
    this.sql.exec("delete from copies where kind = 'session' and until <= ?", Date.now());
    await this.seal(secretOf(token), { user: row.user, kind: "session", name: row.name, client: row.client, held, until: expires, line: row.line });
  }

  /// Deletes the copy a secret opens: a refresh token that was revoked, which ends its client's sessions too, so no
  /// node gets keys with its access tokens again (what a node already has, it keeps until they expire).
  async forget({ secret }) {
    this.forgetCopy(this.sql.exec("select id, kind, line from copies where id = ?", await idOf(secretOf(secret))).toArray()[0]);
    return {};
  }

  /// Deletes a copy, and if it's a client's, every copy of its line: its sessions.
  forgetCopy(row) {
    if (!row) return;
    this.sql.exec("delete from copies where id = ?", row.id);
    if (row.line && ["pending", "client"].includes(row.kind)) this.sql.exec("delete from copies where line = ?", row.line);
  }

  /// Deletes a person's passkeys' copies, but for those of the passkeys in `keep` (by credential id).
  forgetPasskeys({ user, keep }) {
    const rows = this.sql.exec("select id, client from copies where user = ? and kind = 'passkey'", user).toArray();
    for (const row of rows.filter((r) => !(keep || []).includes(r.client))) this.sql.exec("delete from copies where id = ?", row.id);
    return {};
  }

  /// Everything sealed to the recovery key: each scope's key, and each person's key pair's secret half. (Each app's
  /// key is in the bucket, sealed under its scope's.)
  recovery() {
    const scopes = this.sql.exec("select scope, recovery from scopes order by scope").toArray();
    const people = this.sql.exec("select user, recovery from people order by user").toArray();
    return {
      scopes: scopes.map((r) => ({ scope: r.scope, sealed: b64u.encode(r.recovery) })),
      people: people.map((r) => ({ user: r.user, sealed: b64u.encode(r.recovery) })),
    };
  }

  /// The scopes a copy may open: its person's, or those a schedule's run was let use.
  held(held) {
    const granted = this.sql.exec("select scope from grants where user = ?", held.user).toArray().map((r) => r.scope);
    return held.scopes ? granted.filter((s) => held.scopes.includes(s)) : granted;
  }

  copies(user) {
    const rows = this.sql.exec("select id, kind, name, client, made from copies where user = ? and kind in ('passkey', 'client') order by made, rowid", user).toArray();
    return { copies: rows };
  }

  remove(user, id) {
    const row = this.sql.exec("select id, user, kind, line from copies where id = ?", id).toArray()[0];
    if (!row || row.user !== user) refuse(404, "no such copy");
    this.forgetCopy(row);
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
    if (held.scopes && !held.scopes.includes(scope)) refuse(403, `this may not use ${scope}`);
    const grant = this.sql.exec("select sealed from grants where user = ? and scope = ?", held.user, scope).toArray()[0];
    if (!grant) refuse(403, `${scope} isn't open to this person`);
    const key = await openSealed({ secret: b64u.decode(held.key), public: b64u.decode(held.public) }, grant.sealed);
    if (!this.sql.exec("select hash from scopes where scope = ?", scope).toArray()[0]?.hash) this.sql.exec("update scopes set hash = ? where scope = ?", await keyHash(key), scope);
    return key;
  }

  /// The keys a call names: `{ keys: [{ scope, app, database? }] }` → each with its `key`, and until when the
  /// session that gave them lasts.
  async keys(token, claims, { keys }) {
    if (!Array.isArray(keys) || !keys.length || keys.length > 64) refuse(400, "name 1 to 64 keys");
    const { row, held } = await this.sessionOf(token);
    const out = [];
    for (const want of keys) {
      if (!NAME.test(want.app || "")) refuse(400, `${want.app} isn't an app's name`);
      mayUse(held, want.app);
      if (want.database !== undefined && !NAME.test(want.database)) refuse(400, `${want.database} isn't a database's name`);
      const scopeKey = await this.scopeKey(held, want.scope);
      await this.settle(want.scope);
      let key = await this.appKey(want.scope, scopeKey, want.app, want.database);
      if (want.database !== undefined) key = await databaseKey(key, want.database);
      out.push({ ...pick(want), key: b64u.encode(key) });
    }
    return { keys: out, until: row.until };
  }

  /// Settles a scope's key at its first use here (a key under it given out, or an item sealed under it): it's the
  /// scope's key from then on. Refused while the bucket has a scope key its nodes made before the vault had it, until
  /// an admin gives that one here (`kernel keys`), so nothing is sealed under a key their data isn't.
  async settle(scope) {
    if (this.sql.exec("select given from scopes where scope = ?", scope).toArray()[0]?.given) return;
    const name = `${folderOf(scope)}/_key`;
    const resp = await fetch((await bucketUrls(this.bucket(), { read: [name] }, 60)).read[name]);
    if (resp.ok) refuse(409, `${scope}'s key was made by its nodes before the vault had it: an admin gives it here with kernel keys`);
    if (resp.status !== 404) refuse(502, `the bucket said ${resp.status} to reading ${scope}'s key`);
    this.sql.exec("update scopes set given = ? where scope = ? and given is null", Date.now(), scope);
  }

  bucket() {
    return this.env.BUCKET ? JSON.parse(this.env.BUCKET) : refuse(503, "the install has no bucket");
  }

  /// An app's key in a scope, kept in the bucket beside its data, sealed under the scope's key: the first to need it
  /// (the vault, or a node) makes it, and whichever is written first is the key. As a node makes it (src/db.rs:
  /// `app_key`), it's random, unless `database` was written before app keys were: then it's the key derived from the
  /// scope's that its data was sealed with, so it still opens.
  async appKey(scope, scopeKey, app, database) {
    const name = appKeyName(scope, app);
    const prefix = `${folderOf(scope)}/${app}/${database}`;
    const earlier = database === undefined ? [] : [`${prefix}/newest`, `${prefix}/000000000001`];
    const urls = await bucketUrls(this.bucket(), { read: [name, ...earlier], create: [name] }, 60);
    let resp = await fetch(urls.read[name]);
    if (resp.status === 404) {
      const written = (await Promise.all(earlier.map((n) => fetch(urls.read[n]).then((r) => r.ok)))).some(Boolean);
      const key = written ? await hkdf(scopeKey, `one/app\0${app}`) : random(32);
      const made = await fetch(urls.create[name], { method: "PUT", headers: { "if-none-match": "*" }, body: await sealAppKey(scopeKey, app, key) });
      if (made.ok) return key;
      if (made.status !== 412) refuse(502, `the bucket said ${made.status} to making ${app}'s key`);
      resp = await fetch(urls.read[name]);
    }
    if (!resp.ok) refuse(502, `the bucket said ${resp.status} to reading ${app}'s key`);
    return openAppKey(scopeKey, app, new Uint8Array(await resp.arrayBuffer()));
  }

  /// An invite link's secret for someone new, called `name`, and until when it works, made by the auth service as
  /// for `kernel invite`: asked by an admin of the install (as the vault last heard who is), signed in.
  async invite(token, claims, { name }) {
    if (opensItself(this.env)) refuse(403, "people here join by signing in, with no invite");
    if (token.startsWith("run.")) refuse(403, "work left running invites nobody");
    await this.sessionOf(token);
    if (!this.sql.exec("select 1 from admins where scope = 'global' and user = ?", claims.sub).toArray().length) refuse(403, "only an admin invites someone");
    const resp = await this.env.AUTH.fetch(new Request("https://auth/internal/invite", {
      method: "POST",
      headers: { "content-type": "application/json", "x-kernel-secret": this.env.KERNEL_SECRET },
      body: JSON.stringify({ name }),
    }));
    const out = await resp.json().catch(() => ({}));
    if (!resp.ok) refuse(resp.status, out.error || "the auth service made no invite");
    return { token: out.token, until: out.until };
  }

  /// Erases an app's data in a scope by destroying its key: `{ scope, app }`, by a session that opens the scope and
  /// isn't a schedule's run. Its data stays in the bucket, which nothing can open again.
  async erase(token, claims, { scope, app }) {
    const { row, held } = await this.sessionOf(token);
    if (!NAME.test(app || "")) refuse(400, `${app} isn't an app's name`);
    if (held.apps) refuse(403, "this may not erase apps");
    await this.scopeKey(held, scope);
    const name = appKeyName(scope, app);
    const resp = await fetch((await bucketUrls(this.bucket(), { remove: [name] }, 60)).remove[name], { method: "DELETE" });
    if (!resp.ok && resp.status !== 404) refuse(502, `the bucket said ${resp.status} to erasing ${app}'s key`);
    return {};
  }

  /// Makes an app's request with its Vault fields filled in (see items.js).
  async fetchFor(token, claims, body) {
    const { row, held } = await this.sessionOf(token);
    if (!NAME.test(body.app || "")) refuse(400, "name the app whose request it is");
    mayUse(held, body.app);
    return this.items.fetch(held, body).catch((err) => refuse(400, err.message));
  }

  /// Fills `text` for a sidecar of the app `app` to send to `host` itself.
  async fillFor(token, claims, { app, host, text }) {
    const { row, held } = await this.sessionOf(token);
    if (!NAME.test(app || "") || typeof host !== "string" || typeof text !== "string") refuse(400, "give app, host and text");
    mayUse(held, app);
    const filled = await this.items.fill(held, app, text, host.toLowerCase()).catch((err) => refuse(400, err.message));
    return { text: filled };
  }

  /// Presigned URLs to read and create objects, and to replace a database's `newest` (only while it's still the
  /// version named by its ETag): `{ read: [name], create: [name], replace: [{ name, etag }] }`. Each name is
  /// `<scope's folder>/<app>/…` in a scope the session opens, or `log/…`, the install's log, for one that opens
  /// Everyone's.
  async urls(token, claims, { read = [], create = [], replace = [] }) {
    if (![read, create, replace].every(Array.isArray) || read.length + create.length + replace.length > 256) refuse(400, "name at most 256 objects");
    const bucket = this.bucket();
    const { held } = await this.sessionOf(token);
    const scopes = this.held(held);
    const open = new Set(scopes.map(folderOf));
    const reachable = (name) => {
      const parts = String(name).split("/");
      if (parts.some((p) => !p || p === "." || p === "..")) return false;
      if (parts[0] === "log") return scopes.includes("global") && !held.apps && parts.length > 1;
      return open.has(parts[0]) && NAME.test(parts[1] || "") && (!held.apps || held.apps.includes(parts[1])) && parts.length > 2;
    };
    for (const name of [...read, ...create, ...replace.map((r) => r?.name)]) {
      if (!reachable(name)) refuse(403, `${name} isn't an object this session may reach`);
    }
    for (const r of replace) {
      if (!r.name.endsWith("/newest") || typeof r.etag !== "string" || !r.etag) refuse(403, "only a database's newest is replaced, and only by its ETag");
    }
    return { ...(await bucketUrls(bucket, { read, create, replace }, URLS)), until: Date.now() + URLS * 1000 };
  }
}

/// Refuses unless a session may use `app` (standing work may use only the apps it was let run).
function mayUse(held, app) {
  if (held.apps && !held.apps.includes(app)) refuse(403, `this may not use ${app}`);
}

/// A scope's key's hash, which says whether a key given is it.
const keyHash = async (key) => b64u.encode(await hkdf(key, "one/scope-key-hash"));

const pick = ({ scope, app, database }) => (database === undefined ? { scope, app } : { scope, app, database });

export default {
  fetch(request, env) {
    return env.VAULT.get(env.VAULT.idFromName("vault")).fetch(request);
  },
};
