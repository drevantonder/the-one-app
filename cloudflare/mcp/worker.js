// The remote MCP server: remote MCP clients' device. ChatGPT, Claude.ai and the like can't run our code, so each one a user
// signs in with is a device of theirs here, a client of the network like any app, which the user approves like any
// device. It speaks MCP to them over HTTPS at `<address>/mcp`, and asks a node's own `/mcp` as that device, over iroh
// (`<cli>/http/1`, one-client in cloudflare/node/src/clients.rs), handing it the keys a call needs, sealed to that node and
// bound to the device's key; it sends nodes no token. A tool that changes who can do what it answers itself, signing
// as that device (`one::settings_tools`), since a node signs that only for its own machine's programs.
//
// A device's secret is kept only sealed under its client's own tokens (copies.js), so nothing here opens it alone: the
// Worker is on its clients' token path (its own token endpoint, in the authorization server's metadata it serves for
// MCP clients, passing each request on to auth), sealing the secret under each new refresh and access token. A client's
// first sign-in makes its device and asks auth to let it join; the user approves it by its code on one of their
// devices, which lists it, seals it the keys of their scopes and lets it in at the relay. Until then its MCP requests
// are answered with that code. Signing in again makes a new device, as the old one's secret opened only under the
// tokens it had.
//
// Each device is a Durable Object (`Client`), one per client and user, so no two users' state meet: its iroh
// endpoint, with no socket, which the relay hands the datagrams sent to its key by RPC (`deliver`), its connections to
// nodes kept warm between calls, and the keys it opened, all while it's asked; once nobody has asked for `IDLE`, it
// closes its endpoint, forgets its keys and ends itself.
//
// It trusts no node for who's who or for keys, as every client device: what it knows, the keys it opens and the 428s it
// takes are one-client's keyring (client/src/keyring.rs, through cloudflare/node/src/clients.rs), which keeps what each
// tool needed in this object's storage, so a call hands only its own.
//
// Bindings: AUTH (auth), RELAY (the relay's `McpDevices` entrypoint), SIGNER (the URL signer), CLIENT
// (this Worker's `Client`), ADDRESS (the network's), NAME (its name), LOCATION (where objects are made), IDLE (ms,
// optional).

import { DurableObject } from "cloudflare:workers";
import { initSync, device_start, device_deliver, device_fetch, device_nodes, device_close, device_view } from "./one_cloudflare_node.js";
import wasm from "./one_cloudflare_node_bg.wasm";
import { b64u, ed25519Public, hex, joinCode, random, signBytes, signedRequest, untilTaken } from "../shared/crypto.js";
import { CORS, json, near, textOf } from "../shared/http.js";
import { CODES, IROH_PATHS, READS, RELAY } from "../shared/protocol.js";
import { Issuer, bearer } from "../shared/token.js";
import { Copies } from "./copies.js";

initSync({ module: wasm });

const IDLE = 30_000;
const TO_NODE = ["content-type", "accept", "mcp-session-id", "mcp-protocol-version"];
const FROM_NODE = ["content-type", "mcp-session-id"];
const BODY_MOST = 1 << 20;
const VIEW_EVERY = 10_000;
/// How long the relay keeps a device's binding, if it doesn't say, and how long before that ends it's bound again.
const BOUND = 3_600_000;
const BIND_AGAIN = 600_000;
/// Where its storage keeps what its device's keyring keeps, by name.
const KEPT = "kept:";
/// How long before auth forgets a request to join it's asked again, and how soon one auth didn't take is.
const JOIN_EARLY = 60_000;
const JOIN_AGAIN = 30_000;

const origin = (env) => new URL(env.ADDRESS).origin;

let issuer = null;
const issuerOf = (env) => (issuer ||= new Issuer(`${origin(env)}/api/auth`, () => env.AUTH.fetch(`${origin(env)}/api/auth/jwks`)));

/// A token's claims, unchecked: only for one auth just gave us by its binding.
const claimsOf = (token) => JSON.parse(new TextDecoder().decode(b64u.decode(token.split(".")[1])));
/// The `code` a node's refusal carries, if the body is one: a node's own body is JSON, but a proxy's isn't.
const codeOf = (body) => {
  try {
    return JSON.parse(new TextDecoder().decode(body))?.code;
  } catch {}
};

const deviceOf = (env, client, user) => env.CLIENT.get(env.CLIENT.idFromName(`${client}\n${user}`), near(env));

/// What an MCP client is told when it has no sign-in, or one that won't do: where to sign in (RFC 9728).
function signInFirst(env, error) {
  const invalid = error ? 'error="invalid_token", ' : "";
  const challenge = `Bearer ${invalid}resource_metadata="${origin(env)}/.well-known/oauth-protected-resource/mcp", scope="openid profile offline_access"`;
  return json({ error: error || `sign in at ${origin(env)} first` }, 401, { "www-authenticate": challenge });
}

/// The authorization server's metadata, as auth says it, but with this Worker's token endpoint: what MCP clients find
/// from the resource's (RFC 8414's path for the issuer `<address>/api/auth`). Apps read auth's own, at the root.
async function authorizationServer(env, which) {
  const r = await env.AUTH.fetch(`${origin(env)}/.well-known/${which}`);
  if (!r.ok) return r;
  const meta = await r.json();
  meta.token_endpoint = `${origin(env)}/mcp/token`;
  return json(meta, 200, CORS);
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
  const body = await textOf(request, BODY_MOST);
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
    // Auth rotated the refresh token, but its device's secret isn't sealed under the new one: the client isn't given
    // tokens that open nothing, and asks again with the old refresh token, which auth answers the same for a minute.
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
  const body = await textOf(request, BODY_MOST);
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

export class Client extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.copies = new Copies(ctx.storage);
    this.asking = 0;
    this.out = [];
    /// Who's who as it last read it: `{ nodes: Map(key → seal), at }`.
    this.view = null;
    this.viewing = Promise.resolve();
    /// Its endpoint, while it's awake: `{ key, phase, ready, ticket, until }`, one at a time, `phase` "starting" until
    /// `ready` (it's bound at the relay, its endpoint started, and it's read who's who), then "running"; none while it's
    /// asleep. `end` takes it away at once, so what's asked after starts another, and closes it once it's started: a
    /// start it ends midway stops there.
    this.run = null;
  }

  relay() {
    return this.env.RELAY;
  }

  /// A client signed in (`old` null) or refreshed its sign-in: its device's secret, sealed under its new tokens (auth's
  /// same answer again, to a refresh whose answer was lost, opens it as it is). A sign-in, or a refresh whose old token
  /// opens nothing, makes a new device and asks auth to let it join; the device it replaces is removed once the new one
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
    device.name = (about.goes ? `${called} (${about.goes})` : called).slice(0, 64);
    device.code = await joinCode(hex.decode(device.key), hex.decode(device.seal));
    await this.end();
    const was = await this.ctx.storage.get("device");
    const replaces = [...((await this.ctx.storage.get("replaces")) || []), ...(was?.key ? [was.key] : [])].slice(-4);
    await this.ctx.storage.delete(["order", "join"]);
    await this.ctx.storage.delete([...(await this.ctx.storage.list({ prefix: KEPT })).keys()]);
    await this.ctx.storage.put({ device, replaces });
    await this.copies.start(new Uint8Array([...seed, ...sealSecret]), refresh, access, until);
    await this.join(device, access);
  }

  /// A device's stages: asking auth to let it join, waiting there (`join`, until when auth keeps its request), approved
  /// (the relay lets its key in), and running (`run`). Asks auth, unless it's waiting there: as every client does, one
  /// auth forgot (after 15 minutes) or didn't take is asked again, so its code always has a request behind it.
  async join(device, access) {
    const waiting = await this.ctx.storage.get("join");
    if (waiting?.key === device.key && waiting.until > Date.now()) return;
    let until = Date.now() + JOIN_AGAIN;
    try {
      const r = await this.env.AUTH.fetch(`${origin(this.env)}/api/auth/devices/requests`, {
        method: "POST",
        headers: { authorization: `Bearer ${access}`, "content-type": "application/json" },
        body: JSON.stringify({ key: device.key, seal: device.seal, name: device.name, platform: "cloudflare", reads: READS }),
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
      const approve = `Approve ${device.name} on one of your devices: in Settings › Devices, enter ${device.code.replace(/(.{4})(?=.)/g, "$1 ")}.`;
      if (!(await this.approved(device))) {
        await this.join(device, token);
        return answerAll(body, approve);
      }
      let nodes;
      try {
        nodes = await this.start(me);
      } catch (err) {
        // Taken out since it was approved: the relay no longer lets its key in, nor the network lists it.
        this.isApproved = false;
        return answerAll(body, err.notLetIn || err.code === CODES.notLetIn ? approve : `${device.name} couldn't start: ${err.message}`);
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
      return bound?.error ? bound : null;
    });
    if (!bound?.ticket) throw Object.assign(new Error(`the relay didn't take its key: ${bound?.error}`), { code: bound?.code });
    run.ticket = bound.ticket;
    run.until = Number(bound.until) || Date.now() + BOUND - 60_000;
    await this.ctx.storage.put("bound", { key: run.key, ticket: bound.ticket });
  }

  /// Reads who's who from the bucket (what's new, after the first time), one reading at a time, and the nodes it asks
  /// with it: those who's who lists as running its user's calls, that are on the relay now, in run priority's
  /// order as it last read it.
  see(me) {
    const reading = this.viewing.then(async () => {
      const said = JSON.parse(await device_view(me.key, me.seal.secret, this.on(me)));
      if (said.user !== me.user) throw Object.assign(new Error(`this device is not listed as one of ${this.env.YOURS}'s`), { notLetIn: true });
      this.view = { nodes: new Map(said.nodes.map((n) => [n.key, n.seal])), at: Date.now() };
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

  /// Reads run priority's order from the network's state, for the nodes it asks: only nodes the network lists are
  /// ever in it, whatever the state says.
  async learnOrder(me) {
    if (!this.run || this.run.learned) return;
    this.run.learned = true;
    try {
      const r = await device_fetch(me.key, "GET", "/v1/state", [], new Uint8Array(), me.until, this.on(me));
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

  /// What its device's keyring is handed (cloudflare/node/src/clients.rs `On`, one-client's `js::On`): the network's
  /// address, and its URL signer, which the keyring reads the bucket through, auth, which it asks of devices waiting to
  /// join and invites, and the relay, which it tells as it takes a device out (by their bindings: a Worker can't fetch
  /// its own zone's routes); its key's signature; this object's storage; and whether the relay still lets it in.
  on(me) {
    const { storage } = this.ctx;
    const env = this.env;
    const to = { "/urls": env.SIGNER, [`${IROH_PATHS.relay}/tell`]: env.RELAY };
    return {
      address: origin(env),
      async ask(method, url, name, value, body) {
        const path = new URL(url).pathname;
        const binding = path.startsWith("/api/auth/") ? env.AUTH : to[path];
        if (!binding) throw new Error("the remote MCP server's devices ask only the URL signer and auth, and tell only the relay");
        const headers = { "content-type": "application/json", ...(name ? { [name]: value } : {}) };
        const r = await binding.fetch(url, { method, headers, body });
        return { status: r.status, body: await r.text() };
      },
      sign: (bytes) => signBytes(me.seed, bytes),
      get: async (name) => (await storage.get(KEPT + name)) ?? null,
      put: (name, bytes) => storage.put(KEPT + name, bytes),
      delete: (name) => storage.delete(KEPT + name),
      letIn: async () => !!(await this.relay().allowed(me.key)),
    };
  }

  /// Asks a node's `/mcp` as this device, with the keys its tool needs (the keyring's 428s); once more, if no node
  /// answered, with the nodes on the relay now.
  async forward(me, headers, body) {
    const bytes = new TextEncoder().encode(body);
    for (let tries = 0; ; tries++) {
      let r;
      try {
        r = await device_fetch(me.key, "POST", "/mcp", Object.entries(headers).flat(), bytes, me.until, this.on(me));
      } catch (err) {
        if (err.sent) return answerAll(body, `Your device stopped answering: ${err.message}`);
        if (tries === 0 && (await this.reorder(me)).length) continue;
        return answerAll(body, this.order.length ? err.message : "None of your devices is on right now.");
      }
      this.ctx.waitUntil(this.learnOrder(me));
      if (r.status === 428) {
        let error = null;
        try {
          error = JSON.parse(new TextDecoder().decode(r.body)).error;
        } catch {}
        return answerAll(body, typeof error === "string" ? error.slice(0, 500) : "That needs keys this device wasn't given.");
      }
      const out = {};
      for (let i = 0; i < r.headers.length; i += 2) if (FROM_NODE.includes(r.headers[i])) out[r.headers[i]] = r.headers[i + 1];
      return { status: r.status, headers: out, body: r.body };
    }
  }

  /// Removes the devices this one replaced, made by earlier sign-ins of the same client, once this one's approved: as
  /// its user's device, which takes each off the relay and signs it removed. Tried once a device each time the object
  /// starts.
  async removeReplaced(me) {
    if (this.removedFor === me.key) return;
    this.removedFor = me.key;
    const replaced = (await this.ctx.storage.get("replaces")) || [];
    const left = [];
    for (const key of replaced) {
      try {
        const r = await device_fetch(me.key, "POST", "/v1/devices/remove", ["content-type", "application/json"], new TextEncoder().encode(JSON.stringify({ device: key })), me.until, this.on(me));
        if (r.status !== 200 && codeOf(r.body) !== CODES.notListed) throw new Error(`the node said ${r.status}: ${new TextDecoder().decode(r.body).slice(0, 200)}`);
      } catch (err) {
        console.warn(`removing the device ${key.slice(0, 8)} this one replaced: ${err.message}`);
        left.push(key);
      }
    }
    if (replaced.length) await this.ctx.storage.put("replaces", left);
    if (left.length) this.removedFor = null;
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
