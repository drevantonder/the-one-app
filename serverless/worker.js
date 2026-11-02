// The serverless node: a node of the install on Cloudflare Workers, which the gateway sends `/v1/` and `/mcp` to by
// service binding. It holds nothing open. Each request brings its person's access token, checked as every node checks
// one (src/lib.rs, which is the machine node's own Rust); for a call it asks the vault for the keys and bucket URLs of
// what the call names, runs the app in an isolate of its own (Worker Loader, src/runtime/shim.js as its `ctx`), writes
// what the call wrote as one step (src/db.rs), and forgets. What it can't do itself goes on to the install's node on
// a machine (`MACHINE`), as the gateway sent it.
//
// Bindings: AUTH and VAULT (the kernel's services), LOADER (Worker Loader), MACHINE (a node on a machine: a Workers
// VPC service; or MACHINE_URL, its address), ADDRESS (the install's) and NAME (its name). `built.js`, made when it's
// deployed, is the shim and the built-in apps' modules.

import { RpcTarget } from "cloudflare:workers";
import { initSync, verify, open, query, write, close, statements, screen, config, prefix, following as after } from "./one_serverless.js";
import wasm from "./one_serverless_bg.wasm";
import { shim, apps, compatibilityDate } from "./built.js";

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

/// Asks the vault, with the person's access token.
async function vault(env, token, method, path, body) {
  const r = await env.VAULT.fetch(`https://vault${path}`, { method, headers: { authorization: `Bearer ${token}`, "content-type": "application/json" }, body: body === undefined ? undefined : JSON.stringify(body) });
  const out = await r.json().catch(() => ({}));
  if (!r.ok) refuse(r.status === 403 ? 403 : 502, `the vault said: ${out.error || r.status}`);
  return out;
}

/// People's names, by id, as the auth service says them: only names, kept while this isolate lives.
const names = new Map();

async function nameOf(env, token, user) {
  if (!names.has(user)) {
    const r = await env.AUTH.fetch(`${env.ADDRESS}/api/auth/oauth2/userinfo`, { headers: { authorization: `Bearer ${token}` } });
    const info = r.ok ? await r.json().catch(() => ({})) : {};
    if (!info.name) return "";
    names.set(user, info.name);
  }
  return names.get(user);
}

/// Whom a request is for: an access token the auth service at the install's address signed for it, whose session at
/// the vault is open. → `{ token, user, client, name }`.
async function whom(request, env) {
  const auth = request.headers.get("authorization") || "";
  const token = auth.toLowerCase().startsWith("bearer ") ? auth.slice(7).trim() : refuse(403, "sign in first: a serverless node takes only sign-ins");
  const claims = await claimsOf(env, token);
  const aud = Array.isArray(claims.aud) ? claims.aud : [claims.aud];
  if (!aud.includes(env.ADDRESS)) refuse(401, `that sign-in isn't for ${env.ADDRESS}`);
  const session = await vault(env, token, "GET", "/session");
  if (session.user !== claims.sub) refuse(401, "that sign-in's session is someone else's");
  return { token, user: claims.sub, client: claims.azp, name: await nameOf(env, token, claims.sub) };
}

/// What an app is told about who it runs for (src/apps.rs: `Me`).
const meOf = (who) => ({ key: "", user: who.user, name: "", role: null, user_name: who.name, agent: null });

/// A scope as an app's `ctx` has it (src/apps.rs: `ScopeHandle`).
const handleOf = (env, who, scope) => ({ id: scope, name: scope === "global" ? env.NAME : scope.startsWith("user:") ? who.name : "", role: null });

// The bucket, through presigned URLs the vault signs for each call.

/// The names of the `n` steps after `name`, if it's a database's step.
const following = (name, n) => JSON.parse(after(name, n));

/// The bucket as src/db.rs reads and writes it (src/lib.rs: `Js`). A database's steps are read one after another, so
/// asking for one fetches the next few at once; each fetched object is used once, so a read after a lost race is
/// fresh.
class Bucket {
  constructor(env, token) {
    this.env = env;
    this.token = token;
    this.reads = new Map();
    this.fetched = new Map();
  }

  async readUrl(name) {
    if (!this.reads.has(name)) {
      const out = await vault(this.env, this.token, "POST", "/urls", { read: [name, ...following(name, 15).filter((n) => !this.reads.has(n))] });
      for (const [n, url] of Object.entries(out.read)) this.reads.set(n, url);
    }
    return this.reads.get(name);
  }

  async fetchObject(name) {
    const r = await fetch(await this.readUrl(name));
    if (r.status === 404) return null;
    if (!r.ok) throw new Error(`the bucket said ${r.status} to reading ${name}`);
    return { bytes: new Uint8Array(await r.arrayBuffer()), tag: r.headers.get("etag") };
  }

  get(name) {
    for (const n of [name, ...following(name, 5)]) if (!this.fetched.has(n)) this.fetched.set(n, this.fetchObject(n));
    const got = this.fetched.get(name);
    this.fetched.delete(name);
    return got;
  }

  async put(url, headers, name, bytes) {
    this.fetched.delete(name);
    const r = await fetch(url, { method: "PUT", headers, body: bytes });
    if (r.status === 412) return false;
    if (!r.ok) throw new Error(`the bucket said ${r.status} to writing ${name}`);
    return true;
  }

  async create(name, bytes) {
    const out = await vault(this.env, this.token, "POST", "/urls", { create: [name] });
    return this.put(out.create[name], { "if-none-match": "*" }, name, bytes);
  }

  async replace(name, bytes, tag) {
    if (tag === null) return this.create(name, bytes);
    const out = await vault(this.env, this.token, "POST", "/urls", { replace: [{ name, etag: tag }] });
    return this.put(out.replace[name], { "if-match": tag }, name, bytes);
  }
}

// Apps.

/// An app this node carries, in an isolate of its own: no network, and the node's answers handed in with each call.
function appWorker(env, name) {
  const app = apps[name] || refuse(404, `${name} doesn't run on a serverless node yet`);
  return env.LOADER.get(`${name}-${app.hash}`, () => ({
    compatibilityDate,
    compatibilityFlags: ["nodejs_als"],
    mainModule: "shim.js",
    modules: { "shim.js": shim, "app.js": app.module },
    globalOutbound: null,
  }));
}

const described = new Map();

/// What an app says it is and does (src/runtime/shim.js: `describe`).
async function describe(env, name) {
  if (!described.has(name)) {
    const reply = await appWorker(env, name).getEntrypoint("Call").run("describe", null, {}, null);
    described.set(name, reply.ok);
  }
  return described.get(name);
}

/// A call's databases, opened from the bucket as it first reaches each, and closed when it's done.
class Call {
  constructor(env, who, app, scopes) {
    Object.assign(this, { env, who, app, scopes, bucket: new Bucket(env, who.token), dbs: new Map() });
  }

  db(scope) {
    if (!this.scopes.includes(scope)) refuse(403, `the call didn't name ${scope}`);
    if (!this.dbs.has(scope)) this.dbs.set(scope, this.open(scope));
    return this.dbs.get(scope);
  }

  async open(scope) {
    const { keys } = await vault(this.env, this.who.token, "POST", "/keys", { keys: [{ scope, app: this.app, database: "main" }] });
    const migrations = (await describe(this.env, this.app)).schema?.[kindOf(scope)] || [];
    return open(this.bucket, prefix(scope, this.app, "main"), keys[0].key, JSON.stringify(migrations));
  }

  async query(scope, sql, params) {
    return JSON.parse(query(await this.db(scope), sql, JSON.stringify(params)));
  }

  /// Writes a call's appends: each scope's statements as one step. → the databases it wrote.
  async write(appends) {
    const each = JSON.parse(statements(JSON.stringify(appends), JSON.stringify(this.scopes)));
    for (const [scope, list] of Object.entries(each)) await write(await this.db(scope), this.bucket, JSON.stringify(list));
    return Object.keys(each).map((scope) => prefix(scope, this.app, "main"));
  }

  close() {
    for (const db of this.dbs.values()) db.then(close, () => {});
  }
}

/// What the node does for a call while it runs, as an app asks it (src/node.rs: `internal`).
class ForCall extends RpcTarget {
  constructor(call, done) {
    super();
    this.call = call;
    this.done = done;
  }

  async ask(path, method, body) {
    try {
      if (method === "POST" && path === "query") {
        const q = JSON.parse(body);
        return Response.json({ ok: await this.call.query(q.scope, q.sql, q.params || []) });
      }
      if (method === "POST" && path === "appends") {
        await this.call.write(JSON.parse(body).appends || []);
        return Response.json({});
      }
      if (method === "POST" && path === "done") {
        this.done();
        return Response.json({});
      }
      return new Response(`a serverless node doesn't do ${path} for apps yet: use a node on one of your machines`, { status: 501 });
    } catch (err) {
      return new Response(err.message, { status: 400 });
    }
  }
}

/// The scopes a call reaches: those it names (`user`, `global`, a group's id or `group:<id>`), or else the person's
/// own and Everyone's. The vault gives keys only for those the person may open.
function scopesFor(who, named) {
  return (named || ["user", "global"]).map((s) => (s === "user" ? `user:${who.user}` : s === "global" || s.includes(":") ? s : `group:${s}`));
}

/// Runs a call of an app this node carries. Work it leaves running (`ctx.waitUntil`) keeps its databases open until
/// it says it's done.
async function call(env, ctx, who, req) {
  const scopes = scopesFor(who, req.scopes);
  const run = new Call(env, who, req.app, scopes);
  let finished;
  const done = new Promise((resolve) => (finished = resolve));
  let later = false;
  try {
    const body = { op: req.op, input: req.input ?? {}, me: meOf(who), now: Date.now(), scopes: scopes.map((s) => handleOf(env, who, s)), read_only: false, token: crypto.randomUUID(), from: null };
    const reply = await appWorker(env, req.app).getEntrypoint("Call").run("call", null, body, new ForCall(run, finished));
    if (reply.error !== undefined) refuse(400, reply.error);
    const wrote = await run.write(reply.appends || []);
    later = reply.later === true;
    // What ran, for the Worker's logs: never what it read or wrote.
    console.log(JSON.stringify({ call: `${req.app}.${req.op}`, user: who.user, wrote, ms: Date.now() - body.now }));
    return { ok: reply.ok ?? null };
  } finally {
    if (later) ctx.waitUntil(Promise.race([done, new Promise((r) => setTimeout(r, 600_000))]).then(() => run.close()));
    else run.close();
  }
}

/// An app's screen, as its page (src/screen.rs).
async function screenOf(env, who, app, name, params, scope) {
  const reply = await appWorker(env, app).getEntrypoint("Call").run("screen", name, {}, null);
  const body = reply.ok ?? '<div id="missing"></div><script>missing.innerHTML = one.notFound()</script>';
  const context = { app, screen: name, params, me: meOf(who), scope, config: { name: CONFIG.name, cli: CONFIG.cli, api: env.ADDRESS, made_here: null } };
  return screen(JSON.stringify(context), body);
}

// Requests.

/// Sends a request on to the install's node on a machine, as the gateway sent it here.
async function toMachine(request, env) {
  if (!env.MACHINE && !env.MACHINE_URL) refuse(503, "that needs a node on one of your machines, and this install's serverless node knows none yet");
  const url = new URL(request.url);
  try {
    // Through a Cloudflare Tunnel the host is only what the node sees: the gateway's, as it sent it here.
    return env.MACHINE ? await env.MACHINE.fetch(`http://${url.host}${url.pathname}${url.search}`, request) : await fetch(`${env.MACHINE_URL}${url.pathname}${url.search}`, request);
  } catch {
    refuse(502, "the node on your machine can't be reached");
  }
}

/// Whether this node runs an app's calls and screens itself: one it carries that keeps only to its own data, using no
/// other app, sidecar or Vault item. The rest go to a node on a machine.
async function runsHere(env, app) {
  if (!apps[app]) return false;
  const d = await describe(env, app);
  return !d.uses?.apps?.length && !d.uses?.sidecars?.length && !d.vault;
}

async function route(request, env, ctx) {
  const url = new URL(request.url);
  const path = url.pathname;
  if (request.method === "POST" && path === "/v1/call") {
    const req = await request.clone().json().catch(() => refuse(400, "the body isn't JSON"));
    if (req.sidecar || req.server || req.as_user || !(await runsHere(env, req.app))) return toMachine(request, env);
    return json(await call(env, ctx, await whom(request, env), req));
  }
  const screenAt = path.match(/^\/v1\/screens\/([a-z0-9-]+)\/([A-Za-z0-9_-]+)$/);
  if (request.method === "GET" && screenAt && (await runsHere(env, screenAt[1]))) {
    const who = await whom(request, env);
    const params = JSON.parse(url.searchParams.get("params") || "{}");
    const html = await screenOf(env, who, screenAt[1], screenAt[2], params, url.searchParams.get("scope"));
    return new Response(html, { headers: { "content-type": "text/html; charset=utf-8" } });
  }
  return toMachine(request, env);
}

export default {
  async fetch(request, env, ctx) {
    try {
      return await route(request, env, ctx);
    } catch (err) {
      if (err instanceof Refused) return json({ error: err.message }, err.status);
      console.error(err.stack || err);
      return json({ error: String(err?.message ?? err) }, 400);
    }
  },
};
