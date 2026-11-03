// The relay: how the install's endpoints reach each other when they can't directly, speaking iroh's relay protocol at
// the install's address (`/relay`, `/ping`, `/generate_204`, which the gateway sends here). One Durable Object per
// install keeps every endpoint's WebSocket, with Hibernation, so it sleeps between packets; iroh-relay's own code
// (relay_protos_bg.wasm, built from protos/ by build.sh) checks each endpoint's handshake and reads and writes its
// frames. It's blind: what endpoints send each other is QUIC, sealed end to end, so it can only drop it.
//
// It admits only the install's endpoints, on its one list of them: `{ by, admin }` per key, `by` the key that let it
// in. The install's first devices, its admins', are put in when the install is made (`DEVICES`, from the kernel's
// deploy, taken only while the list is empty); after that, devices tell it, and nothing else changes it. A computer
// joining with a pairing code a device made gets in too, by the ticket it shows as iroh's relay auth token, until it's
// added. The URL signer asks it, by `allowed(key)`.
//
// What a device tells it is `POST /relay/tell`, its body a request signed by an allowed key (`signedRequest` in
// shared/crypto.js, domain `one/relay`, each `(by, at)` taken once) with one of: `{ add, admin? }`, a key (hex) to let
// in, an admin only by an admin; `{ remove }`, a key to take out, by an admin, the key that let it in, or itself;
// `{ pairing: { hash, until } }`, the SHA-256 (hex) of a pairing code's ticket and until when it works (ms).
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import wasm from "./relay_protos_bg.wasm";
import { initSync, challenge, verify, confirm, deny, from_client, endpoint_gone } from "./relay_protos.js";
import { checkRequest, FRESH } from "../shared/crypto.js";

initSync({ module: wasm });

/// The WebSocket subprotocol iroh's endpoints ask for: version 2 of its relay protocol.
const PROTOCOL = "iroh-relay-v2";
/// What a tell is signed for.
const TELL = "one/relay";
/// How long a socket may take to answer its challenge, how many sockets that haven't one address may hold, and how
/// long a pairing code may work.
const PROVE_WITHIN = 30_000;
const UNPROVEN_PER_ADDRESS = 8;
const PAIRING_FOR = 7 * 86_400_000;
/// How often a sender is told that an endpoint it sends to isn't here.
const GONE_EVERY = 5_000;

const hex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");
const unhex = (s) => Uint8Array.from(s.match(/../g) || [], (h) => parseInt(h, 16));
const isKey = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
const sha256 = async (text) => hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
const refuse = (status, error) => Response.json({ error }, { status });
const CORS = { "Access-Control-Allow-Origin": "*" };

/// The key an endpoint says it is in iroh's `x-iroh-relay-client-auth-v1` header (base64url of postcard, the key's 32
/// bytes first), unchecked: its signature is over TLS that Cloudflare ends, so it only lets a stranger be refused
/// before it's handed a socket. The challenge still proves the key.
function claimed(request) {
  const header = request.headers.get("x-iroh-relay-client-auth-v1");
  if (!header) return null;
  try {
    const bytes = Uint8Array.from(atob(header.replace(/-/g, "+").replace(/_/g, "/")), (c) => c.charCodeAt(0));
    return bytes.length >= 32 ? hex(bytes.slice(0, 32)) : null;
  } catch {
    return null;
  }
}

/// iroh's relay auth token: `Authorization: Bearer <token>` from a native endpoint, `?token=` from a browser's.
function token(request, url) {
  const auth = request.headers.get("authorization") || "";
  const bearer = /^bearer (.+)$/i.exec(auth);
  return bearer ? bearer[1].trim() : url.searchParams.get("token");
}

export default class extends WorkerEntrypoint {
  async fetch(request) {
    const url = new URL(request.url);
    // iroh's latency probe; a browser's endpoint needs CORS to read it.
    if (url.pathname === "/ping") return new Response(null, { status: 200, headers: CORS });
    // iroh's captive portal check, answered as iroh-relay's own server does.
    if (url.pathname === "/generate_204") {
      const headers = { ...CORS };
      const challenge = request.headers.get("x-iroh-challenge");
      if (challenge && /^[A-Za-z0-9._-]{1,63}$/.test(challenge)) headers["X-Iroh-Response"] = `response ${challenge}`;
      return new Response(null, { status: 204, headers });
    }
    if (url.pathname === "/relay" || url.pathname === "/relay/tell") return this.relay().fetch(request);
    return refuse(404, "not the relay's");
  }

  /// Whether `key` (hex) is one of the install's endpoints: `{ admin }` if it is, for the URL signer, by its binding.
  async allowed(key) {
    return isKey(key) ? this.relay().allowed(key) : null;
  }

  /// The install's one relay, made near its people (`LOCATION`, a location hint) rather than wherever its first request
  /// came from.
  relay() {
    return this.env.RELAY.get(this.env.RELAY.idFromName("relay"), this.env.LOCATION ? { locationHint: this.env.LOCATION } : {});
  }
}

export class Relay extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    /// Each proven endpoint's socket, by its key; kept in the sockets' attachments across hibernation.
    this.sockets = new Map();
    for (const ws of ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said?.id) this.sockets.set(said.id, ws);
    }
    /// When each sender was last told an endpoint isn't here, `<from> <to>` → ms; lost on hibernation, which is fine.
    this.told = new Map();
  }

  /// The install's endpoints (`keys`: key → `{ by, admin }`), pairing codes (`pairings`: the ticket's hash →
  /// `{ until, by, key }`, `key` the computer it let in, once one has), and the tells taken (`tells`: `<by> <at>` → at).
  async list() {
    let list = await this.ctx.storage.get("list");
    if (!list) {
      list = { keys: {}, pairings: {}, tells: {} };
      for (const key of this.env.DEVICES ? JSON.parse(this.env.DEVICES) : []) list.keys[key] = { by: "install", admin: true };
      if (Object.keys(list.keys).length) await this.ctx.storage.put("list", list);
    }
    return list;
  }

  /// `{ admin }` if `key` is one of the install's endpoints; a computer let in by a pairing code isn't, until it's added.
  async allowed(key, list) {
    list ||= await this.list();
    const it = list.keys[key];
    return it ? { admin: it.admin } : null;
  }

  async fetch(request) {
    const url = new URL(request.url);
    if (url.pathname === "/relay/tell") return this.tell(request);
    if (request.headers.get("upgrade") !== "websocket") return refuse(426, "the relay takes an endpoint's WebSocket");
    const protocols = (request.headers.get("sec-websocket-protocol") || "").split(",").map((p) => p.trim());
    if (!protocols.includes(PROTOCOL)) return refuse(400, `the relay speaks ${PROTOCOL}`);
    const list = await this.list();
    const now = Date.now();
    // A pairing code's ticket, live and not taken by another computer.
    const ticket = token(request, url);
    let joining = null;
    if (ticket) {
      const hash = await sha256(ticket);
      const pairing = list.pairings[hash];
      if (!pairing || pairing.until < now) return refuse(403, "that pairing code has expired");
      joining = hash;
    }
    const key = claimed(request);
    if (key && !(await this.allowed(key, list)) && !(joining && [undefined, key].includes(list.pairings[joining].key))) {
      return refuse(403, "the relay takes only your One App's devices");
    }
    const address = request.headers.get("cf-connecting-ip") || "";
    let unproven = 0;
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said.id) continue;
      if (now - said.at > PROVE_WITHIN) ws.close(1008, "it never answered its challenge");
      else if (said.address === address) unproven++;
    }
    if (unproven >= UNPROVEN_PER_ADDRESS) return refuse(429, "too many sockets from there that haven't proved their key");
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    const made = challenge();
    server.serializeAttachment({ challenge: hex(made.slice(0, 16)), at: now, address, joining });
    server.send(made.slice(16));
    return new Response(null, { status: 101, webSocket: client, headers: { "Sec-WebSocket-Protocol": PROTOCOL } });
  }

  async webSocketMessage(ws, message) {
    if (typeof message === "string") return ws.close(1003, "binary only");
    const bytes = new Uint8Array(message);
    const said = ws.deserializeAttachment();
    if (!said.id) return this.prove(ws, said, bytes);
    let out;
    try {
      out = from_client(unhex(said.id), bytes);
    } catch (e) {
      return ws.close(1002, String(e.message || e).slice(0, 100));
    }
    if (out[0] === 1) return ws.send(out.subarray(1));
    if (out[0] !== 0) return;
    const to = hex(out.subarray(1, 33));
    const other = this.sockets.get(to);
    if (other) {
      try {
        return other.send(out.subarray(33));
      } catch {}
    }
    // Its sender drops its path through here sooner.
    const pair = `${said.id} ${to}`;
    const now = Date.now();
    if (now - (this.told.get(pair) || 0) < GONE_EVERY) return;
    this.told.set(pair, now);
    ws.send(endpoint_gone(unhex(to)));
  }

  /// A socket's answer to its challenge: it's let in if it proves an allowed key, or the key its pairing code is for.
  async prove(ws, said, bytes) {
    let id;
    try {
      id = hex(verify(unhex(said.challenge), bytes));
    } catch {
      ws.send(deny("that isn't the key's signature"));
      return ws.close(1008, "auth");
    }
    const list = await this.list();
    if (!(await this.allowed(id, list))) {
      const pairing = said.joining && list.pairings[said.joining];
      if (!pairing || pairing.until < Date.now() || (pairing.key && pairing.key !== id)) {
        ws.send(deny("the relay takes only your One App's devices"));
        return ws.close(1008, "not allowed");
      }
      // A pairing code lets in one computer: this one, again if it reconnects, until it expires or it's admitted.
      if (!pairing.key) {
        pairing.key = id;
        await this.ctx.storage.put("list", list);
      }
    }
    // The newest socket of a key is the one it's reached at.
    const old = this.sockets.get(id);
    if (old && old !== ws) {
      try {
        old.close(1000, "the endpoint connected again");
      } catch {}
    }
    ws.serializeAttachment({ id });
    this.sockets.set(id, ws);
    ws.send(confirm());
  }

  /// What a device of the install tells the relay: a key to let in or take out, or a pairing code it made.
  async tell(request) {
    let told;
    try {
      if (request.method !== "POST") throw new Error("a tell is a POST");
      ({ fields: told } = await checkRequest(TELL, await request.json()));
    } catch (e) {
      return refuse(403, String(e.message || e));
    }
    const now = Date.now();
    const list = await this.list();
    const by = list.keys[told.by];
    if (!by) return refuse(403, "only your One App's devices tell the relay");
    const taken = `${told.by} ${told.at}`;
    for (const [t, at] of Object.entries(list.tells)) if (Math.abs(now - at) > 2 * FRESH) delete list.tells[t];
    if (taken in list.tells) return refuse(403, "that tell was taken already");
    list.tells[taken] = told.at;
    if (told.add !== undefined) {
      if (!isKey(told.add)) return refuse(403, "add is a key, in hex");
      if (told.admin && !by.admin) return refuse(403, "only an admin's device adds an admin's");
      list.keys[told.add] = { by: told.by, admin: told.admin === true };
      for (const [hash, p] of Object.entries(list.pairings)) if (p.key === told.add) delete list.pairings[hash];
    } else if (told.remove !== undefined) {
      const it = list.keys[told.remove];
      if (!it) return refuse(403, "that key isn't on the list");
      if (!by.admin && it.by !== told.by && told.remove !== told.by) return refuse(403, "only an admin's device, the one that let it in, or itself takes a key out");
      delete list.keys[told.remove];
      try {
        this.sockets.get(told.remove)?.close(1008, "taken out");
      } catch {}
    } else if (told.pairing !== undefined) {
      const { hash, until } = told.pairing || {};
      if (!isKey(hash) || typeof until !== "number" || until <= now) return refuse(403, "a pairing is its ticket's hash, in hex, and until when it works");
      for (const [h, p] of Object.entries(list.pairings)) if (p.until < now) delete list.pairings[h];
      list.pairings[hash] = { until: Math.min(until, now + PAIRING_FOR), by: told.by };
    } else {
      return refuse(403, "a tell adds or removes a key, or makes a pairing code");
    }
    await this.ctx.storage.put("list", list);
    return new Response(null, { status: 204 });
  }

  webSocketClose(ws, code) {
    this.gone(ws);
    try {
      ws.close(code === 1005 ? 1000 : code, "closed");
    } catch {}
  }

  webSocketError(ws) {
    this.gone(ws);
  }

  gone(ws) {
    const said = ws.deserializeAttachment();
    if (said?.id && this.sockets.get(said.id) === ws) this.sockets.delete(said.id);
  }
}
