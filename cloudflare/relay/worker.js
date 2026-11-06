// The relay: how the network's endpoints reach each other when they can't directly, speaking iroh's relay protocol at
// the network's address (`/relay`, `/ping`, `/generate_204`, which the gateway sends here). One Durable Object per
// network keeps every endpoint's WebSocket, with Hibernation, so it sleeps between packets; iroh-relay's own code
// (relay_protos_bg.wasm, built from protos/ by build.sh) checks each endpoint's handshake and reads and writes its
// frames. It's blind: what endpoints send each other is QUIC, sealed end to end, so it can only drop it.
//
// It admits only the network's endpoints, on its one list of them: `{ by, admin, role }` per key, `by` the key that let
// it in, `role` what it is (`client`, `compute` or `service`, or a machine's several, with spaces). The network's first
// devices, and its recovery key as a client, are put in when the network is made (`DEVICES`, `{ key, role, admin }`
// each, from the Workers'
// deploy, taken only while the list is empty, which it never is again: the last admin can't be taken out); after that,
// devices tell it, and nothing else changes it. A computer joining with a pairing code a device made gets in too, by
// the ticket it shows as iroh's relay auth token, until the code expires, and reaches only that device until it's
// added. A session's endpoint gets in too (a browser's: its device key can't be read out of WebCrypto, and iroh signs
// with raw bytes), by the certificate it shows as its auth token (keys::certify), signed by a client's key on the list,
// naming the key it proves, until then: it reaches every endpoint as the key that signed it would, until its
// certificate's `until` or that key is taken out or made something else. With the certificate, `.`, and a request that
// key signed just now (`proveSocket`: within ten seconds, its nonce taken once), it's taken in at once, with no
// challenge's round trip; without one, or with one that's stale or taken, it answers the challenge as any key does. The
// URL signer, sign-in and the push Worker ask it, by `allowed(key)`: `{ admin, role }`, or null.
//
// A browser's WebSocket can't set a header, so its certificate is `?token=` in the relay's URL, where logs may keep
// it. iroh shows the same token at every reconnect, so its proof is used once, by the socket the web client opens
// itself (net-worker.js); a certificate alone gets nothing: the relay lets in only the endpoint that proves the key it
// names, and answers computes only to a request that key signed.
//
// `POST /relay/computes` answers a request signed by a key on the list, or by a session's key with its certificate
// (`one/relay`, `{ computes: true, certificate? }`, each `(by, at)` taken once, as a tell is), with the compute
// endpoints' keys, the Cloudflare node's among them, in no order: a new client's first nodes to ask; and `online`, those
// of them connected here now, which a client races first. 403 is only for a key that isn't let in; a request
// that's wrong, or signed by a clock that's off, is 400, so a client keeps the nodes it knows. A device's own request
// that says it reads less of who's who than this build (`reads`, shared/crypto.js's `READS`; an older build's says
// nothing) is 426, telling it to update. A browser's session runs the code the address serves, the newest, so it isn't
// asked.
//
// What a device tells it is `POST /relay/tell`, its body a request signed by a key on the list (`signedRequest` in
// shared/crypto.js, domain `one/relay`, each `(by, at)` taken once) with one of:
// - `{ add, role, admin? }`: a key (hex) to let in, and what it is: a client by any key on the list, a compute or a
//   service only by an admin's or a compute's (nodes add the computers they pair and their people's services), and an
//   admin only by an admin's; one already in is changed only by an admin's, keeping who let it in.
// - `{ remove }`: a key to take out, by an admin, the key that let it in, or itself, with the keys it let in that aren't
//   admins', and theirs, and the pairing codes they made; never the last admin. A pairing code's computer is taken out
//   the same way, by its code's maker too.
// - `{ pairing: { hash, until } }`: the SHA-256 (hex) of a pairing code's ticket and until when it works (ms).
// - `{ unpair }`: a pairing code's hash, to end it, by its maker or an admin.
//
// One endpoint isn't a socket: the Cloudflare node (`BOUND`, its key), a Durable Object of its own Worker (`NODE`) that
// sleeps until someone calls it. Datagrams sent to it go to it by RPC, gathered each turn, waking it; what it sends
// comes back by RPC (`fromNode`, on the `ServerlessNode` entrypoint, which only its Worker binds) and goes out as from
// it. It's one of the network's endpoints, as compute.
//
// Nor are the remote MCP server's devices (cloudflare/mcp/), one per remote MCP client, each in a Durable Object of that Worker's
// (`MCP`) while it runs: one binds its key (`bind`, a request signed by that key, which must be on the list) to its
// object, which then gets the datagrams sent to that key by RPC (`deliver`), and passes back by RPC what it sends
// (`fromClient`), until it unbinds or its binding lapses. Binding gives it a ticket, which sending and unbinding must
// show, so one device's object sends only as itself. The remote MCP server binds the `McpDevices` entrypoint, never the
// relay's object, so it can't tell the relay anything a device didn't sign.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import wasm from "./relay_protos_bg.wasm";
import { initSync, challenge, verify, confirm, deny, from_client, endpoint_gone, datagrams_of, to_client } from "./relay_protos.js";
import { b64u, checkCertificate, checkRequest, FRESH, READS, SOCKET, SOCKET_FRESH } from "../shared/crypto.js";

initSync({ module: wasm });

/// The WebSocket subprotocol iroh's endpoints ask for: version 2 of its relay protocol.
const PROTOCOL = "iroh-relay-v2";
/// What a tell is signed for, and its most bytes.
const TELL = "one/relay";
const TELL_BYTES = 4096;
/// How long a socket may take to answer its challenge, how many sockets that haven't may come from one place (an IPv4
/// address, an IPv6 /64), from one IPv6 /48 (a site's, which holds 65,536 /64s) and from everywhere, how long a
/// pairing code may work, and how many one device may have.
const PROVE_WITHIN = 30_000;
const UNPROVEN_PER_PLACE = 8;
const UNPROVEN_PER_SITE = 32;
const UNPROVEN = 256;
const PAIRING_FOR = 7 * 86_400_000;
/// How often an endpoint's last time here is noted.
const HOUR = 3_600_000;
const PAIRINGS_PER_DEVICE = 8;
const ROLES = ["client", "compute", "service"];
/// Whether a key on the list plays `role`.
const is = (it, role) => !!it && it.role.split(" ").includes(role);
/// How often a sender is told that an endpoint it sends to isn't here, and how many such it remembers.
const GONE_EVERY = 5_000;
const GONE_KEPT = 1024;
/// The most datagrams for the Cloudflare node gathered in one turn before they're handed to it.
const FOR_NODE_MOST = 1024;

const hex = (bytes) => [...new Uint8Array(bytes)].map((b) => b.toString(16).padStart(2, "0")).join("");
const unhex = (s) => Uint8Array.from(s.match(/../g) || [], (h) => parseInt(h, 16));
const isKey = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
const sha256 = async (text) => hex(await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text)));
const refuse = (status, error) => Response.json({ error }, { status });
const CORS = { "Access-Control-Allow-Origin": "*" };

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
    if (url.pathname === "/ping") return new Response(null, { status: 200, headers: CORS });
    // iroh's captive portal check, answered as iroh-relay's own server does.
    if (url.pathname === "/generate_204") {
      const headers = { ...CORS };
      const challenge = request.headers.get("x-iroh-challenge");
      if (challenge && /^[A-Za-z0-9._-]{1,63}$/.test(challenge)) headers["X-Iroh-Response"] = `response ${challenge}`;
      return new Response(null, { status: 204, headers });
    }
    if (url.pathname === "/relay/tell") return this.tell(request);
    if (url.pathname === "/relay/computes") return this.ask(request, "computes");
    if (url.pathname === "/relay/seen") return this.ask(request, "seen");
    if (url.pathname === "/relay") return this.relay().fetch(request);
    return refuse(404, "not the relay's");
  }

  /// A tell's signature is checked here first, where a forged one costs the network's one relay nothing; the relay
  /// checks it again, and the rest.
  async tell(request) {
    try {
      if (request.method !== "POST") throw new Error("a tell is a POST");
      if (Number(request.headers.get("content-length") || 0) > TELL_BYTES) throw new Error("a tell is small");
      const body = await request.arrayBuffer();
      if (body.byteLength > TELL_BYTES) throw new Error("a tell is small");
      const signed = JSON.parse(new TextDecoder().decode(body));
      await checkRequest(TELL, signed);
      const error = await this.relay().tell(signed);
      return error ? refuse(403, error) : new Response(null, { status: 204 });
    } catch (e) {
      return refuse(403, String(e.message || e));
    }
  }

  /// What the relay tells the network's endpoints (`computes`, or when each was last `seen`), for a request signed by a
  /// key on the list or a session's endpoint.
  async ask(request, what) {
    let asked;
    try {
      if (request.method !== "POST") throw new Error(`ask for ${what} with a POST`);
      const body = await request.arrayBuffer();
      if (body.byteLength > TELL_BYTES) throw new Error("that's too big");
      const { by, fields } = await checkRequest(TELL, JSON.parse(new TextDecoder().decode(body)));
      if (fields[what] !== true) throw new Error(`ask with { ${what}: true }, signed`);
      asked = { by, at: fields.at, reads: Number.isSafeInteger(fields.reads) ? fields.reads : 0 };
      if (fields.certificate !== undefined) {
        const session = await checkCertificate(fields.certificate);
        if (session.endpoint !== by) throw new Error("sign with the key your certificate names");
        asked.session = session.by;
      }
    } catch (e) {
      return refuse(400, String(e.message || e));
    }
    const said = await this.relay()[what](asked.by, asked);
    if (said?.update) return refuse(426, `Update this device: ${this.env.YOURS}'s other devices have moved on to what this one doesn't read yet.`);
    // Which computes are connected now, which a client races first (an older one reads only `computes`).
    if (what === "computes" && Array.isArray(said)) return Response.json({ computes: said, online: await this.relay().connected(said) });
    if (said && typeof said === "object") return Response.json({ [what]: said });
    return said === null ? refuse(403, `the relay takes only ${this.env.YOURS}'s devices`) : refuse(400, said);
  }

  /// Whether `key` (hex) is one of the network's endpoints: `{ admin }` if it is, for the URL signer, by its binding.
  async allowed(key) {
    return isKey(key) ? this.relay().allowed(key) : null;
  }

  relay() {
    return relay(this.env);
  }
}

/// The network's one relay, made near its people (`LOCATION`, a location hint) rather than wherever its first request
/// came from.
const near = (env) => (env.LOCATION ? { locationHint: env.LOCATION } : {});
const relay = (env) => env.RELAY.get(env.RELAY.idFromName("relay"), near(env));

/// What the remote MCP server asks the relay, by its binding: its devices' own bindings, and who's here. Nothing else of the
/// relay's object, so a hacked remote MCP server tells it nothing and sends as no endpoint but a device it holds the key of.
export class McpDevices extends WorkerEntrypoint {
  /// Binds a device's key to its object, by `request` signed with that key: → `{ ticket, until }`, or `{ error }`.
  bind(request, name) {
    return relay(this.env).bind(request, name);
  }

  /// Unbinds a device's key, by the ticket its binding gave.
  unbind(key, ticket) {
    return relay(this.env).unbind(key, ticket);
  }

  /// Datagrams a device sends, as from its key, by the ticket its binding gave: → the keys of those not here.
  fromClient(key, ticket, items) {
    return relay(this.env).fromClient(key, ticket, items);
  }

  allowed(key) {
    return isKey(key) ? relay(this.env).allowed(key) : null;
  }

  computes(by) {
    return relay(this.env).computes(by);
  }

  connected(keys) {
    return Array.isArray(keys) ? relay(this.env).connected(keys.filter(isKey)) : [];
  }
}

/// What the Cloudflare node asks the relay, by its binding: sending as itself, and who's here.
export class ServerlessNode extends WorkerEntrypoint {
  fromNode(items) {
    return relay(this.env).fromNode(items);
  }

  connected(keys) {
    return Array.isArray(keys) ? relay(this.env).connected(keys.filter(isKey)) : [];
  }
}

export class Relay extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    this.sql = ctx.storage.sql;
    // The list: each key with the key that let it in; pairing codes, with the device that made them and the computer
    // they let in, once one has; and the tells taken.
    this.sql.exec(`CREATE TABLE IF NOT EXISTS keys (key TEXT PRIMARY KEY, by TEXT NOT NULL, admin INTEGER NOT NULL, role TEXT NOT NULL);
      CREATE TABLE IF NOT EXISTS pairings (hash TEXT PRIMARY KEY, until INTEGER NOT NULL, by TEXT NOT NULL, key TEXT);
      CREATE TABLE IF NOT EXISTS tells (by TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (by, at));
      CREATE TABLE IF NOT EXISTS proofs (nonce TEXT PRIMARY KEY, at INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS bound (key TEXT PRIMARY KEY, name TEXT NOT NULL, until INTEGER NOT NULL, ticket TEXT NOT NULL DEFAULT '');
      CREATE TABLE IF NOT EXISTS seen (key TEXT PRIMARY KEY, at INTEGER NOT NULL);
      DROP TABLE IF EXISTS switched;`);
    // A binding made before tickets: none shows one, so each is made again.
    try {
      this.sql.exec("ALTER TABLE bound ADD COLUMN ticket TEXT NOT NULL DEFAULT ''");
    } catch {}
    this.sql.exec("DELETE FROM bound WHERE ticket = ''");
    // The first devices were let in by the network, which a release from before it was called one wrote as `install`.
    this.sql.exec("UPDATE keys SET by = 'network' WHERE by = 'install'");
    if (!this.sql.exec("SELECT 1 FROM keys LIMIT 1").toArray().length) {
      for (const { key, role, admin } of this.env.DEVICES ? JSON.parse(this.env.DEVICES) : []) {
        if (isKey(key) && String(role).split(" ").every((r) => ROLES.includes(r))) this.sql.exec("INSERT OR IGNORE INTO keys VALUES (?, 'network', ?, ?)", key, admin === true ? 1 : 0, role);
      }
    }
    /// Each proven endpoint's socket, by its key; kept in the sockets' attachments across hibernation.
    this.sockets = new Map();
    for (const ws of ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said?.id) this.sockets.set(said.id, ws);
    }
    /// The remote MCP server's devices bound here: key → `{ name, until, ticket }`, its object's name, and what it shows.
    this.bound = new Map();
    for (const b of this.sql.exec("SELECT key, name, until, ticket FROM bound WHERE until > ?", Date.now()).toArray()) this.bound.set(b.key, { name: b.name, until: b.until, ticket: b.ticket });
    /// When each sender was last told an endpoint isn't here, `<from> <to>` → ms; lost on hibernation, which is fine.
    this.told = new Map();
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

  /// `{ admin, role }` if `key` is one of the network's endpoints; a computer let in by a pairing code isn't, until it's
  /// added.
  allowed(key) {
    if (key === this.env.BOUND) return { admin: false, role: "compute" };
    const it = this.key(key);
    return it ? { admin: it.admin === 1, role: it.role } : null;
  }

  /// Whether `by` may be told what the network's endpoints are told: it's on the list, or is a session's a client on it
  /// vouched for (`session`); null if neither. A request's (`at`) is taken once: → why not, if it was taken already.
  asks(by, { at, session } = {}) {
    // Who's asking first, as `tell` does, so a stranger's requests are never kept.
    if (session !== undefined ? !is(this.key(session), "client") : !this.key(by)) return null;
    if (at !== undefined && !this.take(by, at)) return "that request was taken already";
    return true;
  }

  /// The compute endpoints' keys, for whom `asks` lets ask: → else null or why not.
  computes(by, asked) {
    const may = this.asks(by, asked);
    if (may !== true) return may;
    if (asked && !asked.session && asked.reads < READS) return { update: true };
    const keys = this.sql.exec("SELECT key FROM keys WHERE ' ' || role || ' ' LIKE '% compute %'").toArray().map((r) => r.key);
    return this.env.BOUND ? [...keys, this.env.BOUND] : keys;
  }

  /// When each endpoint on the list was last let in here (ms), a session's as the device that vouched for it, for whom
  /// `asks` lets ask: → `{ key: at }`, else null or why not.
  seen(by, asked) {
    const may = this.asks(by, asked);
    if (may !== true) return may;
    return Object.fromEntries(this.sql.exec("SELECT seen.key, seen.at FROM seen JOIN keys ON keys.key = seen.key").toArray().map((r) => [r.key, r.at]));
  }

  /// Takes a signed request, `(by, at)`, once: → whether it wasn't taken before.
  take(by, at) {
    const now = Date.now();
    this.sql.exec("DELETE FROM tells WHERE at < ? OR at > ?", now - 2 * FRESH, now + 2 * FRESH);
    if (this.sql.exec("SELECT 1 FROM tells WHERE by = ? AND at = ?", by, at).toArray().length) return false;
    this.sql.exec("INSERT INTO tells VALUES (?, ?)", by, at);
    return true;
  }

  async fetch(request) {
    const url = new URL(request.url);
    if (request.headers.get("upgrade") !== "websocket") return refuse(426, "the relay takes an endpoint's WebSocket");
    const protocols = (request.headers.get("sec-websocket-protocol") || "").split(",").map((p) => p.trim());
    if (!protocols.includes(PROTOCOL)) return refuse(400, `the relay speaks ${PROTOCOL}`);
    const now = Date.now();
    // A pairing code's ticket (64 hex), or a session's certificate, if it shows one: whether it lets this key in is
    // decided once the key is proved, here if the certificate comes with its endpoint's proof.
    const ticket = token(request, url);
    const joining = isKey(ticket) ? await sha256(ticket) : null;
    if (joining && !(this.pairing(joining)?.until > now)) return refuse(403, "that pairing code has expired");
    let session = null;
    let proved = false;
    if (ticket && !joining) {
      const [certificate, proof] = ticket.split(".");
      try {
        session = await checkCertificate(certificate, now);
      } catch (e) {
        return refuse(403, String(e.message || e));
      }
      proved = !!proof && (await this.proves(proof, session.endpoint, now));
    }
    // Taken in at once only for a certificate its signer made and its endpoint's own proof, so this tells only them
    // whether they're on the list.
    if (proved && is(this.key(session.by), "client")) {
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
        ws.serializeAttachment({});
        ws.close(1008, "it never answered its challenge");
        continue;
      }
      unproven++;
      if (said.from === from) here++;
      if (said.site === site) near++;
    }
    if (here >= UNPROVEN_PER_PLACE || near >= UNPROVEN_PER_SITE || unproven >= UNPROVEN) return refuse(429, "too many sockets that haven't proved their key");
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    const made = challenge();
    server.serializeAttachment({ challenge: hex(made.slice(0, 16)), at: now, from, site, joining, session });
    server.send(made.slice(16));
    return new Response(null, { status: 101, webSocket: client, headers: { "Sec-WebSocket-Protocol": PROTOCOL } });
  }

  async webSocketMessage(ws, message) {
    if (typeof message === "string") return ws.close(1003, "binary only");
    const bytes = new Uint8Array(message);
    const said = ws.deserializeAttachment();
    if (said?.challenge) return this.prove(ws, said, bytes);
    if (!said?.id) return;
    const now = Date.now();
    // A pairing code's computer, until it's added: only while its code works, and only to the device that made it. A
    // session's endpoint, only until its certificate's `until`.
    if (said.pairing && now > said.until) return this.end(said.id, "its pairing code expired");
    if (said.by && now > said.until) return this.end(said.id, "its session ended");
    let out;
    try {
      out = from_client(unhex(said.id), bytes);
    } catch (e) {
      return ws.close(1002, String(e.message || e).slice(0, 100));
    }
    if (out[0] === 1) return ws.send(out.subarray(1));
    if (out[0] !== 0) return;
    const to = hex(out.subarray(1, 33));
    if (said.pairing && to !== said.to) return;
    if (to === this.env.BOUND && this.env.NODE) return this.toNode(said.id, bytes);
    const client = this.boundTo(to);
    if (client) return this.toClient(to, client, said.id, bytes);
    const other = this.sockets.get(to);
    if (other) {
      try {
        return other.send(out.subarray(33));
      } catch {}
    }
    // Its sender drops its path through here sooner.
    const pair = `${said.id} ${to}`;
    if (now - (this.told.get(pair) || 0) < GONE_EVERY) return;
    if (this.told.size >= GONE_KEPT) {
      for (const [p, at] of this.told) if (now - at >= GONE_EVERY) this.told.delete(p);
      if (this.told.size >= GONE_KEPT) this.told.clear();
    }
    this.told.set(pair, now);
    ws.send(endpoint_gone(unhex(to)));
  }

  /// Datagrams for the Cloudflare node, from `src`: gathered this turn, then handed to it by RPC, which wakes it.
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
          to.send(to_client(unhex(this.env.BOUND), seg, contents));
        } catch {}
    }
    return [...gone];
  }

  /// Which of `keys` are connected here now.
  connected(keys) {
    return keys.filter((k) => this.sockets.has(k));
  }

  /// The remote MCP server's object a key is bound to, if it is: its name.
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
      ({ fields } = await checkRequest(TELL, request));
      if (!this.take(fields.by, fields.at)) throw new Error("that binding was taken already");
    } catch (e) {
      return { error: String(e.message || e) };
    }
    if (fields.bind !== true || typeof name !== "string" || !name || name.length > 256) return { error: "a binding is a key's signed bind, to an object's name" };
    if (!this.key(fields.by)) return { error: "that key isn't one of the network's: a device of its person approves it" };
    if (this.sockets.has(fields.by)) return { error: "that key is connected here itself" };
    const until = Date.now() + 3_600_000;
    const ticket = hex(crypto.getRandomValues(new Uint8Array(32)));
    this.sql.exec("INSERT OR REPLACE INTO bound VALUES (?, ?, ?, ?)", fields.by, name, until, ticket);
    this.bound.set(fields.by, { name, until, ticket });
    return { ticket, until };
  }

  /// Unbinds `key`: by the remote MCP server, with its binding's ticket, or here, as it's taken out or its binding lapses.
  unbind(key, ticket) {
    if (ticket !== undefined && !this.ticketed(key, ticket)) return;
    this.bound.delete(key);
    this.sql.exec("DELETE FROM bound WHERE key = ?", key);
  }

  /// Whether `ticket` is the one `key`'s binding gave.
  ticketed(key, ticket) {
    const b = this.bound.get(key);
    return !!b && typeof ticket === "string" && ticket.length === 64 && b.ticket === ticket;
  }

  /// Datagrams for one of the remote MCP server's devices, `to`, from `src`: gathered this turn, then handed to its object.
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

  /// Datagrams one of the remote MCP server's devices sends, `[dst, segment size, contents]` each, passed on as from it, if
  /// it's bound: → the keys of those not here.
  fromClient(key, ticket, items) {
    if (!Array.isArray(items)) return [];
    if (!this.boundTo(key) || !this.ticketed(key, ticket)) return items.map(([dst]) => dst);
    const gone = new Set();
    for (const [dst, seg, contents] of items) {
      const to = this.sockets.get(dst);
      if (!to) gone.add(dst);
      else
        try {
          to.send(to_client(unhex(key), seg, contents));
        } catch {}
    }
    return [...gone];
  }

  /// A socket's answer to its challenge: it's let in if it proves a key on the list, or the key its pairing code let in.
  async prove(ws, said, bytes) {
    let id;
    try {
      id = hex(verify(unhex(said.challenge), bytes));
    } catch {
      id = null;
    }
    let attachment = id && this.key(id) ? { id } : null;
    // A session's endpoint: the key its certificate names, signed by a client's key on the list. Checked here, not when
    // it connects, so that tells no one who's on the list either.
    if (id && !attachment && said.session?.endpoint === id && is(this.key(said.session.by), "client") && said.session.until > Date.now()) {
      attachment = { id, by: said.session.by, until: said.session.until };
    }
    if (id && !attachment && said.joining) {
      const pairing = this.pairing(said.joining);
      if (pairing && pairing.until > Date.now() && [null, id].includes(pairing.key)) {
        // A pairing code lets in one computer: this one, again if it reconnects, until it expires or it's added.
        this.sql.exec("UPDATE pairings SET key = ? WHERE hash = ?", id, pairing.hash);
        attachment = { id, pairing: pairing.hash, until: pairing.until, to: pairing.by };
      }
    }
    if (!attachment) {
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

  /// Takes a socket in as `id`, as `attachment` says, and tells it so.
  async admit(ws, id, attachment) {
    if (attachment.until) await this.wake(attachment.until);
    // The newest socket of a key is the one it's reached at.
    for (const other of this.ctx.getWebSockets()) {
      if (other !== ws && other.deserializeAttachment()?.id === id) {
        other.serializeAttachment({});
        try {
          other.close(1000, "the endpoint connected again");
        } catch {}
      }
    }
    ws.serializeAttachment(attachment);
    this.sockets.set(id, ws);
    ws.send(confirm());
    // When it was last here, as its device (a session's, the one that vouched for it), at most once an hour.
    const device = attachment.by || id, now = Date.now();
    if (this.key(device)) this.sql.exec("INSERT INTO seen VALUES (?, ?) ON CONFLICT (key) DO UPDATE SET at = excluded.at WHERE at < ?", device, now, now - HOUR);
  }

  /// Closes `key`'s socket, if it has one.
  end(key, why) {
    const ws = this.sockets.get(key);
    this.sockets.delete(key);
    if (!ws) return;
    ws.serializeAttachment({});
    try {
      ws.close(1008, why);
    } catch {}
  }

  /// An alarm at `until`, if none is sooner, to close pairing codes' computers once their codes expire, and sessions'
  /// endpoints once theirs end.
  async wake(until) {
    const at = await this.ctx.storage.getAlarm();
    if (!at || at > until) await this.ctx.storage.setAlarm(until);
  }

  async alarm() {
    const now = Date.now();
    let next = null;
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (!said?.pairing && !said?.by) continue;
      if (said.until <= now) this.end(said.id, said.pairing ? "its pairing code expired" : "its session ended");
      else next = Math.min(next ?? said.until, said.until);
    }
    this.sql.exec("DELETE FROM pairings WHERE until <= ?", now);
    if (next) await this.ctx.storage.setAlarm(next);
  }

  /// What a device told the relay, `signed` by it (`signedRequest`), checked here whoever passed it on: → why it was
  /// refused, or nothing.
  async tell(signed) {
    const now = Date.now();
    let told;
    try {
      ({ fields: told } = await checkRequest(TELL, signed));
    } catch (e) {
      return String(e.message || e);
    }
    const by = this.key(told.by);
    if (!by) return `only ${this.env.YOURS}'s devices tell the relay`;
    if (!this.take(told.by, told.at)) return "that tell was taken already";
    const admin = by.admin === 1;
    if (told.add !== undefined) {
      if (!isKey(told.add)) return "add is a key, in hex";
      if (!ROLES.includes(told.role)) return `add says its role: ${ROLES.join(", ")}`;
      const wants = told.admin === true;
      const it = this.key(told.add);
      if (wants && !admin) return "only an admin's device adds an admin's";
      // Adding a key again as it is, by the device that let it in (approving a device again to finish it), changes
      // nothing and is done.
      const same = it && it.by === told.by && it.role === told.role && it.admin === (wants ? 1 : 0);
      if (it && !same) {
        if (!admin) return "that key is in already: only an admin's device changes it";
        if (it.admin === 1 && !wants && this.admins() === 1) return "that's the last admin's key";
        this.sql.exec("UPDATE keys SET admin = ?, role = ? WHERE key = ?", wants ? 1 : 0, told.role, told.add);
        // A key that's no longer a client vouches for no session.
        if (told.role !== "client") this.endSessionsOf(told.add);
      } else if (!it) {
        if (told.role !== "client" && !admin && !is(by, "compute")) return "only an admin's device or a compute adds a compute or a service";
        this.sql.exec("INSERT INTO keys VALUES (?, ?, ?, ?)", told.add, told.by, wants ? 1 : 0, told.role);
      }
      // A pairing code's computer, added, is one of the network's: it reaches every endpoint now.
      this.sql.exec("DELETE FROM pairings WHERE key = ?", told.add);
      const ws = this.sockets.get(told.add);
      if (ws?.deserializeAttachment()?.pairing) ws.serializeAttachment({ id: told.add });
      return null;
    }
    if (told.remove !== undefined) {
      if (!isKey(told.remove)) return "remove is a key, in hex";
      const it = this.key(told.remove);
      if (!it) {
        const pairing = this.sql.exec("SELECT hash, by FROM pairings WHERE key = ?", told.remove).toArray()[0];
        if (!pairing) return "that key isn't on the list";
        if (!admin && pairing.by !== told.by) return "only an admin's device or the one that made its pairing code takes it out";
        this.sql.exec("DELETE FROM pairings WHERE hash = ?", pairing.hash);
        this.end(told.remove, "taken out");
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
      this.sql.exec("INSERT INTO pairings VALUES (?, ?, ?, NULL)", hash, Math.min(until, now + PAIRING_FOR), told.by);
      return null;
    }
    if (told.unpair !== undefined) {
      const pairing = isKey(told.unpair) && this.pairing(told.unpair);
      if (!pairing) return "there's no such pairing code";
      if (!admin && pairing.by !== told.by) return "only an admin's device or the one that made it ends a pairing code";
      this.sql.exec("DELETE FROM pairings WHERE hash = ?", pairing.hash);
      if (pairing.key && !this.key(pairing.key)) this.end(pairing.key, "its pairing code ended");
      return null;
    }
    return "a tell adds or removes a key, or makes or ends a pairing code";
  }

  /// Takes `key` out, with the keys it let in that aren't admins' (theirs too, as far as they go) and the pairing codes
  /// they made, closing their sockets. Admins it let in stay: only a tell naming one takes it out.
  remove(key) {
    const gone = [key];
    for (let i = 0; i < gone.length; i++) {
      for (const { key: next } of this.sql.exec("SELECT key FROM keys WHERE by = ? AND admin = 0", gone[i]).toArray()) {
        if (!gone.includes(next)) gone.push(next);
      }
    }
    for (const k of gone) {
      this.sql.exec("DELETE FROM keys WHERE key = ?", k);
      this.unbind(k);
      for (const { key: joined } of this.sql.exec("SELECT key FROM pairings WHERE by = ? AND key IS NOT NULL", k).toArray()) this.end(joined, "taken out");
      this.sql.exec("DELETE FROM pairings WHERE by = ?", k);
      this.end(k, "taken out");
      this.endSessionsOf(k);
    }
  }

  /// Closes the sockets of the sessions `key` vouched for.
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
