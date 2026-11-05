// This browser as a device of its person (ARCHITECTURE.md): its own Ed25519 key, which signs, and
// X25519 key, which what's sealed to it is sealed to, both made here, not extractable, kept in IndexedDB by person. It
// gets in when one of their devices approves it; then it opens its scopes' epochs, sealed to it, and from them the keys
// a call needs, which it hands the node with the call (`x-one-keys`). What it opens stays in this page's memory, as keys
// that can't be read back, and goes with it. What changes who can do what it signs itself (members.js).
//
// Who's who it takes only from the install's own signed entries, checked by its client's WebAssembly (`checked`), never
// from what a node says (/v1/state): which seal key a device has, who opens a scope, which hash an epoch's key must
// have, which nodes may be handed a scope's keys, and who may approve or remove what. Each is answered there, from the
// scope database and its permissions (client/src/members.rs, `Who`), and only looked up here. A hacked node holds no
// keys, and gets none this way.

import { askedFor, b64u, databaseKey, ed25519Public, epochHash, hex, hkdfKey, inviteProof, joinCode, oldestNeeded, openAppKey, openSealed, passkeyKeys, random, sealAppKey, sealHandoff, sealTo, sha256, signEntry, signedRequest, twinPublic, untilTaken, x25519Public } from "../shared/crypto.js";
import { del, get, put } from "./kept.js";
import { members } from "./members.js";

const text = new TextEncoder();

/// How long standing work lasts (src/node/standing.rs), unless its person renews it as they use the app.
const GRANT_DAYS = 30, DAY = 86_400_000, HOUR = 3_600_000;

// MARK: Kept in this browser

const raw = async (key) => new Uint8Array(await crypto.subtle.exportKey("raw", key));

/// This browser's keys for `user`, made the first time: `{ key, seal }` (hex) to list it by, and the private halves,
/// which can't be read back.
export async function browserKeys(user) {
  let d = await get(`device:${user}`);
  if (!d) {
    // The keys its sign-in was for, made before it knew whose.
    d = (await get("device:")) || (await newKeys());
    await put(`device:${user}`, d);
    await del("device:");
  }
  return { key: d.key, seal: d.sealKey, sign: d.sign, opens: { secret: d.seal, public: hex.decode(d.sealKey) } };
}

async function newKeys() {
  const sign = await crypto.subtle.generateKey({ name: "Ed25519" }, false, ["sign", "verify"]);
  const seal = await crypto.subtle.generateKey({ name: "X25519" }, false, ["deriveBits"]);
  return { sign: sign.privateKey, key: hex.encode(await raw(sign.publicKey)), seal: seal.privateKey, sealKey: hex.encode(await raw(seal.publicKey)) };
}

/// The device a sign-in in this browser is for (`join`, its key ‖ seal key): the keys the first person it signs in as
/// takes, made before it knows whose (`browserKeys`).
export async function signingInFor() {
  let d = await get("device:");
  if (!d) await put("device:", (d = await newKeys()));
  return d.key + d.sealKey;
}

/// A passkey as a device, from its PRF output (`keys::passkey_keys`).
export async function passkeyKeysOf(prf) {
  const pk = await passkeyKeys(prf);
  const seal = await x25519Public(pk.seal);
  return { key: hex.encode(await ed25519Public(pk.sign)), seal: hex.encode(seal), sign: pk.sign, opens: { secret: pk.seal, public: seal } };
}

/// What this browser asks auth to let it in with: its keys and name, and for someone new, proof of their invite.
export async function joinAsk(keys, name, invite) {
  return { key: keys.key, seal: keys.seal, name, ...(invite ? { proof: await inviteProof(text.encode(invite), hex.decode(keys.key), hex.decode(keys.seal)) } : {}) };
}

/// The code this browser shows to be approved: its key's, as 4 and 4.
export const codeOf = async (key, seal) => (await joinCode(hex.decode(key), hex.decode(seal))).replace(/^(.{4})/, "$1 ");

// MARK: Who opens what, as the install's signed entries say (`k`, one-client's `members::view`)

/// What a scope's epochs are sealed to: the client devices of the people who may use it, and nothing that only runs
/// calls.
const openers = (k, scope) => k.openers[scope] || [];

/// The scopes `user`'s devices open: everyone's, their own, and their groups'.
const scopesOf = (k, user) => k.people[user]?.uses || [];

/// Whether `user` may start a scope's new epoch: an admin of it (everyone is of their own).
const starts = (k, scope, user) => !!k.people[user]?.starts.includes(scope);

/// Whether a node runs each of `scopes`' calls, so it may be handed their keys.
const runsAll = (k, node, scopes) => !!k?.runs_for[node] && scopes.every((s) => k.runs_for[node].includes(s));

// MARK: The device

export class Device {
  /// `me`: this browser's keys; `user`: its person; `state()`: the node's /v1/state as last read, for what it shows;
  /// `api(path, opts)`: a /v1 request as this person; `bearer(path, init)`: one to the address with the access token;
  /// `replay(user, given)`: `global`'s entries this browser kept and those `given` (a pack), checked and replayed by
  /// one-client (net.global).
  constructor(me, { user, state, api, bearer, replay }) {
    Object.assign(this, { me, user, state, api, bearer, replay });
    /// The members as last checked (`checked`): what it seals and hands by.
    this.known = null;
    /// Each scope's epochs this browser opened: n → an HKDF key, its bytes gone.
    this.epochs = new Map();
    /// Each app's key in a scope by epoch, the same: `scope app` → n → key.
    this.appKeys = new Map();
    /// Databases' keys, by `scope app database n`: bytes, as a node is handed them.
    this.dbKeys = new Map();
    /// What each kind of request needed the last time (`one.needs`, which names no key).
    this.needs = JSON.parse(localStorage.getItem("one.needs") || "{}");
    /// The grants this browser is renewing now.
    this.renewing = new Set();
    /// When `renewGrants` last checked the entries again.
    this.renewChecked = 0;
    /// What's being fetched now, by what it is, for everything that wants it at once to wait on the one fetch.
    this.fetching = new Map();
    /// The 428 each kind of request is learning from now, by its route: `{ asked, done }`.
    this.learning = new Map();
  }

  /// `fetch()`, once for everyone who asks for `id` while it's under way.
  once(id, fetch) {
    if (!this.fetching.has(id)) this.fetching.set(id, fetch().finally(() => this.fetching.delete(id)));
    return this.fetching.get(id);
  }

  /// The members as the install's signed entries say them, checked here: as last checked, or with what's new from a
  /// node if `fresh` (before it changes who opens what) or never. The first time, the install's first entry is kept,
  /// and an install that starts with any other refused. While no node gives the entries (the serverless node doesn't),
  /// it goes on with those it checked last, unless it's `fresh`.
  async checked(fresh = false) {
    if (fresh || !this.known) {
      const kept = await get(`global:${this.user}`);
      const r = await this.api(`/v1/entries/global?have=${kept?.have || ""}`, { raw: true }).catch((err) => {
        if (fresh || !kept) throw err;
        return null;
      });
      // What an older release kept, without all its answers (each person's `keeps` came last), is replayed again.
      const given = r && r.status !== 204 ? new Uint8Array(await r.arrayBuffer()) : undefined;
      const k = !given && kept?.members?.people?.[this.user]?.keeps ? kept.members : await this.replay(this.user, given);
      if (!k.people[this.user]) throw new Error("Your One App doesn't list you yet, so this device didn't use it.");
      this.known = k;
    }
    return this.known;
  }

  /// The members as last checked, kept in this browser, to start with at once.
  async load() {
    const kept = (await get(`global:${this.user}`))?.members;
    this.known ||= kept?.people?.[this.user]?.keeps ? kept : null;
  }

  // Epochs and keys.

  /// Opens epoch `n`'s key sealed to `as` from what the node gave, if its hash is the one its epoch entry gave.
  async openEpoch(scope, n, sealed, as = this.me) {
    const key = await openSealed(as.opens, b64u.decode(sealed));
    const hash = await epochHash(key);
    const made = async (fresh) => (await this.checked(fresh)).epochs[scope]?.hashes?.[n];
    if (hash !== ((await made(false)) ?? (await made(true)))) throw new Error("A key for this didn't check out, so it wasn't used.");
    return key;
  }

  /// A scope's epochs sealed to `as`, as raw keys, opened now (for sealing them to another device), by number.
  async epochBytes(scope, as = this.me, note = noting) {
    const { epoch, sealed } = await note({ step: "keys", of: "epochs", scope: kindOf(scope) }, () => this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&to=${as.key}`));
    const out = new Map();
    for (const [n, s] of Object.entries(sealed || {})) out.set(Number(n), await this.openEpoch(scope, Number(n), s, as));
    return { newest: epoch || 0, keys: out };
  }

  /// Epoch `n` of a scope, as a key kept in memory: opened once (all its epochs at once, however many ask), or made if
  /// it's the scope's first and theirs to make.
  async epoch(scope, n, note = noting) {
    const had = this.epochs.get(scope)?.get(n);
    if (had) return had;
    const newest = await this.once(`epochs ${scope}`, async () => {
      const { newest, keys } = await this.epochBytes(scope, this.me, note);
      const all = this.epochs.get(scope) || new Map();
      for (const [m, key] of keys) all.set(m, await hkdfKey(key));
      this.epochs.set(scope, all);
      return newest;
    });
    // Its first made once, however many ask for it at once.
    const made = () => this.epochs.get(scope)?.has(n);
    if (!made() && n === 1 && newest === 0) await this.once(`first ${scope}`, async () => made() || (starts(await this.checked(true), scope, this.user) && (await this.newEpoch(scope, 1))));
    if (!made()) throw new Error("This device can't open that yet.");
    return this.epochs.get(scope).get(n);
  }

  /// Starts a scope's epoch `n`: a random key, written first as an `epoch` entry naming its hash (which, first in
  /// causal order, makes it), then sealed to the recovery key and each client device of the people who may open it but
  /// those in `except` (keys being taken out), before it's used.
  async newEpoch(scope, n, { except = [] } = {}) {
    const k = await this.checked(true);
    if (!k.recovery) throw new Error("Your One App has no recovery key to keep its keys with.");
    const key = random(32);
    const hash = await epochHash(key);
    const to = openers(k, scope).filter((e) => !except.includes(e.key));
    if (!to.some((e) => e.key === this.me.key) && !except.includes(this.me.key)) to.push({ key: this.me.key, seal: this.me.seal });
    await this.write("global", members.epoch({ scope, n, to: to.map((e) => e.key), hash }));
    if ((await this.checked(true)).epochs[scope]?.hashes?.[n] !== hash) throw new Error("Another device made new keys for this just now. Try again.");
    await this.api("/v1/keys", { method: "POST", body: { scope, n, to: "recovery", sealed: b64u.encode(await sealTo(twinPublic(hex.decode(k.recovery)), key)) } });
    for (const e of to) await this.api("/v1/keys", { method: "POST", body: { scope, n, to: e.key, sealed: b64u.encode(await sealTo(hex.decode(e.seal), key)) } });
    const all = this.epochs.get(scope) || new Map();
    all.set(n, await hkdfKey(key));
    this.epochs.set(scope, all);
  }

  /// This person's own things, once their device is in: where they run besides their own machines, if their devices
  /// never said (the machines that run everyone's, chosen without asking), and their own first key, if it isn't made.
  async settleOwn() {
    const mine = `user:${this.user}`;
    let k = await this.checked();
    if (k.people[this.user].runs_until_chosen.length || !k.epochs[mine]) k = await this.checked(true);
    const on = k.people[this.user].runs_until_chosen;
    if (on.length) await this.write("global", members.runs({ on }));
    if (!k.epochs[mine]) await this.newEpoch(mine, 1);
  }

  /// An app's key in epoch `n` of a scope: the one sealed under the epoch, or a new one if it has none yet.
  async appKey(scope, app, n, note = noting) {
    const id = `${scope} ${app}`;
    const had = this.appKeys.get(id)?.get(n);
    if (had) return had;
    const key = await this.once(`app ${id} ${n}`, async () => hkdfKey(await this.appKeyBytes(scope, app, n, note)));
    if (!this.appKeys.has(id)) this.appKeys.set(id, new Map());
    this.appKeys.get(id).set(n, key);
    return key;
  }

  /// An app's key in epoch `n` of a scope as its bytes, opened now (for sealing it to a device that keeps the app
  /// running): the one sealed under the epoch, or a new one if it has none yet. Its epochs' are asked for once, however
  /// many ask at once.
  async appKeyBytes(scope, app, n, note = noting) {
    const [epoch, all] = await Promise.all([
      this.epoch(scope, n, note),
      this.once(`sealed ${scope} ${app}`, () => note({ step: "keys", of: app, scope: kindOf(scope) }, () => this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&app=${encodeURIComponent(app)}`))),
    ]);
    let sealed = all.sealed?.[n];
    if (!sealed) {
      const made = random(32);
      sealed = (await this.api("/v1/keys", { method: "POST", body: { scope, n, app, sealed: b64u.encode(await sealAppKey(epoch, app, made)) } })).sealed;
      made.fill(0);
    }
    return openAppKey(epoch, app, b64u.decode(sealed));
  }

  // Standing work.

  /// Keeps `app` running in `scope` on one of the install's devices (`endpoint`) for `days`, 30 at most (0 stops it):
  /// the app's key in each epoch of the scope, sealed to it first, then the `grant` naming each one's hash. Stopping
  /// it starts a new epoch of the scope, which the device isn't sealed, so the keys it held open nothing written after.
  async grant({ endpoint, app, scope, days = GRANT_DAYS }) {
    const k = await this.checked();
    const e = k.devices.find((d) => d.key === endpoint && d.user);
    if (!e) throw new Error("That device isn’t one of yours.");
    const until = days === 0 ? 0 : Date.now() + Math.min(days, GRANT_DAYS) * DAY;
    const keys = {};
    if (until) {
      const made = Object.keys(k.epochs[scope]?.hashes || {}).map(Number);
      for (const n of made.length ? made : [1]) {
        const key = await this.appKeyBytes(scope, app, n);
        await this.api("/v1/keys", { method: "POST", body: { scope, n, app, to: endpoint, sealed: b64u.encode(await sealTo(hex.decode(e.seal), key)) } });
        keys[n] = await epochHash(key);
        key.fill(0);
      }
    }
    await this.write("global", members.grant({ endpoint, scope, app, until, keys }));
    // Checked again, so what it renews next goes by this.
    await this.checked(true);
    if (!until) await this.renew([scope], []);
  }

  /// Renews, in the background, this person's grants of `app` as they use it: once a day at most, and at once when
  /// the scope has an epoch a grant's keys don't reach. One of theirs that ran out is stopped, as Stop does. Only those
  /// the install's entries hold, as checked here (`keeps`), never those a node lists: a hacked one could list itself.
  async renewGrants(app) {
    // Checked again first, at most once an hour, so it renews what its person's other devices granted too.
    if (!(this.renewChecked > Date.now() - HOUR)) {
      this.renewChecked = Date.now();
      await this.checked(true).catch(() => {});
    }
    const now = Date.now();
    for (const g of this.known?.people[this.user]?.keeps || []) {
      if (g.until && g.until <= now) {
        const id = `${g.endpoint} ${g.scope} ${g.app}`;
        if (this.renewing.has(id)) continue;
        this.renewing.add(id);
        this.grant({ endpoint: g.endpoint, app: g.app, scope: g.scope, days: 0 }).catch(() => {}).finally(() => this.renewing.delete(id));
        continue;
      }
      const newest = this.known?.epochs[g.scope]?.n || 0;
      const reaches = Math.max(0, ...Object.keys(g.keys || {}).map(Number));
      if (g.app !== app || g.until <= now || (g.until > now + (GRANT_DAYS - 1) * DAY && reaches >= newest)) continue;
      const id = `${g.endpoint} ${g.scope} ${g.app}`;
      if (this.renewing.has(id)) continue;
      this.renewing.add(id);
      this.grant({ endpoint: g.endpoint, app: g.app, scope: g.scope }).catch(() => {}).finally(() => this.renewing.delete(id));
    }
  }

  /// What `need`'s keys are opened from that this browser hasn't opened yet, asked for all at once: each scope's epochs
  /// sealed to it, and each app's keys in them, one request each. Only what's there is opened; an app's key that isn't
  /// is left for `appKey`, which makes it.
  async fetchKeys(need, note = noting) {
    const scopes = [...new Set(need.filter((n) => !this.epochs.get(n.scope)?.has(n.epoch)).map((n) => n.scope))];
    const apps = [...new Set(need.filter((n) => !this.appKeys.get(`${n.scope} ${n.app}`)?.has(n.epoch)).map((n) => `${n.scope} ${n.app}`))];
    const sealed = (id) => {
      const [scope, app] = id.split(" ");
      return this.once(`sealed ${id}`, () => note({ step: "keys", of: app, scope: kindOf(scope) }, () => this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&app=${encodeURIComponent(app)}`)));
    };
    const [, got] = await Promise.all([
      Promise.all(scopes.map((scope) => this.epoch(scope, Math.max(...need.filter((n) => n.scope === scope).map((n) => n.epoch)), note).catch(() => {}))),
      Promise.all(apps.map((id) => sealed(id).catch(() => null))),
    ]);
    for (const [i, id] of apps.entries()) {
      const [scope, app] = id.split(" ");
      for (const [m, s] of Object.entries(got[i]?.sealed || {})) {
        const epoch = this.epochs.get(scope)?.get(Number(m));
        if (!epoch || this.appKeys.get(id)?.has(Number(m))) continue;
        const key = await Promise.resolve().then(() => openAppKey(epoch, app, b64u.decode(s))).then(hkdfKey, () => null);
        if (!key) continue;
        if (!this.appKeys.has(id)) this.appKeys.set(id, new Map());
        this.appKeys.get(id).set(Number(m), key);
      }
    }
  }

  async databaseKey({ scope, app, database, epoch }, note = noting) {
    const id = `${scope} ${app} ${database} ${epoch}`;
    if (!this.dbKeys.has(id)) this.dbKeys.set(id, await databaseKey(await this.appKey(scope, app, epoch, note), database));
    return this.dbKeys.get(id);
  }

  // Handing a call its keys.

  /// What a request is, to remember what it needed: an app's call by its app, anything else by its path.
  static route(path, body) {
    const at = new URL(path, location.origin).pathname;
    return at === "/v1/call" ? `call ${body?.app || ""}` : at.replace(/^\/v1\/screens\/([^/]+)\/.*/, "/v1/screens/$1");
  }

  /// The `x-one-keys` a request goes with, to `node`, the one it's sent to: what its kind of request needed the last
  /// time, of the scopes the install's entries say that node runs, sealed to the seal key they list for it, for this
  /// browser's key, until `until`. No other node can open it; null if it runs none of them.
  /// A request sent while another of its kind learns what it needs waits for that first, and goes with it.
  async handoff(route, until, node, note = noting) {
    await this.learning.get(route)?.done.catch(() => {});
    const need = (this.needs[route]?.need || []).filter((n) => runsAll(this.known, node, [n.scope]));
    const seal = this.known?.devices.find((d) => d.key === node)?.seal;
    if (!need.length || !seal) return null;
    await this.fetchKeys(need, note);
    const keys = await Promise.all(need.map(async (n) => ({ ...n, key: b64u.encode(await this.databaseKey(n, note)) })));
    return sealHandoff(hex.decode(seal), { for: this.me.key, until, keys });
  }

  /// A node's 428, believed only from `node`, the one that answered it as its connection proved: it must ask only what
  /// `askedFor` lets it, be listed by the install's entries, and run every scope it needs keys of, as they say. Of each, only epochs from the one its newest copy is under are
  /// handed (`oldestNeeded`): older ones open only its history. What it needed is all this kind of request hands next
  /// time, to whichever node it's sent to, of the scopes that one runs. Throws if it asks for more, or this browser can't
  /// open it. Requests of a kind that get the
  /// same 428 at once learn from it once, and one sent before another learned goes again with what that learned. `note(step, fetch)` times each thing it asks for (developer mode's traces).
  learn(route, answer, node, note = noting, sentWith = this.needs[route]) {
    // Another of its kind learned since this one was sent (`sentWith`, what it went with): it goes again with that.
    if (this.needs[route] !== sentWith) return Promise.resolve();
    const asked = JSON.stringify([node, answer]);
    const now = this.learning.get(route);
    if (now?.asked === asked) return now.done;
    const done = this.learnNow(route, answer, node, note);
    const it = { asked, done };
    this.learning.set(route, it);
    done.catch(() => {}).finally(() => this.learning.get(route) === it && this.learning.delete(route));
    return done;
  }

  async learnNow(route, answer, node, note) {
    const need = askedFor(route, node, answer);
    if (!need) throw new Error("A device asked for keys that request doesn’t use, so none were given.");
    const scopes = need.map((n) => n.scope);
    let k = await this.checked();
    if (!runsAll(k, node, scopes)) k = await note({ step: "members" }, () => this.checked(true));
    if (!runsAll(k, node, scopes)) throw new Error("A device that doesn’t run that asked for its keys, so none were given.");
    if (!k.devices.some((d) => d.key === node && d.seal)) throw new Error("A device that isn’t one of yours asked for keys, so none were given.");
    // The keys' own fetching goes at the same time: it only opens what's there, whichever epochs are handed.
    const [hints] = await Promise.all([note({ step: "newest", databases: new Set(need.map(prefixOf)).size }, () => this.hints(need)), this.fetchKeys(need, note)]);
    const handed = hints ? need.filter((n) => n.epoch >= oldestNeeded(hints[prefixOf(n)] ?? null, k.epochs[n.scope]?.n || 0)) : need;
    if (!handed.length) throw new Error("A device asked for older keys than it needs, so none were given.");
    await Promise.all(handed.map((n) => this.databaseKey(n, note)));
    this.needs[route] = { need: handed };
    try { localStorage.setItem("one.needs", JSON.stringify(this.needs)); } catch {}
  }

  /// Each database of `needs`' newest hint, by its prefix in the bucket (`prefixOf`): its text, or null if it has
  /// none, as the URL signer reads it, which a browser can't itself. Null if the signer doesn't say (an older one).
  async hints(needs) {
    const newest = [...new Set(needs.map(prefixOf))];
    const r = await fetch("/urls", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await this.signed("one/urls", { newest })) });
    const out = await r.json().catch(() => ({}));
    if (!r.ok) throw new Error(out.error || "Your One App's storage didn't answer. Try again.");
    return out.newest || null;
  }

  // Signing.

  /// Signs an entry of `content` in `log` as `as` (this browser, or a passkey), after what the node has, and writes it.
  /// `content` may be a function of the entry's seq, for one that says which entry it is.
  async write(log, content, as = this.me) {
    for (let tries = 0; ; tries++) {
      const heads = await this.api(`/v1/entries?scope=${encodeURIComponent(log)}`);
      const last = heads.logs?.[as.key];
      const fields = { log, seq: last ? last.seq + 1 : 0, backlink: last?.hash ?? null, previous: (heads.previous || []).filter((h) => h !== last?.hash), time: Date.now(), user: this.user, epoch: heads.epoch };
      const e = await signEntry(as.sign, hex.decode(as.key), fields, JSON.stringify(typeof content === "function" ? content(fields.seq) : content));
      try {
        return await this.api("/v1/entries", { method: "POST", body: [{ header: b64u.encode(e.header), signature: b64u.encode(e.signature), body: b64u.encode(e.body) }] });
      } catch (err) {
        // Another of the person's writes went first: once more, after it.
        if (tries || !/already|follow/.test(err.message)) throw err;
      }
    }
  }

  /// A request to `path` signed for `domain` by `as`, with `fields`.
  signed(domain, fields, as = this.me) {
    return signedRequest(as.sign, hex.decode(as.key), domain, fields);
  }

  /// Tells the relay to let a key in (`{ add, admin? }`) or take one out (`{ remove }`), signed by `as`.
  async tell(fields, as = this.me) {
    const said = await untilTaken(async (at) => {
      const r = await fetch("/relay/tell", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await signedRequest(as.sign, hex.decode(as.key), "one/relay", fields, at)) });
      return r.ok ? "" : (await r.json().catch(() => null))?.error || "Your One App's network didn't take that.";
    });
    if (said) throw new Error(said);
  }

  /// When each of the install's devices was last let onto its network, as the relay says: `{ key: ms }`, or none from a
  /// relay that doesn't say.
  async seen() {
    const r = await fetch("/relay/seen", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await this.signed("one/relay", { seen: true })) });
    return r.ok ? (await r.json()).seen || {} : {};
  }

  /// One of auth's routes, signed by this browser: a GET's or DELETE's request in `x-one-request`, `query` after it.
  async authSigned(method, path, fields = {}, query = "") {
    const request = await this.signed("one/auth", { route: `${method} ${path}`, ...fields });
    const init = method === "POST" ? { method, headers: { "content-type": "application/json" }, body: JSON.stringify(request) } : { method, headers: { "x-one-request": b64u.encode(text.encode(JSON.stringify(request))) } };
    return answer(await fetch(path + query, init));
  }

  // Devices joining.

  /// Devices waiting to be let in that this browser may approve: its person's own, and for an admin, those of people
  /// they invited from this browser, whose proof is right. Each with its code.
  async requests() {
    const own = (await answer(await this.bearer("/api/auth/devices/requests"))).requests || [];
    const invited = (await this.checked()).people[this.user]?.invites ? (await this.authSigned("GET", "/api/auth/devices/requests").catch(() => ({}))).requests || [] : [];
    const all = [...own, ...invited].filter((r, i, a) => !r.service && r.key !== this.me.key && a.findIndex((x) => x.key === r.key) === i);
    const ours = await Promise.all(all.map(async (r) => r.user === this.user || (await this.invitedHere(r))));
    return Promise.all(all.filter((_, i) => ours[i]).map(async (r) => ({ ...r, code: await codeOf(r.key, r.seal), mine: r.user === this.user })));
  }

  /// Whether a newcomer's device came from an invite made in this browser: its proof is that invite's.
  async invitedHere(r) {
    const invite = await get(`invite:${r.user}`);
    return !!invite && !!r.proof && r.proof === (await inviteProof(text.encode(invite.secret), hex.decode(r.key), hex.decode(r.seal)));
  }

  /// Turns down a device waiting to be let in: its request is forgotten.
  async refuse(r) {
    if (r.mine) await answer(await this.bearer(`/api/auth/devices/requests/${r.key}`, { method: "DELETE" }));
    else await this.authSigned("DELETE", `/api/auth/devices/requests/${r.key}`);
  }

  /// Lets in a device that asked: lists it (and for a newcomer, them), seals it the epochs it may open, lets it onto the
  /// network and clears its request.
  async approve(r, as = this.me, { signIn } = {}) {
    const k = await this.checked(true);
    const may = k.people[this.user];
    const known = !!k.people[r.user];
    // Listed already by an approval that didn't finish (its mark at auth failed): the rest is done again.
    const listed = k.devices.some((d) => d.key === r.key && d.user === r.user && d.seal === r.seal);
    const own = may.approves.includes(r.user);
    if (!own && known && !listed) throw new Error("Only their own devices approve one of theirs.");
    let newUser = null;
    if (!own) {
      if (!may.invites) throw new Error("Only an admin lets someone new in.");
      if (!(await this.invitedHere(r))) throw new Error("That device didn’t come from an invite made here.");
      newUser = { name: (await get(`invite:${r.user}`)).name || r.person, role: "member" };
    }
    if (!listed) await this.write("global", members.device({ key: r.key, seal: r.seal, user: r.user, name: r.name, newUser, device: r.device, signIn, place: r.place }), as);
    await this.sealTo(r, newUser ? ["global"] : scopesOf(k, r.user), as);
    await this.tell({ add: r.key, role: "client", ...(!newUser && may.invites ? { admin: true } : {}) }, as).catch((err) => {
      if (!(listed && /in already/.test(err.message))) throw err;
    });
    // Signed in again on the same device: its sign-in before goes, as signing out would have taken it, so its scopes
    // keep their keys.
    for (const old of r.device ? k.devices.filter((d) => d.user === r.user && d.device === r.device && d.key !== r.key && !d.passkey && (d.roles || []).includes("client")) : []) {
      await this.tell({ remove: old.key }, as).catch(() => {});
      const next = (await get(`global:${this.user}`))?.have.split(",").find((h) => h.startsWith(`${old.key}.`))?.split(".")[1];
      await this.write("global", members.removeDevice({ key: old.key, seq: next ? Number(next) - 1 : null }), as);
    }
    // Marked approved, for the device waiting on it to see, with the install it's in: the only one it then takes.
    const approved = `?approved&genesis=${(await get(`global:${this.user}`))?.root || ""}`;
    if (newUser) await this.authSigned("DELETE", `/api/auth/devices/requests/${r.key}`, {}, approved).catch(() => {});
    else await this.bearer(`/api/auth/devices/requests/${r.key}${approved}`, { method: "DELETE" }).catch(() => {});
  }

  /// Seals each epoch of `scopes` that `as` opens to a device (`{ key, seal }`), but those it has.
  async sealTo(device, scopes, as = this.me) {
    for (const scope of scopes) {
      const { keys } = await this.epochBytes(scope, as);
      for (const [n, key] of keys) {
        await this.api("/v1/keys", { method: "POST", body: { scope, n, to: device.key, sealed: b64u.encode(await sealTo(hex.decode(device.seal), key)) } }).catch((err) => {
          if (!/already/.test(err.message)) throw err;
        });
        key.fill(0);
      }
    }
  }

  /// Takes one of this person's devices out (or, for an admin, anyone's): off the network first, then unlisted, then
  /// each scope it opened that this person may move on gets a new epoch it doesn't have.
  async remove(key, as = this.me) {
    // Itself, it goes in the order `leave` says: its removal counts only if it's the last entry of its own that does.
    if (key === this.me.key) return this.leave();
    const k = await this.checked(true);
    const it = k.devices.find((e) => e.key === key);
    if (!it) throw new Error("That device isn’t listed.");
    if (!k.people[this.user].removes.includes(key)) throw new Error("Only its person or an admin takes a device out.");
    await this.tell({ remove: key }, as).catch((err) => {
      if (!/isn.t|not/.test(err.message)) throw err;
    });
    // The last of its entries this browser checked, from which on none counts.
    const next = (await get(`global:${this.user}`))?.have.split(",").find((h) => h.startsWith(`${key}.`))?.split(".")[1];
    await this.write("global", members.removeDevice({ key, seq: next ? Number(next) - 1 : null }), as);
    await this.renew(scopesOf(k, it.user), [key]);
  }

  /// Takes this browser out of its person's devices, as it signs out, so signing in again makes a new one that
  /// replaces it, never a second. Itself it may (the install lets a device take out its person's, itself among them),
  /// but nothing it writes after counts, so it goes in this order: its `remove` entry, whose seq is its own, so that one
  /// counts and none after; then off the network, which ends its connection, so nothing is asked after. Its scopes keep
  /// their keys, as it forgets its own (ARCHITECTURE.md): a device in doubt is removed from another, which makes new ones.
  async leave() {
    const k = await this.checked(true);
    if (!k.devices.some((d) => d.key === this.me.key)) return;
    await this.write("global", (seq) => members.removeDevice({ key: this.me.key, seq }));
    await this.tell({ remove: this.me.key });
  }

  /// Takes a person out, for an admin: each of their devices off the network first, then them (with the last of each of
  /// their devices' entries it had seen), then each scope they could open but their own (everyone's and their groups')
  /// gets a new epoch they don't have. Their own stays as it is.
  async removePerson(user) {
    const k = await this.checked(true);
    if (user === this.user) throw new Error("You can’t remove yourself. Another admin can.");
    if (!k.people[this.user].removes_people) throw new Error("Only an admin takes someone out.");
    const theirs = k.devices.filter((e) => e.user === user).map((e) => e.key);
    for (const key of theirs) {
      await this.tell({ remove: key }).catch((err) => {
        if (!/isn.t|not/.test(err.message)) throw err;
      });
    }
    // The last of each of their devices' entries this browser checked, from which on none counts.
    const have = (await get(`global:${this.user}`))?.have.split(",") || [];
    const seqs = Object.fromEntries(theirs.map((key) => {
      const next = have.find((h) => h.startsWith(`${key}.`))?.split(".")[1];
      return [key, next ? Number(next) - 1 : null];
    }));
    await this.write("global", members.removeUser({ user, seqs }));
    await this.renew(scopesOf(k, user).filter((s) => s !== `user:${user}`), theirs);
  }

  /// A new epoch of each scope this person may start one of, sealed to none of `except`.
  async renew(scopes, except) {
    const k = await this.checked(true);
    for (const scope of scopes.filter((s) => starts(k, s, this.user))) {
      const newest = k.epochs[scope]?.n || 0;
      if (newest) await this.newEpoch(scope, newest + 1, { except });
    }
  }

  // Invites.

  /// An invite link for someone new called `name`, made by this admin's browser: its secret stays here, to check the
  /// newcomer's device by when it asks to be let in. → `{ link, until }`.
  async invite(name) {
    const secret = b64u.encode(random(24));
    const user = hex.encode(random(8));
    const until = Date.now() + 7 * 86_400_000;
    await this.authSigned("POST", "/api/auth/invites", { id: hex.encode(await sha256(secret)), name: name.trim(), user, until });
    await put(`invite:${user}`, { secret, name: name.trim(), user, until });
    return { link: `${location.origin}/invite#${secret}`, until };
  }

  // Passkeys.

  /// Lists a passkey (from its PRF output in this tab) as a way back in: it's sealed what this browser opens.
  async listPasskey(prf, credential) {
    const pk = await passkeyKeysOf(prf);
    await this.write("global", members.passkey({ key: pk.key, seal: pk.seal, user: this.user, name: "Passkey", credential }));
    const k = await this.checked(true);
    await this.sealTo(pk, scopesOf(k, this.user));
    // An admin's passkey is an admin's device at the relay too, so the devices it lets in are let in as theirs.
    await this.tell({ add: pk.key, role: "client", ...(k.people[this.user].invites ? { admin: true } : {}) });
  }

  /// This browser let in by a listed passkey of its person's, which signs for it and opens what's sealed to it.
  async approveByPasskey(prf) {
    const pk = await passkeyKeysOf(prf);
    await this.approve({ key: this.me.key, seal: this.me.seal, user: this.user, name: browserName() }, pk);
  }
}

/// Runs `fetch`, timing nothing: what `note` is out of developer mode.
const noting = (_, fetch) => fetch();

/// A scope as a trace names it: its kind alone, never whose (src/node/trace.rs `kind`).
const kindOf = (scope) => (scope === "global" ? "everyone" : scope.startsWith("group:") ? "group" : "user");

/// Where a database of `need`'s is in the bucket (src/bucket.rs `folder`).
const prefixOf = (n) => `${n.scope.replace(":", "-")}/${n.app}/${n.database}`;

/// A response's JSON, or its error, thrown.
async function answer(r) {
  const out = await r.json().catch(() => ({}));
  if (!r.ok) throw Object.assign(new Error(out.error || out.message || "That didn’t work. Try again."), { status: r.status });
  return out;
}

/// What the person's devices call this browser: "Chrome on Mac".
export function browserName() {
  const ua = navigator.userAgent;
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "A browser";
  const os = /iPhone|iPad/.test(ua) ? "iPhone" : /Mac OS X/.test(ua) ? "Mac" : /Android/.test(ua) ? "Android" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "";
  return os ? `${browser} on ${os}` : browser;
}

export { answer };
