// A stand-in auth service for tests: it publishes its keys at /jwks and signs whatever claims it's sent at /sign, as
// the auth service signs access tokens (EdDSA, with a kid).
import { b64u } from "../shared/crypto.js";

let pair;
async function keys() {
  if (!pair) {
    const generated = await crypto.subtle.generateKey({ name: "Ed25519" }, true, ["sign", "verify"]);
    pair = { ...generated, jwk: { ...(await crypto.subtle.exportKey("jwk", generated.publicKey)), kid: "test-key", alg: "EdDSA" } };
  }
  return pair;
}

const part = (v) => b64u.encode(new TextEncoder().encode(JSON.stringify(v)));

export default {
  async fetch(request) {
    const { pathname } = new URL(request.url);
    const k = await keys();
    if (pathname.endsWith("/jwks")) return Response.json({ keys: [k.jwk] });
    if (pathname.endsWith("/sign")) {
      const signing = `${part({ alg: "EdDSA", kid: k.jwk.kid, typ: "at+jwt" })}.${part(await request.json())}`;
      const signature = await crypto.subtle.sign({ name: "Ed25519" }, k.privateKey, new TextEncoder().encode(signing));
      return new Response(`${signing}.${b64u.encode(signature)}`);
    }
    return new Response("not found", { status: 404 });
  },
};
