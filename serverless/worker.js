// The serverless node: a node of the install on Cloudflare Workers, which the gateway sends `/v1/` to by
// service binding. It holds no keys. Each request brings its person's access token, checked as every node checks one
// (src/lib.rs, which is the machine node's own Rust), and the keys of the databases its call reaches, sealed to this
// node (`x-one-keys`); one that lacks some is answered 428 with what it needs, as a node on a machine answers. Who can do
// what it reads from the install's log in the bucket (src/log.rs), and each app's latest deploy from the bucket too,
// through the URL signer's URLs, keeping what it read sealed in its Durable Object's storage, so a wake fetches only
// what's newer; it runs an app in an isolate of its own (Worker Loader, src/runtime/shim.js as its `ctx`) and writes
// what a call wrote as one step (src/db.rs). An app's requests go out from here, filled in with the Vault's items it's
// switched on for, read as any node reads them: each scope's `vault` database, opened with the keys handed for it.
//
// Calls of apps that keep to their own data run here when the install's order (src/rank.rs, set in Devices) puts this
// node before every machine, or the machines are far. Everything else goes first to the install's nodes on machines:
// calls over iroh, from this node's own endpoint, to a node that runs every scope the call reaches for its person, with
// only that call's keys, sealed to it, and never the sign-in's token; the rest as the gateway sent it (`MACHINE`). When
// none answers, this node answers what it can (the install's state, apps, widgets, badges, and any app with no sidecar)
// and says plainly that the rest needs a device that's off.
//
// Its Durable Objects are of one class, `Node`, in two parts. Each person's requests run in an object of their own
// (`user:<id>`), which alone holds what they handed and what it opened with it: their databases, the entries it read,
// their apps' isolates and its own copy of what it read from the bucket, so nothing one person's call opened is ever
// another's. The object `node` is this node as an iroh endpoint (src/net.rs), of its own key, with no socket, which
// the relay wakes with the datagrams sent to it and which sends its own back through the relay; it passes each ask it
// gets to the asker's person's object, and asks devices for calls for them, its connections living as long as it does.
// It ends itself once nobody has asked for a while, since iroh's timers would keep it awake, and billed, for good.
//
// Bindings: AUTH and SIGNER (the kernel's services), LOADER (Worker Loader), NODE (this Worker's `Node`),
// NODE_KEY (its endpoint's key, a secret), RELAY (the relay's `ServerlessNode` entrypoint), LOCATION (where objects are made),
// MACHINE (a node on a machine: a Workers VPC service; or MACHINE_URL, its address), ADDRESS (the install's) and NAME
// (its name). `built.js`, made when it's deployed, is the shim.

import { DurableObject, RpcTarget } from "cloudflare:workers";
import { initSync, verify, open, opens, refresh, query, write, close, statements, vault_items, vault_fill, vault_schema, screen, client_screen, config, prefix, following as after, read_log, groups, scopes_of, epochs as epochsOf, runners, forward, install, bundle, endpoint_of, signed, handoff, needs as needed, endpoint_start, endpoint_deliver, endpoint_ask, endpoint_close } from "./one_serverless.js";
import wasm from "./one_serverless_bg.wasm";
import { shim, compatibilityDate } from "./built.js";

initSync({ module: wasm });
const CONFIG = JSON.parse(config());

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

const kindOf = (scope) => (scope === "global" ? "global" : scope.startsWith("group:") ? "group" : "user");

/// What a person reads when something needs a device and none is on.
const OFF = "none of your devices is on right now";

/// The most a request's body may carry, and how many of each thing a person's object keeps at once: sign-ins, and the
/// databases each handed keys of.
const BODY_MOST = 4 << 20;
const SIGN_INS_MOST = 64;
const DATABASES_MOST = 512;

/// The header an ask over iroh is marked with as it's passed to a person's object: it runs there, or not at all, since
/// whoever asked tried the machines. Taken off whatever the gateway sends.
const ASKED = "x-one-asked";

// Who asked.

/// The auth service's public keys, kept five minutes or until a token names one that isn't among them.
let jwks = null;
let jwksAt = 0;

async function claimsOf(env, token) {
  const iss = `${env.ADDRESS}/api/auth`;
  for (const fresh of [false, true]) {
    if (fresh || !jwks || jwksAt < Date.now() - 300_000) {
      const r = await env.AUTH.fetch(`${iss}/jwks`);
      if (!r.ok) refuse(503, "the auth service didn't give its keys");
      jwks = await r.text();
      jwksAt = Date.now();
    }
    try {
      return JSON.parse(verify(token, jwks, iss, Date.now()));
    } catch (err) {
      if (fresh || !/key the auth service/.test(err.message)) refuse(401, `that sign-in won't do: ${err.message}`);
    }
  }
}

const tokenOf = (request) => {
  const auth = request.headers.get("authorization") || "";
  return auth.toLowerCase().startsWith("bearer ") && auth.slice(7).trim() ? auth.slice(7).trim() : null;
};

/// Sign-ins checked in this isolate before their request goes to its person's object, by a hash of their token.
const checked = new Map();

/// Whom an access token is for, as the auth service at the install's address signed it, checked once and kept in
/// `kept` (by a hash of the token) until it expires: → `{ hash, token, user, client, clientName, until }`.
async function signIn(env, token, kept) {
  const hash = btoa(String.fromCharCode(...new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(token)))));
  const now = Date.now();
  const was = kept.get(hash);
  if (was && now < was.until) return was;
  const claims = await claimsOf(env, token);
  const aud = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  // A remote MCP client's token is for the MCP Worker (`<address>/mcp`), whose device it is, and never a sign-in here.
  if (!aud.includes(env.ADDRESS)) refuse(401, `that sign-in isn't for ${env.ADDRESS}`);
  const who = { hash, token, user: claims.sub, client: claims.azp, clientName: claims.azp || "", until: claims.exp * 1000 };
  if (kept.size >= SIGN_INS_MOST * 16) {
    for (const [h, s] of kept) if (s.until <= now) kept.delete(h);
    while (kept.size >= SIGN_INS_MOST * 16) kept.delete(kept.keys().next().value);
  }
  kept.set(hash, who);
  return who;
}

/// A person's object.
const personOf = (env, user) => env.NODE.get(env.NODE.idFromName(`user:${user}`), near(env));

/// A request's body, as JSON, read only up to `BODY_MOST`.
async function bodyOf(request) {
  if (Number(request.headers.get("content-length") || 0) > BODY_MOST) refuse(413, "that's more than a request may carry");
  if (!request.body) return {};
  const reader = request.body.getReader();
  const parts = [];
  let size = 0;
  for (;;) {
    const { done, value } = await reader.read();
    if (done) break;
    size += value.byteLength;
    if (size > BODY_MOST) {
      reader.cancel().catch(() => {});
      refuse(413, "that's more than a request may carry");
    }
    parts.push(value);
  }
  const bytes = new Uint8Array(size);
  parts.reduce((at, part) => (bytes.set(part, at), at + part.byteLength), 0);
  try {
    return JSON.parse(new TextDecoder().decode(bytes));
  } catch {
    refuse(400, "the body isn't JSON");
  }
}

// The bucket, through the URL signer's presigned URLs, and what this node read of it, sealed.

/// The names of the `n` steps after `name`, if it's a database's step.
const following = (name, n) => JSON.parse(after(name, n));

const newestOf = (scope, app, database) => `${prefix(scope, app, database)}/newest`;

/// What a person's object has read from the bucket, as sealed as it was there, in its own storage: every name but a
/// database's `newest` is created once and never changed, so a copy or step kept here is never stale, and a wake reads
/// only what's newer from the bucket. It holds no keys, so this opens nothing. Kept to `SEALED_MOST`, the least lately
/// used going first; an erased app's are forgotten.
const SEALED_MOST = 1 << 28;
const PART = 1 << 20;

class Sealed {
  constructor(sql) {
    this.sql = sql;
    sql.exec("CREATE TABLE IF NOT EXISTS sealed (name TEXT, part INTEGER, bytes BLOB, PRIMARY KEY (name, part))");
    sql.exec("CREATE TABLE IF NOT EXISTS kept (name TEXT PRIMARY KEY, tag TEXT, size INTEGER, used INTEGER)");
    sql.exec("CREATE TABLE IF NOT EXISTS erased (app TEXT PRIMARY KEY)");
  }

  static keeps(name) {
    return !name.endsWith("/newest");
  }

  has(name) {
    return this.sql.exec("SELECT 1 FROM kept WHERE name = ?", name).toArray().length > 0;
  }

  get(name) {
    const [kept] = this.sql.exec("SELECT tag, size FROM kept WHERE name = ?", name).toArray();
    if (!kept) return null;
    const bytes = new Uint8Array(kept.size);
    let at = 0;
    for (const { bytes: part } of this.sql.exec("SELECT bytes FROM sealed WHERE name = ? ORDER BY part", name)) {
      bytes.set(new Uint8Array(part), at);
      at += part.byteLength;
    }
    if (at !== kept.size) return null;
    this.sql.exec("UPDATE kept SET used = ? WHERE name = ?", Date.now(), name);
    return { bytes, tag: kept.tag };
  }

  put(name, bytes, tag) {
    if (!Sealed.keeps(name) || !tag) return;
    this.sql.exec("DELETE FROM sealed WHERE name = ?", name);
    for (let at = 0, part = 0; at < bytes.length || part === 0; at += PART, part++) this.sql.exec("INSERT INTO sealed VALUES (?, ?, ?)", name, part, bytes.subarray(at, at + PART));
    this.sql.exec("INSERT OR REPLACE INTO kept VALUES (?, ?, ?, ?)", name, tag, bytes.length, Date.now());
    let [{ total }] = this.sql.exec("SELECT COALESCE(SUM(size), 0) AS total FROM kept").toArray();
    for (const old of this.sql.exec("SELECT name, size FROM kept ORDER BY used").toArray()) {
      if (total <= SEALED_MOST) break;
      this.sql.exec("DELETE FROM sealed WHERE name = ?", old.name);
      this.sql.exec("DELETE FROM kept WHERE name = ?", old.name);
      total -= old.size;
    }
  }

  /// Forgets what it kept of apps erased since it last looked (`<folder>/<app>/…`), once each: their names may be
  /// made again for an app of the same name.
  forgetErased(apps) {
    for (const app of apps) {
      if (this.sql.exec("SELECT 1 FROM erased WHERE app = ?", app).toArray().length) continue;
      const like = `%/${app.replace(/[%_\\]/g, "\\$&")}/%`;
      const inApp = "substr(name, instr(name, '/')) LIKE ? ESCAPE '\\'";
      this.sql.exec(`DELETE FROM sealed WHERE ${inApp}`, like);
      this.sql.exec(`DELETE FROM kept WHERE ${inApp}`, like);
      this.sql.exec("INSERT INTO erased VALUES (?)", app);
    }
  }
}

/// The bucket as src/db.rs reads and writes it (src/lib.rs: `Js`). A database's steps are read one after another, so
/// asking for one fetches the next few at once; each fetched object is used once, so a read after a lost race is fresh.
/// What's read or written is kept sealed (`Sealed`), and only what isn't there is fetched, its URLs signed in one
/// request to the URL signer for everything asked for in the same turn, and kept while they last.
class Bucket {
  constructor(env, sealed) {
    Object.assign(this, { env, sealed, urls: new Map(), fetched: new Map(), wanted: null });
  }

  /// Asks the URL signer, signed by this node's endpoint key: it gives URLs to the install's endpoints alone.
  async sign(fields) {
    const request = signed(this.env.NODE_KEY, "one/urls", JSON.stringify(fields), Date.now());
    const r = await this.env.SIGNER.fetch(`${this.env.ADDRESS}/urls`, { method: "POST", headers: { "content-type": "application/json" }, body: request });
    const out = await r.json().catch(() => ({}));
    if (!r.ok) throw Object.assign(new Error(`the URL signer said: ${out.error || r.status}`), { unanswered: r.status >= 500 });
    return out;
  }

  /// A URL to read `name`: those wanted in one turn are signed in one request.
  readUrl(name) {
    const kept = this.urls.get(name);
    if (kept && kept.until > Date.now() + 60_000) return kept.url;
    if (!this.wanted) {
      const wanted = (this.wanted = new Map());
      queueMicrotask(() => {
        this.wanted = null;
        const names = [...wanted.keys()];
        this.sign({ read: names }).then(
          (out) => {
            for (const n of names) {
              this.urls.set(n, { url: out.read[n], until: out.until });
              wanted.get(n).resolve(out.read[n]);
            }
          },
          (err) => wanted.forEach((w) => w.reject(err)),
        );
      });
    }
    if (!this.wanted.has(name)) {
      let resolve, reject;
      const url = new Promise((y, n) => ((resolve = y), (reject = n)));
      this.wanted.set(name, { url, resolve, reject });
    }
    return this.wanted.get(name).url;
  }

  async fetchObject(name) {
    const kept = Sealed.keeps(name) && this.sealed.get(name);
    if (kept) return kept;
    const r = await fetch(await this.readUrl(name));
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`the bucket said ${r.status} to reading ${name}`);
    const got = { bytes: new Uint8Array(await r.arrayBuffer()), tag: r.headers.get("etag") };
    this.sealed.put(name, got.bytes, got.tag);
    return got;
  }

  /// Starts fetching `names`, for a read soon.
  prefetch(names) {
    for (const n of names) if (!this.fetched.has(n)) this.fetched.set(n, this.fetchObject(n).catch(() => null));
  }

  get(name) {
    // The steps after one aren't in the bucket yet, mostly: their URLs come with its, and only those read are fetched.
    this.prefetch([name]);
    for (const n of following(name, 15)) if (!this.urls.has(n) && !this.sealed.has(n)) this.readUrl(n).catch(() => {});
    for (const n of following(name, 5)) this.prefetch([n]);
    const got = this.fetched.get(name);
    this.fetched.delete(name);
    return got;
  }

  /// Writes `bytes` to `name` at a signed URL: → its ETag, or null if the bucket refused it as already there (412).
  async put(url, headers, name, bytes) {
    this.fetched.delete(name);
    // A write with no answer may have been made, which src/lib.rs reads as `unanswered`; one refused wasn't.
    const unanswered = (message) => Object.assign(new Error(message), { unanswered: true });
    const r = await fetch(url, { method: "PUT", headers, body: bytes }).catch((err) => {
      throw unanswered(`the bucket couldn't be reached to write ${name}: ${err.message}`);
    });
    if (r.status === 412) return null;
    if (r.status >= 500) throw unanswered(`the bucket said ${r.status} to writing ${name}`);
    if (!r.ok) throw new Error(`the bucket said ${r.status} to writing ${name}`);
    return r.headers.get("etag") || "";
  }

  async create(name, bytes) {
    const out = await this.sign({ create: [name] });
    const tag = await this.put(out.create[name], { "if-none-match": "*" }, name, bytes);
    // Kept as the bucket tagged it, so reading it again from here is as reading it there; untagged, it's read there.
    if (tag) this.sealed.put(name, bytes, tag);
    return tag !== null;
  }

  wait(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }

  async replace(name, bytes, tag) {
    if (tag === null) return this.create(name, bytes);
    const out = await this.sign({ replace: [{ name, etag: tag }] });
    return (await this.put(out.replace[name], { "if-match": tag }, name, bytes)) !== null;
  }
}

/// Runs `f` on an open database once what's running on it is done: a write takes it while it waits for the bucket.
function serially(db, f) {
  const run = db.queue.then(f);
  db.queue = run.catch(() => {});
  return run;
}

/// Databases a person's object keeps open, the `OPEN_MOST` used last.
const OPEN_MOST = 4;

// A person's object: everything a request of theirs reads, opens and runs.

class Person {
  constructor(object) {
    const { env, ctx } = object;
    Object.assign(this, { env, ctx, object, owner: null });
    this.bucket = new Bucket(env, new Sealed(ctx.storage.sql));
    /// Sign-ins it checked, by a hash of their token, until they expire.
    this.signedIn = new Map();
    /// The keys handed for each sign-in's calls (`x-one-keys`), by the same hash: `{ until, keys }`, `keys` a
    /// database's (`<scope>/<app>/<database>`) keys by epoch, base64url. Forgotten when they expire, or its person loses
    /// the scope.
    this.handed = new Map();
    /// When each scope's entries were last read here, and with which keys.
    this.logsRead = new Map();
    /// Databases it has open, by where they are in the bucket, each brought up to what the bucket has when a call
    /// reaches it again: `{ handle, keys, queue, used, users }`. A call uses one only if it was handed exactly the keys
    /// it was opened with.
    this.kept = new Map();
    /// Each deploy's module, by bundle; what each app's latest deploy says it is; its person's name.
    this.modules = new Map();
    this.described = new Map();
    this.name = null;
  }

  /// Whom a request is for: an access token for this object's person. The keys it hands are kept once it's checked.
  async whom(request) {
    const token = tokenOf(request) || refuse(403, "sign in first: a serverless node takes only sign-ins");
    const who = await signIn(this.env, token, this.signedIn);
    if (!this.env.NODE.idFromName(`user:${who.user}`).equals(this.ctx.id)) refuse(403, "that sign-in is someone else's");
    this.owner ||= who.user;
    if (this.name === null) this.name = await this.nameOf(token);
    const sealed = request.headers.get("x-one-keys");
    if (sealed) this.take(who, sealed);
    return { ...who, name: this.name };
  }

  /// Its person's name, as the auth service says it.
  async nameOf(token) {
    const r = await this.env.AUTH.fetch(`${this.env.ADDRESS}/api/auth/oauth2/userinfo`, { headers: { authorization: `Bearer ${token}` } });
    const info = r.ok ? await r.json().catch(() => ({})) : {};
    return info.name || "";
  }

  /// Keeps the keys a caller handed with a sign-in's calls, no longer than its token lasts, and starts fetching what
  /// they open. Keys sealed to another node, or for another sign-in, are left.
  take(who, sealed) {
    let opened;
    try {
      opened = JSON.parse(handoff(this.env.NODE_KEY, who.token, sealed, Date.now(), who.until));
    } catch {
      return;
    }
    const now = Date.now();
    if (!this.handed.has(who.hash) && this.handed.size >= SIGN_INS_MOST) {
      for (const [h, s] of this.handed) if (s.until <= now) this.handed.delete(h);
      while (this.handed.size >= SIGN_INS_MOST) this.handed.delete(this.handed.keys().next().value);
    }
    const kept = this.handed.get(who.hash) || { until: 0, keys: new Map() };
    kept.until = Math.max(kept.until, opened.until);
    const wanted = [];
    for (const k of opened.keys) {
      const id = `${k.scope}/${k.app}/${k.database}`;
      if (!kept.keys.has(id)) {
        if (kept.keys.size >= DATABASES_MOST) continue;
        kept.keys.set(id, {});
      }
      kept.keys.get(id)[k.epoch] = k.key;
      try {
        wanted.push(newestOf(k.scope, k.app, k.database));
      } catch {}
    }
    this.handed.set(who.hash, kept);
    this.bucket.prefetch(wanted);
  }

  /// A sign-in's keys of `app`'s `database` in `scope`, by epoch, or null.
  keysOf(who, scope, app, database) {
    const kept = this.handed.get(who.hash);
    return (kept && kept.until > Date.now() && kept.keys.get(`${scope}/${app}/${database}`)) || null;
  }

  /// Each of `scopes`' current epoch, as the install's signed entries say: `{ <scope>: n }`.
  epochs(scopes) {
    return JSON.parse(epochsOf(this.owner, JSON.stringify(scopes)));
  }

  /// Says with 428 what a sign-in needs, of `wants` (`{ scope, app, database, have, missing? }`), sealed to this node:
  /// its caller sends the call again with them.
  async needs(wants) {
    const epochs = this.logsRead.has("global") ? this.epochs([...new Set(wants.map((w) => w.scope))]) : {};
    const need = JSON.parse(await needed(this.bucket, JSON.stringify(wants), JSON.stringify(epochs)));
    for (const w of wants) if (w.missing && !need.some((n) => n.scope === w.scope && n.app === w.app && n.database === w.database && n.epoch === w.missing)) need.push({ scope: w.scope, epoch: w.missing, app: w.app, database: w.database });
    this.lacking(need);
  }

  /// Says with 428 that a sign-in lacks `need` (`[{ scope, epoch, app, database }]`), keys sealed to this node.
  lacking(need) {
    const me = JSON.parse(endpoint_of(this.env.NODE_KEY));
    const list = need.map((n) => `${n.scope}'s ${n.app}/${n.database} in epoch ${n.epoch}`).join(", ");
    throw Object.assign(new Refused(428, `it needs keys it wasn't handed: ${list}`), { said: { need, to: me.key, seal: me.seal } });
  }

  /// Runs `f` with a database's keys (JSON); one that stops at an epoch it wasn't handed says so with 428.
  async withKeys(who, scope, app, database, f) {
    const handed = this.keysOf(who, scope, app, database);
    if (!handed) return this.needs([{ scope, app, database, have: [] }]);
    const keys = JSON.stringify(handed);
    try {
      return await f(keys);
    } catch (err) {
      if (err.missing === undefined) throw err;
      return this.needs([{ scope, app, database, have: Object.keys(handed).map(Number), missing: err.missing }]);
    }
  }

  // Who can do what.

  /// Reads `scopes`' entries for a request, each with the keys its person handed: a scope whose they didn't hand is
  /// asked for with 428, so its entries are never shown to someone who can't open them.
  async readLogs(who, scopes) {
    await Promise.all(scopes.map((scope) => this.withKeys(who, scope, "node", "log", async (keys) => {
      const last = this.logsRead.get(scope);
      if (last?.keys === keys && Date.now() - last.at < 2000) return;
      await read_log(this.owner, this.bucket, scope, keys);
      this.logsRead.set(scope, { keys, at: Date.now() });
    })));
  }

  /// Forgets what a sign-in was handed for scopes its person may no longer use, as soon as the install's log says so,
  /// and the sign-in itself once they're out of the install. Someone who hasn't joined yet is refused, as machines
  /// refuse them.
  forgetLost(who) {
    const { scopes, joined, removed } = JSON.parse(scopes_of(this.owner, who.user));
    if (removed || !joined) {
      this.signedIn.delete(who.hash);
      this.handed.delete(who.hash);
      refuse(403, removed ? "You were removed: an admin adds you back." : "you haven't joined yet: the person who invited you approves this device with the code it shows");
    }
    const may = new Set(scopes);
    const kept = this.handed.get(who.hash);
    if (kept) for (const id of kept.keys.keys()) if (!may.has(id.split("/")[0])) kept.keys.delete(id);
  }

  /// The install as a request's person sees it (src/lib.rs: `install`): `{ state, apps, me, epochs }`, from the
  /// entries of Everyone's, their own and the groups' they handed the keys of.
  async installFor(who) {
    const mine = ["global", `user:${who.user}`];
    await this.readLogs(who, ["global"]);
    this.forgetLost(who);
    await this.readLogs(who, [mine[1]]);
    const theirs = JSON.parse(groups(this.owner, who.user)).filter((g) => this.handed.get(who.hash)?.keys.has(`${g}/node/log`)).map((g) => this.readLogs(who, [g]).then(() => g, () => null));
    const read = [...mine, ...(await Promise.all(theirs)).filter(Boolean)];
    const view = JSON.parse(install(this.owner, who.user, who.clientName, JSON.stringify(read), Date.now()));
    this.bucket.sealed.forgetErased(view.erased || []);
    view.epochs = this.epochs(read);
    return view;
  }

  // Apps.

  moduleOf(who, hash) {
    if (!this.modules.has(hash)) {
      const got = this.withKeys(who, "global", "node", "blobs", (keys) => bundle(this.bucket, keys, hash));
      this.modules.set(hash, got);
      got.catch(() => this.modules.delete(hash));
    }
    return this.modules.get(hash);
  }

  /// An app's latest deploy, in an isolate of its own for this person alone: no network, and the node's answers handed
  /// in with each call. Another person's calls never share its module's state.
  appWorker(who, app) {
    return this.env.LOADER.get(`${this.owner}/${app.name}-${app.bundle}`, async () => ({
      compatibilityDate,
      compatibilityFlags: ["nodejs_als"],
      mainModule: "shim.js",
      modules: { "shim.js": shim, "app.js": await this.moduleOf(who, app.bundle) },
      globalOutbound: null,
    }));
  }

  /// What an app's latest deploy says it is and does (src/runtime/shim.js: `describe`).
  describe(who, app) {
    const id = `${app.name}-${app.bundle}`;
    if (!this.described.has(id)) {
      // Its bundle first: keys it needs are asked for as they are (428), which Worker Loader would hide.
      const got = this.moduleOf(who, app.bundle).then(() => this.appWorker(who, app).getEntrypoint("Call").run("describe", null, {}, null)).then((r) => r.ok);
      this.described.set(id, got);
      got.catch(() => this.described.delete(id));
    }
    return this.described.get(id);
  }

  /// A database at `at` opened with exactly `keys` (JSON) under the scope's `current` epoch: the one kept, brought up
  /// to the bucket, or a new one, kept in place of one opened with other keys that no call is using.
  async database(at, keys, current, migrations) {
    const was = this.kept.get(at);
    if (was && opens(was.handle, keys, current)) {
      was.used = Date.now();
      was.users++;
      try {
        await serially(was, () => refresh(was.handle, this.bucket, migrations));
      } catch (err) {
        was.users--;
        throw err;
      }
      return was;
    }
    const db = { handle: await open(this.owner, this.bucket, at, keys, current, migrations), keys, queue: Promise.resolve(), used: Date.now(), users: 1 };
    if (was && was.users > 0) return Object.assign(db, { own: true });
    if (was) serially(was, () => close(was.handle));
    this.kept.set(at, db);
    const oldest = [...this.kept.entries()].filter(([, d]) => d.users === 0).sort((a, b) => a[1].used - b[1].used);
    for (const [name, old] of oldest.slice(0, Math.max(0, this.kept.size - OPEN_MOST))) {
      this.kept.delete(name);
      serially(old, () => close(old.handle));
    }
    return db;
  }

  /// The scopes a call reaches: an app's, as `Run.scopes` gives them; a sidecar's, those it names, or its person's own.
  callScopes(who, view, req) {
    const app = !req.sidecar && !req.device && deployed(view, req.app);
    if (app) return new Run(this, who, view).scopes(app, req.scopes).map((h) => h.id);
    return (req.scopes || ["user"]).map((s) => (s === "user" ? `user:${who.user}` : s === "global" || String(s).includes(":") ? String(s) : `group:${s}`));
  }

  /// The keys a sign-in handed this node for `app` in `scopes`, and for any `more` (`{ scope, app, database, epoch }`)
  /// of those scopes: `[{ scope, epoch, app, database, key }]`, what's passed on with a call.
  handedFor(who, scopes, app, more = []) {
    const kept = this.handed.get(who.hash);
    const out = [];
    if (!kept || kept.until <= Date.now()) return out;
    for (const [id, byEpoch] of kept.keys) {
      const [scope, a, database] = id.split("/");
      if (!scopes.includes(scope)) continue;
      for (const [epoch, key] of Object.entries(byEpoch)) {
        const asked = more.some((n) => n.scope === scope && n.app === a && n.database === database && n.epoch === Number(epoch));
        if (a === app || asked) out.push({ scope, epoch: Number(epoch), app: a, database, key });
      }
    }
    return out;
  }

  /// Asks the install's nodes on machines that run every scope `call` reaches for its person (`runners`), in Where
  /// Things Run's order until one is reached, to run it as them, with only the keys of those scopes they handed this
  /// node, sealed to that node, never their token: → its answer, `{ ok }` or what failed as `/v1/call` says it; or null
  /// if none was reached. Keys it says it needs that this node holds go with the call again; the rest are its caller's
  /// to hand this node, with 428.
  async onMachine(who, view, req, wait) {
    const scopes = this.callScopes(who, view, req);
    const order = new Map(view.state.install.devices.map((d) => [d.key, d.order]));
    const runs = JSON.parse(runners(this.owner, who.user, JSON.stringify(scopes))).sort((a, b) => (order.get(a.key) ?? Infinity) - (order.get(b.key) ?? Infinity));
    if (!runs.length) return null;
    const until = Math.min(this.handed.get(who.hash)?.until ?? who.until, who.until);
    let more = [];
    for (;;) {
      const keys = JSON.stringify(this.handedFor(who, scopes, req.app, more));
      const messages = Object.fromEntries(runs.map((r) => [r.key, forward(this.env.NODE_KEY, who.user, who.clientName, JSON.stringify(req), r.seal, until, keys, JSON.stringify(scopes))]));
      const r = await this.object.endpoint().ask(runs.map((r) => r.key), messages, wait);
      if (r.unreached !== undefined) {
        console.log(JSON.stringify({ machine: "off", why: r.unreached }));
        return null;
      }
      if (r.connect) machineRtt = r.connect;
      const out = JSON.parse(r.answer);
      if (!out.need) return out;
      const theirs = out.need.filter((n) => scopes.includes(n.scope));
      const holds = (n) => this.keysOf(who, n.scope, n.app, n.database)?.[n.epoch] !== undefined;
      const lacking = theirs.filter((n) => !holds(n));
      if (lacking.length) this.lacking(lacking);
      const fresh = theirs.filter((n) => !more.some((m) => m.scope === n.scope && m.app === n.app && m.database === n.database && m.epoch === n.epoch));
      if (!fresh.length || theirs.length < out.need.length) return { error: out.error };
      more = [...more, ...fresh];
    }
  }
}

/// A call's databases, opened from the bucket, or kept open in its person's object, as it first reaches each.
class Call {
  constructor(person, who, app, scopes, view) {
    Object.assign(this, { person, who, app, scopes, view, dbs: new Map() });
  }

  db(scope) {
    if (!this.scopes.includes(scope)) refuse(403, `the call didn't name ${scope}`);
    if (!this.dbs.has(scope)) this.dbs.set(scope, this.open(scope));
    return this.dbs.get(scope);
  }

  async open(scope) {
    const p = this.person;
    const migrations = JSON.stringify((await p.describe(this.who, this.app)).schema?.[kindOf(scope)] || []);
    const at = prefix(scope, this.app.name, "main");
    const current = this.view.epochs[scope] ?? p.epochs([scope])[scope];
    return p.withKeys(this.who, scope, this.app.name, "main", async (keys) => ({ db: await p.database(at, keys, current, migrations), keys, current }));
  }

  /// Runs `f` on a scope's database in its turn, if it's still open with the call's keys, under the scope's current
  /// epoch as it opened, or for a write (`writes`), as it is now: a new epoch since is the call's to be handed first.
  async on(scope, f, writes = false) {
    const { db, keys, current: then } = await this.db(scope);
    return serially(db, () => {
      const current = writes ? this.person.epochs([scope])[scope] : then;
      if (!opens(db.handle, keys, current)) {
        if (!Object.hasOwn(JSON.parse(keys), current)) this.person.lacking([{ scope, epoch: current, app: this.app.name, database: "main" }]);
        refuse(503, "its database was closed while it ran: try again");
      }
      return f(db.handle);
    });
  }

  /// The Vault's items in the scopes the call named: each one's `vault` database (the node's), opened with the keys
  /// handed for it, for this call alone, as `[[scope, handle]]` (JSON); those it lacks are said with 428, all at once.
  vault() {
    const p = this.person;
    this.items ||= (async () => {
      const lacks = this.scopes.filter((scope) => !p.keysOf(this.who, scope, "node", "vault"));
      if (lacks.length) await p.needs(lacks.map((scope) => ({ scope, app: "node", database: "vault", have: [] })));
      const current = (scope) => this.view.epochs[scope] ?? p.epochs([scope])[scope];
      return Promise.all(this.scopes.map((scope) => p.withKeys(this.who, scope, "node", "vault", async (keys) => [scope, await open(p.owner, p.bucket, prefix(scope, "node", "vault"), keys, current(scope), vault_schema())])));
    })();
    return this.items.then((opened) => JSON.stringify(opened));
  }

  query(scope, sql, params) {
    return this.on(scope, (handle) => JSON.parse(query(handle, sql, JSON.stringify(params))));
  }

  /// Writes a call's appends: each scope's statements as one step. Nobody is told: this node runs only while no
  /// machine's answers, and devices read what changed from the bucket. → the databases it wrote.
  async write(appends) {
    const each = JSON.parse(statements(JSON.stringify(appends), JSON.stringify(this.scopes)));
    for (const [scope, list] of Object.entries(each)) await this.on(scope, (handle) => write(handle, this.person.bucket, JSON.stringify(list)), true);
    return Object.keys(each).map((scope) => prefix(scope, this.app.name, "main"));
  }

  close() {
    for (const opened of this.dbs.values())
      opened.then(({ db }) => {
        db.users--;
        if (db.own) serially(db, () => close(db.handle));
      }, () => {});
    this.items?.then((opened) => opened.forEach(([, handle]) => close(handle)), () => {});
  }
}

/// A request's work, as each app it runs asks the node things (src/node.rs: `internal`).
class Run {
  constructor(person, who, view) {
    Object.assign(this, { person, who, view, env: person.env, ctx: person.ctx });
  }

  /// The scope handles an app reaches for this person (src/lib.rs: `install`'s `reach`), or those of `named` (`user`,
  /// `global`, a group's id or `group:<id>`) it reaches: no other.
  scopes(app, named) {
    if (!named) return app.reach;
    if (!Array.isArray(named)) refuse(400, "scopes is a list");
    const ids = named.map((s) => String(s)).map((s) => (s === "user" ? `user:${this.who.user}` : s === "global" || s.includes(":") ? s : `group:${s}`));
    return ids.map((id) => app.reach.find((h) => h.id === id) || refuse(403, `${app.name} isn't on any of your pages in ${id}`));
  }

  /// Runs one of an app's requests (`call`, `widget`, `options`, `badge`) with `body`'s own fields, over `scopes`.
  /// A call's writes are written once it returns; work it leaves running (`ctx.waitUntil`) keeps its databases open
  /// until it says it's done. `from` is the app whose `ctx.apps` made it. → what the app gave.
  async run(app, what, fields, scopes, from = null, depth = 0) {
    if (depth > 8) refuse(400, "apps called each other too deep");
    const p = this.person;
    const d = await p.describe(this.who, app);
    if (d.uses?.sidecars?.length) refuse(503, needsDevice(d, app.name));
    if (!scopes.length) refuse(403, `${d.title || app.name} isn't on any of your pages`);
    const writes = what === "call";
    const call = new Call(p, this.who, app, scopes.map((h) => h.id), this.view);
    let finished;
    const done = new Promise((resolve) => (finished = resolve));
    let later = false;
    try {
      const body = { ...fields, me: this.view.me, now: Date.now(), scopes, read_only: !writes, token: crypto.randomUUID(), from };
      const reply = await p.appWorker(this.who, app).getEntrypoint("Call").run(what, null, body, new ForCall(this, app, call, finished, depth));
      // Keys it found it needs while it ran are asked for as they are.
      if (call.need) throw call.need;
      if (reply.error !== undefined) refuse(400, reply.error);
      if (writes) {
        const wrote = await call.write(reply.appends || []);
        later = reply.later === true;
        // What ran, for the Worker's logs: never what it read or wrote.
        console.log(JSON.stringify({ call: `${app.name}.${fields.op}`, user: this.who.user, from, wrote, ms: Date.now() - body.now }));
      }
      return reply.ok ?? null;
    } finally {
      if (later) this.ctx.waitUntil(Promise.race([done, new Promise((r) => setTimeout(r, 600_000))]).then(() => call.close()));
      else call.close();
    }
  }

  /// A call of `req.app`'s operation `req.op`, here: → `{ ok }`.
  async call(req, from = null, depth = 0) {
    const p = this.person;
    const app = deployed(this.view, req.app) || refuse(404, `no app named ${req.app}`);
    const scopes = this.scopes(app, req.scopes);
    // The keys of every database it may open, asked for at once, before it runs, unless each has its scope's current.
    const lacking = scopes.filter((h) => !Object.hasOwn(p.keysOf(this.who, h.id, app.name, "main") || {}, this.view.epochs[h.id] ?? 1));
    if (lacking.length) await p.needs(lacking.map((h) => ({ scope: h.id, app: app.name, database: "main", have: Object.keys(p.keysOf(this.who, h.id, app.name, "main") || {}).map(Number) })));
    return { ok: await this.run(app, "call", { op: req.op, input: req.input ?? {} }, scopes, from, depth) };
  }

  /// The apps a page shows and the scopes each reads (src/node.rs: `page_contexts`): with `page` (`user`,
  /// `group:<id>` or `global`), those installed there, each reaching only it; without, every app, reaching all it may.
  pages(page) {
    const at = page === "user" ? `user:${this.who.user}` : page;
    return this.view.apps
      .filter((a) => a.bundle && !a.deleted)
      .map((a) => ({ app: a, scopes: at ? (a.installed.includes(at) ? a.reach.filter((h) => h.id === at) : []) : a.reach }))
      .filter((p) => p.scopes.length);
  }

  /// Every widget of a page, drawn for this person (src/node.rs: `widgets`). One whose app needs a device says so.
  async widgets(page) {
    const key = page === `user:${this.who.user}` ? "user" : page;
    const placed = (this.view.state.arrangements?.[key]?.items || []).filter((i) => typeof i.widget === "string" && i.options && Object.keys(i.options).length);
    const each = await Promise.all(this.pages(page).map(async ({ app, scopes }) => {
      const d = await this.person.describe(this.who, app).catch(() => null);
      return Promise.all((d?.widgets || []).map(async (w) => {
        const size = w.size || "medium";
        const draw = (at, options) => this.run(app, "widget", { name: w.name, size: at, options }, scopes);
        const one = { app: app.name, widget: w.name, title: w.title, description: w.description, size };
        try {
          const root = await draw(size, {});
          const sizes = { [size]: root };
          for (const other of (w.sizes || []).filter((s) => s !== size)) await draw(other, {}).then((t) => (sizes[other] = t), () => {});
          one.sizes = sizes;
          one.root = root;
          if (w.options?.length) one.options = await this.run(app, "options", { name: w.name }, scopes).catch(() => w.options);
          const mine = {};
          for (const item of placed.filter((i) => i.app === app.name && i.widget === w.name)) {
            const at = item.size || size;
            mine[item.id || ""] = await draw(at, item.options).then((t) => ({ sizes: { [at]: t } }), (err) => ({ error: err.message }));
          }
          if (Object.keys(mine).length) one.placed = mine;
        } catch (err) {
          one.error = err.message;
          one.root = { t: "text", text: err.status === 503 ? err.message : "This widget stopped working.", tone: "muted" };
        }
        return one;
      }));
    }));
    return each.flat();
  }

  /// Each app's badge on a page, as `widgets` reaches scopes; none or 0 left out.
  async badges(page) {
    const out = {};
    await Promise.all(this.pages(page).map(async ({ app, scopes }) => {
      const d = await this.person.describe(this.who, app).catch(() => null);
      if (d?.badge !== true || d.uses?.sidecars?.length) return;
      const n = await this.run(app, "badge", {}, scopes).catch(() => 0);
      if (n > 0) out[app.name] = n;
    }));
    return out;
  }

  /// Every app in the install, with what each does for this person where it's installed for them (src/node.rs:
  /// `catalog`). What it says of itself comes from its latest deploy.
  async apps() {
    return Promise.all(this.view.apps.map(async ({ reach, ...info }) => {
      const app = deployed(this.view, info.name);
      const d = app && (await this.person.describe(this.who, app).catch(() => null));
      const mine = reach.length > 0;
      const shown = ["title", "icon", "color", "domain", "tagline", "description", "uses", "connector", "needs"];
      for (const [k, v] of Object.entries(d || {})) if (k !== "name" && k !== "schedules" && (mine || shown.includes(k))) info[k] = v;
      return info;
    }));
  }
}

/// An app in the install with a deploy, as `view` has it; or null.
const deployed = (view, name) => view.apps.find((a) => a.name === name && a.bundle && !a.deleted) || null;

/// Whether an app keeps to its own data: it uses no other app, sidecar or Vault item.
const plain = (d) => !d.uses?.apps?.length && !d.uses?.sidecars?.length && !d.vault;

/// What a person reads when an app needs a sidecar on a device and none is on.
const needsDevice = (d, name) => `${d?.title || name} runs on your devices, and ${OFF}`;

/// What the node does for an app while it runs, as it asks (src/node.rs: `internal`): its databases, other apps'
/// operations, the Vault's items it's switched on for, and its requests out, filled in from them.
class ForCall extends RpcTarget {
  constructor(run, app, call, done, depth) {
    super();
    Object.assign(this, { run, app, call, done, depth });
  }

  /// An app's request out, `{ url, method, headers, body64 }`, made here with its placeholders filled in from the
  /// call's Vault items (src/lib.rs: `vault_fill`): → `{ status, headers, body64 }`. A redirect goes back to the app,
  /// so a secret never follows one to another host.
  async fetch(q) {
    const host = new URL(q.url).hostname;
    const bytes = q.body64 ? Uint8Array.from(atob(q.body64.replaceAll("-", "+").replaceAll("_", "/")), (c) => c.charCodeAt(0)) : null;
    let text = null;
    try {
      text = bytes && new TextDecoder("utf-8", { fatal: true }).decode(bytes);
    } catch {}
    const filled = JSON.parse(vault_fill(await this.call.vault(), this.app.name, this.run.who.user, host, JSON.stringify({ url: q.url, headers: q.headers || {}, body: text })));
    const r = await fetch(filled.url, { method: q.method, headers: filled.headers, body: text === null ? bytes : filled.body, redirect: "manual" });
    const got = new Uint8Array(await r.arrayBuffer());
    let body64 = "";
    for (let i = 0; i < got.length; i += 0x8000) body64 += String.fromCharCode(...got.subarray(i, i + 0x8000));
    body64 = btoa(body64).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
    return { status: r.status, headers: Object.fromEntries(r.headers), body64 };
  }

  async ask(path, method, body) {
    try {
      const { who, view, person } = this.run;
      const q = body ? JSON.parse(typeof body === "string" ? body : new TextDecoder().decode(body)) : {};
      switch (`${method} ${path}`) {
        case "POST query":
          return Response.json({ ok: await this.call.query(q.scope, q.sql, q.params || []) });
        case "POST appends":
          await this.call.write(q.appends || []);
          return Response.json({});
        case "POST done":
          this.done();
          return Response.json({});
        case "POST call": {
          // A sidecar, or an app this node can't run, goes to a node on a machine, when one answers.
          const callee = deployed(view, q.app);
          const d = callee && !q.sidecar && !q.device ? await person.describe(who, callee) : null;
          if (!d || d.uses?.sidecars?.length) return Response.json(await machineCall(this.run, q, d));
          return Response.json(await this.run.call({ app: q.app, op: q.op, input: q.input }, this.app.name, this.depth + 1));
        }
        case "GET vault":
          return new Response(vault_items(await this.call.vault(), this.app.name, who.user), { headers: { "content-type": "application/json" } });
        case "POST fetch":
          return Response.json(await this.fetch(q));
        case "GET catalog":
          return Response.json(await this.run.apps());
        case "GET devices":
          return Response.json(view.state.install.devices);
      }
      return new Response(`a serverless node doesn't do ${path} for apps yet, and ${OFF}`, { status: 501 });
    } catch (err) {
      if (err.status === 428) this.call.need = err;
      return new Response(err.message, { status: err.status === 503 ? 503 : 400 });
    }
  }
}

/// An app's call that needs a node on a machine (a sidecar's, or one of an app that uses one), as this person.
async function machineCall(run, q, d) {
  const call = { app: q.app, op: q.op, input: q.input ?? {}, sidecar: q.sidecar ? q.app : undefined, device: q.device };
  const out = await run.person.onMachine(run.who, run.view, call, 15_000);
  if (!out) refuse(503, needsDevice(d, q.app));
  return answered(out, q.app);
}

/// What a node on a machine answered, as an app's call here takes it.
function answered(out, app) {
  if (out.ok !== undefined) return out;
  refuse(400, out.error || out.elsewhere || `${app} failed`);
}

/// An app's screen, as its page (src/screen.rs).
async function screenOf(person, who, view, app, name, params, scope) {
  const reply = await person.appWorker(who, app).getEntrypoint("Call").run("screen", name, {}, null);
  const body = reply.ok ?? '<div id="missing"></div><script>missing.innerHTML = one.notFound()</script>';
  const context = { app: app.name, screen: name, params, me: view.me, scope, config: { name: CONFIG.name, cli: CONFIG.cli, api: `http://localhost:${CONFIG.port}`, made_here: CONFIG.made_here } };
  return new Response(screen(JSON.stringify(context), body), { headers: { "content-type": "text/html; charset=utf-8" } });
}

// Its endpoint, which reaches devices over iroh.

/// How long reaching a device may take, and how long this node stays awake once nobody asks it anything.
const CONNECT = 2_000;
const IDLE = 30_000;

const hex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");
const unhex = (s) => Uint8Array.from(s.match(/../g) || [], (h) => parseInt(h, 16));
const near = (env) => (env.LOCATION ? { locationHint: env.LOCATION } : {});

// Requests.

/// Until when a node on a machine didn't answer through the tunnel, so requests meanwhile don't wait for it again.
let machineOffUntil = 0;

/// A machine whose round trip is longer than this is far (src/node.rs's FAR): this node, near the person asking, runs
/// what it can rather than wait on it.
const FAR = 250;
/// How long the last connection made to a machine's node took: one round trip, through the relay.
let machineRtt = 0;

const PLACES = { first: 0, needed: 1, last: 2 };

/// Whether an app's call this node could run goes to a machine first: one is not far, and comes at this node's place
/// in the install's order or before it (at the same place a machine costs less).
function machineFirst(view) {
  if (machineRtt > FAR) return false;
  const mine = PLACES[view.state.install.serverless?.use] ?? PLACES.needed;
  return view.state.install.devices.some((s) => (PLACES[s.use] ?? PLACES.first) <= mine);
}

/// Sends a request on to the install's node on a machine, as the gateway sent it here: its answer, or null when none
/// answers within `wait` (an answer that has begun may stream for as long as it takes).
async function fromMachine(request, env, wait = 15_000) {
  if ((!env.MACHINE && !env.MACHINE_URL) || Date.now() < machineOffUntil) return null;
  const url = new URL(request.url);
  const abort = new AbortController();
  const timer = setTimeout(() => abort.abort(), wait);
  try {
    // Through a Cloudflare Tunnel the host is only what the node sees: the gateway's, as it sent it here.
    const sent = new Request(request, { signal: abort.signal });
    const r = env.MACHINE ? await env.MACHINE.fetch(`http://${url.host}${url.pathname}${url.search}`, sent) : await fetch(`${env.MACHINE_URL}${url.pathname}${url.search}`, sent);
    // A node's own failures are JSON; the tunnel's, when no node is behind it, aren't.
    if (r.status >= 500 && !(r.headers.get("content-type") || "").includes("json")) throw new Error(`the tunnel said ${r.status}`);
    return r;
  } catch (err) {
    console.log(JSON.stringify({ machine: "off", why: String(err?.message ?? err) }));
    machineOffUntil = Date.now() + 30_000;
    return null;
  } finally {
    clearTimeout(timer);
  }
}

/// While no device is on, the events a shell listens for: none, kept open a minute, so it asks again then.
function quietEvents() {
  const enc = new TextEncoder();
  let tick;
  const body = new ReadableStream({
    start(c) {
      c.enqueue(enc.encode(": no device is on\n\n"));
      tick = setInterval(() => c.enqueue(enc.encode(":\n\n")), 15_000);
      setTimeout(() => (clearInterval(tick), c.close()), 55_000);
    },
    cancel: () => clearInterval(tick),
  });
  return new Response(body, { headers: { "content-type": "text/event-stream", "cache-control": "no-store" } });
}

/// Answers a request in its person's object. One `asked` over iroh runs here, or not at all: whoever asked tried the
/// machines.
async function route(p, request) {
  const url = new URL(request.url);
  const path = url.pathname;
  const asked = request.headers.get(ASKED) === "1";
  if (request.method === "POST" && path === "/v1/call") {
    const who = await p.whom(request);
    const req = await bodyOf(request);
    if (typeof req !== "object" || req === null || Array.isArray(req)) refuse(400, "a call is an object");
    const run = new Run(p, who, await p.installFor(who));
    const app = !req.sidecar && !req.device && deployed(run.view, req.app);
    const d = app && (await p.describe(who, app));
    // An app that keeps to its own data runs here, unless a machine comes first; the rest on a machine when one answers.
    if (d && plain(d) && (asked || !machineFirst(run.view))) return json(await run.call(req));
    const r = !asked && (await p.onMachine(who, run.view, req, 120_000));
    if (r) return json(r, r.ok !== undefined ? 200 : 400);
    if (!d || d.uses?.sidecars?.length) refuse(503, needsDevice(d, req.app));
    return json(await run.call(req));
  }
  const screenAt = path.match(/^\/v1\/screens\/([a-z0-9-]+)\/([A-Za-z0-9_-]+)$/);
  if (request.method === "GET" && screenAt) {
    const who = await p.whom(request);
    const view = await p.installFor(who);
    const app = deployed(view, screenAt[1]);
    const d = app && (await p.describe(who, app));
    const params = JSON.parse(url.searchParams.get("params") || "{}");
    if (d && plain(d) && !machineFirst(view)) return screenOf(p, who, view, app, screenAt[2], params, url.searchParams.get("scope"));
    const r = await fromMachine(request, p.env);
    if (r) return r;
    if (!d || d.uses?.sidecars?.length) refuse(503, needsDevice(d, screenAt[1]));
    return screenOf(p, who, view, app, screenAt[2], params, url.searchParams.get("scope"));
  }
  // The rest is a machine's when one answers; while none does, this node answers what it can.
  const r = await fromMachine(request, p.env, path === "/v1/events" ? 5_000 : 8_000);
  if (r) return r;
  const own = request.method === "GET" && path.startsWith("/v1/client-screens/");
  if (own || (request.method === "GET" && ["/v1/state", "/v1/apps", "/v1/widgets", "/v1/badges", "/v1/events"].includes(path))) {
    const who = await p.whom(request);
    if (path === "/v1/events") return quietEvents();
    const run = new Run(p, who, await p.installFor(who));
    if (own) {
      const name = decodeURIComponent(path.slice("/v1/client-screens/".length));
      const params = JSON.parse(url.searchParams.get("params") || "{}");
      const context = { screen: name, params, me: run.view.me, config: { name: CONFIG.name, cli: CONFIG.cli, api: `http://localhost:${CONFIG.port}`, made_here: CONFIG.made_here } };
      return new Response(client_screen(JSON.stringify(context), name), { headers: { "content-type": "text/html; charset=utf-8" } });
    }
    const page = url.searchParams.get("scope");
    if (path === "/v1/state") return json(run.view.state);
    if (path === "/v1/apps") return json(await run.apps());
    if (path === "/v1/widgets") return json(await run.widgets(page));
    return json(await run.badges(page));
  }
  refuse(503, `That needs one of your devices, and ${OFF}.`);
}

/// A failure, as the API says it.
function failure(err) {
  if (err instanceof Refused) return json({ error: err.message, ...err.said }, err.status);
  console.error(err.stack || err);
  return json({ error: String(err?.message ?? err) }, 400);
}

/// A request the gateway sent, or an ask over iroh (`asked`): its sign-in checked before anything it sends is read,
/// then answered in its person's object. One with no sign-in is a machine's, through the Tunnel, if it isn't a call.
async function toPerson(request, env, asked = false) {
  const url = new URL(request.url);
  const headers = new Headers(request.headers);
  headers.delete(ASKED);
  if (asked) headers.set(ASKED, "1");
  request = new Request(request, { headers });
  const token = tokenOf(request);
  if (!token) {
    const r = !asked && url.pathname !== "/v1/call" && !url.pathname.startsWith("/v1/screens/") && (await fromMachine(request, env));
    return r || refuse(403, "sign in first: a serverless node takes only sign-ins");
  }
  const who = await signIn(env, token, checked);
  return personOf(env, who.user).fetch(request);
}

export default {
  fetch: (request, env) => toPerson(request, env).catch(failure),
};

/// A Durable Object of the serverless node: a person's (`user:<id>`), where their requests run, or the node's
/// endpoint (`node`), which the relay wakes and which asks devices for calls.
export class Node extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.isEndpoint = ctx.id.equals(env.NODE.idFromName("node"));
    /// Datagrams it sent this turn, for the relay; asks running now, and when the last one began or ended.
    this.out = [];
    this.asking = 0;
    this.last = Date.now();
  }

  /// A request of its person's, which the Worker checked is theirs and checks again here.
  async fetch(request) {
    if (this.isEndpoint) return json({ error: "the serverless node's endpoint answers only asks over iroh" }, 400);
    this.person ||= new Person(this);
    try {
      return await route(this.person, request);
    } catch (err) {
      return failure(err);
    }
  }

  /// The node's endpoint object, which a person's object asks devices through.
  endpoint() {
    return this.env.NODE.get(this.env.NODE.idFromName("node"), near(this.env));
  }

  only() {
    if (!this.isEndpoint) throw new Error("only the serverless node's endpoint does that");
  }

  /// The relay, by its `ServerlessNode` entrypoint: sending as this node, and who's here.
  relay() {
    return this.env.RELAY;
  }

  /// Its endpoint, started once in the object's life: the first packet or ask after it slept waits for this.
  start() {
    this.only();
    if (!this.started) {
      const began = Date.now();
      const send = (dst, seg, contents) => {
        this.out.push([hex(dst), seg, contents]);
        if (this.out.length > 1) return;
        queueMicrotask(() => {
          const items = this.out;
          this.out = [];
          this.sent = this.relay().fromNode(items).catch(() => {});
          this.ctx.waitUntil(this.sent);
        });
      };
      this.started = endpoint_start(unhex(this.env.NODE_KEY), send, (remote, message) => this.asked(remote, message)).then((key) => {
        console.log(JSON.stringify({ woke: key.slice(0, 8), ms: Date.now() - began }));
        return key;
      });
      this.started.catch(() => (this.started = null));
      this.awake();
    }
    return this.started;
  }

  /// Datagrams the relay was handed for it: `[src, segment size, contents]` each.
  async deliver(batch) {
    await this.start();
    for (const [src, seg, contents] of batch) endpoint_deliver(unhex(src), seg, contents);
  }

  /// Asks the first of `to` (keys, in the order to try them) that's connected to the relay, and reached, `message`:
  /// one for all, or one for each by key. → `{ answer, by, connect }`, `connect` how long making a new connection took
  /// (0 if one was open); or `{ unreached }`, why none was.
  async ask(to, message, wait) {
    this.only();
    this.asking++;
    this.awake();
    try {
      // Only those connected to the relay can be reached from here; with none, it needn't start its endpoint.
      const here = new Set(await this.relay().connected(to));
      if (here.size) await this.start();
      const why = [];
      for (const key of to) {
        if (!here.has(key)) continue;
        const said = typeof message === "string" ? message : message[key];
        if (typeof said !== "string") continue;
        try {
          const { answer, connect } = await endpoint_ask(key, said, CONNECT, wait);
          return { answer, by: key, connect };
        } catch (err) {
          if (!err.unreached) throw err;
          why.push(`${key.slice(0, 8)} ${err.message}`);
        }
      }
      return { unreached: why.join("; ") || "none of them is connected" };
    } finally {
      this.asking--;
      this.awake();
    }
  }

  /// What another endpoint asks it on a stream: a call, run in its person's object as `/v1/call` there runs it. A
  /// notice needs no answer: this node keeps nothing a notice would change.
  async asked(remote, text) {
    this.asking++;
    this.awake();
    try {
      const msg = JSON.parse(text);
      if (msg.kind === "notice") return null;
      if (msg.kind !== "ask" || !msg.body?.call) return JSON.stringify({ error: "the serverless node answers calls" });
      const { me, keys, ...call } = msg.body.call;
      const headers = { "content-type": "application/json" };
      if (me?.token) headers.authorization = `Bearer ${me.token}`;
      if (keys) headers["x-one-keys"] = keys;
      const request = new Request(`${this.env.ADDRESS}/v1/call`, { method: "POST", headers, body: JSON.stringify(call) });
      return await (await toPerson(request, this.env, true).catch(failure)).text();
    } catch (err) {
      return JSON.stringify({ error: String(err?.message ?? err) });
    } finally {
      this.asking--;
      this.awake();
    }
  }

  /// iroh always has a timer pending, which keeps the object in memory, and billed: so once nobody has asked anything
  /// for IDLE, an alarm ends it, and the next packet starts it again, a new connection for whoever calls. Packets alone
  /// don't keep it: connections' keep-alives would, forever.
  awake() {
    this.last = Date.now();
    if (!this.alarmAt || this.alarmAt < this.last + this.idle() / 2) this.arm(this.last + this.idle());
  }

  idle() {
    return Number(this.env.IDLE) || IDLE;
  }

  arm(at) {
    this.alarmAt = at;
    this.ctx.storage.setAlarm(at);
  }

  async alarm() {
    this.alarmAt = null;
    if (!this.started) return;
    if (this.asking) return this.arm(Date.now() + this.idle());
    if (Date.now() - this.last < this.idle() - 1_000) return this.arm(this.last + this.idle());
    // Its connections' ends reach their peers before it does.
    await endpoint_close();
    await this.sent;
    console.log(JSON.stringify({ sleeps: true }));
    this.ctx.abort("idle");
  }
}
