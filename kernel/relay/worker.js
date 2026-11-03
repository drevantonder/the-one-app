// The relay: how the install's endpoints reach each other when they can't directly, speaking iroh's relay protocol at
// the install's address (`/relay`, `/ping`, `/generate_204`, which the gateway sends here). One Durable Object per
// install keeps every endpoint's WebSocket, with Hibernation, so it sleeps between packets; iroh-relay's own code
// (relay_protos_bg.wasm, built from protos/ by build.sh) checks each endpoint's handshake and reads and writes its
// frames. It's blind: what endpoints send each other is QUIC, sealed end to end, so it can only drop it.
//
// It admits only the install's endpoints, on its one list of them: `{ by, admin }` per key, `by` the key that let it
// in. The install's first devices, its admins', are put in when the install is made (`DEVICES`, from the kernel's
// deploy, taken only while the list is empty, which it never is again: the last admin can't be taken out); after that,
// devices tell it, and nothing else changes it. A computer joining with a pairing code a device made gets in too, by
// the ticket it shows as iroh's relay auth token, until the code expires, and reaches only that device until it's
// added. The URL signer asks it, by `allowed(key)`.
//
// What a device tells it is `POST /relay/tell`, its body a request signed by a key on the list (`signedRequest` in
// shared/crypto.js, domain `one/relay`, each `(by, at)` taken once) with one of:
// - `{ add, admin? }`: a key (hex) to let in; one already in is changed only by an admin, and an admin's added only by
//   one.
// - `{ remove }`: a key to take out, by an admin, the key that let it in, or itself, with the keys it let in that aren't
//   admins', and theirs, and the pairing codes they made; never the last admin. A pairing code's computer is taken out
//   the same way, by its code's maker too.
// - `{ pairing: { hash, until } }`: the SHA-256 (hex) of a pairing code's ticket and until when it works (ms).
// - `{ unpair }`: a pairing code's hash, to end it, by its maker or an admin.
import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import wasm from "./relay_protos_bg.wasm";
import { initSync, challenge, verify, confirm, deny, from_client, endpoint_gone } from "./relay_protos.js";
import { checkRequest, FRESH } from "../shared/crypto.js";

initSync({ module: wasm });

/// The WebSocket subprotocol iroh's endpoints ask for: version 2 of its relay protocol.
const PROTOCOL = "iroh-relay-v2";
/// What a tell is signed for, and its most bytes.
const TELL = "one/relay";
const TELL_BYTES = 4096;
/// How long a socket may take to answer its challenge, how many sockets that haven't may come from one place (an IPv4
/// address, an IPv6 /64) and from everywhere, how long a pairing code may work, and how many one device may have.
const PROVE_WITHIN = 30_000;
const UNPROVEN_PER_PLACE = 8;
const UNPROVEN = 256;
const PAIRING_FOR = 7 * 86_400_000;
const PAIRINGS_PER_DEVICE = 8;
/// How often a sender is told that an endpoint it sends to isn't here, and how many such it remembers.
const GONE_EVERY = 5_000;
const GONE_KEPT = 1024;

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

/// Where a socket comes from, for counting those that haven't proved their key: an IPv4 address, or an IPv6 /64.
function place(request) {
  const address = request.headers.get("cf-connecting-ip") || "";
  if (!address.includes(":")) return address;
  const [head, tail = ""] = address.split("::");
  const groups = head.split(":").filter(Boolean);
  const rest = tail.split(":").filter(Boolean);
  const full = [...groups, ...Array(Math.max(0, 8 - groups.length - rest.length)).fill("0"), ...rest];
  return full.slice(0, 4).map((g) => parseInt(g, 16).toString(16)).join(":") + "::/64";
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
    if (url.pathname === "/relay") return this.relay().fetch(request);
    return refuse(404, "not the relay's");
  }

  /// A tell's signature is checked here, where it costs the install's one relay nothing; the relay checks the rest.
  async tell(request) {
    try {
      if (request.method !== "POST") throw new Error("a tell is a POST");
      if (Number(request.headers.get("content-length") || 0) > TELL_BYTES) throw new Error("a tell is small");
      const body = await request.arrayBuffer();
      if (body.byteLength > TELL_BYTES) throw new Error("a tell is small");
      const { fields } = await checkRequest(TELL, JSON.parse(new TextDecoder().decode(body)));
      const error = await this.relay().tell(fields);
      return error ? refuse(403, error) : new Response(null, { status: 204 });
    } catch (e) {
      return refuse(403, String(e.message || e));
    }
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
    this.sql = ctx.storage.sql;
    // The list: each key with the key that let it in; pairing codes, with the device that made them and the computer
    // they let in, once one has; and the tells taken.
    this.sql.exec(`CREATE TABLE IF NOT EXISTS keys (key TEXT PRIMARY KEY, by TEXT NOT NULL, admin INTEGER NOT NULL);
      CREATE TABLE IF NOT EXISTS pairings (hash TEXT PRIMARY KEY, until INTEGER NOT NULL, by TEXT NOT NULL, key TEXT);
      CREATE TABLE IF NOT EXISTS tells (by TEXT NOT NULL, at INTEGER NOT NULL, PRIMARY KEY (by, at));`);
    if (!this.sql.exec("SELECT 1 FROM keys LIMIT 1").toArray().length) {
      for (const key of this.env.DEVICES ? JSON.parse(this.env.DEVICES) : []) {
        if (isKey(key)) this.sql.exec("INSERT OR IGNORE INTO keys VALUES (?, 'install', 1)", key);
      }
    }
    /// Each proven endpoint's socket, by its key; kept in the sockets' attachments across hibernation.
    this.sockets = new Map();
    for (const ws of ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (said?.id) this.sockets.set(said.id, ws);
    }
    /// When each sender was last told an endpoint isn't here, `<from> <to>` → ms; lost on hibernation, which is fine.
    this.told = new Map();
  }

  key(key) {
    return this.sql.exec("SELECT key, by, admin FROM keys WHERE key = ?", key).toArray()[0];
  }

  pairing(hash) {
    return this.sql.exec("SELECT hash, until, by, key FROM pairings WHERE hash = ?", hash).toArray()[0];
  }

  admins() {
    return this.sql.exec("SELECT count(*) AS n FROM keys WHERE admin = 1").one().n;
  }

  /// `{ admin }` if `key` is one of the install's endpoints; a computer let in by a pairing code isn't, until it's added.
  allowed(key) {
    const it = this.key(key);
    return it ? { admin: it.admin === 1 } : null;
  }

  async fetch(request) {
    const url = new URL(request.url);
    if (request.headers.get("upgrade") !== "websocket") return refuse(426, "the relay takes an endpoint's WebSocket");
    const protocols = (request.headers.get("sec-websocket-protocol") || "").split(",").map((p) => p.trim());
    if (!protocols.includes(PROTOCOL)) return refuse(400, `the relay speaks ${PROTOCOL}`);
    const now = Date.now();
    // A pairing code's ticket, if it shows one: whether it lets this key in is decided once the key is proved.
    const ticket = token(request, url);
    const joining = ticket ? await sha256(ticket) : null;
    if (joining && !(this.pairing(joining)?.until > now)) return refuse(403, "that pairing code has expired");
    // Who's let in is said only after the challenge, the same for every key that isn't, so this tells no one who's on
    // the list; sockets that haven't answered are few, from one place and in all.
    const from = place(request);
    let unproven = 0;
    let here = 0;
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
    }
    if (here >= UNPROVEN_PER_PLACE || unproven >= UNPROVEN) return refuse(429, "too many sockets that haven't proved their key");
    const [client, server] = Object.values(new WebSocketPair());
    this.ctx.acceptWebSocket(server);
    const made = challenge();
    server.serializeAttachment({ challenge: hex(made.slice(0, 16)), at: now, from, joining });
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
    // A pairing code's computer, until it's added: only while its code works, and only to the device that made it.
    if (said.pairing && now > said.until) return this.end(said.id, "its pairing code expired");
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

  /// A socket's answer to its challenge: it's let in if it proves a key on the list, or the key its pairing code let in.
  async prove(ws, said, bytes) {
    let id;
    try {
      id = hex(verify(unhex(said.challenge), bytes));
    } catch {
      id = null;
    }
    let attachment = id && this.key(id) ? { id } : null;
    if (id && !attachment && said.joining) {
      const pairing = this.pairing(said.joining);
      if (pairing && pairing.until > Date.now() && [null, id].includes(pairing.key)) {
        // A pairing code lets in one computer: this one, again if it reconnects, until it expires or it's added.
        this.sql.exec("UPDATE pairings SET key = ? WHERE hash = ?", id, pairing.hash);
        attachment = { id, pairing: pairing.hash, until: pairing.until, to: pairing.by };
        await this.wake(pairing.until);
      }
    }
    if (!attachment) {
      ws.serializeAttachment({});
      ws.send(deny("the relay takes only your One App's devices"));
      return ws.close(1008, "not allowed");
    }
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

  /// An alarm at `until`, if none is sooner, to close pairing codes' computers once their codes expire.
  async wake(until) {
    const at = await this.ctx.storage.getAlarm();
    if (!at || at > until) await this.ctx.storage.setAlarm(until);
  }

  async alarm() {
    const now = Date.now();
    let next = null;
    for (const ws of this.ctx.getWebSockets()) {
      const said = ws.deserializeAttachment();
      if (!said?.pairing) continue;
      if (said.until <= now) this.end(said.id, "its pairing code expired");
      else next = Math.min(next ?? said.until, said.until);
    }
    this.sql.exec("DELETE FROM pairings WHERE until <= ?", now);
    if (next) await this.ctx.storage.setAlarm(next);
  }

  /// What a device told the relay, its signature checked by the Worker: → why it was refused, or nothing.
  async tell(told) {
    const now = Date.now();
    const by = this.key(told.by);
    if (!by) return "only your One App's devices tell the relay";
    this.sql.exec("DELETE FROM tells WHERE at < ? OR at > ?", now - 2 * FRESH, now + 2 * FRESH);
    if (this.sql.exec("SELECT 1 FROM tells WHERE by = ? AND at = ?", told.by, told.at).toArray().length) return "that tell was taken already";
    this.sql.exec("INSERT INTO tells VALUES (?, ?)", told.by, told.at);
    const admin = by.admin === 1;
    if (told.add !== undefined) {
      if (!isKey(told.add)) return "add is a key, in hex";
      const wants = told.admin === true;
      const it = this.key(told.add);
      if (wants && !admin) return "only an admin's device adds an admin's";
      if (it) {
        if (!admin) return "that key is in already: only an admin's device changes it";
        if (it.admin === 1 && !wants && this.admins() === 1) return "that's the last admin's key";
        this.sql.exec("UPDATE keys SET admin = ? WHERE key = ?", wants ? 1 : 0, told.add);
      } else {
        this.sql.exec("INSERT INTO keys VALUES (?, ?, ?)", told.add, told.by, wants ? 1 : 0);
      }
      // A pairing code's computer, added, is one of the install's: it reaches every endpoint now.
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
      for (const { key: joined } of this.sql.exec("SELECT key FROM pairings WHERE by = ? AND key IS NOT NULL", k).toArray()) this.end(joined, "taken out");
      this.sql.exec("DELETE FROM pairings WHERE by = ?", k);
      this.end(k, "taken out");
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
