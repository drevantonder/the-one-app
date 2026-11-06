// The Workers' formats, which nodes read too (src/cloudflare.rs; tests/contracts.rs checks they agree).
//
// Keys are a tree per scope. A scope's key is 32 random bytes, and so is each app's key in it, kept sealed under the
// scope's key: nonce ‖ AES-256-GCM(HKDF-SHA256(scope key, "one/app\0<app>"), nonce, app key), so erasing an app (deleting
// its sealed key) destroys its key, and its data with it. A database's key is derived from its app's with HKDF-SHA256.
//
// Sealed to a public key (an X25519 key: a person's, a device's, the recovery key's twin), bytes are
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

/// HKDF-SHA256's 32 bytes from `ikm`: bytes, or a non-extractable HKDF CryptoKey (`hkdfKey`) a browser keeps in memory.
export async function hkdf(ikm, info, salt = new Uint8Array()) {
  const key = ikm instanceof CryptoKey ? ikm : await crypto.subtle.importKey("raw", bytes(ikm), "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: bytes(salt), info: bytes(info) }, key, 256));
}

/// A key to derive from, kept without its bytes: they're wiped once it's made.
export async function hkdfKey(raw) {
  const key = await crypto.subtle.importKey("raw", raw, "HKDF", false, ["deriveBits"]);
  raw.fill(0);
  return key;
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
export async function signingKey(key) {
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

/// The public half of an X25519 secret (a passkey's seal key, say).
export async function x25519Public(secret) {
  const pkcs8 = concat(new Uint8Array([0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20]), bytes(secret));
  const key = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "X25519" }, true, ["deriveBits"]);
  return b64u.decode((await crypto.subtle.exportKey("jwk", key)).x);
}

/// The X25519 key an Ed25519 public key is the twin of (`cloudflare::twin_public`): what's sealed to a Rust endpoint, or
/// to the recovery key, is sealed to it. u = (1 + y) / (1 - y) mod 2^255 - 19.
export function twinPublic(edPublic) {
  const b = bytes(edPublic), P = (1n << 255n) - 19n;
  let y = 0n;
  for (let i = 31; i >= 0; i--) y = (y << 8n) | BigInt(i === 31 ? b[i] & 0x7f : b[i]);
  const pow = (a, e) => {
    let r = 1n;
    for (a %= P; e; e >>= 1n, a = (a * a) % P) if (e & 1n) r = (r * a) % P;
    return r;
  };
  let u = ((1n + y) * pow((1n - y + P) % P, P - 2n)) % P;
  const out = new Uint8Array(32);
  for (let i = 0; i < 32; i++, u >>= 8n) out[i] = Number(u & 255n);
  return out;
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

let lastAt = 0;

/// When to sign what the relay takes once by its `at`: now, or one more than the last this page or Worker signed.
export function nextAt() {
  lastAt = Math.max(Date.now(), lastAt + 1);
  return lastAt;
}

/// Signs and sends again, once, a request that was taken already (another process of this device signed in the same
/// millisecond): `send(at)` signs with `at` and → the problem it was refused with, or nothing.
export async function untilTaken(send) {
  const said = await send(nextAt());
  if (!/taken already/.test(said || "")) return said;
  await new Promise((done) => setTimeout(done, 2 + Math.floor(Math.random() * 23)));
  return send(nextAt());
}

/// A request signed by an endpoint (`key`, its public half `by`): the fields, with `at`, `by` (hex) and `sig`.
export async function signedRequest(key, by, domain, fields, at = nextAt()) {
  const out = { ...fields, at, by: hex.encode(by) };
  return { ...out, sig: b64u.encode(await sign(key, domain, out)) };
}

/// Checks a signed request at `now`: → `{ by, fields }`, or throws.
export async function checkRequest(domain, request, now = Date.now()) {
  const checked = await checkSigned(domain, request);
  if (Math.abs(checked.fields.at - now) > FRESH) throw new Error("it was signed too long ago, or the clock that signed it is wrong");
  return checked;
}

/// A signed request's signature, whenever it was signed: → `{ by, fields }`, or throws.
async function checkSigned(domain, request) {
  if (!request || typeof request !== "object") throw new Error("not a request");
  const { sig, ...fields } = request;
  if (typeof sig !== "string") throw new Error("it isn't signed");
  const by = hex.decode(String(fields.by));
  if (by.length !== 32) throw new Error("not a key: 64 hex characters");
  if (!Number.isSafeInteger(fields.at)) throw new Error("it doesn't say when it was signed");
  if (!(await verify(by, domain, fields, b64u.decode(sig)))) throw new Error("its signature is wrong");
  return { by: fields.by, fields };
}

/// What a device signs to vouch for an endpoint of a session of its own, and for how long at most (ms).
export const ENDPOINT = "one/endpoint";
export const SESSION = 24 * 3_600_000;

/// A device's word (`key`, its public half `by`) that `endpoint` (bytes) is its own until `until`: base64url JSON, as
/// the relay and nodes take it (keys::certify).
export async function certify(key, by, endpoint, until, at = Date.now()) {
  return b64u.encode(canonical(await signedRequest(key, by, ENDPOINT, { endpoint: hex.encode(endpoint), until }, at)));
}

/// What a session's endpoint signs to open its relay socket.
export const SOCKET = "one/socket";

/// How old a session's proof for its relay socket may be, either way: a socket is opened as soon as it's made.
export const SOCKET_FRESH = 10_000;

/// A session's endpoint's word (its key's seed `secret`, its public half `endpoint`) that it's opening a relay socket
/// now, with a nonce of its own: base64url JSON, shown after its certificate (`<certificate>.<proof>`), so the relay
/// takes it in with no challenge's round trip, once.
export async function proveSocket(secret, endpoint, at = Date.now()) {
  return b64u.encode(canonical(await signedRequest(secret, endpoint, SOCKET, { nonce: hex.encode(random(16)) }, at)));
}

/// Checks a certificate at `now`: → `{ by, endpoint, until }` (keys hex), or throws (keys::check_certificate).
export async function checkCertificate(certificate, now = Date.now()) {
  let request;
  try {
    request = JSON.parse(new TextDecoder().decode(b64u.decode(String(certificate).trim())));
  } catch {
    throw new Error("a certificate is base64url JSON");
  }
  const { by, fields } = await checkSigned(ENDPOINT, request);
  if (typeof fields.endpoint !== "string" || !/^[0-9a-f]{64}$/.test(fields.endpoint)) throw new Error("not a key: 64 hex characters");
  if (!Number.isSafeInteger(fields.until)) throw new Error("a certificate says until when");
  if (fields.at > now + FRESH) throw new Error("it was signed in the future, or the clock that signed it is wrong");
  if (fields.until <= now) throw new Error("that session has ended");
  if (fields.until > Math.min(fields.at, now) + SESSION) throw new Error("a session lasts a day at most");
  return { by, endpoint: fields.endpoint, until: fields.until };
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

/// Signs a step of a scope's database for position `at` (src/database/steps.rs): an entry's header, following nothing,
/// under a domain of its own, so neither is ever taken for the other.
export async function signStep(key, by, { log, at, time, user, epoch }, body) {
  body = bytes(body);
  const header = { v: 1, author: hex.encode(by), log, seq: at, backlink: null, previous: [], time, user, epoch, body: hex.encode(await sha256(body)), size: body.length };
  return { header: canonical(header), signature: await sign(key, "one/step", header), body };
}

const isHash = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);
/// A number an entry may hold: a whole one from 0 to `max`, and never above 2^53 - 1, which JSON keeps exactly here.
const isCount = (n, max = Number.MAX_SAFE_INTEGER) => Number.isSafeInteger(n) && n >= 0 && n <= max;
const U32 = 0xffffffff;

/// Checks an entry as Rust does (`Entry::decode`): → its header's fields, `hash` and `content`, or throws.
export async function checkEntry({ header, signature, body }) {
  header = bytes(header);
  body = bytes(body);
  const h = JSON.parse(new TextDecoder().decode(header));
  if (!h || typeof h !== "object" || Array.isArray(h)) throw new Error("not an entry");
  if (hex.encode(canonical(h)) !== hex.encode(header)) throw new Error("its header isn't canonical JSON");
  if (h.v !== 1) throw new Error("not an entry this device reads");
  if (!isHash(h.author)) throw new Error("no author");
  if (typeof h.log !== "string" || !/^(global|(group|user):[0-9a-f]{16})$/.test(h.log)) throw new Error(`${h.log} isn't a scope: global, group:<id> or user:<id>`);
  if (!isCount(h.seq, U32) || !isCount(h.epoch, U32) || !isCount(h.time) || !isCount(h.size)) throw new Error("its numbers aren't whole numbers it may hold");
  if (typeof h.user !== "string") throw new Error("no user");
  if (!isHash(h.body)) throw new Error("no body");
  if (h.backlink !== null && !isHash(h.backlink)) throw new Error("backlink isn't a hash");
  if (!Array.isArray(h.previous) || !h.previous.every(isHash) || h.previous.some((p, i) => i > 0 && !(h.previous[i - 1] < p))) throw new Error("previous isn't in order");
  if ((h.seq === 0) !== (h.backlink === null)) throw new Error("only an entry that starts its log has no backlink");
  if (h.size !== body.length || h.body !== hex.encode(await sha256(body))) throw new Error("its body isn't the one it names");
  if (!(await verify(hex.decode(h.author), "one/entry", h, signature))) throw new Error("its signature is wrong");
  const content = JSON.parse(new TextDecoder().decode(body));
  if (!content || typeof content.app !== "string" || typeof content.kind !== "string") throw new Error("its body isn't an entry's content");
  if (content.blobs !== undefined && !(Array.isArray(content.blobs) && content.blobs.every((b) => typeof b === "string"))) throw new Error("its blobs aren't names");
  return { ...h, hash: hex.encode(await sha256(header)), content };
}

/// A scope's folder in the bucket: `user:<id>` is written `user-<id>`.
const folder = (scope) => scope.replace(":", "-");

/// Where epoch `n` of a scope is kept sealed to `to` (an endpoint's key, hex, or `recovery`).
export const epochName = (scope, n, to) => `${folder(scope)}/_epochs/${n}/${to}`;

/// What an `epoch` entry says of its key: HKDF-SHA256(key, "one/epoch-hash"), hex.
export const epochHash = async (key) => hex.encode(await hkdf(key, "one/epoch-hash"));

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

/// The code a new device shows: the first 8 base32 characters of the sha256 of its key and seal key.
export async function joinCode(key, seal) {
  const h = await sha256(concat(bytes(key), bytes(seal)));
  let bits = "";
  for (const b of h.slice(0, 5)) bits += b.toString(2).padStart(8, "0");
  return Array.from({ length: 8 }, (_, i) => BASE32[parseInt(bits.slice(i * 5, i * 5 + 5), 2)]).join("");
}

/// A newcomer's proof that their invite's secret was theirs: HKDF-SHA256(secret, "one/invite\0" ‖ their key ‖ their
/// seal key), hex.
export async function inviteProof(secret, key, seal) {
  return hex.encode(await hkdf(secret, concat(text.encode("one/invite\0"), bytes(key), bytes(seal))));
}

/// The OIDC nonce a sign-in for a device in the install whose first entry's hash is `genesis` asks its provider for
/// (`sign_in_proof::nonce`): base64url(sha256("one/join\0" ‖ genesis ‖ key ‖ seal)).
export async function joinNonce(genesis, key, seal) {
  return b64u.encode(await sha256(concat(text.encode("one/join\0"), bytes(genesis), bytes(key), bytes(seal))));
}

/// The challenge a passkey signs for a device in the install `genesis` at `at` (ms) (`sign_in_proof::challenge`):
/// "one/join\0" ‖ genesis ‖ key ‖ seal ‖ `at`, 8 bytes big-endian.
export function joinChallenge(genesis, key, seal, at) {
  const time = new Uint8Array(8);
  new DataView(time.buffer).setBigUint64(0, BigInt(at));
  return concat(text.encode("one/join\0"), bytes(genesis), bytes(key), bytes(seal), time);
}

/// A passkey's device keys from its PRF output: `{ sign: Ed25519 seed, seal: X25519 secret }`.
export async function passkeyKeys(prf) {
  return { sign: await hkdf(prf, "one/passkey-device/sign"), seal: await hkdf(prf, "one/passkey-device/seal") };
}

// MARK: What a node is handed

/// The app a kind of request runs, as clients name kinds to remember what each needed: an app's call (`call <app>`),
/// or its screens (`/v1/screens/<app>`). Null for any other. (src/keys.rs `app_of`.)
export function appOf(route) {
  const m = /^call (.+)$/.exec(route) || /^\/v1\/screens\/([^/]+)$/.exec(route);
  return m ? m[1] : null;
}

/// Whether a client hands a node the key of `need`'s database (`{ scope, app, database }`) with a kind of request
/// (`route`), when the node asks (428): only what that kind of request can use. As src/keys.rs `may_hand`, which says
/// why.
export function mayHand(route, need) {
  const node = (of) => need.app === "node" && of.includes(need.database);
  // Apps' deployed code, which everyone in the install may read.
  const code = need.scope === "global" && node(["blobs"]);
  const app = appOf(route);
  if (app) return need.app === app || node(["vault", "log"]) || code;
  if (["/mcp", "/v1/widgets", "/v1/badges"].includes(route)) return (need.app !== "node" && need.database === "main") || node(["vault", "log"]) || code;
  if (route === "/v1/apps") return node(["log"]) || code;
  if (route === "/v1/vault" || route.startsWith("/v1/vault/")) return node(["vault", "log"]);
  return node(["log"]);
}

/// The most keys a node's 428 may ask one request to hand. As src/keys.rs `MOST_NEEDED`, which says why.
export const MOST_NEEDED = 64;

/// What a node's 428 (`answer`) asks a request of `route` to hand, from the node that answered (`node`, hex): its
/// `need`, once each, or null if it asks for anything it mayn't. As src/keys.rs `asked_for`, which says why.
export function askedFor(route, node, answer) {
  if (!node || answer?.to !== node || !Array.isArray(answer.need)) return null;
  const whole = (n) => n && typeof n === "object" && typeof n.scope === "string" && typeof n.app === "string" && typeof n.database === "string" && Number.isInteger(n.epoch) && n.epoch >= 0 && n.epoch <= 0xffffffff;
  if (!answer.need.every(whole)) return null;
  const need = [...new Map(answer.need.map(({ scope, epoch, app, database }) => [JSON.stringify([scope, epoch, app, database]), { scope, epoch, app, database }])).values()];
  return need.length && need.length <= MOST_NEEDED && need.every((n) => mayHand(route, n)) ? need : null;
}

/// How long a client waits, each time, to ask again a node that doesn't list it yet. As src/keys.rs `BEHIND`.
export const BEHIND = [2_000, 4_000];

/// What sign-in said of this device's request to be let in (its status, 0 if it didn't answer, and its JSON): `{ signedOut }`,
/// `{ approved, genesis? }`, `{ gone }`, or `{}` while it waits. As src/keys.rs `waited`, which says what each means.
export function waited(status, said) {
  if (status === 401) return { signedOut: true };
  if (status === 404) return { gone: true };
  if (status >= 200 && status < 300 && said?.approved === true) {
    // Where the approving device read who's who to (src/database/steps.rs `Welcome::parse`): `<freeze>.<at>.<hash>`.
    const w = typeof said.welcome === "string" && /^([1-9]\d{0,15})\.([1-9]\d{0,15})\.[0-9a-f]{64}$/.exec(said.welcome);
    const welcome = w && Number(w[2]) >= Number(w[1]) && Number(w[2]) < 2 ** 53 ? said.welcome : null;
    return { approved: true, ...(typeof said.genesis === "string" && /^[0-9a-f]{64}$/.test(said.genesis) ? { genesis: said.genesis } : {}), ...(welcome ? { welcome } : {}) };
  }
  return {};
}

/// The oldest epoch of `database`'s key a reader needs, of a scope whose newest is `newest`, from its newest hint
/// (`hint`, its text, or null if it has none). As src/keys.rs `oldest_needed`, which says why.
export function oldestNeeded(database, hint, newest) {
  if (database === "blobs") return 1;
  newest = Math.max(1, newest || 0);
  if (hint == null) return newest;
  let v = {};
  try {
    v = JSON.parse(hint) || {};
  } catch {}
  const n = (x) => (Number.isSafeInteger(x) && x > 0 ? x : 0);
  return n(v.copy) === 0 ? 1 : Math.min(Math.max(n(v.epoch), 1), newest);
}
