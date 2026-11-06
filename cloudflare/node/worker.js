// The Cloudflare node: a node of the network on Cloudflare Workers, which client devices ask over iroh as they ask
// any node, last in run priority's order: when none of their person's machines answers. It holds no keys. Each
// request is from the device its connection proved (or the device that vouched for that session's key, as nodes take
// one: `x-one-endpoint`), says whose it is (`x-one-user`), which the network's entries must list that device as, and
// brings the keys of the databases its call reaches, sealed to this node for that device (`x-one-keys`); one that lacks
// some is answered 428 with what it needs, as a node on a machine answers. Who can do what it reads from the network's
// log in the bucket and asks of the scope database it builds from it (src/lib.rs, the machine node's own Rust), and
// each app's latest deploy from the bucket too, through the URL signer's URLs, keeping what it read sealed in its
// Durable Object's storage, so a wake fetches only what's newer; it runs an app in an isolate of its own (Worker Loader, src/app/runtime/shim.js as its `ctx`) and writes
// what a call wrote as one step (src/database/stored.rs). An app's requests go out from here, filled in with the Vault's items it's
// switched on for, read as any node reads them: each scope's `vault` database, opened with the keys handed for it.
//
// It answers what it can itself (the network's state, apps, widgets, badges, screens, and the calls of apps that keep
// to their own data). A call of an app that needs a machine goes on to the network's nodes on machines over iroh, from
// this node's own endpoint, to a node that runs every scope the call reaches for its person, with only that call's
// keys, sealed to it; with none answering, and for anything else only a machine has, it says plainly that it needs a
// device that's off.
//
// Its Durable Objects are of one class, `Node`, in two parts. Each person's requests run in an object of their own
// (`user:<id>`), which alone holds what they handed and what it opened with it: their databases, the entries it read,
// their apps' isolates and its own copy of what it read from the bucket, so nothing one person's call opened is ever
// another's. The object `node` is this node as an iroh endpoint (src/net.rs), of its own key, with no socket, which
// the relay wakes with the datagrams sent to it and which sends its own back through the relay; it passes each request
// it gets to the asker's person's object, and asks devices for calls for them, its connections living as long as it does.
// It ends itself once nobody has asked for a while, since iroh's timers would keep it awake, and billed, for good.
//
// Bindings: SIGNER (the URL signer), LOADER (Worker Loader), NODE (this Worker's `Node`), NODE_KEY (its endpoint's
// key, a secret), RELAY (the relay's `CloudflareNode` entrypoint), LOCATION (where objects are made), ADDRESS (the
// network's), NAME (its name) and AI (Workers AI). `built.js`, made when it's deployed, is the shim, and the services built
// for Cloudflare it carries (`service-<app>.js`), each run here once an admin switches it on.

import { DurableObject, RpcTarget } from "cloudflare:workers";
import { initSync, check_certificate, open, opens, refresh, query, may, write, close, statements, vault_items, vault_fill, vault_schema, screen, config, prefix, following as after, read_scope, groups, scopes_of, epochs as epochsOf, runners, forward, network, bundle, endpoint_of, signed, handoff, epoch_name, app_key_name, needs as needed, endpoint_start, endpoint_deliver, endpoint_ask, endpoint_close, pin_root, sealed_global_steps } from "./one_cloudflare_node.js";
import wasm from "./one_cloudflare_node_bg.wasm";
import { shim, compatibilityDate, services } from "./built.js";
import { BLOBS, CODES, LOG, MOST_URLS, NODE_APP, READS, URLS, VAULT } from "./protocol.js";

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

/// An answer named by its content, but for a page's nonce, as a node on a machine names it (src/screen.rs's `tag`): to
/// a client that has it already, only that.
async function tagged(request, body, type) {
  const nonce = body.split("'nonce-")[1]?.slice(0, 32);
  const content = nonce ? body.replaceAll(nonce, "") : body;
  const digest = new Uint8Array(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(content)));
  const tag = `"${[...digest.slice(0, 16)].map((b) => b.toString(16).padStart(2, "0")).join("")}"`;
  if (request.headers.get("if-none-match") === tag) return new Response(null, { status: 304, headers: { etag: tag } });
  return new Response(body, { headers: { "content-type": type, etag: tag } });
}
const screenPage = (request, html) => tagged(request, html, "text/html; charset=utf-8");

const kindOf = (scope) => (scope === "global" ? "global" : scope.startsWith("group:") ? "group" : "user");

const OFF = "none of your devices is on right now";

const BODY_MOST = 4 << 20;
const SIGN_INS_MOST = 64;
const DATABASES_MOST = 512;

/// Who a client's request is from, as the endpoint object proved it and passes it to a person's object: `{ device,
/// until }`. Only the endpoint object sets it: nothing else reaches a person's object.
const FROM = "x-one-from";
/// Whose a request says it is: the person's object it goes to, which checks the network lists its device as theirs.
const USER = "x-one-user";
/// How long the keys a request hands are kept, at most: they come again with the next.
const HANDED_FOR = 3_600_000;

const personOf = (env, user) => env.NODE.get(env.NODE.idFromName(`user:${user}`), near(env));

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

/// The bucket as src/database/stored.rs reads and writes it (src/lib.rs: `Js`). A database's steps are read one after another, so
/// asking for one fetches the next few at once; each fetched object is used once, so a read after a lost race is fresh.
/// What's read or written is kept sealed (`Sealed`), and only what isn't there is fetched, its URLs signed in one
/// request to the URL signer for everything asked for in the same turn, and kept while they last.
class Bucket {
  constructor(env, sealed) {
    Object.assign(this, { env, sealed, urls: new Map(), fetched: new Map(), wanted: null });
  }

  /// Asks the URL signer, signed by this node's endpoint key: it gives URLs to the network's endpoints alone. With what
  /// this build reads of who's who (`READS`), which a write must.
  async sign(fields) {
    const request = signed(this.env.NODE_KEY, URLS, JSON.stringify({ ...fields, reads: READS }), Date.now());
    const r = await this.env.SIGNER.fetch(`${this.env.ADDRESS}/urls`, { method: "POST", headers: { "content-type": "application/json" }, body: request });
    const out = await r.json().catch(() => ({}));
    if (!r.ok) throw Object.assign(new Error(`the URL signer said: ${out.error || r.status}`), { unanswered: r.status >= 500 });
    return out;
  }

  readUrl(name) {
    const kept = this.urls.get(name);
    if (kept && kept.until > Date.now() + 60_000) return kept.url;
    if (!this.wanted) {
      const wanted = (this.wanted = new Map());
      queueMicrotask(() => {
        this.wanted = null;
        const all = [...wanted.keys()];
        for (let at = 0; at < all.length; at += MOST_URLS) {
          const names = all.slice(at, at + MOST_URLS);
          this.sign({ read: names }).then(
            (out) => {
              for (const n of names) {
                this.urls.set(n, { url: out.read[n], until: out.until });
                wanted.get(n).resolve(out.read[n]);
              }
            },
            (err) => names.forEach((n) => wanted.get(n).reject(err)),
          );
        }
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

  prefetch(names) {
    for (const n of names) if (!this.fetched.has(n)) this.fetched.set(n, this.fetchObject(n).catch(() => null));
  }

  get(name) {
    // One fetched ahead that wasn't there, or whose fetch failed, may be there now: it's fetched again, never kept
    // missing, and a fetch that fails now says so, rather than that it isn't there.
    const ahead = this.fetched.get(name);
    this.fetched.delete(name);
    const got = ahead ? ahead.then((o) => o ?? this.fetchObject(name)) : this.fetchObject(name);
    // The steps after one aren't in the bucket yet, mostly: their URLs come with its, and only those read are fetched.
    // A step's signature is fetched when it's read.
    for (const n of following(name, 15)) if (!this.urls.has(n) && !this.sealed.has(n)) this.readUrl(n).catch(() => {});
    for (const n of following(name, 5)) if (!n.endsWith(".sig")) this.prefetch([n]);
    return got;
  }

  /// The objects directly in `folder`, as `[name, when the bucket wrote it (ms)]`, a page at a time from URLs the URL
  /// signer gives: a database's copies, for a reader whose steps were deleted (src/database/stored.rs `gap`).
  async objects(folder) {
    const unanswered = (message) => Object.assign(new Error(message), { unanswered: true });
    const unescape = (t) => t.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&quot;/g, '"').replace(/&apos;/g, "'").replace(/&amp;/g, "&");
    const out = [];
    let token = null;
    for (;;) {
      const url = (await this.sign({ list: [{ prefix: `${folder}/`, delimiter: "/", token }] })).list[0];
      const r = await fetch(url).catch((err) => {
        throw unanswered(`the bucket couldn't be reached to list ${folder}: ${err.message}`);
      });
      if (r.status >= 500) throw unanswered(`the bucket said ${r.status} to listing ${folder}`);
      if (!r.ok) throw new Error(`the bucket said ${r.status} to listing ${folder}`);
      const text = await r.text();
      for (const [, item] of text.matchAll(/<Contents>([\s\S]*?)<\/Contents>/g)) {
        const key = unescape(item.match(/<Key>([\s\S]*?)<\/Key>/)?.[1] ?? "");
        const name = key.startsWith(`${folder}/`) ? key.slice(folder.length + 1) : "";
        if (name && !name.includes("/")) out.push([name, Date.parse(item.match(/<LastModified>(.*?)<\/LastModified>/)?.[1] ?? "") || 0]);
      }
      const next = text.match(/<NextContinuationToken>(.*?)<\/NextContinuationToken>/)?.[1];
      if (!text.includes("<IsTruncated>true</IsTruncated>") || !next) return out;
      token = unescape(next);
    }
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

const OPEN_MOST = 4;

class Person {
  constructor(object) {
    const { env, ctx } = object;
    Object.assign(this, { env, ctx, object, owner: null });
    this.bucket = new Bucket(env, new Sealed(ctx.storage.sql));
    /// The keys handed for each device's requests (`x-one-keys`), by its key: `{ until, keys }`, `keys` a
    /// database's (`<scope>/<app>/<database>`) keys by epoch, base64url. Forgotten when they expire, or its person loses
    /// the scope.
    this.handed = new Map();
    /// When each scope's entries were last read here, and with which keys.
    this.scopesRead = new Map();
    /// Databases it has open, by where they are in the bucket, each brought up to what the bucket has when a call
    /// reaches it again: `{ handle, keys, queue, used, users }`. A call uses one only if it was handed exactly the keys
    /// it was opened with.
    this.kept = new Map();
    /// Each deploy's module, by bundle; what each app's latest deploy says it is.
    this.modules = new Map();
    this.described = new Map();
  }

  /// Whom a request is from: the device the endpoint object proved, for this object's person, until when its word
  /// lasts (a session's certificate's). Whether the network lists it as theirs is checked as soon as the network's
  /// entries are read (`networkFor`), before anything else of theirs is: until then the keys it hands only read those
  /// entries, and are kept, and what they open fetched, only once it's checked (`forgetLost`). Keys sealed to another
  /// node, or for another device, are left.
  whom(request) {
    const from = JSON.parse(request.headers.get(FROM) || "null");
    const user = request.headers.get(USER) || "";
    if (!from?.device || !this.env.NODE.idFromName(`user:${user}`).equals(this.ctx.id)) refuse(403, "that request is someone else's");
    this.owner ||= user;
    const who = { hash: from.device, device: from.device, user, clientName: "", until: Math.min(from.until ?? Infinity, Date.now() + HANDED_FOR) };
    const sealed = request.headers.get("x-one-keys");
    if (sealed) {
      try {
        who.offered = JSON.parse(handoff(this.env.NODE_KEY, who.device, sealed, Date.now(), who.until));
      } catch {}
    }
    return who;
  }

  take(who, opened) {
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

  /// A sign-in's keys of `app`'s `database` in `scope`, by epoch, or null. Before its device is checked, of the
  /// network's own entries, those it hands now too, which check it.
  keysOf(who, scope, app, database) {
    const kept = this.handed.get(who.hash);
    const found = (kept && kept.until > Date.now() && kept.keys.get(`${scope}/${app}/${database}`)) || null;
    const offered = scope === "global" && app === NODE_APP && database === LOG ? (who.offered?.keys || []).filter((k) => k.scope === scope && k.app === app && k.database === database) : [];
    if (!offered.length) return found;
    const keys = { ...found };
    for (const k of offered) keys[k.epoch] = k.key;
    return keys;
  }

  epochs(scopes) {
    return JSON.parse(epochsOf(this.owner, JSON.stringify(scopes)));
  }

  /// Says with 428 what a device (`who`) needs, of `wants` (`{ scope, app, database, have, missing? }`), sealed to
  /// this node: its caller sends the call again with them.
  async needs(wants, who) {
    const epochs = this.scopesRead.has("global") ? this.epochs([...new Set(wants.map((w) => w.scope))]) : {};
    const need = JSON.parse(await needed(this.bucket, JSON.stringify(wants), JSON.stringify(epochs)));
    // The epoch it stopped at, and those it was handed and read with, which a client hands in place of what it did.
    for (const w of wants) for (const epoch of [w.missing, ...(w.missing ? w.have || [] : [])]) if (epoch && !need.some((n) => n.scope === w.scope && n.app === w.app && n.database === w.database && n.epoch === epoch)) need.push({ scope: w.scope, epoch, app: w.app, database: w.database });
    this.lacking(need, who);
  }

  /// Says with 428 that a device (`who`) lacks `need` (`[{ scope, epoch, app, database }]`), keys sealed to this node,
  /// with the logs' keys it handed, which every request here reads: a client hands a kind of request what its last 428
  /// said, and no more.
  lacking(need, who) {
    for (const [id, byEpoch] of this.handed.get(who.hash)?.keys || []) {
      const [scope, app, database] = id.split("/");
      if (app !== NODE_APP || database !== LOG) continue;
      for (const epoch of Object.keys(byEpoch).map(Number)) if (!need.some((n) => n.scope === scope && n.app === app && n.database === database && n.epoch === epoch)) need.push({ scope, epoch, app, database });
    }
    const me = JSON.parse(endpoint_of(this.env.NODE_KEY));
    const list = need.map((n) => `${n.scope}'s ${n.app}/${n.database} in epoch ${n.epoch}`).join(", ");
    throw Object.assign(new Refused(428, `it needs keys it wasn't handed: ${list}`), { said: { need, to: me.key, seal: me.seal } });
  }

  /// Runs `f` with a database's keys (JSON); one that stops at an epoch it wasn't handed says so with 428.
  async withKeys(who, scope, app, database, f) {
    const handed = this.keysOf(who, scope, app, database);
    if (!handed) return this.needs([{ scope, app, database, have: [] }], who);
    const keys = JSON.stringify(handed);
    try {
      return await f(keys);
    } catch (err) {
      if (err.missing === undefined) throw err;
      return this.needs([{ scope, app, database, have: Object.keys(handed).map(Number), missing: err.missing }], who);
    }
  }

  /// Reads `scopes`' steps for a request, each with the keys its person handed: a scope whose they didn't hand is
  /// asked for with 428, so its steps are never shown to someone who can't open them.
  async readScopes(who, scopes) {
    await Promise.all(scopes.map((scope) => this.withKeys(who, scope, NODE_APP, LOG, async (keys) => {
      const last = this.scopesRead.get(scope);
      if (last?.keys === keys && Date.now() - last.at < 2000) return;
      await read_scope(this.owner, this.bucket, scope, keys);
      this.scopesRead.set(scope, { keys, at: Date.now() });
    })));
  }

  /// Refuses a device the network's log doesn't list as its person's, forgetting what it handed, and forgets what a
  /// device was handed for scopes its person may no longer use, as soon as the log says so.
  forgetLost(who) {
    const { scopes, joined, removed, device, runs } = JSON.parse(scopes_of(this.owner, who.user, who.device));
    if (removed || !joined || device == null) {
      this.handed.delete(who.hash);
      if (removed) refuse(403, "You were removed: an admin adds you back.");
      // As a node on a machine says it, so a device let in a moment ago asks again once this node has read that.
      throw Object.assign(new Refused(403, "this isn't one of your devices: approve it on one that is"), { said: { code: CODES.notLetIn } });
    }
    if (who.offered) this.take(who, who.offered);
    who.offered = null;
    who.clientName = device;
    const may = new Set(scopes);
    const kept = this.handed.get(who.hash);
    if (kept) for (const id of kept.keys.keys()) if (!may.has(id.split("/")[0])) kept.keys.delete(id);
    return runs;
  }

  /// The network as a request's person sees it (src/lib.rs: `network`): `{ state, apps, me, epochs }`, from the
  /// entries of Everyone's, their own and the groups' they handed the keys of. Their device is checked first.
  async networkFor(who) {
    await this.readScopes(who, ["global"]);
    const runs = this.forgetLost(who);
    // Their own log's keys are asked for only if they chose this node to run their own things; it and their groups'
    // are read whenever they're handed.
    const own = `user:${who.user}`;
    if (runs.includes(own) && !this.keysOf(who, own, NODE_APP, LOG)) await this.needs([{ scope: own, app: NODE_APP, database: LOG, have: [] }], who);
    const handed = (s) => this.handed.get(who.hash)?.keys.has(`${s}/node/log`);
    const theirs = [own, ...JSON.parse(groups(this.owner, who.user))].filter(handed).map((s) => this.readScopes(who, [s]).then(() => s, () => null));
    const read = ["global", ...(await Promise.all(theirs)).filter(Boolean)];
    const view = JSON.parse(network(this.owner, who.user, who.clientName, JSON.stringify(read), Date.now()));
    this.bucket.sealed.forgetErased(view.erased || []);
    view.epochs = this.epochs(read);
    return view;
  }

  moduleOf(who, hash) {
    if (!this.modules.has(hash)) {
      const got = this.withKeys(who, "global", NODE_APP, BLOBS, (keys) => bundle(this.bucket, keys, hash));
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

  /// What an app's latest deploy says it is and does (src/app/runtime/shim.js: `describe`).
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
    const db = { handle: await open(this.owner, this.bucket, at, keys, current, migrations, this.env.NODE_KEY), keys, queue: Promise.resolve(), used: Date.now(), users: 1 };
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

  /// The scopes a call reaches: an app's, as `Run.scopes` gives them; a service's, those it names, or its person's own.
  callScopes(who, view, req) {
    const app = !req.service && !req.device && deployed(view, req.app);
    if (app) return new Run(this, who, view).scopes(app, req.scopes).map((h) => h.id);
    return (req.scopes || ["user"]).map((s) => (s === "user" ? `user:${who.user}` : s === "global" || String(s).includes(":") ? String(s) : `group:${s}`));
  }

  /// The keys a device handed this node for `app` in `scopes`, and for any `more` (`{ scope, app, database, epoch }`)
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

  /// Asks the network's nodes on machines that run every scope `call` reaches for its person (`runners`), in Where
  /// Things Run's order until one is reached, to run it as them, with only the keys of those scopes they handed this
  /// node, sealed to that node: → its answer, `{ ok }` or what failed as `/v1/call` says it; or null
  /// if none was reached. Keys it says it needs that this node holds go with the call again; the rest are its caller's
  /// to hand this node, with 428.
  async onMachine(who, view, req, wait) {
    const scopes = this.callScopes(who, view, req);
    const order = new Map(view.state.network.devices.map((d) => [d.key, d.order]));
    const runs = JSON.parse(runners(this.owner, who.user, JSON.stringify(scopes))).sort((a, b) => (order.get(a.key) ?? Infinity) - (order.get(b.key) ?? Infinity));
    if (!runs.length) return null;
    // The keys it was handed earlier count only while they last: once they've run out, a call passed on says no more
    // than this request's own word, and the machine asks for what it needs (428), never refusing it as expired.
    const kept = this.handed.get(who.hash);
    const until = kept && kept.until > Date.now() ? Math.min(kept.until, who.until) : who.until;
    let more = [];
    for (;;) {
      const keys = JSON.stringify(this.handedFor(who, scopes, req.app, more));
      const messages = Object.fromEntries(runs.map((r) => [r.key, forward(this.env.NODE_KEY, who.user, who.clientName, JSON.stringify(req), r.seal, until, keys, JSON.stringify(scopes))]));
      const r = await this.object.endpoint().ask(runs.map((r) => r.key), messages, wait);
      if (r.unreached !== undefined) {
        console.log(JSON.stringify({ machine: "off", why: r.unreached }));
        return null;
      }
      const out = JSON.parse(r.answer);
      if (!out.need) return out;
      const theirs = out.need.filter((n) => scopes.includes(n.scope));
      const holds = (n) => this.keysOf(who, n.scope, n.app, n.database)?.[n.epoch] !== undefined;
      const lacking = theirs.filter((n) => !holds(n));
      if (lacking.length) this.lacking(lacking, who);
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
        if (!Object.hasOwn(JSON.parse(keys), current)) this.person.lacking([{ scope, epoch: current, app: this.app.name, database: "main" }], this.who);
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
      const lacks = this.scopes.filter((scope) => !p.keysOf(this.who, scope, NODE_APP, VAULT));
      if (lacks.length) await p.needs(lacks.map((scope) => ({ scope, app: NODE_APP, database: VAULT, have: [] })), this.who);
      const current = (scope) => this.view.epochs[scope] ?? p.epochs([scope])[scope];
      return Promise.all(this.scopes.map((scope) => p.withKeys(this.who, scope, NODE_APP, VAULT, async (keys) => [scope, await open(p.owner, p.bucket, prefix(scope, NODE_APP, VAULT), keys, current(scope), vault_schema(), p.env.NODE_KEY)])));
    })();
    return this.items.then((opened) => JSON.stringify(opened));
  }

  query(scope, sql, params) {
    return this.on(scope, (handle) => JSON.parse(query(handle, sql, JSON.stringify(params))));
  }

  /// Whether the caller may do each permission to each object of `asks` (`[[permission, object]]`), in order: the
  /// core's rows, and the app's own types and its rows in `scope` (src/node/calls.rs: `/may`).
  async may(scope, asks) {
    if (!this.scopes.includes(scope)) refuse(403, `the call didn't name ${scope}`);
    const types = (await this.person.describe(this.who, this.app)).types;
    const ask = (handle) => JSON.parse(may(this.person.owner, handle, this.who.user, JSON.stringify(asks), JSON.stringify(types || {}), Date.now()));
    return types ? this.on(scope, ask) : ask(undefined);
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

  /// The scope handles an app reaches for this person (src/lib.rs: `network`'s `reach`), or those of `named` (`user`,
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
    if (!servedHere(this.view, d)) refuse(503, needsDevice(d, app.name));
    if (!scopes.length) refuse(403, `${d.title || app.name} isn't on any of your pages`);
    const writes = what === "call";
    const call = new Call(p, this.who, app, scopes.map((h) => h.id), this.view);
    let finished;
    const done = new Promise((resolve) => (finished = resolve));
    let later = false;
    try {
      const body = { ...fields, me: this.view.me, now: Date.now(), scopes, read_only: !writes, token: crypto.randomUUID(), from, address: this.env.ADDRESS?.replace(/\/$/, "") ?? null, handles: this.view.handles };
      const reply = await p.appWorker(this.who, app).getEntrypoint("Call").run(what, null, body, new ForCall(this, app, call, finished, depth));
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

  async call(req, from = null, depth = 0) {
    const p = this.person;
    const app = deployed(this.view, req.app) || refuse(404, `no app named ${req.app}`);
    const scopes = this.scopes(app, req.scopes);
    // The keys of every database it may open, asked for at once, before it runs, unless each has its scope's current.
    const lacking = scopes.filter((h) => !Object.hasOwn(p.keysOf(this.who, h.id, app.name, "main") || {}, this.view.epochs[h.id] ?? 1));
    if (lacking.length) await p.needs(lacking.map((h) => ({ scope: h.id, app: app.name, database: "main", have: Object.keys(p.keysOf(this.who, h.id, app.name, "main") || {}).map(Number) })), this.who);
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
      const d = await this.person.describe(this.who, app).catch(keysFirst);
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
      const d = await this.person.describe(this.who, app).catch(keysFirst);
      if (d?.badge !== true || !servedHere(this.view, d)) return;
      const n = await this.run(app, "badge", {}, scopes).catch(() => 0);
      if (n > 0) out[app.name] = n;
    }));
    return out;
  }

  /// Every app in the network, with what each does for this person where it's installed for them (src/node.rs:
  /// `catalog`). What it says of itself comes from its latest deploy.
  /// `light`, as a window draws them (src/node/apps.rs: `apps_drawn`): what shows each one and where.
  async apps({ light = false } = {}) {
    const drawn = ["name", "title", "icon", "color", "domain", "image", "image_kind", "installed", "deleted", "screens", "widgets", "owner"];
    return Promise.all(this.view.apps.map(async ({ reach, ...info }) => {
      const app = deployed(this.view, info.name);
      const d = app && (await this.person.describe(this.who, app).catch(keysFirst));
      const mine = reach.length > 0;
      const shown = ["title", "icon", "color", "domain", "tagline", "description", "uses", "needs"];
      for (const [k, v] of Object.entries(d || {})) if (k !== "name" && k !== "schedules" && (mine || shown.includes(k))) info[k] = v;
      if (d) info.offers = { screens: !!d.screens?.some((s) => s.name === "main") || !!d.widgets?.length, accounts: !!d.ops?.some((o) => o.name === "accounts") };
      return light ? Object.fromEntries(Object.entries(info).filter(([k]) => drawn.includes(k))) : info;
    }));
  }
}

const deployed = (view, name) => view.apps.find((a) => a.name === name && a.bundle && !a.deleted) || null;

/// A describe that failed: for keys the device didn't hand, said (428), so it hands them with the next; else none.
const keysFirst = (err) => {
  if (err?.status === 428) throw err;
  return null;
};

/// Whether an app keeps to its own data: it uses no other app or Vault item, and its service, if it has one, runs here.
const plain = (d, view) => !d.uses?.apps?.length && servedHere(view, d) && !d.vault;

/// Whether the service `name`, built for Cloudflare, runs here: an admin switched it on (src/state.rs:
/// `cloudflare_node.parts`), and this node carries its build.
const onHere = (view, name) => Object.hasOwn(services, name) && (view.state.network.cloudflare_node?.parts || []).some((p) => p.name === name && p.on);

const servedHere = (view, d) => (d?.uses?.services || []).every((s) => onHere(view, s));

/// A function of a service built for Cloudflare, run here: → `{ ok }`, or null when it can't answer here (it gave
/// nothing, as for a model it doesn't have), for a device to.
async function hereCall(env, name, op, input) {
  const fn = services[name]?.[op];
  if (typeof fn !== "function" || !env.AI) return null;
  const ok = await fn(input ?? {}, { ai: env.AI });
  return ok === undefined ? null : { ok };
}

/// A call of an app's own service, from the app running here: here when it can, else on a device that runs it; with
/// device "all", every device's answer and this node's, as "Cloudflare".
async function ownService(run, q, d) {
  const cloudflare = String(q.device || "").toLowerCase() === "cloudflare";
  if (q.device === "all") {
    const here = await hereCall(run.env, q.app, q.op, q.input).catch((err) => ({ ok: { error: err.message } }));
    const there = await machineCall(run, q, d).catch(() => ({ ok: {} }));
    return { ok: { ...there.ok, ...(here ? { Cloudflare: here.ok } : {}) } };
  }
  const here = !q.device || cloudflare ? await hereCall(run.env, q.app, q.op, q.input) : null;
  if (here) return here;
  if (cloudflare) refuse(400, `the Cloudflare node can't do that for ${d?.title || q.app}`);
  return machineCall(run, q, d);
}

const needsDevice = (d, name) => `${d?.title || name} runs on your devices, and ${OFF}`;

/// What the node does for an app while it runs, as it asks (src/node.rs: `internal`): its databases, other apps'
/// tools, the Vault's items it's switched on for, and its requests out, filled in from them.
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
        case "POST may":
          return Response.json({ ok: await this.call.may(q.scope, q.asks) });
        case "POST appends":
          await this.call.write(q.appends || []);
          return Response.json({});
        case "POST done":
          this.done();
          return Response.json({});
        case "POST call": {
          // Only to the apps it says it uses, and never to a service: a service is its app's alone, and an app with one
          // runs on a machine, never here. A service, or an app this node can't run, goes to a node on a machine.
          const mine = await person.describe(who, this.app);
          if (q.service && q.app === this.app.name && onHere(view, q.app)) return Response.json(await ownService(this.run, q, mine));
          if (q.service || q.device === "all") refuse(403, `${this.app.name} can't call the ${q.app} service: a service is only its own app's, so call the ${q.app} app's tools with ctx.apps`);
          if (!mine?.uses?.apps?.includes(q.app)) refuse(403, `${this.app.name} doesn't use ${q.app}`);
          const callee = deployed(view, q.app);
          if (!callee) refuse(404, `no app named ${q.app}`);
          const d = q.device ? null : await person.describe(who, callee);
          if (!d || !servedHere(view, d)) return Response.json(await machineCall(this.run, q, d));
          return Response.json(await this.run.call({ app: q.app, op: q.op, input: q.input }, this.app.name, this.depth + 1));
        }
        case "GET vault":
          return new Response(vault_items(await this.call.vault(), this.app.name, who.user), { headers: { "content-type": "application/json" } });
        case "POST fetch":
          return Response.json(await this.fetch(q));
        case "GET catalog":
          return Response.json(await this.run.apps());
        case "GET devices":
          return Response.json(view.state.network.devices);
      }
      return new Response(`a Cloudflare node doesn't do ${path} for apps yet, and ${OFF}`, { status: 501 });
    } catch (err) {
      if (err.status === 428) this.call.need = err;
      return new Response(err.message, { status: err.status === 503 ? 503 : 400 });
    }
  }
}

/// An app's call that needs a node on a machine (a service's, or one of an app that uses one), as this person.
async function machineCall(run, q, d) {
  const call = { app: q.app, op: q.op, input: q.input ?? {}, service: q.service ? q.app : undefined, device: q.device };
  const out = await run.person.onMachine(run.who, run.view, call, 15_000);
  if (!out) refuse(503, needsDevice(d, q.app));
  return answered(out, q.app);
}

function answered(out, app) {
  if (out.ok !== undefined) return out;
  refuse(400, out.error || out.elsewhere || `${app} failed`);
}

/// An app's screen, as its page (src/screen.rs).
async function screenOf(request, person, who, view, app, name, params, scope) {
  const reply = await person.appWorker(who, app).getEntrypoint("Call").run("screen", name, {}, null);
  const body = reply.ok ?? "";
  const context = { app: app.name, screen: name, params, me: view.me, scope, config: { name: CONFIG.name, cli: CONFIG.cli, api: `http://localhost:${CONFIG.port}`, made_here: CONFIG.made_here } };
  return screenPage(request, screen(JSON.stringify(context), body, new URL(request.url).searchParams.get("linked") === "true"));
}

const CONNECT = 2_000;
const IDLE = 30_000;

const hex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");
const unhex = (s) => Uint8Array.from(s.match(/../g) || [], (h) => parseInt(h, 16));
const near = (env) => (env.LOCATION ? { locationHint: env.LOCATION } : {});

async function route(p, request) {
  const url = new URL(request.url);
  const path = url.pathname;
  if (request.method === "POST" && path === "/v1/call") {
    const who = p.whom(request);
    const req = await bodyOf(request);
    if (typeof req !== "object" || req === null || Array.isArray(req)) refuse(400, "a call is an object");
    const run = new Run(p, who, await p.networkFor(who));
    const app = !req.service && !req.device && deployed(run.view, req.app);
    const d = app && (await p.describe(who, app));
    // An app that keeps to its own data runs here; the rest on a machine, when one answers.
    if (d && plain(d, run.view)) return json(await run.call(req));
    const r = await p.onMachine(who, run.view, req, 120_000);
    if (r) return json(r, r.ok !== undefined ? 200 : 400);
    if (!d || !servedHere(run.view, d)) refuse(503, needsDevice(d, req.app));
    return json(await run.call(req));
  }
  const screenAt = path.match(/^\/v1\/screens\/([a-z0-9-]+)\/([A-Za-z0-9_-]+)$/);
  if (request.method === "GET" && screenAt) {
    const who = p.whom(request);
    const view = await p.networkFor(who);
    const app = deployed(view, screenAt[1]);
    const d = app && (await p.describe(who, app));
    if (!d || !servedHere(view, d)) refuse(503, needsDevice(d, screenAt[1]));
    return screenOf(request, p, who, view, app, screenAt[2], JSON.parse(url.searchParams.get("params") || "{}"), url.searchParams.get("scope"));
  }
  // A scope's keys, as sealed in the bucket, where every endpoint the relay lets in may read them anyway.
  if (request.method === "GET" && path === "/v1/keys") {
    p.whom(request);
    return json(await sealedKeys(p.bucket, url.searchParams.get("scope") || "", url.searchParams.get("to"), url.searchParams.get("app")));
  }
  // Who's who as the bucket holds it, for a device that checks it all itself (`steps::Reader`): `global`'s scope
  // database's steps, sealed.
  if (request.method === "GET" && path === "/v1/steps/global") {
    const who = p.whom(request);
    await p.readScopes(who, ["global"]);
    p.forgetLost(who);
    return json(JSON.parse(await sealed_global_steps(p.owner, who.user, p.bucket, Number(url.searchParams.get("from") || 0), url.searchParams.get("welcome") ?? undefined)));
  }
  if ((request.method === "GET" && ["/v1/state", "/v1/apps", "/v1/widgets", "/v1/badges", "/v1/events"].includes(path))) {
    const who = p.whom(request);
    const run = new Run(p, who, await p.networkFor(who));
    // Nothing here changes but what devices write, which they'll say once one is on: no events, so the client asks again
    // in a while, and this node sleeps meanwhile.
    if (path === "/v1/events") return new Response(null, { status: 204 });
    const scope = url.searchParams.get("scope");
    if (path === "/v1/state") return json(run.view.state);
    if (path === "/v1/apps") return tagged(request, JSON.stringify(await run.apps({ light: url.searchParams.get("light") === "true" })), "application/json");
    if (path === "/v1/widgets") return json(await run.widgets(scope));
    return json(await run.badges(scope));
  }
  // Anything else only a machine does: to a client that asks one itself (`x-one-elsewhere`), the machines of its
  // person's that are on, as a node on a machine names those that run a call it can't (421).
  if (request.headers.get("x-one-elsewhere")) {
    const view = await p.networkFor(p.whom(request));
    const theirs = view.state.network.devices.filter((d) => d.serves_me).sort((a, b) => a.order - b.order).map((d) => d.key);
    const on = new Set(await p.env.RELAY.connected(theirs));
    const elsewhere = theirs.filter((k) => on.has(k));
    if (elsewhere.length) return json({ error: "a machine does that", elsewhere }, 421);
  }
  refuse(503, `That needs one of your devices, and ${OFF}.`);
}

/// A scope's keys as a node gives them (src/node/devices.rs: `sealed_keys`): each epoch sealed to `to` (an endpoint's
/// key, or `recovery`) → `{ epoch, sealed }`, or an app's key in each epoch → `{ sealed }`, by epoch, base64url. Its
/// epochs are those sealed to the recovery key, every one of them, numbered from 1.
async function sealedKeys(bucket, scope, to, app) {
  if (!to === !app) refuse(400, "give to (an endpoint's key, or recovery) or app");
  if (!/^(global|(user|group):[A-Za-z0-9_-]{1,64})$/.test(scope)) refuse(400, "that isn't a scope");
  if (to ? !/^([0-9a-f]{64}|recovery)$/.test(to) : !/^[a-z0-9]+(-[a-z0-9]+)*$/.test(app)) refuse(400, to ? "to is an endpoint's key, or recovery" : "that isn't an app's name");
  let newest = 0;
  while (newest < 1024 && (await bucket.get(epoch_name(scope, newest + 1, "recovery")))) newest++;
  const sealed = {};
  await Promise.all(Array.from({ length: newest }, (_, i) => i + 1).map(async (n) => {
    const o = await bucket.get(to ? epoch_name(scope, n, to) : app_key_name(scope, app, n));
    if (o) sealed[n] = b64u(o.bytes);
  }));
  return to ? { epoch: newest, sealed } : { sealed };
}

const b64u = (bytes) => {
  let s = "";
  for (let i = 0; i < bytes.length; i += 0x8000) s += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  return btoa(s).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
};

function failure(err) {
  if (err instanceof Refused) return json({ error: err.message, ...err.said }, err.status);
  console.error(err.stack || err);
  return json({ error: String(err?.message ?? err) }, 400);
}

/// Who a client's request over iroh is from: the device `remote` (hex) is, or, for a session's key, the device its
/// certificate (`x-one-endpoint`) says vouched for it, until then. → `{ device, until }`.
function fromOf(remote, headers) {
  const certificate = headers.get("x-one-endpoint");
  if (!certificate) return { device: remote, until: null };
  let session;
  try {
    session = JSON.parse(check_certificate(certificate, Date.now()));
  } catch (err) {
    refuse(403, `that certificate won't do: ${err.message}`);
  }
  if (session.endpoint !== remote) refuse(403, "that certificate is for another endpoint");
  return { device: session.by, until: session.until };
}

export default {
  fetch: () => json({ error: "the Cloudflare node is asked over iroh, as any node is" }, 404),
};

/// A Durable Object of the Cloudflare node: a person's (`user:<id>`), where their requests run, or the node's
/// endpoint (`node`), which the relay wakes and which asks devices for calls.
export class Node extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    // The network's root, as sign-in pins it: a freeze naming any other is never taken for its first.
    if (env.ROOT) pin_root(env.ROOT);
    this.isEndpoint = ctx.id.equals(env.NODE.idFromName("node"));
    /// Datagrams it sent this turn, for the relay; asks running now, and when the last one began or ended.
    this.out = [];
    this.asking = 0;
    this.last = Date.now();
  }

  async fetch(request) {
    if (this.isEndpoint) return json({ error: "the Cloudflare node's endpoint answers only asks over iroh" }, 400);
    this.person ||= new Person(this);
    try {
      return await route(this.person, request);
    } catch (err) {
      return failure(err);
    }
  }

  endpoint() {
    return this.env.NODE.get(this.env.NODE.idFromName("node"), near(this.env));
  }

  only() {
    if (!this.isEndpoint) throw new Error("only the Cloudflare node's endpoint does that");
  }

  relay() {
    return this.env.RELAY;
  }

  /// Its endpoint, started once in the object's life, or again once one closing for being idle (`closing`) has: the
  /// first packet or ask after it slept waits for this.
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
      const closed = (this.closing || Promise.resolve()).catch(() => {});
      this.started = closed.then(() => endpoint_start(unhex(this.env.NODE_KEY), send, (...asked) => this.asked(...asked))).then((key) => {
        console.log(JSON.stringify({ woke: key.slice(0, 8), ms: Date.now() - began }));
        return key;
      });
      const mine = this.started;
      mine.catch(() => this.started === mine && (this.started = null));
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

  /// A client device's request over iroh (`remote` its key, hex; `headers` names and values, one after the other):
  /// answered in its person's object, as `x-one-user` names them, with who asked as the connection proved it.
  /// → `{ status, headers, body }`.
  async asked(remote, method, path, headers, body) {
    this.asking++;
    this.awake();
    try {
      const h = new Headers();
      for (let i = 0; i + 1 < headers.length; i += 2) h.append(headers[i], headers[i + 1]);
      const r = await (async () => {
        const user = h.get(USER) || "";
        if (!/^[A-Za-z0-9_-]{1,64}$/.test(user)) refuse(400, `say whose request it is (${USER})`);
        h.set(FROM, JSON.stringify(fromOf(remote, h)));
        h.delete("x-one-endpoint");
        h.delete("authorization");
        if (body.length > BODY_MOST) refuse(413, "that's more than a request may carry");
        const request = new Request(`${this.env.ADDRESS}${path.startsWith("/") ? path : `/${path}`}`, { method, headers: h, body: ["GET", "HEAD"].includes(method) ? null : body });
        return personOf(this.env, user).fetch(request);
      })().catch(failure);
      return { status: r.status, headers: [...r.headers].flat(), body: new Uint8Array(await r.arrayBuffer()) };
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
    // Its connections' ends reach their peers before it does. What's asked meanwhile starts it again, once it's closed,
    // and keeps it.
    const started = this.started;
    this.started = null;
    const closing = (this.closing = started.then(() => endpoint_close(), () => {}).then(() => this.sent));
    try {
      await closing;
    } finally {
      if (this.closing === closing) this.closing = null;
    }
    if (this.started || this.asking) return this.awake();
    console.log(JSON.stringify({ sleeps: true }));
    this.ctx.abort("idle");
  }
}
