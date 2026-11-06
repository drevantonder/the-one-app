// This browser as a device of its person (ARCHITECTURE.md): its own Ed25519 key, which signs, and X25519 key, which
// what's sealed to it is sealed to, both made here, not extractable, kept in IndexedDB by person. Who's who, the keys a
// request needs and handing them are one-client's keyring, in the worker that reaches the nodes (net-worker.js). What
// changes who can do what it signs itself (members.js), and the epochs that go with it it opens here.
//
// Who's who it takes only from the network's own signed steps, as the keyring read and checked them (`checked`), never
// from what a node says (/v1/state): which seal key a device has, who opens a scope, which hash an epoch's key must
// have, and who may approve or remove what (`view_of` in client/src/members.rs), so a hacked node gets no keys.

import { b64u, ed25519Public, epochHash, hex, hkdfKey, inviteProof, joinCode, openAppKey, openSealed, passkeyKeys, random, sealAppKey, sealTo, sha256, signStep, signedRequest, twinPublic, untilTaken, x25519Public } from "../shared/crypto.js";
import { CODES, NUMBER_MAX, PUSH, READS, RELAY, SIGN_IN_DOMAIN, Yours } from "../shared/protocol.js";
import { del, get, put } from "./kept.js";
import { members } from "./members.js";
import { net } from "./net.js";

const text = new TextEncoder();

/// How long app schedules last (src/node/app_schedules.rs), unless their person renews them as they use the app.
const APP_SCHEDULES_DAYS = 30, DAY = 86_400_000, HOUR = 3_600_000;

const raw = async (key) => new Uint8Array(await crypto.subtle.exportKey("raw", key));

/// This browser's keys for `user`, made the first time: `{ key, seal }` (hex) to list it by, and the private halves,
/// which can't be read back.
export async function browserKeys(user) {
  let d = await get(`device:${user}`);
  if (!d) {
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

/// A passkey as a device, from its PRF output (`keys::passkey_keys`): its seal key's secret as a key that can't be
/// read back, as a browser's is.
export async function passkeyKeysOf(prf) {
  const pk = await passkeyKeys(prf);
  const seal = await x25519Public(pk.seal);
  const secret = await crypto.subtle.importKey("jwk", { kty: "OKP", crv: "X25519", d: b64u.encode(pk.seal), x: b64u.encode(seal) }, { name: "X25519" }, false, ["deriveBits"]);
  pk.seal.fill(0);
  return { key: hex.encode(await ed25519Public(pk.sign)), seal: hex.encode(seal), sign: pk.sign, opens: { secret, public: seal } };
}

export async function joinAsk(keys, name, invite) {
  return { key: keys.key, seal: keys.seal, name, reads: READS, ...(invite ? { proof: await inviteProof(text.encode(invite), hex.decode(keys.key), hex.decode(keys.seal)) } : {}) };
}

/// The code this browser shows to be approved: its key's, as 4 and 4.
export const codeOf = async (key, seal) => (await joinCode(hex.decode(key), hex.decode(seal))).replace(/^(.{4})/, "$1 ");

// Who opens what, as the network's signed steps say (`k`, one-client's `members::view`)

/// What a scope's epochs are sealed to: the client devices of the people who may use it, and nothing that only runs
/// calls.
const openers = (k, scope) => k.openers[scope] || [];

/// The scopes `user`'s devices open: everyone's, their own, and their groups'.
const scopesOf = (k, user) => k.people[user]?.uses || [];

/// Whether `user` may start a scope's new epoch: an admin of it (everyone is of their own).
const starts = (k, scope, user) => !!k.people[user]?.starts.includes(scope);

export class Device {
  /// `me`: this browser's keys; `user`: its person; `state()`: the node's /v1/state as last read, for what it shows;
  /// `api(path, opts)`: a /v1 request as this person; `bearer(path, init)`: one to the address with the access token.
  constructor(me, { user, state, api, bearer }) {
    Object.assign(this, { me, user, state, api, bearer });
    /// The members as last checked (`checked`).
    this.known = null;
    /// Each scope's epochs this browser opened, for what it writes: n → an HKDF key, its bytes gone.
    this.epochs = new Map();
    this.renewing = new Set();
    this.renewChecked = 0;
    /// What's being fetched now, by what it is, for everything that wants it at once to wait on the one fetch.
    this.fetching = new Map();
  }

  /// `fetch()`, once for everyone who asks for `id` while it's under way.
  once(id, fetch) {
    if (!this.fetching.has(id)) this.fetching.set(id, fetch().finally(() => this.fetching.delete(id)));
    return this.fetching.get(id);
  }

  /// Who's who as the network's signed steps say it, as this browser's keyring read them: as last read, or with what's
  /// new in the bucket if `fresh` (before it changes who opens what).
  async checked(fresh = false) {
    if (fresh || !this.known) {
      const k = await net.members(fresh);
      if (!k?.people[this.user]) throw new Error(`${Yours} doesn't list you yet, so this device didn't use it.`);
      this.known = k;
    }
    return this.known;
  }

  /// Opens epoch `n`'s key sealed to `as` from what the node gave, if its hash is the one its `epoch` step gave.
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

  /// Epoch `n` of a scope, as a key kept in memory: opened once (all its epochs at once, however many ask), or made if
  /// it's the scope's first and theirs to make.
  async epoch(scope, n) {
    const had = this.epochs.get(scope)?.get(n);
    if (had) return had;
    const newest = await this.once(`epochs ${scope}`, async () => {
      const { newest, keys } = await this.epochBytes(scope, this.me);
      const all = this.epochs.get(scope) || new Map();
      for (const [m, key] of keys) all.set(m, await hkdfKey(key));
      this.epochs.set(scope, all);
      return newest;
    });
    const made = () => this.epochs.get(scope)?.has(n);
    if (!made() && n === 1 && newest === 0) await this.once(`first ${scope}`, async () => made() || (starts(await this.checked(true), scope, this.user) && (await this.newEpoch(scope, 1))));
    if (!made()) throw new Error("This device can't open that yet.");
    return this.epochs.get(scope).get(n);
  }

  /// Starts a scope's epoch `n`: a random key, written first as an `epoch` step naming its hash (which, first in
  /// causal order, makes it), then sealed to the recovery key, if there is one, and each client device of the people who
  /// may open it but those in `except` (keys being taken out), before it's used.
  async newEpoch(scope, n, { except = [] } = {}) {
    const k = await this.checked(true);
    const key = random(32);
    const hash = await epochHash(key);
    const to = openers(k, scope).filter((e) => !except.includes(e.key));
    if (!to.some((e) => e.key === this.me.key) && !except.includes(this.me.key)) to.push({ key: this.me.key, seal: this.me.seal });
    await this.write("global", members.epoch({ scope, n, to: to.map((e) => e.key), hash }));
    if ((await this.checked(true)).epochs[scope]?.hashes?.[n] !== hash) throw new Error("Another device made new keys for this just now. Try again.");
    // Not while there's none (the one there was forked `global`), until an admin names the next.
    if (k.recovery) await this.api("/v1/keys", { method: "POST", body: { scope, n, to: "recovery", sealed: b64u.encode(await sealTo(twinPublic(hex.decode(k.recovery)), key)) } });
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

  /// An app's key in epoch `n` of a scope as its bytes, opened now (for sealing it to a device that runs the app's
  /// schedules): the one sealed under the epoch, or a new one if it has none yet.
  async appKeyBytes(scope, app, n) {
    const [epoch, all] = await Promise.all([this.epoch(scope, n), this.once(`sealed ${scope} ${app}`, () => this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&app=${encodeURIComponent(app)}`))]);
    let sealed = all.sealed?.[n];
    if (!sealed) {
      const made = random(32);
      sealed = (await this.api("/v1/keys", { method: "POST", body: { scope, n, app, sealed: b64u.encode(await sealAppKey(epoch, app, made)) } })).sealed;
      made.fill(0);
    }
    return openAppKey(epoch, app, b64u.decode(sealed));
  }

  /// Runs `app`'s schedules in `scope` on one of the network's devices (`endpoint`) for `days`, 30 at most (0 stops
  /// them): the app's key in each epoch of the scope, sealed to it first, then the app schedules naming each one's
  /// hash. Stopping them starts a new epoch of the scope, which the device isn't sealed, so the keys it held open
  /// nothing written after.
  async setAppSchedules({ endpoint, app, scope, days = APP_SCHEDULES_DAYS }) {
    const k = await this.checked();
    const e = k.devices.find((d) => d.key === endpoint && d.user);
    if (!e) throw new Error("That device isn’t one of yours.");
    const until = days === 0 ? 0 : Date.now() + Math.min(days, APP_SCHEDULES_DAYS) * DAY;
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
    await this.write("global", members.appSchedules({ endpoint, scope, app, until, keys }));
    await this.checked(true);
    if (!until) await this.renew([scope], []);
  }

  /// Renews, in the background, this person's app schedules of `app` as they use it: once a day at most, and at once
  /// when the scope has an epoch their keys don't reach. One of theirs that ran out is stopped, as Stop does. Only those
  /// the network's steps hold, as checked here (`app_schedules`), never those a node lists: a hacked one could list itself.
  async renewAppSchedules(app) {
    // Checked again first, at most once an hour, so it renews what its person's other devices set too.
    if (!(this.renewChecked > Date.now() - HOUR)) {
      this.renewChecked = Date.now();
      await this.checked(true).catch(() => {});
    }
    const now = Date.now();
    for (const g of this.known?.people[this.user]?.app_schedules || []) {
      if (g.until && g.until <= now) {
        const id = `${g.endpoint} ${g.scope} ${g.app}`;
        if (this.renewing.has(id)) continue;
        this.renewing.add(id);
        this.setAppSchedules({ endpoint: g.endpoint, app: g.app, scope: g.scope, days: 0 }).catch(() => {}).finally(() => this.renewing.delete(id));
        continue;
      }
      const newest = this.known?.epochs[g.scope]?.n || 0;
      const reaches = Math.max(0, ...Object.keys(g.keys || {}).map(Number));
      if (g.app !== app || g.until <= now || (g.until > now + (APP_SCHEDULES_DAYS - 1) * DAY && reaches >= newest)) continue;
      const id = `${g.endpoint} ${g.scope} ${g.app}`;
      if (this.renewing.has(id)) continue;
      this.renewing.add(id);
      this.setAppSchedules({ endpoint: g.endpoint, app: g.app, scope: g.scope }).catch(() => {}).finally(() => this.renewing.delete(id));
    }
  }

  /// Writes `content` to `log`'s database as `as` (this browser, or a passkey): `global`'s only, as a step (`step`).
  async write(log, content, as = this.me) {
    if (log !== "global") throw new Error(`a browser writes only Everyone's who's who, not ${log}`);
    return this.step(content, as);
  }

  /// `content` as a step of `global`'s scope database, signed by `as` for the next position
  /// (src/database/steps.rs), and again for the one after while another device's goes first.
  async step(content, as = this.me) {
    for (let tries = 0; ; tries++) {
      const { at, epoch } = await this.api(`/v1/steps/global?from=${NUMBER_MAX}`);
      const fields = { log: "global", at: at + 1, time: Date.now(), user: this.user, epoch };
      const s = await signStep(as.sign, hex.decode(as.key), fields, JSON.stringify(content));
      try {
        return await this.api("/v1/steps", { method: "POST", body: [{ header: b64u.encode(s.header), signature: b64u.encode(s.signature), body: b64u.encode(s.body) }] });
      } catch (err) {
        if (tries > 3 || err.code !== CODES.taken) throw err;
      }
    }
  }

  signed(domain, fields, as = this.me) {
    return signedRequest(as.sign, hex.decode(as.key), domain, fields);
  }

  /// Registers this browser's push subscription (`{ endpoint, keys }`) with the push Worker, signed by its key, with
  /// its seal key, which what's pushed to it is sealed to (cloudflare/push/), and its person, whom sign-in pushes about
  /// their devices joining.
  async registerPush({ endpoint, keys }) {
    const body = await this.signed(PUSH, { route: "POST /push/register", via: "webpush", address: endpoint, keys: { p256dh: keys.p256dh, auth: keys.auth }, seal: this.me.seal, user: this.user });
    const r = await fetch("/push/register", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(body) });
    if (!r.ok) throw new Error((await r.json().catch(() => null))?.error || `${Yours}'s push service didn't take this browser.`);
  }

  /// Tells the relay to let a key in (`{ add, admin? }`) or take one out (`{ remove }`), signed by `as`.
  async tell(fields, as = this.me) {
    const said = await untilTaken(async (at) => {
      const r = await fetch("/relay/tell", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await signedRequest(as.sign, hex.decode(as.key), RELAY, fields, at)) });
      return r.ok ? null : (await r.json().catch(() => null)) || {};
    });
    if (said) throw Object.assign(new Error(said.error || `${Yours} didn't take that.`), { code: said.code });
  }

  /// Takes a key off the relay's list, as `as`; one it doesn't list is out already.
  async takeOut(key, as) {
    await this.tell({ remove: key }, as).catch((err) => {
      if (err.code !== CODES.notListed) throw err;
    });
  }

  /// When each of the network's devices was last let onto its network, as the relay says: `{ key: ms }`, or none from a
  /// relay that doesn't say.
  async seen() {
    const r = await fetch("/relay/seen", { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify(await this.signed(RELAY, { seen: true })) });
    return r.ok ? (await r.json()).seen || {} : {};
  }

  /// One of sign-in's routes, signed by this browser: a GET's or DELETE's request in `x-one-request`, `query` after it.
  async authSigned(method, path, fields = {}, query = "") {
    const request = await this.signed(SIGN_IN_DOMAIN, { route: `${method} ${path}`, ...fields });
    const init = method === "POST" ? { method, headers: { "content-type": "application/json" }, body: JSON.stringify(request) } : { method, headers: { "x-one-request": b64u.encode(text.encode(JSON.stringify(request))) } };
    return answer(await fetch(path + query, init));
  }

  /// Devices waiting to be let in that this browser may approve: its person's own, and for an admin, those of people
  /// they invited from this browser, whose proof is right. Each with its code.
  async requests() {
    // Signed too, so sign-in knows this browser's device as its person's, to push to.
    const mine = await this.signed(SIGN_IN_DOMAIN, { route: "GET /api/auth/devices/requests" });
    const own = (await answer(await this.bearer("/api/auth/devices/requests", { headers: { "x-one-request": b64u.encode(text.encode(JSON.stringify(mine))) } }))).requests || [];
    const invited = (await this.checked()).people[this.user]?.invites ? (await this.authSigned("GET", "/api/auth/devices/requests").catch(() => ({}))).requests || [] : [];
    const all = [...own, ...invited].filter((r, i, a) => !r.service && r.key !== this.me.key && a.findIndex((x) => x.key === r.key) === i);
    const ours = await Promise.all(all.map(async (r) => r.user === this.user || (await this.invitedHere(r))));
    // Those turned down, or taken out, never again: as the network says now, since another device may have just said so.
    const out = new Set((await this.checked(true).catch(() => this.checked())).removed || []);
    return Promise.all(all.filter((r, i) => ours[i] && !out.has(r.key)).map(async (r) => ({ ...r, code: await codeOf(r.key, r.seal), mine: r.user === this.user })));
  }

  /// Whether a newcomer's device came from an invite made in this browser: its proof is that invite's.
  async invitedHere(r) {
    const invite = await get(`invite:${r.user}`);
    return !!invite && !!r.proof && r.proof === (await inviteProof(text.encode(invite.secret), hex.decode(r.key), hex.decode(r.seal)));
  }

  /// Turns down a device waiting to be let in: its request is forgotten, and one of this person's own is turned down
  /// by a step, so none of their devices lets it in after.
  async refuse(r) {
    const k = await this.checked(true);
    if (r.mine && !(k.removed || []).includes(r.key)) await this.write("global", members.refuse({ key: r.key, seal: r.seal, name: r.name }));
    if (r.mine) await answer(await this.bearer(`/api/auth/devices/requests/${r.key}`, { method: "DELETE" }));
    else await this.authSigned("DELETE", `/api/auth/devices/requests/${r.key}`);
  }

  /// Lets in a device that asked: lists it (and for a newcomer, them), seals it the epochs it may open, lets it onto the
  /// network and clears its request.
  async approve(r, as = this.me, { signIn } = {}) {
    const k = await this.checked(true);
    // Taken out, or turned down: never sealed anything again, whatever a node says.
    if ((k.removed || []).includes(r.key)) throw new Error(`${r.name} was turned down or taken out, so it isn’t let in again. Start again on it, for a new code.`);
    const may = k.people[this.user];
    const known = !!k.people[r.user];
    // Listed already by an approval that didn't finish (its mark at sign-in failed): the rest is done again.
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
      if (!(listed && err.code === CODES.already)) throw err;
    });
    // Signed in again on the same device: its sign-in before goes, as signing out would have taken it, so its scopes
    // keep their keys. One the relay wouldn't take out stays listed, with the next sign-in here to try it again; this
    // one is let in all the same.
    for (const old of r.device ? k.devices.filter((d) => d.user === r.user && d.device === r.device && d.key !== r.key && !d.passkey && (d.roles || []).includes("client")) : []) {
      try {
        await this.takeOut(old.key, as);
      } catch (err) {
        console.warn(`${r.name}'s earlier sign-in is still let in: ${err.message}`);
        continue;
      }
      await this.write("global", members.removeDevice({ key: old.key }), as);
    }
    // Marked approved, for the device waiting on it to see, with the network it's in: the only one it then takes; and
    // once who's who is written as steps, where this browser read them to, which it starts from (src/database/steps.rs
    // `Welcome`).
    const held = await this.checked();
    const approved = `?approved&root=${held.root || ""}${held.welcome ? `&welcome=${held.welcome}` : ""}`;
    if (newUser) await this.authSigned("DELETE", `/api/auth/devices/requests/${r.key}`, {}, approved).catch(() => {});
    else await this.bearer(`/api/auth/devices/requests/${r.key}${approved}`, { method: "DELETE" }).catch(() => {});
  }

  /// Seals each epoch of `scopes` that `as` opens to a device (`{ key, seal }`), but those it has.
  async sealTo(device, scopes, as = this.me) {
    for (const scope of scopes) {
      const { keys } = await this.epochBytes(scope, as);
      for (const [n, key] of keys) {
        await this.api("/v1/keys", { method: "POST", body: { scope, n, to: device.key, sealed: b64u.encode(await sealTo(hex.decode(device.seal), key)) } }).catch((err) => {
          if (err.code !== CODES.already) throw err;
        });
        key.fill(0);
      }
    }
  }

  /// Takes one of this person's devices out (or, for an admin, anyone's): off the network first, then unlisted, then
  /// each scope it opened that this person may move on gets a new epoch it doesn't have.
  async remove(key, as = this.me) {
    // Itself, it goes in the order `leave` says: its removal counts only if it's the last step of its own that does.
    if (key === this.me.key) return this.leave();
    const k = await this.checked(true);
    const it = k.devices.find((e) => e.key === key);
    if (!it) throw new Error("That device isn’t listed.");
    if (!k.people[this.user].removes.includes(key)) throw new Error("Only its person or an admin takes a device out.");
    // It, and what's part of it: its services with endpoints of their own.
    const keys = [key, ...k.devices.filter((d) => d.device === key && d.key !== key).map((d) => d.key)];
    for (const one of keys) {
      await this.takeOut(one, as);
      await this.write("global", members.removeDevice({ key: one }), as);
    }
    await this.renew(scopesOf(k, it.user), keys);
  }

  /// Takes this browser out of its person's devices, as it signs out, so signing in again makes a new one that
  /// replaces it, never a second. Itself it may (the network lets a device take out its person's, itself among them),
  /// but nothing it writes after counts, so it goes in this order: its removal, then off the network, which ends its
  /// connection, so nothing is asked after. Its scopes keep their keys, as it forgets its own (ARCHITECTURE.md): a
  /// device in doubt is removed from another, which makes new ones.
  async leave() {
    const k = await this.checked(true);
    if (!k.devices.some((d) => d.key === this.me.key)) return;
    // A node closes a removed device's connection as it applies its removal, so the answer may never come: off the
    // network all the same.
    const wrote = await this.write("global", members.removeDevice({ key: this.me.key })).then(() => null, (err) => err);
    await this.takeOut(this.me.key, this.me);
    if (wrote) throw wrote;
  }

  /// Takes a person out, for an admin: each of their devices off the network first, then them, then each scope they
  /// could open but their own (everyone's and their groups') gets a new epoch they don't have. Their own stays as it is.
  async removePerson(user) {
    const k = await this.checked(true);
    if (user === this.user) throw new Error("You can’t remove yourself. Another admin can.");
    if (!k.people[this.user].removes_people) throw new Error("Only an admin takes someone out.");
    const theirs = k.devices.filter((e) => e.user === user).map((e) => e.key);
    for (const key of theirs) {
      await this.takeOut(key, this.me);
    }
    await this.write("global", members.removeUser({ user }));
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
