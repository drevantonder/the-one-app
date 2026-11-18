// What lets a passkey unlock its person's key on a new device, as the encryption app keeps that key
// (`apps/encryption/client/native`). A passkey's WebAuthn PRF output, which only the device it's used on ever has,
// derives an X25519 key pair (HPKE's `DeriveKeyPair`, RFC 9180). Its public key is kept with the passkey, and a
// device that holds the person's key encrypts it to that key with HPKE. On a new device, the passkey's PRF output
// opens it here, and it's encrypted to the new device's own key, as a device that holds it lets one in. Nothing but
// the public key leaves this page: not the PRF output, nor the person's key unencrypted.
import { CipherSuite, DhkemX25519HkdfSha256, HkdfSha256 } from "@hpke/core"
import { Chacha20Poly1305 } from "@hpke/chacha20poly1305"
import { ed25519 } from "@noble/curves/ed25519.js"

/// HPKE as the encryption app's `cipher` has it: X25519, HKDF-SHA256 and ChaCha20-Poly1305.
const suite = new CipherSuite({ kem: new DhkemX25519HkdfSha256(), kdf: new HkdfSha256(), aead: new Chacha20Poly1305() })

/// What a passkey's PRF is asked for: the same on every device, and its own for each network. Better Auth's types
/// have WebAuthn's JSON form of extensions, which has no PRF yet, and it hands them on as they are.
export const PRF = { prf: { eval: { first: new TextEncoder().encode(`one/encryption/1/passkey\n${location.host}`) } } } as AuthenticationExtensionsClientInputs

/// What a passkey that can't unlock anything is told.
export const NO_PRF = "This passkey can't unlock your key on a new device, so approve it from a device that holds your key, or with your recovery key. Passkeys in iCloud Keychain, Google Password Manager, 1Password and Windows Hello can."

/// The PRF output a passkey gave, if it gave one. A passkey that says it has PRF may still give none as it's made.
export function output(results: AuthenticationExtensionsClientOutputs | undefined) {
  const first = results?.prf?.results?.first
  return first ? new Uint8Array(first as ArrayBuffer) : undefined
}

const hex = (bytes: ArrayBuffer | Uint8Array) => Array.from(new Uint8Array(bytes), (byte) => byte.toString(16).padStart(2, "0")).join("")
const unhex = (text: string) => Uint8Array.from(text.match(/../g) ?? [], (byte) => parseInt(byte, 16))
const base64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes))
const unbase64 = (text: string) => Uint8Array.from(atob(text), (char) => char.charCodeAt(0))
const text = (what: string) => new TextEncoder().encode(what)

/// The public key a passkey's PRF output derives, in hex, as it's kept with the passkey.
export async function prfKey(prf: Uint8Array) {
  const { publicKey } = await suite.kem.deriveKeyPair(prf)
  return hex(await suite.kem.serializePublicKey(publicKey))
}

/// `owner`'s key, as a device that holds it encrypted it to the passkey `credential`, opened by its PRF output.
export async function opened(prf: Uint8Array, owner: string, credential: string, encrypted: string) {
  const sealed = unbase64(encrypted)
  const recipientKey = await suite.kem.deriveKeyPair(prf)
  const info = text(`one/encryption/1/passkey\n${owner}\n${credential}`)
  return new Uint8Array(await suite.open({ recipientKey, enc: sealed.slice(0, 32), info }, sealed.slice(32)))
}

/**
 * `owner`'s key for the device whose key is `device`, as the encryption app's `approve` takes it: encrypted to the
 * device's key, the X25519 form of its Ed25519 one, and what tells it from another key.
 */
export async function forDevice(key: Uint8Array<ArrayBuffer>, owner: string, device: string) {
  const recipientPublicKey = await suite.kem.deserializePublicKey(ed25519.utils.toMontgomery(unhex(device)))
  const { enc, ct } = await suite.seal({ recipientPublicKey, info: text(`one/encryption/1/owner\n${owner}\n${device}`) }, key)
  const hkdf = await crypto.subtle.importKey("raw", key, "HKDF", false, ["deriveBits"])
  const check = await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: new Uint8Array(32), info: text("one/encryption/1/check") }, hkdf, 256)
  return JSON.stringify({ check: hex(check), slot: base64(new Uint8Array([...new Uint8Array(enc), ...new Uint8Array(ct)])) })
}
