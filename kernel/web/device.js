// This browser as a device of its person (ARCHITECTURE.md, Being built: Devices): its own Ed25519 key, which signs, and
// X25519 key, which what's sealed to it is sealed to, both made here, not extractable, kept in IndexedDB by person. It
// gets in when one of their devices approves it; then it opens its scopes' epochs, sealed to it, and from them the keys
// a call needs, which it hands the node with the call (`x-one-keys`). What it opens stays in this page's memory, as keys
// that can't be read back, and goes with it. What changes who can do what it signs itself (members.js).
//
// Who's who it takes only from the install's own signed entries, checked here (`checked`), never from what a node says
// (/v1/state): which seal key a device has, who opens a scope, which hash an epoch's key must have, and which nodes may
// be handed a scope's keys. A hacked node holds no keys, and gets none this way.

import { b64u, databaseKey, ed25519Public, epochHash, hex, hkdfKey, inviteProof, joinCode, openAppKey, openSealed, passkeyKeys, random, sealAppKey, sealHandoff, sealTo, sha256, signEntry, signedRequest, twinPublic, x25519Public } from "../shared/crypto.js";
import { get, put } from "./kept.js";
import { members } from "./members.js";

const text = new TextEncoder();

/// How long standing work lasts (src/node/standing.rs), unless its person renews it as they use the app.
const GRANT_DAYS = 30, DAY = 86_400_000;

// MARK: Kept in this browser

const raw = async (key) => new Uint8Array(await crypto.subtle.exportKey("raw", key));

/// This browser's keys for `user`, made the first time: `{ key, seal }` (hex) to list it by, and the private halves,
/// which can't be read back.
export async function browserKeys(user) {
  let d = await get(`device:${user}`);
  if (!d) {
    const sign = await crypto.subtle.generateKey({ name: "Ed25519" }, false, ["sign", "verify"]);
    const seal = await crypto.subtle.generateKey({ name: "X25519" }, false, ["deriveBits"]);
    d = { sign: sign.privateKey, key: hex.encode(await raw(sign.publicKey)), seal: seal.privateKey, sealKey: hex.encode(await raw(seal.publicKey)) };
    await put(`device:${user}`, d);
  }
  return { key: d.key, seal: d.sealKey, sign: d.sign, opens: { secret: d.seal, public: hex.decode(d.sealKey) } };
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

// MARK: Who opens what, as the install's signed entries say (`k`, one-client's members view)

/// What a scope's epochs are sealed to: the client devices of the people who may use it, and nothing that only runs
/// calls.
const openers = (k, scope) => k.openers[scope] || [];

/// The scopes `user`'s devices open: everyone's, their own, and their groups'.
const scopesOf = (k, user) => Object.keys(k.roles).filter((s) => k.roles[s][user]);

/// Whether `user` may start a scope's new epoch: an admin of it (everyone is of their own).
const starts = (k, scope, user) => k.roles[scope]?.[user] === "admin";

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
    /// What each kind of request needed the last time, and the node that said so (`one.needs`, which names no key).
    this.needs = JSON.parse(localStorage.getItem("one.needs") || "{}");
    /// The grants this browser is renewing now.
    this.renewing = new Set();
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
      const k = !r || (r.status === 204 && kept) ? kept.members : await this.replay(this.user, r.status === 204 ? undefined : new Uint8Array(await r.arrayBuffer()));
      if (!k.roles.global?.[this.user]) throw new Error("Your One App doesn't list you yet, so this device didn't use it.");
      this.known = k;
    }
    return this.known;
  }

  /// The members as last checked, kept in this browser, to start with at once.
  async load() {
    this.known ||= (await get(`global:${this.user}`))?.members || null;
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
  async epochBytes(scope, as = this.me) {
    const { epoch, sealed } = await this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&to=${as.key}`);
    const out = new Map();
    for (const [n, s] of Object.entries(sealed || {})) out.set(Number(n), await this.openEpoch(scope, Number(n), s, as));
    return { newest: epoch || 0, keys: out };
  }

  /// Epoch `n` of a scope, as a key kept in memory: opened once, or made if it's the scope's first and theirs to make.
  async epoch(scope, n) {
    const had = this.epochs.get(scope)?.get(n);
    if (had) return had;
    const { newest, keys } = await this.epochBytes(scope);
    const all = this.epochs.get(scope) || new Map();
    for (const [m, key] of keys) all.set(m, await hkdfKey(key));
    this.epochs.set(scope, all);
    if (!all.has(n) && n === 1 && newest === 0 && starts(await this.checked(true), scope, this.user)) await this.newEpoch(scope, 1);
    if (!all.has(n)) throw new Error("This device can't open that yet.");
    return all.get(n);
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
    if (!to.some((e) => e.key === this.me.key)) to.push({ key: this.me.key, seal: this.me.seal });
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
    if (k.runs[this.user] == null || !k.epochs[mine]) k = await this.checked(true);
    if (k.runs[this.user] == null) {
      // Machines only: the serverless node is no one's, and runs anyone's when no machine answers.
      const whose = (key) => k.devices.find((d) => d.key === key)?.user;
      const on = Object.keys(k.runs_for).filter((key) => whose(key) && whose(key) !== this.user && k.runs_for[key].includes("global"));
      if (on.length) await this.write("global", members.runs({ on }));
    }
    if (!k.epochs[mine]) await this.newEpoch(mine, 1);
  }

  /// An app's key in epoch `n` of a scope: the one sealed under the epoch, or a new one if it has none yet.
  async appKey(scope, app, n) {
    const id = `${scope} ${app}`;
    const had = this.appKeys.get(id)?.get(n);
    if (had) return had;
    const key = await hkdfKey(await this.appKeyBytes(scope, app, n));
    if (!this.appKeys.has(id)) this.appKeys.set(id, new Map());
    this.appKeys.get(id).set(n, key);
    return key;
  }

  /// An app's key in epoch `n` of a scope as its bytes, opened now (for sealing it to a device that keeps the app
  /// running): the one sealed under the epoch, or a new one if it has none yet.
  async appKeyBytes(scope, app, n) {
    const epoch = await this.epoch(scope, n);
    let sealed = (await this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&app=${encodeURIComponent(app)}`)).sealed?.[n];
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
    const st = this.state();
    const e = st.install.devices.find((d) => d.key === endpoint);
    if (!e) throw new Error("That device isn’t one of yours.");
    const until = days === 0 ? 0 : Date.now() + Math.min(days, GRANT_DAYS) * DAY;
    const keys = {};
    if (until) {
      const made = Object.keys(st.install.epochs?.[scope]?.hashes || {}).map(Number);
      for (const n of made.length ? made : [1]) {
        const key = await this.appKeyBytes(scope, app, n);
        await this.api("/v1/keys", { method: "POST", body: { scope, n, app, to: endpoint, sealed: b64u.encode(await sealTo(hex.decode(e.seal), key)) } });
        keys[n] = await epochHash(key);
        key.fill(0);
      }
    }
    await this.write("global", members.grant({ endpoint, scope, app, until, keys }));
    if (!until) await this.renew([scope], []);
  }

  /// Renews, in the background, this person's grants of `app` as they use it: once a day at most, and at once when
  /// the scope has an epoch a grant's keys don't reach. One of theirs that ran out is stopped, as Stop does.
  renewGrants(app) {
    const st = this.state(), now = Date.now();
    for (const g of st.install?.grants || []) {
      if (g.by === this.user && g.until && g.until <= now) {
        const id = `${g.endpoint} ${g.scope} ${g.app}`;
        if (this.renewing.has(id)) continue;
        this.renewing.add(id);
        this.grant({ endpoint: g.endpoint, app: g.app, scope: g.scope, days: 0 }).catch(() => {}).finally(() => this.renewing.delete(id));
        continue;
      }
      const newest = st.install.epochs?.[g.scope]?.n || 0;
      const reaches = Math.max(0, ...Object.keys(g.keys || {}).map(Number));
      if (g.app !== app || g.by !== this.user || g.until <= now || (g.until > now + (GRANT_DAYS - 1) * DAY && reaches >= newest)) continue;
      const id = `${g.endpoint} ${g.scope} ${g.app}`;
      if (this.renewing.has(id)) continue;
      this.renewing.add(id);
      this.grant({ endpoint: g.endpoint, app: g.app, scope: g.scope }).catch(() => {}).finally(() => this.renewing.delete(id));
    }
  }

  async databaseKey({ scope, app, database, epoch }) {
    const id = `${scope} ${app} ${database} ${epoch}`;
    if (!this.dbKeys.has(id)) this.dbKeys.set(id, await databaseKey(await this.appKey(scope, app, epoch), database));
    return this.dbKeys.get(id);
  }

  // Handing a call its keys.

  /// What a request is, to remember what it needed: an app's call by its app, anything else by its path.
  static route(path, body) {
    const at = new URL(path, location.origin).pathname;
    return at === "/v1/call" ? `call ${body?.app || ""}` : at.replace(/^\/v1\/screens\/([^/]+)\/.*/, "/v1/screens/$1");
  }

  /// The app a kind of request runs, if it names one: an app's call, or its screens.
  static appOf(route) {
    const m = /^call (.+)$/.exec(route) || /^\/v1\/screens\/([^/]+)$/.exec(route);
    return m ? decodeURIComponent(m[1]) : null;
  }

  /// The `x-one-keys` a request goes with: what its kind of request needed the last time, sealed to the node that said
  /// so (to its key's twin, as every node's seal key is), for this browser's key, until `until`. A node that didn't say
  /// so can't open it.
  async handoff(route, until) {
    const last = this.needs[route];
    if (!last?.node || !last.seal || !last.need?.length || !runsAll(this.known, last.node, last.need.map((n) => n.scope))) return null;
    const keys = [];
    for (const n of last.need) keys.push({ ...n, key: b64u.encode(await this.databaseKey(n)) });
    return sealHandoff(hex.decode(last.seal), { for: this.me.key, until, keys });
  }

  /// A node's 428, believed only from `node`, the one that answered it as its connection proved: it must name itself
  /// and the seal key the install's entries list for it, run every scope it needs keys of, as they say, and a request
  /// that runs one app may need only that app's databases (and a scope's Vault, which fills in its items, and its log,
  /// which the serverless node reads who's who from). What
  /// it needed is all this kind of request hands next time, and only to it. Throws if it asks for more, or this browser
  /// can't open it.
  async learn(route, answer, node) {
    const need = Array.isArray(answer.need) ? answer.need : [];
    if (!node || answer.to !== node) throw new Error("A device that didn’t answer asked for keys, so none were given.");
    const scopes = need.map((n) => n.scope);
    let k = await this.checked();
    if (!runsAll(k, node, scopes)) k = await this.checked(true);
    if (!runsAll(k, node, scopes)) throw new Error("A device that doesn’t run that asked for its keys, so none were given.");
    const seal = k.devices.find((d) => d.key === node)?.seal;
    if (!seal || (answer.seal && answer.seal !== seal)) throw new Error("A device asked for keys sealed to another, so none were given.");
    const app = Device.appOf(route);
    if (need.length > 64 || (app && need.some((n) => n.app !== app && !(n.app === "node" && ["vault", "log"].includes(n.database))))) throw new Error("A device asked for keys that request doesn’t use, so none were given.");
    for (const n of need) await this.databaseKey(n);
    this.needs[route] = { node, seal, need };
    try { localStorage.setItem("one.needs", JSON.stringify(this.needs)); } catch {}
  }

  // Signing.

  /// Signs an entry of `content` in `log` as `as` (this browser, or a passkey), after what the node has, and writes it.
  async write(log, content, as = this.me) {
    for (let tries = 0; ; tries++) {
      const heads = await this.api(`/v1/entries?scope=${encodeURIComponent(log)}`);
      const last = heads.logs?.[as.key];
      const fields = { log, seq: last ? last.seq + 1 : 0, backlink: last?.hash ?? null, previous: (heads.previous || []).filter((h) => h !== last?.hash), time: Date.now(), user: this.user, epoch: heads.epoch };
      const e = await signEntry(as.sign, hex.decode(as.key), fields, JSON.stringify(content));
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
    const r = await fetch("/relay/tell", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await this.signed("one/relay", fields, as)) });
    if (!r.ok) throw new Error((await r.json().catch(() => null))?.error || "Your One App's network didn't take that.");
  }

  /// One of auth's routes, signed by this browser: a GET's or DELETE's request in `x-one-request`, `query` after it.
  async authSigned(method, path, fields = {}, query = "") {
    const request = await this.signed("one/auth", { route: `${method} ${path}`, ...fields });
    const init = method === "POST" ? { method, headers: { "content-type": "application/json" }, body: JSON.stringify(request) } : { method, headers: { "x-one-request": b64u.encode(text.encode(JSON.stringify(request))) } };
    return answer(await fetch(path + query, init));
  }

  // Devices joining.

  /// Devices waiting to be let in that this person may approve: their own, and for an admin, those of people they
  /// invited from this browser. Each with its code.
  async requests() {
    const st = this.state();
    const own = (await answer(await this.bearer("/api/auth/devices/requests"))).requests || [];
    const invited = st.install.role === "admin" ? (await this.authSigned("GET", "/api/auth/devices/requests").catch(() => ({}))).requests || [] : [];
    const all = [...own, ...invited].filter((r, i, a) => !r.service && r.key !== this.me.key && a.findIndex((x) => x.key === r.key) === i);
    return Promise.all(all.map(async (r) => ({ ...r, code: await codeOf(r.key, r.seal), mine: r.user === this.user })));
  }

  /// Lets in a device that asked: lists it (and for a newcomer, them), seals it the epochs it may open, lets it onto the
  /// network and clears its request.
  async approve(r, as = this.me) {
    const k = await this.checked(true);
    const known = !!k.roles.global?.[r.user];
    if (r.user !== this.user && known) throw new Error("Only their own devices approve one of theirs.");
    let newUser = null;
    if (r.user !== this.user) {
      const invite = await get(`invite:${r.user}`);
      if (!invite || !r.proof || r.proof !== (await inviteProof(text.encode(invite.secret), hex.decode(r.key), hex.decode(r.seal)))) throw new Error("That device didn’t come from an invite made here.");
      newUser = { name: invite.name || r.person, role: "member" };
    }
    await this.write("global", members.device({ key: r.key, seal: r.seal, user: r.user, name: r.name, newUser }), as);
    await this.sealTo(r, newUser ? ["global"] : scopesOf(k, r.user), as);
    await this.tell({ add: r.key, role: "client", ...(!newUser && k.roles.global?.[this.user] === "admin" ? { admin: true } : {}) }, as);
    // Marked approved, for the device waiting on it to see.
    if (newUser) await this.authSigned("DELETE", `/api/auth/devices/requests/${r.key}`, {}, "?approved").catch(() => {});
    else await this.bearer(`/api/auth/devices/requests/${r.key}?approved`, { method: "DELETE" }).catch(() => {});
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
  async remove(key) {
    const k = await this.checked(true);
    const it = k.devices.find((e) => e.key === key);
    if (!it) throw new Error("That device isn’t listed.");
    await this.tell({ remove: key }).catch((err) => {
      if (!/isn.t|not/.test(err.message)) throw err;
    });
    // The last of its entries this browser checked, from which on none counts.
    const next = (await get(`global:${this.user}`))?.have.split(",").find((h) => h.startsWith(`${key}.`))?.split(".")[1];
    await this.write("global", members.removeDevice({ key, seq: next ? Number(next) - 1 : null }));
    await this.renew(scopesOf(k, it.user), [key]);
  }

  /// Takes a person out, for an admin: each of their devices off the network first, then them (with the last of each of
  /// their devices' entries it had seen), then each scope they could open but their own (everyone's and their groups')
  /// gets a new epoch they don't have. Their own stays as it is.
  async removePerson(user) {
    const k = await this.checked(true);
    if (user === this.user) throw new Error("You can’t remove yourself. Another admin can.");
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
    await this.sealTo(pk, scopesOf(await this.checked(true), this.user));
    await this.tell({ add: pk.key, role: "client" });
  }

  /// This browser let in by a listed passkey of its person's, which signs for it and opens what's sealed to it.
  async approveByPasskey(prf) {
    const pk = await passkeyKeysOf(prf);
    await this.approve({ key: this.me.key, seal: this.me.seal, user: this.user, name: browserName() }, pk);
  }
}

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
