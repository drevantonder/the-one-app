// Access tokens: JWTs the sign-in signs with Ed25519 (EdDSA), its public keys at `<sign-in>/jwks`. Claims: iss (the
// sign-in's URL), sub (the person's user id), aud (what it's for), azp and client_id (the client), scope, sid,
// iat, exp, jti. Nodes check them the same way (src/cloudflare.rs).

import { b64u } from "./crypto.js";

const json = (part) => JSON.parse(new TextDecoder().decode(b64u.decode(part)));

/// The key in `jwks` that signed a token with this header, if sign-in publishes it.
const publishedKey = (jwks, header) => (jwks.keys || []).find((k) => k.kid === header.kid && k.kty === "OKP" && k.crv === "Ed25519");

/// Whether `jwks` holds the key `token` names; null for what isn't a token.
function namesPublishedKey(jwks, token) {
  try {
    return !!publishedKey(jwks, json(String(token).split(".")[0]));
  } catch {
    return null;
  }
}

/// The claims of `token`, if `jwks` signed it, `iss` issued it and it hasn't expired; otherwise it throws.
export async function verify(token, jwks, iss, now = Date.now()) {
  const parts = String(token).split(".");
  if (parts.length !== 3) throw new Error("not a token");
  const header = json(parts[0]);
  if (header.alg !== "EdDSA") throw new Error("not a token signed with Ed25519");
  const jwk = publishedKey(jwks, header);
  if (!jwk) throw new Error("signed by a key sign-in doesn't publish");
  const key = await crypto.subtle.importKey("jwk", { kty: "OKP", crv: "Ed25519", x: jwk.x }, { name: "Ed25519" }, false, ["verify"]);
  const signed = new TextEncoder().encode(`${parts[0]}.${parts[1]}`);
  if (!(await crypto.subtle.verify({ name: "Ed25519" }, key, b64u.decode(parts[2]), signed))) throw new Error("its signature is wrong");
  const claims = json(parts[1]);
  if (claims.iss !== iss) throw new Error(`issued by ${claims.iss}, not ${iss}`);
  if (typeof claims.exp !== "number" || claims.exp * 1000 <= now) throw new Error("it has expired");
  if (typeof claims.sub !== "string" || !claims.sub) throw new Error("it names no one");
  return claims;
}

/// The bearer token in a request, if there is one.
export function bearer(request) {
  const auth = request.headers.get("authorization") || "";
  return auth.toLowerCase().startsWith("bearer ") ? auth.slice(7).trim() : null;
}

/// Sign-in's keys, fetched with `get()` (→ a Response of its JWKS) and kept five minutes, or until a token
/// names a key that isn't among them.
export class Issuer {
  constructor(iss, get) {
    this.iss = iss;
    this.get = get;
  }

  async verify(token) {
    const now = Date.now();
    for (const fresh of [false, true]) {
      if (fresh || !this.jwks || this.at < now - 300_000) {
        const resp = await this.get();
        if (!resp.ok) throw new Error("sign-in didn't give its keys");
        this.jwks = await resp.json();
        this.at = now;
      }
      if (fresh || namesPublishedKey(this.jwks, token) !== false) return await verify(token, this.jwks, this.iss, now);
    }
  }
}
