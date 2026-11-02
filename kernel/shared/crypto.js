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

/// Opens what was sealed to the key pair `{ secret, public }`.
export async function openSealed(pair, sealed) {
  sealed = bytes(sealed);
  if (sealed.length < 48) throw new Error("not sealed");
  const share = sealed.slice(0, 32);
  const me = await crypto.subtle.importKey("jwk", { kty: "OKP", crv: "X25519", d: b64u.encode(pair.secret), x: b64u.encode(pair.public) }, { name: "X25519" }, false, ["deriveBits"]);
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
