// How a piece of the edge knows which key is asking: a server or client signs what it asks with its key (Ed25519,
// the key it's reached by). The server signs the same bytes in `core/server/lib/one_core/edge.ex`, and its NIF in
// `core/server/nif/one_core_iroh/src/outside.rs`.

import { bytesToHex, hexToBytes } from "@noble/hashes/utils.js";

export const hex = { encode: bytesToHex, decode: hexToBytes };

/** The network's own prefix. A server on its list acts for the network, and reaches every user's and group's blobs. */
export const NETWORK = "network/";

export const isKey = (key: unknown): key is string => typeof key === "string" && /^[0-9a-f]{64}$/.test(key);

const utf8 = new TextEncoder();

/**
 * The keys that have signed something, imported, so a key that asks again isn't imported again. A key is kept only
 * once what it signed checked out, and the list starts again when it's long: anyone can make keys.
 */
const imported = new Map<string, CryptoKey>();
const KEPT = 1000;

/** Whether `signature` (hex) is `key`'s (hex) over `message` (text or bytes). */
export async function signedBy(key: unknown, signature: unknown, message: string | Uint8Array): Promise<boolean> {
  if (!isKey(key) || typeof signature !== "string" || !/^[0-9a-f]{128}$/.test(signature)) return false;
  const bytes = typeof message === "string" ? utf8.encode(message) : message;
  try {
    const verifying = imported.get(key) ?? (await crypto.subtle.importKey("raw", hex.decode(key), "Ed25519", false, ["verify"]));
    const signed = await crypto.subtle.verify("Ed25519", verifying, hex.decode(signature), bytes);
    if (signed) {
      if (imported.size >= KEPT) imported.clear();
      imported.set(key, verifying);
    }
    return signed;
  } catch {
    // Not a point on the curve: nobody's key.
    return false;
  }
}

/**
 * How far an ask's clock may be from ours: 15 minutes, as AWS's Signature Version 4 allows. A server signs an ask
 * before it waits its turn to send it (`OneCore.Bucket.S3`'s `@at_once`), and one starting under heavy load waited
 * more than a minute for its turn. An ask seen on the wire is no use to anyone after this.
 */
const FRESH = 15 * 60_000;

/**
 * The answer to `request`, which isn't `verifiedKey`'s, as S3 answers one it refuses: by S3's code, which says
 * whether its clock was too far from ours (`RequestTimeTooSkewed`) or what it signed isn't its key's
 * (`SignatureDoesNotMatch`), and with our time in `Date`. A signer refused for its clock corrects it by that and
 * asks again (`OneCore.Edge.ask!/2`).
 */
export function unsigned(request: Request): Response {
  const skewed = !(Math.abs(Date.now() - Number(request.headers.get("one-at"))) <= FRESH);
  const [error, code] = skewed
    ? ["Sign what you ask by a clock within 15 minutes of the right time", "RequestTimeTooSkewed"]
    : ["Sign what you ask with your key", "SignatureDoesNotMatch"];
  return Response.json({ error, code }, { status: 401, headers: { date: new Date().toUTCString() } });
}

/**
 * The key that signed `request`, whose body is `body`, or null. An ask carries `one-key`, `one-at` (ms) and
 * `one-signature`, over its path, when it was made and its body.
 */
export async function verifiedKey(request: Request, body: string): Promise<string | null> {
  const key = request.headers.get("one-key");
  const at = request.headers.get("one-at");
  if (!(Math.abs(Date.now() - Number(at)) <= FRESH)) return null;
  const message = `one/ask/1\n${new URL(request.url).pathname}\n${at}\n${body}`;
  return (await signedBy(key, request.headers.get("one-signature"), message)) ? key : null;
}
