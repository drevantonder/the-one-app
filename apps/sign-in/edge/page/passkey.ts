// What lets a passkey unlock its person's key on a new device, as the encryption app keeps that key
// (`apps/encryption/client/native`). A passkey's WebAuthn PRF output, which only the device it's used on ever has,
// derives an X25519 key pair (HPKE's `DeriveKeyPair`, RFC 9180). Its public key is kept with the passkey, and a
// device that holds the person's key encrypts it to that key with HPKE. On a new device, the passkey's PRF output
// opens it here, and it's encrypted to the new device's own key, as a device that holds it lets one in. Nothing but
// the public key leaves this page: not the PRF output, nor the person's key unencrypted. The same key opens a group's
// or the network's that's sealed to it (`wrap`), and hands what's encrypted to the servers that act for whose it is
// (`forServers`), as the encryption app's `cipher` and device code do (`apps/encryption/client/native`).
import { CipherSuite, DhkemX25519HkdfSha256, HkdfSha256 } from "@hpke/core"
import { Chacha20Poly1305 } from "@hpke/chacha20poly1305"
import { ed25519 } from "@noble/curves/ed25519.js"
import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js"

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

const base64 = (bytes: Uint8Array) => btoa(String.fromCharCode(...bytes))
const unbase64 = (text: string) => Uint8Array.from(atob(text), (char) => char.charCodeAt(0))
const text = (what: string) => new TextEncoder().encode(what)

/// The public key a passkey's PRF output derives, in hex, as it's kept with the passkey.
export async function prfKey(prf: Uint8Array) {
  const { publicKey } = await suite.kem.deriveKeyPair(prf)
  return bytesToHex(new Uint8Array(await suite.kem.serializePublicKey(publicKey)))
}

/// A passkey's key, as its person checks it's the one a device of theirs allows, as Signal's safety numbers are: 20
/// digits in fours of five from its SHA-256, the same the server lists (`server/sign_in.ex`).
export async function code(key: string) {
  const hash = new Uint8Array(await crypto.subtle.digest("SHA-256", text(`one/passkey/1/code\n${key}`)))
  const number = hash.slice(0, 9).reduce((number, byte) => (number << 8n) | BigInt(byte), 0n)
  return (number % 10n ** 20n).toString().padStart(20, "0").match(/.{5}/g)!.join(" ")
}

/// `owner`'s key, as a device that holds it encrypted it to the passkey `credential`, opened by its PRF output.
export async function opened(prf: Uint8Array, owner: string, credential: string, encrypted: string) {
  const sealed = unbase64(encrypted)
  const recipientKey = await suite.kem.deriveKeyPair(prf)
  const info = text(`one/encryption/1/passkey\n${owner}\n${credential}`)
  return new Uint8Array(await suite.open({ recipientKey, enc: sealed.slice(0, 32), info }, sealed.slice(32)))
}

/// A key derived from another for what `what` says, as the encryption app's `cipher` derives one: HKDF-SHA256 (RFC
/// 5869) with no salt.
async function derived(key: Uint8Array<ArrayBuffer>, what: string) {
  const hkdf = await crypto.subtle.importKey("raw", key, "HKDF", false, ["deriveBits"])
  return new Uint8Array(await crypto.subtle.deriveBits({ name: "HKDF", hash: "SHA-256", salt: new Uint8Array(32), info: text(what) }, hkdf, 256))
}

/// What tells a key from another without giving it away, as the encryption app keeps it beside the key.
export const check = async (key: Uint8Array<ArrayBuffer>) => bytesToHex(await derived(key, "one/encryption/1/check"))

/// `data` encrypted to the machine whose Ed25519 key is `to`, in hex, for what `about` says: HPKE to its X25519 form,
/// as the encryption app's `cipher::seal`.
async function sealed(to: string, about: string, data: Uint8Array<ArrayBuffer>) {
  const recipientPublicKey = await suite.kem.deserializePublicKey(ed25519.utils.toMontgomery(hexToBytes(to)))
  const { enc, ct } = await suite.seal({ recipientPublicKey, info: text(about) }, data)
  return base64(new Uint8Array([...new Uint8Array(enc), ...new Uint8Array(ct)]))
}

/**
 * `owner`'s key for the device whose key is `device`, as the encryption app's `store` takes it: encrypted to the
 * device's key, the X25519 form of its Ed25519 one, and what tells it from another key.
 */
export async function forDevice(key: Uint8Array<ArrayBuffer>, owner: string, device: string) {
  return { check: await check(key), slot: await sealed(device, `one/encryption/1/owner\n${owner}\n${device}`, key) }
}

/// `owner`'s key as `forDevice` encrypted it to the device whose Ed25519 key's seed is `seed`, opened with it: what a
/// device does, which `tooling/keys.ts` does for a device of a network being built.
export async function fromDevice(seed: Uint8Array, owner: string, device: string, slot: string) {
  const sealed = unbase64(slot)
  const recipientKey = await suite.kem.deserializePrivateKey(ed25519.utils.toMontgomerySecret(seed))
  return new Uint8Array(await suite.open({ recipientKey, enc: sealed.slice(0, 32), info: text(`one/encryption/1/owner\n${owner}\n${device}`) }, sealed.slice(32)))
}

/// `owner`'s key, encrypted to their passkey `credential`'s key, which its PRF output derives (`prfKey`).
export async function forPasskey(key: Uint8Array<ArrayBuffer>, owner: string, credential: string, prfKey: string) {
  const recipientPublicKey = await suite.kem.deserializePublicKey(hexToBytes(prfKey))
  const { enc, ct } = await suite.seal({ recipientPublicKey, info: text(`one/encryption/1/passkey\n${owner}\n${credential}`) }, key)
  return base64(new Uint8Array([...new Uint8Array(enc), ...new Uint8Array(ct)]))
}

/// The Ed25519 key `owner`'s key derives that a group's or the network's key is sealed to, for each of its owners, so
/// their passkey opens it with their own: its seed.
const wrapping = (key: Uint8Array<ArrayBuffer>) => derived(key, "one/encryption/1/wrap")
const about = (scope: string, owner: string) => `one/encryption/1/scope\n${scope}\n${owner}`

/// `scope`'s key sealed to its owner `owner`'s own key, as the encryption app's `store` keeps it (`wrapped`).
export async function wrap(scopeKey: Uint8Array<ArrayBuffer>, key: Uint8Array<ArrayBuffer>, owner: string, scope: string) {
  const to = bytesToHex(ed25519.getPublicKey(await wrapping(key)))
  return { check: await check(scopeKey), slot: await sealed(to, about(scope, owner), scopeKey) }
}

/// `scope`'s key, as `wrap` sealed it to `owner`'s, opened with their key.
export async function unwrap(key: Uint8Array<ArrayBuffer>, owner: string, scope: string, wrapped: string) {
  const sealed = unbase64(wrapped)
  const recipientKey = await suite.kem.deserializePrivateKey(ed25519.utils.toMontgomerySecret(await wrapping(key)))
  return new Uint8Array(await suite.open({ recipientKey, enc: sealed.slice(0, 32), info: text(about(scope, owner)) }, sealed.slice(32)))
}

/**
 * An artifact's keys for every epoch up to the next one after `was`, each sealed to one of `servers`, with its
 * owner's signature of which servers they are, as the encryption app's `store` takes them (`artifacts`). Every
 * handover starts a new epoch, so a server that's left out, or held them unlisted, is never given what's written next.
 */
export async function forServers(key: Uint8Array<ArrayBuffer>, artifact: string, was: number, servers: string[]) {
  const epoch = was + 1
  const keys = new Uint8Array(32 * epoch)
  for (let at = 1; at <= epoch; at++) keys.set(await derived(key, `one/encryption/1/artifact\n${artifact}\n${at}`), 32 * (at - 1))
  const slots: Record<string, string> = {}
  try {
    for (const server of servers) slots[server] = await sealed(server, `one/encryption/1/keys\n${artifact}\n${server}`, keys)
  } finally {
    keys.fill(0)
  }
  const signer = await derived(key, "one/encryption/1/signer")
  const list = `one/encryption/1/servers\n${artifact}\n${epoch}\n${servers.toSorted().join("\n")}`
  return { artifact, epoch, was, slots, signature: bytesToHex(ed25519.sign(text(list), signer)), signer: bytesToHex(ed25519.getPublicKey(signer)) }
}
