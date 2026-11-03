// This browser as a device of its person (ARCHITECTURE.md, Being built: Devices): its own Ed25519 key, which signs, and
// X25519 key, which what's sealed to it is sealed to, both made here, not extractable, kept in IndexedDB by person. It
// gets in when one of their devices approves it; then it opens its scopes' epochs, sealed to it, and from them the keys
// a call needs, which it hands the node with the call (`x-one-keys`). What it opens stays in this page's memory, as keys
// that can't be read back, and goes with it. What changes who can do what it signs itself (members.js).

import { b64u, databaseKey, ed25519Public, epochHash, hex, hkdfKey, inviteProof, joinCode, openAppKey, openSealed, passkeyKeys, random, sealAppKey, sealHandoff, sealTo, sha256, signEntry, signedRequest, tokenId, twinPublic, x25519Public } from "../shared/crypto.js";
import { get, put } from "./kept.js";
import { members } from "./members.js";

const text = new TextEncoder();

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

// MARK: Who opens what

const clientOf = (e) => (e.roles || []).includes("client") && e.seal && e.seal !== e.key;

/// The people who may open a scope.
function people(st, scope) {
  const i = st.install;
  if (scope === "global") return i.users.map((u) => u.id);
  if (scope.startsWith("user:")) return [scope.slice(5)];
  return (i.groups.find((g) => `group:${g.id}` === scope)?.members || []).map((m) => m.user);
}

/// What a scope's epochs are sealed to: the client devices of the people who may open it, and nothing that only runs
/// calls.
function openers(st, scope) {
  const who = people(st, scope);
  const all = [...st.install.devices, ...st.install.clients];
  return all.filter((e) => clientOf(e) && who.includes(e.user)).filter((e, i) => all.findIndex((x) => x.key === e.key) === i);
}

/// The scopes `user`'s devices open: everyone's, their own, and their groups'.
const scopesOf = (st, user) => ["global", `user:${user}`, ...st.install.groups.filter((g) => g.members.some((m) => m.user === user)).map((g) => `group:${g.id}`)];

/// Whether this person may start a scope's new epoch.
function starts(st, scope) {
  const me = st.user.id;
  if (scope === "global") return st.install.role === "admin";
  if (scope.startsWith("user:")) return scope === `user:${me}`;
  return st.install.groups.find((g) => `group:${g.id}` === scope)?.role === "admin";
}

// MARK: The device

export class Device {
  /// `me`: this browser's keys; `state()`: the node's /v1/state as last read; `api(path, opts)`: a /v1 request as this
  /// person; `bearer(path, init)`: one to the address with the access token.
  constructor(me, { state, api, bearer }) {
    Object.assign(this, { me, state, api, bearer });
    /// Each scope's epochs this browser opened: n → an HKDF key, its bytes gone.
    this.epochs = new Map();
    /// Each app's key in a scope by epoch, the same: `scope app` → n → key.
    this.appKeys = new Map();
    /// Databases' keys, by `scope app database n`: bytes, as a node is handed them.
    this.dbKeys = new Map();
    /// The node's key to seal handoffs to, and what each kind of request needed (`one.needs`, which names no key).
    this.node = null;
    this.needs = JSON.parse(localStorage.getItem("one.needs") || "{}");
  }

  get user() {
    return this.state().user.id;
  }

  // Epochs and keys.

  /// Opens epoch `n`'s key sealed to `as` from what the node gave, if its hash is the one its epoch entry gave.
  async openEpoch(scope, n, sealed, as = this.me) {
    const key = await openSealed(as.opens, b64u.decode(sealed));
    const said = this.state().install.epochs?.[scope]?.hashes?.[n];
    if (!said || said !== (await epochHash(key))) throw new Error("A key for this didn't check out, so it wasn't used.");
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
    if (!all.has(n) && n === 1 && newest === 0 && starts(this.state(), scope)) await this.newEpoch(scope, 1);
    if (!all.has(n)) throw new Error("This browser hasn't been given the keys to that yet.");
    return all.get(n);
  }

  /// Starts a scope's epoch `n`: a random key, written first as an `epoch` entry naming its hash (which, first in
  /// causal order, makes it), then sealed to the recovery key and each client device of the people who may open it but
  /// those in `except` (keys being taken out), before it's used.
  async newEpoch(scope, n, { except = [] } = {}) {
    const st = this.state();
    if (!st.install.recovery) throw new Error("Your One App has no recovery key to keep its keys with.");
    const key = random(32);
    const hash = await epochHash(key);
    const to = openers(st, scope).filter((e) => !except.includes(e.key));
    if (!to.some((e) => e.key === this.me.key)) to.push({ key: this.me.key, seal: this.me.seal });
    await this.write("global", members.epoch({ scope, n, to: to.map((e) => e.key), hash }));
    const now = await this.api("/v1/state");
    if (now.install.epochs?.[scope]?.hashes?.[n] !== hash) throw new Error("Another device made new keys for this just now. Try again.");
    await this.api("/v1/keys", { method: "POST", body: { scope, n, to: "recovery", sealed: b64u.encode(await sealTo(twinPublic(hex.decode(st.install.recovery)), key)) } });
    for (const e of to) await this.api("/v1/keys", { method: "POST", body: { scope, n, to: e.key, sealed: b64u.encode(await sealTo(hex.decode(e.seal), key)) } });
    const all = this.epochs.get(scope) || new Map();
    all.set(n, await hkdfKey(key));
    this.epochs.set(scope, all);
  }

  /// This person's own things, once their device is in: where they run besides their own machines, if their devices
  /// never said (the machines that run everyone's, chosen without asking), and their own first key, if it isn't made.
  async settleOwn() {
    const st = this.state(), mine = `user:${this.user}`;
    if (st.install.runs == null) {
      const on = st.install.devices.filter((d) => d.user !== this.user && (d.runs_for || []).includes("global")).map((d) => d.key);
      if (on.length) await this.write("global", members.runs({ on }));
    }
    if (!st.install.epochs?.[mine]) await this.newEpoch(mine, 1);
  }

  /// An app's key in epoch `n` of a scope: the one sealed under the epoch, or a new one if it has none yet.
  async appKey(scope, app, n) {
    const id = `${scope} ${app}`;
    const had = this.appKeys.get(id)?.get(n);
    if (had) return had;
    const epoch = await this.epoch(scope, n);
    let sealed = (await this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&app=${encodeURIComponent(app)}`)).sealed?.[n];
    if (!sealed) {
      const made = random(32);
      sealed = (await this.api("/v1/keys", { method: "POST", body: { scope, n, app, sealed: b64u.encode(await sealAppKey(epoch, app, made)) } })).sealed;
      made.fill(0);
    }
    const key = await hkdfKey(await openAppKey(epoch, app, b64u.decode(sealed)));
    if (!this.appKeys.has(id)) this.appKeys.set(id, new Map());
    this.appKeys.get(id).set(n, key);
    return key;
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

  /// The `x-one-keys` a request goes with: the keys it needed before, sealed to the node, for this browser's key when
  /// it reaches the node itself (`token` null), or for `token` through the address, until `until`.
  async handoff(route, token, until) {
    const needs = this.needs[route];
    if (!needs?.length) return null;
    this.node ||= (await this.api("/v1/endpoint", { bare: true })).seal;
    const keys = [];
    for (const n of needs) keys.push({ ...n, key: b64u.encode(await this.databaseKey(n)) });
    return sealHandoff(hex.decode(this.node), { for: token ? await tokenId(token) : this.me.key, until, keys });
  }

  /// A node's 428: what the request needed, remembered for next time (each kind of request keeps its last 64), and the
  /// node to seal them to. Throws if this browser can't open them.
  async learn(route, answer) {
    this.node = answer.seal || this.node;
    const had = this.needs[route] || [];
    const same = (a, b) => a.scope === b.scope && a.app === b.app && a.database === b.database && a.epoch === b.epoch;
    const now = [...(answer.need || []), ...had.filter((h) => !(answer.need || []).some((n) => same(n, h)))].slice(0, 64);
    for (const n of answer.need || []) await this.databaseKey(n);
    this.needs[route] = now;
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

  /// One of auth's routes, signed by this browser: a GET's or DELETE's request in `x-one-request`.
  async authSigned(method, path, fields = {}) {
    const request = await this.signed("one/auth", { route: `${method} ${path}`, ...fields });
    const init = method === "POST" ? { method, headers: { "content-type": "application/json" }, body: JSON.stringify(request) } : { method, headers: { "x-one-request": b64u.encode(text.encode(JSON.stringify(request))) } };
    return answer(await fetch(path, init));
  }

  // Devices joining.

  /// Devices waiting to be let in that this person may approve: their own, and for an admin, those of people they
  /// invited from this browser. Each with its code.
  async requests() {
    const st = this.state();
    const own = (await answer(await this.bearer("/api/auth/devices/requests"))).requests || [];
    const invited = st.install.role === "admin" ? (await this.authSigned("GET", "/api/auth/devices/requests").catch(() => ({}))).requests || [] : [];
    const all = [...own, ...invited].filter((r, i, a) => r.key !== this.me.key && a.findIndex((x) => x.key === r.key) === i);
    return Promise.all(all.map(async (r) => ({ ...r, code: await codeOf(r.key, r.seal), mine: r.user === this.user })));
  }

  /// Lets in a device that asked: lists it (and for a newcomer, them), seals it the epochs it may open, lets it onto the
  /// network and clears its request.
  async approve(r, as = this.me) {
    const st = this.state();
    const known = st.install.users.some((u) => u.id === r.user);
    if (r.user !== this.user && known) throw new Error("Only their own devices approve one of theirs.");
    let newUser = null;
    if (r.user !== this.user) {
      const invite = await get(`invite:${r.user}`);
      if (!invite || !r.proof || r.proof !== (await inviteProof(text.encode(invite.secret), hex.decode(r.key), hex.decode(r.seal)))) throw new Error("That device didn’t come from an invite made here.");
      newUser = { name: invite.name || r.person, role: "member" };
    }
    await this.write("global", members.device({ key: r.key, seal: r.seal, user: r.user, name: r.name, newUser }), as);
    await this.sealTo(r, newUser ? ["global"] : scopesOf(st, r.user), as);
    await this.tell({ add: r.key, role: "client", ...(!newUser && st.install.role === "admin" ? { admin: true } : {}) }, as);
    if (newUser) await this.authSigned("DELETE", `/api/auth/devices/requests/${r.key}`).catch(() => {});
    else await this.bearer(`/api/auth/devices/requests/${r.key}`, { method: "DELETE" }).catch(() => {});
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
    const st = this.state();
    const it = [...st.install.devices, ...st.install.clients].find((e) => e.key === key);
    if (!it) throw new Error("That device isn’t listed.");
    await this.tell({ remove: key }).catch((err) => {
      if (!/isn.t|not/.test(err.message)) throw err;
    });
    const seq = (await this.api("/v1/entries?scope=global")).logs?.[key]?.seq ?? null;
    await this.write("global", members.removeDevice({ key, seq }));
    await this.renew(scopesOf(st, it.user), [key]);
  }

  /// Takes a person out, for an admin: each of their devices off the network first, then them, then each scope they
  /// could open but their own (everyone's and their groups') gets a new epoch they don't have. Their own stays as it is.
  async removePerson(user) {
    const st = this.state();
    if (user === this.user) throw new Error("You can’t remove yourself. Another admin can.");
    const theirs = [...st.install.devices, ...st.install.clients].filter((e) => e.user === user).map((e) => e.key);
    for (const key of theirs) {
      await this.tell({ remove: key }).catch((err) => {
        if (!/isn.t|not/.test(err.message)) throw err;
      });
    }
    await this.write("global", members.removeUser({ user }));
    await this.renew(scopesOf(st, user).filter((s) => s !== `user:${user}`), theirs);
  }

  /// A new epoch of each scope this person may start one of, sealed to none of `except`.
  async renew(scopes, except) {
    for (const scope of scopes.filter((s) => starts(this.state(), s))) {
      const newest = (await this.api(`/v1/keys?scope=${encodeURIComponent(scope)}&to=${this.me.key}`)).epoch || 0;
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
    await this.sealTo(pk, scopesOf(this.state(), this.user));
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
