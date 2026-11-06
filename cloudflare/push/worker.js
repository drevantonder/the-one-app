// The push Worker: how a device reaches another that isn't looking, in every network from its start. It carries notes,
// each sealed to the one device it's for (that device's seal key, as everything is sealed to devices), to Apple's,
// Google's, Microsoft's or a browser's push service, which wake that device; the device opens the note and shows it.
// The Worker, and every push service on the way, carry locked bytes: none of them reads a note.
//
// A device that can get pushes registers its address here, by a request signed with its key (`signedRequest` in
// shared/crypto.js, domain `one/push`, route `POST /push/register`, taken once), only if the relay lists it as one of
// the network's endpoints: `{ via, address, env?, topic?, keys?, seal }`, `via` one of apns (address: the APNs token,
// hex; topic: the app's bundle id, one of `APNS_TOPICS`; env: sandbox or production), fcm (its FCM token), wns (its
// channel URI) or webpush (its subscription's endpoint, with `keys: { p256dh, auth }`), and `seal`, its seal key (hex),
// which only the network's own Workers seal to (below); and `user`, its person, as who's who lists it, once the device
// knows. `POST /push/unregister` forgets it.
//
// One of the network's endpoints sends with `POST /push/send`, signed the same way: `{ notes: [{ to, note, quiet? }] }`,
// each `note` base64url bytes sealed to `to`'s seal key by the sender, which takes that key from the
// network's signed entries, never from here. It says, of each `to`, `sent`, `none` (it registered no address), `gone`
// (its address stopped working, or the relay no longer lists it, so it's forgotten) or `failed`. `quiet` wakes the
// device without a visible alert, where the platform can (APNs' background push); a browser always shows one.
//
// The network's own Workers send through the `Sender` entrypoint, by binding: sign-in, for a device asking to join, whose
// note it seals here to the seal key the device registered (`{ to, plain }`), or sends sealed (`{ to, note }`); and asks
// which devices registered as a person's (`theirs`), whom it pushes about their devices joining. That is only where to
// send: a device names its person unchecked, so one that lies gets notes sealed to itself, which let no one in.
//
// Sends are limited: each sender `SENDS` requests a minute, each device `NOTES` notes a minute.
//
// What it holds is the network's push credentials (`APNS_*`, `FCM_ACCOUNT`, `WNS_*`, `VAPID_KEY`), Workers' secrets, and
// each device's address, in its Durable Object.

import { DurableObject, WorkerEntrypoint } from "cloudflare:workers";
import { b64u, checkRequest, sealTo, hex, rolesOf } from "../shared/crypto.js";
import { PUSH } from "../shared/protocol.js";
import { apns, fcm, isWebPushEndpoint, isWnsChannel, vapidKey, webPush, wns } from "./senders.js";

const VIAS = ["apns", "fcm", "wns", "webpush"];
/// The most a sender sends a minute, the most notes one device gets a minute, the most in one send, and the most a
/// sealed note may be: it has to fit in a push, 4 KB at Apple and in Web Push.
const SENDS = 120;
const NOTES = 30;
const MOST = 64;
const NOTE_BYTES = 2_600;
const MINUTE = 60_000;

const isKey = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
const fail = (status, error) => {
  throw Object.assign(new Error(error), { status });
};
const json = (v, status = 200) => Response.json(v, { status, headers: { "access-control-allow-origin": "*" } });

const push = (env) => env.PUSH.get(env.PUSH.idFromName("push"));

export default class extends WorkerEntrypoint {
  async fetch(request) {
    const url = new URL(request.url);
    if (request.method === "OPTIONS") return new Response(null, { headers: { "access-control-allow-origin": "*", "access-control-allow-methods": "GET, POST", "access-control-allow-headers": "content-type" } });
    // What a browser subscribes with: the network's VAPID public key. And which services this network can push through.
    if (url.pathname === "/push/key" && request.method === "GET") {
      const vapid = this.env.VAPID_KEY ? b64u.encode((await vapidKey(this.env.VAPID_KEY)).public) : null;
      return json({ vapid, via: senders(this.env) });
    }
    const route = { "/push/register": "register", "/push/unregister": "unregister", "/push/send": "send", "/push/devices": "devices" }[url.pathname];
    if (!route || request.method !== "POST") return json({ error: "not here" }, 404);
    const body = await request.json().catch(() => null);
    if (!body) return json({ error: "the body isn't JSON" }, 400);
    // Errors lose all but their message between objects, so it answers with its status.
    const { status, said } = await push(this.env).asked(route, `POST ${url.pathname}`, body);
    return json(said, status);
  }
}

export class Sender extends WorkerEntrypoint {
  /// `notes`: `[{ to, note }]` sealed already, or `[{ to, plain }]`, sealed here to the seal key `to` registered; each
  /// may be `quiet`. → what happened to each, by key.
  send(notes) {
    return push(this.env).send("workers", notes, null);
  }

  theirs(user, most = 20) {
    return push(this.env).theirs(user, most);
  }
}

function senders(env) {
  return VIAS.filter((via) => ({ apns: env.APNS_KEY, fcm: env.FCM_ACCOUNT, wns: env.WNS_CLIENT_SECRET, webpush: env.VAPID_KEY })[via]);
}

export class Push extends DurableObject {
  constructor(ctx, env) {
    super(ctx, env);
    const sql = ctx.storage.sql;
    sql.exec("create table if not exists devices (key text primary key, via text not null, address text not null, env text not null default '', topic text not null default '', keys text, seal text not null, at integer not null, user text)");
    sql.exec("create table if not exists signed (sig text primary key, at integer not null)");
    this.counted = new Map();
  }

  async asked(what, route, body) {
    try {
      return { status: 200, said: await this.answer(what, route, body) };
    } catch (err) {
      return { status: err.status || 500, said: { error: err.message } };
    }
  }

  async answer(what, route, body) {
    const { by, fields } = await checkRequest(PUSH, body).catch((err) => fail(403, `That request isn't signed as it should be: ${err.message}.`));
    if (fields.route !== route) fail(403, `That request was signed for ${fields.route}, not ${route}.`);
    const sql = this.ctx.storage.sql;
    sql.exec("delete from signed where at <= ?", Date.now() - 2 * MINUTE);
    if (sql.exec("select 1 from signed where sig = ?", body.sig).toArray().length) fail(403, "That request was sent already.");
    sql.exec("insert into signed (sig, at) values (?, ?)", body.sig, Date.now());
    const allowed = await this.env.RELAY.allowed(by);
    if (!allowed) fail(403, "That device isn't one of the network's.");
    const node = rolesOf(allowed.role).includes("compute");
    if (what === "register") return this.register(by, fields);
    if (what === "unregister") return this.unregister(by);
    if (what === "devices") return this.devices(node, fields);
    return this.send(by, fields.notes);
  }

  register(by, { via, address, env = "", topic = "", keys, seal, user }) {
    if (!VIAS.includes(via)) fail(400, `via is one of ${VIAS.join(", ")}`);
    if (typeof address !== "string" || !address || address.length > 1024) fail(400, "say its address");
    if (!isKey(seal)) fail(400, "seal is its seal key, hex");
    if (user != null && !(typeof user === "string" && /^[\w-]{1,64}$/.test(user))) fail(400, "user is its person's id");
    if (via === "apns") {
      if (!/^[0-9a-f]{16,200}$/.test(address)) fail(400, "an APNs token is hex");
      if (!["sandbox", "production"].includes(env)) fail(400, "env is sandbox or production");
      if (!JSON.parse(this.env.APNS_TOPICS || "[]").includes(topic)) fail(400, "topic is the app's bundle id");
    }
    if (via === "wns" && !isWnsChannel(address)) fail(400, "a WNS channel is at notify.windows.com");
    if (via === "webpush") {
      if (!isWebPushEndpoint(address)) fail(400, "that endpoint isn't a browser's push service");
      const ok = (s, n) => {
        try {
          return b64u.decode(s).length === n;
        } catch {
          return false;
        }
      };
      if (!keys || !ok(keys.p256dh, 65) || !ok(keys.auth, 16)) fail(400, "keys are the subscription's p256dh and auth");
    }
    const sql = this.ctx.storage.sql;
    sql.exec(
      "insert or replace into devices (key, via, address, env, topic, keys, seal, at, user) values (?, ?, ?, ?, ?, ?, ?, ?, ?)",
      by, via, address, env, topic, via === "webpush" ? JSON.stringify({ p256dh: keys.p256dh, auth: keys.auth }) : null, seal, Date.now(), user ?? null,
    );
    return { registered: true };
  }

  theirs(user, most) {
    return this.ctx.storage.sql.exec("select key from devices where user = ? order by at desc limit ?", String(user), most).toArray().map((r) => r.key);
  }

  unregister(by) {
    this.ctx.storage.sql.exec("delete from devices where key = ?", by);
    return { unregistered: true };
  }

  devices(node, { user, keys = [] }) {
    if (!node) fail(403, "Only a node asks which devices registered.");
    const sql = this.ctx.storage.sql;
    const registered = keys.filter(isKey).flatMap((key) => sql.exec("select key, via, env, at from devices where key = ?", key).toArray());
    return { registered };
  }

  count(who, most) {
    const now = Date.now();
    const c = this.counted.get(who);
    if (!c || c.since <= now - MINUTE) {
      if (this.counted.size > 10_000) this.counted.clear();
      this.counted.set(who, { since: now, n: 1 });
      return true;
    }
    return ++c.n <= most;
  }

  async send(by, notes = []) {
    if (!Array.isArray(notes) || notes.length > MOST) fail(400, `notes is a list of at most ${MOST}`);
    if (!this.count(`sender:${by}`, SENDS)) fail(429, "That device sent too many pushes this minute. Try again in a minute.");
    const out = {};
    const sql = this.ctx.storage.sql;
    await Promise.all(
      notes.map(async ({ to, note, plain, quiet }) => {
        if (!isKey(to)) return;
        if (!this.count(`to:${to}`, NOTES)) return void (out[to] = { failed: "too many to that device this minute" });
        const device = sql.exec("select * from devices where key = ?", to).toArray()[0];
        if (!device) return void (out[to] = { none: true });
        // Only to the network's devices: one the relay no longer lists is forgotten.
        if (!(await this.env.RELAY.allowed(to))) {
          sql.exec("delete from devices where key = ?", to);
          return void (out[to] = { gone: true });
        }
        let bytes;
        try {
          bytes = plain !== undefined && by === "workers" ? await sealTo(hex.decode(device.seal), JSON.stringify(plain)) : b64u.decode(note);
        } catch {
          return void (out[to] = { failed: "that note isn't base64url" });
        }
        if (bytes.length < 48 || bytes.length > NOTE_BYTES) return void (out[to] = { failed: `a note is sealed, and at most ${NOTE_BYTES} bytes` });
        const said = await this.deliver(device, { note: bytes, quiet: !!quiet }).catch((err) => ({ failed: err.message }));
        if (said.gone) sql.exec("delete from devices where key = ? and address = ?", to, device.address);
        out[to] = said.sent ? { sent: true } : said.gone ? { gone: true } : { failed: said.failed };
      }),
    );
    return { to: out };
  }

  deliver(device, what) {
    const env = this.env;
    if (device.via === "apns") return apns(fetch, { team: env.APNS_TEAM_ID, key_id: env.APNS_KEY_ID, key: env.APNS_KEY }, device, { ...what, name: env.NAME });
    if (device.via === "fcm") return fcm(fetch, env.FCM_ACCOUNT ? JSON.parse(env.FCM_ACCOUNT) : null, device, what);
    if (device.via === "wns") return wns(fetch, { tenant: env.WNS_TENANT_ID, client_id: env.WNS_CLIENT_ID, client_secret: env.WNS_CLIENT_SECRET }, device, what);
    return this.vapid().then((vapid) => webPush(fetch, vapid, { address: device.address, keys: JSON.parse(device.keys) }, what));
  }

  async vapid() {
    if (!this.env.VAPID_KEY) return null;
    this.vapidKey ||= vapidKey(this.env.VAPID_KEY).then((k) => ({ ...k, subject: this.env.ADDRESS }));
    return this.vapidKey;
  }
}
