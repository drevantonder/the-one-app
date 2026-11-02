// Access tokens: JWTs the auth server signs with Ed25519 (EdDSA), its public keys at `<auth>/jwks`. Claims: iss (the
// auth server's URL), sub (the person's user id), aud (what it's for), azp and client_id (the client), scope, sid,
// iat, exp, jti. Compute servers check them the same way (src/kernel.rs).

import { b64u } from "./crypto.js";

const json = (part) => JSON.parse(new TextDecoder().decode(b64u.decode(part)));

/// The claims of `token`, if `jwks` signed it, `iss` issued it and it hasn't expired; otherwise it throws.
export async function verify(token, jwks, iss, now = Date.now()) {
  const parts = String(token).split(".");
  if (parts.length !== 3) throw new Error("not a token");
  const header = json(parts[0]);
  if (header.alg !== "EdDSA") throw new Error("not a token signed with Ed25519");
  const jwk = (jwks.keys || []).find((k) => k.kid === header.kid && k.kty === "OKP" && k.crv === "Ed25519");
  if (!jwk) throw new Error("signed by a key the auth server doesn't publish");
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
