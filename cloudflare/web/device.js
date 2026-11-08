// This browser as a device of its user (ARCHITECTURE.md): its own Ed25519 key, which signs, and X25519 key, which
// what's sealed to it is sealed to, both made here, not extractable, kept in IndexedDB by user; and what it asks to be
// let in with. Everything it does as that device is one-client's keyring (client/src/keyring.rs, settings.rs), in the
// worker that reaches the nodes (net-worker.js), which signs with these keys without reading them. The epoch, app and
// database keys it opens live in that worker's WebAssembly memory.

import { b64u, ed25519Public, hex, joinCode, passkeyKeys, x25519Public } from "../shared/crypto.js";
import { del, get, put } from "./kept.js";


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

/// The device a sign-in in this browser is for (`join`, its key ‖ seal key): the keys the first user it signs in as
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

/// The code this browser shows to be approved: its key's, as 4 and 4.
export const codeOf = async (key, seal) => (await joinCode(hex.decode(key), hex.decode(seal))).replace(/^(.{4})/, "$1 ");

/// What the user's devices call this browser: "Chrome on Mac".
export function browserName() {
  const ua = navigator.userAgent;
  const browser = /Edg\//.test(ua) ? "Edge" : /Firefox\//.test(ua) ? "Firefox" : /Chrome\//.test(ua) ? "Chrome" : /Safari\//.test(ua) ? "Safari" : "A browser";
  const os = /iPhone|iPad/.test(ua) ? "iPhone" : /Mac OS X/.test(ua) ? "Mac" : /Android/.test(ua) ? "Android" : /Windows/.test(ua) ? "Windows" : /Linux/.test(ua) ? "Linux" : "";
  return os ? `${browser} on ${os}` : browser;
}
