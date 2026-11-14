// How a piece at the address knows which key is asking: a server or client signs what it asks with its key (Ed25519,
// the key it's reached by). The server signs the same bytes in `server/lib/one_core/address.ex`, and its add-on in
// `server/native/one_core_iroh/src/outside.rs`.

export const hex = {
  encode: (bytes) => Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(""),
  decode: (text) => Uint8Array.from(text.match(/../g) ?? [], (byte) => parseInt(byte, 16)),
};

export const isKey = (key) => typeof key === "string" && /^[0-9a-f]{64}$/.test(key);

const utf8 = new TextEncoder();

/** Whether `signature` (hex) is `key`'s (hex) over `message` (text or bytes). */
export async function signedBy(key, signature, message) {
  if (!isKey(key) || typeof signature !== "string" || !/^[0-9a-f]{128}$/.test(signature)) return false;
  const bytes = typeof message === "string" ? utf8.encode(message) : message;
  try {
    const imported = await crypto.subtle.importKey("raw", hex.decode(key), "Ed25519", false, ["verify"]);
    return await crypto.subtle.verify("Ed25519", imported, hex.decode(signature), bytes);
  } catch {
    // Not a point on the curve: nobody's key.
    return false;
  }
}

/** How far an ask's clock may be from ours. An ask seen on the wire is no use to anyone after this. */
const FRESH = 60_000;

/**
 * The key that signed `request`, whose body is `body`, or null. An ask carries `one-key`, `one-at` (ms) and
 * `one-signature`, over its path, when it was made and its body.
 */
export async function asker(request, body) {
  const key = request.headers.get("one-key");
  const at = request.headers.get("one-at");
  if (!(Math.abs(Date.now() - Number(at)) <= FRESH)) return null;
  const message = `one/ask/1\n${new URL(request.url).pathname}\n${at}\n${body}`;
  return (await signedBy(key, request.headers.get("one-signature"), message)) ? key : null;
}
