// Stand-in sites for tests. The bucket (https://s3.test/network/…, and /network/… asked directly) keeps objects in
// memory, each with an ETag that changes as it does, creating one only if it isn't there when asked with
// If-None-Match: *, and replacing one only while it's the version If-Match names. A provider (Google's addresses, as any
// OIDC provider's) publishes its discovery document and keys, and its token endpoint, given an authorization code, signs
// in whoever it names with an ID token signed by those keys: the code is its claims. Anything else answers with what it
// was sent, except to a browser's push service (`/fcm/send/…`), which keeps what it's sent for `GET /pushes` to give.
const objects = new Map();
const pushes = [];
const tags = new Map();
let written = 0;

const ISSUER = "https://accounts.google.com";
const b64u = (bytes) => btoa(String.fromCharCode(...new Uint8Array(bytes))).replaceAll("+", "-").replaceAll("/", "_").replace(/=+$/, "");
const text = (s) => b64u(new TextEncoder().encode(s));
let signing;
const signingKey = () => (signing ||= crypto.subtle.generateKey({ name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }, true, ["sign", "verify"]));

export default {
  async fetch(request) {
    const url = new URL(request.url);
    if (url.href === `${ISSUER}/.well-known/openid-configuration`) {
      return Response.json({ issuer: ISSUER, authorization_endpoint: `${ISSUER}/o/oauth2/v2/auth`, token_endpoint: "https://oauth2.googleapis.com/token", userinfo_endpoint: "https://openidconnect.googleapis.com/v1/userinfo", jwks_uri: "https://www.googleapis.com/oauth2/v3/certs", id_token_signing_alg_values_supported: ["RS256"] });
    }
    if (url.href === "https://www.googleapis.com/oauth2/v3/certs") {
      const { publicKey } = await signingKey();
      return Response.json({ keys: [{ ...(await crypto.subtle.exportKey("jwk", publicKey)), kid: "k1", alg: "RS256", use: "sig" }] });
    }
    const form = url.host === "oauth2.googleapis.com" && new URLSearchParams(await request.clone().text());
    if (form && form.get("grant_type") === "authorization_code") {
      const now = Math.floor(Date.now() / 1000);
      const claims = { iss: ISSUER, aud: form.get("client_id"), iat: now, exp: now + 3600, ...JSON.parse(atob(form.get("code").replaceAll("-", "+").replaceAll("_", "/"))) };
      const signed = `${text(JSON.stringify({ alg: "RS256", typ: "JWT", kid: "k1" }))}.${text(JSON.stringify(claims))}`;
      // A token some other key signed, as a tampered answer would be.
      const { privateKey } = claims.forged ? await crypto.subtle.generateKey({ name: "RSASSA-PKCS1-v1_5", modulusLength: 2048, publicExponent: new Uint8Array([1, 0, 1]), hash: "SHA-256" }, true, ["sign", "verify"]) : await signingKey();
      const signature = await crypto.subtle.sign("RSASSA-PKCS1-v1_5", privateKey, new TextEncoder().encode(signed));
      return Response.json({ access_token: "a provider's access token", token_type: "Bearer", expires_in: 3600, id_token: `${signed}.${b64u(signature)}` });
    }
    if (url.pathname === "/pushes") return Response.json(pushes.splice(0));
    if (url.pathname.startsWith("/fcm/send/")) {
      const body = new Uint8Array(await request.arrayBuffer());
      pushes.push({ url: request.url, headers: Object.fromEntries(request.headers), body: btoa(String.fromCharCode(...body)) });
      return new Response("", { status: url.pathname.endsWith("/gone") ? 410 : 201 });
    }
    if (url.host === "s3.test" || url.pathname.startsWith("/network/")) {
      const name = url.pathname;
      const etag = { etag: tags.get(name) };
      if (request.method === "GET") return objects.has(name) ? new Response(objects.get(name), { headers: etag }) : new Response("", { status: 404 });
      if (request.method === "HEAD") return new Response(null, objects.has(name) ? { headers: etag } : { status: 404 });
      if (request.method === "DELETE") return new Response(null, { status: objects.delete(name) ? 204 : 404 });
      if (request.headers.get("if-none-match") === "*" && objects.has(name)) return new Response("", { status: 412 });
      if (request.headers.has("if-match") && request.headers.get("if-match") !== tags.get(name)) return new Response("", { status: 412 });
      objects.set(name, new Uint8Array(await request.arrayBuffer()));
      tags.set(name, `"${++written}"`);
      return new Response("", { headers: { etag: tags.get(name) } });
    }
    return Response.json({ method: request.method, url: request.url, headers: Object.fromEntries(request.headers), body: await request.text() });
  },
};
