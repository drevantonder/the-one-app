// The relay: how the network's endpoints reach each other when they can't directly, speaking iroh's relay protocol at
// the network's address (`/relay`, `/ping`, `/generate_204`, which the gateway sends here). One Durable Object per
// network (`Relay`) keeps every endpoint's WebSocket, with Hibernation, so it sleeps between packets; another (`List`)
// keeps the list of the network's endpoints, so a Relay busy with packets, or stalled, holds up no one asking who's on
// it (sign-in, URLs, pushes, telemetry): the Relay asks the List once per socket it takes in, and the List tells the
// Relay which sockets to end when a key is taken out or changed. iroh-relay's own code
// (relay_protos_bg.wasm, built from protos/ by build.sh) checks each endpoint's handshake and reads and writes its
// frames. It's blind: what endpoints send each other is QUIC, sealed end to end, so it can only drop it.
//
// It admits only the network's endpoints, on its one list of them: `{ by, admin, role }` per key, `by` the key that let
// it in, `role` what it is (`client`, `compute` or `service`, or a machine's several, with spaces). The network's first
// devices, and its recovery key as a client, are put in when the network is made (`DEVICES`, `{ key, role, admin }`
// each, from the Workers' deploy, taken only while the list is empty, which it never is again: the last admin can't
// be taken out); after that,
// devices tell it, and nothing else changes it. A computer joining with a pairing code a device made gets in too, by
// the ticket it shows as iroh's relay auth token, until the code expires, and reaches only that device until it's
// added. A session's endpoint gets in too (a browser's: its device key can't be read out of WebCrypto, and iroh signs
// with raw bytes), by the certificate it shows as its auth token (keys::certify), signed by a client's key on the list,
// naming the key it proves, until then: it reaches every endpoint as the key that signed it would, until its
// certificate's `until` or that key is taken out or made something else. With the certificate, `.`, and a request that
// key signed just now (`proveSocket`: within ten seconds, its nonce taken once), it's taken in at once, with no
// challenge's round trip; without one, or with one that's stale or taken, it answers the challenge as any key does. The
// URL signer, auth and the push Worker ask it, by `allowed(key)`: `{ admin, role }`, or null. The telemetry Worker asks
// it `stats()` every hour: its sockets, and what it refused and ended since it was last asked.
//
// A browser's WebSocket can't set a header, so its certificate is `?token=` in the relay's URL, where logs may keep
// it. iroh shows the same token at every reconnect, so its proof is used once, by the socket the web client opens
// itself (net-worker.js); a certificate alone gets nothing: the relay lets in only the endpoint that proves the key it
// names, and answers computes only to a request that key signed.
//
// `POST /relay/computes` answers a request signed by a key on the list, or by a session's key with its certificate
// (`one/relay`, `{ computes: true, certificate? }`, each `(by, at)` taken once, as a tell is), with the compute
// endpoints' keys, the Cloudflare node's among them, in no order: a new client's first nodes to ask; `online`, those
// of them with a socket open here now, which a client races first and every node shows as online; `seen`, when each
// key on the list was last seen (ms), by its socket opening or closing or a Worker asking `allowed` of it; and
// `versions`, each key's that said one, for every client to show; and `workers`, once a deploy has said it, what the
// Workers are on (`{ from, workers, web, node }`: the device they were last deployed from, which an ask to deploy them
// again is left for, and each part's version in a version's words). 403 is only for a key that isn't let in; a request
// that's wrong, or signed by a clock that's off, is 400, so a client keeps the nodes it knows. A device's own request
// that says it reads less of who's who than this build (`reads`, protocol.js's `READS`; an older build's says
// nothing) is 426, telling it to update. A browser's session runs the code the address serves, the newest, so it isn't
// asked. A device's own request can carry `version` (`<commit> <committed> <target>`), kept here, and it's answered
// with `asked`, what was left for it (`update` below), taken once. Both happen before the `reads` check, so a device
// too old to be told anything else still hears. That body, and the signed ask, are frozen, as the signed record is
// (`tests/contracts.rs`): a device left behind has to be able to read them.
//
// What a device tells it is `POST /relay/tell`, its body a request signed by a key on the list (`signedRequest` in
// shared/crypto.js, domain `one/relay`, each `(by, at)` taken once) with one of:
// - `{ add, role, admin?, recovery? }`: a key (hex) to let in, and what it is, by any key on the list (who's who, which
//   every reader checks, is the real gate on what a compute or a service may do), and an admin only by an admin's or the
//   recovery key (`by` is "recovery"; a new one is named so by an admin's, `recovery`); one already in is changed only by
//   those, keeping who let it in. A device with an app runtime of its own drops compute from itself the same way, naming
//   itself with the role client and `admin` as it is, and stays what it was otherwise.
// - `{ remove }`: a key to take out, by an admin, the key that let it in, or itself, with the pairing codes it made; never
//   the last admin, and never a key it let in, which only a tell naming that key takes out. A pairing code's computer is
//   taken out the same way, by its code's maker too.
// - `{ admins: { <key>: bool } }`: whether each key is an admin's now, by an admin's device, as who's who says (`Node::tell_roles`).
// - `{ pairing: { hash, until } }`: the SHA-256 (hex) of a pairing code's ticket and until when it works (ms).
// - `{ unpair }`: a pairing code's hash, to end it, by its maker or an admin.
// - `{ update }`: a key on the list, whose device this signed ask is left for, whole, until it next asks for the computes:
//   any key on the list may leave one, the receiver checks who signed it and that they may remove it, and it can only
//   ever bring the signed record's newest build. Another replaces it.
// - `{ moved: [scope] }`: that the device just wrote to each scope (a scope's name, as `global` or `user:<id>`), by any key
//   on the list, as it tells the devices that hold them: each scope's count goes up by one. `computes` answers with
//   `moved`, each scope's count, for a device to read from the bucket only what moved since it last looked. It's only
//   a hint: the bucket is the truth, and a device reads all of it now and then anyway. The Cloudflare node, which tells
//   no device, counts its own by its binding (`CloudflareNode.moved`).
// - `{ cloudflare: true }`: the same, to deploy the Workers again, left for the device they were last deployed from (`workers.from`),
//   which checks that the asker is an admin.
//
// One endpoint isn't a socket: the Cloudflare node (`BOUND`, its key), a Durable Object of its own Worker (`NODE`) that
// sleeps until someone calls it. Datagrams sent to it go to it by RPC, gathered each turn, waking it; what it sends
// comes back by RPC (`fromNode`, on the `CloudflareNode` entrypoint, which only its Worker binds) and goes out as from
// it. It's one of the network's endpoints, as compute.
//
// Nor are the remote MCP server's devices (cloudflare/mcp/), one per remote MCP client, each in a Durable Object of
// that Worker's (`MCP`) while it runs: one binds its key (`bind`, a request signed by that key, which must be on the list) to its
// object, which then gets the datagrams sent to that key by RPC (`deliver`), and passes back by RPC what it sends
// (`fromClient`), until it unbinds or its binding lapses. Binding gives it a ticket, which sending and unbinding must
// show, so one device's object sends only as itself. The remote MCP server binds the `McpDevices` entrypoint, never the
// relay's object, so it can't tell the relay anything a device didn't sign.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import wasm from "./relay_protos_bg.wasm";
import { initSync, challenge, verify, confirm, deny, from_client, endpoint_gone, datagrams_of, to_client } from "./relay_protos.js";
import { b64u, behind, checkCertificate, checkRequest, hex, isKey, rolesOf, sha256 } from "../shared/crypto.js";
import { CORS, answer, near, refusal } from "../shared/http.js";
import { CODES, FRESH, IROH_PATHS, PAIRING_LIFE, RELAY, ROLES, SOCKET, SOCKET_FRESH, updateRefusal } from "../shared/protocol.js";

initSync({ module: wasm });

/// The WebSocket subprotocol iroh's endpoints ask for: version 2 of its relay protocol.
const PROTOCOL = "iroh-relay-v2";
const TELL_BYTES = 4096;
const VERSION_BYTES = 128;
/// A scope's name (src/database/scopes.rs `Scope`): `global`, or a kind and an id.
const SCOPE = /^(global|[a-z]+:[0-9A-Za-z_-]{1,64})$/;
const MOST_MOVED = 16;
const PROVE_WITHIN = 30_000;
const UNPROVEN_PER_PLACE = 8;
const UNPROVEN_PER_SITE = 32;
const UNPROVEN = 256;
const PAIRINGS_PER_DEVICE = 8;
const is = (it, role) => !!it && rolesOf(it.role).includes(role);
const GONE_EVERY = 5_000;
const GONE_KEPT = 1024;
const FOR_NODE_MOST = 1024;
/// How often a Worker's `allowed(key)` writes that it saw `key`, at most.
const SEEN_EVERY = 300_000;
/// The tables the Relay kept the list in before the List did.
const HANDED = ["keys", "pairings", "tells", "asks", "seen", "objects"];

/// What the Relay counts for `stats`: sockets it refused (`denied` at proof, `crowded` with too many unproven, `forbidden`
/// a pairing code's packet to a device it doesn't reach, `malformed` frames) and ended (`unanswered` challenges,
/// `replaced` by the same key again, `ended` by the list or by time, `failed` with an error).
const STATS = ["denied", "crowded", "forbidden", "malformed", "unanswered", "replaced", "ended", "failed"];

const replayed = (what) => ({ error: `that ${what} was taken already`, code: CODES.replayed });

/// iroh's relay auth token: `Authorization: Bearer <token>` from a native endpoint, `?token=` from a browser's.
function token(request, url) {
  const auth = request.headers.get("authorization") || "";
  const bearer = /^bearer (.+)$/i.exec(auth);
  return bearer ? bearer[1].trim() : url.searchParams.get("token");
}

/// Where a socket comes from, for counting those that haven't proved their key: an IPv4 address, or an IPv6 /64; and
/// its site, an IPv6 /48 (an IPv4 address's is itself).
function place(request) {
  const address = request.headers.get("cf-connecting-ip") || "";
  if (!address.includes(":")) return { from: address, site: address };
  const [head, tail = ""] = address.split("::");
  const groups = head.split(":").filter(Boolean);
  const rest = tail.split(":").filter(Boolean);
  const full = [...groups, ...Array(Math.max(0, 8 - groups.length - rest.length)).fill("0"), ...rest].map((g) => parseInt(g, 16).toString(16));
  return { from: full.slice(0, 4).join(":") + "::/64", site: full.slice(0, 3).join(":") + "::/48" };
}

export default class extends WorkerEntrypoint {
  async fetch(request) {
    const url = new URL(request.url);
    // iroh's latency probe; a browser's endpoint needs CORS to read it.
    if (url.pathname === IROH_PATHS.ping) return new Response(null, { status: 200, headers: CORS });
    // iroh's captive portal check, answered as iroh-relay's own server does.
    if (url.pathname === IROH_PATHS.portal) {
      const headers = { ...CORS };
      const challenge = request.headers.get("x-iroh-challenge");
      if (challenge && /^[A-Za-z0-9._-]{1,63}$/.test(challenge)) headers["x-iroh-response"] = `response ${challenge}`;
      return new Response(null, { status: 204, headers });
    }
    const relay = IROH_PATHS.relay;
    if (url.pathname === `${relay}/tell`) return tell(this.env, request);
    if (url.pathname === `${relay}/computes`) return this.ask(request, "computes");
    if (url.pathname === `${relay}/seen`) return this.ask(request, "seen");
    if (url.pathname === relay) return this.relay().fetch(request);
    if (url.pathname === `${relay}/test` && this.env.TEST) return Response.json(await answer(this.relay().testing(await request.json())));
    return refusal(404, "not the relay's");
  }

  async ask(request, what) {
    let asked;
    try {
      if (request.method !== "POST") throw new Error(`ask for ${what} with a POST`);
      const body = await request.arrayBuffer();
      if (body.byteLength > TELL_BYTES) throw new Error("that's too big");
      const { by, fields } = await checkRequest(RELAY, JSON.parse(new TextDecoder().decode(body)));
      if (fields[what] !== true) throw new Error(`ask with { ${what}: true }, signed`);
      if (fields.version !== undefined && (typeof fields.version !== "string" || fields.version.length > VERSION_BYTES)) throw new Error("a version is short text");
      asked = { by, at: fields.at, reads: Number.isSafeInteger(fields.reads) ? fields.reads : 0, version: fields.version };
      if (fields.certificate !== undefined) {
        const session = await checkCertificate(fields.certificate);
        if (session.endpoint !== by) throw new Error("sign with the key your certificate names");
        asked.session = session.by;
      }
    } catch (e) {
      return refusal(400, String(e.message || e));
    }
    const said = await answer(this.list()[what](asked.by, asked));
    if (said?.update) return refusal(426, { error: updateRefusal(this.env.YOURS), code: CODES.update, ...(said.asked && { asked: said.asked }) });
    if (what === "computes" && said?.computes) return Response.json({ ...said, online: await answer(this.relay().connected(said.computes)) });
    if (what === "seen" && said && typeof said === "object") return Response.json({ seen: said, let_in: await answer(this.list().letIn()) });
    if (said?.code) return refusal(400, said);
    if (said && typeof said === "object") return Response.json({ [what]: said });
    return said === null ? refusal(403, `the relay takes only ${this.env.YOURS}'s devices`) : refusal(400, said);
  }

  async allowed(key) {
    return isKey(key) ? answer(this.list().allowed(key)) : null;
  }

  /// The relay's sockets now, and what it refused and ended since it was last asked: for the telemetry Worker, hourly.
  stats() {
    return answer(this.relay().stats());
  }

  /// The keys of the network's admins' devices, for auth to push to.
  admins(most) {
    return answer(this.list().adminKeys(most));
  }

  relay() {
    return relay(this.env);
  }

  list() {
    return list(this.env);
  }
}

/// A tell's signature is checked here first, where a forged one costs the network's one relay nothing; the relay checks
/// it again, and the rest.
async function tell(env, request) {
  try {
    if (request.method !== "POST") throw new Error("a tell is a POST");
    if (Number(request.headers.get("content-length") || 0) > TELL_BYTES) throw new Error("a tell is small");
    const body = await request.arrayBuffer();
    if (body.byteLength > TELL_BYTES) throw new Error("a tell is small");
    const signed = JSON.parse(new TextDecoder().decode(body));
    await checkRequest(RELAY, signed);
    const error = await answer(list(env).tell(signed));
    return error ? refusal(403, error) : new Response(null, { status: 204 });
  } catch (e) {
    return refusal(403, String(e.message || e));
  }
}

/// The network's one relay, and its one list, made near its users.
const relay = (env) => env.RELAY.get(env.RELAY.idFromName("relay"), near(env));
const list = (env) => env.LIST.get(env.LIST.idFromName("list"), near(env));

/// What the remote MCP server asks the relay, by its binding: its devices' own bindings, and who's here; and what any
/// device tells it, signed (`POST /relay/tell`, as a Worker can't reach its own zone's routes by fetch). Nothing else of
/// the relay's object, so a hacked remote MCP server tells it only what a device it holds the key of may, and sends as
/// no other endpoint.
export class McpDevices extends WorkerEntrypoint {
  fetch(request) {
    return new URL(request.url).pathname === `${IROH_PATHS.relay}/tell` ? tell(this.env, request) : refusal(404, "the remote MCP server's devices only tell the relay");
  }

  bind(request, name) {
    return answer(relay(this.env).bind(request, name));
  }

  unbind(key, ticket) {
    return answer(relay(this.env).unbind(key, ticket));
  }

  fromClient(key, ticket, items) {
    return answer(relay(this.env).fromClient(key, ticket, items));
  }

  allowed(key) {
    return isKey(key) ? answer(list(this.env).allowed(key)) : null;
  }

  connected(keys) {
    return Array.isArray(keys) ? answer(relay(this.env).connected(keys.filter(isKey))) : [];
  }

  /// Each scope's count of writes, as the computes carry it: for a device to read who's who again only once it moved.
  writes() {
    return answer(list(this.env).writes());
  }
}

export class CloudflareNode extends WorkerEntrypoint {
  fromNode(items) {
    return answer(relay(this.env).fromNode(items));
  }

  /// The Cloudflare node's writes, which it tells no device of: as a device's `{ moved }` tell.
  moved(scopes) {
    return answer(list(this.env).move(scopes));
  }

  connected(keys) {
    return Array.isArray(keys) ? answer(relay(this.env).connected(keys.filter(isKey))) : [];
  }
}

export class Relay extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`CREATE TABLE IF NOT EXISTS proofs (nonce TEXT PRIMARY KEY, at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS bound (key TEXT PRIMARY KEY, name TEXT NOT NULL, until INTEGER NOT NULL, ticket TEXT NOT NULL DEFAULT '');
      CREATE TABLE IF NOT EXISTS counted (what TEXT PRIMARY KEY, n INTEGER NOT NULL);`);
    // Rebuilt from the sockets' attachments, which Hibernation keeps.
    this.sockets = new Map();
    for (const ws of ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said?.id) this.sockets.set(said.id, ws);
    }
    this.bound = new Map();
    for (const b of this.sql.exec("SELECT key, name, until, ticket FROM bound WHERE until > ?", Date.now()).toArray()) this.bound.set(b.key, { name: b.name, until: b.until, ticket: b.ticket });
    // `<from> <to>` → ms; lost on hibernation, which is fine.
    this.told = new Map();
    /// Only a test binds `TEST` (tests/cloudflare.rs on the Workers it serves itself; a network's bindings come from its
    /// settings, which hold none): its clock, moved by `testing`, so what ends with time ends without waiting; and a
    /// log of what the relay refused and ended, which a test waits on instead of timing out on what never happens.
    this.skew = 0;
    this.log = [];
    this.waiting = [];
  }

  now() {
    return Date.now() + this.skew;
  }

  /// What the relay did to `key`: one of `STATS`, counted for `stats`, or `"left"` once its socket closed and the list
  /// saw it go.
  note(key, what) {
    if (STATS.includes(what)) this.sql.exec("INSERT INTO counted VALUES (?, 1) ON CONFLICT (what) DO UPDATE SET n = n + 1", what);
    if (!this.env.TEST) return;
    this.log.push([key, what]);
    const heard = this.waiting.filter((w) => w.key === key && w.what === what && this.count(key, what) > w.after);
    this.waiting = this.waiting.filter((w) => !heard.includes(w));
    for (const w of heard) w.resolve();
  }

  count(key, what) {
    return this.log.filter(([k, w]) => k === key && w === what).length;
  }

  /// A test's way in (`/relay/test`): `{ advance: ms }` moves the clock on and ends what it ended; `{ count: [key, what] }`
  /// is how often `note` said it; `{ wait: [key, what, after] }` answers once it has said it more than `after` times;
  /// `{ connected: [key] }` is which of those are online, with a socket here.
  async testing(told) {
    if (told.advance !== undefined) {
      this.skew += told.advance;
      await this.list().advance(told.advance);
      await this.alarm();
    }
    if (told.count) return this.count(...told.count);
    if (told.connected) return this.connected(told.connected);
    if (told.wait) {
      const [key, what, after] = told.wait;
      if (this.count(key, what) <= after) await new Promise((resolve) => this.waiting.push({ key, what, after, resolve }));
    }
    return null;
  }

  list() {
    return list(this.env);
  }

  async fetch(request) {
    const url = new URL(request.url);
    if (request.headers.get("upgrade") !== "websocket") return refusal(426, "the relay takes an endpoint's WebSocket");
    const protocols = (request.headers.get("sec-websocket-protocol") || "").split(",").map((p) => p.trim());
    if (!protocols.includes(PROTOCOL)) return refusal(400, `the relay speaks ${PROTOCOL}`);
    const now = Date.now();
    // A pairing code's ticket (64 hex), or a session's certificate, if it shows one: whether it lets this key in is
    // decided once the key is proved, here if the certificate comes with its endpoint's proof.
    const ticket = token(request, url);
    const joining = isKey(ticket) ? hex.encode(await sha256(ticket)) : null;
    if (joining && !(await answer(this.list().pairs(joining)))) return refusal(403, "that pairing code has expired");
    let session = null;
    let proved = false;
    if (ticket && !joining) {
      const [certificate, proof] = ticket.split(".");
      try {
        session = await checkCertificate(certificate, now);
      } catch (e) {
        return refusal(403, String(e.message || e));
      }
      proved = !!proof && (await this.proves(proof, session.endpoint, now));
    }
    // Taken in at once only for a certificate its signer made and its endpoint's own proof, so this tells only them
    // whether they're on the list.
    if (proved && (await answer(this.list().client(session.by)))) {
      const [client, server] = Object.values(new WebSocketPair());
      this.ctx.acceptWebSocket(server);
      await this.admit(server, session.endpoint, { id: session.endpoint, by: session.by, until: session.until });
      return new Response(null, { status: 101, webSocket: client, headers: { "Sec-WebSocket-Protocol": PROTOCOL } });
    }
    // Who's let in is said only after the challenge, the same for every key that isn't, so this tells no one who's on
    // the list; sockets that haven't answered are few, from one place and in all.
    const { from, site } = place(request);
    let unproven = 0;
    let here = 0;
    let near = 0;
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (!said?.challenge) continue;
      if (now - said.at > PROVE_WITHIN) {
        this.note(null, "unanswered");
        ws.serializeAttachment({});
        ws.close(1008, "it never answered its challenge");
        continue;
      }
      unproven++;
      if (said.from === from) here++;
      if (said.site === site) near++;
    }
    if (here >= UNPROVEN_PER_PLACE || near >= UNPROVEN_PER_SITE || unproven >= UNPROVEN) {
      this.note(null, "crowded");
      return refusal(429, "too many sockets that haven't proved their key");
    }
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    const made = challenge();
    server.serializeAttachment({ challenge: hex.encode(made.slice(0, 16)), at: now, from, site, joining, session });
    server.send(made.slice(16));
    return new Response(null, { status: 101, webSocket: client, headers: { "Sec-WebSocket-Protocol": PROTOCOL } });
  }

  async webSocketMessage(ws, message) {
    if (typeof message === "string") return ws.close(1003, "binary only");
    const bytes = new Uint8Array(message);
    const said = ws.deserializeAttachment();
    if (said?.challenge) return this.prove(ws, said, bytes);
    if (!said?.id) return;
    const now = this.now();
    // A pairing code's computer, until it's added: only while its code works, and only to the device that made it. A
    // session's endpoint, only until its certificate's `until`.
    if (said.pairing && now > said.until) return this.end(said.id, "its pairing code expired");
    if (said.by && now > said.until) return this.end(said.id, "its session ended");
    let out;
    try {
      out = from_client(hex.decode(said.id), bytes);
    } catch (e) {
      this.note(said.id, "malformed");
      return ws.close(1002, String(e.message || e).slice(0, 100));
    }
    if (out[0] === 1) return ws.send(out.subarray(1));
    if (out[0] !== 0) return;
    const to = hex.encode(out.subarray(1, 33));
    if (said.pairing && to !== said.to) return this.note(said.id, "forbidden");
    if (to === this.env.BOUND && this.env.NODE) return this.toNode(said.id, bytes);
    const client = this.boundTo(to);
    if (client) return this.toClient(to, client, said.id, bytes);
    const other = this.sockets.get(to);
    if (other) {
      try {
        return other.send(out.subarray(33));
      } catch {} // a socket that closed since it was listed: the packet is lost, as the network may lose it
    }
    // Its sender drops its path through here sooner.
    const pair = `${said.id} ${to}`;
    if (now - (this.told.get(pair) || 0) < GONE_EVERY) return;
    if (this.told.size >= GONE_KEPT) {
      for (const [p, at] of this.told) if (now - at >= GONE_EVERY) this.told.delete(p);
      if (this.told.size >= GONE_KEPT) this.told.clear();
    }
    this.told.set(pair, now);
    ws.send(endpoint_gone(hex.decode(to)));
  }

  toNode(src, frame) {
    let d;
    try {
      d = datagrams_of(frame);
    } catch {
      return;
    }
    this.forNode ||= [];
    // As a full socket does, it drops what comes faster than it's handed on.
    if (this.forNode.length >= FOR_NODE_MOST) return;
    this.forNode.push([src, (d[0] << 8) | d[1], d.slice(2)]);
    if (this.forNode.length > 1) return;
    queueMicrotask(() => {
      const batch = this.forNode;
      this.forNode = [];
      const node = this.env.NODE.get(this.env.NODE.idFromName("node"), near(this.env));
      this.ctx.waitUntil(node.deliver(batch).catch((err) => console.log(JSON.stringify({ node: "unreached", why: String(err?.message ?? err) }))));
    });
  }

  /// Datagrams the Cloudflare node sends, `[dst, segment size, contents]` each, passed on as from it: → the keys of
  /// those not here, so it stops waiting for them.
  fromNode(items) {
    if (!this.env.BOUND || !Array.isArray(items)) return [];
    const gone = new Set();
    for (const [dst, seg, contents] of items) {
      const to = this.sockets.get(dst);
      if (!to) gone.add(dst);
      else
        try {
          to.send(to_client(hex.decode(this.env.BOUND), seg, contents));
        } catch {} // closed since it was listed: this packet is lost
    }
    return [...gone];
  }

  /// Which of `keys` have a socket open here: the network's one answer to who's online.
  connected(keys) {
    return keys.filter((k) => this.sockets.has(k));
  }

  /// The sockets open now, by what they are (`unproven`, `session`, `pairing`, or each role the list gives a key), and
  /// how often each of `STATS` happened since the last time this was asked.
  async stats() {
    const open = { unproven: 0, session: 0, pairing: 0 };
    const keys = [];
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said?.challenge) open.unproven++;
      else if (said?.by) open.session++;
      else if (said?.pairing) open.pairing++;
      else if (said?.id) keys.push(said.id);
    }
    for (const role of await answer(this.list().roles(keys))) open[role] = (open[role] || 0) + 1;
    const counted = Object.fromEntries(STATS.map((what) => [what, 0]));
    for (const { what, n } of this.sql.exec("DELETE FROM counted RETURNING what, n").toArray()) if (what in counted) counted[what] = n;
    return { open, counted };
  }

  boundTo(key) {
    const b = this.env.MCP && this.bound.get(key);
    if (!b) return null;
    if (b.until > Date.now()) return b.name;
    this.unbind(key);
    return null;
  }

  /// Binds a key to the remote MCP server's object `name` for an hour: `request` signed by that key (`{ bind: true }`, as a
  /// tell is), which is on the list. → `{ ticket, until }`, what sending as it and unbinding it show, and when it lapses
  /// (ms), or `{ error }`; bound again, it's given a new ticket.
  async bind(request, name) {
    let fields;
    try {
      ({ fields } = await checkRequest(RELAY, request));
    } catch (e) {
      return { error: String(e.message || e) };
    }
    if (fields.bind !== true || typeof name !== "string" || !name || name.length > 256) return { error: "a binding is a key's signed bind, to an object's name" };
    const refused = await answer(this.list().binds(fields.by, fields.at, name));
    if (refused) return refused;
    if (this.sockets.has(fields.by)) return { error: "that key is connected here itself" };
    const until = Date.now() + 3_600_000;
    const ticket = hex.encode(crypto.getRandomValues(new Uint8Array(32)));
    this.sql.exec("INSERT OR REPLACE INTO bound VALUES (?, ?, ?, ?)", fields.by, name, until, ticket);
    this.bound.set(fields.by, { name, until, ticket });
    return { ticket, until };
  }

  unbind(key, ticket) {
    if (ticket !== undefined && !this.ticketed(key, ticket)) return;
    this.bound.delete(key);
    this.sql.exec("DELETE FROM bound WHERE key = ?", key);
  }

  ticketed(key, ticket) {
    const b = this.bound.get(key);
    return !!b && typeof ticket === "string" && ticket.length === 64 && b.ticket === ticket;
  }

  toClient(to, name, src, frame) {
    let d;
    try {
      d = datagrams_of(frame);
    } catch {
      return;
    }
    const waiting = (this.forClients ||= new Map());
    if (!waiting.has(to)) {
      waiting.set(to, []);
      queueMicrotask(() => {
        const batch = waiting.get(to);
        waiting.delete(to);
        const object = this.env.MCP.get(this.env.MCP.idFromName(name), near(this.env));
        this.ctx.waitUntil(object.deliver(to, batch).catch((err) => console.log(JSON.stringify({ client: "unreached", why: String(err?.message ?? err) }))));
      });
    }
    waiting.get(to).push([src, (d[0] << 8) | d[1], d.slice(2)]);
  }

  fromClient(key, ticket, items) {
    if (!Array.isArray(items)) return [];
    if (!this.boundTo(key) || !this.ticketed(key, ticket)) return items.map(([dst]) => dst);
    const gone = new Set();
    for (const [dst, seg, contents] of items) {
      const to = this.sockets.get(dst);
      if (!to) gone.add(dst);
      else
        try {
          to.send(to_client(hex.decode(key), seg, contents));
        } catch {} // closed since it was listed: this packet is lost
    }
    return [...gone];
  }

  async prove(ws, said, bytes) {
    let id;
    try {
      id = hex.encode(verify(hex.decode(said.challenge), bytes));
    } catch {
      id = null;
    }
    // Who's let in is decided by the list, after the challenge, so this tells no one who's on it.
    const attachment = id ? await answer(this.list().admits(id, said)) : null;
    if (!attachment) {
      this.note(id, "denied");
      ws.serializeAttachment({});
      ws.send(deny(`the relay takes only ${this.env.YOURS}'s devices`));
      return ws.close(1008, "not allowed");
    }
    await this.admit(ws, id, attachment);
  }

  /// Whether `proof` is `endpoint`'s, signed for its socket within SOCKET_FRESH of now, its nonce never shown before:
  /// each nonce is kept until no proof of its time would be taken, so a token read from a log is only challenged.
  async proves(proof, endpoint, now) {
    try {
      const { by, fields } = await checkRequest(SOCKET, JSON.parse(new TextDecoder().decode(b64u.decode(proof))), now);
      if (by !== endpoint || Math.abs(fields.at - now) > SOCKET_FRESH || !/^[0-9a-f]{32}$/.test(fields.nonce)) return false;
      this.sql.exec("DELETE FROM proofs WHERE at < ?", now - 2 * SOCKET_FRESH);
      if (this.sql.exec("SELECT 1 FROM proofs WHERE nonce = ?", fields.nonce).toArray().length) return false;
      this.sql.exec("INSERT INTO proofs VALUES (?, ?)", fields.nonce, fields.at);
      return true;
    } catch {
      return false;
    }
  }

  async admit(ws, id, attachment) {
    if (attachment.until) await this.wake(attachment.until);
    for (const other of this.ctx.getWebSockets()) {
      if (other !== ws && other.deserializeAttachment()?.id === id) {
        this.note(id, "replaced");
        other.serializeAttachment({});
        try {
          other.close(1000, "the endpoint connected again");
        } catch {} // already closed
      }
    }
    ws.serializeAttachment(attachment);
    this.sockets.set(id, ws);
    ws.send(confirm());
    this.saw(attachment.by || id);
  }

  /// Tells the list it saw `key` now, without waiting on it.
  saw(key) {
    const told = this.list().saw(key).catch((err) => console.log(JSON.stringify({ seen: "unsaid", why: String(err?.message ?? err) })));
    this.ctx.waitUntil(told);
    return told;
  }

  end(key, why) {
    const ws = this.sockets.get(key);
    this.sockets.delete(key);
    if (!ws) return;
    this.note(key, "ended");
    ws.serializeAttachment({});
    try {
      ws.close(1008, why);
    } catch {} // already closed
  }

  /// An alarm at `until`, if none is sooner, to close pairing codes' computers once their codes expire, and sessions'
  /// endpoints once theirs end.
  async wake(until) {
    const at = await this.ctx.storage.getAlarm();
    if (!at || at > until) await this.ctx.storage.setAlarm(until);
  }

  async alarm() {
    const now = this.now();
    let next = null;
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (!said?.pairing && !said?.by) continue;
      if (said.until <= now) this.end(said.id, said.pairing ? "its pairing code expired" : "its session ended");
      else next = Math.min(next ?? said.until, said.until);
    }
    if (next) await this.ctx.storage.setAlarm(next);
  }

  /// What the list says of a key it added, changed or took out, for the sockets here.
  added(key) {
    const ws = this.sockets.get(key);
    if (ws?.deserializeAttachment()?.pairing) ws.serializeAttachment({ id: key });
  }

  ended(key, why) {
    this.end(key, why);
  }

  sessionsEnded(key) {
    this.endSessionsOf(key);
  }

  removed(key) {
    this.unbind(key);
    this.end(key, "taken out");
    this.endSessionsOf(key);
  }

  /// The list's rows from before it had an object of its own, once: → each table's rows, those it has.
  handOver() {
    const had = new Set(this.sql.exec("SELECT name FROM sqlite_master WHERE type = 'table'").toArray().map((r) => r.name));
    return Object.fromEntries(HANDED.filter((t) => had.has(t)).map((t) => [t, this.sql.exec(`SELECT * FROM ${t}`).toArray()]));
  }

  handedOver() {
    for (const t of HANDED) this.sql.exec(`DROP TABLE IF EXISTS ${t}`);
  }

  endSessionsOf(key) {
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said?.by === key) this.end(said.id, "taken out");
    }
  }

  webSocketClose(ws, code) {
    this.gone(ws);
    try {
      ws.close(code === 1005 ? 1000 : code, "closed");
    } catch {} // the peer already closed it
  }

  webSocketError(ws) {
    this.note(null, "failed");
    this.gone(ws);
  }

  gone(ws) {
    const said = ws.deserializeAttachment();
    if (!said?.id || this.sockets.get(said.id) !== ws) return;
    this.sockets.delete(said.id);
    this.saw(said.by || said.id).then(() => this.note(said.id, "left"));
  }
}

/// The network's one list of its endpoints, beside the Relay (see the top of this file).
export class List extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    this.sql.exec(`CREATE TABLE IF NOT EXISTS keys (key TEXT PRIMARY KEY, by TEXT NOT NULL, admin INTEGER NOT NULL, role TEXT NOT NULL, version TEXT);
      CREATE TABLE IF NOT EXISTS pairings (hash TEXT PRIMARY KEY, until INTEGER NOT NULL, by TEXT NOT NULL, key TEXT);
      CREATE TABLE IF NOT EXISTS tells (by TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (by, at));
      CREATE TABLE IF NOT EXISTS seen (key TEXT PRIMARY KEY, at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS asks (key TEXT PRIMARY KEY, signed TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS objects (key TEXT PRIMARY KEY, name TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS moved (scope TEXT PRIMARY KEY, n INTEGER NOT NULL);`);
    /// A test's clock, as the Relay's (`Relay.testing`).
    this.skew = 0;
    /// When `saw` last wrote each key, so a Worker asking `allowed` writes it at most every `SEEN_EVERY`; lost on eviction, which is fine.
    this.lastSaw = new Map();
    ctx.blockConcurrencyWhile(() => this.start());
  }

  /// The list as the Relay kept it before, taken over once; else, for a new network, its first devices (`DEVICES`, taken
  /// only while the list is empty, which it never is again). Then the recovery key, as each deploy says.
  async start() {
    const devices = this.env.DEVICES ? JSON.parse(this.env.DEVICES) : [];
    if (!this.sql.exec("SELECT 1 FROM keys LIMIT 1").toArray().length) {
      const had = await answer(this.relay().handOver());
      const columns = { keys: ["key", "by", "admin", "role", "version"], pairings: ["hash", "until", "by", "key"], tells: ["by", "at"], asks: ["key", "signed"], seen: ["key", "at"], objects: ["key", "name"] };
      for (const [table, rows] of Object.entries(had)) {
        const names = columns[table];
        for (const row of rows) this.sql.exec(`INSERT OR IGNORE INTO ${table} (${names.join(", ")}) VALUES (${names.map(() => "?").join(", ")})`, ...names.map((n) => row[n] ?? null));
      }
      if (had.keys?.length) await this.relay().handedOver();
      else {
        for (const { key, role, admin } of devices) {
          if (isKey(key) && rolesOf(role).every((r) => ROLES.includes(r))) this.sql.exec("INSERT OR IGNORE INTO keys (key, by, admin, role) VALUES (?, 'network', ?, ?)", key, admin === true ? 1 : 0, role);
        }
      }
    }
    for (const { key, recovery } of devices) {
      if (recovery === true && isKey(key)) this.sql.exec("UPDATE keys SET by = 'recovery' WHERE key = ?", key);
    }
  }

  now() {
    return Date.now() + this.skew;
  }

  advance(ms) {
    this.skew += ms;
  }

  relay() {
    return relay(this.env);
  }

  /// What the Relay does to its sockets for a change here, without waiting on it: a busy Relay holds up no tell.
  tellRelay(what) {
    this.ctx.waitUntil(Promise.resolve(what(this.relay())).catch((err) => console.log(JSON.stringify({ relay: "untold", why: String(err?.message ?? err) }))));
  }

  key(key) {
    return this.sql.exec("SELECT key, by, admin, role FROM keys WHERE key = ?", key).toArray()[0];
  }

  pairing(hash) {
    return this.sql.exec("SELECT hash, until, by, key FROM pairings WHERE hash = ?", hash).toArray()[0];
  }

  admins() {
    return this.sql.exec("SELECT count(*) AS n FROM keys WHERE admin = 1").one().n;
  }

  adminKeys(most) {
    return this.sql.exec("SELECT key FROM keys WHERE admin = 1 LIMIT ?", most).toArray().map((r) => r.key);
  }

  /// `{ admin, role }` if `key` is one of the network's endpoints; a computer let in by a pairing code isn't, until it's
  /// added.
  allowed(key) {
    if (key === this.env.BOUND) return { admin: false, role: "compute" };
    const it = this.key(key);
    if (!it) return null;
    if (!(this.now() - (this.lastSaw.get(key) ?? 0) < SEEN_EVERY)) this.saw(key);
    return { admin: it.admin === 1, role: it.role };
  }

  /// Each of `keys`' roles, for the Relay's `stats`: a machine's several count as each.
  roles(keys) {
    return keys.flatMap((k) => rolesOf(this.key(k)?.role ?? ""));
  }

  /// Whether `by` may be told what the network's endpoints are told: it's on the list, or is a session's a client on it
  /// vouched for (`session`); null if neither. A request's (`at`) is taken once: → why not, if it was taken already.
  asks(by, { at, session } = {}) {
    // Who's asking first, as `tell` does, so a stranger's requests are never kept.
    if (session !== undefined ? !is(this.key(session), "client") : !this.key(by)) return null;
    if (at !== undefined && !this.take(by, at)) return replayed("request");
    return true;
  }

  computes(by, asked) {
    const may = this.asks(by, asked);
    if (may !== true) return may;
    // A device's own request says what it's on and hears what's been left for it, whatever else it's told.
    const heard = asked && !asked.session && asked.version !== undefined ? this.heard(by, asked.version) : {};
    if (asked && !asked.session && behind(asked.reads)) return { update: true, ...heard };
    const keys = this.sql.exec("SELECT key FROM keys WHERE ' ' || role || ' ' LIKE '% compute %'").toArray().map((r) => r.key);
    const versions = Object.fromEntries(this.sql.exec("SELECT key, version FROM keys WHERE version IS NOT NULL").toArray().map((r) => [r.key, r.version]));
    const moved = this.writes();
    return { computes: this.env.BOUND ? [...keys, this.env.BOUND] : keys, seen: this.seenAll(), versions, moved, ...(this.env.DEPLOYED && { workers: JSON.parse(this.env.DEPLOYED) }), ...heard };
  }

  /// Keeps what `by` says it's on, and gives it what was left for it, once: → `{ asked }`, or nothing.
  heard(by, version) {
    this.sql.exec("UPDATE keys SET version = ? WHERE key = ? AND version IS NOT ?", version, by, version);
    const ask = this.sql.exec("DELETE FROM asks WHERE key = ? RETURNING signed", by).toArray()[0];
    return ask ? { asked: JSON.parse(ask.signed) } : {};
  }

  /// When each endpoint on the list was last let in here (ms), a session's as the device that vouched for it, for whom
  /// `asks` lets ask: → `{ key: at }`, else null or why not.
  seen(by, asked) {
    const may = this.asks(by, asked);
    if (may !== true) return may;
    return this.seenAll();
  }

  take(by, at) {
    const now = Date.now();
    this.sql.exec("DELETE FROM tells WHERE at < ? OR at > ?", now - 2 * FRESH, now + 2 * FRESH);
    if (this.sql.exec("SELECT 1 FROM tells WHERE by = ? AND at = ?", by, at).toArray().length) return false;
    this.sql.exec("INSERT INTO tells VALUES (?, ?)", by, at);
    return true;
  }

  /// Counts a write to each of `scopes`, as their writer says: → why not, if they aren't scopes' names.
  /// Each scope's count of writes (`move`).
  writes() {
    return Object.fromEntries(this.sql.exec("SELECT scope, n FROM moved").toArray().map((r) => [r.scope, r.n]));
  }

  move(scopes) {
    if (!Array.isArray(scopes) || !scopes.length || scopes.length > MOST_MOVED || !scopes.every((s) => typeof s === "string" && SCOPE.test(s))) return "moved is the names of the scopes written to";
    for (const scope of new Set(scopes)) this.sql.exec("INSERT INTO moved VALUES (?, 1) ON CONFLICT (scope) DO UPDATE SET n = n + 1", scope);
    return null;
  }

  /// Whether a pairing code's ticket hash still works.
  pairs(hash) {
    return this.pairing(hash)?.until > this.now();
  }

  /// Whether `key` is a client's on the list: who may vouch for a session's endpoint.
  client(key) {
    return is(this.key(key), "client");
  }

  /// What a socket that proved it's `id` is let in as, from what it showed when it connected (`said`: a session's
  /// certificate, or a pairing code's ticket hash): `{ id }` for a key on the list, a session's endpoint, a pairing code's
  /// computer, or null.
  admits(id, said) {
    if (this.key(id)) return { id };
    const now = this.now();
    if (said?.session?.endpoint === id && this.client(said.session.by) && said.session.until > now) return { id, by: said.session.by, until: said.session.until };
    const pairing = said?.joining && this.pairing(said.joining);
    if (pairing && pairing.until > now && [null, id].includes(pairing.key)) {
      this.sql.exec("UPDATE pairings SET key = ? WHERE hash = ?", id, pairing.hash);
      return { id, pairing: pairing.hash, until: pairing.until, to: pairing.by };
    }
    return null;
  }

  /// That a device on the list was seen now: its socket opened or closed, or a Worker asked `allowed` of it.
  saw(key) {
    const now = this.now();
    if (!this.key(key)) return;
    this.sql.exec("INSERT INTO seen VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET at = excluded.at", key, now);
    if (this.lastSaw.size >= GONE_KEPT) this.lastSaw.clear();
    this.lastSaw.set(key, now);
  }

  /// When each key on the list was last seen (ms).
  seenAll() {
    return Object.fromEntries(this.sql.exec("SELECT seen.key, seen.at FROM seen JOIN keys ON keys.key = seen.key").toArray().map((r) => [r.key, r.at]));
  }

  /// Whether `by` may bind its key to the remote MCP server's object `name`, its request's `at` taken once: → why not, or
  /// null, keeping which object holds it for when it's taken out.
  binds(by, at, name) {
    if (!this.take(by, at)) return replayed("binding");
    if (!this.key(by)) return { error: "that key isn't one of the network's: a device of its user approves it", code: CODES.notLetIn };
    this.sql.exec("INSERT OR REPLACE INTO objects VALUES (?, ?)", by, name);
    return null;
  }

  /// What a device told the relay, `signed` by it (`signedRequest`), checked here whoever passed it on: → why it was
  /// refused, or nothing.
  async tell(signed) {
    const now = Date.now();
    let told;
    try {
      ({ fields: told } = await checkRequest(RELAY, signed));
    } catch (e) {
      return String(e.message || e);
    }
    const by = this.key(told.by);
    if (!by) return `only ${this.env.YOURS}'s devices tell the relay`;
    if (!this.take(told.by, told.at)) return replayed("tell");
    const admin = by.admin === 1;
    if (told.moved !== undefined) return this.move(told.moved);
    if (told.add !== undefined) {
      if (!isKey(told.add)) return "add is a key, in hex";
      if (!ROLES.includes(told.role)) return `add says its role: ${ROLES.join(", ")}`;
      const wants = told.admin === true;
      const it = this.key(told.add);
      // The recovery key lets back in an admin with no device left, as who's who says; it counts for nothing else here.
      const trusted = admin || by.by === "recovery";
      if (wants && !trusted) return "only an admin's device adds an admin's";
      // Adding a key again as it is, by the device that let it in (approving a device again to finish it), changes
      // nothing and is done.
      const same = it && it.by === told.by && it.role === told.role && it.admin === (wants ? 1 : 0);
      if (it && told.add === told.by && told.role === "client" && is(it, "compute") && it.admin === (wants ? 1 : 0)) {
        this.sql.exec("UPDATE keys SET role = 'client' WHERE key = ?", told.add);
      } else if (it && !same) {
        if (!trusted) return { error: "that key is in already: only an admin's device changes it", code: CODES.already };
        if (it.admin === 1 && !wants && this.admins() === 1) return "that's the last admin's key";
        this.sql.exec("UPDATE keys SET admin = ?, role = ? WHERE key = ?", wants ? 1 : 0, told.role, told.add);
        if (told.role !== "client" && (it.role !== told.role || it.admin !== (wants ? 1 : 0))) this.tellRelay((r) => r.sessionsEnded(told.add));
      } else if (!it) {
        this.sql.exec("INSERT INTO keys (key, by, admin, role) VALUES (?, ?, ?, ?)", told.add, told.by, wants ? 1 : 0, told.role);
      }
      if (told.recovery === true && admin) this.sql.exec("UPDATE keys SET by = 'recovery' WHERE key = ?", told.add);
      // A pairing code's computer, added, is one of the network's: it reaches every endpoint now.
      this.sql.exec("DELETE FROM pairings WHERE key = ?", told.add);
      this.tellRelay((r) => r.added(told.add));
      return null;
    }
    if (told.admins !== undefined) {
      if (!admin) return "only an admin's device says who's an admin";
      const roles = told.admins && typeof told.admins === "object" ? Object.entries(told.admins) : [];
      if (!roles.length || !roles.every(([key, is]) => isKey(key) && typeof is === "boolean")) return "admins maps each key, in hex, to whether it's an admin's";
      for (const [key, is] of roles) {
        const it = this.key(key);
        if (it && (it.admin === 1) !== is && (is || this.admins() > 1)) this.sql.exec("UPDATE keys SET admin = ? WHERE key = ?", is ? 1 : 0, key);
      }
      return null;
    }
    if (told.remove !== undefined) {
      if (!isKey(told.remove)) return "remove is a key, in hex";
      const it = this.key(told.remove);
      if (!it) {
        const pairing = this.sql.exec("SELECT hash, by FROM pairings WHERE key = ?", told.remove).toArray()[0];
        if (!pairing) return { error: "that key isn't on the list", code: CODES.notListed };
        if (!admin && pairing.by !== told.by) return "only an admin's device or the one that made its pairing code takes it out";
        this.sql.exec("DELETE FROM pairings WHERE hash = ?", pairing.hash);
        this.tellRelay((r) => r.ended(told.remove, "taken out"));
        return null;
      }
      if (!admin && it.by !== told.by && told.remove !== told.by) return "only an admin's device, the one that let it in, or itself takes a key out";
      if (it.admin === 1 && this.admins() === 1) return "that's the last admin's key";
      this.remove(told.remove);
      return null;
    }
    if (told.pairing !== undefined) {
      const { hash, until } = told.pairing || {};
      if (!isKey(hash) || !Number.isSafeInteger(until) || until <= now) return "a pairing is its ticket's hash, in hex, and until when it works";
      this.sql.exec("DELETE FROM pairings WHERE until <= ?", now);
      if (this.sql.exec("SELECT count(*) AS n FROM pairings WHERE by = ?", told.by).one().n >= PAIRINGS_PER_DEVICE) return "that device has made too many pairing codes that still work";
      if (this.pairing(hash)) return "that pairing code is made already";
      this.sql.exec("INSERT INTO pairings VALUES (?, ?, ?, NULL)", hash, Math.min(until, now + PAIRING_LIFE), told.by);
      return null;
    }
    if (told.unpair !== undefined) {
      const pairing = isKey(told.unpair) && this.pairing(told.unpair);
      if (!pairing) return "there's no such pairing code";
      if (!admin && pairing.by !== told.by) return "only an admin's device or the one that made it ends a pairing code";
      this.sql.exec("DELETE FROM pairings WHERE hash = ?", pairing.hash);
      if (pairing.key && !this.key(pairing.key)) this.tellRelay((r) => r.ended(pairing.key, "its pairing code ended"));
      return null;
    }
    if (told.update !== undefined || told.cloudflare === true) {
      const key = told.cloudflare === true ? (this.env.DEPLOYED ? JSON.parse(this.env.DEPLOYED).from : null) : told.update;
      if (!isKey(key) || !this.key(key)) return { error: "that key isn't on the list", code: CODES.notListed };
      this.sql.exec("INSERT OR REPLACE INTO asks VALUES (?, ?)", key, JSON.stringify(signed));
      return null;
    }
    return "a tell adds or removes a key, makes or ends a pairing code, or asks a device to update, or to deploy the Workers";
  }

  /// Takes `key` out, with the pairing codes it made, and has the Relay close its sockets and binding. Only a tell naming a
  /// key takes it out: an admin's node names each device the signed steps removed (`Node::tell_removed`), so the devices
  /// it let in stay.
  remove(key) {
    this.sql.exec("DELETE FROM keys WHERE key = ?", key);
    this.sql.exec("DELETE FROM asks WHERE key = ?", key);
    this.forgetObject(key);
    const joined = this.sql.exec("SELECT key FROM pairings WHERE by = ? AND key IS NOT NULL", key).toArray().map((r) => r.key);
    this.sql.exec("DELETE FROM pairings WHERE by = ?", key);
    this.tellRelay((r) => {
      for (const k of joined) r.ended(k, "taken out");
      return r.removed(key);
    });
  }

  /// A removed key of an outside program's object (`bind` keeps which): the object deletes what it holds for that key,
  /// and a device signed in again in its place stays.
  forgetObject(key) {
    const [object] = this.sql.exec("DELETE FROM objects WHERE key = ? RETURNING name", key).toArray();
    if (!object || !this.env.MCP) return;
    const stub = this.env.MCP.get(this.env.MCP.idFromName(object.name), near(this.env));
    this.ctx.waitUntil(stub.removed(key).catch((err) => console.log(JSON.stringify({ forgotten: "not", why: String(err?.message ?? err) }))));
  }

  /// Which key let in each key on the list, for the Devices screen to say which outside program let a device in.
  letIn() {
    return Object.fromEntries(this.sql.exec("SELECT key, by FROM keys").toArray().map((r) => [r.key, r.by]));
  }
}
