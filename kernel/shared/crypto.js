// The kernel's formats, which nodes read too (src/kernel.rs; tests/contracts.rs checks they agree).
//
// Keys are a tree per scope. A scope's key is 32 random bytes, and so is each app's key in it, kept sealed under the
// scope's key: nonce ‖ AES-256-GCM(HKDF-SHA256(scope key, "one/app\0<app>"), nonce, app key), so erasing an app (deleting
// its sealed key) destroys its key, and its data with it. A database's key is derived from its app's with HKDF-SHA256.
//
// Sealed to a public key (an X25519 key: a person's, the recovery key's twin, the schedules service's), bytes are
// share ‖ AES-256-GCM(key, zero nonce, plain), where share is a fresh X25519 public key and key is HKDF-SHA256 of the
// shared secret, salted with share ‖ the recipient's public key. Each key seals once, so the zero nonce is safe.
//
// Sealed under a secret (a passkey's PRF output, a refresh token, an access token), bytes are
// nonce ‖ AES-256-GCM(key, nonce, plain), with key and the copy's id both derived from the secret.

const text = new TextEncoder();

export const b64u = {
  encode(bytes) {
    let s = "";
    for (const b of new Uint8Array(bytes)) s += String.fromCharCode(b);
    return btoa(s).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
  },
  decode(s) {
    if (typeof s !== "string" || !/^[A-Za-z0-9_-]*$/.test(s)) throw new Error("not base64url");
    const bin = atob(s.replaceAll("-", "+").replaceAll("_", "/"));
    return Uint8Array.from(bin, (c) => c.charCodeAt(0));
  },
};

function bytes(v) {
  return typeof v === "string" ? text.encode(v) : new Uint8Array(v);
}

function concat(...parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

export async function hkdf(ikm, info, salt = new Uint8Array()) {
  const key = await crypto.subtle.importKey("raw", bytes(ikm), "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: bytes(salt), info: bytes(info) }, key, 256));
}

export function random(n = 32) {
  return crypto.getRandomValues(new Uint8Array(n));
}

/// An app's key, sealed under its scope's key.
export async function sealAppKey(scopeKey, app, key) {
  const nonce = random(12);
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce }, await aes(await hkdf(scopeKey, `one/app\0${app}`), "encrypt"), key);
  return concat(nonce, new Uint8Array(sealed));
}

/// An app's key, from its sealed copy and its scope's key.
export async function openAppKey(scopeKey, app, sealed) {
  sealed = bytes(sealed);
  const key = await aes(await hkdf(scopeKey, `one/app\0${app}`), "decrypt");
  return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: sealed.slice(0, 12) }, key, sealed.slice(12)));
}

/// A database's key in an app, from the app's key.
export function databaseKey(appKey, database) {
  return hkdf(appKey, `one/database\0${database}`);
}

async function aes(key, use) {
  return crypto.subtle.importKey("raw", key, "AES-GCM", false, [use]);
}

const ZERO = new Uint8Array(12);

/// A fresh X25519 key pair: { secret, public }, each 32 bytes (the secret is X25519's scalar, unclamped).
export async function keyPair() {
  const pair = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits"]);
  const jwk = await crypto.subtle.exportKey("jwk", pair.privateKey);
  return { secret: b64u.decode(jwk.d), public: b64u.decode(jwk.x) };
}

/// Seals `plain` to an X25519 public key.
export async function sealTo(to, plain) {
  to = bytes(to);
  const pair = await crypto.subtle.generateKey({ name: "X25519" }, true, ["deriveBits"]);
  const recipient = await crypto.subtle.importKey("raw", to, { name: "X25519" }, false, []);
  const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: recipient }, pair.privateKey, 256));
  const share = new Uint8Array(await crypto.subtle.exportKey("raw", pair.publicKey));
  const key = await hkdf(shared, "one/sealed-to", concat(share, to));
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv: ZERO }, await aes(key, "encrypt"), bytes(plain));
  return concat(share, new Uint8Array(sealed));
}

/// Opens what was sealed to the key pair `{ secret, public }`: its secret 32 bytes, or a CryptoKey (a browser's, not
/// extractable).
export async function openSealed(pair, sealed) {
  sealed = bytes(sealed);
  if (sealed.length < 48) throw new Error("not sealed");
  const share = sealed.slice(0, 32);
  const me = pair.secret instanceof CryptoKey ? pair.secret : await crypto.subtle.importKey("jwk", { kty: "OKP", crv: "X25519", d: b64u.encode(pair.secret), x: b64u.encode(pair.public) }, { name: "X25519" }, false, ["deriveBits"]);
  const them = await crypto.subtle.importKey("raw", share, { name: "X25519" }, false, []);
  const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: them }, me, 256));
  const key = await hkdf(shared, "one/sealed-to", concat(share, bytes(pair.public)));
  return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: ZERO }, await aes(key, "decrypt"), sealed.slice(32)));
}

/// The id a copy sealed under `secret` is kept by: it names the copy without opening it.
export async function idOf(secret) {
  return b64u.encode(await hkdf(secret, "one/id"));
}

/// Seals `plain` under a secret.
export async function sealUnder(secret, plain) {
  const nonce = random(12);
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce }, await aes(await hkdf(secret, "one/sealed-under"), "encrypt"), bytes(plain));
  return concat(nonce, new Uint8Array(sealed));
}

/// Opens what was sealed under `secret`, or fails.
export async function openUnder(secret, sealed) {
  sealed = bytes(sealed);
  const key = await aes(await hkdf(secret, "one/sealed-under"), "decrypt");
  return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: sealed.slice(0, 12) }, key, sealed.slice(12)));
}

export async function sha256(v) {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes(v)));
}

// Keys as devices hold them (src/keys.rs): endpoints' keys, epochs, objects sealed under an epoch, signed requests,
// and the keys a caller hands a node for a call.

const hex = {
  encode: (bytes) => Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, "0")).join(""),
  decode(s) {
    if (typeof s !== "string" || !/^([0-9a-f]{2})*$/.test(s)) throw new Error("not hex");
    return Uint8Array.from(s.match(/../g) || [], (b) => parseInt(b, 16));
  },
};
export { hex };

/// JSON's bytes with every object's keys in order and no spaces, as serde_json writes a Value: what's signed.
export function canonical(v) {
  const order = (v) => (Array.isArray(v) ? v.map(order) : v && typeof v === "object" ? Object.fromEntries(Object.keys(v).sort().map((k) => [k, order(v[k])])) : v);
  return text.encode(JSON.stringify(order(v)));
}

const signed = (domain, v) => concat(text.encode(domain), new Uint8Array([0]), canonical(v));

/// An Ed25519 private key to sign with: a CryptoKey (a browser's, not extractable), or a 32-byte seed.
async function signingKey(key) {
  if (key instanceof CryptoKey) return key;
  const seed = bytes(key);
  const pub = await ed25519Public(seed);
  return crypto.subtle.importKey("jwk", { kty: "OKP", crv: "Ed25519", d: b64u.encode(seed), x: b64u.encode(pub) }, { name: "Ed25519" }, false, ["sign"]);
}

/// The public half of an Ed25519 seed.
export async function ed25519Public(seed) {
  const pkcs8 = concat(new Uint8Array([0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20]), bytes(seed));
  const key = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, true, ["sign"]);
  return b64u.decode((await crypto.subtle.exportKey("jwk", key)).x);
}

export async function sign(key, domain, v) {
  return new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, await signingKey(key), signed(domain, v)));
}

export async function verify(publicKey, domain, v, signature) {
  const key = await crypto.subtle.importKey("raw", bytes(publicKey), { name: "Ed25519" }, false, ["verify"]);
  return crypto.subtle.verify({ name: "Ed25519" }, key, bytes(signature), signed(domain, v));
}

/// How old a signed request may be, in milliseconds, either way.
export const FRESH = 60_000;

/// A request signed by an endpoint (`key`, its public half `by`): the fields, with `at`, `by` (hex) and `sig`.
export async function signedRequest(key, by, domain, fields, at = Date.now()) {
  const out = { ...fields, at, by: hex.encode(by) };
  return { ...out, sig: b64u.encode(await sign(key, domain, out)) };
}

/// Checks a signed request at `now`: → `{ by, fields }`, or throws.
export async function checkRequest(domain, request, now = Date.now()) {
  if (!request || typeof request !== "object") throw new Error("not a request");
  const { sig, ...fields } = request;
  if (typeof sig !== "string") throw new Error("it isn't signed");
  const by = hex.decode(String(fields.by));
  if (by.length !== 32) throw new Error("not a key: 64 hex characters");
  if (!Number.isSafeInteger(fields.at) || Math.abs(fields.at - now) > FRESH) throw new Error("it was signed too long ago, or the clock that signed it is wrong");
  if (!(await verify(by, domain, fields, b64u.decode(sig)))) throw new Error("its signature is wrong");
  return { by: fields.by, fields };
}

/// An entry (src/entry.rs), signed by an endpoint (`key`, its public half `by`): `fields` `{ log, seq, backlink,
/// previous, time, user, epoch }` (hashes hex, `backlink` null for a log's first) and `body`, its content's JSON.
/// → `{ header, signature, body }`, each bytes, as `POST /v1/entries` takes them (base64url), and its `hash` (hex).
export async function signEntry(key, by, fields, body) {
  body = bytes(body);
  const { log, seq, backlink = null, previous = [], time, user, epoch } = fields;
  const header = { v: 1, author: hex.encode(by), log, seq, backlink, previous: [...new Set(previous)].sort(), time, user, epoch, body: hex.encode(await sha256(body)), size: body.length };
  const raw = canonical(header);
  return { header: raw, signature: await sign(key, "one/entry", header), body, hash: hex.encode(await sha256(raw)) };
}

/// Checks an entry as Rust does (`Entry::decode`): → its header's fields, `hash` and `content`, or throws.
export async function checkEntry({ header, signature, body }) {
  header = bytes(header);
  body = bytes(body);
  const h = JSON.parse(new TextDecoder().decode(header));
  if (hex.encode(canonical(h)) !== hex.encode(header)) throw new Error("its header isn't canonical JSON");
  if (h.v !== 1) throw new Error("not an entry this device reads");
  if (!Array.isArray(h.previous) || h.previous.some((p, i) => i > 0 && !(h.previous[i - 1] < p))) throw new Error("previous isn't in order");
  if ((h.seq === 0) !== (h.backlink === null)) throw new Error("only an entry that starts its log has no backlink");
  if (h.size !== body.length || h.body !== hex.encode(await sha256(body))) throw new Error("its body isn't the one it names");
  if (!(await verify(hex.decode(h.author), "one/entry", h, signature))) throw new Error("its signature is wrong");
  return { ...h, hash: hex.encode(await sha256(header)), content: JSON.parse(new TextDecoder().decode(body)) };
}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`.
const folder = (scope) => scope.replace(":", "-");

/// Where epoch `n` of a scope is kept sealed to `to` (an endpoint's key, hex, or `recovery`).
export const epochName = (scope, n, to) => `${folder(scope)}/_epochs/${n}/${to}`;

/// Where an app's key in epoch `n` is kept, sealed under that epoch's key.
export const appKeyName = (scope, app, n) => `${folder(scope)}/${app}/_key/${n}`;

/// Seals an object under an epoch's key and its name: epoch (u32, big-endian) ‖ nonce ‖ AES-256-GCM(aad = name).
export async function seal(epoch, key, name, plain) {
  const nonce = random(12);
  const head = new Uint8Array(4);
  new DataView(head.buffer).setUint32(0, epoch);
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: text.encode(name) }, await aes(bytes(key), "encrypt"), bytes(plain));
  return concat(head, nonce, new Uint8Array(sealed));
}

/// The epoch a sealed object names.
export function epochOf(sealed) {
  sealed = bytes(sealed);
  if (sealed.length < 32) throw new Error("too short to be sealed");
  return new DataView(sealed.buffer, sealed.byteOffset).getUint32(0);
}

/// Opens what `seal` sealed, with its epoch's key.
export async function open(key, name, sealed) {
  sealed = bytes(sealed);
  epochOf(sealed);
  return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: sealed.slice(4, 16), additionalData: text.encode(name) }, await aes(bytes(key), "decrypt"), sealed.slice(16)));
}

/// The `for` of a handoff with calls that bring `token`.
export const tokenId = async (token) => hex.encode(await sha256(token));

/// What a caller hands a node, `{ for, until, keys: [{ scope, epoch, app, database, key }] }` (each key base64url),
/// sealed to its seal key: the `x-one-keys` header's value.
export async function sealHandoff(to, handoff) {
  return b64u.encode(await sealTo(to, JSON.stringify(handoff)));
}

/// Opens a handoff sealed to the key pair `{ secret, public }`.
export async function openHandoff(pair, sealed) {
  return JSON.parse(new TextDecoder().decode(await openSealed(pair, b64u.decode(sealed))));
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The code a new device shows: the first 8 base32 characters of the sha256 of its key.
export async function joinCode(key) {
  const h = await sha256(bytes(key));
  let bits = "";
  for (const b of h.slice(0, 5)) bits += b.toString(2).padStart(8, "0");
  return Array.from({ length: 8 }, (_, i) => BASE32[parseInt(bits.slice(i * 5, i * 5 + 5), 2)]).join("");
}

/// A newcomer's proof that their invite's secret was theirs: HKDF-SHA256(secret, "one/invite\0" ‖ their key), hex.
export async function inviteProof(secret, key) {
  return hex.encode(await hkdf(secret, concat(text.encode("one/invite\0"), bytes(key))));
}

/// A passkey's device keys from its PRF output: `{ sign: Ed25519 seed, seal: X25519 secret }`.
export async function passkeyKeys(prf) {
  return { sign: await hkdf(prf, "one/passkey-device/sign"), seal: await hkdf(prf, "one/passkey-device/seal") };
}
