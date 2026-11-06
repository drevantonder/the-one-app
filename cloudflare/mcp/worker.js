// The remote MCP server: remote MCP clients' device. ChatGPT, Claude.ai and the like can't run our code, so each one a person
// signs in with is a device of theirs here, a client of the network like any app, which the person approves like any
// device. It speaks MCP to them over HTTPS at `<address>/mcp`, and asks a node's own `/mcp` as that device, over iroh
// (`<cli>/http/1`, one-client in cloudflare/node/src/clients.rs), handing it the keys a call needs, sealed to that node and
// bound to the device's key; it sends nodes no token.
//
// A device's secret is kept only sealed under its client's own tokens (copies.js), so nothing here opens it alone: the
// Worker is on its clients' token path (its own token endpoint, in the authorization server's metadata it serves for
// MCP clients, passing each request on to sign-in), sealing the secret under each new refresh and access token. A client's
// first sign-in makes its device and asks sign-in to let it join; the person approves it by its code on one of their
// devices, which lists it, seals it the keys of their scopes and lets it in at the relay. Until then its MCP requests
// are answered with that code. Signing in again makes a new device, as the old one's secret opened only under the
// tokens it had.
//
// Each device is a Durable Object (`Client`), one per client and person, so no two people's state meet: its iroh
// endpoint, with no socket, which the relay hands the datagrams sent to its key by RPC (`deliver`), its connections to
// nodes kept warm between calls, and the keys it opened, all while it's asked; once nobody has asked for `IDLE`, it
// closes its endpoint, forgets its keys and ends itself.
//
// It trusts no node for who's who or for keys: it reads the network's signed entries from the bucket itself, through
// the URL signer, replays them as nodes do, and asks the scope database it builds from them, with the same Rust as
// every node (cloudflare/node/src/clients.rs). It asks only the compute endpoints that lists as running its person's
// calls, hands each only the keys of the scopes it runs, of databases an MCP request uses (`mayHand`) and epochs from
// each one's newest copy, and takes a 428 only from the node that answered, for its own listed seal key. What a tool needed is kept per tool, so a call hands only its own.
//
// Bindings: AUTH (the sign-in), RELAY (the relay's `McpDevices` entrypoint), SIGNER (the URL signer), CLIENT
// (this Worker's `Client`), ADDRESS (the network's), NAME (its name), LOCATION (where objects are made), IDLE (ms,
// optional).

import { DurableObject } from "cloudflare:workers";
import { initSync, device_start, device_deliver, device_fetch, device_nodes, device_close, device_view, device_keys, following } from "./one_cloudflare_node.js";
import wasm from "./one_cloudflare_node_bg.wasm";
import { b64u, ed25519Public, hex, joinCode, mayHand, random, sealHandoff, signedRequest, untilTaken } from "../shared/crypto.js";
import { READS, RELAY, URLS } from "../shared/protocol.js";
import { Issuer, bearer } from "../shared/token.js";
import { Copies } from "./copies.js";

initSync({ module: wasm });

const IDLE = 30_000;
const TO_NODE = ["content-type", "accept", "mcp-session-id", "mcp-protocol-version"];
const FROM_NODE = ["content-type", "mcp-session-id"];
const BODY_MOST = 1 << 20;
const VIEW_EVERY = 10_000;
const NEEDS_MOST = 16;
const TOOLS_MOST = 128;
/// How long the relay keeps a device's binding, if it doesn't say, and how long before that ends it's bound again.
const BOUND = 3_600_000;
const BIND_AGAIN = 600_000;
/// How long before sign-in forgets a request to join it's asked again, and how soon one sign-in didn't take is.
const JOIN_EARLY = 60_000;
const JOIN_AGAIN = 30_000;

const json = (v, status = 200, headers = {}) => Response.json(v, { status, headers });
const near = (env) => (env.LOCATION ? { locationHint: env.LOCATION } : {});
const origin = (env) => new URL(env.ADDRESS).origin;
const CORS = { "access-control-allow-origin": "*" };

let issuer = null;
const issuerOf = (env) => (issuer ||= new Issuer(`${origin(env)}/api/auth`, () => env.AUTH.fetch(`${origin(env)}/api/auth/jwks`)));

/// A token's claims, unchecked: only for one sign-in just gave us by its binding.
const claimsOf = (token) => JSON.parse(new TextDecoder().decode(b64u.decode(token.split(".")[1])));

const deviceOf = (env, client, user) => env.CLIENT.get(env.CLIENT.idFromName(`${client}\n${user}`), near(env));

/// What an MCP client is told when it has no sign-in, or one that won't do: where to sign in (RFC 9728).
function signInFirst(env, error) {
  const invalid = error ? 'error="invalid_token", ' : "";
  const challenge = `Bearer ${invalid}resource_metadata="${origin(env)}/.well-known/oauth-protected-resource/mcp", scope="openid profile offline_access"`;
  return json({ error: error || `sign in at ${origin(env)} first` }, 401, { "www-authenticate": challenge });
}

/// The authorization server's metadata, as sign-in says it, but with this Worker's token endpoint: what MCP clients find
/// from the resource's (RFC 8414's path for the issuer `<address>/api/auth`). Apps read sign-in's own, at the root.
async function authorizationServer(env, which) {
  const r = await env.AUTH.fetch(`${origin(env)}/.well-known/${which}`);
  if (!r.ok) return r;
  const meta = await r.json();
  meta.token_endpoint = `${origin(env)}/mcp/token`;
  return json(meta, 200, CORS);
}

async function bodyOf(request) {
  if (Number(request.headers.get("content-length") || 0) > BODY_MOST) return null;
  const bytes = await request.arrayBuffer();
  return bytes.byteLength > BODY_MOST ? null : new TextDecoder().decode(bytes);
}

function askedOf(request, body) {
  if (/json/i.test(request.headers.get("content-type") || "")) {
    let o = {};
    try {
      o = JSON.parse(body) || {};
    } catch {}
    return { get: (name) => (typeof o[name] === "string" ? o[name] : null) };
  }
  return new URLSearchParams(body);
}

async function token(request, env) {
  const body = await bodyOf(request);
  if (body === null) return json({ error: "invalid_request", error_description: "that request is too big" }, 413);
  const asked = askedOf(request, body);
  const headers = { "content-type": request.headers.get("content-type") || "application/x-www-form-urlencoded" };
  for (const name of ["authorization", "dpop", "cf-connecting-ip"]) if (request.headers.has(name)) headers[name] = request.headers.get(name);
  const r = await env.AUTH.fetch(`${origin(env)}/api/auth/oauth2/token`, { method: "POST", headers, body });
  const out = await r.json().catch(() => null);
  if (!r.ok || !out?.access_token || !out?.refresh_token) return out ? json(out, r.status, { "cache-control": "no-store" }) : new Response(null, { status: r.status });
  const claims = claimsOf(out.access_token);
  const client = claims.azp || claims.client_id || asked.get("client_id");
  const grant = asked.get("grant_type");
  const old = grant === "refresh_token" ? asked.get("refresh_token") : null;
  const signedIn = { client, user: claims.sub, old, refresh: out.refresh_token, access: out.access_token, until: claims.exp * 1000 };
  try {
    await deviceOf(env, client, claims.sub).signedIn(signedIn);
  } catch (err) {
    // Sign-in rotated the refresh token, but its device's secret isn't sealed under the new one: the client isn't given
    // tokens that open nothing, and asks again with the old refresh token, which sign-in answers the same for a minute.
    console.error(err.stack || err);
    return json({ error: "temporarily_unavailable", error_description: "Try again in a moment." }, 503, { "cache-control": "no-store", "retry-after": "2" });
  }
  return json(out, 200, { "cache-control": "no-store", pragma: "no-cache" });
}

async function mcp(request, env) {
  const token = bearer(request);
  if (!token) return signInFirst(env);
  let claims;
  try {
    claims = await issuerOf(env).verify(token);
  } catch (err) {
    return signInFirst(env, `that sign-in won't do: ${err.message}`);
  }
  // An MCP client's sign-in is for this, `<address>/mcp`; the network's own apps' (`<address>`) open no device here.
  if (![claims.aud].flat().includes(`${origin(env)}/mcp`)) return signInFirst(env, `that sign-in isn't for ${origin(env)}/mcp`);
  if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
  const body = await bodyOf(request);
  if (body === null) return json({ error: "that request is too big" }, 413);
  const headers = {};
  for (const name of TO_NODE) if (request.headers.has(name)) headers[name] = request.headers.get(name);
  const out = await deviceOf(env, claims.azp || claims.client_id, claims.sub).asked(token, claims.exp * 1000, headers, body);
  if (out.signIn) return signInFirst(env, out.signIn);
  return new Response(out.body, { status: out.status, headers: out.headers });
}

export default {
  async fetch(request, env) {
    const path = new URL(request.url).pathname;
    try {
      if (path === "/mcp") return await mcp(request, env);
      if (path === "/mcp/token" && request.method === "POST") return await token(request, env);
      if (path === "/.well-known/oauth-protected-resource/mcp" || path === "/.well-known/oauth-protected-resource") {
        const resource = path.endsWith("/mcp") ? `${origin(env)}/mcp` : origin(env);
        const scopes = ["openid", "profile", "offline_access"];
        return json({ resource, resource_name: env.NAME, authorization_servers: [`${origin(env)}/api/auth`], scopes_supported: scopes, bearer_methods_supported: ["header"] }, 200, CORS);
      }
      if (path === "/.well-known/oauth-authorization-server/api/auth") return await authorizationServer(env, "oauth-authorization-server");
      if (path === "/.well-known/openid-configuration/api/auth") return await authorizationServer(env, "openid-configuration");
      return json({ error: "not found" }, 404);
    } catch (err) {
      console.error(err.stack || err);
      return json({ error: String(err?.message ?? err) }, 500);
    }
  },
};

/// The JSON-RPC answer to each message of an MCP request with no node behind it: `text` as each request's error, a
/// tool call's as its failed result; nothing for notifications. → a response.
function answerAll(body, text) {
  let msg;
  try {
    msg = JSON.parse(body);
  } catch {
    return { status: 400, headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "the body isn't JSON" } }) };
  }
  const one = (m) => {
    if (m?.id === undefined) return null;
    if (m.method === "tools/call") return { jsonrpc: "2.0", id: m.id, result: { isError: true, content: [{ type: "text", text }] } };
    return { jsonrpc: "2.0", id: m.id, error: { code: -32000, message: text } };
  };
  const out = Array.isArray(msg) ? msg.map(one).filter(Boolean) : one(msg);
  if (!out || (Array.isArray(out) && !out.length)) return { status: 202, headers: {}, body: null };
  return { status: 200, headers: { "content-type": "application/json" }, body: JSON.stringify(out) };
}

/// What an MCP request's needs are kept by: its tool (`tools/call <name>`), or its method; none for a batch, whose
/// calls hand only what a 428 names.
function toolOf(body) {
  let msg;
  try {
    msg = JSON.parse(body);
  } catch {
    return null;
  }
  if (!msg || Array.isArray(msg) || typeof msg.method !== "string") return null;
  const name = msg.method === "tools/call" && typeof msg.params?.name === "string" ? ` ${msg.params.name}` : "";
  return `${msg.method}${name}`.slice(0, 160);
}

function needOf(n) {
  const plain = (s, most) => typeof s === "string" && s.length > 0 && s.length <= most && /^[A-Za-z0-9:_-]+$/.test(s);
  return n && plain(n.scope, 80) && plain(n.app, 64) && plain(n.database, 64) ? { scope: n.scope, app: n.app, database: n.database } : null;
}

const sameNeed = (a, b) => a.scope === b.scope && a.app === b.app && a.database === b.database;

/// The bucket as the device's Rust reads it (cloudflare/node/src/lib.rs `Js`), through the URL signer, signed with the
/// device's key: it gives URLs only to the network's endpoints. The steps after one are signed with it, as they're read
/// next.
class Bucket {
  constructor(env, me) {
    Object.assign(this, { env, me, urls: new Map() });
  }

  /// With what this build reads of who's who (`READS`), which the URL signer asks of a client once it's switched.
  async sign(fields) {
    const body = await signedRequest(this.me.seed, hex.decode(this.me.key), URLS, { ...fields, reads: READS });
    const r = await this.env.SIGNER.fetch(`${origin(this.env)}/urls`, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    const out = await r.json().catch(() => ({}));
    if (!r.ok) throw Object.assign(new Error(`the URL signer said: ${out.error || r.status}`), { unanswered: r.status >= 500 });
    return out;
  }

  async get(name) {
    let kept = this.urls.get(name);
    if (!kept || kept.until < Date.now() + 60_000) {
      const names = [name, ...JSON.parse(following(name, 8))];
      const out = await this.sign({ read: names });
      for (const n of names) if (out.read?.[n]) this.urls.set(n, { url: out.read[n], until: out.until });
      kept = this.urls.get(name);
    }
    const r = await fetch(kept.url);
    if (r.status === 404) return null;
    if (!r.ok) throw Object.assign(new Error(`the bucket said ${r.status} to reading ${name}`), { unanswered: r.status >= 500 });
    return { bytes: new Uint8Array(await r.arrayBuffer()), tag: r.headers.get("etag") || "" };
  }

  /// Makes `name` if it isn't there: an app's key in an epoch, which the first to need it makes.
  async create(name, bytes) {
    const out = await this.sign({ create: [name] });
    const r = await fetch(out.create[name], { method: "PUT", headers: { "if-none-match": "*" }, body: bytes });
    if (r.status === 412) return false;
    if (!r.ok) throw Object.assign(new Error(`the bucket said ${r.status} to writing ${name}`), { unanswered: r.status >= 500 });
    return true;
  }

  async replace() {
    throw new Error("a device replaces nothing in the bucket");
  }

  wait(ms) {
    return new Promise((resolve) => setTimeout(resolve, ms));
  }
}

export class Client extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.copies = new Copies(ctx.storage);
    this.asking = 0;
    this.out = [];
    /// Who's who as it last read it: `{ listed, nodes: Map(key → { seal, scopes: Set }), at }`.
    this.view = null;
    this.viewing = Promise.resolve();
    /// What each node was handed for this device's access token: key → `{ until, tools: Set }`.
    this.handed = new Map();
    /// Its endpoint, while it's awake: `{ key, phase, ready, ticket, until }`, one at a time, `phase` "starting" until
    /// `ready` (it's bound at the relay, its endpoint started, and it's read who's who), then "running"; none while it's
    /// asleep. `end` takes it away at once, so what's asked after starts another, and closes it once it's started: a
    /// start it ends midway stops there.
    this.run = null;
  }

  relay() {
    return this.env.RELAY;
  }

  /// A client signed in (`old` null) or refreshed its sign-in: its device's secret, sealed under its new tokens (sign-in's
  /// same answer again, to a refresh whose answer was lost, opens it as it is). A sign-in, or a refresh whose old token
  /// opens nothing, makes a new device and asks sign-in to let it join; the device it replaces is removed once the new one
  /// is approved.
  async signedIn({ client, user, old, refresh, access, until }) {
    await this.ctx.storage.put("name", `${client}\n${user}`);
    if (old && (await this.copies.rotate(old, refresh, access, until))) return;
    const seed = random(32);
    const seal = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits"]);
    const sealSecret = b64u.decode((await crypto.subtle.exportKey("jwk", seal.privateKey)).d);
    const device = { key: hex.encode(await ed25519Public(seed)), seal: hex.encode(new Uint8Array(await crypto.subtle.exportKey("raw", seal.publicKey))), client, user };
    // Its name is the client's own, so where its sign-in goes is shown beside it: a name says nothing a client can't.
    const about = await this.env.AUTH.fetch(`${origin(this.env)}/sign-in/about?client_id=${encodeURIComponent(client)}`).then((r) => (r.ok ? r.json() : {}), () => ({}));
    const called = String(about.client || "MCP client").slice(0, 40);
    device.name = (about.goes ? `${called} (${String(about.goes).slice(0, 20)})` : called).slice(0, 64);
    device.code = await joinCode(hex.decode(device.key), hex.decode(device.seal));
    await this.end();
    const was = await this.ctx.storage.get("device");
    const replaces = [...((await this.ctx.storage.get("replaces")) || []), ...(was?.key ? [was.key] : [])].slice(-4);
    await this.ctx.storage.delete(["needs", "tools", "order", "join"]);
    await this.ctx.storage.delete([...(await this.ctx.storage.list({ prefix: "needs:" })).keys()]);
    await this.ctx.storage.put({ device, replaces });
    await this.copies.start(new Uint8Array([...seed, ...sealSecret]), refresh, access, until);
    await this.join(device, access);
  }

  /// A device's stages: asking sign-in to let it join, waiting there (`join`, until when sign-in keeps its request), approved
  /// (the relay lets its key in), and running (`run`). Asks sign-in, unless it's waiting there: as every client does, one
  /// sign-in forgot (after 15 minutes) or didn't take is asked again, so its code always has a request behind it.
  async join(device, access) {
    const waiting = await this.ctx.storage.get("join");
    if (waiting?.key === device.key && waiting.until > Date.now()) return;
    let until = Date.now() + JOIN_AGAIN;
    try {
      const r = await this.env.AUTH.fetch(`${origin(this.env)}/api/auth/devices/requests`, {
        method: "POST",
        headers: { authorization: `Bearer ${access}`, "content-type": "application/json" },
        body: JSON.stringify({ key: device.key, seal: device.seal, name: device.name, reads: READS }),
      });
      if (r.ok) until = Math.max(until, Number((await r.json().catch(() => ({}))).until) - JOIN_EARLY || 0);
      else console.log(JSON.stringify({ join: "refused", status: r.status, why: (await r.text()).slice(0, 200) }));
    } catch (err) {
      console.log(JSON.stringify({ join: "unasked", why: String(err?.message ?? err) }));
    }
    await this.ctx.storage.put("join", { key: device.key, until });
  }

  /// An MCP request with `token`, an access token until `until` (ms): its device answers it, or says why not. →
  /// `{ status, headers, body }`, or `{ signIn }`, why it must sign in again.
  async asked(token, until, headers, body) {
    this.asking++;
    this.awake();
    try {
      const secret = await this.copies.open(token);
      const device = await this.ctx.storage.get("device");
      if (!secret || !device) return { signIn: "this sign-in opens nothing here: sign in again" };
      const me = { seed: secret.slice(0, 32), seal: { secret: secret.slice(32), public: hex.decode(device.seal) }, key: device.key, user: device.user, until };
      const approve = `Approve ${device.name} on one of your devices: in Settings › Devices, enter ${device.code.slice(0, 4)} ${device.code.slice(4)}.`;
      if (!(await this.approved(device))) {
        await this.join(device, token);
        return answerAll(body, approve);
      }
      let nodes;
      try {
        nodes = await this.start(me);
      } catch (err) {
        // Taken out since it was approved: the relay no longer lets its key in, nor the entries list it.
        this.isApproved = false;
        return answerAll(body, /isn't one of the network's|not listed|no keys of your/.test(err.message) ? approve : `${device.name} couldn't start: ${err.message}`);
      }
      if (!nodes.length) nodes = await this.reorder(me);
      if (!nodes.length) return answerAll(body, "None of your devices is on right now.");
      this.ctx.waitUntil(this.removeReplaced(me));
      return await this.forward(me, headers, body);
    } finally {
      this.asking--;
      this.awake();
    }
  }

  async approved(device) {
    if (this.isApproved) return true;
    this.isApproved = !!(await this.relay().allowed(device.key));
    return this.isApproved;
  }

  async start(me) {
    if (this.run && this.run.key !== me.key) await this.end();
    // One it's closing is closed first, as it's the same endpoint.
    while (!this.run && this.ending) await this.ending;
    let run = this.run;
    if (!run) {
      run = this.run = { key: me.key, phase: "starting", ticket: null, until: 0 };
      run.ready = this.begin(run, me);
      run.ready.catch(() => this.run === run && this.end());
    } else if (run.phase === "running" && Date.now() - (this.view?.at || 0) > VIEW_EVERY) {
      this.ctx.waitUntil(this.see(me).catch((err) => console.log(JSON.stringify({ view: "unread", why: String(err?.message ?? err) }))));
    }
    await run.ready;
    if (this.run !== run) throw new Error("it was signed in again meanwhile: ask again");
    // A busy device outlives its binding, which the relay keeps an hour: bound again before it lapses.
    if (run.until - Date.now() < BIND_AGAIN) {
      run.binding ||= this.bind(run, me).finally(() => (run.binding = null));
      if (run.until - Date.now() < 30_000) await run.binding;
      else this.ctx.waitUntil(run.binding.catch((err) => console.log(JSON.stringify({ bound: "again failed", why: String(err?.message ?? err) }))));
    }
    return this.order;
  }

  async begin(run, me) {
    const began = Date.now();
    const ended = () => {
      if (this.run !== run) throw new Error("it was signed in again meanwhile: ask again");
    };
    const send = (dst, seg, contents) => {
      this.out.push([hex.encode(dst), seg, contents]);
      if (this.out.length > 1) return;
      queueMicrotask(() => {
        const items = this.out;
        this.out = [];
        if (!run.ticket) return;
        this.sent = this.relay().fromClient(run.key, run.ticket, items).catch(() => {});
        this.ctx.waitUntil(this.sent);
      });
    };
    await this.unbindStale();
    ended();
    await this.bind(run, me);
    ended();
    await device_start(me.seed, [], send);
    run.endpoint = true;
    ended();
    await this.see(me);
    ended();
    if (!this.view.listed) throw new Error(`this device is not listed as one of ${this.env.YOURS}'s`);
    run.phase = "running";
    console.log(JSON.stringify({ started: me.key.slice(0, 8), ms: Date.now() - began, listed: this.view.nodes.size, on: this.order.length }));
  }

  /// Binds its key to this object at the relay, which hands it what's sent to it, for an hour: again, once bound,
  /// before that ends, which gives it a new ticket.
  async bind(run, me) {
    const name = await this.ctx.storage.get("name");
    let bound;
    await untilTaken(async (at) => {
      bound = await this.relay().bind(await signedRequest(me.seed, hex.decode(me.key), RELAY, { bind: true }, at), name);
      return bound?.error;
    });
    if (!bound?.ticket) throw new Error(`the relay didn't take its key: ${bound?.error}`);
    run.ticket = bound.ticket;
    run.until = Number(bound.until) || Date.now() + BOUND - 60_000;
    await this.ctx.storage.put("bound", { key: run.key, ticket: bound.ticket });
  }

  /// Reads who's who from the bucket (what's new, after the first time), one reading at a time, and the nodes it asks
  /// with it: those the entries list as running its person's calls, that are on the relay now, in run priority's
  /// order as it last read it.
  see(me) {
    const reading = this.viewing.then(async () => {
      const said = JSON.parse(await device_view(me.key, me.user, me.seal.secret, new Bucket(this.env, me)));
      const nodes = new Map(said.nodes.map((n) => [n.key, { seal: n.seal, scopes: new Set(n.scopes) }]));
      this.view = { listed: said.listed, nodes, at: Date.now() };
      await this.reorder(me);
    });
    this.viewing = reading.catch(() => {});
    return reading;
  }

  async reorder(me) {
    const nodes = this.view?.nodes || new Map();
    const online = new Set(await this.relay().connected([...nodes.keys()]));
    const kept = ((await this.ctx.storage.get("order")) || []).filter((k) => nodes.has(k));
    this.order = [...kept.filter((k) => online.has(k)), ...[...online].filter((k) => !kept.includes(k))];
    if (this.run?.endpoint && this.run.key === me.key) device_nodes(me.key, this.order);
    return this.order;
  }

  /// Reads run priority's order from the network's state, for the nodes it asks: only nodes the entries list are
  /// ever in it, whatever the state says.
  async learnOrder(me) {
    if (!this.run || this.run.learned) return;
    this.run.learned = true;
    try {
      const r = await device_fetch(me.key, "GET", "/v1/state", [], new Uint8Array());
      if (r.status !== 200 || !this.view?.nodes.has(r.node)) return;
      const state = JSON.parse(new TextDecoder().decode(r.body));
      const listed = this.view.nodes;
      const order = (state.network?.devices || []).filter((d) => listed.has(d.key) && Number.isFinite(d.order)).sort((a, b) => a.order - b.order).map((d) => d.key).slice(0, 64);
      await this.ctx.storage.put("order", order);
      const online = new Set(this.order);
      this.order = [...order.filter((k) => online.has(k)), ...this.order.filter((k) => !order.includes(k))];
      device_nodes(me.key, this.order);
    } catch {}
  }

  async needsOf(tool) {
    return (tool && (await this.ctx.storage.get(`needs:${tool}`))) || [];
  }

  /// Keeps what `tool` needed besides what it kept, at most `NEEDS_MOST`, and `tool` among the `TOOLS_MOST` lately
  /// used.
  async remember(tool, needs) {
    if (!tool) return;
    const kept = await this.needsOf(tool);
    for (const n of needs) if (!kept.some((k) => sameNeed(k, n)) && kept.length < NEEDS_MOST) kept.push(n);
    const tools = ((await this.ctx.storage.get("tools")) || []).filter((t) => t !== tool);
    tools.push(tool);
    const gone = tools.splice(0, Math.max(0, tools.length - TOOLS_MOST));
    if (gone.length) await this.ctx.storage.delete(gone.map((t) => `needs:${t}`));
    await this.ctx.storage.put({ [`needs:${tool}`]: kept, tools });
  }

  /// Asks a node's `/mcp` as this device, handing the keys its tool needed before, for the scopes that node runs; a 428
  /// says what else it needs, which it hands if the node that answered is the one it names, by its listed seal key,
  /// and runs those scopes, asking once more.
  async forward(me, headers, body) {
    const bytes = new TextEncoder().encode(body);
    const tool = toolOf(body);
    let handing = await this.handoff(me, this.order[0], tool, await this.needsOf(tool), false).catch(() => null);
    for (let tries = 0; ; tries++) {
      const said = Object.entries(handing ? { ...headers, "x-one-keys": handing.sealed } : headers).flat();
      let r;
      try {
        r = await device_fetch(me.key, "POST", "/mcp", said, bytes);
      } catch (err) {
        if (err.sent) return answerAll(body, `Your device stopped answering: ${err.message}`);
        if (tries === 0 && (await this.reorder(me)).length) {
          handing = null;
          continue;
        }
        return answerAll(body, "None of your devices is on right now.");
      }
      const node = this.view.nodes.get(r.node);
      if (!node) return answerAll(body, `A device ${this.env.YOURS} doesn't list as running your calls answered: it was given nothing.`);
      this.ctx.waitUntil(this.learnOrder(me));
      if (r.status === 400 && handing && handing.to !== r.node && tries === 0) {
        handing = null;
        continue;
      }
      if (r.status === 428 && tries < 2) {
        let need = {};
        try {
          need = JSON.parse(new TextDecoder().decode(r.body));
        } catch {}
        if (need.to !== r.node || need.seal !== node.seal) return answerAll(body, "A device asked for keys sealed to another than itself: it was given none.");
        const wanted = (Array.isArray(need.need) ? need.need : []).map(needOf).filter(Boolean).slice(0, NEEDS_MOST);
        const runs = wanted.filter((n) => node.scopes.has(n.scope));
        if (runs.length < wanted.length) return answerAll(body, "That needs keys of a scope the device that answered doesn't run for you: it was given none.");
        if (!wanted.every((n) => mayHand("/mcp", n))) return answerAll(body, "A device asked for keys an MCP request doesn't use: it was given none.");
        await this.remember(tool, runs);
        const handOver = tool ? await this.needsOf(tool) : runs;
        handing = await this.handoff(me, r.node, tool, handOver.concat(runs.filter((n) => !handOver.some((k) => sameNeed(k, n)))), true).catch(() => null);
        if (handing) continue;
      }
      if (r.status === 428) {
        let error = null;
        try {
          error = JSON.parse(new TextDecoder().decode(r.body)).error;
        } catch {}
        return answerAll(body, typeof error === "string" ? error.slice(0, 500) : "That needs keys this device wasn't given.");
      }
      const out = {};
      for (let i = 0; i < r.headers.length; i += 2) if (FROM_NODE.includes(r.headers[i])) out[r.headers[i]] = r.headers[i + 1];
      if (handing?.to === r.node) {
        const was = this.handed.get(r.node);
        const tools = was?.until === me.until ? was.tools : new Set();
        tools.add(tool);
        this.handed.set(r.node, { until: me.until, tools });
      }
      return { status: r.status, headers: out, body: r.body };
    }
  }

  /// What it hands `node` with a call of `tool`: the keys of `needs` in the scopes the entries say it runs for this
  /// device's person, each epoch it opens, sealed to that node's listed seal key, for this device's key, until its
  /// access token expires. Null if the node has them already (unless `again`), or there's nothing to hand.
  async handoff(me, to, tool, needs, again) {
    const node = to && this.view?.nodes.get(to);
    if (!node) return null;
    const handed = this.handed.get(to);
    if (!again && handed?.until === me.until && handed.tools.has(tool)) return null;
    const theirs = needs.filter((n) => node.scopes.has(n.scope));
    if (!theirs.length) return null;
    const keys = JSON.parse(await device_keys(me.key, JSON.stringify(theirs), new Bucket(this.env, me)));
    if (!keys.length) return null;
    return { to, sealed: await sealHandoff(hex.decode(node.seal), { for: me.key, until: me.until, keys }) };
  }

  /// Removes the devices this one replaced, made by earlier sign-ins of the same client, once this one's approved: as
  /// its person, by the node it asks, which takes each off the relay and lists it as removed. Tried once a device each
  /// time the object starts.
  async removeReplaced(me) {
    if (this.removedFor === me.key) return;
    this.removedFor = me.key;
    const replaced = (await this.ctx.storage.get("replaces")) || [];
    if (!replaced.length) return;
    try {
      const left = [];
      for (const key of replaced) {
        const r = await device_fetch(me.key, "POST", "/v1/remove", ["content-type", "application/json"], new TextEncoder().encode(JSON.stringify({ key }))).catch(() => null);
        const why = r ? new TextDecoder().decode(r.body) : "";
        if (!r || (r.status !== 200 && !/no such/.test(why))) left.push(key);
        console.log(JSON.stringify({ removed: key.slice(0, 8), status: r?.status, why: r?.status === 200 ? undefined : why.slice(0, 200) }));
      }
      await this.ctx.storage.put("replaces", left);
    } catch {}
  }

  /// Datagrams the relay was handed for its key: `[src, segment size, contents]` each, dropped if its endpoint isn't
  /// running (the WebAssembly keeps no other). One for another key is dropped; one for its own while it isn't running
  /// is from a binding it couldn't end, which ends now.
  async deliver(key, batch) {
    if (this.run?.endpoint && this.run.key === key) {
      for (const [src, seg, contents] of batch) device_deliver(hex.decode(key), hex.decode(src), seg, contents);
      return;
    }
    if (!this.run) await this.unbindStale(key);
  }

  /// Ends a binding at the relay this object made and couldn't end (`key`, if given, only if it's that one's).
  async unbindStale(key) {
    const bound = await this.ctx.storage.get("bound");
    if (!bound || (key && bound.key !== key)) return;
    await this.relay().unbind(bound.key, bound.ticket).catch(() => {});
    if ((await this.ctx.storage.get("bound"))?.ticket === bound.ticket) await this.ctx.storage.delete("bound");
  }

  /// iroh always has a timer pending, which would keep the object awake, and billed: once nobody has asked for `IDLE`,
  /// an alarm ends it.
  awake() {
    this.last = Date.now();
    if (!this.alarmAt || this.alarmAt < this.last + this.idle() / 2) {
      this.alarmAt = this.last + this.idle();
      this.ctx.storage.setAlarm(this.alarmAt);
    }
  }

  idle() {
    return Number(this.env.IDLE) || IDLE;
  }

  async alarm() {
    this.alarmAt = null;
    if (this.asking) return this.awake();
    if (Date.now() - this.last < this.idle() - 1_000) {
      this.alarmAt = this.last + this.idle();
      return this.ctx.storage.setAlarm(this.alarmAt);
    }
    if (!this.run) return;
    await this.end();
    // Asked while it closed: it's that request's now, which started another.
    if (this.asking || this.run) return this.awake();
    this.ctx.abort("idle");
  }

  /// Closes its endpoint and unbinds its key, once it's started: its key and the keys it opened leave memory with the
  /// object. A start waits for it (`ending`).
  async end() {
    const run = this.run;
    this.run = this.view = null;
    this.isApproved = false;
    this.handed = new Map();
    this.order = [];
    if (!run) return this.ending;
    run.phase = "ending";
    const before = this.ending;
    const ending = (this.ending = (async () => {
      await before;
      await run.ready?.catch(() => {});
      await run.binding?.catch(() => {});
      if (run.endpoint) await device_close(run.key);
      await this.sent;
      if (run.ticket) {
        await this.relay().unbind(run.key, run.ticket).catch(() => {});
        const bound = await this.ctx.storage.get("bound");
        if (bound?.ticket === run.ticket) await this.ctx.storage.delete("bound");
      }
      console.log(JSON.stringify({ sleeps: run.key.slice(0, 8) }));
    })());
    try {
      await ending;
    } finally {
      if (this.ending === ending) this.ending = null;
    }
  }
}
