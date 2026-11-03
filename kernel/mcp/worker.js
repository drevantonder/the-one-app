// The MCP Worker: remote MCP clients' device. ChatGPT, Claude.ai and the like can't run our code, so each one a person
// signs in with is a device of theirs here, a client of the install like any app, which the person approves like any
// device. It speaks MCP to them over HTTPS at `<address>/mcp`, and asks a node's own `/mcp` as that device, over iroh
// (`<cli>/http/1`, one-client in serverless/src/clients.rs), handing it the keys a call needs, sealed to that node and
// bound to the device's key; it sends nodes no token.
//
// A device's secret is kept only sealed under its client's own tokens (copies.js), so nothing here opens it alone: the
// Worker is on its clients' token path (its own token endpoint, in the authorization server's metadata it serves for
// MCP clients, passing each request on to auth), sealing the secret under each new refresh and access token. A client's
// first sign-in makes its device and asks auth to let it join; the person approves it by its code on one of their
// devices, which lists it, seals it the keys of their scopes and lets it in at the relay. Until then its MCP requests
// are answered with that code. Signing in again makes a new device, as the old one's secret opened only under the
// tokens it had.
//
// Each device is a Durable Object (`Client`), one per client and person, so no two people's state meet: its iroh
// endpoint, with no socket, which the relay hands the datagrams sent to its key by RPC (`deliver`), its connections to
// nodes kept warm between calls, and the keys it opened, all while it's asked; once nobody has asked for `IDLE`, it
// closes its endpoint, forgets its keys and ends itself.
//
// Bindings: AUTH (the auth service), RELAY (the relay's Durable Object), CLIENT (this Worker's `Client`), ADDRESS (the
// install's), NAME (its name), LOCATION (where the relay is made), IDLE (ms, optional).

import { DurableObject } from "cloudflare:workers";
import { initSync, device_start, device_deliver, device_fetch, device_nodes, device_close } from "./one_serverless.js";
import wasm from "./one_serverless_bg.wasm";
import { b64u, databaseKey, ed25519Public, hex, joinCode, openAppKey, openSealed, random, sealAppKey, sealHandoff, signedRequest } from "../shared/crypto.js";
import { Issuer, bearer } from "../shared/token.js";
import { Copies } from "./copies.js";

initSync({ module: wasm });

const IDLE = 30_000;
/// What a device asks the relay with, and the MCP requests' headers a node is given and gives back.
const RELAY = "one/relay";
const TO_NODE = ["content-type", "accept", "mcp-session-id", "mcp-protocol-version"];
const FROM_NODE = ["content-type", "mcp-session-id"];

const json = (v, status = 200, headers = {}) => Response.json(v, { status, headers });
const near = (env) => (env.LOCATION ? { locationHint: env.LOCATION } : {});
const origin = (env) => new URL(env.ADDRESS).origin;
const CORS = { "access-control-allow-origin": "*" };

/// The auth service's keys, as every service checks its tokens.
let issuer = null;
const issuerOf = (env) => (issuer ||= new Issuer(`${origin(env)}/api/auth`, () => env.AUTH.fetch(`${origin(env)}/api/auth/jwks`)));

/// A token's claims, unchecked: only for one auth just gave us by its binding.
const claimsOf = (token) => JSON.parse(new TextDecoder().decode(b64u.decode(token.split(".")[1])));

/// The device of a client and person.
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

/// A token request, passed on to auth: its answer, once the client's device secret is sealed under the new tokens.
async function token(request, env) {
  const body = await request.text();
  const asked = new URLSearchParams(body);
  const headers = { "content-type": request.headers.get("content-type") || "application/x-www-form-urlencoded" };
  if (request.headers.has("authorization")) headers.authorization = request.headers.get("authorization");
  const r = await env.AUTH.fetch(`${origin(env)}/api/auth/oauth2/token`, { method: "POST", headers, body });
  const out = await r.json().catch(() => null);
  if (!r.ok || !out?.access_token || !out?.refresh_token) return out ? json(out, r.status, { "cache-control": "no-store" }) : new Response(null, { status: r.status });
  const claims = claimsOf(out.access_token);
  const client = claims.azp || claims.client_id || asked.get("client_id");
  const grant = asked.get("grant_type");
  const old = grant === "refresh_token" ? asked.get("refresh_token") : null;
  await deviceOf(env, client, claims.sub).signedIn({ client, user: claims.sub, old, refresh: out.refresh_token, access: out.access_token, until: claims.exp * 1000 });
  return json(out, 200, { "cache-control": "no-store", pragma: "no-cache" });
}

/// An MCP request: its device answers it, if its access token opens the device.
async function mcp(request, env) {
  const token = bearer(request);
  if (!token) return signInFirst(env);
  let claims;
  try {
    claims = await issuerOf(env).verify(token);
  } catch (err) {
    return signInFirst(env, `that sign-in won't do: ${err.message}`);
  }
  const aud = [claims.aud].flat();
  if (!aud.includes(env.ADDRESS) && !aud.includes(`${origin(env)}/mcp`)) return signInFirst(env, `that sign-in isn't for ${origin(env)}`);
  if (request.method !== "POST") return new Response(null, { status: 405, headers: { allow: "POST" } });
  const headers = {};
  for (const name of TO_NODE) if (request.headers.has(name)) headers[name] = request.headers.get(name);
  const out = await deviceOf(env, claims.azp || claims.client_id, claims.sub).asked(token, claims.exp * 1000, headers, await request.text());
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

/// One remote MCP client's device, for one person.
export class Client extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.copies = new Copies(ctx.storage);
    this.asking = 0;
    /// Datagrams sent this turn, for the relay.
    this.out = [];
  }

  relay() {
    return this.env.RELAY.get(this.env.RELAY.idFromName("relay"), near(this.env));
  }

  /// A client signed in (`old` null) or refreshed its sign-in: its device's secret, sealed under its new tokens. A
  /// sign-in, or a refresh whose old token opens nothing, makes a new device and asks auth to let it join.
  async signedIn({ client, user, old, refresh, access, until }) {
    await this.ctx.storage.put("name", `${client}\n${user}`);
    if (old && (await this.copies.rotate(old, refresh, access, until))) return;
    const seed = random(32);
    const seal = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits"]);
    const sealSecret = b64u.decode((await crypto.subtle.exportKey("jwk", seal.privateKey)).d);
    const device = { key: hex.encode(await ed25519Public(seed)), seal: hex.encode(new Uint8Array(await crypto.subtle.exportKey("raw", seal.publicKey))), client, user };
    const about = await this.env.AUTH.fetch(`${origin(this.env)}/sign-in/about?client_id=${encodeURIComponent(client)}`).then((r) => (r.ok ? r.json() : {}), () => ({}));
    device.name = String(about.client || "MCP client").slice(0, 64);
    device.code = await joinCode(hex.decode(device.key), hex.decode(device.seal));
    await this.end();
    await this.ctx.storage.delete(["epochs", "apps", "needs", "seals", "approved"]);
    await this.ctx.storage.put("device", device);
    await this.copies.start(new Uint8Array([...seed, ...sealSecret]), refresh, access, until);
    const r = await this.env.AUTH.fetch(`${origin(this.env)}/api/auth/devices/requests`, {
      method: "POST",
      headers: { authorization: `Bearer ${access}`, "content-type": "application/json" },
      body: JSON.stringify({ key: device.key, seal: device.seal, name: device.name }),
    });
    if (!r.ok) console.log(JSON.stringify({ join: "refused", status: r.status, why: (await r.text()).slice(0, 200) }));
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
      const me = { seed: secret.slice(0, 32), seal: { secret: secret.slice(32), public: hex.decode(device.seal) }, key: device.key, until };
      const approve = `Approve ${device.name} on one of your devices: in Settings › Devices, enter ${device.code.slice(0, 4)} ${device.code.slice(4)}.`;
      if (!(await this.approved(device))) return answerAll(body, approve);
      let nodes;
      try {
        nodes = await this.start(me);
      } catch (err) {
        // Taken out since it was approved: the relay no longer lets its key in.
        this.isApproved = false;
        return answerAll(body, /isn't one of the install's/.test(err.message) ? approve : `${device.name} couldn't start: ${err.message}`);
      }
      if (!nodes.length) return answerAll(body, "None of your devices is on right now.");
      return await this.forward(me, headers, body);
    } finally {
      this.asking--;
      this.awake();
    }
  }

  /// Whether a device of its person approved it: the relay lets it in.
  async approved(device) {
    if (this.isApproved) return true;
    this.isApproved = !!(await this.relay().allowed(device.key));
    return this.isApproved;
  }

  /// Its endpoint, started with its key once in the object's life, bound at the relay: → the nodes it asks.
  async start(me) {
    if (!this.started) {
      const send = (dst, seg, contents) => {
        this.out.push([hex.encode(dst), seg, contents]);
        if (this.out.length > 1) return;
        queueMicrotask(() => {
          const items = this.out;
          this.out = [];
          this.sent = this.relay().fromClient(me.key, items).catch(() => {});
          this.ctx.waitUntil(this.sent);
        });
      };
      this.started = (async () => {
        const began = Date.now();
        const name = await this.ctx.storage.get("name");
        const bound = await this.relay().bind(await signedRequest(me.seed, hex.decode(me.key), RELAY, { bind: true }), name);
        if (bound) throw new Error(`the relay didn't take its key: ${bound}`);
        this.me = me;
        await device_start(me.seed, await this.nodes(me), send);
        console.log(JSON.stringify({ started: me.key.slice(0, 8), ms: Date.now() - began }));
      })();
      this.started.catch(() => (this.started = this.me = null));
    }
    await this.started;
    return this.order;
  }

  /// The nodes to ask, in Where Things Run's order as it last read it, of those on the relay now.
  async nodes(me) {
    const online = (await this.relay().computes()).filter((k) => k !== me.key);
    const kept = (await this.ctx.storage.get("order")) || [];
    this.order = [...kept.filter((k) => online.includes(k)), ...online.filter((k) => !kept.includes(k))];
    return this.order;
  }

  /// Reads Where Things Run's order from the install's state, for the next time it starts.
  async learnOrder(me) {
    if (this.learned) return;
    this.learned = true;
    try {
      const r = await device_fetch(me.key, "GET", "/v1/state", [], new Uint8Array());
      if (r.status !== 200) return;
      const state = JSON.parse(new TextDecoder().decode(r.body));
      const order = (state.install?.devices || []).filter((d) => d.key && d.order !== undefined).sort((a, b) => a.order - b.order).map((d) => d.key);
      await this.ctx.storage.put("order", order);
      const online = new Set(this.order);
      this.order = [...order.filter((k) => online.has(k)), ...this.order.filter((k) => !order.includes(k))];
      device_nodes(me.key, this.order);
    } catch {}
  }

  /// Asks a node's `/mcp` as this device, handing the keys it knows its calls need; a 428 says what else they need,
  /// which it opens and hands, asking once more.
  async forward(me, headers, body) {
    const bytes = new TextEncoder().encode(body);
    let handing = await this.handoff(me, this.order[0]).catch(() => null);
    for (let tries = 0; ; tries++) {
      const said = Object.entries(handing ? { ...headers, "x-one-keys": handing } : headers).flat();
      let r;
      try {
        r = await device_fetch(me.key, "POST", "/mcp", said, bytes);
      } catch (err) {
        if (err.sent) return answerAll(body, `Your device stopped answering: ${err.message}`);
        return answerAll(body, "None of your devices is on right now.");
      }
      this.ctx.waitUntil(this.learnOrder(me));
      // Keys sealed to a node other than the one that answered: it says what it needs, sealed to it.
      if (r.status === 400 && handing && tries === 0) {
        handing = null;
        continue;
      }
      if (r.status === 428 && tries < 2) {
        const need = JSON.parse(new TextDecoder().decode(r.body));
        const seals = (await this.ctx.storage.get("seals")) || {};
        seals[need.to] = need.seal;
        await this.ctx.storage.put("seals", seals);
        const needs = (await this.ctx.storage.get("needs")) || [];
        for (const n of need.need || []) if (!needs.some((m) => m.scope === n.scope && m.app === n.app && m.database === n.database)) needs.push({ scope: n.scope, app: n.app, database: n.database });
        await this.ctx.storage.put("needs", needs);
        this.handed = null;
        handing = await this.handoff(me, need.to, need.need).catch(() => null);
        if (handing) continue;
      }
      if (r.status === 428) return answerAll(body, JSON.parse(new TextDecoder().decode(r.body)).error || "That needs keys this device wasn't given.");
      const out = {};
      for (let i = 0; i < r.headers.length; i += 2) if (FROM_NODE.includes(r.headers[i])) out[r.headers[i]] = r.headers[i + 1];
      if (handing) this.handed = { node: this.order[0], until: me.until };
      return { status: r.status, headers: out, body: r.body };
    }
  }

  /// What it hands `node` with a call: the keys of every database its calls have needed, and of `need`, each epoch it
  /// has, sealed to that node, for its key, until its access token expires. Null if it handed them already, or knows
  /// nothing to hand.
  async handoff(me, node, need = []) {
    if (!node || (this.handed?.node === node && this.handed.until === me.until && !need.length)) return null;
    const seal = ((await this.ctx.storage.get("seals")) || {})[node];
    const needs = (await this.ctx.storage.get("needs")) || [];
    if (!seal || !needs.length) return null;
    const keys = [];
    for (const n of needs) {
      const epochs = await this.epochs(me, n.scope, need.filter((m) => m.scope === n.scope).map((m) => m.epoch));
      for (const [epoch, epochKey] of epochs) {
        const app = await this.appKey(me, n.scope, n.app, epoch, epochKey).catch(() => null);
        if (app) keys.push({ scope: n.scope, epoch, app: n.app, database: n.database, key: b64u.encode(await databaseKey(app, n.database)) });
      }
    }
    if (!keys.length) return null;
    return sealHandoff(hex.decode(seal), { for: me.key, until: me.until, keys });
  }

  /// The epochs of `scope` sealed to this device, opened: [[n, key]], fetched again if one of `want` isn't among them.
  async epochs(me, scope, want = []) {
    const all = (await this.ctx.storage.get("epochs")) || {};
    if (!all[scope] || want.some((n) => !all[scope][n])) {
      const r = await device_fetch(me.key, "GET", `/v1/keys?scope=${encodeURIComponent(scope)}&to=${me.key}`, [], new Uint8Array());
      if (r.status === 200) all[scope] = JSON.parse(new TextDecoder().decode(r.body)).sealed || {};
      await this.ctx.storage.put("epochs", all);
    }
    const out = [];
    for (const [n, sealed] of Object.entries(all[scope] || {})) {
      try {
        out.push([Number(n), await openSealed(me.seal, b64u.decode(sealed))]);
      } catch {}
    }
    return out;
  }

  /// An app's key in a scope's epoch: from what nodes keep sealed under that epoch, made by the first to need it.
  async appKey(me, scope, app, n, epochKey) {
    const all = (await this.ctx.storage.get("apps")) || {};
    const id = `${scope}\n${app}`;
    let sealed = all[id]?.[n];
    if (!sealed) {
      const r = await device_fetch(me.key, "GET", `/v1/keys?scope=${encodeURIComponent(scope)}&app=${encodeURIComponent(app)}`, [], new Uint8Array());
      all[id] = r.status === 200 ? JSON.parse(new TextDecoder().decode(r.body)).sealed || {} : {};
      sealed = all[id][n];
      if (!sealed) {
        const made = { scope, n, app, sealed: b64u.encode(await sealAppKey(epochKey, app, random(32))) };
        const p = await device_fetch(me.key, "POST", "/v1/keys", ["content-type", "application/json"], new TextEncoder().encode(JSON.stringify(made)));
        if (p.status !== 200) throw new Error(`the node kept no key of ${app}`);
        sealed = all[id][n] = JSON.parse(new TextDecoder().decode(p.body)).sealed;
      }
      await this.ctx.storage.put("apps", all);
    }
    return openAppKey(epochKey, app, b64u.decode(sealed));
  }

  /// Datagrams the relay was handed for its key: `[src, segment size, contents]` each, dropped if its endpoint isn't
  /// running (the WebAssembly keeps no other).
  async deliver(key, batch) {
    for (const [src, seg, contents] of batch) device_deliver(hex.decode(key), hex.decode(src), seg, contents);
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
    if (!this.started) return;
    await this.end();
    this.ctx.abort("idle");
  }

  /// Closes its endpoint and unbinds its key: its key and the keys it opened leave memory with the object.
  async end() {
    const me = this.me;
    this.me = this.started = null;
    this.isApproved = false;
    if (!me) return;
    await device_close(me.key);
    await this.sent;
    await this.relay().unbind(me.key).catch(() => {});
    console.log(JSON.stringify({ sleeps: me.key.slice(0, 8) }));
  }
}
