// The Workers' formats, which app runtimes read too (oned/cloudflare/; tests/contracts.rs checks they agree).
//
// Keys are a tree per scope. A scope's key is 32 random bytes, and so is each app's key in it, kept sealed under the
// scope's key: 1 ‖ nonce ‖ AES-256-GCM(HKDF-SHA256(scope key, "one/app\0<app>"), nonce, app key), so erasing an app (deleting
// its sealed key) destroys its key, and its data with it. A database's key is derived from its app's with HKDF-SHA256.
//
// Sealed to a public key (an X25519 key: a user's, a device's, the recovery key's twin), bytes are
// share ‖ AES-256-GCM(key, zero nonce, plain), where share is a fresh X25519 public key and key is HKDF-SHA256 of the
// shared secret, salted with share ‖ the recipient's public key. Each key seals once, so the zero nonce is safe. What's
// kept starts with 1, which is authenticated too; what a notification carries doesn't (`sealTo`).
//
// Sealed under a secret (a refresh token, an access token), bytes are
// 1 ‖ nonce ‖ AES-256-GCM(key, nonce, plain, authenticating that 1), with key and the copy's id both derived from the
// secret. What was sealed before that format has no version and opens forever (tests/sealed/, key/src/lib.rs's FORMAT).

import { CODES, ENDPOINT, FRESH, READS, SESSION, SIGNED_NOTE, SOCKET } from "./protocol.js";

const text = new TextEncoder();

export const b64u = {
  encode(bytes) {
    const all = new Uint8Array(bytes);
    let s = "";
    for (let i = 0; i < all.length; i += 0x8000) s += String.fromCharCode(...all.subarray(i, i + 0x8000));
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

export function concat(...parts) {
  const out = new Uint8Array(parts.reduce((n, p) => n + p.length, 0));
  let at = 0;
  for (const p of parts) {
    out.set(p, at);
    at += p.length;
  }
  return out;
}

/// HKDF-SHA256's 32 bytes from `ikm`.
async function hkdf(ikm, info, salt = new Uint8Array()) {
  const key = await crypto.subtle.importKey("raw", bytes(ikm), "HKDF", false, ["deriveBits"]);
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: bytes(salt), info: bytes(info) }, key, 256));
}

export function random(n = 32) {
  return crypto.getRandomValues(new Uint8Array(n));
}

async function aes(key, use) {
  return crypto.subtle.importKey("raw", key, "AES-GCM", false, [use]);
}

const ZERO = new Uint8Array(12);
const NUMBERED = new Uint8Array([1]);

/// Sealed with no version: what the notifications Worker sends, for a phone's shell, which changes only with a new build from the
/// App Store or TestFlight (`seal_unnumbered_to` in key/src/lib.rs).
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
async function openSealed(pair, sealed) {
  sealed = bytes(sealed);
  if (sealed.length < 48) throw new Error("not sealed");
  const me = pair.secret instanceof CryptoKey ? pair.secret : await crypto.subtle.importKey("jwk", { kty: "OKP", crv: "X25519", d: b64u.encode(pair.secret), x: b64u.encode(pair.public) }, { name: "X25519" }, false, ["deriveBits"]);
  const open = async (body, additionalData) => {
    const share = body.slice(0, 32);
    const them = await crypto.subtle.importKey("raw", share, { name: "X25519" }, false, []);
    const shared = new Uint8Array(await crypto.subtle.deriveBits({ name: "X25519", public: them }, me, 256));
    const key = await hkdf(shared, "one/sealed-to", concat(share, bytes(pair.public)));
    return new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: ZERO, additionalData }, await aes(key, "decrypt"), body.slice(32)));
  };
  if (sealed[0] === NUMBERED[0] && sealed.length >= 49) {
    try {
      return await open(sealed.slice(1), NUMBERED);
    } catch {}
  }
  return open(sealed, new Uint8Array());
}

/// Opens a note sealed to the key pair `pair` (`open_note` in notifications/src/lib.rs): → the note, signed by a device of the network
/// (`by`, hex), or unsigned from the network's own Workers. One whose signature is wrong is refused.
export async function openNote(pair, sealed) {
  const { sig, ...note } = JSON.parse(new TextDecoder().decode(await openSealed(pair, sealed)));
  if (sig !== undefined && !(await verify(hex.decode(String(note.by)), SIGNED_NOTE, note, b64u.decode(sig)))) throw new Error("its signature is wrong");
  return note;
}

/// The id a copy sealed under `secret` is kept by: it names the copy without opening it.
export async function idOf(secret) {
  return b64u.encode(await hkdf(secret, "one/id"));
}

export async function sealUnder(secret, plain) {
  const nonce = random(12);
  const sealed = await crypto.subtle.encrypt({ name: "AES-GCM", iv: nonce, additionalData: NUMBERED }, await aes(await hkdf(secret, "one/sealed-under"), "encrypt"), bytes(plain));
  return concat(NUMBERED, nonce, new Uint8Array(sealed));
}

/// What's sealed in key/src/lib.rs's FORMAT starts with it. Something sealed before can start with that byte by chance, so
/// it's tried as both: AES-GCM says which it was.
export async function openUnder(secret, sealed) {
  sealed = bytes(sealed);
  const key = await aes(await hkdf(secret, "one/sealed-under"), "decrypt");
  const open = async (body, additionalData) => new Uint8Array(await crypto.subtle.decrypt({ name: "AES-GCM", iv: body.slice(0, 12), additionalData }, key, body.slice(12)));
  if (sealed[0] === NUMBERED[0]) {
    try {
      return await open(sealed.slice(1), NUMBERED);
    } catch {}
  }
  return open(sealed, new Uint8Array());
}

export async function sha256(v) {
  return new Uint8Array(await crypto.subtle.digest("SHA-256", bytes(v)));
}

// Keys as devices hold them (key/src/lib.rs): endpoints' keys, epochs and signed requests.

const hex = {
  encode: (bytes) => Array.from(new Uint8Array(bytes), (b) => b.toString(16).padStart(2, "0")).join(""),
  decode(s) {
    if (typeof s !== "string" || !/^([0-9a-f]{2})*$/.test(s)) throw new Error("not hex");
    return Uint8Array.from(s.match(/../g) || [], (b) => parseInt(b, 16));
  },
};
export { hex };

/// A key as the network writes one: 64 hex characters.
export const isKey = (s) => typeof s === "string" && /^[0-9a-f]{64}$/.test(s);

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

export async function ed25519Public(seed) {
  const pkcs8 = concat(new Uint8Array([0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x04, 0x22, 0x04, 0x20]), bytes(seed));
  const key = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "Ed25519" }, true, ["sign"]);
  return b64u.decode((await crypto.subtle.exportKey("jwk", key)).x);
}

export async function x25519Public(secret) {
  const pkcs8 = concat(new Uint8Array([0x30, 0x2e, 0x02, 0x01, 0x00, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x6e, 0x04, 0x22, 0x04, 0x20]), bytes(secret));
  const key = await crypto.subtle.importKey("pkcs8", pkcs8, { name: "X25519" }, true, ["deriveBits"]);
  return b64u.decode((await crypto.subtle.exportKey("jwk", key)).x);
}

async function sign(key, domain, v) {
  return signBytes(key, signed(domain, v));
}

/// `key`'s Ed25519 signature of `data` (`signingKey`'s).
export async function signBytes(key, data) {
  // The app's device key: signing happens there, and the key never reaches this page.
  if (typeof key === "function") return key(data);
  return new Uint8Array(await crypto.subtle.sign({ name: "Ed25519" }, await signingKey(key), data));
}

async function verify(publicKey, domain, v, signature) {
  const key = await crypto.subtle.importKey("raw", bytes(publicKey), { name: "Ed25519" }, false, ["verify"]);
  return crypto.subtle.verify({ name: "Ed25519" }, key, bytes(signature), signed(domain, v));
}

/// Whether a device that says it reads `reads` of who's who (nothing, from an older build, is none) reads less than this
/// build: the URL signer, the relay and auth tell it to update.
export const behind = (reads) => (Number.isSafeInteger(reads) ? reads : 0) < READS;

/// The roles in a role text (the relay's list's: roles joined by spaces).
export const rolesOf = (text) => String(text || "").split(" ");

let lastAt = 0;

/// When to sign what the relay takes once by its `at`: now, or one more than the last this page or Worker signed. A clock
/// set back by more than `FRESH` since the last starts again from now: what was signed in the wrong time is refused anyway.
function nextAt() {
  const now = Date.now();
  lastAt = now > lastAt || lastAt - now > FRESH ? now : lastAt + 1;
  return lastAt;
}

/// Signs and sends again, once, a request the relay refused as replayed (another process of this device signed in the
/// same millisecond): `send(at)` signs with `at` and → the refusal, `{ error, code }`, or nothing.
export async function untilTaken(send) {
  const said = await send(nextAt());
  if (said?.code !== CODES.replayed) return said;
  await new Promise((done) => setTimeout(done, 2 + Math.floor(Math.random() * 23)));
  return send(nextAt());
}

/// A request signed by an endpoint (`key`, its public half `by`): the fields, with `at`, `by` (hex) and `sig`.
export async function signedRequest(key, by, domain, fields, at = nextAt()) {
  const out = { ...fields, at, by: hex.encode(by) };
  return { ...out, sig: b64u.encode(await sign(key, domain, out)) };
}

export async function checkRequest(domain, request, now = Date.now()) {
  const checked = await checkSigned(domain, request);
  if (Math.abs(checked.fields.at - now) > FRESH) throw new Error("it was signed too long ago, or the clock that signed it is wrong");
  return checked;
}

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

/// A device's word (`key`, its public half `by`) that `endpoint` (bytes) is its own until `until`: base64url JSON, as
/// the relay and app runtimes take it (keys::certify).
export async function certify(key, by, endpoint, until, at = Date.now()) {
  return b64u.encode(canonical(await signedRequest(key, by, ENDPOINT, { endpoint: hex.encode(endpoint), until }, at)));
}

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
  if (!isKey(fields.endpoint)) throw new Error("not a key: 64 hex characters");
  if (!Number.isSafeInteger(fields.until)) throw new Error("a certificate says until when");
  if (fields.at > now + FRESH) throw new Error("it was signed in the future, or the clock that signed it is wrong");
  if (fields.until <= now) throw new Error("that session has ended");
  if (fields.until > Math.min(fields.at, now) + SESSION) throw new Error("a session lasts a day at most");
  return { by, endpoint: fields.endpoint, until: fields.until };
}

const BASE32 = "ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// The code a new device shows (`keys::join_code`): the first 16 base32 characters of the sha256 of its key and seal key.
export async function joinCode(key, seal) {
  const h = await sha256(concat(bytes(key), bytes(seal)));
  let bits = "";
  for (const b of h.slice(0, 10)) bits += b.toString(2).padStart(8, "0");
  return Array.from({ length: 16 }, (_, i) => BASE32[parseInt(bits.slice(i * 5, i * 5 + 5), 2)]).join("");
}

/// The OIDC nonce a sign-in for a device in the network whose root is `root` asks its provider for
/// (`sign_in_proof::nonce`): base64url(sha256("one/join\0" ‖ root ‖ key ‖ seal)).
export async function joinNonce(root, key, seal) {
  return b64u.encode(await sha256(concat(text.encode("one/join\0"), bytes(root), bytes(key), bytes(seal))));
}

/// The challenge a passkey signs for a device in the network `root` at `at` (ms) (`sign_in_proof::challenge`):
/// "one/join\0" ‖ root ‖ key ‖ seal ‖ `at`, 8 bytes big-endian.
export function joinChallenge(root, key, seal, at) {
  const time = new Uint8Array(8);
  new DataView(time.buffer).setBigUint64(0, BigInt(at));
  return concat(text.encode("one/join\0"), bytes(root), bytes(key), bytes(seal), time);
}


/// Where an OIDC issuer says which keys it signs with: as `sign_in_proof::discovery` says, a slash at its end or none.
export const discoveryUrl = (issuer) => `${String(issuer).replace(/\/+$/, "")}/.well-known/openid-configuration`;
